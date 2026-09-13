//! Write-side types retain storage widths, array element types and reference identity.
//! Interning bounds memory independently of evaluation-stack and CFG size.
use super::{invalid, unsupported};
use crate::{
    assembly::{Assembly, coded},
    error::{Error, Result},
    signature::{self, Type},
};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
};
pub type TypeId = usize;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    Bool,
    Char,
    I1,
    U1,
    I2,
    U2,
    I4,
    U4,
    I8,
    U8,
    I,
    U,
    R4,
    R8,
    Object,
    String,
    Class(u32),
    Array(TypeId),
    Boxed(TypeId),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stack {
    I4,
    I8,
    I,
    F,
    Ref(TypeId),
    Null,
}
impl Stack {
    pub fn numeric(self) -> bool {
        matches!(self, Self::I4 | Self::I8 | Self::I | Self::F)
    }
    pub fn reference(self) -> bool {
        matches!(self, Self::Ref(_) | Self::Null)
    }
}
impl Value {
    pub fn reference(self) -> bool {
        matches!(
            self,
            Self::Object | Self::String | Self::Class(_) | Self::Array(_) | Self::Boxed(_)
        )
    }
}
pub struct Types<'a> {
    pub a: &'a Assembly,
    values: Vec<Value>,
    interned: HashMap<Value, TypeId>,
    tokens: HashMap<u32, TypeId>,
    work: Cell<usize>,
}
impl<'a> Types<'a> {
    pub fn new(a: &'a Assembly) -> Self {
        Self {
            a,
            values: vec![],
            interned: HashMap::new(),
            tokens: HashMap::new(),
            work: Cell::new(0),
        }
    }
    fn spend(&self) -> Result<()> {
        let work = self.work.get();
        if work >= 1_000_000 {
            return Err(Error::limit("IL type-resolution work limit exceeded"));
        }
        self.work.set(work + 1);
        Ok(())
    }
    pub fn intern(&mut self, value: Value) -> Result<TypeId> {
        if let Some(id) = self.interned.get(&value) {
            return Ok(*id);
        }
        if self.values.len() >= 4096 {
            return Err(Error::limit("IL validation exceeds 4096 distinct types"));
        }
        let id = self.values.len();
        self.values.push(value);
        self.interned.insert(value, id);
        Ok(id)
    }
    pub fn value(&self, id: TypeId) -> Value {
        self.values[id]
    }
    pub fn stack(&self, id: TypeId) -> Stack {
        match self.value(id) {
            Value::Bool
            | Value::Char
            | Value::I1
            | Value::U1
            | Value::I2
            | Value::U2
            | Value::I4
            | Value::U4 => Stack::I4,
            Value::I8 | Value::U8 => Stack::I8,
            Value::I | Value::U => Stack::I,
            Value::R4 | Value::R8 => Stack::F,
            _ => Stack::Ref(id),
        }
    }
    // Core type aliases are scoped to framework assembly references, never just names.
    pub fn core_name(&self, token: u32) -> Result<Option<String>> {
        if token >> 24 != 1 {
            return Ok(None);
        }
        self.a.validate_token(token)?;
        let row = &self.a.metadata().type_refs[(token & 0xffffff) as usize - 1];
        let scope = coded(row.resolution_scope);
        if scope >> 24 != 35 {
            return Ok(None);
        }
        let mut found = None;
        for reference in &self.a.references {
            self.spend()?;
            if reference.token == scope {
                found = Some(reference);
                break;
            }
        }
        let Some(reference) = found else {
            return Ok(None);
        };
        let identity = &reference.identity;
        if !matches!(
            identity.name.as_str(),
            "mscorlib" | "System.Runtime" | "System.Private.CoreLib" | "netstandard"
        ) || !matches!(
            identity.public_key_token.as_str(),
            "b77a5c561934e089" | "b03f5f7f11d50a3a" | "7cec85d7bea7798e" | "cc7b13ffcd2ddd51"
        ) || identity.culture != "neutral"
        {
            return Ok(None);
        }
        if self.a.string(row.type_namespace)? != "System" {
            return Ok(None);
        }
        Ok(Some(self.a.string(row.type_name)?.to_owned()))
    }
    fn primitive(name: &str) -> Option<Value> {
        Some(match name {
            "bool" | "Boolean" => Value::Bool,
            "char" | "Char" => Value::Char,
            "sbyte" | "SByte" => Value::I1,
            "byte" | "Byte" => Value::U1,
            "short" | "Int16" => Value::I2,
            "ushort" | "UInt16" => Value::U2,
            "int" | "Int32" => Value::I4,
            "uint" | "UInt32" => Value::U4,
            "long" | "Int64" => Value::I8,
            "ulong" | "UInt64" => Value::U8,
            "nint" | "IntPtr" => Value::I,
            "nuint" | "UIntPtr" => Value::U,
            "float" | "Single" => Value::R4,
            "double" | "Double" => Value::R8,
            "string" | "String" => Value::String,
            "object" | "Object" => Value::Object,
            _ => return None,
        })
    }
    pub fn signature(&mut self, ty: &Type) -> Result<TypeId> {
        self.signature_depth(ty, 0)
    }
    fn signature_depth(&mut self, ty: &Type, depth: usize) -> Result<TypeId> {
        self.spend()?;
        if depth > 48 {
            return Err(Error::limit("Write signature nesting exceeds 48"));
        }
        match ty {
            Type::Primitive(name) => self.intern(Self::primitive(name).ok_or_else(|| {
                unsupported(format!(
                    "Type {name} is not supported by IL write validation"
                ))
            })?),
            Type::Named(token) | Type::ValueType(token) => {
                self.a.validate_token(*token)?;
                let reference = matches!(ty, Type::Named(_));
                let id = if token >> 24 == 1
                    && self
                        .core_name(*token)?
                        .as_deref()
                        .and_then(Self::primitive)
                        .is_none()
                {
                    if !reference {
                        return Err(unsupported("Custom value types are not editable yet"));
                    }
                    self.intern(Value::Class(*token))?
                } else {
                    self.token_depth(*token, depth + 1)?
                };
                if self.value(id).reference() != reference {
                    return Err(invalid(
                        "Signature class/value-type marker disagrees with the type definition",
                    ));
                }
                if let Some(old) = self.tokens.insert(*token, id)
                    && old != id
                {
                    return Err(invalid("Conflicting signature type identities"));
                }
                Ok(id)
            }
            Type::Array(element, 1) => {
                let element = self.signature_depth(element, depth + 1)?;
                self.intern(Value::Array(element))
            }
            Type::Array(_, _) | Type::MultiArray(_, _) => Err(unsupported(
                "Multidimensional and non-zero-based arrays are not editable yet; zero-based vectors and jagged arrays are supported",
            )),
            Type::Generic { .. } | Type::GenericInstance { .. } => Err(unsupported(
                "Generic methods and constructed generic types require generic-aware IL validation",
            )),
            Type::ByRef(_) | Type::Pointer(_) | Type::FunctionPointer(_) => Err(unsupported(
                "Byref, pointer and function-pointer signatures require address/lifetime validation",
            )),
            Type::Modified { .. } | Type::Pinned(_) => Err(unsupported(
                "Modified or pinned signatures are not editable yet",
            )),
            Type::Unknown => Err(unsupported("The method contains an unknown signature type")),
        }
    }
    pub fn token(&mut self, token: u32) -> Result<TypeId> {
        self.token_depth(token, 0)
    }
    fn token_depth(&mut self, token: u32, depth: usize) -> Result<TypeId> {
        self.spend()?;
        if depth > 48 {
            return Err(Error::limit("Write type resolution exceeds 48"));
        }
        if let Some(id) = self.tokens.get(&token) {
            return Ok(*id);
        }
        self.a.validate_token(token)?;
        let value = match token >> 24 {
            1 => {
                let core = self.core_name(token)?;
                if let Some(value) = core.as_deref().and_then(Self::primitive) {
                    value
                } else {
                    return Err(unsupported(format!(
                        "The kind of external type {} cannot be established from this method's signatures",
                        self.a.resolve(token)
                    )));
                }
            }
            2 => {
                let row = &self.a.metadata().type_defs[(token & 0xffffff) as usize - 1];
                if self.a.generics.contains_key(&token) {
                    return Err(unsupported(
                        "Methods on generic declaring types are not editable yet",
                    ));
                }
                if matches!(
                    self.core_name(coded(row.extends))?.as_deref(),
                    Some("ValueType" | "Enum")
                ) {
                    return Err(unsupported(
                        "Custom structs and enums require value-type IL validation",
                    ));
                }
                Value::Class(token)
            }
            27 => {
                let row = &self.a.metadata().type_specs[(token & 0xffffff) as usize - 1];
                let ty = signature::type_spec(self.a.blob(row.signature)?)?;
                let id = self.signature_depth(&ty, depth + 1)?;
                self.tokens.insert(token, id);
                return Ok(id);
            }
            _ => {
                return Err(invalid(
                    "Instruction requires a TypeDef, TypeRef or TypeSpec token",
                ));
            }
        };
        let id = self.intern(value)?;
        self.tokens.insert(token, id);
        Ok(id)
    }
    pub fn compatible(&self, actual: Stack, expected: Stack) -> Result<bool> {
        self.spend()?;
        Ok(actual == expected
            || match (actual, expected) {
                (Stack::Null, Stack::Ref(_)) => true,
                (Stack::Ref(a), Stack::Ref(b)) => self.assignable(a, b, 0)?,
                _ => false,
            })
    }
    fn assignable(&self, from: TypeId, to: TypeId, depth: usize) -> Result<bool> {
        self.spend()?;
        if depth > 48 {
            return Err(Error::limit("Reference compatibility exceeds 48 levels"));
        }
        if from == to {
            return Ok(true);
        }
        Ok(match (self.value(from), self.value(to)) {
            (a, Value::Object) => a.reference(),
            (Value::Array(a), Value::Array(b)) => {
                self.value(a).reference()
                    && self.value(b).reference()
                    && self.assignable(a, b, depth + 1)?
            }
            (Value::Class(a), Value::Class(b)) => self.derives(a, b)?,
            _ => false,
        })
    }
    // Only declared local bases/interfaces are traversed. Missing external hierarchies
    // are never guessed from a display name.
    pub fn derives(&self, from: u32, to: u32) -> Result<bool> {
        let mut pending = vec![from];
        let mut seen = HashSet::new();
        while let Some(t) = pending.pop() {
            self.spend()?;
            if t == to {
                return Ok(true);
            }
            if !seen.insert(t) {
                continue;
            }
            if seen.len() > 64 || pending.len() > 256 {
                return Err(Error::limit("IL validation inheritance graph is too large"));
            }
            if t >> 24 != 2 {
                continue;
            }
            self.a.validate_token(t)?;
            let row = &self.a.metadata().type_defs[(t & 0xffffff) as usize - 1];
            let parent = coded(row.extends);
            if parent != 0 {
                pending.push(parent)
            }
            for i in &self.a.metadata().interface_impls {
                self.spend()?;
                if i.class != t & 0xffffff {
                    continue;
                }
                pending.push(coded(i.interface));
                if pending.len() > 256 {
                    return Err(Error::limit("Too many interface edges"));
                }
            }
        }
        Ok(false)
    }
    pub fn merge(&mut self, left: Stack, right: Stack) -> Result<Stack> {
        if self.compatible(left, right)? {
            return Ok(right);
        }
        if self.compatible(right, left)? {
            return Ok(left);
        }
        if left.reference() && right.reference() {
            return Ok(Stack::Ref(self.intern(Value::Object)?));
        }
        Err(invalid(
            "Incompatible stack types/heights at control-flow merge",
        ))
    }
    pub fn describe(&self, ty: Stack) -> String {
        match ty {
            Stack::Ref(id) => self.describe_value(id),
            _ => format!("{ty:?}"),
        }
    }
    fn describe_value(&self, id: TypeId) -> String {
        match self.value(id) {
            Value::Class(t) => self.a.resolve(t),
            Value::Array(e) => format!("{}[]", self.describe_value(e)),
            Value::Boxed(e) => format!("boxed {}", self.describe_value(e)),
            v => format!("{v:?}"),
        }
    }
}
