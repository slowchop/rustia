//! Callback ownership follows the existing generation-safe heap function lifetime.

use super::*;

impl HostFunctionBinding {
    fn name(&self) -> &str {
        match self {
            Self::Global { name, .. } => name,
            Self::Module { namespace, .. } => namespace,
        }
    }
}

impl Vm {
    /// Allocates an unrooted host function. Persistent installation is explicit.
    /// Low-level callers must keep the returned value reachable across collection.
    pub fn register_host_function(&mut self, op: HostFunctionOp) -> Result<Value> {
        self.validate_hardened_host_function(&op)?;
        validate_registered_op(self.profile, &op.name, op.fuel_cost, op.capability)?;
        self.ensure_memory_growth(VmCell::host_function_live_bytes(&op), &[])?;
        let handle = self
            .heap
            .alloc_host_function(op, self.globals)
            .ok_or_else(|| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
        Ok(Value::function(handle))
    }

    /// Creates a disposable rooted callback, admitting its function and root atomically.
    pub fn create_retained_function(&mut self, op: HostFunctionOp) -> Result<RetainedValue> {
        if self.settings.guest_surface.is_hardened_leaf() {
            return Err(Error::new(ErrorKind::HardenedLeafViolation {
                feature: "live function construction",
            })
            .without_source_location());
        }
        self.ensure_healthy()?;
        validate_registered_op(self.profile, &op.name, op.fuel_cost, op.capability)?;
        self.ensure_memory_growth(
            VmCell::host_function_live_bytes(&op).saturating_add(external_root_node_live_bytes()),
            &[],
        )?;
        let handle = self
            .heap
            .alloc_host_function(op, self.globals)
            .ok_or_else(|| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
        match self.retain_value(Value::function(handle)) {
            Ok(value) => Ok(value),
            Err(error) => {
                self.heap.discard_host_function(handle);
                Err(error)
            }
        }
    }

    pub fn install_host_function(&mut self, op: HostFunctionOp) -> Result<()> {
        self.install_host_functions(vec![op])
    }

    /// Atomically installs persistent host functions as globals.
    pub fn install_host_functions(&mut self, functions: Vec<HostFunctionOp>) -> Result<()> {
        self.ensure_host_configuration_idle()?;
        if self.guest_environment_sealed {
            return Err(Error::new(ErrorKind::GuestEnvironmentSealed).without_source_location());
        }
        for function in &functions {
            self.validate_hardened_host_function(function)?;
            validate_registered_op(
                self.profile,
                &function.name,
                function.fuel_cost,
                function.capability,
            )?;
        }
        self.host_function_bindings
            .try_reserve(functions.len())
            .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
        let checkpoint = self.host_function_bindings.len();
        self.begin_guest_transaction()?;
        let install = (|| {
            for function in functions {
                let name = function.name.clone();
                let value = self.register_host_function(function.clone())?;
                self.set_host_configuration_global(&name, value)?;
                self.host_function_bindings
                    .push(HostFunctionBinding::Global { name, function });
                self.mark_control_memory_dirty();
            }
            self.ensure_memory_growth(0, &[])?;
            self.commit_guest_transaction()
        })();
        self.finish_host_install(checkpoint, install)
    }

    /// Atomically installs a persistent namespace rebuilt by decision reset.
    pub fn install_host_module(
        &mut self,
        namespace: impl Into<String>,
        functions: Vec<HostFunctionOp>,
    ) -> Result<()> {
        self.ensure_host_configuration_idle()?;
        if self.guest_environment_sealed {
            return Err(Error::new(ErrorKind::GuestEnvironmentSealed).without_source_location());
        }
        let namespace = namespace.into();
        for function in &functions {
            self.validate_hardened_host_function(function)?;
            validate_registered_op(
                self.profile,
                &format!("{namespace}.{}", function.name),
                function.fuel_cost,
                function.capability,
            )?;
        }
        self.host_function_bindings
            .try_reserve(1)
            .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
        let checkpoint = self.host_function_bindings.len();
        self.begin_guest_transaction()?;
        let install = (|| {
            let table = self.empty_table();
            // Each function admission may collect. Keep the namespace alive until
            // its global binding has been installed, including under quota pressure.
            let table_root = self.retain_value(table)?;
            for function in &functions {
                let value = self.register_host_function(function.clone())?;
                let key = self.string(&function.name);
                self.raw_table_set_value(table, key, value)?;
            }
            if self.settings.guest_surface.is_hardened_leaf() {
                let handle = table.as_table().expect("new host module must be a table");
                self.heap
                    .table_freeze(handle)
                    .expect("new host module must be valid");
            }
            self.set_host_configuration_global(&namespace, table)?;
            drop(table_root);
            self.host_function_bindings
                .push(HostFunctionBinding::Module {
                    namespace,
                    functions,
                });
            self.mark_control_memory_dirty();
            self.ensure_memory_growth(0, &[])?;
            self.commit_guest_transaction()
        })();
        self.finish_host_install(checkpoint, install)
    }

    fn finish_host_install(&mut self, checkpoint: usize, result: Result<()>) -> Result<()> {
        if result.is_err() {
            self.host_function_bindings.truncate(checkpoint);
            self.rollback_guest_transaction();
        } else {
            // The newest registration owns each namespace. Old guest functions still
            // own their independent definitions until their final reference disappears.
            for index in (0..self.host_function_bindings.len()).rev() {
                if self.host_function_bindings[index + 1..]
                    .iter()
                    .any(|later| later.name() == self.host_function_bindings[index].name())
                {
                    self.host_function_bindings.remove(index);
                }
            }
        }
        self.mark_control_memory_dirty();
        result
    }

    pub(super) fn ensure_host_configuration_idle(&self) -> Result<()> {
        self.ensure_healthy()?;
        if !self.call_stack.is_empty()
            || !self.running_coroutines.is_empty()
            || !self.running_coroutine_stack_bases.is_empty()
            || !self.active_coroutine_frames.is_empty()
            || !self.yield_collectors.is_empty()
            || !self.guest_transactions.is_empty()
            || self.non_yieldable_depth != 0
        {
            return Err(
                Error::new(ErrorKind::HostConfigurationWhileRunning).without_source_location()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn callback(name: String, n: usize) -> HostFunctionOp {
        HostFunctionOp::new(
            name,
            1,
            OpCapability::Pure,
            Arc::new(move |_, _| Ok(vec![Value::number(n as f64)])),
        )
    }

    #[test]
    fn persistent_configuration_rejects_active_savepoints() {
        let mut vm = Vm::new(Profile::LuauCompat);
        vm.begin_guest_transaction().unwrap();
        assert!(matches!(
            vm.install_host_function(callback("f".into(), 1))
                .unwrap_err()
                .kind,
            ErrorKind::HostConfigurationWhileRunning
        ));
        assert!(vm.get_global_value("f").unwrap().is_nil());
        vm.rollback_guest_transaction();
        vm.reset_decision().unwrap();
        assert!(vm.get_global_value("f").unwrap().is_nil());
    }

    #[test]
    fn namespace_admission_under_quota_pressure_never_publishes_collected_functions() {
        let mut failed = 0;
        let mut succeeded = 0;
        for extra in (0..40_000).step_by(512) {
            let mut vm = Vm::new(Profile::LuauCompat);
            let base = vm.current_live_bytes();
            let snapshot = vm.heap.transaction_snapshot_live_bytes()
                + guest_transaction_snapshot_live_bytes(&vm);
            vm.settings.memory_limit_bytes = Some(base + snapshot + extra);
            let functions = (0..24).map(|n| callback(format!("f{n}"), n)).collect();
            match vm.install_host_module("api", functions) {
                Err(error) => {
                    assert!(matches!(error.kind, ErrorKind::OutOfMemory), "{error}");
                    assert!(vm.get_global_value("api").unwrap().is_nil());
                    assert!(vm.host_function_bindings.is_empty());
                    failed += 1;
                }
                Ok(()) => {
                    // Remove the limit only after installation to inspect the complete result.
                    vm.settings.memory_limit_bytes = None;
                    let table = vm.get_global_value("api").unwrap();
                    for n in 0..24 {
                        let key = vm.string(format!("f{n}"));
                        let function = vm.raw_table_get_value(table, key).unwrap();
                        let result = vm.call_retained_function(function, &[]).unwrap();
                        assert_eq!(result[0].as_number(), Some(n as f64));
                    }
                    succeeded += 1;
                }
            }
        }
        assert!(failed > 0 && succeeded > 0);
    }
}
