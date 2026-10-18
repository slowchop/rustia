//! Opaque, VM-local Rust payloads used by the embedding facade.

use crate::{Error, ErrorKind, HostFunctionOp, Result};
use std::any::{type_name, Any, TypeId};
use std::cell::{Ref, RefCell, RefMut};
use std::fmt;
use std::mem::size_of;

/// Persistent method configuration for one Rust payload type.
#[derive(Clone, Debug)]
pub struct HostObjectTypeDefinition {
    pub rust_type_id: TypeId,
    pub rust_type_name: &'static str,
    pub name: String,
    pub methods: Vec<HostFunctionOp>,
}

/// Shared ownership is shallow across guest savepoints. Payload mutation is
/// external state and is never a guest-heap rollback operation.
pub struct HostObjectData {
    rust_type_id: TypeId,
    rust_type_name: &'static str,
    payload: RefCell<Box<dyn Any>>,
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
    pub(crate) fn new(rust_type_name: &'static str, payload: Box<dyn Any>) -> Self {
        Self {
            rust_type_id: payload.as_ref().type_id(),
            rust_type_name,
            payload: RefCell::new(payload),
        }
    }

    pub fn rust_type_id(&self) -> TypeId {
        self.rust_type_id
    }

    pub fn rust_type_name(&self) -> &'static str {
        self.rust_type_name
    }

    pub fn try_borrow<T: 'static>(&self) -> Result<Ref<'_, T>> {
        let value = self.payload.try_borrow().map_err(|_| {
            object_error(format!(
                "Rust object '{}' is already mutably borrowed",
                self.rust_type_name
            ))
        })?;
        Ref::filter_map(value, |value| value.downcast_ref::<T>())
            .map_err(|_| wrong_object_type(type_name::<T>()))
    }

    pub fn try_borrow_mut<T: 'static>(&self) -> Result<RefMut<'_, T>> {
        let value = self.payload.try_borrow_mut().map_err(|_| {
            object_error(format!(
                "Rust object '{}' is already borrowed",
                self.rust_type_name
            ))
        })?;
        RefMut::filter_map(value, |value| value.downcast_mut::<T>())
            .map_err(|_| wrong_object_type(type_name::<T>()))
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
