//! C# language constructs reconstructed from `samples/Language` (compiled, never run).
use decompiler_core::{assembly::Assembly, resolution::local_accessor, typeview::type_source};
const LANGUAGE: &[u8] = include_bytes!("../../../samples/fixtures/ILens.Language.dll");
fn view(a: &Assembly, name: &str) -> String {
    let token = a
        .nodes
        .iter()
        .find(|n| n.token >> 24 == 2 && n.name == name)
        .unwrap()
        .token;
    type_source(a, token, &|t| local_accessor(a, t))
        .unwrap()
        .text
}
fn assert_contains(text: &str, expected: &[&str]) {
    for e in expected {
        assert!(text.contains(e), "missing {e:?} in\n{text}");
    }
}
#[test]
fn operators_constructors_and_framework_members() {
    let a = Assembly::load(LANGUAGE.to_vec()).unwrap();
    assert_contains(
        &view(&a, "Money"),
        &[
            // A struct constructor has no base constructor call to hoist.
            "public Money(decimal amount)\n        {\n            this.Amount = amount;",
            "public static Money operator +(Money left, Money right)",
            "return new Money(left.Amount + right.Amount);",
            "return !(left == right);",
            "if (!(obj is Money))",
        ],
    );
    let point = view(&a, "Point");
    assert_contains(
        &point,
        &[
            "return typeof(Point);",
            "EqualityComparer<int>.Default.GetHashCode(this.X)",
            "stringBuilder.Append(' ');",
            // Reference equality must not call the operator being defined.
            "if ((object)left == right)",
            "if ((object)left == null)",
        ],
    );
    assert!(!point.contains("_ = stringBuilder"), "{point}");
    assert_contains(
        &view(&a, "Bag"),
        &[
            "public Bag()\n        {\n            this.items = new List<string>();",
            "this.changed?.Invoke(this, EventArgs.Empty);\n        }",
        ],
    );
}
