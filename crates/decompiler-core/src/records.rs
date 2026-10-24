//! C# record classes. The compiler synthesizes equality, printing, cloning and
//! deconstruction members marked [CompilerGenerated]; ILSpy/dnSpy show the record
//! declaration instead. A primary constructor is recognized only when its body is
//! exactly the stores of its parameters into init-only auto-properties.
use crate::{
    assembly::{Assembly, coded},
    attributes::compiler_generated,
    cil::Operand,
    signature::Type,
};
use std::collections::HashSet;

pub struct Record {
    /// `(int X, int Y)` for a positional record, otherwise empty.
    pub parameters: String,
    /// Synthesized members, and the primary constructor with its properties.
    pub hidden: HashSet<u32>,
}

const SYNTHESIZED: &[&str] = &[
    "ToString",
    "PrintMembers",
    "op_Equality",
    "op_Inequality",
    "GetHashCode",
    "Equals",
    "<Clone>$",
    "Deconstruct",
    "get_EqualityContract",
    "EqualityContract",
];

/// Whether a setter is init-only: its return type carries modreq(IsExternalInit).
pub fn init_only(a: &Assembly, setter: u32) -> bool {
    a.method_ref(setter).is_ok_and(|m| {
        matches!(&m.signature.return_type, Type::Modified { required: true, modifier, .. }
            if a.resolve_full(*modifier) == "System.Runtime.CompilerServices.IsExternalInit")
    })
}

pub fn record(a: &Assembly, ty: u32) -> Option<Record> {
    let node = a
        .nodes
        .iter()
        .find(|n| n.token == ty && n.kind == "class")?;
    let members: Vec<_> = a.nodes.iter().filter(|n| n.parent == ty).collect();
    if !members
        .iter()
        .any(|m| m.name == "<Clone>$" && compiler_generated(a, m.token))
    {
        return None;
    }
    // Derived records pass parameters to their base record; only object bases here.
    let row = &a.metadata().type_defs[(node.token & 0xffffff) as usize - 1];
    if a.resolve_full(coded(row.extends)) != "System.Object" {
        return None;
    }
    let mut hidden: HashSet<u32> = HashSet::new();
    for m in &members {
        let synthesized = SYNTHESIZED.contains(&m.name.as_str())
            || m.name == ".ctor"
                && a.method_ref(m.token).is_ok_and(
                    |r| matches!(r.signature.parameters.as_slice(), [Type::Named(t)] if *t == ty),
                );
        if synthesized && compiler_generated(a, m.token) {
            hidden.insert(m.token);
            hidden.extend(
                a.semantics
                    .get(&m.token)
                    .into_iter()
                    .flatten()
                    .map(|(_, t)| *t),
            );
        }
    }
    let parameters = primary_constructor(a, &members, &mut hidden).unwrap_or_default();
    Some(Record { parameters, hidden })
}

// `.ctor(T1 P1, …) { this.<P1>k__BackingField = P1; …; base..ctor(); }` whose
// properties are init-only auto-properties named like the parameters.
fn primary_constructor(
    a: &Assembly,
    members: &[&crate::assembly::Node],
    hidden: &mut HashSet<u32>,
) -> Option<String> {
    let candidates: Vec<u32> = members
        .iter()
        .filter(|m| m.name == ".ctor" && !hidden.contains(&m.token))
        .map(|m| m.token)
        .collect();
    let matched: Vec<_> = candidates
        .into_iter()
        .filter_map(|ctor| positional(a, members, ctor))
        .collect();
    let [(parameters, tokens)] = matched.as_slice() else {
        return None;
    };
    hidden.extend(tokens);
    Some(parameters.clone())
}
/// The parameter list and the members a positional constructor stands for.
fn positional(
    a: &Assembly,
    members: &[&crate::assembly::Node],
    ctor: u32,
) -> Option<(String, Vec<u32>)> {
    let m = a.method_ref(ctor).ok()?;
    let names = a.parameter_names(ctor, &m.signature);
    if names.is_empty() {
        return None;
    }
    let body = a.body(ctor).ok()??;
    let code: Vec<_> = body.instructions.iter().filter(|i| i.opcode != 0).collect();
    let mut stored = vec![];
    let mut k = 0;
    while k + 2 < code.len() && code[k].opcode == 0x02 && code[k + 2].opcode == 0x7d {
        let argument = match (code[k + 1].opcode, &code[k + 1].operand) {
            (0x03..=0x05, _) => usize::from(code[k + 1].opcode - 0x02),
            (0x0e, Operand::Variable(v)) => usize::from(*v),
            _ => break,
        };
        let Operand::Token(field) = code[k + 2].operand else {
            break;
        };
        stored.push((argument, field));
        k += 3;
    }
    let tail: Vec<u16> = code[k..].iter().map(|i| i.opcode).collect();
    let base_call = matches!(code.get(k + 1).map(|i| &i.operand), Some(Operand::Token(t))
        if a.method_ref(*t).is_ok_and(|b| b.name == ".ctor" && a.resolve_full(b.owner) == "System.Object"));
    if tail != [0x02, 0x28, 0x2a] || !base_call || stored.len() != names.len() {
        return None;
    }
    let mut tokens = vec![ctor];
    for (index, (argument, field)) in stored.iter().enumerate() {
        let name = &names[index];
        let property = members
            .iter()
            .find(|p| p.token >> 24 == 23 && p.name == *name)?;
        let backing = members.iter().find(|f| f.token == *field)?;
        let accessors = a.semantics.get(&property.token)?;
        let init = accessors.iter().any(|(s, t)| *s == 1 && init_only(a, *t));
        if *argument != index + 1
            || backing.name != format!("<{name}>k__BackingField")
            || !init
            || !accessors.iter().all(|(_, t)| compiler_generated(a, *t))
        {
            return None;
        }
        tokens.extend([property.token, backing.token]);
        tokens.extend(accessors.iter().map(|(_, t)| *t));
    }
    Some((
        format!("({})", a.parameter_list(ctor, &m.signature)),
        tokens,
    ))
}
