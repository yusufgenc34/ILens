use decompiler_core::assembly::Assembly;
fn fixture() -> Assembly {
    Assembly::load(include_bytes!("../../../samples/fixtures/ILens.Patterns.dll").to_vec())
        .expect("fixture should parse")
}
#[test]
fn parse_and_resolve() {
    let a = fixture();
    assert_eq!(a.info.identity.name, "ILens.Patterns");
    assert!(a.nodes.iter().any(|n| n.name == "Calculator"));
    let n = a
        .nodes
        .iter()
        .find(|n| {
            n.name == "Add"
                && n.full_name.contains("Calculator.Add")
                && !n.full_name.contains("ICalculator")
        })
        .unwrap();
    let body = a.body(n.token).unwrap().unwrap();
    assert_eq!(
        body.instructions
            .iter()
            .map(|i| i.name.as_str())
            .collect::<Vec<_>>(),
        ["ldarg.1", "ldarg.2", "add", "ret"]
    );
    assert!(
        a.declaration(n.token)
            .unwrap()
            .contains("int left, int right")
    );
    assert!(!a.resources().unwrap().is_empty());
}
#[test]
fn all_fixture_methods_decode() {
    let a = fixture();
    for n in a.nodes.iter().filter(|n| n.token >> 24 == 6) {
        a.body(n.token)
            .unwrap_or_else(|e| panic!("{}: {e}", n.name));
    }
}
#[test]
fn heaps_and_search() {
    let a = fixture();
    assert!(
        a.search("greeting")
            .unwrap()
            .iter()
            .any(|r| r.name == "Greeting")
    );
    assert!(
        a.search("Hello from local")
            .unwrap()
            .iter()
            .any(|r| r.kind == "string")
    );
}
#[test]
fn truncated_inputs() {
    let bytes = include_bytes!("../../../samples/fixtures/ILens.Patterns.dll");
    for n in (0..bytes.len()).step_by(37) {
        assert!(
            Assembly::load(bytes[..n].to_vec()).is_err(),
            "truncation {n} accepted"
        );
    }
}
#[test]
fn fixture_cfg_and_stack() {
    let a = fixture();
    for n in a.nodes.iter().filter(|n| n.token >> 24 == 6) {
        if let Some(body) = a.body(n.token).unwrap() {
            let cfg = decompiler_core::cfg::ControlFlowGraph::build(&body).unwrap();
            decompiler_core::analysis::analyze(&a, n.token, &body, &cfg)
                .unwrap_or_else(|e| panic!("{}: {e}", n.full_name));
        }
    }
}
#[test]
fn common_csharp_patterns() {
    let a = fixture();
    for (name, expected) in [
        ("Add", "return (left + right);"),
        ("Absolute", "if ("),
        ("Sum", "for ("),
        ("Countdown", "while ("),
        ("Describe", "switch ("),
        ("MakeArray", "new System.Int32[3]"),
        ("SafeDivide", "catch (System.DivideByZeroException"),
        ("Identity", "T value"),
        ("UnsignedMax", "unchecked((uint)(-1))"),
        ("LargeInteger", "unchecked((ulong)(unchecked((long)(-1))))"),
        ("UnsignedDivide", "unchecked((uint)"),
    ] {
        let n = a
            .nodes
            .iter()
            .find(|n| n.name == name && !n.full_name.contains("ICalculator"))
            .unwrap();
        let result = decompiler_core::decompile::decompile(&a, n.token).unwrap();
        assert_eq!(
            result.quality, "csharp_like",
            "{name}: {:?}",
            result.diagnostics
        );
        assert!(
            result.csharp.contains(expected),
            "{name}: {}",
            result.csharp
        );
    }
}
#[test]
fn all_reconstruction_is_bounded() {
    let a = fixture();
    for n in a.nodes.iter().filter(|n| n.token >> 24 == 6) {
        let r = decompiler_core::decompile::decompile(&a, n.token).unwrap();
        assert!(!r.csharp.is_empty());
    }
}
#[test]
fn dependency_resolution_uses_identity_and_signature() {
    use decompiler_core::session::Session;
    let mut session = Session::default();
    let main = session
        .load(include_bytes!("../../../samples/fixtures/ILens.Patterns.dll").to_vec())
        .unwrap();
    let member = {
        let a = session.assembly(main.id).unwrap();
        a.metadata()
            .member_refs
            .iter()
            .enumerate()
            .find(|(_, m)| a.string(m.name).unwrap() == "Twice")
            .map(|(i, _)| 0x0a000001 + i as u32)
            .unwrap()
    };
    assert!(session.resolve_definition(main.id, member).is_none());
    let dependency = session
        .load(include_bytes!("../../../samples/fixtures/ILens.Dependency.dll").to_vec())
        .unwrap();
    let (resolved_id, token) = session.resolve_definition(main.id, member).unwrap();
    assert_eq!(resolved_id, dependency.id);
    assert!(
        session
            .assembly(resolved_id)
            .unwrap()
            .resolve(token)
            .contains("Twice")
    );
    session.close(dependency.id);
    assert!(session.resolve_definition(main.id, member).is_none());
}
#[test]
fn mismatched_version_is_not_silently_unified() {
    let a = fixture();
    let mut identity = a.info.identity.clone();
    identity.version = "99.0.0.0".into();
    assert!(!a.info.identity.matches(&identity));
    identity = a.info.identity.clone();
    identity.public_key_token = "abcdef0123456789".into();
    assert!(!a.info.identity.matches(&identity));
}

#[test]
fn async_and_iterator_patterns_are_explicit_limitations() {
    let a = fixture();
    for name in ["Later", "Range"] {
        let token = a.nodes.iter().find(|n| n.name == name).unwrap().token;
        let result = decompiler_core::decompile::decompile(&a, token).unwrap();
        assert_eq!(result.quality, "annotated_il");
        assert!(result.diagnostics[0].detail.contains("state-machine"));
        assert!(result.il.contains("IL_"));
    }
}
