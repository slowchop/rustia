//! Synchronous method adapters. Receivers are checked before other arguments.

use crate::function::{adapter::Token, host_arg_from_raw, host_embed_error, host_return_to_raw};
use crate::{FromHostArgument, FromRustia, IntoHostReturn, Object, Result};
use rustia_vm::{HostFunctionOp, OpCapability, Value, Vm};
use std::any::{type_name, TypeId};
use std::sync::Arc;

pub(crate) mod adapter {
    use super::*;

    pub(crate) trait Read<T: 'static, A, R>: Send + Sync + 'static {
        fn into_object_method(
            self,
            token: Token,
            name: String,
            fuel_cost: u32,
            capability: OpCapability,
        ) -> HostFunctionOp;
    }

    pub(crate) trait Write<T: 'static, A, R>: Send + Sync + 'static {
        fn into_mut_object_method(
            self,
            token: Token,
            name: String,
            fuel_cost: u32,
            capability: OpCapability,
        ) -> HostFunctionOp;
    }

    pub(crate) trait Iterate<T: 'static, S, C, R>: Send + Sync + 'static {
        fn into_object_iterator(
            self,
            token: Token,
            fuel_cost: u32,
            capability: OpCapability,
        ) -> (HostFunctionOp, HostFunctionOp);
    }

    pub(crate) trait IterationReturn<C> {
        type Item: IntoHostReturn;
        fn into_iteration_return(self) -> Result<Option<Self::Item>>;
        fn is_fallible() -> bool;
    }

    #[cfg(not(feature = "transferable"))]
    pub(crate) trait IterationState: 'static {}
    #[cfg(not(feature = "transferable"))]
    impl<T: 'static> IterationState for T {}

    #[cfg(feature = "transferable")]
    pub(crate) trait IterationState: Send + Sync + 'static {}
    #[cfg(feature = "transferable")]
    impl<T: Send + Sync + 'static> IterationState for T {}
}

/// A read-only Rust-object method accepted by [`crate::ObjectType`].
///
/// Closures receive `&T`, then up to eight typed arguments. This sealed marker
/// is implemented automatically; use ordinary closures or function pointers.
#[allow(private_bounds)]
pub trait IntoObjectMethod<T: 'static, A, R>: adapter::Read<T, A, R> {}
impl<T: 'static, A, R, F: adapter::Read<T, A, R>> IntoObjectMethod<T, A, R> for F {}

/// An immediately mutable Rust-object method accepted by [`crate::ObjectType`].
///
/// Closures receive `&mut T`, then up to eight typed arguments. Such methods
/// declare external mutation and deterministic construction rejects them.
#[allow(private_bounds)]
pub trait IntoMutObjectMethod<T: 'static, A, R>: adapter::Write<T, A, R> {}
impl<T: 'static, A, R, F: adapter::Write<T, A, R>> IntoMutObjectMethod<T, A, R> for F {}

/// A typed generalized-iterator step accepted by [`crate::ObjectType`].
///
/// The factory receives the payload once and returns owned state for one loop.
/// The step receives that private state and the previous control value. Returning
/// `None` completes the loop. A returned tuple's
/// first value is both Luau's next control value and the loop's first visible
/// value, exactly like a regular Luau iterator function. Rustia requires that
/// first value to have exactly the same type as the incoming control.
///
/// ```compile_fail
/// use rustia::ObjectType;
/// struct Values;
/// let _ = ObjectType::<Values>::new("Values").iterator(1, |_| Ok::<_, rustia::Error>(()),
///     |_: &mut (), _: Option<i64>| Some(("wrong control", 1_i64)));
/// ```
///
/// An iterator step must return at least its next control value:
///
/// ```compile_fail
/// use rustia::ObjectType;
/// struct Values;
/// let _ = ObjectType::<Values>::new("Values").iterator(1, |_| Ok::<_, rustia::Error>(()),
///     |_: &mut (), _: Option<i64>| Some(()));
/// ```
#[allow(private_bounds)]
pub trait IntoObjectIterator<T: 'static, S, C, R>: adapter::Iterate<T, S, C, R> {}
impl<T: 'static, S, C, R, F: adapter::Iterate<T, S, C, R>> IntoObjectIterator<T, S, C, R> for F {}

impl<C: IntoHostReturn> adapter::IterationReturn<C> for Option<C> {
    type Item = C;

    fn into_iteration_return(self) -> Result<Option<C>> {
        Ok(self)
    }

    fn is_fallible() -> bool {
        false
    }
}

impl<C: IntoHostReturn> adapter::IterationReturn<C> for Result<Option<C>> {
    type Item = C;

    fn into_iteration_return(self) -> Result<Option<C>> {
        self
    }

    fn is_fallible() -> bool {
        true
    }
}

macro_rules! iteration_tuple_return {
    ($($name:ident),+ $(,)?) => {
        impl<C, $($name),+> adapter::IterationReturn<C> for Option<(C, $($name,)+)>
        where
            (C, $($name,)+): IntoHostReturn,
        {
            type Item = (C, $($name,)+);

            fn into_iteration_return(self) -> Result<Option<Self::Item>> {
                Ok(self)
            }

            fn is_fallible() -> bool {
                false
            }
        }

        impl<C, $($name),+> adapter::IterationReturn<C> for Result<Option<(C, $($name,)+)>>
        where
            (C, $($name,)+): IntoHostReturn,
        {
            type Item = (C, $($name,)+);

            fn into_iteration_return(self) -> Result<Option<Self::Item>> {
                self
            }

            fn is_fallible() -> bool {
                true
            }
        }
    };
}

iteration_tuple_return!(A);
iteration_tuple_return!(A, B);
iteration_tuple_return!(A, B, D);
iteration_tuple_return!(A, B, D, E);
iteration_tuple_return!(A, B, D, E, G);
iteration_tuple_return!(A, B, D, E, G, H);
iteration_tuple_return!(A, B, D, E, G, H, I);

pub(crate) fn checked_receiver<T: 'static>(
    vm: &mut Vm,
    args: &[Value],
) -> rustia_vm::Result<rustia_vm::HostObjectOwner> {
    vm.checked_host_object(
        args.first().copied().unwrap_or_else(Value::nil),
        TypeId::of::<T>(),
        type_name::<T>(),
    )
}

macro_rules! read_method {
    ($count:expr; $($arg:ident),*) => {
        impl<T, F, R, $($arg),*> adapter::Read<T, ($($arg,)*), R> for F
        where T: 'static, F: Fn(&T, $($arg),*) -> R + Send + Sync + 'static,
              R: IntoHostReturn, $($arg: FromHostArgument,)*
        {
            #[allow(non_snake_case, unused_mut, unused_variables)]
            fn into_object_method(self, _: Token, name: String, fuel_cost: u32, capability: OpCapability) -> HostFunctionOp {
                let diagnostic_name = name.clone();
                HostFunctionOp::new(name, fuel_cost, capability, Arc::new(move |vm, args| {
                    let object = checked_receiver::<T>(vm, args)?;
                    let mut index = 1;
                    $(let $arg = host_arg_from_raw(vm, args, index, &diagnostic_name)?; index += 1;)*
                    let _ = index;
                    let result = {
                        let receiver = object.try_borrow::<T>()?;
                        self(&receiver, $($arg),*)
                    };
                    host_return_to_raw(vm, &diagnostic_name, result)
                })).with_signature(
                    1 + $count,
                    vec![type_name::<Object<T>>(), $(type_name::<$arg>()),*],
                    R::rustia_type_names(Token), R::is_fallible(Token),
                )
            }
        }
    };
}

macro_rules! write_method {
    ($count:expr; $($arg:ident),*) => {
        impl<T, F, R, $($arg),*> adapter::Write<T, ($($arg,)*), R> for F
        where T: 'static, F: Fn(&mut T, $($arg),*) -> R + Send + Sync + 'static,
              R: IntoHostReturn, $($arg: FromHostArgument,)*
        {
            #[allow(non_snake_case, unused_mut, unused_variables)]
            fn into_mut_object_method(self, _: Token, name: String, fuel_cost: u32, capability: OpCapability) -> HostFunctionOp {
                let diagnostic_name = name.clone();
                HostFunctionOp::new(name, fuel_cost, capability, Arc::new(move |vm, args| {
                    let object = checked_receiver::<T>(vm, args)?;
                    let mut index = 1;
                    $(let $arg = host_arg_from_raw(vm, args, index, &diagnostic_name)?; index += 1;)*
                    let _ = index;
                    let result = {
                        let mut receiver = object.try_borrow_mut::<T>()?;
                        self(&mut receiver, $($arg),*)
                    };
                    host_return_to_raw(vm, &diagnostic_name, result)
                })).with_signature(
                    1 + $count,
                    vec![type_name::<Object<T>>(), $(type_name::<$arg>()),*],
                    R::rustia_type_names(Token), R::is_fallible(Token),
                )
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
read_method!(7; A, B, C, D, E, G, H);
read_method!(8; A, B, C, D, E, G, H, I);
write_method!(0;);
write_method!(1; A);
write_method!(2; A, B);
write_method!(3; A, B, C);
write_method!(4; A, B, C, D);
write_method!(5; A, B, C, D, E);
write_method!(6; A, B, C, D, E, G);
write_method!(7; A, B, C, D, E, G, H);
write_method!(8; A, B, C, D, E, G, H, I);

impl<T, Factory, Step, S, C, R> adapter::Iterate<T, S, C, R> for (Factory, Step)
where
    T: 'static,
    Factory: Fn(&T) -> Result<S> + Send + Sync + 'static,
    Step: Fn(&mut S, Option<C>) -> R + Send + Sync + 'static,
    S: adapter::IterationState,
    C: FromRustia,
    R: adapter::IterationReturn<C>,
{
    fn into_object_iterator(
        self,
        _: Token,
        fuel_cost: u32,
        capability: OpCapability,
    ) -> (HostFunctionOp, HostFunctionOp) {
        let (factory, step) = self;
        let factory_op = HostFunctionOp::new(
            "__iter",
            fuel_cost,
            capability,
            Arc::new(move |vm, args| {
                let receiver_raw = args.first().copied().unwrap_or_else(Value::nil);
                let object = checked_receiver::<T>(vm, args)?;
                let state = {
                    let receiver = object.try_borrow::<T>()?;
                    factory(&receiver)
                }
                .map_err(|error| host_embed_error("__iter", "return value", error))?;
                let iterator = vm.host_object_iterator_step(receiver_raw)?;
                let state = vm.create_host_iterator_state(
                    receiver_raw,
                    TypeId::of::<S>(),
                    type_name::<S>(),
                    Box::new(state),
                )?;
                Ok(vec![iterator, state, Value::nil()])
            }),
        )
        .with_signature(
            1,
            vec![type_name::<Object<T>>()],
            vec!["iterator", type_name::<S>(), "nil"],
            true,
        );
        let step_op = HostFunctionOp::new(
            "__iter_step",
            fuel_cost,
            capability,
            Arc::new(move |vm, args| {
                let control = host_arg_from_raw::<Option<C>>(vm, args, 1, "__iter")?;
                let state = checked_receiver::<S>(vm, args)?;
                let result = {
                    let mut state = state.try_borrow_mut::<S>()?;
                    step(&mut state, control)
                };
                match result
                    .into_iteration_return()
                    .map_err(|error| host_embed_error("__iter", "return value", error))?
                {
                    Some(values) => host_return_to_raw(vm, "__iter", values),
                    None => Ok(vec![Value::nil()]),
                }
            }),
        )
        .with_signature(
            2,
            vec![type_name::<S>(), type_name::<Option<C>>()],
            <R::Item as crate::function::adapter::Return>::rustia_type_names(Token),
            R::is_fallible(),
        );
        (factory_op, step_op)
    }
}
