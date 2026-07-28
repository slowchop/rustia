use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct CapturedOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_rustia<I, S>(args: I, stdin: Option<&str>) -> CapturedOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustia"));
    command
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().expect("rustia CLI should start");
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .expect("piped stdin should be available")
            .write_all(input.as_bytes())
            .expect("test input should be written");
    }

    let output = child.wait_with_output().expect("rustia CLI should finish");
    CapturedOutput {
        code: output
            .status
            .code()
            .expect("rustia CLI should not be killed"),
        stdout: String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("stderr should be UTF-8"),
    }
}

#[track_caller]
fn assert_output(output: &CapturedOutput, code: i32, stdout: &str, stderr: &str) {
    assert_eq!(output.code, code, "stderr:\n{}", output.stderr);
    assert_eq!(output.stdout, stdout, "stderr:\n{}", output.stderr);
    assert_eq!(output.stderr, stderr, "stdout:\n{}", output.stdout);
}

struct TestScript {
    directory: PathBuf,
    path: PathBuf,
}

impl TestScript {
    fn new(source: &str) -> Self {
        Self::new_bytes(source.as_bytes())
    }

    fn new_bytes(source: &[u8]) -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "rustia-cli-black-box-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("test directory should be created");
        let path = directory.join("script.luau");
        fs::write(&path, source).expect("test script should be written");
        Self { directory, path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write_relative(&self, path: &str, source: &str) -> PathBuf {
        let path = self.directory.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("module directory should be created");
        }
        fs::write(&path, source).expect("module source should be written");
        path
    }
}

impl Drop for TestScript {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn runs_a_script_file() {
    let script = TestScript::new("return 6 * 7\n");
    let output = run_rustia([script.path().as_os_str()], None);

    assert_output(&output, 0, "", "");
}

#[test]
fn reads_a_script_from_stdin() {
    let output = run_rustia(["-"], Some("return 20 + 22\n"));

    assert_output(&output, 0, "", "");
}

#[test]
fn evaluates_command_line_source() {
    let output = run_rustia(["--eval", "return 84 / 2"], None);

    assert_output(&output, 0, "", "");
}

#[test]
fn guest_print_reaches_stdout_and_chunk_returns_are_ignored() {
    let output = run_rustia(
        [
            "--eval",
            "print('hello', 42); local cycle = {}; cycle.self = cycle; return function() end, cycle",
        ],
        None,
    );

    assert_output(&output, 0, "hello\t42\n", "");
}

#[test]
fn file_input_accepts_shebangs_and_non_utf8_luau_bytes() {
    let script = TestScript::new_bytes(b"#!/usr/bin/env rustia\nprint(string.byte('\xff'))\n");
    let output = run_rustia([script.path().as_os_str()], None);

    assert_output(&output, 0, "255\n", "");
}

#[test]
fn repl_preserves_state_reports_errors_and_continues() {
    let input = "\
answer = 41
=answer + 1
break
=answer + 2
:quit
";
    let output = run_rustia(
        ["--source-name", "black-box-session", "--repl"],
        Some(input),
    );

    assert_eq!(output.code, 0, "stderr:\n{}", output.stderr);
    assert_eq!(output.stdout, "42\n43\n", "stderr:\n{}", output.stderr);
    assert!(
        output.stderr.contains("break outside loop"),
        "{}",
        output.stderr
    );
    assert!(
        output.stderr.contains(" --> black-box-session[3]:1:1"),
        "{}",
        output.stderr
    );
    assert!(
        output.stderr.contains("1 | break\n  | ^^^^^"),
        "{}",
        output.stderr
    );
}

#[test]
fn exit_codes_distinguish_help_execution_errors_and_usage_errors() {
    let help = run_rustia(["--help"], None);
    assert_eq!(help.code, 0, "stderr:\n{}", help.stderr);
    assert!(help.stdout.starts_with("usage: rustia"), "{}", help.stdout);
    assert!(help.stderr.is_empty(), "{}", help.stderr);

    let execution_error = run_rustia(["--eval", "break"], None);
    assert_eq!(
        execution_error.code, 1,
        "stderr:\n{}",
        execution_error.stderr
    );
    assert!(
        execution_error.stdout.is_empty(),
        "{}",
        execution_error.stdout
    );

    let usage_error = run_rustia(std::iter::empty::<&str>(), None);
    assert_eq!(usage_error.code, 2, "stderr:\n{}", usage_error.stderr);
    assert!(usage_error.stdout.is_empty(), "{}", usage_error.stdout);
    assert!(
        usage_error.stderr.starts_with("missing script path"),
        "{}",
        usage_error.stderr
    );
    assert!(usage_error.stderr.contains("usage: rustia"));
}

#[test]
fn missing_script_is_an_io_failure() {
    let script = TestScript::new("return 42\n");
    let missing_path = script.directory.join("missing.luau");
    let output = run_rustia([missing_path.as_os_str()], None);

    assert_eq!(output.code, 1, "stderr:\n{}", output.stderr);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert!(
        output
            .stderr
            .starts_with(&format!("{}: ", missing_path.display())),
        "{}",
        output.stderr
    );
}

#[test]
fn fuel_exhaustion_is_an_execution_failure() {
    let output = run_rustia(["--fuel", "10", "--eval", "while true do end"], None);

    assert_eq!(output.code, 1, "stderr:\n{}", output.stderr);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert!(
        output
            .stderr
            .contains("<eval>:1: instruction fuel exhausted after 10 instructions"),
        "{}",
        output.stderr
    );
    assert!(
        output.stderr.contains("1 | while true do end\n  | ^"),
        "{}",
        output.stderr
    );
}

#[test]
fn renders_compile_diagnostics_with_source_context() {
    let source = "\
local answer = 42
break
";
    let output = run_rustia(
        ["--source-name", "compile-test.luau", "--eval", source],
        None,
    );

    assert_eq!(output.code, 1, "stderr:\n{}", output.stderr);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert!(
        output.stderr.contains("break outside loop"),
        "{}",
        output.stderr
    );
    assert!(
        output.stderr.contains(" --> compile-test.luau:2:1"),
        "{}",
        output.stderr
    );
    assert!(
        output.stderr.contains("2 | break\n  | ^^^^^"),
        "{}",
        output.stderr
    );
}

#[test]
fn renders_runtime_diagnostics_with_source_context() {
    let source = "\
local values = {}
values[0 / 0] = 1
";
    let output = run_rustia(
        ["--source-name", "runtime-test.luau", "--eval", source],
        None,
    );

    assert_eq!(output.code, 1, "stderr:\n{}", output.stderr);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert!(
        output.stderr.contains("runtime-test.luau:2:"),
        "{}",
        output.stderr
    );
    assert!(
        output.stderr.contains(" --> runtime-test.luau:2"),
        "{}",
        output.stderr
    );
    assert!(
        output.stderr.contains("2 | values[0 / 0] = 1\n  | ^"),
        "{}",
        output.stderr
    );
}

#[test]
fn file_modules_resolve_relative_paths_and_cache_canonical_identity() {
    let script = TestScript::new(
        "\
local first = require('./modules/answer')
local second = require('./modules/../modules/answer.lua')
print(first.value, second.value)
",
    );
    script.write_relative(
        "modules/answer.lua",
        "\
_G.module_runs = (_G.module_runs or 0) + 1
return { value = 40 + _G.module_runs + 1 }
",
    );

    let output = run_rustia([script.path().as_os_str()], None);

    assert_output(&output, 0, "42\t42\n", "");
}

#[test]
fn nested_modules_resolve_relative_to_the_referring_module() {
    let script = TestScript::new("print(require('./modules/outer').answer)\n");
    script.write_relative(
        "modules/outer.luau",
        "return { answer = require('../shared/value') }\n",
    );
    script.write_relative("shared/value.luau", "return 42\n");

    let output = run_rustia([script.path().as_os_str()], None);

    assert_output(&output, 0, "42\n", "");
}

#[test]
fn module_runtime_diagnostics_show_the_module_source() {
    let script = TestScript::new("require('./broken')\n");
    let module = script.write_relative(
        "broken.luau",
        "\
local values = {}
values[0 / 0] = 1
",
    );

    let output = run_rustia([script.path().as_os_str()], None);

    assert_eq!(output.code, 1, "stderr:\n{}", output.stderr);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert!(
        output.stderr.contains(&format!(
            " --> {}:2",
            module.canonicalize().unwrap().display()
        )),
        "{}",
        output.stderr
    );
    assert!(
        output.stderr.contains("2 | values[0 / 0] = 1\n  | ^"),
        "{}",
        output.stderr
    );
}

#[test]
fn module_cycles_report_the_canonical_cycle_path() {
    let script = TestScript::new("require('./a')\n");
    let a = script.write_relative("a.luau", "return require('./b')\n");
    let b = script.write_relative("b.luau", "return require('./a')\n");

    let output = run_rustia([script.path().as_os_str()], None);

    assert_eq!(output.code, 1, "stderr:\n{}", output.stderr);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert!(
        output
            .stderr
            .contains("requiring a module that is in the process of being required"),
        "{}",
        output.stderr
    );
    assert!(
        output
            .stderr
            .contains(&a.canonicalize().unwrap().display().to_string()),
        "{}",
        output.stderr
    );
    assert!(
        output
            .stderr
            .contains(&b.canonicalize().unwrap().display().to_string()),
        "{}",
        output.stderr
    );
}

#[test]
fn deterministic_cli_has_no_ambient_filesystem_require() {
    let script = TestScript::new("require('./answer')\n");
    script.write_relative("answer.luau", "return 42\n");

    let output = run_rustia(
        [
            OsStr::new("--profile"),
            OsStr::new("deterministic-sim"),
            script.path().as_os_str(),
        ],
        None,
    );

    assert_eq!(output.code, 1, "stderr:\n{}", output.stderr);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert!(
        output.stderr.contains("attempt to call a nil value"),
        "{}",
        output.stderr
    );
}
