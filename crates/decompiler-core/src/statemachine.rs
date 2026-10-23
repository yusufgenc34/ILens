//! Rebuilds `async` and iterator methods from the compiler's state machine, as
//! ILSpy/dnSpy do. The kickoff method only starts the machine; its `MoveNext` holds the
//! real body. The state dispatch is evaluated statically, suspension points become
//! `yield return`/`await`, and machine fields become the original locals and
//! parameters. Any machine field left afterwards rejects the result, so an unsupported
//! shape falls back to IL instead of producing wrong source.
use crate::{
    analysis,
    assembly::{Assembly, coded},
    cfg::ControlFlowGraph,
    cil::MethodBody,
    error::{Error, Result},
    ir::{Edge, Expr, ExprKind, Statement, Terminator, TypedIr},
    reader::Reader,
    resolution::Accessor,
    signature::Type,
    transform,
};
use std::collections::{HashMap, HashSet};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Iterator,
    Async,
}
pub struct Lowered {
    pub kind: Kind,
    /// MoveNext's body with the machine's own exception region removed.
    pub body: MethodBody,
    pub cfg: ControlFlowGraph,
    pub ir: TypedIr,
    pub entry: usize,
}
fn unsupported(detail: &str) -> Error {
    Error::limitation(format!(
        "State-machine reconstruction: {detail}. Inspect the original CIL and generated types."
    ))
}
/// The state-machine kind and type named by the method's
/// [AsyncStateMachine]/[IteratorStateMachine] attribute.
pub fn kind_of(a: &Assembly, token: u32) -> Option<(Kind, u32)> {
    for index in a.attributes.get(&token)? {
        let attribute = &a.metadata().custom_attributes[*index];
        let constructor = a.method_ref(coded(attribute.attr_type)).ok()?;
        let kind = match a.resolve_full(constructor.owner).as_str() {
            "System.Runtime.CompilerServices.AsyncStateMachineAttribute" => Kind::Async,
            "System.Runtime.CompilerServices.IteratorStateMachineAttribute" => Kind::Iterator,
            _ => continue,
        };
        let blob = a.blob(attribute.value).ok()?;
        let mut r = Reader::new(blob);
        if r.u16().ok()? != 1 {
            return None;
        }
        let length = r.compressed().ok()? as usize;
        let name = std::str::from_utf8(r.take(length).ok()?).ok()?;
        let name = name.split(',').next()?.rsplit(['+', '.']).next()?;
        let owner = *a.owners.get(&token)?;
        let machine = a
            .nodes
            .iter()
            .find(|n| n.token >> 24 == 2 && n.parent == owner && n.name == name)?;
        return Some((kind, machine.token));
    }
    None
}
fn is_this(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Variable(n) => n == "this",
        ExprKind::Deref(inner) | ExprKind::Address(inner) => is_this(inner),
        _ => false,
    }
}
// The machine field read or written by `this.field` or `ref this.field`.
fn field_of(e: &Expr) -> Option<u32> {
    match &e.kind {
        ExprKind::Field {
            token,
            instance: Some(i),
            ..
        } if is_this(i) => Some(*token),
        ExprKind::Address(inner) | ExprKind::Deref(inner) => field_of(inner),
        _ => None,
    }
}
fn variable_of(e: &Expr) -> Option<&str> {
    match &e.kind {
        ExprKind::Variable(n) => Some(n),
        ExprKind::Address(inner) | ExprKind::Deref(inner) => variable_of(inner),
        _ => None,
    }
}
fn constant(e: &Expr) -> Option<i64> {
    match &e.kind {
        ExprKind::Constant(c) => match c.as_str() {
            "true" => Some(1),
            "false" => Some(0),
            c => c.trim_end_matches(['L', 'U', 'u']).parse().ok(),
        },
        _ => None,
    }
}
struct Machine {
    state: u32,
    /// Locals that hold a copy of the state field.
    copies: HashSet<String>,
}
impl Machine {
    fn is_state(&self, e: &Expr) -> bool {
        field_of(e) == Some(self.state) || variable_of(e).is_some_and(|v| self.copies.contains(v))
    }
    fn dispatch_statement(&self, s: &Statement) -> bool {
        match s {
            Statement::Assign { target, value } => {
                variable_of(target).is_some_and(|v| self.copies.contains(v))
                    && field_of(value) == Some(self.state)
            }
            _ => false,
        }
    }
    // The branch a condition takes when the state is `k`, or None when it does not depend on it.
    fn evaluate(&self, condition: &Expr, k: i64) -> Option<bool> {
        match &condition.kind {
            ExprKind::Unary { op, value } if op == "!" => self.evaluate(value, k).map(|b| !b),
            ExprKind::Binary {
                op,
                left,
                right,
                unsigned,
                ..
            } if self.is_state(left) => {
                let c = constant(right)?;
                let (l, r) = if *unsigned {
                    (k as u32 as i64, c as u32 as i64)
                } else {
                    (k, c)
                };
                Some(match op.as_str() {
                    "==" => l == r,
                    "!=" => l != r,
                    "<" => l < r,
                    "<=" => l <= r,
                    ">" => l > r,
                    ">=" => l >= r,
                    _ => return None,
                })
            }
            _ if self.is_state(condition) => Some(k != 0),
            _ => None,
        }
    }
    /// The first block that runs real code when MoveNext starts in state `k`.
    fn resume(&self, ir: &TypedIr, k: i64) -> Option<usize> {
        let blocks: HashMap<usize, usize> = ir
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.id, i))
            .collect();
        let mut id = ir.blocks.first()?.id;
        for _ in 0..128 {
            let b = &ir.blocks[*blocks.get(&id)?];
            if b.statements.iter().any(|s| !self.dispatch_statement(s)) {
                return Some(id);
            }
            id = match &b.terminator {
                Terminator::Jump(e) if e.values.is_empty() => e.target,
                Terminator::Condition { condition, yes, no } => match self.evaluate(condition, k) {
                    Some(true) => yes.target,
                    Some(false) => no.target,
                    None => return Some(id),
                },
                Terminator::Switch {
                    value,
                    arms,
                    default,
                } if self.is_state(value) => {
                    usize::try_from(k)
                        .ok()
                        .and_then(|i| arms.get(i))
                        .unwrap_or(default)
                        .target
                }
                _ => return Some(id),
            };
        }
        None
    }
}
fn jump(target: usize) -> Terminator {
    Terminator::Jump(Edge {
        target,
        values: vec![],
    })
}
fn calls(e: &Expr, name: &str) -> bool {
    let mut found = false;
    let mut s = Statement::Evaluate(e.clone());
    transform::statement_exprs(&mut s, &mut |x: &mut Expr| {
        if matches!(&x.kind, ExprKind::Call { name: n, .. } if n == name) {
            found = true;
        }
    });
    found
}
fn iterator(machine: &Machine, ir: &mut TypedIr, current: u32) -> Result<usize> {
    let mut resumes = HashMap::new();
    for i in 0..ir.blocks.len() {
        let b = &ir.blocks[i];
        let yields = matches!(&b.terminator, Terminator::Return(Some(v)) if constant(v) == Some(1));
        if yields {
            let value = b.statements.iter().position(|s| matches!(s, Statement::Assign { target, .. } if field_of(target) == Some(current)));
            let state = b.statements.iter().find_map(|s| match s {
                Statement::Assign { target, value } if field_of(target) == Some(machine.state) => {
                    constant(value)
                }
                _ => None,
            });
            let (Some(value), Some(state)) = (value, state) else {
                return Err(unsupported(
                    "a `return true` without a current value and next state",
                ));
            };
            let resume = *resumes
                .entry(state)
                .or_insert_with(|| machine.resume(ir, state))
                .as_ref()
                .ok_or_else(|| unsupported("a resume state could not be dispatched"))?;
            let b = &mut ir.blocks[i];
            if let Statement::Assign { value: v, .. } = b.statements[value].clone() {
                b.statements[value] = Statement::Yield(v);
            }
            b.terminator = jump(resume);
        } else if matches!(&b.terminator, Terminator::Return(Some(v)) if constant(v) == Some(0)) {
            ir.blocks[i].terminator = Terminator::Return(None);
        }
    }
    machine
        .resume(ir, 0)
        .ok_or_else(|| unsupported("the initial state could not be dispatched"))
}
fn async_body(
    machine: &Machine,
    ir: &mut TypedIr,
    body: &mut MethodBody,
    builder: u32,
) -> Result<usize> {
    // The compiler wraps the body in try { … } catch (Exception) { SetException(…) }.
    let handler = body
        .exceptions
        .iter()
        .position(|e| {
            e.kind == "catch"
                && ir.blocks.iter().any(|b| {
                    b.offset >= e.handler_start
                        && b.offset < e.handler_end
                        && b.statements.iter().any(
                            |s| matches!(s, Statement::Evaluate(e) if calls(e, "SetException")),
                        )
                })
        })
        .ok_or_else(|| unsupported("the builder's exception handler was not found"))?;
    body.exceptions.remove(handler);
    for i in 0..ir.blocks.len() {
        let Terminator::Condition { condition, yes, no } = ir.blocks[i].terminator.clone() else {
            continue;
        };
        let (negated, test) = match &condition.kind {
            ExprKind::Unary { op, value } if op == "!" => (true, value.as_ref()),
            _ => (false, &condition),
        };
        let ExprKind::Call {
            name,
            accessor,
            instance: Some(awaiter),
            ..
        } = &test.kind
        else {
            continue;
        };
        let completed =
            name == "get_IsCompleted" || accessor.as_ref().is_some_and(|p| p.name == "IsCompleted");
        let Some(awaiter) = variable_of(awaiter)
            .filter(|_| completed)
            .map(str::to_owned)
        else {
            continue;
        };
        let done = if negated { no.target } else { yes.target };
        // awaiter = E.GetAwaiter(); immediately before the completion test.
        let b = &mut ir.blocks[i];
        let Some(get) = b.statements.iter().rposition(|s| {
            matches!(s, Statement::Assign { target, value: Expr { kind: ExprKind::Call { name, .. }, .. } }
                if variable_of(target) == Some(awaiter.as_str()) && name == "GetAwaiter")
        }) else {
            return Err(unsupported("an await has no GetAwaiter call"));
        };
        let Statement::Assign {
            value:
                Expr {
                    kind:
                        ExprKind::Call {
                            instance: Some(source),
                            ..
                        },
                    ..
                },
            ..
        } = b.statements.remove(get)
        else {
            unreachable!("matched above");
        };
        // A temporary that only carries the awaitable is folded into the await.
        let mut awaited = match &source.kind {
            ExprKind::Address(inner) => (**inner).clone(),
            _ => (*source).clone(),
        };
        if let Some(temporary) = variable_of(&awaited).map(str::to_owned)
            && get > 0
            && let Statement::Assign { target, value } = &b.statements[get - 1]
            && variable_of(target) == Some(temporary.as_str())
        {
            awaited = value.clone();
            b.statements.remove(get - 1);
        }
        b.terminator = jump(done);
        // awaiter.GetResult() on the completed path is the value of the await.
        let mut target = done;
        let mut replaced = false;
        for _ in 0..8 {
            let Some(block) = ir.blocks.iter_mut().find(|b| b.id == target) else {
                break;
            };
            let mut replace = |e: &mut Expr| {
                if !replaced
                    && let ExprKind::Call {
                        name,
                        instance: Some(instance),
                        ..
                    } = &e.kind
                    && name == "GetResult"
                    && variable_of(instance) == Some(awaiter.as_str())
                {
                    e.kind = ExprKind::Await(Box::new(awaited.clone()));
                    replaced = true;
                }
            };
            for s in &mut block.statements {
                transform::statement_exprs(s, &mut replace);
            }
            if replaced {
                break;
            }
            match &block.terminator {
                Terminator::Jump(e) => target = e.target,
                _ => break,
            }
        }
        if !replaced {
            return Err(unsupported(
                "an await has no GetResult on its completed path",
            ));
        }
    }
    // builder.SetResult(x) ends the method: return x.
    for b in &mut ir.blocks {
        if let Some(i) = b.statements.iter().position(|s| {
            matches!(s, Statement::Evaluate(Expr { kind: ExprKind::Call { name, instance: Some(instance), .. }, .. })
                if name == "SetResult" && field_of(instance) == Some(builder))
        }) {
            let Statement::Evaluate(Expr { kind: ExprKind::Call { args, .. }, .. }) = b.statements[i].clone() else {
                unreachable!("matched above");
            };
            b.statements.truncate(i);
            b.terminator = Terminator::Return(args.into_iter().next());
        }
    }
    machine
        .resume(ir, -1)
        .ok_or_else(|| unsupported("the initial state could not be dispatched"))
}
pub fn lower(
    a: &Assembly,
    kickoff: u32,
    accessor: &dyn Fn(u32) -> Option<Accessor>,
) -> Result<Lowered> {
    let (kind, machine_type) =
        kind_of(a, kickoff).ok_or_else(|| unsupported("no state-machine type"))?;
    let move_next = a
        .nodes
        .iter()
        .find(|n| {
            n.parent == machine_type
                && n.token >> 24 == 6
                && (n.name == "MoveNext" || n.name.ends_with(".MoveNext"))
        })
        .ok_or_else(|| unsupported("MoveNext is missing"))?
        .token;
    let mut body = a
        .body(move_next)?
        .ok_or_else(|| unsupported("MoveNext has no body"))?;
    let mut cfg = ControlFlowGraph::build(&body)?;
    let stack = analysis::analyze(a, move_next, &body, &cfg)?;
    let mut ir = crate::decompile::lift_with(a, move_next, &body, &cfg, stack, accessor)?;
    let fields: HashMap<u32, (String, Type)> = a
        .nodes
        .iter()
        .filter(|n| n.parent == machine_type && n.token >> 24 == 4)
        .filter_map(|n| {
            a.field_ref(n.token)
                .ok()
                .map(|(_, _, ty)| (n.token, (n.name.clone(), ty)))
        })
        .collect();
    let field = |name: &str| fields.iter().find(|(_, (n, _))| n == name).map(|(t, _)| *t);
    let state = field("<>1__state").ok_or_else(|| unsupported("no state field"))?;
    let mut copies = HashSet::new();
    for b in &ir.blocks {
        for s in &b.statements {
            if let Statement::Assign { target, value } = s
                && field_of(value) == Some(state)
                && let Some(v) = variable_of(target)
            {
                copies.insert(v.to_owned());
            }
        }
    }
    let machine = Machine { state, copies };
    let entry = match kind {
        Kind::Iterator => {
            if !body.exceptions.is_empty() {
                return Err(unsupported(
                    "iterators with try/finally blocks are not rebuilt yet",
                ));
            }
            let current = field("<>2__current").ok_or_else(|| unsupported("no current field"))?;
            iterator(&machine, &mut ir, current)?
        }
        Kind::Async => {
            let builder = field("<>t__builder").ok_or_else(|| unsupported("no builder field"))?;
            async_body(&machine, &mut ir, &mut body, builder)?
        }
    };
    // State bookkeeping disappears once the dispatch has been resolved.
    for b in &mut ir.blocks {
        b.statements.retain(|s| match s {
            Statement::Assign { target, .. } => {
                field_of(target) != Some(state)
                    && !variable_of(target).is_some_and(|v| machine.copies.contains(v))
                    && !field_of(target).is_some_and(|f| {
                        fields.get(&f).is_some_and(|(n, _)| n.starts_with("<>u__"))
                    })
            }
            _ => true,
        });
    }
    // Keep only code reachable from the recovered entry.
    let mut reachable = HashSet::new();
    let mut pending = vec![entry];
    let index: HashMap<usize, usize> = ir
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();
    while let Some(id) = pending.pop() {
        if !reachable.insert(id) {
            continue;
        }
        if let Some(i) = index.get(&id) {
            pending.extend(targets(&ir.blocks[*i].terminator));
        }
    }
    for h in &body.exceptions {
        if let Some(b) = ir.blocks.iter().find(|b| b.offset == h.handler_start)
            && reachable.iter().any(|r| {
                ir.blocks
                    .iter()
                    .any(|x| x.id == *r && x.offset >= h.try_start && x.offset < h.try_end)
            })
        {
            let mut stack = vec![b.id];
            while let Some(id) = stack.pop() {
                if reachable.insert(id)
                    && let Some(i) = index.get(&id)
                {
                    stack.extend(targets(&ir.blocks[*i].terminator));
                }
            }
        }
    }
    ir.blocks.retain(|b| reachable.contains(&b.id));
    body.exceptions
        .retain(|h| ir.blocks.iter().any(|b| b.offset == h.try_start));
    // Machine fields become the method's parameters, `this` and hoisted locals.
    let method = a.method_ref(kickoff)?;
    let parameters = a.parameter_names(kickoff, &method.signature);
    let is_static = !method.signature.has_this;
    let mut hoisted: HashMap<u32, usize> = HashMap::new();
    let mut failure = None;
    let mut rename = |e: &mut Expr| {
        let Some(f) = field_of(e).filter(|_| matches!(e.kind, ExprKind::Field { .. })) else {
            return;
        };
        let Some((name, ty)) = fields.get(&f) else {
            return;
        };
        if name == "<>4__this" && !is_static {
            *e = Expr::var("this", ty.clone());
        } else if parameters.contains(name) {
            *e = Expr::var(name.clone(), ty.clone());
        } else if let Some(local) = name
            .strip_prefix('<')
            .and_then(|n| n.split_once(">5__"))
            .map(|(n, _)| n.to_owned())
        {
            let index = *hoisted.entry(f).or_insert_with(|| {
                ir.locals.push(ty.clone());
                ir.local_names.insert(ir.locals.len() - 1, local.clone());
                ir.locals.len() - 1
            });
            *e = Expr::var(format!("local{index}"), ty.clone());
        } else {
            failure = Some(name.clone());
        }
    };
    for b in &mut ir.blocks {
        for s in &mut b.statements {
            transform::statement_exprs(s, &mut rename);
        }
        transform::terminator_exprs(&mut b.terminator, &mut rename);
    }
    if let Some(name) = failure {
        return Err(unsupported(&format!(
            "machine field {name} has no source equivalent"
        )));
    }
    let mut leftover = false;
    let mut check = |e: &mut Expr| {
        if is_this(e) && is_static || field_of(e).is_some_and(|f| fields.contains_key(&f)) {
            leftover = true;
        }
    };
    for b in &mut ir.blocks {
        for s in &mut b.statements {
            transform::statement_exprs(s, &mut check);
        }
        transform::terminator_exprs(&mut b.terminator, &mut check);
    }
    if leftover {
        return Err(unsupported("machine state remains after rebuilding"));
    }
    // Control-flow edges follow the rewritten terminators.
    for block in &mut cfg.blocks {
        block.successors.clear();
        block.predecessors.clear();
    }
    let positions: HashMap<usize, usize> = cfg
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();
    for b in &ir.blocks {
        for t in targets(&b.terminator) {
            if let (Some(from), Some(to)) = (positions.get(&b.id), positions.get(&t)) {
                cfg.blocks[*from].successors.push(t);
                cfg.blocks[*to].predecessors.push(b.id);
            }
        }
    }
    body.init_locals = true;
    Ok(Lowered {
        kind,
        body,
        cfg,
        ir,
        entry,
    })
}
fn targets(t: &Terminator) -> Vec<usize> {
    match t {
        Terminator::Jump(e) => vec![e.target],
        Terminator::Condition { yes, no, .. } => vec![yes.target, no.target],
        Terminator::Switch { arms, default, .. } => {
            arms.iter().chain([default]).map(|e| e.target).collect()
        }
        _ => vec![],
    }
}
