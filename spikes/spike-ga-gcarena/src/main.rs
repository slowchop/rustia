//! gc-arena 0.6.0 micro-spike for rustia.
//! Tests: (1) Send gate, (2) serde bridge, (3) steppable fuel-driven collect, (4) ergonomics.

use gc_arena::lock::RefLock;
use gc_arena::{Arena, Collect, Gc, Mutation, Rootable, Static};

// ---- A small scripting-style value graph ----
// String is not 'gc-free Collect by default, so we park host-owned leaf data in `Static`.
#[derive(Collect, Clone)]
#[collect(no_drop)]
enum Value<'gc> {
    Nil,
    Num(f64),
    Str(Gc<'gc, Static<String>>),
    Table(Gc<'gc, RefLock<Table<'gc>>>),
}

#[derive(Collect)]
#[collect(no_drop)]
struct Table<'gc> {
    // association list is enough to prove nested Gc graphs
    entries: Vec<(Static<String>, Value<'gc>)>,
}

// Root of the arena: a top-level table.
#[derive(Collect)]
#[collect(no_drop)]
struct Root<'gc> {
    globals: Gc<'gc, RefLock<Table<'gc>>>,
}

type MyArena = Arena<Rootable![Root<'_>]>;

fn build() -> MyArena {
    Arena::<Rootable![Root<'_>]>::new(|mc| {
        let t = Gc::new(mc, RefLock::new(Table { entries: Vec::new() }));
        // nested table holding a string + number
        let inner = Gc::new(mc, RefLock::new(Table { entries: Vec::new() }));
        inner.unlock(mc).borrow_mut().entries.push((
            Static("greeting".to_string()),
            Value::Str(Gc::new(mc, Static("hello".to_string()))),
        ));
        inner
            .unlock(mc)
            .borrow_mut()
            .entries
            .push((Static("n".to_string()), Value::Num(42.0)));
        t.unlock(mc)
            .borrow_mut()
            .entries
            .push((Static("inner".to_string()), Value::Table(inner)));
        Root { globals: t }
    })
}

// ============ (1) SEND GATE ============
fn assert_send<T: Send>() {}

// A sound Send wrapper. SAFETY invariant documented below.
struct SendArena(MyArena);
// SAFETY: `Arena` is !Send only because (a) its `Context` holds raw `NonNull` GcBox
// pointers and (b) `Metrics` holds a non-atomic `Rc<MetricsInner>`. Both are confined
// to the arena: Gc pointers are branded by the invariant 'gc lifetime and provably
// cannot escape a `mutate` call, and the only way to obtain an external clone of the
// `Rc` is `Arena::metrics()`. If the embedding NEVER retains a `Metrics` handle (or any
// arena-derived handle) outside the arena across a thread boundary, then moving the whole
// arena to another thread is a unique-owner move with no aliasing and no cross-thread
// refcount race. We never expose `metrics()` from `SendArena`, so the invariant holds.
unsafe impl Send for SendArena {}

fn test_send() {
    // This line is the gate. Toggle to observe the raw compiler error.
    #[cfg(feature = "prove_not_send")]
    assert_send::<MyArena>();

    // The wrapper is Send:
    assert_send::<SendArena>();

    // Prove we can actually MOVE it to another thread and keep using it there.
    let arena = SendArena(build());
    let handle = std::thread::spawn(move || {
        let a = arena;
        a.0.mutate(|_mc, root| {
            let g = root.globals.borrow();
            g.entries.len()
        })
    });
    let n = handle.join().unwrap();
    println!("[SEND] moved arena to pool thread; root table len = {n}");
}

// ============ (2) SERDE ============
// Direction A (C4-relevant): read a script Value graph OUT into a Rust struct.
// Reading only needs &-refs, no Mutation, so the branded lifetime is NOT an obstacle.
// We show it via JSON as the intermediary (a real impl would be a serde::Deserializer
// over &Value, identical lifetime story).
fn value_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Nil => J::Null,
        Value::Num(n) => J::from(*n),
        Value::Str(s) => J::String(s.0.clone()),
        Value::Table(t) => {
            let t = t.borrow();
            let mut m = serde_json::Map::new();
            for (k, val) in &t.entries {
                m.insert(k.0.clone(), value_to_json(val));
            }
            J::Object(m)
        }
    }
}

// Direction B (harder): deserialize external bytes INTO a fresh arena. Allocation needs
// `&Mutation<'gc>`, which std `Deserialize` cannot thread. The idiomatic fix is a
// `DeserializeSeed` carrying `&Mutation`; here we show the equivalent hand-written
// recursive builder running INSIDE `mutate` (a DeserializeSeed is mechanically the same).
fn json_to_value<'gc>(mc: &Mutation<'gc>, j: &serde_json::Value) -> Value<'gc> {
    use serde_json::Value as J;
    match j {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Num(if *b { 1.0 } else { 0.0 }),
        J::Number(n) => Value::Num(n.as_f64().unwrap()),
        J::String(s) => Value::Str(Gc::new(mc, Static(s.clone()))),
        J::Array(a) => {
            let entries = a
                .iter()
                .enumerate()
                .map(|(i, e)| (Static(i.to_string()), json_to_value(mc, e)))
                .collect();
            Value::Table(Gc::new(mc, RefLock::new(Table { entries })))
        }
        J::Object(o) => {
            let entries = o
                .iter()
                .map(|(k, e)| (Static(k.clone()), json_to_value(mc, e)))
                .collect();
            Value::Table(Gc::new(mc, RefLock::new(Table { entries })))
        }
    }
}

fn test_serde() {
    let arena = build();
    // Serialize out
    let json = arena.mutate(|_mc, root| {
        let v = Value::Table(root.globals);
        value_to_json(&v)
    });
    let s = serde_json::to_string(&json).unwrap();
    println!("[SERDE] serialized graph -> {s}");

    // Round-trip back into a FRESH arena
    let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
    let arena2 = Arena::<Rootable![Root<'_>]>::new(|mc| {
        let v = json_to_value(mc, &parsed);
        let globals = match v {
            Value::Table(t) => t,
            _ => Gc::new(mc, RefLock::new(Table { entries: Vec::new() })),
        };
        Root { globals }
    });
    let s2 = arena2.mutate(|_mc, root| {
        serde_json::to_string(&value_to_json(&Value::Table(root.globals))).unwrap()
    });
    println!("[SERDE] round-tripped into fresh arena -> {s2}");
    assert_eq!(s, s2, "round trip must be identical");
    println!("[SERDE] round trip identical: OK");
}

// ============ (3) STEPPABLE: fuel-driven collection ============
// VM root that retains a "live" table and lets us churn garbage.
#[derive(Collect)]
#[collect(no_drop)]
struct VmRoot<'gc> {
    live: Gc<'gc, RefLock<Vec<Value<'gc>>>>,
}
type VmArena = Arena<Rootable![VmRoot<'_>]>;

fn test_steppable() {
    let mut arena = VmArena::new(|mc| VmRoot {
        live: Gc::new(mc, RefLock::new(Vec::new())),
    });

    let fuel_per_step: i64 = 1000;
    let mut total_fuel: i64 = 0;
    let collections_at_start = arena.metrics().total_gc_count();

    // Stackless step loop: mutate (allocate garbage + a little retained), then at the
    // fuel boundary drive GC deterministically. mutate and collect never overlap.
    for step in 0..2000i64 {
        // ---- MUTATE phase (no collection happens here) ----
        arena.mutate(|mc, root| {
            // allocate transient garbage that is immediately dropped (unreachable)
            for _ in 0..10 {
                let _garbage = Gc::new(mc, Static(format!("garbage-{step}")));
            }
            // occasionally retain something reachable from root
            if step % 100 == 0 {
                root.live
                    .unlock(mc)
                    .borrow_mut()
                    .push(Value::Num(step as f64));
            }
        });

        // ---- deterministic fuel accounting ----
        total_fuel += fuel_per_step;

        // ---- COLLECT phase, driven purely by fuel (not wall-clock/alloc pressure) ----
        // We translate "fuel spent this step" into GC debt and pay it down. Because we
        // call collect_debt only at the step boundary, collection is fully deterministic
        // w.r.t. the fuel schedule.
        arena.metrics().adjust_debt(fuel_per_step as f64);
        arena.collect_debt();
    }

    let heap_bytes = arena.metrics().total_gc_allocation();
    let collections = arena.metrics().total_gc_count() - collections_at_start;
    let live_len = arena.mutate(|_mc, root| root.live.borrow().len());
    println!(
        "[STEPPABLE] {total_fuel} fuel over 2000 steps; live retained = {live_len}; \
         total heap bytes ever = {heap_bytes}; gc cycles run = {collections}"
    );
    // Force a full cycle at a chosen boundary (deterministic checkpoint).
    arena.finish_cycle();
    println!("[STEPPABLE] explicit finish_cycle ran at chosen checkpoint: OK");
}

fn main() {
    test_send();
    test_serde();
    test_steppable();
    println!("ALL SPIKE TESTS RAN");
}
