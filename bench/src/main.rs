use std::collections::{BTreeMap, BTreeSet};
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

#[cfg(all(feature = "mlua-luau", feature = "mlua-lua54"))]
compile_error!("features mlua-luau and mlua-lua54 cannot be enabled in the same binary; run them as separate benchmark binaries");

const DEFAULT_TARGET_MS: u64 = 250;
const DEFAULT_REPS: usize = 9;
const DEFAULT_MAX_N: u64 = 1_000_000_000;
const HISTORY_PATH: &str = "bench/history/runs.jsonl";
const RESULTS_PATH: &str = "bench/RESULTS.md";
const CURRENT_RESULTS_JSON_PATH: &str = "bench/current-results.json";
const WASM_GATE_PATH: &str = "bench/wasm-gate.json";

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
    Workload {
        id: "string_building",
        file_stem: "15_string_building",
        unit: "build+concat 64 string fragments",
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
        implemented: true,
    },
    EngineSlot {
        id: "koto",
        dialect: Dialect::Koto,
        feature: Some("koto"),
        enabled: cfg!(feature = "koto"),
        implemented: true,
    },
    EngineSlot {
        id: "mlua-lua54",
        dialect: Dialect::Lua54,
        feature: Some("mlua-lua54"),
        enabled: cfg!(feature = "mlua-lua54"),
        implemented: true,
    },
    EngineSlot {
        id: "mlua-luau",
        dialect: Dialect::Luau,
        feature: Some("mlua-luau"),
        enabled: cfg!(feature = "mlua-luau"),
        implemented: true,
    },
    EngineSlot {
        id: "rhai",
        dialect: Dialect::Rhai,
        feature: Some("rhai-engine"),
        enabled: cfg!(feature = "rhai-engine"),
        implemented: true,
    },
    EngineSlot {
        id: "roto",
        dialect: Dialect::Roto,
        feature: Some("roto"),
        enabled: cfg!(feature = "roto"),
        implemented: true,
    },
    EngineSlot {
        id: "rquickjs",
        dialect: Dialect::JavaScript,
        feature: Some("rquickjs-engine"),
        enabled: cfg!(feature = "rquickjs-engine"),
        implemented: true,
    },
    EngineSlot {
        id: "wasmi",
        dialect: Dialect::Wasm,
        feature: Some("wasmi-engine"),
        enabled: cfg!(feature = "wasmi-engine"),
        implemented: true,
    },
    EngineSlot {
        id: "wasmtime",
        dialect: Dialect::Wasm,
        feature: Some("wasmtime-engine"),
        enabled: cfg!(feature = "wasmtime-engine"),
        implemented: true,
    },
];

#[derive(Debug)]
struct Config {
    engines: Vec<String>,
    workloads: Vec<String>,
    target: Duration,
    reps: usize,
    max_n: u64,
    fixed_n: Option<u64>,
    history: Option<PathBuf>,
    results: Option<PathBuf>,
    latest_json: Option<PathBuf>,
    samples_json: Option<PathBuf>,
    commit: Option<String>,
    list: bool,
    list_json: bool,
    wasm_gate_json: bool,
    check_history: bool,
    refresh_results: bool,
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
            fixed_n: None,
            history: Some(PathBuf::from(HISTORY_PATH)),
            results: Some(PathBuf::from(RESULTS_PATH)),
            latest_json: Some(PathBuf::from(CURRENT_RESULTS_JSON_PATH)),
            samples_json: None,
            commit: None,
            list: false,
            list_json: false,
            wasm_gate_json: false,
            check_history: false,
            refresh_results: false,
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
    n_selection: NSelection,
    reps: usize,
    ns_per_unit_min: f64,
    ns_per_unit_median: f64,
    samples_ns_per_unit: Vec<f64>,
    units_per_sec: f64,
    checksum: String,
    commit: String,
    timestamp_unix: u64,
    target: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NSelection {
    Calibrated,
    Fixed,
}

impl NSelection {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Calibrated => "calibrated",
            Self::Fixed => "fixed",
        }
    }
}

#[derive(Clone, Debug)]
struct ResultRow {
    timestamp_unix: u64,
    commit: String,
    target: String,
    engine: String,
    dialect: String,
    workload: String,
    unit: String,
    n: u64,
    ns_per_unit_min: f64,
    ns_per_unit_median: f64,
    units_per_sec: f64,
    checksum: String,
}

#[derive(Clone, Debug)]
struct HistoryRecord {
    timestamp_unix: u64,
    commit: String,
    target: String,
    engine: String,
    dialect: String,
    workload: String,
    unit: String,
    n: u64,
    ns_per_unit_min: f64,
    ns_per_unit_median: f64,
    units_per_sec: f64,
    checksum: String,
}

#[derive(Debug)]
struct HistorySummary {
    records: usize,
    engines: BTreeSet<String>,
    workloads: BTreeSet<String>,
    targets: BTreeSet<String>,
    first_timestamp_unix: u64,
    last_timestamp_unix: u64,
}

impl ResultRow {
    fn from_measurement(record: &Measurement) -> Self {
        Self {
            timestamp_unix: record.timestamp_unix,
            commit: record.commit.clone(),
            target: record.target.clone(),
            engine: record.engine.clone(),
            dialect: record.dialect.to_owned(),
            workload: record.workload.clone(),
            unit: record.unit.to_owned(),
            n: record.n,
            ns_per_unit_min: record.ns_per_unit_min,
            ns_per_unit_median: record.ns_per_unit_median,
            units_per_sec: record.units_per_sec,
            checksum: record.checksum.clone(),
        }
    }
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
    if config.wasm_gate_json {
        print_wasm_gate_json()?;
        return Ok(());
    }
    if config.list_json {
        print_inventory_json()?;
        return Ok(());
    }
    if config.list {
        print_inventory();
        return Ok(());
    }
    if config.check_history {
        let history = config
            .history
            .as_deref()
            .ok_or_else(|| Error::new("--check-history requires history output to be enabled"))?;
        let summary = check_history(history)?;
        println!(
            "history ok: {} records, {} engines, {} workloads, {} targets, {}..{}",
            summary.records,
            summary.engines.len(),
            summary.workloads.len(),
            summary.targets.len(),
            summary.first_timestamp_unix,
            summary.last_timestamp_unix
        );
        return Ok(());
    }
    if config.refresh_results {
        let history = config
            .history
            .as_deref()
            .ok_or_else(|| Error::new("--refresh-results requires history output to be enabled"))?;
        let mut outputs = Vec::new();
        if let Some(results) = config.results.as_deref() {
            let rows = write_results(results, Some(history), &[])?;
            outputs.push(format!("{rows} markdown rows -> {}", results.display()));
        }
        if let Some(latest_json) = config.latest_json.as_deref() {
            let rows = write_latest_json(latest_json, Some(history), &[])?;
            outputs.push(format!("{rows} JSON rows -> {}", latest_json.display()));
        }
        if outputs.is_empty() {
            return Err(Error::new(
                "--refresh-results requires results or latest JSON output to be enabled",
            ));
        }
        println!(
            "results refreshed from {}: {}",
            history.display(),
            outputs.join(", ")
        );
        return Ok(());
    }

    validate_measurement_outputs(&config)?;

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
            let measurement = measure(&config, engine, workload, &source)?;
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

    // Persist raw evidence before its summary row. A sidecar without history can be retried; a
    // history row whose raw fixed-work evidence failed to write cannot be reconstructed.
    if let Some(samples_json) = &config.samples_json {
        write_samples_json(samples_json, &records)?;
    }
    if let Some(history) = &config.history {
        append_history(history, &records)?;
    }
    if let Some(results) = &config.results {
        write_results(results, config.history.as_deref(), &records)?;
    }
    if let Some(latest_json) = &config.latest_json {
        write_latest_json(latest_json, config.history.as_deref(), &records)?;
    }

    Ok(())
}

fn validate_measurement_outputs(config: &Config) -> Result<()> {
    if config.fixed_n.is_some() && config.history.is_some() && config.samples_json.is_none() {
        return Err(Error::new(
            "fixed-N history publication requires --samples-json raw evidence",
        ));
    }
    Ok(())
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Config> {
    let mut config = Config::default();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--list" => config.list = true,
            "--list-json" => config.list_json = true,
            "--wasm-gate-json" => config.wasm_gate_json = true,
            "--check-history" => config.check_history = true,
            "--refresh-results" => config.refresh_results = true,
            "--no-history" => config.history = None,
            "--no-results" => config.results = None,
            "--no-latest-json" => config.latest_json = None,
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
            "--fixed-n" => {
                let value = args
                    .next()
                    .ok_or_else(|| Error::new("--fixed-n requires a value"))?;
                let fixed_n = value
                    .parse::<u64>()
                    .map_err(|_| Error::new(format!("invalid --fixed-n value {value:?}")))?;
                if fixed_n == 0 {
                    return Err(Error::new("--fixed-n must be greater than zero"));
                }
                config.fixed_n = Some(fixed_n);
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
            "--latest-json" => {
                let value = args
                    .next()
                    .ok_or_else(|| Error::new("--latest-json requires a path"))?;
                config.latest_json = Some(PathBuf::from(value));
            }
            "--samples-json" => {
                let value = args
                    .next()
                    .ok_or_else(|| Error::new("--samples-json requires a path"))?;
                config.samples_json = Some(PathBuf::from(value));
            }
            "--commit" => {
                let value = args
                    .next()
                    .ok_or_else(|| Error::new("--commit requires a value"))?;
                if value.trim().is_empty() {
                    return Err(Error::new("--commit must not be empty"));
                }
                if value.ends_with("-dirty") {
                    return Err(Error::new(
                        "--commit must name a clean benchmark baseline commit",
                    ));
                }
                config.commit = Some(value);
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
    println!("  --list-json");
    println!("  --wasm-gate-json");
    println!("  --check-history");
    println!("  --refresh-results");
    println!("  --engines rustia,koto,mlua-luau");
    println!("  --workloads dense_tables,sparse_tables,closures_upvalues,generic_iteration,stdlib_math_table");
    println!("  --target-ms 250");
    println!("  --reps 9");
    println!("  --fixed-n <n>");
    println!("  --history bench/history/runs.jsonl");
    println!("  --results bench/RESULTS.md");
    println!("  --latest-json bench/current-results.json");
    println!("  --samples-json <path>");
    println!("  --commit <clean-git-id>");
    println!("  --no-history");
    println!("  --no-results");
    println!("  --no-latest-json");
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

fn print_inventory_json() -> Result<()> {
    let text = serde_json::to_string_pretty(&inventory_json_value())
        .map_err(|error| Error::new(format!("inventory JSON failed: {error}")))?;
    println!("{text}");
    Ok(())
}

fn print_wasm_gate_json() -> Result<()> {
    let text = serde_json::to_string_pretty(&wasm_gate_json_value())
        .map_err(|error| Error::new(format!("wasm gate JSON failed: {error}")))?;
    println!("{text}");
    Ok(())
}

fn inventory_json_value() -> serde_json::Value {
    let engines = ENGINE_SLOTS
        .iter()
        .map(|slot| {
            serde_json::json!({
                "id": slot.id,
                "dialect": slot.dialect.as_str(),
                "feature": slot.feature,
                "enabled": slot.enabled,
                "implemented": slot.implemented,
                "available": slot.enabled && slot.implemented,
                "status": if slot.enabled && slot.implemented {
                    "available"
                } else {
                    unavailable_reason(slot)
                },
            })
        })
        .collect::<Vec<_>>();
    let workloads = WORKLOADS
        .iter()
        .map(|workload| {
            serde_json::json!({
                "id": workload.id,
                "file_stem": workload.file_stem,
                "unit": workload.unit,
                "default": workload.default,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "schema_version": 1,
        "binary": "script-bench",
        "history_schema": "jsonl-v1",
        "default_history": HISTORY_PATH,
        "default_results": RESULTS_PATH,
        "default_current_results_json": CURRENT_RESULTS_JSON_PATH,
        "default_latest_json": CURRENT_RESULTS_JSON_PATH,
        "default_wasm_gate": WASM_GATE_PATH,
        "wasm_gate": wasm_gate_json_value(),
        "engines": engines,
        "workloads": workloads,
    })
}

fn wasm_gate_json_value() -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "schema": "rustia.bench.wasm-gate.v1",
        "source": "script-bench-harness",
        "updated": "2026-07-22",
        "scope": "Production rustia wasm value-layout and dispatch pre-lock gate. Grabby integration work is excluded.",
        "status": "complete",
        "gate_complete": true,
        "summary": "The recurring public wasm benchmark uses Node/V8, the most widely used JavaScript runtime. The completed V8, JavaScriptCore-family, and SpiderMonkey pre-lock evidence is retained as historical coverage, and br_table remains the portable dispatch baseline.",
        "requirements": [
            {
                "id": "v8-production-rustia-wasm",
                "label": "V8 production rustia wasm",
                "engine": "V8",
                "runtime": "node",
                "required": true,
                "status": "measured",
                "evidence": [
                    {
                        "kind": "current-results",
                        "path": "bench/wasm/current-results.json",
                        "summary": "Production rustia wasm retained-function workloads measured under the active Node/V8 runtime.",
                    },
                    {
                        "kind": "design-spike",
                        "path": "design/65-wasmjit.md",
                        "summary": "V8 wasm-emit hot-kernel spike was verified, but it is not a production rustia wasm measurement.",
                    },
                ],
                "next": "Keep V8 history fresh as the benchmark harness and production wasm artifact evolve.",
            },
            {
                "id": "javascriptcore-production-rustia-wasm",
                "label": "JavaScriptCore production rustia wasm",
                "engine": "JavaScriptCore",
                "runtime": "jsc / bun / Safari Technology Preview",
                "required": true,
                "status": "measured",
                "evidence": [
                    {
                        "kind": "historical-results",
                        "path": "bench/wasm/history/20260720T100056Z-62bed545-full-matrix.json",
                        "summary": "The retained full matrix measured production rustia wasm under Bun's JavaScriptCore runtime.",
                    },
                ],
                "next": "Add a Safari Technology Preview or jsc-shell run when that runtime is available.",
            },
            {
                "id": "spidermonkey-production-rustia-wasm",
                "label": "SpiderMonkey production rustia wasm",
                "engine": "SpiderMonkey",
                "runtime": "js / sm",
                "required": true,
                "status": "measured",
                "evidence": [
                    {
                        "kind": "historical-results",
                        "path": "bench/wasm/history/20260720T100056Z-62bed545-full-matrix.json",
                        "summary": "The retained full matrix measured production rustia wasm under the SpiderMonkey js shell.",
                    },
                ],
                "next": "Keep SpiderMonkey history fresh as the benchmark harness and production wasm artifact evolve.",
            },
            {
                "id": "call-ref-vs-br-table",
                "label": "call_ref vs br_table dispatch",
                "engine": "Wasm",
                "runtime": "Node/V8 active; V8 / JavaScriptCore / SpiderMonkey historical",
                "required": true,
                "status": "measured",
                "evidence": [
                    {
                        "kind": "dispatch-current-results",
                        "path": "bench/wasm/dispatch-current-results.json",
                        "summary": "The active dispatch comparison is measured under Node/V8.",
                    },
                    {
                        "kind": "historical-results",
                        "path": "bench/wasm/history/20260720T100056Z-62bed545-dispatch-matrix.json",
                        "summary": "The retained full matrix covers V8, JavaScriptCore-family, and SpiderMonkey runtimes.",
                    },
                ],
                "next": "Keep the microbench in the wasm gate checks so future runtime shifts cannot reopen the dispatch assumption silently.",
            },
        ],
    })
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
        #[cfg(feature = "boa")]
        "boa" => Ok(Box::new(BoaPrepared::new(_workload, _source)?)),
        #[cfg(feature = "mlua-lua54")]
        "mlua-lua54" => Ok(Box::new(MluaLua54Prepared::new(_workload, _source)?)),
        #[cfg(feature = "mlua-luau")]
        "mlua-luau" => Ok(Box::new(MluaLuauPrepared::new(_workload, _source)?)),
        #[cfg(feature = "rhai-engine")]
        "rhai" => Ok(Box::new(RhaiPrepared::new(_workload, _source)?)),
        #[cfg(feature = "roto")]
        "roto" => Ok(Box::new(RotoPrepared::new(_workload, _source)?)),
        #[cfg(feature = "rquickjs-engine")]
        "rquickjs" => Ok(Box::new(RquickJsPrepared::new(_workload, _source)?)),
        #[cfg(feature = "wasmi-engine")]
        "wasmi" => Ok(Box::new(WasmiPrepared::new(_workload, _source)?)),
        #[cfg(feature = "wasmtime-engine")]
        "wasmtime" => Ok(Box::new(WasmtimePrepared::new(_workload, _source)?)),
        id => Err(Error::new(format!(
            "engine adapter {id:?} is not implemented yet"
        ))),
    }
}

fn measure(
    config: &Config,
    engine: &EngineSlot,
    workload: &Workload,
    source: &str,
) -> Result<Measurement> {
    measure_with_factory(config, engine, workload, || {
        prepare(engine, workload, source)
    })
}

fn measure_with_factory(
    config: &Config,
    engine: &EngineSlot,
    workload: &Workload,
    mut factory: impl FnMut() -> Result<Box<dyn PreparedWorkload>>,
) -> Result<Measurement> {
    let (n, n_selection) = if let Some(n) = config.fixed_n {
        (n, NSelection::Fixed)
    } else {
        let mut n = 16_u64.min(config.max_n);
        loop {
            let mut calibration = factory()?;
            let _ = calibration.run(n)?;
            let (_, elapsed) = run_once(&mut *calibration, n)?;
            if elapsed >= config.target || n >= config.max_n {
                if elapsed.as_nanos() > 0 {
                    let scale = config.target.as_secs_f64() / elapsed.as_secs_f64();
                    n = ((n as f64 * scale).max(1.0).min(config.max_n as f64)) as u64;
                }
                break;
            }
            n = n.saturating_mul(2).min(config.max_n);
        }
        (n, NSelection::Calibrated)
    };

    let n_f64 = n as f64;
    let mut samples_ns_per_unit = Vec::with_capacity(config.reps);
    let mut checksum = String::new();
    for _ in 0..config.reps {
        let mut prepared = factory()?;
        let _ = prepared.run(n)?;
        let (sample_checksum, elapsed) = run_once(&mut *prepared, n)?;
        checksum = sample_checksum;
        samples_ns_per_unit.push(elapsed.as_secs_f64() * 1e9 / n_f64);
    }
    let mut sorted_samples = samples_ns_per_unit.clone();
    sorted_samples.sort_by(|lhs, rhs| lhs.total_cmp(rhs));
    let ns_per_unit_min = sorted_samples[0];
    let ns_per_unit_median = sorted_samples[sorted_samples.len() / 2];

    Ok(Measurement {
        engine: engine.id.to_owned(),
        dialect: engine.dialect.as_str(),
        workload: workload.id.to_owned(),
        unit: workload.unit,
        n,
        n_selection,
        reps: config.reps,
        ns_per_unit_min,
        ns_per_unit_median,
        samples_ns_per_unit,
        units_per_sec: 1e9 / ns_per_unit_min,
        checksum,
        commit: config.commit.clone().unwrap_or_else(git_commit),
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

#[cfg(feature = "boa")]
struct BoaPrepared {
    context: boa_engine::Context,
    function: boa_engine::object::JsObject,
}

#[cfg(feature = "boa")]
impl BoaPrepared {
    fn new(workload: &Workload, source: &str) -> Result<Self> {
        let mut context = boa_engine::Context::default();
        let value = context
            .eval(boa_engine::Source::from_bytes(source))
            .map_err(|error| Error::new(format!("initialize {} failed: {error}", workload.id)))?;
        let function = value.as_callable().ok_or_else(|| {
            Error::new(format!(
                "javascript workload {} must evaluate to a function",
                workload.id
            ))
        })?;
        Ok(Self { context, function })
    }
}

#[cfg(feature = "boa")]
impl PreparedWorkload for BoaPrepared {
    fn run(&mut self, n: u64) -> Result<String> {
        let result = self
            .function
            .call(
                &boa_engine::JsValue::undefined(),
                &[boa_engine::JsValue::new(n as f64)],
                &mut self.context,
            )
            .map_err(|error| Error::new(format!("boa workload call failed: {error}")))?;
        Ok(boa_checksum(&result))
    }
}

#[cfg(feature = "boa")]
fn boa_checksum(value: &boa_engine::JsValue) -> String {
    if value.is_undefined() {
        return "undefined".to_owned();
    }
    if value.is_null() {
        return "null".to_owned();
    }
    if let Some(value) = value.as_boolean() {
        return format!("bool:{value}");
    }
    if let Some(value) = value.as_number() {
        return format!("f64:{value:.17}");
    }
    if let Some(value) = value.as_string() {
        return format!("string:{}", value.to_std_string_escaped());
    }
    value.display().to_string()
}

#[cfg(feature = "mlua-lua54")]
struct MluaLua54Prepared {
    _lua: mlua_lua54::Lua,
    function: mlua_lua54::Function,
}

#[cfg(feature = "mlua-lua54")]
impl MluaLua54Prepared {
    fn new(workload: &Workload, source: &str) -> Result<Self> {
        let lua = mlua_lua54::Lua::new();
        let function = lua
            .load(source)
            .eval::<mlua_lua54::Function>()
            .map_err(|error| Error::new(format!("initialize {} failed: {error}", workload.id)))?;
        Ok(Self {
            _lua: lua,
            function,
        })
    }
}

#[cfg(feature = "mlua-lua54")]
impl PreparedWorkload for MluaLua54Prepared {
    fn run(&mut self, n: u64) -> Result<String> {
        let n = i64::try_from(n).map_err(|_| Error::new("mlua-lua54 n exceeded i64::MAX"))?;
        let result = self
            .function
            .call::<mlua_lua54::Value>(n)
            .map_err(|error| Error::new(format!("mlua-lua54 workload call failed: {error}")))?;
        Ok(mlua_lua54_checksum(&result))
    }
}

#[cfg(feature = "mlua-lua54")]
fn mlua_lua54_checksum(value: &mlua_lua54::Value) -> String {
    match value {
        mlua_lua54::Value::Nil => "nil".to_owned(),
        mlua_lua54::Value::Boolean(value) => format!("bool:{value}"),
        mlua_lua54::Value::Integer(value) => format!("i64:{value}"),
        mlua_lua54::Value::Number(value) => format!("f64:{value:.17}"),
        mlua_lua54::Value::String(value) => match value.to_str() {
            Ok(text) => format!("string:{text}"),
            Err(_) => format!("bytes:{}", value.as_bytes().len()),
        },
        other => format!("{}", other.type_name()),
    }
}

#[cfg(feature = "mlua-luau")]
struct MluaLuauPrepared {
    _lua: mlua_luau::Lua,
    function: mlua_luau::Function,
}

#[cfg(feature = "mlua-luau")]
impl MluaLuauPrepared {
    fn new(workload: &Workload, source: &str) -> Result<Self> {
        let lua = mlua_luau::Lua::new();
        let function = lua
            .load(source)
            .eval::<mlua_luau::Function>()
            .map_err(|error| Error::new(format!("initialize {} failed: {error}", workload.id)))?;
        Ok(Self {
            _lua: lua,
            function,
        })
    }
}

#[cfg(feature = "mlua-luau")]
impl PreparedWorkload for MluaLuauPrepared {
    fn run(&mut self, n: u64) -> Result<String> {
        let n = i64::try_from(n).map_err(|_| Error::new("mlua-luau n exceeded i64::MAX"))?;
        let result = self
            .function
            .call::<mlua_luau::Value>(n)
            .map_err(|error| Error::new(format!("mlua-luau workload call failed: {error}")))?;
        Ok(mlua_luau_checksum(&result))
    }
}

#[cfg(feature = "mlua-luau")]
fn mlua_luau_checksum(value: &mlua_luau::Value) -> String {
    match value {
        mlua_luau::Value::Nil => "nil".to_owned(),
        mlua_luau::Value::Boolean(value) => format!("bool:{value}"),
        mlua_luau::Value::Integer(value) => format!("i64:{value}"),
        mlua_luau::Value::Number(value) => format!("f64:{value:.17}"),
        mlua_luau::Value::String(value) => match value.to_str() {
            Ok(text) => format!("string:{text}"),
            Err(_) => format!("bytes:{}", value.as_bytes().len()),
        },
        other => format!("{}", other.type_name()),
    }
}

#[cfg(feature = "rhai-engine")]
struct RhaiPrepared {
    engine: rhai::Engine,
    ast: rhai::AST,
    scope: rhai::Scope<'static>,
}

#[cfg(feature = "rhai-engine")]
impl RhaiPrepared {
    fn new(workload: &Workload, source: &str) -> Result<Self> {
        let engine = rhai::Engine::new();
        let ast = engine
            .compile(source)
            .map_err(|error| Error::new(format!("compile {} failed: {error}", workload.id)))?;
        Ok(Self {
            engine,
            ast,
            scope: rhai::Scope::new(),
        })
    }
}

#[cfg(feature = "rhai-engine")]
impl PreparedWorkload for RhaiPrepared {
    fn run(&mut self, n: u64) -> Result<String> {
        let n = rhai::INT::try_from(n).map_err(|_| Error::new("rhai n exceeded INT::MAX"))?;
        let result = self
            .engine
            .call_fn::<rhai::Dynamic>(&mut self.scope, &self.ast, "f", (n,))
            .map_err(|error| Error::new(format!("rhai workload call failed: {error}")))?;
        Ok(rhai_checksum(&result))
    }
}

#[cfg(feature = "rhai-engine")]
fn rhai_checksum(value: &rhai::Dynamic) -> String {
    if let Ok(value) = value.as_int() {
        return format!("i64:{value}");
    }
    if let Ok(value) = value.as_float() {
        return format!("f64:{value:.17}");
    }
    if let Some(value) = value.clone().try_cast::<bool>() {
        return format!("bool:{value}");
    }
    if let Some(value) = value.clone().try_cast::<rhai::ImmutableString>() {
        return format!("string:{value}");
    }
    value.type_name().to_owned()
}

#[cfg(feature = "roto")]
enum RotoFunction {
    I64(roto::TypedFunc<roto::NoCtx, fn(u64) -> i64>),
    U64(roto::TypedFunc<roto::NoCtx, fn(u64) -> u64>),
    F64(roto::TypedFunc<roto::NoCtx, fn(u64) -> f64>),
}

#[cfg(feature = "roto")]
struct RotoPrepared {
    function: RotoFunction,
}

#[cfg(feature = "roto")]
impl RotoPrepared {
    fn new(workload: &Workload, source: &str) -> Result<Self> {
        let runtime = roto::Runtime::new();
        let mut package = roto::FileTree::test_file(workload.id, source, 0)
            .compile(&runtime)
            .map_err(|error| Error::new(format!("compile {} failed: {error}", workload.id)))?;
        let function = match workload.id {
            "stdlib_math_table" => RotoFunction::F64(
                package
                    .get_function::<fn(u64) -> f64>("f")
                    .map_err(|error| {
                        Error::new(format!("get roto function {} failed: {error}", workload.id))
                    })?,
            ),
            "string_building" => RotoFunction::U64(
                package
                    .get_function::<fn(u64) -> u64>("f")
                    .map_err(|error| {
                        Error::new(format!("get roto function {} failed: {error}", workload.id))
                    })?,
            ),
            _ => RotoFunction::I64(package.get_function::<fn(u64) -> i64>("f").map_err(
                |error| Error::new(format!("get roto function {} failed: {error}", workload.id)),
            )?),
        };
        Ok(Self { function })
    }
}

#[cfg(feature = "roto")]
impl PreparedWorkload for RotoPrepared {
    fn run(&mut self, n: u64) -> Result<String> {
        match &self.function {
            RotoFunction::I64(function) => Ok(format!("i64:{}", function.call(n))),
            RotoFunction::U64(function) => Ok(format!("u64:{}", function.call(n))),
            RotoFunction::F64(function) => Ok(format!("f64:{:.17}", function.call(n))),
        }
    }
}

#[cfg(feature = "rquickjs-engine")]
struct RquickJsPrepared {
    function: rquickjs::Persistent<rquickjs::Function<'static>>,
    context: rquickjs::Context,
    _runtime: rquickjs::Runtime,
}

#[cfg(feature = "rquickjs-engine")]
impl RquickJsPrepared {
    fn new(workload: &Workload, source: &str) -> Result<Self> {
        let runtime = rquickjs::Runtime::new()
            .map_err(|error| Error::new(format!("create rquickjs runtime failed: {error}")))?;
        let context = rquickjs::Context::full(&runtime)
            .map_err(|error| Error::new(format!("create rquickjs context failed: {error}")))?;
        let function = context.with(|ctx| {
            let function = ctx.eval::<rquickjs::Function, _>(source).map_err(|error| {
                Error::new(format!("initialize {} failed: {error}", workload.id))
            })?;
            Ok(rquickjs::Persistent::save(&ctx, function))
        })?;
        Ok(Self {
            function,
            context,
            _runtime: runtime,
        })
    }
}

#[cfg(feature = "rquickjs-engine")]
impl PreparedWorkload for RquickJsPrepared {
    fn run(&mut self, n: u64) -> Result<String> {
        self.context.with(|ctx| {
            let function = self
                .function
                .clone()
                .restore(&ctx)
                .map_err(|error| Error::new(format!("rquickjs restore failed: {error}")))?;
            let result = function
                .call::<_, rquickjs::Value>((n as f64,))
                .map_err(|error| Error::new(format!("rquickjs workload call failed: {error}")))?;
            rquickjs_checksum(&result)
        })
    }
}

#[cfg(feature = "rquickjs-engine")]
fn rquickjs_checksum(value: &rquickjs::Value<'_>) -> Result<String> {
    if value.is_undefined() {
        return Ok("undefined".to_owned());
    }
    if value.is_null() {
        return Ok("null".to_owned());
    }
    if let Some(value) = value.as_bool() {
        return Ok(format!("bool:{value}"));
    }
    if let Some(value) = value.as_number() {
        return Ok(format!("f64:{value:.17}"));
    }
    if let Some(value) = value.as_string() {
        return value
            .to_string()
            .map(|text| format!("string:{text}"))
            .map_err(|error| Error::new(format!("rquickjs string checksum failed: {error}")));
    }
    Ok(value.type_name().to_owned())
}

#[cfg(feature = "wasmi-engine")]
struct WasmiPrepared {
    store: wasmi::Store<()>,
    function: wasmi::TypedFunc<i64, i64>,
}

#[cfg(feature = "wasmi-engine")]
impl WasmiPrepared {
    fn new(workload: &Workload, source: &str) -> Result<Self> {
        let engine = wasmi::Engine::default();
        let module = wasmi::Module::new(&engine, source)
            .map_err(|error| Error::new(format!("compile {} failed: {error}", workload.id)))?;
        let mut store = wasmi::Store::new(&engine, ());
        let linker = wasmi::Linker::new(&engine);
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|error| Error::new(format!("instantiate {} failed: {error}", workload.id)))?;
        let function = instance
            .get_typed_func::<i64, i64>(&mut store, "f")
            .map_err(|error| {
                Error::new(format!(
                    "wasmi workload {} must export f(i64) -> i64: {error}",
                    workload.id
                ))
            })?;
        Ok(Self { store, function })
    }
}

#[cfg(feature = "wasmi-engine")]
impl PreparedWorkload for WasmiPrepared {
    fn run(&mut self, n: u64) -> Result<String> {
        let n = i64::try_from(n).map_err(|_| Error::new("wasmi n exceeded i64::MAX"))?;
        let result = self
            .function
            .call(&mut self.store, n)
            .map_err(|error| Error::new(format!("wasmi workload call failed: {error}")))?;
        Ok(format!("i64:{result}"))
    }
}

#[cfg(feature = "wasmtime-engine")]
struct WasmtimePrepared {
    store: wasmtime::Store<()>,
    function: wasmtime::TypedFunc<i64, i64>,
}

#[cfg(feature = "wasmtime-engine")]
impl WasmtimePrepared {
    fn new(workload: &Workload, source: &str) -> Result<Self> {
        let engine = wasmtime::Engine::default();
        let module = wasmtime::Module::new(&engine, source)
            .map_err(|error| Error::new(format!("compile {} failed: {error}", workload.id)))?;
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[])
            .map_err(|error| Error::new(format!("instantiate {} failed: {error}", workload.id)))?;
        let function = instance
            .get_typed_func::<i64, i64>(&mut store, "f")
            .map_err(|error| {
                Error::new(format!(
                    "wasmtime workload {} must export f(i64) -> i64: {error}",
                    workload.id
                ))
            })?;
        Ok(Self { store, function })
    }
}

#[cfg(feature = "wasmtime-engine")]
impl PreparedWorkload for WasmtimePrepared {
    fn run(&mut self, n: u64) -> Result<String> {
        let n = i64::try_from(n).map_err(|_| Error::new("wasmtime n exceeded i64::MAX"))?;
        let result = self
            .function
            .call(&mut self.store, n)
            .map_err(|error| Error::new(format!("wasmtime workload call failed: {error}")))?;
        Ok(format!("i64:{result}"))
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

fn write_results(path: &Path, history: Option<&Path>, records: &[Measurement]) -> Result<usize> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
    }
    let rows = latest_result_rows(history, records)?;
    let mut text = String::new();
    text.push_str("# script-bench-harness Results\n\n");
    let rustia_baselines = rows
        .iter()
        .filter(|row| row.engine == "rustia")
        .map(|row| {
            (
                (row.target.as_str(), row.workload.as_str()),
                row.ns_per_unit_min,
            )
        })
        .collect::<BTreeMap<_, _>>();
    text.push_str(
        "| target | engine | workload | ns/unit min | ns/unit median | speed vs rustia | units/sec | n | checksum |\n",
    );
    text.push_str("|---|---|---|---:|---:|---:|---:|---:|---|\n");
    for row in &rows {
        let speed_vs_rustia = rustia_baselines
            .get(&(row.target.as_str(), row.workload.as_str()))
            .map(|baseline| format!("{:.2}x", baseline / row.ns_per_unit_min))
            .unwrap_or_else(|| "n/a".to_owned());
        text.push_str(&format!(
            "| {} | {} | {} | {:.2} | {:.2} | {} | {:.0} | {} | `{}` |\n",
            row.target,
            row.engine,
            row.workload,
            row.ns_per_unit_min,
            row.ns_per_unit_median,
            speed_vs_rustia,
            row.units_per_sec,
            row.n,
            row.checksum.replace('|', "\\|")
        ));
    }
    let row_count = rows.len();
    fs::write(path, text).map_err(|error| Error::io(path, error))?;
    Ok(row_count)
}

fn write_latest_json(
    path: &Path,
    history: Option<&Path>,
    records: &[Measurement],
) -> Result<usize> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
    }
    let rows = latest_result_rows(history, records)?;
    let rustia_baselines = rows
        .iter()
        .filter(|row| row.engine == "rustia")
        .map(|row| {
            (
                (row.target.as_str(), row.workload.as_str()),
                row.ns_per_unit_min,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let targets = rows
        .iter()
        .map(|row| row.target.as_str())
        .collect::<BTreeSet<_>>();
    let engines = rows
        .iter()
        .map(|row| row.engine.as_str())
        .collect::<BTreeSet<_>>();
    let workloads = rows
        .iter()
        .map(|row| row.workload.as_str())
        .collect::<BTreeSet<_>>();
    let commits = rows
        .iter()
        .map(|row| row.commit.as_str())
        .collect::<BTreeSet<_>>();
    let first_timestamp_unix = rows.iter().map(|row| row.timestamp_unix).min().unwrap_or(0);
    let last_timestamp_unix = rows.iter().map(|row| row.timestamp_unix).max().unwrap_or(0);
    let row_values = rows
        .iter()
        .map(|row| {
            let speed_vs_rustia = rustia_baselines
                .get(&(row.target.as_str(), row.workload.as_str()))
                .map(|baseline| baseline / row.ns_per_unit_min);
            serde_json::json!({
                "timestamp_unix": row.timestamp_unix,
                "commit": row.commit,
                "target": row.target,
                "engine": row.engine,
                "dialect": row.dialect,
                "workload": row.workload,
                "unit": row.unit,
                "n": row.n,
                "ns_per_unit_min": row.ns_per_unit_min,
                "ns_per_unit_median": row.ns_per_unit_median,
                "units_per_sec": row.units_per_sec,
                "checksum": row.checksum,
                "speed_vs_rustia": speed_vs_rustia,
            })
        })
        .collect::<Vec<_>>();
    let value = serde_json::json!({
        "schema_version": 1,
        "schema": "rustia.bench.current-results.v1",
        "source": "script-bench-harness",
        "metadata": {
            "description": "Current benchmark snapshot generated from committed history; no benchmark measurements are run by the renderer.",
            "history": history
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "in-memory measurements".to_owned()),
            "history_schema": "jsonl-v1",
            "selection": "latest available result per target, engine, and workload in the source history",
            "row_count": rows.len(),
            "targets": targets.into_iter().collect::<Vec<_>>(),
            "engines": engines.into_iter().collect::<Vec<_>>(),
            "workloads": workloads.into_iter().collect::<Vec<_>>(),
            "commits": commits.into_iter().collect::<Vec<_>>(),
            "measurement_window_unix": {
                "start": first_timestamp_unix,
                "end": last_timestamp_unix,
            },
        },
        "rows": row_values,
    });
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| Error::new(format!("latest JSON failed: {error}")))?;
    fs::write(path, format!("{text}\n")).map_err(|error| Error::io(path, error))?;
    Ok(rows.len())
}

fn write_samples_json(path: &Path, records: &[Measurement]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
    }
    let measurements = records
        .iter()
        .map(|record| {
            serde_json::json!({
                "timestamp_unix": record.timestamp_unix,
                "commit": record.commit,
                "target": record.target,
                "engine": record.engine,
                "dialect": record.dialect,
                "workload": record.workload,
                "unit": record.unit,
                "n": record.n,
                "n_selection": record.n_selection.as_str(),
                "reps": record.reps,
                "ns_per_unit_min": record.ns_per_unit_min,
                "ns_per_unit_median": record.ns_per_unit_median,
                "units_per_sec": record.units_per_sec,
                "checksum": record.checksum,
                "samples_ns_per_unit": record.samples_ns_per_unit,
            })
        })
        .collect::<Vec<_>>();
    let sample_count = records
        .iter()
        .map(|record| record.samples_ns_per_unit.len())
        .sum::<usize>();
    let value = serde_json::json!({
        "schema_version": 1,
        "schema": "rustia.bench.raw-samples.v1",
        "source": "script-bench-harness",
        "metadata": {
            "description": "Raw per-repetition timings in execution order. This sidecar is separate from the append-only public history schema.",
            "measurement_isolation": "fresh prepared workload per repetition",
            "warmup": "one untimed invocation immediately before each recorded sample",
            "sample_order": "execution order (unsorted)",
            "measurement_count": records.len(),
            "sample_count": sample_count,
        },
        "measurements": measurements,
    });
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| Error::new(format!("samples JSON failed: {error}")))?;
    fs::write(path, format!("{text}\n")).map_err(|error| Error::io(path, error))?;
    Ok(())
}

fn latest_result_rows(history: Option<&Path>, records: &[Measurement]) -> Result<Vec<ResultRow>> {
    let mut latest = BTreeMap::<(String, String, String), ResultRow>::new();
    if let Some(history) = history {
        for record in read_history_records(history)? {
            let row = ResultRow {
                timestamp_unix: record.timestamp_unix,
                commit: record.commit,
                target: record.target,
                engine: record.engine,
                dialect: record.dialect,
                workload: record.workload,
                unit: record.unit,
                n: record.n,
                ns_per_unit_min: record.ns_per_unit_min,
                ns_per_unit_median: record.ns_per_unit_median,
                units_per_sec: record.units_per_sec,
                checksum: record.checksum,
            };
            latest.insert(
                (row.target.clone(), row.engine.clone(), row.workload.clone()),
                row,
            );
        }
    }
    for record in records {
        let row = ResultRow::from_measurement(record);
        latest.insert(
            (row.target.clone(), row.engine.clone(), row.workload.clone()),
            row,
        );
    }

    let mut rows = latest.into_values().collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.target
            .cmp(&right.target)
            .then_with(|| engine_order(&left.engine).cmp(&engine_order(&right.engine)))
            .then_with(|| left.engine.cmp(&right.engine))
            .then_with(|| workload_order(&left.workload).cmp(&workload_order(&right.workload)))
            .then_with(|| left.workload.cmp(&right.workload))
    });
    Ok(rows)
}

fn check_history(path: &Path) -> Result<HistorySummary> {
    let records = read_history_records(path)?;
    if records.is_empty() {
        return Err(Error::new(format!(
            "{}: history must contain at least one record",
            path.display()
        )));
    }

    let mut engines = BTreeSet::new();
    let mut workloads = BTreeSet::new();
    let mut targets = BTreeSet::new();
    let mut first_timestamp_unix = u64::MAX;
    let mut last_timestamp_unix = 0;
    for record in &records {
        engines.insert(record.engine.clone());
        workloads.insert(record.workload.clone());
        targets.insert(record.target.clone());
        first_timestamp_unix = first_timestamp_unix.min(record.timestamp_unix);
        last_timestamp_unix = last_timestamp_unix.max(record.timestamp_unix);
    }

    Ok(HistorySummary {
        records: records.len(),
        engines,
        workloads,
        targets,
        first_timestamp_unix,
        last_timestamp_unix,
    })
}

fn read_history_records(path: &Path) -> Result<Vec<HistoryRecord>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::io(path, error)),
    };

    let mut records = Vec::new();
    let mut previous_timestamp = None;
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
            Error::new(format!(
                "{} line {line_number}: invalid JSON: {error}",
                path.display()
            ))
        })?;
        let record = history_record_from_json(path, line_number, &value)?;
        if let Some(previous_timestamp) = previous_timestamp {
            if record.timestamp_unix < previous_timestamp {
                return Err(Error::new(format!(
                    "{} line {line_number}: timestamp_unix must be append-only; {timestamp} is before previous {previous_timestamp}",
                    path.display(),
                    timestamp = record.timestamp_unix
                )));
            }
        }
        previous_timestamp = Some(record.timestamp_unix);
        records.push(record);
    }
    Ok(records)
}

fn history_record_from_json(
    path: &Path,
    line_number: usize,
    value: &serde_json::Value,
) -> Result<HistoryRecord> {
    let timestamp_unix = json_u64(path, line_number, value, "timestamp_unix")?;
    let commit = json_non_empty_string(path, line_number, value, "commit")?;
    if commit.ends_with("-dirty") {
        return Err(Error::new(format!(
            "{} line {line_number}: committed history must not use dirty commit id {commit:?}",
            path.display()
        )));
    }
    let target = json_non_empty_string(path, line_number, value, "target")?;
    let engine = json_non_empty_string(path, line_number, value, "engine")?;
    let dialect = json_non_empty_string(path, line_number, value, "dialect")?;
    let workload = json_non_empty_string(path, line_number, value, "workload")?;
    let unit = json_non_empty_string(path, line_number, value, "unit")?;
    let n = json_positive_u64(path, line_number, value, "n")?;
    let _reps = json_positive_u64(path, line_number, value, "reps")?;
    let ns_per_unit_min = json_positive_f64(path, line_number, value, "ns_per_unit_min")?;
    let ns_per_unit_median = json_positive_f64(path, line_number, value, "ns_per_unit_median")?;
    let units_per_sec = json_positive_f64(path, line_number, value, "units_per_sec")?;
    let checksum = json_non_empty_string(path, line_number, value, "checksum")?;

    let engine_slot = ENGINE_SLOTS
        .iter()
        .find(|slot| slot.id == engine)
        .ok_or_else(|| {
            Error::new(format!(
                "{} line {line_number}: unknown engine {engine:?}",
                path.display()
            ))
        })?;
    if engine_slot.dialect.as_str() != dialect {
        return Err(Error::new(format!(
            "{} line {line_number}: engine {engine:?} uses dialect {:?}, got {dialect:?}",
            path.display(),
            engine_slot.dialect.as_str()
        )));
    }

    let workload_spec = WORKLOADS
        .iter()
        .find(|entry| entry.id == workload)
        .ok_or_else(|| {
            Error::new(format!(
                "{} line {line_number}: unknown workload {workload:?}",
                path.display()
            ))
        })?;
    if workload_spec.unit != unit {
        return Err(Error::new(format!(
            "{} line {line_number}: workload {workload:?} uses unit {:?}, got {unit:?}",
            path.display(),
            workload_spec.unit
        )));
    }
    if ns_per_unit_median < ns_per_unit_min {
        return Err(Error::new(format!(
            "{} line {line_number}: ns_per_unit_median must be >= ns_per_unit_min",
            path.display()
        )));
    }

    Ok(HistoryRecord {
        timestamp_unix,
        commit,
        target,
        engine,
        dialect,
        workload,
        unit,
        n,
        ns_per_unit_min,
        ns_per_unit_median,
        units_per_sec,
        checksum,
    })
}

fn json_string(
    path: &Path,
    line_number: usize,
    value: &serde_json::Value,
    field: &str,
) -> Result<String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| history_field_error(path, line_number, field, "string"))
}

fn json_non_empty_string(
    path: &Path,
    line_number: usize,
    value: &serde_json::Value,
    field: &str,
) -> Result<String> {
    let text = json_string(path, line_number, value, field)?;
    if text.is_empty() {
        return Err(history_field_error(
            path,
            line_number,
            field,
            "non-empty string",
        ));
    }
    Ok(text)
}

fn json_u64(
    path: &Path,
    line_number: usize,
    value: &serde_json::Value,
    field: &str,
) -> Result<u64> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| history_field_error(path, line_number, field, "u64"))
}

fn json_positive_u64(
    path: &Path,
    line_number: usize,
    value: &serde_json::Value,
    field: &str,
) -> Result<u64> {
    let number = json_u64(path, line_number, value, field)?;
    if number == 0 {
        return Err(history_field_error(
            path,
            line_number,
            field,
            "positive u64",
        ));
    }
    Ok(number)
}

fn json_f64(
    path: &Path,
    line_number: usize,
    value: &serde_json::Value,
    field: &str,
) -> Result<f64> {
    value
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| history_field_error(path, line_number, field, "finite number"))
}

fn json_positive_f64(
    path: &Path,
    line_number: usize,
    value: &serde_json::Value,
    field: &str,
) -> Result<f64> {
    let number = json_f64(path, line_number, value, field)?;
    if number <= 0.0 {
        return Err(history_field_error(
            path,
            line_number,
            field,
            "positive finite number",
        ));
    }
    Ok(number)
}

fn history_field_error(path: &Path, line_number: usize, field: &str, expected: &str) -> Error {
    Error::new(format!(
        "{} line {line_number}: expected {field:?} to be a {expected}",
        path.display()
    ))
}

fn engine_order(engine: &str) -> usize {
    ENGINE_SLOTS
        .iter()
        .position(|slot| slot.id == engine)
        .unwrap_or(usize::MAX)
}

fn workload_order(workload: &str) -> usize {
    WORKLOADS
        .iter()
        .position(|entry| entry.id == workload)
        .unwrap_or(usize::MAX)
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
    let commit = Command::new("git")
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
        .unwrap_or_else(|| "unknown".to_owned());
    if commit != "unknown" && git_worktree_dirty() {
        format!("{commit}-dirty")
    } else {
        commit
    }
}

fn git_worktree_dirty() -> bool {
    let unstaged_dirty = Command::new("git")
        .args(["diff", "--quiet", "--ignore-submodules", "--"])
        .status()
        .map(|status| !status.success())
        .unwrap_or(false);
    let staged_dirty = Command::new("git")
        .args(["diff", "--cached", "--quiet", "--ignore-submodules", "--"])
        .status()
        .map(|status| !status.success())
        .unwrap_or(false);
    unstaged_dirty || staged_dirty
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    struct RecordingPrepared {
        id: usize,
        run_count: usize,
        runs: Rc<RefCell<Vec<(usize, usize, u64)>>>,
    }

    impl PreparedWorkload for RecordingPrepared {
        fn run(&mut self, n: u64) -> Result<String> {
            self.run_count += 1;
            self.runs.borrow_mut().push((self.id, self.run_count, n));
            Ok(format!("instance:{}:run:{}", self.id, self.run_count))
        }
    }

    #[test]
    fn inventory_json_reports_source_implemented_slots_independent_of_features() {
        let value = inventory_json_value();
        let engines = value
            .get("engines")
            .and_then(serde_json::Value::as_array)
            .expect("engines array");

        assert_eq!(engines.len(), 10);
        for engine in engines {
            let id = engine
                .get("id")
                .and_then(serde_json::Value::as_str)
                .expect("engine id");
            let enabled = engine
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .expect("engine enabled");
            let implemented = engine
                .get("implemented")
                .and_then(serde_json::Value::as_bool)
                .expect("engine implemented");
            let available = engine
                .get("available")
                .and_then(serde_json::Value::as_bool)
                .expect("engine available");
            let status = engine
                .get("status")
                .and_then(serde_json::Value::as_str)
                .expect("engine status");

            assert!(
                implemented,
                "{id} should report the source adapter as implemented"
            );
            assert_eq!(
                available, enabled,
                "{id} availability should track feature enablement"
            );
            if available {
                assert_eq!(status, "available", "{id} status");
            } else {
                assert!(
                    status.starts_with("compile with --features "),
                    "{id} disabled status should name the feature, got {status:?}"
                );
            }
        }
    }

    #[test]
    fn inventory_json_preserves_workload_schema_and_defaults() {
        let value = inventory_json_value();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["binary"], "script-bench");
        assert_eq!(value["history_schema"], "jsonl-v1");
        assert_eq!(value["default_history"], HISTORY_PATH);
        assert_eq!(value["default_results"], RESULTS_PATH);
        assert_eq!(
            value["default_current_results_json"],
            CURRENT_RESULTS_JSON_PATH
        );
        assert_eq!(value["default_latest_json"], CURRENT_RESULTS_JSON_PATH);

        let workloads = value
            .get("workloads")
            .and_then(serde_json::Value::as_array)
            .expect("workloads array");
        assert_eq!(workloads.len(), 12);

        let defaults = workloads
            .iter()
            .filter(|workload| workload["default"].as_bool() == Some(true))
            .map(|workload| workload["id"].as_str().expect("workload id"))
            .collect::<Vec<_>>();
        assert_eq!(
            defaults,
            [
                "dense_tables",
                "sparse_tables",
                "closures_upvalues",
                "generic_iteration",
                "stdlib_math_table"
            ]
        );
    }

    #[test]
    fn default_workloads_have_sources_for_all_benchmark_dialects() {
        let repo = repo_root().expect("repo root");
        for workload in WORKLOADS.iter().filter(|workload| workload.default) {
            assert_sources_for_all_benchmark_dialects(&repo, workload);
        }
    }

    #[test]
    fn string_building_has_sources_for_all_benchmark_dialects() {
        let repo = repo_root().expect("repo root");
        let workload = WORKLOADS
            .iter()
            .find(|entry| entry.id == "string_building")
            .expect("string_building workload");

        assert!(
            !workload.default,
            "string_building should stay opt-in until it has stable public baselines"
        );
        assert_sources_for_all_benchmark_dialects(&repo, workload);
    }

    #[test]
    fn check_history_accepts_valid_jsonl_and_summarizes_it() {
        let path = write_temp_history(
            "valid",
            &format!(
                "{}\n{}\n",
                sample_history_json(100, "rustia", "luau", "dense_tables"),
                sample_history_json(101, "koto", "koto", "sparse_tables")
            ),
        );

        let summary = check_history(&path).expect("valid history");
        let _ = fs::remove_file(&path);

        assert_eq!(summary.records, 2);
        assert_eq!(
            summary.engines,
            BTreeSet::from(["koto".to_owned(), "rustia".to_owned()])
        );
        assert_eq!(
            summary.workloads,
            BTreeSet::from(["dense_tables".to_owned(), "sparse_tables".to_owned()])
        );
        assert_eq!(summary.targets, BTreeSet::from(["test-target".to_owned()]));
        assert_eq!(summary.first_timestamp_unix, 100);
        assert_eq!(summary.last_timestamp_unix, 101);
    }

    #[test]
    fn check_history_rejects_unknown_engine() {
        let path = write_temp_history(
            "unknown-engine",
            &format!(
                "{}\n",
                sample_history_json(100, "missing-engine", "luau", "dense_tables")
            ),
        );

        let error = check_history(&path).expect_err("unknown engine should fail");
        let _ = fs::remove_file(&path);

        assert!(error.to_string().contains("unknown engine"));
    }

    #[test]
    fn check_history_rejects_non_append_only_timestamps() {
        let path = write_temp_history(
            "timestamp-order",
            &format!(
                "{}\n{}\n",
                sample_history_json(200, "rustia", "luau", "dense_tables"),
                sample_history_json(199, "rustia", "luau", "sparse_tables")
            ),
        );

        let error = check_history(&path).expect_err("out-of-order timestamp should fail");
        let _ = fs::remove_file(&path);

        assert!(error.to_string().contains("append-only"));
    }

    #[test]
    fn check_history_rejects_dirty_commit_records() {
        let path = write_temp_history(
            "dirty-commit",
            &format!(
                "{}\n",
                sample_history_json_with_commit(
                    100,
                    "rustia",
                    "luau",
                    "dense_tables",
                    "abc1234-dirty"
                )
            ),
        );

        let error = check_history(&path).expect_err("dirty commit should fail");
        let _ = fs::remove_file(&path);

        assert!(error.to_string().contains("dirty commit id"));
    }

    #[test]
    fn parse_args_rejects_dirty_commit_override() {
        let error = parse_args(["--commit".to_owned(), "abc1234-dirty".to_owned()].into_iter())
            .expect_err("dirty commit override should fail");

        assert!(error.to_string().contains("clean benchmark baseline"));
    }

    #[test]
    fn parse_args_accepts_optional_raw_samples_path() {
        let config = parse_args(
            [
                "--samples-json".to_owned(),
                "tmp/raw-samples.json".to_owned(),
            ]
            .into_iter(),
        )
        .expect("samples path");

        assert_eq!(
            config.samples_json,
            Some(PathBuf::from("tmp/raw-samples.json"))
        );
    }

    #[test]
    fn parse_args_accepts_positive_fixed_n_and_rejects_zero() {
        let config =
            parse_args(["--fixed-n".to_owned(), "1500".to_owned()].into_iter()).expect("fixed n");
        assert_eq!(config.fixed_n, Some(1500));

        let error = parse_args(["--fixed-n".to_owned(), "0".to_owned()].into_iter())
            .expect_err("zero fixed n should fail");
        assert!(error.to_string().contains("greater than zero"));
    }

    #[test]
    fn fixed_n_history_requires_raw_evidence_but_diagnostics_do_not() {
        let publication = Config {
            fixed_n: Some(1500),
            ..Config::default()
        };
        let error = validate_measurement_outputs(&publication)
            .expect_err("fixed history without samples must be rejected");
        assert!(error.to_string().contains("requires --samples-json"));

        let with_evidence = Config {
            samples_json: Some(PathBuf::from("raw.json")),
            ..publication
        };
        validate_measurement_outputs(&with_evidence).expect("raw evidence enables publication");

        let diagnostic = Config {
            fixed_n: Some(1500),
            history: None,
            ..Config::default()
        };
        validate_measurement_outputs(&diagnostic).expect("no-history diagnostics need no sidecar");
    }

    #[test]
    fn each_recorded_sample_uses_fresh_preparation_after_an_untimed_warmup() {
        let config = Config {
            target: Duration::ZERO,
            max_n: 1,
            reps: 3,
            commit: Some("test".to_owned()),
            ..Config::default()
        };
        let next_id = Rc::new(Cell::new(0_usize));
        let runs = Rc::new(RefCell::new(Vec::new()));
        let measurement = measure_with_factory(&config, &ENGINE_SLOTS[0], &WORKLOADS[0], {
            let next_id = Rc::clone(&next_id);
            let runs = Rc::clone(&runs);
            move || {
                let id = next_id.get();
                next_id.set(id + 1);
                Ok(Box::new(RecordingPrepared {
                    id,
                    run_count: 0,
                    runs: Rc::clone(&runs),
                }) as Box<dyn PreparedWorkload>)
            }
        })
        .expect("measurement");

        assert_eq!(next_id.get(), 4, "one calibration plus three samples");
        assert_eq!(
            runs.borrow().as_slice(),
            &[
                (0, 1, 1),
                (0, 2, 1),
                (1, 1, 1),
                (1, 2, 1),
                (2, 1, 1),
                (2, 2, 1),
                (3, 1, 1),
                (3, 2, 1),
            ]
        );
        assert_eq!(measurement.samples_ns_per_unit.len(), 3);
        assert_eq!(measurement.n_selection, NSelection::Calibrated);
        assert_eq!(measurement.checksum, "instance:3:run:2");
    }

    #[test]
    fn fixed_n_bypasses_calibration_and_uses_equal_work_for_every_sample() {
        let config = Config {
            fixed_n: Some(1500),
            max_n: 1,
            reps: 3,
            commit: Some("test".to_owned()),
            ..Config::default()
        };
        let next_id = Rc::new(Cell::new(0_usize));
        let runs = Rc::new(RefCell::new(Vec::new()));
        let measurement = measure_with_factory(&config, &ENGINE_SLOTS[0], &WORKLOADS[0], {
            let next_id = Rc::clone(&next_id);
            let runs = Rc::clone(&runs);
            move || {
                let id = next_id.get();
                next_id.set(id + 1);
                Ok(Box::new(RecordingPrepared {
                    id,
                    run_count: 0,
                    runs: Rc::clone(&runs),
                }) as Box<dyn PreparedWorkload>)
            }
        })
        .expect("fixed measurement");

        assert_eq!(
            next_id.get(),
            3,
            "fixed mode must not prepare calibration state"
        );
        assert_eq!(
            runs.borrow().as_slice(),
            &[
                (0, 1, 1500),
                (0, 2, 1500),
                (1, 1, 1500),
                (1, 2, 1500),
                (2, 1, 1500),
                (2, 2, 1500),
            ]
        );
        assert_eq!(measurement.n, 1500);
        assert_eq!(measurement.n_selection, NSelection::Fixed);
        assert_eq!(measurement.samples_ns_per_unit.len(), 3);
    }

    #[test]
    fn write_results_can_refresh_from_history_without_new_measurements() {
        let history = write_temp_history(
            "refresh-results-history",
            &format!(
                "{}\n{}\n",
                sample_history_json(100, "rustia", "luau", "dense_tables"),
                sample_history_json(101, "koto", "koto", "sparse_tables")
            ),
        );
        let results = temp_path("refresh-results", "md");

        let row_count = write_results(&results, Some(&history), &[]).expect("write results");
        let text = fs::read_to_string(&results).expect("read results");
        let _ = fs::remove_file(&history);
        let _ = fs::remove_file(&results);

        assert_eq!(row_count, 2);
        assert!(text.contains("| test-target | rustia | dense_tables |"));
        assert!(text.contains("| test-target | koto | sparse_tables |"));
    }

    #[test]
    fn write_latest_json_refreshes_target_aware_rows_from_history() {
        let history = write_temp_history(
            "latest-json-history",
            &format!(
                "{}\n{}\n",
                sample_history_json(100, "rustia", "luau", "dense_tables"),
                sample_history_json(101, "koto", "koto", "sparse_tables")
            ),
        );
        let latest = temp_path("latest-json", "json");

        let row_count = write_latest_json(&latest, Some(&history), &[]).expect("write latest JSON");
        let text = fs::read_to_string(&latest).expect("read latest JSON");
        let value = serde_json::from_str::<serde_json::Value>(&text).expect("parse latest JSON");
        let _ = fs::remove_file(&history);
        let _ = fs::remove_file(&latest);

        assert_eq!(row_count, 2);
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["schema"], "rustia.bench.current-results.v1");
        assert_eq!(value["metadata"]["row_count"], 2);
        assert_eq!(value["metadata"]["targets"][0], "test-target");
        assert_eq!(value["metadata"]["commits"][0], "test");
        assert_eq!(value["metadata"]["measurement_window_unix"]["start"], 100);
        assert_eq!(value["metadata"]["measurement_window_unix"]["end"], 101);
        assert_eq!(value["rows"][0]["target"], "test-target");
        assert_eq!(value["rows"][0]["engine"], "rustia");
        assert_eq!(value["rows"][0]["speed_vs_rustia"].as_f64(), Some(1.0));
    }

    #[test]
    fn write_samples_json_preserves_execution_order_without_changing_history_records() {
        let samples = temp_path("raw-samples", "json");
        let mut measurement = sample_measurement(100, "rustia", "luau", "dense_tables", "test");
        measurement.n = 1500;
        measurement.n_selection = NSelection::Fixed;

        write_samples_json(&samples, std::slice::from_ref(&measurement))
            .expect("write samples JSON");
        let text = fs::read_to_string(&samples).expect("read samples JSON");
        let value = serde_json::from_str::<serde_json::Value>(&text).expect("parse samples JSON");
        let _ = fs::remove_file(&samples);

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["schema"], "rustia.bench.raw-samples.v1");
        assert_eq!(value["metadata"]["measurement_count"], 1);
        assert_eq!(value["metadata"]["sample_count"], 3);
        assert_eq!(value["measurements"][0]["n"], 1500);
        assert_eq!(value["measurements"][0]["n_selection"], "fixed");
        assert_eq!(
            value["measurements"][0]["samples_ns_per_unit"],
            serde_json::json!([3.0, 1.0, 2.0])
        );
        assert!(
            !record_json(&measurement).contains("samples_ns_per_unit"),
            "the append-only public history schema must remain unchanged"
        );
        assert!(
            !record_json(&measurement).contains("n_selection"),
            "fixed-N provenance must stay out of the public history schema"
        );
    }

    fn sample_history_json(
        timestamp_unix: u64,
        engine: &'static str,
        dialect: &'static str,
        workload: &'static str,
    ) -> String {
        sample_history_json_with_commit(timestamp_unix, engine, dialect, workload, "test")
    }

    fn sample_history_json_with_commit(
        timestamp_unix: u64,
        engine: &'static str,
        dialect: &'static str,
        workload: &'static str,
        commit: &'static str,
    ) -> String {
        record_json(&sample_measurement(
            timestamp_unix,
            engine,
            dialect,
            workload,
            commit,
        ))
    }

    fn sample_measurement(
        timestamp_unix: u64,
        engine: &'static str,
        dialect: &'static str,
        workload: &'static str,
        commit: &'static str,
    ) -> Measurement {
        let unit = WORKLOADS
            .iter()
            .find(|entry| entry.id == workload)
            .expect("known workload")
            .unit;
        Measurement {
            engine: engine.to_owned(),
            dialect,
            workload: workload.to_owned(),
            unit,
            n: 10,
            n_selection: NSelection::Calibrated,
            reps: 3,
            ns_per_unit_min: 1.0,
            ns_per_unit_median: 1.5,
            samples_ns_per_unit: vec![3.0, 1.0, 2.0],
            units_per_sec: 1_000_000_000.0,
            checksum: "i64:42".to_owned(),
            commit: commit.to_owned(),
            timestamp_unix,
            target: "test-target".to_owned(),
        }
    }

    fn assert_sources_for_all_benchmark_dialects(repo: &Path, workload: &Workload) {
        for dialect in [
            Dialect::Luau,
            Dialect::Koto,
            Dialect::Lua54,
            Dialect::JavaScript,
            Dialect::Rhai,
            Dialect::Roto,
            Dialect::Wasm,
        ] {
            assert!(
                workload_path(repo, dialect, workload).is_some(),
                "missing {} source for workload {}",
                dialect.as_str(),
                workload.id
            );
        }
    }

    fn write_temp_history(name: &str, text: &str) -> PathBuf {
        let path = temp_path(name, "jsonl");
        fs::write(&path, text).expect("write temp history");
        path
    }

    fn temp_path(name: &str, extension: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        env::temp_dir().join(format!(
            "script-bench-{name}-{}-{unique}.{extension}",
            std::process::id()
        ))
    }
}
