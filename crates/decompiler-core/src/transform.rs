//! Bounded, evaluation-order-aware simplification of typed IR.
use crate::ir::*;
mod arrays;
use std::collections::{HashMap, HashSet};
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
        | Deref(value)
        | Await(value) => walk(value, f),
        Call { instance, args, .. } => {
            if let Some(i) = instance {
                walk(i, f);
            }
            for a in args {
                walk(a, f);
            }
        }
        New { args, .. } | ArrayInitializer { values: args, .. } => {
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
        Statement::Let { value, .. } | Statement::Evaluate(value) | Statement::Yield(value) => {
            walk(value, f)
        }
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
        New { args, .. } | ArrayInitializer { values: args, .. } => {
            args.iter().map(expression_depth).max().unwrap_or(0)
        }
        Field { instance, .. } | FunctionPointer { instance, .. } => {
            instance.as_ref().map_or(0, |e| expression_depth(e))
        }
        _ => 0,
    }
}

// Taking a variable's address can let a call mutate it even without a visible
// assignment. Never propagate such reads across another evaluation.
fn exposed_variables(ir: &mut TypedIr) -> HashSet<String> {
    let mut exposed = HashSet::new();
    let mut scan = |e: &mut Expr| {
        if let ExprKind::Address(value) = &e.kind
            && let ExprKind::Variable(name) = &value.kind
        {
            exposed.insert(name.clone());
        }
    };
    for block in &mut ir.blocks {
        for statement in &mut block.statements {
            statement_exprs(statement, &mut scan);
        }
        terminator_exprs(&mut block.terminator, &mut scan);
    }
    exposed
}

fn movable_leaf(e: &Expr, exposed: &HashSet<String>) -> bool {
    match &e.kind {
        ExprKind::Variable(n) => !exposed.contains(n),
        ExprKind::Constant(_) | ExprKind::String(_) | ExprKind::Null => true,
        _ => false,
    }
}

// For an effectful definition, its use must be reached before any other effect,
// heap read or potentially throwing expression. Traversal follows C# evaluation
// order. Array stores need a fresh-array bounds proof before moving an
// effectful RHS across lvalue evaluation; unknown receivers stay explicit.
#[derive(PartialEq)]
enum Prefix {
    Pure,
    Found,
    Blocked,
}
fn evaluation_prefix(e: &Expr, name: &str, exposed: &HashSet<String>) -> Prefix {
    use ExprKind::*;
    if matches!(&e.kind, Variable(n) if n == name) {
        return Prefix::Found;
    }
    if movable_leaf(e, exposed) {
        return Prefix::Pure;
    }
    // Allocation happens before an array initializer's elements. Moving a call
    // into an element would reverse it with a possibly failing allocation.
    let children: Vec<&Expr> = match &e.kind {
        ArrayInitializer { .. } => return Prefix::Blocked,
        Binary { left, right, .. }
        | Index {
            array: left,
            index: right,
        } => vec![left, right],
        Unary { value, .. }
        | Cast { value, .. }
        | Length(value)
        | Address(value)
        | Deref(value)
        | NewArray { length: value, .. } => vec![value],
        Call { instance, args, .. } => instance
            .iter()
            .map(|v| v.as_ref())
            .chain(args.iter())
            .collect(),
        New { args, .. } => args.iter().collect(),
        Field { instance, .. } | FunctionPointer { instance, .. } => {
            instance.iter().map(|v| v.as_ref()).collect()
        }
        _ => vec![],
    };
    for child in children {
        let prefix = evaluation_prefix(child, name, exposed);
        if prefix != Prefix::Pure {
            return prefix;
        }
    }
    Prefix::Blocked
}
fn first_evaluation(
    statement: Option<&Statement>,
    terminator: &Terminator,
    name: &str,
    exposed: &HashSet<String>,
    array_store_proven: bool,
) -> bool {
    let mut expressions: Vec<&Expr> = vec![];
    if let Some(s) = statement {
        match s {
            Statement::Assign { target, value } => {
                match &target.kind {
                    ExprKind::Variable(_) => {}
                    ExprKind::Index { array, index } if array_store_proven => {
                        expressions.extend([array.as_ref(), index.as_ref()])
                    }
                    _ => expressions.push(target),
                }
                // Keep effectful assignment receivers in explicit temporaries;
                // only harmless copies are propagated into lvalues.
                if expressions
                    .iter()
                    .any(|e| evaluation_prefix(e, name, exposed) != Prefix::Pure)
                {
                    return false;
                }
                expressions.push(value);
            }
            Statement::Let { value, .. } | Statement::Evaluate(value) | Statement::Yield(value) => {
                expressions.push(value)
            }
        }
    } else {
        match terminator {
            // Edge values on conditional exits are not evaluated on every path.
            Terminator::Condition { condition, .. } => expressions.push(condition),
            Terminator::Switch { value, .. } => expressions.push(value),
            Terminator::Jump(edge) => expressions.extend(edge.values.iter()),
            Terminator::Return(Some(value)) | Terminator::Throw(Some(value)) => {
                expressions.push(value)
            }
            _ => {}
        }
    }
    for expression in expressions {
        match evaluation_prefix(expression, name, exposed) {
            Prefix::Found => return true,
            Prefix::Blocked => return false,
            Prefix::Pure => {}
        }
    }
    false
}

fn array_store_proven(
    statement: Option<&Statement>,
    before: usize,
    arrays: &HashMap<String, (usize, usize)>,
    writes: &HashMap<String, Vec<usize>>,
    exposed: &HashSet<String>,
) -> bool {
    let Some(Statement::Assign {
        target: Expr {
            kind: ExprKind::Index { array, index },
            ..
        },
        ..
    }) = statement
    else {
        return false;
    };
    let (ExprKind::Variable(name), ExprKind::Constant(index)) = (&array.kind, &index.kind) else {
        return false;
    };
    let Some(&(definition, length)) = arrays.get(name) else {
        return false;
    };
    definition < before
        && !exposed.contains(name)
        && index.parse::<usize>().is_ok_and(|index| index < length)
        && !writes.get(name).is_some_and(|positions| {
            let pos = positions.partition_point(|&p| p <= definition);
            positions.get(pos).is_some_and(|&p| p < before)
        })
}

pub fn simplify(ir: &mut TypedIr) {
    let exposed = exposed_variables(ir);
    // Bound transformation work and generated expression nesting on hostile IL.
    for _ in 0..32 {
        let counts = count(ir);
        let mut changed = false;
        for b in &mut ir.blocks {
            let mut uses = HashMap::new();
            let mut writes: HashMap<String, Vec<usize>> = HashMap::new();
            let mut arrays = HashMap::new();
            for (i, s) in b.statements.iter_mut().enumerate() {
                if let Statement::Assign {
                    target:
                        Expr {
                            kind: ExprKind::Variable(n),
                            ..
                        },
                    ..
                } = s
                {
                    writes.entry(n.clone()).or_default().push(i);
                }
                let definition = match &*s {
                    Statement::Let { name, value, .. } => Some((name, value)),
                    Statement::Assign {
                        target:
                            Expr {
                                kind: ExprKind::Variable(name),
                                ..
                            },
                        value,
                    } => Some((name, value)),
                    _ => None,
                };
                if let Some((
                    name,
                    Expr {
                        kind: ExprKind::NewArray { length, .. },
                        ..
                    },
                )) = definition
                    && let ExprKind::Constant(length) = &length.kind
                    && let Ok(length) = length.parse::<usize>()
                {
                    arrays.entry(name.clone()).or_insert((i, length));
                }
                statement_exprs(s, &mut |e| {
                    if let ExprKind::Variable(n) = &e.kind {
                        uses.insert(n.clone(), i);
                    }
                });
            }
            let end = b.statements.len();
            terminator_exprs(&mut b.terminator, &mut |e| {
                if let ExprKind::Variable(n) = &e.kind {
                    uses.insert(n.clone(), end);
                }
            });
            let mut remove = HashSet::new();
            for i in 0..end {
                let (name, value) = match &b.statements[i] {
                    Statement::Let { name, value, .. } if counts.get(name) == Some(&1) => {
                        (name.clone(), value.clone())
                    }
                    _ => continue,
                };
                let Some(&next) = uses.get(&name).filter(|&&next| next > i) else {
                    continue;
                };
                let leaf = movable_leaf(&value, &exposed);
                if let ExprKind::Variable(source) = &value.kind
                    && writes.get(source).is_some_and(|positions| {
                        let pos = positions.partition_point(|&p| p <= i);
                        // A write at the use itself happens after RHS evaluation.
                        positions.get(pos).is_some_and(|&p| p < next)
                    })
                {
                    continue;
                }
                if !leaf
                    && (next != i + 1
                        || !first_evaluation(
                            b.statements.get(next),
                            &b.terminator,
                            &name,
                            &exposed,
                            array_store_proven(
                                b.statements.get(next),
                                i,
                                &arrays,
                                &writes,
                                &exposed,
                            ),
                        ))
                {
                    continue;
                }
                let mut next_depth = 0;
                let mut measure = |e: &mut Expr| {
                    next_depth = next_depth.max(expression_depth(e));
                };
                if let Some(s) = b.statements.get_mut(next) {
                    statement_exprs(s, &mut measure);
                } else {
                    terminator_exprs(&mut b.terminator, &mut measure);
                }
                if expression_depth(&value) + next_depth > 48 {
                    continue;
                }
                let mut replace = |e: &mut Expr| {
                    if matches!(&e.kind, ExprKind::Variable(n) if n == &name) {
                        *e = value.clone();
                    }
                };
                if let Some(s) = b.statements.get_mut(next) {
                    statement_exprs(s, &mut replace);
                } else {
                    terminator_exprs(&mut b.terminator, &mut replace);
                }
                remove.insert(i);
                changed = true;
            }
            let mut i = 0;
            b.statements.retain(|_| {
                let keep = !remove.contains(&i);
                i += 1;
                keep
            });
        }
        changed |= arrays::fold(ir, &exposed);
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

/// `C.<>9__N ?? (C.<>9__N = new D(C.<>9.<M>b__N))`: the compiler caches non-capturing
/// lambdas in a static field of its `<>c` class. The cache is an optimization, so the
/// join receives the delegate creation itself and the caching branch disappears.
/// Returns whether any terminator changed.
pub fn cached_delegates(a: &crate::assembly::Assembly, ir: &mut TypedIr, exceptions: bool) -> bool {
    let cache = |token: u32| {
        a.nodes.iter().any(|n| {
            n.token == token
                && n.name.starts_with("<>9__")
                && a.nodes
                    .iter()
                    .any(|t| t.token == n.parent && t.name == "<>c")
        })
    };
    let mut dropped = vec![];
    let mut joins = vec![];
    for i in 0..ir.blocks.len() {
        let Terminator::Condition { condition, yes, no } = ir.blocks[i].terminator.clone() else {
            continue;
        };
        // brtrue v, v != null or v == null on the loaded cache value.
        let (variable, present, missing) = match &condition.kind {
            ExprKind::Variable(v) => (v.clone(), yes, no),
            ExprKind::Binary {
                op, left, right, ..
            } if matches!(right.kind, ExprKind::Null) => {
                let ExprKind::Variable(v) = &left.kind else {
                    continue;
                };
                match op.as_str() {
                    "!=" => (v.clone(), yes, no),
                    "==" => (v.clone(), no, yes),
                    _ => continue,
                }
            }
            _ => continue,
        };
        let Some(load) = ir.blocks[i].statements.iter().position(|s| {
            matches!(s, Statement::Let { name, value: Expr { kind: ExprKind::Field { token, instance: None, .. }, .. }, .. }
                if *name == variable && cache(*token))
        }) else {
            continue;
        };
        let Statement::Let {
            value:
                Expr {
                    kind: ExprKind::Field { token: field, .. },
                    ..
                },
            ..
        } = ir.blocks[i].statements[load].clone()
        else {
            continue;
        };
        let Some(fill) = ir.blocks.iter().position(|b| b.id == missing.target) else {
            continue;
        };
        let block = &ir.blocks[fill];
        let creation = block.statements.iter().find_map(|s| match s {
            Statement::Let { value, .. } | Statement::Assign { value, .. }
                if matches!(&value.kind, ExprKind::New { args, .. }
                    if matches!(args.last().map(|a| &a.kind), Some(ExprKind::FunctionPointer { .. }))) =>
            {
                Some(value.clone())
            }
            _ => None,
        });
        let stores = block.statements.iter().any(|s| {
            matches!(s, Statement::Assign { target: Expr { kind: ExprKind::Field { token, instance: None, .. }, .. }, .. } if *token == field)
        });
        let rejoins =
            matches!(&block.terminator, Terminator::Jump(e) if e.target == present.target);
        let (Some(creation), true, true) = (creation, stores, rejoins) else {
            continue;
        };
        // The cached value is used only by the test and the join's incoming values.
        let used_elsewhere = ir.blocks[i].statements[load + 1..].iter().any(|s| {
            let mut found = false;
            let mut s = s.clone();
            statement_exprs(&mut s, &mut |e| {
                if matches!(&e.kind, ExprKind::Variable(n) if *n == variable) {
                    found = true;
                }
            });
            found
        });
        if used_elsewhere {
            continue;
        }
        let values = present
            .values
            .iter()
            .map(|v| match &v.kind {
                ExprKind::Variable(n) if *n == variable => creation.clone(),
                _ => v.clone(),
            })
            .collect();
        ir.blocks[i].statements.remove(load);
        ir.blocks[i].terminator = Terminator::Jump(Edge {
            target: present.target,
            values,
        });
        dropped.push(missing.target);
        joins.push(present.target);
    }
    if !dropped.is_empty() {
        // Caching branches no other edge enters are gone with their test.
        let targeted: HashSet<usize> = ir
            .blocks
            .iter()
            .flat_map(|b| crate::ir::targets(&b.terminator))
            .collect();
        ir.blocks
            .retain(|b| !dropped.contains(&b.id) || targeted.contains(&b.id));
    }
    // A join now entered only from the test continues it, so its stack slots become
    // ordinary single-use values instead of merge variables.
    if !exceptions {
        while let Some((from, into)) = joins.iter().find_map(|j| {
            let entering: Vec<usize> = ir
                .blocks
                .iter()
                .enumerate()
                .filter(|(_, b)| crate::ir::targets(&b.terminator).contains(j))
                .map(|(i, _)| i)
                .collect();
            let into = ir.blocks.iter().position(|b| b.id == *j)?;
            match entering.as_slice() {
                [from]
                    if matches!(ir.blocks[*from].terminator, Terminator::Jump(_))
                        && *from != into =>
                {
                    Some((*from, into))
                }
                _ => None,
            }
        }) {
            let join = ir.blocks.remove(into);
            let from = if into < from { from - 1 } else { from };
            joins.retain(|j| *j != join.id);
            let Terminator::Jump(edge) =
                std::mem::replace(&mut ir.blocks[from].terminator, join.terminator)
            else {
                unreachable!()
            };
            let entry = ir
                .stack
                .entry_stacks
                .get_mut(join.id)
                .and_then(Option::take)
                .unwrap_or_default();
            let block = &mut ir.blocks[from];
            for (n, value) in edge.values.into_iter().enumerate() {
                let ty = match entry.get(n) {
                    Some(t) if !matches!(t, crate::signature::Type::Unknown) => t.clone(),
                    _ => value.ty.clone(),
                };
                block.statements.push(Statement::Let {
                    name: format!("stack_{:04X}_{n}", join.offset),
                    ty,
                    value,
                });
            }
            block.statements.extend(join.statements);
        }
    }
    !dropped.is_empty()
}
