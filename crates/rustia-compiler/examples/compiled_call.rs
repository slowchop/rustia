//! Development driver for the experimental generated-call contract.
use rustia_compiler::{
    compiled_call::{generate_from_bundle, MAX_SOURCE_BYTES},
    source_bundle::SourceBundle,
};
use std::{
    env,
    error::Error,
    fs::{self, File},
    io::Read,
    path::PathBuf,
};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let source = PathBuf::from(args.next().ok_or("expected a Luau source path")?);
    let output = PathBuf::from(args.next().ok_or("expected a Rust output path")?);
    if args.next().is_some() {
        return Err("expected only source and output paths".into());
    }
    let mut bytes = Vec::new();
    File::open(&source)?
        .take((MAX_SOURCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(format!(
            "{}: compiled module exceeds {MAX_SOURCE_BYTES} source bytes",
            source.display()
        )
        .into());
    }
    let entry = "entry.luau";
    let bundle = SourceBundle::new([(entry, bytes)])?;
    let (module, receipt) = generate_from_bundle(&bundle, entry)
        .map_err(|error| format!("{}: {error}", source.display()))?;
    fs::write(output, module.rust)?;
    println!(
        "{} {} {:016x}",
        receipt.compiler_abi,
        module.capture_name,
        module.initial_capture.to_bits()
    );
    Ok(())
}
