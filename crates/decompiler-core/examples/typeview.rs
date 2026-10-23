//! Prints the dnSpy-style C# document for a type. Never executes managed code.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("Pass a PE path and a type name")?;
    let name = args.next().ok_or("Pass a type name")?;
    let a = decompiler_core::assembly::Assembly::load(std::fs::read(path)?)?;
    let node = a
        .nodes
        .iter()
        .find(|n| n.token >> 24 == 2 && (n.name == name || n.full_name == name))
        .ok_or("Type not found")?;
    let view = decompiler_core::typeview::type_source(&a, node.token, &|t| {
        decompiler_core::resolution::local_accessor(&a, t)
    })?;
    print!("{}", view.text);
    Ok(())
}
