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
use crate::{cil_encode::EditableInstruction, emulate};
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
