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
    pub fn get_assembly_info(&self, id: u32) -> Result<JsValue, JsValue> {
        js(&self.session.overview(id).map_err(error)?)
    }
    pub fn get_tree(&self, id: u32) -> Result<JsValue, JsValue> {
        js(&self.session.assembly(id).map_err(error)?.nodes)
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
        js(
            &decompiler_core::export::member(self.session.assembly(id).map_err(error)?, token)
                .map_err(error)?,
        )
    }
    pub fn export_resource(&self, id: u32, token: u32, revision: u32) -> Result<Vec<u8>, JsValue> {
        self.session.check_revision(revision).map_err(error)?;
        decompiler_core::export::resource(self.session.assembly(id).map_err(error)?, token)
            .map_err(error)
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
        let rows = serde_json::from_str(instructions).map_err(|e| {
            error(Error::new(
                decompiler_core::error::ErrorCode::InvalidEdit,
                e.to_string(),
            ))
        })?;
        js(&self
            .session
            .apply_edit(id, token, revision, rows)
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
