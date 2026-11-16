//! Keep existing stateful contract tests active in both ownership configurations.
#![allow(dead_code, unused_imports)]
#[cfg(feature = "send")]
pub use std::sync::Arc as Rc;
#[cfg(not(feature = "send"))]
pub use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
#[cfg(feature = "send")]
pub struct RefCell<T>(std::sync::Mutex<T>);
#[cfg(feature = "send")]
impl<T> RefCell<T> {
    pub fn new(value: T) -> Self {
        Self(std::sync::Mutex::new(value))
    }
    pub fn borrow(&self) -> std::sync::MutexGuard<'_, T> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub fn borrow_mut(&self) -> std::sync::MutexGuard<'_, T> {
        self.borrow()
    }
}
#[cfg(feature = "send")]
pub struct Cell<T: Copy>(RefCell<T>);
#[cfg(feature = "send")]
impl<T: Copy> Cell<T> {
    pub fn new(value: T) -> Self {
        Self(RefCell::new(value))
    }
    pub fn get(&self) -> T {
        *self.0.borrow()
    }
    pub fn set(&self, value: T) {
        *self.0.borrow_mut() = value;
    }
}
