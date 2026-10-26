use decompiler_core::{
    assembly::Assembly,
    cil_encode::EditableInstruction,
    deobfuscate::{Job, Options},
    edit,
    emulate::{self, Budget, Evaluator, Value},
    pe_write,
    session::Session,
};
use std::{collections::BTreeMap, rc::Rc};
const INPUT: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Deobfuscation.dll");
fn token(a: &Assembly, name: &str) -> u32 {
    a.nodes
        .iter()
        .find(|n| n.name == name && n.token >> 24 == 6)
        .unwrap()
        .token
}
fn options() -> Options {
    Options {
        strings: true,
        ..Options::default()
    }
}
fn run(
    bytes: &[u8],
    options: Options,
) -> (Assembly, decompiler_core::deobfuscate::Report, Vec<u8>) {
    let a = Assembly::load(bytes.to_vec()).unwrap();
    let mut job = Job::new(1, 1, 0, &a, options).unwrap();
    while !job.progress().done {
        job.step(&a).unwrap();
    }
    let report = job.finish(&a).unwrap();
    let bytes = job.output.unwrap();
    (Assembly::load(bytes.clone()).unwrap(), report, bytes)
}
fn patched(name: &str, rows: &[(&str, &str, &str)]) -> Vec<u8> {
    let a = Assembly::load(INPUT.to_vec()).unwrap();
    let rows: Vec<_> = rows
        .iter()
        .map(|(id, opcode, operand)| EditableInstruction {
            id: (*id).into(),
            opcode: (*opcode).into(),
            operand: (*operand).into(),
        })
        .collect();
    let t = token(&a, name);
    let edit = edit::validate(&a, t, &rows).unwrap();
    pe_write::write(&a, &BTreeMap::from([(t, edit)])).unwrap()
}
fn value(a: &Assembly, name: &str, args: Vec<Value>) -> Value {
    Evaluator::new(a, Budget::default())
        .call(token(a, name), args)
        .unwrap()
}
#[test]
fn pure_utf16_decoder_changes_real_calls_and_preserves_unresolved_bodies() {
    let original = Assembly::load(INPUT.to_vec()).unwrap();
    let (a, report, _) = run(INPUT, options());
    assert!(report.changed_methods >= 2, "{report:#?}");
    assert!(report.counts.strings >= 2);
    for (name, expected) in [("Message", "Hello ILens"), ("Unicode", "😀")] {
        let body = a.body(token(&a, name)).unwrap().unwrap();
        assert!(
            !body.instructions.iter().any(|i| i.name == "call"),
            "{name}"
        );
        let literal = body
            .instructions
            .iter()
            .find(|i| i.name == "ldstr")
            .unwrap()
            .token()
            .unwrap();
        assert_eq!(a.user_string(literal).unwrap(), expected);
        edit::verify(&a, token(&a, name), &body).unwrap();
    }
    for name in [
        "Decode",
        "Dynamic",
        "ExternalCall",
        "Initialized",
        "Recursive",
        "Infinite",
        "Allocate",
        "WithHandlers",
        "NonInterned",
        "LoneSurrogate",
    ] {
        let before = original.body(token(&original, name)).unwrap().unwrap();
        let after = a.body(token(&a, name)).unwrap().unwrap();
        assert_eq!(
            before
                .instructions
                .iter()
                .map(|i| &i.bytes)
                .collect::<Vec<_>>(),
            after
                .instructions
                .iter()
                .map(|i| &i.bytes)
                .collect::<Vec<_>>(),
            "{name}"
        );
    }
    assert!(
        report
            .methods
            .iter()
            .flat_map(|m| &m.reasons)
            .any(|s| s.contains("initializer"))
    );
    assert!(
        report
            .methods
            .iter()
            .flat_map(|m| &m.reasons)
            .any(|s| s.contains("Non-interned"))
    );
    assert!(
        report
            .methods
            .iter()
            .flat_map(|m| &m.reasons)
            .any(|s| s.contains("surrogate"))
    );
    assert_eq!(a.info.identity, original.info.identity);
    assert_eq!(
        a.nodes
            .iter()
            .map(|n| (n.token, &n.name))
            .collect::<Vec<_>>(),
        original
            .nodes
            .iter()
            .map(|n| (n.token, &n.name))
            .collect::<Vec<_>>()
    );
}
#[test]
fn evaluation_blocks_external_state_initializers_recursion_and_unbounded_work() {
    let a = Assembly::load(INPUT.to_vec()).unwrap();
    for (name, needle) in [
        ("ExternalCall", "allowlist"),
        ("Initialized", "initializer"),
        ("Recursive", "depth"),
        ("Infinite", "budget"),
        ("Allocate", "length"),
    ] {
        let e = Evaluator::new(&a, Budget::default())
            .call(token(&a, name), vec![])
            .unwrap_err();
        assert!(e.0.contains(needle), "{name}: {e}");
    }
    let e = Evaluator::new(&a, Budget::new(100, 1))
        .call(token(&a, "Message"), vec![])
        .unwrap_err();
    assert!(e.0.contains("allocation"));
    let e = Evaluator::new(&a, Budget::new(0, 1024))
        .call(token(&a, "Message"), vec![])
        .unwrap_err();
    assert!(e.0.contains("instruction budget"));
    // Concrete pure evaluation returns exact UTF-16, including non-BMP characters.
    let Value::String(units, _) = value(
        &a,
        "Decode",
        vec![
            Value::String(Rc::new("😀".encode_utf16().collect()), true),
            Value::I32(0),
        ],
    ) else {
        panic!()
    };
    assert_eq!(String::from_utf16(&units).unwrap(), "😀");
}
#[test]
fn constants_branches_and_join_boundaries_preserve_results() {
    let input = patched(
        "Arithmetic",
        &[
            ("a", "ldc.i4.7", ""),
            ("b", "ldc.i4.5", ""),
            ("c", "xor", ""),
            ("d", "ret", ""),
        ],
    );
    let (a, r, _) = run(&input, Options::default());
    assert_eq!(r.counts.constants, 1);
    assert!(matches!(
        value(&a, "Arithmetic", vec![Value::I32(0)]),
        Value::I32(2)
    ));
    let input = patched(
        "Branch",
        &[
            ("a", "ldc.i4.0", ""),
            ("b", "brtrue.s", "no"),
            ("c", "ldc.i4", "42"),
            ("d", "ret", ""),
            ("no", "ldc.i4.m1", ""),
            ("end", "ret", ""),
        ],
    );
    let (a, r, _) = run(&input, Options::default());
    assert_eq!(r.counts.branches, 1, "{r:#?}");
    assert!(matches!(
        value(&a, "Branch", vec![Value::I32(7)]),
        Value::I32(42)
    ));
    let input = patched(
        "Arithmetic",
        &[
            ("a", "ldarg.0", ""),
            ("b", "brtrue.s", "other"),
            ("c", "ldc.i4.1", ""),
            ("d", "br.s", "join"),
            ("other", "ldc.i4.2", ""),
            ("join", "ldc.i4.3", ""),
            ("sum", "add", ""),
            ("end", "ret", ""),
        ],
    );
    let (a, r, bytes) = run(&input, Options::default());
    assert_eq!(r.counts.constants, 0);
    assert_eq!(bytes, input);
    assert!(matches!(
        value(&a, "Arithmetic", vec![Value::I32(0)]),
        Value::I32(4)
    ));
    assert!(matches!(
        value(&a, "Arithmetic", vec![Value::I32(1)]),
        Value::I32(5)
    ));
}
#[test]
fn integer_semantics_and_throwing_operations_are_not_guessed() {
    assert_eq!(emulate::binary("add", i32::MAX, 1), Some(i32::MIN));
    assert_eq!(emulate::binary("div", i32::MIN, -1), None);
    assert_eq!(emulate::binary("div", 1, 0), None);
    assert_eq!(emulate::binary("shr.un", -1, 33), Some(i32::MAX));
    assert_eq!(emulate::binary("shr", -2, 1), Some(-1));
    assert_eq!(emulate::binary("shl", 1, 32), Some(1));
    assert_eq!(emulate::binary("add.ovf", i32::MAX, 1), None);
    let input = patched(
        "Arithmetic",
        &[
            ("a", "ldc.i4.1", ""),
            ("b", "ldc.i4.0", ""),
            ("c", "div", ""),
            ("d", "ret", ""),
        ],
    );
    let (_, r, bytes) = run(&input, Options::default());
    assert_eq!(r.counts.constants, 0);
    assert_eq!(bytes, input);
    let (_, r, bytes) = run(INPUT, Options::default());
    assert_eq!(r.outcome, "no_changes");
    assert_eq!(bytes, INPUT);
}
#[test]
fn sessions_cancel_stale_jobs_and_keep_copies_out_of_resolution() {
    let mut s = Session::default();
    let original = s.load(INPUT.to_vec()).unwrap();
    let old = s.deobfuscate_begin(original.id, options()).unwrap();
    let mut p = s.deobfuscate_begin(original.id, options()).unwrap();
    assert!(s.deobfuscate_step(old.job).is_err());
    s.deobfuscate_cancel(old.job);
    while !p.done {
        p = s.deobfuscate_step(p.job).unwrap();
    }
    s.deobfuscate_finish(p.job).unwrap();
    let output = s.deobfuscate_download(p.job).unwrap();
    let copy = s.deobfuscate_open(p.job).unwrap();
    assert_ne!(original.id, copy.id);
    assert_eq!(copy.derived_from, Some(original.id));
    assert_eq!(s.assembly(original.id).unwrap().bytes, INPUT);
    assert_eq!(s.assembly(copy.id).unwrap().bytes, output);
    assert_eq!(s.deobfuscate_open(p.job).unwrap().id, copy.id);
    assert_eq!(s.compiler_ids(None).unwrap(), vec![original.id]);
    assert_eq!(
        s.compiler_ids(Some(original.id)).unwrap(),
        vec![original.id]
    );
    assert_eq!(s.compiler_ids(Some(copy.id)).unwrap(), vec![copy.id]);
    let mut dependency = Session::default();
    let source = dependency
        .load(include_bytes!("../../../samples/fixtures/ILens.Patterns.dll").to_vec())
        .unwrap();
    let lib = dependency
        .load(include_bytes!("../../../samples/fixtures/ILens.Dependency.dll").to_vec())
        .unwrap();
    let mut p = dependency
        .deobfuscate_begin(lib.id, Options::default())
        .unwrap();
    while !p.done {
        p = dependency.deobfuscate_step(p.job).unwrap();
    }
    dependency.deobfuscate_finish(p.job).unwrap();
    let copied = dependency.deobfuscate_open(p.job).unwrap();
    let refs = dependency.references(source.id).unwrap();
    assert!(
        refs.as_array()
            .unwrap()
            .iter()
            .any(|r| r["resolved_id"] == lib.id)
    );
    dependency.close(lib.id);
    assert!(dependency.assembly(copied.id).is_ok());
    let refs = dependency.references(source.id).unwrap();
    assert!(
        !refs
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["resolved_id"] == copied.id)
    );
    assert!(dependency.deobfuscate_download(p.job).is_err());
    let p = s.deobfuscate_begin(original.id, options()).unwrap();
    s.deobfuscate_cancel(p.job);
    assert!(s.deobfuscate_step(p.job).is_err());
    let p = s.deobfuscate_begin(original.id, options()).unwrap();
    s.close(original.id);
    assert!(s.deobfuscate_step(p.job).is_err());
    s.dispose();
    assert!(s.assembly(copy.id).is_err());
}

#[test]
fn edits_and_workspace_mutations_invalidate_jobs_without_publishing_partial_output() {
    let mut s = Session::default();
    let source = s.load(INPUT.to_vec()).unwrap();
    let t = token(s.assembly(source.id).unwrap(), "Arithmetic");
    let p = s.deobfuscate_begin(source.id, options()).unwrap();
    // A premature finish releases the job; no partial assembly can be downloaded.
    assert!(s.deobfuscate_finish(p.job).is_err());
    assert!(s.deobfuscate_download(p.job).is_err());
    let p = s.deobfuscate_begin(source.id, options()).unwrap();
    s.deobfuscate_step(p.job).unwrap();
    let preview = s.open_edit(source.id, t).unwrap();
    let rows = vec![
        EditableInstruction {
            id: "a".into(),
            opcode: "ldc.i4".into(),
            operand: "7".into(),
        },
        EditableInstruction {
            id: "b".into(),
            opcode: "ret".into(),
            operand: String::new(),
        },
    ];
    let edited = s
        .apply_edit(source.id, t, preview.revision, rows, None, None)
        .unwrap();
    assert!(s.deobfuscate_step(p.job).is_err());
    assert!(s.deobfuscate_download(p.job).is_err());
    assert!(
        s.deobfuscate_begin(source.id, options())
            .unwrap_err()
            .detail
            .contains("pending method edits")
    );
    s.discard_edit(source.id, t, edited.revision).unwrap();
    let p = s.deobfuscate_begin(source.id, options()).unwrap();
    s.load(include_bytes!("../../../samples/fixtures/ILens.Dependency.dll").to_vec())
        .unwrap();
    assert!(s.deobfuscate_step(p.job).is_err());
    assert_eq!(s.assembly(source.id).unwrap().bytes, INPUT);
}

#[test]
fn branch_chains_simplify_without_breaking_cycles() {
    let input = patched(
        "Arithmetic",
        &[
            ("a", "br.s", "b"),
            ("b", "br.s", "c"),
            ("c", "ldc.i4.7", ""),
            ("d", "ret", ""),
        ],
    );
    let (a, r, _) = run(&input, Options::default());
    assert_eq!(r.counts.branches, 1);
    assert_eq!(r.counts.unreachable_instructions, 1);
    assert!(matches!(
        value(&a, "Arithmetic", vec![Value::I32(0)]),
        Value::I32(7)
    ));
    let input = patched("Arithmetic", &[("a", "br.s", "b"), ("b", "br.s", "a")]);
    let (_, r, bytes) = run(&input, Options::default());
    assert_eq!(r.counts.branches, 0);
    assert_eq!(bytes, input);
}

const PATTERNS: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Patterns.dll");
/// Owned stand-in for renaming protection: overwrite the #Strings entries named by
/// declaration rows in place with same-length control characters. Shared suffixes
/// are hidden together, as with real invisible-name obfuscators.
fn hide_names(bytes: &[u8], names: &[&str]) -> Vec<u8> {
    let a = Assembly::load(bytes.to_vec()).unwrap();
    let heap = |name: &str| {
        let s = a.store.streams.iter().find(|s| s.name == name).unwrap();
        a.image.metadata.start + s.offset
    };
    let tables = decompiler_core::metadata_write::Tables::decode(&bytes[heap("#~")..]).unwrap();
    let mut out = bytes.to_vec();
    for (k, name) in names.iter().enumerate() {
        let mut found = false;
        for (table, column) in [(2, 1), (4, 1), (6, 3), (8, 2), (20, 1), (23, 1)] {
            for row in &tables.rows[table] {
                if a.string(row[column]).unwrap() == *name {
                    let at = heap("#Strings") + row[column] as usize;
                    out[at] = k as u8 + 1;
                    out[at + 1..at + name.len()].fill(0x1e);
                    found = true;
                }
            }
        }
        assert!(found, "{name}");
    }
    out
}
fn rename_only() -> Options {
    Options {
        constants: false,
        branches: false,
        strings: false,
        resource_strings: false,
        control_flow: false,
        anti_debug: false,
        rename: true,
        proxies: false,
    }
}
fn hidden(name: &str) -> bool {
    name.chars().any(char::is_control)
}
#[test]
fn renaming_restores_readable_names_and_keeps_name_bound_relations() {
    // "Calculator" is stored as a suffix of "ICalculator", so it is hidden as well.
    let names = [
        "ICalculator",
        "Add",
        "Value",
        "get_Value",
        "set_Value",
        "Changed",
        "add_Changed",
        "remove_Changed",
        "left",
        "right",
    ];
    let input = hide_names(PATTERNS, &names);
    let obfuscated = Assembly::load(input.clone()).unwrap();
    let original = Assembly::load(PATTERNS.to_vec()).unwrap();
    assert!(obfuscated.nodes.iter().any(|n| hidden(&n.name)));
    let (a, report, _) = run(&input, rename_only());
    assert_eq!(report.outcome, "changed", "{report:#?}");
    assert!(report.counts.renamed > 0);
    assert!(
        !a.nodes.iter().any(|n| hidden(&n.name)),
        "hidden names remain"
    );
    assert!(a.info.obfuscation.evidence.is_empty());
    // Tokens are stable: interface method and its implicit implementation share a name.
    let adds: Vec<_> = original
        .nodes
        .iter()
        .filter(|n| n.token >> 24 == 6 && n.name == "Add")
        .map(|n| n.token)
        .collect();
    assert_eq!(adds.len(), 2);
    let renamed: Vec<_> = adds
        .iter()
        .map(|t| a.nodes.iter().find(|n| n.token == *t).unwrap().name.clone())
        .collect();
    assert_eq!(renamed[0], renamed[1]);
    assert!(renamed[0].starts_with("vmethod_"), "{renamed:?}");
    // Accessors follow their property/event names.
    for (old, prefix) in [
        ("get_Value", "get_Property_"),
        ("set_Value", "set_Property_"),
        ("add_Changed", "add_Event_"),
    ] {
        let t = original.nodes.iter().find(|n| n.name == old).unwrap().token;
        let n = &a.nodes.iter().find(|n| n.token == t).unwrap().name;
        assert!(n.starts_with(prefix), "{old} -> {n}");
    }
    let class = original
        .nodes
        .iter()
        .find(|n| n.name == "Calculator")
        .unwrap()
        .token;
    assert!(
        a.nodes
            .iter()
            .find(|n| n.token == class)
            .unwrap()
            .name
            .starts_with("Class")
    );
    let interface = original
        .nodes
        .iter()
        .find(|n| n.name == "ICalculator")
        .unwrap()
        .token;
    assert!(
        a.nodes
            .iter()
            .find(|n| n.token == interface)
            .unwrap()
            .name
            .starts_with("Interface")
    );
    // Bodies are untouched and still reconstruct as before.
    for (i, row) in original.metadata().method_defs.iter().enumerate() {
        let t = 0x06000001 + i as u32;
        if row.rva == 0 {
            continue;
        }
        let shape = |b: decompiler_core::cil::MethodBody| {
            b.instructions
                .into_iter()
                .map(|i| i.bytes)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shape(obfuscated.body(t).unwrap().unwrap()),
            shape(a.body(t).unwrap().unwrap())
        );
        let before = decompiler_core::decompile::decompile(&original, t)
            .unwrap()
            .quality;
        assert_eq!(
            decompiler_core::decompile::decompile(&a, t)
                .unwrap()
                .quality,
            before,
            "{t:08X}"
        );
    }
    let add = decompiler_core::decompile::decompile(&a, adds[1])
        .unwrap()
        .csharp;
    assert!(add.contains("arg0") && add.contains("arg1"), "{add}");
}
#[test]
fn renaming_leaves_clean_assemblies_byte_identical_and_survives_field_rva_layouts() {
    for input in [PATTERNS, INPUT] {
        let (_, report, bytes) = run(input, rename_only());
        assert_eq!(report.outcome, "no_changes");
        assert_eq!(bytes, input);
    }
    // The general IL writer rejects Field-RVA storage; renaming still runs and the
    // IL transforms are dropped with a notice instead of failing the whole job.
    let resource = include_bytes!("../../../samples/fixtures/ILens.ResourceStrings.dll");
    let a = Assembly::load(resource.to_vec()).unwrap();
    assert!(pe_write::capability(&a).is_err());
    let report = run(
        resource,
        Options {
            rename: true,
            ..Options::default()
        },
    )
    .1;
    assert!(
        report
            .notices
            .iter()
            .any(|n| n.contains("Generic IL transforms skipped")),
        "{report:#?}"
    );
    assert!(Job::new(1, 1, 0, &a, Options::default()).is_err());
}
#[test]
fn function_map_inverts_calls_and_maps_cleaned_names_back_to_originals() {
    let a = Assembly::load(PATTERNS.to_vec()).unwrap();
    let map = decompiler_core::function_map::build(&a, None).unwrap();
    assert_eq!(map.functions.len(), a.metadata().method_defs.len());
    let by: std::collections::HashMap<u32, _> =
        map.functions.iter().map(|f| (f.token, f)).collect();
    // called_by is exactly the inverse of local call edges.
    for f in &map.functions {
        for t in f.calls.iter().filter(|t| *t >> 24 == 6) {
            assert!(by[t].called_by.contains(&f.token), "{} -> {t:08X}", f.name);
        }
        for c in &f.called_by {
            assert!(by[c].calls.contains(&f.token));
        }
    }
    assert!(
        map.functions
            .iter()
            .any(|f| f.tags.contains(&"constructor"))
    );
    assert!(map.functions.iter().any(|f| f.tags.contains(&"accessor")));
    assert!(!map.externals.is_empty());
    // Type relationships power the diagram: Calculator implements ICalculator and
    // stores an EventHandler field; the interface and enum kinds are reported.
    let ty = |name: &str| map.types.iter().find(|t| t.name == name).unwrap();
    let calc = ty("Calculator");
    assert_eq!(calc.kind, "class");
    assert!(calc.interfaces.iter().any(|i| i.name == "ICalculator"));
    assert!(calc.uses.iter().any(|u| u.name == "EventHandler"));
    assert!(calc.members >= 3);
    assert!(calc.base.is_none(), "System.Object base should be filtered");
    assert_eq!(ty("ICalculator").kind, "interface");
    assert!(!map.types.iter().any(|t| t.name == "<Module>"));

    assert!(map.functions.iter().all(|f| f.original_name.is_none()));

    // On a cleaned copy each renamed method reports its obfuscated original.
    let input = hide_names(PATTERNS, &["Add", "left", "right"]);
    let mut session = Session::default();
    let id = session.load(input).unwrap().id;
    let p = session.deobfuscate_begin(id, rename_only()).unwrap();
    let mut done = p.done;
    while !done {
        done = session.deobfuscate_step(p.job).unwrap().done;
    }
    session.deobfuscate_finish(p.job).unwrap();
    let copy = session.deobfuscate_open(p.job).unwrap().id;
    let map = session.function_map(copy).unwrap();
    let renamed: Vec<_> = map
        .functions
        .iter()
        .filter(|f| f.original_name.is_some())
        .collect();
    // Add (interface + implementation) and NullableAdd, which shares the suffix.
    assert_eq!(renamed.len(), 3, "{renamed:#?}");
    for f in renamed {
        assert!(f.name.starts_with("vmethod_") || f.name.starts_with("smethod_"));
        assert!(
            f.original_name.as_deref().unwrap().contains("\\u0001"),
            "{:?}",
            f.original_name
        );
    }
    // The original assembly itself has no derived names to report.
    assert!(
        session
            .function_map(id)
            .unwrap()
            .functions
            .iter()
            .all(|f| f.original_name.is_none())
    );
}

const PROXIES: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Proxies.dll");
#[test]
fn proxy_unwrapping_redirects_forwarders_and_preserves_real_calls() {
    let proxies = Options {
        constants: false,
        branches: false,
        strings: false,
        resource_strings: false,
        control_flow: false,
        anti_debug: false,
        rename: false,
        proxies: true,
    };
    let (a, report, _) = run(PROXIES, proxies);
    assert_eq!(report.outcome, "changed", "{report:#?}");
    assert!(report.counts.proxies >= 4);
    let calls = |name: &str| -> Vec<(String, String)> {
        let t = token(&a, name);
        a.body(t)
            .unwrap()
            .unwrap()
            .instructions
            .iter()
            .filter(|i| matches!(i.name.as_str(), "call" | "callvirt"))
            .map(|i| (i.name.clone(), a.resolve(i.token().unwrap())))
            .collect()
    };
    // Pure and chained forwarders are redirected straight to the real target.
    assert_eq!(
        calls("UseStatic"),
        [(
            "call".into(),
            "int ILens.Proxies.Calls.Real(int, int)".into()
        )]
    );
    assert_eq!(
        calls("UseChain"),
        [(
            "call".into(),
            "int ILens.Proxies.Calls.Real(int, int)".into()
        )]
    );
    // The instance target keeps callvirt.
    assert_eq!(
        calls("UseInstance"),
        [(
            "callvirt".into(),
            "int ILens.Proxies.Widget.Scale(int)".into()
        )]
    );
    assert_eq!(
        calls("UseVoid"),
        [(
            "callvirt".into(),
            "void ILens.Proxies.Widget.Reset()".into()
        )]
    );
    // Non-forwarders are left untouched.
    assert_eq!(
        calls("UseNotProxy"),
        [(
            "call".into(),
            "int ILens.Proxies.Calls.NotProxy(int)".into()
        )]
    );
    assert_eq!(
        calls("UseSwapped"),
        [(
            "call".into(),
            "int ILens.Proxies.Calls.Swapped(int, int)".into()
        )]
    );
    // Every changed body still passes the strict writer verifier.
    for name in ["UseStatic", "UseChain", "UseInstance", "UseVoid"] {
        let t = token(&a, name);
        let body = a.body(t).unwrap().unwrap();
        edit::verify(&a, t, &body).unwrap();
    }
    // Forwarder definitions themselves are retained.
    assert!(
        a.nodes
            .iter()
            .any(|n| n.name == "Forward" && n.token >> 24 == 6)
    );
}
const PACKED: &[u8] = include_bytes!("../../../samples/fixtures/ILens.ArrayStrings.dll");
const PACKED_HOST: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Packed.dll");
#[test]
fn array_backed_decoders_resolve_through_the_same_size_literal_writer() {
    let (a, report, _) = run(PACKED, options());
    // A Field-RVA layout rules the general IL writer out; the literal writer carries it.
    assert!(
        report
            .notices
            .iter()
            .any(|n| n.contains("Generic IL transforms skipped")),
        "{:?}",
        report.notices
    );
    let text = |name: &str| {
        let body = a.body(token(&a, name)).unwrap().unwrap();
        body.instructions
            .iter()
            .find(|i| i.name == "ldstr")
            .map(|i| a.user_string(i.token().unwrap()).unwrap())
    };
    // Static storage copied in by InitializeArray, XOR-decoded, then UTF-8 decoded.
    assert_eq!(text("Greeting").as_deref(), Some("Hello ILens"));
    assert_eq!(text("Shifted").as_deref(), Some("Idmmn!HMdor"));
    // The same shape with a char[] initializer and Base64 text.
    assert_eq!(text("Wide").as_deref(), Some("ILens wide"));
    assert_eq!(text("Base64").as_deref(), Some("Hello ILens"));
    // A key that produces invalid UTF-8 must stop rather than emit replacements.
    assert_eq!(text("Malformed"), None);
    // A nonliteral argument and a non-interned result are both left as calls.
    assert_eq!(text("Dynamic"), None);
    assert_eq!(text("Fresh"), None);
    let reason = |name: &str| {
        report
            .methods
            .iter()
            .find(|m| m.token == token(&a, name))
            .map(|m| m.reasons.join(" "))
            .unwrap_or_default()
    };
    assert!(
        reason("Malformed").contains("UTF-8"),
        "{}",
        reason("Malformed")
    );
    assert_eq!(report.counts.strings, 4);
    // Offsets, exception regions and Field-RVA data are preserved byte for byte.
    let original = Assembly::load(PACKED.to_vec()).unwrap();
    for name in ["Greeting", "Decode", "Widen"] {
        let before = original.body(token(&original, name)).unwrap().unwrap();
        let after = a.body(token(&a, name)).unwrap().unwrap();
        assert_eq!(before.code_size, after.code_size);
        assert_eq!(before.max_stack, after.max_stack);
        assert_eq!(
            before
                .instructions
                .iter()
                .map(|i| i.offset)
                .collect::<Vec<_>>(),
            after
                .instructions
                .iter()
                .map(|i| i.offset)
                .collect::<Vec<_>>()
        );
    }
}
#[test]
fn array_and_encoding_model_rejects_unsupported_shapes() {
    let a = Assembly::load(PACKED.to_vec()).unwrap();
    // Base64 is strict: no whitespace, no missing padding, no non-zero trailing bits.
    for bad in ["SGVsbG8gSUxlbnM", "SGVsbG8 gSUxlbnM=", "SGVsbG8gSUxlbnO="] {
        let e = Evaluator::new(&a, Budget::default())
            .call(
                token(&a, "FromBase64"),
                vec![Value::String(Rc::new(bad.encode_utf16().collect()), true)],
            )
            .unwrap_err();
        assert!(e.0.contains("Base64"), "{bad}: {e}");
    }
    let Value::String(units, _) = value(&a, "Decode", vec![Value::I32(0)]) else {
        panic!()
    };
    assert_eq!(String::from_utf16(&units).unwrap(), "Hello ILens");
    // Field-RVA reads are accounted for like any other evaluation allocation.
    let e = Evaluator::new(&a, Budget::new(20_000, 16))
        .call(token(&a, "Decode"), vec![Value::I32(0)])
        .unwrap_err();
    assert!(e.0.contains("allocation"), "{e}");
}
#[test]
fn switch_dispatchers_resolve_to_direct_branches_where_the_state_is_a_literal() {
    let options = Options {
        control_flow: true,
        constants: false,
        branches: false,
        ..Options::default()
    };
    let (a, report, _) = run(INPUT, options);
    let opcodes = |name: &str| {
        a.body(token(&a, name))
            .unwrap()
            .unwrap()
            .instructions
            .iter()
            .map(|i| i.name.clone())
            .collect::<Vec<_>>()
    };
    // Each literal state transition becomes one branch, and the dispatcher it fed is
    // then unreachable and removed with it.
    let flattened = opcodes("Flattened");
    assert_eq!(report.counts.flattened, 3);
    assert!(!flattened.contains(&"switch".to_string()), "{flattened:?}");
    assert!(
        !flattened.iter().any(|o| o.starts_with("stloc")),
        "{flattened:?}"
    );
    assert!(report.counts.unreachable_instructions > 0);
    // A state computed from the argument is not resolvable, so nothing is touched.
    let computed = opcodes("ComputedState");
    assert!(computed.contains(&"switch".to_string()), "{computed:?}");
    let original = Assembly::load(INPUT.to_vec()).unwrap();
    assert_eq!(
        computed,
        original
            .body(token(&original, "ComputedState"))
            .unwrap()
            .unwrap()
            .instructions
            .iter()
            .map(|i| i.name.clone())
            .collect::<Vec<_>>()
    );
}
#[test]
fn debugger_queries_are_answered_as_if_no_debugger_is_attached() {
    let options = Options {
        anti_debug: true,
        constants: false,
        branches: false,
        ..Options::default()
    };
    let (a, report, bytes) = run(INPUT, options);
    // A same-size patch: no metadata is rebuilt and no byte moves.
    assert_eq!(bytes.len(), INPUT.len());
    assert_eq!(report.counts.debugger_checks, 2);
    let original = Assembly::load(INPUT.to_vec()).unwrap();
    let before = original.body(token(&original, "Guarded")).unwrap().unwrap();
    let after = a.body(token(&a, "Guarded")).unwrap().unwrap();
    assert_eq!(before.code_size, after.code_size);
    // Every original instruction boundary survives, so branch targets still resolve.
    let kept: Vec<_> = after.instructions.iter().map(|i| i.offset).collect();
    assert!(before.instructions.iter().all(|i| kept.contains(&i.offset)));
    // IsAttached reads false and Break() is gone; nothing else about the method moved.
    let ops: Vec<_> = after.instructions.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        ops,
        [
            "nop",
            "nop",
            "nop",
            "nop",
            "ldc.i4.0",
            "brfalse.s",
            "ldc.i4.m1",
            "ret",
            "nop",
            "nop",
            "nop",
            "nop",
            "nop",
            "ldarg.0",
            "ret"
        ]
    );
    // Running the constant and branch passes afterwards folds the guard away entirely.
    let (folded, _, _) = run(
        &bytes,
        Options {
            constants: true,
            branches: true,
            ..Options::default()
        },
    );
    let ops: Vec<_> = folded
        .body(token(&folded, "Guarded"))
        .unwrap()
        .unwrap()
        .instructions
        .iter()
        .map(|i| i.name.clone())
        .collect();
    // The guarded `return -1` is unreachable once the query reads false, so it goes.
    assert!(!ops.iter().any(|o| o == "ldc.i4.m1"), "{ops:?}");
    assert!(
        ops.ends_with(&["ldarg.0".to_string(), "ret".to_string()]),
        "{ops:?}"
    );
}
#[test]
fn string_usage_lists_loading_methods_across_the_workspace() {
    let mut session = Session::default();
    let id = session.load(INPUT.to_vec()).unwrap().id;
    let a = session.assembly(id).unwrap();
    // The literal `ExternalCall` loads; its own module's sites are navigable.
    let body = a.body(token(a, "ExternalCall")).unwrap().unwrap();
    let literal = body
        .instructions
        .iter()
        .find(|i| i.name == "ldstr")
        .unwrap()
        .token()
        .unwrap();
    let usages = session.string_usages(id, literal).unwrap();
    assert_eq!(usages.len(), 1, "{usages:?}");
    assert_eq!(
        usages[0].0,
        Some(token(session.assembly(id).unwrap(), "ExternalCall"))
    );
    // Another assembly that uses the same literal is matched by text, not by token.
    let other = session.load(PACKED_HOST.to_vec()).unwrap().id;
    let usages = session.string_usages(id, literal).unwrap();
    assert_eq!(usages.len(), 2, "{usages:?}");
    assert_eq!(usages[1].0, None);
    assert!(usages[1].1.contains("ILens.Packed · "), "{usages:?}");
    assert!(usages[1].1.contains("Shared"), "{usages:?}");
    session.close(other);
    assert_eq!(session.string_usages(id, literal).unwrap().len(), 1);
}
