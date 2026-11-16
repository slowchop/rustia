//! Host-created compiled entries without executing a guest frame.
use super::*;

#[cfg(test)]
#[path = "program_entry_tests.rs"]
mod tests;

impl Vm {
    /// Creates a rooted callable for a program's main function in an exact environment.
    ///
    /// This performs no guest execution and creates no execution budget or call
    /// frame. Calling the result uses the ordinary callable path, including its
    /// caller's fuel/depth and bytecode validation. Main parameters, varargs,
    /// nested prototypes and source metadata are retained without a wrapper.
    /// The entry inherits the current module cohort; no module is resolved or
    /// cached. Host-owned module policy remains outside this primitive.
    ///
    /// The environment must be a live table root from this VM. Program, function
    /// and root growth are quota-admitted before publication. An enclosing
    /// installation rollback invalidates the new root. Hardened leaves reject
    /// this persistent-function construction.
    pub fn retain_program_entry(
        &mut self,
        program: Program,
        environment: &RetainedValue,
    ) -> Result<RetainedValue> {
        self.ensure_healthy()?;
        if self.settings.guest_surface.is_hardened_leaf() {
            return Err(Error::new(ErrorKind::HardenedLeafViolation {
                feature: "retained program entry",
            })
            .without_source_location());
        }
        let environment = environment.value(self)?.as_table().ok_or_else(|| {
            Error::new(ErrorKind::TypeError { expected: "table" }).without_source_location()
        })?;
        if !self.heap.has_table(environment) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "chunk environment table handle is missing",
            })
            .without_source_location());
        }
        let root_growth =
            external_root_node_live_bytes().saturating_add(if self.guest_transactions.is_empty() {
                0
            } else {
                size_of::<u64>()
            });
        let (entry, _) = self.create_program_entry(
            program,
            environment,
            self.current_module_cohort(),
            root_growth,
        )?;
        self.retain_value(entry)
    }

    /// Shared construction for loadstring, builtin require and trusted hosts.
    /// Callers keep the environment rooted until construction completes.
    pub(super) fn create_program_entry(
        &mut self,
        mut program: Program,
        environment: u32,
        cohort: u32,
        additional: usize,
    ) -> Result<(Value, u32)> {
        self.preflight_dynamic_program_construction(&program)?;
        self.admit_diagnostic_source(&mut program);
        let loaded = LoadedProgram::new(program);
        self.ensure_loaded_program_growth(
            &loaded,
            VmCell::dynamic_main_function_live_bytes().saturating_add(additional),
        )?;
        // No collecting operation separates program insertion and its function.
        let program = self
            .dynamic_programs
            .insert(loaded)
            .ok_or_else(|| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
        let function = self
            .heap
            .alloc_dynamic_main_function_in_cohort(program, environment, cohort)
            .ok_or_else(|| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
        Ok((Value::function(function), program))
    }
}
