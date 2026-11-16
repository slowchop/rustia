//! Native completion state consumed by the same internal call driver.
//! Native frames never escape as resumable coroutine frames: an escaping yield
//! is rejected at this boundary, after inner protected frames are discarded.
use super::*;

#[derive(Clone)]
pub(super) struct NativeFrameStorage(Vec<NativeFrame>);

#[derive(Clone)]
pub(super) struct NativeFrame {
    pub(super) program_id: u32,
    pub(super) callback_stack_base: usize,
    pub(super) depth: usize,
    callback_site: Option<ErrorLocation>,
    state: NativeCompletion,
}

#[derive(Clone)]
enum NativeCompletion {
    Implicit(ImplicitResult),
    ToString,
    Foreach(ForeachState),
    GSub(GSubState),
}

#[derive(Clone, Copy)]
struct ForeachState {
    table: u32,
    callback: Value,
    cursor: ForeachCursor,
}
#[derive(Clone, Copy)]
enum ForeachCursor {
    Pairs(Value),
    Array { next: usize, end: usize },
}

#[derive(Clone)]
struct GSubState {
    text: Vec<u8>,
    pattern: GSubPattern,
    replacement: Value,
    max: usize,
    cursor: usize,
    count: usize,
    output: Vec<u8>,
    matched: Option<GSubMatch>,
}

impl NativeFrameStorage {
    fn new(frame: NativeFrame) -> Result<Self> {
        let mut storage = Vec::new();
        storage.try_reserve_exact(1).map_err(|_| oom())?;
        storage.push(frame);
        Ok(Self(storage))
    }
    pub(super) fn frame(&self) -> &NativeFrame {
        &self.0[0]
    }
    fn into_frame(mut self) -> NativeFrame {
        self.0.pop().expect("one native completion")
    }
    pub(super) fn roots(&self, roots: &mut Vec<Value>) {
        if let NativeCompletion::Foreach(state) = &self.frame().state {
            roots.extend_from_slice(&[Value::table(state.table), state.callback]);
            if let ForeachCursor::Pairs(key) = state.cursor {
                roots.push(key);
            }
        }
        if let NativeCompletion::GSub(state) = &self.frame().state {
            roots.push(state.replacement);
        }
    }
    pub(super) fn memory(&self, usage: &mut MemoryUsage) {
        usage.add_vec(&self.0);
        add_error_location_memory(self.frame().callback_site.as_ref(), usage);
        if let NativeCompletion::GSub(state) = &self.frame().state {
            usage.add_vec(&state.text);
            usage.add_vec(&state.output);
            pattern_memory(&state.pattern, usage);
            if let Some(matched) = &state.matched {
                usage.add_vec(&matched.captures);
            }
        }
    }
    pub(super) fn shrink(&mut self) {
        self.0.shrink_to_fit();
        if let NativeCompletion::GSub(state) = &mut self.0[0].state {
            state.text.shrink_to_fit();
            state.output.shrink_to_fit();
            shrink_pattern(&mut state.pattern);
            if let Some(matched) = &mut state.matched {
                matched.captures.shrink_to_fit();
            }
        }
    }
    pub(super) fn try_clone(&self) -> Result<Self> {
        let frame = self.frame();
        let state = match &frame.state {
            NativeCompletion::Implicit(result) => NativeCompletion::Implicit(*result),
            NativeCompletion::ToString => NativeCompletion::ToString,
            NativeCompletion::Foreach(state) => NativeCompletion::Foreach(*state),
            NativeCompletion::GSub(state) => NativeCompletion::GSub(GSubState {
                text: try_copy_vec(&state.text)?,
                pattern: clone_pattern(&state.pattern)?,
                replacement: state.replacement,
                max: state.max,
                cursor: state.cursor,
                count: state.count,
                output: try_copy_vec(&state.output)?,
                matched: state
                    .matched
                    .as_ref()
                    .map(|m| {
                        Ok(GSubMatch {
                            start: m.start,
                            end: m.end,
                            captures: try_copy_vec(&m.captures)?,
                        })
                    })
                    .transpose()?,
            }),
        };
        Self::new(NativeFrame {
            program_id: frame.program_id,
            callback_stack_base: frame.callback_stack_base,
            depth: frame.depth,
            callback_site: frame.callback_site.clone(),
            state,
        })
    }
}

impl Vm {
    pub(super) fn push_implicit_boundary(
        &mut self,
        request: &mut DeferredBytecodeCall,
        result: ImplicitResult,
    ) -> Result<()> {
        let frame = NativeFrame {
            program_id: request.fallback_program,
            callback_stack_base: self.call_stack.len(),
            depth: request.depth,
            callback_site: request.call_site.clone(),
            state: NativeCompletion::Implicit(result),
        };
        self.push_deferred_caller(
            request,
            CoroutineFrame::Native(NativeFrameStorage::new(frame)?),
        )
    }

    pub(super) fn native_tostring_transfer(
        &mut self,
        program: &Program,
        args: &[Value],
        depth: usize,
    ) -> VmFlowResult<Vec<Value>> {
        validate_native_arity(args, 1, usize::MAX, "tostring argument")?;
        let Some(method) = self.tostring_metamethod(args[0])? else {
            let bytes = self.value_to_tostring(args[0])?;
            return Ok(VmFlow::Return(vec![self
                .heap
                .intern_string_value_bytes(&bytes)]));
        };
        if method.kind() != ValueKind::Function {
            return Err(Error::new(ErrorKind::TypeError {
                expected: "function",
            }));
        }
        self.check_recursion_depth(depth.saturating_add(1))?;
        let callback_site = self.current_call_site(program);
        let VmFlow::Call(mut request) = self.defer_non_bytecode_call(
            program,
            method,
            &[args[0]],
            depth.saturating_add(1),
            callback_site.clone(),
        )?
        else {
            unreachable!("transfer constructor")
        };
        let frame = NativeFrame {
            program_id: request.fallback_program,
            callback_stack_base: self.call_stack.len(),
            depth,
            state: NativeCompletion::ToString,
            callback_site,
        };
        self.push_deferred_caller(
            &mut request,
            CoroutineFrame::Native(NativeFrameStorage::new(frame)?),
        )?;
        self.admit_native_request(request).map(into_vec_flow)
    }

    // Function replacements only. Other replacement shapes retain their current
    // implementation until their implicit __index completion is represented.
    pub(super) fn native_gsub_transfer(
        &mut self,
        program: &Program,
        args: &[Value],
        depth: usize,
    ) -> VmFlowResult<Vec<Value>> {
        validate_native_arity(args, 3, 4, "string.gsub arguments")?;
        let text = self.value_to_string(args[0])?;
        let pattern = self.value_to_string(args[1])?;
        let max = if let Some(value) = args.get(3).copied().filter(|v| v.kind() != ValueKind::Nil) {
            self.coerce_to_i32(value, "integer string.gsub count")?
                .max(0) as usize
        } else {
            text.len().saturating_add(1)
        };
        if max == 0 {
            return Ok(VmFlow::Return(vec![
                self.heap.intern_string_value_bytes(&text),
                Value::number(0.0),
            ]));
        }
        if let Some((output, count)) =
            self.try_recursive_reverse_gsub(program, &text, &pattern, args[2])?
        {
            return Ok(VmFlow::Return(vec![
                self.heap.intern_string_value_bytes(&output),
                Value::number(count as f64),
            ]));
        }
        let state = GSubState {
            text,
            pattern: parse_gsub_pattern(&pattern)?,
            replacement: args[2],
            max,
            cursor: 0,
            count: 0,
            output: Vec::new(),
            matched: None,
        };
        // No callback or collection occurs while the local state is prepared.
        // The finished request is rooted and quota-admitted before dispatch.
        let frame = NativeFrame {
            program_id: 0,
            callback_stack_base: self.call_stack.len(),
            depth,
            state: NativeCompletion::GSub(state),
            callback_site: self.current_call_site(program),
        };
        self.advance_gsub(frame, Some(program)).map(into_vec_flow)
    }

    pub(super) fn complete_native_frame(
        &mut self,
        storage: NativeFrameStorage,
        results: ReturnValues,
    ) -> VmFlowResult<ReturnValues> {
        let mut frame = storage.into_frame();
        match &mut frame.state {
            NativeCompletion::Implicit(result) => result.complete(results).map(VmFlow::Return),
            NativeCompletion::ToString => {
                let value = results.first().copied().unwrap_or_else(Value::nil);
                let bytes = self.string_value_bytes(value)?.ok_or_else(|| {
                    Error::new(ErrorKind::RuntimeError {
                        message: "'__tostring' must return a string".into(),
                    })
                })?;
                Ok(VmFlow::Return(
                    vec![self.heap.intern_string_value_bytes(&bytes)].into(),
                ))
            }
            NativeCompletion::Foreach(_) => {
                let result = results.first().copied().unwrap_or_else(Value::nil);
                if result.kind() != ValueKind::Nil {
                    Ok(VmFlow::Return(vec![result].into()))
                } else {
                    self.advance_foreach(frame, None)
                }
            }
            NativeCompletion::GSub(state) => {
                let matched = state
                    .matched
                    .take()
                    .expect("gsub callback has a pending match");
                let replacement = self
                    .gsub_replacement_result(results.first().copied().unwrap_or_else(Value::nil))?;
                push_gsub_output(
                    &mut state.output,
                    replacement
                        .as_deref()
                        .unwrap_or(&state.text[matched.start..matched.end]),
                )?;
                state.cursor = matched.end;
                state.count += 1;
                if matched.start == matched.end {
                    if state.cursor >= state.text.len() {
                        state.max = state.count;
                    } else {
                        push_gsub_output(
                            &mut state.output,
                            &state.text[state.cursor..state.cursor + 1],
                        )?;
                        state.cursor += 1;
                    }
                }
                self.advance_gsub(frame, None)
            }
        }
    }

    pub(super) fn native_foreach_transfer(
        &mut self,
        program: &Program,
        args: &[Value],
        depth: usize,
        mode: TableForeachMode,
    ) -> VmFlowResult<Vec<Value>> {
        let (table, callback) = table_foreach_arguments(args)?;
        let cursor = match mode {
            TableForeachMode::Pairs => ForeachCursor::Pairs(Value::nil()),
            TableForeachMode::Array => ForeachCursor::Array {
                next: 1,
                end: self.table_foreachi_len(table)?,
            },
        };
        let frame = NativeFrame {
            program_id: 0,
            callback_stack_base: self.call_stack.len(),
            depth,
            callback_site: self.current_call_site(program),
            state: NativeCompletion::Foreach(ForeachState {
                table,
                callback,
                cursor,
            }),
        };
        self.advance_foreach(frame, Some(program))
            .map(into_vec_flow)
    }

    fn advance_foreach(
        &mut self,
        mut frame: NativeFrame,
        initial_program: Option<&Program>,
    ) -> VmFlowResult<ReturnValues> {
        let NativeCompletion::Foreach(state) = &mut frame.state else {
            unreachable!("foreach completion")
        };
        let (key, value) = match state.cursor {
            ForeachCursor::Pairs(previous) => {
                let next = self
                    .table_next_after(state.table, previous)
                    .ok_or_else(|| {
                        Error::new(ErrorKind::InvalidProgram {
                            reason: "table.foreach table handle is missing",
                        })
                    })?;
                match next {
                    TableNext::Entry(key, value) => {
                        state.cursor = ForeachCursor::Pairs(key);
                        (key, value)
                    }
                    TableNext::End => return Ok(VmFlow::Return(vec![Value::nil()].into())),
                    TableNext::InvalidKey => return Err(Error::new(ErrorKind::InvalidKeyToNext)),
                }
            }
            ForeachCursor::Array { next, end } => {
                if next > end {
                    return Ok(VmFlow::Return(vec![Value::nil()].into()));
                }
                let key = Value::number(next as f64);
                let value = self.raw_table_get(state.table, key)?;
                state.cursor = ForeachCursor::Array {
                    next: next + 1,
                    end,
                };
                (key, value)
            }
        };
        // Foreach historically calls at the native entry depth (unlike gsub).
        // Charge its native extra fuel once, before creating this cursor.
        self.check_recursion_depth(frame.depth)?;
        let callee = state.callback;
        let mut request = if let Some(program) = initial_program {
            let VmFlow::Call(request) = self.defer_non_bytecode_call(
                program,
                callee,
                &[key, value],
                frame.depth,
                frame.callback_site.clone(),
            )?
            else {
                unreachable!("transfer constructor")
            };
            request
        } else {
            DeferredBytecodeCall {
                callee,
                args: try_copy_vec(&[key, value])?,
                depth: frame.depth,
                call_site: frame.callback_site.clone(),
                fallback_program: frame.program_id,
                callers: Vec::new(),
            }
        };
        frame.program_id = request.fallback_program;
        self.push_deferred_caller(
            &mut request,
            CoroutineFrame::Native(NativeFrameStorage::new(frame)?),
        )?;
        // The popped state and freshly read pair are traced before the only
        // collection-capable step, with full saved control memory admitted.
        self.admit_native_request(request)
    }

    fn advance_gsub(
        &mut self,
        mut frame: NativeFrame,
        initial_program: Option<&Program>,
    ) -> VmFlowResult<ReturnValues> {
        let NativeCompletion::GSub(state) = &mut frame.state else {
            unreachable!("gsub completion")
        };
        let matched = if state.count < state.max {
            find_next_gsub_match(&state.text, &state.pattern, state.cursor)?
        } else {
            None
        };
        let Some(matched) = matched else {
            push_gsub_output(&mut state.output, &state.text[state.cursor..])?;
            return Ok(VmFlow::Return(
                vec![
                    self.heap.intern_string_value_bytes(&state.output),
                    Value::number(state.count as f64),
                ]
                .into(),
            ));
        };
        push_gsub_output(&mut state.output, &state.text[state.cursor..matched.start])?;
        let args = self.gsub_replacement_args(&state.text, &matched)?;
        self.check_recursion_depth(frame.depth.saturating_add(1))?;
        let callee = state.replacement;
        state.matched = Some(matched);
        let mut request = if let Some(program) = initial_program {
            let VmFlow::Call(request) = self.defer_non_bytecode_call(
                program,
                callee,
                &args,
                frame.depth.saturating_add(1),
                frame.callback_site.clone(),
            )?
            else {
                unreachable!("transfer constructor")
            };
            request
        } else {
            DeferredBytecodeCall {
                callee,
                args,
                depth: frame.depth.saturating_add(1),
                call_site: frame.callback_site.clone(),
                fallback_program: frame.program_id,
                callers: Vec::new(),
            }
        };
        frame.program_id = request.fallback_program;
        self.push_deferred_caller(
            &mut request,
            CoroutineFrame::Native(NativeFrameStorage::new(frame)?),
        )?;
        self.admit_native_request(request)
    }

    fn admit_native_request(
        &mut self,
        mut request: DeferredBytecodeCall,
    ) -> VmFlowResult<ReturnValues> {
        // This is the only collection-capable operation after taking a native
        // completion out of the driver. Put its full state back in traced,
        // accounted VM storage before quota collection; also root fresh args.
        let mut roots = try_copy_vec(&request.args)?;
        roots.try_reserve(1).map_err(|_| oom())?;
        roots.push(request.callee);
        self.active_coroutine_frames
            .try_reserve(1)
            .map_err(|_| oom())?;
        self.active_coroutine_frames
            .push(std::mem::take(&mut request.callers));
        self.mark_control_memory_dirty();
        let admitted = self.ensure_memory_growth(
            request.args.len().saturating_mul(size_of::<Value>()),
            &roots,
        );
        request.callers = self
            .active_coroutine_frames
            .pop()
            .expect("native admission frame slot");
        self.mark_control_memory_dirty();
        admitted?;
        Ok(VmFlow::Call(request))
    }
}

fn oom() -> Error {
    Error::new(ErrorKind::OutOfMemory).without_source_location()
}

fn clone_steps(steps: &[CaptureStep]) -> Result<Vec<CaptureStep>> {
    let mut cloned = Vec::new();
    cloned.try_reserve_exact(steps.len()).map_err(|_| oom())?;
    for step in steps {
        cloned.push(match step {
            CaptureStep::Capture { capture_id, steps } => CaptureStep::Capture {
                capture_id: *capture_id,
                steps: clone_steps(steps)?,
            },
            other => other.clone(),
        });
    }
    Ok(cloned)
}
fn clone_pattern(pattern: &GSubPattern) -> Result<GSubPattern> {
    Ok(match pattern {
        GSubPattern::Plain(bytes) => GSubPattern::Plain(try_copy_vec(bytes)?),
        GSubPattern::NonCapturing(p) => GSubPattern::NonCapturing(NonCapturingPattern {
            anchored_start: p.anchored_start,
            steps: try_copy_vec(&p.steps)?,
        }),
        GSubPattern::Capturing(p) => GSubPattern::Capturing(CapturingPattern {
            anchored_start: p.anchored_start,
            steps: clone_steps(&p.steps)?,
            capture_count: p.capture_count,
        }),
        other => other.clone(),
    })
}
fn steps_memory(steps: &Vec<CaptureStep>, usage: &mut MemoryUsage) {
    usage.add_vec(steps);
    for step in steps {
        if let CaptureStep::Capture { steps, .. } = step {
            steps_memory(steps, usage);
        }
    }
}
fn pattern_memory(pattern: &GSubPattern, usage: &mut MemoryUsage) {
    match pattern {
        GSubPattern::Plain(bytes) => usage.add_vec(bytes),
        GSubPattern::NonCapturing(p) => usage.add_vec(&p.steps),
        GSubPattern::Capturing(p) => steps_memory(&p.steps, usage),
        _ => {}
    }
}
fn shrink_steps(steps: &mut Vec<CaptureStep>) {
    steps.shrink_to_fit();
    for step in steps {
        if let CaptureStep::Capture { steps, .. } = step {
            shrink_steps(steps);
        }
    }
}
fn shrink_pattern(pattern: &mut GSubPattern) {
    match pattern {
        GSubPattern::Plain(bytes) => bytes.shrink_to_fit(),
        GSubPattern::NonCapturing(p) => p.steps.shrink_to_fit(),
        GSubPattern::Capturing(p) => shrink_steps(&mut p.steps),
        _ => {}
    }
}
