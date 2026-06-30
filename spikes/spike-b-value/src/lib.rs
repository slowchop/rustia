//! Spike B (axis B, value representation): implement the HEAP path the walking-skeleton skipped —
//! a `Value` carrying insertion-ordered tables + interned strings — for the TWO leading layouts
//! (NaN-box-with-handle 8B vs 16-byte tagged) plus a 24-byte koto-style baseline, and microbench
//! table get/set, string ops, value clone/move, the IAUS kernel (canon-ON every f64 op), and the
//! NaN-box tag-space-vs-canonical-NaN soundness check. Native + wasm(node).
//!
//! Honest-measurement rules: data-dependent churned inputs, black_box, no constant-folding/DCE,
//! canon ON in the kernel hot path (the design flagged the canon-ON general-number cost UNMEASURED).
//!
//! Heap is modeled as handle-into-arena (Vec indexed by u32) for BOTH spiked layouts — exactly what
//! the design describes ("tagged handle in the NaN payload" vs "a Gc/handle word"). GC (axis D) is
//! out of scope; the Value-layout cost we isolate is encode/decode + sizeof + Copy-bandwidth, which
//! is independent of the collector.

#![allow(static_mut_refs)]

use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

// =====================================================================================
// Seedless FxHash (rustc-hash algorithm) — the design's chosen hasher. Deterministic, no seed.
// Table ITERATION order is insertion order (a Vec), independent of this hash, so cross-arch
// 32/64-bit hash differences never affect sim determinism.
// =====================================================================================

#[derive(Default)]
pub struct FxHasher {
    hash: u64,
}
const FXK: u64 = 0x51_7c_c1_b7_27_22_0a_95;
impl FxHasher {
    #[inline(always)]
    fn add(&mut self, i: u64) {
        self.hash = (self.hash.rotate_left(5) ^ i).wrapping_mul(FXK);
    }
}
impl Hasher for FxHasher {
    #[inline(always)]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline(always)]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    #[inline(always)]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }
    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.add(b as u64);
        }
    }
    #[inline(always)]
    fn finish(&self) -> u64 {
        self.hash
    }
}
type FxBuild = BuildHasherDefault<FxHasher>;

// =====================================================================================
// Table key: canonicalized. Number keys collapse -0.0 -> +0.0 (Luau table-key rule). Distinct
// tag spaces for number / string-handle so a numeric handle can't collide a number key.
// =====================================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Key {
    tag: u8,
    bits: u64,
}
impl std::hash::Hash for Key {
    #[inline(always)]
    fn hash<H: Hasher>(&self, h: &mut H) {
        h.write_u8(self.tag);
        h.write_u64(self.bits);
    }
}
impl Key {
    #[inline(always)]
    fn num(x: f64) -> Key {
        // -0.0 -> +0.0 ONLY at table-key normalization (per design). NaN keys excluded by caller.
        let x = if x == 0.0 { 0.0 } else { x };
        Key { tag: 0, bits: x.to_bits() }
    }
    #[inline(always)]
    fn int(i: i64) -> Key {
        Key { tag: 0, bits: (i as f64).to_bits() } // ints unify with numbers as Luau does
    }
    #[inline(always)]
    fn str(h: u32) -> Key {
        Key { tag: 1, bits: h as u64 }
    }
}

// =====================================================================================
// The heap. Shared, generic over the Value layout V so the ONLY thing that varies between
// benches is the Value encode/decode + sizeof. Interned strings + insertion-ordered tables.
// =====================================================================================

pub enum HeapKind {
    Str = 0,
    Table = 1,
    Closure = 2,
    Buffer = 3,
}

pub struct Table<V: Copy> {
    keys: Vec<Key>,
    vals: Vec<V>,
    index: HashMap<Key, u32, FxBuild>,
}
impl<V: Copy> Table<V> {
    fn new() -> Self {
        Table { keys: Vec::new(), vals: Vec::new(), index: HashMap::default() }
    }
    #[inline]
    fn get(&self, k: Key) -> Option<V> {
        self.index.get(&k).map(|&i| self.vals[i as usize])
    }
    #[inline]
    fn set(&mut self, k: Key, v: V) {
        match self.index.get(&k) {
            Some(&i) => self.vals[i as usize] = v,
            None => {
                let i = self.keys.len() as u32;
                self.keys.push(k);
                self.vals.push(v);
                self.index.insert(k, i);
            }
        }
    }
    fn len(&self) -> usize {
        self.keys.len()
    }
}

pub struct Heap<V: Copy> {
    strings: Vec<Box<str>>,
    intern: HashMap<u64, u32, FxBuild>, // fnv-of-bytes -> handle (interning)
    tables: Vec<Table<V>>,
}
impl<V: Copy> Heap<V> {
    pub fn new() -> Self {
        Heap { strings: Vec::new(), intern: HashMap::default(), tables: Vec::new() }
    }
    #[inline]
    pub fn intern(&mut self, s: &str) -> u32 {
        let mut hh: u64 = 0xcbf29ce484222325;
        for &b in s.as_bytes() {
            hh ^= b as u64;
            hh = hh.wrapping_mul(0x100000001b3);
        }
        if let Some(&h) = self.intern.get(&hh) {
            // (collision-free for this spike's corpus; real impl compares bytes)
            return h;
        }
        let h = self.strings.len() as u32;
        self.strings.push(s.into());
        self.intern.insert(hh, h);
        h
    }
    #[inline]
    pub fn str_bytes(&self, h: u32) -> &str {
        &self.strings[h as usize]
    }
    pub fn new_table(&mut self) -> u32 {
        let h = self.tables.len() as u32;
        self.tables.push(Table::new());
        h
    }
}
impl<V: Copy> Default for Heap<V> {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================================
// The Value-layout trait. Each layout implements encode/decode for the hot operations.
// =====================================================================================

pub trait VmValue: Copy + 'static {
    const NAME: &'static str;
    fn num(x: f64) -> Self;
    fn num_canon(x: f64) -> Self; // NaN -> one qNaN; -0.0 stays (per-op -0.0 canon is FORBIDDEN)
    fn as_num(self) -> f64;
    fn is_num(self) -> bool;
    fn int(i: i32) -> Self;
    fn as_int(self) -> i32;
    fn nil() -> Self;
    fn boolean(b: bool) -> Self;
    fn truthy(self) -> bool;
    fn str_handle(h: u32) -> Self;
    fn table_handle(h: u32) -> Self;
    fn as_handle(self) -> u32;
    fn is_str(self) -> bool;
    fn is_table(self) -> bool;
    /// raw bits for checksum / anti-DCE
    fn checksum(self) -> u64;
    /// table-key conversion (the masking cost of decoding the discriminant shows up here)
    #[inline(always)]
    fn to_key(self) -> Key {
        if self.is_str() {
            Key::str(self.as_handle())
        } else if self.is_num() {
            Key::num(self.as_num())
        } else {
            Key::int(self.as_int() as i64)
        }
    }
}

// -------------------------------------------------------------------------------------
// Layout 1: NaN-boxed 8-byte Value. f64 inline; tagged values live in the quiet-NaN range.
//   number:        (bits & QNAN) != QNAN
//   tagged (sign clear): low 3 bits 1=nil 2=false 3=true 4=int(payload bits 3..35)
//   heap   (sign set):   low 48 bits = handle, bits 48..50 = heap subtag (Str/Table/Closure/Buffer)
// -------------------------------------------------------------------------------------

const QNAN: u64 = 0x7ffc_0000_0000_0000;
const SIGN: u64 = 0x8000_0000_0000_0000;
const TAG_NIL: u64 = QNAN | 1;
const TAG_FALSE: u64 = QNAN | 2;
const TAG_TRUE: u64 = QNAN | 3;
const TAG_INT_LO: u64 = QNAN | 4;
const HEAP_MASK48: u64 = 0x0000_ffff_ffff_ffff;
const HEAP_SUB_STR: u64 = 0 << 48;
const HEAP_SUB_TABLE: u64 = 1 << 48;
/// The one canonical quiet-NaN every f64-producing op collapses NaN to.
pub const CANON_NAN: u64 = 0x7ff8_0000_0000_0000;

#[derive(Clone, Copy, PartialEq)]
pub struct NbValue(pub u64);

impl VmValue for NbValue {
    const NAME: &'static str = "nanbox-8B";
    #[inline(always)]
    fn num(x: f64) -> Self {
        NbValue(x.to_bits())
    }
    #[inline(always)]
    fn num_canon(x: f64) -> Self {
        // SOUNDNESS: collapse every NaN to CANON_NAN (bit 50 CLEAR) so no number aliases the tag
        // space (tag space requires bits 51 AND 50 set). -0.0 deliberately preserved.
        if x.is_nan() {
            NbValue(CANON_NAN)
        } else {
            NbValue(x.to_bits())
        }
    }
    #[inline(always)]
    fn as_num(self) -> f64 {
        f64::from_bits(self.0)
    }
    #[inline(always)]
    fn is_num(self) -> bool {
        (self.0 & QNAN) != QNAN
    }
    #[inline(always)]
    fn int(i: i32) -> Self {
        NbValue(TAG_INT_LO | ((i as u32 as u64) << 3))
    }
    #[inline(always)]
    fn as_int(self) -> i32 {
        ((self.0 >> 3) as u32) as i32
    }
    #[inline(always)]
    fn nil() -> Self {
        NbValue(TAG_NIL)
    }
    #[inline(always)]
    fn boolean(b: bool) -> Self {
        NbValue(if b { TAG_TRUE } else { TAG_FALSE })
    }
    #[inline(always)]
    fn truthy(self) -> bool {
        self.0 != TAG_FALSE && self.0 != TAG_NIL
    }
    #[inline(always)]
    fn str_handle(h: u32) -> Self {
        NbValue(SIGN | QNAN | HEAP_SUB_STR | h as u64)
    }
    #[inline(always)]
    fn table_handle(h: u32) -> Self {
        NbValue(SIGN | QNAN | HEAP_SUB_TABLE | h as u64)
    }
    #[inline(always)]
    fn as_handle(self) -> u32 {
        (self.0 & HEAP_MASK48) as u32
    }
    #[inline(always)]
    fn is_str(self) -> bool {
        (self.0 & SIGN) != 0 && (self.0 & QNAN) == QNAN && (self.0 & (3 << 48)) == HEAP_SUB_STR
    }
    #[inline(always)]
    fn is_table(self) -> bool {
        (self.0 & SIGN) != 0 && (self.0 & QNAN) == QNAN && (self.0 & (3 << 48)) == HEAP_SUB_TABLE
    }
    #[inline(always)]
    fn checksum(self) -> u64 {
        self.0
    }
}

// -------------------------------------------------------------------------------------
// Layout 2: 16-byte tagged union. Copy scalars + a payload word. No NaN-box masking — a separate
// tag byte is read directly. Costs 2x the register-file/Value memory bandwidth.
// -------------------------------------------------------------------------------------

pub const T_NUM: u8 = 0;
pub const T_INT: u8 = 1;
pub const T_NIL: u8 = 2;
pub const T_BOOL: u8 = 3;
pub const T_STR: u8 = 4;
pub const T_TABLE: u8 = 5;
pub const T_CLOSURE: u8 = 6;
pub const T_BUFFER: u8 = 7;

#[derive(Clone, Copy, PartialEq)]
pub struct TagValue {
    pub bits: u64, // f64 bits / i32 / handle / bool
    pub tag: u8,
}

impl VmValue for TagValue {
    const NAME: &'static str = "tagged-16B";
    #[inline(always)]
    fn num(x: f64) -> Self {
        TagValue { bits: x.to_bits(), tag: T_NUM }
    }
    #[inline(always)]
    fn num_canon(x: f64) -> Self {
        let b = if x.is_nan() { CANON_NAN } else { x.to_bits() };
        TagValue { bits: b, tag: T_NUM }
    }
    #[inline(always)]
    fn as_num(self) -> f64 {
        f64::from_bits(self.bits)
    }
    #[inline(always)]
    fn is_num(self) -> bool {
        self.tag == T_NUM
    }
    #[inline(always)]
    fn int(i: i32) -> Self {
        TagValue { bits: i as u32 as u64, tag: T_INT }
    }
    #[inline(always)]
    fn as_int(self) -> i32 {
        self.bits as u32 as i32
    }
    #[inline(always)]
    fn nil() -> Self {
        TagValue { bits: 0, tag: T_NIL }
    }
    #[inline(always)]
    fn boolean(b: bool) -> Self {
        TagValue { bits: b as u64, tag: T_BOOL }
    }
    #[inline(always)]
    fn truthy(self) -> bool {
        !(self.tag == T_NIL || (self.tag == T_BOOL && self.bits == 0))
    }
    #[inline(always)]
    fn str_handle(h: u32) -> Self {
        TagValue { bits: h as u64, tag: T_STR }
    }
    #[inline(always)]
    fn table_handle(h: u32) -> Self {
        TagValue { bits: h as u64, tag: T_TABLE }
    }
    #[inline(always)]
    fn as_handle(self) -> u32 {
        self.bits as u32
    }
    #[inline(always)]
    fn is_str(self) -> bool {
        self.tag == T_STR
    }
    #[inline(always)]
    fn is_table(self) -> bool {
        self.tag == T_TABLE
    }
    #[inline(always)]
    fn checksum(self) -> u64 {
        self.bits ^ ((self.tag as u64) << 56)
    }
}

// -------------------------------------------------------------------------------------
// Layout 3 (baseline): 24-byte koto-style fat value. Copy variant for sizeof/move comparison.
// (koto's real KValue is also NON-Copy — clone bumps an Arc; that tax is measured separately in
// the clone_arc bench, since the ~20% Arc clone/drop self-time was Spike D's finding.)
// -------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub struct E24 {
    pub tag: u8,
    pub a: u64,
    pub b: u64, // second word koto-class values carry (e.g. range start/end, ptr+meta)
}
impl VmValue for E24 {
    const NAME: &'static str = "fat-24B";
    #[inline(always)]
    fn num(x: f64) -> Self {
        E24 { tag: T_NUM, a: x.to_bits(), b: 0 }
    }
    #[inline(always)]
    fn num_canon(x: f64) -> Self {
        let bb = if x.is_nan() { CANON_NAN } else { x.to_bits() };
        E24 { tag: T_NUM, a: bb, b: 0 }
    }
    #[inline(always)]
    fn as_num(self) -> f64 {
        f64::from_bits(self.a)
    }
    #[inline(always)]
    fn is_num(self) -> bool {
        self.tag == T_NUM
    }
    #[inline(always)]
    fn int(i: i32) -> Self {
        E24 { tag: T_INT, a: i as u32 as u64, b: 0 }
    }
    #[inline(always)]
    fn as_int(self) -> i32 {
        self.a as u32 as i32
    }
    #[inline(always)]
    fn nil() -> Self {
        E24 { tag: T_NIL, a: 0, b: 0 }
    }
    #[inline(always)]
    fn boolean(b: bool) -> Self {
        E24 { tag: T_BOOL, a: b as u64, b: 0 }
    }
    #[inline(always)]
    fn truthy(self) -> bool {
        !(self.tag == T_NIL || (self.tag == T_BOOL && self.a == 0))
    }
    #[inline(always)]
    fn str_handle(h: u32) -> Self {
        E24 { tag: T_STR, a: h as u64, b: 0 }
    }
    #[inline(always)]
    fn table_handle(h: u32) -> Self {
        E24 { tag: T_TABLE, a: h as u64, b: 0 }
    }
    #[inline(always)]
    fn as_handle(self) -> u32 {
        self.a as u32
    }
    #[inline(always)]
    fn is_str(self) -> bool {
        self.tag == T_STR
    }
    #[inline(always)]
    fn is_table(self) -> bool {
        self.tag == T_TABLE
    }
    #[inline(always)]
    fn checksum(self) -> u64 {
        self.a ^ self.b ^ ((self.tag as u64) << 56)
    }
}

// =====================================================================================
// Q16.16 + LUTs + fnv1a (verbatim from the skeleton, for the golden + kernel).
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
const MOMENTUM: i32 = ONE + ONE / 4;
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
// Bytecode + assembler (generic over V). Verbatim opcode shape from the skeleton.
// =====================================================================================

#[derive(Clone, Copy)]
pub struct Instr {
    op: u8,
    a: u8,
    b: u8,
    c: u8,
    aux: i32,
}
const OP_HALT: u8 = 0;
const OP_LOADK: u8 = 1;
const OP_MOVE: u8 = 2;
const OP_NADD: u8 = 3;
const OP_NSUB: u8 = 4;
const OP_NMUL: u8 = 5;
const OP_NDIV: u8 = 6;
const OP_NMOD: u8 = 7;
const OP_NMODK: u8 = 8;
const OP_NMULK: u8 = 9;
const OP_NLT: u8 = 10;
const OP_NLE: u8 = 11;
const OP_NEQ: u8 = 12;
const OP_NEQK: u8 = 13;
const OP_FORNPREP: u8 = 14;
const OP_FORNLOOP: u8 = 15;
const OP_AGET: u8 = 16;
const OP_JMP: u8 = 17;
const OP_JMPIFNOT: u8 = 18;
const OP_JMPIF: u8 = 19;
const OP_RET: u8 = 20;
const OP_ILT: u8 = 21;
const OP_IEQ: u8 = 22;
const OP_CALL: u8 = 23;

const NR: usize = 24;
const NA: usize = 8;

#[inline(always)]
fn ins(op: u8, a: u8, b: u8, c: u8, aux: i32) -> Instr {
    Instr { op, a, b, c, aux }
}

pub struct Program<V: Copy> {
    pub code: Vec<Instr>,
    pub consts: Vec<V>,
}

struct Asm<V: VmValue> {
    code: Vec<Instr>,
    consts: Vec<V>,
    labels: HashMap<&'static str, i32>,
    fixups: Vec<(usize, &'static str)>,
}
impl<V: VmValue + PartialEq> Asm<V> {
    fn new() -> Self {
        Asm { code: Vec::new(), consts: Vec::new(), labels: HashMap::new(), fixups: Vec::new() }
    }
    fn k(&mut self, v: V) -> u8 {
        for (i, e) in self.consts.iter().enumerate() {
            if e.checksum() == v.checksum() {
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
    fn finish(mut self) -> Program<V> {
        for (idx, name) in &self.fixups {
            let t = *self.labels.get(name).expect("undefined label");
            self.code[*idx].aux = t;
        }
        Program { code: self.code, consts: self.consts }
    }
}

/// REALISTIC f64 argmax (mirrors the skeleton build_argmax_f64).
pub fn build_argmax_f64<V: VmValue + PartialEq>() -> Program<V> {
    let mut m = Asm::<V>::new();
    let k0 = m.k(V::num(0.0));
    let k5 = m.k(V::num(5.0));
    let k1 = m.k(V::num(1.0));
    let k3 = m.k(V::num(3.0));
    let kneg1 = m.k(V::num(-1.0));
    let k125 = m.k(V::num(1.25));
    m.emit(OP_LOADK, 1, 0, k0);
    m.emit(OP_LOADK, 2, 0, kneg1);
    m.emit(OP_LOADK, 3, 0, k0);
    m.emit(OP_LOADK, 4, 0, k5);
    m.emit(OP_LOADK, 5, 0, k1);
    m.emit_j(OP_FORNPREP, 3, "A_END");
    m.label("A_BODY");
    {
        m.emit(OP_LOADK, 6, 0, k1);
        m.emit(OP_LOADK, 7, 0, k0);
        m.emit(OP_LOADK, 8, 0, k3);
        m.emit(OP_LOADK, 9, 0, k1);
        m.emit_j(OP_FORNPREP, 7, "C_END");
        m.label("C_BODY");
        {
            m.emit(OP_AGET, 10, 7, 0);
            m.emit(OP_NADD, 11, 3, 7);
            m.emit(OP_NMODK, 11, 11, k3);
        }
        m.emit(OP_NEQK, 13, 11, k0);
        m.emit_j(OP_JMPIFNOT, 13, "NOT0");
        m.emit(OP_NMUL, 12, 10, 10);
        m.emit_j(OP_JMP, 0, "HAVECV");
        m.label("NOT0");
        m.emit(OP_NEQK, 13, 11, k1);
        m.emit_j(OP_JMPIFNOT, 13, "ELSE");
        m.emit(OP_MOVE, 12, 10, 0);
        m.emit_j(OP_JMP, 0, "HAVECV");
        m.label("ELSE");
        m.emit(OP_LOADK, 13, 0, k1);
        m.emit(OP_NSUB, 12, 13, 10);
        m.label("HAVECV");
        m.emit(OP_NMUL, 6, 6, 12);
        m.emit_j(OP_FORNLOOP, 7, "C_BODY");
        m.label("C_END");
        m.emit(OP_NEQ, 13, 3, 0);
        m.emit_j(OP_JMPIFNOT, 13, "NOMOM");
        m.emit(OP_NMULK, 6, 6, k125);
        m.label("NOMOM");
        m.emit(OP_NLT, 13, 2, 6);
        m.emit_j(OP_JMPIFNOT, 13, "NOBEST");
        m.emit(OP_MOVE, 2, 6, 0);
        m.emit(OP_MOVE, 1, 3, 0);
        m.label("NOBEST");
        m.emit_j(OP_FORNLOOP, 3, "A_BODY");
    }
    m.label("A_END");
    m.emit(OP_RET, 1, 0, 0);
    m.finish()
}

/// TIER-2 Q16.16 argmax (host fxmul/curve) for the golden.
pub fn build_argmax_q16<V: VmValue + PartialEq>() -> Program<V> {
    let mut m = Asm::<V>::new();
    let k0 = m.k(V::num(0.0));
    let k5 = m.k(V::num(5.0));
    let k1f = m.k(V::num(1.0));
    let k3 = m.k(V::num(3.0));
    let k_one = m.k(V::int(ONE));
    let k_negone = m.k(V::int(-1));
    let k_mom = m.k(V::int(MOMENTUM));
    m.emit(OP_LOADK, 1, 0, k0);
    m.emit(OP_LOADK, 2, 0, k_negone);
    m.emit(OP_LOADK, 3, 0, k0);
    m.emit(OP_LOADK, 4, 0, k5);
    m.emit(OP_LOADK, 5, 0, k1f);
    m.emit_j(OP_FORNPREP, 3, "A_END");
    m.label("A_BODY");
    {
        m.emit(OP_LOADK, 6, 0, k_one);
        m.emit(OP_LOADK, 7, 0, k0);
        m.emit(OP_LOADK, 8, 0, k3);
        m.emit(OP_LOADK, 9, 0, k1f);
        m.emit_j(OP_FORNPREP, 7, "C_END");
        m.label("C_BODY");
        {
            m.emit(OP_AGET, 10, 7, 0);
            m.emit(OP_NADD, 11, 3, 7);
            m.emit(OP_NMODK, 11, 11, k3);
            m.emit_aux(OP_CALL, 12, 11, 10, 1);
            m.emit_aux(OP_CALL, 6, 6, 12, 0);
        }
        m.emit_j(OP_FORNLOOP, 7, "C_BODY");
        m.label("C_END");
        m.emit(OP_NEQ, 13, 3, 0);
        m.emit_j(OP_JMPIFNOT, 13, "NOMOM");
        m.emit(OP_LOADK, 14, 0, k_mom);
        m.emit_aux(OP_CALL, 6, 6, 14, 0);
        m.label("NOMOM");
        m.emit(OP_ILT, 13, 2, 6);
        m.emit_j(OP_JMPIFNOT, 13, "NOBEST");
        m.emit(OP_MOVE, 2, 6, 0);
        m.emit(OP_MOVE, 1, 3, 0);
        m.label("NOBEST");
        m.emit_j(OP_FORNLOOP, 3, "A_BODY");
    }
    m.label("A_END");
    m.emit(OP_RET, 1, 0, 0);
    m.finish()
}

// =====================================================================================
// Generic VM. const CANON toggles per-op NaN canonicalization on every f64-producing op.
// =====================================================================================

pub struct Vm<V: VmValue> {
    pub reg: [V; NR],
    pub arr: [V; NA],
}
impl<V: VmValue> Vm<V> {
    pub fn new() -> Self {
        Vm { reg: [V::nil(); NR], arr: [V::nil(); NA] }
    }
    /// num-result helper: with CANON, route through num_canon (soundness + determinism).
    #[inline(always)]
    fn mknum<const CANON: bool>(x: f64) -> V {
        if CANON {
            V::num_canon(x)
        } else {
            V::num(x)
        }
    }
    #[inline]
    pub fn run<const FUEL: bool, const CANON: bool>(
        &mut self,
        code: &[Instr],
        consts: &[V],
        luts: &Luts,
        fuel_budget: u64,
    ) -> V {
        let mut pc: usize = 0;
        let mut fuel: u64 = fuel_budget;
        let reg = &mut self.reg;
        let arr = &mut self.arr;
        loop {
            let i = unsafe { *code.get_unchecked(pc) };
            pc += 1;
            if FUEL {
                fuel = fuel.wrapping_sub(1);
                if fuel == 0 {
                    return V::nil();
                }
            }
            let a = i.a as usize;
            match i.op {
                OP_HALT => return V::nil(),
                OP_LOADK => reg[a] = unsafe { *consts.get_unchecked(i.c as usize) },
                OP_MOVE => reg[a] = reg[i.b as usize],
                OP_NADD => {
                    reg[a] = Self::mknum::<CANON>(
                        reg[i.b as usize].as_num() + reg[i.c as usize].as_num(),
                    )
                }
                OP_NSUB => {
                    reg[a] = Self::mknum::<CANON>(
                        reg[i.b as usize].as_num() - reg[i.c as usize].as_num(),
                    )
                }
                OP_NMUL => {
                    reg[a] = Self::mknum::<CANON>(
                        reg[i.b as usize].as_num() * reg[i.c as usize].as_num(),
                    )
                }
                OP_NDIV => {
                    reg[a] = Self::mknum::<CANON>(
                        reg[i.b as usize].as_num() / reg[i.c as usize].as_num(),
                    )
                }
                OP_NMOD => {
                    let x = reg[i.b as usize].as_num();
                    let y = reg[i.c as usize].as_num();
                    // Luau floored modulo
                    reg[a] = Self::mknum::<CANON>(x - (x / y).floor() * y);
                }
                OP_NMODK => {
                    let k = unsafe { consts.get_unchecked(i.c as usize) }.as_num();
                    let x = reg[i.b as usize].as_num();
                    reg[a] = Self::mknum::<CANON>(x - (x / k).floor() * k);
                }
                OP_NMULK => {
                    let k = unsafe { consts.get_unchecked(i.c as usize) }.as_num();
                    reg[a] = Self::mknum::<CANON>(reg[i.b as usize].as_num() * k);
                }
                OP_NLT => {
                    reg[a] =
                        V::boolean(reg[i.b as usize].as_num() < reg[i.c as usize].as_num())
                }
                OP_NLE => {
                    reg[a] =
                        V::boolean(reg[i.b as usize].as_num() <= reg[i.c as usize].as_num())
                }
                OP_NEQ => {
                    reg[a] =
                        V::boolean(reg[i.b as usize].as_num() == reg[i.c as usize].as_num())
                }
                OP_NEQK => {
                    let k = unsafe { consts.get_unchecked(i.c as usize) }.as_num();
                    reg[a] = V::boolean(reg[i.b as usize].as_num() == k);
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
                    reg[a] = Self::mknum::<CANON>(idx);
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
                OP_JMP => pc = i.aux as usize,
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
                    reg[a] = V::boolean(reg[i.b as usize].as_int() < reg[i.c as usize].as_int())
                }
                OP_IEQ => {
                    reg[a] = V::boolean(reg[i.b as usize].as_int() == reg[i.c as usize].as_int())
                }
                OP_CALL => {
                    let v0 = reg[i.b as usize];
                    let v1 = reg[i.c as usize];
                    reg[a] = match i.aux {
                        0 => {
                            let p = (v0.as_int() as i64) * (v1.as_int() as i64);
                            V::int((p >> 16) as i32)
                        }
                        1 => V::int(curve_kind(luts, v0.as_num() as usize, v1.as_int())),
                        _ => V::nil(),
                    };
                }
                _ => return V::nil(),
            }
        }
    }
}
impl<V: VmValue> Default for Vm<V> {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================================
// The agent world (verbatim shape from the skeleton). Generic VM call for the kernel.
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
    pub fn tick_native(&mut self) {
        self.tick += 1;
        for i in 0..self.n {
            let inputs = self.inputs[i];
            let last = self.last_action[i];
            let mut rng = ChaCha8Rng::seed_from_u64(fnv1a(i as u64, self.tick));
            let mut scores = [0i32; N_ACTIONS];
            for (a, s) in scores.iter_mut().enumerate() {
                let mut prod = ONE;
                for (c, &input) in inputs.iter().enumerate() {
                    prod = fx_mul(prod, curve_kind(&self.luts, (a + c) % 3, input));
                }
                *s = prod;
            }
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
    #[inline]
    pub fn tick_vm_realistic<V: VmValue, const FUEL: bool, const CANON: bool>(
        &mut self,
        vm: &mut Vm<V>,
        prog: &Program<V>,
    ) {
        self.tick += 1;
        for i in 0..self.n {
            let inputs = self.inputs[i];
            let last = self.last_action[i];
            let mut rng = ChaCha8Rng::seed_from_u64(fnv1a(i as u64, self.tick));
            vm.arr[0] = V::num(inputs[0] as f64 / 65536.0);
            vm.arr[1] = V::num(inputs[1] as f64 / 65536.0);
            vm.arr[2] = V::num(inputs[2] as f64 / 65536.0);
            vm.arr[3] = V::num(inputs[3] as f64 / 65536.0);
            vm.reg[0] = V::num(last as f64);
            let best = vm.run::<FUEL, CANON>(&prog.code, &prog.consts, &self.luts, u64::MAX);
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
    #[inline]
    pub fn tick_vm_q16<V: VmValue>(&mut self, vm: &mut Vm<V>, prog: &Program<V>) {
        self.tick += 1;
        for i in 0..self.n {
            let inputs = self.inputs[i];
            let last = self.last_action[i];
            let mut rng = ChaCha8Rng::seed_from_u64(fnv1a(i as u64, self.tick));
            vm.arr[0] = V::int(inputs[0]);
            vm.arr[1] = V::int(inputs[1]);
            vm.arr[2] = V::int(inputs[2]);
            vm.arr[3] = V::int(inputs[3]);
            vm.reg[0] = V::num(last as f64);
            let best = vm.run::<false, false>(&prog.code, &prog.consts, &self.luts, u64::MAX);
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
// HEAP MICROBENCHES (generic over V). Each returns a checksum (anti-DCE). Callers time them.
// Inputs are churned so nothing constant-folds; values depend on prior iterations.
// =====================================================================================

const STR_CORPUS: &[&str] = &[
    "health", "stamina", "hunger", "threat", "distance", "mood", "ammo", "cover",
    "aggression", "fear", "curiosity", "fatigue", "morale", "supply", "range", "speed",
];

/// Build a table populated with the corpus string keys + 32 integer keys. Returns (heap, table).
fn build_struct_table<V: VmValue>(heap: &mut Heap<V>) -> u32 {
    let t = heap.new_table();
    for (i, s) in STR_CORPUS.iter().enumerate() {
        let sh = heap.intern(s);
        let key = Key::str(sh);
        heap.tables[t as usize].set(key, V::num(i as f64 * 0.125));
    }
    for i in 0..32i64 {
        heap.tables[t as usize].set(Key::int(i), V::num(i as f64));
    }
    t
}

/// Table GET: churned index walk over existing keys (mix of string + int). Returns checksum.
pub fn bench_table_get<V: VmValue>(iters: u64) -> u64 {
    let mut heap = Heap::<V>::new();
    let t = build_struct_table(&mut heap);
    // pre-resolve the string handles into Values so the get path is decode->key->hash->lookup
    let mut str_vals: Vec<V> = Vec::new();
    for s in STR_CORPUS {
        str_vals.push(V::str_handle(heap.intern(s)));
    }
    let n = heap.tables[t as usize].len();
    let mut acc = 0u64;
    let mut idx = 0x9e3779b97f4a7c15u64;
    let tbl = &heap.tables[t as usize];
    for _ in 0..iters {
        idx = idx.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let pick = (idx >> 33) as usize;
        // alternate string-key gets and int-key gets, data-dependent
        let kv: V = if pick & 1 == 0 {
            str_vals[pick % str_vals.len()]
        } else {
            V::int((pick % n) as i32)
        };
        let key = kv.to_key();
        if let Some(v) = tbl.get(key) {
            acc = acc.wrapping_add(v.checksum());
        } else {
            acc = acc.wrapping_add(1);
        }
    }
    std::hint::black_box(acc)
}

/// Table SET: overwrite existing keys with churned values (steady-state, no growth). Checksum.
pub fn bench_table_set<V: VmValue>(iters: u64) -> u64 {
    let mut heap = Heap::<V>::new();
    let t = build_struct_table(&mut heap);
    let mut str_vals: Vec<V> = Vec::new();
    for s in STR_CORPUS {
        str_vals.push(V::str_handle(heap.intern(s)));
    }
    let n = heap.tables[t as usize].len();
    let mut idx = 0x1234567u64;
    let tbl = &mut heap.tables[t as usize];
    for _ in 0..iters {
        idx = idx.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let pick = (idx >> 33) as usize;
        let kv: V = if pick & 1 == 0 {
            str_vals[pick % str_vals.len()]
        } else {
            V::int((pick % n) as i32)
        };
        let key = kv.to_key();
        tbl.set(key, V::num((idx & 0xffff) as f64));
    }
    let mut acc = 0u64;
    for v in &tbl.vals {
        acc = acc.wrapping_add(v.checksum());
    }
    std::hint::black_box(acc)
}

/// Table INSERT/grow: fresh table each batch, insert k entries (models building a record). Checksum.
pub fn bench_table_insert<V: VmValue>(iters: u64) -> u64 {
    let mut heap = Heap::<V>::new();
    let mut str_vals: Vec<V> = Vec::new();
    for s in STR_CORPUS {
        str_vals.push(V::str_handle(heap.intern(s)));
    }
    let mut acc = 0u64;
    let mut done = 0u64;
    let mut seed = 0xabcdefu64;
    while done < iters {
        let t = heap.new_table();
        let tbl = &mut heap.tables[t as usize];
        for j in 0..STR_CORPUS.len() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let kv = str_vals[j];
            tbl.set(kv.to_key(), V::num((seed & 0xff) as f64));
            done += 1;
        }
        acc = acc.wrapping_add(tbl.get(str_vals[0].to_key()).unwrap().checksum());
        // drop the table arena periodically to avoid unbounded growth
        if heap.tables.len() > 4096 {
            heap.tables.clear();
        }
    }
    std::hint::black_box(acc)
}

/// String INTERN: churned lookups (hits) + occasional misses (new strings). Returns checksum.
pub fn bench_string_intern<V: VmValue>(iters: u64) -> u64 {
    let mut heap = Heap::<V>::new();
    // pre-warm corpus
    for s in STR_CORPUS {
        heap.intern(s);
    }
    let mut acc = 0u64;
    let mut idx = 0x55u64;
    let mut scratch = String::with_capacity(16);
    for _ in 0..iters {
        idx = idx.wrapping_mul(6364136223846793005).wrapping_add(1);
        let pick = (idx >> 40) as usize % STR_CORPUS.len();
        let h = heap.intern(STR_CORPUS[pick]); // hit
        acc = acc.wrapping_add(h as u64);
        // also exercise byte access (str ops touch the heap object)
        let b = heap.str_bytes(h).as_bytes();
        acc = acc.wrapping_add(b[0] as u64).wrapping_add(b.len() as u64);
        std::hint::black_box(&mut scratch);
    }
    std::hint::black_box(acc)
}

/// String EQ via interned-handle identity: the win of interning (handle compare, no byte compare).
pub fn bench_string_eq<V: VmValue>(iters: u64) -> u64 {
    let mut heap = Heap::<V>::new();
    let mut hs: Vec<V> = Vec::new();
    for s in STR_CORPUS {
        hs.push(V::str_handle(heap.intern(s)));
    }
    let mut acc = 0u64;
    let mut idx = 0x77u64;
    for _ in 0..iters {
        idx = idx.wrapping_mul(6364136223846793005).wrapping_add(1);
        let i = (idx >> 40) as usize % hs.len();
        let j = (idx >> 20) as usize % hs.len();
        let a = hs[i];
        let b = hs[j];
        // interned eq = same handle iff both strings (decode discriminant + handle)
        let eq = a.is_str() && b.is_str() && a.as_handle() == b.as_handle();
        acc = acc.wrapping_add(eq as u64);
    }
    std::hint::black_box(acc)
}

/// CLONE/MOVE: shuffle a register-file-sized array of Values (churned src/dst), like the VM's
/// MOVE/LOADK traffic. Measures per-Value copy bandwidth (8 vs 16 vs 24 bytes).
pub fn bench_clone_move<V: VmValue>(iters: u64) -> u64 {
    let mut reg = [V::nil(); 32];
    for (i, r) in reg.iter_mut().enumerate() {
        *r = V::num(i as f64 * 1.5);
    }
    let mut idx = 0x9e3779b9u64;
    for _ in 0..iters {
        idx = idx.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let dst = ((idx >> 27) & 31) as usize;
        let src = ((idx >> 40) & 31) as usize;
        reg[dst] = reg[src]; // the Value move
    }
    let mut acc = 0u64;
    for r in &reg {
        acc = acc.wrapping_add(r.checksum());
    }
    std::hint::black_box(acc)
}

/// BULK CLONE: clone a Vec<V> of N values (memcpy bandwidth) — the per-VM snapshot / register-window
/// save cost. Shows the 8/16/24-byte ratio cleanly.
pub fn bench_bulk_clone<V: VmValue>(iters: u64) -> u64 {
    const N: usize = 256;
    let mut src = Vec::with_capacity(N);
    for i in 0..N {
        src.push(V::num(i as f64));
    }
    let mut acc = 0u64;
    for _ in 0..iters {
        let c = src.clone();
        acc = acc.wrapping_add(c[std::hint::black_box(0)].checksum());
        std::hint::black_box(&c);
    }
    std::hint::black_box(acc)
}

// -------------------------------------------------------------------------------------
// koto Arc-tax model: an Arc-backed heap value (NON-Copy). clone() bumps an atomic refcount;
// drop() decrements. This is the ~20% self-time Spike D attributed to koto's Arc clone/drop.
// -------------------------------------------------------------------------------------
use std::sync::Arc;

#[derive(Clone)]
pub enum KotoVal {
    Num(f64),
    Str(Arc<str>),
    Table(Arc<Vec<KotoVal>>),
}

/// CLONE/MOVE for an Arc-backed value: half the moves hit an Arc variant (atomic inc/dec).
pub fn bench_clone_arc(iters: u64) -> u64 {
    let s: Arc<str> = Arc::from("aggression");
    let t: Arc<Vec<KotoVal>> = Arc::new(vec![KotoVal::Num(1.0), KotoVal::Num(2.0)]);
    let mut reg: Vec<KotoVal> = Vec::with_capacity(32);
    for i in 0..32 {
        reg.push(if i % 2 == 0 {
            KotoVal::Str(s.clone())
        } else {
            KotoVal::Table(t.clone())
        });
    }
    let mut idx = 0x9e3779b9u64;
    let mut acc = 0u64;
    for _ in 0..iters {
        idx = idx.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let dst = ((idx >> 27) & 31) as usize;
        let src = ((idx >> 40) & 31) as usize;
        // a real clone+drop: clone src (refcount++), assign to dst (drops old dst, refcount--)
        reg[dst] = reg[src].clone();
        acc = acc.wrapping_add(match &reg[dst] {
            KotoVal::Num(_) => 0,
            KotoVal::Str(_) => 1,
            KotoVal::Table(_) => 2,
        });
    }
    std::hint::black_box(&reg);
    std::hint::black_box(acc)
}

// =====================================================================================
// NaN-box SOUNDNESS verification. Does the tag space alias canonicalized-NaN f64s?
// The tag space is (bits & QNAN) == QNAN, i.e. exponent==0x7ff AND bits 51,50 BOTH set.
// CANON_NAN = 0x7ff8.. has bit 50 CLEAR => is_num==true (not aliased). An UNcanonicalized
// NaN with bits 51,50 set aliases as nil/int/handle => the soundness bug.
// Returns (n_payloads_tested, n_alias_before_canon, n_alias_after_canon).
// =====================================================================================

pub fn soundness_scan() -> (u64, u64, u64) {
    let mut tested = 0u64;
    let mut alias_before = 0u64;
    let mut alias_after = 0u64;
    // Enumerate a wide spread of NaN payloads, both signs, including the wasm-nondeterministic
    // "set the top mantissa bits" patterns that hardware/JITs actually emit.
    for sign in [0u64, SIGN] {
        // sweep the high mantissa bits (50,51) and a spread of low payloads
        for hi in 0u64..4 {
            for step in 0u64..256 {
                let low = step.wrapping_mul(0x0000_0001_0000_0007) & 0x0003_ffff_ffff_ffff;
                // exponent all-ones (0x7ff) + mantissa nonzero == a NaN
                let mant = (hi << 50) | low | 1; // ensure nonzero mantissa
                let bits = sign | (0x7ffu64 << 52) | mant;
                if !f64::from_bits(bits).is_nan() {
                    continue;
                }
                tested += 1;
                // BEFORE canon: does the raw NaN read as a non-number (i.e. alias a tag)?
                let raw = NbValue(bits);
                if !raw.is_num() {
                    alias_before += 1;
                }
                // AFTER canon: route through the per-op canon constructor
                let canon = NbValue::num_canon(f64::from_bits(bits));
                if !canon.is_num() {
                    alias_after += 1;
                }
            }
        }
    }
    (tested, alias_before, alias_after)
}

/// Classify what a given uncanonicalized NaN aliases as (for the report's concrete example).
pub fn classify_alias(bits: u64) -> &'static str {
    let v = NbValue(bits);
    if v.is_num() {
        "number (safe)"
    } else if v.0 == TAG_NIL {
        "NIL (!!)"
    } else if v.0 == TAG_FALSE {
        "FALSE (!!)"
    } else if v.0 == TAG_TRUE {
        "TRUE (!!)"
    } else if (v.0 & QNAN) == QNAN && (v.0 & SIGN) == 0 && (v.0 & 7) == 4 {
        "INT (!!)"
    } else if (v.0 & SIGN) != 0 {
        "HEAP HANDLE (!!)"
    } else {
        "other-tag (!!)"
    }
}

// =====================================================================================
// wasm exports. JS times each (performance.now) and divides by the op count. Layout suffixes:
//   _nb = NaN-box 8B, _tag = tagged 16B, _e24 = fat 24B.
// =====================================================================================

macro_rules! export_bench {
    ($name:ident, $f:ident, $V:ty) => {
        #[no_mangle]
        pub extern "C" fn $name(iters: u32) -> u64 {
            $f::<$V>(iters as u64)
        }
    };
}

export_bench!(b_table_get_nb, bench_table_get, NbValue);
export_bench!(b_table_get_tag, bench_table_get, TagValue);
export_bench!(b_table_get_e24, bench_table_get, E24);
export_bench!(b_table_set_nb, bench_table_set, NbValue);
export_bench!(b_table_set_tag, bench_table_set, TagValue);
export_bench!(b_table_set_e24, bench_table_set, E24);
export_bench!(b_table_insert_nb, bench_table_insert, NbValue);
export_bench!(b_table_insert_tag, bench_table_insert, TagValue);
export_bench!(b_string_intern_nb, bench_string_intern, NbValue);
export_bench!(b_string_intern_tag, bench_string_intern, TagValue);
export_bench!(b_string_eq_nb, bench_string_eq, NbValue);
export_bench!(b_string_eq_tag, bench_string_eq, TagValue);
export_bench!(b_clone_move_nb, bench_clone_move, NbValue);
export_bench!(b_clone_move_tag, bench_clone_move, TagValue);
export_bench!(b_clone_move_e24, bench_clone_move, E24);
export_bench!(b_bulk_clone_nb, bench_bulk_clone, NbValue);
export_bench!(b_bulk_clone_tag, bench_bulk_clone, TagValue);
export_bench!(b_bulk_clone_e24, bench_bulk_clone, E24);

#[no_mangle]
pub extern "C" fn b_clone_arc(iters: u32) -> u64 {
    bench_clone_arc(iters as u64)
}

// ---- kernel exports (canon ON and OFF) ----

static mut W: Option<World> = None;
static mut VM_NB: Option<Vm<NbValue>> = None;
static mut VM_TAG: Option<Vm<TagValue>> = None;
static mut VM_E24: Option<Vm<E24>> = None;
static mut PG_NB: Option<Program<NbValue>> = None;
static mut PG_TAG: Option<Program<TagValue>> = None;
static mut PG_E24: Option<Program<E24>> = None;
static mut WN: usize = 0;

#[no_mangle]
pub extern "C" fn bench_setup(n: u32) -> u64 {
    unsafe {
        W = Some(World::new(n as usize, 0xA1_BACE_50));
        VM_NB = Some(Vm::new());
        VM_TAG = Some(Vm::new());
        VM_E24 = Some(Vm::new());
        PG_NB = Some(build_argmax_f64());
        PG_TAG = Some(build_argmax_f64());
        PG_E24 = Some(build_argmax_f64());
        WN = n as usize;
    }
    n as u64
}
#[no_mangle]
pub extern "C" fn bench_n() -> u64 {
    unsafe { WN as u64 }
}

macro_rules! kernel_export {
    ($name:ident, $vm:ident, $pg:ident, $V:ty, $canon:literal) => {
        #[no_mangle]
        pub extern "C" fn $name(ticks: u32) -> u64 {
            unsafe {
                let w = W.as_mut().unwrap();
                let vm = $vm.as_mut().unwrap();
                let p = $pg.as_ref().unwrap();
                for _ in 0..ticks {
                    w.tick_vm_realistic::<$V, true, $canon>(vm, p);
                }
                w.state_hash()
            }
        }
    };
}
kernel_export!(kernel_nb_canon, VM_NB, PG_NB, NbValue, true);
kernel_export!(kernel_nb_nocanon, VM_NB, PG_NB, NbValue, false);
kernel_export!(kernel_tag_canon, VM_TAG, PG_TAG, TagValue, true);
kernel_export!(kernel_tag_nocanon, VM_TAG, PG_TAG, TagValue, false);
kernel_export!(kernel_e24_canon, VM_E24, PG_E24, E24, true);

#[no_mangle]
pub extern "C" fn bench_run_native(ticks: u32) -> u64 {
    unsafe {
        let w = W.as_mut().unwrap();
        for _ in 0..ticks {
            w.tick_native();
        }
        w.state_hash()
    }
}

/// Q16 golden in wasm (NaN-box). Must equal 0x31c6ec2734ea6fa2.
#[no_mangle]
pub extern "C" fn bench_q16_golden_nb() -> u64 {
    let prog = build_argmax_q16::<NbValue>();
    let mut vm = Vm::new();
    let mut w = World::new(4000, 0xA1_BACE_50);
    for _ in 0..200 {
        w.tick_vm_q16(&mut vm, &prog);
    }
    w.state_hash()
}
#[no_mangle]
pub extern "C" fn bench_q16_golden_tag() -> u64 {
    let prog = build_argmax_q16::<TagValue>();
    let mut vm = Vm::new();
    let mut w = World::new(4000, 0xA1_BACE_50);
    for _ in 0..200 {
        w.tick_vm_q16(&mut vm, &prog);
    }
    w.state_hash()
}

/// soundness scan exported so the wasm engine's NaN handling is exercised too.
#[no_mangle]
pub extern "C" fn b_soundness() -> u64 {
    let (tested, before, after) = soundness_scan();
    // pack: high 21 bits tested-ish; we just return after-count (must be 0) in low, before in mid
    (tested << 40) | (before << 20) | after
}

// sizeof exports (constant, but read in JS to confirm wasm32 sizes)
#[no_mangle]
pub extern "C" fn sizeof_nb() -> u64 {
    std::mem::size_of::<NbValue>() as u64
}
#[no_mangle]
pub extern "C" fn sizeof_tag() -> u64 {
    std::mem::size_of::<TagValue>() as u64
}
#[no_mangle]
pub extern "C" fn sizeof_e24() -> u64 {
    std::mem::size_of::<E24>() as u64
}
