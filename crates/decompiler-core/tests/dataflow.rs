use decompiler_core::{
    analysis,
    assembly::Assembly,
    cfg::ControlFlowGraph,
    cil::{self, MethodBody},
};
fn fixture() -> Assembly {
    Assembly::load(include_bytes!("../../../samples/fixtures/ILens.Patterns.dll").to_vec()).unwrap()
}
fn body(code: &[u8]) -> MethodBody {
    MethodBody {
        max_stack: 8,
        init_locals: false,
        local_signature: 0,
        code_size: code.len() as u32,
        file_offset: 0,
        instructions: cil::decode(code).unwrap(),
        exceptions: vec![],
    }
}
#[test]
fn rejects_stack_underflow() {
    let a = fixture();
    let token = a.nodes.iter().find(|n| n.name == "Multiply").unwrap().token;
    let b = body(&[0x58, 0x2a]);
    let cfg = ControlFlowGraph::build(&b).unwrap();
    assert!(
        analysis::analyze(&a, token, &b, &cfg)
            .unwrap_err()
            .detail
            .contains("underflow")
    );
}
#[test]
fn rejects_mismatched_join_heights() {
    let a = fixture();
    let token = a.nodes.iter().find(|n| n.name == "Multiply").unwrap().token;
    let b = body(&[0x02, 0x2d, 1, 0x16, 0x16, 0x2a]);
    let cfg = ControlFlowGraph::build(&b).unwrap();
    assert!(analysis::analyze(&a, token, &b, &cfg).is_err());
}
#[test]
fn discovers_backedge_and_exit() {
    let b = body(&[0x02, 0x2c, 2, 0x2b, 0xfb, 0x16, 0x2a]);
    let cfg = ControlFlowGraph::build(&b).unwrap();
    assert_eq!(cfg.blocks.len(), 3);
    assert_eq!(cfg.blocks[1].successors, vec![0]);
    assert_eq!(cfg.blocks[0].successors, vec![1, 2]);
}
#[test]
fn rejects_argument_index_outside_signature() {
    let a = fixture();
    let token = a.nodes.iter().find(|n| n.name == "Multiply").unwrap().token;
    let b = body(&[0x0e, 250, 0x2a]);
    let cfg = ControlFlowGraph::build(&b).unwrap();
    assert!(
        analysis::analyze(&a, token, &b, &cfg)
            .unwrap_err()
            .detail
            .contains("variable")
    );
}

#[test]
fn cyclic_stack_edge_is_a_parallel_assignment() {
    use decompiler_core::{
        ast::{self, Ast},
        ir::{Edge, Expr},
        signature::Type,
    };
    let b = body(&[0x02, 0x2c, 2, 0x2b, 0xfb, 0x16, 0x2a]);
    let cfg = ControlFlowGraph::build(&b).unwrap();
    let edge = Edge {
        target: 0,
        values: vec![
            Expr::var("stack_0000_1", Type::primitive("int")),
            Expr::var("stack_0000_0", Type::primitive("int")),
        ],
    };
    let ast = ast::edge_assignments(&edge, &cfg);
    assert!(
        matches!(&ast[..], [Ast::ParallelAssign { targets, values }] if targets.len() == 2 && values.len() == 2)
    );
}
