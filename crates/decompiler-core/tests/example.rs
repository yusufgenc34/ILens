//! The start page's example assembly must read like the source it was compiled from.
use decompiler_core::{assembly::Assembly, decompile};

fn source(name: &str) -> String {
    let a = Assembly::load(include_bytes!("../../../samples/fixtures/ILens.Example.dll").to_vec())
        .unwrap();
    let token = a.nodes.iter().find(|n| n.name == name).unwrap().token;
    let result = decompile::decompile(&a, token).unwrap();
    assert_eq!(result.quality, "csharp_like", "{:?}", result.diagnostics);
    result.csharp
}
#[test]
fn guard_clause_and_length_test_read_as_written() {
    let text = source("Average");
    let guard = text.find("if (values.Length == 0)").expect(&text);
    assert!(guard < text.find("foreach").expect(&text), "{text}");
    assert!(!text.contains("nuint"), "{text}");
    assert!(!text.contains("unchecked"), "{text}");
}
#[test]
fn enum_comparisons_use_members() {
    let text = source("Area");
    assert!(text.contains("== ILens.Example.Shape.Circle"), "{text}");
    assert!(!text.contains("shape == 0"), "{text}");
}
#[test]
fn negated_conditions_are_swapped() {
    let text = source("Greet");
    assert!(
        text.contains("return System.String.IsNullOrEmpty(name) ? "),
        "{text}"
    );
}
