//! Minimal register-bytecode interpreter — Spike B dispatch-tax benchmark.
//!
//! Goal: measure the per-dispatched-instruction cost of a portable `match`-in-a-loop
//! register VM (the dispatch style that lowers to `br_table` on wasm and a jump table
//! on native), comparing native vs wasm-in-V8.
//!
//! The program executed is a tight fixed-point arithmetic loop with a call/return,
//! shaped like the grabby AI kernel (IAUS utility selection + SARSA Q16.16): load-const,
//! move, int add/sub/mul, fixed-point mul (>>16), compare, conditional branch, jump,
//! call, return.

#![allow(static_mut_refs)]

// ---- opcodes ----
const OP_HALT: u8 = 0;
const OP_LOADK: u8 = 1; // r[a] = c
const OP_MOV: u8 = 2; // r[a] = r[b]
const OP_ADD: u8 = 3; // r[a] = r[b] + r[c8]
const OP_SUB: u8 = 4; // r[a] = r[b] - r[c8]
const OP_MUL: u8 = 5; // r[a] = r[b] * r[c8]
const OP_MULFX: u8 = 6; // r[a] = (r[b] * r[c8]) >> 16   (Q16.16)
const OP_LT: u8 = 7; // r[a] = (r[b] < r[c8]) ? 1 : 0
const OP_JMPZ: u8 = 8; // if r[a]==0 pc = c
const OP_JMP: u8 = 9; // pc = c
const OP_CALL: u8 = 10; // push pc; pc = c
const OP_RET: u8 = 11; // pc = pop
const OP_ADDK: u8 = 12; // r[a] = r[b] + c

#[derive(Clone, Copy)]
struct Instr {
    op: u8,
    a: u8,
    b: u8,
    c8: u8, // third register operand (for 3-reg ops)
    c: i32, // immediate / jump target
}

#[inline(always)]
fn i(op: u8, a: u8, b: u8, c8: u8, c: i32) -> Instr {
    Instr { op, a, b, c8, c }
}

const LOOP_N: i32 = 1000;

/// Hand-assembled program. Returns (code, entry_pc).
fn build_program() -> Vec<Instr> {
    // register map:
    //   r0 = checksum accumulator
    //   r1 = loop counter i
    //   r2 = loop limit
    //   r3 = const 7
    //   r4 = const 3
    //   r5 = compare result
    //   r6 = scratch
    //   r7 = scratch
    //
    // addresses (must match jump targets below):
    //   LOOP_TOP = 5, END = 14, FUNC = 15
    const LOOP_TOP: i32 = 5;
    const END: i32 = 14;
    const FUNC: i32 = 15;
    vec![
        /* 0 */ i(OP_LOADK, 1, 0, 0, 0),         // i = 0
        /* 1 */ i(OP_LOADK, 2, 0, 0, LOOP_N),    // limit
        /* 2 */ i(OP_LOADK, 0, 0, 0, 0),         // acc = 0
        /* 3 */ i(OP_LOADK, 3, 0, 0, 7),         // const 7
        /* 4 */ i(OP_LOADK, 4, 0, 0, 3),         // const 3
        // LOOP_TOP = 5
        /* 5 */ i(OP_LT, 5, 1, 2, 0),            // r5 = i < limit
        /* 6 */ i(OP_JMPZ, 5, 0, 0, END),        // if !r5 -> END
        /* 7 */ i(OP_MUL, 6, 1, 4, 0),           // r6 = i * 3
        /* 8 */ i(OP_ADD, 6, 6, 3, 0),           // r6 = r6 + 7
        /* 9 */ i(OP_MULFX, 6, 6, 3, 0),         // r6 = (r6 * 7) >> 16
        /* 10 */ i(OP_ADD, 0, 0, 6, 0),          // acc += r6
        /* 11 */ i(OP_CALL, 0, 0, 0, FUNC),      // call helper
        /* 12 */ i(OP_ADDK, 1, 1, 0, 1),         // i += 1
        /* 13 */ i(OP_JMP, 0, 0, 0, LOOP_TOP),   // loop
        // END = 14
        /* 14 */ i(OP_HALT, 0, 0, 0, 0),
        // FUNC = 15
        /* 15 */ i(OP_ADDK, 0, 0, 0, 5),         // acc += 5
        /* 16 */ i(OP_SUB, 0, 0, 4, 0),          // acc -= 3
        /* 17 */ i(OP_MUL, 7, 0, 4, 0),          // r7 = acc * 3 (touch)
        /* 18 */ i(OP_RET, 0, 0, 0, 0),
    ]
}

/// Total instructions dispatched in the last run of `bench` / `run_once`.
static mut LAST_FUEL: u64 = 0;

/// Run the program once. Returns checksum; records dispatched-instruction count in LAST_FUEL.
#[inline(never)]
fn run_once(code: &[Instr]) -> i64 {
    let mut reg = [0i64; 16];
    let mut stack = [0i32; 64];
    let mut sp: usize = 0;
    let mut pc: usize = 0;
    let mut fuel: u64 = 0;

    loop {
        let ins = unsafe { *code.get_unchecked(pc) };
        fuel += 1; // deterministic instruction-fuel accounting (representative)
        pc += 1;
        match ins.op {
            OP_HALT => break,
            OP_LOADK => {
                reg[ins.a as usize] = ins.c as i64;
            }
            OP_MOV => {
                reg[ins.a as usize] = reg[ins.b as usize];
            }
            OP_ADD => {
                reg[ins.a as usize] =
                    reg[ins.b as usize].wrapping_add(reg[ins.c8 as usize]);
            }
            OP_SUB => {
                reg[ins.a as usize] =
                    reg[ins.b as usize].wrapping_sub(reg[ins.c8 as usize]);
            }
            OP_MUL => {
                reg[ins.a as usize] =
                    reg[ins.b as usize].wrapping_mul(reg[ins.c8 as usize]);
            }
            OP_MULFX => {
                let p = reg[ins.b as usize].wrapping_mul(reg[ins.c8 as usize]);
                reg[ins.a as usize] = p >> 16;
            }
            OP_LT => {
                reg[ins.a as usize] =
                    (reg[ins.b as usize] < reg[ins.c8 as usize]) as i64;
            }
            OP_JMPZ => {
                if reg[ins.a as usize] == 0 {
                    pc = ins.c as usize;
                }
            }
            OP_JMP => {
                pc = ins.c as usize;
            }
            OP_CALL => {
                stack[sp] = pc as i32;
                sp += 1;
                pc = ins.c as usize;
            }
            OP_RET => {
                sp -= 1;
                pc = stack[sp] as usize;
            }
            OP_ADDK => {
                reg[ins.a as usize] = reg[ins.b as usize].wrapping_add(ins.c as i64);
            }
            _ => break,
        }
    }
    unsafe {
        LAST_FUEL = fuel;
    }
    reg[0]
}

/// Run the fixed program `iters` times. Returns a checksum to prevent DCE.
#[no_mangle]
pub extern "C" fn bench(iters: u64) -> f64 {
    let code = build_program();
    let mut acc: i64 = 0;
    let mut total_fuel: u64 = 0;
    for _ in 0..iters {
        acc = acc.wrapping_add(run_once(&code));
        total_fuel = total_fuel.wrapping_add(unsafe { LAST_FUEL });
    }
    unsafe {
        LAST_FUEL = total_fuel;
    }
    acc as f64
}

/// Dispatched-instruction count from the most recent `bench` call (total across all iters).
#[no_mangle]
pub extern "C" fn last_fuel() -> f64 {
    unsafe { LAST_FUEL as f64 }
}

/// Dispatches per single program run (run once, return fuel).
#[no_mangle]
pub extern "C" fn dispatches_per_run() -> f64 {
    let code = build_program();
    let _ = run_once(&code);
    unsafe { LAST_FUEL as f64 }
}

// =====================================================================
// SECOND DISPATCH VARIANT: function-pointer-table ("call") threading.
// On native this lowers to indirect calls through a table; on wasm it
// lowers to `call_indirect` through a function table. Same program.
// =====================================================================

struct Vm<'a> {
    reg: [i64; 16],
    stack: [i32; 64],
    sp: usize,
    pc: usize,
    code: &'a [Instr],
    running: bool,
    fuel: u64,
}

type Handler = fn(&mut Vm, Instr);

fn h_halt(vm: &mut Vm, _: Instr) {
    vm.running = false;
}
fn h_loadk(vm: &mut Vm, ins: Instr) {
    vm.reg[ins.a as usize] = ins.c as i64;
}
fn h_mov(vm: &mut Vm, ins: Instr) {
    vm.reg[ins.a as usize] = vm.reg[ins.b as usize];
}
fn h_add(vm: &mut Vm, ins: Instr) {
    vm.reg[ins.a as usize] = vm.reg[ins.b as usize].wrapping_add(vm.reg[ins.c8 as usize]);
}
fn h_sub(vm: &mut Vm, ins: Instr) {
    vm.reg[ins.a as usize] = vm.reg[ins.b as usize].wrapping_sub(vm.reg[ins.c8 as usize]);
}
fn h_mul(vm: &mut Vm, ins: Instr) {
    vm.reg[ins.a as usize] = vm.reg[ins.b as usize].wrapping_mul(vm.reg[ins.c8 as usize]);
}
fn h_mulfx(vm: &mut Vm, ins: Instr) {
    let p = vm.reg[ins.b as usize].wrapping_mul(vm.reg[ins.c8 as usize]);
    vm.reg[ins.a as usize] = p >> 16;
}
fn h_lt(vm: &mut Vm, ins: Instr) {
    vm.reg[ins.a as usize] = (vm.reg[ins.b as usize] < vm.reg[ins.c8 as usize]) as i64;
}
fn h_jmpz(vm: &mut Vm, ins: Instr) {
    if vm.reg[ins.a as usize] == 0 {
        vm.pc = ins.c as usize;
    }
}
fn h_jmp(vm: &mut Vm, ins: Instr) {
    vm.pc = ins.c as usize;
}
fn h_call(vm: &mut Vm, ins: Instr) {
    vm.stack[vm.sp] = vm.pc as i32;
    vm.sp += 1;
    vm.pc = ins.c as usize;
}
fn h_ret(vm: &mut Vm, _: Instr) {
    vm.sp -= 1;
    vm.pc = vm.stack[vm.sp] as usize;
}
fn h_addk(vm: &mut Vm, ins: Instr) {
    vm.reg[ins.a as usize] = vm.reg[ins.b as usize].wrapping_add(ins.c as i64);
}

static HANDLERS: [Handler; 13] = [
    h_halt, h_loadk, h_mov, h_add, h_sub, h_mul, h_mulfx, h_lt, h_jmpz, h_jmp, h_call, h_ret,
    h_addk,
];

#[inline(never)]
fn run_once_threaded(code: &[Instr]) -> i64 {
    let mut vm = Vm {
        reg: [0i64; 16],
        stack: [0i32; 64],
        sp: 0,
        pc: 0,
        code,
        running: true,
        fuel: 0,
    };
    while vm.running {
        let ins = unsafe { *vm.code.get_unchecked(vm.pc) };
        vm.fuel += 1;
        vm.pc += 1;
        let h = unsafe { *HANDLERS.get_unchecked(ins.op as usize) };
        h(&mut vm, ins);
    }
    unsafe {
        LAST_FUEL = vm.fuel;
    }
    vm.reg[0]
}

#[no_mangle]
pub extern "C" fn bench_threaded(iters: u64) -> f64 {
    let code = build_program();
    let mut acc: i64 = 0;
    let mut total_fuel: u64 = 0;
    for _ in 0..iters {
        acc = acc.wrapping_add(run_once_threaded(&code));
        total_fuel = total_fuel.wrapping_add(unsafe { LAST_FUEL });
    }
    unsafe {
        LAST_FUEL = total_fuel;
    }
    acc as f64
}
