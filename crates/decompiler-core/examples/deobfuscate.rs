//! Local static-analysis diagnostic; never loads an assembly into a CLR.
use decompiler_core::{
    assembly::Assembly,
    deobfuscate::{Job, Options},
};
use std::io::Write;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: cargo run -p decompiler-core --example deobfuscate -- INPUT OUTPUT (output must not exist)".into());
    }
    let a = Assembly::load(std::fs::read(&args[0])?)?;
    let mut job = Job::new(
        1,
        1,
        0,
        &a,
        Options {
            constants: false,
            branches: false,
            strings: false,
            resource_strings: true,
            rename: true,
            proxies: true,
        },
    )?;
    while !job.progress().done {
        job.step(&a)?;
    }
    let report = job.finish(&a)?;
    println!(
        "{}: {} methods, {} string calls replaced, {} unresolved calls, {} names",
        report.outcome,
        report.changed_methods,
        report.counts.strings,
        report.unresolved_calls,
        report.counts.renamed
    );
    for notice in &report.notices {
        println!("  {notice}");
    }
    if report.unresolved_calls > 0 {
        for m in report
            .methods
            .iter()
            .filter(|m| !m.reasons.is_empty())
            .take(8)
        {
            println!("0x{:08X}: {}", m.token, m.reasons.join("; "));
        }
    }
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[1])?;
    output.write_all(job.output.as_ref().ok_or("Missing validated output")?)?;
    Ok(())
}
