//! An in-memory resolver; assembly identity is exact and no network resolution occurs.
use crate::{
    assembly::{Assembly, AssemblyInfo, coded},
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
    edits: BTreeMap<(u32, u32), crate::edit::ValidatedEdit>,
    pub revision: u32,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            assemblies: BTreeMap::new(),
            next_id: 1,
            cache: VecDeque::new(),
            cache_bytes: 0,
            edits: BTreeMap::new(),
            revision: 0,
        }
    }
}
impl Session {
    fn check_input_budget(&self, bytes: &[u8]) -> Result<()> {
        // Check retained input before allocating another set of parsed metadata.
        // Reopening identical bytes is still permitted when the workspace is full.
        if self.assemblies.values().any(|a| a.bytes == bytes) {
            return Ok(());
        }
        if self.assemblies.len() >= 64 {
            return Err(Error::limit("A workspace supports at most 64 assemblies"));
        }
        let total: usize = self.assemblies.values().map(|a| a.bytes.len()).sum();
        if bytes.len() > (128 * 1024 * 1024usize).saturating_sub(total) {
            return Err(Error::limit("Workspace assembly buffers exceed 128 MiB"));
        }
        Ok(())
    }
    pub fn load(&mut self, bytes: Vec<u8>) -> Result<Loaded> {
        self.check_input_budget(&bytes)?;
        self.insert_assembly(Assembly::load(bytes)?)
    }
    pub fn load_dependency(
        &mut self,
        bytes: Vec<u8>,
        expected: &crate::assembly::Identity,
    ) -> Result<Option<Loaded>> {
        self.load_dependency_kind(bytes, expected, false)
    }
    /// For bytes taken from a known reference-pack directory. Older framework
    /// packs may omit ReferenceAssemblyAttribute but still contain only stubs.
    pub fn load_reference_dependency(
        &mut self,
        bytes: Vec<u8>,
        expected: &crate::assembly::Identity,
    ) -> Result<Option<Loaded>> {
        self.load_dependency_kind(bytes, expected, true)
    }
    fn load_dependency_kind(
        &mut self,
        bytes: Vec<u8>,
        expected: &crate::assembly::Identity,
        reference_only: bool,
    ) -> Result<Option<Loaded>> {
        self.check_input_budget(&bytes)?;
        let mut assembly = Assembly::load(bytes)?;
        assembly.info.reference_assembly |= reference_only;
        if !expected.matches(&assembly.info.identity) {
            return Ok(None);
        }
        self.insert_assembly(assembly).map(Some)
    }
    fn insert_assembly(&mut self, a: Assembly) -> Result<Loaded> {
        for (id, existing) in &self.assemblies {
            if existing.info.identity.matches(&a.info.identity) {
                if existing.bytes == a.bytes {
                    return Ok(Loaded {
                        id: *id,
                        info: existing.info.clone(),
                    });
                }
                return Err(Error::metadata(
                    "A different assembly with this exact identity is already loaded. Close the workspace before replacing it.",
                ));
            }
        }
        if self.assemblies.len() >= 64 {
            return Err(Error::limit("A workspace supports at most 64 assemblies"));
        }
        let total = self
            .assemblies
            .values()
            .map(|a| a.bytes.len())
            .sum::<usize>();
        if total + a.bytes.len() > 128 * 1024 * 1024 {
            return Err(Error::limit("Workspace assembly buffers exceed 128 MiB"));
        }
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| Error::limit("Assembly ID space exhausted"))?;
        let info = a.info.clone();
        self.assemblies.insert(id, a);
        self.bump_revision();
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
        let result =
            decompile::decompile_with(self.assembly(id)?, token, &|t| self.accessor(id, t))?;
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
                    crate::signature::Type::Named(t) | crate::signature::Type::ValueType(t) => t,
                    crate::signature::Type::GenericInstance { base, .. } => {
                        if let crate::signature::Type::Named(t)
                        | crate::signature::Type::ValueType(t) = *base
                        {
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
                let mut found = candidates.into_iter().filter(|n| {
                    crate::resolution::same_member(source, token, target, n.token, field)
                });
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
        metadata["required_reference"] = json!(self.required_reference(id, token));
        Ok(metadata)
    }
    /// Follow only explicit metadata scopes; never infer a package or filename from a type name.
    pub fn required_reference(
        &self,
        id: u32,
        mut token: u32,
    ) -> Option<&crate::assembly::Reference> {
        let a = self.assembly(id).ok()?;
        for _ in 0..49 {
            let rid = (token & 0xffffff).checked_sub(1)? as usize;
            token = match token >> 24 {
                35 => return a.references.iter().find(|r| r.token == token),
                1 => coded(a.metadata().type_refs.get(rid)?.resolution_scope),
                10 => coded(a.metadata().member_refs.get(rid)?.class),
                43 => coded(a.metadata().method_specs.get(rid)?.method),
                27 => {
                    let row = a.metadata().type_specs.get(rid)?;
                    let mut ty = crate::signature::type_spec(a.blob(row.signature).ok()?).ok()?;
                    if let crate::signature::Type::GenericInstance { base, .. } = ty {
                        ty = *base;
                    }
                    match ty {
                        crate::signature::Type::Named(t) | crate::signature::Type::ValueType(t) => {
                            t
                        }
                        _ => return None,
                    }
                }
                _ => return None,
            };
        }
        None
    }
    pub fn close(&mut self, id: u32) {
        self.assemblies.remove(&id);
        self.edits.retain(|(a, _), _| *a != id);
        self.bump_revision();
        self.cache.retain(|((a, _), _, _)| *a != id);
        self.cache_bytes = self.cache.iter().map(|(_, _, n)| n).sum();
    }
    pub fn dispose(&mut self) {
        self.assemblies.clear();
        self.cache.clear();
        self.cache_bytes = 0;
        self.edits.clear();
        self.bump_revision();
    }
    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.cache.clear();
        self.cache_bytes = 0;
    }
    pub fn check_revision(&self, revision: u32) -> Result<()> {
        if self.revision != revision {
            return Err(Error::new(
                crate::error::ErrorCode::Cancelled,
                "Workspace changed during the operation; start it again",
            ));
        }
        Ok(())
    }
    pub fn open_edit(&self, id: u32, token: u32) -> Result<crate::edit::EditPreview> {
        crate::edit::preview(
            self.assembly(id)?,
            token,
            self.revision,
            self.edits.get(&(id, token)),
        )
    }
    pub fn apply_edit(
        &mut self,
        id: u32,
        token: u32,
        revision: u32,
        rows: Vec<crate::cil_encode::EditableInstruction>,
    ) -> Result<crate::edit::EditPreview> {
        self.check_revision(revision)?;
        if self.edits.len() >= 256 && !self.edits.contains_key(&(id, token)) {
            return Err(Error::limit("At most 256 method edits per workspace"));
        }
        let a = self.assembly(id)?;
        let edit = crate::edit::validate(a, token, &rows)?;
        let original = a
            .body(token)?
            .ok_or_else(|| Error::cil("Missing method body"))?;
        let original_code = crate::cil_encode::encode(&crate::cil_encode::from_body(&original))?;
        if original_code == edit.code {
            self.edits.remove(&(id, token));
        } else {
            self.edits.insert((id, token), edit);
        }
        self.bump_revision();
        self.open_edit(id, token)
    }
    pub fn discard_edit(
        &mut self,
        id: u32,
        token: u32,
        revision: u32,
    ) -> Result<crate::edit::EditPreview> {
        self.check_revision(revision)?;
        self.edits.remove(&(id, token));
        self.bump_revision();
        self.open_edit(id, token)
    }
    pub fn edits_info(&self, id: u32) -> Result<Value> {
        let a = self.assembly(id)?;
        Ok(
            json!({"revision":self.revision,"methods":self.edits.iter().filter(|((assembly,_),_)|*assembly==id).map(|((_,token),edit)|json!({"token":token,"name":a.resolve(*token),"code_size":edit.code.len()})).collect::<Vec<_>>(),"writer_error":crate::pe_write::capability(a).err()}),
        )
    }
    pub fn export_modified(&self, id: u32, revision: u32) -> Result<Vec<u8>> {
        self.check_revision(revision)?;
        let edits = self
            .edits
            .iter()
            .filter(|((assembly, _), _)| *assembly == id)
            .map(|((_, token), edit)| (*token, edit.clone()))
            .collect();
        crate::pe_write::write(self.assembly(id)?, &edits)
    }
}
