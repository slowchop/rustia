//! Development driver for the experimental generated-call contract.
use rustia_compiler::compiled_call::generate;
use std::{env, error::Error, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let source = PathBuf::from(args.next().ok_or("expected a Luau source path")?);
    let output = PathBuf::from(args.next().ok_or("expected a Rust output path")?);
    if args.next().is_some() {
        return Err("expected only source and output paths".into());
    }
    let module = generate(&fs::read_to_string(&source)?)
        .map_err(|error| format!("{}: {error}", source.display()))?;
    fs::write(output, module.rust)?;
    println!(
        "rustia-compiled-call-v1 {} {:016x}",
        module.capture_name,
        module.initial_capture.to_bits()
    );
    Ok(())
}
