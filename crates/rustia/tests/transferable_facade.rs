#![cfg(feature = "transferable")]

use rustia::{
    Exports, Function, HostSchema, Object, ObjectType, Rustia, RustiaBuilder, ServiceEntropy,
    Table, Value,
};

struct TransferHost;

impl HostSchema for TransferHost {
    type Input = ();
    type Slots = ();
    type External = ();
    type Effect = ();
    type SlotWrite = ();
    type Record = String;
}

struct Counter(i64);

fn assert_send<T: Send>() {}

#[test]
fn interpreter_and_live_handles_move_between_workers() {
    assert_send::<Rustia>();
    assert_send::<Value>();
    assert_send::<Table>();
    assert_send::<Function>();
    assert_send::<Exports>();
    assert_send::<Rustia<TransferHost>>();
    assert_send::<Object<Counter>>();

    let mut lua = Rustia::builder().build().unwrap();
    let exports = lua
        .load(
            "local count = 0\n\
             return {\n\
                 label = 9,\n\
                 next = function() count += 1; return count end,\n\
                 state = { owner = 'main' },\n\
             }",
        )
        .source_name("transfer.luau")
        .install()
        .unwrap();
    let next = exports.function(&mut lua, "next").unwrap();
    let state = exports
        .get_live(&mut lua, "state")
        .unwrap()
        .as_table(&lua)
        .unwrap()
        .unwrap();

    let (mut lua, exports, next, state) = std::thread::spawn(move || {
        assert_eq!(next.call::<_, i64>(&mut lua, ()).unwrap(), 1);
        state.set(&mut lua, "owner", "worker").unwrap();
        (lua, exports, next, state)
    })
    .join()
    .unwrap();

    assert_eq!(next.call::<_, i64>(&mut lua, ()).unwrap(), 2);
    assert_eq!(exports.get::<i64>(&mut lua, "label").unwrap(), 9);
    assert_eq!(
        state
            .get(&mut lua, "owner")
            .unwrap()
            .to_owned(&mut lua)
            .unwrap(),
        rustia::OwnedValue::String("worker".to_owned())
    );
}

#[test]
fn typed_callbacks_and_object_payloads_remain_transferable() {
    let mut lua = RustiaBuilder::standard().build().unwrap();
    lua.register_function("double", |value: i64| value * 2)
        .unwrap();
    lua.register_object_type(
        ObjectType::new("Counter").method("value", 1, |counter: &Counter| counter.0),
    )
    .unwrap();
    let counter = lua.create_object(Counter(21)).unwrap();

    let (mut lua, counter) = std::thread::spawn(move || {
        assert_eq!(lua.eval::<i64>("return double(21)").unwrap(), 42);
        assert_eq!(
            counter
                .call_method::<_, i64>(&mut lua, "value", ())
                .unwrap(),
            21
        );
        (lua, counter)
    })
    .join()
    .unwrap();

    assert_eq!(
        counter
            .call_method::<_, i64>(&mut lua, "value", ())
            .unwrap(),
        21
    );
}

#[test]
fn transferable_service_captures_can_keep_mutable_state() {
    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = lines.clone();
    let lua = Rustia::builder()
        .stdout(move |line| captured.lock().unwrap().push(line.to_owned()))
        .build()
        .unwrap();

    let mut lua = std::thread::spawn(move || {
        let mut lua = lua;
        lua.exec("print('worker')").unwrap();
        lua
    })
    .join()
    .unwrap();
    lua.exec("print('main')").unwrap();

    assert_eq!(
        *lines.lock().unwrap(),
        ["worker\n".to_owned(), "main\n".to_owned()]
    );
}

#[test]
fn cloned_entropy_services_support_concurrent_vm_construction() {
    use std::sync::{Arc, Barrier};

    let concurrent_entry = Arc::new(Barrier::new(2));
    let entropy = ServiceEntropy::from_fn({
        let concurrent_entry = Arc::clone(&concurrent_entry);
        move || {
            concurrent_entry.wait();
            42
        }
    });

    let workers: Vec<_> = (0..2)
        .map(|_| {
            let entropy = entropy.clone();
            std::thread::spawn(move || RustiaBuilder::standard().entropy(entropy).build().unwrap())
        })
        .collect();

    for worker in workers {
        assert_send::<Rustia>();
        worker.join().expect("VM construction must not panic");
    }
}
