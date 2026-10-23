//! Generates `data/bcl.txt` from Microsoft reference assemblies (metadata only; nothing
//! is executed): `cargo run -p decompiler-core --example bcl_summary -- <ref dir> > data/bcl.txt`.
use decompiler_core::{assembly::Assembly, bcl::canonical, export::constant, signature};
use std::collections::BTreeMap;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args()
        .nth(1)
        .ok_or("Pass a reference assembly directory")?;
    let mut paths: Vec<_> = std::fs::read_dir(directory)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "dll"))
        .collect();
    paths.sort();
    // Sorted output keeps regenerated data stable for review.
    let mut types: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in paths {
        let Ok(a) = Assembly::load(std::fs::read(&path)?) else {
            continue;
        };
        for node in a.nodes.iter().filter(|n| n.token >> 24 == 2) {
            // Public types, including public types nested in public types.
            let mut visible = true;
            let mut current = Some(node);
            while let Some(t) = current {
                visible &= matches!(t.flags & 7, 1 | 2);
                current = a
                    .nodes
                    .iter()
                    .find(|n| n.token == t.parent && t.parent >> 24 == 2);
            }
            if !visible {
                continue;
            }
            let kind = match node.kind.as_str() {
                "struct" => 's',
                "interface" => 'i',
                "enum" => 'e',
                "delegate" => 'd',
                _ => 'c',
            };
            let mut lines = vec![];
            for property in a
                .nodes
                .iter()
                .filter(|n| n.parent == node.token && n.token >> 24 == 23)
            {
                let row = &a.metadata().properties[(property.token & 0xffffff) as usize - 1];
                let Ok(sig) = a.blob(row.property_type).and_then(signature::method) else {
                    continue;
                };
                if !sig.parameters.is_empty() {
                    continue;
                }
                let mut access = String::new();
                for (semantics, method) in a.semantics.get(&property.token).into_iter().flatten() {
                    let public = a
                        .method_row(*method)
                        .is_ok_and(|m| matches!(m.flags & 7, 4..=6));
                    match semantics {
                        2 if public => access.push('g'),
                        1 if public => access.push('s'),
                        _ => {}
                    }
                }
                if !access.is_empty() {
                    lines.push(format!(
                        "P {} {access} {}",
                        property.name,
                        canonical(&a, &sig.return_type)
                    ));
                }
            }
            if kind == 'e' {
                for field in a
                    .nodes
                    .iter()
                    .filter(|n| n.parent == node.token && n.token >> 24 == 4 && n.name != "value__")
                {
                    if let Ok(Some(value)) = constant(&a, field.token)
                        && let Ok(value) = value.trim_end_matches(['L', 'U']).parse::<i128>()
                    {
                        lines.push(format!("E {} {value}", field.name));
                    }
                }
            }
            let name = a.resolve_full(node.token);
            let entry = types.entry(format!("T {kind} {name}")).or_default();
            if entry.is_empty() {
                *entry = lines;
            }
        }
    }
    for (header, lines) in types {
        println!("{header}");
        for line in lines {
            println!("{line}");
        }
    }
    Ok(())
}
