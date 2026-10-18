use rustia::{
    Effects, FromHostArgument, FromRustia, HostContext, HostFunction, HostMut, HostSchema, Input,
    IntoEffectFunction, IntoExternalFunction, IntoHostFunction, IntoHostFunctionDefinition,
    IntoHostReturn, IntoHostValue, IntoInputFunction, IntoRustia, IntoSlotReadFunction,
    IntoSlotWriteFunction, InvocationLimits, OwnedValue, Result, Rustia, RustiaBuilder, SlotWrites,
    Slots, Table,
};

// These helpers exercise the nameable facade bounds an application uses when
// wrapping Rustia registration. They must not require raw VM adapter methods.
fn pure<F, A, R>(function: F) -> HostFunction
where
    F: IntoHostFunction<A, R>,
{
    HostFunction::pure(function)
}

fn register<F, S>(lua: &mut Rustia, name: &str, function: F) -> Result<()>
where
    F: IntoHostFunctionDefinition<(), S>,
{
    lua.register_function(name, function)
}

#[derive(Debug, Eq, PartialEq)]
struct Points(i64);

impl FromRustia for Points {
    fn from_rustia(value: OwnedValue) -> Result<Self> {
        i64::from_rustia(value).map(Self)
    }
}

impl IntoRustia for Points {
    fn into_rustia(self) -> Result<OwnedValue> {
        self.0.into_rustia()
    }
}

fn add_points(points: Points, table: Table) -> Result<(Points, Table)> {
    Ok((Points(points.0 + 2), table))
}

#[test]
fn public_bounds_accept_function_pointers_custom_conversions_and_live_results() -> Result<()> {
    fn argument<T: FromHostArgument>() {}
    fn value<T: IntoHostValue>() {}
    fn results<T: IntoHostReturn>() {}

    argument::<Points>();
    value::<Points>();
    results::<Result<(Points, Table)>>();

    let mut lua = Rustia::builder().build()?;
    register(&mut lua, "add_points", pure(add_points))?;
    register(&mut lua, "plain", || 7_i64)?;
    let matched = lua
        .load(
            "local t = {}; t.self = t; local points, alias = add_points(40, t); \
             return points == 42 and alias == t and alias.self == alias and plain() == 7",
        )
        .eval::<bool>()?;
    assert!(matched);
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

struct Turn<'a> {
    input: &'a i64,
    slots: &'a i64,
    external: &'a mut i64,
}

impl HostContext<Game> for Turn<'_> {
    fn input(&self) -> &i64 {
        self.input
    }

    fn slots(&self) -> &i64 {
        self.slots
    }

    fn external_mut(&mut self) -> &mut i64 {
        self.external
    }

    fn record_effect(&self, sequence: u32, effect: i64) -> (u32, &'static str, i64) {
        (sequence, "effect", effect)
    }

    fn record_slot_write(&self, sequence: u32, write: i64) -> (u32, &'static str, i64) {
        (sequence, "slot", write)
    }

    fn record_bytes(&self, _record: &(u32, &'static str, i64)) -> u64 {
        16
    }
}

fn input<F, A, R>(function: F) -> HostFunction<Game>
where
    F: IntoInputFunction<Game, A, R>,
{
    HostFunction::input(function)
}

fn slot_read<F, A, R>(function: F) -> HostFunction<Game>
where
    F: IntoSlotReadFunction<Game, A, R>,
{
    HostFunction::slot_read(function)
}

fn effect<F, A, R>(function: F) -> HostFunction<Game>
where
    F: IntoEffectFunction<Game, A, R>,
{
    HostFunction::effect(function)
}

fn slot_write<F, A, R>(function: F) -> HostFunction<Game>
where
    F: IntoSlotWriteFunction<Game, A, R>,
{
    HostFunction::slot_write(function)
}

fn external<F, A, R>(function: F) -> HostFunction<Game>
where
    F: IntoExternalFunction<Game, A, R>,
{
    HostFunction::external(function)
}

#[test]
fn public_context_bounds_preserve_borrowed_host_implementations_and_staged_order() -> Result<()> {
    let mut lua = RustiaBuilder::deterministic().build_for_host::<Game>()?;
    lua.register_function("read", input(|input: Input<'_, Game>| *input.get()))?;
    lua.register_function("slot", slot_read(|slots: Slots<'_, Game>| *slots.get()))?;
    lua.register_function(
        "emit",
        effect(|mut effects: Effects<'_, Game>, value: Points| effects.emit(value.0)),
    )?;
    lua.register_function(
        "write",
        slot_write(|mut writes: SlotWrites<'_, Game>, value: Points| writes.write(value.0)),
    )?;
    let input = 40;
    let slots = 2;
    let mut external = 0;
    let mut turn = Turn {
        input: &input,
        slots: &slots,
        external: &mut external,
    };
    let outcome = lua.invoke(
        &mut turn,
        InvocationLimits::new(1_000)
            .with_max_records(2)
            .with_max_record_bytes(32),
        |invocation| {
            invocation
                .load("emit(read() + slot()); write(9); return read()")
                .eval::<i64>()
        },
    )?;
    assert_eq!(*outcome.value(), 40);
    assert_eq!(outcome.records(), &[(0, "effect", 42), (1, "slot", 9)]);
    assert_eq!(external, 0);
    Ok(())
}

#[test]
fn public_external_bound_preserves_immediate_mutation_and_deterministic_rejection() -> Result<()> {
    fn change(mut host: HostMut<'_, Game>, value: Points) {
        *host.get_mut() = value.0;
    }

    let mut deterministic = RustiaBuilder::deterministic().build_for_host::<Game>()?;
    assert!(deterministic
        .register_function("change", external(change))
        .is_err());

    let mut lua = RustiaBuilder::standard().build_for_host::<Game>()?;
    lua.register_function("change", external(change))?;
    let mut external = 0;
    let mut turn = Turn {
        input: &0,
        slots: &0,
        external: &mut external,
    };
    let result = lua.invoke(&mut turn, InvocationLimits::new(1_000), |invocation| {
        invocation
            .load("change(42); error('after mutation')")
            .exec()
    });
    assert!(result.is_err());
    assert_eq!(external, 42);
    Ok(())
}
