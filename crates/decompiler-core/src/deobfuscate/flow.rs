//! Bounded de-flattening of switch-dispatcher control flow.
//!
//! A flattened method keeps its next block in a state local and returns to one
//! `ldloc state; switch (...)` dispatcher after every block. Where a transition
//! stores a literal state, the dispatcher's choice is already decided, so the
//! transition can branch straight to the block the dispatcher would have picked.
//!
//! Nothing is inferred about the state value: only stores whose literal sits
//! immediately before them, in the same basic block, are resolved. Any other store
//! leaves the local live and the dispatcher reachable and untouched.
use crate::{cil_encode::EditableInstruction, deobfuscate::eaz::constant, emulate};
use std::collections::{HashMap, HashSet};

/// The local a `ldloc`/`stloc`/`ldloca` row addresses, in any of its encodings.
fn local(row: &EditableInstruction) -> Option<(&'static str, u32)> {
    let (kind, rest) = ["ldloca", "ldloc", "stloc"]
        .into_iter()
        .find_map(|k| row.opcode.strip_prefix(k).map(|rest| (k, rest)))?;
    let index = match rest {
        "" | ".s" => row.operand.trim().parse().ok()?,
        _ => rest.strip_prefix('.')?.parse().ok()?,
    };
    Some((kind, index))
}
/// `(dispatcher row, its switch targets)` keyed by the row index of the `ldloc`.
type Dispatchers = HashMap<usize, (u32, Vec<String>)>;
fn dispatchers(rows: &[EditableInstruction]) -> Dispatchers {
    let mut found = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        let Some(("ldloc", state)) = local(row) else {
            continue;
        };
        let Some(switch) = rows.get(index + 1).filter(|r| r.opcode == "switch") else {
            continue;
        };
        // The row after the dispatcher is where an out-of-range state falls through to.
        let Some(fallthrough) = rows.get(index + 2) else {
            continue;
        };
        let mut targets: Vec<String> = switch
            .operand
            .split(',')
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect();
        targets.push(fallthrough.id.clone());
        found.insert(index, (state, targets));
    }
    found
}
/// Rewrites resolvable state transitions into direct branches and reports how many.
/// Rows made unreachable, including the dispatcher itself, are left for the existing
/// unreachable-instruction pass to remove.
pub(crate) fn unflatten(rows: &mut [EditableInstruction]) -> usize {
    let dispatch = dispatchers(rows);
    if dispatch.is_empty() || rows.len() > 65_536 {
        return 0;
    }
    let labels: HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.as_str(), i))
        .collect();
    let branch_targets: HashSet<&str> = rows
        .iter()
        .filter(|r| {
            matches!(r.opcode.as_str(), "br" | "br.s" | "switch")
                || r.opcode.starts_with('b')
                || r.opcode.starts_with("leave")
        })
        .flat_map(|r| r.operand.split(',').map(str::trim))
        .filter(|t| labels.contains_key(t))
        .collect();
    // A state local may only be read by its dispatchers and never have its address taken.
    let mut rejected = HashSet::new();
    for (index, row) in rows.iter().enumerate() {
        match local(row) {
            Some(("ldloca", state)) => {
                rejected.insert(state);
            }
            Some(("ldloc", state)) if !dispatch.contains_key(&index) => {
                rejected.insert(state);
            }
            _ => {}
        }
    }
    let mut rewrites = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let Some(("stloc", state)) = local(row) else {
            continue;
        };
        if rejected.contains(&state) || index == 0 || branch_targets.contains(row.id.as_str()) {
            continue;
        }
        let source = &rows[index - 1];
        let Some(value) = emulate::constant(&source.opcode, &source.operand) else {
            continue;
        };
        // Where control reaches the dispatcher from here: straight through, or one branch.
        let next = match rows.get(index + 1) {
            Some(r) if matches!(r.opcode.as_str(), "br" | "br.s") => {
                labels.get(r.operand.trim()).copied()
            }
            Some(_) => Some(index + 1),
            None => None,
        };
        let Some((dispatched, targets)) = next.and_then(|n| dispatch.get(&n)) else {
            continue;
        };
        if *dispatched != state {
            continue;
        }
        let target = usize::try_from(value)
            .ok()
            .filter(|&v| v < targets.len() - 1)
            .map_or_else(
                || targets[targets.len() - 1].clone(),
                |v| targets[v].clone(),
            );
        rewrites.push((index, target));
    }
    for (index, target) in &rewrites {
        rows[index - 1].opcode = "nop".into();
        rows[index - 1].operand.clear();
        rows[*index].opcode = "br".into();
        rows[*index].operand = target.clone();
    }
    rewrites.len()
}

/// The same de-flattening against a decoded body, as in-place byte patches
/// `(code offset, replacement bytes)`. Only the literal, the store and the existing
/// branch's displacement are touched; the branch keeps its opcode and its width, so no
/// instruction moves and no exception region shifts. Used where whole-method rewriting
/// is unavailable, which is most protected assemblies.
pub(crate) fn patches(body: &crate::cil::MethodBody) -> Vec<(u32, Vec<u8>)> {
    use crate::cil::{Instruction, Operand};
    /// The local a `ldloc`/`stloc`/`ldloca` instruction addresses, in any encoding.
    fn addressed(i: &Instruction) -> Option<(&'static str, u32)> {
        let (kind, rest) = ["ldloca", "ldloc", "stloc"]
            .into_iter()
            .find_map(|k| i.name.strip_prefix(k).map(|rest| (k, rest)))?;
        let index = match (rest, &i.operand) {
            ("" | ".s", Operand::Variable(n)) => u32::from(*n),
            ("" | ".s", _) => return None,
            _ => rest.strip_prefix('.')?.parse().ok()?,
        };
        Some((kind, index))
    }
    let at = |offset: u32| {
        body.instructions
            .binary_search_by_key(&offset, |i| i.offset)
            .ok()
    };
    // Dispatchers, by the offset of their `ldloc`: (state local, case targets, fallthrough).
    let mut dispatch = HashMap::new();
    for (index, i) in body.instructions.iter().enumerate() {
        let Some(("ldloc", state)) = addressed(i) else {
            continue;
        };
        let (Some(switch), Some(after)) = (
            body.instructions
                .get(index + 1)
                .filter(|r| r.name == "switch"),
            body.instructions.get(index + 2),
        ) else {
            continue;
        };
        let mut targets = switch.targets();
        targets.push(after.offset);
        dispatch.insert(i.offset, (state, targets));
    }
    if dispatch.is_empty() {
        return vec![];
    }
    // A state local may only be read by its dispatchers and never have its address taken.
    let rejected: HashSet<u32> = body
        .instructions
        .iter()
        .filter_map(|i| match addressed(i) {
            Some(("ldloca", state)) => Some(state),
            Some(("ldloc", state)) if !dispatch.contains_key(&i.offset) => Some(state),
            _ => None,
        })
        .collect();
    let boundaries = crate::pe_write::literal_boundaries(body);
    let mut out = Vec::new();
    for (index, store) in body.instructions.iter().enumerate() {
        let Some(("stloc", state)) = addressed(store) else {
            continue;
        };
        let (Some(source), Some(branch)) = (
            index.checked_sub(1).and_then(|i| body.instructions.get(i)),
            body.instructions.get(index + 1),
        ) else {
            continue;
        };
        // Nothing may enter between the literal and the branch, and nothing may jump at
        // the branch itself: its target is about to mean something else.
        if rejected.contains(&state)
            || !matches!(branch.name.as_str(), "br" | "br.s")
            || boundaries.contains(&store.offset)
            || boundaries.contains(&branch.offset)
            || source.offset + source.size != store.offset
            || store.offset + store.size != branch.offset
        {
            continue;
        }
        let Some(value) = constant(source) else {
            continue;
        };
        let Operand::Branch(current) = branch.operand else {
            continue;
        };
        let Some((dispatched, targets)) = dispatch.get(&current) else {
            continue;
        };
        if *dispatched != state || at(current).is_none() {
            continue;
        }
        let target = usize::try_from(value)
            .ok()
            .filter(|&v| v < targets.len() - 1)
            .map_or_else(|| targets[targets.len() - 1], |v| targets[v]);
        let end = branch.offset + branch.size;
        let Some(delta) = i64::from(target).checked_sub(i64::from(end)) else {
            continue;
        };
        // The branch keeps its encoding, so the new target must fit the operand it has.
        let encoded = match (
            branch.name.as_str(),
            i8::try_from(delta),
            i32::try_from(delta),
        ) {
            ("br.s", Ok(d), _) => vec![0x2b, d as u8],
            ("br", _, Ok(d)) => [&[0x38u8][..], &d.to_le_bytes()[..]].concat(),
            _ => continue,
        };
        if encoded.len() as u32 != branch.size {
            continue;
        }
        let mut bytes = vec![0u8; (store.offset + store.size - source.offset) as usize];
        bytes.extend(encoded);
        out.push((source.offset, bytes));
    }
    out
}
