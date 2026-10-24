//! Rebuilds `async` and iterator methods from the compiler's state machine, as
//! ILSpy/dnSpy do. The kickoff method only starts the machine; its `MoveNext` holds the
//! real body. The state dispatch is evaluated statically, suspension points become
//! `yield return`/`await`, and machine fields become the original locals and
//! parameters. Any machine field left afterwards rejects the result, so an unsupported
//! shape falls back to IL instead of producing wrong source.
use crate::{
    analysis,
    assembly::{Assembly, coded},
    ast::{self, Ast},
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
    /// `async IAsyncEnumerable<T>`: awaits and `yield return` in one machine.
    AsyncIterator,
}
pub struct Lowered {
    pub kind: Kind,
    /// MoveNext's body with the machine's own exception region removed.
    pub body: MethodBody,
    pub cfg: ControlFlowGraph,
    pub ir: TypedIr,
    pub entry: usize,
    /// The body of an iterator's try/finally, moved by the compiler into `<>m__Finally`.
    pub finally: Option<Vec<Ast>>,
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
            "System.Runtime.CompilerServices.AsyncIteratorStateMachineAttribute" => {
                Kind::AsyncIterator
            }
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
                } => {
                    // `switch (state)` or the offset form `switch (state - c)`.
                    let index = match &value.kind {
                        _ if self.is_state(value) => k,
                        ExprKind::Binary {
                            op, left, right, ..
                        } if self.is_state(left) => match (op.as_str(), constant(right)) {
                            ("-", Some(c)) => k - c,
                            ("+", Some(c)) => k + c,
                            _ => return Some(id),
                        },
                        _ => return Some(id),
                    };
                    usize::try_from(index)
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
// Statement markers for the protected range of an iterator's try/finally. They never
// reach the emitter: `structure` replaces them with the `try` statement.
const ENTER: &str = "\0try";
const LEAVE: &str = "\0finally";
fn marker(name: &str) -> Statement {
    Statement::Evaluate(Expr {
        ty: Type::primitive("void"),
        kind: ExprKind::Call {
            token: 0,
            owner: 0,
            name: name.to_owned(),
            instance: None,
            args: vec![],
            generic_args: vec![],
            virtual_call: false,
            accessor: None,
        },
    })
}
fn is_marker(node: &Ast, name: &str) -> bool {
    matches!(node, Ast::Statement(Statement::Evaluate(Expr { kind: ExprKind::Call { token: 0, name: n, .. }, .. })) if n == name)
}
fn this_call<'e>(s: &'e Statement, prefix: &str) -> Option<&'e Expr> {
    match s {
        Statement::Evaluate(
            e @ Expr {
                kind:
                    ExprKind::Call {
                        name,
                        instance: Some(i),
                        ..
                    },
                ..
            },
        ) if is_this(i) && name.starts_with(prefix) => Some(e),
        _ => None,
    }
}
/// MoveNext inside a protected region returns through a local:
/// `result = v; leave end; … end: return result;` becomes `return v`.
fn returns_through_local(ir: &mut TypedIr) {
    let epilogues: HashMap<usize, String> = ir
        .blocks
        .iter()
        .filter_map(|b| match &b.terminator {
            Terminator::Return(Some(Expr {
                kind: ExprKind::Variable(v),
                ..
            })) if b.statements.is_empty() => Some((b.id, v.clone())),
            _ => None,
        })
        .collect();
    for b in &mut ir.blocks {
        let Terminator::Jump(e) = &b.terminator else {
            continue;
        };
        let Some(v) = epilogues.get(&e.target).filter(|_| e.values.is_empty()) else {
            continue;
        };
        if let Some(Statement::Assign { target, value }) = b.statements.last()
            && variable_of(target) == Some(v.as_str())
            && constant(value).is_some()
        {
            let value = value.clone();
            b.statements.pop();
            b.terminator = Terminator::Return(Some(value));
        }
    }
}
/// The state a block stores, also through a copy (`dup; stloc; stfld` stores a local).
fn stored_state(machine: &Machine, statements: &[Statement]) -> Option<i64> {
    statements.iter().enumerate().find_map(|(i, s)| match s {
        Statement::Assign { target, value } if field_of(target) == Some(machine.state) => {
            constant(value).or_else(|| {
                let v = variable_of(value)?;
                statements[..i].iter().rev().find_map(|s| match s {
                    Statement::Assign { target, value } if variable_of(target) == Some(v) => {
                        constant(value)
                    }
                    Statement::Let { name, value, .. } if name == v => constant(value),
                    _ => None,
                })
            })
        }
        _ => None,
    })
}
fn state_constant(machine: &Machine, s: &Statement) -> Option<i64> {
    match s {
        Statement::Assign { target, value } if field_of(target) == Some(machine.state) => {
            constant(value)
        }
        _ => None,
    }
}
/// An iterator's try/finally: the compiler moves the finally body into `<>m__FinallyN`,
/// marks the protected range with a state of -3 or below, and wraps MoveNext in a fault
/// handler that calls Dispose. The fault handler is dropped and the range is marked;
/// returns the finally method. A single, non-nested try/finally is supported.
fn iterator_finally(
    machine: &Machine,
    ir: &mut TypedIr,
    body: &mut MethodBody,
) -> Result<Option<u32>> {
    body.exceptions.retain(|e| {
        !(e.kind == "fault"
            && ir.blocks.iter().any(|b| {
                b.offset == e.handler_start
                    && b.statements.iter().any(|s| {
                        this_call(s, "System.IDisposable.Dispose").is_some()
                            || this_call(s, "Dispose").is_some()
                    })
            }))
    });
    if body.exceptions.iter().any(|e| e.kind == "fault") {
        return Err(unsupported("an iterator fault handler was not recognized"));
    }
    let finally_methods: HashSet<u32> = ir
        .blocks
        .iter()
        .flat_map(|b| &b.statements)
        .filter_map(|s| match this_call(s, "<>m__Finally") {
            Some(Expr {
                kind: ExprKind::Call { token, .. },
                ..
            }) => Some(*token),
            _ => None,
        })
        .collect();
    let finally = match finally_methods.into_iter().collect::<Vec<_>>().as_slice() {
        [] => return Ok(None),
        [one] => *one,
        _ => {
            return Err(unsupported(
                "iterators with several try/finally blocks are not rebuilt yet",
            ));
        }
    };
    // Re-entering the range after a `yield return` resumes inside it; only the first
    // assignment of the protected state, outside those resume points, opens the range.
    let protected: HashSet<i64> = ir
        .blocks
        .iter()
        .flat_map(|b| &b.statements)
        .filter_map(|s| state_constant(machine, s).filter(|k| *k <= -3))
        .collect();
    if protected.len() != 1 {
        return Err(unsupported(
            "iterators with nested try/finally blocks are not rebuilt yet",
        ));
    }
    let yielded: HashSet<i64> = ir
        .blocks
        .iter()
        .filter(|b| matches!(&b.terminator, Terminator::Return(Some(v)) if constant(v) == Some(1)))
        .flat_map(|b| &b.statements)
        .filter_map(|s| state_constant(machine, s))
        .collect();
    let resumes: HashSet<usize> = yielded
        .iter()
        .filter_map(|k| machine.resume(ir, *k))
        .collect();
    let mut entries = vec![];
    for b in &ir.blocks {
        for (i, s) in b.statements.iter().enumerate() {
            let first = b.statements[..i]
                .iter()
                .all(|s| machine.dispatch_statement(s));
            if state_constant(machine, s).is_some_and(|k| protected.contains(&k))
                && !(resumes.contains(&b.id) && first)
            {
                entries.push((b.id, i));
            }
        }
    }
    let [(block, index)] = entries.as_slice() else {
        return Err(unsupported(
            "the start of an iterator's try/finally is ambiguous",
        ));
    };
    let b = ir
        .blocks
        .iter_mut()
        .find(|b| b.id == *block)
        .expect("listed above");
    b.statements[*index] = marker(ENTER);
    for b in &mut ir.blocks {
        for s in &mut b.statements {
            if this_call(s, "<>m__Finally").is_some() {
                *s = marker(LEAVE);
            }
        }
    }
    Ok(Some(finally))
}
/// Wraps the statements between the range markers into `try { … } finally { … }`.
fn wrap_finally(nodes: &mut Vec<Ast>, finally: &[Ast]) -> bool {
    if let Some(start) = nodes.iter().position(|n| is_marker(n, ENTER)) {
        let end = nodes[start + 1..]
            .iter()
            .rposition(|n| is_marker(n, LEAVE))
            .map_or(nodes.len(), |i| start + 1 + i);
        // ENTER through the statement before the closing LEAVE, which follows them.
        let mut protected: Vec<Ast> = nodes.drain(start..end).skip(1).collect();
        if nodes.get(start).is_some_and(|n| is_marker(n, LEAVE)) {
            nodes.remove(start);
        }
        // Early exits inside the range already run the finally block.
        drop_markers(&mut protected);
        nodes.insert(
            start,
            Ast::Try {
                body: protected,
                handlers: vec![ast::Handler {
                    kind: "finally".to_owned(),
                    catch_type: None,
                    variable: None,
                    body: finally.to_vec(),
                }],
            },
        );
        return true;
    }
    children(nodes)
        .into_iter()
        .any(|list| wrap_finally(list, finally))
}
fn children(nodes: &mut [Ast]) -> Vec<&mut Vec<Ast>> {
    let mut lists = vec![];
    for n in nodes {
        match n {
            Ast::If { yes, no, .. } => {
                lists.push(yes);
                lists.push(no);
            }
            Ast::While { body, .. } | Ast::For { body, .. } => lists.push(body),
            Ast::Switch { arms, default, .. } => {
                lists.extend(arms.iter_mut());
                lists.push(default);
            }
            Ast::Try { body, handlers } => {
                lists.push(body);
                lists.extend(handlers.iter_mut().map(|h| &mut h.body));
            }
            _ => {}
        }
    }
    lists
}
fn drop_markers(nodes: &mut Vec<Ast>) {
    nodes.retain(|n| !is_marker(n, LEAVE));
    for list in children(nodes) {
        drop_markers(list);
    }
}
fn has_marker(nodes: &mut [Ast]) -> bool {
    nodes
        .iter()
        .any(|n| is_marker(n, ENTER) || is_marker(n, LEAVE))
        || children(nodes).into_iter().any(|l| has_marker(l))
}
/// Structures a lowered machine, restoring an iterator's try/finally.
pub fn structure(lowered: &Lowered) -> Result<Vec<Ast>> {
    let mut nodes = ast::structure_from(
        &lowered.ir,
        &lowered.cfg,
        &lowered.body.exceptions,
        lowered.entry,
    );
    if let Some(finally) = &lowered.finally
        && (!wrap_finally(&mut nodes, finally) || has_marker(&mut nodes))
    {
        return Err(unsupported(
            "the protected range of a try/finally could not be delimited",
        ));
    }
    Ok(nodes)
}
/// `async IAsyncEnumerable<T>`: `yield return` stores the current value and completes the
/// promise with true; the end completes the builder and the promise with false; dispose
/// mode is a separate path of the same machine and is not part of the source body.
fn async_iterator(machine: &Machine, ir: &mut TypedIr, fields: (u32, u32, u32, u32)) -> Result<()> {
    let (current, builder, dispose, promise) = fields;
    let disposing = |e: &Expr| match &e.kind {
        _ if field_of(e) == Some(dispose) => Some(true),
        ExprKind::Unary { op, value } if op == "!" && field_of(value) == Some(dispose) => {
            Some(false)
        }
        _ => None,
    };
    for b in &mut ir.blocks {
        if let Terminator::Condition { condition, yes, no } = &b.terminator
            && let Some(when) = disposing(condition)
        {
            let next = if when { no.target } else { yes.target };
            b.terminator = jump(next);
        }
        let completes = b.statements.iter().any(|s| {
            matches!(s, Statement::Evaluate(Expr { kind: ExprKind::Call { name, instance: Some(i), .. }, .. })
                if name == "Complete" && field_of(i) == Some(builder))
        });
        // The handler's Complete precedes SetException; the end's precedes SetResult.
        let ends = b.statements.iter().any(|s| {
            matches!(s, Statement::Evaluate(Expr { kind: ExprKind::Call { name, instance: Some(i), .. }, .. })
                if name == "SetResult" && field_of(i) == Some(promise))
        });
        if completes && ends {
            b.statements.clear();
            b.terminator = Terminator::Return(None);
        }
    }
    let signals: HashSet<usize> = ir
        .blocks
        .iter()
        .filter(|b| {
            b.statements.iter().any(|s| {
                matches!(s, Statement::Evaluate(Expr { kind: ExprKind::Call { name, instance: Some(i), args, .. }, .. })
                    if name == "SetResult" && field_of(i) == Some(promise)
                        && args.first().and_then(constant) == Some(1))
            })
        })
        .map(|b| b.id)
        .collect();
    let mut resumes = HashMap::new();
    for i in 0..ir.blocks.len() {
        let b = &ir.blocks[i];
        let Terminator::Jump(e) = &b.terminator else {
            continue;
        };
        if !signals.contains(&e.target) {
            continue;
        }
        let value = b.statements.iter().position(
            |s| matches!(s, Statement::Assign { target, .. } if field_of(target) == Some(current)),
        );
        let state = stored_state(machine, &b.statements);
        let (Some(value), Some(state)) = (value, state) else {
            return Err(unsupported(
                "a yield without a current value and next state",
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
    }
    Ok(())
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
    let mut finally = None;
    let required = |name: &str| field(name).ok_or_else(|| unsupported(&format!("no {name} field")));
    let entry = match kind {
        Kind::Iterator => {
            let current = required("<>2__current")?;
            returns_through_local(&mut ir);
            finally = iterator_finally(&machine, &mut ir, &mut body)?;
            iterator(&machine, &mut ir, current)?
        }
        Kind::Async => {
            let builder = required("<>t__builder")?;
            async_body(&machine, &mut ir, &mut body, builder)?
        }
        Kind::AsyncIterator => {
            let builder = required("<>t__builder")?;
            let fields = (
                required("<>2__current")?,
                builder,
                required("<>w__disposeMode")?,
                required("<>v__promiseOfValueOrEnd")?,
            );
            async_iterator(&machine, &mut ir, fields)?;
            async_body(&machine, &mut ir, &mut body, builder)?
        }
    };
    // The finally method's body joins the method with the same field mapping.
    let mut finally_ir = match finally {
        Some(token) => {
            let fbody = a
                .body(token)?
                .ok_or_else(|| unsupported("the finally method has no body"))?;
            if !fbody.exceptions.is_empty() || !a.locals(&fbody)?.is_empty() {
                return Err(unsupported(
                    "the finally method declares its own locals or regions",
                ));
            }
            let fcfg = ControlFlowGraph::build(&fbody)?;
            let fstack = analysis::analyze(a, token, &fbody, &fcfg)?;
            let mut fir = crate::decompile::lift_with(a, token, &fbody, &fcfg, fstack, accessor)?;
            for b in &mut fir.blocks {
                b.statements
                    .retain(|s| state_constant(&machine, s).is_none());
            }
            Some((fir, fcfg))
        }
        None => None,
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
    // `dup` copies of a stored state are left without readers.
    let mut read = HashSet::new();
    let mut note = |e: &mut Expr| {
        if let ExprKind::Variable(n) = &e.kind {
            read.insert(n.clone());
        }
    };
    for b in &mut ir.blocks {
        for s in &mut b.statements {
            match s {
                Statement::Let { value, .. } => {
                    let mut value = Statement::Evaluate(value.clone());
                    transform::statement_exprs(&mut value, &mut note);
                }
                Statement::Assign { target, value } => {
                    // A variable target is written, not read.
                    let mut parts = vec![Statement::Evaluate(value.clone())];
                    if !matches!(target.kind, ExprKind::Variable(_)) {
                        parts.push(Statement::Evaluate(target.clone()));
                    }
                    for p in &mut parts {
                        transform::statement_exprs(p, &mut note);
                    }
                }
                _ => transform::statement_exprs(s, &mut note),
            }
        }
        transform::terminator_exprs(&mut b.terminator, &mut note);
    }
    for b in &mut ir.blocks {
        b.statements.retain(|s| {
            !matches!(s, Statement::Let { name, value, .. } if constant(value).is_some() && !read.contains(name))
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
            pending.extend(crate::ir::targets(&ir.blocks[*i].terminator));
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
                    stack.extend(crate::ir::targets(&ir.blocks[*i].terminator));
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
        } else if name.starts_with("<>7__wrap") || name.starts_with("<>s__") {
            // Values the compiler spilled across a suspension point.
            let index = *hoisted.entry(f).or_insert_with(|| {
                ir.locals.push(ty.clone());
                ir.locals.len() - 1
            });
            *e = Expr::var(format!("local{index}"), ty.clone());
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
    for b in ir
        .blocks
        .iter_mut()
        .chain(finally_ir.iter_mut().flat_map(|(f, _)| f.blocks.iter_mut()))
    {
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
    for b in ir
        .blocks
        .iter_mut()
        .chain(finally_ir.iter_mut().flat_map(|(f, _)| f.blocks.iter_mut()))
    {
        for s in &mut b.statements {
            transform::statement_exprs(s, &mut check);
        }
        transform::terminator_exprs(&mut b.terminator, &mut check);
    }
    if leftover {
        return Err(unsupported("machine state remains after rebuilding"));
    }
    // Control-flow edges follow the rewritten terminators.
    crate::ir::rebuild_edges(&mut cfg, &ir);
    body.init_locals = true;
    let finally = finally_ir.map(|(fir, fcfg)| {
        let mut nodes = ast::structure(&fir, &fcfg, &[]);
        if matches!(nodes.last(), Some(Ast::Return(None))) {
            nodes.pop();
        }
        nodes.retain(|n| !matches!(n, Ast::Label(_)));
        nodes
    });
    Ok(Lowered {
        kind,
        body,
        cfg,
        ir,
        entry,
        finally,
    })
}
