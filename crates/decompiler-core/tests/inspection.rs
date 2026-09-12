use decompiler_core::assembly::Assembly;

#[test]
fn framework_is_read_from_the_compiled_attribute_not_the_clr_version() {
    let a = Assembly::load(include_bytes!("../../../samples/fixtures/ILens.Patterns.dll").to_vec())
        .unwrap();
    assert_eq!(a.info.runtime, "v4.0.30319");
    assert_eq!(a.info.target_framework.display_name, ".NET 10.0");
    assert_eq!(
        a.info.target_framework.moniker.as_deref(),
        Some(".NETCoreApp,Version=v10.0")
    );
    assert_eq!(a.info.target_framework.status, "declared");
    assert_eq!(a.info.obfuscation.status, "no_markers");
    assert!(a.info.diagnostics.is_empty());
}

#[test]
fn applied_watermarks_are_resolved_through_metadata_constructors() {
    let a = Assembly::load(
        include_bytes!("../../../samples/fixtures/ILens.InspectionMarkers.dll").to_vec(),
    )
    .unwrap();
    assert_eq!(a.info.obfuscation.status, "markers_found");
    for tool in [
        "ConfuserEx",
        "Dotfuscator",
        "SmartAssembly",
        "Babel Obfuscator",
    ] {
        let evidence = a
            .info
            .obfuscation
            .evidence
            .iter()
            .find(|e| e.tool == Some(tool))
            .expect(tool);
        assert_eq!(evidence.confidence, "marker");
        let token = evidence.token.unwrap();
        assert_eq!(token >> 24, 12);
        assert!(
            a.inspect(token).unwrap()["attribute"]["constructor"]
                .as_str()
                .unwrap()
                .contains("Attribute")
        );
        assert!(
            a.metadata()
                .custom_attributes
                .get((token & 0xffffff) as usize - 1)
                .is_some()
        );
    }
    assert!(a.info.obfuscation.scan_complete);
    assert_eq!(a.info.obfuscation.evidence.len(), 4);
}

#[test]
fn unused_marker_types_and_configuration_do_not_prove_obfuscation() {
    let a = Assembly::load(include_bytes!("../../../samples/fixtures/ILens.NoTarget.dll").to_vec())
        .unwrap();
    assert!(a.nodes.iter().any(|n| n.name == "ConfusedByAttribute"));
    assert_eq!(a.info.target_framework.status, "not_declared");
    assert_eq!(a.info.target_framework.version, None);
    assert_eq!(a.info.obfuscation.status, "no_markers");
    assert!(a.info.obfuscation.evidence.is_empty());
}

#[test]
fn suspicious_metadata_names_warn_without_attributing_a_tool() {
    let mut bytes = include_bytes!("../../../samples/fixtures/ILens.NoTarget.dll").to_vec();
    let offset = bytes.windows(10).position(|b| b == b"Calculate\0").unwrap();
    let replacement = "a\u{202e}bcdef".as_bytes();
    assert_eq!(replacement.len(), 9);
    bytes[offset..offset + 9].copy_from_slice(replacement);
    let a = Assembly::load(bytes).unwrap();
    assert_eq!(a.info.obfuscation.status, "possible");
    assert!(
        a.info
            .obfuscation
            .evidence
            .iter()
            .all(|e| e.confidence == "heuristic" && e.tool.is_none())
    );
}

#[test]
fn malformed_framework_attribute_does_not_prevent_analysis() {
    let mut bytes = include_bytes!("../../../samples/fixtures/ILens.Patterns.dll").to_vec();
    let offset = bytes
        .windows(b".NETCoreApp,Version=v10.0".len())
        .position(|b| b == b".NETCoreApp,Version=v10.0")
        .unwrap();
    // Damage the serialized attribute prolog without damaging the PE or heaps.
    bytes[offset - 3] = 2;
    let a = Assembly::load(bytes).unwrap();
    assert_eq!(a.info.target_framework.status, "invalid");
    assert!(!a.info.diagnostics.is_empty());
    assert!(a.nodes.iter().any(|n| n.name == "Sum"));
}
