//! Fold only complete, consecutive initialization of a fresh, unexposed vector.
use super::*;

pub(super) fn fold(ir: &mut TypedIr, exposed: &HashSet<String>) -> bool {
    let counts = count(ir);
    let mut definitions: HashMap<String, usize> = HashMap::new();
    for block in &ir.blocks {
        for statement in &block.statements {
            let name = match statement {
                Statement::Let { name, .. } => Some(name),
                Statement::Assign {
                    target:
                        Expr {
                            kind: ExprKind::Variable(name),
                            ..
                        },
                    ..
                } => Some(name),
                _ => None,
            };
            if let Some(name) = name {
                *definitions.entry(name.clone()).or_default() += 1;
            }
        }
    }
    let mut changed = false;
    for block in &mut ir.blocks {
        let mut i = 0;
        while i < block.statements.len() {
            let (name, value, is_local) = match &block.statements[i] {
                Statement::Let { name, value, .. } => (name, value, false),
                Statement::Assign {
                    target:
                        Expr {
                            kind: ExprKind::Variable(name),
                            ..
                        },
                    value,
                } => (name, value, true),
                _ => {
                    i += 1;
                    continue;
                }
            };
            let ExprKind::NewArray { element, length } = &value.kind else {
                i += 1;
                continue;
            };
            let ExprKind::Constant(size) = &length.kind else {
                i += 1;
                continue;
            };
            let Ok(size) = size.parse::<usize>() else {
                i += 1;
                continue;
            };
            // Exactly N stores and a single later use. No other read (including a
            // catch handler), overwrite or escaped address may observe a partly
            // initialized array. The final use must follow in this same block.
            if !(1..=128).contains(&size)
                || exposed.contains(name)
                || definitions.get(name) != Some(&1)
                || counts.get(name) != Some(&(size + 1 + usize::from(is_local)))
                || i + size >= block.statements.len()
            {
                i += 1;
                continue;
            }
            let mut values = Vec::with_capacity(size);
            for (index, statement) in block.statements[i + 1..=i + size].iter().enumerate() {
                let Statement::Assign {
                    target:
                        Expr {
                            kind:
                                ExprKind::Index {
                                    array,
                                    index: offset,
                                },
                            ..
                        },
                    value,
                } = statement
                else {
                    break;
                };
                if !matches!(&array.kind, ExprKind::Variable(n) if n == name)
                    || !matches!(&offset.kind, ExprKind::Constant(n) if n.parse::<usize>() == Ok(index))
                    || expression_depth(value) >= 47
                {
                    break;
                }
                values.push(value.clone());
            }
            if values.len() != size {
                i += 1;
                continue;
            }
            let mut final_use = false;
            let mut inspect = |e: &mut Expr| {
                if matches!(&e.kind, ExprKind::Variable(n) if n == name) {
                    final_use = true;
                }
            };
            // Clone just the next statement to avoid aliasing the initial definition.
            if let Some(mut next) = block.statements.get(i + size + 1).cloned() {
                statement_exprs(&mut next, &mut inspect);
            } else {
                terminator_exprs(&mut block.terminator, &mut inspect);
            }
            if !final_use {
                i += 1;
                continue;
            }
            let initializer = ExprKind::ArrayInitializer {
                element: element.clone(),
                values,
            };
            match &mut block.statements[i] {
                Statement::Let { value, .. } | Statement::Assign { value, .. } => {
                    value.kind = initializer
                }
                _ => unreachable!(),
            }
            block.statements.drain(i + 1..=i + size);
            changed = true;
            i += 1;
        }
    }
    changed
}
