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
            "if (!(obj is Money money))",
            "return money.Amount == this.Amount;",
        ],
    );
    // Synthesized record members are implied by the declaration, as dnSpy shows it.
    assert_contains(&view(&a, "Point"), &["public record Point(int X, int Y);"]);
    let method = |name: &str| {
        let n = a
            .nodes
            .iter()
            .find(|n| n.token >> 24 == 6 && n.full_name.contains("Point.") && n.name == name)
            .unwrap();
        decompiler_core::decompile::decompile_view(&a, n.token, &|t| local_accessor(&a, t))
            .unwrap()
            .csharp
    };
    assert_contains(&method("get_EqualityContract"), &["return typeof(Point);"]);
    assert_contains(
        &method("GetHashCode"),
        &["EqualityComparer<int>.Default.GetHashCode(this.X)"],
    );
    let printed = method("ToString");
    assert_contains(&printed, &["stringBuilder.Append(' ');"]);
    assert!(!printed.contains("_ = stringBuilder"), "{printed}");
    // Reference equality must not call the operator being defined.
    assert_contains(
        &method("op_Equality"),
        &["return (object)left == right || (object)left != null && left.Equals(right);"],
    );
    assert_contains(
        &view(&a, "Bag"),
        &[
            "public Bag()\n        {\n            this.items = new List<string>();",
            "this.changed?.Invoke(this, EventArgs.Empty);\n        }",
        ],
    );
}
#[test]
fn indexers_parameters_extensions_and_tuples() {
    let a = Assembly::load(LANGUAGE.to_vec()).unwrap();
    let constructs = view(&a, "Constructs");
    assert_contains(
        &constructs,
        &[
            "public static bool TryParse(string text, out int value)",
            "return int.TryParse(text, out value);",
            "public static int Defaulted(int value = 7, string text = \"x\")",
            "return text.Shout();",
            "return a + b;",
            "bag[0] = \"first\";\n            return bag[0];",
            "public static (int, int) Tuple(int a, int b)",
            "return (a + b, a * b);",
            "return num + valueTuple.Item2;",
        ],
    );
    assert_contains(
        &view(&a, "Bag"),
        &[
            "public string this[int index]",
            "return this.items[index];",
            "public void Add(params string[] values)",
        ],
    );
    let extensions = view(&a, "Extensions");
    assert_contains(
        &extensions,
        &["public static string Shout(this string text)"],
    );
    // Syntax replaces the attributes the compiler emits for it.
    assert!(!extensions.contains("[Extension]"), "{extensions}");
    assert!(!view(&a, "Bag").contains("[DefaultMember"));
}
#[test]
fn cached_lambdas_and_local_functions_read_as_source() {
    let a = Assembly::load(LANGUAGE.to_vec()).unwrap();
    let constructs = view(&a, "Constructs");
    assert_contains(
        &constructs,
        &[
            "return x => x * 3;",
            "return values.Where<int>(v => v > 2).Select<int, int>(v => v * 2).Sum();",
            "return Twice(value) + 1;\n\n            static int Twice(int x)\n            {\n                return x * 2;\n            }",
        ],
    );
    // The caching field and the generated method are not shown.
    assert!(!constructs.contains("<>9__"), "{constructs}");
    assert!(!constructs.contains("g__Twice"), "{constructs}");
}
#[test]
fn iterator_finally_async_iterators_and_spilled_awaits() {
    let a = Assembly::load(LANGUAGE.to_vec()).unwrap();
    let constructs = view(&a, "Constructs");
    assert_contains(
        &constructs,
        &[
            "public static IEnumerable<int> Guarded(int n)\n        {\n            try\n            {\n                for (int i = 0; i < n; i++)\n                {\n                    yield return i;\n                }\n            }\n            finally\n            {\n                Constructs.counter--;\n            }\n        }",
            "public static async IAsyncEnumerable<int> Stream(int n)\n        {\n            for (int i = 0; i < n; i++)\n            {\n                await Task.Yield();\n                yield return i;\n            }\n        }",
            "public static async Task<int> AwaitTwice(Task<int> first, Task<int> second)",
            "= await first;",
        ],
    );
    assert!(
        !constructs.contains("State-machine reconstruction"),
        "{constructs}"
    );
}
#[test]
fn conditionals_patterns_and_statement_idioms() {
    let a = Assembly::load(LANGUAGE.to_vec()).unwrap();
    let constructs = view(&a, "Constructs");
    assert_contains(
        &constructs,
        &[
            "return value > 0 ? \"positive\" : \"other\";",
            "return text ?? \"none\";",
            "return text?.Length;",
            "if (value is string text)\n            {\n                return text;\n            }\n            if (!(value is int num))\n            {\n                return \"other\";\n            }\n            return num > 3 ? \"big\" : \"other\";",
            "return value == 0 ? \"zero\" : value == 1 ? \"one\" : \"many\";",
            "return $\"{name} has {count} items\";",
            "return new Point(1, 2) with { Y = 5 };",
            "return new List<int> { 1, 2, 3 };",
            "return new StringBuilder { Capacity = 64 };",
            "lock (Constructs.Gate)\n            {\n                Constructs.counter++;\n            }",
        ],
    );
    // Roslyn's length/character dispatch is the original string switch again.
    assert_contains(
        &constructs,
        &[
            "switch (key)\n            {\n                case \"eta\":\n                    return 7;",
            "case \"alpha\":\n                    return 1;",
            "default:\n                    return 0;\n            }",
        ],
    );
    assert!(!constructs.contains("key.Length"), "{constructs}");
}
#[test]
fn declarations_read_like_source() {
    let patterns =
        Assembly::load(include_bytes!("../../../samples/fixtures/ILens.Patterns.dll").to_vec())
            .unwrap();
    // Interface members carry no modifiers; an int enum names no underlying type.
    assert_contains(
        &view(&patterns, "ICalculator"),
        &["        int Add(int left, int right);"],
    );
    let mode = view(&patterns, "Mode");
    assert_contains(&mode, &["public enum Mode\n"]);
    let editing =
        Assembly::load(include_bytes!("../../../samples/fixtures/ILens.Editing.dll").to_vec())
            .unwrap();
    // `cgt.un` of a reference and null is the non-null test.
    assert_contains(&view(&editing, "Operations"), &["return value != null;"]);
}
