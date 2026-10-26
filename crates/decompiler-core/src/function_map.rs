//! Whole-assembly function map: every method with its call edges, callers, string
//! literals and role tags, built from decoded CIL only. Nothing executes.
use crate::{
    assembly::{Assembly, coded},
    error::{Error, Result},
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const MAX_METHODS: usize = 65_536;
const MAX_EDGES: usize = 500_000;
const STRINGS_PER_FUNCTION: usize = 8;
const STRING_CHARS: usize = 120;

#[derive(Debug, Serialize)]
pub struct Function {
    pub token: u32,
    pub name: String,
    pub owner: u32,
    pub owner_name: String,
    pub signature: String,
    /// Role tags: entry, constructor, static-constructor, accessor, event-handler,
    /// pinvoke, abstract, virtual, static, unreferenced.
    pub tags: Vec<&'static str>,
    pub size: u32,
    pub instructions: usize,
    /// Distinct call targets: MethodDef tokens of this module or external tokens.
    pub calls: Vec<u32>,
    pub called_by: Vec<u32>,
    pub strings: Vec<String>,
    pub string_count: usize,
    /// The name at this token in the assembly a cleaned copy was derived from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
/// A reference from one type to another, for the relationship diagram.
#[derive(Debug, Serialize)]
pub struct TypeLink {
    pub token: u32,
    pub name: String,
}
/// A declared type and its structural relationships to other types.
#[derive(Debug, Serialize)]
pub struct TypeNode {
    pub token: u32,
    pub name: String,
    pub full_name: String,
    pub namespace: String,
    pub kind: String,
    pub members: usize,
    /// Base class (only when it is a resolvable type).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<TypeLink>,
    pub interfaces: Vec<TypeLink>,
    /// Distinct types this type stores as fields (composition / "has-a").
    pub uses: Vec<TypeLink>,
    /// Enclosing type for a nested type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nested_in: Option<u32>,
}
#[derive(Debug, Serialize)]
pub struct FunctionMap {
    pub functions: Vec<Function>,
    pub types: Vec<TypeNode>,
    /// Display names of external call targets (MemberRefs), keyed by decimal
    /// token: JS object keys must be strings.
    pub externals: BTreeMap<String, String>,
    pub edges: usize,
    pub truncated: bool,
    pub entry_point: u32,
}

fn clip(text: &str) -> String {
    let mut out: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(STRING_CHARS)
        .collect();
    if text.chars().count() > STRING_CHARS {
        out.push('…');
    }
    out
}
/// Original names are usually invisible characters; show them as escapes.
fn visible(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_control()
                || crate::inspection::hidden_character(c)
                || !c.is_ascii() && !c.is_alphanumeric()
            {
                format!("\\u{:04X}", c as u32)
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// `original` is the assembly a deobfuscated copy was derived from; tokens are
/// stable between the two, so its names map one-to-one.
pub fn build(a: &Assembly, original: Option<&Assembly>) -> Result<FunctionMap> {
    let methods = a.metadata().method_defs.len();
    if methods > MAX_METHODS {
        return Err(Error::limit("Function maps support at most 65,536 methods"));
    }
    let _scope = a.short_names("");
    let names: BTreeMap<u32, &crate::assembly::Node> = a
        .nodes
        .iter()
        .filter(|n| n.token >> 24 == 6)
        .map(|n| (n.token, n))
        .collect();
    let original_names: BTreeMap<u32, &str> = original
        .map(|o| {
            o.nodes
                .iter()
                .filter(|n| n.token >> 24 == 6)
                .map(|n| (n.token, n.name.as_str()))
                .collect()
        })
        .unwrap_or_default();
    let entry_point = a.image.entry_point;
    let mut externals = BTreeMap::new();
    let mut edges = 0usize;
    let mut truncated = false;
    let mut functions = Vec::with_capacity(methods);
    for (index, row) in a.metadata().method_defs.iter().enumerate() {
        let token = 0x0600_0001 + index as u32;
        let owner = a.owners.get(&token).copied().unwrap_or(0);
        let node = names.get(&token);
        let name = node.map_or_else(|| a.resolve(token), |n| n.name.clone());
        let method = a.method_ref(token).ok();
        let signature = method.as_ref().map_or_else(String::new, |m| {
            format!(
                "{} {}({})",
                a.format_type(&m.signature.return_type, token),
                name,
                a.parameter_names(token, &m.signature)
                    .iter()
                    .zip(&m.signature.parameters)
                    .map(|(p, t)| format!("{} {p}", a.format_type(t, token)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });
        let mut tags = Vec::new();
        if token == entry_point {
            tags.push("entry");
        }
        match name.as_str() {
            ".ctor" => tags.push("constructor"),
            ".cctor" => tags.push("static-constructor"),
            _ => {}
        }
        if node.is_some_and(|n| n.accessor_of.is_some()) {
            tags.push("accessor");
        }
        if let Some(m) = &method
            && m.signature.return_type.is_void()
            && m.signature.parameters.len() == 2
            && a.format_type(&m.signature.parameters[0], token) == "object"
            && a.format_type(&m.signature.parameters[1], token)
                .ends_with("EventArgs")
        {
            tags.push("event-handler");
        }
        if row.flags & 0x2000 != 0 {
            tags.push("pinvoke");
        }
        if row.flags & 0x400 != 0 {
            tags.push("abstract");
        } else if row.flags & 0x40 != 0 {
            tags.push("virtual");
        }
        if row.flags & 0x10 != 0 {
            tags.push("static");
        }
        let mut function = Function {
            token,
            name,
            owner,
            owner_name: a.resolve(owner),
            signature,
            tags,
            size: 0,
            instructions: 0,
            calls: vec![],
            called_by: vec![],
            strings: vec![],
            string_count: 0,
            original_name: original_names
                .get(&token)
                .filter(|o| **o != function_name(node))
                .map(|o| visible(o)),
            error: None,
        };
        match a.body(token) {
            Ok(Some(body)) => {
                function.size = body.code_size;
                function.instructions = body.instructions.len();
                let mut calls = BTreeSet::new();
                for i in &body.instructions {
                    match i.opcode {
                        0x28 | 0x6f | 0x73 | 0x27 | 0xfe06 | 0xfe07 => {
                            let Some(mut target) = i.token() else {
                                continue;
                            };
                            // A generic instantiation calls its underlying method.
                            if target >> 24 == 0x2b
                                && let Some(spec) = a
                                    .metadata()
                                    .method_specs
                                    .get((target & 0xff_ffff) as usize - 1)
                            {
                                target = coded(spec.method);
                            }
                            if target >> 24 == 0x0a {
                                externals
                                    .entry(target.to_string())
                                    .or_insert_with(|| a.resolve(target));
                            }
                            if matches!(target >> 24, 0x06 | 0x0a) {
                                calls.insert(target);
                            }
                        }
                        0x72 => {
                            function.string_count += 1;
                            if function.strings.len() < STRINGS_PER_FUNCTION
                                && let Some(t) = i.token()
                                && let Ok(text) = a.user_string(t)
                            {
                                function.strings.push(clip(&text));
                            }
                        }
                        _ => {}
                    }
                }
                if edges + calls.len() > MAX_EDGES {
                    truncated = true;
                } else {
                    edges += calls.len();
                    function.calls = calls.into_iter().collect();
                }
            }
            Ok(None) => {}
            Err(e) => function.error = Some(e.detail.chars().take(200).collect()),
        }
        functions.push(function);
    }
    // Invert local edges; methods nobody calls are tagged when nothing else
    // (virtual dispatch, events, the runtime) can reach them by name.
    let mut callers: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for f in &functions {
        for target in f.calls.iter().filter(|t| *t >> 24 == 6) {
            callers.entry(*target).or_default().insert(f.token);
        }
    }
    for f in &mut functions {
        f.called_by = callers
            .remove(&f.token)
            .map(|c| c.into_iter().collect())
            .unwrap_or_default();
        let reachable = f.tags.iter().any(|t| {
            matches!(
                *t,
                "entry"
                    | "constructor"
                    | "static-constructor"
                    | "accessor"
                    | "event-handler"
                    | "virtual"
                    | "abstract"
                    | "pinvoke"
            )
        });
        if f.called_by.is_empty() && !reachable {
            f.tags.push("unreferenced");
        }
    }
    let types = type_nodes(a, &names);
    Ok(FunctionMap {
        functions,
        types,
        externals,
        edges,
        truncated,
        entry_point,
    })
}
fn function_name<'a>(node: Option<&&'a crate::assembly::Node>) -> &'a str {
    node.map_or("", |n| n.name.as_str())
}
/// The short display name of a type token, or its resolved name for refs/specs.
fn type_link(a: &Assembly, token: u32) -> Option<TypeLink> {
    if token & 0x00ff_ffff == 0 {
        return None;
    }
    let full = a.resolve(token);
    let name = full
        .split('<')
        .next()
        .unwrap_or(&full)
        .rsplit('.')
        .next()
        .unwrap_or(&full);
    let name = crate::assembly::strip_arity(name);
    (!name.is_empty()).then_some(TypeLink { token, name })
}
/// One node per TypeDef with its base, interfaces, field-type uses and member count.
fn type_nodes(a: &Assembly, methods: &BTreeMap<u32, &crate::assembly::Node>) -> Vec<TypeNode> {
    let m = a.metadata();
    // Members per owning type (methods already indexed, plus fields).
    let mut members: BTreeMap<u32, usize> = BTreeMap::new();
    for owner in methods.values().filter_map(|n| a.owners.get(&n.token)) {
        *members.entry(*owner).or_default() += 1;
    }
    for index in 0..m.fields.len() {
        if let Some(owner) = a.owners.get(&(0x0400_0001 + index as u32)) {
            *members.entry(*owner).or_default() += 1;
        }
    }
    // Interfaces implemented per type, from the InterfaceImpl table.
    let mut interfaces: BTreeMap<u32, Vec<TypeLink>> = BTreeMap::new();
    for row in &m.interface_impls {
        let class = 0x0200_0000 | row.class;
        if let Some(link) = type_link(a, coded(row.interface)) {
            interfaces.entry(class).or_default().push(link);
        }
    }
    let mut nodes = Vec::new();
    for (index, row) in m.type_defs.iter().enumerate() {
        let token = 0x0200_0001 + index as u32;
        let Some(node) = a.nodes.iter().find(|n| n.token == token) else {
            continue;
        };
        if node.name == "<Module>" {
            continue;
        }
        let base = type_link(a, coded(row.extends)).filter(|b| {
            !matches!(
                a.resolve_full(b.token).as_str(),
                "System.Object"
                    | "System.ValueType"
                    | "System.Enum"
                    | "System.Delegate"
                    | "System.MulticastDelegate"
            )
        });
        // Field types that are themselves types (composition edges), de-duplicated.
        let mut uses: Vec<TypeLink> = Vec::new();
        let mut seen = BTreeSet::new();
        for field_index in 0..m.fields.len() {
            let field_token = 0x0400_0001 + field_index as u32;
            if a.owners.get(&field_token) != Some(&token) {
                continue;
            }
            if let Ok((_, _, ty)) = a.field_ref(field_token)
                && let Some(t) = field_type_token(&ty)
                && t != token
                && seen.insert(t)
                && let Some(link) = type_link(a, t)
            {
                uses.push(link);
                if uses.len() >= 32 {
                    break;
                }
            }
        }
        nodes.push(TypeNode {
            token,
            name: crate::assembly::strip_arity(&node.name),
            full_name: node.full_name.clone(),
            namespace: node.namespace.clone(),
            kind: node.kind.clone(),
            members: members.get(&token).copied().unwrap_or(0),
            base,
            interfaces: interfaces.remove(&token).unwrap_or_default(),
            uses,
            nested_in: a.owners.get(&token).copied().filter(|p| p >> 24 == 2),
        });
    }
    nodes
}
/// The underlying named/value type of a field signature, if it is one.
fn field_type_token(ty: &crate::signature::Type) -> Option<u32> {
    use crate::signature::Type;
    match ty {
        Type::Named(t) | Type::ValueType(t) => Some(*t),
        Type::Array(inner, _) | Type::MultiArray(inner, _) => field_type_token(inner),
        Type::GenericInstance { base, .. } => field_type_token(base),
        _ => None,
    }
}
