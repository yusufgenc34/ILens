use decompiler_core::{assembly::Assembly, cil, metadata::MetadataStore};
#[test]
fn deterministic_hostile_mutations_never_panic() {
    let original = include_bytes!("../../../samples/fixtures/ILens.Patterns.dll");
    let mut state = 0x6d6f6e6fu32;
    for _ in 0..1000 {
        let mut bytes = original.to_vec();
        for _ in 0..5 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let index = state as usize % bytes.len();
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            bytes[index] = state as u8;
        }
        let result = std::panic::catch_unwind(|| {
            if let Ok(a) = Assembly::load(bytes) {
                for n in a.nodes.iter().filter(|n| n.token >> 24 == 6).take(100) {
                    let _ = a.body(n.token);
                }
            }
        });
        assert!(result.is_ok(), "parser panicked on deterministic mutation");
    }
}
#[test]
fn malicious_stream_range_and_allocation_are_rejected_before_library_parsing() {
    let original = include_bytes!("../../../samples/fixtures/ILens.Patterns.dll");
    let a = Assembly::load(original.to_vec()).unwrap();
    let md = &original[a.image.metadata.clone()];
    let mut data = md.to_vec();
    data[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(MetadataStore::parse(&data).is_err());
    let mut data = md.to_vec();
    let length = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
    let first_stream = 20 + length;
    data[first_stream..first_stream + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(MetadataStore::parse(&data).is_err());
    let mut data = md.to_vec();
    let table = a.store.streams.iter().find(|s| s.name == "#~").unwrap();
    data[table.offset + 24..table.offset + 28].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(MetadataStore::parse(&data).is_err());
}
#[test]
fn hostile_cil_switch_and_prefixes_are_rejected() {
    assert!(cil::decode(&[0x45, 255, 255, 255, 255]).is_err());
    assert!(cil::decode(&[0xfe, 0x13]).is_err());
    assert!(cil::decode(&[0x2b, 2, 0xfe, 0x13, 0x2a]).is_err());
}
