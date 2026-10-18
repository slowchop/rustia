use rustia::{Exports, HostSchema, OwnedValue, Profile, Result as RustiaResult, Rustia, Value};
use serde::de::IgnoredAny;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::convert::Infallible;

#[derive(Debug, Deserialize, PartialEq)]
struct Scenario {
    name: String,
    #[serde(default)]
    enabled: bool,
    enemies: Vec<Enemy>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Enemy {
    kind: String,
    hp: u32,
}

#[derive(Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
enum Directive {
    Hold,
    Count(u32),
    Pair(u32, String),
    Named { value: u32 },
}

fn eval_live(rustia: &mut Rustia, source: &str) -> RustiaResult<Value> {
    rustia.load(source).eval_live()
}

fn install(rustia: &mut Rustia, source: &str) -> RustiaResult<Exports> {
    rustia.load(source).install()
}

#[test]
fn live_value_table_and_exports_match_the_owned_format() {
    let mut rustia = rustia::RustiaBuilder::standard()
        .profile(Profile::ReproducibleStandard)
        .build()
        .unwrap();
    let exports = install(
        &mut rustia,
        r#"
            return {
                name = "arena",
                enemies = {
                    { kind = "scout", hp = 5 },
                    { kind = "brute", hp = 9 },
                },
            }
            "#,
    )
    .unwrap();

    let live: Scenario = exports.deserialize(&mut rustia).unwrap();
    let owned: Scenario = exports.value(&mut rustia).unwrap().deserialize().unwrap();
    assert_eq!(live, owned);

    let value = exports.as_value();
    assert_eq!(value.deserialize::<Scenario>(&mut rustia).unwrap(), owned);
    let table = value.as_table(&rustia).unwrap().unwrap();
    assert_eq!(table.deserialize::<Scenario>(&mut rustia).unwrap(), owned);
}

#[test]
fn live_serde_preserves_enum_and_checked_numeric_semantics() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Payload {
        unit: Directive,
        count: Directive,
        pair: Directive,
        named: Directive,
        numeric: BTreeMap<i32, String>,
        wide: i128,
    }

    let mut rustia = rustia::RustiaBuilder::standard()
        .profile(Profile::ReproducibleStandard)
        .build()
        .unwrap();
    let value = eval_live(
        &mut rustia,
        r#"
            return {
                unit = "hold",
                count = { count = 4 },
                pair = { pair = { 7, "scout" } },
                named = { named = { value = 9 } },
                numeric = { [4] = "four", [2] = "two" },
                wide = 1267650600228229401496703205376,
            }
            "#,
    )
    .unwrap();

    let payload: Payload = value.deserialize(&mut rustia).unwrap();
    let owned_payload: Payload = value.to_owned(&mut rustia).unwrap().deserialize().unwrap();
    assert_eq!(payload, owned_payload);
    assert_eq!(payload.unit, Directive::Hold);
    assert_eq!(payload.count, Directive::Count(4));
    assert_eq!(payload.pair, Directive::Pair(7, "scout".to_owned()));
    assert_eq!(payload.named, Directive::Named { value: 9 });
    assert_eq!(
        payload.numeric,
        BTreeMap::from([(2, "two".to_owned()), (4, "four".to_owned())])
    );
    assert_eq!(payload.wide, 1_i128 << 100);

    for source in ["return 1.5", "return -1", "return 0/0", "return math.huge"] {
        let value = eval_live(&mut rustia, source).unwrap();
        assert!(value.deserialize::<u64>(&mut rustia).is_err(), "{source}");
    }
}

#[test]
fn live_serde_covers_options_newtypes_tuples_chars_bytes_and_map_keys() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Identifier(u32);

    #[derive(Debug, Deserialize, PartialEq)]
    struct Shapes {
        maybe: Option<u32>,
        missing: Option<u32>,
        id: Identifier,
        pair: (u32, String),
        letter: char,
        bytes: Vec<u8>,
        flags: BTreeMap<bool, String>,
        directives: BTreeMap<Directive, u32>,
    }

    let mut rustia = rustia::RustiaBuilder::standard()
        .profile(Profile::ReproducibleStandard)
        .build()
        .unwrap();
    let value = eval_live(
        &mut rustia,
        r#"
            return {
                maybe = 4,
                id = 9,
                pair = { 7, "seven" },
                letter = "x",
                bytes = { 0, 127, 255 },
                flags = { [true] = "yes", [false] = "no" },
                directives = { hold = 3 },
            }
            "#,
    )
    .unwrap();
    let live: Shapes = value.deserialize(&mut rustia).unwrap();
    let owned: Shapes = value.to_owned(&mut rustia).unwrap().deserialize().unwrap();
    assert_eq!(live, owned);
    assert_eq!(live.missing, None);

    assert_eq!(
        eval_live(&mut rustia, "return nil")
            .unwrap()
            .deserialize::<Option<u32>>(&mut rustia)
            .unwrap(),
        None
    );

    for source in [
        "return { count = 4, extra = 5 }",
        "return { [{}] = 1 }",
        "return { [string.char(255)] = 1 }",
    ] {
        let value = eval_live(&mut rustia, source).unwrap();
        if source.contains("count") {
            assert!(value.deserialize::<Directive>(&mut rustia).is_err());
        } else {
            assert!(value
                .deserialize::<BTreeMap<String, u32>>(&mut rustia)
                .is_err());
        }
    }
}

#[test]
fn dense_sequences_are_ordered_and_map_shaped_sequences_are_rejected() {
    let mut rustia = rustia::RustiaBuilder::standard()
        .profile(Profile::ReproducibleStandard)
        .build()
        .unwrap();
    let dense = eval_live(&mut rustia, "return { [3] = 'c', [1] = 'a', [2] = 'b' }").unwrap();
    assert_eq!(
        dense.deserialize::<Vec<String>>(&mut rustia).unwrap(),
        ["a", "b", "c"]
    );
    assert_eq!(
        dense
            .deserialize::<BTreeMap<u32, String>>(&mut rustia)
            .unwrap(),
        BTreeMap::from([
            (1, "a".to_owned()),
            (2, "b".to_owned()),
            (3, "c".to_owned()),
        ])
    );

    for source in [
        "return { [1] = 'a', [3] = 'c' }",
        "return { [1] = 'a', label = 'mixed' }",
    ] {
        let value = eval_live(&mut rustia, source).unwrap();
        assert!(value.deserialize::<Vec<String>>(&mut rustia).is_err());
        let owned = value.to_owned(&mut rustia).unwrap();
        assert!(owned.deserialize::<Vec<String>>().is_err());
    }
}

#[test]
fn live_serde_rejects_cycles_depth_and_unsupported_values() {
    let mut rustia = rustia::RustiaBuilder::standard()
        .profile(Profile::ReproducibleStandard)
        .build()
        .unwrap();
    let cycle = eval_live(&mut rustia, "local t = {}; t.self = t; return t").unwrap();
    let error = cycle
        .deserialize::<BTreeMap<String, IgnoredAny>>(&mut rustia)
        .unwrap_err()
        .to_string();
    assert!(error.contains("cyclic table graph"), "{error}");

    let mutual = eval_live(
        &mut rustia,
        "local a, b = {}, {}; a.next = b; b.next = a; return a",
    )
    .unwrap();
    assert!(mutual
        .deserialize::<BTreeMap<String, IgnoredAny>>(&mut rustia)
        .is_err());

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Deep {
        #[serde(default)]
        next: Option<Box<Deep>>,
    }
    let deep = eval_live(
        &mut rustia,
        "local root = {}; local cursor = root; \
             for _ = 1, 65 do local next = {}; cursor.next = next; cursor = next end; \
             return root",
    )
    .unwrap();
    let error = deep
        .deserialize::<Deep>(&mut rustia)
        .unwrap_err()
        .to_string();
    assert!(error.contains("maximum table depth"), "{error}");

    for source in [
        "return function() end",
        "return buffer.create(4)",
        "return vector.create(1, 2, 3)",
        "return string.char(255)",
    ] {
        let value = eval_live(&mut rustia, source).unwrap();
        assert!(
            value.deserialize::<IgnoredAny>(&mut rustia).is_err(),
            "{source}"
        );
    }
}

#[test]
fn shared_acyclic_tables_deserialize_normally() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Child {
        value: u32,
    }
    #[derive(Debug, Deserialize, PartialEq)]
    struct Shared {
        left: Child,
        right: Child,
    }

    let mut rustia = rustia::RustiaBuilder::standard()
        .profile(Profile::ReproducibleStandard)
        .build()
        .unwrap();
    let value = eval_live(
        &mut rustia,
        "local child = { value = 7 }; return { left = child, right = child }",
    )
    .unwrap();
    assert_eq!(
        value.deserialize::<Shared>(&mut rustia).unwrap(),
        Shared {
            left: Child { value: 7 },
            right: Child { value: 7 },
        }
    );
}

#[test]
fn handle_identity_and_decision_epoch_checks_are_preserved() {
    let mut first = rustia::RustiaBuilder::standard()
        .profile(Profile::ReproducibleStandard)
        .build()
        .unwrap();
    let value = eval_live(&mut first, "return { answer = 42 }").unwrap();
    let table = value.as_table(&first).unwrap().unwrap();
    let exports = install(&mut first, "return { answer = 42 }").unwrap();

    let mut second = rustia::RustiaBuilder::standard()
        .profile(Profile::ReproducibleStandard)
        .build()
        .unwrap();
    assert!(value
        .deserialize::<BTreeMap<String, u32>>(&mut second)
        .is_err());

    eval_live(&mut first, "return 'unrelated evaluation'").unwrap();
    install(&mut first, "return { unrelated = true }").unwrap();
    assert_eq!(
        value
            .deserialize::<BTreeMap<String, u32>>(&mut first)
            .unwrap()["answer"],
        42
    );
    assert_eq!(
        table
            .deserialize::<BTreeMap<String, u32>>(&mut first)
            .unwrap()["answer"],
        42
    );
    assert_eq!(
        exports
            .deserialize::<BTreeMap<String, u32>>(&mut first)
            .unwrap()["answer"],
        42
    );

    first.reset_decision().unwrap();
    assert!(value
        .deserialize::<BTreeMap<String, u32>>(&mut first)
        .is_err());
}

struct TypedSchema;

impl HostSchema for TypedSchema {
    type Input = ();
    type Slots = ();
    type External = ();
    type Effect = Infallible;
    type SlotWrite = Infallible;
    type Record = Infallible;
}

#[test]
fn live_serde_supports_typed_host_vms() {
    let mut rustia = rustia::RustiaBuilder::standard()
        .profile(Profile::ReproducibleStandard)
        .build_for_host::<TypedSchema>()
        .unwrap();
    let table = rustia.create_table().unwrap();
    table.set(&mut rustia, "answer", 42_u32).unwrap();
    assert_eq!(
        table
            .deserialize::<BTreeMap<String, u32>>(&mut rustia)
            .unwrap()["answer"],
        42
    );
}

#[test]
fn live_deserialization_does_not_mutate_vm_memory_or_state() {
    let mut rustia = rustia::RustiaBuilder::standard()
        .profile(Profile::ReproducibleStandard)
        .build()
        .unwrap();
    let value = eval_live(
        &mut rustia,
        "local shared = { value = 7 }; collectgarbage('collect'); \
             return { left = shared, right = shared }",
    )
    .unwrap();
    let before = rustia.memory_stats();
    for _ in 0..128 {
        let _: BTreeMap<String, BTreeMap<String, u32>> = value.deserialize(&mut rustia).unwrap();
    }
    let after = rustia.memory_stats();
    assert_eq!(after.live_bytes(), before.live_bytes());
    assert_eq!(after.reserved_bytes(), before.reserved_bytes());
    assert_eq!(after.peak_live_bytes(), before.peak_live_bytes());
    assert_eq!(after.peak_reserved_bytes(), before.peak_reserved_bytes());

    struct Panics;
    impl<'de> Deserialize<'de> for Panics {
        fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            panic!("intentional deserialize panic")
        }
    }

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Panics = value.deserialize(&mut rustia).unwrap();
    }));
    assert!(panic.is_err());
    let recovered: BTreeMap<String, BTreeMap<String, u32>> =
        value.deserialize(&mut rustia).unwrap();
    assert_eq!(recovered["left"]["value"], 7);

    let eager = value.to_owned(&mut rustia).unwrap();
    assert!(matches!(eager, OwnedValue::Table(_)));
}
