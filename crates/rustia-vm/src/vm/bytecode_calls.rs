//! Explicit transfers between bytecode functions used by expanded arguments.
//! A saved caller carries the same complete state as a suspended coroutine.
use super::*;

impl Vm {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run_function(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        function: &Function,
        args: &[Value],
        context: FunctionCallContext<'_>,
        resumed: Option<ResumedFunctionState>,
        fuel: &mut Execution<'_>,
    ) -> VmFlowResult<ReturnValues> {
        let stack_base = self.call_stack.len();
        let first =
            self.run_function_step(program, program_id, function, args, context, resumed, fuel);
        self.drive_deferred_bytecode_flow(first, stack_base, fuel)
    }

    /// Drives internal calls; genuine guest yields remain visible to the caller.
    /// `stack_base` is the call-stack length before the initial dispatch.
    pub(super) fn drive_deferred_bytecode_flow(
        &mut self,
        initial: VmFlowResult<ReturnValues>,
        stack_base: usize,
        fuel: &mut Execution<'_>,
    ) -> VmFlowResult<ReturnValues> {
        if !matches!(initial, Ok(VmFlow::Call(_))) {
            return initial;
        }
        let pending_slot = self.active_coroutine_frames.len();
        if self.active_coroutine_frames.try_reserve(1).is_err() {
            if let Ok(VmFlow::Call(request)) = &initial {
                self.release_require_frames(&request.callers);
            }
            self.call_stack.truncate(stack_base);
            self.mark_control_memory_dirty();
            return Err(Error::new(ErrorKind::OutOfMemory).without_source_location());
        }
        self.active_coroutine_frames.push(Vec::new());
        self.mark_control_memory_dirty();
        let result = self.drive_bytecode_calls(initial, pending_slot, fuel);
        self.call_stack.truncate(stack_base);
        let pending = self.active_coroutine_frames.pop();
        if let Some(pending) = &pending {
            self.release_require_frames(pending);
        }
        debug_assert!(pending.is_some());
        debug_assert_eq!(self.active_coroutine_frames.len(), pending_slot);
        self.mark_control_memory_dirty();
        // A collection inside the driver kept coroutine indices stable. Once
        // the last transfer scope closes, release any dead registry entries.
        if self.guest_transactions.is_empty() {
            self.prune_unreachable_coroutines_if_idle();
        }
        result
    }

    fn drive_bytecode_calls(
        &mut self,
        mut flow: VmFlowResult<ReturnValues>,
        pending_slot: usize,
        fuel: &mut Execution<'_>,
    ) -> VmFlowResult<ReturnValues> {
        loop {
            match flow {
                Ok(VmFlow::Call(request)) => {
                    let DeferredBytecodeCall {
                        callee,
                        args,
                        depth,
                        call_site,
                        fallback_program,
                        callers,
                    } = request;
                    if self.active_coroutine_frames[pending_slot]
                        .try_reserve(callers.len())
                        .is_err()
                    {
                        self.release_require_frames(&callers);
                        return Err(Error::new(ErrorKind::OutOfMemory).without_source_location());
                    }
                    self.active_coroutine_frames[pending_slot].extend(callers.into_iter().rev());
                    self.mark_control_memory_dirty();
                    flow = self.enter_deferred_bytecode_call(
                        fallback_program,
                        callee,
                        args,
                        depth,
                        call_site,
                        fuel,
                    );
                }
                Ok(VmFlow::Return(results)) => {
                    let Some(frame) = self.active_coroutine_frames[pending_slot].pop() else {
                        return Ok(VmFlow::Return(results));
                    };
                    self.mark_control_memory_dirty();
                    flow = match frame {
                        CoroutineFrame::Bytecode(frame) => {
                            // Only a bytecode continuation owns a saved call header.
                            self.call_stack.pop();
                            self.resume_deferred_bytecode_frame(
                                frame.into_inner(),
                                results.into_vec(),
                                None,
                                fuel,
                            )
                        }
                        CoroutineFrame::Require(frame) => self
                            .finish_required_module(frame, results.into_vec())
                            .map(|results| VmFlow::Return(results.into())),
                        CoroutineFrame::PCall | CoroutineFrame::XPCall(_) => {
                            let mut protected = Vec::new();
                            protected
                                .try_reserve_exact(results.len() + 1)
                                .map_err(|_| {
                                    Error::new(ErrorKind::OutOfMemory).without_source_location()
                                })?;
                            protected.push(Value::boolean(true));
                            protected.extend(results);
                            Ok(VmFlow::Return(protected.into()))
                        }
                    };
                }
                Ok(VmFlow::Yield(mut suspension)) => {
                    let pending = &mut self.active_coroutine_frames[pending_slot];
                    if suspension.frames.try_reserve(pending.len()).is_err() {
                        self.release_require_frames(&suspension.frames);
                        return Err(Error::new(ErrorKind::OutOfMemory).without_source_location());
                    }
                    while let Some(mut frame) = pending.pop() {
                        // A real suspension discards the active call headers.
                        // The handler will resume in a fresh coroutine stack.
                        if let CoroutineFrame::XPCall(frame) = &mut frame {
                            frame.traceback_base = 0;
                        }
                        suspension.frames.push(frame);
                    }
                    return Ok(VmFlow::Yield(suspension));
                }
                Err(error) => {
                    if error_is_uncatchable(&error, fuel) {
                        return Err(error);
                    }
                    let Some(frame) = self.active_coroutine_frames[pending_slot].pop() else {
                        return Err(error);
                    };
                    self.mark_control_memory_dirty();
                    flow = match frame {
                        CoroutineFrame::Bytecode(frame) => {
                            self.call_stack.pop();
                            self.resume_deferred_bytecode_frame(
                                frame.into_inner(),
                                Vec::new(),
                                Some(error),
                                fuel,
                            )
                        }
                        CoroutineFrame::Require(frame) => {
                            self.release_module_load(&frame);
                            Err(error)
                        }
                        CoroutineFrame::PCall => {
                            self.last_error_traceback = None;
                            self.mark_control_memory_dirty();
                            let error = self.error_object(error);
                            Ok(VmFlow::Return(vec![Value::boolean(false), error].into()))
                        }
                        CoroutineFrame::XPCall(frame) => self
                            .resume_xpcall_error_handler(frame, error, fuel)
                            .map(|results| VmFlow::Return(results.into())),
                    };
                }
            }
        }
    }

    pub(super) fn retain_dynamic_program_ref_with_roots(
        &mut self,
        program: &Program,
        roots: Vec<Value>,
        frames: Vec<CoroutineFrame>,
    ) -> Result<u32> {
        // Program admission can collect. Keep uninstalled call targets, results,
        // and native completion metadata visible until retention finishes.
        self.call_stack
            .try_reserve(1)
            .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
        // The second reservation can fail after call-stack capacity grew.
        self.mark_control_memory_dirty();
        self.active_coroutine_frames
            .try_reserve(1)
            .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
        let environment = self.current_environment()?;
        self.call_stack.push(CallStackFrame {
            program: None,
            function: None,
            debug_name: None,
            environment,
            thread_environment: self.current_thread_environment(),
            module_cohort: self.current_module_cohort(),
            call_site: None,
            roots,
            stackless_roots: Vec::new(),
            stackless_debug_frames: Vec::new(),
        });
        self.active_coroutine_frames.push(frames);
        self.mark_control_memory_dirty();
        let retained = self.retain_dynamic_program_ref(program);
        self.active_coroutine_frames.pop();
        self.call_stack.pop();
        self.mark_control_memory_dirty();
        if self.guest_transactions.is_empty() {
            self.prune_unreachable_coroutines_if_idle();
        }
        retained
    }

    fn enter_deferred_bytecode_call(
        &mut self,
        fallback_program: u32,
        mut callee: Value,
        mut args: Vec<Value>,
        mut depth: usize,
        mut call_site: Option<ErrorLocation>,
        fuel: &mut Execution<'_>,
    ) -> VmFlowResult<ReturnValues> {
        // Resolve callable receivers before entering bytecode, rather than
        // re-entering `run_function` through each recursive `__call` boundary.
        // This uses the same receiver order and depth increments as ordinary
        // `call_metamethod`; actual guest suspension stays with the driver.
        let handle = loop {
            self.check_recursion_depth(depth)?;
            if let Some(handle) = callee.as_function() {
                break handle;
            }
            let method = if matches!(callee.kind(), ValueKind::Table | ValueKind::Userdata) {
                self.metamethod_for_value(callee, "__call")?
            } else {
                None
            };
            let Some(method) = method else {
                let program = self.dynamic_program(fallback_program)?;
                return self.call_value_with_site_resumable(
                    &program, callee, &args, depth, call_site, fuel,
                );
            };
            args.try_reserve(1)
                .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
            args.insert(0, callee);
            callee = method;
            depth = depth.saturating_add(1);
            call_site = None;
        };
        self.mutate_last_call_stack_frame(|frame| {
            extend_heap_backed_roots(&mut frame.stackless_roots, &[callee]);
            extend_heap_backed_roots(&mut frame.stackless_roots, &args);
        });
        let (kind, data) = self
            .heap
            .function_dispatch_data(handle)
            .ok_or_else(|| invalid_deferred_call("bytecode function handle is missing"))?;
        let (program_id, prototype) = match kind {
            FunctionKind::Bytecode { program, prototype } => {
                (program.unwrap_or(fallback_program), Some(prototype))
            }
            FunctionKind::DynamicMain { program } => (program, None),
            _ => {
                let program = self.dynamic_program(fallback_program)?;
                return self.call_function_kind_with_site_resumable(
                    &program, callee, handle, kind, data, &args, depth, call_site, fuel,
                );
            }
        };
        let program = self.dynamic_program(program_id)?;
        let function = match prototype {
            Some(index) => program
                .functions
                .get(index as usize)
                .ok_or_else(|| invalid_deferred_call("bytecode function prototype is missing"))?,
            None => &program.main,
        };
        let (captures, environment, module_cohort) =
            data.ok_or_else(|| invalid_deferred_call("bytecode function call data is missing"))?;
        let thread_environment = if self.call_stack.is_empty() && self.running_coroutines.is_empty()
        {
            environment
        } else {
            self.current_thread_environment()
        };
        self.run_function_step(
            &program,
            Some(program_id),
            function,
            &args,
            FunctionCallContext {
                current_function: Some(callee),
                captures: &captures,
                depth: depth + 1,
                environment,
                thread_environment,
                module_cohort,
                call_site,
                prototype,
            },
            None,
            fuel,
        )
        .map_err(|error| attach_existing_instruction_source(&program, function, error))
    }

    fn resume_deferred_bytecode_frame(
        &mut self,
        frame: SuspendedBytecodeFrame,
        results: Vec<Value>,
        error: Option<Error>,
        fuel: &mut Execution<'_>,
    ) -> VmFlowResult<ReturnValues> {
        let program = self.dynamic_program(frame.program_id)?;
        let function = match frame.prototype {
            Some(index) => program
                .functions
                .get(index as usize)
                .ok_or_else(|| invalid_deferred_call("saved caller prototype is missing"))?,
            None => &program.main,
        };
        self.run_function_step(
            &program,
            Some(frame.program_id),
            function,
            &[],
            FunctionCallContext {
                current_function: frame.current_function,
                captures: &frame.captures,
                depth: frame.depth,
                environment: frame.environment,
                thread_environment: frame.thread_environment,
                module_cohort: frame.module_cohort,
                call_site: frame.call_site,
                prototype: frame.prototype,
            },
            Some(ResumedFunctionState {
                pc: frame.pc,
                registers: frame.registers,
                register_cells: frame.register_cells,
                varargs: frame.varargs,
                pending_return_prefixes: frame.pending_return_prefixes,
                continuation: frame.continuation,
                resume_results: results,
                resume_error: error,
                stackless_self_calls: frame.stackless_self_calls,
                tail_call_count: frame.tail_call_count,
            }),
            fuel,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn defer_bytecode_frame(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        context: &FunctionCallContext<'_>,
        pc: usize,
        registers: RegisterValues,
        register_cells: RegisterCells,
        varargs: Vec<Value>,
        pending_return_prefixes: Vec<Value>,
        continuation: BytecodeContinuation,
        stackless_self_calls: Vec<StacklessSelfCallFrame>,
        tail_call_count: usize,
        mut request: DeferredBytecodeCall,
    ) -> VmFlowResult<ReturnValues> {
        self.mutate_last_call_stack_frame(|frame| {
            extend_heap_backed_roots(&mut frame.stackless_roots, &[request.callee]);
            extend_heap_backed_roots(&mut frame.stackless_roots, &request.args);
            extend_bytecode_continuation_heap_roots(&mut frame.stackless_roots, &continuation);
        });
        let program_id = match program_id {
            Some(id) => id,
            None => match self.retain_dynamic_program_ref(program) {
                Ok(id) => id,
                Err(error) => {
                    self.release_require_frames(&request.callers);
                    return Err(error);
                }
            },
        };
        // Internal transfers save exactly the same complete caller state as a
        // genuine guest yield, but the dispatcher consumes them immediately.
        let saved = self.suspend_bytecode_frame(
            Some(program_id),
            context,
            pc,
            registers,
            register_cells,
            varargs,
            pending_return_prefixes,
            continuation,
            stackless_self_calls,
            tail_call_count,
            CoroutineYield {
                yielded: Vec::new(),
                frames: std::mem::take(&mut request.callers),
            },
        )?;
        let VmFlow::Yield(saved) = saved else {
            return Err(invalid_deferred_call(
                "saving a caller did not produce frames",
            ));
        };
        request.callers = saved.frames;
        Ok(VmFlow::Call(request))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn defer_expanded_bytecode_call(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        context: &FunctionCallContext<'_>,
        pc: usize,
        mut registers: RegisterValues,
        register_cells: RegisterCells,
        varargs: Vec<Value>,
        pending_return_prefixes: Vec<Value>,
        state: ExpandedCallState,
        stackless_self_calls: Vec<StacklessSelfCallFrame>,
        tail_call_count: usize,
        callee: Value,
        args: Vec<Value>,
    ) -> VmFlowResult<ReturnValues> {
        // Retaining a hand-built program can collect. Keep the fresh selected
        // callee, arguments, and intermediate chain results rooted first.
        self.mutate_last_call_stack_frame(|frame| {
            extend_heap_backed_roots(&mut frame.stackless_roots, &[callee]);
            extend_heap_backed_roots(&mut frame.stackless_roots, &args);
            state.heap_roots(&mut frame.stackless_roots);
        });
        let program_id = match program_id {
            Some(id) => id,
            None => self.retain_dynamic_program_ref(program)?,
        };
        self.sync_register_cells(&mut registers, &register_cells)?;
        let depth = state.depth;
        let call_site = state.call_site.clone();
        let caller = SuspendedBytecodeFrameStorage::try_new(SuspendedBytecodeFrame {
            program_id,
            prototype: context.prototype,
            current_function: context.current_function,
            captures: try_copy_vec(context.captures)?,
            depth: context.depth,
            environment: context.environment,
            thread_environment: self.current_thread_environment(),
            module_cohort: context.module_cohort,
            call_site: context.call_site.clone(),
            pc,
            registers,
            register_cells,
            varargs,
            pending_return_prefixes,
            continuation: BytecodeContinuation::ExpandedCalls(state),
            stackless_self_calls,
            tail_call_count,
        })?;
        let mut callers = Vec::new();
        callers
            .try_reserve_exact(1)
            .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
        callers.push(CoroutineFrame::Bytecode(caller));
        Ok(VmFlow::Call(DeferredBytecodeCall {
            callee,
            args,
            depth,
            call_site,
            fallback_program: program_id,
            callers,
        }))
    }
}

fn invalid_deferred_call(reason: &'static str) -> Error {
    Error::new(ErrorKind::InvalidProgram { reason })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_xpcall_roots_its_uninstalled_target_and_arguments_during_retention() {
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let mut function = Function::new(vec![Instruction::new(Opcode::Return, 0, 1, 0)], 1);
        function.parameter_count = 1;
        let program = Program::new(function, Vec::new());
        let program_id = vm.retain_dynamic_program_ref(&program).unwrap();
        let handle = vm
            .heap
            .alloc_dynamic_main_function_in_cohort(program_id, vm.globals, 0)
            .unwrap();
        let argument = vm.empty_table();
        let handler = vm.get_global_value("tostring").unwrap();
        let quota = vm.current_live_bytes() + 16 * 1024;
        let garbage = vm.heap.alloc_buffer(vec![0; 32 * 1024]).unwrap();
        vm.settings.memory_limit_bytes = Some(quota);
        let mut state = vm.begin_execution();
        let mut hosts = RejectHostDispatcher;
        let mut execution = Execution {
            state: &mut state,
            hosts: &mut hosts,
        };
        let flow = vm.native_xpcall_resumable(
            &program,
            &[Value::function(handle), handler, argument],
            0,
            None,
            &mut execution,
        );
        assert!(
            !vm.heap.has_buffer(garbage),
            "retention must collect the unrooted garbage"
        );
        let flow = vm
            .drive_deferred_bytecode_flow(flow.map(into_return_values_flow), 0, &mut execution)
            .unwrap();
        let VmFlow::Return(results) = flow else {
            panic!("protected bytecode target should complete");
        };
        assert_eq!(results[0].as_bool(), Some(true));
        assert_eq!(results[1].as_table(), argument.as_table());
        assert!(vm.heap.has_table(argument.as_table().unwrap()));
        assert!(vm.heap.function_kind(handle).is_some());
        assert!(vm.call_stack.is_empty());
        assert!(vm.active_coroutine_frames.is_empty());
    }

    #[test]
    fn failed_rooted_retention_restores_its_temporary_control_scopes() {
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let value = vm.empty_table();
        let program = Program::new(
            Function::new(vec![Instruction::new(Opcode::Return, 0, 0, 0)], 1),
            Vec::new(),
        );
        vm.settings.memory_limit_bytes = Some(0);
        assert!(matches!(
            vm.retain_dynamic_program_ref_with_roots(
                &program,
                vec![value],
                vec![CoroutineFrame::PCall]
            )
            .unwrap_err()
            .kind,
            ErrorKind::OutOfMemory,
        ));
        assert!(vm.heap.has_table(value.as_table().unwrap()));
        assert!(vm.call_stack.is_empty());
        assert!(vm.active_coroutine_frames.is_empty());
    }
}
