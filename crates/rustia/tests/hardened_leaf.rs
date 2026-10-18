use rustia::{
    HostContext, HostModule, HostSchema, InvocationLimits, Profile, Result, Rustia, RustiaBuilder,
    VmErrorKind, RUSTIA_HARDENED_LEAF_GUEST_ABI_VERSION,
};
mod support {
    pub mod hardened_transactions;
}
use support::hardened_transactions as tx;

const MEMORY_LIMIT: usize = 4 * 1024 * 1024;

struct LeafSchema;

impl HostSchema for LeafSchema {
    type Input = ();
    type Slots = ();
    type External = ();
    type Effect = ();
    type SlotWrite = ();
    type Record = ();
}

impl HostContext<LeafSchema> for () {
    fn input(&self) -> &() {
        self
    }

    fn slots(&self) -> &() {
        self
    }

    fn external_mut(&mut self) -> &mut () {
        self
    }

    fn record_effect(&self, _sequence: u32, _effect: ()) {}

    fn record_slot_write(&self, _sequence: u32, _write: ()) {}

    fn record_bytes(&self, _record: &()) -> u64 {
        0
    }
}

fn leaf_vm() -> Rustia<LeafSchema> {
    let mut vm = RustiaBuilder::deterministic_leaf()
        .memory_limit_bytes(MEMORY_LIMIT)
        .build_for_host::<LeafSchema>()
        .expect("hardened leaf VM");
    vm.install_host_module(HostModule::<LeafSchema>::new("leaf").pure("answer", 1, || 42i32))
        .expect("host module");
    vm
}

fn tx_vm(memory_limit_bytes: usize) -> Rustia<tx::TxSchema> {
    tx::configure_vm(
        RustiaBuilder::deterministic_leaf()
            .memory_limit_bytes(memory_limit_bytes)
            .build_for_host::<tx::TxSchema>()
            .expect("transaction test VM"),
    )
}

fn invoke_string(vm: &mut Rustia<LeafSchema>, module: &rustia::CompiledModule) -> Result<String> {
    let mut context = ();
    vm.invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
        let exports = invocation.install(module)?;
        let decide = exports.function_in(invocation, "decide")?;
        decide.call_in::<_, _, String>(invocation, ())
    })
    .map(|outcome| outcome.into_parts().0)
}

#[test]
fn hardened_leaf_builder_requires_deterministic_profile_and_memory_limit() {
    let missing_limit = RustiaBuilder::deterministic_leaf()
        .build_for_host::<LeafSchema>()
        .err()
        .expect("memory limit is mandatory");
    assert!(matches!(
        missing_limit.kind(),
        rustia::ErrorKind::Vm(error)
            if matches!(
                error.kind(),
                VmErrorKind::HardenedLeafViolation {
                    feature: "missing logical-live memory limit"
                }
            )
    ));

    let wrong_profile = RustiaBuilder::deterministic_leaf()
        .profile(Profile::ReproducibleStandard)
        .memory_limit_bytes(MEMORY_LIMIT)
        .build_for_host::<LeafSchema>()
        .err()
        .expect("hardened leaves require DeterministicSim");
    assert!(matches!(
        wrong_profile.kind(),
        rustia::ErrorKind::Vm(error)
            if matches!(
                error.kind(),
                VmErrorKind::HardenedLeafViolation {
                    feature: "non-DeterministicSim profile"
                }
            )
    ));

    let context_free = RustiaBuilder::deterministic_leaf()
        .memory_limit_bytes(MEMORY_LIMIT)
        .build()
        .err()
        .expect("hardened leaves require a typed host facade");
    assert!(matches!(
        context_free.kind(),
        rustia::ErrorKind::Vm(error)
            if matches!(
                error.kind(),
                VmErrorKind::HardenedLeafViolation {
                    feature: "context-free facade"
                }
            )
    ));

    assert_eq!(RUSTIA_HARDENED_LEAF_GUEST_ABI_VERSION, 1);
}

#[test]
fn failed_first_install_rolls_back_the_hardened_seal_checkpoint() {
    let mut vm = leaf_vm();
    let bad = Rustia::compile_module("bad-first-install.luau", "error('boom')")
        .expect("failing module compiles");
    let mut context = ();
    let error = vm
        .invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
            invocation.install(&bad)
        })
        .expect_err("first installation fails");
    assert!(error.to_string().contains("boom"), "{error}");

    vm.reset_decision()
        .expect("rollback restores a valid pre-seal checkpoint");
    let good = Rustia::compile_module(
        "good-after-failure.luau",
        "return { decide = function() return 'recovered' end }",
    )
    .expect("recovery module compiles");
    assert_eq!(invoke_string(&mut vm, &good).unwrap(), "recovered");
}

#[test]
fn hardened_leaf_surface_is_allowlisted_frozen_and_reset_stable() {
    let mut vm = leaf_vm();
    let module = Rustia::compile_module(
        "surface.luau",
        r#"
return {
    decide = function()
        local forbidden = {
            debug,
            getfenv,
            setfenv,
            setmetatable,
            getmetatable,
            newproxy,
            rawequal,
            rawget,
            rawset,
            rawlen,
            loadstring,
            collectgarbage,
            gcinfo,
            print,
            os,
            buffer,
            vector,
            coroutine,
            math.random,
            math.randomseed,
            _G,
            RTTI,
        }
        for _, value in forbidden do
            if value ~= nil then
                return "forbidden-present"
            end
        end
        if not table.isfrozen(table)
            or not table.isfrozen(string)
            or not table.isfrozen(math)
            or not table.isfrozen(utf8)
            or not table.isfrozen(bit32)
        then
            return "mutable-library"
        end
        local scratch = { -4, 7 }
        table.insert(scratch, leaf.answer())
        return tostring(math.abs(scratch[1]) + scratch[2] + scratch[3])
    end,
}
"#,
    )
    .expect("surface module");

    assert!(matches!(
        (vm.insert_global("decision_input", 7)).as_ref().err().map(|error| error.kind()),
        Some(rustia::ErrorKind::Vm(error))
            if matches!(
                error.kind(),
                VmErrorKind::HardenedLeafViolation {
                    feature: "ordinary global inputs"
                }
            )
    ));
    assert_eq!(
        invoke_string(&mut vm, &module).expect("first decision"),
        "53"
    );
    vm.reset_decision().expect("decision reset");
    assert_eq!(
        invoke_string(&mut vm, &module).expect("second decision"),
        "53"
    );
    assert!(matches!(
        (vm.install_host_module(HostModule::<LeafSchema>::new("late").pure("value", 1, || 1i32))).as_ref().err().map(|error| error.kind()),
        Some(rustia::ErrorKind::Vm(error)) if matches!(error.kind(), VmErrorKind::GuestEnvironmentSealed)
    ));

    for (source_name, source) in [
        ("mutate-table.luau", "table.insert = nil; return {}"),
        ("mutate-string.luau", "string.byte = nil; return {}"),
        ("mutate-math.luau", "math.abs = nil; return {}"),
        ("mutate-utf8.luau", "utf8.len = nil; return {}"),
        ("mutate-bit32.luau", "bit32.band = nil; return {}"),
    ] {
        let mut vm = leaf_vm();
        let module = Rustia::compile_module(source_name, source).expect("mutation module");
        let mut context = ();
        let error = vm
            .invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
                invocation.install(&module)
            })
            .expect_err("standard-library tables must be frozen");
        assert!(
            error.to_string().contains("readonly table"),
            "{source_name}: {error}"
        );
    }
}

#[test]
fn hardened_leaf_stateless_entry_runs_without_retained_exports() {
    let mut vm = leaf_vm();
    let entry = Rustia::compile_entry(
        "entry.luau",
        r#"
local scratch = { 1, 2, 3 }
return leaf.answer() + scratch[1] + scratch[2] + scratch[3]
"#,
    )
    .expect("stateless entry");

    let mut lane = Vec::new();
    for expected_epoch in 1..=3 {
        let mut context = ();
        let value = vm
            .invoke_entry_into::<i32>(
                &mut context,
                InvocationLimits::new(20_000),
                &mut lane,
                &entry,
            )
            .expect("entry invocation")
            .into_parts()
            .0;
        assert_eq!(value, 48);
        assert_eq!(vm.decision_epoch(), expected_epoch);
    }

    let nested = Rustia::compile_entry(
        "nested-entry.luau",
        "local one = 1\nlocal two = 2\nlocal three = 3\nreturn (function() return one + two + three end)()",
    )
    .err()
    .expect("nested functions are not stateless entries");
    assert!(nested
        .to_string()
        .contains("stateless entry contains nested functions or captured upvalues"));
    assert!(
        nested.to_string().starts_with("nested-entry.luau:4:"),
        "{nested}"
    );

    let located = Rustia::compile_entry(
        "located-policy-error.luau",
        "local ok = 1\nforbidden_global = 2\nreturn ok",
    )
    .err()
    .expect("global mutation must fail entry compilation");
    assert!(
        located
            .to_string()
            .starts_with("located-policy-error.luau:2:"),
        "{located}"
    );

    let mut standard = RustiaBuilder::deterministic()
        .build_for_host::<LeafSchema>()
        .expect("standard typed VM");
    let standard_entry = Rustia::compile_entry("standard-entry.luau", "return 1").expect("entry");
    let mut context = ();
    let mut lane = Vec::new();
    let error = standard
        .invoke_entry_into::<i32>(
            &mut context,
            InvocationLimits::new(100),
            &mut lane,
            &standard_entry,
        )
        .expect_err("hardened entries must reject an ordinary surface");
    assert!(matches!(
        error.kind(),
        rustia::ErrorKind::Vm(error)
            if matches!(
                error.kind(),
                VmErrorKind::HardenedLeafViolation {
                    feature: "non-hardened guest surface"
                }
            )
    ));
    assert_eq!(standard.decision_epoch(), 1);
}

#[test]
fn hardened_leaf_failures_do_not_retain_rich_diagnostics() {
    let mut vm = leaf_vm();
    let entry = Rustia::compile_entry("leaf-failure.luau", "error('stop')").unwrap();
    let mut lane = Vec::new();
    let error = vm
        .invoke_entry_into::<i32>(&mut (), InvocationLimits::new(20_000), &mut lane, &entry)
        .err()
        .unwrap();
    assert!(error.diagnostic().is_none());
    assert_eq!(vm.diagnostic_source_bytes(), 0);
    assert!(!error.report().to_string().contains("called from:"));
}

#[test]
fn compiled_entry_is_shareable_across_hardened_vms_and_module_handles_go_stale() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<rustia::CompiledEntry>();

    let entry = Rustia::compile_entry(
        "shared-entry.luau",
        "local scratch = { 1, 2, 3 }\nreturn leaf.answer() + #scratch",
    )
    .expect("shared entry");
    let mut first = leaf_vm();
    let mut second = leaf_vm();
    for (vm, shared) in [(&mut first, entry.clone()), (&mut second, entry)] {
        let mut context = ();
        let mut lane = Vec::new();
        let receipt = vm
            .invoke_entry_into::<i32>(
                &mut context,
                InvocationLimits::new(20_000),
                &mut lane,
                &shared,
            )
            .expect("shared entry invocation");
        assert_eq!(*receipt.value(), 45);
        assert_eq!(vm.decision_epoch(), 1);
    }

    let mut vm = leaf_vm();
    let module = Rustia::compile_module(
        "hardened-compatibility-module.luau",
        "return { decide = function() return leaf.answer() end }",
    )
    .expect("compatibility module");
    let mut context = ();
    let exports = vm
        .invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
            invocation.install(&module)
        })
        .expect("module install")
        .into_parts()
        .0;
    let function = vm
        .invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
            exports.function_in(invocation, "decide")
        })
        .expect("function lookup")
        .into_parts()
        .0;
    vm.reset_decision().expect("hardened reset");
    let error = vm
        .invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
            function.call_in::<_, _, i32>(invocation, ())
        })
        .expect_err("stale hardened function must reject");
    assert!(matches!(
        error.as_vm_error().map(|error| error.kind()),
        Some(VmErrorKind::StaleRetainedValue { .. })
    ));
}

#[test]
fn hardened_leaf_rejects_global_upvalue_and_host_module_mutation() {
    let cases = [
        (
            "global mutation",
            "global-write.luau",
            "forbidden_global = 1; return {}",
        ),
        (
            "captured upvalues",
            "upvalue.luau",
            "local hidden = 1; return { decide = function() return hidden end }",
        ),
    ];

    for (feature, name, source) in cases {
        let mut vm = leaf_vm();
        let module = Rustia::compile_module(name, source).expect("policy-tested source compiles");
        let mut context = ();
        let error = vm
            .invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
                invocation.install(&module)
            })
            .expect_err("policy violation must reject before guest execution");
        assert!(
            matches!(
            error.kind(),
            rustia::ErrorKind::Vm(error)
                        if matches!(
                            error.kind(),
                            VmErrorKind::HardenedLeafViolation {
                                feature: actual
                            } if *actual == feature
                        )
                ),
            "{name}: {error}"
        );
    }

    let mut vm = leaf_vm();
    let module = Rustia::compile_module(
        "host-module-write.luau",
        "leaf.answer = function() return 0 end; return {}",
    )
    .expect("host-module mutation source compiles");
    let mut context = ();
    let error = vm
        .invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
            invocation.install(&module)
        })
        .expect_err("host module table must be frozen");
    assert!(error.to_string().contains("readonly table"), "{error}");
}

#[test]
fn hardened_entry_lifecycle_is_atomic_and_recovers_from_every_failure_class() {
    tx::hardened_entry_lifecycle_is_atomic_and_recovers_from_every_failure_class(tx_vm(512 * 1024));
}

#[test]
fn mixed_hardened_failures_reset_to_an_exact_memory_plateau() {
    tx::mixed_hardened_failures_reset_to_an_exact_memory_plateau(tx_vm(MEMORY_LIMIT));
}

#[test]
fn hardened_checkpoint_reset_reaches_an_exact_heterogeneous_memory_plateau() {
    tx::hardened_checkpoint_reset_reaches_an_exact_heterogeneous_memory_plateau(tx_vm(
        MEMORY_LIMIT,
    ));
}

#[test]
fn ordinary_deterministic_surface_remains_compatible() {
    let mut vm = RustiaBuilder::deterministic()
        .build()
        .expect("ordinary deterministic VM");
    let result: String = vm
        .eval(
            r#"
return table.concat({
    type(debug),
    type(getfenv),
    type(rawset),
    type(buffer),
    type(vector),
    type(coroutine),
    type(math.random),
    type(_G),
}, ",")
"#,
        )
        .expect("ordinary deterministic surface");
    assert_eq!(
        result,
        "table,function,function,table,table,table,function,table"
    );
}
