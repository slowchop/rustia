// spike-snap: snapshot+restore microbench for grabby MP-rollback (24-deep ring, <=1ms restore)
// Representative per-agent AI state: Q-table(24 f64) + eligibility trace(24 f64) + mood + last_action.
// Compares: (a) host-columnar Vec clone, (b) slotmap-backed heap, (d) persistent im::Vector / rpds::Vector,
// (e) serialize-snapshot (bincode). All under a realistic "every agent mutates every tick" workload.

use serde::{Deserialize, Serialize};
use slotmap::{DefaultKey, SlotMap};
use std::hint::black_box;
use std::time::Instant;

const N_AGENTS: usize = 4000;
const NQ: usize = 24; // Q-table width / eligibility-trace width
const RING: usize = 24; // rollback ring depth

#[repr(C)]
#[derive(Clone, Copy, Serialize, Deserialize)]
struct AgentState {
    qtable: [f64; NQ],
    etrace: [f64; NQ],
    mood: f64,
    last_action: u32,
    _pad: u32,
}

impl AgentState {
    fn seed(i: usize) -> Self {
        let mut s = AgentState {
            qtable: [0.0; NQ],
            etrace: [0.0; NQ],
            mood: (i as f64 * 0.001).sin(),
            last_action: (i % NQ) as u32,
            _pad: 0,
        };
        for j in 0..NQ {
            s.qtable[j] = ((i * 31 + j * 7) % 97) as f64 * 0.01;
            s.etrace[j] = ((i * 13 + j * 5) % 53) as f64 * 0.001;
        }
        s
    }
    // One RL-ish step. `r` is a data-dependent reward (anti-constant-fold). Touches the WHOLE state.
    #[inline]
    fn step(&mut self, r: f64) -> f64 {
        let a = self.last_action as usize % NQ;
        let lr = 0.1;
        let gamma = 0.95;
        // decay full eligibility trace (full-state write each tick)
        for j in 0..NQ {
            self.etrace[j] *= gamma;
        }
        self.etrace[a] += 1.0;
        // TD update over q-table weighted by trace
        let target = r + gamma * self.qtable[a];
        let delta = target - self.qtable[a];
        for j in 0..NQ {
            self.qtable[j] += lr * delta * self.etrace[j];
            // keep bounded so repeated benching can't blow to inf/NaN
            if self.qtable[j] > 1e6 {
                self.qtable[j] = 1e6;
            } else if self.qtable[j] < -1e6 {
                self.qtable[j] = -1e6;
            }
        }
        self.mood = (self.mood * 0.99 + delta * 0.01).clamp(-1.0, 1.0);
        // argmax -> next action (data-dependent control flow)
        let mut best = 0usize;
        let mut bestv = self.qtable[0];
        for j in 1..NQ {
            if self.qtable[j] > bestv {
                bestv = self.qtable[j];
                best = j;
            }
        }
        self.last_action = best as u32;
        bestv
    }
}

// ---- timing harness: returns median ns/iter over `trials`, each running `iters` ops ----
fn bench<F: FnMut() -> f64>(iters: usize, trials: usize, warm: usize, mut f: F) -> f64 {
    let mut sink = 0.0f64;
    for _ in 0..warm {
        sink += f();
    }
    let mut samples = Vec::with_capacity(trials);
    for _ in 0..trials {
        let t = Instant::now();
        for _ in 0..iters {
            sink += f();
        }
        let ns = t.elapsed().as_nanos() as f64 / iters as f64;
        samples.push(ns);
    }
    black_box(sink);
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}

fn fmt(ns: f64) -> String {
    if ns >= 1000.0 {
        format!("{:>9.3} us", ns / 1000.0)
    } else {
        format!("{:>9.1} ns", ns)
    }
}

fn verdict(restore_ns: f64) -> &'static str {
    if restore_ns <= 1_000_000.0 {
        "PASS (<=1ms)"
    } else {
        "FAIL (>1ms)"
    }
}

fn main() {
    println!("spike-snap: N_AGENTS={N_AGENTS}, NQ={NQ}, RING={RING}");
    let bytes = N_AGENTS * std::mem::size_of::<AgentState>();
    println!(
        "AgentState = {} bytes; live state = {} KiB; full 24-deep ring (eager copy) = {:.2} MiB\n",
        std::mem::size_of::<AgentState>(),
        bytes / 1024,
        (bytes * RING) as f64 / (1024.0 * 1024.0)
    );

    // reward generator: data-dependent, varies each call (no constant folding)
    let mut rng: u64 = 0x9e3779b97f4a7c15;
    let mut next_r = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        ((rng >> 40) as f64 / (1u64 << 24) as f64) - 0.5
    };

    // ======================================================================
    // (a) HOST-COLUMNAR Vec<AgentState>  (AoS POD; SoA has identical byte cost)
    // ======================================================================
    {
        let mut live: Vec<AgentState> = (0..N_AGENTS).map(AgentState::seed).collect();
        // pre-allocate ring slots so steady-state snapshot reuses buffers (clone_from)
        let mut ring: Vec<Vec<AgentState>> = (0..RING).map(|_| live.clone()).collect();

        let tick = bench(200, 25, 20, || {
            let r = next_r();
            let mut acc = 0.0;
            for a in live.iter_mut() {
                acc += a.step(r);
            }
            black_box(&live);
            acc
        });
        let mut slot = 0usize;
        let snap = bench(500, 25, 50, || {
            ring[slot].clone_from(black_box(&live)); // reuse buffer (no malloc steady-state)
            slot = (slot + 1) % RING;
            black_box(&ring[slot]);
            ring[slot][0].mood
        });
        let mut slot2 = 0usize;
        let restore = bench(500, 25, 50, || {
            live.clone_from(black_box(&ring[slot2]));
            slot2 = (slot2 + 1) % RING;
            black_box(&live);
            live[0].mood
        });
        println!("(a) host-columnar Vec<AgentState> (clone_from, buffer reuse)");
        println!("    tick(mutate 4000):{}  snapshot:{}  restore:{}   => {}", fmt(tick), fmt(snap), fmt(restore), verdict(restore));
        // also report fresh-clone (malloc each time) snapshot for honesty
        let snap_fresh = bench(300, 15, 30, || {
            let c = black_box(&live).clone();
            let m = c[0].mood;
            black_box(c);
            m
        });
        println!("    [snapshot via fresh .clone() incl malloc:{}]\n", fmt(snap_fresh));
    }

    // ======================================================================
    // (b) SLOTMAP generational-index heap
    // ======================================================================
    {
        let mut sm: SlotMap<DefaultKey, AgentState> = SlotMap::with_capacity(N_AGENTS);
        let keys: Vec<DefaultKey> = (0..N_AGENTS).map(|i| sm.insert(AgentState::seed(i))).collect();
        let mut ring: Vec<SlotMap<DefaultKey, AgentState>> = (0..RING).map(|_| sm.clone()).collect();

        let tick = bench(200, 25, 20, || {
            let r = next_r();
            let mut acc = 0.0;
            for k in keys.iter() {
                acc += sm[*k].step(r);
            }
            black_box(&sm);
            acc
        });
        let mut slot = 0usize;
        let snap = bench(500, 25, 50, || {
            ring[slot].clone_from(black_box(&sm));
            slot = (slot + 1) % RING;
            black_box(&ring[slot]);
            0.0
        });
        let mut slot2 = 0usize;
        let restore = bench(500, 25, 50, || {
            sm.clone_from(black_box(&ring[slot2]));
            slot2 = (slot2 + 1) % RING;
            black_box(&sm);
            0.0
        });
        println!("(b) slotmap SlotMap<DefaultKey, AgentState> (clone_from)");
        println!("    tick(mutate 4000):{}  snapshot:{}  restore:{}   => {}\n", fmt(tick), fmt(snap), fmt(restore), verdict(restore));
    }

    // ======================================================================
    // (d) PERSISTENT im::Vector<AgentState>  (structural sharing, O(log n) CoW)
    // ======================================================================
    {
        let mut live: im::Vector<AgentState> = (0..N_AGENTS).map(AgentState::seed).collect();
        let mut ring: Vec<im::Vector<AgentState>> = (0..RING).map(|_| live.clone()).collect();

        // tick: every agent mutated -> CoW path per element (the realistic grabby workload)
        let tick = bench(50, 15, 10, || {
            let r = next_r();
            let mut acc = 0.0;
            for i in 0..N_AGENTS {
                let s = live.get_mut(i).unwrap();
                acc += s.step(r);
            }
            black_box(&live);
            acc
        });
        // snapshot = O(1) handle clone (Arc bump); but note: a snapshot held while
        // live keeps mutating forces CoW of every touched chunk next tick (no real saving here).
        let mut slot = 0usize;
        let snap = bench(2000, 25, 100, || {
            ring[slot] = black_box(&live).clone();
            slot = (slot + 1) % RING;
            black_box(&ring[slot]);
            0.0
        });
        let mut slot2 = 0usize;
        let restore = bench(2000, 25, 100, || {
            live = black_box(&ring[slot2]).clone();
            slot2 = (slot2 + 1) % RING;
            black_box(&live);
            0.0
        });
        println!("(d) persistent im::Vector<AgentState> (O(1) clone snapshot, CoW mutate)");
        println!("    tick(mutate 4000):{}  snapshot:{}  restore:{}   => {}", fmt(tick), fmt(snap), fmt(restore), verdict(restore));
        // sanity: tick WITH a live snapshot retained (forces full CoW each tick)
        let _snapshot_held = live.clone();
        let tick_held = bench(50, 15, 10, || {
            let r = next_r();
            let mut acc = 0.0;
            for i in 0..N_AGENTS {
                acc += live.get_mut(i).unwrap().step(r);
            }
            black_box(&live);
            acc
        });
        black_box(_snapshot_held);
        println!("    [tick while 1 snapshot retained (full CoW):{}]\n", fmt(tick_held));
    }

    // ======================================================================
    // (e) SERIALIZE-snapshot: bincode encode/decode of Vec<AgentState>
    // ======================================================================
    {
        let mut live: Vec<AgentState> = (0..N_AGENTS).map(AgentState::seed).collect();
        // mutate a little so it's live
        for a in live.iter_mut() { a.step(0.1); }
        let encoded = bincode::serialize(&live).unwrap();
        println!("(e) serialize-snapshot (bincode)  encoded size = {} KiB", encoded.len() / 1024);
        let snap = bench(200, 15, 20, || {
            let v = bincode::serialize(black_box(&live)).unwrap();
            let n = v.len() as f64;
            black_box(v);
            n
        });
        let restore = bench(200, 15, 20, || {
            let v: Vec<AgentState> = bincode::deserialize(black_box(&encoded)).unwrap();
            let m = v[0].mood;
            black_box(v);
            m
        });
        println!("    snapshot(serialize):{}  restore(deserialize):{}   => {}\n", fmt(snap), fmt(restore), verdict(restore));
    }

    // ======================================================================
    // (d') PERSISTENT rpds::Vector  (fully-persistent, set_mut CoW)
    // ======================================================================
    {
        use rpds::Vector;
        let mut live: Vector<AgentState> = Vector::new();
        for i in 0..N_AGENTS {
            live.push_back_mut(AgentState::seed(i));
        }
        let mut ring: Vec<Vector<AgentState>> = (0..RING).map(|_| live.clone()).collect();
        let tick = bench(30, 11, 6, || {
            let r = next_r();
            let mut acc = 0.0;
            for i in 0..N_AGENTS {
                let mut s = *live.get(i).unwrap();
                acc += s.step(r);
                live.set_mut(i, s);
            }
            black_box(&live);
            acc
        });
        let mut slot = 0usize;
        let snap = bench(2000, 25, 100, || {
            ring[slot] = black_box(&live).clone();
            slot = (slot + 1) % RING;
            black_box(&ring[slot]);
            0.0
        });
        let mut slot2 = 0usize;
        let restore = bench(2000, 25, 100, || {
            live = black_box(&ring[slot2]).clone();
            slot2 = (slot2 + 1) % RING;
            black_box(&live);
            0.0
        });
        println!("(d') persistent rpds::Vector<AgentState> (O(1) clone snapshot, set_mut CoW)");
        println!("    tick(mutate 4000):{}  snapshot:{}  restore:{}   => {}\n", fmt(tick), fmt(snap), fmt(restore), verdict(restore));
    }
}
