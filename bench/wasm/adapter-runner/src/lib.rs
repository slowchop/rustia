use rustia::{
    Effects, Function, HostContext, HostModule, HostSchema, Input, InvocationLimits, Profile,
    Result as RustiaResult, Rustia, RustiaBuilder, VmMemoryStats,
};
use std::cell::RefCell;
use std::slice;
use std::str;

const ADAPTER_RUSTIA: u32 = 1;
const ADAPTER_KOTO: u32 = 2;
const ADAPTER_WASMI: u32 = 3;
const ADAPTER_RHAI: u32 = 4;

thread_local! {
    static BENCH: RefCell<Option<BenchState>> = const { RefCell::new(None) };
    static LAST_RESULT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

// Keep engines inline so the adapter benchmark does not add a box indirection
// to one implementation merely because their state sizes differ.
#[allow(clippy::large_enum_variant)]
enum BenchState {
    Rustia {
        engine: Rustia,
        function: Function,
    },
    Koto {
        koto: koto::prelude::Koto,
        function: koto::prelude::KValue,
    },
    Rhai {
        engine: rhai::Engine,
        ast: rhai::AST,
        scope: rhai::Scope<'static>,
    },
    Wasmi {
        store: wasmi::Store<()>,
        function: wasmi::TypedFunc<i64, i64>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HardenedProbeRecord {
    sequence: u32,
    value: i64,
}

struct HardenedProbeSchema;

impl HostSchema for HardenedProbeSchema {
    type Input = i64;
    type Slots = ();
    type External = ();
    type Effect = i64;
    type SlotWrite = ();
    type Record = HardenedProbeRecord;
}

struct HardenedProbeContext {
    input: i64,
    external: (),
}

impl HostContext<HardenedProbeSchema> for HardenedProbeContext {
    fn input(&self) -> &i64 {
        &self.input
    }

    fn slots(&self) -> &() {
        &()
    }

    fn external_mut(&mut self) -> &mut () {
        &mut self.external
    }

    fn record_effect(&self, sequence: u32, value: i64) -> HardenedProbeRecord {
        HardenedProbeRecord { sequence, value }
    }

    fn record_slot_write(&self, sequence: u32, _write: ()) -> HardenedProbeRecord {
        HardenedProbeRecord {
            sequence,
            value: i64::MIN,
        }
    }

    fn record_bytes(&self, _record: &HardenedProbeRecord) -> u64 {
        16
    }
}

#[no_mangle]
pub extern "C" fn bench_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len);
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

#[no_mangle]
/// # Safety
///
/// `ptr` must have been returned by `bench_alloc` with the same `capacity`, and it must not be
/// used after this call returns.
pub unsafe extern "C" fn bench_dealloc(ptr: *mut u8, capacity: usize) {
    if !ptr.is_null() {
        drop(Vec::from_raw_parts(ptr, 0, capacity));
    }
}

#[no_mangle]
pub extern "C" fn bench_abi_version() -> u32 {
    1
}

#[no_mangle]
pub extern "C" fn bench_adapter_mask() -> u32 {
    (1 << ADAPTER_RUSTIA) | (1 << ADAPTER_KOTO) | (1 << ADAPTER_WASMI) | (1 << ADAPTER_RHAI)
}

#[no_mangle]
/// # Safety
///
/// `ptr` must point to `len` initialized bytes that remain valid for the duration of this call.
pub unsafe extern "C" fn bench_prepare(adapter: u32, ptr: *const u8, len: usize) -> i32 {
    let bytes = slice::from_raw_parts(ptr, len);
    let source = match str::from_utf8(bytes) {
        Ok(source) => source,
        Err(error) => {
            set_last_result(format!(
                "error\nbench workload source is not valid UTF-8: {error}"
            ));
            return 1;
        }
    };
    match prepare(adapter, source) {
        Ok(state) => {
            BENCH.with(|bench| {
                *bench.borrow_mut() = Some(state);
            });
            set_last_result("ok\nbench workload prepared");
            0
        }
        Err(error) => {
            set_last_result(format!("error\nbench workload prepare failed: {error}"));
            1
        }
    }
}

#[no_mangle]
/// Prepares the rustia adapter with a logical-live VM quota.
///
/// # Safety
///
/// `ptr` must point to `len` initialized bytes that remain valid for the
/// duration of this call.
pub unsafe extern "C" fn bench_prepare_rustia_limited(
    ptr: *const u8,
    len: usize,
    memory_limit_bytes: usize,
) -> i32 {
    let bytes = slice::from_raw_parts(ptr, len);
    let source = match str::from_utf8(bytes) {
        Ok(source) => source,
        Err(error) => {
            set_last_result(format!(
                "error\nbench workload source is not valid UTF-8: {error}"
            ));
            return 1;
        }
    };
    match prepare_rustia(source, Some(memory_limit_bytes)) {
        Ok(state) => {
            BENCH.with(|bench| {
                *bench.borrow_mut() = Some(state);
            });
            set_last_result("ok\nlimited rustia workload prepared");
            0
        }
        Err(error) => {
            set_last_result(format!(
                "error\nlimited rustia workload prepare failed: {error}"
            ));
            1
        }
    }
}

#[no_mangle]
pub extern "C" fn bench_run(n: u32) -> f64 {
    BENCH.with(|bench| {
        let mut bench = bench.borrow_mut();
        let Some(state) = bench.as_mut() else {
            set_last_result("error\nbench workload is not prepared");
            return f64::NAN;
        };
        match run_prepared(state, n) {
            Ok(value) => value,
            Err(error) => {
                set_last_result(format!("error\nbench workload call failed: {error}"));
                f64::NAN
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn bench_rustia_decision_reset_probe() -> i32 {
    match run_rustia_decision_reset_probe() {
        Ok(()) => {
            set_last_result("ok\nrustia decision reset probe passed");
            0
        }
        Err(error) => {
            set_last_result(format!(
                "error\nrustia decision reset probe failed: {error}"
            ));
            1
        }
    }
}

#[no_mangle]
pub extern "C" fn bench_rustia_hardened_leaf_probe() -> i32 {
    match run_rustia_hardened_leaf_probe() {
        Ok(()) => {
            set_last_result("ok\nrustia hardened leaf probe passed");
            0
        }
        Err(error) => {
            set_last_result(format!("error\nrustia hardened leaf probe failed: {error}"));
            1
        }
    }
}

#[no_mangle]
pub extern "C" fn bench_result_ptr() -> *const u8 {
    LAST_RESULT.with(|result| result.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn bench_result_len() -> usize {
    LAST_RESULT.with(|result| result.borrow().len())
}

fn prepare(adapter: u32, source: &str) -> Result<BenchState, String> {
    match adapter {
        ADAPTER_RUSTIA => prepare_rustia(source, None),
        ADAPTER_KOTO => prepare_koto(source),
        ADAPTER_WASMI => prepare_wasmi(source),
        ADAPTER_RHAI => prepare_rhai(source),
        _ => Err(format!("unknown adapter id {adapter}")),
    }
}

fn prepare_rustia(source: &str, memory_limit_bytes: Option<usize>) -> Result<BenchState, String> {
    let wrapped = format!("return {{ f = (function()\n{source}\nend)() }}");
    let mut engine = match memory_limit_bytes {
        Some(limit) => Rustia::builder()
            .profile(Profile::ReproducibleStandard)
            .memory_limit_bytes(limit)
            .build()
            .map_err(|error| error.to_string())?,
        None => Rustia::new(Profile::ReproducibleStandard),
    };
    let exports = engine
        .compile_and_run_exports_named("bench/workload.luau", &wrapped)
        .map_err(|error| error.to_string())?;
    let function = exports
        .function(&mut engine, "f")
        .map_err(|error| error.to_string())?;
    Ok(BenchState::Rustia { engine, function })
}

fn run_rustia_decision_reset_probe() -> Result<(), String> {
    const DECISIONS: u64 = 300;
    const WARMUP_DECISIONS: u64 = 50;

    let mut engine = Rustia::builder()
        .profile(Profile::DeterministicSim)
        .memory_tracking(true)
        .build()
        .map_err(|error| error.to_string())?;
    engine
        .add_pure_fn("bias", || 7.0)
        .map_err(|error| error.to_string())?;
    let module = Rustia::compile(
        "wasm-reset-probe.luau",
        "\
local temporary = {}
for i = 1, 48 do
    temporary[i] = { label = 'wasm-reset-' .. i, value = i }
end
local thread = coroutine.create(function()
    coroutine.yield(temporary)
end)
coroutine.resume(thread)
return {
    answer = input + bias() + math.random(),
    count = #temporary,
}
",
    )
    .map_err(|error| error.to_string())?;

    let mut expected_answer = None;
    let mut plateau: Option<VmMemoryStats> = None;
    for decision in 0..DECISIONS {
        let input_is_clear: bool = engine
            .run("return input == nil")
            .map_err(|error| error.to_string())?;
        if !input_is_clear {
            return Err(format!(
                "decision input survived reset before epoch {decision}"
            ));
        }
        engine
            .insert_global("input", 5.0)
            .map_err(|error| error.to_string())?;
        let exports = engine
            .install(module.clone())
            .map_err(|error| error.to_string())?;
        if exports
            .get::<usize>(&mut engine, "count")
            .map_err(|error| error.to_string())?
            != 48
        {
            return Err(format!("guest heap result changed in epoch {decision}"));
        }
        let answer = exports
            .get::<f64>(&mut engine, "answer")
            .map_err(|error| error.to_string())?;
        match expected_answer {
            Some(expected) if answer != expected => {
                return Err(format!(
                    "RNG sequence changed in epoch {decision}: expected {expected}, got {answer}"
                ));
            }
            None => expected_answer = Some(answer),
            _ => {}
        }

        engine.reset_decision().map_err(|error| error.to_string())?;
        if exports.is_current(&engine) {
            return Err(format!(
                "pre-reset exports remained current after epoch {decision}"
            ));
        }
        let stats = engine.memory_stats();
        if decision == WARMUP_DECISIONS - 1 {
            plateau = Some(stats);
        } else if decision >= WARMUP_DECISIONS && plateau != Some(stats) {
            return Err(format!(
                "VM-owned memory left its warm plateau after epoch {decision}: expected {:?}, got {stats:?}",
                plateau.expect("plateau is initialized after warmup")
            ));
        }
    }

    if engine.decision_epoch() != DECISIONS {
        return Err(format!(
            "expected decision epoch {DECISIONS}, got {}",
            engine.decision_epoch()
        ));
    }
    Ok(())
}

fn run_rustia_hardened_leaf_probe() -> Result<(), String> {
    const DECISIONS: i64 = 300;
    const WARMUP_DECISIONS: i64 = 50;
    const MEMORY_LIMIT_BYTES: usize = 4 * 1024 * 1024;

    let mut engine = RustiaBuilder::deterministic_leaf()
        .memory_limit_bytes(MEMORY_LIMIT_BYTES)
        .memory_tracking(true)
        .build_for_host::<HardenedProbeSchema>()
        .map_err(|error| error.to_string())?;
    engine
        .install_host_module(
            HostModule::<HardenedProbeSchema>::new("leaf")
                .input("input", 2, |input: Input<'_, HardenedProbeSchema>| {
                    *input.get()
                })
                .effect(
                    "emit",
                    2,
                    |mut effects: Effects<'_, HardenedProbeSchema>,
                     value: i64|
                     -> RustiaResult<()> { effects.emit(value) },
                ),
        )
        .map_err(|error| error.to_string())?;
    let entry = Rustia::<HardenedProbeSchema>::compile_entry(
        "wasm-hardened-leaf-probe.luau",
        r#"
local input = leaf.input()
local count = 8 + (input % 5)
local scratch = {}
for i = 1, count do
    scratch[i] = {
        label = string.rep("x", 16 + ((i + input) % 4)),
        value = input + i,
    }
end
leaf.emit(count)
return scratch[1].value
"#,
    )
    .map_err(|error| error.to_string())?;
    let failed_entry = Rustia::<HardenedProbeSchema>::compile_entry(
        "wasm-hardened-leaf-failure.luau",
        "leaf.emit(999)\nerror('expected failure')",
    )
    .map_err(|error| error.to_string())?;
    let limits = InvocationLimits::new(100_000)
        .with_max_records(2)
        .with_max_record_bytes(32);
    let sentinel = HardenedProbeRecord {
        sequence: 99,
        value: 99,
    };
    let mut lane = vec![sentinel.clone()];
    let mut failed_context = HardenedProbeContext {
        input: -1,
        external: (),
    };
    if engine
        .invoke_entry_into::<i64>(&mut failed_context, limits, &mut lane, &failed_entry)
        .is_ok()
    {
        return Err("failing hardened entry unexpectedly succeeded".to_owned());
    }
    if lane != [sentinel] {
        return Err("failing hardened entry published a record".to_owned());
    }
    if engine.decision_epoch() != 1 {
        return Err(format!(
            "failing hardened entry did not reset: expected epoch 1, got {}",
            engine.decision_epoch()
        ));
    }

    let mut plateau: Option<VmMemoryStats> = None;
    for decision in 0..DECISIONS {
        lane.clear();
        let mut context = HardenedProbeContext {
            input: decision,
            external: (),
        };
        let receipt = engine
            .invoke_entry_into::<i64>(&mut context, limits, &mut lane, &entry)
            .map_err(|error| error.to_string())?;
        if *receipt.value() != decision + 1 {
            return Err(format!(
                "hardened entry returned the wrong value in decision {decision}"
            ));
        }
        let expected_record = HardenedProbeRecord {
            sequence: 0,
            value: 8 + (decision % 5),
        };
        if lane != [expected_record] {
            return Err(format!(
                "hardened entry recorded the wrong effect in decision {decision}: {lane:?}"
            ));
        }
        let stats = engine.memory_stats();
        if decision == WARMUP_DECISIONS - 1 {
            plateau = Some(stats);
        } else if decision >= WARMUP_DECISIONS && plateau != Some(stats) {
            return Err(format!(
                "hardened VM-owned memory left its warm plateau after decision {decision}: expected {:?}, got {stats:?}",
                plateau.expect("plateau is initialized after warmup")
            ));
        }
    }

    let expected_epoch = u64::try_from(DECISIONS).expect("positive decisions") + 1;
    if engine.decision_epoch() != expected_epoch {
        return Err(format!(
            "expected hardened decision epoch {expected_epoch}, got {}",
            engine.decision_epoch()
        ));
    }
    Ok(())
}

fn prepare_koto(source: &str) -> Result<BenchState, String> {
    use koto::prelude::Koto;

    let mut koto = Koto::default();
    koto.compile_and_run(source)
        .map_err(|error| error.to_string())?;
    let function = koto
        .exports()
        .get("f")
        .ok_or_else(|| "koto workload must export function f".to_owned())?;
    Ok(BenchState::Koto { koto, function })
}

fn prepare_rhai(source: &str) -> Result<BenchState, String> {
    let engine = rhai::Engine::new();
    let ast = engine.compile(source).map_err(|error| error.to_string())?;
    Ok(BenchState::Rhai {
        engine,
        ast,
        scope: rhai::Scope::new(),
    })
}

fn prepare_wasmi(source: &str) -> Result<BenchState, String> {
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, source).map_err(|error| error.to_string())?;
    let mut store = wasmi::Store::new(&engine, ());
    let linker = wasmi::Linker::new(&engine);
    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|error| error.to_string())?;
    let function = instance
        .get_typed_func::<i64, i64>(&store, "f")
        .map_err(|error| error.to_string())?;
    Ok(BenchState::Wasmi { store, function })
}

fn run_prepared(state: &mut BenchState, n: u32) -> Result<f64, String> {
    match state {
        BenchState::Rustia { engine, function } => function
            .call::<_, f64>(engine, (n as f64,))
            .map_err(|error| error.to_string()),
        BenchState::Koto { koto, function } => {
            use koto::prelude::{KNumber, KValue};

            let result = koto
                .call_function(function.clone(), &[KValue::Number(KNumber::I64(n.into()))])
                .map_err(|error| error.to_string())?;
            match result {
                KValue::Number(KNumber::I64(value)) => Ok(value as f64),
                KValue::Number(KNumber::F64(value)) => Ok(value),
                KValue::Bool(value) => Ok(if value { 1.0 } else { 0.0 }),
                other => Err(format!("expected numeric koto result, got {other:?}")),
            }
        }
        BenchState::Rhai { engine, ast, scope } => {
            let result = engine
                .call_fn::<rhai::Dynamic>(scope, ast, "f", (rhai::INT::from(n),))
                .map_err(|error| error.to_string())?;
            if let Ok(value) = result.as_int() {
                return Ok(value as f64);
            }
            if let Ok(value) = result.as_float() {
                return Ok(value);
            }
            if let Some(value) = result.clone().try_cast::<bool>() {
                return Ok(if value { 1.0 } else { 0.0 });
            }
            Err(format!(
                "expected numeric rhai result, got {}",
                result.type_name()
            ))
        }
        BenchState::Wasmi { store, function } => {
            let value = function
                .call(store, i64::from(n))
                .map_err(|error| error.to_string())?;
            Ok(value as f64)
        }
    }
}

fn set_last_result(result: impl Into<String>) {
    LAST_RESULT.with(|slot| {
        *slot.borrow_mut() = result.into().into_bytes();
    });
}
