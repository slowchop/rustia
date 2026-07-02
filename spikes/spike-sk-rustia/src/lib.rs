//! rustia walking-skeleton: a from-scratch register-bytecode VM, measured on the grabby
//! real AI-kernel shape, to answer: does a from-scratch Rust VM hit the ~0.7-1.5us Luau-class
//! bar on native AND wasm?
//!
//! Mirrors ../spike-a-luau/src/main.rs (the Luau reference) and the
//! grabby ai_bakeoff kernel (lib.rs / koto_path.rs) EXACTLY for an apples-to-apples comparison:
//!   - World of n agents, Q16.16 state, ChaCha8 + fnv1a(id,tick) sub-streams.
//!   - per agent per tick: derive inputs, run the VM argmax (IAUS product+momentum), eps-greedy,
//!     reward, SARSA(lambda), input-drift, mood; then a contagion pass. All non-argmax math in Rust.
//!   - REALISTIC f64 argmax (perf proxy, matches grabby koto-float / spike-A realistic).
//!   - TIER-2 Q16.16 argmax with host fxmul/curve (matches grabby KotoSelector) -> golden hash.
//!
//! The VM uses a REAL NaN-boxed 8-byte Value (mask/shift encode/decode), one big match->br_table
//! dispatch loop, fixed-width instructions, a numeric-FOR superinstruction, and per-instruction fuel.

#![allow(static_mut_refs)]

use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};

// =====================================================================================
// Q16.16 fixed-point (verbatim from grabby ai_bakeoff/src/lib.rs)
// =====================================================================================

pub const ONE: i32 = 1 << 16;

#[inline]
pub fn fx_mul(a: i32, b: i32) -> i32 {
    (((a as i64) * (b as i64)) >> 16) as i32
}
#[inline]
pub fn clamp01(x: i32) -> i32 {
    x.clamp(0, ONE)
}

const LUT_N: usize = 256;
pub struct Luts {
    logistic: [i32; LUT_N + 1],
}
impl Luts {
    pub fn new() -> Self {
        let mut logistic = [0i32; LUT_N + 1];
        for (i, slot) in logistic.iter_mut().enumerate() {
            let x = -6.0 + 12.0 * (i as f64) / (LUT_N as f64);
            let y = 1.0 / (1.0 + libm::exp(-x));
            *slot = (y * ONE as f64 + 0.5) as i32;
        }
        Self { logistic }
    }
    #[inline]
    pub fn logistic(&self, x: i32) -> i32 {
        let x = clamp01(x);
        let pos = (x as i64) * (LUT_N as i64);
        let idx = (pos >> 16) as usize;
        let frac = (pos & 0xFFFF) as i32;
        let a = self.logistic[idx];
        let b = self.logistic[(idx + 1).min(LUT_N)];
        a + fx_mul(b - a, frac)
    }
}
impl Default for Luts {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
pub fn fnv1a(entity_id: u64, tick: u64) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in entity_id.to_le_bytes().iter().chain(tick.to_le_bytes().iter()) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub const N_ACTIONS: usize = 6;
pub const N_CONSIDERATIONS: usize = 4;
pub const N_NEIGHBORS: usize = 4;
const ALPHA: i32 = ONE / 10;
const GAMMA_LAMBDA: i32 = (ONE * 9) / 10;
const CONTAGION_K: i32 = ONE / 20;
const MOMENTUM: i32 = ONE + ONE / 4; // 1.25*ONE = 81920
const EPS_PPK: u32 = 51;

#[inline]
pub fn curve_kind(luts: &Luts, kind: usize, input: i32) -> i32 {
    match kind {
        0 => luts.logistic(input),
        1 => fx_mul(input, input),
        _ => clamp01(ONE - input),
    }
}

// =====================================================================================
// NaN-boxed 8-byte Value. f64 stored inline; tag space carved out of the quiet-NaN range.
//   number (f64):  any bit pattern where (bits & QNAN) != QNAN
//   tagged (bits & QNAN) == QNAN, sign bit clear:  low 3 bits = 1 nil / 2 false / 3 true / 4 int
//       int payload = i32 in bits [3..35)
//   heap handle:   sign bit set:  handle in low 48 bits  (not exercised on the hot path here)
// Real encode/decode (mask/shift) is paid on every type check / int (un)box.
// =====================================================================================

const QNAN: u64 = 0x7ffc_0000_0000_0000;
const SIGN: u64 = 0x8000_0000_0000_0000;
const TAG_NIL: u64 = QNAN | 1;
const TAG_FALSE: u64 = QNAN | 2;
const TAG_TRUE: u64 = QNAN | 3;
const TAG_INT_LO: u64 = QNAN | 4; // discriminator in low 3 bits; payload from bit 3

#[derive(Clone, Copy, PartialEq)]
pub struct Value(pub u64);

impl Value {
    #[inline(always)]
    pub fn num(x: f64) -> Value {
        Value(x.to_bits())
    }
    /// Canonicalizing number constructor (NaN -> fixed qNaN, -0.0 -> +0.0). Used only on the
    /// determinism path; the perf hot path uses `num` so it stays apples-to-apples with Luau/koto-float.
    #[inline(always)]
    pub fn num_canon(x: f64) -> Value {
        if x.is_nan() {
            Value(0x7ff8_0000_0000_0000)
        } else if x == 0.0 {
            Value(0.0f64.to_bits()) // collapse -0.0 -> +0.0
        } else {
            Value(x.to_bits())
        }
    }
    #[inline(always)]
    pub fn is_num(self) -> bool {
        (self.0 & QNAN) != QNAN
    }
    #[inline(always)]
    pub fn as_num(self) -> f64 {
        f64::from_bits(self.0)
    }
    #[inline(always)]
    pub fn nil() -> Value {
        Value(TAG_NIL)
    }
    #[inline(always)]
    pub fn boolean(b: bool) -> Value {
        Value(if b { TAG_TRUE } else { TAG_FALSE })
    }
    #[inline(always)]
    pub fn truthy(self) -> bool {
        // false and nil are falsey; everything else (incl. 0.0 and 0) is truthy (Lua semantics)
        self.0 != TAG_FALSE && self.0 != TAG_NIL
    }
    #[inline(always)]
    pub fn int(i: i32) -> Value {
        Value(TAG_INT_LO | ((i as u32 as u64) << 3))
    }
    #[inline(always)]
    pub fn as_int(self) -> i32 {
        ((self.0 >> 3) as u32) as i32
    }
    #[inline(always)]
    pub fn is_int(self) -> bool {
        (self.0 & QNAN) == QNAN && (self.0 & SIGN) == 0 && (self.0 & 7) == 4
    }
}

// =====================================================================================
// Bytecode: fixed-width 8-byte instructions, read straight in the dispatch loop.
//   op:u8  a:u8(dest reg)  b:u8(src reg)  c:u8(src reg / const idx)  aux:i32(jump target)
// =====================================================================================

#[derive(Clone, Copy)]
pub struct Instr {
    op: u8,
    a: u8,
    b: u8,
    c: u8,
    aux: i32,
}

// opcodes
const OP_HALT: u8 = 0;
const OP_LOADK: u8 = 1; // r[a] = K[c]
const OP_MOVE: u8 = 2; // r[a] = r[b]
const OP_NADD: u8 = 3; // r[a] = num(r[b]) + num(r[c])
const OP_NSUB: u8 = 4;
const OP_NMUL: u8 = 5;
const OP_NDIV: u8 = 6;
const OP_NMOD: u8 = 7;
const OP_NMODK: u8 = 8; // r[a] = num(r[b]) % K[c]
const OP_NMULK: u8 = 9; // r[a] = num(r[b]) * K[c]
const OP_NLT: u8 = 10; // r[a] = bool(num(r[b]) < num(r[c]))
const OP_NLE: u8 = 11;
const OP_NEQ: u8 = 12;
const OP_NEQK: u8 = 13; // r[a] = bool(num(r[b]) == K[c])
const OP_FORNPREP: u8 = 14; // numeric-for prep over r[a],r[a+1],r[a+2]; skip to aux if empty
const OP_FORNLOOP: u8 = 15; // numeric-for step+test; jump back to aux (body top) while continuing
const OP_AGET: u8 = 16; // r[a] = arr[ num(r[b]) as usize ]
const OP_JMP: u8 = 17; // pc = aux
const OP_JMPIFNOT: u8 = 18; // if !r[a].truthy() pc = aux
const OP_JMPIF: u8 = 19; // if r[a].truthy() pc = aux
const OP_RET: u8 = 20; // return r[a]
const OP_ILT: u8 = 21; // r[a] = bool(int(r[b]) < int(r[c]))
const OP_IEQ: u8 = 22;
const OP_CALL: u8 = 23; // r[a] = host[aux](r[b], r[c])   (aux: 0=fxmul, 1=curve)

const NR: usize = 24; // register file size
const NA: usize = 8; // VM-local array size (holds the IAUS inputs)

#[inline(always)]
fn ins(op: u8, a: u8, b: u8, c: u8, aux: i32) -> Instr {
    Instr { op, a, b, c, aux }
}

pub struct Program {
    pub code: Vec<Instr>,
    pub consts: Vec<Value>,
}

// =====================================================================================
// Tiny label-resolving assembler (no Luau parser needed; just the kernel bytecode).
// =====================================================================================

struct Asm {
    code: Vec<Instr>,
    consts: Vec<Value>,
    labels: std::collections::HashMap<&'static str, i32>,
    fixups: Vec<(usize, &'static str)>,
}
impl Asm {
    fn new() -> Self {
        Asm {
            code: Vec::new(),
            consts: Vec::new(),
            labels: std::collections::HashMap::new(),
            fixups: Vec::new(),
        }
    }
    fn k(&mut self, v: Value) -> u8 {
        // dedup constants
        for (i, &e) in self.consts.iter().enumerate() {
            if e == v {
                return i as u8;
            }
        }
        self.consts.push(v);
        (self.consts.len() - 1) as u8
    }
    fn emit(&mut self, op: u8, a: u8, b: u8, c: u8) {
        self.code.push(ins(op, a, b, c, 0));
    }
    fn emit_aux(&mut self, op: u8, a: u8, b: u8, c: u8, aux: i32) {
        self.code.push(ins(op, a, b, c, aux));
    }
    fn emit_j(&mut self, op: u8, a: u8, label: &'static str) {
        let idx = self.code.len();
        self.code.push(ins(op, a, 0, 0, -1));
        self.fixups.push((idx, label));
    }
    fn label(&mut self, name: &'static str) {
        self.labels.insert(name, self.code.len() as i32);
    }
    fn finish(mut self) -> Program {
        for (idx, name) in &self.fixups {
            let t = *self.labels.get(name).expect("undefined label");
            self.code[*idx].aux = t;
        }
        Program {
            code: self.code,
            consts: self.consts,
        }
    }
}

/// REALISTIC f64 argmax — mirrors spike-A `ARGMAX_SRC` (kind0=x*x, kind1=x, kind2=1-x, *1.25 momentum).
/// Calling convention: arr[0..4] = inputs (f64), r0 = last (f64). Returns best action in r1 (f64).
pub fn build_argmax_f64() -> Program {
    let mut m = Asm::new();
    let k0 = m.k(Value::num(0.0));
    let k5 = m.k(Value::num(5.0));
    let k1 = m.k(Value::num(1.0));
    let k3 = m.k(Value::num(3.0));
    let kneg1 = m.k(Value::num(-1.0));
    let k125 = m.k(Value::num(1.25));

    // r0=last r1=best r2=best_v  r3..r5=a-loop  r6=prod  r7..r9=c-loop
    // r10=x r11=kind r12=cv r13=cond
    m.emit(OP_LOADK, 1, 0, k0); // best = 0
    m.emit(OP_LOADK, 2, 0, kneg1); // best_v = -1
    m.emit(OP_LOADK, 3, 0, k0); // a = 0
    m.emit(OP_LOADK, 4, 0, k5); // a_limit = 5
    m.emit(OP_LOADK, 5, 0, k1); // a_step = 1
    m.emit_j(OP_FORNPREP, 3, "A_END");
    m.label("A_BODY");
    {
        m.emit(OP_LOADK, 6, 0, k1); // prod = 1
        m.emit(OP_LOADK, 7, 0, k0); // c = 0
        m.emit(OP_LOADK, 8, 0, k3); // c_limit = 3
        m.emit(OP_LOADK, 9, 0, k1); // c_step = 1
        m.emit_j(OP_FORNPREP, 7, "C_END");
        m.label("C_BODY");
        {
            m.emit(OP_AGET, 10, 7, 0); // x = arr[c]
            m.emit(OP_NADD, 11, 3, 7); // kind = a + c
            m.emit(OP_NMODK, 11, 11, k3); // kind %= 3
        }
        m.emit(OP_NEQK, 13, 11, k0); // r13 = (kind == 0)
        m.emit_j(OP_JMPIFNOT, 13, "NOT0");
        m.emit(OP_NMUL, 12, 10, 10); // cv = x*x
        m.emit_j(OP_JMP, 0, "HAVECV");
        m.label("NOT0");
        m.emit(OP_NEQK, 13, 11, k1); // r13 = (kind == 1)
        m.emit_j(OP_JMPIFNOT, 13, "ELSE");
        m.emit(OP_MOVE, 12, 10, 0); // cv = x
        m.emit_j(OP_JMP, 0, "HAVECV");
        m.label("ELSE");
        m.emit(OP_LOADK, 13, 0, k1); // 1.0
        m.emit(OP_NSUB, 12, 13, 10); // cv = 1.0 - x
        m.label("HAVECV");
        m.emit(OP_NMUL, 6, 6, 12); // prod *= cv
        m.emit_j(OP_FORNLOOP, 7, "C_BODY");
        m.label("C_END");
        m.emit(OP_NEQ, 13, 3, 0); // r13 = (a == last)
        m.emit_j(OP_JMPIFNOT, 13, "NOMOM");
        m.emit(OP_NMULK, 6, 6, k125); // prod *= 1.25
        m.label("NOMOM");
        m.emit(OP_NLT, 13, 2, 6); // r13 = (best_v < prod)
        m.emit_j(OP_JMPIFNOT, 13, "NOBEST");
        m.emit(OP_MOVE, 2, 6, 0); // best_v = prod
        m.emit(OP_MOVE, 1, 3, 0); // best = a
        m.label("NOBEST");
        m.emit_j(OP_FORNLOOP, 3, "A_BODY");
    }
    m.label("A_END");
    m.emit(OP_RET, 1, 0, 0);
    m.finish()
}

/// TIER-2 Q16.16 argmax — mirrors grabby `KotoSelector` SCRIPT (host fxmul + curve, same LUTs).
/// arr[0..4] = inputs (Q16.16 ints, boxed via Value::int), r0 = last (f64). Returns best in r1 (f64).
/// host: aux 0 = fxmul(a_int,b_int), aux 1 = curve(kind_num, x_int).
pub fn build_argmax_q16() -> Program {
    let mut m = Asm::new();
    let k0 = m.k(Value::num(0.0));
    let k5 = m.k(Value::num(5.0));
    let k1f = m.k(Value::num(1.0));
    let k3 = m.k(Value::num(3.0));
    let k_one = m.k(Value::int(ONE)); // 65536
    let k_negone = m.k(Value::int(-1)); // best_v sentinel
    let k_mom = m.k(Value::int(MOMENTUM)); // 81920

    // r0=last(f64) r1=best(f64) r2=best_v(int) r3..r5=a-loop(f64) r6=prod(int) r7..r9=c-loop(f64)
    // r10=x(int) r11=kind(f64) r12=cv(int) r13=cond r14=MOM(int)
    m.emit(OP_LOADK, 1, 0, k0); // best = 0
    m.emit(OP_LOADK, 2, 0, k_negone); // best_v = -1 (int)
    m.emit(OP_LOADK, 3, 0, k0); // a = 0
    m.emit(OP_LOADK, 4, 0, k5); // a_limit = 5
    m.emit(OP_LOADK, 5, 0, k1f); // a_step = 1
    m.emit_j(OP_FORNPREP, 3, "A_END");
    m.label("A_BODY");
    {
        m.emit(OP_LOADK, 6, 0, k_one); // prod = ONE (int)
        m.emit(OP_LOADK, 7, 0, k0); // c = 0
        m.emit(OP_LOADK, 8, 0, k3); // c_limit = 3
        m.emit(OP_LOADK, 9, 0, k1f); // c_step = 1
        m.emit_j(OP_FORNPREP, 7, "C_END");
        m.label("C_BODY");
        {
            m.emit(OP_AGET, 10, 7, 0); // x = arr[c]  (int)
            m.emit(OP_NADD, 11, 3, 7); // kind = a + c  (f64)
            m.emit(OP_NMODK, 11, 11, k3); // kind %= 3  (f64)
            m.emit_aux(OP_CALL, 12, 11, 10, 1); // cv = curve(kind, x)   host 1
            m.emit_aux(OP_CALL, 6, 6, 12, 0); // prod = fxmul(prod, cv) host 0
        }
        m.emit_j(OP_FORNLOOP, 7, "C_BODY");
        m.label("C_END");
        m.emit(OP_NEQ, 13, 3, 0); // r13 = (a == last)
        m.emit_j(OP_JMPIFNOT, 13, "NOMOM");
        m.emit(OP_LOADK, 14, 0, k_mom); // MOM (int)
        m.emit_aux(OP_CALL, 6, 6, 14, 0); // prod = fxmul(prod, MOM)
        m.label("NOMOM");
        m.emit(OP_ILT, 13, 2, 6); // r13 = (best_v < prod)  (int)
        m.emit_j(OP_JMPIFNOT, 13, "NOBEST");
        m.emit(OP_MOVE, 2, 6, 0); // best_v = prod
        m.emit(OP_MOVE, 1, 3, 0); // best = a
        m.label("NOBEST");
        m.emit_j(OP_FORNLOOP, 3, "A_BODY");
    }
    m.label("A_END");
    m.emit(OP_RET, 1, 0, 0);
    m.finish()
}

// =====================================================================================
// The VM: register file + local array + fuel. One big match -> br_table dispatch loop.
// =====================================================================================

pub struct Vm {
    pub reg: [Value; NR],
    pub arr: [Value; NA],
    pub fuel: u64,
}

impl Vm {
    pub fn new() -> Self {
        Vm {
            reg: [Value::nil(); NR],
            arr: [Value::nil(); NA],
            fuel: 0,
        }
    }

    /// Run `code` (with const pool `consts`) from pc 0. `FUEL` const-generic toggles the
    /// per-instruction deterministic fuel meter (decrement + trap-check) so we can size its cost.
    /// `fuel_budget` is the starting budget (prime it huge in the bench; it never exhausts).
    #[inline]
    pub fn run<const FUEL: bool>(
        &mut self,
        code: &[Instr],
        consts: &[Value],
        luts: &Luts,
        fuel_budget: u64,
    ) -> Value {
        let mut pc: usize = 0;
        let mut fuel: u64 = fuel_budget;
        let reg = &mut self.reg;
        let arr = &mut self.arr;
        loop {
            // fixed-width decode straight from the stream (no two-level decode)
            let i = unsafe { *code.get_unchecked(pc) };
            pc += 1;
            if FUEL {
                // deterministic instruction-fuel: decrement budget, trap on exhaustion.
                fuel = fuel.wrapping_sub(1);
                if fuel == 0 {
                    // never hit in the bench (budget is primed huge); the dec+cmp is the measured cost
                    return Value::nil();
                }
            }
            let a = i.a as usize;
            match i.op {
                OP_HALT => return Value::nil(),
                OP_LOADK => {
                    reg[a] = unsafe { *consts.get_unchecked(i.c as usize) };
                }
                OP_MOVE => {
                    reg[a] = reg[i.b as usize];
                }
                OP_NADD => {
                    reg[a] = Value::num(reg[i.b as usize].as_num() + reg[i.c as usize].as_num());
                }
                OP_NSUB => {
                    reg[a] = Value::num(reg[i.b as usize].as_num() - reg[i.c as usize].as_num());
                }
                OP_NMUL => {
                    reg[a] = Value::num(reg[i.b as usize].as_num() * reg[i.c as usize].as_num());
                }
                OP_NDIV => {
                    reg[a] = Value::num(reg[i.b as usize].as_num() / reg[i.c as usize].as_num());
                }
                OP_NMOD => {
                    reg[a] = Value::num(reg[i.b as usize].as_num() % reg[i.c as usize].as_num());
                }
                OP_NMODK => {
                    let k = unsafe { consts.get_unchecked(i.c as usize) }.as_num();
                    reg[a] = Value::num(reg[i.b as usize].as_num() % k);
                }
                OP_NMULK => {
                    let k = unsafe { consts.get_unchecked(i.c as usize) }.as_num();
                    reg[a] = Value::num(reg[i.b as usize].as_num() * k);
                }
                OP_NLT => {
                    reg[a] = Value::boolean(
                        reg[i.b as usize].as_num() < reg[i.c as usize].as_num(),
                    );
                }
                OP_NLE => {
                    reg[a] = Value::boolean(
                        reg[i.b as usize].as_num() <= reg[i.c as usize].as_num(),
                    );
                }
                OP_NEQ => {
                    reg[a] = Value::boolean(
                        reg[i.b as usize].as_num() == reg[i.c as usize].as_num(),
                    );
                }
                OP_NEQK => {
                    let k = unsafe { consts.get_unchecked(i.c as usize) }.as_num();
                    reg[a] = Value::boolean(reg[i.b as usize].as_num() == k);
                }
                OP_FORNPREP => {
                    let idx = reg[a].as_num();
                    let limit = reg[a + 1].as_num();
                    let step = reg[a + 2].as_num();
                    let skip = if step > 0.0 { idx > limit } else { idx < limit };
                    if skip {
                        pc = i.aux as usize;
                    }
                }
                OP_FORNLOOP => {
                    let step = reg[a + 2].as_num();
                    let idx = reg[a].as_num() + step;
                    reg[a] = Value::num(idx);
                    let limit = reg[a + 1].as_num();
                    let cont = if step > 0.0 { idx <= limit } else { idx >= limit };
                    if cont {
                        pc = i.aux as usize;
                    }
                }
                OP_AGET => {
                    let idx = reg[i.b as usize].as_num() as usize;
                    reg[a] = unsafe { *arr.get_unchecked(idx) };
                }
                OP_JMP => {
                    pc = i.aux as usize;
                }
                OP_JMPIFNOT => {
                    if !reg[a].truthy() {
                        pc = i.aux as usize;
                    }
                }
                OP_JMPIF => {
                    if reg[a].truthy() {
                        pc = i.aux as usize;
                    }
                }
                OP_RET => return reg[a],
                OP_ILT => {
                    reg[a] =
                        Value::boolean(reg[i.b as usize].as_int() < reg[i.c as usize].as_int());
                }
                OP_IEQ => {
                    reg[a] =
                        Value::boolean(reg[i.b as usize].as_int() == reg[i.c as usize].as_int());
                }
                OP_CALL => {
                    let v0 = reg[i.b as usize];
                    let v1 = reg[i.c as usize];
                    reg[a] = match i.aux {
                        0 => {
                            // fxmul(a,b) = (a*b)>>16, Q16.16 ints
                            let p = (v0.as_int() as i64) * (v1.as_int() as i64);
                            Value::int((p >> 16) as i32)
                        }
                        1 => {
                            // curve(kind, x)
                            Value::int(curve_kind(luts, v0.as_num() as usize, v1.as_int()))
                        }
                        _ => Value::nil(),
                    };
                }
                _ => return Value::nil(),
            }
        }
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

/// Count dispatched instructions for one run (report nicety; not on the perf path).
pub fn count_dispatches(prog: &Program, luts: &Luts, q16: bool) -> u64 {
    let mut vm = Vm::new();
    if q16 {
        for k in 0..4 {
            vm.arr[k] = Value::int(((k as i32 + 1) * 9000) % (ONE + 1));
        }
    } else {
        for k in 0..4 {
            vm.arr[k] = Value::num((k as f64 + 1.0) * 0.17);
        }
    }
    vm.reg[0] = Value::num(2.0);
    // run with a known huge budget and FUEL on; remaining is read off a clone of the loop
    // (we re-implement the count via a thin wrapper to avoid perturbing `run`).
    let budget: u64 = 1 << 40;
    vm.fuel = budget;
    let _ = vm.run_counting(&prog.code, &prog.consts, luts);
    budget - vm.fuel
}

impl Vm {
    /// Like `run` but counts dispatched instructions into `self.fuel` (decremented). For reports.
    pub fn run_counting(&mut self, code: &[Instr], consts: &[Value], luts: &Luts) -> Value {
        let mut pc: usize = 0;
        loop {
            let i = code[pc];
            pc += 1;
            self.fuel -= 1;
            let a = i.a as usize;
            match i.op {
                OP_HALT => return Value::nil(),
                OP_LOADK => self.reg[a] = consts[i.c as usize],
                OP_MOVE => self.reg[a] = self.reg[i.b as usize],
                OP_NADD => {
                    self.reg[a] =
                        Value::num(self.reg[i.b as usize].as_num() + self.reg[i.c as usize].as_num())
                }
                OP_NSUB => {
                    self.reg[a] =
                        Value::num(self.reg[i.b as usize].as_num() - self.reg[i.c as usize].as_num())
                }
                OP_NMUL => {
                    self.reg[a] =
                        Value::num(self.reg[i.b as usize].as_num() * self.reg[i.c as usize].as_num())
                }
                OP_NDIV => {
                    self.reg[a] =
                        Value::num(self.reg[i.b as usize].as_num() / self.reg[i.c as usize].as_num())
                }
                OP_NMOD => {
                    self.reg[a] =
                        Value::num(self.reg[i.b as usize].as_num() % self.reg[i.c as usize].as_num())
                }
                OP_NMODK => {
                    self.reg[a] =
                        Value::num(self.reg[i.b as usize].as_num() % consts[i.c as usize].as_num())
                }
                OP_NMULK => {
                    self.reg[a] =
                        Value::num(self.reg[i.b as usize].as_num() * consts[i.c as usize].as_num())
                }
                OP_NLT => {
                    self.reg[a] = Value::boolean(
                        self.reg[i.b as usize].as_num() < self.reg[i.c as usize].as_num(),
                    )
                }
                OP_NLE => {
                    self.reg[a] = Value::boolean(
                        self.reg[i.b as usize].as_num() <= self.reg[i.c as usize].as_num(),
                    )
                }
                OP_NEQ => {
                    self.reg[a] = Value::boolean(
                        self.reg[i.b as usize].as_num() == self.reg[i.c as usize].as_num(),
                    )
                }
                OP_NEQK => {
                    self.reg[a] = Value::boolean(
                        self.reg[i.b as usize].as_num() == consts[i.c as usize].as_num(),
                    )
                }
                OP_FORNPREP => {
                    let idx = self.reg[a].as_num();
                    let limit = self.reg[a + 1].as_num();
                    let step = self.reg[a + 2].as_num();
                    let skip = if step > 0.0 { idx > limit } else { idx < limit };
                    if skip {
                        pc = i.aux as usize;
                    }
                }
                OP_FORNLOOP => {
                    let step = self.reg[a + 2].as_num();
                    let idx = self.reg[a].as_num() + step;
                    self.reg[a] = Value::num(idx);
                    let limit = self.reg[a + 1].as_num();
                    let cont = if step > 0.0 { idx <= limit } else { idx >= limit };
                    if cont {
                        pc = i.aux as usize;
                    }
                }
                OP_AGET => {
                    let idx = self.reg[i.b as usize].as_num() as usize;
                    self.reg[a] = self.arr[idx];
                }
                OP_JMP => pc = i.aux as usize,
                OP_JMPIFNOT => {
                    if !self.reg[a].truthy() {
                        pc = i.aux as usize;
                    }
                }
                OP_JMPIF => {
                    if self.reg[a].truthy() {
                        pc = i.aux as usize;
                    }
                }
                OP_RET => return self.reg[a],
                OP_ILT => {
                    self.reg[a] = Value::boolean(
                        self.reg[i.b as usize].as_int() < self.reg[i.c as usize].as_int(),
                    )
                }
                OP_IEQ => {
                    self.reg[a] = Value::boolean(
                        self.reg[i.b as usize].as_int() == self.reg[i.c as usize].as_int(),
                    )
                }
                OP_CALL => {
                    let v0 = self.reg[i.b as usize];
                    let v1 = self.reg[i.c as usize];
                    self.reg[a] = match i.aux {
                        0 => {
                            let p = (v0.as_int() as i64) * (v1.as_int() as i64);
                            Value::int((p >> 16) as i32)
                        }
                        1 => Value::int(curve_kind(luts, v0.as_num() as usize, v1.as_int())),
                        _ => Value::nil(),
                    };
                }
                _ => return Value::nil(),
            }
        }
    }
}

// =====================================================================================
// The agent world (Q16.16 state) — verbatim shape from grabby ai_bakeoff, with VM-driven
// selection variants added. Non-argmax math (eps-greedy/SARSA/drift/mood/contagion) stays in Rust.
// =====================================================================================

pub struct World {
    pub n: usize,
    pub tick: u64,
    inputs: Vec<[i32; N_CONSIDERATIONS]>,
    q: Vec<[i32; N_ACTIONS]>,
    trace: Vec<[i32; N_ACTIONS]>,
    last_action: Vec<u8>,
    mood: Vec<i32>,
    neighbors: Vec<[u32; N_NEIGHBORS]>,
    mood_prev: Vec<i32>,
    pub luts: Luts,
}

impl World {
    pub fn new(n: usize, world_seed: u64) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(world_seed);
        let mut inputs = Vec::with_capacity(n);
        let mut neighbors = Vec::with_capacity(n);
        for i in 0..n {
            let mut inp = [0i32; N_CONSIDERATIONS];
            for c in inp.iter_mut() {
                *c = (rng.next_u32() % (ONE as u32 + 1)) as i32;
            }
            inputs.push(inp);
            let mut nb = [0u32; N_NEIGHBORS];
            for slot in nb.iter_mut() {
                *slot = if n > 1 { rng.next_u32() % n as u32 } else { 0 };
            }
            for slot in nb.iter_mut() {
                if *slot == i as u32 && n > 1 {
                    *slot = (*slot + 1) % n as u32;
                }
            }
            neighbors.push(nb);
        }
        World {
            n,
            tick: 0,
            inputs,
            q: vec![[0; N_ACTIONS]; n],
            trace: vec![[0; N_ACTIONS]; n],
            last_action: vec![0u8; n],
            mood: vec![0i32; n],
            neighbors,
            mood_prev: vec![0i32; n],
            luts: Luts::new(),
        }
    }

    #[inline]
    fn learn(&mut self, i: usize, action: u8, reward: i32) {
        let a = action as usize;
        for t in self.trace[i].iter_mut() {
            *t = fx_mul(*t, GAMMA_LAMBDA);
        }
        self.trace[i][a] = ONE;
        let delta = reward - self.q[i][a];
        let step = fx_mul(ALPHA, delta);
        for (qa, &ta) in self.q[i].iter_mut().zip(self.trace[i].iter()) {
            *qa += fx_mul(step, ta);
        }
    }

    #[inline]
    fn contagion(&mut self) {
        self.mood_prev.copy_from_slice(&self.mood);
        for i in 0..self.n {
            let me = self.mood_prev[i];
            let mut acc = 0i32;
            for &nb in &self.neighbors[i] {
                acc += self.mood_prev[nb as usize] - me;
            }
            self.mood[i] = (self.mood[i] + fx_mul(CONTAGION_K, acc)).clamp(-ONE, ONE);
        }
    }

    /// The Rust-native floor (anchor): verbatim from grabby lib.rs tick().
    pub fn tick_native(&mut self) {
        self.tick += 1;
        for i in 0..self.n {
            let inputs = self.inputs[i];
            let last = self.last_action[i];
            let mut rng = ChaCha8Rng::seed_from_u64(fnv1a(i as u64, self.tick));
            // native_scores
            let mut scores = [0i32; N_ACTIONS];
            for (a, s) in scores.iter_mut().enumerate() {
                let mut prod = ONE;
                for (c, &input) in inputs.iter().enumerate() {
                    prod = fx_mul(prod, curve_kind(&self.luts, (a + c) % 3, input));
                }
                *s = prod;
            }
            // argmax_momentum
            let mut s = scores;
            s[last as usize] = fx_mul(s[last as usize], MOMENTUM);
            let mut best = 0usize;
            let mut best_v = s[0];
            for (a, &v) in s.iter().enumerate().skip(1) {
                if v > best_v {
                    best_v = v;
                    best = a;
                }
            }
            let mut action = best as u8;
            if (rng.next_u32() & 1023) < EPS_PPK {
                action = (rng.next_u32() % N_ACTIONS as u32) as u8;
            }
            let reward = inputs[action as usize % N_CONSIDERATIONS];
            self.learn(i, action, reward);
            self.last_action[i] = action;
            let drift = (self.mood[i] >> 6) + ((reward - ONE / 2) >> 6);
            for c in self.inputs[i].iter_mut() {
                *c = clamp01(*c + drift);
            }
            self.mood[i] = (self.mood[i] + ((reward - ONE / 2) >> 4)).clamp(-ONE, ONE);
        }
        self.contagion();
    }

    /// REALISTIC: VM f64 argmax per agent per tick (mirrors spike-A tick_luau_realistic, but the
    /// call is a direct Rust call into the VM rather than an mlua FFI boundary).
    #[inline]
    pub fn tick_vm_realistic<const FUEL: bool>(&mut self, vm: &mut Vm, prog: &Program) {
        self.tick += 1;
        for i in 0..self.n {
            let inputs = self.inputs[i];
            let last = self.last_action[i];
            let mut rng = ChaCha8Rng::seed_from_u64(fnv1a(i as u64, self.tick));
            // marshal as f64 in [0,1] (matches grabby KotoFloatSelector / spike-A realistic)
            vm.arr[0] = Value::num(inputs[0] as f64 / 65536.0);
            vm.arr[1] = Value::num(inputs[1] as f64 / 65536.0);
            vm.arr[2] = Value::num(inputs[2] as f64 / 65536.0);
            vm.arr[3] = Value::num(inputs[3] as f64 / 65536.0);
            vm.reg[0] = Value::num(last as f64);
            // prime the deterministic fuel budget huge (never exhausts here)
            let best = vm.run::<FUEL>(&prog.code, &prog.consts, &self.luts, u64::MAX);
            let mut action = best.as_num() as u8;
            if (rng.next_u32() & 1023) < EPS_PPK {
                action = (rng.next_u32() % N_ACTIONS as u32) as u8;
            }
            let reward = inputs[action as usize % N_CONSIDERATIONS];
            self.learn(i, action, reward);
            self.last_action[i] = action;
            let drift = (self.mood[i] >> 6) + ((reward - ONE / 2) >> 6);
            for c in self.inputs[i].iter_mut() {
                *c = clamp01(*c + drift);
            }
            self.mood[i] = (self.mood[i] + ((reward - ONE / 2) >> 4)).clamp(-ONE, ONE);
        }
        self.contagion();
    }

    /// TIER-2 determinism: VM Q16.16 argmax (host fxmul/curve), everything else identical to native.
    /// Should reproduce the golden state_hash bit-for-bit.
    #[inline]
    pub fn tick_vm_q16(&mut self, vm: &mut Vm, prog: &Program) {
        self.tick += 1;
        for i in 0..self.n {
            let inputs = self.inputs[i];
            let last = self.last_action[i];
            let mut rng = ChaCha8Rng::seed_from_u64(fnv1a(i as u64, self.tick));
            vm.arr[0] = Value::int(inputs[0]);
            vm.arr[1] = Value::int(inputs[1]);
            vm.arr[2] = Value::int(inputs[2]);
            vm.arr[3] = Value::int(inputs[3]);
            vm.reg[0] = Value::num(last as f64);
            let best = vm.run::<false>(&prog.code, &prog.consts, &self.luts, u64::MAX);
            let mut action = best.as_num() as u8;
            // same RNG order as native: draw1 always; draw2 only when exploring
            if (rng.next_u32() & 1023) < EPS_PPK {
                action = (rng.next_u32() % N_ACTIONS as u32) as u8;
            }
            let reward = inputs[action as usize % N_CONSIDERATIONS];
            self.learn(i, action, reward);
            self.last_action[i] = action;
            let drift = (self.mood[i] >> 6) + ((reward - ONE / 2) >> 6);
            for c in self.inputs[i].iter_mut() {
                *c = clamp01(*c + drift);
            }
            self.mood[i] = (self.mood[i] + ((reward - ONE / 2) >> 4)).clamp(-ONE, ONE);
        }
        self.contagion();
    }

    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |v: i64| {
            for b in v.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        mix(self.tick as i64);
        for i in 0..self.n {
            for &q in &self.q[i] {
                mix(q as i64);
            }
            for &t in &self.trace[i] {
                mix(t as i64);
            }
            mix(self.last_action[i] as i64);
            mix(self.mood[i] as i64);
        }
        h
    }
}

// =====================================================================================
// wasm exports: setup a static World+VM+program, then run timed ticks. The JS runner times
// run() with performance.now() and divides by n*ticks. Mirrors the native harness.
// =====================================================================================

static mut W_WORLD: Option<World> = None;
static mut W_VM: Option<Vm> = None;
static mut W_PROG_F64: Option<Program> = None;
static mut W_PROG_Q16: Option<Program> = None;
static mut W_N: usize = 0;

/// Build a fresh world of `n` agents (seed 0xA1BACE50) + the VM + both programs. Returns n.
#[no_mangle]
pub extern "C" fn bench_setup(n: u32) -> u64 {
    unsafe {
        W_WORLD = Some(World::new(n as usize, 0xA1_BACE_50));
        W_VM = Some(Vm::new());
        W_PROG_F64 = Some(build_argmax_f64());
        W_PROG_Q16 = Some(build_argmax_q16());
        W_N = n as usize;
    }
    n as u64
}

/// Run `ticks` of the REALISTIC f64 VM path (fuel ON). Returns the state hash (checksum / anti-DCE).
#[no_mangle]
pub extern "C" fn bench_run_realistic(ticks: u32) -> u64 {
    unsafe {
        let w = W_WORLD.as_mut().unwrap();
        let vm = W_VM.as_mut().unwrap();
        let p = W_PROG_F64.as_ref().unwrap();
        for _ in 0..ticks {
            w.tick_vm_realistic::<true>(vm, p);
        }
        w.state_hash()
    }
}

/// Run `ticks` of the REALISTIC f64 VM path with fuel OFF (sizes the fuel-meter cost).
#[no_mangle]
pub extern "C" fn bench_run_realistic_nofuel(ticks: u32) -> u64 {
    unsafe {
        let w = W_WORLD.as_mut().unwrap();
        let vm = W_VM.as_mut().unwrap();
        let p = W_PROG_F64.as_ref().unwrap();
        for _ in 0..ticks {
            w.tick_vm_realistic::<false>(vm, p);
        }
        w.state_hash()
    }
}

/// Run `ticks` of the Rust-native floor (anchor in wasm).
#[no_mangle]
pub extern "C" fn bench_run_native(ticks: u32) -> u64 {
    unsafe {
        let w = W_WORLD.as_mut().unwrap();
        for _ in 0..ticks {
            w.tick_native();
        }
        w.state_hash()
    }
}

/// Number of agents in the current world (for the JS us/decision divisor).
#[no_mangle]
pub extern "C" fn bench_n() -> u64 {
    unsafe { W_N as u64 }
}

/// TIER-2 cross-arch determinism in wasm: fresh 4000-agent world, 200 ticks of the Q16.16 VM
/// argmax, return the state_hash. Must equal the golden 0x31c6ec2734ea6fa2 on every arch.
#[no_mangle]
pub extern "C" fn bench_q16_golden() -> u64 {
    let prog = build_argmax_q16();
    let mut vm = Vm::new();
    let mut w = World::new(4000, 0xA1_BACE_50);
    for _ in 0..200 {
        w.tick_vm_q16(&mut vm, &prog);
    }
    w.state_hash()
}
