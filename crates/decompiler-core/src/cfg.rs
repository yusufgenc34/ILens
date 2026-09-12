use crate::{
    cil::{Flow, MethodBody},
    error::{Error, Result},
};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
#[derive(Debug, Clone, Serialize)]
pub struct Block {
    pub id: usize,
    pub start: u32,
    pub end: u32,
    pub first: usize,
    pub last: usize,
    pub successors: Vec<usize>,
    pub predecessors: Vec<usize>,
    pub exception_successors: Vec<usize>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ControlFlowGraph {
    pub blocks: Vec<Block>,
    #[serde(skip)]
    pub by_offset: HashMap<u32, usize>,
}
impl ControlFlowGraph {
    pub fn build(body: &MethodBody) -> Result<Self> {
        let mut leaders = BTreeSet::from([0]);
        for i in &body.instructions {
            leaders.extend(i.targets());
            if matches!(
                i.op.flow,
                Flow::Branch | Flow::CondBranch | Flow::Return | Flow::Throw
            ) && i.offset + i.size < body.code_size
            {
                leaders.insert(i.offset + i.size);
            }
        }
        for e in &body.exceptions {
            leaders.extend([e.try_start, e.handler_start]);
            if e.try_end < body.code_size {
                leaders.insert(e.try_end);
            }
            if e.handler_end < body.code_size {
                leaders.insert(e.handler_end);
            }
            if let Some(f) = e.filter_start {
                leaders.insert(f);
            }
        }
        if leaders.len() > 8192 {
            return Err(Error::limit("Method exceeds 8192 basic blocks"));
        }
        let boundaries: Vec<_> = leaders.into_iter().collect();
        let starts: HashMap<_, _> = body
            .instructions
            .iter()
            .enumerate()
            .map(|(i, ins)| (ins.offset, i))
            .collect();
        let mut blocks = Vec::new();
        let by_offset: HashMap<_, _> = boundaries
            .iter()
            .enumerate()
            .map(|(i, p)| (*p, i))
            .collect();
        for (id, start) in boundaries.iter().enumerate() {
            let end = boundaries.get(id + 1).copied().unwrap_or(body.code_size);
            let first = *starts
                .get(start)
                .ok_or_else(|| Error::cil("Invalid block boundary"))?;
            let last = starts.get(&end).copied().unwrap_or(body.instructions.len());
            blocks.push(Block {
                id,
                start: *start,
                end,
                first,
                last,
                successors: vec![],
                predecessors: vec![],
                exception_successors: vec![],
            });
        }
        for b in &mut blocks {
            let last = body
                .instructions
                .get(b.last - 1)
                .ok_or_else(|| Error::cil("Empty basic block"))?;
            for t in last.targets() {
                b.successors.push(
                    *by_offset
                        .get(&t)
                        .ok_or_else(|| Error::cil("Invalid CFG target"))?,
                );
            }
            if !matches!(last.op.flow, Flow::Branch | Flow::Return | Flow::Throw)
                && b.end < body.code_size
            {
                b.successors.push(b.id + 1);
            }
            if !matches!(last.op.flow, Flow::Branch | Flow::Return | Flow::Throw)
                && b.end == body.code_size
            {
                return Err(Error::cil("Control flow falls past the method body"));
            }
            b.successors.sort_unstable();
            b.successors.dedup();
            for e in &body.exceptions {
                if b.start >= e.try_start && b.start < e.try_end {
                    let target = e.filter_start.unwrap_or(e.handler_start);
                    b.exception_successors.push(
                        *by_offset
                            .get(&target)
                            .ok_or_else(|| Error::cil("Invalid exception block"))?,
                    );
                }
            }
            b.exception_successors.sort_unstable();
            b.exception_successors.dedup();
        }
        for id in 0..blocks.len() {
            for next in blocks[id].successors.clone() {
                blocks[next].predecessors.push(id);
            }
        }
        Ok(Self { blocks, by_offset })
    }
}
