use decompiler_core::{
    assembly::Assembly,
    cil_encode::{self, EditableInstruction},
    edit,
    error::ErrorCode,
    pe_write,
    signature::{self, Type},
};
use std::collections::BTreeMap;
const PAT: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Patterns.dll");
const EDIT: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Editing.dll");
fn token(a: &Assembly, name: &str) -> u32 {
    a.nodes
        .iter()
        .find(|n| {
            n.name == name && n.token >> 24 == 6 && a.method_row(n.token).is_ok_and(|r| r.rva != 0)
        })
        .unwrap()
        .token
}
fn rows(a: &Assembly, name: &str) -> Vec<EditableInstruction> {
    cil_encode::from_body(&a.body(token(a, name)).unwrap().unwrap())
}
fn instruction(id: &str, opcode: &str, operand: impl Into<String>) -> EditableInstruction {
    EditableInstruction {
        id: id.into(),
        opcode: opcode.into(),
        operand: operand.into(),
    }
}
fn rejected(a: &Assembly, name: &str, rows: &[EditableInstruction], expected: &str) {
    let error = edit::validate(a, token(a, name), rows).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidEdit, "{error}");
    assert!(error.detail.contains(expected), "{error}");
}
#[test]
fn signatures_keep_value_kind_and_vector_shape() {
    assert_eq!(
        signature::type_spec(&[0x12, 8]).unwrap(),
        Type::Named(0x02000002)
    );
    assert_eq!(
        signature::type_spec(&[0x11, 8]).unwrap(),
        Type::ValueType(0x02000002)
    );
    assert!(!signature::type_spec(&[0x11, 8]).unwrap().is_reference());
    assert_eq!(
        signature::type_spec(&[0x1d, 8]).unwrap(),
        Type::Array(Box::new(Type::primitive("int")), 1)
    );
    assert_eq!(
        signature::type_spec(&[0x14, 8, 1, 0, 0]).unwrap(),
        Type::MultiArray(Box::new(Type::primitive("int")), 1)
    );
    assert!(
        !signature::type_spec(&[0x15, 0x11, 8, 1, 8])
            .unwrap()
            .is_reference()
    );
}
#[test]
fn arrays_boxing_casts_and_instance_members_open_and_validate() {
    let a = Assembly::load(PAT.to_vec()).unwrap();
    for name in [
        "MakeArray",
        "First",
        "Each",
        "Box",
        "Unbox",
        "Cast",
        "Add",
        "Adjust",
        "get_Value",
        "set_Value",
    ] {
        edit::preview(&a, token(&a, name), 0, None).unwrap_or_else(|e| panic!("{name}: {e}"));
        edit::validate(&a, token(&a, name), &rows(&a, name))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    let a = Assembly::load(EDIT.to_vec()).unwrap();
    for name in [
        "Create",
        "Call",
        "InterfaceCall",
        "DerivedCall",
        "SetProperty",
        "StringLength",
        "ThrowException",
        "RuntimeType",
        "ToObject",
        "AsCounter",
        "CastCounter",
        "Strings",
        "Objects",
        "Jagged",
        "Counters",
        "FirstCounter",
        "Covariant",
        "Select",
        "SelectKinds",
        "NotNull",
        "Length",
        "ByteAt",
        "ShortAt",
        "LongAt",
        "FloatAt",
        "DoubleAt",
        "NativeAt",
        "StoreByte",
        "StoreShort",
        "StoreLong",
        "StoreFloat",
        "StoreDouble",
        "StoreNative",
        "Increase",
        "Rename",
    ] {
        edit::preview(&a, token(&a, name), 0, None).unwrap_or_else(|e| panic!("{name}: {e}"));
        edit::validate(&a, token(&a, name), &rows(&a, name))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}
#[test]
fn changed_array_and_object_methods_roundtrip_together() {
    for (bytes, names) in [
        (
            PAT,
            vec![
                "MakeArray",
                "First",
                "Each",
                "Box",
                "Unbox",
                "Cast",
                "Adjust",
                "get_Value",
                "set_Value",
            ],
        ),
        (
            EDIT,
            vec![
                "Create",
                "Call",
                "DerivedCall",
                "InterfaceCall",
                "SetProperty",
                "Strings",
                "Objects",
                "Jagged",
                "Select",
                "SelectKinds",
                "Increase",
                "Rename",
            ],
        ),
    ] {
        let a = Assembly::load(bytes.to_vec()).unwrap();
        let mut edits = BTreeMap::new();
        for name in names {
            let mut rows = rows(&a, name);
            if name == "MakeArray" {
                rows.iter_mut().find(|i| i.operand == "42").unwrap().operand = "43".into();
            } else {
                rows.insert(0, instruction("padding", "nop", ""));
            }
            edits.insert(
                token(&a, name),
                edit::validate(&a, token(&a, name), &rows).unwrap(),
            );
        }
        let output = pe_write::write(&a, &edits).unwrap();
        let parsed = Assembly::load(output).unwrap();
        for (token, edit) in edits {
            let body = parsed.body(token).unwrap().unwrap();
            edit::verify(&parsed, token, &body).unwrap();
            assert_eq!(
                cil_encode::encode(&cil_encode::from_body(&body)).unwrap(),
                edit.code
            );
        }
    }
}
#[test]
fn array_verifier_rejects_wrong_storage_value_index_and_token() {
    let a = Assembly::load(PAT.to_vec()).unwrap();
    let mut bad = rows(&a, "MakeArray");
    bad.iter_mut()
        .find(|r| r.opcode == "stelem.i4")
        .unwrap()
        .opcode = "stelem.ref".into();
    rejected(&a, "MakeArray", &bad, "array element type");
    let mut bad = rows(&a, "MakeArray");
    let i = bad.iter_mut().find(|r| r.operand == "42").unwrap();
    i.opcode = "ldnull".into();
    i.operand.clear();
    rejected(&a, "MakeArray", &bad, "Array element requires");
    let mut bad = rows(&a, "First");
    bad[1].opcode = "ldnull".into();
    rejected(&a, "First", &bad, "index requires");
    let mut bad = rows(&a, "MakeArray");
    bad.iter_mut()
        .find(|r| r.opcode == "newarr")
        .unwrap()
        .operand = format!("0x{:08X}", token(&a, "First"));
    rejected(&a, "MakeArray", &bad, "TypeDef, TypeRef or TypeSpec");
    let a = Assembly::load(EDIT.to_vec()).unwrap();
    let mut bad = rows(&a, "Strings");
    let i = bad.iter_mut().find(|r| r.opcode == "ldstr").unwrap();
    i.opcode = "ldc.i4.1".into();
    i.operand.clear();
    rejected(&a, "Strings", &bad, "Array element requires");
    let mut bad = rows(&a, "FloatAt");
    bad.iter_mut()
        .find(|r| r.opcode == "ldelem.r4")
        .unwrap()
        .opcode = "ldelem.r8".into();
    rejected(&a, "FloatAt", &bad, "array element type");
}
#[test]
fn instance_verifier_checks_receivers_static_flags_and_readonly_fields() {
    let a = Assembly::load(EDIT.to_vec()).unwrap();
    let mut bad = rows(&a, "Call");
    bad[0].opcode = "ldc.i4.1".into();
    rejected(&a, "Call", &bad, "Expected");
    let mut bad = rows(&a, "Call");
    bad.iter_mut()
        .find(|i| i.opcode == "callvirt")
        .unwrap()
        .operand = format!("0x{:08X}", token(&a, "ToObject"));
    rejected(&a, "Call", &bad, "callvirt requires");
    let mut bad = rows(&a, "OtherRead");
    let field = rows(&a, "Increase")
        .into_iter()
        .find(|i| i.opcode == "ldfld")
        .unwrap()
        .operand;
    bad.iter_mut()
        .find(|i| i.opcode == "ldfld")
        .unwrap()
        .operand = field;
    rejected(&a, "OtherRead", &bad, "Expected");
    let mut bad = rows(&a, "Increase");
    bad.iter_mut().find(|i| i.opcode == "ldfld").unwrap().opcode = "ldsfld".into();
    rejected(&a, "Increase", &bad, "static flag");
    let readonly = rows(&a, "ReadOnly")
        .into_iter()
        .find(|i| i.opcode == "ldfld")
        .unwrap()
        .operand;
    let bad = vec![
        instruction("a", "ldarg.0", ""),
        instruction("b", "ldc.i4.1", ""),
        instruction("c", "stfld", readonly),
        instruction("d", "ldc.i4.0", ""),
        instruction("e", "ret", ""),
    ];
    rejected(&a, "ReadOnly", &bad, "init-only");
    let mut bad = rows(&a, "Create");
    bad.iter_mut()
        .find(|i| i.opcode == "newobj")
        .unwrap()
        .operand = format!("0x{:08X}", token(&a, "ToObject"));
    rejected(&a, "Create", &bad, "instance constructor");
}
#[test]
fn reference_merges_widen_and_revisit_successors() {
    let a = Assembly::load(EDIT.to_vec()).unwrap();
    // Both input orders must widen Counter/String to object. Backedge changes the
    // initial stack entry; downstream blocks must be rechecked with the wider type.
    let rows = vec![
        instruction("entry", "ldarg.1", ""),
        instruction("head", "br", "use"),
        instruction("use", "pop", ""),
        instruction("cond", "ldarg.0", ""),
        instruction("test", "brfalse", "end"),
        instruction("text", "ldarg.2", ""),
        instruction("loop", "br", "head"),
        instruction("end", "ldnull", ""),
        instruction("return", "ret", ""),
    ];
    edit::validate(&a, token(&a, "SelectKinds"), &rows).unwrap();
    let mut bad = rows.clone();
    bad[5] = instruction("text", "ldc.i4.1", "");
    rejected(&a, "SelectKinds", &bad, "Incompatible stack");
}
#[test]
fn unsupported_signatures_have_specific_diagnostics() {
    let a = Assembly::load(EDIT.to_vec()).unwrap();
    for (name, detail) in [
        ("Generic", "Generic methods"),
        ("Matrix", "Multidimensional"),
        ("ByRef", "Byref"),
        ("Struct", "structs"),
        ("ExceptionRegion", "exception handlers"),
    ] {
        let error = edit::preview(&a, token(&a, name), 0, None)
            .err()
            .expect("unsupported method");
        assert_eq!(error.code, ErrorCode::UnsupportedEdit);
        assert!(error.detail.contains(detail), "{name}: {error}");
    }
}

#[test]
fn delegate_constructors_cannot_turn_arbitrary_integers_into_code_pointers() {
    let mut a = Assembly::load(PAT.to_vec()).unwrap();
    let delegate = a
        .nodes
        .iter()
        .find(|n| n.name == "Transform" && n.token >> 24 == 2)
        .unwrap()
        .token;
    let ctor = a
        .nodes
        .iter()
        .find(|n| n.parent == delegate && n.name == ".ctor")
        .unwrap()
        .token;
    let rows = vec![
        instruction("target", "ldnull", ""),
        instruction("number", "ldc.i4.0", ""),
        instruction("pointer", "conv.i", ""),
        instruction("delegate", "newobj", format!("0x{ctor:08X}")),
        instruction("return", "ret", ""),
    ];
    let error = edit::validate(&a, token(&a, "Box"), &rows).unwrap_err();
    assert_eq!(error.code, ErrorCode::UnsupportedEdit);
    assert!(error.detail.contains("delegates"), "{error}");
    // Forging the implementation flags must not bypass the declaring-type check.
    a.store.parsed.method_defs[(ctor & 0xffffff) as usize - 1].impl_flags = 0;
    let error = edit::validate(&a, token(&a, "Box"), &rows).unwrap_err();
    assert_eq!(error.code, ErrorCode::UnsupportedEdit);
    assert!(error.detail.contains("delegate"), "{error}");
}
