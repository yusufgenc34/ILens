use crate::{
    cil::{self, MethodBody, Operand, OperandKind},
    error::{Error, Result},
    metadata::MetadataStore,
    pe::Image,
    reader::Reader,
    signature::{self, MethodSignature, Type},
};
use clrmeta::CodedIndex;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
pub fn coded(c: CodedIndex) -> u32 {
    c.table.map(|t| (t as u32) << 24).unwrap_or(0) | c.row
}
fn row<T>(rows: &[T], token: u32) -> Result<&T> {
    rows.get((token & 0xffffff).wrapping_sub(1) as usize)
        .ok_or_else(|| Error::metadata(format!("Invalid token {token:#010x}")))
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub version: String,
    pub culture: String,
    pub public_key_token: String,
}
impl Identity {
    pub fn matches(&self, other: &Self) -> bool {
        self.name.eq_ignore_ascii_case(&other.name)
            && self.version == other.version
            && self.culture.eq_ignore_ascii_case(&other.culture)
            && self
                .public_key_token
                .eq_ignore_ascii_case(&other.public_key_token)
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct AssemblyInfo {
    pub identity: Identity,
    pub runtime: String,
    pub target_framework: crate::inspection::FrameworkInfo,
    pub obfuscation: crate::inspection::ObfuscationInfo,
    pub size: usize,
    pub machine: String,
    pub type_count: usize,
    pub method_count: usize,
    pub diagnostics: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub token: u32,
    pub parent: u32,
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub full_name: String,
    pub flags: u32,
}
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub token: u32,
    pub kind: String,
    pub name: String,
    pub context: String,
}
#[derive(Debug, Clone)]
pub struct MethodRef {
    pub token: u32,
    pub owner: u32,
    pub name: String,
    pub signature: MethodSignature,
    pub generic_args: Vec<Type>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Reference {
    pub token: u32,
    pub identity: Identity,
}

pub struct Assembly {
    pub bytes: Vec<u8>,
    pub image: Image,
    pub store: MetadataStore,
    pub info: AssemblyInfo,
    pub nodes: Vec<Node>,
    pub references: Vec<Reference>,
    pub owners: HashMap<u32, u32>,
    names: HashMap<u32, String>,
    pub generics: HashMap<u32, BTreeMap<u16, String>>,
    nested: HashMap<u32, u32>,
    pub attributes: HashMap<u32, Vec<usize>>,
    pub semantics: HashMap<u32, Vec<(u16, u32)>>,
}
fn charge_names(total: &mut usize, bytes: usize) -> Result<()> {
    *total = total
        .checked_add(bytes)
        .ok_or_else(|| Error::limit("Name index overflow"))?;
    if *total > 32 * 1024 * 1024 {
        return Err(Error::limit("Declaration index exceeds 32 MiB of names"));
    }
    Ok(())
}
fn charge_node(total: &mut usize, n: &Node) -> Result<()> {
    charge_names(total, n.name.len() + n.full_name.len() + n.namespace.len())
}
impl Assembly {
    pub fn load(bytes: Vec<u8>) -> Result<Self> {
        let image = Image::parse(&bytes)?;
        let store = MetadataStore::parse(&bytes[image.metadata.clone()])?;
        let m = &store.parsed;
        let identity = if let Some(a) = m.assembly() {
            Identity {
                name: a.name.clone(),
                version: a.version_string(),
                culture: a
                    .culture
                    .clone()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "neutral".to_owned()),
                public_key_token: a
                    .public_key_token_string()
                    .unwrap_or_else(|| "null".to_owned()),
            }
        } else {
            Identity {
                name: m.strings.get(m.modules[0].name)?.to_owned(),
                version: "0.0.0.0".to_owned(),
                culture: "neutral".to_owned(),
                public_key_token: "null".to_owned(),
            }
        };
        let info = AssemblyInfo {
            identity,
            runtime: m.version().to_owned(),
            target_framework: Default::default(),
            obfuscation: Default::default(),
            size: bytes.len(),
            machine: match image.machine {
                0x14c => "AnyCPU / x86",
                0x8664 => "x64",
                0xaa64 => "ARM64",
                _ => "Other managed PE",
            }
            .to_owned(),
            type_count: m.type_defs.len(),
            method_count: m.method_defs.len(),
            diagnostics: vec![],
        };
        let mut a = Self {
            bytes,
            image,
            store,
            info,
            nodes: vec![],
            references: vec![],
            owners: HashMap::new(),
            names: HashMap::new(),
            generics: HashMap::new(),
            nested: HashMap::new(),
            attributes: HashMap::new(),
            semantics: HashMap::new(),
        };
        a.index()?;
        (
            a.info.target_framework,
            a.info.obfuscation,
            a.info.diagnostics,
        ) = crate::inspection::inspect(&a);
        Ok(a)
    }
    pub fn metadata(&self) -> &clrmeta::Metadata {
        &self.store.parsed
    }
    pub fn string(&self, index: u32) -> Result<&str> {
        if index == 0 {
            Ok("")
        } else {
            Ok(self.metadata().strings.get(index)?)
        }
    }
    pub fn blob(&self, index: u32) -> Result<&[u8]> {
        if index == 0 {
            Ok(&[])
        } else {
            Ok(self.metadata().blobs.get(index)?)
        }
    }
    fn index(&mut self) -> Result<()> {
        let m = &self.store.parsed;
        let mut name_bytes = 0;
        for (i, t) in m.type_defs.iter().enumerate() {
            let token = 0x02000001 + i as u32;
            let mend = m
                .type_defs
                .get(i + 1)
                .map(|t| t.method_list)
                .unwrap_or(m.method_defs.len() as u32 + 1);
            let fend = m
                .type_defs
                .get(i + 1)
                .map(|t| t.field_list)
                .unwrap_or(m.fields.len() as u32 + 1);
            for rid in t.method_list..mend {
                self.owners.insert(0x06000000 | rid, token);
            }
            for rid in t.field_list..fend {
                self.owners.insert(0x04000000 | rid, token);
            }
        }
        for (i, p) in m.property_maps.iter().enumerate() {
            let end = m
                .property_maps
                .get(i + 1)
                .map(|p| p.property_list)
                .unwrap_or(m.properties.len() as u32 + 1);
            for rid in p.property_list..end {
                self.owners.insert(0x17000000 | rid, 0x02000000 | p.parent);
            }
        }
        for (i, p) in m.event_maps.iter().enumerate() {
            let end = m
                .event_maps
                .get(i + 1)
                .map(|p| p.event_list)
                .unwrap_or(m.events.len() as u32 + 1);
            for rid in p.event_list..end {
                self.owners.insert(0x14000000 | rid, 0x02000000 | p.parent);
            }
        }
        for n in &m.nested_classes {
            if self
                .nested
                .insert(0x02000000 | n.nested_class, 0x02000000 | n.enclosing_class)
                .is_some()
            {
                return Err(Error::metadata("Duplicate nested class parent"));
            }
        }
        for token in self.nested.keys() {
            let mut t = *token;
            let mut depth = 0;
            while let Some(p) = self.nested.get(&t) {
                depth += 1;
                if depth > 48 {
                    return Err(Error::metadata("Nested type cycle or depth exceeds 48"));
                }
                t = *p;
            }
        }
        for g in &m.generic_params {
            let name = self.string(g.name)?.to_owned();
            charge_names(&mut name_bytes, name.len())?;
            self.generics
                .entry(coded(g.owner))
                .or_default()
                .insert(g.number, name);
        }
        for (i, at) in m.custom_attributes.iter().enumerate() {
            self.attributes.entry(coded(at.parent)).or_default().push(i);
        }
        for s in &m.method_semantics {
            self.semantics
                .entry(coded(s.association))
                .or_default()
                .push((s.semantics, 0x06000000 | s.method));
        }
        for i in 0..m.type_defs.len() {
            let name = self.type_name_depth(0x02000001 + i as u32, 0)?;
            charge_names(&mut name_bytes, name.len())?;
            self.names.insert(0x02000001 + i as u32, name);
        }
        for (i, t) in m.type_defs.iter().enumerate() {
            let token = 0x02000001 + i as u32;
            let base = self.resolve(coded(t.extends));
            let kind = if t.flags & 0x20 != 0 {
                "interface"
            } else if base == "System.Enum" {
                "enum"
            } else if base == "System.MulticastDelegate" {
                "delegate"
            } else if base == "System.ValueType" {
                "struct"
            } else {
                "class"
            };
            self.nodes.push(Node {
                token,
                parent: self.nested.get(&token).copied().unwrap_or(0),
                kind: kind.to_owned(),
                name: self.string(t.type_name)?.to_owned(),
                namespace: self.string(t.type_namespace)?.to_owned(),
                full_name: self.resolve(token),
                flags: t.flags,
            });
            charge_node(&mut name_bytes, self.nodes.last().expect("just inserted"))?;
        }
        for (i, f) in m.fields.iter().enumerate() {
            self.nodes
                .push(self.node(0x04000001 + i as u32, "field", f.name, f.flags as u32)?);
            charge_node(&mut name_bytes, self.nodes.last().expect("just inserted"))?;
        }
        for (i, method) in m.method_defs.iter().enumerate() {
            let name = self.string(method.name)?;
            self.nodes.push(self.node(
                0x06000001 + i as u32,
                if name == ".ctor" || name == ".cctor" {
                    "constructor"
                } else {
                    "method"
                },
                method.name,
                method.flags as u32,
            )?);
            charge_node(&mut name_bytes, self.nodes.last().expect("just inserted"))?;
        }
        for (i, p) in m.properties.iter().enumerate() {
            self.nodes.push(self.node(
                0x17000001 + i as u32,
                "property",
                p.name,
                p.flags as u32,
            )?);
            charge_node(&mut name_bytes, self.nodes.last().expect("just inserted"))?;
        }
        for (i, e) in m.events.iter().enumerate() {
            self.nodes.push(self.node(
                0x14000001 + i as u32,
                "event",
                e.name,
                e.event_flags as u32,
            )?);
            charge_node(&mut name_bytes, self.nodes.last().expect("just inserted"))?;
        }
        for (i, r) in m.assembly_refs.iter().enumerate() {
            let key = self.blob(r.public_key_or_token)?;
            if key.len() > 16_384 || (r.flags & 1 == 0 && !key.is_empty() && key.len() != 8) {
                return Err(Error::metadata(
                    "Invalid assembly-reference public key or token",
                ));
            }
            let key = if key.is_empty() {
                "null".to_owned()
            } else {
                let token = if r.flags & 1 != 0 {
                    clrmeta::crypto::public_key_token(key).to_vec()
                } else {
                    key.to_vec()
                };
                token.iter().map(|b| format!("{b:02x}")).collect()
            };
            charge_names(
                &mut name_bytes,
                self.string(r.name)?.len() + self.string(r.culture)?.len() + key.len(),
            )?;
            self.references.push(Reference {
                token: 0x23000001 + i as u32,
                identity: Identity {
                    name: self.string(r.name)?.to_owned(),
                    version: format!(
                        "{}.{}.{}.{}",
                        r.major_version, r.minor_version, r.build_number, r.revision_number
                    ),
                    culture: if r.culture == 0 {
                        "neutral".to_owned()
                    } else {
                        self.string(r.culture)?.to_owned()
                    },
                    public_key_token: key,
                },
            });
        }
        if m.method_defs
            .iter()
            .any(|m| m.impl_flags & 3 != 0 && m.rva != 0)
        {
            self.info.diagnostics.push("Some methods have native/runtime implementations; their metadata is inspectable but no CIL is available.".to_owned());
        }
        Ok(())
    }
    fn node(&self, token: u32, kind: &str, name: u32, flags: u32) -> Result<Node> {
        let parent = self.owners.get(&token).copied().unwrap_or(0);
        let name = self.string(name)?.to_owned();
        Ok(Node {
            token,
            parent,
            kind: kind.to_owned(),
            full_name: format!("{}.{}", self.resolve(parent), name),
            namespace: String::new(),
            name,
            flags,
        })
    }
    pub fn validate_token(&self, token: u32) -> Result<()> {
        if token >> 24 == 0x70 {
            self.user_string(token)?;
            return Ok(());
        }
        self.store
            .raw_row(&self.bytes[self.image.metadata.clone()], token)?;
        Ok(())
    }
    fn type_name_depth(&self, token: u32, depth: usize) -> Result<String> {
        if depth > 48 {
            return Err(Error::limit("Type resolution exceeds 48 levels"));
        }
        let m = self.metadata();
        match token >> 24 {
            1 => {
                let t = row(&m.type_refs, token)?;
                let name = self.string(t.type_name)?;
                let ns = self.string(t.type_namespace)?;
                let scope = coded(t.resolution_scope);
                if scope >> 24 == 1 {
                    Ok(format!(
                        "{}.{}",
                        self.type_name_depth(scope, depth + 1)?,
                        name
                    ))
                } else {
                    Ok(if ns.is_empty() {
                        name.to_owned()
                    } else {
                        format!("{ns}.{name}")
                    })
                }
            }
            2 => {
                let t = row(&m.type_defs, token)?;
                let name = self.string(t.type_name)?;
                let ns = self.string(t.type_namespace)?;
                if let Some(parent) = self.nested.get(&token) {
                    Ok(format!(
                        "{}.{}",
                        self.type_name_depth(*parent, depth + 1)?,
                        name
                    ))
                } else {
                    Ok(if ns.is_empty() {
                        name.to_owned()
                    } else {
                        format!("{ns}.{name}")
                    })
                }
            }
            27 => {
                let s = row(&m.type_specs, token)?;
                let t = signature::type_spec(self.blob(s.signature)?)?;
                self.format_type_depth(&t, 0, depth + 1)
            }
            26 => Ok(self.string(row(&m.module_refs, token)?.name)?.to_owned()),
            _ => Err(Error::metadata(format!(
                "Token {token:#x} does not identify a type"
            ))),
        }
    }
    pub fn format_type(&self, ty: &Type, context: u32) -> String {
        self.format_type_depth(ty, context, 0)
            .unwrap_or_else(|_| "/* unresolved type */ object".to_owned())
    }
    fn format_type_depth(&self, ty: &Type, context: u32, depth: usize) -> Result<String> {
        if depth > 48 {
            return Err(Error::limit("Recursive TypeSpec or signature"));
        }
        let inner = |t: &Type| self.format_type_depth(t, context, depth + 1);
        let rendered = match ty {
            Type::Primitive(n) => n.clone(),
            Type::Unknown => "dynamic /* unknown stack type */".to_owned(),
            Type::Named(t) | Type::ValueType(t) => self.type_name_depth(*t, depth + 1)?,
            Type::Generic { method, index } => {
                let owner = if *method {
                    context
                } else {
                    *self.owners.get(&context).unwrap_or(&context)
                };
                self.generics
                    .get(&owner)
                    .and_then(|g| g.get(&(*index as u16)))
                    .map(|n| identifier(n))
                    .unwrap_or_else(|| format!("{}{}", if *method { "M" } else { "T" }, index))
            }
            Type::MultiArray(t, 1) => format!("{}[*]", inner(t)?),
            Type::Array(t, rank) | Type::MultiArray(t, rank) => format!(
                "{}[{}]",
                inner(t)?,
                ",".repeat(rank.saturating_sub(1) as usize)
            ),
            Type::Pointer(t) => format!("{}*", inner(t)?),
            Type::ByRef(t) => format!("ref {}", inner(t)?),
            Type::Pinned(t) => format!("{} /* pinned */", inner(t)?),
            Type::GenericInstance { base, args } => format!(
                "{}<{}>",
                strip_arity(&inner(base)?),
                args.iter()
                    .map(inner)
                    .collect::<Result<Vec<_>>>()?
                    .join(", ")
            ),
            Type::Modified {
                required,
                modifier,
                inner: t,
            } => format!(
                "{} /* {}({}) */",
                inner(t)?,
                if *required { "modreq" } else { "modopt" },
                self.type_name_depth(*modifier, depth + 1)?
                    .replace("*/", "* /")
            ),
            Type::FunctionPointer(s) => format!(
                "delegate*<{}, {}>",
                s.parameters
                    .iter()
                    .map(inner)
                    .collect::<Result<Vec<_>>>()?
                    .join(", "),
                inner(&s.return_type)?
            ),
        };
        if rendered.len() > 16384 {
            return Err(Error::limit("Resolved type name exceeds 16 KiB"));
        }
        Ok(rendered)
    }
    pub fn resolve(&self, token: u32) -> String {
        self.resolve_depth(token, 0)
            .unwrap_or_else(|_| format!("unresolved_0x{token:08X}"))
    }
    fn resolve_depth(&self, token: u32, depth: usize) -> Result<String> {
        if depth > 48 {
            return Err(Error::limit("Token resolution depth exceeds 48"));
        }
        if let Some(name) = self.names.get(&token) {
            return Ok(name.clone());
        }
        let m = self.metadata();
        Ok(match token >> 24 {
            0 => self.string(row(&m.modules, token)?.name)?.to_owned(),
            1 | 2 | 26 | 27 => self.type_name_depth(token, depth + 1)?,
            4 => format!(
                "{}.{}",
                self.resolve(*self.owners.get(&token).unwrap_or(&0)),
                self.string(row(&m.fields, token)?.name)?
            ),
            6 => {
                let mr = self.method_ref(token)?;
                self.method_display(&mr)
            }
            10 => {
                let r = row(&m.member_refs, token)?;
                format!(
                    "{}.{}",
                    self.resolve_depth(coded(r.class), depth + 1)?,
                    self.string(r.name)?
                )
            }
            17 => "local/callsite signature".to_owned(),
            20 => self.string(row(&m.events, token)?.name)?.to_owned(),
            23 => self.string(row(&m.properties, token)?.name)?.to_owned(),
            32 => self.info.identity.name.clone(),
            35 => self.string(row(&m.assembly_refs, token)?.name)?.to_owned(),
            40 => self
                .string(row(&m.manifest_resources, token)?.name)?
                .to_owned(),
            43 => {
                let r = row(&m.method_specs, token)?;
                let args = signature::method_spec(self.blob(r.instantiation)?)?;
                format!(
                    "{}<{}>",
                    self.resolve_depth(coded(r.method), depth + 1)?,
                    args.iter()
                        .map(|t| self.format_type(t, 0))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            0x70 => serde_json::to_string(&self.user_string(token)?).unwrap_or_default(),
            _ => format!("metadata_0x{token:08X}"),
        })
    }
    pub fn method_ref(&self, token: u32) -> Result<MethodRef> {
        let m = self.metadata();
        let (base, generic_args) = if token >> 24 == 43 {
            let r = row(&m.method_specs, token)?;
            (
                coded(r.method),
                signature::method_spec(self.blob(r.instantiation)?)?,
            )
        } else {
            (token, vec![])
        };
        let (owner, name, sig) = match base >> 24 {
            6 => {
                let r = row(&m.method_defs, base)?;
                (
                    *self.owners.get(&base).unwrap_or(&0),
                    self.string(r.name)?.to_owned(),
                    r.signature,
                )
            }
            10 => {
                let r = row(&m.member_refs, base)?;
                (coded(r.class), self.string(r.name)?.to_owned(), r.signature)
            }
            17 => {
                let r = row(&m.stand_alone_sigs, base)?;
                (0, "calli".to_owned(), r.signature)
            }
            _ => {
                return Err(Error::metadata(
                    "Expected MethodDef, MemberRef, MethodSpec or StandAloneSig",
                ));
            }
        };
        let mut signature = signature::method(self.blob(sig)?)?;
        let type_args = if owner >> 24 == 27 {
            let r = row(&m.type_specs, owner)?;
            if let Type::GenericInstance { args, .. } =
                signature::type_spec(self.blob(r.signature)?)?
            {
                args
            } else {
                vec![]
            }
        } else {
            vec![]
        };
        signature.return_type = substitute(&signature.return_type, &type_args, &generic_args);
        signature.parameters = signature
            .parameters
            .iter()
            .map(|t| substitute(t, &type_args, &generic_args))
            .collect();
        Ok(MethodRef {
            token,
            owner,
            name,
            signature,
            generic_args,
        })
    }
    pub fn method_display(&self, m: &MethodRef) -> String {
        format!(
            "{} {}.{}{}({})",
            self.format_type(&m.signature.return_type, m.token),
            self.resolve(m.owner),
            m.name,
            if m.generic_args.is_empty() {
                String::new()
            } else {
                format!(
                    "<{}>",
                    m.generic_args
                        .iter()
                        .map(|t| self.format_type(t, m.token))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
            m.signature
                .parameters
                .iter()
                .map(|t| self.format_type(t, m.token))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
    pub fn method_row(&self, token: u32) -> Result<&clrmeta::MethodDefRow> {
        if token >> 24 != 6 {
            return Err(Error::metadata("Expected MethodDef token"));
        }
        row(&self.metadata().method_defs, token)
    }
    pub fn field_ref(&self, token: u32) -> Result<(u32, String, Type)> {
        let m = self.metadata();
        let (owner, name, sig) = match token >> 24 {
            4 => {
                let f = row(&m.fields, token)?;
                (*self.owners.get(&token).unwrap_or(&0), f.name, f.signature)
            }
            10 => {
                let f = row(&m.member_refs, token)?;
                (coded(f.class), f.name, f.signature)
            }
            _ => return Err(Error::metadata("Expected field token")),
        };
        Ok((
            owner,
            self.string(name)?.to_owned(),
            signature::field(self.blob(sig)?)?,
        ))
    }
    pub fn parameter_names(&self, token: u32, sig: &MethodSignature) -> Vec<String> {
        let mut names: Vec<_> = (0..sig.parameters.len())
            .map(|i| format!("arg{i}"))
            .collect();
        if let Ok(m) = self.method_row(token) {
            let end = self
                .metadata()
                .method_defs
                .get((token & 0xffffff) as usize)
                .map(|m| m.param_list)
                .unwrap_or(self.metadata().params.len() as u32 + 1);
            for index in m.param_list..end {
                if let Ok(p) = row(&self.metadata().params, index)
                    && p.sequence > 0
                    && let Some(n) = names.get_mut(p.sequence as usize - 1)
                    && let Ok(name) = self.string(p.name)
                    && !name.is_empty()
                {
                    *n = identifier(name);
                }
            }
        }
        names
    }
    pub fn declaration(&self, token: u32) -> Result<String> {
        let m = self.metadata();
        Ok(match token >> 24 {
            6 => {
                let r = self.method_row(token)?;
                let mr = self.method_ref(token)?;
                let sig = &mr.signature;
                let names = self.parameter_names(token, sig);
                let params = sig
                    .parameters
                    .iter()
                    .zip(names)
                    .map(|(t, n)| format!("{} {n}", self.format_type(t, token)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let gs = self
                    .generics
                    .get(&token)
                    .map(|g| {
                        format!(
                            "<{}>",
                            g.values()
                                .map(|s| identifier(s))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })
                    .unwrap_or_default();
                let n = if mr.name == ".ctor" || mr.name == ".cctor" {
                    strip_arity(&self.resolve(mr.owner))
                        .rsplit('.')
                        .next()
                        .unwrap_or("constructor")
                        .to_owned()
                } else {
                    format!(
                        "{} {}{}",
                        self.format_type(&sig.return_type, token),
                        identifier(&mr.name),
                        gs
                    )
                };
                format!(
                    "{}{}{} {}({})",
                    visibility(r.flags as u32),
                    if r.flags & 16 != 0 { " static" } else { "" },
                    if r.flags & 0x400 != 0 {
                        " abstract"
                    } else if r.flags & 0x2000 != 0 {
                        " extern"
                    } else if r.flags & 0x40 != 0 {
                        " virtual"
                    } else {
                        ""
                    },
                    n,
                    params
                )
            }
            4 => {
                let r = row(&m.fields, token)?;
                let (_, n, t) = self.field_ref(token)?;
                format!(
                    "{}{} {} {};",
                    visibility(r.flags as u32),
                    if r.flags & 16 != 0 { " static" } else { "" },
                    self.format_type(&t, token),
                    identifier(&n)
                )
            }
            23 => {
                let p = row(&m.properties, token)?;
                let s = signature::method(self.blob(p.property_type)?)?;
                let access = self
                    .semantics
                    .get(&token)
                    .into_iter()
                    .flatten()
                    .map(|(f, _)| {
                        if f & 2 != 0 {
                            "get;"
                        } else if f & 1 != 0 {
                            "set;"
                        } else {
                            ""
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                format!(
                    "{} {} {{ {} }}",
                    self.format_type(&s.return_type, token),
                    identifier(self.string(p.name)?),
                    access
                )
            }
            20 => {
                let e = row(&m.events, token)?;
                format!(
                    "event {} {};",
                    self.resolve(coded(e.event_type)),
                    identifier(self.string(e.name)?)
                )
            }
            2 => {
                let t = row(&m.type_defs, token)?;
                let n = self.nodes.iter().find(|n| n.token == token);
                let kind = n.map(|n| n.kind.as_str()).unwrap_or("class");
                let gs = self
                    .generics
                    .get(&token)
                    .map(|g| {
                        format!(
                            "<{}>",
                            g.values()
                                .map(|s| identifier(s))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })
                    .unwrap_or_default();
                let mut bases = Vec::new();
                if t.extends.row != 0 {
                    let base = self.resolve(coded(t.extends));
                    if !matches!(
                        base.as_str(),
                        "System.Object"
                            | "System.ValueType"
                            | "System.Enum"
                            | "System.MulticastDelegate"
                    ) {
                        bases.push(base);
                    }
                }
                for i in &m.interface_impls {
                    if i.class == token & 0xffffff {
                        bases.push(self.resolve(coded(i.interface)));
                    }
                }
                format!(
                    "{} {kind} {}{}{}",
                    if t.flags & 7 == 1 || t.flags & 7 == 2 {
                        "public"
                    } else {
                        "internal"
                    },
                    identifier(&strip_arity(self.string(t.type_name)?)),
                    gs,
                    if bases.is_empty() {
                        String::new()
                    } else {
                        format!(" : {}", bases.join(", "))
                    }
                )
            }
            _ => self.resolve(token),
        })
    }
    pub fn body(&self, token: u32) -> Result<Option<MethodBody>> {
        let m = self.method_row(token)?;
        if m.rva == 0 {
            return Ok(None);
        }
        if m.impl_flags & 3 != 0 {
            return Err(Error::limitation(
                "This method has a native or runtime implementation",
            ));
        }
        let mut resolved_bytes = 0usize;
        let mut body = cil::read_body(&self.image, &self.bytes, m.rva)?;
        if body.local_signature != 0 {
            if body.local_signature >> 24 != 17 {
                return Err(Error::cil("Local signature token is not StandAloneSig"));
            }
            self.validate_token(body.local_signature)?;
        }
        for i in &mut body.instructions {
            if let Operand::Token(t) = i.operand {
                self.validate_token(t)?;
                let table = t >> 24;
                let valid = match i.op.operand {
                    OperandKind::InlineString => table == 0x70,
                    OperandKind::InlineMethod => matches!(table, 6 | 10 | 43),
                    OperandKind::InlineField => matches!(table, 4 | 10),
                    OperandKind::InlineType => matches!(table, 1 | 2 | 27),
                    OperandKind::InlineSig => table == 17,
                    OperandKind::InlineTok => matches!(table, 1 | 2 | 4 | 6 | 10 | 27 | 43),
                    _ => true,
                };
                if !valid {
                    return Err(Error::cil(
                        "Operand token uses an incompatible metadata table",
                    ));
                }
                let resolved = self.resolve(t);
                resolved_bytes = resolved_bytes.saturating_add(resolved.len());
                if resolved_bytes > 4 * 1024 * 1024 {
                    return Err(Error::limit("Resolved method operands exceed 4 MiB"));
                }
                i.resolved = Some(resolved);
            }
        }
        for e in &body.exceptions {
            if let Some(t) = e.catch_type {
                if !matches!(t >> 24, 1 | 2 | 27) {
                    return Err(Error::cil("Invalid catch type token"));
                }
                self.validate_token(t)?;
            }
        }
        Ok(Some(body))
    }
    pub fn locals(&self, body: &MethodBody) -> Result<Vec<Type>> {
        if body.local_signature == 0 {
            Ok(vec![])
        } else {
            let sig = row(&self.metadata().stand_alone_sigs, body.local_signature)?;
            signature::locals(self.blob(sig.signature)?)
        }
    }
    pub fn user_string(&self, token: u32) -> Result<String> {
        let offset = token & 0xffffff;
        let data = self.metadata().user_strings.data();
        let mut r = Reader::new(
            data.get(offset as usize..)
                .ok_or_else(|| Error::metadata("User string index out of bounds"))?,
        );
        let n = r.compressed()? as usize;
        if n > 1024 * 1024 {
            return Err(Error::limit("User string exceeds 1 MiB"));
        }
        if n == 0 {
            return Ok(String::new());
        }
        let b = r.take(n)?;
        if n % 2 != 1 {
            return Err(Error::metadata("Invalid UTF16 user string length"));
        }
        Ok(String::from_utf16_lossy(
            &b[..n - 1]
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        ))
    }
    pub fn strings(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let q = query.to_lowercase();
        let mut hits = Vec::new();
        let data = self.metadata().user_strings.data();
        let mut r = Reader::new(data);
        if !data.is_empty() {
            r.u8()?;
        }
        while r.pos < data.len() && hits.len() < limit {
            let offset = r.pos;
            let n = r.compressed()? as usize;
            if n == 0 {
                continue;
            }
            r.take(n)?;
            let s = self.user_string(0x70000000 | offset as u32)?;
            if s.to_lowercase().contains(&q) {
                hits.push(SearchHit {
                    token: 0x70000000 | offset as u32,
                    kind: "string".to_owned(),
                    name: s,
                    context: "#US · user string".to_owned(),
                });
            }
        }
        Ok(hits)
    }
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Ok(vec![]);
        }
        let mut result: Vec<_> = self
            .nodes
            .iter()
            .filter(|n| {
                n.full_name.to_lowercase().contains(&query)
                    || format!("{:08x}", n.token).contains(&query)
            })
            .take(250)
            .map(|n| SearchHit {
                token: n.token,
                kind: n.kind.clone(),
                name: n.name.clone(),
                context: n.full_name.clone(),
            })
            .collect();
        result.extend(self.strings(&query, 250 - result.len())?);
        Ok(result)
    }
    pub fn resources(&self) -> Result<Vec<Value>> {
        self.metadata().manifest_resources.iter().enumerate().map(|(i,r)|Ok(json!({"token":0x28000001+i as u32,"name":self.string(r.name)?,"offset":r.offset,"flags":r.flags,"implementation":coded(r.implementation),"embedded":r.implementation.row==0}))).collect()
    }
    pub fn inspect(&self, token: u32) -> Result<Value> {
        self.validate_token(token)?;
        if token >> 24 == 0x70 {
            return Ok(json!({"token":token,"heap":"#US","value":self.user_string(token)?}));
        }
        let mut value = json!({"token":token,"token_hex":format!("0x{token:08X}"),"name":self.resolve(token),"declaring_type":self.owners.get(&token),"declaration":self.declaration(token)?,"raw":cil::hex(self.store.raw_row(&self.bytes[self.image.metadata.clone()],token)?)});
        let attrs=self.attributes.get(&token).into_iter().flatten().take(1024).map(|i|{let at=&self.metadata().custom_attributes[*i];json!({"constructor_token":coded(at.attr_type),"constructor":self.resolve(coded(at.attr_type)),"blob":self.blob(at.value).map(|b|cil::hex(&b[..b.len().min(4096)])).unwrap_or_default()})}).collect::<Vec<_>>();
        value["attributes"] = json!(attrs);
        if token >> 24 == 12 {
            let attribute = row(&self.metadata().custom_attributes, token)?;
            let constructor = coded(attribute.attr_type);
            let blob = self.blob(attribute.value)?;
            value["attribute"] = json!({
                "parent_token": coded(attribute.parent),
                "constructor_token": constructor,
                "constructor": self.resolve(constructor),
                "blob": cil::hex(&blob[..blob.len().min(4096)]),
                "blob_size": blob.len(),
                "blob_truncated": blob.len() > 4096,
            });
        }
        value["accessors"] = json!(
            self.semantics
                .get(&token)
                .into_iter()
                .flatten()
                .map(|(f, t)| json!({"semantics":f,"token":t,"name":self.resolve(*t)}))
                .collect::<Vec<_>>()
        );
        if token >> 24 == 6 {
            let m = self.method_row(token)?;
            value["rva"] = json!(format!("0x{:08X}", m.rva));
            value["flags"] = json!(m.flags);
            value["implementation_flags"] = json!(m.impl_flags);
            value["signature"] = json!(self.method_ref(token)?.signature);
            value["signature_blob"] = json!(cil::hex(self.blob(m.signature)?));
        }
        if token >> 24 == 2 {
            let t = row(&self.metadata().type_defs, token)?;
            value["flags"] = json!(t.flags);
            value["extends"] =
                json!({"token":coded(t.extends),"name":self.resolve(coded(t.extends))});
        }
        value["pinvoke"]=json!(self.metadata().impl_maps.iter().filter(|p|coded(p.member_forwarded)==token).map(|p|json!({"library":self.metadata().module_refs.get(p.import_scope.saturating_sub(1) as usize).and_then(|m|self.string(m.name).ok()),"entry_point":self.string(p.import_name).ok(),"flags":p.mapping_flags})).collect::<Vec<_>>());
        Ok(value)
    }
}
pub fn visibility(flags: u32) -> &'static str {
    match flags & 7 {
        1 => "private",
        2 => "private protected",
        3 => "internal",
        4 => "protected",
        5 => "protected internal",
        6 => "public",
        _ => "internal",
    }
}
pub fn identifier(name: &str) -> String {
    let mut s = String::new();
    for (i, c) in name.chars().enumerate() {
        if c == '_' || c.is_alphabetic() || (i > 0 && c.is_numeric()) {
            s.push(c);
        } else {
            s.push_str(&format!("_u{:04X}_", c as u32));
        }
    }
    if s.is_empty() {
        s.push('_');
    }
    if [
        "class",
        "namespace",
        "public",
        "private",
        "static",
        "void",
        "return",
        "event",
        "new",
        "base",
        "this",
        "ref",
        "out",
        "in",
        "string",
        "object",
        "int",
        "bool",
        "operator",
        "default",
        "params",
        "internal",
        "var",
        "null",
        "true",
        "false",
        "try",
        "catch",
        "finally",
        "throw",
        "switch",
        "case",
        "for",
        "while",
        "using",
        "delegate",
        "struct",
        "interface",
        "enum",
    ]
    .contains(&s.as_str())
    {
        s.insert(0, '@');
    }
    s
}
pub fn strip_arity(name: &str) -> String {
    let mut out = String::new();
    let mut skip = false;
    for c in name.chars() {
        if c == '`' {
            skip = true;
            continue;
        }
        if skip && c.is_ascii_digit() {
            continue;
        }
        skip = false;
        out.push(c);
    }
    out
}
fn substitute(t: &Type, types: &[Type], methods: &[Type]) -> Type {
    match t {
        Type::Generic { method, index } => if *method { methods } else { types }
            .get(*index as usize)
            .cloned()
            .unwrap_or_else(|| t.clone()),
        Type::Array(t, n) => Type::Array(Box::new(substitute(t, types, methods)), *n),
        Type::MultiArray(t, n) => Type::MultiArray(Box::new(substitute(t, types, methods)), *n),
        Type::ByRef(t) => Type::ByRef(Box::new(substitute(t, types, methods))),
        Type::GenericInstance { base, args } => Type::GenericInstance {
            base: base.clone(),
            args: args.iter().map(|t| substitute(t, types, methods)).collect(),
        },
        _ => t.clone(),
    }
}
