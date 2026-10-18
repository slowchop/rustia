use rustia::{
    Effects, Error, FromRustia, HostContext, HostSchema, Input, IntoRustiaMulti, InvocationLimits,
    NewObject, Object, ObjectType, OwnedValue, Result, Rustia, RustiaBuilder, SlotWrites, Slots,
};
use std::cell::Cell;
use std::rc::Rc;

struct Player {
    score: i64,
}

fn player_type() -> ObjectType<Player> {
    ObjectType::new("Player").method("score", 1, |p: &Player| p.score)
}

#[test]
fn expanded_call_continuations_root_typed_objects() -> Result<()> {
    let mut lua = RustiaBuilder::standard()
        .memory_limit_bytes(2 * 1024 * 1024)
        .fuel(100_000)
        .build()?;
    lua.register_object_type(player_type())?;
    lua.register_function("make_player", || NewObject::new(Player { score: 21 }))?;
    let source = r#"
        local function pass(...) return ... end
        local function pressure()
            for i=1,48 do local garbage=table.create(16000,i) end
        end
        local function argument(...)
            coroutine.yield('argument')
            string.len=function(object,x,y)
                coroutine.yield('callee')
                return object:score()+x+y
            end
            return ...
        end
        local co=coroutine.create(function()
            return string.len(make_player(),pass(pass(pass(pass(argument(4,5))))))
        end)
        local a,b=coroutine.resume(co)
        pressure()
        local c,d=coroutine.resume(co)
        pressure()
        local e,f=coroutine.resume(co)
        return tostring(a)..':'..b..':'..tostring(c)..':'..d..':'..tostring(e)..':'..f
    "#;
    assert_eq!(
        lua.eval::<String>(source)?,
        "true:argument:true:callee:true:30"
    );
    lua.reset_decision()?;
    assert_eq!(
        lua.eval::<String>(source)?,
        "true:argument:true:callee:true:30"
    );
    Ok(())
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn method_metadata_is_available_before_registration() {
    let definition =
        player_type().method_mut("set", 5, |p: &mut Player, score: i64| -> Result<()> {
            p.score = score;
            Ok(())
        });
    assert_eq!(definition.name(), "Player");
    assert_eq!(definition.fuel_policy(), rustia::RUSTIA_HOST_FUEL_POLICY);
    let methods: Vec<_> = definition.methods().collect();
    assert_eq!(methods.len(), 2);
    assert_eq!(methods[0].name, "score");
    assert_eq!(
        methods[0].fuel_abi_version,
        rustia::RUSTIA_HOST_FUEL_ABI_VERSION
    );
    assert_eq!(methods[0].capability, rustia::OpCapability::Pure);
    assert_eq!(methods[0].parameter_count, 1);
    assert_eq!(
        methods[0].parameter_types,
        &[std::any::type_name::<Object<Player>>()]
    );
    assert_eq!(methods[0].return_types, &[std::any::type_name::<i64>()]);
    assert!(!methods[0].fallible);
    assert_eq!(methods[1].name, "set");
    assert_eq!(methods[1].fuel_cost, 5);
    assert_eq!(
        methods[1].capability,
        rustia::OpCapability::ReadsExternalMutableState
    );
    assert_eq!(methods[1].parameter_count, 2);
    assert!(methods[1].return_types.is_empty());
    assert!(methods[1].fallible);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn typed_objects_preserve_identity_and_old_methods_across_replacement() -> Result<()> {
    let mut lua = Rustia::builder().build()?;
    lua.register_object_type(player_type())?;
    let old = lua.create_object(Player { score: 21 })?;
    lua.globals()?.set_live(&mut lua, "old", &old.as_value())?;
    lua.exec("alias = old; detached = old.score")?;
    lua.register_object_type(
        ObjectType::new("Player").method("score", 1, |p: &Player| p.score * 2),
    )?;
    let new = lua.create_object(Player { score: 21 })?;
    lua.globals()?.set_live(&mut lua, "new", &new.as_value())?;
    assert!(lua.eval::<bool>("return old == alias and old ~= new and old:score() == 21 and new:score() == 42 and detached(new) == 21")?);
    assert_eq!(old.call_method::<_, i64>(&mut lua, "score", ())?, 21);
    assert_eq!(new.call_method::<_, i64>(&mut lua, "score", ())?, 42);
    assert!(old.as_value().as_object::<Player>(&mut lua)?.is_some());
    assert!(old.as_value().as_object::<String>(&mut lua)?.is_none());
    assert!(lua
        .load("return newproxy()")
        .eval_live()?
        .as_object::<Player>(&mut lua)?
        .is_none());
    lua.reset_decision()?;
    assert!(old.call_method::<_, i64>(&mut lua, "score", ()).is_err());
    let after_reset = lua.create_object(Player { score: 5 })?;
    assert_eq!(
        after_reset.call_method::<_, i64>(&mut lua, "score", ())?,
        10
    );
    Ok(())
}

struct ConversionProbe;
impl FromRustia for ConversionProbe {
    fn from_rustia(_: OwnedValue) -> Result<Self> {
        Err(Error::embed("argument conversion reached"))
    }
}

struct HostConversionProbe;
impl IntoRustiaMulti for HostConversionProbe {
    fn into_rustia_multi(self) -> Result<Vec<OwnedValue>> {
        Err(Error::embed("host conversion reached"))
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn fresh_results_are_validated_before_owned_result_allocation() -> Result<()> {
    struct Unregistered;
    let mut lua = RustiaBuilder::standard()
        .memory_limit_bytes(2 * 1024 * 1024)
        .build()?;
    lua.register_function("bad_result", || {
        (vec![1_i64; 131_072], NewObject::new(Unregistered))
    })?;
    let message: String =
        lua.eval("local ok, err = pcall(bad_result); assert(not ok); return tostring(err)")?;
    assert!(message.contains("not registered"), "{message}");
    assert_eq!(lua.eval::<i64>("return 42")?, 42);
    let mut leaf = RustiaBuilder::deterministic_leaf()
        .memory_limit_bytes(2 * 1024 * 1024)
        .build_for_host::<Game>()?;
    let before = leaf.memory_stats();
    assert!(leaf.create_object(Player { score: 1 }).is_err());
    assert_eq!(leaf.memory_stats().live_bytes(), before.live_bytes());
    Ok(())
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
#[test]
fn a_panicking_method_releases_the_borrow_and_reset_drops_payload_once() -> Result<()> {
    struct Dropped(Rc<Cell<usize>>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let drops = Rc::new(Cell::new(0));
    let mut lua = Rustia::builder().build()?;
    lua.register_object_type(ObjectType::new("Dropped").method_mut(
        "panic",
        1,
        |_: &mut Dropped| -> () {
            panic!("test host panic");
        },
    ))?;
    let object = lua.create_object(Dropped(drops.clone()))?;
    lua.globals()?
        .set_live(&mut lua, "object", &object.as_value())?;
    assert!(lua.exec("pcall(function() object:panic() end)").is_err());
    assert!(
        lua.exec("return 1").is_err(),
        "host panic must poison further guest execution"
    );
    lua.reset_decision()?;
    assert_eq!(
        drops.get(),
        1,
        "the stale Rust handle must not retain the payload"
    );
    drop(object);
    assert_eq!(drops.get(), 1);
    assert_eq!(lua.eval::<i64>("return 42")?, 42);
    Ok(())
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn receivers_are_checked_before_other_argument_conversions() -> Result<()> {
    let mut first = Rustia::builder().build()?;
    first.register_object_type(player_type().method(
        "probe",
        1,
        |_: &Player, _: ConversionProbe| (),
    ))?;
    let player = first.create_object(Player { score: 7 })?;
    first
        .globals()?
        .set_live(&mut first, "player", &player.as_value())?;
    let message: String = first
        .eval("local ok, err = pcall(player.probe, {}, 1); assert(not ok); return tostring(err)")?;
    assert!(
        !message.contains("argument conversion reached"),
        "{message}"
    );
    let error = player
        .call_method::<_, ()>(
            &mut Rustia::builder().build()?,
            "probe",
            HostConversionProbe,
        )
        .unwrap_err();
    assert!(!error.to_string().contains("host conversion reached"));
    first.reset_decision()?;
    let error = player
        .call_method::<_, ()>(&mut first, "probe", HostConversionProbe)
        .unwrap_err();
    assert!(!error.to_string().contains("host conversion reached"));
    Ok(())
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn callback_arguments_and_mixed_fresh_results_keep_objects_live() -> Result<()> {
    let mut lua = Rustia::builder().build()?;
    lua.register_object_type(
        player_type()
            .method("copy", 1, |p: &Player| {
                NewObject::new(Player { score: p.score })
            })
            .method("same", 1, |_: &Player, other: Object<Player>| other),
    )?;
    lua.register_function("spawn", |score: i64| {
        ("ready", NewObject::new(Player { score }), 9_i64)
    })?;
    lua.register_function("identity", |object: Object<Player>| object)?;
    assert!(lua.eval::<bool>("local tag, p, n = spawn(42); local copy = p:copy(); return tag == 'ready' and n == 9 and copy ~= p and copy:score() == 42 and identity(p) == p and copy:same(p) == p")?);
    let object = lua
        .load("local _, p = spawn(12); return p")
        .eval_live()?
        .as_object::<Player>(&mut lua)?
        .unwrap();
    let copies = object.call_method_values(&mut lua, "copy", &[])?;
    let copy = copies[0].as_object::<Player>(&mut lua)?.unwrap();
    assert_eq!(copy.call_method::<_, i64>(&mut lua, "score", ())?, 12);
    assert!(
        object.call_method::<_, ()>(&mut lua, "copy", ()).is_err(),
        "owned returns must reject objects even when discarded"
    );
    Ok(())
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn owned_arguments_keep_nested_tables_alive_during_later_allocations() -> Result<()> {
    let mut lua = RustiaBuilder::standard()
        .memory_limit_bytes(2 * 1024 * 1024)
        .build()?;
    lua.register_object_type(player_type().method(
        "check",
        1,
        |_: &Player, first: Vec<Vec<i64>>, second: Vec<i64>| {
            assert_eq!(first.len(), 64);
            assert!(first
                .iter()
                .all(|row| row.len() == 512 && row.iter().all(|n| *n == 7)));
            assert_eq!(second.len(), 32_768);
            assert!(second.iter().all(|n| *n == 9));
            42_i64
        },
    ))?;
    let object = lua.create_object(Player { score: 0 })?;
    let garbage = lua.load("return table.create(70000, 1)").eval_live()?;
    drop(garbage);
    assert_eq!(
        object.call_method::<_, i64>(
            &mut lua,
            "check",
            (vec![vec![7_i64; 512]; 64], vec![9_i64; 32_768])
        )?,
        42
    );
    let exports = lua
        .load("return { check = function(a, b) return a[64][512] + b[32768] end }")
        .install()?;
    let function = exports.function(&mut lua, "check")?;
    let garbage = lua.load("return table.create(70000, 1)").eval_live()?;
    drop(garbage);
    assert_eq!(
        function.call::<_, i64>(&mut lua, (vec![vec![7_i64; 512]; 64], vec![9_i64; 32_768]))?,
        16
    );
    Ok(())
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn forbidden_definitions_are_atomic_and_payloads_need_no_clone_or_send() -> Result<()> {
    struct Local(Rc<Cell<i64>>);
    let mut lua = RustiaBuilder::deterministic().build()?;
    lua.register_object_type(ObjectType::new("Local").method("value", 1, |v: &Local| v.0.get()))?;
    assert!(lua
        .register_object_type(
            ObjectType::new("Local")
                .method("value", 1, |_: &Local| 999_i64)
                .method_mut("change", 1, |v: &mut Local| v.0.set(10))
        )
        .is_err());
    let value = Rc::new(Cell::new(7));
    let object = lua.create_object(Local(value.clone()))?;
    assert_eq!(object.call_method::<_, i64>(&mut lua, "value", ())?, 7);
    assert_eq!(Rc::strong_count(&value), 2);
    let clone = object.clone();
    lua.reset_decision()?;
    assert_eq!(
        Rc::strong_count(&value),
        1,
        "stale root clones cannot retain payload ownership"
    );
    assert!(clone.call_method::<_, i64>(&mut lua, "value", ()).is_err());
    Ok(())
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn immediate_mutation_survives_errors_and_failed_installation() -> Result<()> {
    for builder in [
        RustiaBuilder::standard(),
        #[cfg(any(
            not(all(target_arch = "wasm32", target_os = "unknown")),
            feature = "web-live"
        ))]
        RustiaBuilder::live(),
    ] {
        let mut lua = builder.build()?;
        lua.register_object_type(
            player_type().method_mut("add", 1, |p: &mut Player, n: i64| {
                p.score += n;
                p.score
            }),
        )?;
        let player = lua.create_object(Player { score: 1 })?;
        lua.globals()?
            .set_live(&mut lua, "player", &player.as_value())?;
        assert!(lua.exec("player:add(2); error('failed')").is_err());
        assert_eq!(player.call_method::<_, i64>(&mut lua, "score", ())?, 3);
        assert!(lua
            .load("player:add(4); error('candidate failed')")
            .install()
            .is_err());
        assert_eq!(player.call_method::<_, i64>(&mut lua, "score", ())?, 7);
        assert!(lua.eval::<bool>("local ok = pcall(function() player:add(8); error('caught') end); return not ok and player:score() == 15")?);
    }
    Ok(())
}

struct Game;
impl HostSchema for Game {
    type Input = i64;
    type Slots = i64;
    type External = i64;
    type Effect = i64;
    type SlotWrite = i64;
    type Record = (u32, &'static str, i64);
}
struct Turn {
    input: i64,
    slots: i64,
    external: i64,
}
impl HostContext<Game> for Turn {
    fn input(&self) -> &i64 {
        &self.input
    }
    fn slots(&self) -> &i64 {
        &self.slots
    }
    fn external_mut(&mut self) -> &mut i64 {
        &mut self.external
    }
    fn record_effect(&self, sequence: u32, value: i64) -> (u32, &'static str, i64) {
        (sequence, "effect", value)
    }
    fn record_slot_write(&self, sequence: u32, value: i64) -> (u32, &'static str, i64) {
        (sequence, "write", value)
    }
    fn record_bytes(&self, _: &(u32, &'static str, i64)) -> u64 {
        24
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn deterministic_methods_use_the_existing_ordered_journal_and_fresh_context() -> Result<()> {
    let mut lua = RustiaBuilder::deterministic().build_for_host::<Game>()?;
    lua.register_object_type(
        ObjectType::<Player, Game>::new("Player")
            .input("input", 2, |p: &Player, input: Input<'_, Game>| {
                p.score + input.get()
            })
            .slot_read("saved", 2, |p: &Player, slots: Slots<'_, Game>| {
                p.score + slots.get()
            })
            .effect(
                "request",
                2,
                |p: &Player, mut effects: Effects<'_, Game>, n: i64| effects.emit(p.score + n),
            )
            .effect(
                "failed_request",
                2,
                |_: &Player, mut effects: Effects<'_, Game>| -> Result<()> {
                    effects.emit(999)?;
                    Err(Error::embed("request rejected"))
                },
            )
            .slot_write(
                "save",
                2,
                |p: &Player, mut writes: SlotWrites<'_, Game>, n: i64| writes.write(p.score + n),
            ),
    )?;
    let player = lua.create_object(Player { score: 10 })?;
    lua.globals()?
        .set_live(&mut lua, "player", &player.as_value())?;
    for input in [1, 2] {
        let mut turn = Turn {
            input,
            slots: 3,
            external: 0,
        };
        let limits = InvocationLimits::new(1000)
            .with_max_records(4)
            .with_max_record_bytes(96);
        let outcome = lua.invoke(&mut turn, limits, |call| {
            assert_eq!(player.call_method_in::<_, _, i64>(call, "input", ())?, 10 + input);
            call.load("player:request(4); assert(not pcall(function() player:failed_request() end)); player:save(5); return player:saved()").eval::<i64>()
        })?;
        assert_eq!(*outcome.value(), 13);
        assert_eq!(outcome.records(), &[(0, "effect", 14), (1, "write", 15)]);
        assert_eq!(
            turn.slots, 3,
            "staged changes require the application's explicit commit"
        );
        assert!(lua
            .invoke(&mut turn, limits, |call| {
                call.load("player:request(8); error('cancel')").exec()
            })
            .is_err());
        assert_eq!(turn.slots, 3);
    }
    Ok(())
}
