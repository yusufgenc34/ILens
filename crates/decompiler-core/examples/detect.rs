//! Prints the static packer/protector scan for a file. Never executes or unpacks code.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("Pass a PE path")?;
    let bytes = std::fs::read(path)?;
    let assembly = decompiler_core::assembly::Assembly::load(bytes.clone()).ok();
    let report = decompiler_core::detect::scan(&bytes, assembly.as_ref())?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
