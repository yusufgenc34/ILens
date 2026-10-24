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
    Enum(u32, TypeId),
    /// A generic parameter of the verified method (true) or its type: (method, index,
    /// constrained to reference types).
    Generic(bool, u32, bool),
    /// A constructed generic type: (definition, interned argument list, is a class).
    Instance(u32, usize, bool),
    /// A local struct (sequential or auto layout, nongeneric).
    Struct(u32),
    /// A managed pointer (`ref T`) in a signature.
    Pointer(TypeId),
    /// A multidimensional array of this element type and rank.
    MultiArray(TypeId, u32),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stack {
    I4,
    I8,
    I,
    F,
    Ref(TypeId),
    /// A value whose type is known only by identity: a struct, a generic parameter that
    /// may be a value type, or a constructed generic struct.
    Value(TypeId),
    /// A managed pointer to a location of this type.
    Pointer(TypeId),
    Null,
    /// `this` in an instance constructor before its base or chained constructor call.
    This,
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
            Self::Object
                | Self::String
                | Self::Class(_)
                | Self::Array(_)
                | Self::Boxed(_)
                | Self::MultiArray(..)
                | Self::Generic(_, _, true)
                | Self::Instance(_, _, true)
        )
    }
}
pub struct Types<'a> {
    pub a: &'a Assembly,
    values: Vec<Value>,
    interned: HashMap<Value, TypeId>,
    tokens: HashMap<u32, TypeId>,
    work: Cell<usize>,
    /// New user strings an edit introduces, addressed by placeholder tokens.
    pub pending_strings: usize,
    /// Some(false) in an instance constructor, Some(true) in a type initializer.
    pub initializer: Option<bool>,
    /// On the path being verified, whether the constructor already called its base.
    pub this_initialized: bool,
    /// The verified method and its declaring type, whose generic parameters `!!n`/`!n`
    /// name.
    pub context: (u32, u32),
    lists: Vec<Vec<TypeId>>,
    interned_lists: HashMap<Vec<TypeId>, usize>,
    /// Argument and local types, for `ldarga`/`ldloca`.
    pub arg_types: Vec<TypeId>,
    pub local_types: Vec<TypeId>,
}
impl<'a> Types<'a> {
    pub fn new(a: &'a Assembly) -> Self {
        Self {
            a,
            values: vec![],
            interned: HashMap::new(),
            tokens: HashMap::new(),
            work: Cell::new(0),
            pending_strings: 0,
            initializer: None,
            this_initialized: true,
            context: (0, 0),
            lists: vec![],
            interned_lists: HashMap::new(),
            arg_types: vec![],
            local_types: vec![],
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
            Value::Enum(_, underlying) => self.stack(underlying),
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
            Value::Generic(_, _, false) | Value::Instance(_, _, false) | Value::Struct(_) => {
                Stack::Value(id)
            }
            Value::Pointer(target) => Stack::Pointer(target),
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
            Type::MultiArray(element, rank) | Type::Array(element, rank) => {
                let element = self.signature_depth(element, depth + 1)?;
                self.intern(Value::MultiArray(element, *rank))
            }
            Type::Generic { method, index } => {
                let owner = if *method {
                    self.context.0
                } else {
                    self.context.1
                };
                if owner == 0 {
                    return Err(unsupported("Generic parameters outside a generic context"));
                }
                // `where T : class` makes T a reference; otherwise it may be a value.
                let reference = self.a.metadata().generic_params.iter().any(|p| {
                    coded(p.owner) == owner && u32::from(p.number) == *index && p.flags & 4 != 0
                });
                self.intern(Value::Generic(*method, *index, reference))
            }
            Type::GenericInstance { base, args } => {
                let (Type::Named(definition) | Type::ValueType(definition)) = base.as_ref() else {
                    return Err(invalid("Malformed generic instance"));
                };
                let mut ids = Vec::with_capacity(args.len());
                for arg in args {
                    ids.push(self.signature_depth(arg, depth + 1)?);
                }
                let list = self.list(ids)?;
                self.intern(Value::Instance(
                    *definition,
                    list,
                    matches!(base.as_ref(), Type::Named(_)),
                ))
            }
            Type::ByRef(inner) => {
                let target = self.signature_depth(inner, depth + 1)?;
                self.intern(Value::Pointer(target))
            }
            Type::Pointer(_) | Type::FunctionPointer(_) => Err(unsupported(
                "Unmanaged and function pointers are not verifiable",
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
    /// A type its context proves to be a class: a catch clause's type (only objects are
    /// thrown) or a class's base type. An external TypeRef is then admitted as a class
    /// even when no signature establishes its kind.
    pub fn class_type(&mut self, token: u32) -> Result<TypeId> {
        if token >> 24 == 1 && !self.tokens.contains_key(&token) {
            self.a.validate_token(token)?;
            if self
                .core_name(token)?
                .as_deref()
                .and_then(Self::primitive)
                .is_none()
            {
                let id = self.intern(Value::Class(token))?;
                self.tokens.insert(token, id);
                return Ok(id);
            }
        }
        if token >> 24 == 27 {
            return Err(unsupported(
                "Generic catch and base types are not editable yet",
            ));
        }
        self.token(token)
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
                // A generic definition named by itself is its instance over its own
                // parameters, as `this` in its methods.
                if let Some(parameters) = self.a.generics.get(&token) {
                    if token != self.context.1 {
                        return Err(unsupported(
                            "Open generic types are only verified as the declaring type",
                        ));
                    }
                    let count = parameters.len() as u32;
                    let ids = (0..count)
                        .map(|index| {
                            self.signature_depth(
                                &Type::Generic {
                                    method: false,
                                    index,
                                },
                                depth + 1,
                            )
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let list = self.list(ids)?;
                    let class = !matches!(
                        self.core_name(coded(row.extends))?.as_deref(),
                        Some("ValueType" | "Enum")
                    );
                    let id = self.intern(Value::Instance(token, list, class))?;
                    self.tokens.insert(token, id);
                    return Ok(id);
                }
                match self.core_name(coded(row.extends))?.as_deref() {
                    Some("Enum") => {
                        if row.flags & 0x100 == 0 || row.flags & 0x20 != 0 {
                            return Err(invalid("An enum must be a sealed value type"));
                        }
                        let mut fields = self.a.nodes.iter().filter(|n| {
                            n.parent == token && n.token >> 24 == 4 && n.flags & 0x10 == 0
                        });
                        let field = fields
                            .next()
                            .ok_or_else(|| invalid("Enum storage field is missing"))?;
                        if fields.next().is_some()
                            || field.name != "value__"
                            || field.flags & 0x600 != 0x600
                        {
                            return Err(invalid("Invalid enum storage field"));
                        }
                        let (_, _, ty) = self.a.field_ref(field.token)?;
                        let Type::Primitive(name) = &ty else {
                            return Err(invalid("Enum storage must be an integral primitive"));
                        };
                        let underlying = match Self::primitive(name) {
                            Some(
                                v @ (Value::I1
                                | Value::U1
                                | Value::I2
                                | Value::U2
                                | Value::I4
                                | Value::U4
                                | Value::I8
                                | Value::U8),
                            ) => self.intern(v)?,
                            _ => return Err(invalid("Unsupported enum storage type")),
                        };
                        let id = self.intern(Value::Enum(token, underlying))?;
                        self.tokens.insert(token, id);
                        return Ok(id);
                    }
                    Some("ValueType") => {
                        if row.flags & 0x18 == 0x10 {
                            return Err(unsupported(
                                "Explicit-layout structs require overlap analysis",
                            ));
                        }
                        let id = self.intern(Value::Struct(token))?;
                        self.tokens.insert(token, id);
                        return Ok(id);
                    }
                    _ => {}
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
    pub fn storage(&self, id: TypeId) -> Value {
        match self.value(id) {
            Value::Enum(_, underlying) => self.value(underlying),
            value => value,
        }
    }
    fn list(&mut self, ids: Vec<TypeId>) -> Result<usize> {
        if let Some(index) = self.interned_lists.get(&ids) {
            return Ok(*index);
        }
        if self.lists.len() >= 4096 {
            return Err(Error::limit("Too many generic instantiations"));
        }
        self.lists.push(ids.clone());
        self.interned_lists.insert(ids, self.lists.len() - 1);
        Ok(self.lists.len() - 1)
    }
    pub fn compatible(&self, actual: Stack, expected: Stack) -> Result<bool> {
        self.spend()?;
        Ok(actual == expected
            || match (actual, expected) {
                (Stack::Null, Stack::Ref(b)) => self.value(b).reference(),
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
            Stack::This => "uninitialized this".to_owned(),
            Stack::Value(id) => self.describe_value(id),
            Stack::Pointer(id) => format!("ref {}", self.describe_value(id)),
            _ => format!("{ty:?}"),
        }
    }
    fn describe_value(&self, id: TypeId) -> String {
        match self.value(id) {
            Value::Class(t) | Value::Enum(t, _) | Value::Struct(t) => self.a.resolve(t),
            Value::Pointer(target) => format!("ref {}", self.describe_value(target)),
            Value::MultiArray(e, rank) => format!(
                "{}[{}]",
                self.describe_value(e),
                ",".repeat(rank.saturating_sub(1) as usize)
            ),
            Value::Array(e) => format!("{}[]", self.describe_value(e)),
            Value::Boxed(e) => format!("boxed {}", self.describe_value(e)),
            Value::Generic(method, index, _) => {
                format!("{}{index}", if method { "!!" } else { "!" })
            }
            Value::Instance(t, list, _) => format!(
                "{}<{}>",
                self.a.resolve(t),
                self.lists[list]
                    .iter()
                    .map(|a| self.describe_value(*a))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            v => format!("{v:?}"),
        }
    }
}
