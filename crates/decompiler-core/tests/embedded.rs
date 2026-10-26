use decompiler_core::{assembly::Assembly, embedded};
const PACKED: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Packed.dll");
const PLAIN: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Example.dll");

#[test]
fn embedded_managed_payloads_are_listed_and_unparsable_ones_are_only_flagged() {
    let a = Assembly::load(PACKED.to_vec()).unwrap();
    let found = embedded::payloads(&a).unwrap();
    assert!(!found.truncated);
    assert_eq!(found.assemblies.len(), 1, "{found:#?}");
    let payload = &found.assemblies[0];
    assert_eq!(payload.name, "ILens.Packed.Payload.dll");
    assert_eq!(payload.entry, -1);
    assert_eq!(payload.token >> 24, 0x28);
    assert!(payload.identity.starts_with("ILens.Example, Version="));
    assert_eq!(payload.size, PLAIN.len());
    assert!(payload.types > 0 && payload.methods > 0);
    // A resource that only starts like a PE is reported, never guessed at or decoded.
    assert_eq!(found.unrecognized, ["ILens.Packed.Scrambled.bin"]);
}
#[test]
fn an_assembly_without_payloads_reports_none() {
    let a = Assembly::load(PLAIN.to_vec()).unwrap();
    let found = embedded::payloads(&a).unwrap();
    assert!(found.assemblies.is_empty() && found.unrecognized.is_empty() && !found.truncated);
}
