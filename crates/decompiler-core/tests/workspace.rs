use decompiler_core::{assembly::Assembly, cil_encode, edit, export, resources, session::Session};
const MAIN: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Workspace.dll");
const DEP: &[u8] = include_bytes!("../../../samples/fixtures/ILens.WorkspaceDependency.dll");
fn token(a: &Assembly, name: &str, table: u32) -> u32 {
    a.nodes
        .iter()
        .find(|n| n.name == name && n.token >> 24 == table)
        .unwrap()
        .token
}
fn loaded() -> (Session, u32, u32) {
    let mut s = Session::default();
    let main = s.load(MAIN.to_vec()).unwrap().id;
    let dep = s.load(DEP.to_vec()).unwrap().id;
    (s, main, dep)
}

#[test]
fn dependencies_reconstruct_proven_properties_and_invalidate_cached_output() {
    let mut s = Session::default();
    let id = s.load(MAIN.to_vec()).unwrap().id;
    let method = token(s.assembly(id).unwrap(), "DependencyProperty", 6);
    assert!(
        s.decompile(id, method)
            .unwrap()
            .csharp
            .contains("get_Text()")
    );
    let expected = s
        .assembly(id)
        .unwrap()
        .references
        .iter()
        .find(|r| r.identity.name == "ILens.WorkspaceDependency")
        .unwrap()
        .identity
        .clone();
    let mut wrong = expected.clone();
    wrong.version = "99.0.0.0".into();
    assert!(s.load_dependency(DEP.to_vec(), &wrong).unwrap().is_none());
    let dep = s
        .load_dependency(DEP.to_vec(), &expected)
        .unwrap()
        .unwrap()
        .id;
    let output = s.decompile(id, method).unwrap();
    assert!(
        output.csharp.contains("widget.Text = value"),
        "{}",
        output.csharp
    );
    assert!(output.csharp.contains("return widget.Text"));
    assert!(!output.csharp.contains("get_Text()"));
    let exported =
        export::member_with(s.assembly(id).unwrap(), method, &|t| s.accessor(id, t)).unwrap();
    assert_eq!(exported.body, output.source_body);
    let normal = token(s.assembly(id).unwrap(), "OrdinaryGetterName", 6);
    assert!(
        s.decompile(id, normal)
            .unwrap()
            .csharp
            .contains("get_NotAProperty()")
    );
    assert_eq!(s.load(DEP.to_vec()).unwrap().id, dep);
    let mut other = DEP.to_vec();
    *other.last_mut().unwrap() ^= 1;
    assert!(s.load(other).is_err());
    s.close(dep);
    assert!(
        s.decompile(id, method)
            .unwrap()
            .csharp
            .contains("get_Text()")
    );
}
#[test]
fn source_references_identify_utf16_ranges_from_emission() {
    let (mut s, id, _) = loaded();
    for name in ["UnicodeBeforeCall", "DependencyProperty", "ReadField"] {
        let method = token(s.assembly(id).unwrap(), name, 6);
        let output = s.decompile(id, method).unwrap();
        assert!(!output.csharp.contains('\0'));
        assert!(!output.references.is_empty(), "{name}");
        let units: Vec<_> = output.csharp.encode_utf16().collect();
        for reference in output.references {
            let text = String::from_utf16(&units[reference.start..reference.end]).unwrap();
            assert!(!text.is_empty() && !text.contains('\n'), "{text}");
            s.assembly(id)
                .unwrap()
                .validate_token(reference.token)
                .unwrap();
            if text == "Text" {
                assert!(s.accessor(id, reference.token).is_some());
            }
        }
    }
}
#[test]
fn analyzer_finds_cross_assembly_reads_writes_and_inheritance() {
    let (s, id, dep) = loaded();
    for (name, table, expected) in [
        ("Number", 4, "reads"),
        ("Number", 4, "writes"),
        ("Widget", 2, "extends"),
        ("IWidget", 2, "implements"),
        ("get_Text", 6, "calls"),
    ] {
        let a = s.assembly(dep).unwrap();
        let target = if name == "get_Text" {
            let owner = token(a, "Widget", 2);
            a.nodes
                .iter()
                .find(|n| n.name == name && a.owners.get(&n.token) == Some(&owner))
                .unwrap()
                .token
        } else {
            token(a, name, table)
        };
        let mut cursor = 0;
        let mut hits = vec![];
        loop {
            let page = s.analyze_page(dep, target, id, cursor, false).unwrap();
            cursor = page.next;
            hits.extend(page.hits);
            if page.done {
                break;
            }
        }
        assert!(
            hits.iter().any(|h| h.relation == expected),
            "{name}: {expected}"
        );
        assert!(hits.iter().all(|h| h.assembly == id));
    }
}
#[test]
fn resource_table_reads_actual_compiled_strings_without_executing_markup() {
    let a = Assembly::load(MAIN.to_vec()).unwrap();
    let resources = a.resources().unwrap();
    let row = resources
        .iter()
        .find(|r| {
            r["name"]
                .as_str()
                .unwrap()
                .ends_with("Inspection.resources")
        })
        .unwrap();
    let data = export::resource(&a, row["token"].as_u64().unwrap() as u32).unwrap();
    let preview = resources::preview(&data).unwrap();
    assert_eq!(preview.kind, "resources");
    let item = preview
        .entries
        .iter()
        .find(|e| e.name == "LiteralMarkup")
        .unwrap();
    assert_eq!(
        resources::entry_bytes(&data, item.index).unwrap(),
        b"<script>never executed</script>"
    );
    for end in 4..data.len() {
        let result = std::panic::catch_unwind(|| resources::preview(&data[..end]));
        assert!(result.is_ok());
        assert!(
            result.unwrap().is_err(),
            "Truncated resource accepted at {end}"
        );
    }
}
#[test]
fn primitive_resource_values_have_checked_offsets_and_exact_integer_precision() {
    // A minimal standard v2 file with one UInt64 value, not a serialized object.
    let mut data = vec![];
    for value in [0xbeefcaceu32, 1, 0, 2, 1, 0] {
        data.extend(value.to_le_bytes());
    }
    data.extend(0u32.to_le_bytes());
    data.extend(0u32.to_le_bytes()); // hash and name position
    data.extend(43u32.to_le_bytes()); // data section: 36-byte header + 7-byte name record
    data.extend([2, b'N', 0]);
    data.extend(0u32.to_le_bytes());
    data.push(11);
    data.extend(u64::MAX.to_le_bytes());
    let preview = resources::preview(&data).unwrap();
    assert_eq!(
        preview.entries[0].value.as_deref(),
        Some("18446744073709551615")
    );
    data[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(resources::preview(&data).is_err());
}
#[test]
fn enum_edits_preserve_storage_width_and_roundtrip_modified_bodies() {
    let a = Assembly::load(MAIN.to_vec()).unwrap();
    let mut edits = std::collections::BTreeMap::new();
    for name in [
        "ChooseMode",
        "ReadMode",
        "StoreMode",
        "BoxMode",
        "UnboxMode",
        "ReadLongMode",
        "StoreLongMode",
    ] {
        let t = token(&a, name, 6);
        let body = a.body(t).unwrap().unwrap();
        let mut rows = cil_encode::from_body(&body);
        rows.insert(
            0,
            cil_encode::EditableInstruction {
                id: "inserted".into(),
                opcode: "nop".into(),
                operand: String::new(),
            },
        );
        edits.insert(
            t,
            edit::validate(&a, t, &rows).unwrap_or_else(|e| panic!("{name}: {e}")),
        );
    }
    let modified = decompiler_core::pe_write::write(&a, &edits).unwrap();
    let reopened = Assembly::load(modified).unwrap();
    for t in edits.keys() {
        assert_eq!(
            reopened.body(*t).unwrap().unwrap().instructions[0].name,
            "nop"
        );
    }
    let t = token(&a, "StoreMode", 6);
    let mut rows = cil_encode::from_body(&a.body(t).unwrap().unwrap());
    rows.iter_mut()
        .find(|r| r.opcode == "stelem.i1")
        .unwrap()
        .opcode = "stelem.i8".into();
    assert!(
        edit::validate(&a, t, &rows)
            .unwrap_err()
            .detail
            .contains("element type")
    );
}

#[test]
fn reference_assemblies_show_declarations_not_placeholder_implementations() {
    let a = Assembly::load(
        include_bytes!("../../../samples/fixtures/ILens.WorkspaceDependency.ref.dll").to_vec(),
    )
    .unwrap();
    assert!(a.info.reference_assembly);
    assert!(
        !Assembly::load(DEP.to_vec())
            .unwrap()
            .info
            .reference_assembly
    );
    let t = token(&a, "get_Text", 6);
    let output = decompiler_core::decompile::decompile(&a, t).unwrap();
    assert_eq!(output.quality, "declaration");
    assert!(output.source_body.is_none());
    assert!(output.csharp.contains("Reference assembly"));
    assert!(!output.csharp.contains("throw null"));
    assert!(edit::validate(&a, t, &[]).is_err());
}
#[test]
fn unresolved_member_metadata_identifies_its_explicit_assembly_scope() {
    let mut s = Session::default();
    let id = s.load(MAIN.to_vec()).unwrap().id;
    let a = s.assembly(id).unwrap();
    let method = token(a, "DependencyProperty", 6);
    let call = a
        .body(method)
        .unwrap()
        .unwrap()
        .instructions
        .into_iter()
        .find_map(|i| {
            if let decompiler_core::cil::Operand::Token(t) = i.operand {
                s.required_reference(id, t)
                    .filter(|r| r.identity.name == "ILens.WorkspaceDependency")
                    .map(|_| t)
            } else {
                None
            }
        })
        .unwrap();
    let metadata = s.metadata(id, call).unwrap();
    assert_eq!(
        metadata["required_reference"]["identity"]["name"],
        "ILens.WorkspaceDependency"
    );
    assert!(metadata["resolved_definition"].is_null());
    s.load(DEP.to_vec()).unwrap();
    assert!(!s.metadata(id, call).unwrap()["resolved_definition"].is_null());
    assert!(s.required_reference(id, 0).is_none());
    assert!(s.required_reference(id, 0x01ffffff).is_none());
}

#[test]
fn known_reference_pack_provenance_also_blocks_unmarked_stub_reconstruction() {
    let mut s = Session::default();
    let identity = Assembly::load(DEP.to_vec()).unwrap().info.identity;
    let loaded = s
        .load_reference_dependency(DEP.to_vec(), &identity)
        .unwrap()
        .unwrap();
    assert!(loaded.info.reference_assembly);
    let t = token(s.assembly(loaded.id).unwrap(), "get_NotAProperty", 6);
    assert!(s.decompile(loaded.id, t).unwrap().source_body.is_none());
    // Reopening an already loaded implementation does not reclassify it.
    let mut s = Session::default();
    s.load(DEP.to_vec()).unwrap();
    assert!(
        !s.load_reference_dependency(DEP.to_vec(), &identity)
            .unwrap()
            .unwrap()
            .info
            .reference_assembly
    );
}

#[test]
fn automatic_dependency_fixture_is_a_managed_executable() {
    let a = Assembly::load(
        include_bytes!("../../../samples/fixtures/ILens.AutomaticDependencies.exe").to_vec(),
    )
    .unwrap();
    assert_eq!(a.image.entry_point >> 24, 6);
    assert_eq!(
        a.nodes
            .iter()
            .find(|n| n.token == a.image.entry_point)
            .unwrap()
            .name,
        "Main"
    );
    for name in ["System.Runtime", "System.Collections", "System.Console"] {
        assert!(a.references.iter().any(|r| r.identity.name == name));
    }
}
