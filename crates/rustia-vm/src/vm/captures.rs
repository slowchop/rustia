//! Rooted inspection and idle capture edits for embedding-owned state migration.
use super::*;

#[cfg(test)]
#[path = "captures_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InspectedFunctionKind {
    Script,
    Native,
    Host,
}

#[derive(Clone, Debug)]
pub struct FunctionInspection {
    pub kind: InspectedFunctionKind,
    pub name: Option<String>,
    pub source: Option<String>,
    pub line_defined: Option<u32>,
    pub capture_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureStorage {
    Direct,
    SharedCell,
}

pub struct RetainedCapture {
    pub index: usize,
    pub name: Option<String>,
    pub storage: CaptureStorage,
    pub value: RetainedValue,
}

fn missing(reason: &'static str) -> Error {
    Error::new(ErrorKind::InvalidProgram { reason }).without_source_location()
}
fn capture_error(message: &str) -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: message.into(),
    })
    .without_source_location()
}

impl Vm {
    fn inspected_function(&self, function: &RetainedValue) -> Result<(u32, FunctionKind)> {
        let handle = function.value(self)?.as_function().ok_or_else(|| {
            Error::new(ErrorKind::TypeError {
                expected: "function",
            })
        })?;
        Ok((
            handle,
            self.heap
                .function_kind(handle)
                .ok_or_else(|| missing("function handle is missing"))?,
        ))
    }
    fn capture_prototype(&self, kind: FunctionKind) -> Result<(Arc<Program>, Option<u32>)> {
        match kind {
            FunctionKind::Bytecode {
                program: Some(program),
                prototype,
            } => Ok((self.dynamic_program(program)?, Some(prototype))),
            FunctionKind::DynamicMain { program } => Ok((self.dynamic_program(program)?, None)),
            _ => Err(capture_error(
                "capture inspection requires a retained script function",
            )),
        }
    }
    pub fn retained_function_info(&self, function: &RetainedValue) -> Result<FunctionInspection> {
        let (handle, kind) = self.inspected_function(function)?;
        match kind {
            FunctionKind::Host => Ok(FunctionInspection {
                kind: InspectedFunctionKind::Host,
                name: Some(self.host_function(handle)?.name.clone()),
                source: None,
                line_defined: None,
                capture_count: 0,
            }),
            FunctionKind::Native(native) => Ok(FunctionInspection {
                kind: InspectedFunctionKind::Native,
                name: Some(debug_info_native_target(function.value(self)?, native).name),
                source: None,
                line_defined: None,
                capture_count: 0,
            }),
            _ => {
                let (program, prototype) = self.capture_prototype(kind)?;
                let function = match prototype {
                    Some(index) => program
                        .functions
                        .get(index as usize)
                        .ok_or_else(|| missing("function prototype is missing"))?,
                    None => &program.main,
                };
                Ok(FunctionInspection {
                    kind: InspectedFunctionKind::Script,
                    name: function.debug_name.clone(),
                    source: program.source_name.as_deref().map(str::to_owned),
                    line_defined: function.debug_line_defined,
                    capture_count: function.upvalues.len(),
                })
            }
        }
    }
    /// Reads one zero-based capture without executing guest code.
    pub fn retained_function_capture(
        &mut self,
        function: &RetainedValue,
        index: usize,
    ) -> Result<Option<RetainedCapture>> {
        let (handle, kind) = self.inspected_function(function)?;
        let (program, prototype) = self.capture_prototype(kind)?;
        let metadata = match prototype {
            Some(index) => program
                .functions
                .get(index as usize)
                .ok_or_else(|| missing("function prototype is missing"))?,
            None => &program.main,
        };
        if index >= metadata.upvalues.len() {
            return Ok(None);
        }
        let capture = self
            .heap
            .function_capture(handle, index)
            .ok_or_else(|| missing("function capture is missing"))?;
        let value = self
            .heap
            .capture_get(capture)
            .ok_or_else(|| missing("function capture cell is missing"))?;
        let value = self.retain_value(value)?;
        Ok(Some(RetainedCapture {
            index,
            name: metadata.upvalue_name(index).map(str::to_owned),
            storage: if capture.cell().is_some() {
                CaptureStorage::SharedCell
            } else {
                CaptureStorage::Direct
            },
            value,
        }))
    }
    /// Edits one zero-based capture while no guest call is running. Direct
    /// captures affect this closure; shared cells affect every closure alias.
    pub fn set_retained_function_capture(
        &mut self,
        function: &RetainedValue,
        index: usize,
        value: &RetainedValue,
    ) -> Result<()> {
        self.ensure_idle_capture_edit()?;
        let (handle, kind) = self.inspected_function(function)?;
        let _ = self.capture_prototype(kind)?;
        if self.heap.function_capture(handle, index).is_none() {
            return Err(capture_error("capture index is out of bounds"));
        }
        let value = value.value(self)?;
        self.ensure_memory_growth(0, &[function.value(self)?, value])?;
        self.heap
            .set_function_capture(handle, index, value)
            .ok_or_else(|| missing("function capture is missing"))
    }

    /// Checks the common entry conditions for compiled capture calls and idle
    /// capture edits before an external backend runs.
    #[doc(hidden)]
    pub fn ensure_idle_capture_edit(&self) -> Result<()> {
        self.ensure_inspection_mutation_allowed()?;
        if !self.call_stack.is_empty()
            || !self.running_coroutines.is_empty()
            || !self.running_coroutine_stack_bases.is_empty()
            || !self.running_coroutine_active_frame_bases.is_empty()
            || !self.active_coroutine_frames.is_empty()
            || !self.yield_collectors.is_empty()
            || self.non_yieldable_depth != 0
        {
            return Err(capture_error("capture mutation requires an idle VM"));
        }
        Ok(())
    }

    /// Check an existing closure against the compiler's numeric interpreter
    /// entry before binding an executable to it. The compiler and embedder
    /// identify the frozen source and executable separately.
    #[doc(hidden)]
    pub fn retained_numeric_call_matches(
        &self,
        function: &RetainedValue,
        expected: &Program,
    ) -> Result<bool> {
        let (_, kind) = self.inspected_function(function)?;
        let FunctionKind::Bytecode {
            program: Some(program),
            prototype,
        } = kind
        else {
            return Ok(false);
        };
        let program = self.dynamic_program(program)?;
        let (Some(actual), [expected_function]) = (
            program.functions.get(prototype as usize),
            expected.functions.as_slice(),
        ) else {
            return Ok(false);
        };
        Ok(program.source_name == expected.source_name
            && program.logical_units().is_none()
            && expected.logical_units().is_none()
            && actual.code == expected_function.code
            && actual.max_registers == expected_function.max_registers
            && actual.parameter_count == expected_function.parameter_count
            && actual.is_vararg == expected_function.is_vararg
            && actual.upvalues == expected_function.upvalues
            && actual.upvalue_names == expected_function.upvalue_names
            && actual.captured_registers == expected_function.captured_registers
            && actual.parameter_types == expected_function.parameter_types
            && actual.code.iter().all(|step| {
                if step.op != Opcode::LoadConst {
                    return true;
                }
                let index = usize::from(step.c);
                matches!(
                    (program.constants().get(index), expected.constants().get(index)),
                    (Some(Constant::Number(left)), Some(Constant::Number(right)))
                        if left.to_bits() == right.to_bits(),
                )
            }))
    }

    /// Run an externally built, trusted numeric backend at the idle VM boundary.
    /// The compiler must bind the backend to this exact interpreter closure.
    /// The caller passes the same execution budget it would use for the
    /// interpreter. `None` leaves that budget for the ordinary call.
    #[doc(hidden)]
    pub fn try_run_compiled_numeric_capture(
        &mut self,
        function: &RetainedValue,
        capture_name: &str,
        parameter: Value,
        execution: &mut VmExecution,
        invoke: impl FnOnce(f64, f64) -> Result<(f64, f64)>,
    ) -> Result<Option<Value>> {
        let settings = &self.settings;
        if !matches!(
            self.profile,
            Profile::ReproducibleStandard | Profile::LuauCompat
        ) || settings.guest_surface.is_hardened_leaf()
            || settings.enable_conformance_native_type_checks
            || settings.recursion_limit == 0
            || settings.enable_memory_tracking
            || settings.enable_opcode_census
            || settings.enable_conformance_coverage
            || settings.enable_conformance_luau_call_limit
            || self.guest_transaction_active()
        {
            return Ok(None);
        }
        let Some(parameter) = compiled_number(parameter) else {
            return Ok(None);
        };
        self.ensure_idle_capture_edit()?;
        let (handle, kind) = self.inspected_function(function)?;
        let (program, prototype) = self.capture_prototype(kind)?;
        let Some(closure) = prototype.and_then(|index| program.functions.get(index as usize))
        else {
            return Ok(None);
        };
        let Some(return_pc) = closure
            .code
            .iter()
            .position(|step| step.op == Opcode::Return)
        else {
            return Ok(None);
        };
        // The compiler may append an unreachable empty return. Admitted
        // operations have an identity clock schedule and cannot allocate
        // guest objects or dispatch calls.
        if closure.parameter_count != 1
            || closure.is_vararg
            || !closure.captured_registers.is_empty()
            || closure.upvalues.len() != 1
            || closure.upvalue_name(0) != Some(capture_name)
            || program.logical_units().is_some()
            || closure.code[return_pc].b != 1
            || closure.code[..return_pc].iter().any(|step| match step.op {
                Opcode::LoadConst => !matches!(
                    program.constants().get(usize::from(step.c)),
                    Some(Constant::Number(_))
                ),
                Opcode::Move
                | Opcode::GetUpvalue
                | Opcode::SetUpvalue
                | Opcode::ClearLocalCell
                | Opcode::Add
                | Opcode::Sub
                | Opcode::Mul
                | Opcode::Div => false,
                _ => true,
            })
        {
            return Ok(None);
        }
        let Some(next_clock) = self.executed_instructions.checked_add(return_pc as u64 + 1) else {
            return Ok(None);
        };
        // Keep collection at the original instruction boundary. In particular
        // an external backend cannot reproduce the interpreter's frame roots.
        if next_clock >= self.next_weak_collection_instruction {
            return Ok(None);
        }
        let cell = self
            .heap
            .function_capture(handle, 0)
            .ok_or_else(|| missing("compiled function capture is missing"))?;
        let current = self
            .heap
            .capture_get(cell)
            .ok_or_else(|| missing("compiled function capture cell is missing"))?;
        let Some(current) = compiled_number(current) else {
            return Ok(None);
        };
        // Let the interpreter reproduce the exact error and partial state
        // when the remainder of this invocation cannot cover the trace.
        let remaining = if let Some(remaining) = execution.fuel_remaining() {
            let cost = closure.code[..=return_pc]
                .iter()
                .try_fold(0_u64, |cost, &step| {
                    Ok::<_, Error>(cost.saturating_add(self.instruction_fuel_cost(step)?))
                })?;
            let Some(remaining) = remaining.checked_sub(cost) else {
                return Ok(None);
            };
            Some(remaining)
        } else {
            None
        };
        if let Some(limit) = self.settings.memory_limit_bytes {
            // Quota checks during this trace see one call frame, its register
            // root vector, and an empty continuation-driver slot. The trace
            // cannot grow the heap or install any extra roots. If this bound
            // would collect, leave the normal frame and roots to the VM.
            let frame_bytes = size_of::<CallStackFrame>()
                .saturating_add(size_of::<Vec<CoroutineFrame>>())
                .saturating_add(closure.debug_name.as_ref().map_or(0, String::len))
                .saturating_add(closure.max_registers.saturating_mul(size_of::<Value>()));
            if self.current_live_bytes().saturating_add(frame_bytes) > limit {
                return Ok(None);
            }
        }
        let (next, result) = invoke(current, parameter)?;
        if next.to_bits() != current.to_bits() {
            self.heap
                .set_function_capture(handle, 0, Value::number(next))
                .ok_or_else(|| missing("compiled function capture is missing"))?;
        }
        execution.fuel = remaining;
        self.executed_instructions = next_clock;
        self.prune_unreachable_coroutines_if_idle();
        self.release_host_error_traceback();
        Ok(Some(Value::number(result)))
    }
    pub fn retained_function_environment(
        &mut self,
        function: &RetainedValue,
    ) -> Result<RetainedValue> {
        let (handle, _) = self.inspected_function(function)?;
        let environment = self
            .heap
            .function_environment(handle)
            .ok_or_else(|| missing("function environment is missing"))?;
        self.retain_value(Value::table(environment))
    }
    fn retained_table_handle(&mut self, table: &RetainedValue) -> Result<u32> {
        let handle = table
            .value(self)?
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        if !self.heap.has_table(handle) {
            return Err(missing("table handle is missing"));
        }
        Ok(handle)
    }
    pub fn retained_table_next(
        &mut self,
        table: &RetainedValue,
        after: Option<&RetainedValue>,
    ) -> Result<Option<(RetainedValue, RetainedValue)>> {
        let table = Value::table(self.retained_table_handle(table)?);
        let after = after
            .map(|value| value.value(self))
            .transpose()?
            .unwrap_or_else(Value::nil);
        let Some((key, value)) = self.raw_table_next_value(table, after)? else {
            return Ok(None);
        };
        let mut roots = self.retain_values(&[key, value])?;
        let value = roots.pop().expect("two retained values");
        Ok(Some((roots.pop().expect("two retained values"), value)))
    }
    pub fn retained_table_is_readonly(&mut self, table: &RetainedValue) -> Result<bool> {
        let table = self.retained_table_handle(table)?;
        self.heap
            .table_is_readonly(table)
            .ok_or_else(|| missing("table handle is missing"))
    }
    pub fn set_retained_table_readonly(
        &mut self,
        table: &RetainedValue,
        readonly: bool,
    ) -> Result<()> {
        self.ensure_inspection_mutation_allowed()?;
        let table = self.retained_table_handle(table)?;
        self.ensure_memory_growth(0, &[Value::table(table)])?;
        self.heap
            .table_set_readonly(table, readonly)
            .ok_or_else(|| missing("table handle is missing"))
    }
    pub fn retained_table_metatable(
        &mut self,
        table: &RetainedValue,
    ) -> Result<Option<RetainedValue>> {
        let table = self.retained_table_handle(table)?;
        self.heap
            .table_metatable(table)
            .ok_or_else(|| missing("table handle is missing"))?
            .map(|meta| self.retain_value(Value::table(meta)))
            .transpose()
    }
    pub fn set_retained_table_metatable(
        &mut self,
        table: &RetainedValue,
        metatable: Option<&RetainedValue>,
    ) -> Result<()> {
        self.ensure_inspection_mutation_allowed()?;
        let table = self.retained_table_handle(table)?;
        let metatable = metatable
            .map(|meta| self.retained_table_handle(meta))
            .transpose()?;
        if self.heap.table_is_readonly(table) == Some(true) {
            return Err(capture_error(READONLY_TABLE_ERROR));
        }
        self.ensure_memory_growth(
            0,
            &[
                Value::table(table),
                metatable.map(Value::table).unwrap_or_else(Value::nil),
            ],
        )?;
        self.heap
            .table_set_metatable(table, metatable)
            .ok_or_else(|| missing("table handle is missing"))
    }
    fn ensure_inspection_mutation_allowed(&self) -> Result<()> {
        self.ensure_healthy()?;
        if self.settings.guest_surface.is_hardened_leaf() {
            return Err(Error::new(ErrorKind::HardenedLeafViolation {
                feature: "captured-state mutation",
            })
            .without_source_location());
        }
        Ok(())
    }
}

fn compiled_number(value: Value) -> Option<f64> {
    value
        .as_number()
        .filter(|number| !number.is_nan() || number.to_bits() == crate::number::CANON_NAN_BITS)
}
