//! Debugger queries that an anti-debug guard branches on.
//!
//! Nothing is removed and no guard method is deleted. The copy is rewritten to answer
//! the three `System.Diagnostics.Debugger` queries the way a process runs when no
//! debugger is attached, which is the state the copy is read in. The constant and
//! branch passes then fold the guard away on their own. Every replacement is exactly
//! as long as the `call` it replaces, so no offset moves.
use crate::assembly::{Assembly, coded};
use std::collections::HashMap;

/// `nop nop nop nop ldc.i4.0`: the five bytes a bool-returning query becomes.
const FALSE: [u8; 5] = [0x00, 0x00, 0x00, 0x00, 0x16];
/// Five `nop`s: a void query leaves nothing on the stack, so it simply goes away.
const NOTHING: [u8; 5] = [0x00; 5];

/// Scope checks select the modeled member, they do not authenticate a DLL.
fn core_scope(a: &Assembly, type_ref: u32) -> bool {
    let Some(row) = a
        .metadata()
        .type_refs
        .get((type_ref & 0xffffff).saturating_sub(1) as usize)
    else {
        return false;
    };
    let scope = coded(row.resolution_scope);
    a.references.iter().any(|r| {
        r.token == scope
            && r.identity.culture == "neutral"
            && matches!(
                (
                    r.identity.name.as_str(),
                    r.identity.public_key_token.as_str()
                ),
                ("mscorlib", "b77a5c561934e089")
                    | ("System.Runtime", "b03f5f7f11d50a3a")
                    | ("System.Private.CoreLib", "7cec85d7bea7798e")
                    | ("netstandard", "cc7b13ffcd2ddd51")
                    | ("System.Diagnostics.Debug", "b03f5f7f11d50a3a")
            )
    })
}
/// MemberRef tokens of the modeled `Debugger` queries, each with its replacement bytes.
pub(crate) fn debugger_members(a: &Assembly) -> HashMap<u32, [u8; 5]> {
    use crate::signature::Type;
    let mut found = HashMap::new();
    for index in 0..a.metadata().member_refs.len().min(1 << 20) {
        let token = 0x0a000001 + index as u32;
        let Ok(m) = a.method_ref(token) else { continue };
        if m.owner >> 24 != 1
            || a.resolve_full(m.owner) != "System.Diagnostics.Debugger"
            || !core_scope(a, m.owner)
            || m.signature.has_this
            || m.signature.generic_count != 0
            || !m.signature.parameters.is_empty()
        {
            continue;
        }
        let bytes = match (m.name.as_str(), &m.signature.return_type) {
            ("get_IsAttached" | "IsLogging", Type::Primitive(t)) if t == "bool" => FALSE,
            ("Break", t) if t.is_void() => NOTHING,
            _ => continue,
        };
        found.insert(token, bytes);
    }
    found
}
