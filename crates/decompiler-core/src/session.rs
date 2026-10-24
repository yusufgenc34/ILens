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
    // Edits are keyed by assembly ID and token, so loading unrelated assemblies must
    // not invalidate an open editor. Only edit-set changes advance this counter.
    pub edit_revision: u32,
    // External definitions per assembly, valid for one workspace revision.
    externals: std::cell::RefCell<BTreeMap<u32, (u32, std::rc::Rc<crate::edit::Externals>)>>,
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
            edit_revision: 0,
            externals: std::cell::RefCell::new(BTreeMap::new()),
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
            decompile::decompile_view(self.assembly(id)?, token, &|t| self.accessor(id, t))?;
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
        self.bump_edit_revision();
        self.cache.retain(|((a, _), _, _)| *a != id);
        self.cache_bytes = self.cache.iter().map(|(_, _, n)| n).sum();
    }
    pub fn dispose(&mut self) {
        self.assemblies.clear();
        self.cache.clear();
        self.cache_bytes = 0;
        self.edits.clear();
        self.bump_edit_revision();
    }
    fn bump_revision(&mut self) {
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
        for (target_id, target) in &self.assemblies {
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
            self.assemblies
                .iter()
                .find(|(i, a)| **i != id && identity.matches(&a.info.identity))
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
    pub fn edits_info(&self, id: u32) -> Result<Value> {
        let a = self.assembly(id)?;
        Ok(
            json!({"revision":self.revision,"edit_revision":self.edit_revision,"methods":self.edits.iter().filter(|((assembly,_),_)|*assembly==id).map(|((_,token),edit)|json!({"token":token,"name":a.resolve(*token),"code_size":edit.code.len()})).collect::<Vec<_>>(),"writer_error":crate::pe_write::capability(a).err()}),
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
