//! Opaque Rust payloads with feature-selected ownership.

use crate::ownership::Shared;
use crate::{Error, ErrorKind, HostFunctionOp, Result};
use std::any::{type_name, Any, TypeId};
#[cfg(not(feature = "send"))]
use std::cell::{Ref, RefCell, RefMut};
use std::fmt;
use std::mem::size_of;
#[cfg(feature = "send")]
use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard, TryLockError},
};

/// Shared payload owner; local by default, transferable with `send`.
pub type HostObjectOwner = Shared<HostObjectData>;
/// Accepted owned payload; `send` requires shared reads to be thread-safe.
#[cfg(not(feature = "send"))]
pub type HostObjectPayload = dyn Any;
#[cfg(feature = "send")]
pub type HostObjectPayload = dyn Any + Send + Sync;
#[cfg(not(feature = "send"))]
type PayloadCell = RefCell<Box<HostObjectPayload>>;
#[cfg(feature = "send")]
type PayloadCell = RwLock<Box<HostObjectPayload>>;

/// Scoped read access to a type-checked payload.
#[cfg(not(feature = "send"))]
pub type HostObjectRef<'a, T> = Ref<'a, T>;
#[cfg(feature = "send")]
pub struct HostObjectRef<'a, T> {
    guard: RwLockReadGuard<'a, Box<HostObjectPayload>>,
    marker: PhantomData<&'a T>,
}
/// Scoped exclusive access to a type-checked payload.
#[cfg(not(feature = "send"))]
pub type HostObjectMut<'a, T> = RefMut<'a, T>;
#[cfg(feature = "send")]
pub struct HostObjectMut<'a, T> {
    guard: RwLockWriteGuard<'a, Box<HostObjectPayload>>,
    marker: PhantomData<&'a mut T>,
}
#[cfg(feature = "send")]
impl<T: 'static> Deref for HostObjectRef<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.guard
            .downcast_ref()
            .expect("payload type checked before borrowing")
    }
}
#[cfg(feature = "send")]
impl<T: 'static> Deref for HostObjectMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.guard
            .downcast_ref()
            .expect("payload type checked before borrowing")
    }
}
#[cfg(feature = "send")]
impl<T: 'static> DerefMut for HostObjectMut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.guard
            .downcast_mut()
            .expect("payload type checked before borrowing")
    }
}

/// Optional userdata operations. Each slot uses the ordinary host callback
/// capability, fuel, dispatcher and ownership rules. Argument arity is left to
/// the low-level handler; the facade adds typed adapters.
#[derive(Clone, Debug, Default)]
pub struct HostObjectHooks {
    pub index: Option<HostFunctionOp>,
    pub new_index: Option<HostFunctionOp>,
    pub equals: Option<HostFunctionOp>,
    pub display: Option<HostFunctionOp>,
    pub call: Option<HostFunctionOp>,
}

impl HostObjectHooks {
    /// Operations in stable slot order; names are the guest metamethod keys.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &HostFunctionOp)> {
        [
            ("__index", self.index.as_ref()),
            ("__newindex", self.new_index.as_ref()),
            ("__eq", self.equals.as_ref()),
            ("__tostring", self.display.as_ref()),
            ("__call", self.call.as_ref()),
        ]
        .into_iter()
        .filter_map(|(name, op)| op.map(|op| (name, op)))
    }
}

/// Persistent method configuration for one Rust payload type.
#[derive(Clone, Debug)]
pub struct HostObjectTypeDefinition {
    pub rust_type_id: TypeId,
    pub rust_type_name: &'static str,
    pub name: String,
    pub methods: Vec<HostFunctionOp>,
    pub hooks: HostObjectHooks,
}

/// Shared ownership is shallow across guest savepoints. Payload mutation is
/// external state and is never a guest-heap rollback operation.
pub struct HostObjectData {
    rust_type_id: TypeId,
    rust_type_name: &'static str,
    payload: PayloadCell,
}

impl fmt::Debug for HostObjectData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostObjectData")
            .field("rust_type_name", &self.rust_type_name)
            .finish_non_exhaustive()
    }
}

impl HostObjectData {
    pub(crate) fn new(rust_type_name: &'static str, payload: Box<HostObjectPayload>) -> Self {
        Self {
            rust_type_id: payload.as_ref().type_id(),
            rust_type_name,
            payload: PayloadCell::new(payload),
        }
    }

    pub fn rust_type_id(&self) -> TypeId {
        self.rust_type_id
    }

    pub fn rust_type_name(&self) -> &'static str {
        self.rust_type_name
    }

    #[cfg(not(feature = "send"))]
    pub fn try_borrow<T: 'static>(&self) -> Result<HostObjectRef<'_, T>> {
        let value = self.payload.try_borrow().map_err(|_| {
            object_error(format!(
                "Rust object '{}' is already mutably borrowed",
                self.rust_type_name
            ))
        })?;
        Ref::filter_map(value, |value| value.downcast_ref::<T>())
            .map_err(|_| wrong_object_type(type_name::<T>()))
    }

    #[cfg(not(feature = "send"))]
    pub fn try_borrow_mut<T: 'static>(&self) -> Result<HostObjectMut<'_, T>> {
        let value = self.payload.try_borrow_mut().map_err(|_| {
            object_error(format!(
                "Rust object '{}' is already borrowed",
                self.rust_type_name
            ))
        })?;
        RefMut::filter_map(value, |value| value.downcast_mut::<T>())
            .map_err(|_| wrong_object_type(type_name::<T>()))
    }

    #[cfg(feature = "send")]
    pub fn try_borrow<T: 'static>(&self) -> Result<HostObjectRef<'_, T>> {
        let guard = match self.payload.try_read() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => {
                return Err(object_error(format!(
                    "Rust object '{}' is already mutably borrowed",
                    self.rust_type_name
                )))
            }
        };
        if !guard.is::<T>() {
            return Err(wrong_object_type(type_name::<T>()));
        }
        Ok(HostObjectRef {
            guard,
            marker: PhantomData,
        })
    }

    #[cfg(feature = "send")]
    pub fn try_borrow_mut<T: 'static>(&self) -> Result<HostObjectMut<'_, T>> {
        let guard = match self.payload.try_write() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => {
                return Err(object_error(format!(
                    "Rust object '{}' is already borrowed",
                    self.rust_type_name
                )))
            }
        };
        if !guard.is::<T>() {
            return Err(wrong_object_type(type_name::<T>()));
        }
        Ok(HostObjectMut {
            guard,
            marker: PhantomData,
        })
    }

    /// Fixed VM-owned owner/cell bookkeeping, excluding opaque payload storage.
    pub(crate) const fn live_bytes() -> usize {
        size_of::<Self>().saturating_add(2 * size_of::<usize>())
    }
}

pub(crate) fn object_error(message: String) -> Error {
    Error::new(ErrorKind::RuntimeError { message }).without_source_location()
}

pub(crate) fn wrong_object_type(expected: &str) -> Error {
    object_error(format!("expected Rust object '{expected}'"))
}

#[cfg(all(test, feature = "send"))]
mod send_tests {
    use super::*;

    #[test]
    fn alias_reads_and_conflicts_never_wait_for_the_same_payload() {
        let object = HostObjectData::new("counter", Box::new(7_i64));
        let first = object.try_borrow::<i64>().unwrap();
        let second = object.try_borrow::<i64>().unwrap();
        assert_eq!((*first, *second), (7, 7));
        assert!(object.try_borrow_mut::<i64>().is_err());
        drop((first, second));
        let mut exclusive = object.try_borrow_mut::<i64>().unwrap();
        *exclusive = 9;
        assert!(object.try_borrow::<i64>().is_err());
        assert!(object.try_borrow_mut::<i64>().is_err());
        drop(exclusive);
        assert!(object.try_borrow::<String>().is_err());
        assert_eq!(*object.try_borrow::<i64>().unwrap(), 9);
    }

    #[test]
    fn synchronization_poison_does_not_add_a_second_vm_poison_policy() {
        let object = HostObjectData::new("counter", Box::new(7_i64));
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut value = object.try_borrow_mut::<i64>().unwrap();
            *value = 11;
            panic!("callback panic after immediate payload mutation");
        }));
        assert!(panic.is_err());
        // The host execution boundary separately poisons its VM. This guard
        // must remain usable after that VM's explicit reset, like RefCell.
        assert_eq!(*object.try_borrow::<i64>().unwrap(), 11);
        *object.try_borrow_mut::<i64>().unwrap() = 12;
    }
}
