//! spike-wasmjit: measure the "wasm-emit JIT / AOT-to-wasm" path for hot Luau kernels in-browser.
//!
//! ONE representative hot kernel (the grabby IAUS argmax shape, f64 "realistic" variant):
//!   given arr[0..4] (4 considerations in [0,1)) and `last` (the previous action index 0..6),
//!   for each of 6 actions compute the product of 4 curves (kind=(a+c)%3: 0=>x*x, 1=>x, 2=>1-x),
//!   apply *1.25 momentum if a==last, and return (best action index, best score). Data-dependent:
//!   inputs are produced by a per-iteration LCG so nothing can be constant-folded.
//!
//! The SAME kernel logic is realized THREE ways, all measured in wasm-in-V8:
//!   1) INTERPRETER  — run the kernel as bytecode through the REAL spike-sk-rustia register VM
//!                     (big match -> br_table dispatch), here compiled to wasm32-unknown-unknown.
//!   2) STATIC-AOT   — the kernel as a plain Rust fn compiled to wasm32 (ideal compiled code).
//!   3) RUNTIME-GEN  — at RUNTIME we EMIT a wasm module (via the wasm-encoder crate) that encodes
//!                     the identical driver loop + kernel, instantiate it in V8, and run it.
//!   4) SHARED-HEAP  — like (3) but the module IMPORTS the host linear memory and reads/writes the
//!                     inputs array living in that shared heap (realistic "JIT'd code touching the
//!                     VM heap"; exercises memory loads/stores + bounds checks).
//!
//! Every path produces a bit-identical FNV checksum over (best, best_v.bits) across all iterations,
//! which both pins down "same kernel logic" and acts as anti-DCE. The host module exports the
//! interpreter + AOT drivers + the codegen functions (so codegen itself is timed inside wasm-in-V8,
//! matching the in-browser scenario), plus a scratch pointer for the shared-heap variant.

#![allow(static_mut_refs)]

use spike_sk_rustia::{build_argmax_f64, Luts, Value, Vm};
use wasm_encoder::{
    BlockType, CodeSection, ExportKind, ExportSection, Function, FunctionSection, Ieee64,
    ImportSection, InstructionSink, MemArg, MemoryType, Module, TypeSection, ValType,
};

// ---- shared kernel/driver constants (identical across all four paths) -------------------------
const LCG_A: u64 = 6364136223846793005; // PCG/Knuth LCG multiplier
const LCG_C: u64 = 1442695040888963407; // LCG increment
const INV_2P53: f64 = 1.0 / 9007199254740992.0; // 2^-53, exact in f64
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const N_ACTIONS: i64 = 6;
const N_CONSID: i64 = 4;

#[inline(always)]
fn lcg(s: &mut u64) -> u64 {
    *s = s.wrapping_mul(LCG_A).wrapping_add(LCG_C);
    *s
}

#[inline(always)]
fn gen_inputs(s: &mut u64) -> ([f64; 4], i64) {
    let x0 = ((lcg(s) >> 11) as f64) * INV_2P53;
    let x1 = ((lcg(s) >> 11) as f64) * INV_2P53;
    let x2 = ((lcg(s) >> 11) as f64) * INV_2P53;
    let x3 = ((lcg(s) >> 11) as f64) * INV_2P53;
    let last = ((lcg(s) >> 32) % (N_ACTIONS as u64)) as i64;
    ([x0, x1, x2, x3], last)
}

// =====================================================================================
// (2) STATIC-AOT kernel + driver — the "ideal compiled code the engine JITs" reference.
// =====================================================================================

#[inline(always)]
fn kernel_aot(x: [f64; 4], last: i64) -> (i64, f64) {
    let mut best: i64 = 0;
    let mut best_v: f64 = -1.0;
    let mut a: i64 = 0;
    while a < N_ACTIONS {
        let mut prod = 1.0f64;
        let mut c: i64 = 0;
        while c < N_CONSID {
            let xx = x[c as usize];
            let kind = (a + c) % 3;
            let cv = if kind == 0 {
                xx * xx
            } else if kind == 1 {
                xx
            } else {
                1.0 - xx
            };
            prod *= cv;
            c += 1;
        }
        if a == last {
            prod *= 1.25;
        }
        if best_v < prod {
            best_v = prod;
            best = a;
        }
        a += 1;
    }
    (best, best_v)
}

#[inline(always)]
fn fold(chk: &mut u64, best: i64, best_v: f64) {
    *chk ^= best as u64;
    *chk = chk.wrapping_mul(FNV_PRIME);
    *chk ^= best_v.to_bits();
    *chk = chk.wrapping_mul(FNV_PRIME);
}

pub fn run_aot_impl(iters: i32, seed: i32) -> u64 {
    let mut s = seed as u32 as u64;
    let mut chk = FNV_OFFSET;
    for _ in 0..iters {
        let (x, last) = gen_inputs(&mut s);
        let (best, best_v) = kernel_aot(x, last);
        fold(&mut chk, best, best_v);
    }
    chk
}

// =====================================================================================
// (1) INTERPRETER driver — runs the SAME kernel as bytecode through the real skeleton VM.
// =====================================================================================

pub fn run_interp_impl(iters: i32, seed: i32) -> u64 {
    let prog = build_argmax_f64();
    let luts = Luts::new();
    let mut vm = Vm::new();
    let mut s = seed as u32 as u64;
    let mut chk = FNV_OFFSET;
    for _ in 0..iters {
        let (x, last) = gen_inputs(&mut s);
        vm.arr[0] = Value::num(x[0]);
        vm.arr[1] = Value::num(x[1]);
        vm.arr[2] = Value::num(x[2]);
        vm.arr[3] = Value::num(x[3]);
        vm.reg[0] = Value::num(last as f64);
        // FUEL=false: give the interpreter its best shot (conservative for the JIT-wins argument).
        let _ = vm.run::<false>(&prog.code, &prog.consts, &luts, u64::MAX);
        let best = vm.reg[1].as_num() as i64; // best action index 0..5
        let best_v = vm.reg[2].as_num(); // best score (f64)
        fold(&mut chk, best, best_v);
    }
    chk
}

// =====================================================================================
// (3)/(4) RUNTIME wasm emitter (wasm-encoder). Emits an exported function that does the
//          identical driver loop + argmax kernel. `shared`=> import host memory and keep the
//          inputs array in that heap (store after gen, load inside the kernel).
// =====================================================================================

// local layout helper
struct Locals {
    iters: u32, // i32 param
    base: u32,  // i32 param (shared only; else unused)
    i: u32,     // i32 iter counter
    cmp: u32,   // i32 argmax compare
    s: u32,     // i64 lcg state
    chk: u32,   // i64 checksum
    last: u32,  // i64
    a: u32,     // i64 action loop var
    best: u32,  // i64
    kind: u32,  // i64
    x0: u32,    // f64
    x1: u32,
    x2: u32,
    x3: u32,
    prod: u32, // f64
    best_v: u32,
    xt: u32, // f64 scratch (shared: loaded input)
}

fn layout(shared: bool) -> (Locals, Vec<(u32, ValType)>, u32) {
    // params
    let nparams: u32 = if shared { 3 } else { 2 };
    // iters=0, seed=1, [base=2]
    let base = if shared { 2 } else { u32::MAX };
    // declared locals start at nparams
    let i = nparams; // i32
    let cmp = nparams + 1; // i32
    let s = nparams + 2; // i64
    let chk = nparams + 3;
    let last = nparams + 4;
    let a = nparams + 5;
    let best = nparams + 6;
    let kind = nparams + 7;
    let x0 = nparams + 8; // f64
    let x1 = nparams + 9;
    let x2 = nparams + 10;
    let x3 = nparams + 11;
    let prod = nparams + 12;
    let best_v = nparams + 13;
    let xt = nparams + 14;
    let decls = vec![
        (2u32, ValType::I32), // i, cmp
        (6u32, ValType::I64), // s, chk, last, a, best, kind
        (7u32, ValType::F64), // x0..x3, prod, best_v, xt
    ];
    (
        Locals {
            iters: 0,
            base,
            i,
            cmp,
            s,
            chk,
            last,
            a,
            best,
            kind,
            x0,
            x1,
            x2,
            x3,
            prod,
            best_v,
            xt,
        },
        decls,
        nparams,
    )
}

fn f64c(x: f64) -> Ieee64 {
    Ieee64::from(x)
}

// emit one LCG step writing the derived f64 into local `dst`.
fn emit_input(s: &mut InstructionSink, l: &Locals, dst: u32) {
    s.local_get(l.s)
        .i64_const(LCG_A as i64)
        .i64_mul()
        .i64_const(LCG_C as i64)
        .i64_add()
        .local_tee(l.s)
        .i64_const(11)
        .i64_shr_u()
        .f64_convert_i64_u()
        .f64_const(f64c(INV_2P53))
        .f64_mul()
        .local_set(dst);
}

// emit one LCG step deriving `last` (0..N_ACTIONS) into local l.last.
fn emit_last(s: &mut InstructionSink, l: &Locals) {
    s.local_get(l.s)
        .i64_const(LCG_A as i64)
        .i64_mul()
        .i64_const(LCG_C as i64)
        .i64_add()
        .local_tee(l.s)
        .i64_const(32)
        .i64_shr_u()
        .i64_const(N_ACTIONS)
        .i64_rem_u()
        .local_set(l.last);
}

// memarg for an 8-byte aligned f64 access at base+offset in memory 0.
fn ma(offset: u64) -> MemArg {
    MemArg { offset, align: 3, memory_index: 0 }
}

// SHARED variant: route the freshly-generated inputs through the imported host heap — store
// x0..x3 to base+{0,8,16,24} then load them straight back into x0..x3. This exercises real
// bounds-checked stores+loads against the VM's linear memory (the "JIT'd code touching the heap"
// case) while keeping the kernel body byte-identical to the pure-compute variant.
fn emit_store_reload(s: &mut InstructionSink, l: &Locals) {
    s.local_get(l.base).local_get(l.x0).f64_store(ma(0));
    s.local_get(l.base).local_get(l.x1).f64_store(ma(8));
    s.local_get(l.base).local_get(l.x2).f64_store(ma(16));
    s.local_get(l.base).local_get(l.x3).f64_store(ma(24));
    s.local_get(l.base).f64_load(ma(0)).local_set(l.x0);
    s.local_get(l.base).f64_load(ma(8)).local_set(l.x1);
    s.local_get(l.base).f64_load(ma(16)).local_set(l.x2);
    s.local_get(l.base).f64_load(ma(24)).local_set(l.x3);
}

fn xc_local(l: &Locals, c: i64) -> u32 {
    match c {
        0 => l.x0,
        1 => l.x1,
        2 => l.x2,
        _ => l.x3,
    }
}

// LOOPED kernel: action loop `a` is a real wasm loop var (kind=(a+c)%3 is runtime -> branchless
// `select`); the 4 considerations are unrolled (fixed trip count, as a JIT would). Reads x0..x3.
fn emit_kernel_looped(s: &mut InstructionSink, l: &Locals) {
    s.i64_const(0).local_set(l.best);
    s.f64_const(f64c(-1.0)).local_set(l.best_v);
    s.i64_const(0).local_set(l.a);
    s.loop_(BlockType::Empty);
    s.f64_const(f64c(1.0)).local_set(l.prod);
    for c in 0..N_CONSID {
        let xc = xc_local(l, c);
        // kind = (a + c) % 3
        s.local_get(l.a).i64_const(c).i64_add().i64_const(3).i64_rem_u().local_set(l.kind);
        // prod *= cv ; cv = select(kind==0, x*x, select(kind==1, x, 1-x))
        s.local_get(l.prod);
        s.local_get(xc).local_get(xc).f64_mul(); // x*x
        s.local_get(xc); // x
        s.f64_const(f64c(1.0)).local_get(xc).f64_sub(); // 1 - x
        s.local_get(l.kind).i64_const(1).i64_eq();
        s.select(); // kind==1 ? x : 1-x
        s.local_get(l.kind).i64_const(0).i64_eq();
        s.select(); // cv
        s.f64_mul().local_set(l.prod);
    }
    // momentum: prod = (a==last) ? prod*1.25 : prod
    s.local_get(l.prod).f64_const(f64c(1.25)).f64_mul();
    s.local_get(l.prod);
    s.local_get(l.a).local_get(l.last).i64_eq();
    s.select();
    s.local_set(l.prod);
    // argmax
    s.local_get(l.best_v).local_get(l.prod).f64_lt().local_set(l.cmp);
    s.local_get(l.a).local_get(l.best).local_get(l.cmp).select().local_set(l.best);
    s.local_get(l.prod).local_get(l.best_v).local_get(l.cmp).select().local_set(l.best_v);
    // a++ ; continue while a < N_ACTIONS
    s.local_get(l.a).i64_const(1).i64_add().local_tee(l.a);
    s.i64_const(N_ACTIONS).i64_lt_u().br_if(0);
    s.end(); // end loop
}

// UNROLLED kernel: both loops unrolled, so kind=(a+c)%3 is a codegen-time constant and we emit
// ONLY the selected curve (no branch/select for kind) — exactly what an AOT-to-wasm compiler
// specialized to the known kernel shape would produce. Reads x0..x3.
fn emit_kernel_unrolled(s: &mut InstructionSink, l: &Locals) {
    s.i64_const(0).local_set(l.best);
    s.f64_const(f64c(-1.0)).local_set(l.best_v);
    for a in 0..N_ACTIONS {
        s.f64_const(f64c(1.0)).local_set(l.prod);
        for c in 0..N_CONSID {
            let xc = xc_local(l, c);
            let kind = (a + c) % 3;
            match kind {
                0 => {
                    // prod *= x*x
                    s.local_get(l.prod).local_get(xc).local_get(xc).f64_mul().f64_mul();
                }
                1 => {
                    // prod *= x
                    s.local_get(l.prod).local_get(xc).f64_mul();
                }
                _ => {
                    // prod *= (1 - x)
                    s.local_get(l.prod).f64_const(f64c(1.0)).local_get(xc).f64_sub().f64_mul();
                }
            }
            s.local_set(l.prod);
        }
        // momentum: prod = (a==last) ? prod*1.25 : prod  (a is constant; last is runtime)
        s.local_get(l.prod).f64_const(f64c(1.25)).f64_mul();
        s.local_get(l.prod);
        s.i64_const(a).local_get(l.last).i64_eq();
        s.select();
        s.local_set(l.prod);
        // argmax
        s.local_get(l.best_v).local_get(l.prod).f64_lt().local_set(l.cmp);
        s.i64_const(a).local_get(l.best).local_get(l.cmp).select().local_set(l.best);
        s.local_get(l.prod).local_get(l.best_v).local_get(l.cmp).select().local_set(l.best_v);
    }
}

/// Emit a complete module at runtime. `shared` => import host memory and round-trip inputs through
/// it; `unroll` => emit the fully-unrolled, kind-specialized kernel (vs the looped/select kernel).
pub fn emit_module_cfg(shared: bool, unroll: bool) -> Vec<u8> {
    let (l, decls, _np) = layout(shared);
    let mut module = Module::new();

    // ---- type section ----
    let mut types = TypeSection::new();
    if shared {
        types.ty().function([ValType::I32, ValType::I32, ValType::I32], [ValType::I64]);
    } else {
        types.ty().function([ValType::I32, ValType::I32], [ValType::I64]);
    }
    module.section(&types);

    // ---- import section (shared: import host linear memory) ----
    if shared {
        let mut imports = ImportSection::new();
        imports.import(
            "env",
            "memory",
            MemoryType { minimum: 1, maximum: None, memory64: false, shared: false, page_size_log2: None },
        );
        module.section(&imports);
    }

    // ---- function + export ----
    let mut funcs = FunctionSection::new();
    funcs.function(0);
    module.section(&funcs);
    let mut exports = ExportSection::new();
    exports.export(if shared { "run_shared" } else { "run_gen" }, ExportKind::Func, 0);
    module.section(&exports);

    // ---- code ----
    let mut func = Function::new(decls);
    {
        let mut s = func.instructions();
        s.i64_const(FNV_OFFSET as i64).local_set(l.chk);
        s.local_get(1).i64_extend_i32_u().local_set(l.s); // seed (u32) -> s
        s.i32_const(0).local_set(l.i);

        s.block(BlockType::Empty); // guard: skip if iters < 1
        s.local_get(l.iters).i32_const(1).i32_lt_s().br_if(0);
        {
            s.loop_(BlockType::Empty);
            {
                emit_input(&mut s, &l, l.x0);
                emit_input(&mut s, &l, l.x1);
                emit_input(&mut s, &l, l.x2);
                emit_input(&mut s, &l, l.x3);
                emit_last(&mut s, &l);
                if shared {
                    emit_store_reload(&mut s, &l);
                }
                if unroll {
                    emit_kernel_unrolled(&mut s, &l);
                } else {
                    emit_kernel_looped(&mut s, &l);
                }
                // fold checksum: chk ^= best; chk *= PRIME; chk ^= bits(best_v); chk *= PRIME
                s.local_get(l.chk).local_get(l.best).i64_xor().local_set(l.chk);
                s.local_get(l.chk).i64_const(FNV_PRIME as i64).i64_mul().local_set(l.chk);
                s.local_get(l.chk).local_get(l.best_v).i64_reinterpret_f64().i64_xor().local_set(l.chk);
                s.local_get(l.chk).i64_const(FNV_PRIME as i64).i64_mul().local_set(l.chk);
                // i++ ; continue while i < iters
                s.local_get(l.i).i32_const(1).i32_add().local_tee(l.i);
                s.local_get(l.iters).i32_lt_s().br_if(0);
                s.end(); // end outer loop
            }
            s.end(); // end guard block
        }
        s.local_get(l.chk);
        s.end(); // end function
    }
    let mut code = CodeSection::new();
    code.function(&func);
    module.section(&code);
    module.finish()
}

/// Default emitter used by the harness: looped kernel (naive runtime-emit), pure-compute or shared.
pub fn emit_module(shared: bool) -> Vec<u8> {
    emit_module_cfg(shared, false)
}

// =====================================================================================
// wasm exports for the in-V8 harness.
// =====================================================================================

static mut GEN: Vec<u8> = Vec::new();
static mut SCRATCH: [f64; 16] = [0.0; 16];

#[no_mangle]
pub extern "C" fn run_interp(iters: i32, seed: i32) -> u64 {
    run_interp_impl(iters, seed)
}

#[no_mangle]
pub extern "C" fn run_aot(iters: i32, seed: i32) -> u64 {
    run_aot_impl(iters, seed)
}

#[inline(always)]
fn stash(bytes: Vec<u8>) -> u64 {
    unsafe {
        GEN = bytes;
        ((GEN.as_ptr() as u64) << 32) | (GEN.len() as u64)
    }
}

/// Generate the looped run_gen module at runtime; stash bytes; return (ptr<<32 | len).
#[no_mangle]
pub extern "C" fn codegen_run() -> u64 {
    stash(emit_module_cfg(false, false))
}

/// Generate the UNROLLED/kind-specialized run_gen module (AOT-to-wasm style) at runtime.
#[no_mangle]
pub extern "C" fn codegen_run_opt() -> u64 {
    stash(emit_module_cfg(false, true))
}

/// Generate the looped shared-heap module at runtime; stash bytes; return (ptr<<32 | len).
#[no_mangle]
pub extern "C" fn codegen_shared() -> u64 {
    stash(emit_module_cfg(true, false))
}

/// Generate the UNROLLED shared-heap module at runtime.
#[no_mangle]
pub extern "C" fn codegen_shared_opt() -> u64 {
    stash(emit_module_cfg(true, true))
}

/// Pointer to a host-heap scratch buffer (8-aligned [f64;16]) for the shared-heap variant.
#[no_mangle]
pub extern "C" fn scratch_ptr() -> u32 {
    unsafe { SCRATCH.as_ptr() as u32 }
}
