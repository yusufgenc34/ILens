//! Conservative CFG region structuring. Arbitrary graphs retain explicit labels and gotos.
use crate::{cfg::ControlFlowGraph, ir::*, signature::Type};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
#[derive(Debug, Clone, Serialize)]
pub enum Ast {
    Statement(Statement),
    ParallelAssign {
        targets: Vec<Expr>,
        values: Vec<Expr>,
    },
    Label(usize),
    Goto(usize),
    If {
        condition: Expr,
        yes: Vec<Ast>,
        no: Vec<Ast>,
    },
    While {
        condition: Expr,
        body: Vec<Ast>,
    },
    For {
        initializer: Box<Statement>,
        condition: Expr,
        increment: Box<Statement>,
        body: Vec<Ast>,
    },
    Switch {
        value: Expr,
        arms: Vec<Vec<Ast>>,
        default: Vec<Ast>,
    },
    Return(Option<Expr>),
    Throw(Option<Expr>),
    Break,
}
/// Edge arguments are simultaneous: a backedge can rotate stack slots.
/// Tuple assignment evaluates every RHS before overwriting any destination.
pub fn edge_assignments(e: &Edge, cfg: &ControlFlowGraph) -> Vec<Ast> {
    let targets: Vec<_> = e
        .values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            Expr::var(
                format!("stack_{:04X}_{i}", cfg.blocks[e.target].start),
                v.ty.clone(),
            )
        })
        .collect();
    match e.values.as_slice() {
        [] => vec![],
        [value] => vec![Ast::Statement(Statement::Assign {
            target: targets[0].clone(),
            value: value.clone(),
        })],
        _ => vec![Ast::ParallelAssign {
            targets,
            values: e.values.clone(),
        }],
    }
}
fn not(condition: Expr) -> Expr {
    if let ExprKind::Unary { op, value } = &condition.kind
        && op == "!"
    {
        return *value.clone();
    }
    Expr {
        ty: Type::primitive("bool"),
        kind: ExprKind::Unary {
            op: "!".to_owned(),
            value: Box::new(condition),
        },
    }
}
struct Builder<'a> {
    ir: &'a TypedIr,
    cfg: &'a ControlFlowGraph,
    blocks: HashMap<usize, &'a IrBlock>,
    seen: HashSet<usize>,
    post: Vec<HashSet<usize>>,
}
impl Builder<'_> {
    fn edge(&self, e: &Edge) -> Vec<Ast> {
        edge_assignments(e, self.cfg)
    }
    fn chain(&self, mut start: usize, end: usize) -> Option<Vec<usize>> {
        let mut path = Vec::new();
        while start != end {
            if path.len() > 256 || path.contains(&start) || self.seen.contains(&start) {
                return None;
            }
            let b = self.blocks.get(&start)?;
            if let Terminator::Jump(e) = &b.terminator {
                if !e.values.is_empty() {
                    return None;
                }
                path.push(start);
                start = e.target;
            } else {
                return None;
            }
        }
        for node in &path {
            if self.cfg.blocks[*node]
                .predecessors
                .iter()
                .any(|p| *p != end && !path.contains(p))
            {
                return None;
            }
        }
        Some(path)
    }
    fn region(&mut self, mut id: usize, stop: Option<usize>, depth: usize) -> Vec<Ast> {
        let mut out = Vec::new();
        loop {
            if Some(id) == stop {
                return out;
            }
            if depth > 64 || self.seen.contains(&id) {
                out.push(Ast::Goto(id));
                return out;
            }
            let Some(b) = self.blocks.get(&id).copied() else {
                out.push(Ast::Goto(id));
                return out;
            };
            self.seen.insert(id);
            out.push(Ast::Label(id));
            match &b.terminator {
                Terminator::Condition { condition, yes, no } => {
                    let cycle = if yes.values.is_empty() && no.values.is_empty() {
                        self.chain(yes.target, id)
                            .map(|path| (path, condition.clone(), no.target))
                            .or_else(|| {
                                self.chain(no.target, id)
                                    .map(|path| (path, not(condition.clone()), yes.target))
                            })
                    } else {
                        None
                    };
                    if let Some((path, condition, exit)) = cycle {
                        let mut loop_body = Vec::new();
                        for p in path {
                            self.seen.insert(p);
                            if let Some(p) = self.blocks.get(&p) {
                                loop_body.extend(p.statements.iter().cloned().map(Ast::Statement));
                            }
                        }
                        if b.statements.is_empty() {
                            out.push(Ast::While {
                                condition,
                                body: loop_body,
                            });
                        } else {
                            let mut prefix: Vec<_> =
                                b.statements.iter().cloned().map(Ast::Statement).collect();
                            prefix.push(Ast::If {
                                condition: not(condition),
                                yes: vec![Ast::Break],
                                no: vec![],
                            });
                            prefix.extend(loop_body);
                            out.push(Ast::While {
                                condition: Expr::constant("true", Type::primitive("bool")),
                                body: prefix,
                            });
                        }
                        id = exit;
                        continue;
                    }
                    out.extend(b.statements.iter().cloned().map(Ast::Statement));
                    let join = self
                        .post
                        .get(yes.target)
                        .and_then(|p| {
                            self.post.get(no.target).map(|n| {
                                p.intersection(n)
                                    .copied()
                                    .filter(|j| *j != id && *j < self.cfg.blocks.len())
                                    .max_by_key(|j| self.post[*j].len())
                            })
                        })
                        .flatten();
                    let mut y = self.edge(yes);
                    y.extend(self.region(yes.target, join.or(stop), depth + 1));
                    let mut n = self.edge(no);
                    n.extend(self.region(no.target, join.or(stop), depth + 1));
                    out.push(Ast::If {
                        condition: condition.clone(),
                        yes: y,
                        no: n,
                    });
                    if let Some(j) = join
                        && Some(j) != stop
                    {
                        id = j;
                        continue;
                    }
                    return out;
                }
                _ => out.extend(b.statements.iter().cloned().map(Ast::Statement)),
            }
            match &b.terminator {
                Terminator::Jump(e) => {
                    out.extend(self.edge(e));
                    id = e.target;
                }
                Terminator::Return(v) => {
                    out.push(Ast::Return(v.clone()));
                    return out;
                }
                Terminator::Throw(v) => {
                    out.push(Ast::Throw(v.clone()));
                    return out;
                }
                Terminator::EndFinally => return out,
                Terminator::Switch {
                    value,
                    arms,
                    default,
                } => {
                    let branches = arms
                        .iter()
                        .map(|e| {
                            let mut v = self.edge(e);
                            v.push(Ast::Goto(e.target));
                            v
                        })
                        .collect();
                    let mut d = self.edge(default);
                    d.push(Ast::Goto(default.target));
                    out.push(Ast::Switch {
                        value: value.clone(),
                        arms: branches,
                        default: d,
                    });
                    return out;
                }
                Terminator::Condition { .. } => return out,
            }
        }
    }
}
fn postdominators(cfg: &ControlFlowGraph) -> Vec<HashSet<usize>> {
    let n = cfg.blocks.len();
    if n > 512 {
        return vec![];
    }
    let all: HashSet<_> = (0..=n).collect();
    let mut sets = vec![all; n + 1];
    sets[n] = HashSet::from([n]);
    for _ in 0..n + 1 {
        let mut changed = false;
        for b in cfg.blocks.iter().rev() {
            let successors = if b.successors.is_empty() {
                vec![n]
            } else {
                b.successors.clone()
            };
            let mut next = sets[successors[0]].clone();
            for s in &successors[1..] {
                next.retain(|p| sets[*s].contains(p));
            }
            next.insert(b.id);
            if sets[b.id] != next {
                sets[b.id] = next;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    sets
}
pub fn structure(ir: &TypedIr, cfg: &ControlFlowGraph) -> Vec<Ast> {
    let mut b = Builder {
        ir,
        cfg,
        blocks: ir.blocks.iter().map(|b| (b.id, b)).collect(),
        seen: HashSet::new(),
        post: postdominators(cfg),
    };
    let mut ast = b.region(0, None, 0);
    for block in &b.ir.blocks {
        if !b.seen.contains(&block.id) {
            ast.extend(b.region(block.id, None, 0));
        }
    }
    clean(&mut ast);
    ast
}
fn labels(nodes: &[Ast], needed: &mut HashSet<usize>) {
    for n in nodes {
        match n {
            Ast::Goto(t) => {
                needed.insert(*t);
            }
            Ast::If { yes, no, .. } => {
                labels(yes, needed);
                labels(no, needed);
            }
            Ast::While { body, .. } | Ast::For { body, .. } => labels(body, needed),
            Ast::Switch { arms, default, .. } => {
                for a in arms {
                    labels(a, needed);
                }
                labels(default, needed);
            }
            _ => {}
        }
    }
}
fn clean(nodes: &mut Vec<Ast>) {
    let mut needed = HashSet::new();
    labels(nodes, &mut needed);
    fn trim(nodes: &mut Vec<Ast>, needed: &HashSet<usize>) {
        nodes.retain(|n| !matches!(n,Ast::Label(id)if !needed.contains(id)));
        for n in nodes.iter_mut() {
            match n {
                Ast::If { yes, no, .. } => {
                    trim(yes, needed);
                    trim(no, needed);
                }
                Ast::While { body, .. } | Ast::For { body, .. } => trim(body, needed),
                Ast::Switch { arms, default, .. } => {
                    for a in arms {
                        trim(a, needed);
                    }
                    trim(default, needed);
                }
                _ => {}
            }
        }
        let mut i = 1;
        while i < nodes.len() {
            if let (
                Ast::Statement(
                    initializer @ Statement::Assign {
                        target:
                            Expr {
                                kind: ExprKind::Variable(v),
                                ..
                            },
                        ..
                    },
                ),
                Ast::While { condition, body },
            ) = (&nodes[i - 1], &nodes[i])
                && let Some(Ast::Statement(
                    increment @ Statement::Assign {
                        target:
                            Expr {
                                kind: ExprKind::Variable(w),
                                ..
                            },
                        value:
                            Expr {
                                kind:
                                    ExprKind::Binary {
                                        op, left, right, ..
                                    },
                                ..
                            },
                    },
                )) = body.last()
                && v == w
                && matches!(&left.kind,ExprKind::Variable(n)if n==v)
                && matches!(&right.kind,ExprKind::Constant(n)if n=="1")
                && (op == "+" || op == "-")
            {
                let node = Ast::For {
                    initializer: Box::new(initializer.clone()),
                    condition: condition.clone(),
                    increment: Box::new(increment.clone()),
                    body: body[..body.len() - 1].to_vec(),
                };
                nodes.splice(i - 1..=i, [node]);
                continue;
            }
            i += 1;
        }
    }
    trim(nodes, &needed);
}
