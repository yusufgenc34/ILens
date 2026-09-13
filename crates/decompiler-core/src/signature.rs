//! Bounded ECMA-335 signature grammar, independent of parser-library types.
use crate::{
    error::{Error, Result},
    reader::Reader,
};
use serde::Serialize;
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Type {
    Primitive(String),
    Named(u32),
    ValueType(u32),
    Generic {
        method: bool,
        index: u32,
    },
    Array(Box<Type>, u32),
    MultiArray(Box<Type>, u32),
    Pointer(Box<Type>),
    ByRef(Box<Type>),
    GenericInstance {
        base: Box<Type>,
        args: Vec<Type>,
    },
    Modified {
        required: bool,
        modifier: u32,
        inner: Box<Type>,
    },
    Pinned(Box<Type>),
    FunctionPointer(Box<MethodSignature>),
    Unknown,
}
impl Type {
    pub fn primitive(name: &str) -> Self {
        Self::Primitive(name.to_owned())
    }
    pub fn is_void(&self) -> bool {
        *self == Self::primitive("void")
    }
    pub fn is_bool(&self) -> bool {
        *self == Self::primitive("bool")
    }
    pub fn is_reference(&self) -> bool {
        matches!(
            self,
            Self::Named(_) | Self::Array(_, _) | Self::MultiArray(_, _)
        ) || matches!(self, Self::GenericInstance { base, .. } if base.is_reference())
            || *self == Self::primitive("string")
            || *self == Self::primitive("object")
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MethodSignature {
    pub has_this: bool,
    pub explicit_this: bool,
    pub convention: u8,
    pub generic_count: u32,
    pub return_type: Type,
    pub parameters: Vec<Type>,
    pub vararg_at: Option<usize>,
}
fn count(r: &mut Reader<'_>) -> Result<u32> {
    let n = r.compressed()?;
    if n > 1024 {
        Err(Error::limit("Signature count exceeds 1024"))
    } else {
        Ok(n)
    }
}
fn token(r: &mut Reader<'_>) -> Result<u32> {
    let n = r.compressed()?;
    let table = match n & 3 {
        0 => 2,
        1 => 1,
        2 => 27,
        _ => return Err(Error::metadata("Invalid TypeDefOrRef signature tag")),
    };
    if n >> 2 == 0 || n >> 2 > 0xffffff {
        return Err(Error::metadata("Invalid signature type row"));
    }
    Ok((table << 24) | (n >> 2))
}
fn ty(r: &mut Reader<'_>, depth: usize) -> Result<Type> {
    if depth > 48 {
        return Err(Error::limit("Signature nesting exceeds 48"));
    }
    let code = r.u8()?;
    let name = match code {
        1 => Some("void"),
        2 => Some("bool"),
        3 => Some("char"),
        4 => Some("sbyte"),
        5 => Some("byte"),
        6 => Some("short"),
        7 => Some("ushort"),
        8 => Some("int"),
        9 => Some("uint"),
        10 => Some("long"),
        11 => Some("ulong"),
        12 => Some("float"),
        13 => Some("double"),
        14 => Some("string"),
        22 => Some("TypedReference"),
        24 => Some("nint"),
        25 => Some("nuint"),
        28 => Some("object"),
        _ => None,
    };
    if let Some(n) = name {
        return Ok(Type::primitive(n));
    }
    Ok(match code {
        0x0f => Type::Pointer(Box::new(ty(r, depth + 1)?)),
        0x10 => Type::ByRef(Box::new(ty(r, depth + 1)?)),
        0x11 => Type::ValueType(token(r)?),
        0x12 => Type::Named(token(r)?),
        0x13 | 0x1e => Type::Generic {
            method: code == 0x1e,
            index: count(r)?,
        },
        0x1d => Type::Array(Box::new(ty(r, depth + 1)?), 1),
        0x14 => {
            let element = ty(r, depth + 1)?;
            let rank = count(r)?;
            if rank == 0 || rank > 32 {
                return Err(Error::metadata("Invalid array rank"));
            }
            let sizes = count(r)?;
            if sizes > rank {
                return Err(Error::metadata("Array sizes exceed rank"));
            }
            for _ in 0..sizes {
                r.compressed()?;
            }
            let bounds = count(r)?;
            if bounds > rank {
                return Err(Error::metadata("Array bounds exceed rank"));
            }
            for _ in 0..bounds {
                r.compressed_signed()?;
            }
            Type::MultiArray(Box::new(element), rank)
        }
        0x15 => {
            let kind = r.u8()?;
            if kind != 0x11 && kind != 0x12 {
                return Err(Error::metadata("Invalid generic instance"));
            }
            let t = token(r)?;
            let base = Box::new(if kind == 0x11 {
                Type::ValueType(t)
            } else {
                Type::Named(t)
            });
            let n = count(r)?;
            let mut args = Vec::new();
            for _ in 0..n {
                args.push(ty(r, depth + 1)?);
            }
            Type::GenericInstance { base, args }
        }
        0x1b => Type::FunctionPointer(Box::new(method_reader(r, depth + 1)?)),
        0x1f | 0x20 => {
            let modifier = token(r)?;
            Type::Modified {
                required: code == 0x1f,
                modifier,
                inner: Box::new(ty(r, depth + 1)?),
            }
        }
        0x45 => Type::Pinned(Box::new(ty(r, depth + 1)?)),
        _ => {
            return Err(Error::metadata(format!(
                "Unsupported signature element {code:#x}"
            )));
        }
    })
}
fn method_reader(r: &mut Reader<'_>, depth: usize) -> Result<MethodSignature> {
    if depth > 48 {
        return Err(Error::limit("Function pointer nesting exceeds 48"));
    }
    let flags = r.u8()?;
    if !matches!(flags & 15, 0..=5 | 8 | 9 | 11) {
        return Err(Error::metadata("Invalid method calling convention"));
    }
    let generic_count = if flags & 0x10 != 0 { count(r)? } else { 0 };
    let n = count(r)?;
    let return_type = ty(r, depth + 1)?;
    let mut parameters = Vec::new();
    let mut vararg_at = None;
    for i in 0..n {
        if r.data.get(r.pos) == Some(&0x41) {
            r.u8()?;
            if vararg_at.is_some() {
                return Err(Error::metadata("Repeated vararg sentinel"));
            }
            vararg_at = Some(i as usize);
        }
        parameters.push(ty(r, depth + 1)?);
    }
    Ok(MethodSignature {
        has_this: flags & 0x20 != 0,
        explicit_this: flags & 0x40 != 0,
        convention: flags & 15,
        generic_count,
        return_type,
        parameters,
        vararg_at,
    })
}
fn complete(r: &Reader<'_>) -> Result<()> {
    if r.pos != r.data.len() {
        Err(Error::metadata("Trailing signature bytes"))
    } else {
        Ok(())
    }
}
pub fn method(data: &[u8]) -> Result<MethodSignature> {
    let mut r = Reader::new(data);
    let result = method_reader(&mut r, 0)?;
    complete(&r)?;
    Ok(result)
}
pub fn field(data: &[u8]) -> Result<Type> {
    let mut r = Reader::new(data);
    if r.u8()? != 6 {
        return Err(Error::metadata("Invalid field signature"));
    }
    let result = ty(&mut r, 0)?;
    complete(&r)?;
    Ok(result)
}
pub fn type_spec(data: &[u8]) -> Result<Type> {
    let mut r = Reader::new(data);
    let result = ty(&mut r, 0)?;
    complete(&r)?;
    Ok(result)
}
pub fn locals(data: &[u8]) -> Result<Vec<Type>> {
    let mut r = Reader::new(data);
    if r.u8()? != 7 {
        return Err(Error::metadata("Invalid local signature"));
    }
    let n = count(&mut r)?;
    let mut result = Vec::new();
    for _ in 0..n {
        result.push(ty(&mut r, 0)?);
    }
    complete(&r)?;
    Ok(result)
}
pub fn method_spec(data: &[u8]) -> Result<Vec<Type>> {
    let mut r = Reader::new(data);
    if r.u8()? != 0x0a {
        return Err(Error::metadata("Invalid generic method instantiation"));
    }
    let n = count(&mut r)?;
    let mut result = Vec::new();
    for _ in 0..n {
        result.push(ty(&mut r, 0)?);
    }
    complete(&r)?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signatures() {
        let s = method(&[0, 2, 8, 8, 8]).unwrap();
        assert_eq!(s.parameters.len(), 2);
        assert_eq!(s.return_type, Type::primitive("int"));
        assert_eq!(
            field(&[6, 0x1d, 8]).unwrap(),
            Type::Array(Box::new(Type::primitive("int")), 1)
        );
    }
    #[test]
    fn bounded() {
        let mut bytes = vec![0x1d; 100];
        bytes.push(8);
        assert!(type_spec(&bytes).is_err());
        assert!(method(&[0, 0, 8, 1]).is_err());
    }
    #[test]
    fn signed_compressed() {
        assert_eq!(Reader::new(&[0x7f]).compressed_signed().unwrap(), -1);
        assert_eq!(Reader::new(&[0x01]).compressed_signed().unwrap(), -64);
    }
}
