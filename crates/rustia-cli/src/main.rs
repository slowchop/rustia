use rustia::{Profile, Rustia};
use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: rustia <script.luau>");
        return ExitCode::from(2);
    };

    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("{path}: {error}");
            return ExitCode::from(1);
        }
    };

    let mut rustia = Rustia::new(Profile::ReproducibleStandard);
    match rustia.run_owned(&source) {
        Ok(value) => {
            println!("{value:?}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}
