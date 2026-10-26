//! Static detection of call-forwarding "proxy" methods, a common call-indirection
//! obfuscation. A proxy is a static method whose only work is to push its arguments
//! in order and forward them to one target with a single call/callvirt, then return.
//! Call sites of such a proxy can be redirected straight to the target: the stack is
//! identical, so the substitution is behaviour-preserving. Proxy definitions are kept;
//! only the `call proxy` instruction at each site is rewritten. Nothing executes.
use crate::{
    assembly::Assembly,
    cil::{Instruction, Operand},
};
use std::collections::HashMap;

/// Argument index loaded by an ldarg form, or `None` for any other instruction.
fn arg_index(i: &Instruction) -> Option<usize> {
    match (i.name.as_str(), &i.operand) {
        ("ldarg.0", _) => Some(0),
        ("ldarg.1", _) => Some(1),
        ("ldarg.2", _) => Some(2),
        ("ldarg.3", _) => Some(3),
        ("ldarg" | "ldarg.s", Operand::Variable(n)) => Some(*n as usize),
        _ => None,
    }
}

/// `token` → (call opcode to use, final target token) for every recognized forwarder.
pub struct Table {
    map: HashMap<u32, (u16, u32)>,
}
impl Table {
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    /// The call to substitute for `call proxy`, if `proxy` is a recognized forwarder.
    pub fn resolve(&self, proxy: u32) -> Option<(u16, u32)> {
        self.map.get(&proxy).copied()
    }
}

/// A single-hop forwarder's (opcode, target), without chain resolution.
fn forwarder(a: &Assembly, token: u32) -> Option<(u16, u32)> {
    let row = a.method_row(token).ok()?;
    // Static, ordinary IL method body (no P/Invoke, no runtime/internal impl).
    if row.rva == 0 || row.flags & 0x10 == 0 || row.flags & 0x2000 != 0 || row.impl_flags != 0 {
        return None;
    }
    let m = a.method_ref(token).ok()?;
    if m.signature.has_this
        || m.signature.generic_count != 0
        || m.signature.vararg_at.is_some()
        || a.generics.contains_key(&m.owner)
    {
        return None;
    }
    let body = a.body(token).ok()??;
    if !body.exceptions.is_empty() {
        return None;
    }
    let code: Vec<&Instruction> = body
        .instructions
        .iter()
        .filter(|i| i.name != "nop")
        .collect();
    let n = m.signature.parameters.len();
    // Exactly: ldarg.0 … ldarg.(n-1), one call/callvirt, ret.
    if code.len() != n + 2 {
        return None;
    }
    for (k, instruction) in code.iter().take(n).enumerate() {
        if arg_index(instruction) != Some(k) {
            return None;
        }
    }
    let opcode = match code[n].name.as_str() {
        "call" => 0x28,
        "callvirt" => 0x6f,
        _ => return None,
    };
    if code[n + 1].name != "ret" {
        return None;
    }
    let target = code[n].token()?;
    if target == token || !matches!(target >> 24, 0x06 | 0x0a | 0x2b) {
        return None;
    }
    let t = a.method_ref(target).ok()?;
    // The target must consume exactly the forwarded arguments and return the same type.
    if t.signature.parameters.len() + usize::from(t.signature.has_this) != n
        || t.signature.return_type != m.signature.return_type
    {
        return None;
    }
    Some((opcode, target))
}

/// Builds the forwarder table and follows proxy→proxy chains to the final target.
pub fn detect(a: &Assembly) -> Table {
    let mut raw: HashMap<u32, (u16, u32)> = HashMap::new();
    for index in 0..a.metadata().method_defs.len() {
        let token = 0x0600_0001 + index as u32;
        if let Some(hop) = forwarder(a, token) {
            raw.insert(token, hop);
        }
    }
    let mut map = HashMap::with_capacity(raw.len());
    for (&proxy, &(mut op, mut target)) in &raw {
        let mut seen = vec![proxy];
        // Skip intermediate proxies; each hop keeps the stack identical.
        while let Some(&(next_op, next_target)) = raw.get(&target) {
            if seen.contains(&target) {
                break; // cycle: stop at the current resolved target
            }
            seen.push(target);
            op = next_op;
            target = next_target;
            if seen.len() > 64 {
                break;
            }
        }
        if target != proxy {
            map.insert(proxy, (op, target));
        }
    }
    Table { map }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PROXIES: &[u8] = include_bytes!("../../../../samples/fixtures/ILens.Proxies.dll");
    fn token(a: &Assembly, name: &str) -> u32 {
        a.nodes
            .iter()
            .find(|n| n.token >> 24 == 6 && n.name == name)
            .unwrap()
            .token
    }
    #[test]
    fn forwarders_resolve_through_chains_and_reject_non_forwarders() {
        let a = Assembly::load(PROXIES.to_vec()).unwrap();
        let table = detect(&a);
        // Pure static forwarder → its target.
        assert_eq!(
            table.resolve(token(&a, "Forward")),
            Some((0x28, token(&a, "Real")))
        );
        // Chain Hop1 → Hop2 → Real resolves fully to Real.
        assert_eq!(
            table.resolve(token(&a, "Hop1")),
            Some((0x28, token(&a, "Real")))
        );
        assert_eq!(
            table.resolve(token(&a, "Hop2")),
            Some((0x28, token(&a, "Real")))
        );
        // Instance target keeps callvirt.
        let (op, target) = table.resolve(token(&a, "ForwardScale")).unwrap();
        assert_eq!(op, 0x6f);
        assert_eq!(a.method_ref(target).unwrap().name, "Scale");
        assert_eq!(table.resolve(token(&a, "ForwardReset")).unwrap().0, 0x6f);
        // Not forwarders: extra work, argument reordering, and the real targets.
        assert_eq!(table.resolve(token(&a, "NotProxy")), None);
        assert_eq!(table.resolve(token(&a, "Swapped")), None);
        assert_eq!(table.resolve(token(&a, "Real")), None);
        assert_eq!(table.resolve(token(&a, "UseStatic")), None);
    }
}
