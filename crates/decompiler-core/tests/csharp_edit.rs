//! C# method editing: the skeleton the compiler sees, and merging a body compiled by
//! Roslyn (the fixture is built by `npm run fixtures` with the browser's options).
use decompiler_core::{assembly::Assembly, csharp_edit, edit, pe_write};
use std::collections::BTreeMap;
const EDITING: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Editing.dll");
const COMPILED: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Editing.csharp-edit.dll");
const EDITED_SOURCE: &str = include_str!("../../../samples/CSharpEditHarness/Increase.edit.cs");
fn increase(a: &Assembly) -> u32 {
    a.nodes
        .iter()
        .find(|n| n.token >> 24 == 6 && n.name == "Increase")
        .unwrap()
        .token
}
#[test]
fn the_skeleton_stubs_every_other_member() {
    let a = Assembly::load(EDITING.to_vec()).unwrap();
    let source = csharp_edit::source(&a, increase(&a)).unwrap();
    assert!(source.method.starts_with("public int Increase(int amount)"));
    assert!(source.before.contains("public extern Counter(int value);"));
    assert!(
        source
            .before
            .contains("public virtual int Read() => throw null;")
    );
    assert!(
        source
            .after
            .contains("public int Property { get => throw null; set => throw null; }")
    );
    // Compiler-generated backing fields are not part of the skeleton.
    assert!(!source.before.contains("BackingField"));
    // The committed fixture source is this skeleton with an edited body.
    assert!(EDITED_SOURCE.starts_with(&source.before));
    assert!(EDITED_SOURCE.ends_with(&source.after));
}
#[test]
fn a_compiled_body_merges_with_new_references_and_writes() {
    let a = Assembly::load(EDITING.to_vec()).unwrap();
    let token = increase(&a);
    let merged = csharp_edit::merge(&a, token, COMPILED).unwrap();
    // System.Console is new to the assembly: an AssemblyRef, TypeRef and MemberRef.
    let tables: Vec<usize> = merged.rows.iter().map(|r| r.table()).collect();
    assert!(
        tables.contains(&35) && tables.contains(&1) && tables.contains(&10),
        "{tables:?}"
    );
    // The string literal and the local survive the mapping.
    assert!(
        merged
            .instructions
            .iter()
            .any(|i| i.operand == "\"adding \"")
    );
    assert_eq!(merged.locals.len(), 1);
    // References are added first; the body then verifies against them.
    let with_rows = Assembly::load(pe_write::add_rows(&a, &merged.rows).unwrap()).unwrap();
    let edit = edit::validate_types(
        &with_rows,
        token,
        &merged.instructions,
        Some(merged.locals.clone()),
        Some(&merged.regions),
    )
    .unwrap();
    let output = pe_write::write(&with_rows, &BTreeMap::from([(token, edit)])).unwrap();
    let b = Assembly::load(output).unwrap();
    let source = decompiler_core::decompile::decompile(&b, token)
        .unwrap()
        .csharp;
    assert!(
        source.contains("System.Console.WriteLine(\"adding \" + "),
        "{source}"
    );
    assert!(source.contains("this.Value += "), "{source}");
}
#[test]
fn signature_changes_and_generated_types_are_refused() {
    let a = Assembly::load(EDITING.to_vec()).unwrap();
    // Merging into a method whose signature the compiled code does not contain.
    let read = a
        .nodes
        .iter()
        .find(|n| n.token >> 24 == 6 && n.name == "StringLength")
        .unwrap()
        .token;
    let error = csharp_edit::merge(&a, read, COMPILED).unwrap_err();
    assert!(
        error.detail.contains("declaring type") || error.detail.contains("signature"),
        "{}",
        error.detail
    );
}
