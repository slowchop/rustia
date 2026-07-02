use std::collections::BTreeSet;
use std::env;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(feature = "rustia-engine")]
use rustia_compiler::compile_with_options;
#[cfg(feature = "rustia-engine")]
use rustia_parser::parse_source;
#[cfg(feature = "rustia-engine")]
use rustia_vm::{Profile, Program, Value, ValueKind, Vm};

const DEFAULT_TARGET_MS: u64 = 250;
const DEFAULT_REPS: usize = 9;
const DEFAULT_MAX_N: u64 = 1_000_000_000;
const HISTORY_PATH: &str = "bench/history/runs.jsonl";
const RESULTS_PATH: &str = "bench/RESULTS.md";

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
struct Error {
    message: String,
}

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn io(path: &Path, error: std::io::Error) -> Self {
        Self::new(format!("{}: {error}", path.display()))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Dialect {
    Luau,
    Koto,
    Lua54,
    JavaScript,
    Rhai,
    Roto,
    Wasm,
}

impl Dialect {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Luau => "luau",
            Self::Koto => "koto",
            Self::Lua54 => "lua54",
            Self::JavaScript => "javascript",
            Self::Rhai => "rhai",
            Self::Roto => "roto",
            Self::Wasm => "wasm",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Workload {
    id: &'static str,
    file_stem: &'static str,
    unit: &'static str,
    default: bool,
}

const WORKLOADS: &[Workload] = &[
    Workload {
        id: "dense_tables",
        file_stem: "01_dense_tables",
        unit: "build+sum+sort of 256-elem array",
        default: true,
    },
    Workload {
        id: "sparse_tables",
        file_stem: "02_sparse_tables",
        unit: "fill+read 128 hash entries",
        default: true,
    },
    Workload {
        id: "oop_metamethods",
        file_stem: "03_oop_metamethods",
        unit: "(p+v)*s + :dot + :len2 step",
        default: false,
    },
    Workload {
        id: "closures_upvalues",
        file_stem: "04_closures_upvalues",
        unit: "one upvalue-mutating closure call",
        default: true,
    },
    Workload {
        id: "string_patterns",
        file_stem: "05_string_patterns",
        unit: "gmatch+match+gsub over 60-char line",
        default: false,
    },
    Workload {
        id: "string_format",
        file_stem: "06_string_format",
        unit: "one mixed-spec string.format",
        default: false,
    },
    Workload {
        id: "coroutines",
        file_stem: "07_coroutines",
        unit: "one resume->yield round trip",
        default: false,
    },
    Workload {
        id: "pcall_xpcall",
        file_stem: "08_pcall_xpcall",
        unit: "pcall+xpcall pair",
        default: false,
    },
    Workload {
        id: "generic_iteration",
        file_stem: "09_generic_iteration",
        unit: "__iter(32)+pairs(5)+ipairs(32) pass",
        default: true,
    },
    Workload {
        id: "stdlib_math_table",
        file_stem: "10_stdlib_math_table",
        unit: "math fastcalls + buffered push",
        default: true,
    },
    Workload {
        id: "serialization",
        file_stem: "11_serialization",
        unit: "serialize+parse 32-record round trip",
        default: false,
    },
];

#[derive(Clone, Copy)]
struct EngineSlot {
    id: &'static str,
    dialect: Dialect,
    feature: Option<&'static str>,
    enabled: bool,
    implemented: bool,
}

const ENGINE_SLOTS: &[EngineSlot] = &[
    EngineSlot {
        id: "rustia",
        dialect: Dialect::Luau,
        feature: Some("rustia-engine"),
        enabled: cfg!(feature = "rustia-engine"),
        implemented: true,
    },
    EngineSlot {
        id: "boa",
        dialect: Dialect::JavaScript,
        feature: Some("boa"),
        enabled: cfg!(feature = "boa"),
        implemented: false,
    },
    EngineSlot {
        id: "koto",
        dialect: Dialect::Koto,
        feature: Some("koto"),
        enabled: cfg!(feature = "koto"),
        implemented: cfg!(feature = "koto"),
    },
    EngineSlot {
        id: "mlua-lua54",
        dialect: Dialect::Lua54,
        feature: Some("mlua-lua54"),
        enabled: cfg!(feature = "mlua-lua54"),
        implemented: false,
    },
    EngineSlot {
        id: "mlua-luau",
        dialect: Dialect::Luau,
        feature: Some("mlua-luau"),
        enabled: cfg!(feature = "mlua-luau"),
        implemented: false,
    },
    EngineSlot {
        id: "rhai",
        dialect: Dialect::Rhai,
        feature: Some("rhai-engine"),
        enabled: cfg!(feature = "rhai-engine"),
        implemented: false,
    },
    EngineSlot {
        id: "roto",
        dialect: Dialect::Roto,
        feature: Some("roto"),
        enabled: cfg!(feature = "roto"),
        implemented: false,
    },
    EngineSlot {
        id: "rquickjs",
        dialect: Dialect::JavaScript,
        feature: Some("rquickjs-engine"),
        enabled: cfg!(feature = "rquickjs-engine"),
        implemented: false,
    },
    EngineSlot {
        id: "wasmi",
        dialect: Dialect::Wasm,
        feature: Some("wasmi-engine"),
        enabled: cfg!(feature = "wasmi-engine"),
        implemented: false,
    },
    EngineSlot {
        id: "wasmtime",
        dialect: Dialect::Wasm,
        feature: Some("wasmtime-engine"),
        enabled: cfg!(feature = "wasmtime-engine"),
        implemented: false,
    },
];

#[derive(Debug)]
struct Config {
    engines: Vec<String>,
    workloads: Vec<String>,
    target: Duration,
    reps: usize,
    max_n: u64,
    history: Option<PathBuf>,
    results: Option<PathBuf>,
    list: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engines: ENGINE_SLOTS
                .iter()
                .filter(|slot| slot.enabled && slot.implemented)
                .map(|slot| slot.id.to_owned())
                .collect(),
            workloads: WORKLOADS
                .iter()
                .filter(|workload| workload.default)
                .map(|workload| workload.id.to_owned())
                .collect(),
            target: Duration::from_millis(DEFAULT_TARGET_MS),
            reps: DEFAULT_REPS,
            max_n: DEFAULT_MAX_N,
            history: Some(PathBuf::from(HISTORY_PATH)),
            results: Some(PathBuf::from(RESULTS_PATH)),
            list: false,
        }
    }
}

#[derive(Clone, Debug)]
struct Measurement {
    engine: String,
    dialect: &'static str,
    workload: String,
    unit: &'static str,
    n: u64,
    reps: usize,
    ns_per_unit_min: f64,
    ns_per_unit_median: f64,
    units_per_sec: f64,
    checksum: String,
    commit: String,
    timestamp_unix: u64,
    target: String,
}

trait PreparedWorkload {
    fn run(&mut self, n: u64) -> Result<String>;
}

fn main() {
    match run() {
        Ok(()) => {}
        Err(error) => {
            eprintln!("script-bench failed: {error}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<()> {
    let config = parse_args(env::args().skip(1))?;
    if config.list {
        print_inventory();
        return Ok(());
    }

    let repo = repo_root()?;
    let selected_engines = select_engines(&config.engines)?;
    let selected_workloads = select_workloads(&config.workloads)?;
    let mut records = Vec::new();

    for engine in selected_engines {
        if !engine.enabled || !engine.implemented {
            println!("skip {:<12} {}", engine.id, unavailable_reason(engine));
            continue;
        }

        for workload in &selected_workloads {
            let Some(source_path) = workload_path(&repo, engine.dialect, workload) else {
                println!(
                    "skip {:<12} {:<20} no {} source",
                    engine.id,
                    workload.id,
                    engine.dialect.as_str()
                );
                continue;
            };
            let source =
                fs::read_to_string(&source_path).map_err(|error| Error::io(&source_path, error))?;
            let mut prepared = prepare(engine, workload, &source)?;
            let measurement = measure(&config, engine, workload, &mut *prepared)?;
            println!(
                "{:<12} {:<20} {:>10.2} ns/unit min {:>10.2} median n={:<8} checksum={}",
                measurement.engine,
                measurement.workload,
                measurement.ns_per_unit_min,
                measurement.ns_per_unit_median,
                measurement.n,
                measurement.checksum
            );
            records.push(measurement);
        }
    }

    if records.is_empty() {
        return Err(Error::new("no benchmark measurements were produced"));
    }

    if let Some(history) = &config.history {
        append_history(history, &records)?;
    }
    if let Some(results) = &config.results {
        write_results(results, &records)?;
    }

    Ok(())
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Config> {
    let mut config = Config::default();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--list" => config.list = true,
            "--no-history" => config.history = None,
            "--no-results" => config.results = None,
            "--engines" => {
                let value = args
                    .next()
                    .ok_or_else(|| Error::new("--engines requires a comma-separated value"))?;
                config.engines = split_csv(&value);
            }
            "--workloads" => {
                let value = args
                    .next()
                    .ok_or_else(|| Error::new("--workloads requires a comma-separated value"))?;
                config.workloads = split_csv(&value);
            }
            "--target-ms" => {
                let value = args
                    .next()
                    .ok_or_else(|| Error::new("--target-ms requires a value"))?;
                let millis = value
                    .parse::<u64>()
                    .map_err(|_| Error::new(format!("invalid --target-ms value {value:?}")))?;
                config.target = Duration::from_millis(millis);
            }
            "--reps" => {
                let value = args
                    .next()
                    .ok_or_else(|| Error::new("--reps requires a value"))?;
                config.reps = value
                    .parse::<usize>()
                    .map_err(|_| Error::new(format!("invalid --reps value {value:?}")))?;
                if config.reps == 0 {
                    return Err(Error::new("--reps must be greater than zero"));
                }
            }
            "--max-n" => {
                let value = args
                    .next()
                    .ok_or_else(|| Error::new("--max-n requires a value"))?;
                config.max_n = value
                    .parse::<u64>()
                    .map_err(|_| Error::new(format!("invalid --max-n value {value:?}")))?;
                if config.max_n == 0 {
                    return Err(Error::new("--max-n must be greater than zero"));
                }
            }
            "--history" => {
                let value = args
                    .next()
                    .ok_or_else(|| Error::new("--history requires a path"))?;
                config.history = Some(PathBuf::from(value));
            }
            "--results" => {
                let value = args
                    .next()
                    .ok_or_else(|| Error::new("--results requires a path"))?;
                config.results = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            _ => return Err(Error::new(format!("unknown argument {arg:?}"))),
        }
    }
    Ok(config)
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

fn print_usage() {
    println!("usage: script-bench [options]");
    println!("  --list");
    println!("  --engines rustia,koto,mlua-luau");
    println!("  --workloads dense_tables,sparse_tables,closures_upvalues,generic_iteration,stdlib_math_table");
    println!("  --target-ms 250");
    println!("  --reps 9");
    println!("  --history bench/history/runs.jsonl");
    println!("  --results bench/RESULTS.md");
    println!("  --no-history");
    println!("  --no-results");
}

fn print_inventory() {
    println!("engines:");
    for slot in ENGINE_SLOTS {
        let status = if slot.enabled && slot.implemented {
            "available"
        } else {
            unavailable_reason(slot)
        };
        println!("  {:<12} {:<10} {}", slot.id, slot.dialect.as_str(), status);
    }
    println!();
    println!("workloads:");
    for workload in WORKLOADS {
        let default = if workload.default { "default" } else { "" };
        println!("  {:<20} {:<8} {}", workload.id, default, workload.unit);
    }
}

fn unavailable_reason(slot: &EngineSlot) -> &'static str {
    if !slot.enabled {
        match slot.feature {
            Some(feature) => match feature {
                "rustia-engine" => "compile with --features rustia-engine",
                "boa" => "compile with --features boa",
                "koto" => "compile with --features koto",
                "mlua-lua54" => "compile with --features mlua-lua54",
                "mlua-luau" => "compile with --features mlua-luau",
                "rhai-engine" => "compile with --features rhai-engine",
                "roto" => "compile with --features roto",
                "rquickjs-engine" => "compile with --features rquickjs-engine",
                "wasmi-engine" => "compile with --features wasmi-engine",
                "wasmtime-engine" => "compile with --features wasmtime-engine",
                _ => "feature disabled",
            },
            None => "feature disabled",
        }
    } else if !slot.implemented {
        "adapter pending"
    } else {
        "available"
    }
}

fn select_engines(selected: &[String]) -> Result<Vec<&'static EngineSlot>> {
    let known = ENGINE_SLOTS
        .iter()
        .map(|slot| slot.id)
        .collect::<BTreeSet<_>>();
    selected
        .iter()
        .map(|id| {
            ENGINE_SLOTS
                .iter()
                .find(|slot| slot.id == id)
                .ok_or_else(|| Error::new(format!("unknown engine {id:?}; known: {known:?}")))
        })
        .collect()
}

fn select_workloads(selected: &[String]) -> Result<Vec<&'static Workload>> {
    let known = WORKLOADS
        .iter()
        .map(|workload| workload.id)
        .collect::<BTreeSet<_>>();
    selected
        .iter()
        .map(|id| {
            WORKLOADS
                .iter()
                .find(|workload| workload.id == id)
                .ok_or_else(|| Error::new(format!("unknown workload {id:?}; known: {known:?}")))
        })
        .collect()
}

fn repo_root() -> Result<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| Error::new("bench crate must live under <repo>/bench"))
}

fn workload_path(repo: &Path, dialect: Dialect, workload: &Workload) -> Option<PathBuf> {
    let extension = match dialect {
        Dialect::Luau => "luau",
        Dialect::Koto => "koto",
        Dialect::Lua54 => "lua",
        Dialect::JavaScript => "js",
        Dialect::Rhai => "rhai",
        Dialect::Roto => "roto",
        Dialect::Wasm => "wat",
    };
    let path = repo
        .join("bench/workloads")
        .join(dialect.as_str())
        .join(format!("{}.{}", workload.file_stem, extension));
    path.exists().then_some(path)
}

fn prepare(
    engine: &EngineSlot,
    _workload: &Workload,
    _source: &str,
) -> Result<Box<dyn PreparedWorkload>> {
    match engine.id {
        #[cfg(feature = "rustia-engine")]
        "rustia" => Ok(Box::new(RustiaPrepared::new(_workload, _source)?)),
        #[cfg(feature = "koto")]
        "koto" => Ok(Box::new(KotoPrepared::new(_workload, _source)?)),
        id => Err(Error::new(format!(
            "engine adapter {id:?} is not implemented yet"
        ))),
    }
}

fn measure(
    config: &Config,
    engine: &EngineSlot,
    workload: &Workload,
    prepared: &mut dyn PreparedWorkload,
) -> Result<Measurement> {
    let mut n = 16_u64;
    loop {
        let (_, elapsed) = run_once(prepared, n)?;
        if elapsed >= config.target || n >= config.max_n {
            if elapsed.as_nanos() > 0 {
                let scale = config.target.as_secs_f64() / elapsed.as_secs_f64();
                n = ((n as f64 * scale).max(1.0).min(config.max_n as f64)) as u64;
            }
            break;
        }
        n = n.saturating_mul(2).min(config.max_n);
    }

    let _ = run_once(prepared, n)?;
    let mut samples = Vec::with_capacity(config.reps);
    let mut checksum = String::new();
    for _ in 0..config.reps {
        let (sample_checksum, elapsed) = run_once(prepared, n)?;
        checksum = sample_checksum;
        samples.push(elapsed.as_secs_f64());
    }
    samples.sort_by(|lhs, rhs| lhs.total_cmp(rhs));
    let min_s = samples[0];
    let median_s = samples[samples.len() / 2];
    let n_f64 = n as f64;

    Ok(Measurement {
        engine: engine.id.to_owned(),
        dialect: engine.dialect.as_str(),
        workload: workload.id.to_owned(),
        unit: workload.unit,
        n,
        reps: config.reps,
        ns_per_unit_min: min_s * 1e9 / n_f64,
        ns_per_unit_median: median_s * 1e9 / n_f64,
        units_per_sec: n_f64 / min_s,
        checksum,
        commit: git_commit(),
        timestamp_unix: unix_time(),
        target: format!("{}-{}", env::consts::OS, env::consts::ARCH),
    })
}

fn run_once(prepared: &mut dyn PreparedWorkload, n: u64) -> Result<(String, Duration)> {
    let start = Instant::now();
    let checksum = prepared.run(n)?;
    Ok((checksum, start.elapsed()))
}

#[cfg(feature = "rustia-engine")]
struct RustiaPrepared {
    program: Program,
    vm: Vm,
    function: Value,
}

#[cfg(feature = "rustia-engine")]
impl RustiaPrepared {
    fn new(workload: &Workload, source: &str) -> Result<Self> {
        let chunk = parse_source(source)
            .map_err(|error| Error::new(format!("parse {} failed: {error}", workload.id)))?;
        let program = compile_with_options(
            &chunk,
            rustia_compiler::CompileOptions::default().with_source(workload.id, source),
        )
        .map_err(|error| Error::new(format!("compile {} failed: {error}", workload.id)))?;
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let function = vm
            .run(&program)
            .map_err(|error| Error::new(format!("initialize {} failed: {error}", workload.id)))?;
        if function.kind() != ValueKind::Function {
            return Err(Error::new(format!(
                "workload {} must return a function",
                workload.id
            )));
        }
        Ok(Self {
            program,
            vm,
            function,
        })
    }
}

#[cfg(feature = "rustia-engine")]
impl PreparedWorkload for RustiaPrepared {
    fn run(&mut self, n: u64) -> Result<String> {
        let result = self
            .vm
            .call_function(&self.program, self.function, &[Value::number(n as f64)])
            .map_err(|error| Error::new(format!("rustia workload call failed: {error}")))?;
        value_checksum(&mut self.vm, result)
    }
}

#[cfg(feature = "koto")]
struct KotoPrepared {
    koto: koto::prelude::Koto,
    function: koto::prelude::KValue,
}

#[cfg(feature = "koto")]
impl KotoPrepared {
    fn new(workload: &Workload, source: &str) -> Result<Self> {
        use koto::prelude::Koto;

        let mut koto = Koto::default();
        koto.compile_and_run(source)
            .map_err(|error| Error::new(format!("initialize {} failed: {error}", workload.id)))?;
        let function = koto.exports().get("f").ok_or_else(|| {
            Error::new(format!(
                "koto workload {} must export function f",
                workload.id
            ))
        })?;
        Ok(Self { koto, function })
    }
}

#[cfg(feature = "koto")]
impl PreparedWorkload for KotoPrepared {
    fn run(&mut self, n: u64) -> Result<String> {
        use koto::prelude::{KNumber, KValue};

        let n = i64::try_from(n).map_err(|_| Error::new("koto n exceeded i64::MAX"))?;
        let result = self
            .koto
            .call_function(self.function.clone(), &[KValue::Number(KNumber::I64(n))])
            .map_err(|error| Error::new(format!("koto workload call failed: {error}")))?;
        Ok(koto_checksum(&result))
    }
}

#[cfg(feature = "koto")]
fn koto_checksum(value: &koto::prelude::KValue) -> String {
    use koto::prelude::{KNumber, KValue};

    match value {
        KValue::Null => "null".to_owned(),
        KValue::Bool(value) => format!("bool:{value}"),
        KValue::Number(KNumber::I64(value)) => format!("i64:{value}"),
        KValue::Number(KNumber::F64(value)) => format!("f64:{value:.17}"),
        KValue::Str(value) => format!("string:{}", value.as_str()),
        other => format!("{other:?}"),
    }
}

#[cfg(feature = "rustia-engine")]
fn value_checksum(vm: &mut Vm, value: Value) -> Result<String> {
    match value.kind() {
        ValueKind::Nil => Ok("nil".to_owned()),
        ValueKind::Boolean => Ok(format!("bool:{}", value.as_bool().unwrap_or(false))),
        ValueKind::Number => Ok(format!("number:{:.17}", value.as_number().unwrap_or(0.0))),
        ValueKind::String => vm
            .string_value(value)
            .map_err(|error| Error::new(format!("string checksum failed: {error}")))?
            .map(|text| format!("string:{text}"))
            .ok_or_else(|| Error::new("string value missing")),
        other => Ok(format!("{other:?}")),
    }
}

fn append_history(path: &Path, records: &[Measurement]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| Error::io(path, error))?;
    for record in records {
        writeln!(file, "{}", record_json(record)).map_err(|error| Error::io(path, error))?;
    }
    Ok(())
}

fn write_results(path: &Path, records: &[Measurement]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
    }
    let mut text = String::new();
    text.push_str("# script-bench-harness Results\n\n");
    text.push_str(
        "| engine | workload | ns/unit min | ns/unit median | units/sec | n | checksum |\n",
    );
    text.push_str("|---|---|---:|---:|---:|---:|---|\n");
    for record in records {
        text.push_str(&format!(
            "| {} | {} | {:.2} | {:.2} | {:.0} | {} | `{}` |\n",
            record.engine,
            record.workload,
            record.ns_per_unit_min,
            record.ns_per_unit_median,
            record.units_per_sec,
            record.n,
            record.checksum.replace('|', "\\|")
        ));
    }
    fs::write(path, text).map_err(|error| Error::io(path, error))
}

fn record_json(record: &Measurement) -> String {
    format!(
        "{{\"timestamp_unix\":{},\"commit\":\"{}\",\"target\":\"{}\",\"engine\":\"{}\",\"dialect\":\"{}\",\"workload\":\"{}\",\"unit\":\"{}\",\"n\":{},\"reps\":{},\"ns_per_unit_min\":{},\"ns_per_unit_median\":{},\"units_per_sec\":{},\"checksum\":\"{}\"}}",
        record.timestamp_unix,
        json_escape(&record.commit),
        json_escape(&record.target),
        json_escape(&record.engine),
        json_escape(record.dialect),
        json_escape(&record.workload),
        json_escape(record.unit),
        record.n,
        record.reps,
        finite_json_number(record.ns_per_unit_min),
        finite_json_number(record.ns_per_unit_median),
        finite_json_number(record.units_per_sec),
        json_escape(&record.checksum)
    )
}

fn finite_json_number(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.6}")
    } else {
        "null".to_owned()
    }
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped
}

fn git_commit() -> String {
    Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
