use decompiler_core::{
    assembly::Assembly,
    cil_encode::{self, EditableInstruction},
    edit, export, pe_write,
    session::Session,
};
use std::collections::BTreeMap;
const DEP: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Dependency.dll");
const PAT: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Patterns.dll");
fn method(a: &Assembly, name: &str) -> u32 {
    a.nodes
        .iter()
        .find(|n| n.name == name && n.token >> 24 == 6)
        .unwrap()
        .token
}
#[test]
fn emits_complete_static_type_and_all_pattern_members() {
    let a = Assembly::load(DEP.to_vec()).unwrap();
    let plan = export::plan(&a, None).unwrap();
    let t = plan.iter().find(|t| t.name == "Arithmetic").unwrap();
    assert!(t.declaration.contains("public static class Arithmetic"));
    let m = export::member(&a, method(&a, "Twice")).unwrap();
    assert!(m.body.unwrap().contains("return value * 2;"));
    assert!(m.diagnostics.is_empty());
    let p = Assembly::load(PAT.to_vec()).unwrap();
    let plan = export::plan(&p, None).unwrap();
    let count: usize = plan.iter().map(|t| t.members.len()).sum();
    // Closure classes and state machines are hidden; every other member is planned.
    let hidden: Vec<u32> = p
        .nodes
        .iter()
        .filter(|n| {
            n.token >> 24 == 2
                && n.parent >> 24 == 2
                && n.name.starts_with('<')
                && decompiler_core::attributes::compiler_generated(&p, n.token)
        })
        .map(|n| n.token)
        .collect();
    assert_eq!(hidden.len(), 3);
    assert!(plan.iter().all(|t| !hidden.contains(&t.token)));
    assert_eq!(
        count,
        p.nodes
            .iter()
            .filter(|n| n.token >> 24 != 2 && !hidden.contains(&n.parent))
            .count()
    );
    // async and iterator bodies are rebuilt from their state machines.
    let asynchronous = export::member(&p, method(&p, "Later")).unwrap();
    assert_eq!(asynchronous.quality, "csharp_like");
    assert!(asynchronous.declaration.contains("public static async "));
    assert_eq!(
        asynchronous.body.as_deref(),
        Some("{\n    await System.Threading.Tasks.Task.Yield();\n    return value + 1;\n}\n")
    );
    assert!(!asynchronous.il.is_empty());
    let iterator = export::member(&p, method(&p, "Range")).unwrap();
    assert_eq!(
        iterator.body.as_deref(),
        Some("{\n    for (int i = 0; i < n; i++)\n    {\n        yield return i;\n    }\n}\n")
    );
}
#[test]
fn validates_and_reopens_changed_binary() {
    let a = Assembly::load(DEP.to_vec()).unwrap();
    let token = method(&a, "Twice");
    let mut rows = cil_encode::from_body(&a.body(token).unwrap().unwrap());
    rows.iter_mut()
        .find(|r| r.opcode == "ldc.i4.2")
        .unwrap()
        .opcode = "ldc.i4.3".into();
    let valid = edit::validate(&a, token, &rows).unwrap();
    let output = pe_write::write(&a, &BTreeMap::from([(token, valid)])).unwrap();
    let b = Assembly::load(output.clone()).unwrap();
    assert_eq!(a.info.identity, b.info.identity);
    assert!(
        b.body(token)
            .unwrap()
            .unwrap()
            .instructions
            .iter()
            .any(|i| i.name == "ldc.i4.3")
    );
    assert!(goblin::pe::PE::parse(&output).is_ok());
    assert_eq!(pe_write::write(&a, &BTreeMap::new()).unwrap(), DEP);
}
#[test]
fn rejects_bad_stack_types_tokens_and_stale_edits() {
    let mut session = Session::default();
    let loaded = session.load(DEP.to_vec()).unwrap();
    let token = method(session.assembly(loaded.id).unwrap(), "Twice");
    let original = session.open_edit(loaded.id, token).unwrap();
    let mut rows = original.instructions.clone();
    rows.iter_mut()
        .find(|r| r.opcode == "ldc.i4.2")
        .unwrap()
        .opcode = "ldnull".into();
    assert!(
        session
            .apply_edit(loaded.id, token, original.revision, rows, None, None)
            .is_err()
    );
    let mut rows = original.instructions.clone();
    rows.iter_mut()
        .find(|r| r.opcode == "ldc.i4.2")
        .unwrap()
        .opcode = "ldc.i4.3".into();
    let current = session
        .apply_edit(loaded.id, token, original.revision, rows, None, None)
        .unwrap();
    assert!(current.changed);
    assert!(
        session
            .export_modified(loaded.id, original.revision)
            .is_err()
    );
    assert!(
        !session
            .export_modified(loaded.id, current.revision)
            .unwrap()
            .is_empty()
    );
    let cleared = session
        .discard_edit(loaded.id, token, current.revision)
        .unwrap();
    assert!(!cleared.changed);
    assert_eq!(
        session
            .export_modified(loaded.id, cleared.revision)
            .unwrap(),
        DEP
    );
}
#[test]
fn verifies_loops_and_exception_methods() {
    let a = Assembly::load(PAT.to_vec()).unwrap();
    let t = method(&a, "Sum");
    let rows = cil_encode::from_body(&a.body(t).unwrap().unwrap());
    assert!(edit::validate(&a, t, &rows).is_ok());
    // try/catch methods are editable; their clauses keep their labels.
    let safe = method(&a, "SafeDivide");
    let rows = cil_encode::from_body(&a.body(safe).unwrap().unwrap());
    let edit = edit::validate(&a, safe, &rows).unwrap();
    assert_eq!(edit.regions.len(), 1);
    assert_eq!(edit.regions[0].kind, "catch");
    let t = method(&a, "Multiply");
    let rows = vec![
        EditableInstruction {
            id: "a".into(),
            opcode: "ldc.i8".into(),
            operand: "1".into(),
        },
        EditableInstruction {
            id: "b".into(),
            opcode: "ret".into(),
            operand: String::new(),
        },
    ];
    assert!(edit::validate(&a, t, &rows).is_err());
}
#[test]
fn growing_methods_use_a_checked_section_layout() {
    let a = Assembly::load(DEP.to_vec()).unwrap();
    let token = method(&a, "Twice");
    let mut rows = cil_encode::from_body(&a.body(token).unwrap().unwrap());
    for i in 0..100 {
        rows.insert(
            0,
            EditableInstruction {
                id: format!("pad{i}"),
                opcode: "nop".into(),
                operand: String::new(),
            },
        );
    }
    let edit = edit::validate(&a, token, &rows).unwrap();
    let output = pe_write::write(&a, &BTreeMap::from([(token, edit)])).unwrap();
    let parsed = Assembly::load(output).unwrap();
    assert!(parsed.body(token).unwrap().unwrap().code_size > 100);
}

#[test]
fn resource_extraction_checks_payload_length() {
    let mut a = Assembly::load(PAT.to_vec()).unwrap();
    let resource = &a.metadata().manifest_resources[0];
    let offset = resource.offset as usize;
    let range = a
        .image
        .range(
            &a.bytes,
            a.image.resources_rva,
            a.image.resources_size as usize,
        )
        .unwrap();
    let payload = export::resource(&a, 0x28000001).unwrap();
    assert_eq!(
        payload,
        include_bytes!("../../../samples/Patterns/message.txt")
    );
    a.bytes[range.start + offset..range.start + offset + 4]
        .copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(export::resource(&a, 0x28000001).is_err());
}
#[test]
fn writer_removes_debug_payload_bytes_and_blocks_signed_inputs() {
    use decompiler_core::reader::u32_at;
    let mut bytes = DEP.to_vec();
    let initial = Assembly::load(bytes.clone()).unwrap();
    let pe = u32_at(&bytes, 0x3c).unwrap() as usize;
    let directories = pe + 24 + 96;
    let debug = initial
        .image
        .range(&bytes, u32_at(&bytes, directories + 48).unwrap(), 28)
        .unwrap();
    let section = &initial.image.sections[0];
    let pointer = (section.pointer_to_raw_data + section.size_of_raw_data - 80) as usize;
    assert!(bytes[pointer..pointer + 80].iter().all(|b| *b == 0));
    let path = b"RSDS00000000000000000000/build/neutral/fixture.pdb\0";
    bytes[pointer..pointer + path.len()].copy_from_slice(path);
    for (offset, value) in [
        (12, 2),
        (16, path.len() as u32),
        (
            20,
            section.virtual_address + pointer as u32 - section.pointer_to_raw_data,
        ),
        (24, pointer as u32),
    ] {
        bytes[debug.start + offset..debug.start + offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let a = Assembly::load(bytes.clone()).unwrap();
    let token = method(&a, "Twice");
    let mut rows = cil_encode::from_body(&a.body(token).unwrap().unwrap());
    rows[1].opcode = "ldc.i4.3".into();
    let output = pe_write::write(
        &a,
        &BTreeMap::from([(token, edit::validate(&a, token, &rows).unwrap())]),
    )
    .unwrap();
    assert!(
        output[pointer..pointer + path.len()]
            .iter()
            .all(|b| *b == 0)
    );
    assert_eq!(u32_at(&output, directories + 48).unwrap(), 0);
    let cli = a
        .image
        .range(&bytes, u32_at(&bytes, directories + 112).unwrap(), 72)
        .unwrap();
    let flags = u32_at(&bytes, cli.start + 16).unwrap() | 8;
    bytes[cli.start + 16..cli.start + 20].copy_from_slice(&flags.to_le_bytes());
    assert!(pe_write::capability(&Assembly::load(bytes).unwrap()).is_err());
}
#[test]
fn constructor_initialization_is_emitted_from_ast_and_resources_survive_writing() {
    let a = Assembly::load(PAT.to_vec()).unwrap();
    let owner = a
        .nodes
        .iter()
        .find(|n| n.name == "Calculator" && n.token >> 24 == 2)
        .unwrap()
        .token;
    let constructor = a
        .nodes
        .iter()
        .find(|n| n.name == ".ctor" && n.parent == owner)
        .unwrap()
        .token;
    let source = export::member(&a, constructor).unwrap();
    // A parameterless base call is implicit in C#, so the declaration omits it.
    assert_eq!(
        source.declaration, "public Calculator(int offset)",
        "{:?}",
        source.diagnostics
    );
    assert_eq!(source.quality, "csharp_like");
    assert!(source.body.unwrap().contains("Offset"));
    let token = method(&a, "Multiply");
    let mut rows = cil_encode::from_body(&a.body(token).unwrap().unwrap());
    rows.iter_mut().find(|r| r.opcode == "mul").unwrap().opcode = "add".into();
    let output = pe_write::write(
        &a,
        &BTreeMap::from([(token, edit::validate(&a, token, &rows).unwrap())]),
    )
    .unwrap();
    let parsed = Assembly::load(output).unwrap();
    assert_eq!(
        export::resource(&a, 0x28000001).unwrap(),
        export::resource(&parsed, 0x28000001).unwrap()
    );
    assert_eq!(a.info.method_count, parsed.info.method_count);
}
#[test]
fn enum_literals_are_source_constants_not_field_rva_limitations() {
    let a = Assembly::load(PAT.to_vec()).unwrap();
    let token = a
        .nodes
        .iter()
        .find(|n| n.name == "Thorough" && n.token >> 24 == 4)
        .unwrap()
        .token;
    let field = export::member(&a, token).unwrap();
    assert_eq!(field.declaration, "Thorough = 2,");
    assert!(field.diagnostics.is_empty());
}
#[test]
fn loading_dependencies_keeps_open_edits_current() {
    let mut session = Session::default();
    let loaded = session.load(DEP.to_vec()).unwrap();
    let token = method(session.assembly(loaded.id).unwrap(), "Twice");
    let original = session.open_edit(loaded.id, token).unwrap();
    // Automatic dependency resolution loads assemblies while an editor is open.
    session.load(PAT.to_vec()).unwrap();
    let mut rows = original.instructions.clone();
    rows.iter_mut()
        .find(|r| r.opcode == "ldc.i4.2")
        .unwrap()
        .opcode = "ldc.i4.3".into();
    let current = session
        .apply_edit(loaded.id, token, original.revision, rows, None, None)
        .unwrap();
    session.load(PAT.to_vec()).unwrap();
    assert!(
        !session
            .export_modified(loaded.id, current.revision)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn new_string_literals_grow_the_user_string_heap() {
    let a = Assembly::load(include_bytes!("../../../samples/fixtures/ILens.Editing.dll").to_vec())
        .unwrap();
    let edited = |name: &str, literal: &str| {
        let token = method(&a, name);
        let mut rows = cil_encode::from_body(&a.body(token).unwrap().unwrap());
        let first = rows
            .iter_mut()
            .find(|r| r.opcode.starts_with("ldarg"))
            .unwrap();
        first.opcode = "ldstr".into();
        first.operand = serde_json::to_string(literal).unwrap();
        (token, edit::validate(&a, token, &rows).unwrap())
    };
    let (length, edit) = edited("StringLength", "yeni metin ✓ 'quoted'");
    // New literals are shown as text and kept for the writer.
    assert_eq!(edit.strings, ["yeni metin ✓ 'quoted'"]);
    assert!(
        edit.instructions
            .iter()
            .any(|i| i.operand == "\"yeni metin ✓ 'quoted'\"")
    );
    // An existing #US entry keeps its token.
    let (_, reused) = edited("StringLength", "fixture");
    assert!(reused.strings.is_empty());
    let output = pe_write::write(&a, &BTreeMap::from([(length, edit)])).unwrap();
    let b = Assembly::load(output.clone()).unwrap();
    let body = b.body(length).unwrap().unwrap();
    let ldstr = body
        .instructions
        .iter()
        .find(|i| i.name == "ldstr")
        .unwrap();
    let decompiler_core::cil::Operand::Token(token) = ldstr.operand else {
        panic!("ldstr without token")
    };
    assert_eq!(b.user_string(token).unwrap(), "yeni metin ✓ 'quoted'");
    // Every original user string still resolves to the same text.
    for i in a.nodes.iter().filter(|n| n.token >> 24 == 6) {
        let Ok(Some(before)) = a.body(i.token) else {
            continue;
        };
        for instruction in before.instructions.iter().filter(|x| x.name == "ldstr") {
            if let decompiler_core::cil::Operand::Token(t) = instruction.operand {
                assert_eq!(a.user_string(t).unwrap(), b.user_string(t).unwrap());
            }
        }
    }
    assert!(goblin::pe::PE::parse(&output).is_ok());
    assert!(
        decompiler_core::decompile::decompile(&b, length)
            .unwrap()
            .csharp
            .contains("return \"yeni metin ✓ 'quoted'\".Length;")
    );
}
#[test]
fn metadata_tables_round_trip_byte_for_byte() {
    for file in std::fs::read_dir("../../samples/fixtures").unwrap() {
        let path = file.unwrap().path();
        if path.extension().is_none_or(|e| e != "dll") {
            continue;
        }
        let Ok(a) = Assembly::load(std::fs::read(&path).unwrap()) else {
            continue;
        };
        let metadata = &a.bytes[a.image.metadata.clone()];
        let stream = a.store.streams.iter().find(|s| s.name == "#~").unwrap();
        let tables = &metadata[stream.offset..stream.offset + stream.size];
        let decoded = decompiler_core::metadata_write::Tables::decode(tables).unwrap();
        // Compilers may pad the stream with zeros past the rows.
        let encoded = decoded.encode();
        assert_eq!(encoded, tables[..encoded.len()], "{}", path.display());
        assert!(tables[encoded.len()..].iter().all(|b| *b == 0));
        // Rebuilding with nothing replaced reproduces the metadata.
        assert_eq!(
            decompiler_core::metadata_write::rebuild(metadata, &[]).unwrap(),
            metadata,
            "{}",
            path.display()
        );
    }
}
#[test]
fn new_locals_add_a_local_signature() {
    let a = Assembly::load(include_bytes!("../../../samples/fixtures/ILens.Editing.dll").to_vec())
        .unwrap();
    let token = method(&a, "StringLength");
    assert!(
        a.locals(&a.body(token).unwrap().unwrap())
            .unwrap()
            .is_empty()
    );
    let row = |opcode: &str, operand: &str, id: &str| EditableInstruction {
        id: id.into(),
        opcode: opcode.into(),
        operand: operand.into(),
    };
    let original = cil_encode::from_body(&a.body(token).unwrap().unwrap());
    let call = original
        .iter()
        .find(|r| r.opcode == "callvirt")
        .unwrap()
        .operand
        .clone();
    let rows = vec![
        row("ldarg.0", "", "a"),
        row("stloc.0", "", "b"),
        row("ldloc.0", "", "c"),
        row("callvirt", &call, "d"),
        row("stloc.1", "", "e"),
        row("ldloc.1", "", "f"),
        row("ret", "", "g"),
    ];
    // Locals the body uses must be declared.
    assert!(edit::validate(&a, token, &rows).is_err());
    let locals = ["string".to_owned(), "int".to_owned()];
    let edit = edit::validate_with_locals(&a, token, &rows, Some(&locals)).unwrap();
    // A wrong declared type is rejected by the verifier.
    let wrong = ["int".to_owned(), "int".to_owned()];
    assert!(edit::validate_with_locals(&a, token, &rows, Some(&wrong)).is_err());
    let output = pe_write::write(&a, &BTreeMap::from([(token, edit)])).unwrap();
    let b = Assembly::load(output.clone()).unwrap();
    let body = b.body(token).unwrap().unwrap();
    assert!(body.init_locals);
    assert_eq!(
        b.locals(&body).unwrap(),
        [
            decompiler_core::signature::Type::primitive("string"),
            decompiler_core::signature::Type::primitive("int")
        ]
    );
    assert_eq!(
        b.metadata().stand_alone_sigs.len(),
        a.metadata().stand_alone_sigs.len() + 1
    );
    assert!(goblin::pe::PE::parse(&output).is_ok());
    // A second edit with the same locals reuses the new signature row on rewrite.
    let again = edit::validate_with_locals(&b, token, &rows, Some(&locals)).unwrap();
    assert!(again.locals.is_none());
}
#[test]
fn exception_regions_are_verified_and_written() {
    let a = Assembly::load(PAT.to_vec()).unwrap();
    let token = method(&a, "SafeDivide");
    let body = a.body(token).unwrap().unwrap();
    let rows = cil_encode::from_body(&body);
    let original = edit::validate(&a, token, &rows).unwrap();
    let mut regions = original.regions.clone();
    // A handler overlapping its try range is rejected.
    let mut overlapping = regions.clone();
    overlapping[0].handler_start = overlapping[0].try_start.clone();
    assert!(edit::validate_request(&a, token, &rows, None, Some(&overlapping)).is_err());
    // A finally handler starts with an empty stack, so the catch's `pop` underflows.
    regions[0].kind = "finally".into();
    regions[0].catch_type = None;
    let error = edit::validate_request(&a, token, &rows, None, Some(&regions)).unwrap_err();
    assert!(error.detail.contains("underflow"), "{}", error.detail);
    // Rewriting with an unchanged catch clause reproduces it exactly.
    let mut changed = rows.clone();
    let constant = changed.iter_mut().find(|r| r.opcode == "ldc.i4.0").unwrap();
    constant.opcode = "ldc.i4.m1".into();
    let edit = edit::validate(&a, token, &changed).unwrap();
    let output = pe_write::write(&a, &BTreeMap::from([(token, edit)])).unwrap();
    let b = Assembly::load(output).unwrap();
    let written = b.body(token).unwrap().unwrap();
    assert_eq!(written.exceptions.len(), 1);
    assert_eq!(written.exceptions[0].kind, "catch");
    assert_eq!(
        written.exceptions[0].catch_type,
        body.exceptions[0].catch_type
    );
    assert!(
        decompiler_core::decompile::decompile(&b, token)
            .unwrap()
            .csharp
            .contains("num = -1;")
    );
}
#[test]
fn constructor_bodies_track_this_initialization() {
    let a = Assembly::load(include_bytes!("../../../samples/fixtures/ILens.Editing.dll").to_vec())
        .unwrap();
    let ctor = a
        .nodes
        .iter()
        .find(|n| n.token >> 24 == 6 && n.name == ".ctor" && n.full_name.contains("Counter..ctor"))
        .unwrap()
        .token;
    let rows = cil_encode::from_body(&a.body(ctor).unwrap().unwrap());
    // The unchanged constructor verifies: own-field stores before base(), then base().
    edit::validate(&a, ctor, &rows).unwrap();
    // Returning without calling the base constructor is rejected.
    // (The base call and the receiver pushed for it are both removed.)
    let call_at = rows.iter().position(|r| r.opcode == "call").unwrap();
    let mut without_base = rows.clone();
    without_base.drain(call_at - 1..=call_at);
    let error = edit::validate(&a, ctor, &without_base).unwrap_err();
    assert!(
        error.detail.contains("base or chained constructor"),
        "{}",
        error.detail
    );
    // Changing an init-only field's value is allowed in its own constructor.
    let mut changed = rows.clone();
    changed
        .iter_mut()
        .find(|r| r.opcode == "ldc.i4.7")
        .unwrap()
        .opcode = "ldc.i4.8".into();
    let edit = edit::validate(&a, ctor, &changed).unwrap();
    let output = pe_write::write(&a, &BTreeMap::from([(ctor, edit)])).unwrap();
    let b = Assembly::load(output).unwrap();
    assert!(
        decompiler_core::decompile::decompile(&b, ctor)
            .unwrap()
            .csharp
            .contains("this.ReadOnly = 8;")
    );
    // Type initializers are ordinary static bodies.
    let cctor = a
        .nodes
        .iter()
        .find(|n| n.token >> 24 == 6 && n.name == ".cctor" && n.full_name.contains("Counter"))
        .unwrap()
        .token;
    let mut rows = cil_encode::from_body(&a.body(cctor).unwrap().unwrap());
    rows.iter_mut()
        .find(|r| r.opcode == "ldstr")
        .unwrap()
        .operand = "\"renamed\"".into();
    edit::validate(&a, cctor, &rows).unwrap();
}
