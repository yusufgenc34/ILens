//! An in-memory resolver; assembly identity is exact and no network resolution occurs.
use crate::{
    assembly::{Assembly, AssemblyInfo},
    decompile::{self, MethodAnalysis},
    error::{Error, Result},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
#[derive(Serialize)]
pub struct Loaded {
    pub id: u32,
    pub info: AssemblyInfo,
}
pub struct Session {
    assemblies: BTreeMap<u32, Assembly>,
    next_id: u32,
    cache: VecDeque<((u32, u32), MethodAnalysis, usize)>,
    cache_bytes: usize,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            assemblies: BTreeMap::new(),
            next_id: 1,
            cache: VecDeque::new(),
            cache_bytes: 0,
        }
    }
}
impl Session {
    pub fn load(&mut self, bytes: Vec<u8>) -> Result<Loaded> {
        if self.assemblies.len() >= 16 {
            return Err(Error::limit("A workspace supports at most 16 assemblies"));
        }
        let total = self
            .assemblies
            .values()
            .map(|a| a.bytes.len())
            .sum::<usize>();
        if total + bytes.len() > 128 * 1024 * 1024 {
            return Err(Error::limit("Workspace assembly buffers exceed 128 MiB"));
        }
        let a = Assembly::load(bytes)?;
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| Error::limit("Assembly ID space exhausted"))?;
        let info = a.info.clone();
        self.assemblies.insert(id, a);
        Ok(Loaded { id, info })
    }
    pub fn assembly(&self, id: u32) -> Result<&Assembly> {
        self.assemblies
            .get(&id)
            .ok_or_else(|| Error::metadata("Assembly is closed or unknown"))
    }
    pub fn decompile(&mut self, id: u32, token: u32) -> Result<MethodAnalysis> {
        if let Some(i) = self
            .cache
            .iter()
            .position(|(key, _, _)| *key == (id, token))
        {
            let entry = self
                .cache
                .remove(i)
                .ok_or_else(|| Error::metadata("Cache entry disappeared"))?;
            let result = entry.1.clone();
            self.cache.push_back(entry);
            return Ok(result);
        }
        let result = decompile::decompile(self.assembly(id)?, token)?;
        // Serialized size is a conservative accounting base with 4x structural overhead allowance.
        let size = serde_json::to_vec(&result)
            .map_err(|e| Error::metadata(e.to_string()))?
            .len()
            .saturating_mul(4);
        while !self.cache.is_empty()
            && (self.cache.len() >= 32 || self.cache_bytes + size > 32 * 1024 * 1024)
        {
            if let Some((_, _, n)) = self.cache.pop_front() {
                self.cache_bytes = self.cache_bytes.saturating_sub(n);
            }
        }
        if size <= 32 * 1024 * 1024 {
            self.cache.push_back(((id, token), result.clone(), size));
            self.cache_bytes += size;
        }
        Ok(result)
    }
    pub fn references(&self, id: u32) -> Result<Value> {
        let a = self.assembly(id)?;
        Ok(json!(a.references.iter().map(|r|json!({"token":r.token,"identity":r.identity,"resolved_id":self.assemblies.iter().find(|(_,a)|r.identity.matches(&a.info.identity)).map(|(id,_)|id),"status":if self.assemblies.values().any(|a|r.identity.matches(&a.info.identity)){"resolved"}else{"unresolved"}})).collect::<Vec<_>>()))
    }
    pub fn xrefs_page(&self, id: u32, token: u32, start: usize, count: usize) -> Result<Value> {
        let a = self.assembly(id)?;
        a.validate_token(token)?;
        let methods = &a.metadata().method_defs;
        let end = start.saturating_add(count.min(64)).min(methods.len());
        let mut hits = Vec::new();
        let mut skipped = Vec::new();
        for i in start.min(methods.len())..end {
            let t = 0x06000001 + i as u32;
            match a.body(t) {
                Ok(Some(body)) => {
                    for ins in body.instructions {
                        if ins.token() == Some(token) {
                            hits.push(json!({"token":t,"name":a.resolve(t),"offset":ins.offset}));
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    if skipped.len() < 10 {
                        skipped.push(json!({"token":t,"error":e}));
                    }
                }
            }
        }
        Ok(
            json!({"hits":hits,"skipped":skipped,"next":end,"total":methods.len(),"done":end>=methods.len()}),
        )
    }
    pub fn overview(&self, id: u32) -> Result<Value> {
        let a = self.assembly(id)?;
        Ok(
            json!({"info":a.info,"pe":a.image,"streams":a.store.streams,"tables":a.store.parsed.tables_header.tables().map(|(table,count)|json!({"name":format!("{table:?}"),"id":table as u8,"rows":count})).collect::<Vec<_>>(),"references":self.references(id)?,"resources":a.resources()?}),
        )
    }
    pub fn type_view(&self, id: u32, token: u32) -> Result<Value> {
        let a = self.assembly(id)?;
        a.validate_token(token)?;
        let mut text = a.declaration(token)?;
        if token >> 24 == 2 {
            text.push_str("\n{\n");
            for n in a.nodes.iter().filter(|n| n.parent == token).take(2000) {
                text.push_str("    ");
                match a.declaration(n.token) {
                    Ok(s) => text.push_str(&s),
                    Err(e) => text.push_str(&format!("// {}", e.message)),
                }
                if n.token >> 24 == 6 {
                    text.push(';');
                }
                text.push('\n');
                if text.len() > 1024 * 1024 {
                    text.push_str("    // Type preview truncated at 1 MiB. Select individual members in the explorer.\n");
                    break;
                }
            }
            text.push_str("}\n");
        }
        Ok(json!({"declaration":text,"metadata":self.metadata(id,token)?}))
    }
    /// Resolve an external declaration only when a loaded assembly matches the full identity.
    /// Ambiguous overloads and forwarded types remain unresolved; name-only guessing is forbidden.
    pub fn resolve_definition(&self, id: u32, token: u32) -> Option<(u32, u32)> {
        self.resolve_definition_depth(id, token, 0)
    }
    fn resolve_definition_depth(&self, id: u32, token: u32, depth: usize) -> Option<(u32, u32)> {
        use crate::assembly::coded;
        if depth > 48 {
            return None;
        }
        let source = self.assemblies.get(&id)?;
        let rid = (token & 0xffffff).checked_sub(1)? as usize;
        match token >> 24 {
            2 | 4 | 6 => {
                source.validate_token(token).ok()?;
                Some((id, token))
            }
            43 => self.resolve_definition_depth(
                id,
                coded(source.metadata().method_specs.get(rid)?.method),
                depth + 1,
            ),
            27 => {
                let row = source.metadata().type_specs.get(rid)?;
                let ty = crate::signature::type_spec(source.blob(row.signature).ok()?).ok()?;
                let base = match ty {
                    crate::signature::Type::Named(t) => t,
                    crate::signature::Type::GenericInstance { base, .. } => {
                        if let crate::signature::Type::Named(t) = *base {
                            t
                        } else {
                            return None;
                        }
                    }
                    _ => return None,
                };
                self.resolve_definition_depth(id, base, depth + 1)
            }
            1 => {
                let mut scope = coded(source.metadata().type_refs.get(rid)?.resolution_scope);
                let mut n = 0;
                while scope >> 24 == 1 {
                    n += 1;
                    if n > 48 {
                        return None;
                    }
                    scope = coded(
                        source
                            .metadata()
                            .type_refs
                            .get((scope & 0xffffff).checked_sub(1)? as usize)?
                            .resolution_scope,
                    );
                }
                let target_id = if scope >> 24 == 35 {
                    let identity = &source
                        .references
                        .iter()
                        .find(|r| r.token == scope)?
                        .identity;
                    self.assemblies
                        .iter()
                        .find(|(_, a)| identity.matches(&a.info.identity))
                        .map(|(i, _)| *i)?
                } else if scope >> 24 == 0 {
                    id
                } else {
                    return None;
                };
                let target = self.assemblies.get(&target_id)?;
                let name = source.resolve(token);
                let mut matches = target
                    .nodes
                    .iter()
                    .filter(|n| n.token >> 24 == 2 && n.full_name == name);
                let found = matches.next()?.token;
                if matches.next().is_some() {
                    None
                } else {
                    Some((target_id, found))
                }
            }
            10 => {
                let member = source.metadata().member_refs.get(rid)?;
                let (target_id, owner) =
                    self.resolve_definition_depth(id, coded(member.class), depth + 1)?;
                let target = self.assemblies.get(&target_id)?;
                let name = source.string(member.name).ok()?;
                let field = source.blob(member.signature).ok()?.first() == Some(&6);
                let candidates: Vec<_> = target
                    .nodes
                    .iter()
                    .filter(|n| {
                        n.parent == owner
                            && n.name == name
                            && n.token >> 24 == if field { 4 } else { 6 }
                    })
                    .collect();
                let fingerprint = |a: &Assembly, t: u32| -> Option<String> {
                    if field {
                        let (_, _, ty) = a.field_ref(t).ok()?;
                        Some(a.format_type(&ty, 0))
                    } else {
                        let m = a.method_ref(t).ok()?;
                        Some(format!(
                            "{}:{}:{}:{}({})",
                            m.signature.has_this,
                            m.signature.convention,
                            m.signature.generic_count,
                            a.format_type(&m.signature.return_type, 0),
                            m.signature
                                .parameters
                                .iter()
                                .map(|t| a.format_type(t, 0))
                                .collect::<Vec<_>>()
                                .join(",")
                        ))
                    }
                };
                let expected = fingerprint(source, token)?;
                let mut found = candidates
                    .into_iter()
                    .filter(|n| fingerprint(target, n.token).as_deref() == Some(&expected));
                let token = found.next()?.token;
                if found.next().is_some() {
                    None
                } else {
                    Some((target_id, token))
                }
            }
            _ => None,
        }
    }
    pub fn metadata(&self, id: u32, token: u32) -> Result<Value> {
        let mut metadata = self.assembly(id)?.inspect(token)?;
        metadata["resolved_definition"] = json!(
            self.resolve_definition(id, token)
                .map(|(assembly, token)| json!({"assembly": assembly, "token": token}))
        );
        Ok(metadata)
    }
    pub fn close(&mut self, id: u32) {
        self.assemblies.remove(&id);
        self.cache.retain(|((a, _), _, _)| *a != id);
        self.cache_bytes = self.cache.iter().map(|(_, _, n)| n).sum();
    }
    pub fn dispose(&mut self) {
        self.assemblies.clear();
        self.cache.clear();
        self.cache_bytes = 0;
    }
}
