//! Read-only local profiling; uploaded assemblies are never invoked.
use decompiler_core::{assembly::Assembly, decompile};
use std::time::Instant;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("Pass an assembly path")?;
    let bytes = std::fs::read(path)?;
    let start = Instant::now();
    let a = Assembly::load(bytes)?;
    let load = start.elapsed();
    let start = Instant::now();
    let results = a.search("Method79")?;
    let search = start.elapsed();
    let start = Instant::now();
    let token = a
        .nodes
        .iter()
        .find(|n| n.token >> 24 == 6 && n.name == "Method79")
        .ok_or("Method79 missing")?
        .token;
    let result = decompile::decompile(&a, token)?;
    println!(
        "{}",
        serde_json::json!({"bytes":a.bytes.len(),"types":a.info.type_count,"methods":a.info.method_count,"load_ms":load.as_secs_f64()*1000.0,"search_ms":search.as_secs_f64()*1000.0,"search_hits":results.len(),"decompile_ms":start.elapsed().as_secs_f64()*1000.0,"quality":result.quality})
    );
    Ok(())
}
