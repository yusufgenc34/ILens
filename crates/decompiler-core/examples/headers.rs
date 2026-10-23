//! Prints the explorer's PE/CLI header view as JSON. Never executes managed code.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("Pass a PE path")?;
    let a = decompiler_core::assembly::Assembly::load(std::fs::read(path)?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&decompiler_core::headers::headers(&a)?)?
    );
    Ok(())
}
