use rustia::{Rustia, RustiaBuilder, Value, ValueType, VmMemoryStats, VmSettings};
use std::collections::VecDeque;
use std::num::NonZeroU64;

const MEMORY_LIMIT_BYTES: usize = 2 * 1024 * 1024;
const DECISION_COUNT: usize = 96;
const WAVES_PER_DECISION: usize = 32;
const RETAINED_ROOT_COUNT: usize = 4;
const PHASE_WARMUP_WAVES: usize = 24;
const DECISION_WARMUP_COUNT: usize = 16;
const OVERSIZED_BUFFER_BYTES: usize = 8 * 1024 * 1024;

const PRESSURE_MODULE: &str = r#"
local ring = {}
local weak = setmetatable({}, { __mode = "kv" })
local sequence = 0

local function make_root()
    sequence += 1
    local seed = sequence
    local root = {
        seed = seed,
        nodes = {},
    }
    root.self = root

    for i = 1, 16 do
        local node = {
            owner = root,
            index = i,
            label = string.rep("x", 24 + (i % 5)),
        }
        if i > 1 then
            node.previous = root.nodes[i - 1]
        end
        root.nodes[i] = node
    end
    root.nodes[#root.nodes].next = root.nodes[1]

    root.bytes = buffer.create(1024 + ((seed % 4) * 64))
    for i = 0, 31 do
        buffer.writeu8(root.bytes, i, (seed + i) % 256)
    end
    root.vector = vector.create(seed % 17, (seed + 1) % 17, (seed + 2) % 17)
    root.proxy = newproxy(true)
    getmetatable(root.proxy).owner = root
    root.closure = function(delta)
        return root.seed + delta + buffer.readu8(root.bytes, 0)
    end
    root.thread = coroutine.create(function()
        coroutine.yield(root)
        return root.closure(2)
    end)

    local resumed, yielded = coroutine.resume(root.thread)
    assert(resumed and yielded == root)
    weak[root] = root.nodes[1]

    local slot = ((sequence - 1) % 4) + 1
    ring[slot] = root
    return root
end

local function collect()
    collectgarbage("collect")
    collectgarbage("collect")
    local weak_roots = 0
    for _ in pairs(weak) do
        weak_roots += 1
    end
    return weak_roots
end

local function validate()
    collectgarbage("collect")
    local live_roots = 0
    for _, root in pairs(ring) do
        assert(root.self == root)
        assert(root.nodes[#root.nodes].next == root.nodes[1])
        assert(type(root.bytes) == "buffer")
        assert(type(root.vector) == "vector")
        assert(type(root.proxy) == "userdata")
        assert(coroutine.status(root.thread) == "suspended")
        assert(root.closure(1) == root.seed + 1 + buffer.readu8(root.bytes, 0))
        live_roots += 1
    end

    local weak_roots = 0
    for root, node in pairs(weak) do
        assert(root.self == root and node.owner == root)
        weak_roots += 1
    end
    return live_roots == 4 and weak_roots == 4, weak_roots
end

local function probe()
    return sequence
end

local function oom_probe()
    local sentinel = ring[1]
    local before = sentinel.closure(0)
    local ok, message = pcall(buffer.create, __OVERSIZED_BUFFER_BYTES__)
    local after = buffer.create(32)
    buffer.writeu8(after, 0, 73)
    return not ok
        and string.find(tostring(message), "not enough memory", 1, true) ~= nil
        and sentinel.closure(0) == before
        and buffer.readu8(after, 0) == 73
end

local function clear()
    for i = 1, 4 do
        ring[i] = nil
    end
    collectgarbage("collect")
    collectgarbage("collect")
    local weak_roots = 0
    for _ in pairs(weak) do
        weak_roots += 1
    end
    return weak_roots
end

return {
    wave = make_root,
    collect = collect,
    validate = validate,
    probe = probe,
    oom_probe = oom_probe,
    clear = clear,
}
"#;

fn pressure_module_source() -> String {
    PRESSURE_MODULE.replace(
        "__OVERSIZED_BUFFER_BYTES__",
        &OVERSIZED_BUFFER_BYTES.to_string(),
    )
}

fn pressure_vm() -> Rustia {
    let settings = VmSettings {
        deterministic_gc_interval: NonZeroU64::new(128).expect("non-zero collection interval"),
        memory_limit_bytes: Some(MEMORY_LIMIT_BYTES),
        ..VmSettings::default()
    };
    RustiaBuilder::standard()
        .settings(settings)
        .build()
        .expect("mixed-heap pressure VM")
}

fn assert_within_limit(stats: VmMemoryStats, phase: &str) {
    assert!(
        stats.live_bytes <= MEMORY_LIMIT_BYTES,
        "{phase} live memory {} exceeded the configured {}-byte limit",
        stats.live_bytes,
        MEMORY_LIMIT_BYTES
    );
    assert!(
        stats.reserved_bytes >= stats.live_bytes,
        "{phase} reserved memory {} was below live memory {}",
        stats.reserved_bytes,
        stats.live_bytes
    );
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn wasm_linear_memory_pages() -> usize {
    core::arch::wasm32::memory_size::<0>()
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn mixed_heap_pressure_plateaus_and_recovers_atomically() {
    let module =
        Rustia::compile("mixed-heap-pressure.luau", &pressure_module_source()).expect("module");
    let mut vm = pressure_vm();
    assert_eq!(
        vm.vm_settings().memory_limit_bytes,
        Some(MEMORY_LIMIT_BYTES)
    );
    assert_eq!(
        vm.execution_profile(),
        RustiaBuilder::standard().execution_profile()
    );

    let mut reset_plateau = None;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    let mut wasm_page_plateau = None;

    for decision in 0..DECISION_COUNT {
        let exports = vm.install(module.clone()).expect("pressure module install");
        let wave = exports.function(&mut vm, "wave").expect("wave function");
        let collect = exports
            .function(&mut vm, "collect")
            .expect("collect function");
        let validate = exports
            .function(&mut vm, "validate")
            .expect("validate function");
        let probe = exports.function(&mut vm, "probe").expect("probe function");
        let oom_probe = exports
            .function(&mut vm, "oom_probe")
            .expect("OOM probe function");
        let clear = exports.function(&mut vm, "clear").expect("clear function");
        let epoch = vm.decision_epoch();
        let mut retained = VecDeque::<Value>::with_capacity(RETAINED_ROOT_COUNT + 1);
        let mut phase_plateaus: [Option<VmMemoryStats>; 4] = [None; 4];

        for wave_index in 0..WAVES_PER_DECISION {
            let mut values = wave.call_values(&mut vm, &[]).expect("mixed pressure wave");
            assert_eq!(values.len(), 1);
            let value = values.pop().expect("one wave result");
            assert_eq!(
                value.kind(&vm).expect("current root kind"),
                ValueType::Table
            );
            retained.push_back(value);
            if retained.len() > RETAINED_ROOT_COUNT {
                retained.pop_front();
            }
            if (wave_index + 1) % 4 == 0 {
                assert_eq!(
                    collect
                        .call::<_, usize>(&mut vm, ())
                        .expect("quiescent mixed-heap collection"),
                    RETAINED_ROOT_COUNT,
                    "weak roots did not remain bounded in decision {decision}, wave {wave_index}"
                );
            }

            let stats = vm.memory_stats();
            assert_within_limit(stats, "pressure wave");
            if wave_index >= PHASE_WARMUP_WAVES {
                let phase = wave_index % phase_plateaus.len();
                match phase_plateaus[phase] {
                    Some(expected) => {
                        assert_eq!(
                            stats.reserved_bytes, expected.reserved_bytes,
                            "mixed-heap reserved capacity changed after the phase-{phase} plateau \
                             in decision {decision}, wave {wave_index}"
                        );
                        assert!(
                            stats.live_bytes.abs_diff(expected.live_bytes) <= 64,
                            "mixed-heap live accounting left its 64-byte phase-{phase} band \
                             in decision {decision}, wave {wave_index}: {stats:?} vs {expected:?}"
                        );
                    }
                    None => phase_plateaus[phase] = Some(stats),
                }
            }
        }

        let (valid, weak_roots): (bool, usize) =
            validate.call(&mut vm, ()).expect("mixed graph validation");
        assert!(valid);
        assert_eq!(weak_roots, RETAINED_ROOT_COUNT);
        assert!(
            oom_probe
                .call::<_, bool>(&mut vm, ())
                .expect("catchable OOM probe"),
            "oversized allocation was not atomic and recoverable"
        );

        let reload_source = if decision % 2 == 0 {
            "return { version = 1, decide = function(value) return value + 11 end }"
        } else {
            "return { version = 2, decide = function(value) return value + 19 end }"
        };
        let reloaded = vm
            .hot_reload("mixed-reload.luau", reload_source)
            .expect("hot reload");
        let decide = reloaded
            .function(&mut vm, "decide")
            .expect("reloaded function");
        let expected_bias = if decision % 2 == 0 { 11 } else { 19 };
        assert_eq!(
            decide
                .call::<_, usize>(&mut vm, (decision,))
                .expect("current reloaded function"),
            decision + expected_bias
        );
        assert_eq!(
            probe
                .call::<_, usize>(&mut vm, ())
                .expect("retained pre-reload function"),
            WAVES_PER_DECISION
        );

        retained.clear();
        assert_eq!(
            clear
                .call::<_, usize>(&mut vm, ())
                .expect("clear mixed roots"),
            0,
            "weak entries remained after every strong root was removed"
        );
        let stale_after_reset = exports.as_value();
        vm.reset_decision().expect("decision reset");
        assert_eq!(vm.decision_epoch(), epoch + 1);
        assert!(
            stale_after_reset.kind(&vm).is_err(),
            "pre-reset public root remained usable"
        );

        let stats = vm.memory_stats();
        assert_within_limit(stats, "post-reset");
        if decision == DECISION_WARMUP_COUNT {
            reset_plateau = Some(stats);
        } else if decision > DECISION_WARMUP_COUNT {
            assert_eq!(
                Some(stats),
                reset_plateau,
                "post-reset VM memory changed after decision {decision}"
            );
        }

        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            let pages = wasm_linear_memory_pages();
            if decision == DECISION_COUNT / 2 {
                wasm_page_plateau = Some(pages);
            } else if decision > DECISION_COUNT / 2 {
                assert_eq!(
                    Some(pages),
                    wasm_page_plateau,
                    "WebAssembly linear memory grew after the warm plateau at decision {decision}"
                );
            }
        }
    }

    assert_eq!(vm.decision_epoch(), DECISION_COUNT as u64);
    let final_stats = vm.memory_stats();
    assert_eq!(Some(final_stats), reset_plateau);
}
