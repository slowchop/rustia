//! Feature-selected owners. Default embedding keeps its local types and bounds.

#[cfg(not(feature = "send"))]
pub(crate) use std::rc::{Rc as Shared, Weak};
#[cfg(feature = "send")]
pub(crate) use std::sync::{Arc as Shared, Weak};

#[cfg(not(feature = "send"))]
pub(crate) type ServiceCell<T> = std::cell::RefCell<T>;
#[cfg(feature = "send")]
pub(crate) type ServiceCell<T> = std::sync::Mutex<T>;
#[cfg(not(feature = "send"))]
pub(crate) type ServiceGuard<'a, T> = std::cell::RefMut<'a, T>;
#[cfg(feature = "send")]
pub(crate) type ServiceGuard<'a, T> = std::sync::MutexGuard<'a, T>;

pub(crate) fn borrow_service<'a, T: ?Sized>(
    cell: &'a ServiceCell<T>,
    name: &str,
) -> crate::Result<ServiceGuard<'a, T>> {
    #[cfg(not(feature = "send"))]
    {
        let _ = name;
        // Preserve the local mode's RefCell callback panic boundary.
        Ok(cell.borrow_mut())
    }
    #[cfg(feature = "send")]
    {
        match cell.try_lock() {
            Ok(guard) => Ok(guard),
            // Guard poisoning is not VM poisoning. The VM's existing host
            // panic boundary remains authoritative, including reset recovery.
            Err(std::sync::TryLockError::Poisoned(error)) => Ok(error.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => {
                Err(crate::Error::new(crate::ErrorKind::RuntimeError {
                    message: format!("host service '{name}' is already in use"),
                })
                .without_source_location())
            }
        }
    }
}

#[cfg(not(feature = "send"))]
pub(crate) type RegistryCell<T> = std::cell::RefCell<T>;

/// Registry critical sections contain only owned scalar bookkeeping. They never
/// invoke guest/host code or drop retained leases; concurrent last-handle drops
/// may wait for these short sections, rather than losing a root on contention.
#[cfg(feature = "send")]
pub(crate) struct RegistryCell<T>(std::sync::Mutex<T>);

#[cfg(feature = "send")]
impl<T> RegistryCell<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(std::sync::Mutex::new(value))
    }

    pub(crate) fn borrow(&self) -> std::sync::MutexGuard<'_, T> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn borrow_mut(&self) -> std::sync::MutexGuard<'_, T> {
        self.borrow()
    }
}

#[cfg(all(test, feature = "send"))]
mod send_tests {
    use super::*;
    use crate::{HostObjectOwner, RetainedValue, Value, Vm};

    fn assert_send<T: Send>() {}

    #[test]
    fn vm_roots_and_host_objects_are_transferable() {
        assert_send::<Vm>();
        assert_send::<RetainedValue>();
        assert_send::<HostObjectOwner>();

        let mut vm = Vm::new(crate::Profile::ReproducibleStandard);
        let table = vm.create_retained_table().unwrap();
        let (mut vm, table) = std::thread::spawn(move || {
            let value = table.value(&vm).unwrap();
            vm.raw_table_set_value(value, Value::number(1.0), Value::number(7.0))
                .unwrap();
            (vm, table)
        })
        .join()
        .unwrap();
        let value = table.value(&vm).unwrap();
        assert_eq!(
            vm.raw_table_get_value(value, Value::number(1.0))
                .unwrap()
                .as_number(),
            Some(7.0)
        );
    }

    #[test]
    fn service_conflict_is_a_structured_error_and_release_allows_reuse() {
        let cell = ServiceCell::new(7);
        let guard = borrow_service(&cell, "test").unwrap();
        let error = borrow_service(&cell, "test").unwrap_err();
        assert!(matches!(error.kind, crate::ErrorKind::RuntimeError { .. }));
        drop(guard);
        *borrow_service(&cell, "test").unwrap() = 9;
        assert_eq!(*borrow_service(&cell, "test").unwrap(), 9);
    }
}
