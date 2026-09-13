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
    assert!(m.body.unwrap().contains("return (value * 2)"));
    assert!(m.diagnostics.is_empty());
    let p = Assembly::load(PAT.to_vec()).unwrap();
    let plan = export::plan(&p, None).unwrap();
    let count: usize = plan.iter().map(|t| t.members.len()).sum();
    assert_eq!(count, p.nodes.iter().filter(|n| n.token >> 24 != 2).count());
    let asynchronous = export::member(&p, method(&p, "Later")).unwrap();
    assert_eq!(asynchronous.quality, "annotated_il");
    assert!(asynchronous.body.is_none());
    assert!(!asynchronous.il.is_empty());
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
            .apply_edit(loaded.id, token, original.revision, rows)
            .is_err()
    );
    let mut rows = original.instructions.clone();
    rows.iter_mut()
        .find(|r| r.opcode == "ldc.i4.2")
        .unwrap()
        .opcode = "ldc.i4.3".into();
    let current = session
        .apply_edit(loaded.id, token, original.revision, rows)
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
fn verifies_loops_and_blocks_exception_methods() {
    let a = Assembly::load(PAT.to_vec()).unwrap();
    let t = method(&a, "Sum");
    let rows = cil_encode::from_body(&a.body(t).unwrap().unwrap());
    assert!(edit::validate(&a, t, &rows).is_ok());
    assert!(edit::supported_body(&a, method(&a, "SafeDivide")).is_err());
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
    assert!(
        source.declaration.contains(": base()"),
        "{} {:?}",
        source.declaration,
        source.diagnostics
    );
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
