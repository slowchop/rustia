use rustia::{Profile, Rustia, Value, ValueKind};
use std::cell::RefCell;
use std::slice;
use std::str;

thread_local! {
    static ENGINE: RefCell<Rustia> = RefCell::new(Rustia::new(Profile::ReproducibleStandard));
    static LAST_RESULT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
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

fn run_source(source: &str) -> String {
    ENGINE.with(|engine| {
        let mut engine = engine.borrow_mut();
        match engine.compile_and_run(source) {
            Ok(value) => format!("ok\n=> {}", format_value(&mut engine, value)),
            Err(error) => format!("error\n{error}"),
        }
    })
}

fn format_value(engine: &mut Rustia, value: Value) -> String {
    match value.kind() {
        ValueKind::Nil => "nil".to_owned(),
        ValueKind::Boolean => value.as_bool().unwrap_or(false).to_string(),
        ValueKind::Number => format_number(value.as_number().unwrap_or(f64::NAN)),
        ValueKind::String => engine
            .vm_mut()
            .string_value(value)
            .ok()
            .flatten()
            .map(|text| format!("{text:?}"))
            .unwrap_or_else(|| "<missing string>".to_owned()),
        ValueKind::Table | ValueKind::Function | ValueKind::Userdata | ValueKind::Buffer => {
            format!("{value:?}")
        }
        _ => format!("{value:?}"),
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
