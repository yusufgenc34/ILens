use crate::{
    analysis,
    assembly::Assembly,
    ast,
    cfg::ControlFlowGraph,
    cil::MethodBody,
    emit,
    error::{Error, Result},
    ir, transform,
};
use serde::Serialize;
#[derive(Debug, Clone, Serialize)]
pub struct MethodAnalysis {
    pub token: u32,
    pub csharp: String,
    pub il: String,
    pub quality: String,
    pub diagnostics: Vec<Error>,
    pub body: Option<MethodBody>,
    pub cfg: Option<ControlFlowGraph>,
    pub stack: Option<analysis::StackAnalysis>,
}
pub fn decompile(a: &Assembly, token: u32) -> Result<MethodAnalysis> {
    let body = a.body(token)?;
    let mut result = MethodAnalysis {
        token,
        csharp: String::new(),
        il: String::new(),
        quality: "declaration".to_owned(),
        diagnostics: vec![],
        body: body.clone(),
        cfg: None,
        stack: None,
    };
    let Some(body) = body else {
        result.csharp = format!(
            "// No CIL body: abstract, interface, P/Invoke, or runtime-provided member.\n{};",
            a.declaration(token)?
        );
        result.il = "// This member has no CIL body.".to_owned();
        return Ok(result);
    };
    result.il = emit::il(a, token, &body);
    let reconstructed = (|| -> Result<String> {
        let cfg = ControlFlowGraph::build(&body)?;
        result.cfg = Some(cfg.clone());
        if body.instructions.len() > 10_000 {
            return Err(Error::limitation(
                "High-level reconstruction is limited to 10,000 instructions; complete IL remains available",
            ));
        }
        let stack = analysis::analyze(a, token, &body, &cfg)?;
        result.stack = Some(stack.clone());
        if a.attributes.get(&token).into_iter().flatten().any(|i| {
            let name = a.resolve(crate::assembly::coded(
                a.metadata().custom_attributes[*i].attr_type,
            ));
            name.contains("AsyncStateMachineAttribute")
                || name.contains("IteratorStateMachineAttribute")
                || name.contains("AsyncIteratorStateMachineAttribute")
        }) {
            return Err(Error::limitation(
                "Async/iterator state-machine reconstruction is not implemented. Inspect the original CIL and generated types.",
            ));
        }
        let mut ir = ir::lift(a, token, &body, &cfg, stack)?;
        transform::simplify(&mut ir);
        let ast = ast::structure(&ir, &cfg);
        emit::csharp(a, token, &body, &ir, &cfg, &ast)
    })();
    match reconstructed {
        Ok(code) => {
            result.csharp = code;
            result.quality = "csharp_like".to_owned();
        }
        Err(e) => {
            result.quality = "annotated_il".to_owned();
            result.csharp = format!(
                "// C# reconstruction is unavailable for this method.\n// {}\n// Original instructions follow. Select IL for the full disassembly.\n\n{}",
                e.detail.replace(['\n', '\r'], " "),
                result.il
            );
            result.diagnostics.push(e);
        }
    }
    Ok(result)
}
