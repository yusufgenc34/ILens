//! Prints the compilable C# skeleton for editing a method: `before`, the method, `after`.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("Pass a PE path and a method name")?;
    let name = args.next().ok_or("Pass a method name")?;
    let a = decompiler_core::assembly::Assembly::load(std::fs::read(path)?)?;
    let token = a
        .nodes
        .iter()
        .find(|n| n.token >> 24 == 6 && n.name == name)
        .ok_or("Method not found")?
        .token;
    let source = decompiler_core::csharp_edit::source(&a, token)?;
    print!("{}{}{}", source.before, source.method, source.after);
    Ok(())
}
