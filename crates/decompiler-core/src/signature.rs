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
    /// Also `modreq(IsExternalInit) void`, the return type of init-only setters.
    pub fn is_void(&self) -> bool {
        match self {
            Self::Modified { inner, .. } => inner.is_void(),
            _ => *self == Self::primitive("void"),
        }
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
fn put_compressed(n: u32, out: &mut Vec<u8>) -> Result<()> {
    match n {
        0..=0x7f => out.push(n as u8),
        0x80..=0x3fff => out.extend_from_slice(&(0x8000 | n as u16).to_be_bytes()),
        0x4000..=0x1fff_ffff => out.extend_from_slice(&(0xc000_0000 | n).to_be_bytes()),
        _ => return Err(Error::limit("Signature value exceeds 29 bits")),
    }
    Ok(())
}
/// Encodes the element types a written local signature may hold: primitives, vectors
/// and TypeDef/TypeRef/TypeSpec classes or value types.
pub fn encode_type(t: &Type, out: &mut Vec<u8>) -> Result<()> {
    const PRIMITIVES: [(&str, u8); 18] = [
        ("bool", 2),
        ("char", 3),
        ("sbyte", 4),
        ("byte", 5),
        ("short", 6),
        ("ushort", 7),
        ("int", 8),
        ("uint", 9),
        ("long", 10),
        ("ulong", 11),
        ("float", 12),
        ("double", 13),
        ("string", 14),
        ("TypedReference", 22),
        ("nint", 24),
        ("nuint", 25),
        ("object", 28),
        ("void", 1),
    ];
    match t {
        Type::Primitive(name) => {
            let code = PRIMITIVES
                .iter()
                .find(|(n, _)| n == name)
                .ok_or_else(|| Error::metadata(format!("Unknown primitive {name}")))?;
            out.push(code.1);
        }
        Type::Named(token) | Type::ValueType(token) => {
            out.push(if matches!(t, Type::Named(_)) {
                0x12
            } else {
                0x11
            });
            let tag = match token >> 24 {
                2 => 0,
                1 => 1,
                27 => 2,
                _ => {
                    return Err(Error::metadata(
                        "Signature types must be TypeDef/TypeRef/TypeSpec",
                    ));
                }
            };
            put_compressed(((token & 0xff_ffff) << 2) | tag, out)?;
        }
        Type::Array(element, 1) => {
            out.push(0x1d);
            encode_type(element, out)?;
        }
        // General arrays as C# writes them: no sizes, zero lower bounds.
        Type::MultiArray(element, rank) | Type::Array(element, rank) => {
            out.push(0x14);
            encode_type(element, out)?;
            put_compressed(*rank, out)?;
            put_compressed(0, out)?;
            put_compressed(*rank, out)?;
            for _ in 0..*rank {
                out.push(0);
            }
        }
        Type::GenericInstance { base, args } => {
            out.push(0x15);
            encode_type(base, out)?;
            put_compressed(
                u32::try_from(args.len()).map_err(|_| Error::limit("Too many type arguments"))?,
                out,
            )?;
            for arg in args {
                encode_type(arg, out)?;
            }
        }
        Type::Generic { method, index } => {
            out.push(if *method { 0x1e } else { 0x13 });
            put_compressed(*index, out)?;
        }
        Type::ByRef(inner) => {
            out.push(0x10);
            encode_type(inner, out)?;
        }
        Type::Pointer(inner) => {
            out.push(0x0f);
            encode_type(inner, out)?;
        }
        Type::Pinned(inner) => {
            out.push(0x45);
            encode_type(inner, out)?;
        }
        Type::Modified {
            required,
            modifier,
            inner,
        } => {
            out.push(if *required { 0x1f } else { 0x20 });
            let tag = match modifier >> 24 {
                2 => 0,
                1 => 1,
                27 => 2,
                _ => {
                    return Err(Error::metadata(
                        "Modifiers must be TypeDef/TypeRef/TypeSpec",
                    ));
                }
            };
            put_compressed(((modifier & 0xff_ffff) << 2) | tag, out)?;
            encode_type(inner, out)?;
        }
        Type::FunctionPointer(_) | Type::Unknown => {
            return Err(Error::metadata("Function pointers cannot be written yet"));
        }
    }
    Ok(())
}
/// A MethodDefSig/MethodRefSig (ECMA-335 II.23.2.1-2).
pub fn encode_method(sig: &MethodSignature) -> Result<Vec<u8>> {
    let mut out = vec![
        sig.convention
            | if sig.has_this { 0x20 } else { 0 }
            | if sig.explicit_this { 0x40 } else { 0 }
            | if sig.generic_count > 0 { 0x10 } else { 0 },
    ];
    if sig.generic_count > 0 {
        put_compressed(sig.generic_count, &mut out)?;
    }
    put_compressed(
        u32::try_from(sig.parameters.len()).map_err(|_| Error::limit("Too many parameters"))?,
        &mut out,
    )?;
    encode_type(&sig.return_type, &mut out)?;
    for (i, p) in sig.parameters.iter().enumerate() {
        if sig.vararg_at == Some(i) {
            out.push(0x41);
        }
        encode_type(p, &mut out)?;
    }
    Ok(out)
}
pub fn encode_field(ty: &Type) -> Result<Vec<u8>> {
    let mut out = vec![0x06];
    encode_type(ty, &mut out)?;
    Ok(out)
}
pub fn encode_method_spec(args: &[Type]) -> Result<Vec<u8>> {
    let mut out = vec![0x0a];
    put_compressed(
        u32::try_from(args.len()).map_err(|_| Error::limit("Too many type arguments"))?,
        &mut out,
    )?;
    for arg in args {
        encode_type(arg, &mut out)?;
    }
    Ok(out)
}
/// Rewrites every metadata token a type mentions.
pub fn map_tokens(t: &Type, map: &mut dyn FnMut(u32) -> Result<u32>) -> Result<Type> {
    Ok(match t {
        Type::Named(token) => Type::Named(map(*token)?),
        Type::ValueType(token) => Type::ValueType(map(*token)?),
        Type::Array(e, r) => Type::Array(Box::new(map_tokens(e, map)?), *r),
        Type::MultiArray(e, r) => Type::MultiArray(Box::new(map_tokens(e, map)?), *r),
        Type::Pointer(e) => Type::Pointer(Box::new(map_tokens(e, map)?)),
        Type::ByRef(e) => Type::ByRef(Box::new(map_tokens(e, map)?)),
        Type::Pinned(e) => Type::Pinned(Box::new(map_tokens(e, map)?)),
        Type::GenericInstance { base, args } => Type::GenericInstance {
            base: Box::new(map_tokens(base, map)?),
            args: args
                .iter()
                .map(|a| map_tokens(a, map))
                .collect::<Result<_>>()?,
        },
        Type::Modified {
            required,
            modifier,
            inner,
        } => Type::Modified {
            required: *required,
            modifier: map(*modifier)?,
            inner: Box::new(map_tokens(inner, map)?),
        },
        Type::FunctionPointer(sig) => Type::FunctionPointer(Box::new(map_method_tokens(sig, map)?)),
        other => other.clone(),
    })
}
pub fn map_method_tokens(
    sig: &MethodSignature,
    map: &mut dyn FnMut(u32) -> Result<u32>,
) -> Result<MethodSignature> {
    Ok(MethodSignature {
        return_type: map_tokens(&sig.return_type, map)?,
        parameters: sig
            .parameters
            .iter()
            .map(|p| map_tokens(p, map))
            .collect::<Result<_>>()?,
        ..sig.clone()
    })
}
/// A LocalVarSig blob (ECMA-335 II.23.2.6).
pub fn local_signature(types: &[Type]) -> Result<Vec<u8>> {
    let mut out = vec![0x07];
    put_compressed(
        u32::try_from(types.len()).map_err(|_| Error::limit("Too many locals"))?,
        &mut out,
    )?;
    for t in types {
        encode_type(t, &mut out)?;
    }
    Ok(out)
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
    #[test]
    fn local_signatures_round_trip() {
        let types = vec![
            Type::primitive("int"),
            Type::primitive("string"),
            Type::Array(Box::new(Type::primitive("byte")), 1),
            Type::Named(0x0200_0003),
            Type::ValueType(0x0100_0012),
        ];
        let blob = local_signature(&types).unwrap();
        assert_eq!(locals(&blob).unwrap(), types);
    }
    #[test]
    fn signatures_round_trip_through_the_encoder() {
        let sig = MethodSignature {
            has_this: true,
            explicit_this: false,
            convention: 0,
            generic_count: 1,
            return_type: Type::Generic {
                method: true,
                index: 0,
            },
            parameters: vec![
                Type::ByRef(Box::new(Type::primitive("int"))),
                Type::MultiArray(Box::new(Type::primitive("int")), 2),
                Type::GenericInstance {
                    base: Box::new(Type::Named(0x0100_0004)),
                    args: vec![Type::Generic {
                        method: false,
                        index: 1,
                    }],
                },
            ],
            vararg_at: None,
        };
        assert_eq!(method(&encode_method(&sig).unwrap()).unwrap(), sig);
        let field_type = Type::Array(Box::new(Type::ValueType(0x0200_0002)), 1);
        assert_eq!(
            field(&encode_field(&field_type).unwrap()).unwrap(),
            field_type
        );
        let args = vec![Type::primitive("string")];
        assert_eq!(
            method_spec(&encode_method_spec(&args).unwrap()).unwrap(),
            args
        );
    }
}
