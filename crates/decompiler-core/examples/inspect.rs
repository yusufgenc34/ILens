//! Native diagnostic harness. Never executes managed code.
use decompiler_core::{assembly::Assembly, decompile};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("Pass a PE path and optional method name")?;
    let a = Assembly::load(std::fs::read(path)?)?;
    let filter = args.next();
    if let Some(name) = filter {
        for n in a
            .nodes
            .iter()
            .filter(|n| n.token >> 24 == 6 && n.name == name)
        {
            let r = decompile::decompile(&a, n.token)?;
            println!("{}\n{}\n{}", n.full_name, r.csharp, r.il);
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&a.info)?);
    }
    Ok(())
}
