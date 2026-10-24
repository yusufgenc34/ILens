//! Conservative metadata binding. Display names alone never establish type identity.
use crate::{
    assembly::{Assembly, Identity, coded},
    session::Session,
    signature::{self, MethodSignature, Type},
};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Accessor {
    pub name: String,
    pub getter: bool,
    pub is_virtual: bool,
    /// The type's default member with parameters: C# `receiver[arguments]`.
    #[serde(default)]
    pub indexer: bool,
}

pub fn local_accessor(a: &Assembly, token: u32) -> Option<Accessor> {
    if token >> 24 != 6 {
        return None;
    }
    let method = a.method_ref(token).ok()?;
    if method.signature.generic_count != 0 {
        return None;
    }
    let mut candidates = a.semantics.iter().flat_map(|(property, methods)| {
        methods
            .iter()
            .filter(move |(_, m)| *m == token && property >> 24 == 23)
            .map(move |(flags, _)| (*property, *flags))
    });
    let (token, flags) = candidates.next()?;
    if candidates.next().is_some() || !matches!(flags, 1 | 2) {
        return None;
    }
    if !a.owners.contains_key(&token) || a.owners.get(&token) != a.owners.get(&method.token) {
        return None;
    }
    let row = a
        .metadata()
        .properties
        .get((token & 0xffffff).checked_sub(1)? as usize)?;
    let property = signature::method(a.blob(row.property_type).ok()?).ok()?;
    let name = a.string(row.name).ok()?.to_owned();
    // A property with parameters is usable from C# only as the type's indexer.
    let indexer = !property.parameters.is_empty();
    if indexer
        && crate::attributes::default_member(a, *a.owners.get(&token)?).as_deref()
            != Some(name.as_str())
        || property.has_this != method.signature.has_this
    {
        return None;
    }
    let getter = flags == 2;
    if getter {
        if method.signature.parameters != property.parameters
            || method.signature.return_type != property.return_type
        {
            return None;
        }
    } else {
        let mut expected = property.parameters.clone();
        expected.push(property.return_type.clone());
        if !method.signature.return_type.is_void() || method.signature.parameters != expected {
            return None;
        }
    }
    Some(Accessor {
        name,
        getter,
        is_virtual: a.method_row(method.token).ok()?.flags & 0x40 != 0,
        indexer,
    })
}

fn named_identity(a: &Assembly, token: u32) -> Option<(Identity, String)> {
    let mut scope = token;
    for _ in 0..49 {
        match scope >> 24 {
            1 => {
                scope = coded(
                    a.metadata()
                        .type_refs
                        .get((scope & 0xffffff).checked_sub(1)? as usize)?
                        .resolution_scope,
                )
            }
            0 | 2 => return Some((a.info.identity.clone(), a.resolve_full(token))),
            35 => {
                return Some((
                    a.references
                        .iter()
                        .find(|r| r.token == scope)?
                        .identity
                        .clone(),
                    a.resolve_full(token),
                ));
            }
            _ => return None,
        }
    }
    None
}
fn same_type(
    a: &Assembly,
    left: &Type,
    b: &Assembly,
    right: &Type,
    depth: usize,
    budget: &mut usize,
) -> bool {
    if depth > 48 || *budget == 0 {
        return false;
    }
    *budget -= 1;
    match (left, right) {
        (Type::Primitive(l), Type::Primitive(r)) => l == r,
        (Type::Named(l), Type::Named(r)) | (Type::ValueType(l), Type::ValueType(r)) => {
            match (named_identity(a, *l), named_identity(b, *r)) {
                (Some((li, ln)), Some((ri, rn))) => li.matches(&ri) && ln == rn,
                _ => false,
            }
        }
        (
            Type::Generic {
                method: lm,
                index: li,
            },
            Type::Generic {
                method: rm,
                index: ri,
            },
        ) => lm == rm && li == ri,
        (Type::Array(l, ln), Type::Array(r, rn))
        | (Type::MultiArray(l, ln), Type::MultiArray(r, rn)) => {
            ln == rn && same_type(a, l, b, r, depth + 1, budget)
        }
        (Type::Pointer(l), Type::Pointer(r))
        | (Type::ByRef(l), Type::ByRef(r))
        | (Type::Pinned(l), Type::Pinned(r)) => same_type(a, l, b, r, depth + 1, budget),
        (
            Type::GenericInstance { base: l, args: la },
            Type::GenericInstance { base: r, args: ra },
        ) => {
            la.len() == ra.len()
                && same_type(a, l, b, r, depth + 1, budget)
                && la
                    .iter()
                    .zip(ra)
                    .all(|(l, r)| same_type(a, l, b, r, depth + 1, budget))
        }
        (
            Type::Modified {
                required: lr,
                modifier: lm,
                inner: l,
            },
            Type::Modified {
                required: rr,
                modifier: rm,
                inner: r,
            },
        ) => {
            lr == rr
                && same_type(
                    a,
                    &Type::Named(*lm),
                    b,
                    &Type::Named(*rm),
                    depth + 1,
                    budget,
                )
                && same_type(a, l, b, r, depth + 1, budget)
        }
        (Type::FunctionPointer(l), Type::FunctionPointer(r)) => {
            same_signature(a, l, b, r, depth + 1, budget)
        }
        _ => false,
    }
}
fn same_signature(
    a: &Assembly,
    l: &MethodSignature,
    b: &Assembly,
    r: &MethodSignature,
    depth: usize,
    budget: &mut usize,
) -> bool {
    depth <= 48
        && l.has_this == r.has_this
        && l.explicit_this == r.explicit_this
        && l.convention == r.convention
        && l.generic_count == r.generic_count
        && l.vararg_at == r.vararg_at
        && l.parameters.len() == r.parameters.len()
        && same_type(a, &l.return_type, b, &r.return_type, depth + 1, budget)
        && l.parameters
            .iter()
            .zip(&r.parameters)
            .all(|(l, r)| same_type(a, l, b, r, depth + 1, budget))
}
pub fn same_member(a: &Assembly, left: u32, b: &Assembly, right: u32, field: bool) -> bool {
    let mut budget = 4096;
    if field {
        match (a.field_ref(left), b.field_ref(right)) {
            (Ok((_, _, l)), Ok((_, _, r))) => same_type(a, &l, b, &r, 0, &mut budget),
            _ => false,
        }
    } else {
        match (a.method_ref(left), b.method_ref(right)) {
            (Ok(l), Ok(r)) => same_signature(a, &l.signature, b, &r.signature, 0, &mut budget),
            _ => false,
        }
    }
}
impl Session {
    pub fn accessor(&self, id: u32, token: u32) -> Option<Accessor> {
        let (target, method) = self.resolve_definition(id, token)?;
        local_accessor(self.assembly(target).ok()?, method)
    }
}
