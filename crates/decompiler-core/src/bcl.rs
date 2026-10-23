//! Offline knowledge of the .NET base class library, generated at build time from
//! Microsoft's reference assemblies (`examples/bcl_summary.rs`). It answers what a
//! loaded framework assembly would: whether a referenced method is a property accessor,
//! an external type's kind, and enum member values. A reference qualifies only when its
//! scope is an assembly signed with a Microsoft framework key and the full type name and
//! accessor signature match exactly; display names alone never qualify.
use crate::{
    assembly::{Assembly, coded},
    resolution::Accessor,
    signature::Type,
};
use std::{collections::HashMap, sync::OnceLock};

/// Public key tokens of Microsoft's framework assemblies.
const FRAMEWORK_KEYS: [&str; 5] = [
    "b77a5c561934e089",
    "b03f5f7f11d50a3a",
    "cc7b13ffcd2ddd51",
    "7cec85d7bea7798e",
    "31bf3856ad364e35",
];
const DATA: &str = include_str!("../data/bcl.txt");

#[derive(Debug, Default)]
pub struct TypeInfo {
    /// c class, s struct, i interface, e enum, d delegate.
    pub kind: char,
    /// Accessor method name → (property name, getter, canonical method signature).
    pub accessors: HashMap<String, Vec<(String, bool, String)>>,
    /// Enum members in declaration order with their values.
    pub members: Vec<(String, i128)>,
}
fn data() -> &'static HashMap<String, TypeInfo> {
    static TYPES: OnceLock<HashMap<String, TypeInfo>> = OnceLock::new();
    TYPES.get_or_init(|| parse(DATA))
}
/// Format, one record per line:
/// `T <kind> <full name>`, then `P <property> <g|s> <signature>` and `E <member> <value>`
/// lines for that type. Accessor names follow the get_/set_ convention.
pub fn parse(text: &str) -> HashMap<String, TypeInfo> {
    let mut types = HashMap::new();
    let mut current: Option<(String, TypeInfo)> = None;
    for line in text.lines() {
        let mut parts = line.splitn(4, ' ');
        match parts.next() {
            Some("T") => {
                if let Some((name, info)) = current.take() {
                    types.insert(name, info);
                }
                let kind = parts.next().and_then(|k| k.chars().next()).unwrap_or('c');
                let name = parts.collect::<Vec<_>>().join(" ");
                current = Some((
                    name,
                    TypeInfo {
                        kind,
                        ..TypeInfo::default()
                    },
                ));
            }
            Some("P") => {
                let (Some((_, info)), Some(name), Some(access), Some(signature)) =
                    (current.as_mut(), parts.next(), parts.next(), parts.next())
                else {
                    continue;
                };
                for getter in [true, false] {
                    if access.contains(if getter { 'g' } else { 's' }) {
                        let method = format!("{}_{name}", if getter { "get" } else { "set" });
                        info.accessors.entry(method).or_default().push((
                            name.to_owned(),
                            getter,
                            accessor_signature(signature, getter),
                        ));
                    }
                }
            }
            Some("E") => {
                if let (Some((_, info)), Some(name), Some(value)) =
                    (current.as_mut(), parts.next(), parts.next())
                    && let Ok(value) = value.parse()
                {
                    info.members.push((name.to_owned(), value));
                }
            }
            _ => {}
        }
    }
    if let Some((name, info)) = current {
        types.insert(name, info);
    }
    types
}
// A property type `T` gives `T()` for the getter and `void(T)` for the setter.
fn accessor_signature(property: &str, getter: bool) -> String {
    if getter {
        format!("{property}()")
    } else {
        format!("void({property})")
    }
}
/// A signature-independent spelling of a type, using fully qualified metadata names and
/// `!n`/`!!n` for generic parameters, so references and definitions compare equal.
pub fn canonical(a: &Assembly, ty: &Type) -> String {
    canonical_depth(a, ty, 0)
}
fn canonical_depth(a: &Assembly, ty: &Type, depth: usize) -> String {
    if depth > 32 {
        return "?".to_owned();
    }
    let inner = |t: &Type| canonical_depth(a, t, depth + 1);
    match ty {
        Type::Primitive(name) => name.clone(),
        Type::Named(t) | Type::ValueType(t) => a.resolve_full(*t),
        Type::Generic { method, index } => format!("{}{index}", if *method { "!!" } else { "!" }),
        Type::Array(t, rank) | Type::MultiArray(t, rank) => {
            format!(
                "{}[{}]",
                inner(t),
                ",".repeat(rank.saturating_sub(1) as usize)
            )
        }
        Type::Pointer(t) => format!("{}*", inner(t)),
        Type::ByRef(t) => format!("{}&", inner(t)),
        Type::Pinned(t) | Type::Modified { inner: t, .. } => inner(t),
        Type::GenericInstance { base, args } => format!(
            "{}<{}>",
            inner(base),
            args.iter().map(inner).collect::<Vec<_>>().join(",")
        ),
        Type::FunctionPointer(_) | Type::Unknown => "?".to_owned(),
    }
}
/// The framework type a type reference denotes, when its resolution scope is an
/// assembly reference signed with a Microsoft framework key.
pub fn framework_type(a: &Assembly, token: u32) -> Option<(&'static TypeInfo, String)> {
    let token = match token >> 24 {
        27 => {
            let row = a
                .metadata()
                .type_specs
                .get((token & 0xffffff).checked_sub(1)? as usize)?;
            match crate::signature::type_spec(a.blob(row.signature).ok()?).ok()? {
                Type::GenericInstance { base, .. } => match *base {
                    Type::Named(t) | Type::ValueType(t) => t,
                    _ => return None,
                },
                _ => return None,
            }
        }
        _ => token,
    };
    if token >> 24 != 1 {
        return None;
    }
    // Nested type references are scoped by their enclosing type reference.
    let mut scope = token;
    for _ in 0..16 {
        let row = a
            .metadata()
            .type_refs
            .get((scope & 0xffffff).checked_sub(1)? as usize)?;
        scope = coded(row.resolution_scope);
        if scope >> 24 != 1 {
            break;
        }
    }
    let reference = a.references.iter().find(|r| r.token == scope)?;
    if !FRAMEWORK_KEYS.contains(&reference.identity.public_key_token.as_str()) {
        return None;
    }
    let name = a.resolve_full(token);
    data().get(&name).map(|info| (info, name))
}
/// Property accessor semantics for a framework method reference, verified by the
/// reference's full signature against the reference assembly's definition.
pub fn accessor(a: &Assembly, token: u32) -> Option<Accessor> {
    if token >> 24 != 10 {
        return None;
    }
    let method = a.method_ref(token).ok()?;
    if method.signature.generic_count != 0 || method.signature.vararg_at.is_some() {
        return None;
    }
    let (info, _) = framework_type(a, method.owner)?;
    let signature = format!(
        "{}({})",
        canonical(a, &method.signature.return_type),
        method
            .signature
            .parameters
            .iter()
            .map(|p| canonical(a, p))
            .collect::<Vec<_>>()
            .join(",")
    );
    let candidates = info.accessors.get(&method.name)?;
    let [(name, getter, _)] = candidates
        .iter()
        .filter(|(_, _, expected)| *expected == signature)
        .collect::<Vec<_>>()[..]
    else {
        return None;
    };
    Some(Accessor {
        name: name.clone(),
        getter: *getter,
        // Framework members bind by their declared dispatch; callvirt sites only
        // need the property syntax, which C# emits the same way.
        is_virtual: false,
    })
}
/// Kind of a framework type reference: c, s, i, e or d.
pub fn kind(a: &Assembly, token: u32) -> Option<char> {
    framework_type(a, token).map(|(info, _)| info.kind)
}
/// Members of a local or framework enum type, in declaration order.
pub fn members_of(a: &Assembly, token: u32) -> Option<Vec<(String, i128)>> {
    if token >> 24 == 2 {
        if !a.nodes.iter().any(|n| n.token == token && n.kind == "enum") {
            return None;
        }
        return Some(
            a.nodes
                .iter()
                .filter(|n| n.parent == token && n.token >> 24 == 4 && n.name != "value__")
                .filter_map(|n| {
                    let value = crate::export::constant(a, n.token).ok()??;
                    Some((
                        n.name.clone(),
                        value.trim_end_matches(['L', 'U']).parse().ok()?,
                    ))
                })
                .collect(),
        );
    }
    enum_members(a, token).map(<[_]>::to_vec)
}
/// Members of a framework enum named by a serialized type name (custom attribute
/// named arguments), accepted only with a Microsoft framework public key token.
pub fn members_by_name(serialized: &str) -> Option<&'static [(String, i128)]> {
    let mut parts = serialized.split(',').map(str::trim);
    let name = parts.next()?;
    let key = parts.find_map(|p| p.strip_prefix("PublicKeyToken="))?;
    if !FRAMEWORK_KEYS.contains(&key.to_ascii_lowercase().as_str()) {
        return None;
    }
    let info = data().get(&name.replace('+', "."))?;
    (info.kind == 'e').then_some(info.members.as_slice())
}
/// `Enum.Member`, `Enum.A | Enum.B` for flag combinations, or `(Enum)value`.
pub fn enum_text(name: &str, members: &[(String, i128)], value: i128) -> String {
    if let Some((member, _)) = members.iter().find(|(_, v)| *v == value) {
        return format!("{name}.{member}");
    }
    // Decompose into single-bit members, largest first, when they cover every bit.
    let mut rest = value;
    let mut parts = vec![];
    if value > 0 {
        for (member, bits) in members.iter().rev() {
            if *bits > 0 && bits & (bits - 1) == 0 && rest & bits == *bits {
                parts.push(format!("{name}.{member}"));
                rest &= !bits;
            }
        }
    }
    if rest == 0 && !parts.is_empty() {
        parts.reverse();
        return parts.join(" | ");
    }
    format!("({name})({value})")
}
/// Members of a framework enum, for writing `AttributeTargets.Class` instead of `4`.
pub fn enum_members(a: &Assembly, token: u32) -> Option<&'static [(String, i128)]> {
    let (info, _) = framework_type(a, token)?;
    (info.kind == 'e').then_some(info.members.as_slice())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_parse_into_accessors_and_enum_members() {
        let types = parse(
            "T s System.Nullable`1\nP HasValue g bool\nT e System.AttributeTargets\nE Class 4\nE All 32767\n",
        );
        let nullable = &types["System.Nullable`1"];
        assert_eq!(nullable.kind, 's');
        assert_eq!(
            nullable.accessors["get_HasValue"],
            [("HasValue".to_owned(), true, "bool()".to_owned())]
        );
        assert!(!nullable.accessors.contains_key("set_HasValue"));
        assert_eq!(
            types["System.AttributeTargets"].members,
            [("Class".to_owned(), 4), ("All".to_owned(), 32767)]
        );
        let members = [
            ("Class".to_owned(), 4),
            ("Method".to_owned(), 64),
            ("All".to_owned(), 32767),
        ];
        assert_eq!(
            enum_text("AttributeTargets", &members, 4),
            "AttributeTargets.Class"
        );
        assert_eq!(
            enum_text("AttributeTargets", &members, 68),
            "AttributeTargets.Class | AttributeTargets.Method"
        );
        assert_eq!(
            enum_text("AttributeTargets", &members, 5),
            "(AttributeTargets)(5)"
        );
    }
    #[test]
    fn serialized_names_need_a_framework_key() {
        let signed = "System.AttributeTargets, System.Runtime, Version=10.0.0.0, Culture=neutral, PublicKeyToken=b03f5f7f11d50a3a";
        assert!(
            members_by_name(signed).is_some_and(|m| m.iter().any(|(n, v)| n == "Class" && *v == 4))
        );
        let impostor = "System.AttributeTargets, Fake, Version=1.0.0.0, Culture=neutral, PublicKeyToken=0123456789abcdef";
        assert!(members_by_name(impostor).is_none());
        assert!(members_by_name("System.AttributeTargets").is_none());
    }
}
