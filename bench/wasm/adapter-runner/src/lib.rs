use rustia::{Function, Profile, Rustia};
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
pub extern "C" fn bench_result_ptr() -> *const u8 {
    LAST_RESULT.with(|result| result.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn bench_result_len() -> usize {
    LAST_RESULT.with(|result| result.borrow().len())
}

fn prepare(adapter: u32, source: &str) -> Result<BenchState, String> {
    match adapter {
        ADAPTER_RUSTIA => prepare_rustia(source),
        ADAPTER_KOTO => prepare_koto(source),
        ADAPTER_WASMI => prepare_wasmi(source),
        ADAPTER_RHAI => prepare_rhai(source),
        _ => Err(format!("unknown adapter id {adapter}")),
    }
}

fn prepare_rustia(source: &str) -> Result<BenchState, String> {
    let wrapped = format!("return {{ f = (function()\n{source}\nend)() }}");
    let mut engine = Rustia::new(Profile::ReproducibleStandard);
    let exports = engine
        .compile_and_run_exports_named("bench/workload.luau", &wrapped)
        .map_err(|error| error.to_string())?;
    let function = exports
        .function(&mut engine, "f")
        .map_err(|error| error.to_string())?;
    Ok(BenchState::Rustia { engine, function })
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
