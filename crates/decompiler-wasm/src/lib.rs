#![forbid(unsafe_code)]
use decompiler_core::{error::Error, session::Session};
use wasm_bindgen::prelude::*;
fn js<T: serde::Serialize>(value: &T) -> Result<JsValue, JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| JsValue::from_str(&e.to_string()))
}
fn error(e: Error) -> JsValue {
    js(&e).unwrap_or_else(|_| JsValue::from_str("Analysis error"))
}
#[wasm_bindgen]
pub struct Decompiler {
    session: Session,
}
impl Default for Decompiler {
    fn default() -> Self {
        Self::new()
    }
}
#[wasm_bindgen]
impl Decompiler {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            session: Session::default(),
        }
    }
    // Vec takes ownership of the binding's single copy into WASM linear memory.
    pub fn load_assembly(&mut self, bytes: Vec<u8>) -> Result<JsValue, JsValue> {
        js(&self.session.load(bytes).map_err(error)?)
    }
    pub fn load_dependency(&mut self, bytes: Vec<u8>, identity: &str) -> Result<JsValue, JsValue> {
        if identity.len() > 4096 {
            return Err(error(Error::limit("Dependency identity is too large")));
        }
        let expected =
            serde_json::from_str(identity).map_err(|e| error(Error::metadata(e.to_string())))?;
        js(&self
            .session
            .load_dependency(bytes, &expected)
            .map_err(error)?)
    }
    pub fn load_reference_dependency(
        &mut self,
        bytes: Vec<u8>,
        identity: &str,
    ) -> Result<JsValue, JsValue> {
        if identity.len() > 4096 {
            return Err(error(Error::limit("Dependency identity is too large")));
        }
        let expected =
            serde_json::from_str(identity).map_err(|e| error(Error::metadata(e.to_string())))?;
        js(&self
            .session
            .load_reference_dependency(bytes, &expected)
            .map_err(error)?)
    }
    pub fn get_assembly_info(&self, id: u32) -> Result<JsValue, JsValue> {
        js(&self.session.overview(id).map_err(error)?)
    }
    pub fn get_tree(&self, id: u32) -> Result<JsValue, JsValue> {
        js(&self.session.assembly(id).map_err(error)?.nodes)
    }
    pub fn detect_protection(&self, id: u32) -> Result<JsValue, JsValue> {
        let a = self.session.assembly(id).map_err(error)?;
        js(&decompiler_core::detect::scan(&a.bytes, Some(a)).map_err(error)?)
    }
    // Scans files that could not be loaded as assemblies, such as native packer stubs.
    pub fn detect_file(&self, bytes: &[u8]) -> Result<JsValue, JsValue> {
        if bytes.len() > decompiler_core::pe::MAX_FILE {
            return Err(error(Error::limit("Maximum file size is 64 MiB")));
        }
        js(&decompiler_core::detect::scan(bytes, None).map_err(error)?)
    }
    pub fn get_pe_headers(&self, id: u32) -> Result<JsValue, JsValue> {
        js(
            &decompiler_core::headers::headers(self.session.assembly(id).map_err(error)?)
                .map_err(error)?,
        )
    }
    pub fn get_type(&self, id: u32, token: u32) -> Result<JsValue, JsValue> {
        js(&self.session.type_view(id, token).map_err(error)?)
    }
    pub fn get_method(&self, id: u32, token: u32) -> Result<JsValue, JsValue> {
        self.get_metadata(id, token)
    }
    pub fn decompile_method(&mut self, id: u32, token: u32) -> Result<JsValue, JsValue> {
        js(&self.session.decompile(id, token).map_err(error)?)
    }
    pub fn disassemble_method(&self, id: u32, token: u32) -> Result<JsValue, JsValue> {
        let a = self.session.assembly(id).map_err(error)?;
        let body = a.body(token).map_err(error)?;
        js(
            &serde_json::json!({"il":body.as_ref().map(|b|decompiler_core::emit::il(a,token,b)),"body":body}),
        )
    }
    pub fn get_metadata(&self, id: u32, token: u32) -> Result<JsValue, JsValue> {
        js(&self.session.metadata(id, token).map_err(error)?)
    }
    pub fn get_strings(&self, id: u32, query: &str) -> Result<JsValue, JsValue> {
        js(&self
            .session
            .assembly(id)
            .map_err(error)?
            .strings(query, 250)
            .map_err(error)?)
    }
    pub fn get_references(&self, id: u32) -> Result<JsValue, JsValue> {
        js(&self.session.references(id).map_err(error)?)
    }
    pub fn analyze_references(
        &self,
        id: u32,
        token: u32,
        source: u32,
        start: u32,
        outgoing: bool,
    ) -> Result<JsValue, JsValue> {
        js(&self
            .session
            .analyze_page(id, token, source, start as usize, outgoing)
            .map_err(error)?)
    }
    pub fn get_xrefs(&self, id: u32, token: u32, start: u32) -> Result<JsValue, JsValue> {
        js(&self
            .session
            .xrefs_page(id, token, start as usize, 64)
            .map_err(error)?)
    }
    pub fn search(&self, id: u32, query: &str) -> Result<JsValue, JsValue> {
        if query.len() > 512 {
            return Err(error(Error::limit("Search query exceeds 512 bytes")));
        }
        js(&self
            .session
            .assembly(id)
            .map_err(error)?
            .search(query)
            .map_err(error)?)
    }
    pub fn close(&mut self, id: u32) {
        self.session.close(id);
    }
    pub fn dispose(&mut self) {
        self.session.dispose();
    }
    pub fn export_plan(&self, id: u32, token: u32) -> Result<JsValue, JsValue> {
        let types = decompiler_core::export::plan(
            self.session.assembly(id).map_err(error)?,
            if token == 0 { None } else { Some(token) },
        )
        .map_err(error)?;
        js(&serde_json::json!({"revision":self.session.revision,"types":types}))
    }
    pub fn export_member(&self, id: u32, token: u32, revision: u32) -> Result<JsValue, JsValue> {
        self.session.check_revision(revision).map_err(error)?;
        js(&decompiler_core::export::member_with(
            self.session.assembly(id).map_err(error)?,
            token,
            &|t| self.session.accessor(id, t),
        )
        .map_err(error)?)
    }
    pub fn export_resource(&self, id: u32, token: u32, revision: u32) -> Result<Vec<u8>, JsValue> {
        self.session.check_revision(revision).map_err(error)?;
        decompiler_core::export::resource(self.session.assembly(id).map_err(error)?, token)
            .map_err(error)
    }
    pub fn resource_preview(&self, id: u32, token: u32, entry: i32) -> Result<JsValue, JsValue> {
        let bytes = self.resource_bytes(id, token, entry)?;
        js(&decompiler_core::resources::preview(&bytes).map_err(error)?)
    }
    pub fn resource_bytes(&self, id: u32, token: u32, entry: i32) -> Result<Vec<u8>, JsValue> {
        let bytes =
            decompiler_core::export::resource(self.session.assembly(id).map_err(error)?, token)
                .map_err(error)?;
        if entry < 0 {
            Ok(bytes)
        } else {
            decompiler_core::resources::entry_bytes(&bytes, entry as usize).map_err(error)
        }
    }
    pub fn export_dependency(&self, id: u32, revision: u32) -> Result<Vec<u8>, JsValue> {
        self.session.check_revision(revision).map_err(error)?;
        Ok(self.session.assembly(id).map_err(error)?.bytes.clone())
    }
    pub fn open_method_edit(&self, id: u32, token: u32) -> Result<JsValue, JsValue> {
        js(&self.session.open_edit(id, token).map_err(error)?)
    }
    pub fn apply_method_edit(
        &mut self,
        id: u32,
        token: u32,
        revision: u32,
        instructions: &str,
    ) -> Result<JsValue, JsValue> {
        if instructions.len() > 1024 * 1024 {
            return Err(error(Error::limit("Edit input exceeds 1 MiB")));
        }
        // Either the instruction rows, or {instructions, locals} with new local types.
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Request {
            Rows(Vec<decompiler_core::cil_encode::EditableInstruction>),
            Edit {
                instructions: Vec<decompiler_core::cil_encode::EditableInstruction>,
                locals: Option<Vec<String>>,
                regions: Option<Vec<decompiler_core::edit::EditRegion>>,
            },
        }
        let request: Request = serde_json::from_str(instructions).map_err(|e| {
            error(Error::new(
                decompiler_core::error::ErrorCode::InvalidEdit,
                e.to_string(),
            ))
        })?;
        let (rows, locals, regions) = match request {
            Request::Rows(rows) => (rows, None, None),
            Request::Edit {
                instructions,
                locals,
                regions,
            } => (instructions, locals, regions),
        };
        js(&self
            .session
            .apply_edit(id, token, revision, rows, locals, regions)
            .map_err(error)?)
    }
    pub fn discard_method_edit(
        &mut self,
        id: u32,
        token: u32,
        revision: u32,
    ) -> Result<JsValue, JsValue> {
        js(&self
            .session
            .discard_edit(id, token, revision)
            .map_err(error)?)
    }
    pub fn add_member(
        &mut self,
        id: u32,
        revision: u32,
        owner: u32,
        request: &str,
    ) -> Result<JsValue, JsValue> {
        if request.len() > 64 * 1024 {
            return Err(error(Error::limit("Member request exceeds 64 KiB")));
        }
        let request = serde_json::from_str(request).map_err(|e| {
            error(Error::new(
                decompiler_core::error::ErrorCode::InvalidEdit,
                e.to_string(),
            ))
        })?;
        js(&self
            .session
            .add_member(id, revision, owner, request)
            .map_err(error)?)
    }
    pub fn get_table_rows(
        &self,
        id: u32,
        table: u32,
        start: u32,
        count: u32,
    ) -> Result<JsValue, JsValue> {
        js(&self
            .session
            .table_rows(id, table as usize, start, count)
            .map_err(error)?)
    }
    pub fn read_bytes(&self, id: u32, offset: u32, length: u32) -> Result<Vec<u8>, JsValue> {
        self.session
            .read_bytes(id, offset as usize, length as usize)
            .map_err(error)
    }
    fn patches(patches: &str) -> Result<Vec<(usize, u8)>, JsValue> {
        if patches.len() > 32 * 1024 * 1024 {
            return Err(error(Error::limit("Patch list exceeds 32 MiB")));
        }
        serde_json::from_str(patches).map_err(|e| {
            error(Error::new(
                decompiler_core::error::ErrorCode::InvalidEdit,
                e.to_string(),
            ))
        })
    }
    /// Why the patched file no longer parses, or null when it does.
    pub fn check_patched(&self, id: u32, patches: &str) -> Result<JsValue, JsValue> {
        let (_, problem) = self
            .session
            .patched_bytes(id, &Self::patches(patches)?)
            .map_err(error)?;
        js(&problem)
    }
    pub fn export_patched(&self, id: u32, patches: &str) -> Result<Vec<u8>, JsValue> {
        let (bytes, _) = self
            .session
            .patched_bytes(id, &Self::patches(patches)?)
            .map_err(error)?;
        Ok(bytes)
    }
    pub fn get_edits(&self, id: u32) -> Result<JsValue, JsValue> {
        js(&self.session.edits_info(id).map_err(error)?)
    }
    pub fn get_opcodes(&self) -> Result<JsValue, JsValue> {
        js(&decompiler_core::cil_encode::catalog())
    }
    pub fn export_modified_assembly(&self, id: u32, revision: u32) -> Result<Vec<u8>, JsValue> {
        self.session.export_modified(id, revision).map_err(error)
    }
}
