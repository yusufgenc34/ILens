use goblin::pe::{PE, debug::IMAGE_DEBUG_TYPE_REPRO};

const FIXTURES: &[(&str, &[u8])] = &[
    (
        "Patterns",
        include_bytes!("../../../samples/fixtures/ILens.Patterns.dll"),
    ),
    (
        "Dependency",
        include_bytes!("../../../samples/fixtures/ILens.Dependency.dll"),
    ),
    (
        "InspectionMarkers",
        include_bytes!("../../../samples/fixtures/ILens.InspectionMarkers.dll"),
    ),
    (
        "NoTarget",
        include_bytes!("../../../samples/fixtures/ILens.NoTarget.dll"),
    ),
    (
        "public sample",
        include_bytes!("../../../public/samples/ILens.Patterns.dll"),
    ),
    (
        "Editing",
        include_bytes!("../../../samples/fixtures/ILens.Editing.dll"),
    ),
    (
        "Readability",
        include_bytes!("../../../samples/fixtures/ILens.Readability.dll"),
    ),
];

#[test]
fn distributed_fixtures_do_not_contain_debug_symbols_or_host_paths() {
    for &(name, bytes) in FIXTURES {
        let pe = PE::parse(bytes).expect("valid fixture PE");
        if let Some(debug) = pe.debug_data {
            for entry in debug.entries() {
                let entry = entry.expect("valid debug directory");
                // Keep only the empty deterministic-build marker. CodeView and
                // embedded PDB records can expose paths even without a .pdb file.
                assert_eq!(
                    entry.data_type, IMAGE_DEBUG_TYPE_REPRO,
                    "{name}: debug symbols"
                );
                assert_eq!(entry.size_of_data, 0, "{name}: debug payload");
            }
        }
        for path in [
            "/Users/",
            "/home/",
            "/private/",
            "/var/folders/",
            "\\Users\\",
            "\\Documents and Settings\\",
            ".pdb",
        ] {
            for encoding in [
                path.as_bytes().to_vec(),
                path.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            ] {
                assert!(
                    !bytes
                        .windows(encoding.len())
                        .any(|part| part.eq_ignore_ascii_case(&encoding)),
                    "{name}: host path or PDB reference"
                );
            }
        }
    }
}

#[test]
fn public_sample_matches_the_checked_fixture() {
    assert_eq!(FIXTURES[0].1, FIXTURES[4].1);
}
