//! Resumable trailing-argument expansion, with late imports for recognized
//! builtin calls (Luau 0.709 `compileExprCall` ordering).
use super::*;
use crate::bytecode::{supports_call_descriptor, CallDescriptor, CallDescriptorCall};

#[derive(Clone)]
enum ExpandedCallee {
    Early(Value),
    Import(u32),
    Method {
        name: u32,
        receiver_register: Option<u8>,
    },
}

#[derive(Clone)]
struct PendingExpandedCall {
    callee: ExpandedCallee,
    args: Vec<Value>,
}

#[derive(Clone)]
enum ExpandedFinish {
    WriteFixed { base: usize, count: usize },
    Return { prefix: Vec<Value> },
    Append { table: u32, start_index: usize },
}

#[derive(Clone)]
pub(super) struct ExpandedCallState {
    // Outermost first: popping evaluates the next innermost pending call.
    calls: Vec<PendingExpandedCall>,
    finish: ExpandedFinish,
    pub(super) depth: usize,
    pub(super) call_site: Option<ErrorLocation>,
}

pub(super) enum ExpandedCallFlow {
    Transfer(DeferredBytecodeCall),
    Return(Vec<Value>),
    Yield(CoroutineYield),
    Call { callee: Value, args: Vec<Value> },
}

fn invalid(reason: &'static str) -> Error {
    Error::new(ErrorKind::InvalidProgram { reason })
}

fn import_length(program: &Program, index: u32) -> Result<u8> {
    let Some(Constant::Import(path)) = program.constants().get(index as usize) else {
        return Err(invalid(
            "deferred callee must reference an import descriptor",
        ));
    };
    if !(1..=IMPORT_CACHE_LANES).contains(&path.len()) {
        return Err(invalid("import path length must be between one and three"));
    }
    if path.iter().any(|index| {
        !matches!(
            program.constants().get(*index as usize),
            Some(Constant::String(_))
        )
    }) {
        return Err(invalid("import components must reference string constants"));
    }
    if index > i32::MAX as u32 {
        return Err(invalid("deferred import descriptor index overflow"));
    }
    Ok(path.len() as u8)
}

pub(super) fn descriptor(
    program: &Program,
    instruction: Instruction,
    max_registers: usize,
) -> Result<&CallDescriptor> {
    if !supports_call_descriptor(instruction.op) {
        return Err(invalid(
            "instruction does not support expanded call descriptors",
        ));
    }
    let Some(Constant::CallDescriptor(descriptor)) =
        program.constants().get((!instruction.aux) as usize)
    else {
        return Err(invalid("expanded call must reference a call descriptor"));
    };
    if descriptor.aux < 0 {
        return Err(invalid("call descriptor contains negative terminal data"));
    }
    let valid_call_count = (2..=256).contains(&descriptor.calls.len())
        || (descriptor.calls.len() == 1 && descriptor.append_varargs);
    if !valid_call_count {
        return Err(invalid(
            "expanded call descriptor requires two to 256 calls, or one with varargs",
        ));
    }
    match instruction.op {
        Opcode::CallExpand if descriptor.aux > i32::from(u8::MAX) => {
            return Err(invalid("invalid expanded function result count"));
        }
        Opcode::CallExpand => {}
        Opcode::AppendCallExpand => {
            if descriptor.aux & 0xff != 0 {
                return Err(invalid(
                    "expanded append descriptor contains unused argument bits",
                ));
            }
            unpack_append_call_expand_aux(descriptor.aux)?;
        }
        _ if descriptor.aux != 0 => {
            return Err(invalid("expanded call descriptor contains unused aux data"))
        }
        _ => {}
    }
    let mut window = 0usize;
    for call in &descriptor.calls {
        window += 1 + usize::from(call.fixed_args);
        if window > 256 {
            return Err(invalid("expanded call descriptor exceeds register space"));
        }
        if let Some(index) = call.deferred_import {
            import_length(program, index)?;
        }
        if call
            .method_receiver_register
            .is_some_and(|register| usize::from(register) >= max_registers)
        {
            return Err(invalid("late method receiver register out of bounds"));
        }
        if call.method_receiver_register.is_some() && call.deferred_method.is_none() {
            return Err(invalid("late receiver requires method lookup"));
        }
        if let Some(index) = call.deferred_method {
            if call.deferred_import.is_some() || call.fixed_args == 0 {
                return Err(invalid("deferred method requires a receiver and no import"));
            }
            if index > i32::MAX as u32
                || !matches!(
                    program.constants().get(index as usize),
                    Some(Constant::String(_))
                )
            {
                return Err(invalid("deferred method must reference a string constant"));
            }
        }
    }
    Ok(descriptor)
}

/// Decode the original compact forms into the same resumable call windows as
/// descriptor-bearing calls. These at-most-four entries stay on the stack.
fn legacy_descriptor(
    instruction: Instruction,
) -> Result<(i32, SmallVec<[CallDescriptorCall; 4]>, bool)> {
    let (counts, count, aux, append_varargs) = match instruction.op {
        Opcode::CallExpand => {
            if !(0..=i32::from(u8::MAX)).contains(&instruction.aux) {
                return Err(invalid("invalid expanded function result count"));
            }
            (
                [instruction.b, instruction.c, 0, 0],
                2,
                instruction.aux,
                false,
            )
        }
        Opcode::CallExpand2 | Opcode::ReturnCallExpand2 => {
            let (outer, middle, inner) = unpack_nested_expand_aux(instruction.aux)?;
            ([outer, middle, inner, 0], 3, 0, false)
        }
        Opcode::CallExpand3 => {
            let (outer, middle, inner) = unpack_nested_expand_aux(instruction.aux)?;
            ([outer, middle, inner, instruction.c], 4, 0, false)
        }
        Opcode::CallExpandVarargs | Opcode::ReturnCallExpandVarargs => {
            let (outer, middle) = unpack_expand_varargs_aux(instruction.aux)?;
            ([outer, middle, 0, 0], 2, 0, true)
        }
        Opcode::ReturnCallExpand2Varargs => {
            let (outer, middle, inner) = unpack_nested_expand_aux(instruction.aux)?;
            ([outer, middle, inner, 0], 3, 0, true)
        }
        Opcode::ReturnCallExpand => {
            let (outer, inner) = unpack_expand_aux(instruction.aux)?;
            ([outer, inner, 0, 0], 2, 0, false)
        }
        Opcode::AppendCallExpand => {
            let (_, inner) = unpack_append_call_expand_aux(instruction.aux)?;
            (
                [instruction.c, inner, 0, 0],
                2,
                instruction.aux & !255,
                false,
            )
        }
        _ => return Err(invalid("instruction does not support expanded calls")),
    };
    let calls = counts[..count]
        .iter()
        .map(|fixed_args| CallDescriptorCall {
            fixed_args: *fixed_args,
            deferred_import: None,
            deferred_method: None,
            method_receiver_register: None,
        })
        .collect();
    Ok((aux, calls, append_varargs))
}

impl ExpandedCallState {
    pub(super) fn new(
        program: &Program,
        instruction: Instruction,
        registers: &[Value],
        varargs: &[Value],
        depth: usize,
        call_site: Option<ErrorLocation>,
    ) -> Result<Self> {
        let legacy;
        let (aux, call_windows, append_varargs) = if instruction.aux < 0 {
            let descriptor = descriptor(program, instruction, registers.len())?;
            (
                descriptor.aux,
                descriptor.calls.as_slice(),
                descriptor.append_varargs,
            )
        } else {
            legacy = legacy_descriptor(instruction)?;
            (legacy.0, legacy.1.as_slice(), legacy.2)
        };
        let returning = matches!(
            instruction.op,
            Opcode::ReturnCallExpand
                | Opcode::ReturnCallExpand2
                | Opcode::ReturnCallExpandVarargs
                | Opcode::ReturnCallExpand2Varargs
        );
        let base = usize::from(if returning {
            instruction.c
        } else if instruction.op == Opcode::AppendCallExpand {
            instruction.b
        } else {
            instruction.a
        });
        let count = call_windows.len();
        let finish = if returning {
            let first = usize::from(instruction.a);
            let end = first + usize::from(instruction.b);
            let prefix = registers.get(first..end).ok_or_else(|| {
                Error::new(ErrorKind::RegisterOutOfBounds {
                    register: end,
                    max: registers.len(),
                })
            })?;
            ExpandedFinish::Return {
                prefix: try_copy_vec(prefix)?,
            }
        } else if instruction.op == Opcode::AppendCallExpand {
            let table = get_register(registers, instruction.a)?
                .as_table()
                .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
            let (start_index, _) = unpack_append_call_expand_aux(aux)?;
            ExpandedFinish::Append {
                table,
                start_index: start_index as usize,
            }
        } else {
            let count = match instruction.op {
                Opcode::CallExpand => aux as usize,
                Opcode::CallExpand3 => usize::from(instruction.b),
                _ => usize::from(instruction.c),
            };
            if base + count > registers.len() {
                return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                    register: base + count,
                    max: registers.len(),
                }));
            }
            ExpandedFinish::WriteFixed { base, count }
        };
        let mut calls = Vec::new();
        calls
            .try_reserve_exact(count)
            .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
        let mut cursor = base;
        for (level, call) in call_windows.iter().enumerate() {
            let end = cursor + 1 + usize::from(call.fixed_args);
            let values = registers.get(cursor..end).ok_or_else(|| {
                Error::new(ErrorKind::RegisterOutOfBounds {
                    register: end,
                    max: registers.len(),
                })
            })?;
            let callee = match (call.deferred_import, call.deferred_method) {
                (Some(index), _) => ExpandedCallee::Import(index),
                (_, Some(index)) => ExpandedCallee::Method {
                    name: index,
                    receiver_register: call.method_receiver_register,
                },
                _ => ExpandedCallee::Early(values[0]),
            };
            let mut args = try_copy_vec(&values[1..])?;
            if append_varargs && level + 1 == count {
                args.try_reserve(varargs.len())
                    .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
                args.extend_from_slice(varargs);
            }
            calls.push(PendingExpandedCall { callee, args });
            cursor = end;
        }
        Ok(Self {
            calls,
            finish,
            depth,
            call_site,
        })
    }

    pub(super) fn finish(
        self,
        registers: &mut [Value],
        pending_prefixes: &mut Vec<Value>,
        mut results: Vec<Value>,
        vm: &mut Vm,
    ) -> Result<Option<Vec<Value>>> {
        let root_mark = vm
            .mutate_last_call_stack_frame(|frame| {
                let mark = frame.stackless_roots.len();
                self.heap_roots(&mut frame.stackless_roots);
                extend_heap_backed_roots(&mut frame.stackless_roots, &results);
                mark
            })
            .unwrap_or(0);
        let outcome = (|| match self.finish {
            ExpandedFinish::WriteFixed { base, count } => {
                write_call_results(registers, base, count, &results)?;
                Ok(None)
            }
            ExpandedFinish::Return { mut prefix } => {
                prefix
                    .try_reserve(results.len())
                    .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
                prefix.append(&mut results);
                Ok(Some(
                    finish_function_results(pending_prefixes, prefix).into_vec(),
                ))
            }
            ExpandedFinish::Append { table, start_index } => {
                vm.append_call_results_to_table(table, start_index, &results)?;
                Ok(None)
            }
        })();
        vm.truncate_stackless_self_call_roots(root_mark);
        outcome
    }

    pub(super) fn roots(&self, roots: &mut Vec<Value>) {
        for call in &self.calls {
            if let ExpandedCallee::Early(value) = call.callee {
                roots.push(value);
            }
            roots.extend_from_slice(&call.args);
        }
        match &self.finish {
            ExpandedFinish::Return { prefix } => roots.extend_from_slice(prefix),
            ExpandedFinish::Append { table, .. } => roots.push(Value::table(*table)),
            ExpandedFinish::WriteFixed { .. } => {}
        }
    }

    pub(super) fn heap_roots(&self, roots: &mut Vec<Value>) {
        for call in &self.calls {
            if let ExpandedCallee::Early(value) = call.callee {
                extend_heap_backed_roots(roots, &[value]);
            }
            extend_heap_backed_roots(roots, &call.args);
        }
        match &self.finish {
            ExpandedFinish::Return { prefix } => extend_heap_backed_roots(roots, prefix),
            ExpandedFinish::Append { table, .. } => roots.push(Value::table(*table)),
            ExpandedFinish::WriteFixed { .. } => {}
        }
    }

    pub(super) fn try_clone(&self) -> Result<Self> {
        let mut calls = Vec::new();
        calls
            .try_reserve_exact(self.calls.len())
            .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
        for call in &self.calls {
            calls.push(PendingExpandedCall {
                callee: call.callee.clone(),
                args: try_copy_vec(&call.args)?,
            });
        }
        let finish = match &self.finish {
            ExpandedFinish::Return { prefix } => ExpandedFinish::Return {
                prefix: try_copy_vec(prefix)?,
            },
            other => other.clone(),
        };
        Ok(Self {
            calls,
            finish,
            depth: self.depth,
            call_site: self.call_site.clone(),
        })
    }

    pub(super) fn shrink(&mut self) {
        for call in &mut self.calls {
            call.args.shrink_to_fit();
        }
        self.calls.shrink_to_fit();
        if let ExpandedFinish::Return { prefix } = &mut self.finish {
            prefix.shrink_to_fit();
        }
    }

    pub(super) fn memory(&self, usage: &mut MemoryUsage) {
        usage.add_vec(&self.calls);
        for call in &self.calls {
            usage.add_vec(&call.args);
        }
        if let ExpandedFinish::Return { prefix } = &self.finish {
            usage.add_vec(prefix);
        }
        add_error_location_memory(self.call_site.as_ref(), usage);
    }
}

impl Vm {
    pub(super) fn continue_expanded_calls(
        &mut self,
        program: &Program,
        state: &mut ExpandedCallState,
        mut results: Vec<Value>,
        registers: &[Value],
        register_cells: &mut [Option<u32>],
        fuel: &mut Execution<'_>,
    ) -> Result<ExpandedCallFlow> {
        while !state.calls.is_empty() {
            let root_mark = self
                .mutate_last_call_stack_frame(|frame| {
                    let mark = frame.stackless_roots.len();
                    state.heap_roots(&mut frame.stackless_roots);
                    extend_heap_backed_roots(&mut frame.stackless_roots, &results);
                    mark
                })
                .unwrap_or(0);
            let outcome = (|| {
                let mut call = state
                    .calls
                    .pop()
                    .ok_or_else(|| invalid("missing pending expanded call"))?;
                call.args
                    .try_reserve(results.len())
                    .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
                call.args.append(&mut results);
                let callee = match call.callee {
                    ExpandedCallee::Early(value) => value,
                    ExpandedCallee::Method {
                        name: index,
                        receiver_register,
                    } => {
                        if let Some(register) = receiver_register {
                            charge_fuel(&mut fuel.state.fuel, fuel.state.fuel_limit, 1)?;
                            self.executed_instructions = self.executed_instructions.wrapping_add(1);
                            let value = match register_cells
                                .get(usize::from(register))
                                .copied()
                                .flatten()
                            {
                                Some(cell) => self
                                    .heap
                                    .upvalue_get(cell)
                                    .ok_or_else(|| invalid("local receiver cell is missing"))?,
                                None => get_register(registers, register)?,
                            };
                            *call
                                .args
                                .first_mut()
                                .ok_or_else(|| invalid("missing method receiver"))? = value;
                            self.mutate_last_call_stack_frame(|frame| {
                                extend_heap_backed_roots(&mut frame.stackless_roots, &[value]);
                            });
                            self.maybe_collect()?;
                        }
                        let receiver = *call
                            .args
                            .first()
                            .ok_or_else(|| invalid("missing method receiver"))?;
                        charge_fuel(&mut fuel.state.fuel, fuel.state.fuel_limit, 3)?;
                        self.executed_instructions = self.executed_instructions.wrapping_add(1);
                        let key = self.instruction_string_key_value(
                            program,
                            None,
                            index as i32,
                            "negative method-name constant index",
                            "method-name constant must be a string",
                        )?;
                        self.advance_fused_instruction_steps_with_root(1, receiver)?;
                        let callee =
                            self.get_index_value(program, receiver, key, state.depth, fuel)?;
                        self.advance_fused_instruction_steps_with_root(1, callee)?;
                        self.check_method_value(callee, receiver, key)?;
                        callee
                    }
                    ExpandedCallee::Import(index) => {
                        let length = import_length(program, index)?;
                        let cost = u64::from(length) * 2 - 1;
                        charge_fuel(&mut fuel.state.fuel, fuel.state.fuel_limit, cost)?;
                        self.executed_instructions = self.executed_instructions.wrapping_add(1);
                        self.maybe_collect()?;
                        // This fused instruction has no GetImport cache lanes. Reuse
                        // the ordinary lookup semantics without a cache identity.
                        self.get_import(
                            program,
                            None,
                            None,
                            0,
                            Instruction {
                                op: Opcode::GetImport,
                                a: 0,
                                b: length,
                                c: 0,
                                aux: index as i32,
                            },
                            state.depth,
                            fuel,
                        )?
                    }
                };
                // A lookup metamethod can produce a fresh function or callable
                // object that is reachable from neither the registers nor globals.
                self.mutate_last_call_stack_frame(|frame| {
                    extend_heap_backed_roots(&mut frame.stackless_roots, &[callee]);
                });
                if let Some(handle) = callee.as_function() {
                    let kind = self
                        .heap
                        .function_kind(handle)
                        .ok_or_else(|| invalid("function handle is missing"))?;
                    if let Some(results) =
                        self.try_leaf_bytecode_handle(kind, handle, state.depth, fuel)?
                    {
                        return Ok(ExpandedCallFlow::Return(results.into_vec()));
                    }
                    if matches!(
                        kind,
                        FunctionKind::Bytecode { .. } | FunctionKind::DynamicMain { .. }
                    ) {
                        return Ok(ExpandedCallFlow::Call {
                            callee,
                            args: call.args,
                        });
                    }
                }
                if matches!(callee.kind(), ValueKind::Table | ValueKind::Userdata) {
                    return Ok(ExpandedCallFlow::Call {
                        callee,
                        args: call.args,
                    });
                }
                let flow = self.call_value_with_site_resumable(
                    program,
                    callee,
                    &call.args,
                    state.depth,
                    state.call_site.clone(),
                    fuel,
                )?;
                match flow {
                    VmFlow::Return(results) => Ok(ExpandedCallFlow::Return(results.into_vec())),
                    VmFlow::Yield(suspension) => Ok(ExpandedCallFlow::Yield(suspension)),
                    VmFlow::Call(request) => Ok(ExpandedCallFlow::Transfer(request)),
                }
            })();
            self.truncate_stackless_self_call_roots(root_mark);
            match outcome? {
                ExpandedCallFlow::Return(returned) => results = returned,
                other => return Ok(other),
            }
        }
        Ok(ExpandedCallFlow::Return(results))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(finish: ExpandedFinish) -> ExpandedCallState {
        ExpandedCallState {
            calls: vec![
                PendingExpandedCall {
                    callee: ExpandedCallee::Early(Value::table(11)),
                    args: vec![Value::table(12)],
                },
                PendingExpandedCall {
                    callee: ExpandedCallee::Import(0),
                    args: vec![Value::table(13)],
                },
            ],
            finish,
            depth: 1,
            call_site: None,
        }
    }

    #[test]
    fn snapshot_copy_preserves_pending_calls_and_return_prefix_roots() {
        let continuation = BytecodeContinuation::ExpandedCalls(state(ExpandedFinish::Return {
            prefix: vec![Value::table(14)],
        }));
        let mut copy = try_clone_bytecode_continuation(&continuation).unwrap();
        let BytecodeContinuation::ExpandedCalls(ref mut copied) = copy else {
            panic!("wrong continuation");
        };
        copied.calls[0].args.clear();
        let mut roots = Vec::new();
        extend_bytecode_continuation_roots(&mut roots, &continuation);
        assert_eq!(
            roots
                .iter()
                .filter_map(|value| value.as_table())
                .collect::<Vec<_>>(),
            [11, 12, 13, 14]
        );
        let mut heap_roots = Vec::new();
        extend_bytecode_continuation_heap_roots(&mut heap_roots, &continuation);
        assert_eq!(heap_roots.len(), roots.len());
        let mut copied_roots = Vec::new();
        extend_bytecode_continuation_roots(&mut copied_roots, &copy);
        assert_eq!(
            copied_roots
                .iter()
                .filter_map(|value| value.as_table())
                .collect::<Vec<_>>(),
            [11, 13, 14]
        );
        let mut usage = MemoryUsage::default();
        add_continuation_memory(&continuation, &mut usage);
        assert_eq!(
            usage.live_bytes,
            2 * size_of::<PendingExpandedCall>() + 3 * size_of::<Value>()
        );
        assert!(usage.reserved_bytes >= usage.live_bytes);
        shrink_bytecode_continuation_capacity(&mut copy);
    }

    #[test]
    fn table_append_destination_is_a_continuation_root() {
        let continuation = BytecodeContinuation::ExpandedCalls(state(ExpandedFinish::Append {
            table: 15,
            start_index: 1,
        }));
        let mut roots = Vec::new();
        extend_bytecode_continuation_heap_roots(&mut roots, &continuation);
        assert_eq!(roots.last().and_then(|value| value.as_table()), Some(15));
    }

    #[test]
    fn fresh_lookup_callee_survives_collection_during_host_panic_diagnostics() {
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        vm.install_host_function(HostFunctionOp::new(
            "argument",
            1,
            OpCapability::Pure,
            Arc::new(|_, _| Ok(vec![Value::number(7.0)])),
        ))
        .unwrap();
        let lookup = vm
            .register_host_function(HostFunctionOp::new(
                "lookup",
                1,
                OpCapability::Pure,
                Arc::new(|vm, _| {
                    let callee = vm.register_host_function(HostFunctionOp::new(
                        "fresh",
                        1,
                        OpCapability::Pure,
                        Arc::new(|vm, _| {
                            vm.collect_garbage();
                            panic!("host panic after collecting");
                        }),
                    ))?;
                    Ok(vec![callee])
                }),
            ))
            .unwrap();
        let object = vm.empty_table();
        let metatable = vm.empty_table();
        let key = vm.string("__index");
        vm.raw_table_set_value(metatable, key, lookup).unwrap();
        vm.heap
            .table_set_metatable(object.as_table().unwrap(), metatable.as_table())
            .unwrap();
        vm.set_global_value("object", object).unwrap();
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::with_aux(Opcode::GetGlobal, 1, 0, 0, 0),
                    Instruction::with_aux(Opcode::CallExpand, 0, 0, 0, !4),
                    Instruction::new(Opcode::Return, 0, 1, 0),
                ],
                2,
            ),
            vec![
                Constant::String(b"argument".to_vec()),
                Constant::String(b"object".to_vec()),
                Constant::String(b"field".to_vec()),
                Constant::Import(vec![1, 2]),
                Constant::CallDescriptor(Box::new(CallDescriptor {
                    aux: 1,
                    calls: vec![
                        CallDescriptorCall {
                            fixed_args: 0,
                            deferred_import: Some(3),
                            deferred_method: None,
                            method_receiver_register: None,
                        },
                        CallDescriptorCall {
                            fixed_args: 0,
                            deferred_import: None,
                            deferred_method: None,
                            method_receiver_register: None,
                        },
                    ],
                    append_varargs: false,
                })),
            ],
        );
        let error = vm.run(&program).unwrap_err();
        assert!(
            matches!(error.kind, ErrorKind::HostPanicked { ref function } if function == "fresh")
        );
        assert!(vm.is_poisoned());
    }

    #[test]
    fn descriptor_memory_is_accounted() {
        let plain = Program::new(
            Function::new(vec![Instruction::new(Opcode::Return, 0, 1, 0)], 1),
            vec![Constant::Nil],
        );
        let metadata = Program::new(
            plain.main.clone(),
            vec![Constant::CallDescriptor(Box::new(CallDescriptor {
                aux: 1,
                calls: vec![
                    CallDescriptorCall {
                        fixed_args: 0,
                        deferred_import: None,
                        deferred_method: None,
                        method_receiver_register: None,
                    };
                    2
                ],
                append_varargs: false,
            }))],
        );
        let (plain_live, plain_reserved) = plain.memory_usage(true);
        let (metadata_live, metadata_reserved) = metadata.memory_usage(true);
        assert_eq!(
            metadata_live - plain_live,
            size_of::<CallDescriptor>() + 2 * size_of::<CallDescriptorCall>()
        );
        assert_eq!(
            metadata_reserved - plain_reserved,
            size_of::<CallDescriptor>() + 2 * size_of::<CallDescriptorCall>()
        );
    }

    #[test]
    fn compatibility_call_limit_is_distinct_from_the_application_recursion_limit() {
        let configured = Vm::new(Profile::ReproducibleStandard);
        let error = configured.recursion_limit_error();
        assert!(matches!(error.kind, ErrorKind::RecursionLimit { .. }));
        assert!(error.is_transaction_abort());
        assert!(error.is_always_hard_abort());
        let compatibility = Vm::with_settings(
            Profile::ReproducibleStandard,
            VmSettings {
                enable_conformance_luau_call_limit: true,
                ..VmSettings::default()
            },
        );
        let error = compatibility
            .check_recursion_depth(LUAU_CONFORMANCE_CALL_LIMIT)
            .unwrap_err();
        assert!(
            matches!(error.kind, ErrorKind::RuntimeError { ref message } if message == "stack overflow")
        );
        assert!(!error.is_transaction_abort());
        assert!(!error.is_always_hard_abort());
    }
}
