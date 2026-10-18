//! Object methods use the same borrowed views, dispatcher, and journal as functions.

use crate::function::{adapter::Token, host_arg_from_raw, host_return_to_raw};
use crate::invocation::{
    ContextHostFunction, EffectView, ExternalView, InputView, MakeCapabilityView, SlotReadView,
    SlotWriteView,
};
use crate::object_method::checked_receiver;
use crate::{FromHostArgument, HostSchema, IntoHostReturn, Object, ObjectType};
use rustia_vm::OpCapability;
use std::any::type_name;

mod adapter {
    use super::*;

    pub(crate) trait Read<T: 'static, H: HostSchema, A, R, V>:
        Send + Sync + 'static
    {
        fn into_context_object_method(
            self,
            token: Token,
            name: String,
            fuel_cost: u32,
            capability: OpCapability,
        ) -> ContextHostFunction<H>;
    }

    pub(crate) trait Write<T: 'static, H: HostSchema, A, R, V>:
        Send + Sync + 'static
    {
        fn into_mut_context_object_method(
            self,
            token: Token,
            name: String,
            fuel_cost: u32,
            capability: OpCapability,
        ) -> ContextHostFunction<H>;
    }
}

/// A read-only object method that borrows the current invocation's input.
///
/// Closures receive `&T`, [`crate::Input`], then up to eight typed arguments.
/// This sealed marker is implemented automatically for matching closures and
/// function pointers.
///
/// ```compile_fail
/// struct CustomMethod;
/// impl rustia::IntoInputObjectMethod<(), (), (), ()> for CustomMethod {}
/// ```
#[allow(private_bounds)]
pub trait IntoInputObjectMethod<T: 'static, H: HostSchema, A, R>:
    adapter::Read<T, H, A, R, InputView>
{
}
impl<T: 'static, H: HostSchema, A, R, F> IntoInputObjectMethod<T, H, A, R> for F where
    F: adapter::Read<T, H, A, R, InputView>
{
}

/// A read-only object method that borrows the invocation's saved slot snapshot.
///
/// Closures receive `&T`, [`crate::Slots`], then up to eight typed arguments.
/// This sealed marker is implemented automatically for matching callbacks.
#[allow(private_bounds)]
pub trait IntoSlotReadObjectMethod<T: 'static, H: HostSchema, A, R>:
    adapter::Read<T, H, A, R, SlotReadView>
{
}
impl<T: 'static, H: HostSchema, A, R, F> IntoSlotReadObjectMethod<T, H, A, R> for F where
    F: adapter::Read<T, H, A, R, SlotReadView>
{
}

/// An object method that stages effects in the invocation's existing journal.
///
/// Closures receive `&T`, [`crate::Effects`], then up to eight typed arguments.
/// This sealed marker is implemented automatically for matching callbacks.
#[allow(private_bounds)]
pub trait IntoEffectObjectMethod<T: 'static, H: HostSchema, A, R>:
    adapter::Read<T, H, A, R, EffectView>
{
}
impl<T: 'static, H: HostSchema, A, R, F> IntoEffectObjectMethod<T, H, A, R> for F where
    F: adapter::Read<T, H, A, R, EffectView>
{
}

/// An object method that stages slot writes in the invocation's existing journal.
///
/// Closures receive `&T`, [`crate::SlotWrites`], then up to eight typed arguments.
/// This sealed marker is implemented automatically for matching callbacks.
#[allow(private_bounds)]
pub trait IntoSlotWriteObjectMethod<T: 'static, H: HostSchema, A, R>:
    adapter::Read<T, H, A, R, SlotWriteView>
{
}
impl<T: 'static, H: HostSchema, A, R, F> IntoSlotWriteObjectMethod<T, H, A, R> for F where
    F: adapter::Read<T, H, A, R, SlotWriteView>
{
}

/// An object method with immediate access to external application state.
///
/// Closures receive `&T`, [`crate::HostMut`], then up to eight typed arguments.
/// This sealed marker is implemented automatically for matching callbacks.
#[allow(private_bounds)]
pub trait IntoExternalObjectMethod<T: 'static, H: HostSchema, A, R>:
    adapter::Read<T, H, A, R, ExternalView>
{
}
impl<T: 'static, H: HostSchema, A, R, F> IntoExternalObjectMethod<T, H, A, R> for F where
    F: adapter::Read<T, H, A, R, ExternalView>
{
}

/// A method that may change its payload and external application state immediately.
///
/// Closures receive `&mut T`, [`crate::HostMut`], then up to eight typed arguments.
/// This sealed marker is implemented automatically for matching callbacks.
#[allow(private_bounds)]
pub trait IntoExternalMutObjectMethod<T: 'static, H: HostSchema, A, R>:
    adapter::Write<T, H, A, R, ExternalView>
{
}
impl<T: 'static, H: HostSchema, A, R, F> IntoExternalMutObjectMethod<T, H, A, R> for F where
    F: adapter::Write<T, H, A, R, ExternalView>
{
}

impl<T: 'static, H: HostSchema> ObjectType<T, H> {
    /// Adds a read-only method over this invocation's deterministic input.
    pub fn input<F, A, R>(mut self, name: impl Into<String>, fuel_cost: u32, method: F) -> Self
    where
        F: IntoInputObjectMethod<T, H, A, R>,
    {
        self.methods.push(
            method
                .into_context_object_method(
                    Token,
                    name.into(),
                    fuel_cost,
                    OpCapability::ReadsDeterministicInput,
                )
                .into_op(),
        );
        self
    }

    /// Adds a read-only method over this invocation's saved slot snapshot.
    pub fn slot_read<F, A, R>(mut self, name: impl Into<String>, fuel_cost: u32, method: F) -> Self
    where
        F: IntoSlotReadObjectMethod<T, H, A, R>,
    {
        self.methods.push(
            method
                .into_context_object_method(
                    Token,
                    name.into(),
                    fuel_cost,
                    OpCapability::ReadsSlotState,
                )
                .into_op(),
        );
        self
    }

    /// Adds a method that stages effects with `&T` and [`crate::Effects`].
    ///
    /// Effects share the invocation's ordering and output limits. A failing
    /// method discards its own staged suffix; a failure escaping the outer
    /// invocation discards all its records.
    pub fn effect<F, A, R>(mut self, name: impl Into<String>, fuel_cost: u32, method: F) -> Self
    where
        F: IntoEffectObjectMethod<T, H, A, R>,
    {
        self.methods.push(
            method
                .into_context_object_method(
                    Token,
                    name.into(),
                    fuel_cost,
                    OpCapability::EmitsDeterministicEffect,
                )
                .into_op(),
        );
        self
    }

    /// Adds a method that stages saved-state changes with [`crate::SlotWrites`].
    ///
    /// Slot writes use the same journal, ordering, and limits as effects.
    pub fn slot_write<F, A, R>(mut self, name: impl Into<String>, fuel_cost: u32, method: F) -> Self
    where
        F: IntoSlotWriteObjectMethod<T, H, A, R>,
    {
        self.methods.push(
            method
                .into_context_object_method(
                    Token,
                    name.into(),
                    fuel_cost,
                    OpCapability::WritesSlotEffect,
                )
                .into_op(),
        );
        self
    }

    /// Adds a read-only payload method with immediate external-state access.
    ///
    /// The callback receives `&T`, then [`crate::HostMut`]. This declares
    /// external mutable state and is rejected by deterministic construction.
    /// Application changes are outside invocation and installation rollback.
    pub fn external<F, A, R>(mut self, name: impl Into<String>, fuel_cost: u32, method: F) -> Self
    where
        F: IntoExternalObjectMethod<T, H, A, R>,
    {
        self.methods.push(
            method
                .into_context_object_method(
                    Token,
                    name.into(),
                    fuel_cost,
                    OpCapability::ReadsExternalMutableState,
                )
                .into_op(),
        );
        self
    }

    /// Adds a read-only payload method that performs nondeterministic external work.
    ///
    /// The callback receives `&T`, then [`crate::HostMut`]. Deterministic
    /// construction rejects the definition. External effects are immediate.
    pub fn nondeterministic<F, A, R>(
        mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        method: F,
    ) -> Self
    where
        F: IntoExternalObjectMethod<T, H, A, R>,
    {
        self.methods.push(
            method
                .into_context_object_method(
                    Token,
                    name.into(),
                    fuel_cost,
                    OpCapability::NondeterministicEffect,
                )
                .into_op(),
        );
        self
    }

    /// Adds a method with `&mut T` and immediate external-state access.
    ///
    /// This declares external mutable state. Payload and application changes
    /// survive later script errors and failed installations. Reset drops the
    /// payload; it does not undo changes to the external application.
    pub fn external_mut<F, A, R>(
        mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        method: F,
    ) -> Self
    where
        F: IntoExternalMutObjectMethod<T, H, A, R>,
    {
        self.methods.push(
            method
                .into_mut_context_object_method(
                    Token,
                    name.into(),
                    fuel_cost,
                    OpCapability::ReadsExternalMutableState,
                )
                .into_op(),
        );
        self
    }

    /// Adds a mutable payload method that performs nondeterministic external work.
    ///
    /// Receives `&mut T`, then [`crate::HostMut`]. Both changes are immediate,
    /// and deterministic construction rejects the complete definition.
    pub fn nondeterministic_mut<F, A, R>(
        mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        method: F,
    ) -> Self
    where
        F: IntoExternalMutObjectMethod<T, H, A, R>,
    {
        self.methods.push(
            method
                .into_mut_context_object_method(
                    Token,
                    name.into(),
                    fuel_cost,
                    OpCapability::NondeterministicEffect,
                )
                .into_op(),
        );
        self
    }
}

macro_rules! read_method {
    ($count:expr; $($arg:ident),*) => {
        impl<T, H, F, R, V, $($arg),*> adapter::Read<T, H, ($($arg,)*), R, V> for F
        where
            T: 'static, H: HostSchema, V: MakeCapabilityView<H> + 'static,
            F: for<'call> Fn(&T, V::View<'call>, $($arg),*) -> R + Send + Sync + 'static,
            R: IntoHostReturn, $($arg: FromHostArgument,)*
        {
            #[allow(non_snake_case, unused_mut, unused_variables)]
            fn into_context_object_method(self, _: Token, name: String, fuel_cost: u32, capability: OpCapability) -> ContextHostFunction<H> {
                let diagnostic_name = name.clone();
                ContextHostFunction {
                    name, fuel_cost, capability,
                    parameter_count: 1 + $count,
                    parameter_types: vec![type_name::<Object<T>>(), $(type_name::<$arg>()),*],
                    return_types: R::rustia_type_names(Token),
                    fallible: R::is_fallible(Token),
                    handler: Box::new(move |vm, context, journal, args| {
                        let object = checked_receiver::<T>(vm, args)?;
                        let mut index = 1;
                        $(let $arg = host_arg_from_raw(vm, args, index, &diagnostic_name)?; index += 1;)*
                        let _ = index;
                        let result = {
                            let receiver = object.try_borrow::<T>()?;
                            let view = V::make(context, journal);
                            self(&receiver, view, $($arg),*)
                        };
                        host_return_to_raw(vm, &diagnostic_name, result)
                    }),
                }
            }
        }
    };
}

macro_rules! write_method {
    ($count:expr; $($arg:ident),*) => {
        impl<T, H, F, R, V, $($arg),*> adapter::Write<T, H, ($($arg,)*), R, V> for F
        where
            T: 'static, H: HostSchema, V: MakeCapabilityView<H> + 'static,
            F: for<'call> Fn(&mut T, V::View<'call>, $($arg),*) -> R + Send + Sync + 'static,
            R: IntoHostReturn, $($arg: FromHostArgument,)*
        {
            #[allow(non_snake_case, unused_mut, unused_variables)]
            fn into_mut_context_object_method(self, _: Token, name: String, fuel_cost: u32, capability: OpCapability) -> ContextHostFunction<H> {
                let diagnostic_name = name.clone();
                ContextHostFunction {
                    name, fuel_cost, capability,
                    parameter_count: 1 + $count,
                    parameter_types: vec![type_name::<Object<T>>(), $(type_name::<$arg>()),*],
                    return_types: R::rustia_type_names(Token),
                    fallible: R::is_fallible(Token),
                    handler: Box::new(move |vm, context, journal, args| {
                        let object = checked_receiver::<T>(vm, args)?;
                        let mut index = 1;
                        $(let $arg = host_arg_from_raw(vm, args, index, &diagnostic_name)?; index += 1;)*
                        let _ = index;
                        let result = {
                            let mut receiver = object.try_borrow_mut::<T>()?;
                            let view = V::make(context, journal);
                            self(&mut receiver, view, $($arg),*)
                        };
                        host_return_to_raw(vm, &diagnostic_name, result)
                    }),
                }
            }
        }
    };
}

read_method!(0;);
read_method!(1; A);
read_method!(2; A, B);
read_method!(3; A, B, C);
read_method!(4; A, B, C, D);
read_method!(5; A, B, C, D, E);
read_method!(6; A, B, C, D, E, G);
read_method!(7; A, B, C, D, E, G, I);
read_method!(8; A, B, C, D, E, G, I, J);
write_method!(0;);
write_method!(1; A);
write_method!(2; A, B);
write_method!(3; A, B, C);
write_method!(4; A, B, C, D);
write_method!(5; A, B, C, D, E);
write_method!(6; A, B, C, D, E, G);
write_method!(7; A, B, C, D, E, G, I);
write_method!(8; A, B, C, D, E, G, I, J);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Effects, HostContext, HostMut, Input, InvocationLimits, RustiaBuilder, SlotWrites, Slots,
    };

    struct Schema;
    impl HostSchema for Schema {
        type Input = i64;
        type Slots = i64;
        type External = i64;
        type Effect = i64;
        type SlotWrite = i64;
        type Record = (u32, &'static str, i64);
    }

    struct Context<'a> {
        input: &'a i64,
        slots: &'a i64,
        external: &'a mut i64,
    }

    impl HostContext<Schema> for Context<'_> {
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
        fn record_bytes(&self, _: &(u32, &'static str, i64)) -> u64 {
            16
        }
    }

    struct Actor(i64);

    fn input_function(actor: &Actor, input: Input<'_, Schema>) -> i64 {
        actor.0 + input.get()
    }

    fn add_input<F, A, R>(
        definition: ObjectType<Actor, Schema>,
        method: F,
    ) -> ObjectType<Actor, Schema>
    where
        F: IntoInputObjectMethod<Actor, Schema, A, R>,
    {
        definition.input("input", 1, method)
    }

    #[test]
    fn views_use_each_calls_borrowed_context_and_the_existing_ordered_journal() {
        let definition = add_input(ObjectType::<Actor, Schema>::new("Actor"), input_function)
            .slot_read("slot", 1, |actor: &Actor, slots: Slots<'_, Schema>| {
                actor.0 + slots.get()
            })
            .effect(
                "emit",
                1,
                |actor: &Actor, mut effects: Effects<'_, Schema>, amount: i64| {
                    effects.emit(actor.0 + amount)
                },
            )
            .slot_write(
                "write",
                1,
                |actor: &Actor, mut writes: SlotWrites<'_, Schema>, amount: i64| {
                    writes.write(actor.0 + amount)
                },
            )
            .external(
                "external",
                1,
                |actor: &Actor, mut host: HostMut<'_, Schema>| actor.0 + *host.get_mut(),
            )
            .nondeterministic(
                "external_add",
                1,
                |actor: &Actor, mut host: HostMut<'_, Schema>, amount: i64| {
                    *host.get_mut() += actor.0 + amount;
                },
            )
            .external_mut(
                "set",
                1,
                |actor: &mut Actor, mut host: HostMut<'_, Schema>, amount: i64| {
                    actor.0 = amount;
                    *host.get_mut() += amount;
                },
            )
            .nondeterministic_mut(
                "add",
                1,
                |actor: &mut Actor, mut host: HostMut<'_, Schema>, amount: i64| {
                    actor.0 += amount;
                    *host.get_mut() += amount;
                },
            );
        let mut lua = RustiaBuilder::standard()
            .without_stdout()
            .build_for_host::<Schema>()
            .unwrap();
        lua.register_object_type(definition).unwrap();
        let actor = lua.create_object(Actor(2)).unwrap();
        lua.globals()
            .unwrap()
            .set_live(&mut lua, "actor", &actor.as_value())
            .unwrap();
        let mut external = 10;
        for input in [3, 7] {
            let slots = 5;
            let mut context = Context {
                input: &input,
                slots: &slots,
                external: &mut external,
            };
            let outcome = lua.invoke(&mut context, InvocationLimits::new(10_000).with_max_records(2).with_max_record_bytes(64), |call| {
                call.load("actor:emit(8); actor:write(9); return {actor:input(), actor:slot(), actor:external()}")
                    .eval::<Vec<i64>>()
            }).unwrap();
            assert_eq!(outcome.value, vec![input + 2, 7, 12]);
            assert_eq!(outcome.records, [(0, "effect", 10), (1, "slot", 11)]);
        }
        let mut context = Context {
            input: &0,
            slots: &0,
            external: &mut external,
        };
        lua.invoke(
            &mut context,
            InvocationLimits::new(10_000)
                .with_max_records(2)
                .with_max_record_bytes(64),
            |call| {
                call.load("actor:external_add(1); actor:set(4); actor:add(3); error('later')")
                    .exec()
            },
        )
        .unwrap_err();
        assert_eq!(external, 20);
        let mut context = Context {
            input: &0,
            slots: &0,
            external: &mut external,
        };
        let outcome = lua
            .invoke(
                &mut context,
                InvocationLimits::new(10_000)
                    .with_max_records(2)
                    .with_max_record_bytes(64),
                |call| call.load("return actor:input()").eval::<i64>(),
            )
            .unwrap();
        assert_eq!(outcome.value, 7);
        assert!(outcome.records.is_empty());
    }

    #[test]
    fn contextual_adapters_support_eight_explicit_arguments() {
        let definition = ObjectType::<Actor, Schema>::new("Actor")
            .input(
                "sum",
                1,
                |actor: &Actor,
                 input: Input<'_, Schema>,
                 a: i64,
                 b: i64,
                 c: i64,
                 d: i64,
                 e: i64,
                 f: i64,
                 g: i64,
                 h: i64| { actor.0 + input.get() + a + b + c + d + e + f + g + h },
            )
            .external_mut(
                "sum_mut",
                1,
                |actor: &mut Actor,
                 mut host: HostMut<'_, Schema>,
                 a: i64,
                 b: i64,
                 c: i64,
                 d: i64,
                 e: i64,
                 f: i64,
                 g: i64,
                 h: i64| {
                    actor.0 += a + b + c + d + e + f + g + h;
                    *host.get_mut() += actor.0;
                    actor.0
                },
            );
        let mut lua = RustiaBuilder::standard()
            .without_stdout()
            .build_for_host::<Schema>()
            .unwrap();
        lua.register_object_type(definition).unwrap();
        let actor = lua.create_object(Actor(2)).unwrap();
        lua.globals()
            .unwrap()
            .set_live(&mut lua, "actor", &actor.as_value())
            .unwrap();
        let mut external = 0;
        let mut context = Context {
            input: &3,
            slots: &0,
            external: &mut external,
        };
        let outcome = lua
            .invoke(
                &mut context,
                InvocationLimits::new(10_000)
                    .with_max_records(2)
                    .with_max_record_bytes(64),
                |call| {
                    call.load("return {actor:sum(1,2,3,4,5,6,7,8), actor:sum_mut(1,2,3,4,5,6,7,8)}")
                        .eval::<Vec<i64>>()
                },
            )
            .unwrap();
        assert_eq!(outcome.value, vec![41, 38]);
        assert_eq!(external, 38);
    }

    #[test]
    fn wrong_receivers_do_not_project_the_host_context() {
        struct Observe {
            input_reads: std::cell::Cell<u32>,
            external_reads: u32,
            value: i64,
        }
        impl HostContext<Schema> for Observe {
            fn input(&self) -> &i64 {
                self.input_reads.set(self.input_reads.get() + 1);
                &self.value
            }
            fn slots(&self) -> &i64 {
                &self.value
            }
            fn external_mut(&mut self) -> &mut i64 {
                self.external_reads += 1;
                &mut self.value
            }
            fn record_effect(&self, sequence: u32, effect: i64) -> (u32, &'static str, i64) {
                (sequence, "effect", effect)
            }
            fn record_slot_write(&self, sequence: u32, write: i64) -> (u32, &'static str, i64) {
                (sequence, "slot", write)
            }
            fn record_bytes(&self, _: &(u32, &'static str, i64)) -> u64 {
                16
            }
        }
        let mut lua = RustiaBuilder::standard()
            .without_stdout()
            .build_for_host::<Schema>()
            .unwrap();
        lua.register_object_type(
            ObjectType::<Actor, Schema>::new("Actor")
                .input("read", 1, |_: &Actor, input: Input<'_, Schema>| {
                    *input.get()
                })
                .external_mut(
                    "change",
                    1,
                    |_: &mut Actor, mut host: HostMut<'_, Schema>| *host.get_mut() += 1,
                ),
        )
        .unwrap();
        let actor = lua.create_object(Actor(2)).unwrap();
        lua.globals()
            .unwrap()
            .set_live(&mut lua, "actor", &actor.as_value())
            .unwrap();
        let mut context = Observe {
            input_reads: std::cell::Cell::new(0),
            external_reads: 0,
            value: 9,
        };
        let outcome = lua.invoke(&mut context, InvocationLimits::new(10_000), |call| {
            call.load("assert(not pcall(actor.read, {})); assert(not pcall(actor.change, {})); return actor:read()")
                .eval::<i64>()
        }).unwrap();
        assert_eq!(outcome.value, 9);
        assert_eq!(context.input_reads.get(), 1);
        assert_eq!(context.external_reads, 0);
    }
}
