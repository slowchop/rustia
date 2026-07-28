use rustia_conformance::{
    bundled_repository, list_repository_pending, run_repository, run_repository_case, CaseRun,
    Error, PendingCase, Report,
};
use std::env;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;

const RUNNER_STACK_SIZE: usize = 128 * 1024 * 1024;

enum RunResult {
    Case(CaseRun),
    Pending(Vec<PendingCase>),
    Report(Report),
}

enum Command {
    Run {
        repo: PathBuf,
        selector: Option<String>,
    },
    ListPending {
        repo: PathBuf,
    },
}

fn main() -> ExitCode {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    let Some(command) = parse_command(&args) else {
        eprintln!("usage: rustia-conformance [repo] [case-id-or-path]");
        eprintln!("       rustia-conformance --list-pending [repo]");
        eprintln!("       rustia-conformance [repo] --list-pending");
        return ExitCode::from(2);
    };

    match run_with_stack(command) {
        Ok(RunResult::Case(case)) => {
            println!("conformance case {} ({}): pass", case.id, case.path);
            ExitCode::SUCCESS
        }
        Ok(RunResult::Pending(pending)) => {
            println!("conformance pending: {}", pending.len());
            for case in pending {
                println!(
                    "{}\t{}\t{}\t{}",
                    case.oracle,
                    case.id,
                    case.path,
                    case.refs.join(",")
                );
            }
            ExitCode::SUCCESS
        }
        Ok(RunResult::Report(report)) => {
            println!(
                "conformance: {} fixtures, {} refs, {} luau-0.709, {} pending-oracle, {} pending-impl, {} rustia-only",
                report.fixtures,
                report.refs,
                report.luau_oracle,
                report.pending_oracle,
                report.pending_impl,
                report.rustia_only
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("conformance failed: {error}");
            ExitCode::from(1)
        }
    }
}

fn parse_command(args: &[std::ffi::OsString]) -> Option<Command> {
    match args {
        [] => Some(Command::Run {
            repo: bundled_repository().ok()?,
            selector: None,
        }),
        [flag] if flag == OsStr::new("--list-pending") => Some(Command::ListPending {
            repo: bundled_repository().ok()?,
        }),
        [repo] => Some(Command::Run {
            repo: PathBuf::from(repo),
            selector: None,
        }),
        [flag, repo] if flag == OsStr::new("--list-pending") => Some(Command::ListPending {
            repo: PathBuf::from(repo),
        }),
        [repo, flag] if flag == OsStr::new("--list-pending") => Some(Command::ListPending {
            repo: PathBuf::from(repo),
        }),
        [repo, selector] => Some(Command::Run {
            repo: PathBuf::from(repo),
            selector: Some(selector.to_string_lossy().into_owned()),
        }),
        _ => None,
    }
}

fn run_with_stack(command: Command) -> Result<RunResult, Error> {
    thread::Builder::new()
        .name("rustia-conformance-runner".to_owned())
        .stack_size(RUNNER_STACK_SIZE)
        .spawn(move || match command {
            Command::Run { repo, selector } => match selector {
                Some(selector) => run_repository_case(&repo, &selector).map(RunResult::Case),
                None => run_repository(&repo).map(RunResult::Report),
            },
            Command::ListPending { repo } => list_repository_pending(&repo).map(RunResult::Pending),
        })
        .expect("failed to spawn conformance runner")
        .join()
        .expect("conformance runner panicked")
}
