//! Development driver for the experimental capture-backed callback.
use rustia_compiler::compiled_host_call::{generate, MAX_SOURCE_BYTES};
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
    let generated = generate(std::str::from_utf8(&bytes)?)
        .map_err(|error| format!("{}: {error}", source.display()))?;
    fs::write(output, generated)?;
    Ok(())
}
