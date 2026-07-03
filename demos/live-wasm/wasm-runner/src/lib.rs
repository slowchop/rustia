use rustia::{Function, OwnedValue, Profile, Rustia};
use std::cell::RefCell;
use std::slice;
use std::str;

thread_local! {
    static ENGINE: RefCell<Rustia> = RefCell::new(Rustia::new(Profile::ReproducibleStandard));
    static BENCH: RefCell<Option<BenchState>> = const { RefCell::new(None) };
    static LAST_RESULT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

struct BenchState {
    engine: Rustia,
    function: Function,
}

#[no_mangle]
pub extern "C" fn rustia_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len);
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

#[no_mangle]
/// # Safety
///
/// `ptr` must have been returned by `rustia_alloc` with the same `capacity`, and it must not be
/// used after this call returns.
pub unsafe extern "C" fn rustia_dealloc(ptr: *mut u8, capacity: usize) {
    if !ptr.is_null() {
        drop(Vec::from_raw_parts(ptr, 0, capacity));
    }
}

#[no_mangle]
/// # Safety
///
/// `ptr` must point to `len` initialized bytes that remain valid for the duration of this call.
pub unsafe extern "C" fn rustia_run(ptr: *const u8, len: usize) -> i32 {
    let bytes = slice::from_raw_parts(ptr, len);
    let output = match str::from_utf8(bytes) {
        Ok(source) => run_source(source),
        Err(error) => format!("error\ninput is not valid UTF-8: {error}"),
    };

    LAST_RESULT.with(|result| {
        *result.borrow_mut() = output.into_bytes();
    });
    0
}

#[no_mangle]
pub extern "C" fn rustia_result_ptr() -> *const u8 {
    LAST_RESULT.with(|result| result.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn rustia_result_len() -> usize {
    LAST_RESULT.with(|result| result.borrow().len())
}

#[no_mangle]
pub extern "C" fn rustia_bench_abi_version() -> u32 {
    1
}

#[no_mangle]
/// # Safety
///
/// `ptr` must point to `len` initialized bytes that remain valid for the duration of this call.
pub unsafe extern "C" fn rustia_bench_prepare(ptr: *const u8, len: usize) -> i32 {
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
    match prepare_bench_source(source) {
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
pub extern "C" fn rustia_bench_run(n: u32) -> f64 {
    BENCH.with(|bench| {
        let mut bench = bench.borrow_mut();
        let Some(state) = bench.as_mut() else {
            set_last_result("error\nbench workload is not prepared");
            return f64::NAN;
        };
        match state
            .function
            .call::<_, f64>(&mut state.engine, (n as f64,))
        {
            Ok(value) => value,
            Err(error) => {
                set_last_result(format!("error\nbench workload call failed: {error}"));
                f64::NAN
            }
        }
    })
}

fn run_source(source: &str) -> String {
    ENGINE.with(|engine| {
        let mut engine = engine.borrow_mut();
        match engine.run_owned(source) {
            Ok(value) => format!("ok\n=> {}", format_value(&value)),
            Err(error) => format!("error\n{error}"),
        }
    })
}

fn prepare_bench_source(source: &str) -> Result<BenchState, rustia::Error> {
    let wrapped = format!("return {{ f = (function()\n{source}\nend)() }}");
    let mut engine = Rustia::new(Profile::ReproducibleStandard);
    let exports = engine.compile_and_run_exports_named("bench/workload.luau", &wrapped)?;
    let function = exports.function(&mut engine, "f")?;
    Ok(BenchState { engine, function })
}

fn set_last_result(result: impl Into<String>) {
    LAST_RESULT.with(|slot| {
        *slot.borrow_mut() = result.into().into_bytes();
    });
}

fn format_value(value: &OwnedValue) -> String {
    match value {
        OwnedValue::Nil => "nil".to_owned(),
        OwnedValue::Boolean(value) => value.to_string(),
        OwnedValue::Number(value) => format_number(*value),
        OwnedValue::String(value) => format!("{value:?}"),
        OwnedValue::Table(_) => format!("{value:?}"),
    }
}

fn format_number(value: f64) -> String {
    if value.is_nan() {
        "nan".to_owned()
    } else if value == f64::INFINITY {
        "inf".to_owned()
    } else if value == f64::NEG_INFINITY {
        "-inf".to_owned()
    } else if value == 0.0 && value.is_sign_negative() {
        "-0".to_owned()
    } else if value.is_finite() && value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}
