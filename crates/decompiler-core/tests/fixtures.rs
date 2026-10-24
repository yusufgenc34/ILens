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
        ("Add", "return left + right;"),
        // Roslyn compiles `if (c) return a; return b;` and `return c ? a : b;` alike.
        ("Absolute", "return value >= 0 ? value : -value;"),
        ("Sum", "for (int num2 = 0; num2 < count; num2++)"),
        ("Countdown", "while ("),
        ("Describe", "switch ("),
        // Single-use label blocks after an exhaustive switch move into their cases.
        (
            "Describe",
            "    default:\n            return \"many\";\n    }\n}",
        ),
        ("Sum", "    int num = 0;\n"),
        ("Countdown", "value--;"),
        ("MakeArray", "new System.Int32[3]"),
        ("SafeDivide", "catch (System.DivideByZeroException"),
        ("Identity", "T value"),
        ("UnsignedMax", "return uint.MaxValue;"),
        ("LargeInteger", "return ulong.MaxValue;"),
        // Unsigned operands need no reinterpreting casts.
        ("UnsignedDivide", "return a / b;"),
        ("SafeDivide", "catch (System.DivideByZeroException)\n"),
        (
            "ReadLength",
            "using (System.IO.StringReader stringReader = new System.IO.StringReader(path))",
        ),
        ("Each", "foreach (int num3 in values)"),
        (
            "WithFinally",
            "this.Changed?.Invoke(this, System.EventArgs.Empty);",
        ),
        ("MakeAdder", "return value => value + amount;"),
        // Integer orderings invert exactly under negation.
        ("BothPositive", "return a > 0 && b > 0;"),
        ("WithFinally", "this.Value++;"),
        // Assigned in the try and in every catch: no initializer, as C# requires none.
        ("SafeDivide", "    int num;\n"),
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
fn async_and_iterator_methods_are_rebuilt_from_state_machines() {
    let a = fixture();
    for (name, expected) in [
        (
            "Later",
            "await System.Threading.Tasks.Task.Yield();\n    return value + 1;",
        ),
        (
            "Range",
            "for (int i = 0; i < n; i++)\n    {\n        yield return i;\n    }",
        ),
    ] {
        let token = a.nodes.iter().find(|n| n.name == name).unwrap().token;
        let result = decompiler_core::decompile::decompile(&a, token).unwrap();
        assert_eq!(result.quality, "csharp_like", "{:?}", result.diagnostics);
        assert!(result.csharp.contains(expected), "{}", result.csharp);
        // No compiler machinery leaks into the rebuilt body.
        for internal in [
            "<>1__state",
            "<>t__builder",
            "<>2__current",
            "MoveNext",
            "GetAwaiter",
        ] {
            assert!(
                !result.csharp.contains(internal),
                "{internal} in {}",
                result.csharp
            );
        }
        // The IL view still shows the kickoff method that starts the machine.
        assert!(result.il.contains("IL_"));
    }
}
