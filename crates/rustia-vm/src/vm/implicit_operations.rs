//! Resolution and completion for synchronous implicit guest operations.
//! A lookup selects its callback exactly once. Bytecode saves its caller before
//! the selected callback runs; legacy synchronous users drive the same request.
use super::*;

pub(super) enum ImplicitStep<T> {
    Ready(T),
    Call(DeferredBytecodeCall),
}
impl<T> ImplicitStep<T> {
    pub(super) fn map<U>(self, f: impl FnOnce(T) -> U) -> ImplicitStep<U> {
        match self {
            Self::Ready(value) => ImplicitStep::Ready(f(value)),
            Self::Call(call) => ImplicitStep::Call(call),
        }
    }
}
/// A callback result is converted once, before its saved bytecode caller resumes.
#[derive(Clone, Copy)]
pub(super) enum ImplicitResult {
    Raw,
    Truthy { invert: bool },
    Length,
}
impl ImplicitResult {
    pub(super) fn complete(self, values: ReturnValues) -> Result<ReturnValues> {
        let first = values.first().copied().unwrap_or_else(Value::nil);
        match self {
            Self::Raw => Ok(values),
            Self::Truthy { invert } => Ok(ReturnValues::from_slice(&[Value::boolean(
                first.truthy() ^ invert,
            )])),
            Self::Length if first.kind() == ValueKind::Number => {
                Ok(ReturnValues::from_slice(&[first]))
            }
            Self::Length => Err(Error::new(ErrorKind::RuntimeError {
                message: "'__len' must return a number".to_owned(),
            })),
        }
    }
}
impl Vm {
    pub(super) fn finish_implicit_step<T>(
        &mut self,
        step: ImplicitStep<T>,
        fuel: &mut Execution<'_>,
        convert: fn(Vec<Value>) -> T,
    ) -> Result<T> {
        match step {
            ImplicitStep::Ready(value) => Ok(value),
            ImplicitStep::Call(request) => {
                let stack_base = self.call_stack.len();
                match self.drive_deferred_bytecode_flow(
                    Ok(VmFlow::Call(request)),
                    stack_base,
                    fuel,
                )? {
                    VmFlow::Return(values) => Ok(convert(values.into_vec())),
                    VmFlow::Yield(_) => Err(non_yieldable_boundary_error()),
                    VmFlow::Call(_) => unreachable!("driver completed with pending call"),
                }
            }
        }
    }

    pub(super) fn implicit_call(
        &mut self,
        program: &Program,
        method: Value,
        args: &[Value],
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Vec<Value>>> {
        self.implicit_call_result(program, method, args, depth, fuel, ImplicitResult::Raw)
    }
    pub(super) fn implicit_call_result(
        &mut self,
        program: &Program,
        method: Value,
        args: &[Value],
        depth: usize,
        fuel: &mut Execution<'_>,
        result: ImplicitResult,
    ) -> Result<ImplicitStep<Vec<Value>>> {
        if !self.owned_caller_can_transfer() {
            return self
                .call_value(program, method, args, depth, fuel)
                .and_then(|values| {
                    result
                        .complete(values.into())
                        .map(|values| ImplicitStep::Ready(values.into_vec()))
                });
        }
        self.check_recursion_depth(depth)?;
        let VmFlow::Call(mut request) = self.defer_non_bytecode_call(
            program,
            method,
            args,
            depth,
            self.current_call_site(program),
        )?
        else {
            unreachable!("transfer constructor");
        };
        // Dispatch admits the complete saved caller and these arguments before
        // any effect. This marker preserves the old call_value yield boundary.
        self.push_implicit_boundary(&mut request, result)?;
        Ok(ImplicitStep::Call(request))
    }
    pub(super) fn get_index_value(
        &mut self,
        program: &Program,
        object: Value,
        key: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<Value> {
        let step = self.get_index_value_step(program, object, key, depth, fuel)?;
        self.finish_implicit_step(step, fuel, first_result_or_nil)
    }
    pub(super) fn get_index_value_step(
        &mut self,
        program: &Program,
        object: Value,
        key: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        self.check_recursion_depth(depth)?;
        if let Some(table) = object.as_table() {
            return self.get_table_index_with_metamethod_step(program, table, key, depth, fuel);
        }
        if object.kind() == ValueKind::String {
            return self.string_method_index(key).map(ImplicitStep::Ready);
        }
        if object.kind() == ValueKind::Vector {
            return self
                .vector_index_value(object, key)
                .map(ImplicitStep::Ready);
        }
        if object.kind() == ValueKind::Userdata {
            return self.get_object_index_metamethod_step(program, object, key, depth, fuel);
        }
        Err(self.index_error(object, key)?)
    }

    pub(super) fn get_index_value_with_inline_cache_step(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        prototype: Option<u32>,
        pc: usize,
        object: Value,
        key: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        if let Some(table) = object.as_table() {
            return self.get_table_index_with_inline_cache_step(
                program, program_id, prototype, pc, table, key, depth, fuel,
            );
        }
        self.get_index_value_step(program, object, key, depth, fuel)
    }

    #[cfg(test)]
    pub(super) fn get_index_value_with_namecall_inline_cache(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        prototype: Option<u32>,
        pc: usize,
        object: Value,
        key: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<Value> {
        let step = self.get_index_value_with_namecall_inline_cache_step(
            program, program_id, prototype, pc, object, key, depth, fuel,
        )?;
        self.finish_implicit_step(step, fuel, first_result_or_nil)
    }
    pub(super) fn get_index_value_with_namecall_inline_cache_step(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        prototype: Option<u32>,
        pc: usize,
        object: Value,
        key: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        let Some(table) = object.as_table() else {
            return self.get_index_value_step(program, object, key, depth, fuel);
        };
        self.check_recursion_depth(depth)?;
        let Some(program_id) = program_id else {
            return self.get_table_index_with_metamethod_step(program, table, key, depth, fuel);
        };
        let Some(atom) = key.as_string_atom() else {
            return self.get_table_index_with_metamethod_step(program, table, key, depth, fuel);
        };

        if let Some(entry) = self.import_inline_cache(program_id, prototype, pc, 0) {
            if entry.atom() == Some(atom) {
                if let Some(Some(value)) =
                    self.heap.read_string_hash_slot(table, atom, entry.slot())
                {
                    if value.kind() != ValueKind::Nil {
                        #[cfg(any(test, feature = "executor-diagnostics"))]
                        self.record_table_inline_cache_hit(program_id);
                        return Ok(ImplicitStep::Ready(value));
                    }
                }
            }
        }

        if let Some(Some((value, slot))) = self.heap.table_string_hash_probe(table, atom) {
            if value.kind() != ValueKind::Nil {
                self.set_import_inline_cache(
                    program_id,
                    prototype,
                    pc,
                    0,
                    TableStringInlineCache::new(atom, slot),
                );
                return Ok(ImplicitStep::Ready(value));
            }
            if self.table_uses_standard_global_type_surface(table)? {
                return Ok(ImplicitStep::Ready(value));
            }
        }

        let Some(index) = self.metamethod_for_value(object, "__index")? else {
            return self.get_table_index_with_metamethod_step(program, table, key, depth, fuel);
        };
        let Some(index_table) = index.as_table() else {
            return self.get_table_index_with_metamethod_step(program, table, key, depth, fuel);
        };
        self.check_recursion_depth(depth.saturating_add(1))?;

        if let Some(entry) = self.import_inline_cache(program_id, prototype, pc, 1) {
            if entry.atom() == Some(atom) {
                if let Some(Some(value)) =
                    self.heap
                        .read_string_hash_slot(index_table, atom, entry.slot())
                {
                    if value.kind() != ValueKind::Nil {
                        #[cfg(any(test, feature = "executor-diagnostics"))]
                        self.record_table_inline_cache_hit(program_id);
                        return Ok(ImplicitStep::Ready(value));
                    }
                }
            }
        }

        if let Some(Some((value, slot))) = self.heap.table_string_hash_probe(index_table, atom) {
            if value.kind() != ValueKind::Nil {
                self.set_import_inline_cache(
                    program_id,
                    prototype,
                    pc,
                    1,
                    TableStringInlineCache::new(atom, slot),
                );
                return Ok(ImplicitStep::Ready(value));
            }
        }

        self.get_table_index_with_metamethod_step(
            program,
            index_table,
            key,
            depth.saturating_add(1),
            fuel,
        )
    }

    pub(super) fn get_index_value_with_import_inline_cache_step(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        prototype: Option<u32>,
        pc: usize,
        lane: usize,
        object: Value,
        key: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        if let Some(table) = object.as_table() {
            return self.get_table_index_with_import_inline_cache_step(
                program, program_id, prototype, pc, lane, table, key, depth, fuel,
            );
        }
        self.get_index_value_step(program, object, key, depth, fuel)
    }

    pub(super) fn get_table_index_with_metamethod_step(
        &mut self,
        program: &Program,
        table: u32,
        key: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        self.check_recursion_depth(depth)?;
        if let Some(index) = key.as_number().and_then(array_iterator_index) {
            if let Some(value) = self.heap.read_numeric_array_slot(table, index) {
                if !value.is_nil() {
                    return Ok(ImplicitStep::Ready(value));
                }
            }
        }
        if let Some(value) = self.raw_table_slot(table, key)? {
            if value.kind() != ValueKind::Nil
                || self.table_uses_standard_global_type_surface(table)?
            {
                return Ok(ImplicitStep::Ready(value));
            }
        }
        self.get_object_index_metamethod_step(program, Value::table(table), key, depth, fuel)
    }

    #[cfg(test)]
    pub(super) fn get_table_index_with_inline_cache(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        prototype: Option<u32>,
        pc: usize,
        table: u32,
        key: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<Value> {
        let step = self.get_table_index_with_inline_cache_step(
            program, program_id, prototype, pc, table, key, depth, fuel,
        )?;
        self.finish_implicit_step(step, fuel, first_result_or_nil)
    }
    pub(super) fn get_table_index_with_inline_cache_step(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        prototype: Option<u32>,
        pc: usize,
        table: u32,
        key: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        self.check_recursion_depth(depth)?;
        if let Some(index) = key.as_number().and_then(array_iterator_index) {
            if let Some(value) = self.heap.read_numeric_array_slot(table, index) {
                if !value.is_nil() {
                    return Ok(ImplicitStep::Ready(value));
                }
            }
        }
        let Some(program_id) = program_id else {
            return self.get_table_index_with_metamethod_step(program, table, key, depth, fuel);
        };
        let Some(atom) = key.as_string_atom() else {
            return self.get_table_index_with_metamethod_step(program, table, key, depth, fuel);
        };

        if let Some(entry) = self.table_inline_cache(program_id, prototype, pc) {
            if entry.atom() == Some(atom) {
                if let Some(Some(value)) =
                    self.heap.read_string_hash_slot(table, atom, entry.slot())
                {
                    if value.kind() != ValueKind::Nil {
                        #[cfg(any(test, feature = "executor-diagnostics"))]
                        self.record_table_inline_cache_hit(program_id);
                        return Ok(ImplicitStep::Ready(value));
                    }
                }
            }
        }

        if let Some(Some((value, slot))) = self.heap.table_string_hash_probe(table, atom) {
            if value.kind() != ValueKind::Nil {
                self.set_table_inline_cache(
                    program_id,
                    prototype,
                    pc,
                    TableStringInlineCache::new(atom, slot),
                );
                return Ok(ImplicitStep::Ready(value));
            }
        }

        self.get_table_index_with_metamethod_step(program, table, key, depth, fuel)
    }

    pub(super) fn get_table_index_with_import_inline_cache_step(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        prototype: Option<u32>,
        pc: usize,
        lane: usize,
        table: u32,
        key: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        self.check_recursion_depth(depth)?;
        let Some(program_id) = program_id else {
            return self.get_table_index_with_metamethod_step(program, table, key, depth, fuel);
        };
        let Some(atom) = key.as_string_atom() else {
            return self.get_table_index_with_metamethod_step(program, table, key, depth, fuel);
        };

        if let Some(entry) = self.import_inline_cache(program_id, prototype, pc, lane) {
            if entry.atom() == Some(atom) {
                if let Some(Some(value)) =
                    self.heap.read_string_hash_slot(table, atom, entry.slot())
                {
                    if value.kind() != ValueKind::Nil {
                        #[cfg(any(test, feature = "executor-diagnostics"))]
                        self.record_table_inline_cache_hit(program_id);
                        return Ok(ImplicitStep::Ready(value));
                    }
                }
            }
        }

        if let Some(Some((value, slot))) = self.heap.table_string_hash_probe(table, atom) {
            if value.kind() != ValueKind::Nil {
                self.set_import_inline_cache(
                    program_id,
                    prototype,
                    pc,
                    lane,
                    TableStringInlineCache::new(atom, slot),
                );
                return Ok(ImplicitStep::Ready(value));
            }
        }

        self.get_table_index_with_metamethod_step(program, table, key, depth, fuel)
    }

    pub(super) fn get_object_index_metamethod_step(
        &mut self,
        program: &Program,
        object: Value,
        key: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        self.check_recursion_depth(depth)?;
        if let Some(method) = self.host_object_registered_method(object, key) {
            return Ok(ImplicitStep::Ready(method));
        }
        let Some(method) = self.metamethod_for_value(object, "__index")? else {
            return if object.kind() == ValueKind::Table {
                Ok(ImplicitStep::Ready(Value::nil()))
            } else if object.kind() == ValueKind::Userdata {
                self.metamethod_for_value(object, "__namecall")?
                    .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))
                    .map(ImplicitStep::Ready)
            } else {
                Err(Error::new(ErrorKind::TypeError { expected: "table" }))
            };
        };
        match method.kind() {
            ValueKind::Function => self
                .implicit_call(
                    program,
                    method,
                    &[object, key],
                    depth.saturating_add(1),
                    fuel,
                )
                .map(|step| step.map(first_result_or_nil)),
            ValueKind::Table => {
                let table = method.as_table().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "__index table handle is missing",
                    })
                })?;
                self.get_table_index_with_metamethod_step(
                    program,
                    table,
                    key,
                    depth.saturating_add(1),
                    fuel,
                )
            }
            _ => Err(Error::new(ErrorKind::TypeError {
                expected: "function or table",
            })),
        }
    }

    pub(super) fn set_index_value_step(
        &mut self,
        program: &Program,
        object: Value,
        key: Value,
        value: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<()>> {
        self.check_recursion_depth(depth)?;
        if let Some(table) = object.as_table() {
            return self
                .set_table_index_with_metamethod_step(program, table, key, value, depth, fuel);
        }
        if object.kind() == ValueKind::Userdata {
            return self
                .set_object_newindex_metamethod_step(program, object, key, value, depth, fuel);
        }
        Err(self.index_error(object, key)?)
    }

    pub(super) fn set_index_value_with_inline_cache_step(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        prototype: Option<u32>,
        pc: usize,
        object: Value,
        key: Value,
        value: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<()>> {
        if let Some(table) = object.as_table() {
            return self.set_table_index_with_inline_cache_step(
                program, program_id, prototype, pc, table, key, value, depth, fuel,
            );
        }
        self.set_index_value_step(program, object, key, value, depth, fuel)
    }

    pub(super) fn set_table_index_with_metamethod_step(
        &mut self,
        program: &Program,
        table: u32,
        key: Value,
        value: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<()>> {
        self.check_recursion_depth(depth)?;
        let existing = self.raw_table_get(table, key)?;
        if existing.kind() != ValueKind::Nil {
            return self
                .set_table_index(table, key, value)
                .map(ImplicitStep::Ready);
        }
        if let Some(method) = self.metamethod_for_value(Value::table(table), "__newindex")? {
            return self.apply_newindex_metamethod_step(
                program,
                Value::table(table),
                method,
                [key, value],
                depth,
                fuel,
            );
        }
        self.set_table_index(table, key, value)
            .map(ImplicitStep::Ready)
    }

    #[cfg(test)]
    pub(super) fn set_table_index_with_inline_cache(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        prototype: Option<u32>,
        pc: usize,
        table: u32,
        key: Value,
        value: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<()> {
        let step = self.set_table_index_with_inline_cache_step(
            program, program_id, prototype, pc, table, key, value, depth, fuel,
        )?;
        self.finish_implicit_step(step, fuel, |_| ())
    }
    pub(super) fn set_table_index_with_inline_cache_step(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        prototype: Option<u32>,
        pc: usize,
        table: u32,
        key: Value,
        value: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<()>> {
        self.check_recursion_depth(depth)?;
        if let Some(index) = key.as_number().and_then(array_iterator_index) {
            match self.heap.try_write_numeric_array_slot(table, index, value) {
                Some(TableNumericArrayWrite::Stored) => return Ok(ImplicitStep::Ready(())),
                Some(TableNumericArrayWrite::Readonly) => {
                    return Err(readonly_table_error());
                }
                Some(TableNumericArrayWrite::Deferred) | None => {}
            }
        }
        let Some(program_id) = program_id else {
            return self
                .set_table_index_with_metamethod_step(program, table, key, value, depth, fuel);
        };
        let Some(atom) = key.as_string_atom() else {
            return self
                .set_table_index_with_metamethod_step(program, table, key, value, depth, fuel);
        };

        if let Some(entry) = self.table_inline_cache(program_id, prototype, pc) {
            if entry.atom() == Some(atom) {
                match self
                    .heap
                    .write_string_hash_slot(table, atom, entry.slot(), value)
                {
                    Some(TableStringHashWrite::Stored) => {
                        #[cfg(any(test, feature = "executor-diagnostics"))]
                        self.record_table_inline_cache_hit(program_id);
                        return Ok(ImplicitStep::Ready(()));
                    }
                    Some(TableStringHashWrite::Readonly) => {
                        return Err(readonly_table_error());
                    }
                    Some(TableStringHashWrite::Deferred) | None => {}
                }
            }
        }

        if let Some(Some((existing, slot))) = self.heap.table_string_hash_probe(table, atom) {
            if existing.kind() != ValueKind::Nil {
                match self.heap.write_string_hash_slot(table, atom, slot, value) {
                    Some(TableStringHashWrite::Stored) => {
                        self.set_table_inline_cache(
                            program_id,
                            prototype,
                            pc,
                            TableStringInlineCache::new(atom, slot),
                        );
                        return Ok(ImplicitStep::Ready(()));
                    }
                    Some(TableStringHashWrite::Readonly) => {
                        return Err(readonly_table_error());
                    }
                    Some(TableStringHashWrite::Deferred) | None => {}
                }
            }
        }

        self.set_table_index_with_metamethod_step(program, table, key, value, depth, fuel)
    }

    pub(super) fn set_object_newindex_metamethod_step(
        &mut self,
        program: &Program,
        object: Value,
        key: Value,
        value: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<()>> {
        self.check_recursion_depth(depth)?;
        self.reject_host_object_method_write(object, key)?;
        let Some(method) = self.metamethod_for_value(object, "__newindex")? else {
            return Err(Error::new(ErrorKind::TypeError { expected: "table" }));
        };
        self.apply_newindex_metamethod_step(program, object, method, [key, value], depth, fuel)
    }

    pub(super) fn apply_newindex_metamethod_step(
        &mut self,
        program: &Program,
        object: Value,
        method: Value,
        entry: [Value; 2],
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<()>> {
        self.check_recursion_depth(depth)?;
        match method.kind() {
            ValueKind::Function => self
                .implicit_call(
                    program,
                    method,
                    &[object, entry[0], entry[1]],
                    depth.saturating_add(1),
                    fuel,
                )
                .map(|step| step.map(|_| ())),
            ValueKind::Table => {
                let table = method.as_table().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "__newindex table handle is missing",
                    })
                })?;
                self.set_table_index_with_metamethod_step(
                    program,
                    table,
                    entry[0],
                    entry[1],
                    depth.saturating_add(1),
                    fuel,
                )
            }
            _ => Err(Error::new(ErrorKind::TypeError {
                expected: "function or table",
            })),
        }
    }

    pub(super) fn equal_values(
        &mut self,
        program: &Program,
        lhs: Value,
        rhs: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<bool> {
        let step = self.equal_values_step(program, lhs, rhs, depth, fuel)?;
        self.finish_implicit_step(step, fuel, |values| first_result_or_nil(values).truthy())
    }
    pub(super) fn equal_values_step(
        &mut self,
        program: &Program,
        lhs: Value,
        rhs: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<bool>> {
        if let Some(method) = self.compatible_binary_metamethod(lhs, rhs, "__eq")? {
            return self
                .implicit_call(program, method, &[lhs, rhs], depth.saturating_add(1), fuel)
                .map(|step| step.map(|values| first_result_or_nil(values).truthy()));
        }
        self.raw_equal_values(lhs, rhs).map(ImplicitStep::Ready)
    }
}
