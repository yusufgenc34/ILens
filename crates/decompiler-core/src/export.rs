//! Bounded source-export declarations. No filesystem or managed execution.
use crate::{
    assembly::{Assembly, coded, identifier, strip_arity, visibility},
    decompile,
    error::{Error, Result},
    reader::{Reader, slice, u32_at},
    signature,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct TypeDeclaration {
    pub token: u32,
    pub parent: u32,
    pub name: String,
    pub namespace: String,
    pub kind: String,
    pub declaration: String,
    pub members: Vec<u32>,
    pub diagnostics: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct MemberSource {
    pub token: u32,
    pub name: String,
    pub kind: String,
    pub declaration: String,
    pub body: Option<String>,
    pub il: String,
    pub quality: String,
    pub diagnostics: Vec<String>,
    pub accessors: Vec<(String, u32)>,
    pub metadata: serde_json::Value,
}

fn constraints(a: &Assembly, token: u32) -> Result<String> {
    if !a.generics.contains_key(&token) {
        return Ok(String::new());
    }
    let mut result = String::new();
    for (index, parameter) in a.metadata().generic_params.iter().enumerate() {
        if coded(parameter.owner) != token {
            continue;
        }
        let mut terms = Vec::new();
        if parameter.flags & 8 != 0 {
            terms.push("struct".to_owned());
        } else if parameter.flags & 4 != 0 {
            terms.push("class".to_owned());
        }
        for constraint in &a.metadata().generic_param_constraints {
            if constraint.owner == index as u32 + 1 {
                let name = a.resolve(coded(constraint.constraint));
                if name != "System.ValueType" {
                    terms.push(name);
                }
            }
        }
        if parameter.flags & 16 != 0 && parameter.flags & 8 == 0 {
            terms.push("new()".to_owned());
        }
        if !terms.is_empty() {
            result.push_str(&format!(
                " where {} : {}",
                identifier(a.string(parameter.name)?),
                terms.join(", ")
            ));
        }
    }
    Ok(result)
}
fn attribute_notes(a: &Assembly, token: u32) -> Vec<String> {
    let mut notes = Vec::new();
    if a.attributes.get(&token).is_some_and(|v| !v.is_empty()) {
        notes.push("Custom attributes are recorded in member metadata; source attribute emission is incomplete.".to_owned());
    }
    notes
}
pub fn plan(a: &Assembly, root: Option<u32>) -> Result<Vec<TypeDeclaration>> {
    if let Some(token) = root {
        a.validate_token(token)?;
        if token >> 24 != 2 {
            return Err(Error::metadata("Choose a type for type export"));
        }
    }
    let within = |mut token: u32| {
        for _ in 0..49 {
            if Some(token) == root || root.is_none() {
                return true;
            }
            let Some(parent) = a.owners.get(&token) else {
                return false;
            };
            token = *parent;
        }
        false
    };
    let mut children: std::collections::HashMap<u32, Vec<&crate::assembly::Node>> =
        std::collections::HashMap::new();
    for node in &a.nodes {
        children.entry(node.parent).or_default().push(node);
    }
    a.nodes.iter().filter(|n| n.token >> 24 == 2 && within(n.token)).map(|n| {
        let row = &a.metadata().type_defs[(n.token & 0xffffff) as usize - 1];
        let mut diagnostics = attribute_notes(a, n.token);
        if row.flags & 0x18 != 0 || row.flags & 0x2000 != 0 { diagnostics.push("Explicit/sequential layout or interop configuration requires manual reconstruction.".to_owned()); }
        let access = match row.flags & 7 { 1|2 => "public", 3 => "private", 4 => "protected", 6 => "private protected", 7 => "protected internal", _ => "internal" };
        let name = identifier(&strip_arity(&n.name));
        let generics = a.generics.get(&n.token).map(|g| format!("<{}>", g.values().map(|n| identifier(n)).collect::<Vec<_>>().join(", "))).unwrap_or_default();
        let modifier = if n.kind == "class" { match row.flags & 0x180 { 0x180 => " static", 0x80 => " abstract", 0x100 => " sealed", _ => "" } } else { "" };
        let mut bases = Vec::new();
        if row.extends.row != 0 {
            let base = a.resolve(coded(row.extends));
            if !matches!(base.as_str(), "System.Object"|"System.ValueType"|"System.Enum"|"System.MulticastDelegate") { bases.push(base); }
        }
        for i in &a.metadata().interface_impls { if i.class == n.token & 0xffffff { bases.push(a.resolve(coded(i.interface))); } }
        if n.kind == "enum" && let Some(field) = children.get(&n.token).into_iter().flatten().find(|f| f.name == "value__") { bases = vec![a.format_type(&a.field_ref(field.token)?.2, n.token)]; }
        let mut declaration = format!("{access}{modifier} {} {name}{generics}{}{}", n.kind, if bases.is_empty() { String::new() } else { format!(" : {}", bases.join(", ")) }, constraints(a, n.token)?);
        if n.kind == "delegate" {
            if let Some(invoke) = children.get(&n.token).into_iter().flatten().find(|m| m.name == "Invoke") {
                let sig = a.method_ref(invoke.token)?.signature;
                let params = sig.parameters.iter().zip(a.parameter_names(invoke.token, &sig)).map(|(t,n)| format!("{} {n}", a.format_type(t, invoke.token))).collect::<Vec<_>>().join(", ");
                declaration = format!("{access} delegate {} {name}{generics}({params}){};", a.format_type(&sig.return_type, invoke.token), constraints(a,n.token)?);
            } else { diagnostics.push("Delegate Invoke signature is missing.".to_owned()); }
        }
        let mut symbol_names = std::collections::HashMap::new();
        for child in children.get(&n.token).into_iter().flatten() {
            let mapped = identifier(&child.name);
            if let Some(previous) = symbol_names.insert(mapped,child.name.as_str()) && previous != child.name {diagnostics.push("Distinct metadata names collide after C# identifier mapping; manual renaming is required.".to_owned());}
        }
        let members: Vec<u32> = children.get(&n.token).into_iter().flatten().filter(|m| m.token >> 24 != 2).map(|m| m.token).collect();
        if a.generics.contains_key(&n.token) { diagnostics.push("Generic declarations require review of nested arity, variance and reference binding.".to_owned()); }
        if n.namespace.split('.').any(|part| !part.is_empty() && !part.chars().enumerate().all(|(i,c)| c == '_' || c.is_ascii_alphabetic() || i > 0 && c.is_ascii_digit())) { diagnostics.push("Namespace names require manual C# identifier mapping.".to_owned()); }
        if n.name == "<Module>" { declaration = "// CLI module metadata; global members are preserved in IL.".to_owned(); if !members.is_empty() { diagnostics.push("Global module members cannot be emitted inside a normal C# type.".to_owned()); } }
        if n.name != "<Module>" && (n.name.contains('<') || n.name.chars().any(|c| !(c.is_ascii_alphanumeric() || "_`".contains(c)))) { diagnostics.push("Metadata names require synthetic C# identifiers; references may need manual adjustment.".to_owned()); }
        Ok(TypeDeclaration {token:n.token, parent:if Some(n.token)==root {0}else{n.parent}, name:n.name.clone(), namespace:n.namespace.clone(), kind:if n.name == "<Module>" {"module".to_owned()} else {n.kind.clone()}, declaration, members, diagnostics})
    }).collect()
}
fn constant(a: &Assembly, token: u32) -> Result<Option<String>> {
    let Some(c) = a
        .metadata()
        .constants
        .iter()
        .find(|c| coded(c.parent) == token)
    else {
        return Ok(None);
    };
    let bytes = a.blob(c.value)?;
    let mut r = Reader::new(bytes);
    let value = match c.constant_type {
        2 => {
            if r.u8()? == 0 {
                "false".to_owned()
            } else {
                "true".to_owned()
            }
        }
        3 => format!("'\\u{:04X}'", r.u16()?),
        4 => (r.u8()? as i8).to_string(),
        5 => r.u8()?.to_string(),
        6 => (r.u16()? as i16).to_string(),
        7 => r.u16()?.to_string(),
        8 => (r.u32()? as i32).to_string(),
        9 => format!("{}U", r.u32()?),
        10 => format!("{}L", r.u64()? as i64),
        11 => format!("{}UL", r.u64()?),
        14 => {
            if !bytes.len().is_multiple_of(2) {
                return Err(Error::metadata("Invalid string constant"));
            }
            return Ok(Some(
                serde_json::to_string(&String::from_utf16_lossy(
                    &bytes
                        .chunks_exact(2)
                        .map(|b| u16::from_le_bytes([b[0], b[1]]))
                        .collect::<Vec<_>>(),
                ))
                .map_err(|e| Error::metadata(e.to_string()))?,
            ));
        }
        18 => {
            if r.u32()? != 0 {
                return Err(Error::metadata("Invalid null constant"));
            }
            "null".to_owned()
        }
        _ => {
            return Err(Error::limitation(
                "This constant type is preserved in metadata only",
            ));
        }
    };
    if r.pos != bytes.len() {
        return Err(Error::metadata("Constant has unexpected trailing data"));
    }
    Ok(Some(value))
}
pub fn member(a: &Assembly, token: u32) -> Result<MemberSource> {
    a.validate_token(token)?;
    let node = a
        .nodes
        .iter()
        .find(|n| n.token == token)
        .ok_or_else(|| Error::metadata("Not an exportable member"))?;
    let mut output = MemberSource {
        token,
        name: node.name.clone(),
        kind: node.kind.clone(),
        declaration: a.declaration(token)?,
        body: None,
        il: String::new(),
        quality: "declaration".to_owned(),
        diagnostics: attribute_notes(a, token),
        accessors: vec![],
        metadata: a.inspect(token)?,
    };
    if !matches!(node.name.as_str(), ".ctor" | ".cctor")
        && identifier(&node.name) != node.name
        && !identifier(&node.name).starts_with('@')
    {
        output.diagnostics.push(
            "A synthetic C# identifier replaces this metadata name; review reference binding."
                .to_owned(),
        );
    }
    match token >> 24 {
        6 => {
            let result = decompile::decompile(a, token)?;
            if let Some(declaration) = result.source_declaration {
                output.declaration = declaration;
            }
            output.body = result.source_body;
            output.il = result.il;
            output.quality = result.quality;
            output
                .diagnostics
                .extend(result.diagnostics.into_iter().map(|e| e.detail));
            output.declaration.push_str(&constraints(a, token)?);
            let row = a.method_row(token)?;
            if row.flags & 0x40 != 0 && row.flags & 0x100 == 0 {
                output.declaration = output.declaration.replacen(
                    " virtual ",
                    if row.flags & 0x20 != 0 {
                        " sealed override "
                    } else {
                        " override "
                    },
                    1,
                );
            }
            if row.flags & 0x2000 != 0
                || row.impl_flags != 0
                || a.metadata()
                    .method_impls
                    .iter()
                    .any(|i| coded(i.method_body) == token)
            {
                output.diagnostics.push("Interop, implementation flags, or explicit method overrides require manual review.".to_owned());
            }
        }
        4 => {
            let row = &a.metadata().fields[(token & 0xffffff) as usize - 1];
            let is_enum = a
                .nodes
                .iter()
                .any(|n| n.token == node.parent && n.kind == "enum");
            if is_enum && node.name == "value__" {
                output.quality = "implicit".to_owned();
                output.declaration.clear();
            } else if let Some(value) = constant(a, token)? {
                output.declaration = if is_enum {
                    format!("{} = {value},", identifier(&node.name))
                } else {
                    let ty = a.field_ref(token)?.2;
                    format!(
                        "{} const {} {} = {value};",
                        visibility(row.flags as u32),
                        a.format_type(&ty, token),
                        identifier(&node.name)
                    )
                };
            } else if row.flags & 0x40 != 0 {
                output
                    .diagnostics
                    .push("Literal field has no supported constant.".to_owned());
            } else if row.flags & 0x20 != 0 {
                output.declaration = output.declaration.replacen(" ", " readonly ", 1);
            }
            if row.flags & 0x3100 != 0 {
                output
                    .diagnostics
                    .push("Field RVA or marshalling data is not emitted as source.".to_owned());
            }
        }
        23 | 20 => {
            let semantics = a.semantics.get(&token).cloned().unwrap_or_default();
            let first = semantics
                .first()
                .ok_or_else(|| Error::metadata("Property/event has no accessor"))?;
            let flags = a.method_row(first.1)?.flags;
            if flags & 0x440 != 0
                || semantics.iter().any(|(_, t)| {
                    a.method_row(*t)
                        .is_ok_and(|row| row.flags & 0x457 != flags & 0x457)
                        || a.owners.get(t) != a.owners.get(&token)
                })
            {
                output.diagnostics.push("Accessor visibility, virtual/abstract binding or ownership requires manual reconstruction.".to_owned());
            }
            let prefix = format!(
                "{}{}",
                visibility(flags as u32),
                if flags & 16 != 0 { " static" } else { "" }
            );
            if token >> 24 == 23 {
                let row = &a.metadata().properties[(token & 0xffffff) as usize - 1];
                let sig = signature::method(a.blob(row.property_type)?)?;
                if !sig.parameters.is_empty() {
                    output.diagnostics.push(
                        "Indexer parameter/accessor name reconstruction is not yet complete."
                            .to_owned(),
                    );
                }
                output.declaration = format!(
                    "{prefix} {} {}",
                    a.format_type(&sig.return_type, token),
                    identifier(&node.name)
                );
            } else {
                let row = &a.metadata().events[(token & 0xffffff) as usize - 1];
                output.declaration = format!(
                    "{prefix} event {} {}",
                    a.resolve(coded(row.event_type)),
                    identifier(&node.name)
                );
            }
            output.accessors = semantics
                .into_iter()
                .map(|(f, t)| {
                    (
                        (match f {
                            1 => "set",
                            2 => "get",
                            8 => "add",
                            16 => "remove",
                            _ => "unsupported",
                        })
                        .to_owned(),
                        t,
                    )
                })
                .collect();
            if output.accessors.iter().any(|(k, _)| k == "unsupported") {
                output
                    .diagnostics
                    .push("Nonstandard accessor semantics remain in IL.".to_owned());
            }
        }
        _ => return Err(Error::metadata("Unsupported source member token")),
    }
    if output.declaration.len() + output.body.as_ref().map_or(0, String::len) + output.il.len()
        > 4 * 1024 * 1024
    {
        return Err(Error::limit("Exported member exceeds 4 MiB"));
    }
    Ok(output)
}
pub fn resource(a: &Assembly, token: u32) -> Result<Vec<u8>> {
    a.validate_token(token)?;
    if token >> 24 != 40 {
        return Err(Error::metadata("Expected ManifestResource token"));
    }
    let row = &a.metadata().manifest_resources[(token & 0xffffff) as usize - 1];
    if row.implementation.row != 0 {
        return Err(Error::limitation(
            "External resource bytes are unavailable; no files are fetched",
        ));
    }
    let range = a.image.range(
        &a.bytes,
        a.image.resources_rva,
        a.image.resources_size as usize,
    )?;
    let data = &a.bytes[range];
    let offset = row.offset as usize;
    let size = u32_at(data, offset)? as usize;
    if size > 4 * 1024 * 1024 {
        return Err(Error::limit(
            "Embedded resource exceeds the 4 MiB export entry limit",
        ));
    }
    Ok(slice(
        data,
        offset
            .checked_add(4)
            .ok_or_else(|| Error::metadata("Resource offset overflow"))?,
        size,
    )?
    .to_vec())
}
