//! An in-memory resolver; assembly identity is exact and no network resolution occurs.
use crate::{
    assembly::{Assembly, AssemblyInfo, coded},
    decompile::{self, MethodAnalysis},
    error::{Error, Result},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
/// A field or method to add, as the editor submits it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewMemberRequest {
    /// "field" or "method".
    pub kind: String,
    pub name: String,
    /// The field type, or the method's return type ("void" allowed).
    #[serde(rename = "type")]
    pub type_name: String,
    /// "public", "internal", "protected" or "private".
    pub access: String,
    #[serde(default, rename = "static")]
    pub is_static: bool,
    #[serde(default)]
    pub parameters: Vec<NewParameter>,
}
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewParameter {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
}
#[derive(Serialize)]
pub struct Loaded {
    pub id: u32,
    pub info: AssemblyInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<u32>,
}
pub struct Session {
    assemblies: BTreeMap<u32, Assembly>,
    // Analysis copies retain identity but never participate in dependency binding.
    derived: BTreeMap<u32, u32>,
    deobfuscation: Option<crate::deobfuscate::Job>,
    next_deobfuscation: u32,
    next_id: u32,
    cache: VecDeque<((u32, u32), MethodAnalysis, usize)>,
    cache_bytes: usize,
    edits: BTreeMap<(u32, u32), crate::edit::ValidatedEdit>,
    pub revision: u32,
    // Edits are keyed by assembly ID and token, so loading unrelated assemblies must
    // not invalidate an open editor. Only edit-set changes advance this counter.
    pub edit_revision: u32,
    // External definitions per assembly, valid for one workspace revision.
    externals: std::cell::RefCell<BTreeMap<u32, (u32, std::rc::Rc<crate::edit::Externals>)>>,
    // Each assembly with method edits as written, so views show the edited code; valid
    // for one edit revision.
    views: std::cell::RefCell<BTreeMap<u32, (u32, std::rc::Rc<Assembly>)>>,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            assemblies: BTreeMap::new(),
            derived: BTreeMap::new(),
            deobfuscation: None,
            next_deobfuscation: 0,
            next_id: 1,
            cache: VecDeque::new(),
            cache_bytes: 0,
            edits: BTreeMap::new(),
            revision: 0,
            edit_revision: 0,
            externals: std::cell::RefCell::new(BTreeMap::new()),
            views: std::cell::RefCell::new(BTreeMap::new()),
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
        if !expected.binds_to(&assembly.info.identity) {
            return Ok(None);
        }
        self.insert_assembly(assembly).map(Some)
    }
    fn insert_assembly(&mut self, a: Assembly) -> Result<Loaded> {
        for (id, existing) in &self.assemblies {
            if self.derived.contains_key(id) {
                continue;
            }
            if existing.info.identity.matches(&a.info.identity) {
                if existing.bytes == a.bytes {
                    return Ok(Loaded {
                        id: *id,
                        info: existing.info.clone(),
                        derived_from: None,
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
        Ok(Loaded {
            id,
            info,
            derived_from: None,
        })
    }
    pub fn assembly(&self, id: u32) -> Result<&Assembly> {
        self.assemblies
            .get(&id)
            .ok_or_else(|| Error::metadata("Assembly is closed or unknown"))
    }
    /// The assembly as its method edits write it, when it has any. Views decompile this,
    /// so an applied IL or C# edit shows in the code; tokens are unchanged by edits.
    fn edited(&self, id: u32) -> Result<Option<std::rc::Rc<Assembly>>> {
        if !self.edits.keys().any(|(assembly, _)| *assembly == id) {
            return Ok(None);
        }
        if let Some((revision, view)) = self.views.borrow().get(&id)
            && *revision == self.edit_revision
        {
            return Ok(Some(view.clone()));
        }
        let bytes = self.export_modified(id, self.edit_revision)?;
        let view = std::rc::Rc::new(Assembly::load(bytes)?);
        self.views
            .borrow_mut()
            .insert(id, (self.edit_revision, view.clone()));
        Ok(Some(view))
    }
    /// The method's IL listing and body, from the edited assembly when it has edits.
    pub fn disassemble(&self, id: u32, token: u32) -> Result<Value> {
        let edited = self.edited(id)?;
        let a = edited.as_deref().map_or_else(|| self.assembly(id), Ok)?;
        let body = a.body(token)?;
        Ok(json!({"il":body.as_ref().map(|b|crate::emit::il(a,token,b)),"body":body}))
    }
    /// Tokens of the assembly's edited methods.
    pub fn edited_methods(&self, id: u32) -> Vec<u32> {
        self.edits
            .keys()
            .filter(|(assembly, _)| *assembly == id)
            .map(|(_, token)| *token)
            .collect()
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
        let edited = self.edited(id)?;
        let a = edited.as_deref().map_or_else(|| self.assembly(id), Ok)?;
        let result = decompile::decompile_view(a, token, &|t| self.accessor(id, t))?;
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
        Ok(json!(a.references.iter().map(|r| {
            // Exact matches win over framework unification (2.0/3.5 → 4.0).
            let resolved = self.canonical().find(|(_,a)|r.identity.matches(&a.info.identity)).or_else(|| self.canonical().find(|(_,a)|r.identity.binds_to(&a.info.identity))).map(|(id,_)|id);
            json!({"token":r.token,"identity":r.identity,"resolved_id":resolved,"status":if resolved.is_some(){"resolved"}else{"unresolved"}})
        }).collect::<Vec<_>>()))
    }
    fn canonical(&self) -> impl Iterator<Item = (&u32, &Assembly)> {
        self.assemblies
            .iter()
            .filter(|(id, _)| !self.derived.contains_key(id))
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
        let edited = self.edited(id)?;
        let a = edited.as_deref().map_or_else(|| self.assembly(id), Ok)?;
        a.validate_token(token)?;
        if token >> 24 == 2 {
            let source = crate::typeview::type_source(a, token, &|t| self.accessor(id, t))?;
            return Ok(
                json!({"declaration":source.text,"references":source.references,"partial":source.partial,"metadata":self.metadata(id,token)?}),
            );
        }
        if matches!(token >> 24, 4 | 20 | 23) {
            let source = crate::typeview::member_source(a, token, &|t| self.accessor(id, t))?;
            return Ok(
                json!({"declaration":source.text,"references":source.references,"partial":source.partial,"metadata":self.metadata(id,token)?}),
            );
        }
        if token >> 24 == 0x70 {
            // A literal on its own says little; list the methods that load it.
            let usages = self.string_usages(id, token)?;
            let mut text = a.declaration(token)?;
            let mut references = Vec::new();
            text.push_str(&format!(
                "\n\n// Used by {} method{}:\n",
                usages.len(),
                if usages.len() == 1 { "" } else { "s" }
            ));
            for (method, name) in &usages {
                text.push_str("// ");
                let start = text.encode_utf16().count();
                text.push_str(name);
                // Only sites in this assembly can be opened from here; the rest are listed.
                if let Some(method) = method {
                    references.push(crate::emit::SourceReference {
                        start,
                        end: text.encode_utf16().count(),
                        token: *method,
                    });
                }
                text.push('\n');
            }
            if usages.is_empty() {
                text.push_str("// No ldstr site found in the loaded assemblies.\n");
            }
            return Ok(
                json!({"declaration":text,"references":references,"metadata":self.metadata(id,token)?}),
            );
        }
        let text = a.declaration(token)?;
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
            // Properties and events open their member view like fields and methods.
            2 | 4 | 6 | 20 | 23 => {
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
                    self.canonical()
                        .find(|(_, a)| identity.matches(&a.info.identity))
                        .or_else(|| {
                            self.canonical()
                                .find(|(_, a)| identity.binds_to(&a.info.identity))
                        })
                        .map(|(i, _)| *i)?
                } else if scope >> 24 == 0 {
                    id
                } else {
                    return None;
                };
                let target = self.assemblies.get(&target_id)?;
                let name = source.resolve_full(token);
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
    /// One page of a metadata table: ECMA column names and every cell decoded, with
    /// heap values read and table/coded indexes shown as tokens.
    pub fn table_rows(&self, id: u32, table: usize, start: u32, count: u32) -> Result<Value> {
        use crate::metadata::Column;
        let a = self.assembly(id)?;
        if table > 44 {
            return Err(Error::metadata("Unknown metadata table"));
        }
        let metadata = &a.bytes[a.image.metadata.clone()];
        let stream = |name: &str| {
            a.store
                .streams
                .iter()
                .find(|s| s.name == name)
                .map_or(&[][..], |s| &metadata[s.offset..s.offset + s.size])
        };
        let tables = crate::metadata_write::Tables::decode(stream("#~"))?;
        let columns = crate::metadata::schema(table);
        let rows = &tables.rows[table];
        let start = start.max(1);
        let end = (start + count.min(1000)).min(rows.len() as u32 + 1);
        let token_cell = |token: u32| {
            let table = (token >> 24) as usize;
            // A list column may point one past the end: an empty range.
            if (token & 0xff_ffff) as usize > tables.rows.get(table).map_or(0, Vec::len) {
                return json!({"text": format!("0x{token:08X} (end)")});
            }
            let name = match token >> 24 {
                1 | 2 | 4 | 6 | 10 | 20 | 23 | 26 | 27 | 35 | 43 => a.resolve(token),
                _ => String::new(),
            };
            let name = if name.starts_with("unresolved_") {
                String::new()
            } else {
                name
            };
            json!({"text": if name.is_empty() { format!("0x{token:08X}") } else { format!("0x{token:08X} {name}") }, "token": token})
        };
        let cells = |row: &Vec<u32>| -> Vec<Value> {
            columns
                .iter()
                .zip(row)
                .map(|(c, v)| match *c {
                    Column::U16 => json!({"text": format!("0x{v:04X}")}),
                    Column::U32 => json!({"text": format!("0x{v:08X}")}),
                    Column::String => json!({"text": a.string(*v).map(|s| format!("{s:?}")).unwrap_or_else(|_| format!("#Strings[0x{v:X}]"))}),
                    Column::Blob => {
                        let bytes = a.blob(*v).unwrap_or(&[]);
                        let preview: Vec<String> = bytes.iter().take(16).map(|b| format!("{b:02X}")).collect();
                        json!({"text": format!("#Blob[0x{v:X}] {}{}", preview.join(" "), if bytes.len() > 16 { " …" } else { "" })})
                    }
                    Column::Guid => {
                        let heap = stream("#GUID");
                        let at = (*v as usize).saturating_sub(1) * 16;
                        json!({"text": heap.get(at..at + 16).filter(|_| *v != 0).map_or_else(|| "null".to_owned(), |g| g.iter().map(|b| format!("{b:02x}")).collect::<String>())})
                    }
                    Column::Table(t) | Column::List(t) => token_cell(((t as u32) << 24) | v),
                    Column::Coded(k) => {
                        let bits = k.tag_bits();
                        match crate::metadata::coded_tables(k).get((v & ((1 << bits) - 1)) as usize) {
                            Some(Some(t)) if v >> bits != 0 => token_cell(((*t as u32) << 24) | (v >> bits)),
                            _ => json!({"text": format!("0x{v:X}")}),
                        }
                    }
                })
                .collect()
        };
        let page: Vec<Value> = (start..end)
            .map(|row| {
                json!({"row": row, "token": ((table as u32) << 24) | row, "cells": cells(&rows[row as usize - 1])})
            })
            .collect();
        Ok(
            json!({"table": table, "columns": crate::metadata::column_names(table), "total": rows.len(), "start": start, "rows": page}),
        )
    }
    /// File bytes for the hex editor, at most 64 KiB per request.
    pub fn read_bytes(&self, id: u32, offset: usize, length: usize) -> Result<Vec<u8>> {
        let bytes = &self.assembly(id)?.bytes;
        let start = offset.min(bytes.len());
        let end = start.saturating_add(length.min(64 * 1024)).min(bytes.len());
        Ok(bytes[start..end].to_vec())
    }
    /// The file with raw byte patches applied, and whether it still parses as a managed
    /// assembly. Raw patches bypass IL verification; the result says so honestly.
    pub fn patched_bytes(
        &self,
        id: u32,
        patches: &[(usize, u8)],
    ) -> Result<(Vec<u8>, Option<String>)> {
        let mut bytes = self.assembly(id)?.bytes.clone();
        if patches.len() > 1 << 20 {
            return Err(Error::limit("At most 1,048,576 patched bytes"));
        }
        for (offset, value) in patches {
            *bytes
                .get_mut(*offset)
                .ok_or_else(|| Error::metadata("Patch offset outside the file"))? = *value;
        }
        let problem = Assembly::load(bytes.clone())
            .err()
            .map(|e| format!("{}: {}", e.message, e.detail));
        Ok((bytes, problem))
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
        self.derived.remove(&id);
        self.edits.retain(|(a, _), _| *a != id);
        self.views.borrow_mut().remove(&id);
        self.bump_edit_revision();
        self.cache.retain(|((a, _), _, _)| *a != id);
        self.cache_bytes = self.cache.iter().map(|(_, _, n)| n).sum();
    }
    pub fn dispose(&mut self) {
        self.assemblies.clear();
        self.derived.clear();
        self.cache.clear();
        self.cache_bytes = 0;
        self.edits.clear();
        self.views.borrow_mut().clear();
        self.bump_edit_revision();
    }
    fn bump_revision(&mut self) {
        self.deobfuscation = None;
        self.revision = self.revision.wrapping_add(1);
        self.cache.clear();
        self.cache_bytes = 0;
    }
    fn bump_edit_revision(&mut self) {
        self.edit_revision = self.edit_revision.wrapping_add(1);
        self.bump_revision();
    }
    fn check_edit_revision(&self, revision: u32) -> Result<()> {
        if self.edit_revision != revision {
            return Err(Error::new(
                crate::error::ErrorCode::Cancelled,
                "Method edits changed during the operation; reopen the editor",
            ));
        }
        Ok(())
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
    /// The methods whose CIL loads this user string (`ldstr`), so a literal can be
    /// traced back to where it is used. Reads decoded instructions only.
    /// Methods that load this literal. Sites in the assembly the token belongs to carry
    /// their method token and are navigable; sites in other loaded assemblies are matched
    /// by text, since a `#US` token means nothing outside its own module.
    pub fn string_usages(&self, id: u32, token: u32) -> Result<Vec<(Option<u32>, String)>> {
        let a = self.assembly(id)?;
        if token >> 24 != 0x70 {
            return Err(Error::metadata("Expected a user-string token"));
        }
        let text = a.user_string(token).ok();
        let mut found = Vec::new();
        for index in 0..a.metadata().method_defs.len() {
            if found.len() >= 500 {
                return Ok(found);
            }
            let method = 0x0600_0001 + index as u32;
            let Ok(Some(body)) = a.body(method) else {
                continue;
            };
            if body
                .instructions
                .iter()
                .any(|i| i.opcode == 0x72 && i.token() == Some(token))
            {
                found.push((Some(method), a.resolve(method)));
            }
        }
        let Some(text) = text else { return Ok(found) };
        for (&other, b) in &self.assemblies {
            if other == id || self.derived.get(&other).is_some_and(|o| *o == id) {
                continue;
            }
            // Walk the other module's `#US` heap once for the tokens that hold this
            // text, rather than resolving a literal per instruction.
            let heap = b.metadata().user_strings.data();
            let mut tokens = std::collections::HashSet::new();
            let mut offset = 1;
            while offset < heap.len() && tokens.len() < 1024 {
                let mut reader = crate::reader::Reader::new(&heap[offset..]);
                let Ok(len) = reader.compressed() else { break };
                let header = reader.pos;
                let Some(end) = offset.checked_add(header + len as usize) else {
                    break;
                };
                if end > heap.len() {
                    break;
                }
                if len > 0
                    && b.user_string(0x7000_0000 | offset as u32)
                        .is_ok_and(|s| s == text)
                {
                    tokens.insert(0x7000_0000 | offset as u32);
                }
                offset = end.max(offset + 1);
            }
            if tokens.is_empty() {
                continue;
            }
            for index in 0..b.metadata().method_defs.len() {
                if found.len() >= 500 {
                    return Ok(found);
                }
                let method = 0x0600_0001 + index as u32;
                let Ok(Some(body)) = b.body(method) else {
                    continue;
                };
                if body
                    .instructions
                    .iter()
                    .any(|i| i.opcode == 0x72 && i.token().is_some_and(|t| tokens.contains(&t)))
                {
                    found.push((
                        None,
                        format!("{} · {}", b.info.identity.name, b.resolve(method)),
                    ));
                }
            }
        }
        Ok(found)
    }
    /// Every method's call edges, callers, strings and role tags. A deobfuscated
    /// copy also reports the original (obfuscated) name at each stable token.
    pub fn function_map(&self, id: u32) -> Result<crate::function_map::FunctionMap> {
        let a = self.assembly(id)?;
        let original = self.derived.get(&id).and_then(|o| self.assemblies.get(o));
        crate::function_map::build(a, original)
    }
    pub fn embedded_payloads(&self, id: u32) -> Result<crate::embedded::Payloads> {
        crate::embedded::payloads(self.assembly(id)?)
    }
    pub fn deobfuscate_begin(
        &mut self,
        assembly: u32,
        options: crate::deobfuscate::Options,
    ) -> Result<crate::deobfuscate::Progress> {
        self.deobfuscation = None;
        if self.edits.keys().any(|(id, _)| *id == assembly) {
            return Err(crate::edit::unsupported(
                "Save/reopen or discard pending method edits before deobfuscation",
            ));
        }
        self.next_deobfuscation = self
            .next_deobfuscation
            .checked_add(1)
            .ok_or_else(|| Error::limit("Deobfuscation ID space exhausted"))?;
        let job = crate::deobfuscate::Job::new(
            self.next_deobfuscation,
            assembly,
            self.revision,
            self.assembly(assembly)?,
            options,
        )?;
        let progress = job.progress();
        self.deobfuscation = Some(job);
        Ok(progress)
    }
    fn take_deobfuscation(&mut self, id: u32) -> Result<crate::deobfuscate::Job> {
        // A stale request must not cancel a newer job.
        if !self
            .deobfuscation
            .as_ref()
            .is_some_and(|j| j.id == id && j.revision == self.revision)
        {
            return Err(crate::deobfuscate::cancelled());
        }
        self.deobfuscation
            .take()
            .ok_or_else(crate::deobfuscate::cancelled)
    }
    pub fn deobfuscate_step(&mut self, id: u32) -> Result<crate::deobfuscate::Progress> {
        let mut job = self.take_deobfuscation(id)?;
        let progress = job.step(self.assembly(job.assembly)?)?;
        self.deobfuscation = Some(job);
        Ok(progress)
    }
    pub fn deobfuscate_finish(&mut self, id: u32) -> Result<crate::deobfuscate::Report> {
        let mut job = self.take_deobfuscation(id)?;
        let report = job.finish(self.assembly(job.assembly)?)?;
        self.deobfuscation = Some(job);
        Ok(report)
    }
    pub fn deobfuscate_cancel(&mut self, id: u32) {
        if self
            .deobfuscation
            .as_ref()
            .is_some_and(|j| id == 0 || j.id == id)
        {
            self.deobfuscation = None;
        }
    }
    pub fn deobfuscate_download(&self, id: u32) -> Result<Vec<u8>> {
        self.deobfuscation
            .as_ref()
            .filter(|j| j.id == id && j.revision == self.revision)
            .and_then(|j| j.output.clone())
            .ok_or_else(crate::deobfuscate::cancelled)
    }
    pub fn deobfuscate_open(&mut self, id: u32) -> Result<Loaded> {
        let mut job = self.take_deobfuscation(id)?;
        if let Some(id) = job.opened {
            let loaded = Loaded {
                id,
                info: self.assembly(id)?.info.clone(),
                derived_from: Some(job.assembly),
            };
            self.deobfuscation = Some(job);
            return Ok(loaded);
        }
        let bytes = job.output.as_ref().ok_or_else(|| {
            crate::edit::unsupported("Finish and validate deobfuscation before opening a copy")
        })?;
        // Copies are distinct even for a no-op result; charge their retained bytes.
        if self.assemblies.len() >= 64
            || self
                .assemblies
                .values()
                .map(|a| a.bytes.len())
                .sum::<usize>()
                + bytes.len()
                > 128 * 1024 * 1024
        {
            return Err(Error::limit(
                "Opening the analysis copy would exceed workspace limits",
            ));
        }
        let mut a = Assembly::load(bytes.clone())?;
        a.info.diagnostics.push(format!("Derived analysis copy of workspace assembly {}. Its original identity is retained; this copy is excluded from dependency resolution.",job.assembly));
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| Error::limit("Assembly ID space exhausted"))?;
        let loaded = Loaded {
            id,
            info: a.info.clone(),
            derived_from: Some(job.assembly),
        };
        self.assemblies.insert(id, a);
        self.derived.insert(id, job.assembly);
        self.bump_revision();
        job.revision = self.revision;
        job.opened = Some(id);
        self.deobfuscation = Some(job);
        Ok(loaded)
    }
    pub fn open_edit(&self, id: u32, token: u32) -> Result<crate::edit::EditPreview> {
        let externals = self.externals(id);
        crate::edit::with_externals((*externals).clone(), || {
            crate::edit::preview(
                self.assembly(id)?,
                token,
                self.edit_revision,
                self.edits.get(&(id, token)),
            )
        })
    }
    /// What loaded dependencies define for this assembly's type and member references,
    /// so edits can use external enums, structs, classes and members.
    fn externals(&self, id: u32) -> std::rc::Rc<crate::edit::Externals> {
        if let Some((revision, cached)) = self.externals.borrow().get(&id)
            && *revision == self.revision
        {
            return cached.clone();
        }
        let computed = std::rc::Rc::new(self.compute_externals(id));
        self.externals
            .borrow_mut()
            .insert(id, (self.revision, computed.clone()));
        computed
    }
    fn compute_externals(&self, id: u32) -> crate::edit::Externals {
        use crate::edit::{ExternalKind, ExternalType};
        let mut out = crate::edit::Externals::default();
        let Some(source) = self.assemblies.get(&id) else {
            return out;
        };
        if self.assemblies.len() < 2 {
            return out;
        }
        // Per dependency: type full name → TypeDef, (owner, member name) → members.
        let mut types: BTreeMap<u32, std::collections::HashMap<String, u32>> = BTreeMap::new();
        let mut members: BTreeMap<u32, std::collections::HashMap<(u32, String), Vec<u32>>> =
            BTreeMap::new();
        for (target_id, target) in self.canonical() {
            if *target_id == id {
                continue;
            }
            let mut names = std::collections::HashMap::new();
            let mut owned = std::collections::HashMap::<(u32, String), Vec<u32>>::new();
            for n in &target.nodes {
                match n.token >> 24 {
                    2 => {
                        names.insert(n.full_name.clone(), n.token);
                    }
                    4 | 6 => owned
                        .entry((n.parent, n.name.clone()))
                        .or_default()
                        .push(n.token),
                    _ => {}
                }
            }
            types.insert(*target_id, names);
            members.insert(*target_id, owned);
        }
        // The loaded assembly that defines a TypeRef, by its resolution scope.
        let scope_target = |token: u32| -> Option<u32> {
            let mut scope = coded(
                source
                    .metadata()
                    .type_refs
                    .get((token & 0xff_ffff).checked_sub(1)? as usize)?
                    .resolution_scope,
            );
            for _ in 0..48 {
                if scope >> 24 != 1 {
                    break;
                }
                scope = coded(
                    source
                        .metadata()
                        .type_refs
                        .get((scope & 0xff_ffff).checked_sub(1)? as usize)?
                        .resolution_scope,
                );
            }
            if scope >> 24 != 35 {
                return None;
            }
            let identity = &source
                .references
                .iter()
                .find(|r| r.token == scope)?
                .identity;
            self.canonical()
                .find(|(i, a)| **i != id && identity.matches(&a.info.identity))
                .or_else(|| {
                    self.canonical()
                        .find(|(i, a)| **i != id && identity.binds_to(&a.info.identity))
                })
                .map(|(i, _)| *i)
        };
        let mut resolved = std::collections::HashMap::new();
        for row in 1..=source.metadata().type_refs.len() as u32 {
            let token = 0x0100_0000 | row;
            let Some(target_id) = scope_target(token) else {
                continue;
            };
            let Some(definition) = types[&target_id].get(&source.resolve_full(token)).copied()
            else {
                continue;
            };
            let target = &self.assemblies[&target_id];
            resolved.insert(token, (target_id, definition));
            let Some(node) = target.nodes.iter().find(|n| n.token == definition) else {
                continue;
            };
            let row = &target.metadata().type_defs[(definition & 0xff_ffff) as usize - 1];
            let kind = match node.kind.as_str() {
                "interface" => ExternalKind::Interface,
                "struct" => ExternalKind::Struct,
                "enum" => {
                    let underlying = target
                        .nodes
                        .iter()
                        .find(|f| f.parent == definition && f.name == "value__")
                        .and_then(|f| target.field_ref(f.token).ok())
                        .map(|(_, _, ty)| match ty {
                            crate::signature::Type::Primitive(name) => name,
                            _ => String::new(),
                        })
                        .unwrap_or_default();
                    ExternalKind::Enum(underlying)
                }
                kind => ExternalKind::Class {
                    abstract_: row.flags & 0x80 != 0,
                    delegate: kind == "delegate",
                },
            };
            // Base types and interfaces, followed through the dependency's own types.
            let mut bases = vec![];
            let mut pending = vec![definition];
            while let Some(t) = pending.pop() {
                if bases.len() > 64 || t >> 24 != 2 || t & 0xff_ffff == 0 {
                    continue;
                }
                let r = &target.metadata().type_defs[(t & 0xff_ffff) as usize - 1];
                let mut parents = vec![coded(r.extends)];
                parents.extend(
                    target
                        .metadata()
                        .interface_impls
                        .iter()
                        .filter(|i| i.class == t & 0xff_ffff)
                        .map(|i| coded(i.interface)),
                );
                // Interfaces and System.Object have no base (row 0).
                for parent in parents.into_iter().filter(|p| p & 0xff_ffff != 0) {
                    let name = target.resolve_full(parent);
                    if !bases.contains(&name) {
                        bases.push(name);
                        pending.push(parent);
                    }
                }
            }
            out.types.insert(token, ExternalType { kind, bases });
        }
        for row in 1..=source.metadata().member_refs.len() as u32 {
            let token = 0x0a00_0000 | row;
            let member = &source.metadata().member_refs[row as usize - 1];
            let Some((target_id, owner)) = resolved.get(&coded(member.class)).copied() else {
                continue;
            };
            let target = &self.assemblies[&target_id];
            let Ok(name) = source.string(member.name) else {
                continue;
            };
            let field = source
                .blob(member.signature)
                .ok()
                .and_then(|b| b.first().copied())
                == Some(6);
            let mut found = members[&target_id]
                .get(&(owner, name.to_owned()))
                .into_iter()
                .flatten()
                .copied()
                .filter(|t| (t >> 24 == 4) == field)
                .filter(|t| crate::resolution::same_member(source, token, target, *t, field));
            if let (Some(definition), None) = (found.next(), found.next()) {
                let flags = if field {
                    target.metadata().fields[(definition & 0xff_ffff) as usize - 1].flags
                } else {
                    target.metadata().method_defs[(definition & 0xff_ffff) as usize - 1].flags
                };
                out.members.insert(token, flags);
            }
        }
        out
    }
    pub fn apply_edit(
        &mut self,
        id: u32,
        token: u32,
        revision: u32,
        rows: Vec<crate::cil_encode::EditableInstruction>,
        locals: Option<Vec<String>>,
        regions: Option<Vec<crate::edit::EditRegion>>,
    ) -> Result<crate::edit::EditPreview> {
        self.check_edit_revision(revision)?;
        if self.edits.len() >= 256 && !self.edits.contains_key(&(id, token)) {
            return Err(Error::limit("At most 256 method edits per workspace"));
        }
        let a = self.assembly(id)?;
        let externals = self.externals(id);
        let edit = crate::edit::with_externals((*externals).clone(), || {
            crate::edit::validate_request(a, token, &rows, locals.as_deref(), regions.as_deref())
        })?;
        let original = a
            .body(token)?
            .ok_or_else(|| Error::cil("Missing method body"))?;
        let original_code = crate::cil_encode::encode(&crate::cil_encode::from_body(&original))?;
        let same_regions = serde_json::to_vec(&edit.exceptions).ok()
            == serde_json::to_vec(&original.exceptions).ok();
        if original_code == edit.code && edit.locals.is_none() && same_regions {
            self.edits.remove(&(id, token));
        } else {
            self.edits.insert((id, token), edit);
        }
        self.bump_edit_revision();
        self.open_edit(id, token)
    }
    pub fn discard_edit(
        &mut self,
        id: u32,
        token: u32,
        revision: u32,
    ) -> Result<crate::edit::EditPreview> {
        self.check_edit_revision(revision)?;
        self.edits.remove(&(id, token));
        self.bump_edit_revision();
        self.open_edit(id, token)
    }
    /// All workspace ids, including derived views.
    pub fn ids(&self) -> Vec<u32> {
        self.assemblies.keys().copied().collect()
    }
    /// Compiler references use canonical assemblies and, when selected, the edited
    /// snapshot instead of its same-identity original. No duplicate definitions.
    pub fn compiler_ids(&self, selected: Option<u32>) -> Result<Vec<u32>> {
        let identity = selected
            .map(|id| self.assembly(id).map(|a| &a.info.identity))
            .transpose()?;
        let mut ids: Vec<u32> = self
            .canonical()
            .filter(|(_, a)| !identity.is_some_and(|identity| identity.matches(&a.info.identity)))
            .map(|(id, _)| *id)
            .collect();
        if let Some(id) = selected {
            ids.push(id);
        }
        Ok(ids)
    }
    /// The C# skeleton for editing a method, with the line the method starts on.
    pub fn csharp_source(&self, id: u32, token: u32) -> Result<Value> {
        crate::edit::supported_body(self.assembly(id)?, token)?;
        // Start from the method as last edited, not the original body.
        let edited = self.edited(id)?;
        let a = edited.as_deref().map_or_else(|| self.assembly(id), Ok)?;
        let source = crate::csharp_edit::source(a, token)?;
        Ok(
            json!({"method": source.method, "before": source.before, "after": source.after, "line": source.before.lines().count() + 1}),
        )
    }
    /// Applies a C#-compiled body: references it needs are appended first (the workspace
    /// then holds the rewritten assembly, with pending edits written into it), and the
    /// mapped IL is validated and stored like an IL edit.
    pub fn csharp_apply(
        &mut self,
        id: u32,
        token: u32,
        revision: u32,
        compiled: &[u8],
    ) -> Result<Value> {
        self.check_edit_revision(revision)?;
        if compiled.len() > 64 * 1024 * 1024 {
            return Err(Error::limit("Compiled output exceeds 64 MiB"));
        }
        let merged = crate::csharp_edit::merge(self.assembly(id)?, token, compiled)?;
        let mut reloaded = false;
        if !merged.rows.is_empty() {
            let a = self.assembly(id)?;
            let pending: BTreeMap<u32, crate::edit::ValidatedEdit> = self
                .edits
                .iter()
                .filter(|((assembly, _), _)| *assembly == id)
                .map(|((_, t), edit)| (*t, edit.clone()))
                .collect();
            let externals = self.externals(id);
            let base = if pending.is_empty() {
                a.bytes.clone()
            } else {
                crate::edit::with_externals((*externals).clone(), || {
                    crate::pe_write::write(a, &pending)
                })?
            };
            let base = Assembly::load(base)?;
            // Writing pending edits keeps tokens; merge again against the written bytes.
            let merged = crate::csharp_edit::merge(&base, token, compiled)?;
            let replaced = Assembly::load(crate::pe_write::add_rows(&base, &merged.rows)?)?;
            self.assemblies.insert(id, replaced);
            self.edits.retain(|(assembly, _), _| *assembly != id);
            self.cache.retain(|((assembly, _), _, _)| *assembly != id);
            self.cache_bytes = self.cache.iter().map(|(_, _, bytes)| *bytes).sum();
            self.revision += 1;
            self.bump_edit_revision();
            reloaded = true;
        }
        let a = self.assembly(id)?;
        let externals = self.externals(id);
        let edit = crate::edit::with_externals((*externals).clone(), || {
            crate::edit::validate_types(
                a,
                token,
                &merged.instructions,
                Some(merged.locals.clone()),
                Some(&merged.regions),
            )
        })?;
        self.edits.insert((id, token), edit);
        self.bump_edit_revision();
        let preview = self.open_edit(id, token)?;
        Ok(json!({"preview": preview, "reloaded": reloaded, "info": self.assembly(id)?.info}))
    }
    /// Adds a field or method to a type: pending IL edits are written first, the member
    /// is added with renumbered tokens, and the result replaces the assembly in place.
    pub fn add_member(
        &mut self,
        id: u32,
        revision: u32,
        owner: u32,
        request: NewMemberRequest,
    ) -> Result<Value> {
        self.check_edit_revision(revision)?;
        let a = self.assembly(id)?;
        let parse = |text: &str| crate::edit::parse_type(a, text);
        let access: u16 = match request.access.as_str() {
            "private" => 1,
            "internal" => 3,
            "protected" => 4,
            "public" => 6,
            other => {
                return Err(Error::new(
                    crate::error::ErrorCode::InvalidEdit,
                    format!("Unknown access {other}"),
                ));
            }
        };
        let flags = access | if request.is_static { 0x10 } else { 0 };
        let member = match request.kind.as_str() {
            "field" => crate::pe_write::NewMember::Field {
                name: request.name.clone(),
                ty: parse(&request.type_name)?,
                flags,
            },
            "method" => crate::pe_write::NewMember::Method {
                name: request.name.clone(),
                return_type: if request.type_name.trim() == "void" {
                    crate::signature::Type::primitive("void")
                } else {
                    parse(&request.type_name)?
                },
                parameters: request
                    .parameters
                    .iter()
                    .map(|p| Ok((p.name.clone(), parse(&p.type_name)?)))
                    .collect::<Result<_>>()?,
                // Methods hide by signature, as C# declares them.
                flags: flags | 0x80,
            },
            other => {
                return Err(Error::new(
                    crate::error::ErrorCode::InvalidEdit,
                    format!("Unknown member kind {other}"),
                ));
            }
        };
        if request.parameters.len() > 64 {
            return Err(Error::limit(
                "A new method can declare at most 64 parameters",
            ));
        }
        let pending: BTreeMap<u32, crate::edit::ValidatedEdit> = self
            .edits
            .iter()
            .filter(|((assembly, _), _)| *assembly == id)
            .map(|((_, token), edit)| (*token, edit.clone()))
            .collect();
        let base = if pending.is_empty() {
            a.bytes.clone()
        } else {
            let externals = self.externals(id);
            crate::edit::with_externals((*externals).clone(), || {
                crate::pe_write::write(a, &pending)
            })?
        };
        let base = Assembly::load(base)?;
        let added = crate::pe_write::add_member(&base, owner, &member)?;
        let replaced = Assembly::load(added.bytes)?;
        let info = replaced.info.clone();
        self.assemblies.insert(id, replaced);
        self.edits.retain(|(assembly, _), _| *assembly != id);
        self.cache.retain(|((assembly, _), _, _)| *assembly != id);
        self.cache_bytes = self.cache.iter().map(|(_, _, bytes)| *bytes).sum();
        self.revision += 1;
        self.bump_edit_revision();
        Ok(json!({"token": added.token, "info": info}))
    }
    pub fn edits_info(&self, id: u32) -> Result<Value> {
        let a = self.assembly(id)?;
        Ok(
            json!({"revision":self.revision,"edit_revision":self.edit_revision,"methods":self.edits.iter().filter(|((assembly,_),_)|*assembly==id).map(|((_,token),edit)|json!({"token":token,"name":a.resolve(*token),"code_size":edit.code.len()})).collect::<Vec<_>>(),"writer_error":crate::pe_write::capability(a).err(),"resource_writer_error":crate::pe_write::literal_capability(a).err()}),
        )
    }
    pub fn export_modified(&self, id: u32, revision: u32) -> Result<Vec<u8>> {
        self.check_edit_revision(revision)?;
        let edits = self
            .edits
            .iter()
            .filter(|((assembly, _), _)| *assembly == id)
            .map(|((_, token), edit)| (*token, edit.clone()))
            .collect();
        let externals = self.externals(id);
        crate::edit::with_externals((*externals).clone(), || {
            crate::pe_write::write(self.assembly(id)?, &edits)
        })
    }
}
