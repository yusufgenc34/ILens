//! Container mutations exercise parser boundaries; uploaded code is never run.
use decompiler_core::{assembly::Assembly, decompile, edit, error::ErrorCode, pe_write};
const EXE: &[u8] = include_bytes!("../../../samples/fixtures/ILens.AutomaticDependencies.exe");
fn u32_at(bytes: &[u8], p: usize) -> usize {
    u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap()) as usize
}
fn r2r() -> (Vec<u8>, usize) {
    let original = Assembly::load(EXE.to_vec()).unwrap();
    let pe = u32_at(EXE, 0x3c);
    let cli_rva = u32_at(EXE, pe + 24 + 96 + 14 * 8);
    let cli = original.image.range(EXE, cli_rva as u32, 72).unwrap().start;
    let mut bytes = EXE.to_vec();
    // A synthetic R2R header in unused DOS stub space, retaining all genuine CIL.
    bytes[0x40..0x60].fill(0);
    bytes[0x40..0x44].copy_from_slice(b"RTR\0");
    bytes[0x44..0x46].copy_from_slice(&16u16.to_le_bytes());
    bytes[0x48..0x4c].copy_from_slice(&2u32.to_le_bytes()); // SkipTypeValidation, not composite
    bytes[cli + 16..cli + 20].copy_from_slice(&4u32.to_le_bytes()); // IL_LIBRARY, not IL_ONLY
    bytes[cli + 64..cli + 68].copy_from_slice(&0x40u32.to_le_bytes());
    bytes[cli + 68..cli + 72].copy_from_slice(&16u32.to_le_bytes());
    (bytes, cli)
}
#[test]
fn ready_to_run_retains_original_il_but_is_read_only() {
    let original = Assembly::load(EXE.to_vec()).unwrap();
    let (bytes, _) = r2r();
    let assembly = Assembly::load(bytes).unwrap();
    assert!(assembly.info.ready_to_run);
    assert!(
        assembly
            .info
            .diagnostics
            .iter()
            .any(|s| s.contains("ReadyToRun"))
    );
    let token = assembly.image.entry_point;
    assert_eq!(
        decompile::decompile(&assembly, token).unwrap().il,
        decompile::decompile(&original, token).unwrap().il
    );
    assert_eq!(
        edit::supported_body(&assembly, token).unwrap_err().code,
        ErrorCode::UnsupportedEdit
    );
    assert_eq!(
        pe_write::capability(&assembly).unwrap_err().code,
        ErrorCode::UnsupportedEdit
    );
}
#[test]
fn native_headers_remain_bounded_and_unknown_formats_are_rejected() {
    let (base, cli) = r2r();
    for (offset, value) in [(cli + 64, u32::MAX), (cli + 68, u32::MAX), (0x4c, 4097)] {
        let mut bytes = base.clone();
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(Assembly::load(bytes).is_err());
    }
    let mut bytes = base.clone();
    bytes[0x40] = 0;
    assert_eq!(
        Assembly::load(bytes).err().unwrap().code,
        ErrorCode::UnsupportedNative
    );
    let mut bytes = base;
    bytes[0x4c..0x50].copy_from_slice(&1u32.to_le_bytes());
    bytes[cli + 68..cli + 72].copy_from_slice(&28u32.to_le_bytes());
    bytes[0x50..0x54].copy_from_slice(&115u32.to_le_bytes());
    assert!(
        Assembly::load(bytes)
            .err()
            .unwrap()
            .detail
            .contains("Composite")
    );
}
#[test]
fn tree_labels_disambiguate_overloads_and_preserve_accessor_ownership() {
    let a = Assembly::load(include_bytes!("../../../samples/fixtures/ILens.Patterns.dll").to_vec())
        .unwrap();
    assert_eq!(a.info.module_name, "ILens.Patterns.dll");
    let property = a.nodes.iter().find(|n| n.kind == "property").unwrap();
    let accessor = a
        .nodes
        .iter()
        .find(|n| n.accessor_of == Some(property.token))
        .unwrap();
    assert_eq!(accessor.parent, property.parent); // Declaring type unchanged for analysis/export.
    assert!(accessor.display_name.contains('('));
    let method = a.nodes.iter().find(|n| n.name == "Sum").unwrap();
    assert!(
        method.display_name.starts_with("Sum(int) : int"),
        "{}",
        method.display_name
    );
}

#[test]
fn understated_directory_count_recovers_only_the_bounded_cli_slot_read_only() {
    let original = Assembly::load(EXE.to_vec()).unwrap();
    let pe = u32_at(EXE, 60);
    for count in [0u32, 11, 14] {
        let mut bytes = EXE.to_vec();
        bytes[pe + 24 + 92..pe + 24 + 96].copy_from_slice(&count.to_le_bytes());
        let a = Assembly::load(bytes).unwrap();
        assert!(a.image.recovered_cli_directory);
        assert_eq!(a.info.identity, original.info.identity);
        let token = a.image.entry_point;
        assert_eq!(
            decompile::decompile(&a, token).unwrap().il,
            decompile::decompile(&original, token).unwrap().il
        );
        assert!(
            a.info
                .diagnostics
                .iter()
                .any(|d| d.contains("directory count omits"))
        );
        assert_eq!(
            edit::supported_body(&a, token).unwrap_err().code,
            ErrorCode::UnsupportedEdit
        );
        assert_eq!(
            pe_write::capability(&a).unwrap_err().code,
            ErrorCode::UnsupportedEdit
        );
    }
    assert!(!original.image.recovered_cli_directory);
}
#[test]
fn undeclared_cli_recovery_rejects_truncated_slots_invalid_ranges_and_metadata() {
    let pe = u32_at(EXE, 60);
    let mut base = EXE.to_vec();
    base[pe + 24 + 92..pe + 24 + 96].copy_from_slice(&11u32.to_le_bytes());
    let slot = pe + 24 + 96 + 14 * 8;
    for (offset, value) in [(slot, u32::MAX), (slot + 4, u32::MAX), (slot + 4, 1)] {
        let mut bytes = base.clone();
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(Assembly::load(bytes).is_err());
    }
    let mut bytes = base.clone();
    bytes[pe + 20..pe + 22].copy_from_slice(&200u16.to_le_bytes());
    assert!(Assembly::load(bytes).is_err());
    let original = Assembly::load(EXE.to_vec()).unwrap();
    let mut bytes = base;
    bytes[original.image.metadata.start] = 0;
    assert_eq!(
        Assembly::load(bytes).err().unwrap().code,
        ErrorCode::InvalidClr
    );
}
#[test]
fn native_markers_are_passive_diagnostics_and_do_not_override_valid_managed_input() {
    let mut bytes = EXE.to_vec();
    let pe = u32_at(&bytes, 60);
    let slot = pe + 24 + 96 + 14 * 8;
    let markers = b"System.SysUtils\0Vcl.Forms\0TForm\0";
    bytes[0x40..0x40 + markers.len()].copy_from_slice(markers);
    assert!(Assembly::load(bytes.clone()).is_ok());
    bytes[slot..slot + 8].fill(0);
    let error = Assembly::load(bytes.clone()).err().unwrap();
    assert_eq!(error.code, ErrorCode::UnsupportedNative);
    assert!(error.message.contains("VCL markers"));
    assert!(error.detail.contains("heuristic"));
    bytes[0x40] = 0; // A single generic class name is not compiler attribution.
    assert_eq!(
        Assembly::load(bytes).err().unwrap().code,
        ErrorCode::NotManaged
    );
}
