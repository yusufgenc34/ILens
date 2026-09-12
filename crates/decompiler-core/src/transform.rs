//! Only adjacent, single-use temporary definitions are inlined. Evaluation order stays explicit.
use crate::ir::*;
use std::collections::HashMap;
fn children(e: &mut Expr, f: &mut impl FnMut(&mut Expr)) {
    use ExprKind::*;
    match &mut e.kind {
        Binary { left, right, .. } => {
            walk(left, f);
            walk(right, f);
        }
        Unary { value, .. }
        | Cast { value, .. }
        | Length(value)
        | Address(value)
        | Deref(value) => walk(value, f),
        Call { instance, args, .. } => {
            if let Some(i) = instance {
                walk(i, f);
            }
            for a in args {
                walk(a, f);
            }
        }
        New { args, .. } => {
            for a in args {
                walk(a, f)
            }
        }
        Field {
            instance: Some(i), ..
        }
        | FunctionPointer {
            instance: Some(i), ..
        } => walk(i, f),
        Index { array, index } => {
            walk(array, f);
            walk(index, f);
        }
        NewArray { length, .. } => walk(length, f),
        _ => {}
    }
}
fn walk(e: &mut Expr, f: &mut impl FnMut(&mut Expr)) {
    children(e, f);
    f(e);
}
pub fn statement_exprs(s: &mut Statement, f: &mut impl FnMut(&mut Expr)) {
    match s {
        Statement::Let { value, .. } | Statement::Evaluate(value) => walk(value, f),
        Statement::Assign { target, value } => {
            walk(target, f);
            walk(value, f);
        }
    }
}
pub fn terminator_exprs(t: &mut Terminator, f: &mut impl FnMut(&mut Expr)) {
    fn edge(e: &mut Edge, f: &mut impl FnMut(&mut Expr)) {
        for v in &mut e.values {
            walk(v, f);
        }
    }
    match t {
        Terminator::Jump(e) => edge(e, f),
        Terminator::Condition { condition, yes, no } => {
            walk(condition, f);
            edge(yes, f);
            edge(no, f);
        }
        Terminator::Switch {
            value,
            arms,
            default,
        } => {
            walk(value, f);
            for a in arms {
                edge(a, f);
            }
            edge(default, f);
        }
        Terminator::Return(Some(e)) | Terminator::Throw(Some(e)) => walk(e, f),
        _ => {}
    }
}
fn count(ir: &mut TypedIr) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    let mut f = |e: &mut Expr| {
        if let ExprKind::Variable(v) = &e.kind {
            *counts.entry(v.clone()).or_default() += 1;
        }
    };
    for b in &mut ir.blocks {
        for s in &mut b.statements {
            statement_exprs(s, &mut f);
        }
        terminator_exprs(&mut b.terminator, &mut f);
    }
    counts
}
fn contains(s: &mut Statement, name: &str) -> bool {
    let mut found = false;
    statement_exprs(s, &mut |e| {
        if matches!(&e.kind,ExprKind::Variable(n) if n==name) {
            found = true
        }
    });
    found
}
fn expression_depth(e: &Expr) -> usize {
    use ExprKind::*;
    1 + match &e.kind {
        Binary { left, right, .. }
        | Index {
            array: left,
            index: right,
        } => expression_depth(left).max(expression_depth(right)),
        Unary { value, .. }
        | Cast { value, .. }
        | Length(value)
        | Address(value)
        | Deref(value)
        | NewArray { length: value, .. } => expression_depth(value),
        Call { instance, args, .. } => instance
            .as_ref()
            .map_or(0, |e| expression_depth(e))
            .max(args.iter().map(expression_depth).max().unwrap_or(0)),
        New { args, .. } => args.iter().map(expression_depth).max().unwrap_or(0),
        Field { instance, .. } | FunctionPointer { instance, .. } => {
            instance.as_ref().map_or(0, |e| expression_depth(e))
        }
        _ => 0,
    }
}

pub fn simplify(ir: &mut TypedIr) {
    // Bound both transformation work and generated AST nesting on hostile instruction sequences.
    for _ in 0..32 {
        let counts = count(ir);
        let mut changed = false;
        for b in &mut ir.blocks {
            let mut i = 0;
            while i < b.statements.len() {
                let (name, value) = match &b.statements[i] {
                    Statement::Let { name, value, .. } if counts.get(name) == Some(&1) => {
                        (name.clone(), value.clone())
                    }
                    _ => {
                        i += 1;
                        continue;
                    }
                };
                let next_has = if let Some(s) = b.statements.get_mut(i + 1) {
                    contains(s, &name)
                } else {
                    let mut has = false;
                    terminator_exprs(&mut b.terminator, &mut |e| {
                        if matches!(&e.kind,ExprKind::Variable(n)if n==&name) {
                            has = true
                        }
                    });
                    has
                };
                if !next_has {
                    i += 1;
                    continue;
                }
                let depth = expression_depth(&value);
                let mut next_depth = 0;
                let mut measure = |e: &mut Expr| {
                    next_depth = next_depth.max(expression_depth(e));
                };
                if let Some(s) = b.statements.get_mut(i + 1) {
                    statement_exprs(s, &mut measure);
                } else {
                    terminator_exprs(&mut b.terminator, &mut measure);
                }
                if depth + next_depth > 48 {
                    i += 1;
                    continue;
                }
                // Do not move a definition into an assignment target (target evaluation precedes RHS).
                if let Some(Statement::Assign { target, .. }) = b.statements.get(i + 1) {
                    let mut t = target.clone();
                    let mut used = false;
                    walk(&mut t, &mut |e| {
                        if matches!(&e.kind,ExprKind::Variable(n)if n==&name) {
                            used = true
                        }
                    });
                    if used {
                        i += 1;
                        continue;
                    }
                }
                let mut replace = |e: &mut Expr| {
                    if matches!(&e.kind,ExprKind::Variable(n)if n==&name) {
                        *e = value.clone();
                    }
                };
                if let Some(s) = b.statements.get_mut(i + 1) {
                    statement_exprs(s, &mut replace);
                } else {
                    terminator_exprs(&mut b.terminator, &mut replace);
                }
                b.statements.remove(i);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Canonicalize bool comparisons and managed address/load pairs after inlining.
    let mut canonical = |e: &mut Expr| {
        if let ExprKind::Binary {
            op, left, right, ..
        } = &e.kind
            && let (ExprKind::Constant(l), ExprKind::Constant(r)) = (&left.kind, &right.kind)
            && let (Ok(l), Ok(r)) = (l.parse::<i32>(), r.parse::<i32>())
            && matches!(op.as_str(), "==" | "!=")
        {
            let result = if op == "==" { l == r } else { l != r };
            *e = Expr::constant(
                result.to_string(),
                crate::signature::Type::primitive("bool"),
            );
            return;
        }
        if let ExprKind::Deref(v) = &e.kind
            && let ExprKind::Address(inner) = &v.kind
        {
            *e = *inner.clone();
            return;
        }
        if let ExprKind::Binary {
            op,
            left,
            right,
            unsigned: false,
            ..
        } = &e.kind
            && left.ty.is_bool()
            && let ExprKind::Constant(v) = &right.kind
            && v == "0"
            && (op == "==" || op == "!=")
        {
            let value = *left.clone();
            *e = if op == "==" {
                Expr {
                    ty: crate::signature::Type::primitive("bool"),
                    kind: ExprKind::Unary {
                        op: "!".to_owned(),
                        value: Box::new(value),
                    },
                }
            } else {
                value
            };
        }
    };
    for b in &mut ir.blocks {
        for s in &mut b.statements {
            statement_exprs(s, &mut canonical);
        }
        terminator_exprs(&mut b.terminator, &mut canonical);
    }
}
