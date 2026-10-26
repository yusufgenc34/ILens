use decompiler_core::{assembly::Assembly, resolution::local_accessor, typeview::type_source};
const PAT: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Patterns.dll");
fn token(a: &Assembly, name: &str) -> u32 {
    a.nodes
        .iter()
        .find(|n| n.token >> 24 == 2 && n.name == name)
        .unwrap()
        .token
}
#[test]
fn type_documents_have_usings_tokens_and_member_bodies() {
    let a = Assembly::load(PAT.to_vec()).unwrap();
    let view = type_source(&a, token(&a, "Calculator"), &|t| local_accessor(&a, t)).unwrap();
    let text = &view.text;
    for expected in [
        "using System;",
        "namespace ILens.Samples\n{",
        "    public class Calculator : ICalculator\n",
        "        // Token: 0x0600000C RID: 12 RVA: 0x000020E5 File Offset: 0x000002E5\n",
        "            this.Offset = offset;",
        "        // (get) Token: 0x06000008",
        "        public int Value { get; set; }",
        "        public event EventHandler Changed;",
        "    [Example(\"A real compiled fixture, never executed by the decompiler\")]\n    public class Calculator",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
    // Types in the document's own namespace and System are written without qualification.
    assert!(!text.contains("ILens.Samples.Calculator"));
    assert!(!text.contains("System.EventHandler"));
    // Compiler-generated backing fields stay hidden behind auto-properties and events,
    // and an implicit base() call is not written.
    assert!(!text.contains("k__BackingField"), "{text}");
    assert!(!text.contains("private EventHandler Changed;"));
    assert!(!text.contains(": base()"));
    assert!(text.contains("            this.Offset = offset;\n        }"));
    assert!(text.find("using System;").unwrap() < text.find("namespace").unwrap());
    // Navigation ranges point at identifiers in the combined document.
    let units: Vec<u16> = text.encode_utf16().collect();
    assert!(!view.references.is_empty());
    for r in &view.references {
        let word = String::from_utf16(&units[r.start..r.end]).unwrap();
        // Generic object creation covers the whole name, e.g. Nullable<int>.
        assert!(
            word.chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
                && !word.contains('\n'),
            "{word:?}"
        );
    }
    // The short-name context never leaks into analysis names.
    assert_eq!(
        a.resolve(token(&a, "Calculator")),
        "ILens.Samples.Calculator"
    );
}
#[test]
fn explorer_labels_use_short_names_but_full_names_stay_qualified() {
    let a = Assembly::load(PAT.to_vec()).unwrap();
    let fast = a
        .nodes
        .iter()
        .find(|n| n.token >> 24 == 4 && n.name == "Fast")
        .unwrap();
    assert_eq!(fast.display_name, "Fast : Mode");
    assert_eq!(fast.full_name, "ILens.Samples.Mode.Fast");
}
#[test]
fn declarations_attributes_and_member_names_navigate() {
    let a = Assembly::load(PAT.to_vec()).unwrap();
    let calculator = token(&a, "Calculator");
    let view = type_source(&a, calculator, &|t| local_accessor(&a, t)).unwrap();
    let units: Vec<u16> = view.text.encode_utf16().collect();
    let linked: Vec<(String, u32)> = view
        .references
        .iter()
        .map(|r| (String::from_utf16_lossy(&units[r.start..r.end]), r.token))
        .collect();
    let has = |name: &str, table: u32| linked.iter().any(|(n, t)| n == name && t >> 24 == table);
    // Header: the type itself and its interface; the attribute type; member types.
    assert!(
        linked.contains(&("Calculator".into(), calculator)),
        "{linked:?}"
    );
    assert!(has("ICalculator", 2), "{linked:?}");
    assert!(has("Example", 2), "{linked:?}");
    assert!(has("EventHandler", 1), "{linked:?}");
    // Declared members link to their own definitions.
    assert!(
        has("Value", 0x17) && has("Changed", 0x14) && has("Add", 6),
        "{linked:?}"
    );
    // Ranges never overlap and the visible text is unchanged by linking.
    let mut sorted = view.references.clone();
    sorted.sort_by_key(|r| r.start);
    assert!(sorted.windows(2).all(|w| w[0].end <= w[1].start));
    assert!(!view.text.contains('\0') && !view.text.contains('\u{1}'));
    let plain = decompiler_core::assembly::without_links(|| {
        type_source(&a, calculator, &|t| local_accessor(&a, t))
            .unwrap()
            .text
    });
    assert_eq!(view.text, plain);
}
