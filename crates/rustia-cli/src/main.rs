use rustia::{OwnedValue, Profile, Rustia, SourceSpan, VmSettings};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
usage: rustia [options] <script.luau>
       rustia [options] --eval <source>
       rustia [options] -

options:
  -e, --eval <source>        run source passed on the command line
      --profile <profile>    luau-compat | reproducible-standard | deterministic-sim
      --fuel <instructions>  set an instruction fuel budget
      --source-name <name>   override the source name used in diagnostics
  -h, --help                 show this help
";

fn main() -> ExitCode {
    match run_cli(env::args().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Help) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Err(CliError::Usage(message)) => {
            eprintln!("{message}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(CliError::Io { label, error }) => {
            eprintln!("{label}: {error}");
            ExitCode::from(1)
        }
        Err(CliError::Rustia {
            error,
            source_name,
            source,
        }) => {
            eprintln!("{}", format_diagnostic(&error, &source_name, &source));
            ExitCode::from(1)
        }
    }
}

fn run_cli(args: impl IntoIterator<Item = String>) -> Result<(), CliError> {
    let config = parse_args(args)?;
    let (source_name, source) = read_source(&config)?;
    let value = run_source(&config, &source_name, &source).map_err(|error| CliError::Rustia {
        error,
        source_name: source_name.clone(),
        source: source.clone(),
    })?;
    println!("{value:?}");
    Ok(())
}

fn read_source(config: &Config) -> Result<(String, String), CliError> {
    match &config.input {
        Some(Input::File(path)) => {
            let source = fs::read_to_string(path).map_err(|error| CliError::Io {
                label: path.display().to_string(),
                error,
            })?;
            let source_name = config
                .source_name
                .clone()
                .unwrap_or_else(|| path.display().to_string());
            Ok((source_name, source))
        }
        Some(Input::Eval(source)) => Ok((
            config
                .source_name
                .clone()
                .unwrap_or_else(|| "<eval>".to_owned()),
            source.clone(),
        )),
        Some(Input::Stdin) => {
            let mut source = String::new();
            io::stdin()
                .read_to_string(&mut source)
                .map_err(|error| CliError::Io {
                    label: "<stdin>".to_owned(),
                    error,
                })?;
            Ok((
                config
                    .source_name
                    .clone()
                    .unwrap_or_else(|| "<stdin>".to_owned()),
                source,
            ))
        }
        None => Err(CliError::usage("missing script path, --eval source, or -")),
    }
}

fn run_source(config: &Config, source_name: &str, source: &str) -> rustia::Result<OwnedValue> {
    let mut rustia = Rustia::with_settings(config.profile, config.settings.clone());
    rustia.run_owned_named(source_name, source)
}

fn format_diagnostic(error: &rustia::Error, source_name: &str, source: &str) -> String {
    let mut output = error.to_string();
    if let Some(span) = error.source_span() {
        append_span_snippet(&mut output, source_name, source, span);
    } else if let Some(location) = error.source_location() {
        append_line_snippet(&mut output, location.source, source, location.line);
    }
    output
}

fn append_span_snippet(output: &mut String, source_name: &str, source: &str, span: SourceSpan) {
    let position = line_position(source, span.start);
    let line = &source[position.line_start..position.line_end];
    let width = position.line.to_string().len();
    let caret_count = span_caret_count(source, position.line_start, position.line_end, span);

    let _ = write!(
        output,
        "\n --> {source_name}:{}:{}\n{:>width$} |\n{:>width$} | {line}\n{:>width$} | {}{}",
        position.line,
        position.column,
        "",
        position.line,
        "",
        " ".repeat(position.column.saturating_sub(1)),
        "^".repeat(caret_count),
        width = width
    );
}

fn append_line_snippet(output: &mut String, source_name: &str, source: &str, line_number: u32) {
    let Some((line_index, line)) = source_line(source, line_number) else {
        return;
    };
    let width = line_index.to_string().len();

    let _ = write!(
        output,
        "\n --> {source_name}:{line_index}\n{:>width$} |\n{:>width$} | {line}\n{:>width$} | ^",
        "",
        line_index,
        "",
        width = width
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LinePosition {
    line: usize,
    column: usize,
    line_start: usize,
    line_end: usize,
}

fn line_position(source: &str, byte_index: usize) -> LinePosition {
    let target = byte_index.min(source.len());
    let mut line = 1;
    let mut line_start = 0;

    for (index, ch) in source.char_indices() {
        if index >= target {
            break;
        }
        if ch == '\n' {
            line += 1;
            line_start = index + ch.len_utf8();
        }
    }

    let mut line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |offset| line_start + offset);
    if line_end > line_start && source.as_bytes()[line_end - 1] == b'\r' {
        line_end -= 1;
    }
    let column = source[line_start..target].chars().count() + 1;

    LinePosition {
        line,
        column,
        line_start,
        line_end,
    }
}

fn span_caret_count(source: &str, line_start: usize, line_end: usize, span: SourceSpan) -> usize {
    let start = span.start.clamp(line_start, line_end);
    let end = span.end.clamp(start, line_end);
    source[start..end].chars().count().max(1)
}

fn source_line(source: &str, line_number: u32) -> Option<(usize, &str)> {
    let line_index = usize::try_from(line_number).ok()?;
    if line_index == 0 {
        return None;
    }
    let mut current = 1;
    let mut line_start = 0;

    for (index, ch) in source.char_indices() {
        if current == line_index && ch == '\n' {
            let line = trim_line_ending(&source[line_start..index]);
            return Some((line_index, line));
        }
        if ch == '\n' {
            current += 1;
            line_start = index + ch.len_utf8();
        }
    }

    if current == line_index {
        return Some((line_index, trim_line_ending(&source[line_start..])));
    }
    None
}

fn trim_line_ending(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Config, CliError> {
    let mut config = Config::default();
    let mut args = args.into_iter();
    let mut options = true;

    while let Some(arg) = args.next() {
        if options && arg == "--" {
            options = false;
            continue;
        }

        if options {
            if let Some(value) = arg.strip_prefix("--profile=") {
                config.profile = parse_profile(value)?;
                continue;
            }
            if let Some(value) = arg.strip_prefix("--fuel=") {
                config.settings.initial_fuel = Some(parse_fuel(value)?);
                continue;
            }
            if let Some(value) = arg.strip_prefix("--source-name=") {
                config.source_name = Some(parse_non_empty("--source-name", value)?);
                continue;
            }

            match arg.as_str() {
                "-h" | "--help" => return Err(CliError::Help),
                "-e" | "--eval" => {
                    let source = args
                        .next()
                        .ok_or_else(|| CliError::usage("--eval requires source"))?;
                    config.set_input(Input::Eval(source))?;
                }
                "--profile" => {
                    let value = args
                        .next()
                        .ok_or_else(|| CliError::usage("--profile requires a value"))?;
                    config.profile = parse_profile(&value)?;
                }
                "--fuel" => {
                    let value = args
                        .next()
                        .ok_or_else(|| CliError::usage("--fuel requires a value"))?;
                    config.settings.initial_fuel = Some(parse_fuel(&value)?);
                }
                "--source-name" => {
                    let value = args
                        .next()
                        .ok_or_else(|| CliError::usage("--source-name requires a value"))?;
                    config.source_name = Some(parse_non_empty("--source-name", &value)?);
                }
                _ if arg.starts_with('-') && arg != "-" => {
                    return Err(CliError::usage(format!("unknown option {arg:?}")));
                }
                "-" => config.set_input(Input::Stdin)?,
                _ => config.set_input(Input::File(PathBuf::from(arg)))?,
            }
        } else {
            config.set_input(Input::File(PathBuf::from(arg)))?;
        }
    }

    if config.input.is_none() {
        return Err(CliError::usage("missing script path, --eval source, or -"));
    }

    Ok(config)
}

fn parse_profile(value: &str) -> Result<Profile, CliError> {
    match value {
        "luau" | "luau-compat" | "LuauCompat" => Ok(Profile::LuauCompat),
        "standard" | "reproducible" | "reproducible-standard" | "ReproducibleStandard" => {
            Ok(Profile::ReproducibleStandard)
        }
        "deterministic" | "deterministic-sim" | "DeterministicSim" => {
            Ok(Profile::DeterministicSim)
        }
        _ => Err(CliError::usage(format!(
            "unknown profile {value:?}; expected luau-compat, reproducible-standard, or deterministic-sim"
        ))),
    }
}

fn parse_fuel(value: &str) -> Result<u64, CliError> {
    let fuel = value
        .parse::<u64>()
        .map_err(|_| CliError::usage(format!("invalid --fuel value {value:?}")))?;
    if fuel == 0 {
        return Err(CliError::usage("--fuel must be greater than zero"));
    }
    Ok(fuel)
}

fn parse_non_empty(label: &'static str, value: &str) -> Result<String, CliError> {
    if value.is_empty() {
        return Err(CliError::usage(format!("{label} must not be empty")));
    }
    Ok(value.to_owned())
}

#[derive(Clone, Debug)]
struct Config {
    profile: Profile,
    settings: VmSettings,
    input: Option<Input>,
    source_name: Option<String>,
}

impl Config {
    fn set_input(&mut self, input: Input) -> Result<(), CliError> {
        if self.input.is_some() {
            return Err(CliError::usage(
                "provide exactly one script path, --eval source, or -",
            ));
        }
        self.input = Some(input);
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            profile: Profile::ReproducibleStandard,
            settings: VmSettings::default(),
            input: None,
            source_name: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Input {
    File(PathBuf),
    Eval(String),
    Stdin,
}

#[derive(Debug)]
enum CliError {
    Help,
    Usage(String),
    Io {
        label: String,
        error: io::Error,
    },
    Rustia {
        error: rustia::Error,
        source_name: String,
        source: String,
    },
}

impl CliError {
    fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }
}

impl From<rustia::Error> for CliError {
    fn from(error: rustia::Error) -> Self {
        Self::Rustia {
            error,
            source_name: "<unknown>".to_owned(),
            source: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{format_diagnostic, parse_args, parse_profile, run_source, CliError, Input};
    use rustia::{FromRustia, Profile};
    use std::path::PathBuf;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parses_file_defaults() {
        let config = parse_args(args(&["script.luau"])).unwrap();
        assert_eq!(config.profile, Profile::ReproducibleStandard);
        assert_eq!(
            config.input,
            Some(Input::File(PathBuf::from("script.luau")))
        );
        assert_eq!(config.settings.initial_fuel, None);
        assert_eq!(config.source_name, None);
    }

    #[test]
    fn parses_eval_profile_fuel_and_source_name() {
        let config = parse_args(args(&[
            "--profile",
            "deterministic",
            "--fuel=1000",
            "--source-name",
            "inline.luau",
            "--eval",
            "return 42",
        ]))
        .unwrap();

        assert_eq!(config.profile, Profile::DeterministicSim);
        assert_eq!(config.settings.initial_fuel, Some(1000));
        assert_eq!(config.source_name.as_deref(), Some("inline.luau"));
        assert_eq!(config.input, Some(Input::Eval("return 42".to_owned())));
    }

    #[test]
    fn parses_stdin_marker_and_equals_forms() {
        let config = parse_args(args(&[
            "--profile=luau-compat",
            "--fuel",
            "10",
            "--source-name=stdin.luau",
            "-",
        ]))
        .unwrap();

        assert_eq!(config.profile, Profile::LuauCompat);
        assert_eq!(config.settings.initial_fuel, Some(10));
        assert_eq!(config.source_name.as_deref(), Some("stdin.luau"));
        assert_eq!(config.input, Some(Input::Stdin));
    }

    #[test]
    fn rejects_missing_and_duplicate_inputs() {
        let missing = parse_args(args(&[])).unwrap_err();
        assert!(matches!(missing, CliError::Usage(message) if message.contains("missing")));

        let duplicate = parse_args(args(&["one.luau", "two.luau"])).unwrap_err();
        assert!(matches!(duplicate, CliError::Usage(message) if message.contains("exactly one")));
    }

    #[test]
    fn rejects_bad_options() {
        let unknown = parse_args(args(&["--wat"])).unwrap_err();
        assert!(matches!(unknown, CliError::Usage(message) if message.contains("unknown option")));

        let bad_fuel = parse_args(args(&["--fuel", "0", "script.luau"])).unwrap_err();
        assert!(
            matches!(bad_fuel, CliError::Usage(message) if message.contains("greater than zero"))
        );
    }

    #[test]
    fn profile_aliases_match_public_modes() {
        assert_eq!(parse_profile("luau").unwrap(), Profile::LuauCompat);
        assert_eq!(
            parse_profile("standard").unwrap(),
            Profile::ReproducibleStandard
        );
        assert_eq!(
            parse_profile("deterministic-sim").unwrap(),
            Profile::DeterministicSim
        );
    }

    #[test]
    fn run_source_preserves_source_name_in_runtime_errors() {
        let config = parse_args(args(&["--eval", "return 1"])).unwrap();
        let error = run_source(&config, "cli-test.luau", "local t = {}; t[0 / 0] = 1")
            .expect_err("NaN table key should fail");

        assert!(error.to_string().contains("cli-test.luau:1:"), "{error}");
    }

    #[test]
    fn diagnostic_formats_compile_error_source_span() {
        let config = parse_args(args(&["--eval", "return 1"])).unwrap();
        let source = "\
local x = 1
break
";
        let error = run_source(&config, "broken.luau", source)
            .expect_err("break outside loop should fail compilation");
        let diagnostic = format_diagnostic(&error, "broken.luau", source);

        assert!(diagnostic.contains("break outside loop"), "{diagnostic}");
        assert!(diagnostic.contains(" --> broken.luau:2:1"), "{diagnostic}");
        assert!(diagnostic.contains("2 | break"), "{diagnostic}");
        assert!(diagnostic.contains("| ^^^^^"), "{diagnostic}");
    }

    #[test]
    fn diagnostic_formats_runtime_error_source_line() {
        let config = parse_args(args(&["--eval", "return 1"])).unwrap();
        let source = "\
local t = {}
t[0 / 0] = 1
";
        let error =
            run_source(&config, "runtime.luau", source).expect_err("NaN table key should fail");
        let diagnostic = format_diagnostic(&error, "runtime.luau", source);

        assert!(diagnostic.contains("runtime.luau:2:"), "{diagnostic}");
        assert!(diagnostic.contains(" --> runtime.luau:2"), "{diagnostic}");
        assert!(diagnostic.contains("2 | t[0 / 0] = 1"), "{diagnostic}");
        assert!(diagnostic.contains("| ^"), "{diagnostic}");
    }

    #[test]
    fn run_source_returns_owned_result() {
        let config = parse_args(args(&["--eval", "return 1"])).unwrap();
        let value = run_source(&config, "<test>", "return 40 + 2").unwrap();

        assert_eq!(f64::from_rustia(value).unwrap(), 42.0);
    }
}
