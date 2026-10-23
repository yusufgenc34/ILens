use crate::{
    assembly::{Assembly, identifier, strip_arity},
    ast::Ast,
    cfg::ControlFlowGraph,
    cil::{MethodBody, Operand},
    error::{Error, Result},
    ir::*,
    signature::Type,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;
fn safe_comment(s: &str) -> String {
    s.replace("*/", "* /").replace(['\r', '\n'], " ")
}
fn type_name(a: &Assembly, t: &Type, context: u32) -> String {
    a.format_type(t, context)
}
fn unsigned_type(t: &Type) -> Result<&'static str> {
    if *t == Type::primitive("long") || *t == Type::primitive("ulong") {
        Ok("ulong")
    } else if *t == Type::primitive("nint") || *t == Type::primitive("nuint") {
        Ok("nuint")
    } else if matches!(t,Type::Primitive(n) if ["int","uint","bool","byte","sbyte","short","ushort","char"].contains(&n.as_str()))
    {
        Ok("uint")
    } else {
        Err(Error::limitation(
            "Unsigned/unordered comparison cannot be safely represented for this stack type",
        ))
    }
}
thread_local! {
    // Readable local names for the method body currently being rendered.
    static LOCAL_NAMES: std::cell::RefCell<HashMap<String, String>> = std::cell::RefCell::new(HashMap::new());
}
struct LocalNames;
impl Drop for LocalNames {
    fn drop(&mut self) {
        LOCAL_NAMES.with(|names| names.borrow_mut().clear());
    }
}
// ILSpy-style names derived from the local's type: num, text, flag, obj, eventHandler.
fn local_base(a: &Assembly, ty: &Type, context: u32) -> String {
    let named = |token: u32| {
        let name = strip_arity(&a.resolve(token));
        let name = name.rsplit(['.', '+']).next().unwrap_or("").to_owned();
        let mut chars: Vec<char> = name.chars().collect();
        // Interfaces drop their I prefix: IEnumerator -> enumerator.
        if chars.len() > 2 && chars[0] == 'I' && chars[1].is_uppercase() {
            chars.remove(0);
        }
        if chars.is_empty() || !chars.iter().all(|c| c.is_alphanumeric() || *c == '_') {
            return "local".to_owned();
        }
        chars[0] = chars[0].to_ascii_lowercase();
        chars.into_iter().collect()
    };
    match ty {
        Type::Primitive(name) => match name.as_str() {
            "bool" => "flag",
            "char" => "c",
            "string" => "text",
            "object" => "obj",
            "byte" | "sbyte" => "b",
            _ => "num",
        }
        .to_owned(),
        Type::Named(t) | Type::ValueType(t) => match a.format_type(ty, context).as_str() {
            "string" | "System.String" => "text".to_owned(),
            "object" | "System.Object" => "obj".to_owned(),
            "bool" | "System.Boolean" => "flag".to_owned(),
            "int" | "uint" | "long" | "ulong" | "System.Int32" | "System.Int64" => "num".to_owned(),
            _ => named(*t),
        },
        Type::GenericInstance { base, .. } => local_base(a, base, context),
        Type::Array(..) | Type::MultiArray(..) => "array".to_owned(),
        Type::Pinned(t) => local_base(a, t, context),
        Type::ByRef(_) | Type::Pointer(_) | Type::FunctionPointer(_) => "ptr".to_owned(),
        _ => "local".to_owned(),
    }
}
fn strip_final_returns(nodes: &mut Vec<Ast>) {
    match nodes.last_mut() {
        Some(Ast::Return(None)) => {
            nodes.pop();
        }
        Some(Ast::If { yes, no, .. }) => {
            strip_final_returns(yes);
            strip_final_returns(no);
        }
        _ => {}
    }
}
fn catch_variables(ast: &[Ast]) -> Vec<String> {
    let mut found = vec![];
    for n in ast {
        match n {
            Ast::Try { body, handlers } => {
                found.extend(catch_variables(body));
                for h in handlers {
                    found.extend(h.variable.clone());
                    found.extend(catch_variables(&h.body));
                }
            }
            Ast::If { yes, no, .. } => {
                found.extend(catch_variables(yes));
                found.extend(catch_variables(no));
            }
            Ast::While { body, .. } | Ast::For { body, .. } => found.extend(catch_variables(body)),
            Ast::Switch { arms, default, .. } => {
                for arm in arms {
                    found.extend(catch_variables(arm));
                }
                found.extend(catch_variables(default));
            }
            _ => {}
        }
    }
    found
}
fn let_name(s: &Statement, found: &mut Vec<(String, Type)>) {
    if let Statement::Let { name, ty, .. } = s {
        found.push((name.clone(), ty.clone()));
    }
}
fn temporaries(ast: &[Ast], found: &mut Vec<(String, Type)>) {
    for n in ast {
        match n {
            Ast::Statement(s) => let_name(s, found),
            Ast::For {
                initializer,
                increment,
                body,
                ..
            } => {
                let_name(initializer, found);
                let_name(increment, found);
                temporaries(body, found);
            }
            Ast::If { yes, no, .. } => {
                temporaries(yes, found);
                temporaries(no, found);
            }
            Ast::While { body, .. } => temporaries(body, found),
            Ast::Switch { arms, default, .. } => {
                for arm in arms {
                    temporaries(arm, found);
                }
                temporaries(default, found);
            }
            Ast::Try { body, handlers } => {
                temporaries(body, found);
                for h in handlers {
                    temporaries(&h.body, found);
                }
            }
            _ => {}
        }
    }
}
// Simplifier temporaries (v_0013) get the same type-derived names as locals.
fn name_temporaries(a: &Assembly, token: u32, ast: &[Ast]) {
    let mut found = vec![];
    temporaries(ast, &mut found);
    let parameters: Vec<String> = a
        .method_ref(token)
        .map(|m| a.parameter_names(token, &m.signature))
        .unwrap_or_default();
    LOCAL_NAMES.with(|names| {
        let mut names = names.borrow_mut();
        let mut taken: HashSet<String> = names.values().cloned().chain(parameters).collect();
        for (variable, ty) in found {
            if names.contains_key(&variable) {
                continue;
            }
            let base = local_base(a, &ty, token);
            let base = if identifier(&base) == base {
                base
            } else {
                "local".to_owned()
            };
            let mut name = base.clone();
            let mut n = 2;
            while !taken.insert(name.clone()) {
                name = format!("{base}{n}");
                n += 1;
            }
            names.insert(variable, name);
        }
    });
}
// Caught exceptions are named ex, ex2, … as ILSpy names them.
fn name_catches(variables: &[String]) {
    LOCAL_NAMES.with(|names| {
        let mut names = names.borrow_mut();
        let mut taken: HashSet<String> = names.values().cloned().collect();
        for variable in variables {
            let mut name = "ex".to_owned();
            let mut n = 2;
            while !taken.insert(name.clone()) {
                name = format!("ex{n}");
                n += 1;
            }
            names.insert(variable.clone(), name);
        }
    });
}
fn name_locals(
    a: &Assembly,
    token: u32,
    locals: &[Type],
    hints: &HashMap<usize, String>,
) -> LocalNames {
    let mut taken: std::collections::HashSet<String> = a
        .method_ref(token)
        .map(|m| a.parameter_names(token, &m.signature))
        .unwrap_or_default()
        .into_iter()
        .chain(["this", "value"].map(str::to_owned))
        .collect();
    let mut names = HashMap::new();
    for (i, ty) in locals.iter().enumerate() {
        let base = hints
            .get(&i)
            .cloned()
            .unwrap_or_else(|| local_base(a, ty, token));
        let base = if crate::assembly::identifier(&base) == base {
            base
        } else {
            "local".to_owned()
        };
        let mut name = base.clone();
        let mut n = 2;
        while !taken.insert(name.clone()) {
            name = format!("{base}{n}");
            n += 1;
        }
        names.insert(format!("local{i}"), name);
    }
    LOCAL_NAMES.with(|map| *map.borrow_mut() = names);
    LocalNames
}
pub fn expression(a: &Assembly, context: u32, e: &Expr) -> Result<String> {
    Ok(render(a, context, e)?.0)
}
// C# operator precedence, higher binds tighter. Parentheses are added only when a
// child binds more loosely than its position requires.
const PRIMARY: u8 = 15;
const UNARY: u8 = 14;
const ASSIGNMENT: u8 = 1;
fn binary_precedence(op: &str) -> u8 {
    match op {
        "*" | "/" | "%" => 13,
        "+" | "-" => 12,
        "<<" | ">>" => 11,
        "<" | ">" | "<=" | ">=" => 10,
        "==" | "!=" => 9,
        "&" => 8,
        "^" => 7,
        "|" => 6,
        "&&" => 5,
        "||" => 4,
        _ => 0,
    }
}
fn wrap((text, precedence): (String, u8), minimum: u8) -> String {
    if precedence >= minimum {
        text
    } else {
        format!("({text})")
    }
}
// A cast operand that starts with a sign would parse as subtraction: (T)-x.
fn cast_operand(rendered: (String, u8)) -> String {
    if rendered.0.starts_with(['-', '+']) {
        format!("({})", rendered.0)
    } else {
        wrap(rendered, UNARY)
    }
}
fn render(a: &Assembly, context: u32, e: &Expr) -> Result<(String, u8)> {
    let expr = |e: &Expr| render(a, context, e);
    let at = |e: &Expr, minimum: u8| render(a, context, e).map(|r| wrap(r, minimum));
    // Member access, indexing and calls need a primary receiver: ((T)x).Member.
    let receiver = |e: &Expr| -> Result<std::string::String> {
        let s = at(e, PRIMARY)?;
        Ok(s.strip_prefix("ref ").unwrap_or(&s).to_owned())
    };
    use ExprKind::*;
    Ok(match &e.kind {
        Variable(n) => (
            LOCAL_NAMES
                .with(|names| names.borrow().get(n).cloned())
                .unwrap_or_else(|| n.clone()),
            PRIMARY,
        ),
        Constant(v) => (v.clone(), if v.starts_with('-') { UNARY } else { PRIMARY }),
        String(s) => (
            serde_json::to_string(s).map_err(|e| Error::limitation(e.to_string()))?,
            PRIMARY,
        ),
        Null => ("null".to_owned(), PRIMARY),
        Binary {
            op,
            left,
            right,
            checked,
            unsigned,
        } => {
            let precedence = binary_precedence(op);
            // (x as T) == null tests the type: !(x is T); != null is x is T.
            if matches!(op.as_str(), "==" | "!=") {
                let cast = match (&left.kind, &right.kind) {
                    (Cast { mode, .. }, Null) if mode == "as" => Some(left),
                    (Null, Cast { mode, .. }) if mode == "as" => Some(right),
                    _ => None,
                };
                if let Some(cast) = cast
                    && let Cast { value, target, .. } = &cast.kind
                {
                    let test = format!("{} is {}", at(value, 11)?, type_name(a, target, context));
                    return Ok(if op == "!=" {
                        (test, 10)
                    } else {
                        (format!("!({test})"), UNARY)
                    });
                }
            }
            let (mut l, mut r) = (at(left, precedence)?, at(right, precedence + 1)?);
            // CIL `ceq` on references is reference equality; C# `==` would call a
            // user-defined operator (string, delegates, records), so it needs (object).
            if matches!(op.as_str(), "==" | "!=")
                && !*unsigned
                && left.ty.is_reference()
                && right.ty.is_reference()
                && !matches!(left.kind, Null)
                && !matches!(right.kind, Null)
                && (may_overload_equality(a, &left.ty) || may_overload_equality(a, &right.ty))
            {
                l = format!("(object){}", cast_operand(expr(left)?));
            }
            // A null test on a type with its own operator would call that operator
            // (recursively inside it); framework operators treat null like ceq does.
            if matches!(op.as_str(), "==" | "!=") && !*unsigned {
                if matches!(right.kind, Null) && declares_equality(a, &left.ty) {
                    l = format!("(object){}", cast_operand(expr(left)?));
                } else if matches!(left.kind, Null) && declares_equality(a, &right.ty) {
                    r = format!("(object){}", cast_operand(expr(right)?));
                }
            }
            // A character compared with an integer constant reads as a character literal.
            if is_char(&left.ty)
                && let Constant(c) = &right.kind
                && let Ok(n) = c.parse::<i128>()
            {
                r = char_literal(n);
            }
            if is_char(&right.ty)
                && let Constant(c) = &left.kind
                && let Ok(n) = c.parse::<i128>()
            {
                l = char_literal(n);
            }
            // Operands already typed unsigned need no reinterpreting cast.
            let already = |e: &Expr, ty: &str| e.ty == Type::primitive(ty);
            let mut reinterpreted = false;
            if *unsigned {
                if op == "!=" && left.ty.is_reference() && right.ty.is_reference() {
                } else {
                    let ty = unsigned_type(&left.ty)?;
                    if !already(left, ty) {
                        reinterpreted = true;
                        l = format!("unchecked(({ty}){})", cast_operand(expr(left)?));
                    }
                    if op != ">>" && op != "<<" {
                        let ty = unsigned_type(&right.ty)?;
                        if !already(right, ty) {
                            reinterpreted = true;
                            r = format!("unchecked(({ty}){})", cast_operand(expr(right)?));
                        }
                    }
                }
            }
            let value = format!("{l} {op} {r}");
            let (value, precedence) = if *checked {
                (format!("checked({value})"), PRIMARY)
            } else {
                (value, precedence)
            };
            if *unsigned
                && !e.ty.is_bool()
                && (reinterpreted || !matches!(&e.ty, Type::Primitive(n) if n.starts_with('u')))
            {
                // CIL carries the result bits on its original stack type. The checked
                // operation uses unsigned bounds; reinterpret the result only afterward.
                (
                    format!(
                        "unchecked(({}){})",
                        type_name(a, &e.ty, context),
                        cast_operand((value, precedence))
                    ),
                    PRIMARY,
                )
            } else {
                (value, precedence)
            }
        }
        Unary { op, value } => {
            // !(a != b) is a == b; equality inverts exactly, including NaN operands.
            if op == "!"
                && let Binary {
                    op: inner,
                    left,
                    right,
                    checked: false,
                    unsigned,
                } = &value.kind
                && matches!(inner.as_str(), "==" | "!=")
                && (!unsigned || left.ty.is_reference() && right.ty.is_reference())
            {
                let flipped = Expr {
                    kind: Binary {
                        op: if inner == "==" { "!=" } else { "==" }.to_owned(),
                        left: left.clone(),
                        right: right.clone(),
                        checked: false,
                        unsigned: false,
                    },
                    ty: value.ty.clone(),
                };
                return render(a, context, &flipped);
            }
            if op == "!"
                && let Unary {
                    op: inner,
                    value: inner_value,
                } = &value.kind
                && inner == "!"
            {
                return expr(inner_value);
            }
            let operand = expr(value)?;
            let operand = if operand.0.starts_with(['-', '+']) {
                format!("({})", operand.0)
            } else {
                wrap(operand, UNARY)
            };
            (format!("{op}{operand}"), UNARY)
        }
        Call {
            token,
            owner,
            name,
            instance,
            args,
            generic_args,
            accessor,
            ..
        } => {
            // In-place value construction: `local = new T(args)`.
            if name == ".ctor"
                && let Some(i) = instance
                && !matches!(&i.kind, Variable(n) if n == "this" || n == "base")
            {
                let target = match &i.kind {
                    Address(inner) => inner.as_ref(),
                    _ => i.as_ref(),
                };
                let args = args
                    .iter()
                    .map(|e| expr(e).map(|r| r.0))
                    .collect::<Result<Vec<_>>>()?
                    .join(", ");
                return Ok((
                    format!(
                        "{} = new {}({args})",
                        expr(target)?.0,
                        source_reference(*owner, &strip_arity(&a.resolve(*owner)))
                    ),
                    ASSIGNMENT,
                ));
            }
            if instance.is_none() {
                // Type.GetTypeFromHandle(ldtoken T) is typeof(T).
                if name == "GetTypeFromHandle"
                    && let [
                        Expr {
                            kind: TypeToken(t), ..
                        },
                    ] = args.as_slice()
                    && a.resolve_full(*owner) == "System.Type"
                {
                    return Ok((format!("typeof({})", typeof_name(a, *t)), PRIMARY));
                }
                // Operators: only MethodDefs flagged specialname, or references, whose
                // explicit invocation C# forbids anyway.
                let operator = *token >> 24 == 10
                    || a.method_row(*token).is_ok_and(|row| row.flags & 0x800 != 0);
                if operator {
                    if let (Some(op), [left, right]) = (binary_operator(name), args.as_slice()) {
                        // The operator symbol navigates to the operator method.
                        let precedence = binary_precedence(op);
                        return Ok((
                            format!(
                                "{} {} {}",
                                at(left, precedence)?,
                                source_reference(*token, op),
                                at(right, precedence + 1)?
                            ),
                            precedence,
                        ));
                    }
                    if let (Some(op), [value]) = (unary_operator(name), args.as_slice()) {
                        let unary = Expr {
                            ty: e.ty.clone(),
                            kind: Unary {
                                op: op.to_owned(),
                                value: Box::new(value.clone()),
                            },
                        };
                        return render(a, context, &unary);
                    }
                    if let ("op_Implicit", [value]) = (name.as_str(), args.as_slice()) {
                        return expr(value);
                    }
                    if let ("op_Explicit", [value]) = (name.as_str(), args.as_slice()) {
                        return Ok((
                            format!(
                                "({}){}",
                                type_name(a, &e.ty, context),
                                cast_operand(expr(value)?)
                            ),
                            UNARY,
                        ));
                    }
                }
                // string.Concat of strings is string concatenation.
                if name == "Concat"
                    && (2..=4).contains(&args.len())
                    && args
                        .iter()
                        .all(|arg| is_string(a, &arg.ty) || matches!(arg.kind, String(_)))
                    && a.resolve_full(*owner) == "System.String"
                {
                    // Left-associative `+`; each symbol navigates to Concat.
                    let mut joined = at(&args[0], 12)?;
                    for next in &args[1..] {
                        joined = format!(
                            "{joined} {} {}",
                            source_reference(*token, "+"),
                            at(next, 13)?
                        );
                    }
                    return Ok((joined, 12));
                }
            }
            let receiver = if let Some(i) = instance {
                receiver(i)?
            } else {
                strip_arity(&a.resolve(*owner))
            };
            let args = args
                .iter()
                .map(|e| expr(e).map(|r| r.0))
                .collect::<Result<Vec<_>>>()?
                .join(", ");
            if name == ".ctor" {
                return Err(Error::limitation(
                    "Constructor chaining or in-place value construction remains explicit in IL",
                ));
            }
            if let Some(property) = accessor {
                let property_name = source_reference(*token, &identifier(&property.name));
                if property.getter && args.is_empty() {
                    (format!("{receiver}.{property_name}"), PRIMARY)
                } else if !property.getter {
                    (format!("{receiver}.{property_name} = {args}"), ASSIGNMENT)
                } else {
                    return Err(Error::limitation("Unsupported property signature"));
                }
            } else {
                let generic = if generic_args.is_empty() {
                    std::string::String::new()
                } else {
                    format!(
                        "<{}>",
                        generic_args
                            .iter()
                            .map(|t| type_name(a, t, context))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                (
                    format!(
                        "{receiver}.{}{generic}({args})",
                        source_reference(*token, &identifier(name))
                    ),
                    PRIMARY,
                )
            }
        }
        New { owner, args } => {
            if let [
                target,
                Expr {
                    kind: FunctionPointer { token: method, .. },
                    ..
                },
            ] = args.as_slice()
            {
                return delegate(a, context, *owner, target, *method);
            }
            if args
                .iter()
                .any(|e| matches!(e.kind, FunctionPointer { .. }))
            {
                return Err(Error::limitation(
                    "Delegate construction is preserved in IL until method-group binding is proven",
                ));
            }
            (
                format!(
                    "new {}({})",
                    source_reference(*owner, &strip_arity(&a.resolve(*owner))),
                    args.iter()
                        .map(|e| expr(e).map(|r| r.0))
                        .collect::<Result<Vec<_>>>()?
                        .join(", ")
                ),
                PRIMARY,
            )
        }
        Field {
            token: field_token,
            owner,
            name,
            instance,
            ..
        } if property_of_backing_field(a, context, *owner, name).is_some() => {
            let property = property_of_backing_field(a, context, *owner, name).unwrap_or_default();
            let target = if let Some(i) = instance {
                receiver(i)?
            } else {
                strip_arity(&a.resolve(*owner))
            };
            (
                format!(
                    "{target}.{}",
                    source_reference(*field_token, &identifier(&property))
                ),
                PRIMARY,
            )
        }
        Field {
            token: field_token,
            owner,
            name,
            instance,
            ..
        } => (
            format!(
                "{}.{}",
                if let Some(i) = instance {
                    receiver(i)?
                } else {
                    strip_arity(&a.resolve(*owner))
                },
                source_reference(*field_token, &identifier(name))
            ),
            PRIMARY,
        ),
        Index { array, index } => (format!("{}[{}]", receiver(array)?, expr(index)?.0), PRIMARY),
        NewArray { element, length } => (
            format!(
                "new {}[{}]",
                type_name(a, element, context),
                expr(length)?.0
            ),
            PRIMARY,
        ),
        ArrayInitializer { element, values } => {
            let elements = values
                .iter()
                .map(|e| expr(e).map(|r| r.0))
                .collect::<Result<Vec<_>>>()?;
            let contents = if values.len() <= 4 {
                format!("{{ {} }}", elements.join(", "))
            } else {
                format!("\n{{\n    {}\n}}", elements.join(",\n    "))
            };
            (
                format!(
                    "new {}[{}]{}{contents}",
                    type_name(a, element, context),
                    values.len(),
                    if values.len() <= 4 { " " } else { "" }
                ),
                PRIMARY,
            )
        }
        Length(v) => (format!("(nuint){}.Length", receiver(v)?), UNARY),
        Cast {
            mode,
            value,
            target,
        } => {
            let t = type_name(a, target, context);
            // Integer constants convert at compile time: (uint)(-1) is uint.MaxValue.
            if let Some(n) = integer_constant(e) {
                return Ok(match (t.as_str(), n) {
                    ("uint" | "System.UInt32", 0xffff_ffff) => {
                        ("uint.MaxValue".to_owned(), PRIMARY)
                    }
                    ("ulong" | "System.UInt64", 0xffff_ffff_ffff_ffff) => {
                        ("ulong.MaxValue".to_owned(), PRIMARY)
                    }
                    ("uint" | "System.UInt32", n) => (format!("{n}u"), PRIMARY),
                    ("ulong" | "System.UInt64", n) => (format!("{n}UL"), PRIMARY),
                    ("long" | "System.Int64", n) if n < 0 => (format!("{n}L"), UNARY),
                    ("long" | "System.Int64", n) => (format!("{n}L"), PRIMARY),
                    ("int" | "System.Int32", n) => {
                        (n.to_string(), if n < 0 { UNARY } else { PRIMARY })
                    }
                    ("char" | "System.Char", n) if (0..=0xffff).contains(&n) => {
                        (char_literal(n), PRIMARY)
                    }
                    (_, n) if n < 0 => (format!("({t})({n})"), UNARY),
                    (_, n) => (format!("({t}){n}"), UNARY),
                });
            }
            // (int)(nuint)array.Length is the CIL shape of array.Length.
            if let Length(array) = &value.kind
                && matches!(t.as_str(), "int" | "System.Int32")
            {
                return Ok((format!("{}.Length", receiver(array)?), PRIMARY));
            }
            let v = expr(value)?;
            match mode.as_str() {
                "as" => (format!("{} as {t}", wrap(v, 11)), 10),
                // Boxing a value already of the boxed type needs only the object conversion.
                "box" if type_name(a, &value.ty, context) == t => {
                    (format!("(object){}", cast_operand(v)), UNARY)
                }
                "box" => (format!("(object)({t}){}", cast_operand(v)), UNARY),
                "checked_unsigned" => {
                    let u = unsigned_type(&value.ty)?;
                    (
                        format!("checked(({t})unchecked(({u}){}))", cast_operand(v)),
                        PRIMARY,
                    )
                }
                "checked" | "unchecked" => (format!("{mode}(({t}){})", cast_operand(v)), PRIMARY),
                _ => (format!("({t}){}", cast_operand(v)), UNARY),
            }
        }
        Address(v) => (format!("ref {}", expr(v)?.0), PRIMARY),
        Deref(v) => {
            if let Address(inner) = &v.kind {
                expr(inner)?
            } else if let Type::ByRef(_) = &v.ty {
                expr(v)?
            } else {
                return Err(Error::limitation(
                    "Unverified indirect memory access remains in IL",
                ));
            }
        }
        Default(t) => (format!("default({})", type_name(a, t, context)), PRIMARY),
        FunctionPointer { .. } => {
            return Err(Error::limitation(
                "Raw managed function pointer has no safe source-level binding yet",
            ));
        }
        TypeToken(t) => match t >> 24 {
            1 | 2 | 27 => (format!("typeof({}).TypeHandle", a.resolve(*t)), PRIMARY),
            _ => {
                return Err(Error::limitation(
                    "Method/field runtime handles remain in IL",
                ));
            }
        },
        SizeOf(t) => (format!("sizeof({})", type_name(a, t, context)), PRIMARY),
        Await(value) => (format!("await {}", at(value, UNARY)?), UNARY),
    })
}
/// In display views an auto-property's backing field is written as the property, as
/// C# source shows it. That needs the property to be shown as `{ get; set; }` (its
/// accessors have the compiler's exact shapes) and never applies inside those accessors.
/// Exported sources keep the field, since they write accessors with explicit bodies.
fn property_of_backing_field(
    a: &Assembly,
    context: u32,
    owner: u32,
    field: &str,
) -> Option<String> {
    if !crate::assembly::display_view() {
        return None;
    }
    let property = field.strip_prefix('<')?.strip_suffix(">k__BackingField")?;
    let node = a
        .nodes
        .iter()
        .find(|n| n.token >> 24 == 23 && n.parent == owner && n.name == property)?;
    let accessors = a.semantics.get(&node.token)?;
    let field_token = a
        .nodes
        .iter()
        .find(|n| n.token >> 24 == 4 && n.parent == owner && n.name == field)?
        .token;
    let automatic = accessors.iter().all(|(semantics, m)| {
        matches!(semantics, 1 | 2)
            && crate::typeview::backing_field(a, *m, *semantics == 1) == Some(field_token)
    });
    (automatic && !accessors.iter().any(|(_, m)| *m == context)).then(|| property.to_owned())
}
thread_local! {
    // Set while rendering an iterator, where a bare return is `yield break`.
    static ITERATOR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// Source for an async or iterator method rebuilt from its state machine's MoveNext.
pub fn state_machine_source(
    a: &Assembly,
    token: u32,
    lowered: &crate::statemachine::Lowered,
    ast: &[Ast],
) -> Result<MethodSource> {
    let mut declaration = a.declaration(token)?;
    let method = a.method_ref(token)?;
    if matches!(lowered.kind, crate::statemachine::Kind::Async) {
        // `async` precedes the return type: public static async Task<int> Later(int value)
        let returns = format!(
            "{} {}",
            a.format_type(&method.signature.return_type, token),
            identifier(&method.name)
        );
        if let Some(i) = declaration.find(&returns) {
            declaration.insert_str(i, "async ");
        }
    }
    let iterator = matches!(lowered.kind, crate::statemachine::Kind::Iterator);
    ITERATOR.with(|i| i.set(iterator));
    let raw = csharp_body(a, token, &lowered.body, &lowered.ir, &lowered.cfg, ast);
    ITERATOR.with(|i| i.set(false));
    let (body, references) = extract_references(&raw?)?;
    let shift = declaration.encode_utf16().count() + 1;
    Ok(MethodSource {
        declaration,
        body,
        references: references
            .into_iter()
            .map(|mut r| {
                r.start += shift;
                r.end += shift;
                r
            })
            .collect(),
    })
}
thread_local! {
    static LAMBDA_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
/// A compiler-generated lambda (`<Outer>b__N`) becomes a C# lambda with its body
/// inlined; any other target is written as a method group.
fn delegate(
    a: &Assembly,
    context: u32,
    owner: u32,
    target: &Expr,
    method: u32,
) -> Result<(std::string::String, u8)> {
    let m = a.method_ref(method)?;
    let delegate_type = strip_arity(&a.resolve(owner));
    if !(m.name.starts_with('<') && m.name.contains(">b__")) || method >> 24 != 6 {
        let receiver = match &target.kind {
            ExprKind::Null => strip_arity(&a.resolve(m.owner)),
            _ => render(a, context, target).map(|r| wrap(r, PRIMARY))?,
        };
        return Ok((
            format!("new {delegate_type}({receiver}.{})", identifier(&m.name)),
            PRIMARY,
        ));
    }
    if LAMBDA_DEPTH.with(|d| d.get()) >= 8 {
        return Err(Error::limitation("Lambda nesting exceeds 8 levels"));
    }
    // The nested decompilation reuses the per-method naming table; keep the outer one.
    let saved = LOCAL_NAMES.with(|names| names.borrow().clone());
    LAMBDA_DEPTH.with(|d| d.set(d.get() + 1));
    let inner =
        crate::decompile::decompile_with(a, method, &|t| crate::resolution::local_accessor(a, t));
    LAMBDA_DEPTH.with(|d| d.set(d.get() - 1));
    LOCAL_NAMES.with(|names| *names.borrow_mut() = saved);
    let inner = inner?;
    let body = inner
        .source_body
        .ok_or_else(|| Error::limitation("The lambda body remains in IL"))?;
    // Fields of a closure class are the captured variables themselves.
    let closure = strip_arity(&a.resolve_full(m.owner)).contains("DisplayClass");
    let mut body = body;
    if closure {
        for field in a
            .nodes
            .iter()
            .filter(|n| n.parent == m.owner && n.token >> 24 == 4)
        {
            let name = identifier(&field.name);
            body = body.replace(&format!("this.{name}"), &name);
        }
    }
    let parameters = a.parameter_names(method, &m.signature);
    let head = match parameters.as_slice() {
        [single] => single.clone(),
        many => format!("({})", many.join(", ")),
    };
    let lines: Vec<&str> = body.lines().collect();
    // `{ return E; }` becomes an expression lambda.
    if let [open, statement, close] = lines.as_slice()
        && open.trim() == "{"
        && close.trim() == "}"
        && let Some(value) = statement
            .trim()
            .strip_prefix("return ")
            .and_then(|v| v.strip_suffix(';'))
    {
        return Ok((format!("{head} => {value}"), ASSIGNMENT));
    }
    Ok((format!("{head} =>\n{}", body.trim_end()), ASSIGNMENT))
}
/// An integer constant stored into an enum-typed destination, written as dnSpy does:
/// the matching member (`ByteMode.Ready`), or an explicit cast when no member matches.
/// CIL stores enums as their underlying integers; C# only converts 0 implicitly.
fn enum_constant(a: &Assembly, value: &Expr, target: &Type) -> Option<std::string::String> {
    let ExprKind::Constant(text) = &value.kind else {
        return None;
    };
    let number: i128 = text.trim_end_matches(['L', 'U', 'u', 'l']).parse().ok()?;
    let Type::ValueType(enum_token) = target else {
        return None;
    };
    // Local enums and framework enums known from the built-in summary.
    let members = crate::bcl::members_of(a, *enum_token)?;
    let enum_name = strip_arity(&a.resolve(*enum_token));
    Some(crate::bcl::enum_text(&enum_name, &members, number))
}
/// Whether `==` on this static type could bind a user-defined operator. Only `object`,
/// arrays and local classes whose local hierarchy declares no operator are known not to.
fn may_overload_equality(a: &Assembly, ty: &Type) -> bool {
    let mut token = match ty {
        Type::Primitive(name) => return name != "object",
        Type::Array(..) | Type::MultiArray(..) => return false,
        Type::Named(t) => *t,
        Type::GenericInstance { base, .. } => match base.as_ref() {
            Type::Named(t) => *t,
            _ => return true,
        },
        _ => return true,
    };
    for _ in 0..32 {
        if a.resolve_full(token) == "System.Object" {
            return false;
        }
        if token >> 24 != 2 {
            return true;
        }
        if a.nodes.iter().any(|n| {
            n.parent == token
                && n.token >> 24 == 6
                && matches!(n.name.as_str(), "op_Equality" | "op_Inequality")
        }) {
            return true;
        }
        let Some(row) = a.metadata().type_defs.get((token & 0xffffff) as usize - 1) else {
            return true;
        };
        token = crate::assembly::coded(row.extends);
        if token & 0xffffff == 0 {
            return true;
        }
    }
    true
}
/// Whether a local class or one of its local bases declares `==`/`!=`.
fn declares_equality(a: &Assembly, ty: &Type) -> bool {
    let Type::Named(mut token) = *ty else {
        return false;
    };
    for _ in 0..32 {
        if token >> 24 != 2 {
            return false;
        }
        if a.nodes.iter().any(|n| {
            n.parent == token
                && n.token >> 24 == 6
                && matches!(n.name.as_str(), "op_Equality" | "op_Inequality")
        }) {
            return true;
        }
        let Some(row) = a.metadata().type_defs.get((token & 0xffffff) as usize - 1) else {
            return false;
        };
        token = crate::assembly::coded(row.extends);
    }
    false
}
/// A C# character literal for a UTF-16 code unit.
fn char_literal(n: i128) -> std::string::String {
    match u32::try_from(n).ok().and_then(char::from_u32) {
        Some('\'') => "'\\''".to_owned(),
        Some('\\') => "'\\\\'".to_owned(),
        Some(c) if c.is_ascii_graphic() || c == ' ' => format!("'{c}'"),
        _ => format!("'\\u{:04X}'", n.clamp(0, 0xffff)),
    }
}
fn is_char(ty: &Type) -> bool {
    *ty == Type::primitive("char")
}
fn is_string(a: &Assembly, ty: &Type) -> bool {
    *ty == Type::primitive("string")
        || matches!(ty, Type::Named(t) if a.resolve_full(*t) == "System.String")
}
/// An integer constant written for its destination type: enum members
/// (`ByteMode.Ready`) and character literals (`'a'`), as dnSpy shows them.
fn typed_constant(a: &Assembly, value: &Expr, target: &Type) -> Option<std::string::String> {
    if is_char(target)
        && let ExprKind::Constant(text) = &value.kind
        && let Ok(n) = text.parse::<i128>()
    {
        return Some(char_literal(n));
    }
    enum_constant(a, value, target)
}
// `typeof` operand: open generics are written List<> / Dictionary<,>.
fn typeof_name(a: &Assembly, token: u32) -> std::string::String {
    let name = a.resolve(token);
    match (token >> 24, name.rsplit_once('`')) {
        (1 | 2, Some((base, arity))) if arity.chars().all(|c| c.is_ascii_digit()) => {
            let n: usize = arity.parse().unwrap_or(1);
            format!("{base}<{}>", ",".repeat(n.saturating_sub(1)))
        }
        _ => strip_arity(&name),
    }
}
// C# operators behind op_ methods; calling them by name is not legal C#.
fn binary_operator(name: &str) -> Option<&'static str> {
    Some(match name {
        "op_Addition" => "+",
        "op_Subtraction" => "-",
        "op_Multiply" => "*",
        "op_Division" => "/",
        "op_Modulus" => "%",
        "op_Equality" => "==",
        "op_Inequality" => "!=",
        "op_LessThan" => "<",
        "op_GreaterThan" => ">",
        "op_LessThanOrEqual" => "<=",
        "op_GreaterThanOrEqual" => ">=",
        "op_BitwiseAnd" => "&",
        "op_BitwiseOr" => "|",
        "op_ExclusiveOr" => "^",
        "op_LeftShift" => "<<",
        "op_RightShift" => ">>",
        _ => return None,
    })
}
fn unary_operator(name: &str) -> Option<&'static str> {
    Some(match name {
        "op_UnaryNegation" => "-",
        "op_UnaryPlus" => "+",
        "op_LogicalNot" => "!",
        "op_OnesComplement" => "~",
        _ => return None,
    })
}
/// The value of an integer constant after the conversions applied to it, when every
/// step is a plain integer conversion that the C# compiler would evaluate itself.
fn integer_constant(e: &Expr) -> Option<i128> {
    let width = |t: &Type| -> Option<(u32, bool)> {
        match t {
            Type::Primitive(n) => Some(match n.as_str() {
                "sbyte" => (8, true),
                "byte" => (8, false),
                "short" => (16, true),
                "ushort" => (16, false),
                "int" => (32, true),
                "uint" => (32, false),
                "char" => (16, false),
                "long" => (64, true),
                "ulong" => (64, false),
                _ => return None,
            }),
            _ => None,
        }
    };
    match &e.kind {
        ExprKind::Cast {
            mode,
            value,
            target,
        } if matches!(mode.as_str(), "unchecked" | "convert") => {
            let n = match &value.kind {
                ExprKind::Constant(c) => c
                    .trim_end_matches(['L', 'U', 'u', 'l'])
                    .parse::<i128>()
                    .ok()?,
                _ => integer_constant(value)?,
            };
            let (bits, signed) = width(target)?;
            let masked = n & ((1i128 << bits) - 1);
            Some(if signed && masked >> (bits - 1) == 1 {
                masked - (1i128 << bits)
            } else {
                masked
            })
        }
        _ => None,
    }
}
// Compound forms are used only for side-effect-free targets, so evaluating the
// target once instead of twice cannot change behavior.
fn compound(a: &Assembly, token: u32, target: &Expr, value: &Expr) -> Result<Option<String>> {
    let simple = match &target.kind {
        ExprKind::Variable(_) => true,
        ExprKind::Field { instance, .. } => instance
            .as_ref()
            .is_none_or(|i| matches!(i.kind, ExprKind::Variable(_))),
        _ => false,
    };
    let ExprKind::Binary {
        op,
        left,
        right,
        checked: false,
        unsigned: false,
    } = &value.kind
    else {
        return Ok(None);
    };
    if !simple
        || !matches!(
            op.as_str(),
            "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>"
        )
    {
        return Ok(None);
    }
    let target_text = expression(a, token, target)?;
    if expression(a, token, left)? != target_text || left.ty != target.ty {
        return Ok(None);
    }
    if matches!(op.as_str(), "+" | "-") && matches!(&right.kind, ExprKind::Constant(c) if c == "1")
    {
        return Ok(Some(format!("{target_text}{op}{op};")));
    }
    Ok(Some(format!(
        "{target_text} {op}= {};",
        expression(a, token, right)?
    )))
}
fn statement(a: &Assembly, token: u32, s: &Statement) -> Result<String> {
    Ok(match s {
        Statement::Let { name, ty, value } => format!(
            "{} {} = {};",
            type_name(a, ty, token),
            LOCAL_NAMES
                .with(|names| names.borrow().get(name).cloned())
                .unwrap_or_else(|| name.clone()),
            expression(a, token, value)?
        ),
        Statement::Assign { target, value } => match compound(a, token, target, value)? {
            Some(text) => text,
            None => format!(
                "{} = {};",
                expression(a, token, target)?,
                match typed_constant(a, value, &target.ty) {
                    Some(text) => text,
                    None => expression(a, token, value)?,
                }
            ),
        },
        Statement::Yield(value) => format!("yield return {};", expression(a, token, value)?),
        // Discarding a variable or constant has no effect; IL `pop` produces these.
        Statement::Evaluate(Expr {
            kind: ExprKind::Variable(_) | ExprKind::Constant(_) | ExprKind::Null,
            ..
        }) => std::string::String::new(),
        Statement::Evaluate(e) => {
            let v = expression(a, token, e)?;
            // Invocations, object creation and await are statements of their own.
            let invocation = match &e.kind {
                ExprKind::Call { accessor, .. } => accessor.as_ref().is_none_or(|p| !p.getter),
                ExprKind::New { .. } | ExprKind::Await(_) => true,
                _ => false,
            };
            if invocation {
                return Ok(format!("{v};"));
            }
            if e.ty.is_void() {
                format!("{v};")
            } else {
                format!("_ = {v};")
            }
        }
    })
}
fn line(out: &mut String, indent: usize, text: &str) {
    for text in text.lines() {
        let _ = writeln!(out, "{}{}", "    ".repeat(indent), text);
    }
}
fn nodes(
    a: &Assembly,
    token: u32,
    ast: &[Ast],
    out: &mut String,
    indent: usize,
    cfg: &ControlFlowGraph,
) -> Result<()> {
    for n in ast {
        match n {
            Ast::Statement(s) => line(out, indent, &statement(a, token, s)?),
            Ast::ParallelAssign { targets, values } => {
                let render = |items: &[Expr]| -> Result<String> {
                    Ok(items
                        .iter()
                        .map(|e| expression(a, token, e))
                        .collect::<Result<Vec<_>>>()?
                        .join(", "))
                };
                line(
                    out,
                    indent,
                    &format!("({}) = ({});", render(targets)?, render(values)?),
                );
            }
            Ast::Label(id) => line(out, indent, &format!("IL_{:04X}:;", cfg.blocks[*id].start)),
            Ast::Goto(id) => line(
                out,
                indent,
                &format!("goto IL_{:04X};", cfg.blocks[*id].start),
            ),
            Ast::Return(None) if ITERATOR.with(|i| i.get()) => line(out, indent, "yield break;"),
            Ast::Return(v) => {
                let returns = a.method_ref(token)?.signature.return_type;
                let value = match v {
                    Some(e) => Some(match typed_constant(a, e, &returns) {
                        Some(text) => text,
                        None => expression(a, token, e)?,
                    }),
                    None => None,
                };
                line(
                    out,
                    indent,
                    &format!(
                        "return{};",
                        value.map(|s| format!(" {s}")).unwrap_or_default()
                    ),
                )
            }
            Ast::Throw(v) => line(
                out,
                indent,
                &format!(
                    "throw{};",
                    v.as_ref()
                        .map(|e| expression(a, token, e).map(|s| format!(" {s}")))
                        .transpose()?
                        .unwrap_or_default()
                ),
            ),
            Ast::Break => line(out, indent, "break;"),
            Ast::Try { body, handlers } => {
                line(out, indent, "try");
                line(out, indent, "{");
                nodes(a, token, body, out, indent + 1, cfg)?;
                line(out, indent, "}");
                for h in handlers {
                    if h.kind == "finally" {
                        line(out, indent, "finally");
                    } else {
                        let variable = h.variable.as_ref().map(|v| {
                            LOCAL_NAMES
                                .with(|names| names.borrow().get(v).cloned())
                                .unwrap_or_else(|| v.clone())
                        });
                        line(
                            out,
                            indent,
                            &format!(
                                "catch ({}{})",
                                a.resolve(h.catch_type.unwrap_or(0)),
                                variable.map(|v| format!(" {v}")).unwrap_or_default()
                            ),
                        );
                    }
                    line(out, indent, "{");
                    nodes(a, token, &h.body, out, indent + 1, cfg)?;
                    line(out, indent, "}");
                }
            }
            // `if (c) {} else { X }` reads better as `if (!c) { X }`.
            Ast::If { condition, yes, no } if yes.is_empty() && !no.is_empty() => {
                let negated = Expr {
                    ty: condition.ty.clone(),
                    kind: ExprKind::Unary {
                        op: "!".to_owned(),
                        value: Box::new(condition.clone()),
                    },
                };
                line(
                    out,
                    indent,
                    &format!("if ({})", expression(a, token, &negated)?),
                );
                line(out, indent, "{");
                nodes(a, token, no, out, indent + 1, cfg)?;
                line(out, indent, "}");
            }
            Ast::If { condition, yes, no } => {
                line(
                    out,
                    indent,
                    &format!("if ({})", expression(a, token, condition)?),
                );
                line(out, indent, "{");
                nodes(a, token, yes, out, indent + 1, cfg)?;
                line(out, indent, "}");
                // After a branch that always leaves, the else body follows directly.
                if matches!(yes.last(), Some(Ast::Return(_) | Ast::Throw(_))) {
                    nodes(a, token, no, out, indent, cfg)?;
                } else if !no.is_empty() {
                    line(out, indent, "else");
                    line(out, indent, "{");
                    nodes(a, token, no, out, indent + 1, cfg)?;
                    line(out, indent, "}");
                }
            }
            Ast::While { condition, body } => {
                line(
                    out,
                    indent,
                    &format!("while ({})", expression(a, token, condition)?),
                );
                line(out, indent, "{");
                nodes(a, token, body, out, indent + 1, cfg)?;
                line(out, indent, "}");
            }
            Ast::For {
                initializer,
                condition,
                increment,
                body,
            } => {
                line(
                    out,
                    indent,
                    &format!(
                        "for ({}; {}; {})",
                        statement(a, token, initializer)?.trim_end_matches(';'),
                        expression(a, token, condition)?,
                        statement(a, token, increment)?.trim_end_matches(';')
                    ),
                );
                line(out, indent, "{");
                nodes(a, token, body, out, indent + 1, cfg)?;
                line(out, indent, "}");
            }
            Ast::Switch {
                value,
                arms,
                default,
            } => {
                line(
                    out,
                    indent,
                    &format!("switch ({})", expression(a, token, value)?),
                );
                line(out, indent, "{");
                for (i, arm) in arms.iter().enumerate() {
                    line(out, indent + 1, &format!("case {i}:"));
                    nodes(a, token, arm, out, indent + 2, cfg)?;
                }
                line(out, indent + 1, "default:");
                nodes(a, token, default, out, indent + 2, cfg)?;
                line(out, indent, "}");
            }
        }
    }
    Ok(())
}
fn flat_block(
    ir: &IrBlock,
    cfg: &ControlFlowGraph,
    fallthrough: Option<usize>,
    labels: &HashSet<usize>,
) -> Vec<Ast> {
    let edge = |e: &Edge| -> Vec<Ast> {
        let mut s = crate::ast::edge_assignments(e, cfg);
        s.push(Ast::Goto(e.target));
        s
    };
    let mut out = if labels.contains(&ir.id) {
        vec![Ast::Label(ir.id)]
    } else {
        vec![]
    };
    out.extend(ir.statements.iter().cloned().map(Ast::Statement));
    out.push(match &ir.terminator {
        Terminator::Jump(e) => {
            out.extend(crate::ast::edge_assignments(e, cfg));
            if Some(e.target) != fallthrough {
                out.push(Ast::Goto(e.target));
            }
            return out;
        }
        Terminator::Condition { condition, yes, no } => Ast::If {
            condition: condition.clone(),
            yes: edge(yes),
            no: edge(no),
        },
        Terminator::Switch {
            value,
            arms,
            default,
        } => Ast::Switch {
            value: value.clone(),
            arms: arms.iter().map(edge).collect(),
            default: edge(default),
        },
        Terminator::Return(v) => Ast::Return(v.clone()),
        Terminator::Throw(v) => Ast::Throw(v.clone()),
        Terminator::EndFinally => return out,
    });
    out
}
// Simple contiguous EH groups are rendered with explicit labels; nested/overlapping regions fall back.
fn exception_body(
    a: &Assembly,
    token: u32,
    body: &MethodBody,
    ir: &TypedIr,
    cfg: &ControlFlowGraph,
    out: &mut String,
) -> Result<()> {
    let mut groups: BTreeMap<(u32, u32), Vec<_>> = BTreeMap::new();
    for e in &body.exceptions {
        if e.kind == "filter" || e.kind == "fault" {
            return Err(Error::limitation(
                "Exception filters/fault handlers remain in IL",
            ));
        }
        groups.entry((e.try_start, e.try_end)).or_default().push(e);
    }
    let mut previous_end = 0;
    for ((start, end), handlers) in &mut groups {
        handlers.sort_by_key(|e| e.handler_start);
        if *start < previous_end {
            return Err(Error::limitation(
                "Nested or overlapping exception regions require further structuring",
            ));
        }
        let mut next = *end;
        for e in handlers {
            if e.handler_start != next {
                return Err(Error::limitation(
                    "Noncontiguous exception layout requires further structuring",
                ));
            }
            next = e.handler_end;
        }
        previous_end = next;
    }
    // A completed try/catch naturally continues after all sibling handlers in
    // C#. Keep only branches that differ from this lexical fallthrough. In
    // particular, do not emit a goto from an outer block into a try statement.
    let fallthroughs: Vec<_> = ir
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let mut next = ir.blocks.get(i + 1).map(|n| n.offset)?;
            for ((_, end), handlers) in &groups {
                if next == *end || handlers.iter().any(|e| e.handler_end == next) {
                    next = handlers.last()?.handler_end;
                    break;
                }
            }
            // Only a forward lexical continuation; backward branches stay explicit.
            cfg.blocks
                .iter()
                .find(|n| n.start == next && n.start > b.offset)
                .map(|n| n.id)
        })
        .collect();
    let mut labels = HashSet::new();
    for (i, block) in ir.blocks.iter().enumerate() {
        match &block.terminator {
            Terminator::Jump(edge) if Some(edge.target) != fallthroughs[i] => {
                labels.insert(edge.target);
            }
            Terminator::Condition { yes, no, .. } => {
                labels.extend([yes.target, no.target]);
            }
            Terminator::Switch { arms, default, .. } => {
                labels.extend(arms.iter().map(|e| e.target));
                labels.insert(default.target);
            }
            _ => {}
        }
    }
    for (i, b) in ir.blocks.iter().enumerate() {
        let mut indent = 1;
        for ((start, end), handlers) in &groups {
            if b.offset == *start {
                line(out, 1, "try");
                line(out, 1, "{");
            }
            if b.offset >= *start && b.offset < *end {
                indent = 2;
            }
            for e in handlers {
                if b.offset == e.handler_start {
                    line(out, 1, "}");
                    if e.kind == "finally" {
                        line(out, 1, "finally");
                    } else {
                        line(
                            out,
                            1,
                            &format!(
                                "catch ({} exception_{:04X})",
                                a.resolve(e.catch_type.unwrap_or(0)),
                                e.handler_start
                            ),
                        );
                    }
                    line(out, 1, "{");
                    if e.kind == "catch" {
                        line(
                            out,
                            2,
                            &format!(
                                "stack_{:04X}_0 = exception_{:04X};",
                                e.handler_start, e.handler_start
                            ),
                        );
                    }
                }
                if b.offset >= e.handler_start && b.offset < e.handler_end {
                    indent = 2;
                }
            }
            if handlers.last().is_some_and(|e| e.handler_end == b.offset) {
                line(out, 1, "}");
            }
        }
        nodes(
            a,
            token,
            &flat_block(b, cfg, fallthroughs[i], &labels),
            out,
            indent,
            cfg,
        )?;
    }
    if groups
        .values()
        .any(|h| h.last().is_some_and(|e| e.handler_end == body.code_size))
    {
        line(out, 1, "}");
    }
    Ok(())
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct SourceReference {
    pub start: usize,
    pub end: usize,
    pub token: u32,
}
// Internal markers wrap only compiler-emitted identifiers, never user text.
// String literals escape control characters and identifiers sanitize them.
fn source_reference(token: u32, name: &str) -> String {
    format!("\0{token:08X}\u{1}{name}\u{2}")
}
fn extract_references(input: &str) -> Result<(String, Vec<SourceReference>)> {
    let mut output = String::new();
    let mut refs = Vec::new();
    let mut rest = input;
    let mut position = 0;
    while let Some(start) = rest.find('\0') {
        let prefix = &rest[..start];
        output.push_str(prefix);
        position += prefix.encode_utf16().count();
        rest = &rest[start + 1..];
        let (header, tail) = rest
            .split_once('\u{1}')
            .ok_or_else(|| Error::limitation("Invalid source reference marker"))?;
        if header.len() != 8 {
            return Err(Error::limitation("Invalid source reference token"));
        }
        let token = u32::from_str_radix(header, 16)
            .map_err(|_| Error::limitation("Invalid source reference token"))?;
        let (name, tail) = tail
            .split_once('\u{2}')
            .ok_or_else(|| Error::limitation("Unterminated source reference"))?;
        output.push_str(name);
        let end = position + name.encode_utf16().count();
        refs.push(SourceReference {
            start: position,
            end,
            token,
        });
        position = end;
        rest = tail;
    }
    output.push_str(rest);
    Ok((output, refs))
}
pub struct MethodSource {
    pub references: Vec<SourceReference>,
    pub declaration: String,
    pub body: String,
}
pub fn method_source(
    a: &Assembly,
    token: u32,
    body: &MethodBody,
    ir: &TypedIr,
    cfg: &ControlFlowGraph,
    ast: &[Ast],
) -> Result<MethodSource> {
    let method = a.method_ref(token)?;
    let mut declaration = a.declaration(token)?;
    let mut nodes = ast;
    let reordered: Vec<Ast>;
    // Struct constructors need no base constructor call.
    let value_type = a
        .nodes
        .iter()
        .any(|n| n.token == method.owner && matches!(n.kind.as_str(), "struct" | "enum"));
    let chained = ast.iter().any(|n| {
        matches!(n, Ast::Statement(Statement::Evaluate(Expr { kind: ExprKind::Call { name, .. }, .. })) if name == ".ctor")
    });
    if method.name == ".ctor" && (chained || !value_type) {
        // Field stores that precede the base call (C# field initializers and primary
        // constructor parameters) may follow it only when the base constructor cannot
        // observe them: System.Object and System.Attribute run no derived code.
        let base_token = crate::assembly::coded(
            a.metadata().type_defs[(method.owner & 0xffffff) as usize - 1].extends,
        );
        let inert_base = matches!(
            a.resolve_full(base_token).as_str(),
            "System.Object" | "System.Attribute"
        );
        let first_call = ast
            .iter()
            .position(|n| matches!(n, Ast::Statement(Statement::Evaluate(Expr { kind: ExprKind::Call { name, .. }, .. })) if name == ".ctor"))
            .unwrap_or(0);
        let leading_stores = ast[..first_call].iter().all(|n| {
            matches!(n, Ast::Statement(Statement::Let { .. }))
                || matches!(n, Ast::Statement(Statement::Assign {
                    target: Expr { kind: ExprKind::Field { instance: Some(instance), .. }, .. },
                    ..
                }) if matches!(&instance.kind, ExprKind::Variable(v) if v == "this"))
        });
        let ast: &[Ast] = if first_call > 0 && inert_base && leading_stores {
            reordered = ast[first_call..first_call + 1]
                .iter()
                .chain(&ast[..first_call])
                .chain(&ast[first_call + 1..])
                .cloned()
                .collect();
            &reordered
        } else {
            ast
        };
        // Constructor initialization must precede the body in C#. Hoist only
        // a first, direct this/base call with parameters or constant expressions.
        let Some(Ast::Statement(Statement::Evaluate(Expr {
            kind:
                ExprKind::Call {
                    owner,
                    name,
                    instance: Some(instance),
                    args,
                    ..
                },
            ..
        }))) = ast.first()
        else {
            return Err(Error::limitation(
                "Constructor initialization cannot be safely separated from this body",
            ));
        };
        let base = crate::assembly::coded(
            a.metadata().type_defs[(method.owner & 0xffffff) as usize - 1].extends,
        );
        if name != ".ctor"
            || !matches!(&instance.kind, ExprKind::Variable(n) if n == "this")
            || (*owner != method.owner && *owner != base)
            || !body.exceptions.is_empty()
        {
            return Err(Error::limitation(
                "Constructor chaining has an unsupported target or control flow",
            ));
        }
        let parameters = a.parameter_names(token, &method.signature);
        fn permitted(expr: &Expr, parameters: &[String], depth: usize) -> bool {
            if depth > 48 {
                return false;
            }
            match &expr.kind {
                ExprKind::Variable(n) => parameters.contains(n),
                ExprKind::Constant(_) | ExprKind::String(_) | ExprKind::Null => true,
                ExprKind::Unary { value, .. } | ExprKind::Cast { value, .. } => {
                    permitted(value, parameters, depth + 1)
                }
                ExprKind::Binary { left, right, .. } => {
                    permitted(left, parameters, depth + 1)
                        && permitted(right, parameters, depth + 1)
                }
                _ => false,
            }
        }
        if args.iter().any(|arg| !permitted(arg, &parameters, 0)) {
            return Err(Error::limitation(
                "Constructor initializer uses temporary values or side effects that cannot be hoisted",
            ));
        }
        let args = args
            .iter()
            .map(|arg| expression(a, token, arg))
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        // C# calls a parameterless base constructor implicitly.
        if *owner == method.owner || !args.is_empty() {
            declaration.push_str(&format!(
                " : {}({args})",
                if *owner == method.owner {
                    "this"
                } else {
                    "base"
                }
            ));
        }
        nodes = &ast[1..];
    } else if method.name == ".cctor" {
        declaration = declaration
            .strip_prefix("private ")
            .unwrap_or(&declaration)
            .to_owned();
    }
    let raw_body = csharp_body(a, token, body, ir, cfg, nodes)?;
    let (declaration, mut references) = extract_references(&declaration)?;
    let (body, body_references) = extract_references(&raw_body)?;
    let shift = declaration.encode_utf16().count() + 1;
    references.extend(body_references.into_iter().map(|mut r| {
        r.start += shift;
        r.end += shift;
        r
    }));
    Ok(MethodSource {
        declaration,
        body,
        references,
    })
}
use crate::tidy::{depth, mentions};
fn terminal(line: &str) -> bool {
    let line = line.trim();
    line.ends_with(';')
        && (line.starts_with("return") || line.starts_with("throw") || line.starts_with("goto "))
}
// True when lines[close] ends a switch whose every section leaves, so no path falls
// out of the switch to the following statement.
fn switch_never_falls_out(lines: &[String], close: usize) -> bool {
    let d = depth(&lines[close]);
    if lines[close].trim() != "}" {
        return false;
    }
    let Some(open) = (0..close)
        .rev()
        .find(|i| depth(&lines[*i]) == d && lines[*i].trim() == "{")
    else {
        return false;
    };
    if open == 0 || !lines[open - 1].trim_start().starts_with("switch (") {
        return false;
    }
    let mut sections = 0;
    for i in open + 1..=close {
        let line = lines[i].trim();
        let boundary = i == close
            || depth(&lines[i]) == d + 1 && (line.starts_with("case ") || line == "default:");
        if boundary && i > open + 1 {
            if !terminal(&lines[i - 1]) {
                return false;
            }
            sections += 1;
        }
    }
    // A switch without default can skip every case.
    sections > 0 && (open + 1..close).any(|i| lines[i].trim() == "default:")
}
/// Inlines a label block at its only `goto` when nothing else can reach it: the label
/// has one jump, the preceding statement cannot fall through, and the block itself
/// ends by leaving. This turns `case 0: goto IL_0018; … IL_0018:; return "zero";`
/// into `case 0: return "zero";`.
fn inline_labels(body: String) -> String {
    let mut lines: Vec<String> = body.lines().map(str::to_owned).collect();
    for _ in 0..256 {
        let mut changed = false;
        for label_index in 1..lines.len() {
            let label_line = lines[label_index].trim();
            let Some(label) = label_line.strip_suffix(":;") else {
                continue;
            };
            if !label.starts_with("IL_")
                || !(terminal(&lines[label_index - 1])
                    || switch_never_falls_out(&lines, label_index - 1))
            {
                continue;
            }
            let jump = format!("goto {label};");
            let sites: Vec<usize> = (0..lines.len())
                .filter(|i| lines[*i].trim() == jump)
                .collect();
            let [site] = sites[..] else { continue };
            let d = depth(&lines[label_index]);
            let end = (label_index + 1..lines.len())
                .find(|i| {
                    let l = &lines[*i];
                    depth(l) < d || depth(l) == d && l.trim().ends_with(":;")
                })
                .unwrap_or(lines.len());
            let block: Vec<String> = lines[label_index + 1..end].to_vec();
            // The block must end by leaving and contain no nested labels or the jump site.
            if block.is_empty()
                || !terminal(block.last().unwrap())
                || depth(block.last().unwrap()) != d
                || (label_index..end).contains(&site)
                || block.iter().any(|l| l.trim().ends_with(":;"))
            {
                continue;
            }
            let g = depth(&lines[site]);
            let moved: Vec<String> = block
                .iter()
                .map(|l| {
                    let extra = depth(l).saturating_sub(d);
                    format!("{}{}", "    ".repeat(g + extra), l.trim_start())
                })
                .collect();
            lines.drain(label_index..end);
            let site = if site > label_index {
                site - (end - label_index)
            } else {
                site
            };
            lines.splice(site..=site, moved);
            changed = true;
            break;
        }
        if !changed {
            break;
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}
/// Moves `T name = default(T);` to the first write when every use of `name` lies in the
/// block containing that write, as ILSpy/dnSpy declare locals. Bodies with gotos keep
/// their top-level declarations because a jump could skip the new declaration.
fn merge_declarations(body: String, locals: &[(String, String)]) -> String {
    if body.contains("goto ") {
        return body;
    }
    let mut lines: Vec<String> = body.lines().map(str::to_owned).collect();
    for (ty, name) in locals {
        let declared = [
            format!("    {ty} {name} = default({ty});"),
            format!("    {ty} {name};"),
        ];
        let Some(declaration) = lines.iter().position(|l| declared.contains(l)) else {
            continue;
        };
        let uses: Vec<usize> = (0..lines.len())
            .filter(|i| *i != declaration && mentions(&lines[*i], name))
            .collect();
        let Some(&first) = uses.first() else {
            continue;
        };
        let text = lines[first].trim_start();
        let d = depth(&lines[first]);
        let assignment = format!("{name} = ");
        let header = format!("for ({name} = ");
        let (prefix, scope_end) = if let Some(rest) = text.strip_prefix(&header) {
            // for (n = x; …) keeps n inside the loop when the body is the only other user.
            let init = rest.split(';').next().unwrap_or(rest);
            if mentions(init, name) || lines.get(first + 1).map(|l| l.trim()) != Some("{") {
                continue;
            }
            let end = (first + 2..lines.len())
                .find(|i| depth(&lines[*i]) == d && lines[*i].trim() == "}");
            (header, end)
        } else if let Some(rest) = text.strip_prefix(&assignment) {
            // The whole statement, which may span lines, must not read the local.
            let Some(end) = (first..lines.len())
                .find(|i| depth(&lines[*i]) == d && lines[*i].trim_end().ends_with(';'))
            else {
                continue;
            };
            if mentions(rest, name) || (first + 1..=end).any(|i| mentions(&lines[i], name)) {
                continue;
            }
            // The enclosing block ends at the first closing brace one level out.
            let end = (first + 1..lines.len())
                .find(|i| depth(&lines[*i]) + 1 == d && lines[*i].trim() == "}");
            (assignment, end)
        } else {
            continue;
        };
        let Some(scope_end) = scope_end else { continue };
        if uses.iter().any(|i| *i > scope_end) {
            continue;
        }
        let indent = "    ".repeat(d);
        let rest = &lines[first][indent.len() + prefix.len()..];
        lines[first] = if prefix.starts_with("for (") {
            format!("{indent}for ({ty} {name} = {rest}")
        } else {
            format!("{indent}{ty} {name} = {rest}")
        };
        lines.remove(declaration);
    }
    // Drop the separator once every declaration has moved.
    if lines.len() > 2 && lines[0] == "{" && lines[1].is_empty() {
        lines.remove(1);
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}
/// Render an already structured method body for a compilation unit or accessor.
/// Export never reparses a displayed C# string to recover its body.
pub fn csharp_body(
    a: &Assembly,
    token: u32,
    body: &MethodBody,
    ir: &TypedIr,
    cfg: &ControlFlowGraph,
    ast: &[Ast],
) -> Result<String> {
    let mut out = String::new();
    let _names = name_locals(a, token, &ir.locals, &ir.local_names);
    let local = |i: usize| {
        LOCAL_NAMES
            .with(|names| names.borrow().get(&format!("local{i}")).cloned())
            .unwrap_or_else(|| format!("local{i}"))
    };
    // Structured exception regions render inline; other layouts keep the labeled form.
    let structured = body.exceptions.is_empty() || ast.iter().any(|n| matches!(n, Ast::Try { .. }));
    let catches = catch_variables(ast);
    name_catches(&catches);
    name_temporaries(a, token, ast);
    // A void method's final `return;` is implicit in C#, also at the end of its last
    // if/else branches.
    let mut owned = ast.to_vec();
    if structured {
        strip_final_returns(&mut owned);
    }
    let ast = owned.as_slice();
    line(&mut out, 0, "{");
    for (i, t) in ir.locals.iter().enumerate() {
        line(
            &mut out,
            1,
            &format!(
                "{} {}{};",
                type_name(a, t, token),
                local(i),
                if body.init_locals {
                    format!(" = default({})", type_name(a, t, token))
                } else {
                    String::new()
                }
            ),
        );
    }
    for b in &cfg.blocks {
        if let Some(entry) = &ir.stack.entry_stacks[b.id] {
            for (i, t) in entry.iter().enumerate() {
                let name = format!("stack_{:04X}_{i}", b.start);
                // A catch clause declares its own exception variable.
                if structured && catches.contains(&name) {
                    continue;
                }
                line(&mut out, 1, &format!("{} {name};", type_name(a, t, token)));
            }
        }
    }
    if !ir.locals.is_empty() {
        out.push('\n');
    }
    if structured {
        nodes(a, token, ast, &mut out, 1, cfg)?;
    } else {
        exception_body(a, token, body, ir, cfg, &mut out)?;
    }
    line(&mut out, 0, "}");
    let declared: Vec<(String, String)> = (0..ir.locals.len())
        .map(|i| (type_name(a, &ir.locals[i], token), local(i)))
        .collect();
    let mut lines: Vec<String> = inline_labels(out).lines().map(str::to_owned).collect();
    crate::tidy::stack_copies(&mut lines);
    crate::tidy::ref_aliases(&mut lines);
    crate::tidy::empty_else(&mut lines);
    crate::tidy::unused_catch_variables(&mut lines);
    let mut lines: Vec<String> = merge_declarations(lines.join("\n") + "\n", &declared)
        .lines()
        .map(str::to_owned)
        .collect();
    crate::tidy::closures(&mut lines);
    crate::tidy::null_conditional(&mut lines);
    crate::tidy::using_statements(&mut lines);
    crate::tidy::inline_temporaries(&mut lines);
    crate::tidy::compound_assignments(&mut lines);
    crate::tidy::while_to_for(&mut lines);
    crate::tidy::foreach_arrays(&mut lines);
    crate::tidy::unused_declarations(&mut lines);
    let out = lines.join("\n") + "\n";
    if out.len() > 4 * 1024 * 1024 {
        return Err(Error::limit("Reconstructed method exceeds 4 MiB"));
    }
    Ok(out)
}
pub fn il(a: &Assembly, token: u32, body: &MethodBody) -> String {
    let mut out = format!(
        "// {}\n// token 0x{token:08X}\n.maxstack {}\n",
        safe_comment(&a.resolve(token)),
        body.max_stack
    );
    if let Ok(locals) = a.locals(body)
        && !locals.is_empty()
    {
        let _ = writeln!(
            out,
            ".locals {}({})",
            if body.init_locals { "init " } else { "" },
            locals
                .iter()
                .enumerate()
                .map(|(i, t)| format!("[{i}] {} local{i}", a.format_type(t, token)))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    for i in &body.instructions {
        let operand = match &i.operand {
            Operand::None => String::new(),
            Operand::Integer(v) => v.to_string(),
            Operand::Float(v) => v.clone(),
            Operand::Variable(v) => v.to_string(),
            Operand::Token(t) => format!(
                "{} /* 0x{t:08X} */",
                safe_comment(i.resolved.as_deref().unwrap_or("unresolved"))
            ),
            Operand::Branch(t) => format!("IL_{t:04X}"),
            Operand::Switch(ts) => format!(
                "({})",
                ts.iter()
                    .map(|t| format!("IL_{t:04X}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let _ = writeln!(out, "IL_{:04X}:  {:<14} {}", i.offset, i.name, operand);
    }
    for e in &body.exceptions {
        let _ = writeln!(
            out,
            "// .try IL_{:04X}..IL_{:04X} {} {} handler IL_{:04X}..IL_{:04X}{}",
            e.try_start,
            e.try_end,
            e.kind,
            e.catch_type.map(|t| a.resolve(t)).unwrap_or_default(),
            e.handler_start,
            e.handler_end,
            e.filter_start
                .map(|f| format!(" filter IL_{f:04X}"))
                .unwrap_or_default()
        );
    }
    out
}
