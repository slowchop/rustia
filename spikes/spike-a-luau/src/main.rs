//! Spike A: measure Luau us/decision on the grabby REAL AI kernel.
//! Ports the lib.rs Q16.16 kernel (IAUS product + momentum + eps-greedy + SARSA(lambda) + contagion).
//! Native floor is the exact lib.rs port (anchor). Luau paths measured 3 ways:
//!   (a) realistic: one Luau argmax call per agent per tick (matches grabby KotoSelector shape);
//!   (b) vm-internal: whole agent loop inside ONE Luau call per tick (isolates VM speed from FFI);
//!   (c) empty: empty Luau fn call per agent (pure mlua entry overhead).
//! f64 Luau math used for the considerations (perf proxy, matches grabby koto-float; NOT the
//! determinism-constrained path).

use mlua::{Function, Lua, Value};
use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};
use std::time::Instant;

// ---- Q16.16 (verbatim from grabby lib.rs) ----
pub const ONE: i32 = 1 << 16;
#[inline]
pub fn fx_mul(a: i32, b: i32) -> i32 { (((a as i64) * (b as i64)) >> 16) as i32 }
#[inline]
pub fn clamp01(x: i32) -> i32 { x.clamp(0, ONE) }

const LUT_N: usize = 256;
pub struct Luts { logistic: [i32; LUT_N + 1] }
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
fn curve_kind(luts: &Luts, kind: usize, input: i32) -> i32 {
    match kind {
        0 => luts.logistic(input),
        1 => fx_mul(input, input),
        _ => clamp01(ONE - input),
    }
}
#[inline]
fn native_scores(luts: &Luts, inputs: &[i32; N_CONSIDERATIONS]) -> [i32; N_ACTIONS] {
    let mut scores = [0i32; N_ACTIONS];
    for (a, s) in scores.iter_mut().enumerate() {
        let mut prod = ONE;
        for (c, &input) in inputs.iter().enumerate() {
            prod = fx_mul(prod, curve_kind(luts, (a + c) % 3, input));
        }
        *s = prod;
    }
    scores
}
#[inline]
fn argmax_momentum(scores: &[i32; N_ACTIONS], last: u8) -> u8 {
    let mut s = *scores;
    s[last as usize] = fx_mul(s[last as usize], MOMENTUM);
    let mut best = 0usize;
    let mut best_v = s[0];
    for (a, &v) in s.iter().enumerate().skip(1) {
        if v > best_v { best_v = v; best = a; }
    }
    best as u8
}

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
    luts: Luts,
}
impl World {
    pub fn new(n: usize, world_seed: u64) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(world_seed);
        let mut inputs = Vec::with_capacity(n);
        let mut neighbors = Vec::with_capacity(n);
        for i in 0..n {
            let mut inp = [0i32; N_CONSIDERATIONS];
            for c in inp.iter_mut() { *c = (rng.next_u32() % (ONE as u32 + 1)) as i32; }
            inputs.push(inp);
            let mut nb = [0u32; N_NEIGHBORS];
            for slot in nb.iter_mut() { *slot = if n > 1 { rng.next_u32() % n as u32 } else { 0 }; }
            for slot in nb.iter_mut() { if *slot == i as u32 && n > 1 { *slot = (*slot + 1) % n as u32; } }
            neighbors.push(nb);
        }
        World {
            n, tick: 0, inputs,
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
        for t in self.trace[i].iter_mut() { *t = fx_mul(*t, GAMMA_LAMBDA); }
        self.trace[i][a] = ONE;
        let delta = reward - self.q[i][a];
        let step = fx_mul(ALPHA, delta);
        for (qa, &ta) in self.q[i].iter_mut().zip(self.trace[i].iter()) { *qa += fx_mul(step, ta); }
    }
    #[inline]
    fn contagion(&mut self) {
        self.mood_prev.copy_from_slice(&self.mood);
        for i in 0..self.n {
            let me = self.mood_prev[i];
            let mut acc = 0i32;
            for &nb in &self.neighbors[i] { acc += self.mood_prev[nb as usize] - me; }
            self.mood[i] = (self.mood[i] + fx_mul(CONTAGION_K, acc)).clamp(-ONE, ONE);
        }
    }
    // native tick (the floor)
    pub fn tick_native(&mut self) {
        self.tick += 1;
        for i in 0..self.n {
            let inputs = self.inputs[i];
            let last = self.last_action[i];
            let mut rng = ChaCha8Rng::seed_from_u64(fnv1a(i as u64, self.tick));
            let scores = native_scores(&self.luts, &inputs);
            let mut action = argmax_momentum(&scores, last);
            if (rng.next_u32() & 1023) < EPS_PPK { action = (rng.next_u32() % N_ACTIONS as u32) as u8; }
            let reward = inputs[action as usize % N_CONSIDERATIONS];
            self.learn(i, action, reward);
            self.last_action[i] = action;
            let drift = (self.mood[i] >> 6) + ((reward - ONE / 2) >> 6);
            for c in self.inputs[i].iter_mut() { *c = clamp01(*c + drift); }
            self.mood[i] = (self.mood[i] + ((reward - ONE / 2) >> 4)).clamp(-ONE, ONE);
        }
        self.contagion();
    }
    // realistic Luau tick: select() via one Luau call per agent; rest in Rust (matches grabby koto path)
    pub fn tick_luau_realistic(&mut self, argmax: &Function) {
        self.tick += 1;
        for i in 0..self.n {
            let inputs = self.inputs[i];
            let last = self.last_action[i];
            let mut rng = ChaCha8Rng::seed_from_u64(fnv1a(i as u64, self.tick));
            // marshal as f64 in [0,1] (matches grabby KotoFloatSelector)
            let f = |x: i32| x as f64 / 65536.0;
            let best: i64 = argmax
                .call((f(inputs[0]), f(inputs[1]), f(inputs[2]), f(inputs[3]), last as i64))
                .expect("argmax call");
            let mut action = best as u8;
            if (rng.next_u32() & 1023) < EPS_PPK { action = (rng.next_u32() % N_ACTIONS as u32) as u8; }
            let reward = inputs[action as usize % N_CONSIDERATIONS];
            self.learn(i, action, reward);
            self.last_action[i] = action;
            let drift = (self.mood[i] >> 6) + ((reward - ONE / 2) >> 6);
            for c in self.inputs[i].iter_mut() { *c = clamp01(*c + drift); }
            self.mood[i] = (self.mood[i] + ((reward - ONE / 2) >> 4)).clamp(-ONE, ONE);
        }
        self.contagion();
    }
}

// ---- Luau scripts ----

// realistic: just the IAUS argmax (float proxy, mirrors grabby SCRIPT_FLOAT exactly)
const ARGMAX_SRC: &str = r#"
local function argmax(i0, i1, i2, i3, last)
  local inputs = {i0, i1, i2, i3}
  local best = 0
  local best_v = -1.0
  for a = 0, 5 do
    local prod = 1.0
    for c = 0, 3 do
      local x = inputs[c+1]
      local kind = (a + c) % 3
      local cv
      if kind == 0 then cv = x * x
      elseif kind == 1 then cv = x
      else cv = 1.0 - x end
      prod = prod * cv
    end
    if a == last then prod = prod * 1.25 end
    if prod > best_v then best_v = prod; best = a end
  end
  return best
end
return argmax
"#;

// vm-internal: the WHOLE per-agent loop (select + eps-greedy + SARSA(lambda) + input-evolve + mood)
// plus the contagion pass, run inside ONE Luau call per tick. Flat 1-based tables. f64 throughout.
// A cheap per-agent LCG stands in for the eps-greedy draw (representative op, not the golden RNG).
const TICK_SRC: &str = r#"
-- globals set from Rust: N, inputs(flat n*4), q(flat n*6), trace(flat n*6),
-- last_action(n), mood(n), neighbors(flat n*4), mood_prev(n)
local floor = math.floor
local function tickfn(tk)
  for i = 0, N - 1 do
    local b4 = i * 4
    local b6 = i * 6
    local i0 = inputs[b4+1]; local i1 = inputs[b4+2]; local i2 = inputs[b4+3]; local i3 = inputs[b4+4]
    local last = last_action[i+1]
    -- IAUS product + momentum + argmax
    local cons = {i0, i1, i2, i3}
    local best = 0
    local best_v = -1.0
    for a = 0, 5 do
      local prod = 1.0
      for c = 0, 3 do
        local x = cons[c+1]
        local kind = (a + c) % 3
        local cv
        if kind == 0 then cv = x * x
        elseif kind == 1 then cv = x
        else cv = 1.0 - x end
        prod = prod * cv
      end
      if a == last then prod = prod * 1.25 end
      if prod > best_v then best_v = prod; best = a end
    end
    -- eps-greedy via cheap LCG (representative explore draw)
    local r = (i * 2654435761 + tk * 40503) % 1024
    local action = best
    if r < 51 then
      local r2 = (i * 1103515245 + tk * 12345 + 7) % 6
      action = r2
    end
    -- reward = input[action % 4]
    local rewardIdx = action % 4
    local reward = cons[rewardIdx+1]
    -- SARSA(lambda) replacing trace
    for a = 0, 5 do trace[b6+a+1] = trace[b6+a+1] * 0.9 end
    trace[b6+action+1] = 1.0
    local delta = reward - q[b6+action+1]
    local step = 0.1 * delta
    for a = 0, 5 do q[b6+a+1] = q[b6+a+1] + step * trace[b6+a+1] end
    last_action[i+1] = action
    -- input evolution
    local md = mood[i+1]
    local drift = md / 64.0 + (reward - 0.5) / 64.0
    inputs[b4+1] = math.max(0.0, math.min(1.0, i0 + drift))
    inputs[b4+2] = math.max(0.0, math.min(1.0, i1 + drift))
    inputs[b4+3] = math.max(0.0, math.min(1.0, i2 + drift))
    inputs[b4+4] = math.max(0.0, math.min(1.0, i3 + drift))
    mood[i+1] = math.max(-1.0, math.min(1.0, md + (reward - 0.5) / 16.0))
  end
  -- contagion (snapshot then apply)
  for i = 1, N do mood_prev[i] = mood[i] end
  for i = 0, N - 1 do
    local me = mood_prev[i+1]
    local nb = i * 4
    local acc = (mood_prev[neighbors[nb+1]+1] - me)
              + (mood_prev[neighbors[nb+2]+1] - me)
              + (mood_prev[neighbors[nb+3]+1] - me)
              + (mood_prev[neighbors[nb+4]+1] - me)
    mood[i+1] = math.max(-1.0, math.min(1.0, mood[i+1] + 0.05 * acc))
  end
end
return tickfn
"#;

fn bench_native(n: usize, ticks: usize) -> f64 {
    let mut w = World::new(n, 0xA1_BACE_50);
    for _ in 0..(ticks / 10).max(1) { w.tick_native(); } // warmup
    let t0 = Instant::now();
    for _ in 0..ticks { w.tick_native(); }
    let el = t0.elapsed();
    el.as_nanos() as f64 / (n as f64 * ticks as f64) / 1000.0
}

fn bench_luau_realistic(lua: &Lua, n: usize, ticks: usize) -> f64 {
    let argmax: Function = lua.load(ARGMAX_SRC).eval().expect("load argmax");
    let mut w = World::new(n, 0xA1_BACE_50);
    for _ in 0..(ticks / 10).max(1) { w.tick_luau_realistic(&argmax); }
    let t0 = Instant::now();
    for _ in 0..ticks { w.tick_luau_realistic(&argmax); }
    let el = t0.elapsed();
    el.as_nanos() as f64 / (n as f64 * ticks as f64) / 1000.0
}

fn bench_luau_vm_internal(lua: &Lua, n: usize, ticks: usize) -> f64 {
    // init world in Rust, copy flat arrays into Luau globals
    let w = World::new(n, 0xA1_BACE_50);
    let g = lua.globals();
    g.set("N", n as i64).unwrap();
    let inputs = lua.create_table().unwrap();
    let q = lua.create_table().unwrap();
    let trace = lua.create_table().unwrap();
    let last_action = lua.create_table().unwrap();
    let mood = lua.create_table().unwrap();
    let mood_prev = lua.create_table().unwrap();
    let neighbors = lua.create_table().unwrap();
    for i in 0..n {
        for c in 0..N_CONSIDERATIONS { inputs.set(i * 4 + c + 1, w.inputs[i][c] as f64 / 65536.0).unwrap(); }
        for a in 0..N_ACTIONS { q.set(i * 6 + a + 1, 0.0).unwrap(); trace.set(i * 6 + a + 1, 0.0).unwrap(); }
        last_action.set(i + 1, 0i64).unwrap();
        mood.set(i + 1, 0.0).unwrap();
        mood_prev.set(i + 1, 0.0).unwrap();
        for k in 0..N_NEIGHBORS { neighbors.set(i * 4 + k + 1, w.neighbors[i][k] as i64).unwrap(); }
    }
    g.set("inputs", inputs).unwrap();
    g.set("q", q).unwrap();
    g.set("trace", trace).unwrap();
    g.set("last_action", last_action).unwrap();
    g.set("mood", mood).unwrap();
    g.set("mood_prev", mood_prev).unwrap();
    g.set("neighbors", neighbors).unwrap();
    let tickfn: Function = lua.load(TICK_SRC).eval().expect("load tickfn");
    for t in 0..(ticks / 10).max(1) { tickfn.call::<()>(t as i64).unwrap(); }
    let t0 = Instant::now();
    for t in 0..ticks { tickfn.call::<()>(t as i64).unwrap(); }
    let el = t0.elapsed();
    el.as_nanos() as f64 / (n as f64 * ticks as f64) / 1000.0
}

fn bench_empty_call(lua: &Lua, n: usize, ticks: usize) -> f64 {
    let f: Function = lua.load("return function() return 0 end").eval().expect("empty fn");
    let mut acc: i64 = 0;
    // warmup
    for _ in 0..n { acc += f.call::<i64>(()).unwrap(); }
    let t0 = Instant::now();
    for _ in 0..(n * ticks) {
        acc += f.call::<i64>(()).unwrap();
    }
    let el = t0.elapsed();
    std::hint::black_box(acc);
    el.as_nanos() as f64 / (n as f64 * ticks as f64) / 1000.0
}

fn main() {
    let lua = Lua::new();
    let sizes = [1000usize, 5000, 10000];
    // pick ticks so total work is comparable; native fast -> more ticks
    println!("# Spike A: Luau us/decision on grabby AI kernel (single-threaded, --release opt-level 3)");
    println!("# machine: aarch64-mac. mlua 0.11 luau+vendored.\n");

    // empty call overhead (independent of n; measure once with big count)
    let empty = bench_empty_call(&lua, 100_000, 5);
    println!("EMPTY Luau function call overhead: {:.4} us/call\n", empty);

    println!("{:>7} | {:>12} | {:>16} | {:>16}", "agents", "native us/d", "luau-real us/d", "luau-vm us/d");
    println!("{}", "-".repeat(64));
    for &n in &sizes {
        let nat_ticks = if n >= 10000 { 200 } else { 500 };
        let luau_ticks = if n >= 10000 { 50 } else { 100 };
        let nat = bench_native(n, nat_ticks);
        let real = bench_luau_realistic(&lua, n, luau_ticks);
        // fresh Lua for vm-internal to avoid global clashes
        let lua2 = Lua::new();
        let vm = bench_luau_vm_internal(&lua2, n, luau_ticks);
        println!("{:>7} | {:>12.4} | {:>16.4} | {:>16.4}", n, nat, real, vm);
    }
    println!("\n# us/d = microseconds per decision (per agent per tick) = tick_time / n");
}
