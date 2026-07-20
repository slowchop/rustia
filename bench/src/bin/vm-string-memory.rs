#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::env;
use std::mem::size_of;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(any(target_os = "linux", test))]
use std::{fs, hint::black_box, time::Instant};

use rustia_vm::{Constant, Function, Instruction, Opcode, Program, Value, Vm};
#[cfg(any(target_os = "linux", test))]
use rustia_vm::{Profile, ValueKind};

const SCHEMA: &str = "rustia.bench.vm-string-sample.v1";
const CONSTANT_SLOTS: usize = 1_024;
const INSTRUCTION_SLOTS: usize = CONSTANT_SLOTS + 1;
const SHORT_STRING_COUNT: usize = CONSTANT_SLOTS;
const SHORT_STRING_BYTES: usize = 13;
const LONG_STRING_COUNT: usize = 128;
const LONG_STRING_BYTES: usize = 2_048;
const MEMORY_PROBE_WARMUPS: usize = 2;
const MAX_VM_COUNT: usize = 2_048;
const FNV1A64_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A64_PRIME: u64 = 0x0000_0100_0000_01b3;

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scenario {
    Empty,
    ShortUnloaded,
    ShortLoaded,
    LongUnloaded,
    LongLoaded,
}

impl Scenario {
    const ALL: [Self; 5] = [
        Self::Empty,
        Self::ShortUnloaded,
        Self::ShortLoaded,
        Self::LongUnloaded,
        Self::LongLoaded,
    ];

    fn parse(value: &str) -> Result<Self> {
        match value {
            "empty" => Ok(Self::Empty),
            "short-unloaded" | "short_unloaded" => Ok(Self::ShortUnloaded),
            "short-loaded" | "short_loaded" => Ok(Self::ShortLoaded),
            "long-unloaded" | "long_unloaded" => Ok(Self::LongUnloaded),
            "long-loaded" | "long_loaded" => Ok(Self::LongLoaded),
            _ => Err(format!(
                "invalid --scenario {value:?}; expected empty, short-unloaded, short-loaded, long-unloaded, or long-loaded"
            )),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::ShortUnloaded => "short_unloaded",
            Self::ShortLoaded => "short_loaded",
            Self::LongUnloaded => "long_unloaded",
            Self::LongLoaded => "long_loaded",
        }
    }

    const fn executes_constants(self) -> bool {
        matches!(self, Self::ShortLoaded | Self::LongLoaded)
    }

    const fn string_kind(self) -> &'static str {
        match self {
            Self::Empty => "none",
            Self::ShortUnloaded | Self::ShortLoaded => "short",
            Self::LongUnloaded | Self::LongLoaded => "long",
        }
    }

    const fn string_count(self) -> usize {
        match self {
            Self::Empty => 0,
            Self::ShortUnloaded | Self::ShortLoaded => SHORT_STRING_COUNT,
            Self::LongUnloaded | Self::LongLoaded => LONG_STRING_COUNT,
        }
    }

    const fn string_bytes(self) -> usize {
        match self {
            Self::Empty => 0,
            Self::ShortUnloaded | Self::ShortLoaded => SHORT_STRING_BYTES,
            Self::LongUnloaded | Self::LongLoaded => LONG_STRING_BYTES,
        }
    }

    const fn nil_constant_count(self) -> usize {
        CONSTANT_SLOTS - self.string_count()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuildRole {
    Baseline,
    Candidate,
}

impl BuildRole {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "baseline" => Ok(Self::Baseline),
            "candidate" => Ok(Self::Candidate),
            _ => Err(format!(
                "invalid --build-role {value:?}; expected baseline or candidate"
            )),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Candidate => "candidate",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Config {
    scenario: Scenario,
    vm_count: usize,
    round: u64,
    sample_id: String,
    build_role: BuildRole,
    commit: String,
    protocol_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExpectedResult {
    Nil,
    String {
        index: usize,
        length: usize,
        multiplier: u8,
        stride: u8,
    },
}

impl ExpectedResult {
    const fn kind(&self) -> &'static str {
        match self {
            Self::Nil => "nil",
            Self::String { .. } => "string",
        }
    }

    fn byte_len(&self) -> Option<usize> {
        match self {
            Self::Nil => None,
            Self::String { length, .. } => Some(*length),
        }
    }

    fn fnv1a64_hex(&self) -> Option<String> {
        match self {
            Self::Nil => None,
            Self::String {
                index,
                length,
                multiplier,
                stride,
            } => {
                let bytes = indexed_bytes(*index, *length, *multiplier, *stride);
                Some(format!("{:016x}", fnv1a64(&bytes)))
            }
        }
    }

    fn bytes(&self) -> Option<Vec<u8>> {
        match self {
            Self::Nil => None,
            Self::String {
                index,
                length,
                multiplier,
                stride,
            } => Some(indexed_bytes(*index, *length, *multiplier, *stride)),
        }
    }
}

struct Fixture {
    program: Program,
    expected_result: ExpectedResult,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct StatusMemory {
    vm_rss: u64,
    vm_hwm: u64,
    rss_anon: u64,
    rss_file: u64,
    rss_shmem: u64,
}

impl StatusMemory {
    fn json(self) -> serde_json::Value {
        serde_json::json!({
            "vm_rss": self.vm_rss,
            "vm_hwm": self.vm_hwm,
            "rss_anon": self.rss_anon,
            "rss_file": self.rss_file,
            "rss_shmem": self.rss_shmem,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SmapsMemory {
    rss: u64,
    pss: u64,
    private_clean: u64,
    private_dirty: u64,
    anonymous: u64,
    swap: u64,
}

impl SmapsMemory {
    fn json(self) -> serde_json::Value {
        serde_json::json!({
            "rss": self.rss,
            "pss": self.pss,
            "private_clean": self.private_clean,
            "private_dirty": self.private_dirty,
            "anonymous": self.anonymous,
            "swap": self.swap,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ProcessMemory {
    status: StatusMemory,
    smaps_rollup: SmapsMemory,
}

impl ProcessMemory {
    fn json(self) -> serde_json::Value {
        serde_json::json!({
            "status": self.status.json(),
            "smaps_rollup": self.smaps_rollup.json(),
        })
    }

    #[cfg(any(target_os = "linux", test))]
    fn require_no_swap(self, label: &str) -> Result<()> {
        if self.smaps_rollup.swap != 0 {
            return Err(format!(
                "{label} memory snapshot reported {} KiB of swap; evidence requires Swap=0",
                self.smaps_rollup.swap
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Timings {
    program_build: u64,
    vm_create: u64,
    program_run: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct MemorySnapshots {
    clean: ProcessMemory,
    program_ready: ProcessMemory,
    vms_ready: ProcessMemory,
    after_run: ProcessMemory,
}

fn main() {
    if env::args().any(|arg| arg == "--help" || arg == "-h") {
        print_usage();
        return;
    }
    if let Err(error) = run() {
        eprintln!("vm-string-memory: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    #[cfg(not(target_os = "linux"))]
    {
        Err("vm-string-memory evidence is supported only on Linux".to_owned())
    }

    #[cfg(target_os = "linux")]
    {
        let config = parse_args(env::args().skip(1))?;
        let value = collect_sample(&config)?;
        let text = serde_json::to_string(&value)
            .map_err(|error| format!("cannot encode canonical sample JSON: {error}"))?;
        println!("{text}");
        Ok(())
    }
}

fn print_usage() {
    println!(
        "usage: vm-string-memory sample --scenario <id> --vm-count <n> --round <n> \\\n         --sample-id <id> --build-role <baseline|candidate> --commit <40-hex> \\\n         --protocol-sha256 <64-hex>"
    );
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Config> {
    let mut args = args.peekable();
    match args.next().as_deref() {
        Some("sample") => {}
        Some(value) => return Err(format!("unknown command {value:?}; expected sample")),
        None => return Err("missing command; expected sample".to_owned()),
    }

    let mut scenario = None;
    let mut vm_count = None;
    let mut round = None;
    let mut sample_id = None;
    let mut build_role = None;
    let mut commit = None;
    let mut protocol_sha256 = None;

    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("{flag} requires a value"))?;
        match flag.as_str() {
            "--scenario" => set_once(&mut scenario, Scenario::parse(&value)?, &flag)?,
            "--vm-count" => {
                let parsed = value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid --vm-count {value:?}"))?;
                if !(1..=MAX_VM_COUNT).contains(&parsed) {
                    return Err(format!("--vm-count must be between 1 and {MAX_VM_COUNT}"));
                }
                set_once(&mut vm_count, parsed, &flag)?;
            }
            "--round" => {
                let parsed = value
                    .parse::<u64>()
                    .map_err(|_| format!("invalid --round {value:?}"))?;
                if parsed == 0 {
                    return Err("--round must be greater than zero".to_owned());
                }
                set_once(&mut round, parsed, &flag)?;
            }
            "--sample-id" => {
                require_safe_id(&value, "--sample-id")?;
                set_once(&mut sample_id, value, &flag)?;
            }
            "--build-role" => set_once(&mut build_role, BuildRole::parse(&value)?, &flag)?,
            "--commit" => {
                require_lower_hex(&value, 40, "--commit")?;
                set_once(&mut commit, value, &flag)?;
            }
            "--protocol-sha256" => {
                require_lower_hex(&value, 64, "--protocol-sha256")?;
                set_once(&mut protocol_sha256, value, &flag)?;
            }
            _ => return Err(format!("unknown argument {flag:?}")),
        }
    }

    Ok(Config {
        scenario: scenario.ok_or_else(|| "missing --scenario".to_owned())?,
        vm_count: vm_count.ok_or_else(|| "missing --vm-count".to_owned())?,
        round: round.ok_or_else(|| "missing --round".to_owned())?,
        sample_id: sample_id.ok_or_else(|| "missing --sample-id".to_owned())?,
        build_role: build_role.ok_or_else(|| "missing --build-role".to_owned())?,
        commit: commit.ok_or_else(|| "missing --commit".to_owned())?,
        protocol_sha256: protocol_sha256.ok_or_else(|| "missing --protocol-sha256".to_owned())?,
    })
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<()> {
    if slot.replace(value).is_some() {
        return Err(format!("{flag} was supplied more than once"));
    }
    Ok(())
}

fn require_safe_id(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(format!(
            "{label} must be 1..128 ASCII letters, digits, '.', '_', or '-'"
        ));
    }
    Ok(())
}

fn require_lower_hex(value: &str, length: usize, label: &str) -> Result<()> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "{label} must be exactly {length} lowercase hexadecimal characters"
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn collect_sample(config: &Config) -> Result<serde_json::Value> {
    for _ in 0..MEMORY_PROBE_WARMUPS {
        black_box(process_memory_snapshot()?);
    }
    let clean = process_memory_snapshot()?;

    let program_started = Instant::now();
    let fixture = build_fixture(config.scenario)?;
    let program_build = elapsed_ns(program_started, "program build")?;
    let program_ready = process_memory_snapshot()?;

    let vm_started = Instant::now();
    let mut vms = Vec::with_capacity(config.vm_count);
    for _ in 0..config.vm_count {
        vms.push(Vm::new(Profile::ReproducibleStandard));
    }
    let vm_create = elapsed_ns(vm_started, "VM creation")?;

    // Pre-touch the validation result lane before the VM-ready snapshot so evidence deltas do
    // not include measurement-infrastructure pages first touched by the program run.
    let mut results = vec![Value::nil(); config.vm_count];
    black_box(&mut results);
    let vms_ready = process_memory_snapshot()?;

    let run_started = Instant::now();
    for (vm, result) in vms.iter_mut().zip(&mut results) {
        *result = vm
            .run(&fixture.program)
            .map_err(|error| format!("VM program run failed: {error}"))?;
    }
    let program_run = elapsed_ns(run_started, "program run")?;
    black_box(&vms);
    black_box(&results);
    let after_run = process_memory_snapshot()?;

    validate_results(&mut vms, &results, &fixture.expected_result)?;

    let snapshots = MemorySnapshots {
        clean,
        program_ready,
        vms_ready,
        after_run,
    };
    for (label, snapshot) in [
        ("clean", snapshots.clean),
        ("program_ready", snapshots.program_ready),
        ("vms_ready", snapshots.vms_ready),
        ("after_run", snapshots.after_run),
    ] {
        snapshot.require_no_swap(label)?;
    }

    Ok(sample_json(
        config,
        &fixture,
        Timings {
            program_build,
            vm_create,
            program_run,
        },
        snapshots,
    ))
}

#[cfg(any(target_os = "linux", test))]
fn elapsed_ns(started: Instant, label: &str) -> Result<u64> {
    u64::try_from(started.elapsed().as_nanos())
        .map_err(|_| format!("{label} duration overflowed u64 nanoseconds"))
}

fn build_fixture(scenario: Scenario) -> Result<Fixture> {
    let (constants, code, expected_result) = fixture_parts(scenario);
    validate_fixture_shape(scenario, &constants, &code, &expected_result)?;
    Ok(Fixture {
        program: Program::new(Function::new(code, 1), constants),
        expected_result,
    })
}

fn validate_fixture_shape(
    scenario: Scenario,
    constants: &[Constant],
    code: &[Instruction],
    expected_result: &ExpectedResult,
) -> Result<()> {
    let string_count = constants
        .iter()
        .filter(|constant| matches!(constant, Constant::String(_)))
        .count();
    let strings_have_expected_length = constants.iter().all(|constant| match constant {
        Constant::String(bytes) => bytes.len() == scenario.string_bytes(),
        _ => true,
    });
    let load_count = code
        .iter()
        .filter(|instruction| instruction.op == Opcode::LoadConstWide)
        .count();
    let return_count = code
        .iter()
        .filter(|instruction| instruction.op == Opcode::Return)
        .count();
    let return_is_positioned = if scenario.executes_constants() {
        code.last().map(|instruction| instruction.op) == Some(Opcode::Return)
            && matches!(expected_result, ExpectedResult::String { .. })
            && matches!(constants.last(), Some(Constant::String(_)))
    } else {
        code.first().map(|instruction| instruction.op) == Some(Opcode::Return)
            && *expected_result == ExpectedResult::Nil
    };
    if constants.len() != CONSTANT_SLOTS
        || code.len() != INSTRUCTION_SLOTS
        || string_count != scenario.string_count()
        || !strings_have_expected_length
        || load_count != CONSTANT_SLOTS
        || return_count != 1
        || !return_is_positioned
    {
        return Err(format!(
            "internal {:?} fixture violates the fixed 1024-constant/1025-instruction protocol",
            scenario
        ));
    }
    Ok(())
}

fn fixture_parts(scenario: Scenario) -> (Vec<Constant>, Vec<Instruction>, ExpectedResult) {
    let mut constants = Vec::with_capacity(CONSTANT_SLOTS);
    match scenario {
        Scenario::Empty => constants.resize(CONSTANT_SLOTS, Constant::Nil),
        Scenario::ShortUnloaded | Scenario::ShortLoaded => {
            constants.extend(
                (0..SHORT_STRING_COUNT).map(|index| {
                    Constant::String(indexed_bytes(index, SHORT_STRING_BYTES, 37, 53))
                }),
            );
        }
        Scenario::LongUnloaded | Scenario::LongLoaded => {
            constants.resize(CONSTANT_SLOTS - LONG_STRING_COUNT, Constant::Nil);
            constants
                .extend((0..LONG_STRING_COUNT).map(|index| {
                    Constant::String(indexed_bytes(index, LONG_STRING_BYTES, 29, 17))
                }));
        }
    }

    let expected_result = match scenario {
        Scenario::ShortLoaded => ExpectedResult::String {
            index: SHORT_STRING_COUNT - 1,
            length: SHORT_STRING_BYTES,
            multiplier: 37,
            stride: 53,
        },
        Scenario::LongLoaded => ExpectedResult::String {
            index: LONG_STRING_COUNT - 1,
            length: LONG_STRING_BYTES,
            multiplier: 29,
            stride: 17,
        },
        Scenario::Empty | Scenario::ShortUnloaded | Scenario::LongUnloaded => ExpectedResult::Nil,
    };

    let mut loads = (0..CONSTANT_SLOTS)
        .map(|index| Instruction::with_aux(Opcode::LoadConstWide, 0, 0, 0, index as i32))
        .collect::<Vec<_>>();
    let return_instruction = Instruction::new(Opcode::Return, 0, 1, 0);
    let mut code = Vec::with_capacity(INSTRUCTION_SLOTS);
    if scenario.executes_constants() {
        code.append(&mut loads);
        code.push(return_instruction);
    } else {
        code.push(return_instruction);
        code.append(&mut loads);
    }

    (constants, code, expected_result)
}

fn indexed_bytes(index: usize, length: usize, multiplier: u8, stride: u8) -> Vec<u8> {
    let mut bytes = vec![0_u8; length];
    bytes[..8].copy_from_slice(&(index as u64).to_le_bytes());
    for (offset, byte) in bytes[8..].iter_mut().enumerate() {
        *byte = (index as u8)
            .wrapping_mul(multiplier)
            .wrapping_add((offset as u8).wrapping_mul(stride));
    }
    bytes
}

#[cfg(any(target_os = "linux", test))]
fn validate_results(vms: &mut [Vm], results: &[Value], expected: &ExpectedResult) -> Result<()> {
    if vms.len() != results.len() {
        return Err("VM/result validation lane length mismatch".to_owned());
    }
    let expected_bytes = expected.bytes();
    for (index, (vm, result)) in vms.iter_mut().zip(results).enumerate() {
        match expected {
            ExpectedResult::Nil => {
                if result.kind() != ValueKind::Nil {
                    return Err(format!(
                        "VM {index} returned {:?}, expected nil",
                        result.kind()
                    ));
                }
            }
            ExpectedResult::String { .. } => {
                let expected_bytes = expected_bytes
                    .as_deref()
                    .ok_or_else(|| "string validation bytes are missing".to_owned())?;
                if result.kind() != ValueKind::String {
                    return Err(format!(
                        "VM {index} returned {:?}, expected string",
                        result.kind()
                    ));
                }
                let actual = vm
                    .string_value_bytes(*result)
                    .map_err(|error| format!("VM {index} string validation failed: {error}"))?
                    .ok_or_else(|| format!("VM {index} string value was missing"))?;
                if actual != expected_bytes || fnv1a64(&actual) != fnv1a64(expected_bytes) {
                    return Err(format!("VM {index} returned the wrong string bytes"));
                }
            }
        }
    }
    Ok(())
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = FNV1A64_OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV1A64_PRIME);
    }
    hash
}

#[cfg(any(target_os = "linux", test))]
fn process_memory_snapshot() -> Result<ProcessMemory> {
    let status_path = Path::new("/proc/self/status");
    let smaps_path = Path::new("/proc/self/smaps_rollup");
    let status = fs::read_to_string(status_path)
        .map_err(|error| format!("{}: {error}", status_path.display()))?;
    let smaps = fs::read_to_string(smaps_path)
        .map_err(|error| format!("{}: {error}", smaps_path.display()))?;
    Ok(ProcessMemory {
        status: StatusMemory {
            vm_rss: parse_kib_field(&status, "VmRSS", status_path)?,
            vm_hwm: parse_kib_field(&status, "VmHWM", status_path)?,
            rss_anon: parse_kib_field(&status, "RssAnon", status_path)?,
            rss_file: parse_kib_field(&status, "RssFile", status_path)?,
            rss_shmem: parse_kib_field(&status, "RssShmem", status_path)?,
        },
        smaps_rollup: SmapsMemory {
            rss: parse_kib_field(&smaps, "Rss", smaps_path)?,
            pss: parse_kib_field(&smaps, "Pss", smaps_path)?,
            private_clean: parse_kib_field(&smaps, "Private_Clean", smaps_path)?,
            private_dirty: parse_kib_field(&smaps, "Private_Dirty", smaps_path)?,
            anonymous: parse_kib_field(&smaps, "Anonymous", smaps_path)?,
            swap: parse_kib_field(&smaps, "Swap", smaps_path)?,
        },
    })
}

fn parse_kib_field(text: &str, field: &str, path: &Path) -> Result<u64> {
    let mut found = None;
    for line in text.lines() {
        let Some((name, raw_value)) = line.split_once(':') else {
            continue;
        };
        if name != field {
            continue;
        }
        if found.is_some() {
            return Err(format!(
                "{} contains duplicate {field} fields",
                path.display()
            ));
        }
        let mut parts = raw_value.split_whitespace();
        let value = parts
            .next()
            .ok_or_else(|| format!("{} field {field} has no value", path.display()))?
            .parse::<u64>()
            .map_err(|_| format!("{} field {field} is not an integer", path.display()))?;
        if parts.next() != Some("kB") || parts.next().is_some() {
            return Err(format!(
                "{} field {field} is not a canonical kB value",
                path.display()
            ));
        }
        found = Some(value);
    }
    found.ok_or_else(|| format!("{} is missing required field {field}", path.display()))
}

fn sample_json(
    config: &Config,
    fixture: &Fixture,
    timings: Timings,
    snapshots: MemorySnapshots,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "schema": SCHEMA,
        "sample_id": config.sample_id,
        "protocol_sha256": config.protocol_sha256,
        "timestamp_unix": unix_time(),
        "build": {
            "role": config.build_role.as_str(),
            "commit": config.commit,
        },
        "round": config.round,
        "scenario": {
            "id": config.scenario.as_str(),
            "vm_count": config.vm_count,
            "profile": "reproducible-standard",
            "constant_slots": CONSTANT_SLOTS,
            "instruction_slots": fixture.program.main.code.len(),
            "executes_constants": config.scenario.executes_constants(),
            "strings": {
                "kind": config.scenario.string_kind(),
                "count": config.scenario.string_count(),
                "bytes_each": config.scenario.string_bytes(),
                "generator": "indexed-binary-v1",
            },
            "nil_constant_count": config.scenario.nil_constant_count(),
        },
        "platform": {
            "os": env::consts::OS,
            "arch": env::consts::ARCH,
            "pid": std::process::id(),
            "memory_probe_warmups": MEMORY_PROBE_WARMUPS,
        },
        "layout_bytes": {
            "value": size_of::<Value>(),
            "instruction": size_of::<Instruction>(),
            "constant": size_of::<Constant>(),
            "vm": size_of::<Vm>(),
        },
        "timing_ns": {
            "program_build": timings.program_build,
            "vm_create": timings.vm_create,
            "program_run": timings.program_run,
        },
        "memory_kib": {
            "clean": snapshots.clean.json(),
            "program_ready": snapshots.program_ready.json(),
            "vms_ready": snapshots.vms_ready.json(),
            "after_run": snapshots.after_run.json(),
        },
        "validation": {
            "created_vms": config.vm_count,
            "ran_vms": config.vm_count,
            "expected_result_kind": fixture.expected_result.kind(),
            "expected_result_bytes": fixture.expected_result.byte_len(),
            "expected_result_fnv1a64": fixture.expected_result.fnv1a64_hex(),
            "all_results_matched": true,
        },
    })
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

    fn valid_args() -> Vec<String> {
        [
            "sample",
            "--scenario",
            "short-loaded",
            "--vm-count",
            "512",
            "--round",
            "3",
            "--sample-id",
            "round-03-candidate-short-loaded",
            "--build-role",
            "candidate",
            "--commit",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--protocol-sha256",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    #[test]
    fn parses_complete_sample_cli() {
        let config = parse_args(valid_args().into_iter()).expect("valid CLI");
        assert_eq!(config.scenario, Scenario::ShortLoaded);
        assert_eq!(config.vm_count, 512);
        assert_eq!(config.round, 3);
        assert_eq!(config.build_role, BuildRole::Candidate);
        assert_eq!(config.sample_id, "round-03-candidate-short-loaded");
    }

    #[test]
    fn rejects_duplicate_and_noncanonical_identity_arguments() {
        let mut duplicate = valid_args();
        duplicate.extend(["--round".to_owned(), "4".to_owned()]);
        assert!(parse_args(duplicate.into_iter())
            .expect_err("duplicate")
            .contains("more than once"));

        let mut uppercase = valid_args();
        let commit = uppercase
            .iter()
            .position(|value| value == "--commit")
            .expect("commit flag");
        uppercase[commit + 1] = "A".repeat(40);
        assert!(parse_args(uppercase.into_iter())
            .expect_err("uppercase hash")
            .contains("lowercase hexadecimal"));
    }

    #[test]
    fn every_scenario_has_matched_constant_and_instruction_shapes() {
        for scenario in Scenario::ALL {
            let (constants, code, expected) = fixture_parts(scenario);
            assert_eq!(constants.len(), CONSTANT_SLOTS, "{scenario:?}");
            assert_eq!(code.len(), INSTRUCTION_SLOTS, "{scenario:?}");
            assert_eq!(
                code.iter()
                    .filter(|instruction| instruction.op == Opcode::LoadConstWide)
                    .count(),
                CONSTANT_SLOTS,
                "{scenario:?}"
            );
            assert_eq!(
                code.iter()
                    .filter(|instruction| instruction.op == Opcode::Return)
                    .count(),
                1,
                "{scenario:?}"
            );
            if scenario.executes_constants() {
                assert_eq!(
                    code.last().map(|instruction| instruction.op),
                    Some(Opcode::Return)
                );
                assert!(matches!(expected, ExpectedResult::String { .. }));
            } else {
                assert_eq!(
                    code.first().map(|instruction| instruction.op),
                    Some(Opcode::Return)
                );
                assert_eq!(expected, ExpectedResult::Nil);
            }
        }
    }

    #[test]
    fn scenarios_have_exact_string_and_nil_composition() {
        for scenario in Scenario::ALL {
            let (constants, _, _) = fixture_parts(scenario);
            let string_lengths = constants
                .iter()
                .filter_map(|constant| match constant {
                    Constant::String(bytes) => Some(bytes.len()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let nils = constants
                .iter()
                .filter(|constant| matches!(constant, Constant::Nil))
                .count();
            assert_eq!(
                string_lengths.len(),
                scenario.string_count(),
                "{scenario:?}"
            );
            assert!(
                string_lengths
                    .iter()
                    .all(|length| *length == scenario.string_bytes()),
                "{scenario:?}"
            );
            assert_eq!(nils, scenario.nil_constant_count(), "{scenario:?}");
        }
    }

    #[test]
    fn every_fixture_executes_and_returns_the_exact_expected_result() {
        for scenario in Scenario::ALL {
            let fixture = build_fixture(scenario).expect("valid fixture");
            let mut vms = [Vm::new(Profile::ReproducibleStandard)];
            let results = [vms[0].run(&fixture.program).expect("fixture execution")];
            validate_results(&mut vms, &results, &fixture.expected_result)
                .expect("exact fixture result");
        }
    }

    #[test]
    fn fixed_snapshot_parser_requires_canonical_kib_fields() {
        let path = Path::new("/proc/test");
        assert_eq!(
            parse_kib_field("VmRSS:\t123 kB\n", "VmRSS", path).expect("field"),
            123
        );
        assert!(parse_kib_field("VmRSS: 123 bytes\n", "VmRSS", path).is_err());
        assert!(parse_kib_field("VmHWM: 123 kB\n", "VmRSS", path).is_err());
        assert!(parse_kib_field("VmRSS: 1 kB\nVmRSS: 2 kB\n", "VmRSS", path).is_err());
    }

    #[test]
    fn schema_contains_fixed_protocol_and_validation_fields() {
        let config = parse_args(valid_args().into_iter()).expect("valid CLI");
        let fixture = build_fixture(config.scenario).expect("valid fixture");
        let value = sample_json(
            &config,
            &fixture,
            Timings {
                program_build: 1,
                vm_create: 2,
                program_run: 3,
            },
            MemorySnapshots::default(),
        );
        assert_eq!(value["schema"], SCHEMA);
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["scenario"]["constant_slots"], CONSTANT_SLOTS);
        assert_eq!(value["scenario"]["instruction_slots"], INSTRUCTION_SLOTS);
        assert_eq!(
            value["scenario"]["strings"]["bytes_each"],
            SHORT_STRING_BYTES
        );
        assert_eq!(value["validation"]["created_vms"], 512);
        assert_eq!(value["validation"]["all_results_matched"], true);
        assert!(value["memory_kib"]["after_run"]["smaps_rollup"]["swap"].is_u64());
    }
}
