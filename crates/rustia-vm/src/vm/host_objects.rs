//! Persistent type configuration and disposable opaque Rust instances.

use super::*;
use crate::host_object::{object_error, wrong_object_type};
use crate::{HostObjectData, HostObjectOwner, HostObjectPayload, HostObjectTypeDefinition};
use std::any::{type_name, TypeId};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub(super) struct HostObjectTypeBinding {
    pub(super) definition: HostObjectTypeDefinition,
    pub(super) metatable: Option<u32>,
}

impl Vm {
    /// Replaces the persistent definition for this Rust type atomically.
    /// Existing instances retain their original method table.
    pub fn register_host_object_type(
        &mut self,
        definition: HostObjectTypeDefinition,
    ) -> Result<()> {
        self.ensure_host_configuration_idle()?;
        self.ensure_host_objects_allowed()?;
        if definition.name.is_empty() || definition.rust_type_name.is_empty() {
            return Err(object_error(
                "Rust object type names must not be empty".to_owned(),
            ));
        }
        for (index, method) in definition.methods.iter().enumerate() {
            if definition.methods[..index]
                .iter()
                .any(|previous| previous.name == method.name)
            {
                return Err(object_error(format!(
                    "duplicate Rust object method '{}.{}'",
                    definition.name, method.name
                )));
            }
            validate_registered_op(
                self.profile,
                &format!("{}.{}", definition.name, method.name),
                method.fuel_cost,
                method.capability,
            )?;
        }
        for (name, hook) in definition.hooks.iter() {
            validate_registered_op(
                self.profile,
                &format!("{}.{}", definition.name, name),
                hook.fuel_cost,
                hook.capability,
            )?;
        }
        self.host_object_types
            .try_reserve(1)
            .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
        let checkpoint = self.host_object_types.len();
        self.begin_guest_transaction()?;
        let result = (|| {
            let metatable = self.build_host_object_metatable(&definition, true)?;
            self.host_object_types.push(HostObjectTypeBinding {
                definition,
                metatable: Some(metatable),
            });
            self.mark_control_memory_dirty();
            self.ensure_memory_growth(0, &[])?;
            self.commit_guest_transaction()
        })();
        if result.is_err() {
            self.host_object_types.truncate(checkpoint);
            self.rollback_guest_transaction();
        } else {
            let type_id = self
                .host_object_types
                .last()
                .expect("installed object definition")
                .definition
                .rust_type_id;
            for index in (0..checkpoint).rev() {
                if self.host_object_types[index].definition.rust_type_id == type_id {
                    self.host_object_types.remove(index);
                }
            }
        }
        self.mark_control_memory_dirty();
        result
    }

    /// Checks construction policy and registration without allocating guest state.
    pub fn validate_host_object_creation(&self, rust_type_id: TypeId) -> Result<()> {
        self.ensure_healthy()?;
        self.ensure_host_objects_allowed()?;
        if self
            .host_object_types
            .iter()
            .any(|binding| binding.definition.rust_type_id == rust_type_id)
        {
            Ok(())
        } else {
            Err(object_error(
                "Rust object type is not registered in this VM".to_owned(),
            ))
        }
    }

    /// Creates an unrooted callback result. The caller must keep it reachable
    /// across any following allocation or collection.
    pub fn create_host_object(
        &mut self,
        rust_type_id: TypeId,
        rust_type_name: &'static str,
        payload: Box<HostObjectPayload>,
    ) -> Result<Value> {
        self.validate_host_object_creation(rust_type_id)?;
        if payload.as_ref().type_id() != rust_type_id {
            return Err(wrong_object_type(rust_type_name));
        }
        self.ensure_memory_growth(VmCell::host_object_live_bytes(), &[])?;
        let metatable = self.host_object_metatable(rust_type_id)?;
        let data = Shared::new(HostObjectData::new(rust_type_name, payload));
        let handle = self
            .heap
            .alloc_host_object(metatable, data)
            .ok_or_else(|| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
        Ok(Value::userdata(handle))
    }

    /// Admits an instance and its public root atomically.
    #[cfg(not(feature = "send"))]
    pub fn create_retained_host_object<T: 'static>(&mut self, payload: T) -> Result<RetainedValue> {
        self.create_retained_host_object_boxed(
            TypeId::of::<T>(),
            type_name::<T>(),
            Box::new(payload),
        )
    }

    /// Admits a transferable instance and its public root atomically.
    #[cfg(feature = "send")]
    pub fn create_retained_host_object<T: Send + Sync + 'static>(
        &mut self,
        payload: T,
    ) -> Result<RetainedValue> {
        self.create_retained_host_object_boxed(
            TypeId::of::<T>(),
            type_name::<T>(),
            Box::new(payload),
        )
    }

    fn create_retained_host_object_boxed(
        &mut self,
        rust_type_id: TypeId,
        rust_type_name: &'static str,
        payload: Box<HostObjectPayload>,
    ) -> Result<RetainedValue> {
        self.validate_host_object_creation(rust_type_id)?;
        self.ensure_memory_growth(
            VmCell::host_object_live_bytes().saturating_add(external_root_node_live_bytes()),
            &[],
        )?;
        let value = self.create_host_object(rust_type_id, rust_type_name, payload)?;
        match self.retain_value(value) {
            Ok(retained) => Ok(retained),
            Err(error) => {
                self.heap
                    .discard_host_object(value.as_userdata().expect("new host object is userdata"));
                Err(error)
            }
        }
    }

    /// Performs the Rust receiver check before callback argument conversions.
    /// No collector borrow escapes; the returned owner is VM-local.
    pub fn checked_host_object(
        &self,
        value: Value,
        expected_type: TypeId,
        expected_name: &'static str,
    ) -> Result<HostObjectOwner> {
        let data = value
            .as_userdata()
            .and_then(|handle| self.heap.host_object(handle))
            .ok_or_else(|| wrong_object_type(expected_name))?;
        if data.rust_type_id() != expected_type {
            return Err(wrong_object_type(expected_name));
        }
        Ok(data)
    }

    /// Returns the opaque payload's Rust type, or `None` for another value.
    /// This performs no guest allocation or callback.
    pub fn host_object_type_id(&mut self, value: Value) -> Option<TypeId> {
        value
            .as_userdata()
            .and_then(|handle| self.heap.host_object(handle))
            .map(|data| data.rust_type_id())
    }

    /// Finds a method in this instance's original frozen method table.
    pub fn host_object_method(&mut self, value: Value, name: &str) -> Result<Value> {
        let handle = value
            .as_userdata()
            .filter(|handle| self.heap.host_object(*handle).is_some())
            .ok_or_else(|| wrong_object_type("userdata"))?;
        let key = self
            .heap
            .find_string_atom_bytes(name.as_bytes())
            .map(Value::string_atom)
            .ok_or_else(|| object_error(format!("Rust object has no method '{name}'")))?;
        self.host_object_registered_method(Value::userdata(handle), key)
            .ok_or_else(|| object_error(format!("Rust object has no method '{name}'")))
    }

    pub(super) fn host_object_registered_method(
        &mut self,
        value: Value,
        key: Value,
    ) -> Option<Value> {
        let handle = value.as_userdata()?;
        self.heap.host_object(handle)?;
        let metatable = self.heap.userdata_metatable(handle)??;
        // A dynamic index hook replaces __index. Its original method table is
        // retained separately in the same protected, frozen metatable.
        let methods = ["__rustia_methods", "__index"]
            .into_iter()
            .find_map(|name| {
                let atom = self.heap.find_string_atom_bytes(name.as_bytes())?;
                self.heap
                    .table_get(metatable, Value::string_atom(atom))?
                    .as_table()
            })?;
        self.heap
            .table_get(methods, key)
            .filter(|value| value.kind() == ValueKind::Function)
    }

    pub(super) fn reject_host_object_method_write(
        &mut self,
        value: Value,
        key: Value,
    ) -> Result<()> {
        if self.host_object_registered_method(value, key).is_some() {
            return Err(object_error(
                "cannot replace a registered Rust object method".to_owned(),
            ));
        }
        Ok(())
    }

    fn ensure_host_objects_allowed(&self) -> Result<()> {
        if self.settings.guest_surface.is_hardened_leaf() {
            Err(Error::new(ErrorKind::HardenedLeafViolation {
                feature: "Rust object construction",
            })
            .without_source_location())
        } else {
            Ok(())
        }
    }

    fn host_object_metatable(&self, rust_type_id: TypeId) -> Result<u32> {
        self.host_object_types
            .iter()
            .find(|binding| binding.definition.rust_type_id == rust_type_id)
            .and_then(|binding| binding.metatable)
            .ok_or_else(|| object_error("Rust object type is not registered in this VM".to_owned()))
    }

    fn build_host_object_metatable(
        &mut self,
        definition: &HostObjectTypeDefinition,
        admit: bool,
    ) -> Result<u32> {
        let metatable = self.empty_table();
        let root = if admit {
            Some(self.retain_value(metatable)?)
        } else {
            None
        };
        let methods = self.empty_table();
        let index_key = self.string("__index");
        self.heap
            .table_set(metatable.as_table().expect("new table"), index_key, methods)
            .expect("new metatable accepts its index");
        if definition.hooks.index.is_some() {
            let methods_key = self.string("__rustia_methods");
            self.heap
                .table_set(
                    metatable.as_table().expect("new table"),
                    methods_key,
                    methods,
                )
                .expect("new metatable accepts its method table");
        }
        let protected_key = self.string("__metatable");
        self.heap
            .table_set(
                metatable.as_table().expect("new table"),
                protected_key,
                Value::boolean(false),
            )
            .expect("new metatable accepts protection");
        for method in &definition.methods {
            let function = if admit {
                self.register_host_function(method.clone())?
            } else {
                // Reset rebuilds already-admitted immutable configuration without
                // collection or a temporary root/quota charge, like host modules.
                Value::function(
                    self.heap
                        .alloc_host_function(method.clone(), self.globals)
                        .expect("persisted method handle must remain representable"),
                )
            };
            let key = self.string(&method.name);
            self.heap
                .table_set(methods.as_table().expect("new table"), key, function)
                .expect("new method table accepts its function");
        }
        for (name, hook) in definition.hooks.iter() {
            let function = if admit {
                self.register_host_function(hook.clone())?
            } else {
                Value::function(
                    self.heap
                        .alloc_host_function(hook.clone(), self.globals)
                        .expect("persisted hook handle must remain representable"),
                )
            };
            let key = self.string(name);
            self.heap
                .table_set(metatable.as_table().expect("new table"), key, function)
                .expect("new metatable accepts its hook");
        }
        self.heap
            .table_freeze(methods.as_table().expect("new table"));
        self.heap
            .table_freeze(metatable.as_table().expect("new table"));
        drop(root);
        Ok(metatable.as_table().expect("new table"))
    }

    pub(super) fn restore_host_object_types(&mut self) {
        for binding in &mut self.host_object_types {
            binding.metatable = None;
        }
        for index in 0..self.host_object_types.len() {
            let definition = self.host_object_types[index].definition.clone();
            let metatable = self
                .build_host_object_metatable(&definition, false)
                .expect("persisted object methods must remain representable");
            self.host_object_types[index].metatable = Some(metatable);
        }
    }
}
