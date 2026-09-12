//! Fixed-point evaluation stack propagation. This is static dataflow, never an interpreter.
use crate::{
    assembly::Assembly,
    cfg::ControlFlowGraph,
    cil::{Instruction, MethodBody, Operand},
    error::{Error, Result},
    signature::Type,
};
use serde::Serialize;
use std::collections::VecDeque;
#[derive(Debug, Clone, Serialize)]
pub struct StackAnalysis {
    pub entry_stacks: Vec<Option<Vec<Type>>>,
    pub maximum: usize,
    pub reachable_blocks: usize,
}
pub fn variable(i: &Instruction) -> Option<usize> {
    match i.operand {
        Operand::Variable(v) => Some(v as usize),
        _ => {
            let op = i.opcode;
            match op {
                0x02..=0x05 => Some((op - 2) as usize),
                0x06..=0x09 => Some((op - 6) as usize),
                0x0a..=0x0d => Some((op - 10) as usize),
                _ => None,
            }
        }
    }
}
pub fn arguments(a: &Assembly, token: u32) -> Result<Vec<Type>> {
    let m = a.method_ref(token)?;
    let mut args = m.signature.parameters;
    if m.signature.has_this {
        args.insert(0, Type::Named(m.owner));
    }
    Ok(args)
}
pub fn effect(a: &Assembly, token: u32, i: &Instruction) -> Result<(usize, usize)> {
    if matches!(i.opcode, 0x28 | 0x6f | 0x73 | 0x29) {
        let m = a.method_ref(i.token().ok_or_else(|| Error::cil("Call without token"))?)?;
        return Ok((
            m.signature.parameters.len()
                + usize::from(m.signature.has_this && i.opcode != 0x73)
                + usize::from(i.opcode == 0x29),
            usize::from(i.opcode == 0x73 || !m.signature.return_type.is_void()),
        ));
    }
    if i.opcode == 0x2a {
        return Ok((
            usize::from(!a.method_ref(token)?.signature.return_type.is_void()),
            0,
        ));
    }
    if i.opcode == 0x27 {
        return Ok((0, 0));
    }
    if i.op.pop < 0 || i.op.push < 0 {
        return Err(Error::limitation(format!(
            "Variable stack behaviour for {}",
            i.name
        )));
    }
    Ok((i.op.pop as usize, i.op.push as usize))
}
fn output_type(
    a: &Assembly,
    i: &Instruction,
    popped: &[Type],
    args: &[Type],
    locals: &[Type],
) -> Result<Type> {
    let op = i.opcode;
    let var = variable(i);
    Ok(match op {
        0x02..=0x05 | 0x0e | 0xfe09 => args
            .get(var.unwrap_or(usize::MAX))
            .cloned()
            .ok_or_else(|| Error::cil("Argument index out of bounds"))?,
        0x0f | 0xfe0a => Type::ByRef(Box::new(
            args.get(var.unwrap_or(usize::MAX))
                .cloned()
                .ok_or_else(|| Error::cil("Argument address out of bounds"))?,
        )),
        0x06..=0x09 | 0x11 | 0xfe0c => locals
            .get(var.unwrap_or(usize::MAX))
            .cloned()
            .ok_or_else(|| Error::cil("Local index out of bounds"))?,
        0x12 | 0xfe0d => Type::ByRef(Box::new(
            locals
                .get(var.unwrap_or(usize::MAX))
                .cloned()
                .ok_or_else(|| Error::cil("Local address out of bounds"))?,
        )),
        0x14 => Type::primitive("object"),
        0x15..=0x20 => Type::primitive("int"),
        0x21 => Type::primitive("long"),
        0x22 => Type::primitive("float"),
        0x23 => Type::primitive("double"),
        0x28 | 0x29 | 0x6f => {
            a.method_ref(i.token().ok_or_else(|| Error::cil("Missing call token"))?)?
                .signature
                .return_type
        }
        0x72 => Type::primitive("string"),
        0x73 => Type::Named(
            a.method_ref(
                i.token()
                    .ok_or_else(|| Error::cil("Missing constructor token"))?,
            )?
            .owner,
        ),
        0x74 | 0x75 | 0xa5 | 0x71 | 0xa3 => {
            Type::Named(i.token().ok_or_else(|| Error::cil("Missing type token"))?)
        }
        0x79 => Type::ByRef(Box::new(Type::Named(
            i.token().ok_or_else(|| Error::cil("Missing type token"))?,
        ))),
        0x7b | 0x7e => {
            a.field_ref(i.token().ok_or_else(|| Error::cil("Missing field token"))?)?
                .2
        }
        0x7c | 0x7f => Type::ByRef(Box::new(
            a.field_ref(i.token().ok_or_else(|| Error::cil("Missing field token"))?)?
                .2,
        )),
        0x8c => Type::primitive("object"),
        0x8d => Type::Array(
            Box::new(Type::Named(
                i.token().ok_or_else(|| Error::cil("Missing array type"))?,
            )),
            1,
        ),
        0x8e => Type::primitive("nuint"),
        0x8f => Type::ByRef(Box::new(Type::Named(
            i.token()
                .ok_or_else(|| Error::cil("Missing element type"))?,
        ))),
        0x46..=0x4a | 0x90..=0x95 | 0xfe01..=0xfe05 | 0x67..=0x69 | 0x6d | 0xd1 | 0xd2 => {
            Type::primitive("int")
        }
        0x4c | 0x96 | 0x6a | 0x6e => Type::primitive("long"),
        0x4e | 0x98 | 0x6b => Type::primitive("float"),
        0x4f | 0x99 | 0x6c | 0x76 => Type::primitive("double"),
        0x4d | 0x97 | 0xd3 | 0xe0 | 0xfe06 | 0xfe07 | 0xfe0f => Type::primitive("nint"),
        0x50 | 0x9a => match popped.first() {
            Some(Type::Array(t, _)) => *t.clone(),
            _ => Type::primitive("object"),
        },
        0xfe1c => Type::primitive("int"),
        _ => popped.first().cloned().unwrap_or(Type::Unknown),
    })
}
pub fn analyze(
    a: &Assembly,
    token: u32,
    body: &MethodBody,
    cfg: &ControlFlowGraph,
) -> Result<StackAnalysis> {
    let args = arguments(a, token)?;
    let locals = a.locals(body)?;
    let mut entries = vec![None; cfg.blocks.len()];
    entries[0] = Some(vec![]);
    let mut queue = VecDeque::from([0]);
    for e in &body.exceptions {
        let id = cfg.by_offset[&e.handler_start];
        let entry = if e.kind == "catch" || e.kind == "filter" {
            vec![
                e.catch_type
                    .map(Type::Named)
                    .unwrap_or(Type::primitive("object")),
            ]
        } else {
            vec![]
        };
        entries[id] = Some(entry);
        queue.push_back(id);
        if let Some(f) = e.filter_start {
            let id = cfg.by_offset[&f];
            entries[id] = Some(vec![Type::primitive("object")]);
            queue.push_back(id);
        }
    }
    let mut maximum = 0;
    let mut steps = 0;
    while let Some(id) = queue.pop_front() {
        let mut stack = entries[id]
            .clone()
            .ok_or_else(|| Error::cil("Missing entry stack"))?;
        let block = &cfg.blocks[id];
        for i in &body.instructions[block.first..block.last] {
            steps += 1;
            if steps > 2_000_000 {
                return Err(Error::limit("Stack analysis work budget exceeded"));
            }
            maximum = maximum.max(stack.len());
            if let Some(v) = variable(i) {
                let bound = if matches!(i.opcode,0x02..=0x05|0x0e..=0x10|0xfe09..=0xfe0b) {
                    args.len()
                } else {
                    locals.len()
                };
                if v >= bound {
                    return Err(Error::cil(format!("Invalid variable index {v}")));
                }
            }
            let (pop, push) = effect(a, token, i)?;
            if stack.len() < pop {
                return Err(Error::cil(format!(
                    "Stack underflow at IL_{:04X}",
                    i.offset
                )));
            }
            let popped = stack.split_off(stack.len() - pop);
            if i.opcode == 0x25 {
                stack.extend(popped.clone());
                stack.extend(popped);
            } else {
                let ty = output_type(a, i, &popped, &args, &locals)?;
                for _ in 0..push {
                    stack.push(ty.clone());
                }
            }
            if matches!(i.opcode, 0xdd | 0xde) {
                stack.clear();
            }
            if matches!(i.opcode, 0x2a | 0x7a | 0xfe1a | 0xdc | 0xfe11) && !stack.is_empty() {
                return Err(Error::cil(format!("Non-empty stack at {}", i.name)));
            }
            maximum = maximum.max(stack.len());
            if stack.len() > body.max_stack as usize {
                return Err(Error::cil("Evaluation stack exceeds declared maxstack"));
            }
        }
        for next in &block.successors {
            let change = if let Some(existing) = &mut entries[*next] {
                if existing.len() != stack.len() {
                    return Err(Error::cil(format!(
                        "Incompatible stack heights at IL_{:04X}",
                        cfg.blocks[*next].start
                    )));
                }
                let mut change = false;
                for (old, new) in existing.iter_mut().zip(&stack) {
                    if old != new && *old != Type::Unknown {
                        *old = Type::Unknown;
                        change = true;
                    }
                }
                change
            } else {
                entries[*next] = Some(stack.clone());
                true
            };
            if change {
                queue.push_back(*next);
            }
        }
    }
    Ok(StackAnalysis {
        maximum,
        reachable_blocks: entries.iter().filter(|e| e.is_some()).count(),
        entry_stacks: entries,
    })
}
