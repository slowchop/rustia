//! Resumable expanded calls whose recognized builtin callees are looked up
//! after argument evaluation (Luau 0.709 `compileExprCall` ordering).
use super::*;
use crate::bytecode::{expanded_call_count, CallDescriptor};

#[derive(Clone)]
enum ExpandedCallee {
    Early(Value),
    Import(u32),
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
    depth: usize,
    call_site: Option<ErrorLocation>,
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

pub(super) fn descriptor(program: &Program, instruction: Instruction) -> Result<&CallDescriptor> {
    let count = expanded_call_count(instruction.op)
        .ok_or_else(|| invalid("instruction does not support deferred callees"))?;
    let Some(Constant::CallDescriptor(descriptor)) =
        program.constants().get((!instruction.aux) as usize)
    else {
        return Err(invalid("expanded call must reference a call descriptor"));
    };
    if descriptor.aux < 0 {
        return Err(invalid(
            "call descriptor contains a negative argument window",
        ));
    }
    if descriptor.deferred_imports[count..]
        .iter()
        .any(Option::is_some)
    {
        return Err(invalid("call descriptor contains unused deferred imports"));
    }
    for index in descriptor.deferred_imports.iter().flatten() {
        import_length(program, *index)?;
    }
    Ok(descriptor)
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
        let descriptor = descriptor(program, instruction)?;
        let aux = descriptor.aux;
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
        let mut counts = [0usize; 4];
        let count =
            expanded_call_count(instruction.op).ok_or_else(|| invalid("invalid expanded call"))?;
        let mut append_varargs = false;
        match instruction.op {
            Opcode::CallExpand => {
                if aux > i32::from(u8::MAX) {
                    return Err(invalid("invalid expanded function result count"));
                }
                counts[0] = usize::from(instruction.b);
                counts[1] = usize::from(instruction.c);
            }
            Opcode::ReturnCallExpand => {
                let (outer, inner) = unpack_expand_aux(aux)?;
                counts[..2].copy_from_slice(&[usize::from(outer), usize::from(inner)]);
            }
            Opcode::AppendCallExpand => {
                let (_, inner) = unpack_append_call_expand_aux(aux)?;
                counts[..2].copy_from_slice(&[usize::from(instruction.c), usize::from(inner)]);
            }
            Opcode::CallExpand2
            | Opcode::ReturnCallExpand2
            | Opcode::ReturnCallExpand2Varargs
            | Opcode::CallExpand3 => {
                let (outer, middle, inner) = unpack_nested_expand_aux(aux)?;
                counts[..3].copy_from_slice(&[
                    usize::from(outer),
                    usize::from(middle),
                    usize::from(inner),
                ]);
                if instruction.op == Opcode::CallExpand3 {
                    counts[3] = usize::from(instruction.c);
                }
                append_varargs = instruction.op == Opcode::ReturnCallExpand2Varargs;
            }
            Opcode::CallExpandVarargs | Opcode::ReturnCallExpandVarargs => {
                let (outer, inner) = unpack_expand_varargs_aux(aux)?;
                counts[..2].copy_from_slice(&[usize::from(outer), usize::from(inner)]);
                append_varargs = true;
            }
            _ => return Err(invalid("invalid expanded call")),
        }
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
        for (level, argc) in counts[..count].iter().copied().enumerate() {
            let end = cursor + 1 + argc;
            let values = registers.get(cursor..end).ok_or_else(|| {
                Error::new(ErrorKind::RegisterOutOfBounds {
                    register: end,
                    max: registers.len(),
                })
            })?;
            let callee = match descriptor.deferred_imports[level] {
                Some(index) => ExpandedCallee::Import(index),
                None => ExpandedCallee::Early(values[0]),
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
        fuel: &mut Execution<'_>,
    ) -> VmFlowResult<Vec<Value>> {
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
                self.call_value_with_site_resumable(
                    program,
                    callee,
                    &call.args,
                    state.depth,
                    state.call_site.clone(),
                    fuel,
                )
            })();
            self.truncate_stackless_self_call_roots(root_mark);
            match outcome? {
                VmFlow::Return(returned) => results = returned.into_vec(),
                VmFlow::Yield(suspension) => return Ok(VmFlow::Yield(suspension)),
            }
        }
        Ok(VmFlow::Return(results))
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
                    deferred_imports: [Some(3), None, None, None],
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
                deferred_imports: [None; 4],
            }))],
        );
        let (plain_live, plain_reserved) = plain.memory_usage(true);
        let (metadata_live, metadata_reserved) = metadata.memory_usage(true);
        assert_eq!(metadata_live - plain_live, size_of::<CallDescriptor>());
        assert_eq!(
            metadata_reserved - plain_reserved,
            size_of::<CallDescriptor>()
        );
    }
}
