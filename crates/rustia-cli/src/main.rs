use rustia::{
    ModuleSource, OwnedValue, Profile, Rustia, ServiceModuleResolver, SourceSpan, VmSettings,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;

type SourceMap = Rc<RefCell<BTreeMap<String, Vec<u8>>>>;

const USAGE: &str = "\
usage: rustia [options] <script.luau>
       rustia [options] --eval <source>
       rustia [options] -
       rustia [options] --repl

options:
  -e, --eval <source>        run source passed on the command line
      --repl                 run a line-oriented interactive session
      --profile <profile>    luau-compat | reproducible-standard | deterministic-sim | puc51-compat
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
            sources,
        }) => {
            eprintln!(
                "{}",
                format_diagnostic_with_sources(&error, &source_name, &source, &sources)
            );
            ExitCode::from(1)
        }
    }
}

fn run_cli(args: impl IntoIterator<Item = String>) -> Result<(), CliError> {
    let config = parse_args(args)?;
    if config.input == Some(Input::Repl) {
        let stdin = io::stdin();
        let show_prompts = stdin.is_terminal();
        let mut stdout = io::stdout();
        let mut stderr = io::stderr();
        return run_repl(
            &config,
            stdin.lock(),
            &mut stdout,
            &mut stderr,
            show_prompts,
        );
    }

    let (source_name, source) = read_source(&config)?;
    run_source(&config, &source_name, &source)
}

fn read_source(config: &Config) -> Result<(String, Vec<u8>), CliError> {
    match &config.input {
        Some(Input::File(path)) => {
            let source = fs::read(path).map_err(|error| CliError::Io {
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
            source.as_bytes().to_vec(),
        )),
        Some(Input::Stdin) => {
            let mut source = Vec::new();
            io::stdin()
                .read_to_end(&mut source)
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
        Some(Input::Repl) => Err(CliError::usage(
            "--repl does not read a single source chunk",
        )),
        None => Err(CliError::usage(
            "missing script path, --eval source, -, or --repl",
        )),
    }
}

fn run_source(config: &Config, source_name: &str, source: &[u8]) -> Result<(), CliError> {
    let (mut rustia, sources) = configured_rustia(config)?;
    sources
        .borrow_mut()
        .insert(source_name.to_owned(), source.to_vec());
    let executable_source = source_without_shebang(source);
    rustia
        .load(executable_source.as_ref())
        .source_name(source_name)
        .exec()
        .map_err(|error| CliError::Rustia {
            error: Box::new(error),
            source_name: source_name.to_owned(),
            source: source.to_vec(),
            sources: sources.borrow().clone(),
        })
}

fn configured_rustia(config: &Config) -> Result<(Rustia, SourceMap), CliError> {
    let sources = Rc::new(RefCell::new(BTreeMap::new()));
    let mut builder = Rustia::builder()
        .profile(config.profile)
        .settings(config.settings.clone())
        .stdout(|text| print!("{text}"));

    if config.profile != Profile::DeterministicSim {
        let base_directory = module_base_directory(config)?;
        builder = builder.module_resolver(cli_module_resolver(base_directory, sources.clone()));
    }

    let rustia = builder.build()?;
    Ok((rustia, sources))
}

fn module_base_directory(config: &Config) -> Result<PathBuf, CliError> {
    if let Some(Input::File(path)) = &config.input {
        let canonical = fs::canonicalize(path).map_err(|error| CliError::Io {
            label: path.display().to_string(),
            error,
        })?;
        return Ok(canonical
            .parent()
            .map_or_else(|| canonical.clone(), Path::to_path_buf));
    }

    env::current_dir().map_err(|error| CliError::Io {
        label: "current directory".to_owned(),
        error,
    })
}

fn cli_module_resolver(base_directory: PathBuf, sources: SourceMap) -> ServiceModuleResolver {
    let known_modules = Rc::new(RefCell::new(BTreeSet::new()));
    ServiceModuleResolver::from_nondeterministic_fn(move |requested, referrer| {
        let resolution_base = referrer
            .filter(|source_name| known_modules.borrow().contains(*source_name))
            .and_then(|source_name| Path::new(source_name).parent())
            .map_or_else(|| base_directory.clone(), Path::to_path_buf);
        let candidates = cli_module_candidates(&resolution_base, requested)?;
        let mut attempted = Vec::with_capacity(candidates.len());

        for candidate in candidates {
            attempted.push(candidate.display().to_string());
            let canonical = match fs::canonicalize(&candidate) {
                Ok(path) => path,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(format!("could not resolve module {requested:?}: {error}"));
                }
            };
            if !canonical.is_file() {
                continue;
            }
            let source = fs::read(&canonical)
                .map_err(|error| format!("{}: {error}", canonical.display()))?;
            let identity = canonical
                .into_os_string()
                .into_string()
                .map_err(|path| format!("module path is not valid UTF-8: {}", path.display()))?;

            sources
                .borrow_mut()
                .insert(identity.clone(), source.clone());
            known_modules.borrow_mut().insert(identity.clone());
            return Ok(ModuleSource::new(identity, source));
        }

        Err(format!(
            "module {requested:?} was not found (tried {})",
            attempted.join(", ")
        ))
    })
}

fn cli_module_candidates(base_directory: &Path, requested: &str) -> Result<Vec<PathBuf>, String> {
    if requested.is_empty() {
        return Err("module path must not be empty".to_owned());
    }

    let requested = Path::new(requested);
    let path = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        base_directory.join(requested)
    };
    if requested.extension().is_some() {
        return Ok(vec![path]);
    }

    Ok(vec![
        path.with_extension("luau"),
        path.with_extension("lua"),
        path.join("init.luau"),
        path.join("init.lua"),
    ])
}

fn source_without_shebang(source: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if !source.starts_with(b"#!") {
        return std::borrow::Cow::Borrowed(source);
    }
    let mut source = source.to_vec();
    let line_end = source
        .iter()
        .position(|byte| matches!(byte, b'\n' | b'\r'))
        .unwrap_or(source.len());
    source[..line_end].fill(b' ');
    std::borrow::Cow::Owned(source)
}

fn run_repl<R, O, E>(
    config: &Config,
    mut input: R,
    output: &mut O,
    error_output: &mut E,
    show_prompts: bool,
) -> Result<(), CliError>
where
    R: BufRead,
    O: Write,
    E: Write,
{
    let (mut rustia, sources) = configured_rustia(config)?;
    let source_prefix = config.source_name.as_deref().unwrap_or("<repl>");
    let mut line = String::new();
    let mut line_number = 1usize;

    loop {
        if show_prompts {
            output
                .write_all(b"rustia> ")
                .map_err(|error| CliError::Io {
                    label: "<stdout>".to_owned(),
                    error,
                })?;
            output.flush().map_err(|error| CliError::Io {
                label: "<stdout>".to_owned(),
                error,
            })?;
        }

        line.clear();
        let read = input.read_line(&mut line).map_err(|error| CliError::Io {
            label: "<stdin>".to_owned(),
            error,
        })?;
        if read == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            line_number += 1;
            continue;
        }
        if matches!(trimmed, ":quit" | ":exit") {
            break;
        }
        if trimmed == ":help" {
            writeln!(
                output,
                "enter Luau statements, use =expr to print an expression, :quit to exit"
            )
            .map_err(|error| CliError::Io {
                label: "<stdout>".to_owned(),
                error,
            })?;
            line_number += 1;
            continue;
        }

        let (source, display_result) = repl_source(trimmed);
        let source_name = format!("{source_prefix}[{line_number}]");
        sources
            .borrow_mut()
            .insert(source_name.clone(), source.as_bytes().to_vec());
        let result = if display_result {
            rustia
                .load(&source)
                .source_name(&source_name)
                .eval()
                .map(Some)
        } else {
            rustia
                .load(&source)
                .source_name(&source_name)
                .exec()
                .map(|()| None)
        };
        match result {
            Ok(Some(value)) => {
                writeln!(output, "{}", display_repl_value(value)).map_err(|error| {
                    CliError::Io {
                        label: "<stdout>".to_owned(),
                        error,
                    }
                })?;
            }
            Ok(None) => {}
            Err(error) => {
                let diagnostic = {
                    let sources = sources.borrow();
                    format_diagnostic_with_sources(
                        &error,
                        &source_name,
                        source.as_bytes(),
                        &sources,
                    )
                };
                writeln!(error_output, "{diagnostic}").map_err(|error| CliError::Io {
                    label: "<stderr>".to_owned(),
                    error,
                })?;
            }
        }

        line_number += 1;
    }

    Ok(())
}

fn repl_source(input: &str) -> (String, bool) {
    if let Some(expr) = input.strip_prefix('=') {
        (format!("return tostring(({}))", expr.trim_start()), true)
    } else {
        (input.to_owned(), false)
    }
}

fn display_repl_value(value: OwnedValue) -> String {
    match value {
        OwnedValue::String(value) => value,
        OwnedValue::Nil => "nil".to_owned(),
        OwnedValue::Boolean(value) => value.to_string(),
        OwnedValue::Number(value) => value.to_string(),
        OwnedValue::Table(_) => "<table>".to_owned(),
    }
}

#[cfg(test)]
fn format_diagnostic(error: &rustia::Error, source_name: &str, source: &[u8]) -> String {
    format_diagnostic_with_sources(error, source_name, source, &BTreeMap::new())
}

fn format_diagnostic_with_sources(
    error: &rustia::Error,
    source_name: &str,
    source: &[u8],
    sources: &BTreeMap<String, Vec<u8>>,
) -> String {
    let mut output = error.to_string();
    if let Some(span) = error.source_span() {
        append_span_snippet(&mut output, source_name, source, span);
    } else if let Some(location) = error.source_location() {
        let located_source = sources.get(location.source).map_or(source, Vec::as_slice);
        append_line_snippet(&mut output, location.source, located_source, location.line);
    }
    output
}

fn append_span_snippet(output: &mut String, source_name: &str, source: &[u8], span: SourceSpan) {
    let position = line_position(source, span.start);
    let line = String::from_utf8_lossy(&source[position.line_start..position.line_end]);
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

fn append_line_snippet(output: &mut String, source_name: &str, source: &[u8], line_number: u32) {
    let Some((line_index, line)) = source_line(source, line_number) else {
        return;
    };
    let line = String::from_utf8_lossy(line);
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

fn line_position(source: &[u8], byte_index: usize) -> LinePosition {
    let target = byte_index.min(source.len());
    let mut line = 1;
    let mut line_start = 0;

    for (index, byte) in source.iter().copied().enumerate() {
        if index >= target {
            break;
        }
        if byte == b'\n' {
            line += 1;
            line_start = index + 1;
        }
    }

    let mut line_end = source[line_start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(source.len(), |offset| line_start + offset);
    if line_end > line_start && source[line_end - 1] == b'\r' {
        line_end -= 1;
    }
    let column = String::from_utf8_lossy(&source[line_start..target])
        .chars()
        .count()
        + 1;

    LinePosition {
        line,
        column,
        line_start,
        line_end,
    }
}

fn span_caret_count(source: &[u8], line_start: usize, line_end: usize, span: SourceSpan) -> usize {
    let start = span.start.clamp(line_start, line_end);
    let end = span.end.clamp(start, line_end);
    String::from_utf8_lossy(&source[start..end])
        .chars()
        .count()
        .max(1)
}

fn source_line(source: &[u8], line_number: u32) -> Option<(usize, &[u8])> {
    let line_index = usize::try_from(line_number).ok()?;
    if line_index == 0 {
        return None;
    }
    let mut current = 1;
    let mut line_start = 0;

    for (index, byte) in source.iter().copied().enumerate() {
        if current == line_index && byte == b'\n' {
            let line = trim_line_ending(&source[line_start..index]);
            return Some((line_index, line));
        }
        if byte == b'\n' {
            current += 1;
            line_start = index + 1;
        }
    }

    if current == line_index {
        return Some((line_index, trim_line_ending(&source[line_start..])));
    }
    None
}

fn trim_line_ending(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
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
                "--repl" => {
                    config.set_input(Input::Repl)?;
                }
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
        return Err(CliError::usage(
            "missing script path, --eval source, -, or --repl",
        ));
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
        "puc51" | "puc51-compat" | "Puc51Compat" => Ok(Profile::Puc51Compat),
        _ => Err(CliError::usage(format!(
            "unknown profile {value:?}; expected luau-compat, reproducible-standard, deterministic-sim, or puc51-compat"
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
    Repl,
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
        error: Box<rustia::Error>,
        source_name: String,
        source: Vec<u8>,
        sources: BTreeMap<String, Vec<u8>>,
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
            error: Box::new(error),
            source_name: "<unknown>".to_owned(),
            source: Vec::new(),
            sources: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        format_diagnostic, parse_args, parse_profile, run_repl, run_source, CliError, Input,
    };
    use rustia::Profile;
    use std::io::Cursor;
    use std::path::PathBuf;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn rustia_error(error: CliError) -> Box<rustia::Error> {
        match error {
            CliError::Rustia { error, .. } => error,
            other => panic!("expected Rustia error, got {other:?}"),
        }
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
    fn parses_repl_mode() {
        let config = parse_args(args(&[
            "--profile=deterministic-sim",
            "--fuel=1000",
            "--source-name=session",
            "--repl",
        ]))
        .unwrap();

        assert_eq!(config.profile, Profile::DeterministicSim);
        assert_eq!(config.settings.initial_fuel, Some(1000));
        assert_eq!(config.source_name.as_deref(), Some("session"));
        assert_eq!(config.input, Some(Input::Repl));
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
        let error = rustia_error(
            run_source(&config, "cli-test.luau", b"local t = {}; t[0 / 0] = 1")
                .expect_err("NaN table key should fail"),
        );

        assert!(error.to_string().contains("cli-test.luau:1:"), "{error}");
    }

    #[test]
    fn diagnostic_formats_compile_error_source_span() {
        let config = parse_args(args(&["--eval", "return 1"])).unwrap();
        let source = "\
local x = 1
break
";
        let error = rustia_error(
            run_source(&config, "broken.luau", source.as_bytes())
                .expect_err("break outside loop should fail compilation"),
        );
        let diagnostic = format_diagnostic(&error, "broken.luau", source.as_bytes());

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
        let error = rustia_error(
            run_source(&config, "runtime.luau", source.as_bytes())
                .expect_err("NaN table key should fail"),
        );
        let diagnostic = format_diagnostic(&error, "runtime.luau", source.as_bytes());

        assert!(diagnostic.contains("runtime.luau:2:"), "{diagnostic}");
        assert!(diagnostic.contains(" --> runtime.luau:2"), "{diagnostic}");
        assert!(diagnostic.contains("2 | t[0 / 0] = 1"), "{diagnostic}");
        assert!(diagnostic.contains("| ^"), "{diagnostic}");
    }

    #[test]
    fn run_source_accepts_valid_non_owned_return_values() {
        let config = parse_args(args(&["--eval", "return 1"])).unwrap();
        run_source(
            &config,
            "<test>",
            b"local cyclic = {}; cyclic.self = cyclic; return function() end, cyclic",
        )
        .unwrap();
    }

    #[test]
    fn repl_keeps_vm_state_and_supports_expression_shorthand() {
        let config = parse_args(args(&["--repl"])).unwrap();
        let input = Cursor::new("x = 41\n=x + 1\n:quit\n");
        let mut output = Vec::new();
        let mut errors = Vec::new();

        run_repl(&config, input, &mut output, &mut errors, false).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert_eq!(output, "42\n");
        assert!(errors.is_empty(), "{}", String::from_utf8(errors).unwrap());
    }

    #[test]
    fn repl_reports_errors_and_continues() {
        let config = parse_args(args(&["--source-name", "test-session", "--repl"])).unwrap();
        let input = Cursor::new("break\n=40 + 2\n");
        let mut output = Vec::new();
        let mut errors = Vec::new();

        run_repl(&config, input, &mut output, &mut errors, false).unwrap();

        let output = String::from_utf8(output).unwrap();
        let errors = String::from_utf8(errors).unwrap();
        assert_eq!(output, "42\n");
        assert!(errors.contains("break outside loop"), "{errors}");
        assert!(errors.contains(" --> test-session[1]:1:1"), "{errors}");
        assert!(errors.contains("1 | break"), "{errors}");
    }

    #[test]
    fn repl_runtime_errors_use_the_defining_chunk_source() {
        let config = parse_args(args(&["--source-name", "session", "--repl"])).unwrap();
        let input = Cursor::new(
            "function fail_later() local values = {} values[0 / 0] = 1 end\nfail_later()\n",
        );
        let mut output = Vec::new();
        let mut errors = Vec::new();

        run_repl(&config, input, &mut output, &mut errors, false).unwrap();

        let errors = String::from_utf8(errors).unwrap();
        assert!(errors.contains("session[1]"), "{errors}");
        assert!(
            errors.contains("function fail_later() local values = {} values[0 / 0] = 1 end"),
            "{errors}"
        );
    }
}
