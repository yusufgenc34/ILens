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
    pub references: Vec<crate::emit::SourceReference>,
    pub source_body: Option<String>,
    pub source_declaration: Option<String>,
}
/// The code-viewer form: dnSpy-style short type names in C#, while the IL keeps the
/// fully qualified names that IL syntax requires.
pub fn decompile_view(
    a: &Assembly,
    token: u32,
    accessor: &dyn Fn(u32) -> Option<crate::resolution::Accessor>,
) -> Result<MethodAnalysis> {
    let full = decompile_with(a, token, accessor)?;
    if full.quality != "csharp_like" {
        return Ok(full);
    }
    let namespace = {
        let mut owner = *a.owners.get(&token).unwrap_or(&0);
        while let Some(parent) = a.owners.get(&owner).filter(|p| **p >> 24 == 2) {
            owner = *parent;
        }
        a.nodes
            .iter()
            .find(|n| n.token == owner)
            .map(|n| n.namespace.clone())
            .unwrap_or_default()
    };
    let view = {
        let _scope = a.short_names(&namespace);
        decompile_with(a, token, accessor)?
    };
    Ok(MethodAnalysis {
        csharp: view.csharp,
        references: view.references,
        ..full
    })
}
pub fn decompile(a: &Assembly, token: u32) -> Result<MethodAnalysis> {
    decompile_with(a, token, &|t| crate::resolution::local_accessor(a, t))
}
pub fn decompile_with(
    a: &Assembly,
    token: u32,
    accessor: &dyn Fn(u32) -> Option<crate::resolution::Accessor>,
) -> Result<MethodAnalysis> {
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
        references: vec![],
        source_body: None,
        source_declaration: None,
    };
    if a.info.reference_assembly {
        result.csharp = format!(
            "// Reference assembly: API declaration only; the implementation is not included.\n{};",
            a.declaration(token)?
        );
        result.il = format!(
            "// Reference assembly: these are placeholder instructions, not the implementation.\n{}",
            body.as_ref()
                .map(|body| emit::il(a, token, body))
                .unwrap_or_else(|| "// No CIL body.".into())
        );
        result.diagnostics.push(Error::limitation("Reference assemblies provide API metadata, not method implementations. Load an implementation assembly to inspect its code."));
        return Ok(result);
    }
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
        if crate::statemachine::kind_of(a, token).is_some() {
            // async and iterator methods are rebuilt from their compiler state machine.
            let lowered = crate::statemachine::lower(a, token, accessor)?;
            let ast = ast::structure_from(
                &lowered.ir,
                &lowered.cfg,
                &lowered.body.exceptions,
                lowered.entry,
            );
            let source = emit::state_machine_source(a, token, &lowered, &ast)?;
            return finish(a, token, &mut result, source);
        }
        let ir = lift_with(a, token, &body, &cfg, stack, accessor)?;
        let ast = ast::structure(&ir, &cfg, &body.exceptions);
        let source = emit::method_source(a, token, &body, &ir, &cfg, &ast)?;
        finish(a, token, &mut result, source)
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

/// CIL → typed IR with accessor binding and simplification, shared by ordinary methods
/// and state-machine `MoveNext` bodies.
pub fn lift_with(
    a: &Assembly,
    token: u32,
    body: &MethodBody,
    cfg: &ControlFlowGraph,
    stack: analysis::StackAnalysis,
    accessor: &dyn Fn(u32) -> Option<crate::resolution::Accessor>,
) -> Result<ir::TypedIr> {
    let mut ir = ir::lift(a, token, body, cfg, stack)?;
    // Repeated calls in a method should bind a metadata token only once.
    let mut accessors = std::collections::HashMap::new();
    for block in &mut ir.blocks {
        let mut resolve = |expr: &mut ir::Expr| {
            if let ir::ExprKind::Call {
                token,
                accessor: value,
                virtual_call,
                ..
            } = &mut expr.kind
            {
                *value = accessors
                    .entry(*token)
                    .or_insert_with(|| accessor(*token))
                    .clone()
                    .filter(|p| *virtual_call || !p.is_virtual);
            }
        };
        for statement in &mut block.statements {
            transform::statement_exprs(statement, &mut resolve);
        }
        transform::terminator_exprs(&mut block.terminator, &mut resolve);
    }
    transform::simplify(&mut ir);
    Ok(ir)
}
// Header comment, attributes and navigation ranges for the code viewer.
fn finish(
    a: &Assembly,
    token: u32,
    result: &mut MethodAnalysis,
    source: emit::MethodSource,
) -> Result<String> {
    let mut header = crate::typeview::token_comment(a, "", token);
    for attribute in crate::attributes::render(a, token) {
        header.push('\n');
        header.push_str(&attribute);
    }
    let code = format!("{header}\n{}\n{}", source.declaration, source.body);
    let prefix_len = code.encode_utf16().count()
        - source.declaration.encode_utf16().count()
        - 1
        - source.body.encode_utf16().count();
    result.references = source
        .references
        .into_iter()
        .map(|mut r| {
            r.start += prefix_len;
            r.end += prefix_len;
            r
        })
        .collect();
    result.source_body = Some(source.body);
    result.source_declaration = Some(source.declaration);
    Ok(code)
}
