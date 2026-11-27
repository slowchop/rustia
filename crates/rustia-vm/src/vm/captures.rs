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
