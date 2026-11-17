//! Synchronous method adapters. Receivers are checked before other arguments.

use crate::function::{adapter::Token, host_arg_from_raw, host_return_to_raw};
use crate::{FromHostArgument, IntoHostReturn, Object};
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
