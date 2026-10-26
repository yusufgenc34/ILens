//! Static renaming of obfuscated declaration names. Only #Strings name cells change;
//! tokens, signatures, bodies and row order stay fixed. Relations the runtime binds
//! by name are kept consistent: virtual methods, properties and events map each
//! distinct obfuscated string to one new name, so overrides and implicit interface
//! implementations still match. MemberRefs into this module are renamed alongside.
use crate::{
    assembly::Assembly,
    error::{Error, Result},
    metadata_write::Tables,
    signature::Type,
};
use std::collections::{BTreeMap, HashMap, HashSet};

/// One #Strings cell to rewrite: table, zero-based row, column, new name ("" clears).
pub(crate) type Cell = (usize, usize, usize, String);
#[derive(Debug, Default)]
pub(crate) struct Plan {
    pub cells: Vec<Cell>,
    pub types: usize,
    pub members: usize,
    pub unresolved_references: usize,
}

/// Control/invisible characters or characters no C# or compiler-generated name uses.
pub(crate) fn obfuscated(name: &str) -> bool {
    name.chars().any(|c| {
        crate::inspection::hidden_character(c)
            || !(c.is_alphanumeric() || "_<>.`$@-=|,[]{}+#?' ".contains(c))
    })
}
fn token(value: u32, bits: u32, tables: &[u8]) -> u32 {
    match tables.get((value & ((1 << bits) - 1)) as usize) {
        Some(&t) if value >> bits != 0 => (u32::from(t) << 24) | (value >> bits),
        _ => 0,
    }
}
fn compressed(b: &[u8]) -> Option<(u32, usize)> {
    match *b.first()? {
        x if x & 0x80 == 0 => Some((u32::from(x), 1)),
        x if x & 0xc0 == 0x80 => Some(((u32::from(x & 0x3f) << 8) | u32::from(*b.get(1)?), 2)),
        x if x & 0xe0 == 0xc0 => Some((
            (u32::from(x & 0x1f) << 24)
                | (u32::from(*b.get(1)?) << 16)
                | (u32::from(*b.get(2)?) << 8)
                | u32::from(*b.get(3)?),
            4,
        )),
        _ => None,
    }
}
fn identifier(text: &str, fallback: &str) -> String {
    let mut s: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        s = fallback.into();
    }
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_ascii_lowercase().to_string() + c.as_str())
        .unwrap_or_default()
}
/// Picks `make(n)` for the first n not already taken; records it as taken.
fn unique(
    taken: &mut HashSet<String>,
    counter: &mut usize,
    make: impl Fn(usize) -> String,
) -> String {
    loop {
        let name = make(*counter);
        *counter += 1;
        if taken.insert(name.clone()) {
            return name;
        }
    }
}
struct Bijection {
    names: HashMap<String, String>,
    taken: HashSet<String>,
    next: usize,
    prefix: &'static str,
}
impl Bijection {
    fn new(prefix: &'static str, taken: HashSet<String>) -> Self {
        Self {
            names: HashMap::new(),
            taken,
            next: 0,
            prefix,
        }
    }
    fn get(&mut self, old: &str) -> String {
        if let Some(n) = self.names.get(old) {
            return n.clone();
        }
        let prefix = self.prefix;
        let n = unique(&mut self.taken, &mut self.next, |i| format!("{prefix}{i}"));
        self.names.insert(old.into(), n.clone());
        n
    }
}

pub(crate) fn plan(a: &Assembly) -> Result<Plan> {
    if a.store.streams.iter().any(|s| s.name == "#-") {
        return Err(crate::edit::unsupported(
            "Renaming requires compressed (#~) metadata",
        ));
    }
    let t = Tables::decode(
        crate::pe_write::stream(a, "#~").ok_or_else(|| Error::metadata("Missing #~ stream"))?,
    )?;
    if [3, 5, 7, 19, 22].iter().any(|i| t.counts[*i] != 0) {
        return Err(crate::edit::unsupported(
            "Renaming does not support metadata pointer tables",
        ));
    }
    let s = |i: u32| a.string(i).unwrap_or("").to_string();
    let mut plan = Plan::default();
    let cell = |plan: &mut Plan, table: usize, row: usize, column: usize, name: String| {
        plan.cells.push((table, row, column, name));
    };

    // Types: kind-based prefixes with global counters keep (namespace, name, enclosing) unique.
    let types = &t.rows[2];
    let mut type_names: Vec<String> = types.iter().map(|r| s(r[1])).collect();
    let mut namespaces: Vec<String> = types.iter().map(|r| s(r[2])).collect();
    let mut taken: HashSet<String> = type_names
        .iter()
        .filter(|n| !obfuscated(n))
        .cloned()
        .collect();
    let mut counters: BTreeMap<&str, usize> = BTreeMap::new();
    let mut ns = Bijection::new(
        "Namespace",
        namespaces
            .iter()
            .filter(|n| !obfuscated(n))
            .cloned()
            .collect(),
    );
    let old_types = type_names.clone();
    let old_namespaces = namespaces.clone();
    for (i, r) in types.iter().enumerate().skip(1) {
        if obfuscated(&namespaces[i]) {
            namespaces[i] = ns.get(&old_namespaces[i]);
            cell(&mut plan, 2, i, 2, namespaces[i].clone());
        }
        if !obfuscated(&type_names[i]) {
            continue;
        }
        let prefix = kind(a, &t, i, r[0]);
        let arity = type_names[i]
            .rsplit_once('`')
            .filter(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            .map(|(_, n)| format!("`{n}"))
            .unwrap_or_default();
        let counter = counters.entry(prefix).or_default();
        let name = unique(&mut taken, counter, |n| format!("{prefix}{n}{arity}"));
        type_names[i] = name.clone();
        cell(&mut plan, 2, i, 1, name);
        plan.types += 1;
    }
    let short = |token: u32| -> String {
        if token >> 24 == 2 {
            let n = type_names
                .get((token & 0xffffff) as usize - 1)
                .cloned()
                .unwrap_or_default();
            n.split('`').next().unwrap_or("").to_string()
        } else {
            let full = a.resolve_full(token);
            let full = full.split('<').next().unwrap_or("").to_string();
            full.rsplit('.')
                .next()
                .unwrap_or("")
                .split('`')
                .next()
                .unwrap_or("")
                .to_string()
        }
    };
    fn word(ty: &Type, short: &dyn Fn(u32) -> String) -> String {
        match ty {
            Type::Primitive(n) => n.clone(),
            Type::Named(t) | Type::ValueType(t) => short(*t),
            Type::Array(inner, _) | Type::MultiArray(inner, _) => {
                format!("{}Array", word(inner, short))
            }
            Type::GenericInstance { base, .. } => word(base, short),
            Type::Pointer(inner) | Type::ByRef(inner) | Type::Pinned(inner) => word(inner, short),
            Type::Modified { inner, .. } => word(inner, short),
            Type::Generic { .. } => "gparam".into(),
            _ => String::new(),
        }
    }

    // Existing (kept) member names per owner, so synthetic names never collide.
    let mut owned: HashMap<u32, HashSet<String>> = HashMap::new();
    let fields = &t.rows[4];
    let methods = &t.rows[6];
    for (i, r) in fields.iter().enumerate() {
        let n = s(r[1]);
        if !obfuscated(&n) {
            let owner = a.owners.get(&(0x04000001 + i as u32)).copied().unwrap_or(0);
            owned.entry(owner).or_default().insert(n);
        }
    }
    for (i, r) in methods.iter().enumerate() {
        let n = s(r[3]);
        if !obfuscated(&n) {
            let owner = a.owners.get(&(0x06000001 + i as u32)).copied().unwrap_or(0);
            owned.entry(owner).or_default().insert(n);
        }
    }
    // (owner, old name, signature) → new; by name only when every candidate agrees.
    let mut by_sig: HashMap<(u32, String, Vec<u8>), String> = HashMap::new();
    let mut by_name: HashMap<(u32, String, bool), Option<String>> = HashMap::new();
    let mut record = |owner: u32, old: &str, sig: &[u8], field: bool, new: &str| {
        by_sig.insert((owner, old.into(), sig.to_vec()), new.into());
        by_name
            .entry((owner, old.into(), field))
            .and_modify(|v| {
                if v.as_deref() != Some(new) {
                    *v = None
                }
            })
            .or_insert_with(|| Some(new.into()));
    };

    let mut counter: HashMap<(u32, String), usize> = HashMap::new();
    for (i, r) in fields.iter().enumerate() {
        let old = s(r[1]);
        if !obfuscated(&old) {
            continue;
        }
        let token = 0x04000001 + i as u32;
        let owner = a.owners.get(&token).copied().unwrap_or(0);
        let w = a
            .field_ref(token)
            .map(|(_, _, ty)| identifier(&word(&ty, &short), "field"))
            .unwrap_or_else(|_| "field".into());
        let taken = owned.entry(owner).or_default();
        let n = counter.entry((owner, w.clone())).or_default();
        let name = unique(taken, n, |k| format!("{w}_{k}"));
        record(owner, &old, a.blob(r[2]).unwrap_or_default(), true, &name);
        cell(&mut plan, 4, i, 1, name);
        plan.members += 1;
    }

    // Properties and events: one name per distinct obfuscated string.
    let mut association: HashMap<u32, String> = HashMap::new();
    for (table, prefix, tag) in [(23usize, "Property_", 23u32), (20, "Event_", 20)] {
        let taken = t.rows[table]
            .iter()
            .map(|r| s(r[1]))
            .filter(|n| !obfuscated(n))
            .collect();
        let mut names = Bijection::new(prefix, taken);
        for (i, r) in t.rows[table].iter().enumerate() {
            let old = s(r[1]);
            let new = if obfuscated(&old) {
                let n = names.get(&old);
                cell(&mut plan, table, i, 1, n.clone());
                plan.members += 1;
                n
            } else {
                old
            };
            association.insert((tag << 24) | (i as u32 + 1), new);
        }
    }
    let mut accessor: HashMap<u32, String> = HashMap::new();
    for r in &t.rows[24] {
        let target = token(r[2], 1, &[20, 23]);
        let prefix = match r[0] {
            0x1 => "set_",
            0x2 => "get_",
            0x8 => "add_",
            0x10 => "remove_",
            0x20 => "raise_",
            _ => continue,
        };
        if let Some(n) = association.get(&target) {
            accessor.insert(r[1], format!("{prefix}{n}"));
        }
    }
    let mut virtuals = Bijection::new(
        "vmethod_",
        methods
            .iter()
            .map(|r| s(r[3]))
            .filter(|n| !obfuscated(n))
            .collect(),
    );
    let mut method_names: Vec<Option<String>> = vec![None; methods.len()];
    for (i, r) in methods.iter().enumerate() {
        let old = s(r[3]);
        let flags = r[2];
        if !obfuscated(&old) || old.starts_with('.') || flags & 0x2000 != 0 {
            continue;
        }
        let token = 0x06000001 + i as u32;
        let owner = a.owners.get(&token).copied().unwrap_or(0);
        let name = if let Some(n) = accessor.get(&(i as u32 + 1)) {
            n.clone()
        } else if flags & 0x40 != 0 {
            virtuals.get(&old)
        } else {
            let w = if flags & 0x10 != 0 {
                "smethod"
            } else {
                "method"
            };
            let taken = owned.entry(owner).or_default();
            let n = counter.entry((owner, w.into())).or_default();
            unique(taken, n, |k| format!("{w}_{k}"))
        };
        record(owner, &old, a.blob(r[4]).unwrap_or_default(), false, &name);
        method_names[i] = Some(name.clone());
        cell(&mut plan, 6, i, 3, name);
        plan.members += 1;
    }

    // Parameters fall back to the decompiler's argN; generic parameters become T/T1…
    for (i, r) in t.rows[8].iter().enumerate() {
        if obfuscated(&s(r[2])) {
            cell(&mut plan, 8, i, 2, String::new());
        }
    }
    let mut arity: HashMap<u32, usize> = HashMap::new();
    for r in &t.rows[42] {
        *arity.entry(r[2]).or_default() += 1;
    }
    for (i, r) in t.rows[42].iter().enumerate() {
        if obfuscated(&s(r[3])) {
            let n = if arity[&r[2]] == 1 {
                "T".into()
            } else {
                format!("T{}", r[0] + 1)
            };
            cell(&mut plan, 42, i, 3, n);
        }
    }

    // References into this module: TypeRefs scoped to it and MemberRefs on own types.
    let nested: HashSet<u32> = t.rows[41].iter().map(|r| r[0]).collect();
    let top: HashMap<(String, String), usize> = (1..types.len())
        .filter(|i| !nested.contains(&(*i as u32 + 1)))
        .map(|i| ((old_namespaces[i].clone(), old_types[i].clone()), i))
        .collect();
    for (i, r) in t.rows[1].iter().enumerate() {
        if r[0] & 3 != 0 || r[0] >> 2 == 0 {
            continue; // only Module-scoped references name this module's types
        }
        if let Some(&k) = top.get(&(s(r[2]), s(r[1]))) {
            if type_names[k] != old_types[k] {
                cell(&mut plan, 1, i, 1, type_names[k].clone());
            }
            if namespaces[k] != old_namespaces[k] {
                cell(&mut plan, 1, i, 2, namespaces[k].clone());
            }
        }
    }
    for (i, r) in t.rows[10].iter().enumerate() {
        let old = s(r[1]);
        if !obfuscated(&old) {
            continue;
        }
        let parent = token(r[0], 3, &[2, 1, 26, 6, 27]);
        let sig = a.blob(r[2]).unwrap_or_default();
        let field = sig.first() == Some(&0x06);
        let owner = match parent >> 24 {
            2 => parent,
            6 => {
                if let Some(Some(n)) = method_names.get((parent & 0xffffff) as usize - 1) {
                    cell(&mut plan, 10, i, 1, n.clone());
                }
                continue;
            }
            27 => {
                let spec = t.rows[27]
                    .get((parent & 0xffffff) as usize - 1)
                    .and_then(|r| a.blob(r[0]).ok())
                    .unwrap_or_default();
                match (
                    spec.first(),
                    spec.get(1),
                    spec.get(2..).and_then(compressed),
                ) {
                    (Some(0x15), Some(0x11 | 0x12), Some((v, _))) if v & 3 == 0 => {
                        0x02000000 | (v >> 2)
                    }
                    _ => continue,
                }
            }
            _ => continue,
        };
        let new = by_sig
            .get(&(owner, old.clone(), sig.to_vec()))
            .cloned()
            .or_else(|| by_name.get(&(owner, old.clone(), field)).cloned().flatten());
        match new {
            Some(n) => cell(&mut plan, 10, i, 1, n),
            None => plan.unresolved_references += 1,
        }
    }

    // Embedded resources named after a renamed top-level type (WinForms .resources).
    for (i, r) in t.rows[40].iter().enumerate() {
        if r[3] != 0 {
            continue;
        }
        let name = s(r[2]);
        let Some(stem) = name.strip_suffix(".resources") else {
            continue;
        };
        let found = top.iter().find(|((n, ty), _)| {
            stem == if n.is_empty() {
                ty.clone()
            } else {
                format!("{n}.{ty}")
            }
        });
        if let Some((_, &k)) = found
            && (type_names[k] != old_types[k] || namespaces[k] != old_namespaces[k])
        {
            let full = if namespaces[k].is_empty() {
                type_names[k].clone()
            } else {
                format!("{}.{}", namespaces[k], type_names[k])
            };
            cell(&mut plan, 40, i, 2, format!("{full}.resources"));
        }
    }
    Ok(plan)
}

/// Prefix from the first external base type; own base classes are followed.
fn kind(a: &Assembly, t: &Tables, mut row: usize, flags: u32) -> &'static str {
    if flags & 0x20 != 0 {
        return "Interface";
    }
    for _ in 0..32 {
        let base = token(t.rows[2][row][3], 2, &[2, 1, 27]);
        if base == 0 {
            break;
        }
        if base >> 24 == 2 {
            row = (base & 0xffffff) as usize - 1;
            if row >= t.rows[2].len() {
                break;
            }
            continue;
        }
        let full = a.resolve_full(base);
        let full = full.split('<').next().unwrap_or("");
        return match full {
            "System.Enum" => "Enum",
            "System.ValueType" => "Struct",
            "System.MulticastDelegate" | "System.Delegate" => "Delegate",
            "System.Windows.Forms.Form" => "Form",
            "System.Windows.Forms.UserControl" => "UserControl",
            "System.Windows.Forms.Control" => "Control",
            n if n.ends_with("Exception") => "Exception",
            n if n.ends_with("Attribute") => "Attribute",
            n if n.ends_with("EventArgs") => "EventArgs",
            _ => "Class",
        };
    }
    "Class"
}
