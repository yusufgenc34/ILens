//! Conservative CFG region structuring. Arbitrary graphs retain explicit labels and gotos.
use crate::{cfg::ControlFlowGraph, cil::ExceptionRegion, ir::*, signature::Type};
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
    Try {
        body: Vec<Ast>,
        handlers: Vec<Handler>,
    },
}
#[derive(Debug, Clone, Serialize)]
pub struct Handler {
    /// "catch" or "finally".
    pub kind: String,
    pub catch_type: Option<u32>,
    /// The handler's entry stack slot, which holds the caught exception.
    pub variable: Option<String>,
    pub body: Vec<Ast>,
}
/// One protected range with its sibling handlers, in handler order.
#[derive(Debug, Clone)]
struct Group {
    try_start: u32,
    try_end: u32,
    handlers: Vec<ExceptionRegion>,
}
impl Group {
    fn end(&self) -> u32 {
        self.handlers.last().map_or(self.try_end, |h| h.handler_end)
    }
}
/// Groups exception clauses for structuring, or None for layouts that stay explicit:
/// filters, fault blocks, and ranges that overlap without nesting.
fn groups(exceptions: &[ExceptionRegion]) -> Option<Vec<Group>> {
    let mut groups: Vec<Group> = vec![];
    for e in exceptions {
        if !matches!(e.kind.as_str(), "catch" | "finally") {
            return None;
        }
        match groups
            .iter_mut()
            .find(|g| g.try_start == e.try_start && g.try_end == e.try_end)
        {
            Some(g) => g.handlers.push(e.clone()),
            None => groups.push(Group {
                try_start: e.try_start,
                try_end: e.try_end,
                handlers: vec![e.clone()],
            }),
        }
    }
    for g in &mut groups {
        g.handlers.sort_by_key(|h| h.handler_start);
        let mut next = g.try_end;
        for h in &g.handlers {
            if h.handler_start != next {
                return None;
            }
            next = h.handler_end;
        }
    }
    for (i, a) in groups.iter().enumerate() {
        for b in &groups[i + 1..] {
            let (a0, a1, b0, b1) = (a.try_start, a.end(), b.try_start, b.end());
            let disjoint = a1 <= b0 || b1 <= a0;
            let nested = a0 <= b0 && b1 <= a1 || b0 <= a0 && a1 <= b1;
            if !disjoint && !nested {
                return None;
            }
        }
    }
    // Outermost first: try/catch/finally compiles to two ranges sharing a start.
    groups.sort_by_key(|g| (g.try_start, std::cmp::Reverse(g.end())));
    Some(groups)
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
    groups: Vec<Group>,
    entered: HashSet<usize>,
    /// Offset ranges of the try bodies and handlers being structured, innermost last.
    bounds: Vec<(u32, u32)>,
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
    // Structures one protected statement; returns where control continues after it.
    fn protected(&mut self, group: usize, id: usize, depth: usize) -> (Ast, Option<usize>) {
        self.entered.insert(group);
        let g = self.groups[group].clone();
        let exit = self.cfg.by_offset.get(&g.end()).copied();
        self.bounds.push((g.try_start, g.try_end));
        let body = self.region(id, exit, depth + 1);
        self.bounds.pop();
        let mut handlers = vec![];
        for h in &g.handlers {
            let entry = self.cfg.by_offset.get(&h.handler_start).copied();
            self.bounds.push((h.handler_start, h.handler_end));
            let body = entry.map_or_else(Vec::new, |e| self.region(e, exit, depth + 1));
            self.bounds.pop();
            handlers.push(Handler {
                kind: h.kind.clone(),
                catch_type: h.catch_type,
                variable: (h.kind == "catch").then(|| format!("stack_{:04X}_0", h.handler_start)),
                body,
            });
        }
        (Ast::Try { body, handlers }, exit)
    }
    fn region(&mut self, mut id: usize, stop: Option<usize>, depth: usize) -> Vec<Ast> {
        let mut out = Vec::new();
        loop {
            if Some(id) == stop {
                return out;
            }
            // Control leaving the current try or handler range continues outside it.
            if let (Some(&(low, high)), Some(block)) = (self.bounds.last(), self.cfg.blocks.get(id))
                && (block.start < low || block.start >= high)
            {
                out.push(Ast::Goto(id));
                return out;
            }
            if depth <= 64
                && !self.seen.contains(&id)
                && let Some(start) = self.cfg.blocks.get(id).map(|b| b.start)
                && let Some(group) = (0..self.groups.len())
                    .find(|g| self.groups[*g].try_start == start && !self.entered.contains(g))
            {
                let (statement, exit) = self.protected(group, id, depth);
                out.push(statement);
                match exit {
                    Some(next) if Some(next) != stop => {
                        id = next;
                        continue;
                    }
                    _ => return out,
                }
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
/// Structures a method body. Supported exception layouts become `Ast::Try` nodes;
/// others are left out so the emitter keeps its explicit labeled form.
pub fn structure(ir: &TypedIr, cfg: &ControlFlowGraph, exceptions: &[ExceptionRegion]) -> Vec<Ast> {
    structure_from(ir, cfg, exceptions, 0)
}
/// Structures from `entry`, which differs from block 0 for recovered state machines.
pub fn structure_from(
    ir: &TypedIr,
    cfg: &ControlFlowGraph,
    exceptions: &[ExceptionRegion],
    entry: usize,
) -> Vec<Ast> {
    let mut b = Builder {
        ir,
        cfg,
        blocks: ir.blocks.iter().map(|b| (b.id, b)).collect(),
        seen: HashSet::new(),
        post: postdominators(cfg),
        groups: groups(exceptions).unwrap_or_default(),
        entered: HashSet::new(),
        bounds: vec![],
    };
    let mut ast = b.region(entry, None, 0);
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
            Ast::Try { body, handlers } => {
                labels(body, needed);
                for h in handlers {
                    labels(&h.body, needed);
                }
            }
            _ => {}
        }
    }
}
fn lists(nodes: &mut [Ast]) -> Vec<&mut Vec<Ast>> {
    let mut out = vec![];
    for n in nodes {
        match n {
            Ast::If { yes, no, .. } => {
                out.push(yes);
                out.push(no);
            }
            Ast::While { body, .. } | Ast::For { body, .. } => out.push(body),
            Ast::Switch { arms, default, .. } => {
                out.extend(arms.iter_mut());
                out.push(default);
            }
            Ast::Try { body, handlers } => {
                out.push(body);
                out.extend(handlers.iter_mut().map(|h| &mut h.body));
            }
            _ => {}
        }
    }
    out
}
/// Short exits that end in `return`/`throw` and are reached by `goto` from elsewhere.
fn exits(nodes: &mut [Ast], found: &mut HashMap<usize, Vec<Ast>>) {
    for (i, n) in nodes.iter().enumerate() {
        let Ast::Label(id) = n else { continue };
        let tail = &nodes[i + 1..];
        let Some(end) = tail
            .iter()
            .take(4)
            .position(|n| matches!(n, Ast::Return(_) | Ast::Throw(_)))
        else {
            continue;
        };
        if tail[..end].iter().all(|n| matches!(n, Ast::Statement(_))) {
            found.insert(*id, tail[..=end].to_vec());
        }
    }
    for list in lists(nodes) {
        exits(list, found);
    }
}
fn replace_gotos(nodes: &mut Vec<Ast>, exits: &HashMap<usize, Vec<Ast>>) {
    let mut i = 0;
    while i < nodes.len() {
        if let Ast::Goto(id) = nodes[i]
            && let Some(copy) = exits.get(&id)
        {
            nodes.splice(i..=i, copy.iter().cloned());
            i += copy.len();
            continue;
        }
        i += 1;
    }
    for list in lists(nodes) {
        replace_gotos(list, exits);
    }
}
fn clean(nodes: &mut Vec<Ast>) {
    // A shared `return`/`throw` exit is repeated at each jump to it, as ILSpy does; C#
    // cannot jump into the nested block where structuring placed it.
    let mut found = HashMap::new();
    exits(nodes, &mut found);
    replace_gotos(nodes, &found);
    let mut needed = HashSet::new();
    labels(nodes, &mut needed);
    fn trim(nodes: &mut Vec<Ast>, needed: &HashSet<usize>) {
        // Code after a terminal statement under a label nothing jumps to is unreachable.
        let mut i = 1;
        while i < nodes.len() {
            if matches!(nodes[i], Ast::Label(id) if !needed.contains(&id))
                && matches!(
                    nodes[i - 1],
                    Ast::Return(_) | Ast::Throw(_) | Ast::Goto(_) | Ast::Break
                )
            {
                let end = nodes[i + 1..]
                    .iter()
                    .position(|n| matches!(n, Ast::Label(id) if needed.contains(id)))
                    .map_or(nodes.len(), |p| i + 1 + p);
                nodes.drain(i..end);
                continue;
            }
            i += 1;
        }
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
                Ast::Try { body, handlers } => {
                    trim(body, needed);
                    for h in handlers {
                        trim(&mut h.body, needed);
                    }
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
