use decompiler_core::{
    analysis::StackAnalysis,
    assembly::Assembly,
    decompile,
    ir::{Edge, Expr, ExprKind, IrBlock, Statement, Terminator, TypedIr},
    signature::Type,
    transform,
};

fn fixture() -> Assembly {
    Assembly::load(include_bytes!("../../../samples/fixtures/ILens.Readability.dll").to_vec())
        .unwrap()
}
fn source(a: &Assembly, name: &str) -> String {
    let token = a.nodes.iter().find(|n| n.name == name).unwrap().token;
    let result = decompile::decompile(a, token).unwrap();
    assert_eq!(result.quality, "csharp_like", "{:?}", result.diagnostics);
    assert!(result.il.contains("IL_"));
    result.csharp
}
#[test]
fn message_array_is_readable_without_changing_accessor_order() {
    let a = fixture();
    let text = source(&a, "BuildMessage");
    assert!(!text.contains("v_"), "{text}");
    assert!(!text.contains("IL_"), "{text}");
    let start = text.find("new System.String[12]\n").expect(&text);
    let contents = &text[start..text[start..].find("};").unwrap() + start];
    let elements: Vec<_> = contents
        .lines()
        .skip(2)
        .map(|s| s.trim().trim_end_matches(','))
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(
        elements,
        [
            "text",
            "this.Text",
            "text",
            "this.Text",
            "text",
            "System.Convert.ToString(this.Checked)",
            "text",
            "this.Text",
            "text",
            "System.Convert.ToString(this.Checked)",
            "text",
            "this.get_NotAProperty()"
        ]
    );
    assert!(text.contains("try\n") && text.contains("catch (System.Exception"));
    // Locals are named from their types, as ILSpy/dnSpy name them.
    assert!(text.contains("System.String.Concat(array)"), "{text}");
    let token = a
        .nodes
        .iter()
        .find(|n| n.name == "BuildMessage")
        .unwrap()
        .token;
    let original = a.body(token).unwrap().unwrap();
    assert_eq!(
        original
            .instructions
            .iter()
            .filter(|i| i.name == "stelem.ref")
            .count(),
        12
    );
}
#[test]
fn fresh_initializer_folds_but_partial_array_stays_visible_to_catch() {
    let a = fixture();
    assert!(
        source(&a, "Initializer").contains("return new System.Int32[3] { value, value + 1, 42 };")
    );
    let partial = source(&a, "PartialArray");
    assert!(
        partial.contains("array = new System.Int32[3];"),
        "{partial}"
    );
    assert!(partial.contains("10 / value"));
    assert!(partial.contains("array[1] = "));
    // The division still runs before the element store, inside the same statement.
    assert!(partial.contains("array[1] = 10 / value;"), "{partial}");
    // A C# general catch compiles to a System.Object clause.
    assert!(partial.contains("    catch\n    {"), "{partial}");
    assert!(!partial.contains("new System.Int32[3] {"));
    // This catch exits past another normal block: its goto is necessary.
    for part in partial.split("goto ").skip(1) {
        let label = part.split(';').next().unwrap();
        assert!(partial.contains(&format!("{label}:;")), "{partial}");
    }
}
#[test]
fn property_syntax_requires_metadata_semantics() {
    let a = fixture();
    assert!(source(&a, "PropertyRead").contains("this.Text"));
    assert!(!source(&a, "PropertyRead").contains("get_Text"));
    assert!(source(&a, "OrdinaryMethod").contains("this.get_NotAProperty()"));
    assert!(source(&a, "ExceptionMessage").contains("error.get_Message()"));
}
fn var(name: &str) -> Expr {
    Expr::var(name, Type::primitive("int"))
}
fn number(n: i32) -> Expr {
    Expr::constant(n.to_string(), Type::primitive("int"))
}
fn call(name: &str, args: Vec<Expr>) -> Expr {
    Expr {
        ty: Type::primitive("int"),
        kind: ExprKind::Call {
            token: 0x06000001,
            owner: 0x02000001,
            name: name.into(),
            instance: None,
            args,
            generic_args: vec![],
            virtual_call: false,
            accessor: None,
        },
    }
}
fn let_value(name: &str, value: Expr) -> Statement {
    Statement::Let {
        name: name.into(),
        ty: value.ty.clone(),
        value,
    }
}
fn assign(name: &str, value: Expr) -> Statement {
    Statement::Assign {
        target: var(name),
        value,
    }
}
fn simplify(statements: Vec<Statement>, terminator: Terminator) -> IrBlock {
    let mut ir = TypedIr {
        blocks: vec![IrBlock {
            id: 0,
            offset: 0,
            statements,
            terminator,
        }],
        locals: vec![],
        argument_names: vec![],
        return_type: Type::primitive("int"),
        stack: StackAnalysis {
            entry_stacks: vec![Some(vec![])],
            maximum: 8,
            reachable_blocks: 1,
        },
        local_names: Default::default(),
    };
    transform::simplify(&mut ir);
    ir.blocks.remove(0)
}
fn retained(block: &IrBlock, name: &str) -> bool {
    block
        .statements
        .iter()
        .any(|s| matches!(s, Statement::Let { name: n, .. } if n == name))
}
#[test]
fn local_snapshot_survives_overwrite_and_address_escape() {
    let block = simplify(
        vec![
            let_value("saved", var("local0")),
            assign("local0", number(9)),
        ],
        Terminator::Return(Some(var("saved"))),
    );
    assert!(retained(&block, "saved"));
    let address = Expr {
        ty: Type::ByRef(Box::new(Type::primitive("int"))),
        kind: ExprKind::Address(Box::new(var("local0"))),
    };
    let block = simplify(
        vec![
            let_value("saved", var("local0")),
            Statement::Evaluate(call("Mutate", vec![address])),
        ],
        Terminator::Return(Some(var("saved"))),
    );
    assert!(retained(&block, "saved"));
}
#[test]
fn reversed_call_arguments_keep_original_evaluation_order() {
    let block = simplify(
        vec![
            let_value("first", call("First", vec![])),
            let_value("second", call("Second", vec![])),
        ],
        Terminator::Return(Some(call("Combine", vec![var("second"), var("first")]))),
    );
    assert!(
        matches!(&block.statements[..], [Statement::Let { name, value: Expr { kind: ExprKind::Call { name: callee, .. }, .. }, .. }] if name == "first" && callee == "First")
    );
    assert!(
        matches!(&block.terminator, Terminator::Return(Some(Expr { kind: ExprKind::Call { args, .. }, .. })) if matches!(&args[0].kind, ExprKind::Call { name, .. } if name == "Second") && matches!(&args[1].kind, ExprKind::Variable(n) if n == "first"))
    );
}
#[test]
fn captured_heap_read_and_dup_are_not_repeated_or_delayed() {
    let field = Expr {
        ty: Type::primitive("int"),
        kind: ExprKind::Field {
            token: 0x04000001,
            owner: 0x02000001,
            name: "Value".into(),
            instance: None,
        },
    };
    let block = simplify(
        vec![
            let_value("saved", field),
            Statement::Evaluate(call("MutateField", vec![])),
        ],
        Terminator::Return(Some(var("saved"))),
    );
    assert!(retained(&block, "saved"));
    let block = simplify(
        vec![let_value("once", call("Next", vec![]))],
        Terminator::Return(Some(call("Combine", vec![var("once"), var("once")]))),
    );
    assert!(retained(&block, "once"));
}
#[test]
fn eager_call_does_not_become_conditional_edge_evaluation() {
    let block = simplify(
        vec![let_value("eager", call("Next", vec![]))],
        Terminator::Condition {
            condition: var("flag"),
            yes: Edge {
                target: 1,
                values: vec![var("eager")],
            },
            no: Edge {
                target: 2,
                values: vec![number(0)],
            },
        },
    );
    assert!(retained(&block, "eager"));
}
#[test]
fn array_allocation_cannot_move_before_an_eager_call() {
    let initializer = Expr {
        ty: Type::Array(Box::new(Type::primitive("int")), 1),
        kind: ExprKind::ArrayInitializer {
            element: Type::primitive("int"),
            values: vec![var("first")],
        },
    };
    let block = simplify(
        vec![let_value("first", call("First", vec![]))],
        Terminator::Return(Some(initializer)),
    );
    assert!(retained(&block, "first"));
}
#[test]
fn throwing_index_is_not_reordered_with_rhs_call() {
    let index = Expr {
        ty: Type::primitive("int"),
        kind: ExprKind::Binary {
            op: "/".into(),
            left: Box::new(number(10)),
            right: Box::new(var("divisor")),
            checked: false,
            unsigned: false,
        },
    };
    let target = Expr {
        ty: Type::primitive("int"),
        kind: ExprKind::Index {
            array: Box::new(var("array")),
            index: Box::new(index),
        },
    };
    let block = simplify(
        vec![
            let_value("first", call("First", vec![])),
            Statement::Assign {
                target,
                value: var("first"),
            },
        ],
        Terminator::Return(None),
    );
    assert!(retained(&block, "first"));
}

#[test]
fn array_rhs_needs_a_nonnull_in_range_receiver_proof() {
    for (length, index, keeps_call) in [(None, 0, true), (Some(2), 3, true), (Some(2), 0, false)] {
        let mut statements = vec![];
        if let Some(length) = length {
            statements.push(let_value(
                "array",
                Expr {
                    ty: Type::Array(Box::new(Type::primitive("int")), 1),
                    kind: ExprKind::NewArray {
                        element: Type::primitive("int"),
                        length: Box::new(number(length)),
                    },
                },
            ));
        }
        statements.push(let_value("first", call("First", vec![])));
        statements.push(Statement::Assign {
            target: Expr {
                ty: Type::primitive("int"),
                kind: ExprKind::Index {
                    array: Box::new(var("array")),
                    index: Box::new(number(index)),
                },
            },
            value: var("first"),
        });
        let block = simplify(statements, Terminator::Return(None));
        assert_eq!(
            retained(&block, "first"),
            keeps_call,
            "length={length:?}, index={index}: {block:?}"
        );
    }
}
fn binary(op: &str, left: Expr, right: Expr) -> Expr {
    Expr {
        ty: Type::primitive("int"),
        kind: ExprKind::Binary {
            op: op.into(),
            left: Box::new(left),
            right: Box::new(right),
            checked: false,
            unsigned: false,
        },
    }
}
#[test]
fn parentheses_follow_csharp_precedence_and_associativity() {
    let a = fixture();
    let render = |e: &Expr| decompiler_core::emit::expression(&a, 0, e).unwrap();
    let (x, y, z) = (var("x"), var("y"), var("z"));
    let sum = binary("+", x.clone(), y.clone());
    assert_eq!(render(&binary("*", sum.clone(), z.clone())), "(x + y) * z");
    assert_eq!(
        render(&binary("+", x.clone(), binary("*", y.clone(), z.clone()))),
        "x + y * z"
    );
    // Left-associative operators keep parentheses only on the right.
    assert_eq!(
        render(&binary("-", binary("-", x.clone(), y.clone()), z.clone())),
        "x - y - z"
    );
    assert_eq!(
        render(&binary("-", x.clone(), binary("-", y.clone(), z.clone()))),
        "x - (y - z)"
    );
    assert_eq!(
        render(&binary("==", binary("&", x.clone(), y.clone()), number(0))),
        "(x & y) == 0"
    );
    let negative = number(-1);
    let cast = |value: Expr| Expr {
        ty: Type::primitive("long"),
        kind: ExprKind::Cast {
            mode: "convert".into(),
            value: Box::new(value),
            target: Type::primitive("long"),
        },
    };
    // Constant conversions fold as the C# compiler would.
    assert_eq!(render(&cast(negative)), "-1L");
    // (T)-x would parse as a subtraction for a named type, so signs stay wrapped.
    let minus_x = Expr {
        ty: Type::primitive("int"),
        kind: ExprKind::Unary {
            op: "-".into(),
            value: Box::new(x.clone()),
        },
    };
    assert_eq!(render(&cast(minus_x)), "(long)(-x)");
    assert_eq!(render(&cast(sum)), "(long)(x + y)");
    let not_equal = binary("!=", x.clone(), y.clone());
    let negated = Expr {
        ty: Type::primitive("bool"),
        kind: ExprKind::Unary {
            op: "!".into(),
            value: Box::new(not_equal),
        },
    };
    assert_eq!(render(&negated), "x == y");
}
