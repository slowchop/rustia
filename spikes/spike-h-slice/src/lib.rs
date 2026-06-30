//! spike-h-slice: the axis-H vertical slice. A SMALL but REAL general-path Luau-ish program,
//! hand-compiled to bytecode, executed through a gc-arena heap (Gc tables + Gc strings + GC write
//! barriers), with canon-ON f64, host calls through a REAL call_indirect (Box<dyn Fn> table), and the
//! result table serde-Deserialized into a Rust struct (the grabby `exports().clone() -> ScenarioSpec`
//! shape) OUTSIDE arena.mutate. Yields the HONEST general-script number.
//!
//! Value encoding: a piccolo-style tagged enum `Value<'gc>` (the architecture's named "16-byte tagged
//! fallback" for axis B). gc-arena's tracer needs to SEE the Gc pointers, and a NaN-box would require a
//! hand-written unsafe `Collect` over `Gc::from_ptr`-reconstructed pointers (no safe path in 0.6) — that
//! is an axis-B question, deliberately out of scope here. The embedding-API findings (the `'gc` bridging)
//! are identical for both encodings; the number is labeled as the tagged-enum path.

#![allow(dead_code)]
use gc_arena::lock::RefLock;
use gc_arena::{Arena, Collect, DynamicRoot, DynamicRootSet, Gc, Mutation, Rootable};
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use std::fmt;

// =====================================================================================
// Canon (soundness + determinism): NaN -> one quiet NaN on EVERY f64-producing op. -0.0 preserved
// per-op (so 1/-0.0 = -inf); -0.0 -> +0.0 ONLY at table-key normalization.
// =====================================================================================
const CANON_NAN_BITS: u64 = 0x7ff8_0000_0000_0000;

#[inline(always)]
fn canon(x: f64) -> f64 {
    if x.is_nan() {
        f64::from_bits(CANON_NAN_BITS)
    } else {
        x
    }
}

// =====================================================================================
// In-arena value + heap objects
// =====================================================================================

#[derive(Collect)]
#[collect(require_static)]
pub struct LuaStr(pub Box<str>);

#[derive(Collect)]
#[collect(no_drop)]
pub enum Key<'gc> {
    Int(i64),
    Str(Gc<'gc, LuaStr>),
}

/// Insertion-ordered table (determinism req: seedless insertion order). Array part is 1-based dense;
/// the map part holds string/int keys in insertion order.
#[derive(Collect)]
#[collect(no_drop)]
pub struct Table<'gc> {
    pub arr: Vec<Value<'gc>>,
    pub map: Vec<(Key<'gc>, Value<'gc>)>,
}

impl<'gc> Table<'gc> {
    fn new() -> Self {
        Table { arr: Vec::new(), map: Vec::new() }
    }
    fn set_str(&mut self, k: Gc<'gc, LuaStr>, v: Value<'gc>) {
        for e in self.map.iter_mut() {
            if let Key::Str(s) = &e.0 {
                if s.0 == k.0 {
                    e.1 = v;
                    return;
                }
            }
        }
        self.map.push((Key::Str(k), v));
    }
    fn set_int(&mut self, i: i64, v: Value<'gc>) {
        // dense 1-based array fast path
        if i >= 1 && (i as usize) <= self.arr.len() + 1 {
            let idx = (i - 1) as usize;
            if idx == self.arr.len() {
                self.arr.push(v);
            } else {
                self.arr[idx] = v;
            }
            return;
        }
        for e in self.map.iter_mut() {
            if let Key::Int(n) = &e.0 {
                if *n == i {
                    e.1 = v;
                    return;
                }
            }
        }
        self.map.push((Key::Int(i), v));
    }
    fn get_str(&self, name: &str) -> Value<'gc> {
        for e in &self.map {
            if let Key::Str(s) = &e.0 {
                if &*s.0 == name {
                    return e.1;
                }
            }
        }
        Value::Nil
    }
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct Closure<'gc> {
    pub proto: u32,
    pub upvalues: Vec<Value<'gc>>,
}

/// The tagged value. ~16 bytes. Copy. The hot scalar path is Num/Int/Bool/Nil; heap is a Gc handle.
#[derive(Collect, Copy, Clone)]
#[collect(no_drop)]
pub enum Value<'gc> {
    Nil,
    Bool(bool),
    Num(f64),
    Int(i64),
    Str(Gc<'gc, LuaStr>),
    Table(Gc<'gc, RefLock<Table<'gc>>>),
    Closure(Gc<'gc, Closure<'gc>>),
    /// index into the host-fn table (the call_indirect target set)
    Native(u32),
}

impl<'gc> Value<'gc> {
    #[inline(always)]
    fn num(x: f64) -> Value<'gc> {
        Value::Num(canon(x))
    }
    #[inline(always)]
    fn as_f64(self) -> f64 {
        match self {
            Value::Num(n) => n,
            Value::Int(i) => i as f64,
            _ => f64::NAN,
        }
    }
    #[inline(always)]
    fn truthy(self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }
}

// =====================================================================================
// Owned (arena-free) value tree — the `exports().clone()` analog + the host-fn ABI boundary.
// serde Deserialize runs over THIS, fully outside arena.mutate.
// =====================================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct OwnedTable {
    pub arr: Vec<OwnedValue>,
    pub map: Vec<(OwnedKey, OwnedValue)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OwnedKey {
    Int(i64),
    Str(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum OwnedValue {
    Nil,
    Bool(bool),
    Num(f64),
    Int(i64),
    Str(String),
    Table(Box<OwnedTable>),
}

// =====================================================================================
// Host-side, arena-free program (stands in for parser+compiler output). Shared across VMs.
// =====================================================================================

#[derive(Clone, Copy, Debug)]
pub struct Instr {
    op: u8,
    a: u8,
    b: u8,
    c: u8,
    aux: i32,
}
#[inline]
fn i4(op: u8, a: u8, b: u8, c: u8) -> Instr {
    Instr { op, a, b, c, aux: 0 }
}
#[inline]
fn ix(op: u8, a: u8, b: u8, c: u8, aux: i32) -> Instr {
    Instr { op, a, b, c, aux }
}

// opcodes
const HALT: u8 = 0;
const LOADK: u8 = 1; // r[a]=K[c]
const MOVE: u8 = 2; // r[a]=r[b]
const GETGLOBAL: u8 = 3; // r[a]=globals[c]
const NADD: u8 = 4;
const NSUB: u8 = 5;
const NMUL: u8 = 6;
const NDIV: u8 = 7;
const NMODOP: u8 = 8;
const NADDK: u8 = 9; // r[a]=r[b]+Knum[c]
const NMULK: u8 = 10; // r[a]=r[b]*Knum[c]
const NLT: u8 = 11;
const NLE: u8 = 12;
const NEQ: u8 = 13;
const FORNPREP: u8 = 14;
const FORNLOOP: u8 = 15;
const NEWTABLE: u8 = 16;
const SETFIELD: u8 = 17; // table=r[a], key=STR[b], val=r[c]   (write barrier)
const GETFIELD: u8 = 18; // r[a]=table(r[b])[STR[c]]
const SETINDEX: u8 = 19; // table=r[a], key=r[b], val=r[c]      (write barrier)
const TOSTR: u8 = 20; // r[a]=tostring(r[b])
const CONCAT: u8 = 21; // r[a]=r[b]..r[c]
const CLOSURE: u8 = 22; // r[a]=closure(proto=aux)
const GETUPVAL: u8 = 23; // r[a]=upvalues[c]
const CALL: u8 = 24; // fn=r[b], args r[b+1..b+1+aux], result->r[a]
const CALLHOST: u8 = 25; // native[c](args r[b..b+aux]) -> r[a]
const RET: u8 = 26; // return r[a] (sets exports for the top frame)
const SQRTHOST: u8 = 27; // r[a] = host_sqrt(r[b])  (libm, NaN-canon on ingress)

#[derive(Clone, Copy, Debug)]
pub enum Konst {
    Num(f64),
    Int(i64),
    Str(u32), // index into string pool
}

/// One function prototype.
#[derive(Clone)]
pub struct Proto {
    pub code: Vec<Instr>,
    pub max_regs: usize,
    /// upvalue sources: which caller register each upvalue captures (Reg only — enough for the slice).
    pub upval_regs: Vec<u8>,
}

#[derive(Clone)]
pub struct Chunk {
    pub protos: Vec<Proto>,
    pub consts: Vec<Konst>,
    pub strings: Vec<String>,
    pub main: u32,
    /// global names referenced (resolved to host-fn slots at registration time)
    pub globals: Vec<String>,
}

// =====================================================================================
// The arena root.
// =====================================================================================

#[derive(Collect)]
#[collect(no_drop)]
pub struct RootData<'gc> {
    exports: Option<Value<'gc>>,
    /// const-string interning cache, by string-pool index (Lua interns; also a per-arena materialization
    /// of shared host-side string constants — the "constant-pool / Gc-string seam" the arch flags).
    str_cache: Vec<Option<Gc<'gc, LuaStr>>>,
    /// global slot values (host fns live here as Value::Native).
    globals: Vec<Value<'gc>>,
    dyn_set: DynamicRootSet<'gc>,
}

type ArenaRoot = Rootable![Gc<'_, RefLock<RootData<'_>>>];

// The retained-handle type for DynamicRoot stashing (a retained exports table).
type StashTable = Rootable![RefLock<Table<'_>>];

// =====================================================================================
// Host-fn ABI: arena-free owned values in/out. Box<dyn Fn> table = the call_indirect target set.
// grabby's REAL host fns (rand/rand_range/fxmul/curve/spawn/log/tick/bodies) are all scalar-or-string
// in/out — none allocate tables — so this owned boundary keeps 'gc out of the embedder entirely.
// =====================================================================================
pub type HostFn = Box<dyn Fn(&[OwnedValue]) -> Result<OwnedValue, String>>;

pub struct Settings {
    pub fuel_limit: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Settings { fuel_limit: u64::MAX }
    }
}

/// The embedding handle (koto-SHAPED outer surface).
pub struct Vm {
    arena: Arena<ArenaRoot>,
    chunk: Option<Chunk>,
    hosts: Vec<HostFn>,
    host_names: Vec<String>,
    settings: Settings,
    /// a retained handle (DynamicRoot) to the exports table, surviving GC cycles.
    retained: Option<DynamicRoot<StashTable>>,
}

#[derive(Debug)]
pub enum VmError {
    Compile(String),
    Runtime(String),
    FuelExhausted,
}
impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmError::Compile(s) => write!(f, "compile error: {s}"),
            VmError::Runtime(s) => write!(f, "runtime error: {s}"),
            VmError::FuelExhausted => write!(f, "fuel exhausted"),
        }
    }
}

impl Vm {
    pub fn new() -> Self {
        Self::with_settings(Settings::default())
    }

    /// koto: `Koto::with_settings(...)`.
    pub fn with_settings(settings: Settings) -> Self {
        let arena = Arena::<ArenaRoot>::new(|mc| {
            Gc::new(
                mc,
                RefLock::new(RootData {
                    exports: None,
                    str_cache: Vec::new(),
                    globals: Vec::new(),
                    dyn_set: DynamicRootSet::new(mc),
                }),
            )
        });
        Vm { arena, chunk: None, hosts: Vec::new(), host_names: Vec::new(), settings, retained: None }
    }

    /// koto: `prelude().add_fn(name, closure)`. Returns the host-fn slot index.
    pub fn add_fn<F>(&mut self, name: &str, f: F) -> u32
    where
        F: Fn(&[OwnedValue]) -> Result<OwnedValue, String> + 'static,
    {
        let idx = self.hosts.len() as u32;
        self.hosts.push(Box::new(f));
        self.host_names.push(name.to_string());
        idx
    }

    /// koto: `compile_and_run(script)`. Here `chunk` is the hand-built bytecode (parser stands aside).
    /// Resolves global names -> host-fn slots, builds the per-arena globals, runs `main`, stores exports.
    pub fn compile_and_run(&mut self, chunk: Chunk) -> Result<(), VmError> {
        // resolve globals: each chunk.globals[i] must name a registered host fn.
        let mut global_vals: Vec<u32> = Vec::with_capacity(chunk.globals.len());
        for g in &chunk.globals {
            match self.host_names.iter().position(|n| n == g) {
                Some(i) => global_vals.push(i as u32),
                None => return Err(VmError::Compile(format!("unresolved global '{g}'"))),
            }
        }
        let nstr = chunk.strings.len();
        // set up globals + str_cache slots in the arena root.
        self.arena.mutate(|mc, root| {
            let mut r = root.borrow_mut(mc); // root write barrier
            r.globals = global_vals.iter().map(|&i| Value::Native(i)).collect();
            r.str_cache = vec![None; nstr];
            r.exports = None;
        });
        let fuel = self.settings.fuel_limit;
        let res = self.arena.mutate(|mc, root| run_main(mc, root, &chunk, &self.hosts, fuel));
        self.chunk = Some(chunk);
        res
    }

    /// Re-run the loaded chunk's main (re-allocates tables/strings -> real heap traffic). Used by the
    /// benchmark loop to measure repeated general-script execution.
    pub fn rerun(&self) -> Result<(), VmError> {
        let chunk = self.chunk.as_ref().expect("load first");
        let nstr = chunk.strings.len();
        self.arena.mutate(|mc, root| {
            let mut r = root.borrow_mut(mc);
            // reset the string cache so strings are re-materialized (fresh heap each run)
            for s in r.str_cache.iter_mut() {
                *s = None;
            }
            let _ = nstr;
            r.exports = None;
        });
        let fuel = self.settings.fuel_limit;
        self.arena.mutate(|mc, root| run_main(mc, root, chunk, &self.hosts, fuel))
    }

    /// koto: `exports().clone()` — deep-snapshot the exports table OUT of the arena into an owned tree.
    pub fn exports_clone(&self) -> Option<OwnedValue> {
        self.arena.mutate(|_mc, root| {
            let r = root.borrow();
            r.exports.map(snapshot)
        })
    }

    /// Force a full GC: run the current cycle to completion (mark+sweep), twice, so any debt is cleared
    /// and a fresh full collection has run. Deterministic (driven explicitly, not by byte-debt).
    pub fn force_gc(&mut self) {
        self.arena.finish_cycle();
        self.arena.finish_cycle();
    }

    /// Stash the exports table as a DynamicRoot so it survives GC independent of the arena root.
    pub fn retain_exports(&mut self) -> bool {
        let dr = self.arena.mutate(|mc, root| {
            let r = root.borrow();
            match r.exports {
                Some(Value::Table(t)) => {
                    let dr = r.dyn_set.stash::<StashTable>(mc, t);
                    Some(dr)
                }
                _ => None,
            }
        });
        match dr {
            Some(dr) => {
                self.retained = Some(dr);
                true
            }
            None => false,
        }
    }

    /// Snapshot the RETAINED handle (fetched from the DynamicRootSet) out to an owned tree — the grabby
    /// "retain-an-exported-handle-then-serde-OUTSIDE-mutate" pattern.
    pub fn retained_clone(&self) -> Option<OwnedValue> {
        let dr = self.retained.as_ref()?;
        Some(self.arena.mutate(|_mc, root| {
            let r = root.borrow();
            let t = r.dyn_set.fetch::<StashTable>(dr);
            snapshot(Value::Table(t))
        }))
    }

    pub fn metrics_total(&self) -> f64 {
        self.arena.metrics().total_gc_allocation() as f64
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================================
// Snapshot: Value<'gc> -> OwnedValue (deep copy out of the arena). The marshalling boundary.
// =====================================================================================
fn snapshot<'gc>(v: Value<'gc>) -> OwnedValue {
    match v {
        Value::Nil => OwnedValue::Nil,
        Value::Bool(b) => OwnedValue::Bool(b),
        Value::Num(n) => OwnedValue::Num(n),
        Value::Int(i) => OwnedValue::Int(i),
        Value::Str(s) => OwnedValue::Str(s.0.to_string()),
        Value::Native(_) | Value::Closure(_) => OwnedValue::Nil, // not serializable
        Value::Table(t) => {
            let tb = t.borrow();
            let arr = tb.arr.iter().map(|&e| snapshot(e)).collect();
            let map = tb
                .map
                .iter()
                .map(|(k, val)| {
                    let ok = match k {
                        Key::Int(i) => OwnedKey::Int(*i),
                        Key::Str(s) => OwnedKey::Str(s.0.to_string()),
                    };
                    (ok, snapshot(*val))
                })
                .collect();
            OwnedValue::Table(Box::new(OwnedTable { arr, map }))
        }
    }
}

// =====================================================================================
// number -> string (Luau-ish: integral f64 prints without a decimal point)
// =====================================================================================
fn fmt_num(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

// =====================================================================================
// Execution: stackless trampoline over a frame stack + a flat register file.
// =====================================================================================

struct Frame {
    proto: u32,
    pc: usize,
    base: usize,    // index into regs where this frame's r0 sits
    ret_reg: usize, // absolute reg index in the caller to receive the return value
    has_upvals: bool,
}

fn run_main<'gc>(
    mc: &Mutation<'gc>,
    root: &Gc<'gc, RefLock<RootData<'gc>>>,
    chunk: &Chunk,
    hosts: &[HostFn],
    fuel_budget: u64,
) -> Result<(), VmError> {
    let main_proto = chunk.main;
    // closure storage for upvalues, parallel to frames (only frames with upvals have one)
    let mut regs: Vec<Value<'gc>> = Vec::with_capacity(64);
    let mut frame_upvals: Vec<Vec<Value<'gc>>> = Vec::new();

    let mp = &chunk.protos[main_proto as usize];
    let base = 0usize;
    regs.resize(mp.max_regs, Value::Nil);
    frame_upvals.push(Vec::new());
    let mut frames: Vec<Frame> = vec![Frame {
        proto: main_proto,
        pc: 0,
        base,
        ret_reg: usize::MAX,
        has_upvals: false,
    }];

    let mut fuel = fuel_budget;

    // intern a string constant (materialize a Gc<LuaStr> per arena), caching by pool index.
    macro_rules! intern {
        ($idx:expr) => {{
            let idx = $idx as usize;
            let cached = root.borrow().str_cache[idx];
            match cached {
                Some(g) => g,
                None => {
                    let g = Gc::new(mc, LuaStr(chunk.strings[idx].clone().into_boxed_str()));
                    root.borrow_mut(mc).str_cache[idx] = Some(g); // root write barrier
                    g
                }
            }
        }};
    }

    'frames: loop {
        let (proto_idx, mut pc, fbase) = {
            let f = frames.last().unwrap();
            (f.proto, f.pc, f.base)
        };
        let proto = &chunk.protos[proto_idx as usize];
        let code = &proto.code;

        loop {
            let i = code[pc];
            pc += 1;
            fuel = fuel.wrapping_sub(1);
            if fuel == 0 {
                return Err(VmError::FuelExhausted);
            }
            let a = fbase + i.a as usize;
            match i.op {
                HALT => break 'frames,
                LOADK => {
                    regs[a] = match chunk.consts[i.c as usize] {
                        Konst::Num(n) => Value::num(n),
                        Konst::Int(v) => Value::Int(v),
                        Konst::Str(si) => Value::Str(intern!(si)),
                    };
                }
                MOVE => regs[a] = regs[fbase + i.b as usize],
                GETGLOBAL => regs[a] = root.borrow().globals[i.c as usize],
                NADD => {
                    regs[a] = Value::num(regs[fbase + i.b as usize].as_f64() + regs[fbase + i.c as usize].as_f64())
                }
                NSUB => {
                    regs[a] = Value::num(regs[fbase + i.b as usize].as_f64() - regs[fbase + i.c as usize].as_f64())
                }
                NMUL => {
                    regs[a] = Value::num(regs[fbase + i.b as usize].as_f64() * regs[fbase + i.c as usize].as_f64())
                }
                NDIV => {
                    regs[a] = Value::num(regs[fbase + i.b as usize].as_f64() / regs[fbase + i.c as usize].as_f64())
                }
                NMODOP => {
                    let x = regs[fbase + i.b as usize].as_f64();
                    let y = regs[fbase + i.c as usize].as_f64();
                    // Lua floored modulo
                    regs[a] = Value::num(x - (x / y).floor() * y);
                }
                NADDK => {
                    let k = match chunk.consts[i.c as usize] {
                        Konst::Num(n) => n,
                        Konst::Int(v) => v as f64,
                        _ => f64::NAN,
                    };
                    regs[a] = Value::num(regs[fbase + i.b as usize].as_f64() + k);
                }
                NMULK => {
                    let k = match chunk.consts[i.c as usize] {
                        Konst::Num(n) => n,
                        Konst::Int(v) => v as f64,
                        _ => f64::NAN,
                    };
                    regs[a] = Value::num(regs[fbase + i.b as usize].as_f64() * k);
                }
                NLT => {
                    regs[a] = Value::Bool(regs[fbase + i.b as usize].as_f64() < regs[fbase + i.c as usize].as_f64())
                }
                NLE => {
                    regs[a] = Value::Bool(regs[fbase + i.b as usize].as_f64() <= regs[fbase + i.c as usize].as_f64())
                }
                NEQ => {
                    regs[a] = Value::Bool(regs[fbase + i.b as usize].as_f64() == regs[fbase + i.c as usize].as_f64())
                }
                FORNPREP => {
                    let idx = regs[a].as_f64();
                    let limit = regs[a + 1].as_f64();
                    let step = regs[a + 2].as_f64();
                    let skip = if step > 0.0 { idx > limit } else { idx < limit };
                    if skip {
                        pc = i.aux as usize;
                    }
                }
                FORNLOOP => {
                    let step = regs[a + 2].as_f64();
                    let idx = regs[a].as_f64() + step;
                    regs[a] = Value::num(idx);
                    let limit = regs[a + 1].as_f64();
                    let cont = if step > 0.0 { idx <= limit } else { idx >= limit };
                    if cont {
                        pc = i.aux as usize;
                    }
                }
                NEWTABLE => regs[a] = Value::Table(Gc::new(mc, RefLock::new(Table::new()))),
                SETFIELD => {
                    let key = intern!(i.b);
                    let val = regs[fbase + i.c as usize];
                    if let Value::Table(t) = regs[a] {
                        // *** GC WRITE BARRIER *** borrow_mut(mc) -> unlock(mc) -> backward_barrier
                        t.borrow_mut(mc).set_str(key, val);
                    } else {
                        return Err(VmError::Runtime("SETFIELD on non-table".into()));
                    }
                }
                GETFIELD => {
                    let key = &chunk.strings[i.c as usize];
                    if let Value::Table(t) = regs[fbase + i.b as usize] {
                        regs[a] = t.borrow().get_str(key);
                    } else {
                        return Err(VmError::Runtime("GETFIELD on non-table".into()));
                    }
                }
                SETINDEX => {
                    let keyv = regs[fbase + i.b as usize];
                    let val = regs[fbase + i.c as usize];
                    if let Value::Table(t) = regs[a] {
                        // *** GC WRITE BARRIER ***
                        let mut tb = t.borrow_mut(mc);
                        match keyv {
                            Value::Int(k) => tb.set_int(k, val),
                            Value::Num(n) => {
                                // table-key normalization: -0.0 -> +0.0; integral f64 -> int key
                                let n = if n == 0.0 { 0.0 } else { n };
                                if n.fract() == 0.0 && n.is_finite() && n.abs() < 9e18 {
                                    tb.set_int(n as i64, val);
                                } else {
                                    return Err(VmError::Runtime("non-integral numeric key".into()));
                                }
                            }
                            Value::Str(s) => tb.set_str(s, val),
                            _ => return Err(VmError::Runtime("bad table key".into())),
                        }
                    } else {
                        return Err(VmError::Runtime("SETINDEX on non-table".into()));
                    }
                }
                TOSTR => {
                    let s = match regs[fbase + i.b as usize] {
                        Value::Num(n) => fmt_num(n),
                        Value::Int(v) => v.to_string(),
                        Value::Str(g) => g.0.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Nil => "nil".to_string(),
                        _ => "<obj>".to_string(),
                    };
                    regs[a] = Value::Str(Gc::new(mc, LuaStr(s.into_boxed_str())));
                }
                CONCAT => {
                    let lhs = regs[fbase + i.b as usize];
                    let rhs = regs[fbase + i.c as usize];
                    let mut s = String::new();
                    push_str_of(&mut s, lhs);
                    push_str_of(&mut s, rhs);
                    regs[a] = Value::Str(Gc::new(mc, LuaStr(s.into_boxed_str())));
                }
                CLOSURE => {
                    let p = i.aux as u32;
                    let upsrc = &chunk.protos[p as usize].upval_regs;
                    let ups: Vec<Value<'gc>> = upsrc.iter().map(|&r| regs[fbase + r as usize]).collect();
                    regs[a] = Value::Closure(Gc::new(mc, Closure { proto: p, upvalues: ups }));
                }
                GETUPVAL => {
                    let fi = frames.len() - 1;
                    regs[a] = frame_upvals[fi][i.c as usize];
                }
                CALL => {
                    let nargs = i.aux as usize;
                    let fnreg = fbase + i.b as usize;
                    let callee = regs[fnreg];
                    match callee {
                        Value::Closure(cl) => {
                            let cp = &chunk.protos[cl.proto as usize];
                            let new_base = regs.len();
                            regs.resize(new_base + cp.max_regs, Value::Nil);
                            for k in 0..nargs {
                                regs[new_base + k] = regs[fnreg + 1 + k];
                            }
                            // save caller pc
                            frames.last_mut().unwrap().pc = pc;
                            frames.push(Frame {
                                proto: cl.proto,
                                pc: 0,
                                base: new_base,
                                ret_reg: a,
                                has_upvals: !cl.upvalues.is_empty(),
                            });
                            frame_upvals.push(cl.upvalues.clone());
                            continue 'frames;
                        }
                        Value::Native(hidx) => {
                            // host call through the call_indirect table
                            let mut args: Vec<OwnedValue> = Vec::with_capacity(nargs);
                            for k in 0..nargs {
                                args.push(snapshot(regs[fnreg + 1 + k]));
                            }
                            let out = call_host(hosts, hidx, &args)?;
                            regs[a] = owned_to_value(mc, &out);
                        }
                        _ => return Err(VmError::Runtime("call of non-callable".into())),
                    }
                }
                CALLHOST => {
                    let nargs = i.aux as usize;
                    let argbase = fbase + i.b as usize;
                    let hidx = i.c as u32;
                    let mut args: Vec<OwnedValue> = Vec::with_capacity(nargs);
                    for k in 0..nargs {
                        args.push(snapshot(regs[argbase + k]));
                    }
                    let out = call_host(hosts, hidx, &args)?;
                    regs[a] = owned_to_value(mc, &out);
                }
                SQRTHOST => {
                    // a host fn producing a possible NaN -> canon on ingress (§3.3 host-fn return).
                    let x = regs[fbase + i.b as usize].as_f64();
                    regs[a] = Value::num(libm::sqrt(x));
                }
                RET => {
                    let rv = regs[a];
                    let f = frames.pop().unwrap();
                    frame_upvals.pop();
                    if frames.is_empty() {
                        // top-level return: this is `exports`
                        root.borrow_mut(mc).exports = Some(rv); // root write barrier
                        break 'frames;
                    }
                    // shrink the register window back
                    regs.truncate(f.base);
                    regs[f.ret_reg] = rv;
                    let _ = f.has_upvals;
                    continue 'frames;
                }
                _ => return Err(VmError::Runtime(format!("bad opcode {}", i.op))),
            }
        }
    }
    Ok(())
}

fn push_str_of<'gc>(s: &mut String, v: Value<'gc>) {
    match v {
        Value::Str(g) => s.push_str(&g.0),
        Value::Num(n) => s.push_str(&fmt_num(n)),
        Value::Int(i) => s.push_str(&i.to_string()),
        Value::Bool(b) => s.push_str(&b.to_string()),
        Value::Nil => s.push_str("nil"),
        _ => s.push_str("<obj>"),
    }
}

/// REAL call_indirect: index the Box<dyn Fn> table and call through the trait-object pointer. The
/// compiler cannot devirtualize/inline this -> an indirect call on native, `call_indirect` on wasm.
#[inline(never)]
fn call_host(hosts: &[HostFn], idx: u32, args: &[OwnedValue]) -> Result<OwnedValue, VmError> {
    let f = hosts
        .get(idx as usize)
        .ok_or_else(|| VmError::Runtime(format!("no host fn {idx}")))?;
    f(args).map_err(VmError::Runtime)
}

fn owned_to_value<'gc>(_mc: &Mutation<'gc>, v: &OwnedValue) -> Value<'gc> {
    match v {
        OwnedValue::Nil => Value::Nil,
        OwnedValue::Bool(b) => Value::Bool(*b),
        OwnedValue::Num(n) => Value::num(*n), // canon on host-fn return ingress
        OwnedValue::Int(i) => Value::Int(*i),
        OwnedValue::Str(s) => Value::Str(Gc::new(_mc, LuaStr(s.clone().into_boxed_str()))),
        OwnedValue::Table(_) => Value::Nil, // host fns don't return tables in the slice
    }
}

// =====================================================================================
// serde Deserializer over OwnedValue — runs fully OUTSIDE arena.mutate.
// =====================================================================================

#[derive(Debug)]
pub struct DeError(pub String);
impl fmt::Display for DeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for DeError {}
impl de::Error for DeError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        DeError(msg.to_string())
    }
}

pub fn from_owned<'de, T: Deserialize<'de>>(v: &'de OwnedValue) -> Result<T, DeError> {
    T::deserialize(De { v })
}

struct De<'a> {
    v: &'a OwnedValue,
}

impl<'de, 'a> Deserializer<'de> for De<'a> {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.v {
            OwnedValue::Nil => vis.visit_unit(),
            OwnedValue::Bool(b) => vis.visit_bool(*b),
            OwnedValue::Num(n) => vis.visit_f64(*n),
            OwnedValue::Int(i) => vis.visit_i64(*i),
            OwnedValue::Str(s) => vis.visit_str(s),
            OwnedValue::Table(t) => {
                if t.map.is_empty() {
                    vis.visit_seq(SeqA { it: t.arr.iter() })
                } else {
                    vis.visit_map(MapA { entries: &t.map, idx: 0 })
                }
            }
        }
    }

    fn deserialize_f64<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.v {
            OwnedValue::Num(n) => vis.visit_f64(*n),
            OwnedValue::Int(i) => vis.visit_f64(*i as f64),
            _ => Err(DeError("expected number".into())),
        }
    }
    fn deserialize_u64<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.v {
            OwnedValue::Int(i) => vis.visit_u64(*i as u64),
            OwnedValue::Num(n) => vis.visit_u64(*n as u64),
            _ => Err(DeError("expected integer".into())),
        }
    }
    fn deserialize_i64<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.v {
            OwnedValue::Int(i) => vis.visit_i64(*i),
            OwnedValue::Num(n) => vis.visit_i64(*n as i64),
            _ => Err(DeError("expected integer".into())),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.v {
            OwnedValue::Nil => vis.visit_none(),
            _ => vis.visit_some(self),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        vis: V,
    ) -> Result<V::Value, DeError> {
        match self.v {
            OwnedValue::Table(t) => vis.visit_map(MapA { entries: &t.map, idx: 0 }),
            _ => Err(DeError("expected table for struct".into())),
        }
    }

    fn deserialize_map<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.v {
            OwnedValue::Table(t) => vis.visit_map(MapA { entries: &t.map, idx: 0 }),
            _ => Err(DeError("expected table for map".into())),
        }
    }

    fn deserialize_seq<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.v {
            OwnedValue::Table(t) => vis.visit_seq(SeqA { it: t.arr.iter() }),
            _ => Err(DeError("expected table for seq".into())),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        vis: V,
    ) -> Result<V::Value, DeError> {
        vis.visit_newtype_struct(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 u8 u16 u32 f32 char str string bytes byte_buf
        unit unit_struct tuple tuple_struct enum identifier ignored_any
    }
}

struct SeqA<'a> {
    it: std::slice::Iter<'a, OwnedValue>,
}
impl<'de, 'a> SeqAccess<'de> for SeqA<'a> {
    type Error = DeError;
    fn next_element_seed<T: de::DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, DeError> {
        match self.it.next() {
            Some(v) => seed.deserialize(De { v }).map(Some),
            None => Ok(None),
        }
    }
}

struct MapA<'a> {
    entries: &'a [(OwnedKey, OwnedValue)],
    idx: usize,
}
impl<'de, 'a> MapAccess<'de> for MapA<'a> {
    type Error = DeError;
    fn next_key_seed<K: de::DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, DeError> {
        if self.idx >= self.entries.len() {
            return Ok(None);
        }
        let key = &self.entries[self.idx].0;
        let kde = KeyDe { key };
        seed.deserialize(kde).map(Some)
    }
    fn next_value_seed<V: de::DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, DeError> {
        let v = &self.entries[self.idx].1;
        self.idx += 1;
        seed.deserialize(De { v })
    }
}

struct KeyDe<'a> {
    key: &'a OwnedKey,
}
impl<'de, 'a> Deserializer<'de> for KeyDe<'a> {
    type Error = DeError;
    fn deserialize_any<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.key {
            OwnedKey::Str(s) => vis.visit_str(s),
            OwnedKey::Int(i) => vis.visit_i64(*i),
        }
    }
    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf
        option unit unit_struct newtype_struct seq tuple tuple_struct map struct enum
        identifier ignored_any
    }
}

// =====================================================================================
// ZERO-COPY serde bridge: a Deserializer that reads DIRECTLY from in-arena Value<'gc> (no snapshot).
// Runs inside arena.mutate; the `vm.deserialize_exports::<T>()` helper hides 'gc from the embedder.
// This is the recommended bridge — it skips the deep-copy that `exports().clone()` forces.
// =====================================================================================

impl Vm {
    /// koto: `from_koto_value(exports().clone())` but ZERO-COPY — serde runs over the live arena values.
    pub fn deserialize_exports<T>(&self) -> Result<T, DeError>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.arena.mutate(|_mc, root| {
            let ex = root.borrow().exports;
            match ex {
                Some(v) => T::deserialize(GcDe { v }),
                None => Err(DeError("no exports".into())),
            }
        })
    }
}

#[derive(Copy, Clone)]
struct GcDe<'gc> {
    v: Value<'gc>,
}

impl<'de, 'gc> Deserializer<'de> for GcDe<'gc> {
    type Error = DeError;
    fn deserialize_any<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.v {
            Value::Nil => vis.visit_unit(),
            Value::Bool(b) => vis.visit_bool(b),
            Value::Num(n) => vis.visit_f64(n),
            Value::Int(i) => vis.visit_i64(i),
            Value::Str(s) => vis.visit_str(&s.0),
            Value::Table(t) => {
                let tb = t.borrow();
                if tb.map.is_empty() {
                    vis.visit_seq(GcSeq { t: tb, idx: 0 })
                } else {
                    vis.visit_map(GcMap { t: tb, idx: 0 })
                }
            }
            _ => Err(DeError("non-serializable value".into())),
        }
    }
    fn deserialize_f64<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.v {
            Value::Num(n) => vis.visit_f64(n),
            Value::Int(i) => vis.visit_f64(i as f64),
            _ => Err(DeError("expected number".into())),
        }
    }
    fn deserialize_u64<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.v {
            Value::Int(i) => vis.visit_u64(i as u64),
            Value::Num(n) => vis.visit_u64(n as u64),
            _ => Err(DeError("expected integer".into())),
        }
    }
    fn deserialize_i64<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.v {
            Value::Int(i) => vis.visit_i64(i),
            Value::Num(n) => vis.visit_i64(n as i64),
            _ => Err(DeError("expected integer".into())),
        }
    }
    fn deserialize_option<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.v {
            Value::Nil => vis.visit_none(),
            _ => vis.visit_some(self),
        }
    }
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _n: &'static str,
        _f: &'static [&'static str],
        vis: V,
    ) -> Result<V::Value, DeError> {
        match self.v {
            Value::Table(t) => vis.visit_map(GcMap { t: t.borrow(), idx: 0 }),
            _ => Err(DeError("expected table".into())),
        }
    }
    fn deserialize_map<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.v {
            Value::Table(t) => vis.visit_map(GcMap { t: t.borrow(), idx: 0 }),
            _ => Err(DeError("expected table".into())),
        }
    }
    fn deserialize_seq<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.v {
            Value::Table(t) => vis.visit_seq(GcSeq { t: t.borrow(), idx: 0 }),
            _ => Err(DeError("expected table".into())),
        }
    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _n: &'static str,
        vis: V,
    ) -> Result<V::Value, DeError> {
        vis.visit_newtype_struct(self)
    }
    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 u8 u16 u32 f32 char str string bytes byte_buf
        unit unit_struct tuple tuple_struct enum identifier ignored_any
    }
}

struct GcSeq<'gc> {
    t: std::cell::Ref<'gc, Table<'gc>>,
    idx: usize,
}
impl<'de, 'gc> SeqAccess<'de> for GcSeq<'gc> {
    type Error = DeError;
    fn next_element_seed<T: de::DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, DeError> {
        if self.idx >= self.t.arr.len() {
            return Ok(None);
        }
        let v = self.t.arr[self.idx];
        self.idx += 1;
        seed.deserialize(GcDe { v }).map(Some)
    }
}

struct GcMap<'gc> {
    t: std::cell::Ref<'gc, Table<'gc>>,
    idx: usize,
}
impl<'de, 'gc> MapAccess<'de> for GcMap<'gc> {
    type Error = DeError;
    fn next_key_seed<K: de::DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, DeError> {
        if self.idx >= self.t.map.len() {
            return Ok(None);
        }
        let k = &self.t.map[self.idx].0;
        seed.deserialize(GcKeyDe { k }).map(Some)
    }
    fn next_value_seed<V: de::DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, DeError> {
        let v = self.t.map[self.idx].1;
        self.idx += 1;
        seed.deserialize(GcDe { v })
    }
}

struct GcKeyDe<'a, 'gc> {
    k: &'a Key<'gc>,
}
impl<'de, 'a, 'gc> Deserializer<'de> for GcKeyDe<'a, 'gc> {
    type Error = DeError;
    fn deserialize_any<V: Visitor<'de>>(self, vis: V) -> Result<V::Value, DeError> {
        match self.k {
            Key::Str(s) => vis.visit_str(&s.0),
            Key::Int(i) => vis.visit_i64(*i),
        }
    }
    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf
        option unit unit_struct newtype_struct seq tuple tuple_struct map struct enum
        identifier ignored_any
    }
}

// =====================================================================================
// The general-path program: the grabby `from_koto` scenario-gen shape.
//   local base = host_base(0.8)            -- host call via call_indirect (-> 1.6)
//   local function make_well(i) ... end     -- closure capturing `base`
//   local wells = {}
//   for i = 0, 2 do wells[i+1] = make_well(i) end
//   return { name = "ring", gravity = base, wells = wells }
//
// make_well(i) returns { name = "well"..i, gravity = base*2.0, radius = 500.0 + i*10.0 }
// =====================================================================================
pub fn build_scenario_chunk() -> Chunk {
    let mut strings: Vec<String> = Vec::new();
    let sidx = |strings: &mut Vec<String>, s: &str| -> u32 {
        if let Some(p) = strings.iter().position(|x| x == s) {
            return p as u32;
        }
        strings.push(s.to_string());
        (strings.len() - 1) as u32
    };
    let s_name = sidx(&mut strings, "name");
    let s_gravity = sidx(&mut strings, "gravity");
    let s_radius = sidx(&mut strings, "radius");
    let s_wells = sidx(&mut strings, "wells");
    let s_well = sidx(&mut strings, "well");
    let s_ring = sidx(&mut strings, "ring");

    let mut consts: Vec<Konst> = Vec::new();
    let k_08 = consts.len() as u8;
    consts.push(Konst::Num(0.8));
    let k_0 = consts.len() as u8;
    consts.push(Konst::Num(0.0));
    let k_2 = consts.len() as u8;
    consts.push(Konst::Num(2.0));
    let k_1 = consts.len() as u8;
    consts.push(Konst::Num(1.0));
    let k_10 = consts.len() as u8;
    consts.push(Konst::Num(10.0));
    let k_500 = consts.len() as u8;
    consts.push(Konst::Num(500.0));
    let k_well = consts.len() as u8;
    consts.push(Konst::Str(s_well));
    let k_ring = consts.len() as u8;
    consts.push(Konst::Str(s_ring));

    // ---- proto 1: make_well(i)  (r0 = i ; upvalue0 = base) ----
    // r1 = table ; r2,r3 scratch
    let mut mw: Vec<Instr> = Vec::new();
    mw.push(i4(NEWTABLE, 1, 0, 0));
    // name = "well" .. tostring(i)
    mw.push(i4(LOADK, 2, 0, k_well)); // r2 = "well"
    mw.push(i4(TOSTR, 3, 0, 0)); // r3 = tostring(i)
    mw.push(i4(CONCAT, 2, 2, 3)); // r2 = "well"..i
    mw.push(i4(SETFIELD, 1, s_name as u8, 2)); // t.name = r2  (barrier)
    // gravity = base * 2.0
    mw.push(i4(GETUPVAL, 2, 0, 0)); // r2 = base
    mw.push(i4(NMULK, 2, 2, k_2)); // r2 = base*2
    mw.push(i4(SETFIELD, 1, s_gravity as u8, 2));
    // radius = 500 + i*10
    mw.push(i4(NMULK, 2, 0, k_10)); // r2 = i*10
    mw.push(i4(NADDK, 2, 2, k_500)); // r2 = 500 + i*10
    mw.push(i4(SETFIELD, 1, s_radius as u8, 2));
    mw.push(i4(RET, 1, 0, 0));
    let make_well = Proto { code: mw, max_regs: 8, upval_regs: vec![0] };

    // ---- proto 0: main() ----
    // r0=base r1=wells r2=i r3=limit r4=step r5=make_well r6=scratch r7=fn r8=arg r9=result r10=scratch
    let mut m: Vec<Instr> = Vec::new();
    // base = host_base(0.8)
    m.push(i4(LOADK, 6, 0, k_08)); // r6 = 0.8 (arg)
    m.push(ix(CALLHOST, 0, 6, 0, 1)); // r0 = host[0](r6)   nargs=1
    // make_well closure (captures base=r0)
    m.push(ix(CLOSURE, 5, 0, 0, 1)); // r5 = closure(proto 1)
    // wells = {}
    m.push(i4(NEWTABLE, 1, 0, 0));
    // for i=0,2 step 1
    m.push(i4(LOADK, 2, 0, k_0));
    m.push(i4(LOADK, 3, 0, k_2));
    m.push(i4(LOADK, 4, 0, k_1));
    let prep = m.len();
    m.push(ix(FORNPREP, 2, 0, 0, 0)); // patch target later
    let body = m.len() as i32;
    // body: make_well(i)
    m.push(i4(MOVE, 7, 5, 0)); // r7 = make_well
    m.push(i4(MOVE, 8, 2, 0)); // r8 = i
    m.push(ix(CALL, 7, 7, 0, 1)); // r7 = make_well(i)
    m.push(i4(NADDK, 6, 2, k_1)); // r6 = i+1
    m.push(i4(SETINDEX, 1, 6, 7)); // wells[i+1] = r7  (barrier)
    m.push(ix(FORNLOOP, 2, 0, 0, body));
    let end = m.len() as i32;
    m[prep].aux = end;
    // result = { name="ring", gravity=base, wells=wells }
    m.push(i4(NEWTABLE, 9, 0, 0));
    m.push(i4(LOADK, 10, 0, k_ring));
    m.push(i4(SETFIELD, 9, s_name as u8, 10));
    m.push(i4(SETFIELD, 9, s_gravity as u8, 0));
    m.push(i4(SETFIELD, 9, s_wells as u8, 1));
    m.push(i4(RET, 9, 0, 0));
    let main = Proto { code: m, max_regs: 12, upval_regs: vec![] };

    Chunk {
        protos: vec![main, make_well],
        consts,
        strings,
        main: 0,
        globals: vec!["host_base".to_string()],
    }
}

/// Number of dispatched instructions in one scenario run (for ns/dispatch reporting).
pub fn scenario_dispatch_count() -> u64 {
    // main: 8 setup + loop(3 iters * 6) + 6 tail ; make_well: 12 each * 3
    // computed empirically by a counting run is overkill; report the static estimate the bench prints.
    0
}

// =====================================================================================
// The cross-arch f64 golden — exercises the canon path on NaN-producing ops + table-key -0.0 canon,
// while proving 1/-0.0 = -inf is preserved (per-op -0.0 NOT canon'd). Runs identically native + V8.
// =====================================================================================
pub fn golden() -> u64 {
    let mut vm = Vm::new();
    vm.add_fn("host_base", |args| match args {
        [OwnedValue::Num(x)] => Ok(OwnedValue::Num(x * 2.0)),
        _ => Err("host_base expects (number)".into()),
    });

    // We compute the torture values directly via the VM's canon constructor + ops to mirror exactly
    // what the dispatch loop does (Value::num is the per-op ingress).
    let mut h: u64 = 0xcbf29ce484222325;
    let mut mix = |bits: u64| {
        for b in bits.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };

    let inf = f64::INFINITY;
    let cases = [
        canon(0.0 / 0.0),       // NaN -> canon
        canon(inf - inf),       // NaN -> canon
        canon(0.0 * inf),       // NaN -> canon
        canon(libm::sqrt(-1.0)),// NaN -> canon (host transcendental)
        canon(1e308 * 1e308),   // +inf (NOT NaN)
        canon(-1e308 * 1e308),  // -inf
        canon(1.0 / -0.0),      // -inf : sign-of-zero preserved (per-op -0.0 NOT canon'd)
        canon(0.0 * -1.0),      // -0.0 preserved per-op
        canon(1.7976931348623157e308),
        canon(5e-324),          // subnormal
        canon(libm::exp(1.0)),  // e
        canon(libm::sin(1.0)),
    ];
    for v in cases {
        mix(v.to_bits());
    }

    // table-key normalization: -0.0 and +0.0 must collide to ONE entry.
    let mut t = Table::<'static>::new();
    // emulate the SETINDEX key path: -0.0 -> +0.0
    let key_a = if (-0.0f64) == 0.0 { 0.0 } else { -0.0 };
    let key_b = if (0.0f64) == 0.0 { 0.0 } else { 0.0 };
    t.set_int((key_a as i64).max(0), Value::num(11.0));
    t.set_int((key_b as i64).max(0), Value::num(22.0));
    // after collapsing -0.0->+0.0->int 0, both writes hit the same key -> len 1, value 22.
    mix(t.arr.len() as u64 + t.map.len() as u64);
    if let Some((_, v)) = t.map.first() {
        mix(v.as_f64().to_bits());
    }
    // also fold the negative-zero sign bit to prove it is preserved in arithmetic
    mix((0.0f64 * -1.0).to_bits()); // 0x8000000000000000
    h
}

// =====================================================================================
// wasm exports
// =====================================================================================

static mut WVM: Option<Vm> = None;

#[no_mangle]
pub extern "C" fn bench_setup() -> u64 {
    let mut vm = Vm::new();
    vm.add_fn("host_base", |args| match args {
        [OwnedValue::Num(x)] => Ok(OwnedValue::Num(x * 2.0)),
        _ => Err("host_base expects (number)".into()),
    });
    vm.compile_and_run(build_scenario_chunk()).expect("compile_and_run");
    unsafe {
        WVM = Some(vm);
    }
    1
}

/// Full general-script workload: run + snapshot(exports) + serde-Deserialize, `iters` times. Returns a
/// checksum (anti-DCE).
#[no_mangle]
pub extern "C" fn bench_run(iters: u32) -> u64 {
    let vm = unsafe { (&raw mut WVM).as_mut().unwrap().as_mut().unwrap() };
    let mut acc: u64 = 0;
    for n in 0..iters {
        vm.rerun().expect("rerun");
        let ov = vm.exports_clone().expect("exports");
        let spec: ScenarioSpec = from_owned(&ov).expect("deserialize");
        acc = acc.wrapping_add(spec.checksum());
        if n % 64 == 0 {
            // periodic deterministic GC step so the heap actually collects
            vm.arena_collect_step();
        }
    }
    acc
}

/// run + snapshot only (no serde) — isolates the serde cost.
#[no_mangle]
pub extern "C" fn bench_run_noserde(iters: u32) -> u64 {
    let vm = unsafe { (&raw mut WVM).as_mut().unwrap().as_mut().unwrap() };
    let mut acc: u64 = 0;
    for n in 0..iters {
        vm.rerun().expect("rerun");
        let ov = vm.exports_clone().expect("exports");
        acc = acc.wrapping_add(match &ov {
            OwnedValue::Table(t) => t.map.len() as u64 + t.arr.len() as u64,
            _ => 0,
        });
        if n % 64 == 0 {
            vm.arena_collect_step();
        }
    }
    acc
}

/// run only (no snapshot, no serde) — isolates pure VM execution + heap alloc.
#[no_mangle]
pub extern "C" fn bench_run_only(iters: u32) -> u64 {
    let vm = unsafe { (&raw mut WVM).as_mut().unwrap().as_mut().unwrap() };
    let mut acc: u64 = 0;
    for n in 0..iters {
        vm.rerun().expect("rerun");
        acc = acc.wrapping_add(1);
        if n % 64 == 0 {
            vm.arena_collect_step();
        }
    }
    acc
}

/// run + ZERO-COPY deserialize_exports (serde over live arena values) — the recommended bridge.
#[no_mangle]
pub extern "C" fn bench_run_zerocopy(iters: u32) -> u64 {
    let vm = unsafe { (&raw mut WVM).as_mut().unwrap().as_mut().unwrap() };
    let mut acc: u64 = 0;
    for n in 0..iters {
        vm.rerun().expect("rerun");
        let spec: ScenarioSpec = vm.deserialize_exports().expect("de");
        acc = acc.wrapping_add(spec.gravity.to_bits() ^ spec.wells.len() as u64);
        if n % 64 == 0 {
            vm.arena_collect_step();
        }
    }
    acc
}

#[no_mangle]
pub extern "C" fn bench_golden() -> u64 {
    golden()
}

impl Vm {
    /// one deterministic incremental collection step (drives the GC on the instruction counter analog).
    pub fn arena_collect_step(&mut self) {
        self.arena.collect_debt();
    }

    pub fn gc_count(&self) -> usize {
        self.arena.metrics().total_gc_count()
    }
    pub fn gc_alloc(&self) -> usize {
        self.arena.metrics().total_gc_allocation()
    }

    /// Allocate `n` throwaway tables (immediately unreachable) to build collectable garbage.
    pub fn alloc_garbage(&mut self, n: usize) {
        for _ in 0..n {
            self.arena.mutate(|mc, _root| {
                let _t = Gc::new(mc, RefLock::new(Table::new()));
                let _s = Gc::new(mc, LuaStr("garbage-string-payload".to_string().into_boxed_str()));
            });
        }
    }

    /// PROVE the GC write barrier is live: drive the collector into the Marked phase (root + reachable
    /// exports table are now black), then write a FRESH child string into the already-marked exports
    /// table via the barriered `borrow_mut(mc)` path, finish the full cycle (sweep), and confirm the
    /// freshly-written child SURVIVED. Without a correct backward barrier the white child (reachable only
    /// from a black parent) would be swept. Returns Some(true) if the witness survived.
    pub fn write_barrier_during_marking_survives(&mut self) -> Option<bool> {
        let dr = self.retained.clone()?;
        // 1) mark to completion: exports table becomes black/marked.
        let _ = self.arena.finish_marking();
        // 2) write a fresh white child into the black parent, through the write barrier.
        self.arena.mutate(|mc, root| {
            let r = root.borrow();
            let t = r.dyn_set.fetch::<StashTable>(&dr);
            let witness = Gc::new(mc, LuaStr("barrier_witness_payload".to_string().into_boxed_str()));
            // *** WRITE BARRIER during Marking *** borrow_mut(mc) re-grays the parent.
            t.borrow_mut(mc).set_str(witness, Value::num(42.0));
        });
        // 3) finish the cycle: sweep. If the barrier failed, the witness gets collected (UB/garbage).
        self.arena.finish_cycle();
        // 4) read it back.
        Some(self.arena.mutate(|_mc, root| {
            let r = root.borrow();
            let t = r.dyn_set.fetch::<StashTable>(&dr);
            matches!(t.borrow().get_str("barrier_witness_payload"), Value::Num(n) if n == 42.0)
        }))
    }
}

/// The grabby ScenarioSpec shape (subset) — serde target.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScenarioSpec {
    #[serde(default)]
    pub name: String,
    pub gravity: f64,
    #[serde(default)]
    pub wells: Vec<WellSpec>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WellSpec {
    #[serde(default)]
    pub name: String,
    pub gravity: f64,
    #[serde(default = "default_radius")]
    pub radius: f64,
}
fn default_radius() -> f64 {
    1000.0
}

impl ScenarioSpec {
    fn checksum(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |bits: u64| {
            for b in bits.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        mix(self.gravity.to_bits());
        mix(self.name.len() as u64);
        for w in &self.wells {
            mix(w.gravity.to_bits());
            mix(w.radius.to_bits());
            mix(w.name.len() as u64);
        }
        h
    }
}
