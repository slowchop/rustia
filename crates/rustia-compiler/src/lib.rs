use rustia_lexer::Span;
use rustia_parser::parse_source_bytes;
use rustia_parser::{
    AssignmentOp, AssignmentTarget, BinaryOp, Chunk, Expr, FunctionParam, GenericForBinding,
    InterpolatedStringPart, Stmt, TableField, UnaryOp,
};
use rustia_vm::bytecode::{
    CallDescriptor, CallDescriptorCall, Constant, Function, Instruction, InstructionSource, Opcode,
    Program, RuntimeType, UpvalueDescriptor, ASSERT_MESSAGE_MISSING_ARGUMENT, ASSERT_MESSAGE_NONE,
    ITERATOR_CALLER_HAS_NO_CAPTURED_LOCALS,
};
use rustia_vm::number::{binary, NumericOp};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileError {
    pub span: Option<Span>,
    pub message: String,
}

impl CompileError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            span: None,
            message: message.into(),
        }
    }

    fn at(message: impl Into<String>, span: Span) -> Self {
        Self {
            span: Some(span),
            message: message.into(),
        }
    }

    fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(span) = self.span {
            write!(
                formatter,
                "compile error at {}..{}: {}",
                span.start, span.end, self.message
            )
        } else {
            write!(formatter, "compile error: {}", self.message)
        }
    }
}

impl std::error::Error for CompileError {}

pub type Result<T> = std::result::Result<T, CompileError>;

mod builtins;
#[doc(hidden)]
pub mod compiled_call;
pub mod compiled_host_call;
mod o2;
mod repeat_condition;

pub use o2::{ExperimentalO2Options, ProgramReceipts};

pub fn compile(chunk: &Chunk) -> Result<Program> {
    compile_with_options(chunk, CompileOptions::default())
}

pub fn compile_with_options(chunk: &Chunk, options: CompileOptions) -> Result<Program> {
    compile_with_options_and_receipts(chunk, options).map(|(program, _)| program)
}

/// Compiles like [`compile_with_options`] and additionally returns the
/// logical-receipt schedule ([`ProgramReceipts`]) the executable tape prices
/// blocks with. With every [`ExperimentalO2Options`] flag off (the default)
/// the receipts are the identity schedule.
pub fn compile_with_options_and_receipts(
    chunk: &Chunk,
    options: CompileOptions,
) -> Result<(Program, ProgramReceipts)> {
    // Capture discovery is intentionally a complete compiler pass. Nested functions can make a
    // source-earlier assignment target captured (including when that assignment executes again at
    // a loop backedge), so a single source-order pass cannot safely decide whether that write must
    // update a shared cell. The second deterministic pass starts with the complete captured-register
    // sets and therefore emits cell write-throughs at every assignment site. This pass also finds
    // syntactic global writes: Luau disables builtin recognition even for a source-later write.
    let experimental_o2 = options.experimental_o2;
    let (capture_hints, written_globals, needs_repeat_check) =
        Compiler::new(options.clone()).discover_capture_hints(chunk)?;
    let mut compiler = Compiler::with_capture_hints(options, capture_hints);
    if needs_repeat_check {
        compiler.repeat_constants = repeat_condition::check(chunk, &compiler.options)?;
    }
    compiler.written_globals = written_globals;
    let program = compiler.compile_chunk(chunk)?;
    if experimental_o2.any_enabled() {
        o2::optimize_program(&program, experimental_o2)
    } else {
        let receipts = ProgramReceipts::identity(&program);
        Ok((program, receipts))
    }
}

fn collect_capture_hints(main: &BTreeSet<u8>, functions: &[Function]) -> Vec<Vec<u8>> {
    let mut capture_hints = Vec::with_capacity(functions.len().saturating_add(1));
    capture_hints.push(main.iter().copied().collect());
    capture_hints.extend(
        functions
            .iter()
            .map(|function| function.captured_registers.clone()),
    );
    capture_hints
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompileOptions {
    pub builtins: BuiltinEnvironment,
    pub source_name: Option<String>,
    pub source_line_starts: Vec<usize>,
    /// PUC Lua 5.1 table-constructor write order (lparser.c `constructor`):
    /// keyed/named fields write immediately at their source position while
    /// positional values are buffered and flushed in `LFIELDS_PER_FLUSH` (50)
    /// batches plus a final flush, so a later positional batch overwrites an
    /// earlier keyed write to the same index. When false (default), positional
    /// fields write in source order like reference Luau (whose compiler flushes
    /// its SETLIST chunk before any keyed field, preserving insertion order).
    /// Set this only for `Profile::Puc51Compat` guests.
    pub puc51_constructor_order: bool,
    /// Default-disabled C2.3 optimization families ([`ExperimentalO2Options`]).
    ///
    /// While every flag is false (the default and the only supported public
    /// configuration), emitted bytecode is byte-identical to a compiler
    /// without this field. Enabling a family changes emitted instructions and
    /// therefore the fuel/deterministic-GC schedule; that is gated on C2.2's
    /// block receipts per `design/58-vm-core2.md`.
    pub experimental_o2: ExperimentalO2Options,
}

impl CompileOptions {
    pub fn with_source(self, source_name: impl Into<String>, source: &str) -> Self {
        self.with_source_bytes(source_name, source.as_bytes())
    }

    pub fn with_source_bytes(mut self, source_name: impl Into<String>, source: &[u8]) -> Self {
        self.source_name = Some(source_name.into());
        self.source_line_starts = source_line_starts(source);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BuiltinEnvironment {
    #[default]
    Public,
    LuauConformance,
}

// Luau attributes a call to the end of its callee, and a namecall to the
// method name. Keep the outer expression available as the host diagnostic span.
fn call_site_span(expr: &Expr) -> Span {
    let start = match expr {
        Expr::Call { callee, .. } => callee.span().end,
        Expr::MethodCall { method_span, .. } => method_span.start,
        _ => return expr.span(),
    };
    Span::new(start, expr.span().end)
}

fn statement_span(statement: &Stmt) -> Span {
    match statement {
        Stmt::Local { span, .. }
        | Stmt::LocalFunction { span, .. }
        | Stmt::Function { span, .. }
        | Stmt::Assignment { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::ForNumeric { span, .. }
        | Stmt::ForGeneric { span, .. }
        | Stmt::Repeat { span, .. }
        | Stmt::Do { span, .. }
        | Stmt::Break { span }
        | Stmt::Continue { span }
        | Stmt::TypeAlias { span, .. }
        | Stmt::TypeFunction { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::Expr { span, .. } => *span,
    }
}

fn runtime_parameter_type(param: &FunctionParam) -> RuntimeType {
    param
        .annotation
        .as_ref()
        .and_then(runtime_type_annotation)
        .unwrap_or(RuntimeType::Any)
}

fn runtime_type_annotation(annotation: &rustia_parser::TypeExpr) -> Option<RuntimeType> {
    use rustia_parser::TypeExpr;

    match annotation {
        TypeExpr::Named { name, .. } => match name.as_str() {
            "any" => Some(RuntimeType::Any),
            "number" => Some(RuntimeType::Number),
            "string" => Some(RuntimeType::String),
            "table" => Some(RuntimeType::Table),
            "thread" => Some(RuntimeType::Thread),
            "userdata" => Some(RuntimeType::Userdata),
            "vector" => Some(RuntimeType::Vector),
            "buffer" => Some(RuntimeType::Buffer),
            _ => None,
        },
        TypeExpr::Nil(_) => Some(RuntimeType::Nil),
        TypeExpr::Table { .. } => Some(RuntimeType::Table),
        TypeExpr::Function { .. } => Some(RuntimeType::Function),
        TypeExpr::Optional { inner, .. } => match runtime_type_annotation(inner) {
            Some(RuntimeType::String) => Some(RuntimeType::OptionalString),
            Some(RuntimeType::Buffer) => Some(RuntimeType::OptionalBuffer),
            Some(RuntimeType::Any) => Some(RuntimeType::Any),
            _ => None,
        },
        _ => None,
    }
}

struct Compiler {
    options: CompileOptions,
    constants: Vec<Constant>,
    functions: Vec<Function>,
    code: Vec<Instruction>,
    locals: BTreeMap<String, Binding>,
    assigned_globals: BTreeSet<String>,
    // Syntactic writes only. Unlike optimization invalidation, Luau's builtin
    // classifier does not include writes through _G aliases or rawset.
    written_globals: BTreeSet<String>,
    assigned_stdlib_fields: BTreeSet<(String, String)>,
    fastcall_candidate_names: BTreeSet<String>,
    stdlib_tables_may_be_mutated: bool,
    global_environment_escaped: bool,
    disable_builtin_fastpaths: bool,
    upvalues: Vec<UpvalueDescriptor>,
    upvalue_names: Vec<String>,
    captured_registers: BTreeSet<u8>,
    capture_hints: Vec<Vec<u8>>,
    capture_stack: Vec<CaptureScope>,
    source_map: Vec<InstructionSource>,
    diagnostic_span: Option<Span>,
    current_function_is_vararg: bool,
    next_register: u16,
    max_registers: u16,
    needs_repeat_check: bool,
    repeat_constants: repeat_condition::CapturedConstants,
    loop_breaks: Vec<Vec<usize>>,
    loop_continues: Vec<Vec<usize>>,
    loop_scope_depths: Vec<usize>,
    scope_register_starts: Vec<u16>,
}

impl Compiler {
    fn new(options: CompileOptions) -> Self {
        Self::with_capture_hints(options, Vec::new())
    }

    fn with_capture_hints(options: CompileOptions, capture_hints: Vec<Vec<u8>>) -> Self {
        let captured_registers = capture_hints
            .first()
            .into_iter()
            .flatten()
            .copied()
            .collect();
        Self {
            options,
            constants: Vec::new(),
            functions: Vec::new(),
            code: Vec::new(),
            locals: BTreeMap::new(),
            assigned_globals: BTreeSet::new(),
            written_globals: BTreeSet::new(),
            assigned_stdlib_fields: BTreeSet::new(),
            fastcall_candidate_names: BTreeSet::new(),
            stdlib_tables_may_be_mutated: false,
            global_environment_escaped: false,
            disable_builtin_fastpaths: false,
            upvalues: Vec::new(),
            upvalue_names: Vec::new(),
            captured_registers,
            capture_hints,
            capture_stack: Vec::new(),
            source_map: Vec::new(),
            diagnostic_span: None,
            current_function_is_vararg: true,
            next_register: 0,
            max_registers: 0,
            needs_repeat_check: false,
            repeat_constants: BTreeMap::new(),
            loop_breaks: Vec::new(),
            loop_continues: Vec::new(),
            loop_scope_depths: Vec::new(),
            scope_register_starts: Vec::new(),
        }
    }

    fn discover_capture_hints(
        mut self,
        chunk: &Chunk,
    ) -> Result<(Vec<Vec<u8>>, BTreeSet<String>, bool)> {
        // Compile every statement and nested prototype so the normal lexical
        // resolver—not a second shadow implementation—discovers captures.
        // Final bytecode packaging and immutable-origin rewriting are skipped.
        self.compile_block(&chunk.statements)?;
        Ok((
            collect_capture_hints(&self.captured_registers, &self.functions),
            self.written_globals,
            self.needs_repeat_check,
        ))
    }

    fn compile_chunk(mut self, chunk: &Chunk) -> Result<Program> {
        self.compile_block(&chunk.statements)?;
        self.emit_instruction(Instruction::new(Opcode::Halt, 0, 0, 0));
        if !self.capture_hints.is_empty()
            && self.capture_hints.len() != self.functions.len().saturating_add(1)
        {
            return Err(CompileError::new(
                "capture discovery changed the function prototype graph",
            ));
        }
        let max_registers = usize::from(self.max_registers.max(1));
        let main_captured_registers = self.captured_registers.iter().copied().collect();
        let main_source_map = self.source_map;
        let mut main = Function::with_params_and_vararg(self.code, max_registers, 0, true)
            .with_source_map(main_source_map);
        main.captured_registers = main_captured_registers;
        mark_immutable_capture_descriptors(&mut main, &mut self.functions);
        Ok(
            Program::with_functions(main, self.constants, self.functions)
                .with_source_name(self.options.source_name),
        )
    }

    fn compile_block(&mut self, statements: &[Stmt]) -> Result<()> {
        for statement in statements {
            self.compile_statement(statement)?;
        }
        Ok(())
    }

    fn compile_scoped_block(&mut self, statements: &[Stmt]) -> Result<()> {
        let locals = self.locals.clone();
        let fastcall_candidate_names = self.fastcall_candidate_names.clone();
        let next_register = self.next_register;
        self.scope_register_starts.push(next_register);
        self.compile_block(statements)?;
        self.clear_local_cells_from(next_register)?;
        self.scope_register_starts
            .pop()
            .expect("scope register stack should be balanced");
        self.locals = locals;
        self.fastcall_candidate_names = fastcall_candidate_names;
        self.next_register = next_register;
        Ok(())
    }

    fn compile_statement(&mut self, statement: &Stmt) -> Result<()> {
        let previous = self.diagnostic_span.replace(statement_span(statement));
        let result = self.compile_statement_inner(statement);
        self.diagnostic_span = previous;
        result
    }

    fn compile_statement_inner(&mut self, statement: &Stmt) -> Result<()> {
        self.record_source_for_next_instruction(statement_span(statement));
        match statement {
            Stmt::Local {
                bindings,
                init,
                span,
            } => self.compile_local(bindings, init, *span),
            Stmt::LocalFunction {
                name,
                params,
                is_vararg,
                body,
                span,
                ..
            } => self.compile_local_function(name, params, *is_vararg, body, *span),
            Stmt::Function {
                target,
                is_method,
                params,
                is_vararg,
                body,
                span,
                ..
            } => {
                self.compile_function_statement(target, *is_method, params, *is_vararg, body, *span)
            }
            Stmt::Assignment {
                targets,
                op,
                values,
                span,
            } => self.compile_assignment(targets, *op, values, *span),
            Stmt::If {
                condition,
                then_body,
                else_body,
                span,
            } => self.compile_if(condition, then_body, else_body, *span),
            Stmt::While {
                condition,
                body,
                span,
            } => self.compile_while(condition, body, *span),
            Stmt::ForNumeric {
                variable,
                start,
                limit,
                step,
                body,
                span,
                ..
            } => self.compile_numeric_for(variable, start, limit, step.as_ref(), body, *span),
            Stmt::ForGeneric {
                variables,
                iter,
                body,
                span,
            } => self.compile_generic_for(variables, iter, body, *span),
            Stmt::Repeat {
                body,
                condition,
                span,
            } => self.compile_repeat(body, condition, *span),
            Stmt::Do { body, .. } => self.compile_scoped_block(body),
            Stmt::Break { span } => self.compile_break(*span),
            Stmt::Continue { span } => self.compile_continue(*span),
            Stmt::TypeAlias { .. } | Stmt::TypeFunction { .. } => Ok(()),
            Stmt::Return {
                value,
                extra_values,
                span,
            } => self.compile_return(value.as_ref(), extra_values, *span),
            Stmt::Expr { expr, .. } => self.compile_expr_statement(expr),
        }
    }

    fn compile_return(
        &mut self,
        value: Option<&Expr>,
        extra_values: &[Expr],
        span: Span,
    ) -> Result<()> {
        let Some(first_expr) = value else {
            self.emit_instruction(Instruction::new(Opcode::Return, 0, 0, 0));
            return Ok(());
        };

        let result_count = extra_values
            .len()
            .checked_add(1)
            .ok_or_else(|| CompileError::at("return list is too large", span))?;
        if result_count > u8::MAX as usize {
            return Err(CompileError::at("return list is too large", span));
        }
        let last_expr = extra_values.last().unwrap_or(first_expr);
        if self.expr_can_expand(last_expr) {
            let prefix_count = result_count - 1;
            let prefix_first = if prefix_count == 0 {
                0
            } else {
                let prefix_first = self.alloc_register_window_at(prefix_count, span)?;
                self.compile_expr_to(first_expr, prefix_first)?;
                for (index, expr) in extra_values[..extra_values.len().saturating_sub(1)]
                    .iter()
                    .enumerate()
                {
                    let register = checked_register_offset(prefix_first, index + 1, span)?;
                    self.compile_expr_to(expr, register)?;
                }
                prefix_first
            };
            self.compile_return_expand(
                last_expr,
                prefix_first,
                prefix_count,
                call_site_span(last_expr),
            )?;
            return Ok(());
        }
        let first_register = self.alloc_register_window_at(result_count, span)?;
        self.compile_expr_to(first_expr, first_register)?;
        for (index, expr) in extra_values.iter().enumerate() {
            let register = first_register
                .checked_add(
                    u8::try_from(index + 1)
                        .map_err(|_| CompileError::at("return list is too large", span))?,
                )
                .ok_or_else(|| CompileError::at("return register window overflow", span))?;
            self.compile_expr_to(expr, register)?;
        }
        self.emit_instruction(Instruction::new(
            Opcode::Return,
            first_register,
            u8::try_from(result_count)
                .map_err(|_| CompileError::at("return list is too large", span))?,
            0,
        ));
        Ok(())
    }

    fn compile_local(
        &mut self,
        bindings: &[rustia_parser::LocalBinding],
        init: &[Expr],
        span: Span,
    ) -> Result<()> {
        // Type syntax is parsed and retained in the AST, then erased at runtime per ADR §2-F.
        let mut registers = Vec::with_capacity(bindings.len());
        for _ in bindings {
            registers.push(self.alloc_register_at(span)?);
        }
        for register in registers.iter().copied() {
            self.clear_local_cell(register);
        }

        let mut handled_init = false;
        if let Some(last_index) = init.len().checked_sub(1) {
            if init.len() <= registers.len() && self.expr_can_expand(&init[last_index]) {
                for (index, expr) in init[..last_index].iter().enumerate() {
                    self.compile_expr_to(expr, registers[index])?;
                }
                let result_count = registers.len() - last_index;
                let expanded = self.compile_expand_multi_to(
                    &init[last_index],
                    registers[last_index],
                    result_count,
                )?;
                debug_assert!(expanded);
                handled_init = true;
            }
        }

        if !handled_init {
            for (index, register) in registers.iter().copied().enumerate() {
                if let Some(expr) = init.get(index) {
                    self.compile_expr_to(expr, register)?;
                } else {
                    self.load_constant_to(register, Constant::Nil)?;
                }
            }
            for expr in init.iter().skip(registers.len()) {
                let register = self.compile_expr(expr)?;
                self.free_register(register);
            }
        }
        for (index, binding) in bindings.iter().enumerate() {
            if init
                .get(index)
                .is_some_and(|expr| self.expr_is_fastcall_candidate(expr))
            {
                self.fastcall_candidate_names.insert(binding.name.clone());
            } else {
                self.fastcall_candidate_names.remove(&binding.name);
            }
        }
        for (binding, register) in bindings.iter().zip(registers) {
            self.locals
                .insert(binding.name.clone(), Binding::Register(register));
        }
        Ok(())
    }

    fn compile_local_function(
        &mut self,
        name: &str,
        params: &[FunctionParam],
        is_vararg: bool,
        body: &[Stmt],
        span: Span,
    ) -> Result<()> {
        let register = self.alloc_register_at(span)?;
        self.clear_local_cell(register);
        self.fastcall_candidate_names.remove(name);
        self.locals
            .insert(name.to_owned(), Binding::Register(register));
        let prototype = self.compile_function_prototype(
            params,
            is_vararg,
            body,
            Some(name),
            Some(name.to_owned()),
            span,
        )?;
        self.emit_instruction(Instruction::with_aux(
            Opcode::LoadFunction,
            register,
            0,
            0,
            i32::try_from(prototype)
                .map_err(|_| CompileError::at("too many function prototypes", span))?,
        ));
        // A nested closure compiled as part of this initializer can already capture the local
        // function (for example, `f` returns a closure that refers to `f`). In that case
        // LoadFunction must publish the just-created value into the cell it captured. A capture
        // hinted only by a source-later sibling remains an immutable initialization and is
        // captured lazily when that sibling is created.
        if matches!(
            self.locals.get(name),
            Some(Binding::CapturedLocal(captured)) if *captured == register
        ) {
            self.emit_instruction(Instruction::new(
                Opcode::SetLocalCell,
                register,
                register,
                0,
            ));
        }
        Ok(())
    }

    fn compile_function_statement(
        &mut self,
        target: &AssignmentTarget,
        is_method: bool,
        params: &[FunctionParam],
        is_vararg: bool,
        body: &[Stmt],
        span: Span,
    ) -> Result<()> {
        if let AssignmentTarget::Local { name, .. } = target {
            if !self.name_is_lexically_bound(name) {
                self.written_globals.insert(name.clone());
            }
        }
        let mut method_params;
        let effective_params = if is_method {
            method_params = Vec::with_capacity(params.len() + 1);
            method_params.push(FunctionParam {
                name: "self".to_owned(),
                annotation: None,
                span,
            });
            method_params.extend(params.iter().cloned());
            method_params.as_slice()
        } else {
            params
        };

        let self_name = if let AssignmentTarget::Local { name, .. } = target {
            self.locals.contains_key(name).then_some(name.as_str())
        } else {
            None
        };
        if let AssignmentTarget::Local { name, .. } = target {
            if !self.locals.contains_key(name) {
                self.assigned_globals.insert(name.clone());
            }
        }
        if let Some((table_name, field_name)) = self.assignment_target_stdlib_field(target) {
            self.assigned_stdlib_fields.insert((table_name, field_name));
        }
        let debug_name = self.function_assignment_debug_name(target);
        let prototype = self.compile_function_prototype(
            effective_params,
            is_vararg,
            body,
            self_name,
            debug_name,
            span,
        )?;
        let prototype = i32::try_from(prototype)
            .map_err(|_| CompileError::at("too many function prototypes", span))?;

        let target = self.compile_assignment_target(target, true)?;
        match target {
            CompiledAssignmentTarget::Local(register) => {
                self.emit_instruction(Instruction::with_aux(
                    Opcode::LoadFunction,
                    register,
                    0,
                    0,
                    prototype,
                ));
                if self.local_register_is_captured(register) {
                    self.emit_instruction(Instruction::new(
                        Opcode::SetLocalCell,
                        register,
                        register,
                        0,
                    ));
                }
            }
            CompiledAssignmentTarget::CapturedLocal(register) => {
                self.emit_instruction(Instruction::with_aux(
                    Opcode::LoadFunction,
                    register,
                    0,
                    0,
                    prototype,
                ));
                self.emit_instruction(Instruction::new(
                    Opcode::SetLocalCell,
                    register,
                    register,
                    0,
                ));
            }
            CompiledAssignmentTarget::Upvalue(index) => {
                let function_register = self.alloc_register_at(span)?;
                self.emit_instruction(Instruction::with_aux(
                    Opcode::LoadFunction,
                    function_register,
                    0,
                    0,
                    prototype,
                ));
                self.emit_instruction(Instruction::with_aux(
                    Opcode::SetUpvalue,
                    function_register,
                    0,
                    0,
                    i32::from(index),
                ));
                self.free_register(function_register);
            }
            CompiledAssignmentTarget::Table {
                table,
                key,
                global_field,
                stdlib_field,
                ..
            } => {
                let function_register = self.alloc_register_at(span)?;
                self.emit_instruction(Instruction::with_aux(
                    Opcode::LoadFunction,
                    function_register,
                    0,
                    0,
                    prototype,
                ));
                self.emit_source_mapped_instruction(
                    table_set_instruction(table, key, function_register),
                    span,
                );
                if let Some(name) = global_field {
                    self.assigned_globals.insert(name);
                }
                if let Some((table_name, field_name)) = stdlib_field {
                    self.assigned_stdlib_fields.insert((table_name, field_name));
                }
                self.free_register(function_register);
                if let Some(key) = key.register() {
                    self.free_register(key);
                }
                self.free_register(table);
            }
            CompiledAssignmentTarget::Global { key, name } => {
                let function_register = self.alloc_register_at(span)?;
                self.emit_instruction(Instruction::with_aux(
                    Opcode::LoadFunction,
                    function_register,
                    0,
                    0,
                    prototype,
                ));
                self.emit_instruction(Instruction::with_aux(
                    Opcode::SetGlobal,
                    function_register,
                    0,
                    0,
                    key,
                ));
                self.assigned_globals.insert(name);
                self.free_register(function_register);
            }
        }
        Ok(())
    }

    fn compile_assignment(
        &mut self,
        targets: &[AssignmentTarget],
        op: AssignmentOp,
        values: &[Expr],
        span: Span,
    ) -> Result<()> {
        for target in targets {
            if let AssignmentTarget::Local { name, .. } = target {
                if !self.name_is_lexically_bound(name) {
                    self.written_globals.insert(name.clone());
                }
            }
        }
        if targets.is_empty() {
            return Err(CompileError::at("assignment has no targets", span));
        }

        if op != AssignmentOp::Assign {
            if targets.len() != 1 || values.len() != 1 {
                return Err(CompileError::at(
                    "compound assignment requires one target and one value",
                    span,
                ));
            }
            let target = self.compile_assignment_target(&targets[0], false)?;
            let reusable_string_key = match &target {
                CompiledAssignmentTarget::Table {
                    key: CompiledTableKey::StringConstant { .. },
                    ..
                } => {
                    let register = self.alloc_register_at(span)?;
                    debug_assert_ne!(register, 0, "register zero is the no-key-output sentinel");
                    Some(register)
                }
                _ => None,
            };
            let result = self.alloc_register_at(span)?;
            match &target {
                CompiledAssignmentTarget::Local(register) => {
                    if self.local_register_is_captured(*register) {
                        self.emit_instruction(Instruction::new(
                            Opcode::GetLocalCell,
                            result,
                            *register,
                            0,
                        ));
                    } else {
                        self.code
                            .push(Instruction::new(Opcode::Move, result, *register, 0));
                    }
                }
                CompiledAssignmentTarget::CapturedLocal(register) => {
                    self.code
                        .push(Instruction::new(Opcode::GetLocalCell, result, *register, 0));
                }
                CompiledAssignmentTarget::Upvalue(index) => {
                    self.emit_instruction(Instruction::with_aux(
                        Opcode::GetUpvalue,
                        result,
                        0,
                        0,
                        i32::from(*index),
                    ));
                }
                CompiledAssignmentTarget::Table {
                    table,
                    key,
                    key_span,
                    ..
                } => {
                    let instruction = match (*key, reusable_string_key) {
                        (
                            CompiledTableKey::StringConstant { index: key, .. },
                            Some(key_register),
                        ) => {
                            // Compound assignment reuses the loaded key for the later SETINDEX,
                            // matching the unfused LoadConst + GetIndex + SetIndex fuel schedule.
                            Instruction::with_aux(
                                Opcode::GetTableKs,
                                result,
                                *table,
                                key_register,
                                key,
                            )
                        }
                        _ => table_get_instruction(result, *table, *key),
                    };
                    self.emit_source_mapped_instruction(instruction, *key_span);
                }
                CompiledAssignmentTarget::Global { key, .. } => {
                    self.code
                        .push(Instruction::with_aux(Opcode::GetGlobal, result, 0, 0, *key));
                }
            }
            let rhs_register = self.compile_expr(&values[0])?;
            self.emit_instruction(Instruction::new(
                opcode_for_assignment(op),
                result,
                result,
                rhs_register,
            ));
            match &target {
                CompiledAssignmentTarget::Local(register) => {
                    if self.local_register_is_captured(*register) {
                        if result != *register {
                            self.code
                                .push(Instruction::new(Opcode::Move, *register, result, 0));
                        }
                        self.emit_instruction(Instruction::new(
                            Opcode::SetLocalCell,
                            result,
                            *register,
                            0,
                        ));
                    } else {
                        self.code
                            .push(Instruction::new(Opcode::Move, *register, result, 0));
                    }
                }
                CompiledAssignmentTarget::CapturedLocal(register) => {
                    self.code
                        .push(Instruction::new(Opcode::Move, *register, result, 0));
                    self.code
                        .push(Instruction::new(Opcode::SetLocalCell, result, *register, 0));
                }
                CompiledAssignmentTarget::Upvalue(index) => {
                    self.emit_instruction(Instruction::with_aux(
                        Opcode::SetUpvalue,
                        result,
                        0,
                        0,
                        i32::from(*index),
                    ));
                }
                CompiledAssignmentTarget::Table {
                    table,
                    key,
                    key_span,
                    ..
                } => {
                    let instruction = if let Some(key_register) = reusable_string_key {
                        Instruction::new(Opcode::SetIndex, *table, key_register, result)
                    } else {
                        table_set_instruction(*table, *key, result)
                    };
                    self.emit_source_mapped_instruction(instruction, *key_span);
                }
                CompiledAssignmentTarget::Global { key, .. } => {
                    self.code
                        .push(Instruction::with_aux(Opcode::SetGlobal, result, 0, 0, *key));
                }
            }
            self.mark_assignment_target_mutations(&target);
            self.free_register(rhs_register);
            self.free_register(result);
            if let Some(key_register) = reusable_string_key {
                self.free_register(key_register);
            }
            match target {
                CompiledAssignmentTarget::Local(_) => {}
                CompiledAssignmentTarget::CapturedLocal(_) => {}
                CompiledAssignmentTarget::Upvalue(_) => {}
                CompiledAssignmentTarget::Table { table, key, .. } => {
                    if let Some(key) = key.register() {
                        self.free_register(key);
                    }
                    self.free_register(table);
                }
                CompiledAssignmentTarget::Global { .. } => {}
            }
            return Ok(());
        }

        let mut compiled_targets = Vec::with_capacity(targets.len());
        for target in targets {
            compiled_targets.push(self.compile_assignment_target(target, true)?);
        }
        for target in &compiled_targets {
            self.mark_assignment_target_mutations(target);
        }

        if let Some(last_index) = values.len().checked_sub(1) {
            if values.len() <= compiled_targets.len() && self.expr_can_expand(&values[last_index]) {
                let mut value_registers = Vec::with_capacity(last_index);
                for value in &values[..last_index] {
                    value_registers.push(self.compile_expr(value)?);
                }
                let result_count = compiled_targets.len() - last_index;
                let first_result = self.alloc_register_window_at(result_count, span)?;
                let expanded =
                    self.compile_expand_multi_to(&values[last_index], first_result, result_count)?;
                debug_assert!(expanded);

                for (target, source) in compiled_targets.iter().zip(value_registers.iter().copied())
                {
                    self.write_assignment_target_from_source(target, source);
                }
                for (index, target) in compiled_targets.iter().enumerate() {
                    if index < last_index {
                        continue;
                    }
                    let result_index = index - last_index;
                    let source = checked_register_offset(first_result, result_index, span)?;
                    self.write_assignment_target_from_source(target, source);
                }
                self.free_registers_back_to(u16::from(first_result));
                for register in value_registers.into_iter().rev() {
                    self.free_register(register);
                }
                for target in compiled_targets.into_iter().rev() {
                    self.free_assignment_target(target);
                }
                return Ok(());
            }
        }

        let mut value_registers = Vec::with_capacity(values.len());
        for value in values {
            value_registers.push(self.compile_expr(value)?);
        }

        for (index, target) in compiled_targets.iter().enumerate() {
            let missing_value_register;
            let source = if let Some(source) = value_registers.get(index).copied() {
                source
            } else {
                missing_value_register = self.load_constant(Constant::Nil)?;
                missing_value_register
            };

            self.write_assignment_target_from_source(target, source);
            if value_registers.get(index).is_none() {
                self.free_register(source);
            }
        }

        for register in value_registers.into_iter().rev() {
            self.free_register(register);
        }
        for target in compiled_targets.into_iter().rev() {
            self.free_assignment_target(target);
        }
        Ok(())
    }

    fn compile_assignment_target(
        &mut self,
        target: &AssignmentTarget,
        preload_constant_key: bool,
    ) -> Result<CompiledAssignmentTarget> {
        match target {
            AssignmentTarget::Local { name, span } => {
                if let Some(binding) = self.locals.get(name).copied() {
                    match binding {
                        Binding::Register(register) => {
                            return Ok(CompiledAssignmentTarget::Local(register));
                        }
                        Binding::CapturedLocal(register) => {
                            return Ok(CompiledAssignmentTarget::CapturedLocal(register));
                        }
                        Binding::Upvalue(index) => {
                            return Ok(CompiledAssignmentTarget::Upvalue(index));
                        }
                        Binding::CurrentFunction => {
                            return Err(CompileError::at(
                                "cannot assign to function self binding",
                                *span,
                            ));
                        }
                    };
                }
                if let Some(index) = self.resolve_upvalue(name)? {
                    return Ok(CompiledAssignmentTarget::Upvalue(index));
                }
                Ok(CompiledAssignmentTarget::Global {
                    key: self.global_key_index(name)?,
                    name: name.clone(),
                })
            }
            AssignmentTarget::Field {
                object,
                field,
                field_span,
                span,
            } => {
                let global_field = self.global_table_field_assignment(object, field);
                let table = self.compile_expr(object)?;
                let key = self.compiled_string_table_key(
                    field.as_bytes(),
                    *field_span,
                    preload_constant_key,
                )?;
                Ok(CompiledAssignmentTarget::Table {
                    table,
                    key,
                    key_span: *span,
                    global_field,
                    stdlib_field: self.stdlib_table_field_assignment(object, field),
                })
            }
            AssignmentTarget::Index { object, index, .. } => {
                let global_field = self.global_table_index_assignment(object, index);
                let stdlib_field = self.stdlib_table_index_assignment(object, index);
                let table = self.compile_expr(object)?;
                let key = if let Some(Constant::String(bytes)) = const_eval(index) {
                    self.compiled_string_table_key(&bytes, index.span(), preload_constant_key)?
                } else {
                    CompiledTableKey::Register(self.compile_expr(index)?)
                };
                Ok(CompiledAssignmentTarget::Table {
                    table,
                    key,
                    key_span: index.span(),
                    global_field,
                    stdlib_field,
                })
            }
        }
    }

    fn function_assignment_debug_name(&self, target: &AssignmentTarget) -> Option<String> {
        match target {
            AssignmentTarget::Local { name, .. } => Some(name.clone()),
            AssignmentTarget::Field { field, .. } => Some(field.clone()),
            AssignmentTarget::Index { index, .. } => match index.as_ref() {
                Expr::String(bytes, _) => String::from_utf8(bytes.clone()).ok(),
                _ => None,
            },
        }
    }

    fn compile_if(
        &mut self,
        condition: &Expr,
        then_body: &[Stmt],
        else_body: &[Stmt],
        span: Span,
    ) -> Result<()> {
        let condition_register = self.compile_expr(condition)?;
        let false_jump = self.code.len();
        self.emit_instruction(Instruction::with_aux(
            Opcode::JumpIfNot,
            condition_register,
            0,
            0,
            0,
        ));
        self.free_register(condition_register);

        self.compile_scoped_block(then_body)?;
        if else_body.is_empty() {
            self.patch_jump_to_here(false_jump, span)?;
            return Ok(());
        }

        let end_jump = self.code.len();
        self.code
            .push(Instruction::with_aux(Opcode::Jump, 0, 0, 0, 0));
        self.patch_jump_to_here(false_jump, span)?;
        self.compile_scoped_block(else_body)?;
        self.patch_jump_to_here(end_jump, span)
    }

    fn compile_while(&mut self, condition: &Expr, body: &[Stmt], span: Span) -> Result<()> {
        let loop_start = self.code.len();
        let condition_register = self.compile_expr(condition)?;
        let false_jump = self.code.len();
        self.emit_instruction(Instruction::with_aux(
            Opcode::JumpIfNot,
            condition_register,
            0,
            0,
            0,
        ));
        self.free_register(condition_register);

        self.loop_breaks.push(Vec::new());
        self.loop_continues.push(Vec::new());
        self.loop_scope_depths
            .push(self.scope_register_starts.len());
        self.compile_scoped_block(body)?;
        self.patch_loop_continues_to(loop_start, span)?;
        let loop_start = self.jump_target(loop_start, span)?;
        self.code
            .push(Instruction::with_aux(Opcode::Jump, 0, 0, 0, loop_start));
        let loop_end = self.code.len();
        self.patch_jump_to(false_jump, loop_end, span)?;
        self.patch_loop_breaks_to(loop_end, span)
    }

    fn compile_numeric_for(
        &mut self,
        variable: &str,
        start: &Expr,
        limit: &Expr,
        step: Option<&Expr>,
        body: &[Stmt],
        span: Span,
    ) -> Result<()> {
        let locals = self.locals.clone();
        let fastcall_candidate_names = self.fastcall_candidate_names.clone();
        let next_register = self.next_register;

        let index_register = self.alloc_register_at(span)?;
        let limit_register = self.alloc_register_at(span)?;
        let step_register = self.alloc_register_at(span)?;
        let visible_register = self.alloc_register_at(span)?;

        self.compile_expr_to(start, index_register)?;
        self.compile_expr_to(limit, limit_register)?;
        if let Some(step) = step {
            self.compile_expr_to(step, step_register)?;
        } else {
            self.load_constant_to(step_register, Constant::Number(1.0))?;
        }
        self.emit_source_mapped_instruction(
            Instruction::new(Opcode::CoerceForNumber, index_register, index_register, 0),
            start.span(),
        );
        self.emit_source_mapped_instruction(
            Instruction::new(Opcode::CoerceForNumber, limit_register, limit_register, 1),
            limit.span(),
        );
        self.emit_source_mapped_instruction(
            Instruction::new(Opcode::CoerceForNumber, step_register, step_register, 2),
            step.map_or(span, Expr::span),
        );

        self.fastcall_candidate_names.remove(variable);
        self.locals
            .insert(variable.to_owned(), Binding::Register(visible_register));

        let prep_jump = self.code.len();
        self.emit_source_mapped_instruction(
            Instruction::with_aux(Opcode::ForPrep, index_register, 0, 0, 0),
            span,
        );
        let body_start = self.code.len();
        self.loop_breaks.push(Vec::new());
        self.loop_continues.push(Vec::new());
        self.loop_scope_depths
            .push(self.scope_register_starts.len());
        self.compile_scoped_block(body)?;
        let continue_target = self.code.len();
        self.patch_loop_continues_to(continue_target, span)?;
        let body_start = self.jump_target(body_start, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(Opcode::ForLoop, index_register, 0, 0, body_start),
            span,
        );

        let clear_start = self.code.len();
        self.clear_local_cells_from(next_register)?;
        self.patch_jump_to(prep_jump, clear_start, span)?;
        self.patch_loop_breaks_to(clear_start, span)?;
        self.locals = locals;
        self.fastcall_candidate_names = fastcall_candidate_names;
        self.next_register = next_register;
        Ok(())
    }

    fn compile_repeat(&mut self, body: &[Stmt], condition: &Expr, span: Span) -> Result<()> {
        let locals = self.locals.clone();
        let fastcall_candidate_names = self.fastcall_candidate_names.clone();
        let next_register = self.next_register;
        let loop_start = self.code.len();

        self.loop_breaks.push(Vec::new());
        self.loop_continues.push(Vec::new());
        self.loop_scope_depths
            .push(self.scope_register_starts.len());
        self.compile_block(body)?;
        if self
            .loop_continues
            .last()
            .is_some_and(|jumps| !jumps.is_empty())
        {
            self.needs_repeat_check |= body
                .iter()
                .any(|s| matches!(s, Stmt::Local { .. } | Stmt::LocalFunction { .. }));
        }
        let condition_start = self.code.len();
        self.patch_loop_continues_to(condition_start, span)?;
        if let Some(constants) = self.repeat_constants.remove(&condition.span().start) {
            for (name, value) in constants {
                if let Some(Binding::Register(register) | Binding::CapturedLocal(register)) =
                    self.locals.get(&name).copied()
                {
                    self.clear_local_cell(register);
                    self.load_constant_to(register, value)?;
                }
            }
        }
        let condition_register = self.compile_expr(condition)?;
        let loop_start = self.jump_target(loop_start, span)?;
        self.emit_instruction(Instruction::with_aux(
            Opcode::JumpIfNot,
            condition_register,
            0,
            0,
            loop_start,
        ));
        self.free_register(condition_register);

        let clear_start = self.code.len();
        self.clear_local_cells_from(next_register)?;
        self.patch_loop_breaks_to(clear_start, span)?;
        self.locals = locals;
        self.fastcall_candidate_names = fastcall_candidate_names;
        self.next_register = next_register;
        Ok(())
    }

    fn compile_break(&mut self, span: Span) -> Result<()> {
        if self.loop_breaks.is_empty() {
            return Err(CompileError::at("break outside loop", span));
        }
        self.clear_active_loop_scopes()?;
        let jump = self.code.len();
        self.code
            .push(Instruction::with_aux(Opcode::Jump, 0, 0, 0, 0));
        self.loop_breaks
            .last_mut()
            .expect("loop break stack was checked above")
            .push(jump);
        Ok(())
    }

    fn compile_continue(&mut self, span: Span) -> Result<()> {
        if self.loop_continues.is_empty() {
            return Err(CompileError::at("continue outside loop", span));
        }
        self.clear_active_loop_scopes()?;
        let jump = self.code.len();
        self.code
            .push(Instruction::with_aux(Opcode::Jump, 0, 0, 0, 0));
        self.loop_continues
            .last_mut()
            .expect("loop continue stack was checked above")
            .push(jump);
        Ok(())
    }

    fn compile_expr_statement(&mut self, expr: &Expr) -> Result<()> {
        if self.options.builtins == BuiltinEnvironment::LuauConformance {
            if let Expr::Call { callee, args, .. } = expr {
                if matches!(callee.as_ref(), Expr::Name(name, _) if name == "print")
                    && self.builtin_global_is_unmodified("print")
                {
                    self.compile_silenced_print(args)?;
                    return Ok(());
                }
            }
        }

        let register = self.compile_expr(expr)?;
        self.free_register(register);
        Ok(())
    }

    fn compile_silenced_print(&mut self, args: &[Expr]) -> Result<()> {
        for arg in args {
            let register = self.compile_expr(arg)?;
            self.free_register(register);
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<u8> {
        let target = self.alloc_register_at(expr.span())?;
        self.compile_expr_to(expr, target)?;
        Ok(target)
    }

    fn compile_expr_with_debug_name(
        &mut self,
        expr: &Expr,
        debug_name: Option<String>,
    ) -> Result<u8> {
        let target = self.alloc_register_at(expr.span())?;
        self.compile_expr_to_with_debug_name(expr, target, debug_name)?;
        Ok(target)
    }

    fn compile_expr_to_with_debug_name(
        &mut self,
        expr: &Expr,
        target: u8,
        debug_name: Option<String>,
    ) -> Result<()> {
        if let Expr::Function {
            params,
            is_vararg,
            body,
            ..
        } = expr
        {
            let prototype = self.compile_function_prototype(
                params,
                *is_vararg,
                body,
                None,
                debug_name,
                expr.span(),
            )?;
            self.emit_instruction(Instruction::with_aux(
                Opcode::LoadFunction,
                target,
                0,
                0,
                i32::try_from(prototype)
                    .map_err(|_| CompileError::at("too many function prototypes", expr.span()))?,
            ));
            Ok(())
        } else {
            self.compile_expr_to(expr, target)
        }
    }

    fn compile_expr_to(&mut self, expr: &Expr, target: u8) -> Result<()> {
        let previous = self.diagnostic_span.replace(expr.span());
        let result = self.compile_expr_to_inner(expr, target);
        self.diagnostic_span = previous;
        result
    }

    fn compile_expr_to_inner(&mut self, expr: &Expr, target: u8) -> Result<()> {
        if let Some(value) = const_eval(expr) {
            self.load_constant_to(target, value)
                .map_err(|error| error.with_span(expr.span()))?;
            return Ok(());
        }

        match expr {
            Expr::Nil(_) | Expr::Bool(_, _) | Expr::Number(_, _) | Expr::String(_, _) => {
                unreachable!("constant expressions are handled above")
            }
            Expr::InterpolatedString {
                parts,
                expressions,
                span,
            } => self.compile_interpolated_string_to(parts, expressions, target, *span),
            Expr::Varargs(span) => self.compile_varargs_to_count(target, 1, *span),
            Expr::Name(name, _) => {
                if let Some(binding) = self.locals.get(name).copied() {
                    match binding {
                        Binding::Register(source) => {
                            if self.local_register_is_captured(source) {
                                self.emit_instruction(Instruction::new(
                                    Opcode::GetLocalCell,
                                    target,
                                    source,
                                    0,
                                ));
                            } else if source != target {
                                self.code
                                    .push(Instruction::new(Opcode::Move, target, source, 0));
                            }
                        }
                        Binding::CapturedLocal(source) => {
                            self.emit_instruction(Instruction::new(
                                Opcode::GetLocalCell,
                                target,
                                source,
                                0,
                            ));
                        }
                        Binding::Upvalue(index) => {
                            self.emit_instruction(Instruction::with_aux(
                                Opcode::GetUpvalue,
                                target,
                                0,
                                0,
                                i32::from(index),
                            ));
                        }
                        Binding::CurrentFunction => {
                            self.emit_instruction(Instruction::new(
                                Opcode::LoadCurrentFunction,
                                target,
                                0,
                                0,
                            ));
                        }
                    }
                } else if let Some(index) = self.resolve_upvalue(name)? {
                    self.emit_instruction(Instruction::with_aux(
                        Opcode::GetUpvalue,
                        target,
                        0,
                        0,
                        i32::from(index),
                    ));
                } else {
                    let key = self.global_key_index(name)?;
                    self.code
                        .push(Instruction::with_aux(Opcode::GetGlobal, target, 0, 0, key));
                }
                Ok(())
            }
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                ..
            } => {
                self.compile_expr_to(expr, target)?;
                self.code
                    .push(Instruction::new(Opcode::Neg, target, target, 0));
                Ok(())
            }
            Expr::Unary {
                op: UnaryOp::Not,
                expr,
                ..
            } => {
                self.compile_expr_to(expr, target)?;
                self.code
                    .push(Instruction::new(Opcode::Not, target, target, 0));
                Ok(())
            }
            Expr::Unary {
                op: UnaryOp::Len,
                expr,
                ..
            } => {
                self.compile_expr_to(expr, target)?;
                self.code
                    .push(Instruction::new(Opcode::Len, target, target, 0));
                Ok(())
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                if *op == BinaryOp::And {
                    return self.compile_logical_and(lhs, rhs, target, expr.span());
                }
                if *op == BinaryOp::Or {
                    return self.compile_logical_or(lhs, rhs, target, expr.span());
                }
                self.compile_expr_to(lhs, target)?;
                let rhs_register = self.compile_expr(rhs)?;
                self.emit_source_mapped_instruction(
                    Instruction::new(opcode_for_binary(*op), target, target, rhs_register),
                    expr.span(),
                );
                self.free_register(rhs_register);
                Ok(())
            }
            Expr::IfElse {
                condition,
                then_expr,
                else_expr,
                ..
            } => self.compile_if_expression(condition, then_expr, else_expr, target, expr.span()),
            Expr::Field {
                object,
                field,
                field_span,
                ..
            } => self.compile_field_to(object, field, *field_span, target),
            Expr::Index { object, index, .. } => {
                self.compile_expr_to(object, target)?;
                if let Some(Constant::String(bytes)) = const_eval(index) {
                    let key = self.string_constant_index(&bytes, index.span())?;
                    self.emit_source_mapped_instruction(
                        Instruction::with_aux(Opcode::GetTableKs, target, target, 0, key),
                        index.span(),
                    );
                } else {
                    let index_register = self.compile_expr(index)?;
                    self.emit_instruction(Instruction::new(
                        Opcode::GetIndex,
                        target,
                        target,
                        index_register,
                    ));
                    self.free_register(index_register);
                }
                Ok(())
            }
            Expr::Table { fields, .. } => self.compile_table_to(fields, target),
            Expr::Function {
                params,
                is_vararg,
                body,
                ..
            } => {
                let prototype = self.compile_function_prototype(
                    params,
                    *is_vararg,
                    body,
                    None,
                    None,
                    expr.span(),
                )?;
                self.emit_instruction(Instruction::with_aux(
                    Opcode::LoadFunction,
                    target,
                    0,
                    0,
                    i32::try_from(prototype).map_err(|_| {
                        CompileError::at("too many function prototypes", expr.span())
                    })?,
                ));
                Ok(())
            }
            Expr::Paren { expr, .. } | Expr::TypeInstantiation { expr, .. } => {
                self.compile_expr_to(expr, target)
            }
            Expr::Call { callee, args, .. } => {
                self.compile_call_to(callee, args, target, call_site_span(expr))
            }
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => self.compile_method_call_to(object, method, args, target, call_site_span(expr)),
        }
    }

    fn compile_interpolated_string_to(
        &mut self,
        parts: &[InterpolatedStringPart],
        expressions: &[Expr],
        target: u8,
        span: Span,
    ) -> Result<()> {
        let mut has_value = false;
        for part in parts {
            let part_register = if has_value {
                self.alloc_register_at(span)?
            } else {
                target
            };
            match part {
                InterpolatedStringPart::Text(text) => {
                    if text.is_empty() && has_value {
                        continue;
                    }
                    self.load_constant_to(part_register, Constant::String(text.clone()))
                        .map_err(|error| error.with_span(span))?;
                }
                InterpolatedStringPart::Expr(index) => {
                    let expr = expressions.get(*index).ok_or_else(|| {
                        CompileError::at("interpolated string expression is missing", span)
                    })?;
                    self.compile_expr_to(expr, part_register)?;
                    self.emit_source_mapped_instruction(
                        Instruction::new(Opcode::ToString, part_register, part_register, 0),
                        expr.span(),
                    );
                }
            }
            if has_value {
                self.emit_source_mapped_instruction(
                    Instruction::new(Opcode::Concat, target, target, part_register),
                    span,
                );
                self.free_register(part_register);
            } else {
                has_value = true;
            }
        }
        if !has_value {
            self.load_constant_to(target, Constant::String(Vec::new()))
                .map_err(|error| error.with_span(span))?;
        }
        Ok(())
    }

    fn compile_function_prototype(
        &mut self,
        params: &[FunctionParam],
        is_vararg: bool,
        body: &[Stmt],
        self_name: Option<&str>,
        debug_name: Option<String>,
        span: Span,
    ) -> Result<usize> {
        let prototype = self.functions.len();
        self.functions.push(Function::new(Vec::new(), 1));
        let debug_line_defined = self.source_line(span);

        let parent_code = std::mem::take(&mut self.code);
        let parent_source_map = std::mem::take(&mut self.source_map);
        let parent_scope = CaptureScope {
            locals: std::mem::take(&mut self.locals),
            fastcall_candidate_names: std::mem::take(&mut self.fastcall_candidate_names),
            upvalues: std::mem::take(&mut self.upvalues),
            upvalue_names: std::mem::take(&mut self.upvalue_names),
            captured_registers: std::mem::take(&mut self.captured_registers),
        };
        let parent_assigned_globals = self.assigned_globals.clone();
        let parent_assigned_stdlib_fields = self.assigned_stdlib_fields.clone();
        let parent_stdlib_tables_may_be_mutated = self.stdlib_tables_may_be_mutated;
        let parent_global_environment_escaped = self.global_environment_escaped;
        let parent_disable_builtin_fastpaths = self.disable_builtin_fastpaths;
        let parent_next_register = self.next_register;
        let parent_max_registers = self.max_registers;
        let parent_loop_breaks = std::mem::take(&mut self.loop_breaks);
        let parent_loop_continues = std::mem::take(&mut self.loop_continues);
        let parent_loop_scope_depths = std::mem::take(&mut self.loop_scope_depths);
        let parent_scope_register_starts = std::mem::take(&mut self.scope_register_starts);
        let parent_current_function_is_vararg = self.current_function_is_vararg;
        self.capture_stack.push(parent_scope);

        self.next_register = 0;
        self.max_registers = 0;
        self.captured_registers = match self.capture_hints.get(prototype.saturating_add(1)) {
            Some(hints) => hints.iter().copied().collect(),
            None if self.capture_hints.is_empty() => BTreeSet::new(),
            None => {
                return Err(CompileError::at(
                    "capture discovery changed the function prototype graph",
                    span,
                ));
            }
        };
        self.current_function_is_vararg = is_vararg;
        self.disable_builtin_fastpaths = true;
        if let Some(name) = self_name {
            self.locals
                .insert(name.to_owned(), Binding::CurrentFunction);
        }
        for param in params {
            let register = self.alloc_register_at(param.span)?;
            self.locals
                .insert(param.name.clone(), Binding::Register(register));
        }
        self.compile_block(body)?;
        self.emit_instruction(Instruction::new(Opcode::Return, 0, 0, 0));

        let function_code = std::mem::take(&mut self.code);
        let function_source_map = std::mem::take(&mut self.source_map);
        let function_max_registers = usize::from(self.max_registers.max(1));
        let upvalues = std::mem::take(&mut self.upvalues);
        let upvalue_names = std::mem::take(&mut self.upvalue_names);
        debug_assert_eq!(upvalues.len(), upvalue_names.len());
        let captured_registers = self.captured_registers.iter().copied().collect::<Vec<_>>();
        let function_assigned_globals = self.assigned_globals.clone();
        let function_assigned_stdlib_fields = self.assigned_stdlib_fields.clone();
        let function_stdlib_tables_may_be_mutated = self.stdlib_tables_may_be_mutated;
        let function_global_environment_escaped = self.global_environment_escaped;
        let parameter_types = params
            .iter()
            .map(runtime_parameter_type)
            .collect::<Vec<_>>();
        self.captured_registers.clear();
        self.functions[prototype] = Function::with_params_vararg_and_upvalues(
            function_code,
            function_max_registers,
            params.len(),
            is_vararg,
            upvalues,
            captured_registers,
        )
        .with_source_map(function_source_map)
        .with_upvalue_names(upvalue_names)
        .with_debug_name(debug_name)
        .with_debug_line_defined(debug_line_defined)
        .with_parameter_types(parameter_types);

        let parent_scope = self
            .capture_stack
            .pop()
            .expect("function compile scope should be balanced");
        self.code = parent_code;
        self.source_map = parent_source_map;
        self.locals = parent_scope.locals;
        self.fastcall_candidate_names = parent_scope.fastcall_candidate_names;
        self.upvalues = parent_scope.upvalues;
        self.upvalue_names = parent_scope.upvalue_names;
        self.captured_registers = parent_scope.captured_registers;
        self.assigned_globals = parent_assigned_globals;
        self.assigned_globals.extend(function_assigned_globals);
        self.assigned_stdlib_fields = parent_assigned_stdlib_fields;
        self.assigned_stdlib_fields
            .extend(function_assigned_stdlib_fields);
        self.stdlib_tables_may_be_mutated =
            parent_stdlib_tables_may_be_mutated || function_stdlib_tables_may_be_mutated;
        self.global_environment_escaped =
            parent_global_environment_escaped || function_global_environment_escaped;
        self.disable_builtin_fastpaths = parent_disable_builtin_fastpaths;
        self.next_register = parent_next_register;
        self.max_registers = parent_max_registers;
        self.loop_breaks = parent_loop_breaks;
        self.loop_continues = parent_loop_continues;
        self.loop_scope_depths = parent_loop_scope_depths;
        self.scope_register_starts = parent_scope_register_starts;
        self.current_function_is_vararg = parent_current_function_is_vararg;

        if prototype > i32::MAX as usize {
            return Err(CompileError::at("too many function prototypes", span));
        }
        Ok(prototype)
    }

    fn compile_logical_and(
        &mut self,
        lhs: &Expr,
        rhs: &Expr,
        target: u8,
        span: Span,
    ) -> Result<()> {
        self.compile_expr_to(lhs, target)?;
        let jump_index = self.code.len();
        self.code
            .push(Instruction::with_aux(Opcode::JumpIfNot, target, 0, 0, 0));
        self.compile_expr_to(rhs, target)?;
        self.patch_jump_to_here(jump_index, span)
    }

    fn compile_logical_or(&mut self, lhs: &Expr, rhs: &Expr, target: u8, span: Span) -> Result<()> {
        self.compile_expr_to(lhs, target)?;
        let jump_index = self.code.len();
        self.code
            .push(Instruction::with_aux(Opcode::JumpIf, target, 0, 0, 0));
        self.compile_expr_to(rhs, target)?;
        self.patch_jump_to_here(jump_index, span)
    }

    fn compile_if_expression(
        &mut self,
        condition: &Expr,
        then_expr: &Expr,
        else_expr: &Expr,
        target: u8,
        span: Span,
    ) -> Result<()> {
        let condition_register = self.compile_expr(condition)?;
        let false_jump = self.code.len();
        self.emit_instruction(Instruction::with_aux(
            Opcode::JumpIfNot,
            condition_register,
            0,
            0,
            0,
        ));
        self.free_register(condition_register);

        self.compile_expr_to(then_expr, target)?;
        let end_jump = self.code.len();
        self.code
            .push(Instruction::with_aux(Opcode::Jump, 0, 0, 0, 0));
        self.patch_jump_to_here(false_jump, span)?;
        self.compile_expr_to(else_expr, target)?;
        self.patch_jump_to_here(end_jump, span)
    }

    fn compile_field_to(
        &mut self,
        object: &Expr,
        field: &str,
        field_span: Span,
        target: u8,
    ) -> Result<()> {
        if let Some(path) = dotted_import_path(object, field) {
            if (2..=3).contains(&path.len()) && !self.name_is_lexically_bound(&path[0]) {
                return self.compile_import_path_to(&path, target, field_span);
            }
        }
        self.compile_expr_to(object, target)?;
        let key = self.string_constant_index(field.as_bytes(), field_span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(Opcode::GetTableKs, target, target, 0, key),
            field_span,
        );
        Ok(())
    }

    fn compile_import_path_to(&mut self, path: &[String], target: u8, span: Span) -> Result<()> {
        if !(1..=3).contains(&path.len()) {
            return Err(CompileError::at(
                "import path must contain between one and three components",
                span,
            ));
        }
        let mut components = Vec::with_capacity(path.len());
        for component in path {
            let index = self.constant_index(Constant::String(component.as_bytes().to_vec()))?;
            components.push(
                u32::try_from(index)
                    .map_err(|_| CompileError::at("import component index overflow", span))?,
            );
        }
        let descriptor = self.constant_index(Constant::Import(components))?;
        let descriptor = i32::try_from(descriptor)
            .map_err(|_| CompileError::at("import descriptor index overflow", span))?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::GetImport,
                target,
                u8::try_from(path.len())
                    .map_err(|_| CompileError::at("import path length overflow", span))?,
                0,
                descriptor,
            ),
            span,
        );
        Ok(())
    }

    fn compile_table_to(&mut self, fields: &[TableField], target: u8) -> Result<()> {
        let mut array_index = 1.0;
        let array_field_count = fields
            .iter()
            .filter(|field| matches!(field, TableField::Array { .. }))
            .count();
        // PUC constructor pre-sizing (OP_NEWTABLE B/C operands): the hash
        // field count rides along unconditionally; only the Puc51Compat
        // profile consumes it. aux is 0 so constructor markers can never
        // collide with the `table.create` builtin arity (1..=2).
        let hash_field_count = fields.len() - array_field_count;
        if array_field_count == 0 {
            self.emit_instruction(Instruction::with_aux(
                Opcode::NewArray,
                target,
                0,
                0,
                hash_field_count as i32,
            ));
        } else {
            self.load_constant_to(target, Constant::Number(array_field_count as f64))?;
            let hash_register = self.load_constant(Constant::Number(hash_field_count as f64))?;
            self.emit_instruction(Instruction::with_aux(
                Opcode::TableCreate,
                target,
                target,
                hash_register,
                0,
            ));
            self.free_register(hash_register);
        }
        if self.options.puc51_constructor_order {
            return self.compile_table_fields_puc_order(fields, target);
        }
        let last_index = fields.len().saturating_sub(1);
        for (index, field) in fields.iter().enumerate() {
            match field {
                TableField::Array { value, .. } => {
                    if index == last_index {
                        if let Expr::Varargs(span) = value {
                            self.validate_varargs_available(*span)?;
                            if array_index > i32::MAX as f64 {
                                return Err(CompileError::at(
                                    "table constructor vararg index overflow",
                                    *span,
                                ));
                            }
                            self.emit_instruction(Instruction::with_aux(
                                Opcode::AppendVarargs,
                                target,
                                0,
                                0,
                                array_index as i32,
                            ));
                            continue;
                        }
                        if expr_is_call(value) && !self.call_is_conformance_concat(value) {
                            self.compile_append_call_to_table(target, array_index, value)?;
                            continue;
                        }
                    }
                    let key_register = self.load_constant(Constant::Number(array_index))?;
                    let value_register = self.compile_expr(value)?;
                    self.emit_instruction(Instruction::new(
                        Opcode::SetIndex,
                        target,
                        key_register,
                        value_register,
                    ));
                    self.free_register(value_register);
                    self.free_register(key_register);
                    array_index += 1.0;
                }
                TableField::Named { name, value, .. } => {
                    let key =
                        self.compiled_string_table_key(name.as_bytes(), value.span(), true)?;
                    let value_register =
                        self.compile_expr_with_debug_name(value, Some(name.clone()))?;
                    self.code
                        .push(table_set_instruction(target, key, value_register));
                    self.free_register(value_register);
                    if let Some(key) = key.register() {
                        self.free_register(key);
                    }
                }
                TableField::Keyed { key, value, .. } => {
                    if let Some(Constant::String(bytes)) = const_eval(key) {
                        let key = self.compiled_string_table_key(&bytes, key.span(), true)?;
                        let value_register = self.compile_expr(value)?;
                        self.code
                            .push(table_set_instruction(target, key, value_register));
                        self.free_register(value_register);
                        if let Some(key) = key.register() {
                            self.free_register(key);
                        }
                    } else {
                        let key_register = self.compile_expr(key)?;
                        let value_register = self.compile_expr(value)?;
                        self.emit_instruction(Instruction::new(
                            Opcode::SetIndex,
                            target,
                            key_register,
                            value_register,
                        ));
                        self.free_register(value_register);
                        self.free_register(key_register);
                    }
                }
            }
        }
        Ok(())
    }

    /// PUC 5.1 constructor write order (lparser.c `constructor`): keyed and
    /// named fields emit their table write immediately at their source
    /// position, while positional values are evaluated in order into
    /// consecutive registers and written only when `LFIELDS_PER_FLUSH` (50,
    /// lopcodes.h) have accumulated and again after the last field. A later
    /// positional batch therefore overwrites an earlier keyed write to the
    /// same index (`{1, [1] = "one", 2}` yields `t[1] == 1`), the reverse of
    /// the default source-order writes.
    fn compile_table_fields_puc_order(&mut self, fields: &[TableField], target: u8) -> Result<()> {
        const FIELDS_PER_FLUSH: usize = 50;
        let mut array_index = 1.0;
        let mut pending: Vec<u8> = Vec::new();
        let mut pending_start_index = 1.0f64;
        let last_index = fields.len().saturating_sub(1);
        for (index, field) in fields.iter().enumerate() {
            match field {
                TableField::Array { value, .. } => {
                    if index == last_index {
                        if let Expr::Varargs(span) = value {
                            self.flush_puc_constructor_pending(
                                target,
                                &mut pending,
                                pending_start_index,
                            )?;
                            self.validate_varargs_available(*span)?;
                            if array_index > i32::MAX as f64 {
                                return Err(CompileError::at(
                                    "table constructor vararg index overflow",
                                    *span,
                                ));
                            }
                            self.emit_instruction(Instruction::with_aux(
                                Opcode::AppendVarargs,
                                target,
                                0,
                                0,
                                array_index as i32,
                            ));
                            continue;
                        }
                        if expr_is_call(value) && !self.call_is_conformance_concat(value) {
                            self.flush_puc_constructor_pending(
                                target,
                                &mut pending,
                                pending_start_index,
                            )?;
                            self.compile_append_call_to_table(target, array_index, value)?;
                            continue;
                        }
                    }
                    let value_register = self.compile_expr(value)?;
                    if pending.is_empty() {
                        pending_start_index = array_index;
                    }
                    pending.push(value_register);
                    array_index += 1.0;
                    if pending.len() == FIELDS_PER_FLUSH {
                        self.flush_puc_constructor_pending(
                            target,
                            &mut pending,
                            pending_start_index,
                        )?;
                    }
                }
                TableField::Named { name, value, .. } => {
                    let key =
                        self.compiled_string_table_key(name.as_bytes(), value.span(), true)?;
                    let value_register =
                        self.compile_expr_with_debug_name(value, Some(name.clone()))?;
                    self.code
                        .push(table_set_instruction(target, key, value_register));
                    self.free_register(value_register);
                    if let Some(key) = key.register() {
                        self.free_register(key);
                    }
                }
                TableField::Keyed { key, value, .. } => {
                    if let Some(Constant::String(bytes)) = const_eval(key) {
                        let key = self.compiled_string_table_key(&bytes, key.span(), true)?;
                        let value_register = self.compile_expr(value)?;
                        self.code
                            .push(table_set_instruction(target, key, value_register));
                        self.free_register(value_register);
                        if let Some(key) = key.register() {
                            self.free_register(key);
                        }
                    } else {
                        let key_register = self.compile_expr(key)?;
                        let value_register = self.compile_expr(value)?;
                        self.emit_instruction(Instruction::new(
                            Opcode::SetIndex,
                            target,
                            key_register,
                            value_register,
                        ));
                        self.free_register(value_register);
                        self.free_register(key_register);
                    }
                }
            }
        }
        self.flush_puc_constructor_pending(target, &mut pending, pending_start_index)
    }

    /// Emits the buffered positional writes of a PUC-order constructor. The
    /// pending value registers are consecutive from `pending[0]` because each
    /// `compile_expr` leaves its result on top of the register file.
    fn flush_puc_constructor_pending(
        &mut self,
        target: u8,
        pending: &mut Vec<u8>,
        start_index: f64,
    ) -> Result<()> {
        if pending.is_empty() {
            return Ok(());
        }
        for (offset, value_register) in pending.iter().enumerate() {
            let key_register = self.load_constant(Constant::Number(start_index + offset as f64))?;
            self.emit_instruction(Instruction::new(
                Opcode::SetIndex,
                target,
                key_register,
                *value_register,
            ));
            self.free_register(key_register);
        }
        let base_register = pending[0];
        pending.clear();
        self.free_registers_back_to(u16::from(base_register));
        Ok(())
    }

    fn compile_append_call_to_table(
        &mut self,
        table_register: u8,
        array_index: f64,
        expr: &Expr,
    ) -> Result<()> {
        if array_index > i32::MAX as f64 {
            return Err(CompileError::at(
                "table constructor call index overflow",
                expr.span(),
            ));
        }
        let span = call_site_span(expr);
        match expr {
            Expr::Call { callee, args, .. } => {
                if args.len() > u8::MAX as usize {
                    return Err(CompileError::at(
                        "function call has too many arguments",
                        span,
                    ));
                }
                let original_next_register = self.next_register;
                if self.value_call_has_trailing_varargs(args) {
                    return self.compile_append_call_varargs_to_table(
                        table_register,
                        array_index,
                        callee,
                        args,
                        original_next_register,
                        span,
                    );
                }
                let mut argc = args.len();
                if self.value_call_has_expandable_trailing_arg(args) {
                    let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
                        CompileError::at("expanded table call needs a trailing argument", span)
                    })?;
                    let trailing = &args[trailing_index];
                    let Some(result_count) = self.fixed_expansion_result_count(trailing) else {
                        return self.compile_append_call_expand_to_table(
                            table_register,
                            array_index,
                            callee,
                            args,
                            original_next_register,
                            span,
                        );
                    };
                    argc = trailing_index.checked_add(result_count).ok_or_else(|| {
                        CompileError::at("function call has too many arguments", span)
                    })?;
                }
                if argc > u8::MAX as usize {
                    return Err(CompileError::at(
                        "function call has too many arguments",
                        span,
                    ));
                }
                let callee_register = self.alloc_register_window_at(
                    1usize.checked_add(argc).ok_or_else(|| {
                        CompileError::at("function call register window overflow", span)
                    })?,
                    span,
                )?;
                let deferred = self.deferred_builtin_import(callee, args)?.is_some();
                if !deferred {
                    self.compile_expr_to(callee, callee_register)?;
                }
                if self.value_call_has_expandable_trailing_arg(args) {
                    let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
                        CompileError::at("expanded table call needs a trailing argument", span)
                    })?;
                    for (index, arg) in args[..trailing_index].iter().enumerate() {
                        let register = checked_register_offset(callee_register, index + 1, span)?;
                        self.compile_expr_to(arg, register)?;
                    }
                    let trailing_register =
                        checked_register_offset(callee_register, trailing_index + 1, span)?;
                    let result_count = argc - trailing_index;
                    self.compile_expand_multi_to(
                        &args[trailing_index],
                        trailing_register,
                        result_count,
                    )?;
                } else {
                    for (index, arg) in args.iter().enumerate() {
                        let register = checked_register_offset(callee_register, index + 1, span)?;
                        self.compile_expr_to(arg, register)?;
                    }
                }
                if deferred {
                    self.compile_expr_to(callee, callee_register)?;
                }
                self.emit_source_mapped_instruction(
                    Instruction::with_aux(
                        Opcode::AppendCall,
                        table_register,
                        callee_register,
                        u8::try_from(argc).map_err(|_| {
                            CompileError::at("function call has too many arguments", span)
                        })?,
                        array_index as i32,
                    ),
                    span,
                );
                self.free_registers_back_to(original_next_register);
                Ok(())
            }
            Expr::MethodCall {
                object,
                method,
                method_span: _,
                args,
                ..
            } => {
                let argc = args.len().checked_add(1).ok_or_else(|| {
                    CompileError::at("function call has too many arguments", span)
                })?;
                if argc > u8::MAX as usize {
                    return Err(CompileError::at(
                        "function call has too many arguments",
                        span,
                    ));
                }
                let original_next_register = self.next_register;
                if self.method_call_has_trailing_varargs(args) {
                    self.try_compile_general_expanded_call(
                        ExpandedCallTarget::Method { object, method },
                        args,
                        ExpandedCallFinish::Append {
                            table: table_register,
                            index: array_index,
                        },
                        span,
                    )?;
                    self.free_registers_back_to(original_next_register);
                    return Ok(());
                }
                let mut argc = argc;
                if self.method_call_has_expandable_trailing_arg(args) {
                    let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
                        CompileError::at(
                            "expanded table method call needs a trailing argument",
                            span,
                        )
                    })?;
                    let trailing = &args[trailing_index];
                    let Some(result_count) = self.fixed_expansion_result_count(trailing) else {
                        return self.compile_append_method_call_expand_to_table(
                            table_register,
                            array_index,
                            expr,
                            original_next_register,
                        );
                    };
                    argc = trailing_index
                        .checked_add(1)
                        .and_then(|count| count.checked_add(result_count))
                        .ok_or_else(|| {
                            CompileError::at("function call has too many arguments", span)
                        })?;
                }
                if argc > u8::MAX as usize {
                    return Err(CompileError::at(
                        "function call has too many arguments",
                        span,
                    ));
                }
                let callee_register = self.alloc_register_window_at(
                    1usize.checked_add(argc).ok_or_else(|| {
                        CompileError::at("function call register window overflow", span)
                    })?,
                    span,
                )?;
                let self_register = checked_register_offset(callee_register, 1, span)?;
                if self.method_receiver_register(object).is_none() {
                    self.compile_expr_to(object, self_register)?;
                }

                if self.method_call_has_expandable_trailing_arg(args) {
                    let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
                        CompileError::at(
                            "expanded table method call needs a trailing argument",
                            span,
                        )
                    })?;
                    for (index, arg) in args[..trailing_index].iter().enumerate() {
                        let register = checked_register_offset(callee_register, index + 2, span)?;
                        self.compile_expr_to(arg, register)?;
                    }
                    let trailing_register =
                        checked_register_offset(callee_register, trailing_index + 2, span)?;
                    let result_count = argc - trailing_index - 1;
                    self.compile_expand_multi_to(
                        &args[trailing_index],
                        trailing_register,
                        result_count,
                    )?;
                } else {
                    for (index, arg) in args.iter().enumerate() {
                        let register = checked_register_offset(callee_register, index + 2, span)?;
                        self.compile_expr_to(arg, register)?;
                    }
                }
                if self.method_receiver_register(object).is_some() {
                    self.compile_expr_to(object, self_register)?;
                }
                self.emit_namecall_lookup(callee_register, self_register, method, true, span)?;
                self.emit_source_mapped_instruction(
                    Instruction::with_aux(
                        Opcode::AppendCall,
                        table_register,
                        callee_register,
                        u8::try_from(argc).map_err(|_| {
                            CompileError::at("function call has too many arguments", span)
                        })?,
                        array_index as i32,
                    ),
                    span,
                );
                self.free_registers_back_to(original_next_register);
                Ok(())
            }
            _ => unreachable!("compile_append_call_to_table is only used for calls"),
        }
    }

    fn compile_append_call_varargs_to_table(
        &mut self,
        table_register: u8,
        array_index: f64,
        callee: &Expr,
        args: &[Expr],
        original_next_register: u16,
        span: Span,
    ) -> Result<()> {
        let Some(Expr::Varargs(vararg_span)) = args.last() else {
            unreachable!("append-call vararg lowering requires trailing varargs");
        };
        self.validate_varargs_available(*vararg_span)?;
        let prefix_count = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at(
                "vararg table constructor call needs a trailing argument",
                span,
            )
        })?;
        if prefix_count > u8::MAX as usize {
            return Err(CompileError::at(
                "function call has too many arguments",
                span,
            ));
        }

        let callee_register = self.alloc_register_window_at(
            1usize
                .checked_add(prefix_count)
                .ok_or_else(|| CompileError::at("function call register window overflow", span))?,
            span,
        )?;
        let deferred = self.deferred_builtin_import(callee, args)?.is_some();
        if !deferred {
            self.compile_expr_to(callee, callee_register)?;
        }
        for (index, arg) in args[..prefix_count].iter().enumerate() {
            let register = checked_register_offset(callee_register, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
        }
        if deferred {
            self.compile_expr_to(callee, callee_register)?;
        }
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::AppendCallVarargs,
                table_register,
                callee_register,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                array_index as i32,
            ),
            span,
        );
        self.free_registers_back_to(original_next_register);
        Ok(())
    }

    fn compile_append_call_expand_to_table(
        &mut self,
        table_register: u8,
        array_index: f64,
        callee: &Expr,
        args: &[Expr],
        original_next_register: u16,
        span: Span,
    ) -> Result<()> {
        if self.try_compile_general_expanded_call(
            ExpandedCallTarget::Value(callee),
            args,
            ExpandedCallFinish::Append {
                table: table_register,
                index: array_index,
            },
            span,
        )? {
            self.free_registers_back_to(original_next_register);
            return Ok(());
        }
        let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("expanded table call needs a trailing argument", span)
        })?;
        let trailing = &args[trailing_index];
        let trailing_argc = call_arg_count(trailing, span)?;
        let window_len = 1usize
            .checked_add(trailing_index)
            .and_then(|len| len.checked_add(1))
            .and_then(|len| len.checked_add(trailing_argc))
            .ok_or_else(|| {
                CompileError::at("table expanded call register window overflow", span)
            })?;
        let callee_register = self.alloc_register_window_at(window_len, span)?;
        self.compile_expanded_callee(callee, args, callee_register)?;
        for (index, arg) in args[..trailing_index].iter().enumerate() {
            let register = checked_register_offset(callee_register, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
        }
        let trailing_callee = checked_register_offset(callee_register, trailing_index + 1, span)?;
        self.compile_call_for_expanded_args(trailing, trailing_callee, span)?;
        self.emit_expanded_call_instruction(
            Instruction::with_aux(
                Opcode::AppendCallExpand,
                table_register,
                callee_register,
                u8::try_from(trailing_index)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                pack_append_call_expand_aux(array_index, trailing_argc, span)?,
            ),
            Some(callee),
            args,
            span,
        )?;
        self.free_registers_back_to(original_next_register);
        Ok(())
    }

    fn compile_append_method_call_expand_to_table(
        &mut self,
        table_register: u8,
        array_index: f64,
        expr: &Expr,
        original_next_register: u16,
    ) -> Result<()> {
        let Expr::MethodCall {
            object,
            method,
            args,
            ..
        } = expr
        else {
            unreachable!("compile_append_method_call_expand_to_table requires a method call");
        };
        let span = expr.span();
        if self.try_compile_general_expanded_call(
            ExpandedCallTarget::Method { object, method },
            args,
            ExpandedCallFinish::Append {
                table: table_register,
                index: array_index,
            },
            span,
        )? {
            self.free_registers_back_to(original_next_register);
            return Ok(());
        }
        let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("expanded table method call needs a trailing argument", span)
        })?;
        let trailing = &args[trailing_index];
        let trailing_argc = call_arg_count(trailing, span)?;
        let prefix_count = trailing_index
            .checked_add(1)
            .ok_or_else(|| CompileError::at("function call has too many arguments", span))?;
        let window_len = 1usize
            .checked_add(prefix_count)
            .and_then(|len| len.checked_add(1))
            .and_then(|len| len.checked_add(trailing_argc))
            .ok_or_else(|| {
                CompileError::at("table expanded method call register window overflow", span)
            })?;
        let callee_register = self.alloc_register_window_at(window_len, span)?;
        let self_register = checked_register_offset(callee_register, 1, span)?;
        self.compile_expr_to(object, self_register)?;
        self.emit_namecall_lookup(callee_register, self_register, method, false, span)?;
        for (index, arg) in args[..trailing_index].iter().enumerate() {
            let register = checked_register_offset(callee_register, index + 2, span)?;
            self.compile_expr_to(arg, register)?;
        }
        let trailing_callee = checked_register_offset(callee_register, prefix_count + 1, span)?;
        self.compile_call_for_expanded_args(trailing, trailing_callee, span)?;
        self.emit_expanded_call_instruction(
            Instruction::with_aux(
                Opcode::AppendCallExpand,
                table_register,
                callee_register,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                pack_append_call_expand_aux(array_index, trailing_argc, span)?,
            ),
            None,
            args,
            span,
        )?;
        self.free_registers_back_to(original_next_register);
        Ok(())
    }

    fn compile_generic_for(
        &mut self,
        variables: &[GenericForBinding],
        iter: &[Expr],
        body: &[Stmt],
        span: Span,
    ) -> Result<()> {
        if variables.is_empty() {
            return Err(CompileError::at(
                "generic for requires at least one variable",
                span,
            ));
        }
        let Some((iterator, table_expr)) = self.generic_array_iterator(iter) else {
            if let [expr] = iter {
                if !self.expr_can_expand(expr) {
                    return self.compile_generalized_iterator_for(variables, expr, body, span);
                }
            }
            return self.compile_generic_call_for(variables, iter, body, span);
        };

        let locals = self.locals.clone();
        let fastcall_candidate_names = self.fastcall_candidate_names.clone();
        let next_register = self.next_register;

        let table_register = self.alloc_register_at(span)?;
        let _index_register = self.alloc_register_at(span)?;
        let key_register = self.alloc_register_at(span)?;
        let value_register = self.alloc_register_at(span)?;
        let mut extra_registers = Vec::new();
        for _ in variables.iter().skip(2) {
            extra_registers.push(self.alloc_register_at(span)?);
        }

        self.compile_expr_to(table_expr, table_register)?;
        for variable in variables {
            self.fastcall_candidate_names.remove(&variable.name);
        }
        self.locals
            .insert(variables[0].name.clone(), Binding::Register(key_register));
        if let Some(value_binding) = variables.get(1) {
            self.locals.insert(
                value_binding.name.clone(),
                Binding::Register(value_register),
            );
        }
        for (binding, register) in variables
            .iter()
            .skip(2)
            .zip(extra_registers.iter().copied())
        {
            self.locals
                .insert(binding.name.clone(), Binding::Register(register));
        }

        let prep_jump = self.code.len();
        self.emit_instruction(Instruction::with_aux(
            iterator.prep_opcode(),
            table_register,
            0,
            0,
            0,
        ));
        let body_start = self.code.len();
        for register in &extra_registers {
            self.clear_local_cell(*register);
            self.load_constant_to(*register, Constant::Nil)?;
        }
        self.loop_breaks.push(Vec::new());
        self.loop_continues.push(Vec::new());
        self.loop_scope_depths
            .push(self.scope_register_starts.len());
        self.compile_scoped_block(body)?;
        let continue_target = self.code.len();
        self.patch_loop_continues_to(continue_target, span)?;
        let body_start = self.jump_target(body_start, span)?;
        self.emit_instruction(Instruction::with_aux(
            iterator.loop_opcode(),
            table_register,
            0,
            0,
            body_start,
        ));

        let clear_start = self.code.len();
        self.clear_local_cells_from(next_register)?;
        self.patch_jump_to(prep_jump, clear_start, span)?;
        self.patch_loop_breaks_to(clear_start, span)?;
        self.locals = locals;
        self.fastcall_candidate_names = fastcall_candidate_names;
        self.next_register = next_register;
        Ok(())
    }

    fn compile_generic_call_for(
        &mut self,
        variables: &[GenericForBinding],
        iter: &[Expr],
        body: &[Stmt],
        span: Span,
    ) -> Result<()> {
        let Some(last_iter) = iter.last() else {
            return Err(CompileError::at("unsupported generic for iterator", span));
        };
        if variables.len() > u8::MAX as usize {
            return Err(CompileError::at(
                "generic for has too many visible variables",
                span,
            ));
        }

        let locals = self.locals.clone();
        let fastcall_candidate_names = self.fastcall_candidate_names.clone();
        let next_register = self.next_register;
        let window_len = 3usize
            .checked_add(variables.len())
            .ok_or_else(|| CompileError::at("generic for register window overflow", span))?;
        let iterator_register = self.alloc_register_window_at(window_len, span)?;
        let first_visible = checked_register_offset(iterator_register, 3, span)?;

        self.compile_generic_iterator_values(iter, iterator_register, span)?;
        for variable in variables {
            self.fastcall_candidate_names.remove(&variable.name);
        }
        for (index, binding) in variables.iter().enumerate() {
            let register = checked_register_offset(first_visible, index, span)?;
            self.locals
                .insert(binding.name.clone(), Binding::Register(register));
        }

        let visible_count = u8::try_from(variables.len())
            .map_err(|_| CompileError::at("generic for has too many visible variables", span))?;
        let caller_cell_mode = if self.captured_registers.is_empty() {
            ITERATOR_CALLER_HAS_NO_CAPTURED_LOCALS
        } else {
            0
        };
        let prep_jump = self.code.len();
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::GenericCallPrep,
                iterator_register,
                caller_cell_mode,
                visible_count,
                0,
            ),
            call_site_span(last_iter),
        );
        let body_start = self.code.len();
        self.loop_breaks.push(Vec::new());
        self.loop_continues.push(Vec::new());
        self.loop_scope_depths
            .push(self.scope_register_starts.len());
        self.compile_scoped_block(body)?;
        let continue_target = self.code.len();
        self.patch_loop_continues_to(continue_target, span)?;
        let body_start = self.jump_target(body_start, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::GenericCallLoop,
                iterator_register,
                caller_cell_mode,
                visible_count,
                body_start,
            ),
            span,
        );

        let clear_start = self.code.len();
        self.clear_local_cells_from(next_register)?;
        self.patch_jump_to(prep_jump, clear_start, span)?;
        self.patch_loop_breaks_to(clear_start, span)?;
        self.locals = locals;
        self.fastcall_candidate_names = fastcall_candidate_names;
        self.next_register = next_register;
        Ok(())
    }

    fn compile_generalized_iterator_for(
        &mut self,
        variables: &[GenericForBinding],
        expr: &Expr,
        body: &[Stmt],
        span: Span,
    ) -> Result<()> {
        if variables.len() > u8::MAX as usize {
            return Err(CompileError::at(
                "generic for has too many visible variables",
                span,
            ));
        }

        let locals = self.locals.clone();
        let fastcall_candidate_names = self.fastcall_candidate_names.clone();
        let next_register = self.next_register;
        let window_len = 4usize
            .checked_add(variables.len())
            .ok_or_else(|| CompileError::at("generic for register window overflow", span))?;
        let iterator_register = self.alloc_register_window_at(window_len, span)?;
        let first_visible = checked_register_offset(iterator_register, 4, span)?;

        self.compile_expr_to(expr, iterator_register)?;
        for variable in variables {
            self.fastcall_candidate_names.remove(&variable.name);
        }
        for (index, binding) in variables.iter().enumerate() {
            let register = checked_register_offset(first_visible, index, span)?;
            self.locals
                .insert(binding.name.clone(), Binding::Register(register));
        }

        let visible_count = u8::try_from(variables.len())
            .map_err(|_| CompileError::at("generic for has too many visible variables", span))?;
        let caller_cell_mode = if self.captured_registers.is_empty() {
            ITERATOR_CALLER_HAS_NO_CAPTURED_LOCALS
        } else {
            0
        };
        let prep_jump = self.code.len();
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::GeneralizedIterPrep,
                iterator_register,
                caller_cell_mode,
                visible_count,
                0,
            ),
            call_site_span(expr),
        );
        let body_start = self.code.len();
        self.loop_breaks.push(Vec::new());
        self.loop_continues.push(Vec::new());
        self.loop_scope_depths
            .push(self.scope_register_starts.len());
        self.compile_scoped_block(body)?;
        let continue_target = self.code.len();
        self.patch_loop_continues_to(continue_target, span)?;
        let body_start = self.jump_target(body_start, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::GeneralizedIterLoop,
                iterator_register,
                caller_cell_mode,
                visible_count,
                body_start,
            ),
            span,
        );

        let clear_start = self.code.len();
        self.clear_local_cells_from(next_register)?;
        self.patch_jump_to(prep_jump, clear_start, span)?;
        self.patch_loop_breaks_to(clear_start, span)?;
        self.locals = locals;
        self.fastcall_candidate_names = fastcall_candidate_names;
        self.next_register = next_register;
        Ok(())
    }

    fn compile_generic_iterator_values(
        &mut self,
        iter: &[Expr],
        first_register: u8,
        span: Span,
    ) -> Result<()> {
        const ITERATOR_SLOT_COUNT: usize = 3;

        let Some(last_index) = iter.len().checked_sub(1) else {
            return Err(CompileError::at("unsupported generic for iterator", span));
        };

        if iter.len() <= ITERATOR_SLOT_COUNT && self.expr_can_expand(&iter[last_index]) {
            for (index, expr) in iter[..last_index].iter().enumerate() {
                let register = checked_register_offset(first_register, index, span)?;
                self.compile_expr_to(expr, register)?;
            }
            let first_result = checked_register_offset(first_register, last_index, span)?;
            let result_count = ITERATOR_SLOT_COUNT - last_index;
            let expanded =
                self.compile_expand_multi_to(&iter[last_index], first_result, result_count)?;
            debug_assert!(expanded);
            return Ok(());
        }

        let fixed_count = iter.len().min(ITERATOR_SLOT_COUNT);
        for (index, expr) in iter[..fixed_count].iter().enumerate() {
            let register = checked_register_offset(first_register, index, span)?;
            self.compile_expr_to(expr, register)?;
        }
        for index in fixed_count..ITERATOR_SLOT_COUNT {
            let register = checked_register_offset(first_register, index, span)?;
            self.load_constant_to(register, Constant::Nil)?;
        }
        for expr in iter.iter().skip(ITERATOR_SLOT_COUNT) {
            let register = self.compile_expr(expr)?;
            self.free_register(register);
        }
        Ok(())
    }

    fn generic_array_iterator<'a>(
        &self,
        iter: &'a [Expr],
    ) -> Option<(GenericArrayIterator, &'a Expr)> {
        if let [Expr::Call { callee, args, .. }] = iter {
            let Expr::Name(name, _) = callee.as_ref() else {
                return None;
            };
            let iterator = match name.as_str() {
                "ipairs" if self.builtin_global_is_unmodified("ipairs") => {
                    GenericArrayIterator::Ipairs
                }
                "pairs" if self.builtin_global_is_unmodified("pairs") => {
                    GenericArrayIterator::Pairs
                }
                _ => return None,
            };
            let [table_expr] = args.as_slice() else {
                return None;
            };
            if self.expr_can_expand(table_expr) {
                return None;
            }
            return Some((iterator, table_expr));
        }

        if let [Expr::Name(name, _), table_expr] = iter {
            if name == "next" && self.builtin_global_is_unmodified("next") {
                return Some((GenericArrayIterator::Pairs, table_expr));
            }
        }

        if let [table_expr @ Expr::Table { .. }] = iter {
            return Some((GenericArrayIterator::Pairs, table_expr));
        }

        None
    }

    fn patch_jump_to_here(&mut self, instruction_index: usize, span: Span) -> Result<()> {
        self.patch_jump_to(instruction_index, self.code.len(), span)
    }

    fn patch_loop_breaks_to(&mut self, target: usize, span: Span) -> Result<()> {
        let breaks = self
            .loop_breaks
            .pop()
            .expect("loop break stack should be balanced");
        self.loop_scope_depths
            .pop()
            .expect("loop scope stack should be balanced");
        for break_jump in breaks {
            self.patch_jump_to(break_jump, target, span)?;
        }
        Ok(())
    }

    fn patch_loop_continues_to(&mut self, target: usize, span: Span) -> Result<()> {
        let continues = self
            .loop_continues
            .pop()
            .expect("loop continue stack should be balanced");
        for continue_jump in continues {
            self.patch_jump_to(continue_jump, target, span)?;
        }
        Ok(())
    }

    fn clear_active_loop_scopes(&mut self) -> Result<()> {
        let Some(depth) = self.loop_scope_depths.last().copied() else {
            return Ok(());
        };
        let starts = self.scope_register_starts[depth..].to_vec();
        for start in starts.into_iter().rev() {
            self.clear_local_cells_from(start)?;
        }
        Ok(())
    }

    fn patch_jump_to(&mut self, instruction_index: usize, target: usize, span: Span) -> Result<()> {
        let target = self.jump_target(target, span)?;
        self.code[instruction_index].aux = target;
        Ok(())
    }

    fn jump_target(&self, target: usize, span: Span) -> Result<i32> {
        i32::try_from(target).map_err(|_| CompileError::at("function body too large", span))
    }

    fn compile_call_to(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        target: u8,
        span: Span,
    ) -> Result<()> {
        self.mark_global_environment_escape_for_call(callee, args);
        // A trailing call or vararg expression supplies its actual result
        // count, including zero. Parentheses deliberately select one result.
        if args.last().is_some_and(|arg| self.expr_can_expand(arg))
            || (self.deferred_builtin_import(callee, args)?.is_some()
                && args.iter().any(|arg| self.argument_may_run_guest_code(arg)))
        {
            return self.compile_value_call_to(callee, args, target, span);
        }
        // Arity guards select an opcode shape, not a language restriction.
        // Other argument lists and unknown fields must use ordinary calls so
        // errors remain runtime operations and all arguments are evaluated.
        if let Expr::Field {
            object,
            field,
            field_span: _,
            ..
        } = callee
        {
            if matches!(object.as_ref(), Expr::Name(name, _) if name == "table")
                && self.builtin_global_is_unmodified("table")
            {
                if !self.builtin_table_field_is_unmodified("table", field) {
                    return self.compile_value_call_to(callee, args, target, span);
                }
                match field.as_str() {
                    "concat" if (1..=4).contains(&args.len()) => {
                        return self.compile_table_concat_call(args, target, span)
                    }
                    "getn" if args.len() == 1 => {
                        return self.compile_table_unary_call(
                            args,
                            target,
                            span,
                            Opcode::TableGetN,
                            "table.getn",
                        )
                    }
                    "maxn" if args.len() == 1 => {
                        return self.compile_table_unary_call(
                            args,
                            target,
                            span,
                            Opcode::TableMaxN,
                            "table.maxn",
                        )
                    }
                    "create" if (1..=2).contains(&args.len()) => {
                        return self.compile_table_create_call(args, target, span)
                    }
                    "find" if (2..=3).contains(&args.len()) => {
                        return self.compile_table_find_call(args, target, span)
                    }
                    "clear" | "freeze" | "isfrozen" => {
                        return self.compile_value_call_to(callee, args, target, span);
                    }
                    "insert" if (2..=3).contains(&args.len()) => {
                        return self.compile_table_insert_call(args, target, span)
                    }
                    "remove" if (1..=2).contains(&args.len()) => {
                        return self.compile_table_remove_call(args, target, span)
                    }
                    "move" | "sort" | "foreach" | "foreachi" | "clone" => {
                        return self.compile_value_call_to(callee, args, target, span);
                    }
                    "pack" | "unpack" => {
                        return self.compile_value_call_to(callee, args, target, span);
                    }
                    _ => return self.compile_value_call_to(callee, args, target, span),
                }
            }
            if matches!(object.as_ref(), Expr::Name(name, _) if name == "math")
                && self.builtin_global_is_unmodified("math")
            {
                if !self.builtin_table_field_is_unmodified("math", field) {
                    return self.compile_value_call_to(callee, args, target, span);
                }
                if math_specialized_min_arity(field).is_some_and(|minimum| args.len() < minimum) {
                    return self.compile_value_call_to(callee, args, target, span);
                }
                match field.as_str() {
                    "abs" => {
                        if args.is_empty() || math_abs_needs_native_error_path(args) {
                            return self.compile_value_call_to(callee, args, target, span);
                        }
                        return self.compile_math_abs_call(args, target, span);
                    }
                    "floor" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathFloor,
                            "math.floor",
                        )
                    }
                    "ceil" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathCeil,
                            "math.ceil",
                        )
                    }
                    "round" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathRound,
                            "math.round",
                        )
                    }
                    "sign" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathSign,
                            "math.sign",
                        )
                    }
                    "fmod" => {
                        return self.compile_math_window_call(
                            args,
                            target,
                            span,
                            MathWindowCallSpec::new(Opcode::MathFmod, "math.fmod", 2, 2),
                        )
                    }
                    "pow" => {
                        return self.compile_math_window_call(
                            args,
                            target,
                            span,
                            MathWindowCallSpec::new(Opcode::MathPow, "math.pow", 2, 2),
                        )
                    }
                    "sqrt" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathSqrt,
                            "math.sqrt",
                        )
                    }
                    "deg" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathDeg,
                            "math.deg",
                        )
                    }
                    "rad" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathRad,
                            "math.rad",
                        )
                    }
                    "isnan" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathIsNan,
                            "math.isnan",
                        )
                    }
                    "isinf" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathIsInf,
                            "math.isinf",
                        )
                    }
                    "isfinite" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathIsFinite,
                            "math.isfinite",
                        )
                    }
                    "sin" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathSin,
                            "math.sin",
                        )
                    }
                    "cos" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathCos,
                            "math.cos",
                        )
                    }
                    "tan" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathTan,
                            "math.tan",
                        )
                    }
                    "asin" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathAsin,
                            "math.asin",
                        )
                    }
                    "acos" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathAcos,
                            "math.acos",
                        )
                    }
                    "atan" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathAtan,
                            "math.atan",
                        )
                    }
                    "atan2" => {
                        return self.compile_math_window_call(
                            args,
                            target,
                            span,
                            MathWindowCallSpec::new(Opcode::MathAtan2, "math.atan2", 2, 2),
                        )
                    }
                    "sinh" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathSinh,
                            "math.sinh",
                        )
                    }
                    "cosh" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathCosh,
                            "math.cosh",
                        )
                    }
                    "tanh" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathTanh,
                            "math.tanh",
                        )
                    }
                    "exp" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathExp,
                            "math.exp",
                        )
                    }
                    "log" => {
                        return self.compile_math_window_call(
                            args,
                            target,
                            span,
                            MathWindowCallSpec::new(Opcode::MathLog, "math.log", 1, 2),
                        )
                    }
                    "log10" => {
                        return self.compile_math_unary_call(
                            args,
                            target,
                            span,
                            Opcode::MathLog10,
                            "math.log10",
                        )
                    }
                    "ldexp" => {
                        return self.compile_math_window_call(
                            args,
                            target,
                            span,
                            MathWindowCallSpec::new(Opcode::MathLdexp, "math.ldexp", 2, 2),
                        )
                    }
                    "frexp" => return self.compile_value_call_to(callee, args, target, span),
                    "modf" => return self.compile_value_call_to(callee, args, target, span),
                    "random" | "randomseed" => {
                        return self.compile_value_call_to(callee, args, target, span)
                    }
                    "noise" => return self.compile_value_call_to(callee, args, target, span),
                    "clamp" => {
                        return self.compile_math_window_call(
                            args,
                            target,
                            span,
                            MathWindowCallSpec::new(Opcode::MathClamp, "math.clamp", 3, 3),
                        )
                    }
                    "map" => {
                        return self.compile_math_window_call(
                            args,
                            target,
                            span,
                            MathWindowCallSpec::new(Opcode::MathMap, "math.map", 5, 5),
                        )
                    }
                    "lerp" => {
                        return self.compile_math_window_call(
                            args,
                            target,
                            span,
                            MathWindowCallSpec::new(Opcode::MathLerp, "math.lerp", 3, 3),
                        )
                    }
                    "min" => {
                        return self.compile_math_window_call(
                            args,
                            target,
                            span,
                            MathWindowCallSpec::new(
                                Opcode::MathMin,
                                "math.min",
                                1,
                                u8::MAX as usize,
                            ),
                        )
                    }
                    "max" => {
                        return self.compile_math_window_call(
                            args,
                            target,
                            span,
                            MathWindowCallSpec::new(
                                Opcode::MathMax,
                                "math.max",
                                1,
                                u8::MAX as usize,
                            ),
                        )
                    }
                    _ => return self.compile_value_call_to(callee, args, target, span),
                }
            }
            if matches!(object.as_ref(), Expr::Name(name, _) if name == "string")
                && self.builtin_global_is_unmodified("string")
            {
                if !self.builtin_table_field_is_unmodified("string", field) {
                    return self.compile_value_call_to(callee, args, target, span);
                }
                match field.as_str() {
                    "sub" if (2..=3).contains(&args.len()) => {
                        return self.compile_string_sub_call(args, target, span)
                    }
                    "len" if args.len() == 1 => {
                        return self.compile_string_len_call(args, target, span)
                    }
                    "byte" if (1..=3).contains(&args.len()) => {
                        return self.compile_string_byte_call(args, target, span)
                    }
                    "upper" if args.len() == 1 => {
                        return self.compile_string_unary_call(
                            args,
                            target,
                            span,
                            Opcode::StringUpper,
                            "string.upper",
                        )
                    }
                    "lower" if args.len() == 1 => {
                        return self.compile_string_unary_call(
                            args,
                            target,
                            span,
                            Opcode::StringLower,
                            "string.lower",
                        )
                    }
                    "reverse" if args.len() == 1 => {
                        return self.compile_string_unary_call(
                            args,
                            target,
                            span,
                            Opcode::StringReverse,
                            "string.reverse",
                        )
                    }
                    "rep" if args.len() >= 2 => {
                        return self.compile_string_rep_call(args, target, span)
                    }
                    "find" if (2..=4).contains(&args.len()) => {
                        return self.compile_string_find_call(args, target, span)
                    }
                    "split" if (1..=2).contains(&args.len()) => {
                        return self.compile_string_split_call(args, target, span)
                    }
                    "char" | "match" | "gsub" | "gmatch" | "format" | "pack" | "unpack"
                    | "packsize" => return self.compile_value_call_to(callee, args, target, span),
                    _ => return self.compile_value_call_to(callee, args, target, span),
                }
            }
            return self.compile_value_call_to(callee, args, target, span);
        }

        if let Expr::Name(name, _) = callee {
            if self.builtin_global_is_unmodified(name) {
                return match name.as_str() {
                    "assert" => {
                        if let Some(arg) = args.first() {
                            self.compile_expr_to(arg, target)?;
                        } else {
                            self.load_constant_to(target, Constant::Nil)
                                .map_err(|error| error.with_span(span))?;
                        }
                        let message_register = if let Some(message) = args.get(1) {
                            Some(self.compile_expr(message)?)
                        } else {
                            None
                        };
                        if let Some(rest) = args.get(2..) {
                            for arg in rest {
                                let register = self.compile_expr(arg)?;
                                self.free_register(register);
                            }
                        }
                        let message_mode = if args.is_empty() {
                            ASSERT_MESSAGE_MISSING_ARGUMENT
                        } else {
                            message_register.map_or(ASSERT_MESSAGE_NONE, i32::from)
                        };
                        self.emit_source_mapped_instruction(
                            Instruction::with_aux(Opcode::Assert, target, target, 0, message_mode),
                            span,
                        );
                        if let Some(register) = message_register {
                            self.free_register(register);
                        }
                        Ok(())
                    }
                    "tonumber" => {
                        if args.len() == 2 {
                            return self.compile_value_call_to(callee, args, target, span);
                        }
                        if args.len() != 1 {
                            return self.compile_value_call_to(callee, args, target, span);
                        }
                        self.compile_expr_to(&args[0], target)?;
                        self.emit_source_mapped_instruction(
                            Instruction::new(Opcode::ToNumber, target, target, 0),
                            span,
                        );
                        Ok(())
                    }
                    "tostring" => {
                        if args.is_empty() {
                            return self.compile_value_call_to(callee, args, target, span);
                        }
                        if args.len() == 1 && self.expr_can_expand(&args[0]) {
                            return self.compile_value_call_to(callee, args, target, span);
                        }
                        self.compile_expr_to(&args[0], target)?;
                        for arg in &args[1..] {
                            let register = self.compile_expr(arg)?;
                            self.free_register(register);
                        }
                        self.emit_source_mapped_instruction(
                            Instruction::new(Opcode::ToString, target, target, 0),
                            span,
                        );
                        Ok(())
                    }
                    "type" | "typeof" if args.len() == 1 => {
                        self.compile_typeof_call(args, target, span)
                    }
                    "loadstring"
                        if self.options.builtins == BuiltinEnvironment::LuauConformance =>
                    {
                        self.compile_conformance_loadstring_call(callee, args, target, span)
                    }
                    "rawequal" => self.compile_value_call_to(callee, args, target, span),
                    "rawget" if args.len() >= 2 => self.compile_rawget_call(args, target, span),
                    "rawset" if args.len() >= 3 => self.compile_rawset_call(args, target, span),
                    "rawlen" if !args.is_empty() => self.compile_rawlen_call(args, target, span),
                    "next" if (1..=2).contains(&args.len()) => {
                        self.compile_next_call(args, target, span)
                    }
                    "concat" if self.options.builtins == BuiltinEnvironment::LuauConformance => {
                        self.compile_conformance_concat_call(args, target, span)
                    }
                    _ => self.compile_value_call_to(callee, args, target, span),
                };
            }
        }

        self.compile_value_call_to(callee, args, target, span)
    }

    fn compile_value_call_to(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        target: u8,
        span: Span,
    ) -> Result<()> {
        self.mark_global_environment_escape_for_call(callee, args);
        if args.len() > u8::MAX as usize {
            return Err(CompileError::at(
                "function call has too many arguments",
                span,
            ));
        }
        if self.value_call_has_trailing_varargs(args) {
            return self.compile_value_call_varargs_to(callee, args, target, 1, span);
        }
        if self.value_call_has_expandable_trailing_arg(args) {
            return self.compile_call_expand_to(
                ExpandedCallTarget::Value(callee),
                args,
                target,
                1,
                span,
            );
        }
        let deferred = self.deferred_builtin_import(callee, args)?.is_some();
        if !deferred {
            self.compile_expr_to(callee, target)?;
        }
        let mut arg_registers = Vec::with_capacity(args.len());
        for arg in args {
            arg_registers.push(self.compile_expr(arg)?);
        }
        if deferred {
            self.compile_expr_to(callee, target)?;
        }
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                if self.expr_is_fastcall_candidate(callee) {
                    Opcode::FastCall
                } else {
                    Opcode::Call
                },
                target,
                target,
                arg_registers.first().copied().unwrap_or(0),
                i32::try_from(args.len())
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
            ),
            span,
        );
        for register in arg_registers.into_iter().rev() {
            self.free_register(register);
        }
        Ok(())
    }

    fn compile_conformance_loadstring_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        target: u8,
        span: Span,
    ) -> Result<()> {
        let [Expr::String(source, _)] = args else {
            return self.compile_value_call_to(callee, args, target, span);
        };
        let Ok(chunk) = parse_source_bytes(source) else {
            return self.compile_value_call_to(callee, args, target, span);
        };
        let options = CompileOptions::default().with_source_bytes("loadstring", source);
        let Ok(constants) = repeat_condition::check(&chunk, &options) else {
            return self.compile_value_call_to(callee, args, target, span);
        };
        let parent = std::mem::replace(&mut self.repeat_constants, constants);
        let prototype = self.compile_detached_chunk_prototype(&chunk, span)?;
        self.repeat_constants = parent;
        self.emit_instruction(Instruction::with_aux(
            Opcode::LoadFunction,
            target,
            0,
            0,
            i32::try_from(prototype)
                .map_err(|_| CompileError::at("too many function prototypes", span))?,
        ));
        Ok(())
    }

    fn compile_detached_chunk_prototype(&mut self, chunk: &Chunk, span: Span) -> Result<usize> {
        let prototype = self.functions.len();
        self.functions.push(Function::new(Vec::new(), 1));

        let parent_code = std::mem::take(&mut self.code);
        let parent_source_map = std::mem::take(&mut self.source_map);
        let parent_source_line_starts = std::mem::take(&mut self.options.source_line_starts);
        let parent_locals = std::mem::take(&mut self.locals);
        let parent_fastcall_candidate_names = std::mem::take(&mut self.fastcall_candidate_names);
        let parent_upvalues = std::mem::take(&mut self.upvalues);
        let parent_upvalue_names = std::mem::take(&mut self.upvalue_names);
        let parent_captured_registers = std::mem::take(&mut self.captured_registers);
        let parent_capture_stack = std::mem::take(&mut self.capture_stack);
        let parent_assigned_globals = self.assigned_globals.clone();
        let parent_assigned_stdlib_fields = self.assigned_stdlib_fields.clone();
        let parent_stdlib_tables_may_be_mutated = self.stdlib_tables_may_be_mutated;
        let parent_global_environment_escaped = self.global_environment_escaped;
        let parent_disable_builtin_fastpaths = self.disable_builtin_fastpaths;
        let parent_next_register = self.next_register;
        let parent_max_registers = self.max_registers;
        let parent_loop_breaks = std::mem::take(&mut self.loop_breaks);
        let parent_loop_continues = std::mem::take(&mut self.loop_continues);
        let parent_loop_scope_depths = std::mem::take(&mut self.loop_scope_depths);
        let parent_scope_register_starts = std::mem::take(&mut self.scope_register_starts);
        let parent_current_function_is_vararg = self.current_function_is_vararg;

        self.next_register = 0;
        self.max_registers = 0;
        self.captured_registers = match self.capture_hints.get(prototype.saturating_add(1)) {
            Some(hints) => hints.iter().copied().collect(),
            None if self.capture_hints.is_empty() => BTreeSet::new(),
            None => {
                return Err(CompileError::at(
                    "capture discovery changed the function prototype graph",
                    span,
                ));
            }
        };
        self.current_function_is_vararg = true;
        self.disable_builtin_fastpaths = true;
        self.compile_block(&chunk.statements)?;
        self.emit_instruction(Instruction::new(Opcode::Return, 0, 0, 0));

        let function_code = std::mem::take(&mut self.code);
        let function_source_map = std::mem::take(&mut self.source_map);
        let function_max_registers = usize::from(self.max_registers.max(1));
        let function_captured_registers = self.captured_registers.iter().copied().collect();
        self.functions[prototype] = Function::with_params_vararg_and_upvalues(
            function_code,
            function_max_registers,
            0,
            true,
            Vec::new(),
            function_captured_registers,
        )
        .with_source_map(function_source_map);

        self.code = parent_code;
        self.source_map = parent_source_map;
        self.options.source_line_starts = parent_source_line_starts;
        self.locals = parent_locals;
        self.fastcall_candidate_names = parent_fastcall_candidate_names;
        self.upvalues = parent_upvalues;
        self.upvalue_names = parent_upvalue_names;
        self.captured_registers = parent_captured_registers;
        self.capture_stack = parent_capture_stack;
        self.assigned_globals = parent_assigned_globals;
        self.assigned_stdlib_fields = parent_assigned_stdlib_fields;
        self.stdlib_tables_may_be_mutated = parent_stdlib_tables_may_be_mutated;
        self.global_environment_escaped = parent_global_environment_escaped;
        self.disable_builtin_fastpaths = parent_disable_builtin_fastpaths;
        self.next_register = parent_next_register;
        self.max_registers = parent_max_registers;
        self.loop_breaks = parent_loop_breaks;
        self.loop_continues = parent_loop_continues;
        self.loop_scope_depths = parent_loop_scope_depths;
        self.scope_register_starts = parent_scope_register_starts;
        self.current_function_is_vararg = parent_current_function_is_vararg;

        if prototype > i32::MAX as usize {
            return Err(CompileError::at("too many function prototypes", span));
        }
        Ok(prototype)
    }

    fn compile_return_expand(
        &mut self,
        expr: &Expr,
        prefix_first: u8,
        prefix_count: usize,
        span: Span,
    ) -> Result<()> {
        match expr {
            Expr::Call { callee, args, .. } => {
                self.compile_value_return_call(callee, args, prefix_first, prefix_count, span)
            }
            Expr::MethodCall {
                object,
                method,
                method_span: _,
                args,
                ..
            } => self.compile_method_return_call(
                object,
                method,
                args,
                prefix_first,
                prefix_count,
                span,
            ),
            Expr::Varargs(vararg_span) => {
                self.validate_varargs_available(*vararg_span)?;
                self.emit_instruction(Instruction::new(
                    Opcode::ReturnVarargs,
                    prefix_first,
                    u8::try_from(prefix_count)
                        .map_err(|_| CompileError::at("return list is too large", span))?,
                    0,
                ));
                Ok(())
            }
            _ => unreachable!("compile_return_expand is only used for expanding expressions"),
        }
    }

    fn compile_value_return_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        prefix_first: u8,
        prefix_count: usize,
        span: Span,
    ) -> Result<()> {
        if prefix_count > u8::MAX as usize {
            return Err(CompileError::at("return list is too large", span));
        }
        if args.len() > u8::MAX as usize {
            return Err(CompileError::at(
                "function call has too many arguments",
                span,
            ));
        }
        if self.value_call_has_trailing_varargs(args) {
            return self.compile_value_return_call_varargs(
                callee,
                args,
                prefix_first,
                prefix_count,
                span,
            );
        }
        if self.value_call_has_expandable_trailing_arg(args) {
            return self.compile_return_call_expand(
                ExpandedCallTarget::Value(callee),
                args,
                prefix_first,
                prefix_count,
                span,
            );
        }
        let deferred = self.deferred_builtin_import(callee, args)?.is_some();
        let callee_register = self.alloc_register_at(span)?;
        if !deferred {
            self.compile_expr_to(callee, callee_register)?;
        }
        let mut arg_registers = Vec::with_capacity(args.len());
        for arg in args {
            arg_registers.push(self.compile_expr(arg)?);
        }
        let args_start = arg_registers.first().copied().unwrap_or(0);
        if deferred {
            self.compile_expr_to(callee, callee_register)?;
        }
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::ReturnCall,
                prefix_first,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("return list is too large", span))?,
                callee_register,
                pack_return_call_aux(args_start, args.len(), span)?,
            ),
            span,
        );
        Ok(())
    }

    fn compile_return_call_expand(
        &mut self,
        callee: ExpandedCallTarget<'_>,
        args: &[Expr],
        prefix_first: u8,
        prefix_count: usize,
        span: Span,
    ) -> Result<()> {
        if self.try_compile_general_expanded_call(
            callee,
            args,
            ExpandedCallFinish::Return {
                prefix: prefix_first,
                count: prefix_count,
            },
            span,
        )? {
            return Ok(());
        }
        if prefix_count > u8::MAX as usize {
            return Err(CompileError::at("return list is too large", span));
        }
        let scalar_prefix_count = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("expanded return call needs a trailing argument", span)
        })?;
        let outer_prefix_count = scalar_prefix_count + callee.implicit_args();
        if self.call_expr_has_trailing_varargs(&args[scalar_prefix_count]) {
            return self.compile_return_call_expand_varargs(
                callee,
                args,
                prefix_first,
                prefix_count,
                span,
            );
        }
        if self.call_expr_has_expandable_trailing_arg(&args[scalar_prefix_count]) {
            return self.compile_return_call_expand_nested(
                callee,
                args,
                prefix_first,
                prefix_count,
                span,
            );
        }
        let trailing_argc = call_arg_count(&args[scalar_prefix_count], span)?;
        if outer_prefix_count > u8::MAX as usize || trailing_argc > u8::MAX as usize {
            return Err(CompileError::at(
                "function call has too many arguments",
                span,
            ));
        }

        let window_len = 1usize
            .checked_add(outer_prefix_count)
            .and_then(|len| len.checked_add(1))
            .and_then(|len| len.checked_add(trailing_argc))
            .ok_or_else(|| {
                CompileError::at("expanded return call register window overflow", span)
            })?;
        let call_base = self.alloc_register_window_at(window_len, span)?;
        self.compile_expanded_call_prefix(callee, args, scalar_prefix_count, call_base, span)?;
        let trailing_callee = checked_register_offset(call_base, outer_prefix_count + 1, span)?;
        self.compile_call_for_expanded_args(&args[scalar_prefix_count], trailing_callee, span)?;
        self.emit_expanded_call_instruction(
            Instruction::with_aux(
                Opcode::ReturnCallExpand,
                prefix_first,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("return list is too large", span))?,
                call_base,
                pack_call_expand_aux(outer_prefix_count, trailing_argc, span)?,
            ),
            callee.value(),
            args,
            span,
        )?;
        Ok(())
    }

    fn compile_return_call_expand_nested(
        &mut self,
        callee: ExpandedCallTarget<'_>,
        args: &[Expr],
        prefix_first: u8,
        prefix_count: usize,
        span: Span,
    ) -> Result<()> {
        if prefix_count > u8::MAX as usize {
            return Err(CompileError::at("return list is too large", span));
        }
        let scalar_prefix_count = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("nested return call needs a trailing argument", span)
        })?;
        let outer_prefix_count = scalar_prefix_count + callee.implicit_args();
        let trailing = &args[scalar_prefix_count];
        let nested_shape = nested_call_shape(trailing, span)?;
        if self.call_expr_has_trailing_varargs(nested_shape.trailing) {
            return self.compile_return_call_expand2_varargs(
                callee,
                args,
                prefix_first,
                prefix_count,
                span,
            );
        }
        let window_len = nested_expand_window_len(outer_prefix_count, nested_shape, span)?;
        let call_base = self.alloc_register_window_at(window_len, span)?;
        self.compile_expanded_call_prefix(callee, args, scalar_prefix_count, call_base, span)?;
        let middle_callee = checked_register_offset(call_base, outer_prefix_count + 1, span)?;
        self.compile_nested_call_prefix(trailing, middle_callee, span)?;
        let inner_callee =
            checked_register_offset(middle_callee, nested_shape.prefix_count + 1, span)?;
        self.compile_call_for_expanded_args(nested_shape.trailing, inner_callee, span)?;
        self.emit_expanded_call_instruction(
            Instruction::with_aux(
                Opcode::ReturnCallExpand2,
                prefix_first,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("return list is too large", span))?,
                call_base,
                pack_nested_expand_aux(outer_prefix_count, nested_shape, span)?,
            ),
            callee.value(),
            args,
            span,
        )?;
        Ok(())
    }

    fn compile_return_call_expand2_varargs(
        &mut self,
        callee: ExpandedCallTarget<'_>,
        args: &[Expr],
        prefix_first: u8,
        prefix_count: usize,
        span: Span,
    ) -> Result<()> {
        if prefix_count > u8::MAX as usize {
            return Err(CompileError::at("return list is too large", span));
        }
        let scalar_prefix_count = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("nested vararg return call needs a trailing argument", span)
        })?;
        let outer_prefix_count = scalar_prefix_count + callee.implicit_args();
        let middle = &args[scalar_prefix_count];
        let middle_shape = nested_call_shape(middle, span)?;
        let inner = middle_shape.trailing;
        let inner_shape = nested_call_shape(inner, span)?;
        let Expr::Varargs(vararg_span) = inner_shape.trailing else {
            unreachable!("triple nested vararg expansion requires trailing varargs")
        };
        self.validate_varargs_available(*vararg_span)?;
        let window_len =
            nested_expand2_vararg_window_len(outer_prefix_count, middle_shape, inner_shape, span)?;
        let call_base = self.alloc_register_window_at(window_len, span)?;
        self.compile_expanded_call_prefix(callee, args, scalar_prefix_count, call_base, span)?;
        let middle_callee = checked_register_offset(call_base, outer_prefix_count + 1, span)?;
        self.compile_nested_call_prefix(middle, middle_callee, span)?;
        let inner_callee =
            checked_register_offset(middle_callee, middle_shape.prefix_count + 1, span)?;
        self.compile_nested_call_prefix(inner, inner_callee, span)?;
        self.emit_expanded_call_instruction(
            Instruction::with_aux(
                Opcode::ReturnCallExpand2Varargs,
                prefix_first,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("return list is too large", span))?,
                call_base,
                pack_nested_expand2_varargs_aux(
                    outer_prefix_count,
                    middle_shape.prefix_count,
                    inner_shape.prefix_count,
                    span,
                )?,
            ),
            callee.value(),
            args,
            span,
        )?;
        Ok(())
    }

    fn compile_return_call_expand_varargs(
        &mut self,
        callee: ExpandedCallTarget<'_>,
        args: &[Expr],
        prefix_first: u8,
        prefix_count: usize,
        span: Span,
    ) -> Result<()> {
        if prefix_count > u8::MAX as usize {
            return Err(CompileError::at("return list is too large", span));
        }
        let scalar_prefix_count = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("vararg return call needs a trailing argument", span)
        })?;
        let outer_prefix_count = scalar_prefix_count + callee.implicit_args();
        let trailing = &args[scalar_prefix_count];
        let nested_shape = nested_call_shape(trailing, span)?;
        let Expr::Varargs(vararg_span) = nested_shape.trailing else {
            unreachable!("nested vararg expansion requires trailing varargs")
        };
        self.validate_varargs_available(*vararg_span)?;
        let window_len = nested_vararg_window_len(outer_prefix_count, nested_shape, span)?;
        let call_base = self.alloc_register_window_at(window_len, span)?;
        self.compile_expanded_call_prefix(callee, args, scalar_prefix_count, call_base, span)?;
        let middle_callee = checked_register_offset(call_base, outer_prefix_count + 1, span)?;
        self.compile_nested_call_prefix(trailing, middle_callee, span)?;
        self.emit_expanded_call_instruction(
            Instruction::with_aux(
                Opcode::ReturnCallExpandVarargs,
                prefix_first,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("return list is too large", span))?,
                call_base,
                pack_expand_varargs_aux(outer_prefix_count, nested_shape.prefix_count, span)?,
            ),
            callee.value(),
            args,
            span,
        )?;
        Ok(())
    }

    fn compile_value_return_call_varargs(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        prefix_first: u8,
        prefix_count: usize,
        span: Span,
    ) -> Result<()> {
        if prefix_count > u8::MAX as usize {
            return Err(CompileError::at("return list is too large", span));
        }
        let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("vararg return call needs a trailing argument", span)
        })?;
        let Expr::Varargs(vararg_span) = args[trailing_index] else {
            unreachable!("compile_value_return_call_varargs requires trailing varargs")
        };
        self.validate_varargs_available(vararg_span)?;
        if trailing_index > u8::MAX as usize {
            return Err(CompileError::at(
                "function call has too many arguments",
                span,
            ));
        }

        let deferred = self.deferred_builtin_import(callee, args)?.is_some();
        let callee_register = self.alloc_register_at(span)?;
        if !deferred {
            self.compile_expr_to(callee, callee_register)?;
        }
        let mut arg_registers = Vec::with_capacity(trailing_index);
        for arg in &args[..trailing_index] {
            arg_registers.push(self.compile_expr(arg)?);
        }
        let args_start = arg_registers.first().copied().unwrap_or(0);
        if deferred {
            self.compile_expr_to(callee, callee_register)?;
        }
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::ReturnCallVarargs,
                prefix_first,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("return list is too large", span))?,
                callee_register,
                pack_return_call_aux(args_start, trailing_index, span)?,
            ),
            span,
        );
        Ok(())
    }

    fn compile_method_return_call(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        prefix_first: u8,
        prefix_count: usize,
        span: Span,
    ) -> Result<()> {
        if prefix_count > u8::MAX as usize {
            return Err(CompileError::at("return list is too large", span));
        }
        let argc = args
            .len()
            .checked_add(1)
            .ok_or_else(|| CompileError::at("function call has too many arguments", span))?;
        if argc > u8::MAX as usize {
            return Err(CompileError::at(
                "function call has too many arguments",
                span,
            ));
        }
        if self.method_call_has_trailing_varargs(args) {
            return self.compile_method_return_call_varargs(
                object,
                method,
                args,
                prefix_first,
                prefix_count,
                span,
            );
        }
        if self.method_call_has_expandable_trailing_arg(args) {
            return self.compile_method_return_call_expand(
                object,
                method,
                args,
                prefix_first,
                prefix_count,
                span,
            );
        }

        let callee_register = self.alloc_register_at(span)?;
        let self_register = self.alloc_register_at(span)?;
        if self.method_receiver_register(object).is_none() {
            self.compile_expr_to(object, self_register)?;
        }

        for arg in args {
            self.compile_expr(arg)?;
        }
        if self.method_receiver_register(object).is_some() {
            self.compile_expr_to(object, self_register)?;
        }
        self.emit_namecall_lookup(callee_register, self_register, method, true, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::ReturnCall,
                prefix_first,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("return list is too large", span))?,
                callee_register,
                pack_return_call_aux(self_register, argc, span)?,
            ),
            span,
        );
        Ok(())
    }

    fn compile_method_return_call_expand(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        prefix_first: u8,
        prefix_count: usize,
        span: Span,
    ) -> Result<()> {
        self.compile_return_call_expand(
            ExpandedCallTarget::Method { object, method },
            args,
            prefix_first,
            prefix_count,
            span,
        )
    }

    fn compile_method_return_call_varargs(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        prefix_first: u8,
        prefix_count: usize,
        span: Span,
    ) -> Result<()> {
        if prefix_count > u8::MAX as usize {
            return Err(CompileError::at("return list is too large", span));
        }
        let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("vararg return method call needs a trailing argument", span)
        })?;
        let Expr::Varargs(vararg_span) = args[trailing_index] else {
            unreachable!("compile_method_return_call_varargs requires trailing varargs")
        };
        self.validate_varargs_available(vararg_span)?;
        let argc_prefix = trailing_index
            .checked_add(1)
            .ok_or_else(|| CompileError::at("function call has too many arguments", span))?;
        if argc_prefix > u8::MAX as usize {
            return Err(CompileError::at(
                "function call has too many arguments",
                span,
            ));
        }

        let callee_register = self.alloc_register_at(span)?;
        let self_register = self.alloc_register_at(span)?;
        if self.method_receiver_register(object).is_none() {
            self.compile_expr_to(object, self_register)?;
        }

        for arg in &args[..trailing_index] {
            self.compile_expr(arg)?;
        }
        if self.method_receiver_register(object).is_some() {
            self.compile_expr_to(object, self_register)?;
        }
        self.emit_namecall_lookup(callee_register, self_register, method, true, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::ReturnCallVarargs,
                prefix_first,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("return list is too large", span))?,
                callee_register,
                pack_return_call_aux(self_register, argc_prefix, span)?,
            ),
            span,
        );
        Ok(())
    }

    fn compile_expand_multi_to(
        &mut self,
        expr: &Expr,
        first_register: u8,
        result_count: usize,
    ) -> Result<bool> {
        let span = call_site_span(expr);
        match expr {
            Expr::Call { callee, args, .. } => {
                self.compile_value_call_multi_to(callee, args, first_register, result_count, span)?;
                Ok(true)
            }
            Expr::MethodCall {
                object,
                method,
                method_span: _,
                args,
                ..
            } => {
                self.compile_method_call_multi_to(
                    object,
                    method,
                    args,
                    first_register,
                    result_count,
                    span,
                )?;
                Ok(true)
            }
            Expr::Varargs(vararg_span) => {
                self.compile_varargs_to_count(first_register, result_count, *vararg_span)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn compile_value_call_multi_to(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        first_register: u8,
        result_count: usize,
        span: Span,
    ) -> Result<()> {
        self.mark_global_environment_escape_for_call(callee, args);
        if self.value_call_has_trailing_varargs(args) {
            return self.compile_value_call_varargs_to(
                callee,
                args,
                first_register,
                result_count,
                span,
            );
        }
        if self.value_call_has_expandable_trailing_arg(args) {
            return self.compile_call_expand_to(
                ExpandedCallTarget::Value(callee),
                args,
                first_register,
                result_count,
                span,
            );
        }
        self.validate_call_multi_shape(args.len(), result_count, span)?;
        let original_next_register = self.next_register;
        let window_len = result_count.max(args.len().saturating_add(1));
        self.ensure_register_window_len(first_register, window_len, span)?;

        let deferred = self.deferred_builtin_import(callee, args)?.is_some();
        if !deferred {
            self.compile_expr_to(callee, first_register)?;
        }
        for (index, arg) in args.iter().enumerate() {
            let register = checked_register_offset(first_register, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
        }
        if deferred {
            self.compile_expr_to(callee, first_register)?;
        }
        self.emit_source_mapped_instruction(
            Instruction::new(
                Opcode::CallMulti,
                first_register,
                u8::try_from(args.len())
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                u8::try_from(result_count)
                    .map_err(|_| CompileError::at("function call has too many results", span))?,
            ),
            span,
        );
        self.free_registers_back_to(original_next_register);
        Ok(())
    }

    fn compile_call_expand_to(
        &mut self,
        callee: ExpandedCallTarget<'_>,
        args: &[Expr],
        first_register: u8,
        result_count: usize,
        span: Span,
    ) -> Result<()> {
        if self.try_compile_general_expanded_call(
            callee,
            args,
            ExpandedCallFinish::Fixed {
                first: first_register,
                count: result_count,
            },
            span,
        )? {
            return Ok(());
        }
        let scalar_prefix_count = args
            .len()
            .checked_sub(1)
            .ok_or_else(|| CompileError::at("expanded call needs a trailing argument", span))?;
        let outer_prefix_count = scalar_prefix_count + callee.implicit_args();
        if self.call_expr_has_trailing_varargs(&args[scalar_prefix_count]) {
            return self.compile_call_expand_varargs_to(
                callee,
                args,
                first_register,
                result_count,
                span,
            );
        }
        if self.call_expr_has_expandable_trailing_arg(&args[scalar_prefix_count]) {
            return self.compile_call_expand_nested_to(
                callee,
                args,
                first_register,
                result_count,
                span,
            );
        }
        self.validate_call_multi_shape(outer_prefix_count, result_count, span)?;
        let trailing_argc = call_arg_count(&args[scalar_prefix_count], span)?;
        if trailing_argc > u8::MAX as usize {
            return Err(CompileError::at(
                "function call has too many arguments",
                span,
            ));
        }

        let original_next_register = self.next_register;
        let window_len = result_count.max(
            1usize
                .checked_add(outer_prefix_count)
                .and_then(|len| len.checked_add(1))
                .and_then(|len| len.checked_add(trailing_argc))
                .ok_or_else(|| CompileError::at("expanded call register window overflow", span))?,
        );
        self.ensure_register_window_len(first_register, window_len, span)?;

        self.compile_expanded_call_prefix(callee, args, scalar_prefix_count, first_register, span)?;
        let trailing_callee =
            checked_register_offset(first_register, outer_prefix_count + 1, span)?;
        self.compile_call_for_expanded_args(&args[scalar_prefix_count], trailing_callee, span)?;
        self.emit_expanded_call_instruction(
            Instruction::with_aux(
                Opcode::CallExpand,
                first_register,
                u8::try_from(outer_prefix_count)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                u8::try_from(trailing_argc)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                i32::try_from(result_count)
                    .map_err(|_| CompileError::at("function call has too many results", span))?,
            ),
            callee.value(),
            args,
            span,
        )?;
        self.free_registers_back_to(original_next_register);
        Ok(())
    }

    fn compile_call_expand_nested_to(
        &mut self,
        callee: ExpandedCallTarget<'_>,
        args: &[Expr],
        first_register: u8,
        result_count: usize,
        span: Span,
    ) -> Result<()> {
        let scalar_prefix_count = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("nested expanded call needs a trailing argument", span)
        })?;
        let outer_prefix_count = scalar_prefix_count + callee.implicit_args();
        let trailing = &args[scalar_prefix_count];
        let nested_shape = nested_call_shape(trailing, span)?;
        self.validate_call_multi_shape(outer_prefix_count, result_count, span)?;
        if self.call_expr_has_expandable_trailing_arg(nested_shape.trailing) {
            return self.compile_call_expand3_to(callee, args, first_register, result_count, span);
        }

        let original_next_register = self.next_register;
        let window_len = result_count.max(nested_expand_window_len(
            outer_prefix_count,
            nested_shape,
            span,
        )?);
        self.ensure_register_window_len(first_register, window_len, span)?;

        self.compile_expanded_call_prefix(callee, args, scalar_prefix_count, first_register, span)?;
        let middle_callee = checked_register_offset(first_register, outer_prefix_count + 1, span)?;
        self.compile_nested_call_prefix(trailing, middle_callee, span)?;
        let inner_callee =
            checked_register_offset(middle_callee, nested_shape.prefix_count + 1, span)?;
        self.compile_call_for_expanded_args(nested_shape.trailing, inner_callee, span)?;
        self.emit_expanded_call_instruction(
            Instruction::with_aux(
                Opcode::CallExpand2,
                first_register,
                u8::try_from(outer_prefix_count)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                u8::try_from(result_count)
                    .map_err(|_| CompileError::at("function call has too many results", span))?,
                pack_nested_expand_aux(outer_prefix_count, nested_shape, span)?,
            ),
            callee.value(),
            args,
            span,
        )?;
        self.free_registers_back_to(original_next_register);
        Ok(())
    }

    fn compile_call_expand3_to(
        &mut self,
        callee: ExpandedCallTarget<'_>,
        args: &[Expr],
        first_register: u8,
        result_count: usize,
        span: Span,
    ) -> Result<()> {
        let scalar_prefix_count = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("nested expanded call needs a trailing argument", span)
        })?;
        let outer_prefix_count = scalar_prefix_count + callee.implicit_args();
        let middle = &args[scalar_prefix_count];
        let middle_shape = nested_call_shape(middle, span)?;
        let inner = middle_shape.trailing;
        let inner_shape = nested_call_shape(inner, span)?;
        self.validate_call_multi_shape(outer_prefix_count, result_count, span)?;

        let leaf_argc = call_arg_count(inner_shape.trailing, span)?;
        let original_next_register = self.next_register;
        let window_len = result_count.max(nested_expand3_window_len(
            outer_prefix_count,
            middle_shape,
            inner_shape,
            span,
        )?);
        self.ensure_register_window_len(first_register, window_len, span)?;

        self.compile_expanded_call_prefix(callee, args, scalar_prefix_count, first_register, span)?;
        let middle_callee = checked_register_offset(first_register, outer_prefix_count + 1, span)?;
        self.compile_nested_call_prefix(middle, middle_callee, span)?;
        let inner_callee =
            checked_register_offset(middle_callee, middle_shape.prefix_count + 1, span)?;
        self.compile_nested_call_prefix(inner, inner_callee, span)?;
        let leaf_callee =
            checked_register_offset(inner_callee, inner_shape.prefix_count + 1, span)?;
        self.compile_call_for_expanded_args(inner_shape.trailing, leaf_callee, span)?;
        self.emit_expanded_call_instruction(
            Instruction::with_aux(
                Opcode::CallExpand3,
                first_register,
                u8::try_from(result_count)
                    .map_err(|_| CompileError::at("function call has too many results", span))?,
                u8::try_from(leaf_argc)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                pack_nested_expand3_aux(
                    outer_prefix_count,
                    middle_shape.prefix_count,
                    inner_shape.prefix_count,
                    span,
                )?,
            ),
            callee.value(),
            args,
            span,
        )?;
        self.free_registers_back_to(original_next_register);
        Ok(())
    }

    fn compile_call_expand_varargs_to(
        &mut self,
        callee: ExpandedCallTarget<'_>,
        args: &[Expr],
        first_register: u8,
        result_count: usize,
        span: Span,
    ) -> Result<()> {
        let scalar_prefix_count = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("vararg expanded call needs a trailing argument", span)
        })?;
        let outer_prefix_count = scalar_prefix_count + callee.implicit_args();
        let trailing = &args[scalar_prefix_count];
        let nested_shape = nested_call_shape(trailing, span)?;
        let Expr::Varargs(vararg_span) = nested_shape.trailing else {
            unreachable!("nested vararg expansion requires trailing varargs")
        };
        self.validate_varargs_available(*vararg_span)?;
        self.validate_call_multi_shape(outer_prefix_count, result_count, span)?;

        let original_next_register = self.next_register;
        let window_len = result_count.max(nested_vararg_window_len(
            outer_prefix_count,
            nested_shape,
            span,
        )?);
        self.ensure_register_window_len(first_register, window_len, span)?;

        self.compile_expanded_call_prefix(callee, args, scalar_prefix_count, first_register, span)?;
        let middle_callee = checked_register_offset(first_register, outer_prefix_count + 1, span)?;
        self.compile_nested_call_prefix(trailing, middle_callee, span)?;
        self.emit_expanded_call_instruction(
            Instruction::with_aux(
                Opcode::CallExpandVarargs,
                first_register,
                u8::try_from(outer_prefix_count)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                u8::try_from(result_count)
                    .map_err(|_| CompileError::at("function call has too many results", span))?,
                pack_expand_varargs_aux(outer_prefix_count, nested_shape.prefix_count, span)?,
            ),
            callee.value(),
            args,
            span,
        )?;
        self.free_registers_back_to(original_next_register);
        Ok(())
    }

    fn compile_value_call_varargs_to(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        first_register: u8,
        result_count: usize,
        span: Span,
    ) -> Result<()> {
        let trailing_index = args
            .len()
            .checked_sub(1)
            .ok_or_else(|| CompileError::at("vararg call needs a trailing argument", span))?;
        let Expr::Varargs(vararg_span) = args[trailing_index] else {
            unreachable!("compile_value_call_varargs_to requires trailing varargs")
        };
        self.validate_varargs_available(vararg_span)?;
        self.validate_call_multi_shape(trailing_index, result_count, span)?;

        let original_next_register = self.next_register;
        let window_len = result_count.max(
            1usize
                .checked_add(trailing_index)
                .ok_or_else(|| CompileError::at("vararg call register window overflow", span))?,
        );
        self.ensure_register_window_len(first_register, window_len, span)?;

        let deferred = self.deferred_builtin_import(callee, args)?.is_some();
        if !deferred {
            self.compile_expr_to(callee, first_register)?;
        }
        for (index, arg) in args[..trailing_index].iter().enumerate() {
            let register = checked_register_offset(first_register, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
        }
        if deferred {
            self.compile_expr_to(callee, first_register)?;
        }
        self.emit_source_mapped_instruction(
            Instruction::new(
                Opcode::CallVarargs,
                first_register,
                u8::try_from(trailing_index)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                u8::try_from(result_count)
                    .map_err(|_| CompileError::at("function call has too many results", span))?,
            ),
            span,
        );
        self.free_registers_back_to(original_next_register);
        Ok(())
    }

    fn compile_call_for_expanded_args(
        &mut self,
        expr: &Expr,
        callee_register: u8,
        span: Span,
    ) -> Result<()> {
        match expr {
            Expr::Call { callee, args, .. } => {
                self.compile_expanded_callee(callee, args, callee_register)?;
                for (index, arg) in args.iter().enumerate() {
                    let register = checked_register_offset(callee_register, index + 1, span)?;
                    self.compile_expr_to(arg, register)?;
                }
            }
            Expr::MethodCall {
                object,
                method,
                method_span: _,
                args,
                ..
            } => {
                let self_register = checked_register_offset(callee_register, 1, span)?;
                self.compile_expr_to(object, self_register)?;
                self.emit_namecall_lookup(callee_register, self_register, method, false, span)?;
                for (index, arg) in args.iter().enumerate() {
                    let register = checked_register_offset(callee_register, index + 2, span)?;
                    self.compile_expr_to(arg, register)?;
                }
            }
            _ => unreachable!("expanded call argument must be a call"),
        }
        Ok(())
    }

    fn compile_nested_call_prefix(
        &mut self,
        expr: &Expr,
        callee_register: u8,
        span: Span,
    ) -> Result<()> {
        let shape = nested_call_shape(expr, span)?;
        match expr {
            Expr::Call { callee, args, .. } => {
                self.compile_expanded_callee(callee, args, callee_register)?;
                for (index, arg) in args[..shape.scalar_arg_count].iter().enumerate() {
                    let register = checked_register_offset(callee_register, index + 1, span)?;
                    self.compile_expr_to(arg, register)?;
                }
            }
            Expr::MethodCall {
                object,
                method,
                method_span: _,
                args,
                ..
            } => {
                let self_register = checked_register_offset(callee_register, 1, span)?;
                self.compile_expr_to(object, self_register)?;
                self.emit_namecall_lookup(callee_register, self_register, method, false, span)?;
                for (index, arg) in args[..shape.scalar_arg_count].iter().enumerate() {
                    let register = checked_register_offset(callee_register, index + 2, span)?;
                    self.compile_expr_to(arg, register)?;
                }
            }
            _ => unreachable!("nested expanded call must be a call"),
        }
        Ok(())
    }

    fn compile_method_call_multi_to(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        first_register: u8,
        result_count: usize,
        span: Span,
    ) -> Result<()> {
        if self.method_call_has_trailing_varargs(args) {
            return self.compile_method_call_varargs_to(
                object,
                method,
                args,
                first_register,
                result_count,
                span,
            );
        }
        if self.method_call_has_expandable_trailing_arg(args) {
            return self.compile_method_call_expand_to(
                object,
                method,
                args,
                first_register,
                result_count,
                span,
            );
        }
        let argc = args
            .len()
            .checked_add(1)
            .ok_or_else(|| CompileError::at("function call has too many arguments", span))?;
        self.validate_call_multi_shape(argc, result_count, span)?;
        let original_next_register = self.next_register;
        let window_len = result_count.max(argc.saturating_add(1));
        self.ensure_register_window_len(first_register, window_len, span)?;

        let self_register = checked_register_offset(first_register, 1, span)?;
        if self.method_receiver_register(object).is_none() {
            self.compile_expr_to(object, self_register)?;
        }

        for (index, arg) in args.iter().enumerate() {
            let register = checked_register_offset(first_register, index + 2, span)?;
            self.compile_expr_to(arg, register)?;
        }
        if self.method_receiver_register(object).is_some() {
            self.compile_expr_to(object, self_register)?;
        }
        self.emit_namecall_lookup(first_register, self_register, method, true, span)?;
        self.emit_source_mapped_instruction(
            Instruction::new(
                Opcode::CallMulti,
                first_register,
                u8::try_from(argc)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                u8::try_from(result_count)
                    .map_err(|_| CompileError::at("function call has too many results", span))?,
            ),
            span,
        );
        self.free_registers_back_to(original_next_register);
        Ok(())
    }

    fn compile_method_call_to(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        target: u8,
        span: Span,
    ) -> Result<()> {
        if self.method_call_has_trailing_varargs(args) {
            return self.compile_method_call_varargs_to(object, method, args, target, 1, span);
        }
        if self.method_call_has_expandable_trailing_arg(args) {
            return self.compile_method_call_expand_to(object, method, args, target, 1, span);
        }
        let argc = args
            .len()
            .checked_add(1)
            .ok_or_else(|| CompileError::at("function call has too many arguments", span))?;
        if argc > u8::MAX as usize {
            return Err(CompileError::at(
                "function call has too many arguments",
                span,
            ));
        }

        let self_register = self.alloc_register_at(span)?;
        if self.method_receiver_register(object).is_none() {
            self.compile_expr_to(object, self_register)?;
        }

        let mut arg_registers = Vec::with_capacity(args.len());
        for arg in args {
            arg_registers.push(self.compile_expr(arg)?);
        }
        if self.method_receiver_register(object).is_some() {
            self.compile_expr_to(object, self_register)?;
        }
        self.emit_namecall_lookup(target, self_register, method, true, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::Call,
                target,
                target,
                self_register,
                i32::try_from(argc)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
            ),
            span,
        );
        for register in arg_registers.into_iter().rev() {
            self.free_register(register);
        }
        self.free_register(self_register);
        Ok(())
    }

    fn compile_method_call_expand_to(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        first_register: u8,
        result_count: usize,
        span: Span,
    ) -> Result<()> {
        self.compile_call_expand_to(
            ExpandedCallTarget::Method { object, method },
            args,
            first_register,
            result_count,
            span,
        )
    }

    fn compile_method_call_varargs_to(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        first_register: u8,
        result_count: usize,
        span: Span,
    ) -> Result<()> {
        let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("vararg method call needs a trailing argument", span)
        })?;
        let Expr::Varargs(vararg_span) = args[trailing_index] else {
            unreachable!("compile_method_call_varargs_to requires trailing varargs")
        };
        self.validate_varargs_available(vararg_span)?;
        let prefix_count = trailing_index
            .checked_add(1)
            .ok_or_else(|| CompileError::at("function call has too many arguments", span))?;
        self.validate_call_multi_shape(prefix_count, result_count, span)?;

        let original_next_register = self.next_register;
        let window_len = result_count.max(1usize.checked_add(prefix_count).ok_or_else(|| {
            CompileError::at("vararg method call register window overflow", span)
        })?);
        self.ensure_register_window_len(first_register, window_len, span)?;

        let self_register = checked_register_offset(first_register, 1, span)?;
        if self.method_receiver_register(object).is_none() {
            self.compile_expr_to(object, self_register)?;
        }

        for (index, arg) in args[..trailing_index].iter().enumerate() {
            let register = checked_register_offset(first_register, index + 2, span)?;
            self.compile_expr_to(arg, register)?;
        }
        if self.method_receiver_register(object).is_some() {
            self.compile_expr_to(object, self_register)?;
        }
        self.emit_namecall_lookup(first_register, self_register, method, true, span)?;
        self.emit_source_mapped_instruction(
            Instruction::new(
                Opcode::CallVarargs,
                first_register,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                u8::try_from(result_count)
                    .map_err(|_| CompileError::at("function call has too many results", span))?,
            ),
            span,
        );
        self.free_registers_back_to(original_next_register);
        Ok(())
    }

    fn compile_typeof_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if args.len() != 1 {
            return Err(CompileError::at("unsupported type arity", span));
        }
        self.ensure_register_window_len(target, 1, span)?;
        let value_register = self.compile_expr(&args[0])?;
        self.code
            .push(Instruction::new(Opcode::TypeOf, target, value_register, 0));
        self.free_register(value_register);
        Ok(())
    }

    fn compile_math_abs_call(&mut self, args: &[Expr], target: u8, _span: Span) -> Result<()> {
        self.compile_expr_to(&args[0], target)?;
        let mut ignored_registers = Vec::new();
        for arg in args.iter().skip(1) {
            ignored_registers.push(self.compile_expr(arg)?);
        }
        for register in ignored_registers.into_iter().rev() {
            self.free_register(register);
        }
        self.code
            .push(Instruction::new(Opcode::Abs, target, target, 0));
        Ok(())
    }

    fn compile_math_unary_call(
        &mut self,
        args: &[Expr],
        target: u8,
        span: Span,
        opcode: Opcode,
        name: &str,
    ) -> Result<()> {
        if args.is_empty() {
            return Err(CompileError::at(format!("unsupported {name} arity"), span));
        }
        self.compile_expr_to(&args[0], target)?;
        let mut ignored_registers = Vec::new();
        for arg in args.iter().skip(1) {
            ignored_registers.push(self.compile_expr(arg)?);
        }
        for register in ignored_registers.into_iter().rev() {
            self.free_register(register);
        }
        self.code
            .push(Instruction::with_aux(opcode, target, target, 0, 1));
        Ok(())
    }

    fn compile_math_window_call(
        &mut self,
        args: &[Expr],
        target: u8,
        span: Span,
        spec: MathWindowCallSpec,
    ) -> Result<()> {
        if args.len() < spec.min_arity || args.len() > u8::MAX as usize {
            return Err(CompileError::at(
                format!("unsupported {} arity", spec.name),
                span,
            ));
        }
        let consumed_arity = if spec.max_arity == u8::MAX as usize {
            args.len()
        } else {
            spec.max_arity.min(args.len())
        };
        let argc = u8::try_from(consumed_arity).map_err(|_| {
            CompileError::at(format!("{} call has too many arguments", spec.name), span)
        })?;
        let mut arg_registers = Vec::with_capacity(args.len());
        for arg in args {
            arg_registers.push(self.compile_expr(arg)?);
        }
        self.emit_instruction(Instruction::with_aux(
            spec.opcode,
            target,
            arg_registers[0],
            0,
            i32::from(argc),
        ));
        for register in arg_registers.into_iter().rev() {
            self.free_register(register);
        }
        Ok(())
    }

    fn compile_rawget_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if args.len() < 2 {
            return Err(CompileError::at("unsupported rawget arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        let key_register = self.compile_expr(&args[1])?;
        let mut ignored_registers = Vec::new();
        for arg in args.iter().skip(2) {
            ignored_registers.push(self.compile_expr(arg)?);
        }
        self.emit_instruction(Instruction::new(
            Opcode::RawGet,
            target,
            target,
            key_register,
        ));
        for register in ignored_registers.into_iter().rev() {
            self.free_register(register);
        }
        self.free_register(key_register);
        Ok(())
    }

    fn compile_rawset_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if args.len() < 3 {
            return Err(CompileError::at("unsupported rawset arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        let key_register = self.compile_expr(&args[1])?;
        let value_register = self.compile_expr(&args[2])?;
        let mut ignored_registers = Vec::new();
        for arg in args.iter().skip(3) {
            ignored_registers.push(self.compile_expr(arg)?);
        }
        self.emit_instruction(Instruction::new(
            Opcode::RawSet,
            target,
            key_register,
            value_register,
        ));
        for register in ignored_registers.into_iter().rev() {
            self.free_register(register);
        }
        self.free_register(value_register);
        self.free_register(key_register);
        self.mark_rawset_mutation(args);
        self.stdlib_tables_may_be_mutated = true;
        Ok(())
    }

    fn compile_rawlen_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if args.is_empty() {
            return Err(CompileError::at("unsupported rawlen arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        let mut ignored_registers = Vec::new();
        for arg in args.iter().skip(1) {
            ignored_registers.push(self.compile_expr(arg)?);
        }
        self.code
            .push(Instruction::new(Opcode::RawLen, target, target, 0));
        for register in ignored_registers.into_iter().rev() {
            self.free_register(register);
        }
        Ok(())
    }

    fn compile_next_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if args.is_empty() || args.len() > 2 {
            return Err(CompileError::at("unsupported next arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        let after_register = if let Some(after) = args.get(1) {
            self.compile_expr(after)?
        } else {
            self.load_constant(Constant::Nil)?
        };
        self.emit_instruction(Instruction::new(
            Opcode::NextKey,
            target,
            target,
            after_register,
        ));
        self.free_register(after_register);
        Ok(())
    }

    fn compile_table_concat_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if args.is_empty() || args.len() > 4 {
            return Err(CompileError::at("unsupported table.concat arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        let separator_register = if let Some(separator) = args.get(1) {
            self.compile_expr(separator)?
        } else {
            self.load_constant(Constant::String(Vec::new()))?
        };
        let mut range_registers = Vec::new();
        for arg in args.iter().skip(2) {
            range_registers.push(self.compile_expr(arg)?);
        }
        self.emit_instruction(Instruction::with_aux(
            Opcode::TableConcat,
            target,
            target,
            separator_register,
            i32::try_from(args.len())
                .map_err(|_| CompileError::at("unsupported table.concat arity", span))?,
        ));
        for register in range_registers.into_iter().rev() {
            self.free_register(register);
        }
        self.free_register(separator_register);
        Ok(())
    }

    fn compile_table_unary_call(
        &mut self,
        args: &[Expr],
        target: u8,
        span: Span,
        opcode: Opcode,
        name: &str,
    ) -> Result<()> {
        if args.len() != 1 {
            return Err(CompileError::at(format!("unsupported {name} arity"), span));
        }
        self.compile_expr_to(&args[0], target)?;
        self.emit_instruction(Instruction::new(opcode, target, target, 0));
        Ok(())
    }

    fn compile_table_find_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if !(2..=3).contains(&args.len()) {
            return Err(CompileError::at("unsupported table.find arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        let needle_register = self.compile_expr(&args[1])?;
        let start_register = if let Some(start) = args.get(2) {
            Some(self.compile_expr(start)?)
        } else {
            None
        };
        self.emit_instruction(Instruction::with_aux(
            Opcode::TableFind,
            target,
            target,
            needle_register,
            i32::try_from(args.len())
                .map_err(|_| CompileError::at("unsupported table.find arity", span))?,
        ));
        if let Some(register) = start_register {
            self.free_register(register);
        }
        self.free_register(needle_register);
        Ok(())
    }

    fn compile_table_create_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if !(1..=2).contains(&args.len()) {
            return Err(CompileError::at("unsupported table.create arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        let value_register = if let Some(value) = args.get(1) {
            Some(self.compile_expr(value)?)
        } else {
            None
        };
        self.emit_instruction(Instruction::with_aux(
            Opcode::TableCreate,
            target,
            target,
            value_register.unwrap_or(0),
            i32::try_from(args.len())
                .map_err(|_| CompileError::at("unsupported table.create arity", span))?,
        ));
        if let Some(register) = value_register {
            self.free_register(register);
        }
        Ok(())
    }

    fn compile_table_insert_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if !(2..=3).contains(&args.len()) {
            return Err(CompileError::at("unsupported table.insert arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        let first_arg_register = self.compile_expr(&args[1])?;
        let value_register = if let Some(value) = args.get(2) {
            Some(self.compile_expr(value)?)
        } else {
            None
        };
        self.emit_instruction(Instruction::with_aux(
            Opcode::TableInsert,
            target,
            target,
            first_arg_register,
            i32::try_from(args.len())
                .map_err(|_| CompileError::at("unsupported table.insert arity", span))?,
        ));
        if let Some(register) = value_register {
            self.free_register(register);
        }
        self.free_register(first_arg_register);
        Ok(())
    }

    fn compile_table_remove_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if !(1..=2).contains(&args.len()) {
            return Err(CompileError::at("unsupported table.remove arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        let position_register = if let Some(position) = args.get(1) {
            Some(self.compile_expr(position)?)
        } else {
            None
        };
        self.emit_instruction(Instruction::with_aux(
            Opcode::TableRemove,
            target,
            target,
            position_register.unwrap_or(0),
            i32::try_from(args.len())
                .map_err(|_| CompileError::at("unsupported table.remove arity", span))?,
        ));
        if let Some(register) = position_register {
            self.free_register(register);
        }
        Ok(())
    }

    fn compile_string_sub_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if !(2..=3).contains(&args.len()) {
            return Err(CompileError::at("unsupported string.sub arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        let start_register = self.compile_expr(&args[1])?;
        let end_register = if let Some(end) = args.get(2) {
            Some(self.compile_expr(end)?)
        } else {
            None
        };
        self.emit_instruction(Instruction::with_aux(
            Opcode::StringSub,
            target,
            target,
            start_register,
            i32::try_from(args.len())
                .map_err(|_| CompileError::at("unsupported string.sub arity", span))?,
        ));
        if let Some(register) = end_register {
            self.free_register(register);
        }
        self.free_register(start_register);
        Ok(())
    }

    fn compile_string_len_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if args.len() != 1 {
            return Err(CompileError::at("unsupported string.len arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        self.code
            .push(Instruction::new(Opcode::StringLen, target, target, 0));
        Ok(())
    }

    fn compile_string_byte_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if args.is_empty() || args.len() > 3 {
            return Err(CompileError::at("unsupported string.byte arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        let mut index_registers = Vec::new();
        for arg in args.iter().skip(1) {
            index_registers.push(self.compile_expr(arg)?);
        }
        let start_register = index_registers.first().copied().unwrap_or(0);
        self.emit_instruction(Instruction::with_aux(
            Opcode::StringByte,
            target,
            target,
            start_register,
            i32::try_from(args.len())
                .map_err(|_| CompileError::at("unsupported string.byte arity", span))?,
        ));
        for register in index_registers.into_iter().rev() {
            self.free_register(register);
        }
        Ok(())
    }

    fn compile_string_unary_call(
        &mut self,
        args: &[Expr],
        target: u8,
        span: Span,
        opcode: Opcode,
        name: &str,
    ) -> Result<()> {
        if args.len() != 1 {
            return Err(CompileError::at(format!("unsupported {name} arity"), span));
        }
        self.compile_expr_to(&args[0], target)?;
        self.emit_instruction(Instruction::new(opcode, target, target, 0));
        Ok(())
    }

    fn compile_string_rep_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if args.len() < 2 {
            return Err(CompileError::at("unsupported string.rep arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        let count_register = self.compile_expr(&args[1])?;
        let mut ignored_registers = Vec::new();
        for arg in args.iter().skip(2) {
            ignored_registers.push(self.compile_expr(arg)?);
        }
        for register in ignored_registers.into_iter().rev() {
            self.free_register(register);
        }
        self.emit_instruction(Instruction::with_aux(
            Opcode::StringRep,
            target,
            target,
            count_register,
            i32::try_from(args.len())
                .map_err(|_| CompileError::at("unsupported string.rep arity", span))?,
        ));
        self.free_register(count_register);
        Ok(())
    }

    fn compile_string_find_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if !(2..=4).contains(&args.len()) {
            return Err(CompileError::at("unsupported string.find arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        let pattern_register = self.compile_expr(&args[1])?;
        let mut optional_registers = Vec::new();
        for arg in args.iter().skip(2) {
            optional_registers.push(self.compile_expr(arg)?);
        }
        self.emit_instruction(Instruction::with_aux(
            Opcode::StringFind,
            target,
            target,
            pattern_register,
            i32::try_from(args.len())
                .map_err(|_| CompileError::at("unsupported string.find arity", span))?,
        ));
        for register in optional_registers.into_iter().rev() {
            self.free_register(register);
        }
        self.free_register(pattern_register);
        Ok(())
    }

    fn compile_string_split_call(&mut self, args: &[Expr], target: u8, span: Span) -> Result<()> {
        if !(1..=2).contains(&args.len()) {
            return Err(CompileError::at("unsupported string.split arity", span));
        }
        self.compile_expr_to(&args[0], target)?;
        let separator_register = if let Some(separator) = args.get(1) {
            Some(self.compile_expr(separator)?)
        } else {
            None
        };
        self.emit_instruction(Instruction::with_aux(
            Opcode::StringSplit,
            target,
            target,
            separator_register.unwrap_or(0),
            i32::try_from(args.len())
                .map_err(|_| CompileError::at("unsupported string.split arity", span))?,
        ));
        if let Some(register) = separator_register {
            self.free_register(register);
        }
        Ok(())
    }

    fn compile_conformance_concat_call(
        &mut self,
        args: &[Expr],
        target: u8,
        span: Span,
    ) -> Result<()> {
        let argc = u8::try_from(args.len())
            .map_err(|_| CompileError::at("concat helper call has too many arguments", span))?;
        if args.is_empty() {
            self.load_constant_to(target, Constant::String(Vec::new()))?;
            return Ok(());
        }

        let mut arg_registers = Vec::with_capacity(args.len());
        for arg in args {
            arg_registers.push(self.compile_expr(arg)?);
        }
        self.emit_instruction(Instruction::new(
            Opcode::ConcatArgs,
            target,
            arg_registers[0],
            argc,
        ));
        for register in arg_registers.into_iter().rev() {
            self.free_register(register);
        }
        Ok(())
    }

    fn load_constant(&mut self, constant: Constant) -> Result<u8> {
        let register = self.alloc_register_at(Span::new(0, 0))?;
        self.load_constant_to(register, constant)?;
        Ok(register)
    }

    fn load_constant_to(&mut self, register: u8, constant: Constant) -> Result<()> {
        let constant = self.constant_index(constant)?;
        self.emit_constant_index_to(register, constant)
    }

    fn emit_constant_index_to(&mut self, register: u8, constant: usize) -> Result<()> {
        if let Ok(constant) = u8::try_from(constant) {
            self.code
                .push(Instruction::new(Opcode::LoadConst, register, 0, constant));
        } else {
            let constant =
                i32::try_from(constant).map_err(|_| CompileError::new("constant pool overflow"))?;
            self.emit_instruction(Instruction::with_aux(
                Opcode::LoadConstWide,
                register,
                0,
                0,
                constant,
            ));
        }
        Ok(())
    }

    fn string_constant_index(&mut self, bytes: &[u8], span: Span) -> Result<i32> {
        let index = self.constant_index(Constant::String(bytes.to_vec()))?;
        i32::try_from(index).map_err(|_| CompileError::at("string constant index overflow", span))
    }

    fn compiled_string_table_key(
        &mut self,
        bytes: &[u8],
        span: Span,
        preload: bool,
    ) -> Result<CompiledTableKey> {
        let index = self.string_constant_index(bytes, span)?;
        let preloaded = if preload {
            let register = self.alloc_register_at(span)?;
            debug_assert_ne!(register, 0, "register zero is the fused-key sentinel");
            let constant = usize::try_from(index)
                .map_err(|_| CompileError::at("negative string constant index", span))?;
            self.emit_constant_index_to(register, constant)?;
            Some(register)
        } else {
            None
        };
        Ok(CompiledTableKey::StringConstant { index, preloaded })
    }

    fn emit_namecall_lookup(
        &mut self,
        callee: u8,
        receiver: u8,
        method: &str,
        checked: bool,
        span: Span,
    ) -> Result<()> {
        let method = self.string_constant_index(method.as_bytes(), span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::NameCall,
                callee,
                receiver,
                u8::from(checked),
                method,
            ),
            span,
        );
        Ok(())
    }

    fn clear_local_cell(&mut self, register: u8) {
        self.code
            .push(Instruction::new(Opcode::ClearLocalCell, register, 0, 0));
    }

    fn clear_local_cells_from(&mut self, start_register: u16) -> Result<()> {
        for register in start_register..self.next_register {
            let register =
                u8::try_from(register).map_err(|_| CompileError::new("register file overflow"))?;
            self.clear_local_cell(register);
        }
        Ok(())
    }

    fn global_key_index(&mut self, name: &str) -> Result<i32> {
        i32::try_from(self.constant_index(Constant::String(name.as_bytes().to_vec()))?)
            .map_err(|_| CompileError::new("constant pool overflow"))
    }

    fn argument_may_run_guest_code(&self, argument: &Expr) -> bool {
        if const_eval(argument).is_some() {
            return false;
        }
        match argument {
            Expr::Name(name, _) => !self.name_is_lexically_bound(name),
            Expr::Paren { expr, .. } | Expr::TypeInstantiation { expr, .. } => {
                self.argument_may_run_guest_code(expr)
            }
            _ => true,
        }
    }

    fn method_receiver_register(&self, object: &Expr) -> Option<u8> {
        match object {
            // Grouped locals retain their register. Explicit instantiation evaluates into
            // a temporary before the arguments, even when its operand is a local.
            Expr::Paren { expr, .. } => self.method_receiver_register(expr),
            Expr::Name(name, _) => match self.locals.get(name) {
                Some(Binding::Register(register) | Binding::CapturedLocal(register)) => {
                    Some(*register)
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn compile_expanded_call_prefix(
        &mut self,
        callee: ExpandedCallTarget<'_>,
        args: &[Expr],
        scalar_count: usize,
        base: u8,
        span: Span,
    ) -> Result<()> {
        match callee {
            ExpandedCallTarget::Value(value) => self.compile_expanded_callee(value, args, base)?,
            ExpandedCallTarget::Method { object, .. } => {
                let receiver = checked_register_offset(base, 1, span)?;
                if self.method_receiver_register(object).is_none() {
                    self.compile_expr_to(object, receiver)?;
                }
            }
        }
        for (index, arg) in args[..scalar_count].iter().enumerate() {
            let register = checked_register_offset(base, index + 1 + callee.implicit_args(), span)?;
            self.compile_expr_to(arg, register)?;
        }
        Ok(())
    }

    fn try_compile_general_expanded_call(
        &mut self,
        callee: ExpandedCallTarget<'_>,
        args: &[Expr],
        finish: ExpandedCallFinish,
        span: Span,
    ) -> Result<bool> {
        let mut levels = Vec::new();
        let mut target = callee;
        let mut arguments = args;
        let append_varargs = loop {
            let trailing = arguments.last().filter(|last| self.expr_can_expand(last));
            let scalar_count = arguments.len() - usize::from(trailing.is_some());
            levels.push((target, arguments, scalar_count));
            match trailing {
                Some(Expr::Call { callee, args, .. }) => {
                    target = ExpandedCallTarget::Value(callee);
                    arguments = args;
                }
                Some(Expr::MethodCall {
                    object,
                    method,
                    args,
                    ..
                }) => {
                    target = ExpandedCallTarget::Method { object, method };
                    arguments = args;
                }
                Some(Expr::Varargs(vararg_span)) => {
                    self.validate_varargs_available(*vararg_span)?;
                    break true;
                }
                _ => break false,
            }
        };
        let needs_general = match finish {
            ExpandedCallFinish::Fixed { .. } => levels.len() > if append_varargs { 2 } else { 4 },
            ExpandedCallFinish::Return { .. } => levels.len() > 3,
            ExpandedCallFinish::Append { .. } => levels.len() > 2 || append_varargs,
        };
        if !needs_general
            && !levels
                .iter()
                .any(|(target, _, _)| matches!(target, ExpandedCallTarget::Method { .. }))
        {
            return Ok(false);
        }
        let mut calls = Vec::with_capacity(levels.len());
        let mut window_len = 0usize;
        for (target, _, scalar_count) in &levels {
            let fixed_args = scalar_count + target.implicit_args();
            let fixed_args = u8::try_from(fixed_args)
                .map_err(|_| CompileError::at("function call has too many arguments", span))?;
            window_len = window_len
                .checked_add(1 + usize::from(fixed_args))
                .ok_or_else(|| CompileError::at("expanded call register window overflow", span))?;
            calls.push(CallDescriptorCall {
                fixed_args,
                deferred_import: None,
                deferred_method: None,
                method_receiver_register: None,
            });
        }
        let original_next_register = self.next_register;
        let (base, instruction, aux) = match finish {
            ExpandedCallFinish::Fixed { first, count } => {
                self.validate_call_multi_shape(0, count, span)?;
                self.ensure_register_window_len(first, window_len.max(count), span)?;
                (
                    first,
                    Instruction::new(Opcode::CallExpand, first, 0, 0),
                    count as i32,
                )
            }
            ExpandedCallFinish::Return { prefix, count } => {
                let count = u8::try_from(count)
                    .map_err(|_| CompileError::at("return list is too large", span))?;
                let base = self.alloc_register_window_at(window_len, span)?;
                (
                    base,
                    Instruction::new(Opcode::ReturnCallExpand, prefix, count, base),
                    0,
                )
            }
            ExpandedCallFinish::Append { table, index } => {
                let base = self.alloc_register_window_at(window_len, span)?;
                (
                    base,
                    Instruction::new(Opcode::AppendCallExpand, table, base, 0),
                    pack_append_call_expand_aux(index, 0, span)?,
                )
            }
        };
        let mut cursor = usize::from(base);
        for ((target, arguments, scalar_count), call) in levels.into_iter().zip(&mut calls) {
            if let Some(callee) = target.value() {
                self.mark_global_environment_escape_for_call(callee, arguments);
                call.deferred_import = self.deferred_builtin_import(callee, arguments)?;
            }
            if let ExpandedCallTarget::Method { object, method } = target {
                call.method_receiver_register = self.method_receiver_register(object);
                call.deferred_method =
                    Some(self.string_constant_index(method.as_bytes(), span)? as u32);
            }
            self.compile_expanded_call_prefix(target, arguments, scalar_count, cursor as u8, span)?;
            cursor += 1 + usize::from(call.fixed_args);
        }
        let index = self.constant_index(Constant::CallDescriptor(Box::new(CallDescriptor {
            aux,
            calls,
            append_varargs,
        })))?;
        let mut instruction = instruction;
        instruction.aux = !i32::try_from(index)
            .map_err(|_| CompileError::at("call descriptor index overflow", span))?;
        self.emit_source_mapped_instruction(instruction, span);
        if !matches!(finish, ExpandedCallFinish::Return { .. }) {
            self.free_registers_back_to(original_next_register);
        }
        Ok(true)
    }

    fn compile_expanded_callee(&mut self, callee: &Expr, args: &[Expr], target: u8) -> Result<()> {
        if self.deferred_builtin_import(callee, args)?.is_some() {
            // The descriptor performs and charges this lookup after arguments.
            Ok(())
        } else {
            self.compile_expr_to(callee, target)
        }
    }

    fn emit_expanded_call_instruction(
        &mut self,
        mut instruction: Instruction,
        callee: Option<&Expr>,
        args: &[Expr],
        span: Span,
    ) -> Result<()> {
        let mut calls = Vec::new();
        let mut current_callee = callee;
        let mut current_args = args;
        let mut implicit_args = usize::from(callee.is_none());
        let append_varargs = matches!(
            instruction.op,
            Opcode::CallExpandVarargs
                | Opcode::ReturnCallExpandVarargs
                | Opcode::ReturnCallExpand2Varargs
        );
        let levels = match instruction.op {
            Opcode::CallExpand3 => 4,
            Opcode::CallExpand2 | Opcode::ReturnCallExpand2 | Opcode::ReturnCallExpand2Varargs => 3,
            _ => 2,
        };
        for level in 0..levels {
            let deferred_import = match current_callee {
                Some(callee) => self.deferred_builtin_import(callee, current_args)?,
                None => None,
            };
            let fixed_args = current_args.len() + implicit_args
                - usize::from(level + 1 < levels || append_varargs);
            calls.push(CallDescriptorCall {
                fixed_args: u8::try_from(fixed_args)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                deferred_import,
                deferred_method: None,
                method_receiver_register: None,
            });
            match current_args.last() {
                Some(Expr::Call { callee, args, .. }) => {
                    current_callee = Some(callee);
                    current_args = args;
                    implicit_args = 0;
                }
                Some(Expr::MethodCall { args, .. }) => {
                    current_callee = None;
                    current_args = args;
                    implicit_args = 1;
                }
                _ => break,
            }
        }
        if calls.iter().any(|call| call.deferred_import.is_some()) {
            let aux = match instruction.op {
                Opcode::CallExpand => instruction.aux,
                Opcode::AppendCallExpand => instruction.aux & !255,
                _ => 0,
            };
            let index =
                self.constant_index(Constant::CallDescriptor(Box::new(CallDescriptor {
                    aux,
                    calls,
                    append_varargs,
                })))?;
            instruction.aux = !i32::try_from(index)
                .map_err(|_| CompileError::at("call descriptor index overflow", span))?;
        }
        self.emit_source_mapped_instruction(instruction, span);
        Ok(())
    }

    fn deferred_builtin_import(&mut self, callee: &Expr, args: &[Expr]) -> Result<Option<u32>> {
        let path = match callee {
            Expr::Name(name, _)
                if builtins::global(name)
                    || (name == "select"
                        && args.len() == 2
                        && matches!(args[1], Expr::Varargs(_))) =>
            {
                vec![name.as_str()]
            }
            Expr::Field { object, field, .. } => {
                let Expr::Name(table, _) = object.as_ref() else {
                    return Ok(None);
                };
                if !builtins::field(table, field) {
                    return Ok(None);
                }
                vec![table.as_str(), field.as_str()]
            }
            _ => return Ok(None),
        };
        if self.name_is_lexically_bound(path[0]) || self.written_globals.contains(path[0]) {
            return Ok(None);
        }
        let mut components = Vec::with_capacity(path.len());
        for name in path {
            let index = self.constant_index(Constant::String(name.as_bytes().to_vec()))?;
            components.push(
                u32::try_from(index)
                    .map_err(|_| CompileError::new("import component index overflow"))?,
            );
        }
        let descriptor = self.constant_index(Constant::Import(components))?;
        Ok(Some(u32::try_from(descriptor).map_err(|_| {
            CompileError::new("import descriptor index overflow")
        })?))
    }

    fn builtin_global_is_unmodified(&self, name: &str) -> bool {
        !self.disable_builtin_fastpaths
            && !self.global_environment_escaped
            && !self.locals.contains_key(name)
            && !self.assigned_globals.contains(name)
    }

    fn name_is_lexically_bound(&self, name: &str) -> bool {
        self.locals.contains_key(name)
            || self
                .capture_stack
                .iter()
                .rev()
                .any(|scope| scope.locals.contains_key(name))
    }

    fn expr_is_fastcall_candidate(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Name(name, _) => self.name_is_fastcall_candidate(name),
            Expr::Field { object, .. } => dotted_import_root(object).is_some_and(|root| {
                matches!(
                    root,
                    "bit32"
                        | "buffer"
                        | "coroutine"
                        | "debug"
                        | "math"
                        | "os"
                        | "string"
                        | "table"
                        | "utf8"
                        | "vector"
                ) && !self.name_is_lexically_bound(root)
            }),
            Expr::Paren { expr, .. } => self.expr_is_fastcall_candidate(expr),
            _ => false,
        }
    }

    fn name_is_fastcall_candidate(&self, name: &str) -> bool {
        match self.locals.get(name) {
            Some(Binding::Register(_) | Binding::CapturedLocal(_) | Binding::CurrentFunction) => {
                return self.fastcall_candidate_names.contains(name);
            }
            Some(Binding::Upvalue(_)) | None => {}
        }
        for scope in self.capture_stack.iter().rev() {
            match scope.locals.get(name) {
                Some(
                    Binding::Register(_) | Binding::CapturedLocal(_) | Binding::CurrentFunction,
                ) => return scope.fastcall_candidate_names.contains(name),
                Some(Binding::Upvalue(_)) | None => {}
            }
        }
        false
    }

    fn mark_global_environment_escape_for_call(&mut self, callee: &Expr, args: &[Expr]) {
        let name = mutation_source_name(callee);
        if name == Some("rawset") && self.builtin_global_is_unmodified("rawset") {
            self.mark_rawset_mutation(args);
            self.stdlib_tables_may_be_mutated = true;
        }
        if matches!(name, Some("getfenv" | "setfenv")) {
            self.global_environment_escaped = true;
        }
    }

    fn builtin_table_field_is_unmodified(&self, table: &str, field: &str) -> bool {
        self.builtin_global_is_unmodified(table)
            && !self.stdlib_tables_may_be_mutated
            && !self
                .assigned_stdlib_fields
                .contains(&(table.to_owned(), field.to_owned()))
    }

    fn resolve_upvalue(&mut self, name: &str) -> Result<Option<u8>> {
        if let Some(binding) = self.locals.get(name).copied() {
            return match binding {
                Binding::Upvalue(index) => Ok(Some(index)),
                Binding::Register(_) | Binding::CapturedLocal(_) | Binding::CurrentFunction => {
                    Ok(None)
                }
            };
        }
        let Some(scope_index) = self.capture_stack.len().checked_sub(1) else {
            return Ok(None);
        };
        let Some(descriptor) = self.resolve_upvalue_from_scope(scope_index, name) else {
            return Ok(None);
        };
        let upvalue_index = self.intern_current_upvalue(descriptor, name)?;
        Ok(Some(upvalue_index))
    }

    fn resolve_upvalue_from_scope(
        &mut self,
        scope_index: usize,
        name: &str,
    ) -> Option<UpvalueDescriptor> {
        if let Some(binding) = self.capture_stack[scope_index].locals.get(name).copied() {
            match binding {
                Binding::Register(register) | Binding::CapturedLocal(register) => {
                    self.capture_stack[scope_index]
                        .locals
                        .insert(name.to_owned(), Binding::CapturedLocal(register));
                    self.capture_stack[scope_index]
                        .captured_registers
                        .insert(register);
                    return Some(UpvalueDescriptor::ParentLocal(register));
                }
                Binding::Upvalue(index) => return Some(UpvalueDescriptor::ParentUpvalue(index)),
                Binding::CurrentFunction => {}
            }
        }

        let parent_scope_index = scope_index.checked_sub(1)?;
        let descriptor = self.resolve_upvalue_from_scope(parent_scope_index, name)?;
        let upvalue_index = self.intern_scope_upvalue(scope_index, descriptor, name)?;
        Some(UpvalueDescriptor::ParentUpvalue(upvalue_index))
    }

    fn intern_current_upvalue(&mut self, descriptor: UpvalueDescriptor, name: &str) -> Result<u8> {
        if let Some(Binding::Upvalue(index)) = self.locals.get(name).copied() {
            return Ok(index);
        }
        // Exiting a lexical block restores its visible local-name map, but
        // captures already emitted by that block still belong to this function.
        // Resolve by descriptor before allocating another slot for that binding.
        if let Some(index) = self.upvalues.iter().position(|saved| *saved == descriptor) {
            let index =
                u8::try_from(index).map_err(|_| CompileError::new("too many function upvalues"))?;
            self.locals.insert(name.to_owned(), Binding::Upvalue(index));
            return Ok(index);
        }
        let index = u8::try_from(self.upvalues.len())
            .map_err(|_| CompileError::new("too many function upvalues"))?;
        self.upvalues.push(descriptor);
        self.upvalue_names.push(name.to_owned());
        self.locals.insert(name.to_owned(), Binding::Upvalue(index));
        Ok(index)
    }

    fn intern_scope_upvalue(
        &mut self,
        scope_index: usize,
        descriptor: UpvalueDescriptor,
        name: &str,
    ) -> Option<u8> {
        if let Some(Binding::Upvalue(index)) =
            self.capture_stack[scope_index].locals.get(name).copied()
        {
            return Some(index);
        }
        let scope = &mut self.capture_stack[scope_index];
        if let Some(index) = scope.upvalues.iter().position(|saved| *saved == descriptor) {
            let index = u8::try_from(index).ok()?;
            scope
                .locals
                .insert(name.to_owned(), Binding::Upvalue(index));
            return Some(index);
        }
        let index = u8::try_from(self.capture_stack[scope_index].upvalues.len()).ok()?;
        self.capture_stack[scope_index].upvalues.push(descriptor);
        self.capture_stack[scope_index]
            .upvalue_names
            .push(name.to_owned());
        self.capture_stack[scope_index]
            .locals
            .insert(name.to_owned(), Binding::Upvalue(index));
        Some(index)
    }

    fn global_table_alias_is_unmodified(&self) -> bool {
        self.builtin_global_is_unmodified("_G")
    }

    fn global_table_field_assignment(&self, object: &Expr, field: &str) -> Option<String> {
        if mutation_source_name(object) == Some("_G") && self.global_table_alias_is_unmodified() {
            Some(field.to_owned())
        } else {
            None
        }
    }

    fn global_table_index_assignment(&self, object: &Expr, index: &Expr) -> Option<String> {
        if mutation_source_name(object) != Some("_G") || !self.global_table_alias_is_unmodified() {
            return None;
        }
        let Some(Constant::String(name)) = const_eval(index) else {
            return None;
        };
        String::from_utf8(name).ok()
    }

    fn mark_rawset_mutation(&mut self, args: &[Expr]) {
        let Some(object) = args.first() else {
            return;
        };
        let Some(index) = args.get(1) else {
            return;
        };
        if mutation_source_name(object) != Some("_G") || !self.global_table_alias_is_unmodified() {
            return;
        }
        let Some(Constant::String(name)) = const_eval(index) else {
            self.global_environment_escaped = true;
            return;
        };
        match String::from_utf8(name) {
            Ok(name) => {
                self.assigned_globals.insert(name);
            }
            Err(_) => {
                self.global_environment_escaped = true;
            }
        }
    }

    fn assignment_target_stdlib_field(
        &self,
        target: &AssignmentTarget,
    ) -> Option<(String, String)> {
        match target {
            AssignmentTarget::Field { object, field, .. } => {
                self.stdlib_table_field_assignment(object, field)
            }
            AssignmentTarget::Index { object, index, .. } => {
                self.stdlib_table_index_assignment(object, index)
            }
            AssignmentTarget::Local { .. } => None,
        }
    }

    fn stdlib_table_field_assignment(
        &self,
        object: &Expr,
        field: &str,
    ) -> Option<(String, String)> {
        let Expr::Name(table, _) = object else {
            return None;
        };
        if matches!(table.as_str(), "math" | "string" | "table")
            && self.builtin_global_is_unmodified(table)
        {
            Some((table.clone(), field.to_owned()))
        } else {
            None
        }
    }

    fn stdlib_table_index_assignment(
        &self,
        object: &Expr,
        index: &Expr,
    ) -> Option<(String, String)> {
        let Some(Constant::String(field)) = const_eval(index) else {
            return None;
        };
        let field = String::from_utf8(field).ok()?;
        self.stdlib_table_field_assignment(object, &field)
    }

    fn mark_assignment_target_mutations(&mut self, target: &CompiledAssignmentTarget) {
        match target {
            CompiledAssignmentTarget::Global { name, .. } => {
                self.assigned_globals.insert(name.clone());
            }
            CompiledAssignmentTarget::Table {
                global_field: Some(name),
                ..
            } => {
                self.assigned_globals.insert(name.clone());
            }
            CompiledAssignmentTarget::Table {
                stdlib_field: Some((table, field)),
                ..
            } => {
                self.assigned_stdlib_fields
                    .insert((table.clone(), field.clone()));
            }
            CompiledAssignmentTarget::Table {
                global_field: None,
                stdlib_field: None,
                ..
            } => {
                self.stdlib_tables_may_be_mutated = true;
            }
            CompiledAssignmentTarget::Local(_)
            | CompiledAssignmentTarget::CapturedLocal(_)
            | CompiledAssignmentTarget::Upvalue(_) => {}
        }
    }

    fn write_assignment_target_from_source(
        &mut self,
        target: &CompiledAssignmentTarget,
        source: u8,
    ) {
        match target {
            CompiledAssignmentTarget::Local(register) => {
                if source != *register {
                    self.code
                        .push(Instruction::new(Opcode::Move, *register, source, 0));
                }
                if self.local_register_is_captured(*register) {
                    self.code
                        .push(Instruction::new(Opcode::SetLocalCell, source, *register, 0));
                }
            }
            CompiledAssignmentTarget::CapturedLocal(register) => {
                if source != *register {
                    self.code
                        .push(Instruction::new(Opcode::Move, *register, source, 0));
                }
                self.code
                    .push(Instruction::new(Opcode::SetLocalCell, source, *register, 0));
            }
            CompiledAssignmentTarget::Upvalue(index) => {
                self.emit_instruction(Instruction::with_aux(
                    Opcode::SetUpvalue,
                    source,
                    0,
                    0,
                    i32::from(*index),
                ));
            }
            CompiledAssignmentTarget::Table {
                table,
                key,
                key_span,
                ..
            } => {
                self.emit_source_mapped_instruction(
                    table_set_instruction(*table, *key, source),
                    *key_span,
                );
            }
            CompiledAssignmentTarget::Global { key, .. } => {
                self.code
                    .push(Instruction::with_aux(Opcode::SetGlobal, source, 0, 0, *key));
            }
        }
    }

    fn free_assignment_target(&mut self, target: CompiledAssignmentTarget) {
        match target {
            CompiledAssignmentTarget::Local(_) => {}
            CompiledAssignmentTarget::CapturedLocal(_) => {}
            CompiledAssignmentTarget::Upvalue(_) => {}
            CompiledAssignmentTarget::Table { table, key, .. } => {
                if let Some(key) = key.register() {
                    self.free_register(key);
                }
                self.free_register(table);
            }
            CompiledAssignmentTarget::Global { .. } => {}
        }
    }

    fn local_register_is_captured(&self, register: u8) -> bool {
        self.captured_registers.contains(&register)
    }

    fn constant_index(&mut self, constant: Constant) -> Result<usize> {
        if let Some(index) = self
            .constants
            .iter()
            .position(|existing| constants_equal(existing, &constant))
        {
            return Ok(index);
        }
        if self.constants.len() >= i32::MAX as usize {
            return Err(CompileError::new("constant pool overflow"));
        }
        self.constants.push(constant);
        Ok(self.constants.len() - 1)
    }

    fn alloc_register_at(&mut self, span: Span) -> Result<u8> {
        if self.next_register > u8::MAX as u16 {
            return Err(CompileError::at("register file overflow", span));
        }
        let register = self.next_register as u8;
        self.next_register += 1;
        self.max_registers = self.max_registers.max(self.next_register);
        Ok(register)
    }

    fn alloc_register_window_at(&mut self, count: usize, span: Span) -> Result<u8> {
        if count == 0 {
            return Err(CompileError::at("register window cannot be empty", span));
        }
        let first = self.alloc_register_at(span)?;
        for _ in 1..count {
            self.alloc_register_at(span)?;
        }
        Ok(first)
    }

    fn ensure_register_window_len(
        &mut self,
        first_register: u8,
        len: usize,
        span: Span,
    ) -> Result<()> {
        let end = usize::from(first_register)
            .checked_add(len)
            .ok_or_else(|| CompileError::at("register window overflow", span))?;
        if end > usize::from(u8::MAX) + 1 {
            return Err(CompileError::at("register file overflow", span));
        }
        while usize::from(self.next_register) < end {
            self.alloc_register_at(span)?;
        }
        Ok(())
    }

    fn free_register(&mut self, register: u8) {
        if self.next_register == u16::from(register) + 1 {
            self.clear_local_cell(register);
            self.next_register -= 1;
        }
    }

    fn free_registers_back_to(&mut self, next_register: u16) {
        while self.next_register > next_register {
            self.next_register -= 1;
            self.clear_local_cell(self.next_register as u8);
        }
    }

    fn emit_source_mapped_instruction(&mut self, instruction: Instruction, span: Span) {
        self.record_source_for_next_instruction(span);
        if let Some(context) = self
            .diagnostic_span
            .filter(|context| context.start <= span.start && context.end >= span.end)
        {
            if let Some(entry) = self.source_map.last_mut() {
                entry.span = Some(rustia_vm::diagnostic::DiagnosticSpan {
                    start: context.start,
                    end: context.end,
                });
            }
        }
        self.code.push(instruction);
    }

    fn emit_instruction(&mut self, instruction: Instruction) {
        if let (Some(span), Some(last)) = (self.diagnostic_span, self.source_map.last()) {
            let span = Some(rustia_vm::diagnostic::DiagnosticSpan {
                start: span.start,
                end: span.end,
            });
            // Preserve the guest's existing line map; expression spans are
            // host-only annotations and must not change oracle error strings.
            let pc = self.code.len();
            if last.pc == pc {
                self.source_map
                    .last_mut()
                    .expect("source entry exists")
                    .span = span;
            } else if last.span != span {
                self.source_map.push(InstructionSource {
                    pc,
                    line: last.line,
                    span,
                });
            }
        }
        self.code.push(instruction);
    }

    fn record_source_for_next_instruction(&mut self, span: Span) {
        let Some(line) = self.source_line(span) else {
            return;
        };
        let pc = self.code.len();
        if let Some(last) = self.source_map.last_mut() {
            if last.pc == pc {
                last.line = line;
                last.span = Some(rustia_vm::diagnostic::DiagnosticSpan {
                    start: span.start,
                    end: span.end,
                });
                return;
            }
        }
        self.source_map.push(InstructionSource {
            pc,
            line,
            span: Some(rustia_vm::diagnostic::DiagnosticSpan {
                start: span.start,
                end: span.end,
            }),
        });
    }

    fn source_line(&self, span: Span) -> Option<u32> {
        if self.options.source_line_starts.is_empty() {
            return None;
        }
        let line_index = self
            .options
            .source_line_starts
            .partition_point(|start| *start <= span.start);
        u32::try_from(line_index).ok()
    }

    fn validate_call_multi_shape(
        &self,
        argc: usize,
        result_count: usize,
        span: Span,
    ) -> Result<()> {
        if argc > u8::MAX as usize {
            return Err(CompileError::at(
                "function call has too many arguments",
                span,
            ));
        }
        if result_count > u8::MAX as usize {
            return Err(CompileError::at("function call has too many results", span));
        }
        Ok(())
    }

    fn validate_varargs_available(&self, span: Span) -> Result<()> {
        if self.current_function_is_vararg {
            Ok(())
        } else {
            Err(CompileError::at(
                "cannot use '...' outside a vararg function",
                span,
            ))
        }
    }

    fn compile_varargs_to_count(
        &mut self,
        first_register: u8,
        result_count: usize,
        span: Span,
    ) -> Result<()> {
        self.validate_varargs_available(span)?;
        if result_count > u8::MAX as usize {
            return Err(CompileError::at(
                "vararg expression has too many results",
                span,
            ));
        }
        self.ensure_register_window_len(first_register, result_count, span)?;
        self.emit_instruction(Instruction::new(
            Opcode::LoadVarargs,
            first_register,
            u8::try_from(result_count)
                .map_err(|_| CompileError::at("vararg expression has too many results", span))?,
            0,
        ));
        Ok(())
    }

    fn call_is_conformance_concat(&self, expr: &Expr) -> bool {
        let Expr::Call { callee, .. } = expr else {
            return false;
        };
        matches!(
            callee.as_ref(),
            Expr::Name(name, _)
                if self.options.builtins == BuiltinEnvironment::LuauConformance
                    && name == "concat"
                    && self.builtin_global_is_unmodified(name)
        )
    }

    fn expr_can_expand(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Varargs(_) => true,
            _ => expr_is_call(expr) && !self.call_is_conformance_concat(expr),
        }
    }

    fn value_call_has_trailing_varargs(&self, args: &[Expr]) -> bool {
        matches!(args.last(), Some(Expr::Varargs(_)))
    }

    fn method_call_has_trailing_varargs(&self, args: &[Expr]) -> bool {
        matches!(args.last(), Some(Expr::Varargs(_)))
    }

    fn call_expr_has_trailing_varargs(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Call { args, .. } | Expr::MethodCall { args, .. } => {
                matches!(args.last(), Some(Expr::Varargs(_)))
            }
            _ => false,
        }
    }

    fn value_call_has_expandable_trailing_arg(&self, args: &[Expr]) -> bool {
        let Some(last) = args.last() else {
            return false;
        };
        expr_is_call(last) && !self.call_is_conformance_concat(last)
    }

    fn method_call_has_expandable_trailing_arg(&self, args: &[Expr]) -> bool {
        let Some(last) = args.last() else {
            return false;
        };
        expr_is_call(last) && !self.call_is_conformance_concat(last)
    }

    fn call_expr_has_expandable_trailing_arg(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Call { args, .. } | Expr::MethodCall { args, .. } => {
                self.value_call_has_expandable_trailing_arg(args)
            }
            _ => false,
        }
    }

    fn fixed_expansion_result_count(&self, expr: &Expr) -> Option<usize> {
        let Expr::Call { callee, args, .. } = expr else {
            return None;
        };
        if matches!(callee.as_ref(), Expr::Name(name, _) if name == "assert")
            && self.builtin_global_is_unmodified("assert")
            && !args.last().is_some_and(|arg| self.expr_can_expand(arg))
        {
            return Some(args.len());
        }
        None
    }
}

fn checked_register_offset(first_register: u8, offset: usize, span: Span) -> Result<u8> {
    let register = usize::from(first_register)
        .checked_add(offset)
        .ok_or_else(|| CompileError::at("register window overflow", span))?;
    u8::try_from(register).map_err(|_| CompileError::at("register file overflow", span))
}

fn source_line_starts(bytes: &[u8]) -> Vec<usize> {
    let mut starts = vec![0];
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => {
                index += 1;
                if bytes.get(index) == Some(&b'\r') {
                    index += 1;
                }
                starts.push(index);
            }
            b'\r' => {
                index += 1;
                if bytes.get(index) == Some(&b'\n') {
                    index += 1;
                }
                starts.push(index);
            }
            _ => index += 1,
        }
    }
    starts
}

#[derive(Clone, Copy)]
enum ExpandedCallTarget<'a> {
    Value(&'a Expr),
    Method { object: &'a Expr, method: &'a str },
}

impl<'a> ExpandedCallTarget<'a> {
    fn implicit_args(self) -> usize {
        usize::from(matches!(self, Self::Method { .. }))
    }

    fn value(self) -> Option<&'a Expr> {
        match self {
            Self::Value(value) => Some(value),
            Self::Method { .. } => None,
        }
    }
}

#[derive(Clone, Copy)]
enum ExpandedCallFinish {
    Fixed { first: u8, count: usize },
    Return { prefix: u8, count: usize },
    Append { table: u8, index: f64 },
}

#[derive(Clone, Copy)]
struct NestedCallShape<'a> {
    prefix_count: usize,
    scalar_arg_count: usize,
    trailing: &'a Expr,
}

fn nested_call_shape(expr: &Expr, span: Span) -> Result<NestedCallShape<'_>> {
    match expr {
        Expr::Call { args, .. } => {
            let trailing_index = args
                .len()
                .checked_sub(1)
                .ok_or_else(|| CompileError::at("nested call needs a trailing argument", span))?;
            Ok(NestedCallShape {
                prefix_count: trailing_index,
                scalar_arg_count: trailing_index,
                trailing: &args[trailing_index],
            })
        }
        Expr::MethodCall { args, .. } => {
            let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
                CompileError::at("nested method call needs a trailing argument", span)
            })?;
            Ok(NestedCallShape {
                prefix_count: trailing_index.checked_add(1).ok_or_else(|| {
                    CompileError::at("function call has too many arguments", span)
                })?,
                scalar_arg_count: trailing_index,
                trailing: &args[trailing_index],
            })
        }
        _ => unreachable!("nested_call_shape is only used for calls"),
    }
}

fn nested_expand_window_len(
    outer_prefix_count: usize,
    nested_shape: NestedCallShape<'_>,
    span: Span,
) -> Result<usize> {
    let inner_argc = call_arg_count(nested_shape.trailing, span)?;
    1usize
        .checked_add(outer_prefix_count)
        .and_then(|len| len.checked_add(1))
        .and_then(|len| len.checked_add(nested_shape.prefix_count))
        .and_then(|len| len.checked_add(1))
        .and_then(|len| len.checked_add(inner_argc))
        .ok_or_else(|| CompileError::at("nested expanded call register window overflow", span))
}

fn nested_vararg_window_len(
    outer_prefix_count: usize,
    nested_shape: NestedCallShape<'_>,
    span: Span,
) -> Result<usize> {
    1usize
        .checked_add(outer_prefix_count)
        .and_then(|len| len.checked_add(1))
        .and_then(|len| len.checked_add(nested_shape.prefix_count))
        .ok_or_else(|| CompileError::at("nested vararg call register window overflow", span))
}

fn nested_expand2_vararg_window_len(
    outer_prefix_count: usize,
    middle_shape: NestedCallShape<'_>,
    inner_shape: NestedCallShape<'_>,
    span: Span,
) -> Result<usize> {
    1usize
        .checked_add(outer_prefix_count)
        .and_then(|len| len.checked_add(1))
        .and_then(|len| len.checked_add(middle_shape.prefix_count))
        .and_then(|len| len.checked_add(1))
        .and_then(|len| len.checked_add(inner_shape.prefix_count))
        .ok_or_else(|| CompileError::at("nested vararg call register window overflow", span))
}

fn nested_expand3_window_len(
    outer_prefix_count: usize,
    middle_shape: NestedCallShape<'_>,
    inner_shape: NestedCallShape<'_>,
    span: Span,
) -> Result<usize> {
    let leaf_argc = call_arg_count(inner_shape.trailing, span)?;
    1usize
        .checked_add(outer_prefix_count)
        .and_then(|len| len.checked_add(1))
        .and_then(|len| len.checked_add(middle_shape.prefix_count))
        .and_then(|len| len.checked_add(1))
        .and_then(|len| len.checked_add(inner_shape.prefix_count))
        .and_then(|len| len.checked_add(1))
        .and_then(|len| len.checked_add(leaf_argc))
        .ok_or_else(|| CompileError::at("nested expanded call register window overflow", span))
}

fn pack_return_call_aux(args_start: u8, argc: usize, span: Span) -> Result<i32> {
    if argc > u8::MAX as usize {
        return Err(CompileError::at(
            "function call has too many arguments",
            span,
        ));
    }
    let argc = u8::try_from(argc)
        .map_err(|_| CompileError::at("function call has too many arguments", span))?;
    Ok((i32::from(args_start) << 8) | i32::from(argc))
}

fn pack_call_expand_aux(prefix_count: usize, trailing_argc: usize, span: Span) -> Result<i32> {
    if prefix_count > u8::MAX as usize || trailing_argc > u8::MAX as usize {
        return Err(CompileError::at(
            "function call has too many arguments",
            span,
        ));
    }
    let prefix_count = u8::try_from(prefix_count)
        .map_err(|_| CompileError::at("function call has too many arguments", span))?;
    let trailing_argc = u8::try_from(trailing_argc)
        .map_err(|_| CompileError::at("function call has too many arguments", span))?;
    Ok((i32::from(prefix_count) << 8) | i32::from(trailing_argc))
}

fn pack_append_call_expand_aux(array_index: f64, trailing_argc: usize, span: Span) -> Result<i32> {
    const MAX_PACKED_APPEND_START: u32 = (i32::MAX as u32) >> 8;
    if array_index <= 0.0
        || array_index.fract() != 0.0
        || array_index > f64::from(MAX_PACKED_APPEND_START)
    {
        return Err(CompileError::at(
            "table constructor call index overflow",
            span,
        ));
    }
    if trailing_argc > u8::MAX as usize {
        return Err(CompileError::at(
            "function call has too many arguments",
            span,
        ));
    }
    let array_index = u32::try_from(array_index as u64)
        .map_err(|_| CompileError::at("table constructor call index overflow", span))?;
    let trailing_argc = u8::try_from(trailing_argc)
        .map_err(|_| CompileError::at("function call has too many arguments", span))?;
    let packed = (array_index << 8) | u32::from(trailing_argc);
    i32::try_from(packed)
        .map_err(|_| CompileError::at("table constructor call index overflow", span))
}

fn pack_nested_expand_aux(
    outer_prefix_count: usize,
    nested_shape: NestedCallShape<'_>,
    span: Span,
) -> Result<i32> {
    let inner_argc = call_arg_count(nested_shape.trailing, span)?;
    if outer_prefix_count > u8::MAX as usize
        || nested_shape.prefix_count > u8::MAX as usize
        || inner_argc > u8::MAX as usize
    {
        return Err(CompileError::at(
            "function call has too many arguments",
            span,
        ));
    }
    Ok((i32::try_from(outer_prefix_count)
        .map_err(|_| CompileError::at("function call has too many arguments", span))?
        << 16)
        | (i32::try_from(nested_shape.prefix_count)
            .map_err(|_| CompileError::at("function call has too many arguments", span))?
            << 8)
        | i32::try_from(inner_argc)
            .map_err(|_| CompileError::at("function call has too many arguments", span))?)
}

fn pack_nested_expand3_aux(
    outer_prefix_count: usize,
    middle_prefix_count: usize,
    inner_prefix_count: usize,
    span: Span,
) -> Result<i32> {
    pack_nested_expand2_varargs_aux(
        outer_prefix_count,
        middle_prefix_count,
        inner_prefix_count,
        span,
    )
}

fn pack_nested_expand2_varargs_aux(
    outer_prefix_count: usize,
    middle_prefix_count: usize,
    inner_prefix_count: usize,
    span: Span,
) -> Result<i32> {
    if outer_prefix_count > u8::MAX as usize
        || middle_prefix_count > u8::MAX as usize
        || inner_prefix_count > u8::MAX as usize
    {
        return Err(CompileError::at(
            "function call has too many arguments",
            span,
        ));
    }
    Ok((i32::try_from(outer_prefix_count)
        .map_err(|_| CompileError::at("function call has too many arguments", span))?
        << 16)
        | (i32::try_from(middle_prefix_count)
            .map_err(|_| CompileError::at("function call has too many arguments", span))?
            << 8)
        | i32::try_from(inner_prefix_count)
            .map_err(|_| CompileError::at("function call has too many arguments", span))?)
}

fn pack_expand_varargs_aux(
    outer_prefix_count: usize,
    middle_prefix_count: usize,
    span: Span,
) -> Result<i32> {
    if outer_prefix_count > u8::MAX as usize || middle_prefix_count > u8::MAX as usize {
        return Err(CompileError::at(
            "function call has too many arguments",
            span,
        ));
    }
    Ok((i32::try_from(outer_prefix_count)
        .map_err(|_| CompileError::at("function call has too many arguments", span))?
        << 8)
        | i32::try_from(middle_prefix_count)
            .map_err(|_| CompileError::at("function call has too many arguments", span))?)
}

fn call_arg_count(expr: &Expr, span: Span) -> Result<usize> {
    match expr {
        Expr::Call { args, .. } => Ok(args.len()),
        Expr::MethodCall { args, .. } => args
            .len()
            .checked_add(1)
            .ok_or_else(|| CompileError::at("function call has too many arguments", span)),
        _ => unreachable!("call_arg_count is only used for calls"),
    }
}

fn expr_is_call(expr: &Expr) -> bool {
    matches!(expr, Expr::Call { .. } | Expr::MethodCall { .. })
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CaptureOrigin {
    scope: usize,
    register: u8,
}

#[derive(Clone, Copy)]
enum CaptureParent {
    Missing,
    One(usize),
    Ambiguous,
}

#[derive(Clone, Copy)]
enum CaptureOriginState {
    Unresolved,
    Resolving,
    Resolved(Option<CaptureOrigin>),
}

/// Rewrites only captures whose lexical source is provably never assigned.
/// Existing cell descriptors are the conservative bytecode contract; the two
/// value variants are an optimization emitted after the complete nested
/// function graph and all writes are visible.
fn mark_immutable_capture_descriptors(main: &mut Function, functions: &mut [Function]) {
    let scope_count = functions.len().saturating_add(1);
    let mut parents = vec![CaptureParent::Missing; scope_count];

    for scope in 0..scope_count {
        let function = if scope == 0 {
            &*main
        } else {
            &functions[scope - 1]
        };
        for instruction in &function.code {
            if instruction.op != Opcode::LoadFunction || instruction.aux < 0 {
                continue;
            }
            let Some(child_scope) = usize::try_from(instruction.aux)
                .ok()
                .and_then(|prototype| prototype.checked_add(1))
                .filter(|child| *child < scope_count)
            else {
                continue;
            };
            parents[child_scope] = match parents[child_scope] {
                CaptureParent::Missing => CaptureParent::One(scope),
                CaptureParent::One(existing) if existing == scope => CaptureParent::One(scope),
                CaptureParent::One(_) | CaptureParent::Ambiguous => CaptureParent::Ambiguous,
            };
        }
    }

    let mut origins = functions
        .iter()
        .map(|function| vec![CaptureOriginState::Unresolved; function.upvalues.len()])
        .collect::<Vec<_>>();
    for prototype in 0..functions.len() {
        for upvalue in 0..functions[prototype].upvalues.len() {
            let _ = resolve_capture_origin(prototype, upvalue, functions, &parents, &mut origins);
        }
    }

    let mut mutated = BTreeSet::new();
    for scope in 0..scope_count {
        let function = if scope == 0 {
            &*main
        } else {
            &functions[scope - 1]
        };
        for instruction in &function.code {
            match instruction.op {
                Opcode::SetLocalCell => {
                    mutated.insert(CaptureOrigin {
                        scope,
                        register: instruction.b,
                    });
                }
                Opcode::SetUpvalue if scope != 0 && instruction.aux >= 0 => {
                    if let Some(origin) = usize::try_from(instruction.aux)
                        .ok()
                        .and_then(|index| origins[scope - 1].get(index))
                        .and_then(|state| match state {
                            CaptureOriginState::Resolved(origin) => *origin,
                            CaptureOriginState::Unresolved | CaptureOriginState::Resolving => None,
                        })
                    {
                        mutated.insert(origin);
                    }
                }
                _ => {}
            }
        }
    }

    for (prototype, function) in functions.iter_mut().enumerate() {
        for (index, descriptor) in function.upvalues.iter_mut().enumerate() {
            let immutable = matches!(
                origins[prototype].get(index),
                Some(CaptureOriginState::Resolved(Some(origin))) if !mutated.contains(origin)
            );
            *descriptor = match (*descriptor, immutable) {
                (UpvalueDescriptor::ParentLocal(register), true)
                | (UpvalueDescriptor::ParentLocalValue(register), true) => {
                    UpvalueDescriptor::ParentLocalValue(register)
                }
                (UpvalueDescriptor::ParentUpvalue(index), true)
                | (UpvalueDescriptor::ParentUpvalueValue(index), true) => {
                    UpvalueDescriptor::ParentUpvalueValue(index)
                }
                (UpvalueDescriptor::ParentLocal(register), false)
                | (UpvalueDescriptor::ParentLocalValue(register), false) => {
                    UpvalueDescriptor::ParentLocal(register)
                }
                (UpvalueDescriptor::ParentUpvalue(index), false)
                | (UpvalueDescriptor::ParentUpvalueValue(index), false) => {
                    UpvalueDescriptor::ParentUpvalue(index)
                }
            };
        }
    }
}

fn resolve_capture_origin(
    prototype: usize,
    upvalue: usize,
    functions: &[Function],
    parents: &[CaptureParent],
    states: &mut [Vec<CaptureOriginState>],
) -> Option<CaptureOrigin> {
    match *states.get(prototype)?.get(upvalue)? {
        CaptureOriginState::Resolved(origin) => return origin,
        CaptureOriginState::Resolving => return None,
        CaptureOriginState::Unresolved => {}
    }
    states[prototype][upvalue] = CaptureOriginState::Resolving;

    let child_scope = prototype.checked_add(1)?;
    let parent_scope = match *parents.get(child_scope)? {
        CaptureParent::One(parent) => parent,
        CaptureParent::Missing | CaptureParent::Ambiguous => {
            states[prototype][upvalue] = CaptureOriginState::Resolved(None);
            return None;
        }
    };
    let descriptor = *functions.get(prototype)?.upvalues.get(upvalue)?;
    let origin = match descriptor {
        UpvalueDescriptor::ParentLocal(register)
        | UpvalueDescriptor::ParentLocalValue(register) => Some(CaptureOrigin {
            scope: parent_scope,
            register,
        }),
        UpvalueDescriptor::ParentUpvalue(parent_upvalue)
        | UpvalueDescriptor::ParentUpvalueValue(parent_upvalue) => {
            parent_scope.checked_sub(1).and_then(|parent| {
                resolve_capture_origin(
                    parent,
                    usize::from(parent_upvalue),
                    functions,
                    parents,
                    states,
                )
            })
        }
    };
    states[prototype][upvalue] = CaptureOriginState::Resolved(origin);
    origin
}

#[derive(Clone, Copy)]
enum Binding {
    Register(u8),
    CapturedLocal(u8),
    Upvalue(u8),
    CurrentFunction,
}

#[derive(Clone)]
struct CaptureScope {
    locals: BTreeMap<String, Binding>,
    fastcall_candidate_names: BTreeSet<String>,
    upvalues: Vec<UpvalueDescriptor>,
    upvalue_names: Vec<String>,
    captured_registers: BTreeSet<u8>,
}

#[derive(Clone)]
enum CompiledAssignmentTarget {
    Local(u8),
    CapturedLocal(u8),
    Upvalue(u8),
    Table {
        table: u8,
        key: CompiledTableKey,
        key_span: Span,
        global_field: Option<String>,
        stdlib_field: Option<(String, String)>,
    },
    Global {
        key: i32,
        name: String,
    },
}

#[derive(Clone, Copy)]
enum CompiledTableKey {
    Register(u8),
    StringConstant {
        index: i32,
        /// Register holding the original logical constant load when evaluation must remain
        /// before an intervening right-hand side. `None` means the load is fused here.
        preloaded: Option<u8>,
    },
}

impl CompiledTableKey {
    fn register(self) -> Option<u8> {
        match self {
            Self::Register(register) => Some(register),
            Self::StringConstant { preloaded, .. } => preloaded,
        }
    }
}

fn table_get_instruction(result: u8, table: u8, key: CompiledTableKey) -> Instruction {
    match key {
        CompiledTableKey::Register(key) => Instruction::new(Opcode::GetIndex, result, table, key),
        CompiledTableKey::StringConstant {
            index,
            preloaded: None,
        } => Instruction::with_aux(Opcode::GetTableKs, result, table, 0, index),
        CompiledTableKey::StringConstant {
            preloaded: Some(key),
            ..
        } => Instruction::new(Opcode::GetIndex, result, table, key),
    }
}

fn table_set_instruction(table: u8, key: CompiledTableKey, value: u8) -> Instruction {
    match key {
        CompiledTableKey::Register(key) => Instruction::new(Opcode::SetIndex, table, key, value),
        CompiledTableKey::StringConstant { index, preloaded } => Instruction::with_aux(
            Opcode::SetTableKs,
            table,
            value,
            preloaded.unwrap_or(0),
            index,
        ),
    }
}

#[derive(Clone, Copy)]
enum GenericArrayIterator {
    Ipairs,
    Pairs,
}

#[derive(Clone, Copy)]
struct MathWindowCallSpec {
    opcode: Opcode,
    name: &'static str,
    min_arity: usize,
    max_arity: usize,
}

impl MathWindowCallSpec {
    const fn new(opcode: Opcode, name: &'static str, min_arity: usize, max_arity: usize) -> Self {
        Self {
            opcode,
            name,
            min_arity,
            max_arity,
        }
    }
}

impl GenericArrayIterator {
    fn prep_opcode(self) -> Opcode {
        match self {
            Self::Ipairs => Opcode::IpairsPrep,
            Self::Pairs => Opcode::PairsPrep,
        }
    }

    fn loop_opcode(self) -> Opcode {
        match self {
            Self::Ipairs => Opcode::IpairsLoop,
            Self::Pairs => Opcode::PairsLoop,
        }
    }
}

/// These wrappers preserve a name's value, so writes through it must invalidate
/// the same fast paths. This is separate from when a callee or receiver is read.
fn mutation_source_name(mut expr: &Expr) -> Option<&str> {
    while let Expr::Paren { expr: inner, .. } | Expr::TypeInstantiation { expr: inner, .. } = expr {
        expr = inner;
    }
    match expr {
        Expr::Name(name, _) => Some(name),
        _ => None,
    }
}

fn dotted_import_path(object: &Expr, field: &str) -> Option<Vec<String>> {
    fn collect(expr: &Expr, path: &mut Vec<String>) -> bool {
        match expr {
            Expr::Name(name, _) => {
                path.push(name.clone());
                true
            }
            Expr::Field { object, field, .. } => {
                if !collect(object, path) {
                    return false;
                }
                path.push(field.clone());
                true
            }
            Expr::Paren { expr, .. } => collect(expr, path),
            _ => false,
        }
    }

    let mut path = Vec::with_capacity(3);
    if !collect(object, &mut path) {
        return None;
    }
    path.push(field.to_owned());
    Some(path)
}

fn dotted_import_root(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Name(name, _) => Some(name),
        Expr::Field { object, .. } | Expr::Paren { expr: object, .. } => dotted_import_root(object),
        _ => None,
    }
}

fn const_eval(expr: &Expr) -> Option<Constant> {
    match expr {
        Expr::Nil(_) => Some(Constant::Nil),
        Expr::Bool(value, _) => Some(Constant::Boolean(*value)),
        Expr::Number(value, _) => Some(Constant::Number(*value)),
        Expr::String(value, _) => Some(Constant::String(value.clone())),
        Expr::InterpolatedString { .. } => None,
        Expr::Varargs(_) => None,
        Expr::Name(_, _) => None,
        Expr::Unary {
            op: UnaryOp::Neg,
            expr,
            ..
        } => {
            let Constant::Number(value) = const_eval(expr)? else {
                return None;
            };
            Some(Constant::Number(binary(NumericOp::Neg, value, 0.0)))
        }
        Expr::Unary {
            op: UnaryOp::Not,
            expr,
            ..
        } => Some(Constant::Boolean(!constant_truthy(&const_eval(expr)?))),
        Expr::Unary {
            op: UnaryOp::Len, ..
        } => None,
        Expr::Binary { op, lhs, rhs, .. } => const_eval_binary(*op, lhs, rhs),
        Expr::IfElse { .. } => None,
        Expr::Paren { expr, .. } | Expr::TypeInstantiation { expr, .. } => const_eval(expr),
        Expr::Field { .. }
        | Expr::Index { .. }
        | Expr::Table { .. }
        | Expr::Function { .. }
        | Expr::Call { .. }
        | Expr::MethodCall { .. } => None,
    }
}

fn math_abs_needs_native_error_path(args: &[Expr]) -> bool {
    let Some(first) = args.first() else {
        return true;
    };
    if matches!(first, Expr::Table { .. } | Expr::Function { .. }) {
        return true;
    }
    matches!(
        const_eval(first),
        Some(Constant::Nil | Constant::Boolean(_))
    )
}

fn math_specialized_min_arity(field: &str) -> Option<usize> {
    match field {
        "abs" | "floor" | "ceil" | "round" | "sign" | "sqrt" | "deg" | "rad" | "isnan"
        | "isinf" | "isfinite" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh"
        | "cosh" | "tanh" | "exp" | "log" | "log10" | "min" | "max" => Some(1),
        "fmod" | "pow" | "atan2" | "ldexp" => Some(2),
        "clamp" | "lerp" => Some(3),
        "map" => Some(5),
        _ => None,
    }
}

fn const_eval_binary(op: BinaryOp, lhs: &Expr, rhs: &Expr) -> Option<Constant> {
    let lhs = const_eval(lhs)?;
    let rhs = const_eval(rhs)?;
    fold_binary_constants(op, lhs, rhs)
}

fn fold_binary_constants(op: BinaryOp, lhs: Constant, rhs: Constant) -> Option<Constant> {
    match op {
        BinaryOp::Eq => Some(Constant::Boolean(constant_luau_equals(&lhs, &rhs))),
        BinaryOp::Ne => Some(Constant::Boolean(!constant_luau_equals(&lhs, &rhs))),
        BinaryOp::And => {
            if constant_truthy(&lhs) {
                Some(rhs)
            } else {
                Some(lhs)
            }
        }
        BinaryOp::Or => {
            if constant_truthy(&lhs) {
                Some(lhs)
            } else {
                Some(rhs)
            }
        }
        BinaryOp::Concat => {
            let Constant::String(mut lhs) = lhs else {
                return None;
            };
            let Constant::String(rhs) = rhs else {
                return None;
            };
            lhs.extend_from_slice(&rhs);
            Some(Constant::String(lhs))
        }
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            compare_ordered_constants(op, &lhs, &rhs).map(Constant::Boolean)
        }
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
            let Constant::Number(lhs) = lhs else {
                return None;
            };
            let Constant::Number(rhs) = rhs else {
                return None;
            };
            Some(Constant::Number(binary(
                numeric_op_for_binary(op),
                lhs,
                rhs,
            )))
        }
        BinaryOp::FloorDiv | BinaryOp::Pow => {
            let Constant::Number(lhs) = lhs else {
                return None;
            };
            let Constant::Number(rhs) = rhs else {
                return None;
            };
            Some(Constant::Number(binary(
                numeric_op_for_binary(op),
                lhs,
                rhs,
            )))
        }
    }
}

fn opcode_for_binary(op: BinaryOp) -> Opcode {
    match op {
        BinaryOp::Eq => Opcode::Equal,
        BinaryOp::Ne => Opcode::NotEqual,
        BinaryOp::And | BinaryOp::Or => unreachable!("logical ops are lowered with jumps"),
        BinaryOp::Lt => Opcode::Less,
        BinaryOp::Le => Opcode::LessEqual,
        BinaryOp::Gt => Opcode::Greater,
        BinaryOp::Ge => Opcode::GreaterEqual,
        BinaryOp::Concat => Opcode::Concat,
        BinaryOp::Add => Opcode::Add,
        BinaryOp::Sub => Opcode::Sub,
        BinaryOp::Mul => Opcode::Mul,
        BinaryOp::Div => Opcode::Div,
        BinaryOp::FloorDiv => Opcode::FloorDiv,
        BinaryOp::Mod => Opcode::Mod,
        BinaryOp::Pow => Opcode::Pow,
    }
}

fn opcode_for_assignment(op: AssignmentOp) -> Opcode {
    match op {
        AssignmentOp::Assign => unreachable!("plain assignment is not a binary opcode"),
        AssignmentOp::Add => Opcode::Add,
        AssignmentOp::Sub => Opcode::Sub,
        AssignmentOp::Mul => Opcode::Mul,
        AssignmentOp::Div => Opcode::Div,
        AssignmentOp::FloorDiv => Opcode::FloorDiv,
        AssignmentOp::Mod => Opcode::Mod,
        AssignmentOp::Pow => Opcode::Pow,
        AssignmentOp::Concat => Opcode::Concat,
    }
}

fn numeric_op_for_binary(op: BinaryOp) -> NumericOp {
    match op {
        BinaryOp::Add => NumericOp::Add,
        BinaryOp::Sub => NumericOp::Sub,
        BinaryOp::Mul => NumericOp::Mul,
        BinaryOp::Div => NumericOp::Div,
        BinaryOp::FloorDiv => NumericOp::FloorDiv,
        BinaryOp::Mod => NumericOp::Mod,
        BinaryOp::Pow => NumericOp::Pow,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            unreachable!("comparison is not a numeric op")
        }
        BinaryOp::Concat => unreachable!("concat is not a numeric op"),
        BinaryOp::And | BinaryOp::Or => unreachable!("logical op is not a numeric op"),
    }
}

fn constants_equal(lhs: &Constant, rhs: &Constant) -> bool {
    match (lhs, rhs) {
        (Constant::Nil, Constant::Nil) => true,
        (Constant::Boolean(lhs), Constant::Boolean(rhs)) => lhs == rhs,
        (Constant::Number(lhs), Constant::Number(rhs)) => lhs.to_bits() == rhs.to_bits(),
        (Constant::String(lhs), Constant::String(rhs)) => lhs == rhs,
        (Constant::Import(lhs), Constant::Import(rhs)) => lhs == rhs,
        (Constant::CallDescriptor(lhs), Constant::CallDescriptor(rhs)) => {
            lhs.aux == rhs.aux
                && lhs.append_varargs == rhs.append_varargs
                && lhs.calls.len() == rhs.calls.len()
                && lhs.calls.iter().zip(&rhs.calls).all(|(lhs, rhs)| {
                    lhs.fixed_args == rhs.fixed_args
                        && lhs.deferred_import == rhs.deferred_import
                        && lhs.deferred_method == rhs.deferred_method
                        && lhs.method_receiver_register == rhs.method_receiver_register
                })
        }
        _ => false,
    }
}

fn constant_luau_equals(lhs: &Constant, rhs: &Constant) -> bool {
    match (lhs, rhs) {
        (Constant::Nil, Constant::Nil) => true,
        (Constant::Boolean(lhs), Constant::Boolean(rhs)) => lhs == rhs,
        (Constant::Number(lhs), Constant::Number(rhs)) => lhs == rhs,
        (Constant::String(lhs), Constant::String(rhs)) => lhs == rhs,
        _ => false,
    }
}

fn constant_truthy(value: &Constant) -> bool {
    !matches!(value, Constant::Nil | Constant::Boolean(false))
}

fn compare_ordered_constants(op: BinaryOp, lhs: &Constant, rhs: &Constant) -> Option<bool> {
    let ordering = match (lhs, rhs) {
        (Constant::Number(lhs), Constant::Number(rhs)) => lhs.partial_cmp(rhs),
        (Constant::String(lhs), Constant::String(rhs)) => Some(lhs.cmp(rhs)),
        _ => return None,
    };
    Some(match op {
        BinaryOp::Lt => ordering.is_some_and(std::cmp::Ordering::is_lt),
        BinaryOp::Le => ordering.is_some_and(|ordering| !ordering.is_gt()),
        BinaryOp::Gt => ordering.is_some_and(std::cmp::Ordering::is_gt),
        BinaryOp::Ge => ordering.is_some_and(|ordering| !ordering.is_lt()),
        _ => unreachable!("expected numeric comparison op"),
    })
}

#[cfg(test)]
mod tests {
    use super::{compile, compile_with_options, BuiltinEnvironment, CompileOptions};
    use rustia_parser::{parse_source, parse_source_bytes};
    use rustia_vm::bytecode::{Constant, Instruction, Opcode, Program, UpvalueDescriptor};
    use rustia_vm::{ErrorKind, HostFunctionOp, OpCapability, Profile, Vm, VmSettings};
    use std::sync::Arc;

    #[test]
    fn compiles_and_runs_typed_numeric_subset() {
        let chunk =
            parse_source("type Alias<T> = T | nil\nlocal x: number = 40 + 2; return x").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::DeterministicSim);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(42.0));
    }

    #[test]
    fn numeric_and_generic_for_bindings_erase_their_annotations() {
        let source = "local n=0; for i:typeof(unknown())=1,2 do n+=i end; \
            for k:number,v:number in ipairs({4}) do n+=v end; return n";
        let program = compile(&parse_source(source).unwrap()).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&program).unwrap().as_number(), Some(7.0));
    }

    #[test]
    fn type_function_erasure_emits_no_body_constants_or_prototypes() {
        let ordinary = compile(&parse_source("return 42").unwrap()).unwrap();
        let declared = compile(&parse_source(
            "export type function f() local function g() error('unused') end return g end; return 42",
        ).unwrap()).unwrap();
        assert_eq!(declared.main.code, ordinary.main.code);
        assert_eq!(declared.constants().len(), ordinary.constants().len());
        assert!(declared.functions.is_empty());
    }

    #[test]
    fn guest_call_root_snapshot_preserves_newly_weak_outer_values() {
        let chunk = parse_source(
            r#"
                local holder = {}
                local outerOnly = {}
                holder[1] = outerOnly
                local function activateWeakValues()
                    setmetatable(holder, { __mode = "v" })
                    collectgarbage()
                end
                activateWeakValues()
                return holder[1] == outerOnly
            "#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn constant_folded_number_equality_uses_luau_semantics() {
        let cases = [
            ("return -0 == 0", true),
            ("return 0 ~= -0", false),
            ("return 0 / 0 == 0 / 0", false),
            ("return 0 / 0 ~= 0 / 0", true),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            assert!(program
                .main
                .code
                .iter()
                .all(|instruction| !matches!(instruction.op, Opcode::Equal | Opcode::NotEqual)));
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_bool(), Some(expected));
        }
    }

    #[test]
    fn constant_pool_keeps_signed_zero_numbers_distinct() {
        let chunk = parse_source("return tostring(-0) .. '|' .. tostring(0)").unwrap();
        let program = compile(&chunk).unwrap();
        let zero_bits = program
            .constants()
            .iter()
            .filter_map(|constant| match constant {
                Constant::Number(value) if *value == 0.0 => Some(value.to_bits()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(zero_bits.contains(&(-0.0f64).to_bits()));
        assert!(zero_bits.contains(&0.0f64.to_bits()));

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("-0|0"));
    }

    #[test]
    fn compiles_multi_name_locals_with_luau_nil_fill_and_extra_discard() {
        let cases = [
            ("local a, b, c = 1, 2, 3 return c", Some(3.0), None),
            (
                "local a, b, c = 1, 2 return c",
                None,
                Some(rustia_vm::ValueKind::Nil),
            ),
            ("local a = 1, 2 return a", Some(1.0), None),
        ];

        for (source, expected_number, expected_kind) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            if let Some(expected) = expected_number {
                assert_eq!(result.as_number(), Some(expected));
            }
            if let Some(expected) = expected_kind {
                assert_eq!(result.kind(), expected);
            }
        }
    }

    #[test]
    fn compiles_local_assignment_after_rhs_evaluation() {
        let cases = [
            ("local a = 1 a = 2 return a", 2.0),
            ("local a = 1 local b = 2 a, b = b, a return a", 2.0),
            ("local a = 1 local b = 2 a, b = b, a return b", 1.0),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }
    }

    #[test]
    fn compiles_compound_assignment_for_locals_and_tables() {
        let cases = [
            ("local a = 1 a += 2 return a", 3.0),
            ("local a = 5 a -= 2 return a", 3.0),
            ("local a = 2 a *= 4 return a", 8.0),
            ("local a = 9 a /= 3 return a", 3.0),
            ("local a = 5 a %= 2 return a", 1.0),
            ("local a = 2 a ^= 3 return a", 8.0),
            (
                "a = 1 function set_a() a = 10 return 2 end a += set_a() return a",
                3.0,
            ),
            ("local t = { value = 3 } t.value *= 3 return t.value", 9.0),
            ("local t = { 4 } local b = -4 t[1] /= b return t[1]", -1.0),
            (
                "t = { value = 1 } function set_t() t.value = 10 return 2 end t.value += set_t() return t.value",
                3.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let chunk = parse_source("local a = 'a' a ..= 'b' return a").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("ab"));

        let chunk =
            parse_source("local t = { value = 'a' } t.value ..= 'b' return t.value").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("ab"));
    }

    #[test]
    fn compiles_floor_division_and_power_with_luau_precedence() {
        let cases = [
            ("local a = 1 a = a // 2 return a", 0.0),
            ("local a = -3 a = a // 2 return a", -2.0),
            ("local a = 3 a = a ^ 2 return a", 9.0),
            ("return 2 ^ 3 ^ 2", 512.0),
            ("return -2 ^ 2", -4.0),
            ("return 2 ^ -2", 0.25),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }
    }

    #[test]
    fn compiles_string_concat_with_luau_associativity() {
        let cases = [
            ("local a = '1' a = a .. '2' return a", "12"),
            ("local a = '1' a = a .. '2' .. '3' return a", "123"),
            ("local a = 'n=' a = a .. 3 return a", "n=3"),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }
    }

    #[test]
    fn compiles_string_relational_comparisons() {
        let cases = [
            ("return 'a' < 'b'", true),
            ("return 'a' <= 'a'", true),
            ("return 'a' > ''", true),
            ("return 'abc' >= 'abd'", false),
            ("local a = 'abc' local b = 'abd' return a < b", true),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_bool(), Some(expected));
        }
    }

    #[test]
    fn compiles_rawequal_builtin() {
        let cases = [
            ("return rawequal(true, 5)", false),
            ("return rawequal(nil, nil)", true),
            ("return rawequal(true, false)", false),
            ("return rawequal(true, true)", true),
            ("return rawequal(0, -0)", true),
            ("return rawequal(1, 2)", false),
            ("return rawequal('a', 'a')", true),
            ("return rawequal('a', 'b')", false),
            ("local a = {} return rawequal(a, a)", true),
            ("local a = {} local b = {} return rawequal(a, b)", false),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_bool(), Some(expected));
        }
    }

    #[test]
    fn compiles_raw_table_builtins() {
        let number_cases = [
            ("local t = {6, 9, 7} return rawget(t, 2)", 9.0),
            ("local t = {} rawset(t, 'x', 7, 8, 9) return t.x", 7.0),
            ("local t = {1, 2, nil} return rawlen(t)", 2.0),
            (
                "local t = {} rawset(t, 1, nil) rawset(t, 2, 'x') return rawlen(t)",
                0.0,
            ),
            ("return rawlen('foo')", 3.0),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let nil_cases = [
            "local t = {} return rawget(t, nil)",
            "local t = {} return rawget(t, 0/0)",
            "local t = {} return t[0/0]",
        ];

        for source in nil_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.kind(), rustia_vm::ValueKind::Nil);
        }
    }

    #[test]
    fn table_writes_report_exact_nil_and_nan_key_errors() {
        let cases = [
            ("local t = {} t[nil] = 2", "table index is nil"),
            ("local t = {} t[0/0] = 2", "table index is NaN"),
            ("local t = {} rawset(t, nil, 2)", "table index is nil"),
            ("local t = {} rawset(t, 0/0, 2)", "table index is NaN"),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let error = vm.run(&program).unwrap_err();
            assert!(error.to_string().contains(expected));
        }
    }

    #[test]
    fn compiles_type_and_typeof_builtins() {
        let cases = [
            ("return type(nil)", "nil"),
            ("return type(true)", "boolean"),
            ("return type(1 < 2)", "boolean"),
            ("return type(-3)", "number"),
            ("return type('x')", "string"),
            ("return type({})", "table"),
            ("return typeof(5)", "number"),
            ("return typeof(nil)", "nil"),
            ("return typeof({})", "table"),
            (
                "local function f(a) return type(math.modf(a)) end return f(2)",
                "number",
            ),
            (
                "local function f(a) return typeof(math.modf(a)) end return f('2')",
                "number",
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }

        for source in [
            "local function nothing() end return pcall(function() return tostring(nothing()) end)",
            "local function nothing() end local ok = pcall(function() return tostring(nothing()) end) return ok",
        ] {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_bool(), Some(false));
        }
    }

    #[test]
    fn compiles_array_literals_and_ipairs_generic_for() {
        let cases = [
            (
                "local a = '' for k in ipairs({5, 6, 7}) do a = a .. k end return a",
                "123",
            ),
            (
                "local a = '' for k, v in ipairs({5, 6, 7}) do a = a .. v end return a",
                "567",
            ),
            (
                "local a = '' for k, v in ipairs({5, 6, 7, nil, 8}) do a = a .. k .. v end return a",
                "152637",
            ),
            (
                "local a = '' for k, v in ipairs({5, 6, 7}) do if k == 2 then continue end a = a .. v end return a",
                "57",
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }
    }

    #[test]
    fn compiles_array_pairs_generic_for() {
        let string_cases = [
            (
                "local a = '' for k in pairs({5, 6, 7}) do a = a .. k end return a",
                "123",
            ),
            (
                "local a = '' for k, v in pairs({5, 6, 7}) do a = a .. v end return a",
                "567",
            ),
            (
                "local a = '' for k, v in pairs({5, 6, 7, nil, 8}) do a = a .. k end return a",
                "1235",
            ),
            (
                "local a = '' for k, v in pairs({5, 6, 7, nil, 8}) do a = a .. v end return a",
                "5678",
            ),
            (
                "local a = '' for k, v, p in pairs({a=1, b=2, c=3}) do a = a .. tostring(p) end return a",
                "nilnilnil",
            ),
        ];

        for (source, expected) in string_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }

        let number_cases = [
            (
                "local a = 1 for _ in pairs({1, 2, 3}) do a = a * 2 if a == 4 then break end end return a",
                4.0,
            ),
            (
                "local a = 1 for b in pairs({1, 2, 3}) do a = a * 2 b = nil end return a",
                8.0,
            ),
            (
                "local a = {} for k in pairs({a=1, b=2, c=3}) do a[k] = 1 end return a.a + a.b + a.c",
                3.0,
            ),
            (
                "local a = {} for k, v in pairs({a=1, b=2, c=3}) do a[k] = v end return a.a + a.b + a.c",
                6.0,
            ),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }
    }

    #[test]
    fn compiles_manual_next_generic_for_over_arrays() {
        let cases = [
            (
                "local a = '' for k in next, {5, 6, 7} do a = a .. k end return a",
                "123",
            ),
            (
                "local a = '' for k, v in next, {5, 6, 7} do a = a .. k end return a",
                "123",
            ),
            (
                "local a = '' for k, v in next, {5, 6, 7} do a = a .. v end return a",
                "567",
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }

        let number_cases = [
            (
                "local a = {} for k in next, {a=1, b=2, c=3} do a[k] = 1 end return a.a + a.b + a.c",
                3.0,
            ),
            (
                "local a = {} for k, v in next, {a=1, b=2, c=3} do a[k] = v end return a.a + a.b + a.c",
                6.0,
            ),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }
    }

    #[test]
    fn compiles_array_table_length_and_index_reads() {
        let cases = [
            ("local t = {} return #t", 0.0),
            ("local t = {nil} return #t", 0.0),
            ("local t = {1, 2} return #t", 2.0),
            ("local t = {1, 2} return t[1] + t[2]", 3.0),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }
    }

    fn contains_register_block_shape(code: &[Instruction], operation: Opcode) -> bool {
        code.windows(4).any(|instructions| {
            let [lhs_move, rhs_move, binary, clear] = instructions else {
                return false;
            };
            lhs_move.op == Opcode::Move
                && lhs_move.c == 0
                && lhs_move.aux == 0
                && rhs_move.op == Opcode::Move
                && rhs_move.c == 0
                && rhs_move.aux == 0
                && binary.op == operation
                && binary.a == lhs_move.a
                && binary.b == lhs_move.a
                && binary.c == rhs_move.a
                && binary.aux == 0
                && clear.op == Opcode::ClearLocalCell
                && clear.a == rhs_move.a
                && clear.b == 0
                && clear.c == 0
                && clear.aux == 0
        })
    }

    fn contains_register_lifetime_block_shape(code: &[Instruction], operation: Opcode) -> bool {
        code.windows(6).any(|instructions| {
            let [lhs_move, rhs_move, binary, temporary_clear, destination_move, target_clear] =
                instructions
            else {
                return false;
            };
            lhs_move.op == Opcode::Move
                && lhs_move.c == 0
                && lhs_move.aux == 0
                && rhs_move.op == Opcode::Move
                && rhs_move.c == 0
                && rhs_move.aux == 0
                && binary.op == operation
                && binary.a == lhs_move.a
                && binary.b == lhs_move.a
                && binary.c == rhs_move.a
                && binary.aux == 0
                && temporary_clear.op == Opcode::ClearLocalCell
                && temporary_clear.a == rhs_move.a
                && temporary_clear.b == 0
                && temporary_clear.c == 0
                && temporary_clear.aux == 0
                && destination_move.op == Opcode::Move
                && destination_move.b == lhs_move.a
                && destination_move.c == 0
                && destination_move.aux == 0
                && destination_move.a != lhs_move.a
                && destination_move.a != rhs_move.a
                && target_clear.op == Opcode::ClearLocalCell
                && target_clear.a == lhs_move.a
                && target_clear.b == 0
                && target_clear.c == 0
                && target_clear.aux == 0
        })
    }

    #[test]
    fn compiler_register_binary_shapes_remain_available_to_vm_plans() {
        for (source, operation, expected) in [
            ("local a, b = 1, 2 return a + b", Opcode::Add, 3.0),
            (
                "local t = {7} local i = 1 return t[i]",
                Opcode::GetIndex,
                7.0,
            ),
        ] {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            assert!(
                contains_register_block_shape(&program.main.code, operation),
                "compiler stopped emitting the VM's verified register-block shape for {operation:?}"
            );

            let mut vm = Vm::new(Profile::ReproducibleStandard);
            assert_eq!(vm.run(&program).unwrap().as_number(), Some(expected));
        }
    }

    #[test]
    fn compiler_register_lifetime_shapes_remain_available_to_vm_plans() {
        for (source, operation, expected) in [
            (
                "local a, b, result = 1, 2, 0 result = a + b return result",
                Opcode::Add,
                3.0,
            ),
            (
                "local t = {7} local i, result = 1, 0 result = t[i] return result",
                Opcode::GetIndex,
                7.0,
            ),
        ] {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            assert!(
                contains_register_lifetime_block_shape(&program.main.code, operation),
                "compiler stopped emitting the VM's verified register-lifetime shape for {operation:?}"
            );

            let mut vm = Vm::new(Profile::ReproducibleStandard);
            assert_eq!(vm.run(&program).unwrap().as_number(), Some(expected));
        }
    }

    #[test]
    fn compiles_named_and_computed_table_fields() {
        let cases = [
            ("local t = {data = 4} return t.data", 4.0),
            ("local t = {[1 + 2] = 4} return t[3]", 4.0),
            ("local t = {[1] = 1, [2] = 2} return t[1] + t[2]", 3.0),
            (
                "local t = {data = 4, [1 + 2] = 5} return t.data + t[3]",
                9.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }
    }

    #[test]
    fn compiles_puc51_constructor_write_order() {
        let fifty_positionals_then_keyed = format!(
            "local t = {{{}, [1] = 99}} return t[1]",
            (1..=50)
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let keyed_between_batches = format!(
            "local t = {{{}, [1] = 99, {}}} return t[1]",
            (1..=25)
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            (26..=50)
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let cases = [
            // Keyed writes land immediately; positional flushes overwrite them.
            ("local t = {1, 2, 3, [1] = 9, 4} return t[1]", 1.0),
            ("local t = {1, [1] = 9} return t[1]", 1.0),
            ("local t = {[1] = 9, 1} return t[1]", 1.0),
            ("local t = {1, x = 5, [1] = 9, 2} return t[1] + t.x", 6.0),
            // Trailing expansions flush after every keyed write.
            (
                "local function f() return 10, 20 end local t = {1, [2] = 9, f()} return t[2]",
                10.0,
            ),
            (
                "local function g(...) local t = {1, [2] = 9, ...} return t[2] end return g(10, 20)",
                10.0,
            ),
            // The 50-item flush batch is observable: a keyed write after the
            // 50th positional survives, one inside the batch does not.
            (fifty_positionals_then_keyed.as_str(), 99.0),
            (keyed_between_batches.as_str(), 1.0),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile_with_options(
                &chunk,
                CompileOptions {
                    puc51_constructor_order: true,
                    ..CompileOptions::default()
                },
            )
            .unwrap();
            let mut vm = Vm::new(Profile::Puc51Compat);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "source: {source}");
        }
    }

    #[test]
    fn compiles_puc51_constructor_evaluation_order_stays_source_order() {
        let chunk = parse_source(
            "local log = {} \
             local function v(x) log[#log + 1] = x return x end \
             local t = {v(1), [1] = 9, v(2)} \
             return #log * 100 + t[1]",
        )
        .unwrap();
        let program = compile_with_options(
            &chunk,
            CompileOptions {
                puc51_constructor_order: true,
                ..CompileOptions::default()
            },
        )
        .unwrap();
        let mut vm = Vm::new(Profile::Puc51Compat);
        let result = vm.run(&program).unwrap();
        // Both values were evaluated in source order (2 logged) even though the
        // positional write won the slot.
        assert_eq!(result.as_number(), Some(201.0));
    }

    #[test]
    fn compiles_table_index_assignment() {
        let cases = [
            ("local t = {6, 9, 7} t[2] = 10 return t[2]", 10.0),
            ("local t = {6, 9, 7} t[0] = 5 return t[0]", 5.0),
            ("local t = {} t[-0] = 5 return t[0]", 5.0),
            ("local t = {6, 9, 7} t[4] = 10 return t[4]", 10.0),
            ("local t = {6, 9, 7} t[4.5] = 10 return t[4.5]", 10.0),
            ("local t = {6, 9, 7} t['a'] = 11 return t['a']", 11.0),
            ("local t = {} t.a = 12 return t.a", 12.0),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }
    }

    #[test]
    fn compiles_wide_constant_loads() {
        let mut source = String::new();
        for index in 0..300 {
            source.push_str(&format!("v = '{index}'\n"));
        }
        source.push_str("return v");

        let chunk = parse_source(&source).unwrap();
        let program = compile(&chunk).unwrap();
        assert!(program
            .main
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::LoadConstWide));
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("299"));
    }

    #[test]
    fn compiles_surrogate_unicode_escape_bytes() {
        let chunk =
            parse_source(r#"return "\u{D800}\u{DFFF}" == "\237\160\128\237\191\191""#).unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn global_function_declaration_disables_conformance_helper_fast_path_in_body() {
        let chunk = parse_source(
            r#"
function concat(head, ...)
    if select('#', ...) == 0 then
        return tostring(head)
    else
        return tostring(head) .. "," .. concat(...)
    end
end

return concat(pcall(function () return 1,2,3 end))
"#,
        )
        .unwrap();
        let program = compile_with_options(
            &chunk,
            CompileOptions {
                builtins: BuiltinEnvironment::LuauConformance,
                ..CompileOptions::default()
            },
        )
        .unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(
            vm.string_value(result).unwrap().as_deref(),
            Some("true,1,2,3")
        );
    }

    #[test]
    fn compiles_luau_global_environment_assignments() {
        let cases = [
            ("a = 1 a = 2 return a", 2.0),
            ("a = 1 return _G.a", 1.0),
            ("_G.foo = 1 return _G['foo']", 1.0),
            ("_G['bar'] = 1 return _G.bar", 1.0),
            ("a = 1 local a = 2 return _G.a", 1.0),
            ("function f() return 42 end return f()", 42.0),
            ("function print() return 42 end return print()", 42.0),
            (
                "math = { abs = function(x) return 99 end } return math.abs(-1)",
                99.0,
            ),
            (
                "_G.table = { getn = function(t) return 99 end } return table.getn({1})",
                99.0,
            ),
            (
                "string = { len = function(x) return 99 end } return string.len('abc')",
                99.0,
            ),
            (
                "math.abs = function(x) return 99 end return math.abs(-1)",
                99.0,
            ),
            (
                "function math.abs(x) return x + 1 end return math.abs(41)",
                42.0,
            ),
            ("math.huge = 7 return math.huge", 7.0),
            (
                "math.frexp = function(x) return 42 end return math.frexp(1)",
                42.0,
            ),
            (
                "table.getn = function(t) return 99 end return table.getn({1})",
                99.0,
            ),
            (
                "string.len = function(x) return 99 end return string.len('abc')",
                99.0,
            ),
            (
                "local seen = false for k in pairs(_G) do if k == 'bit32' then seen = true end end if seen then return -1 end return bit32.extract(2, 1)",
                1.0,
            ),
            (
                "local a = {} local preserve = { os = 1, string = 1, debug = 1, io = 1, coroutine = 1, table = 1, math = 1 } for n, v in pairs(_G) do a[n] = v end for n, v in pairs(a) do if not preserve[n] and type(v) ~= 'function' and not string.find(n, '^[%u_]') then _G[n] = nil end end return bit32.extract(2, 1)",
                1.0,
            ),
            (
                "local seen = false for k in pairs(_G) do if k == 'vector' then seen = true end end if seen then return -1 end return vector.dot(vector.one, vector.one)",
                3.0,
            ),
            (
                "local mt = getmetatable(_G) local ok_index = pcall(function() mt.__index.vector = nil end) local ok_meta = pcall(function() setmetatable(_G, {}) end) if ok_index or ok_meta or type(vector.create) ~= 'function' then return -1 end return 1",
                1.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let nil_cases = ["return missing_global", "local _G = {} _G.x = 1 return x"];
        for source in nil_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.kind(), rustia_vm::ValueKind::Nil, "{source}");
        }

        let chunk = parse_source("assert = nil assert(false)").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert!(matches!(
            error.kind,
            ErrorKind::RuntimeError { ref message } if message == "attempt to call a nil value"
        ));

        let chunk = parse_source("rawset(_G, 'assert', nil) assert(false)").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert!(matches!(
            error.kind,
            ErrorKind::RuntimeError { ref message } if message == "attempt to call a nil value"
        ));

        let cases = [
            (
                "function f() return assert(false) end assert = function() return 7 end return f()",
                7.0,
            ),
            (
                "function f() return math.abs(-2) end math.abs = function() return 99 end return f()",
                99.0,
            ),
        ];
        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }
    }

    #[test]
    fn compiles_luau_vector_core_and_library_surface() {
        let chunk = parse_source(
            r#"
local v = vector.create(1, 2, 3)
local t = {}
t[vector.create(0, 0, 0)] = 7
local ok_nan_key = pcall(function()
    t[vector.create(0 / 0, 2, 3)] = 1
end)
local ok_dot, dot_error = pcall(function()
    return v:Dot("x")
end)
local canonical_key = next({ [vector.create(-0, 0, 0)] = true })

return type(v) == "vector"
    and tostring(v) == "1, 2, 3"
    and vector.create(1, 2, 3) == vector.create(1, 2, 3)
    and vector.create("1", "2", "3") == v
    and vector.create("1", "2") == vector.create(1, 2, 0)
    and vector("1", "2", "3") == v
    and vector("1", "2") == vector.create(1, 2, 0)
    and rawequal(vector.create(0, 1, 2), vector.create(-0, 1, 2))
    and 1 / canonical_key.x == math.huge
    and t[vector.create(-0, 0, 0)] == 7
    and vector.create(1, 2, 4) * "8" == vector.create(8, 16, 32)
    and vector.create(1, 0, 0):Cross(vector.create(0, 1, 0)) == vector.create(0, 0, 1)
    and vector.dot(vector.one, vector.create(2, 3, 4)) == 9
    and vector.lerp(vector.zero, vector.one, 0.5) == vector.create(0.5, 0.5, 0.5)
    and not ok_nan_key
    and not ok_dot
    and dot_error:find("invalid argument #2", 1, true) ~= nil
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn unknown_stdlib_field_calls_compile_and_fail_at_runtime() {
        for source in [
            "return math.unreal(1)",
            "return table.forever({})",
            "return string.unreal('abc')",
        ] {
            let program = compile(&parse_source(source).unwrap()).unwrap();
            let error = Vm::new(Profile::ReproducibleStandard)
                .run(&program)
                .expect_err("calling a missing field is a runtime error");
            assert!(
                matches!(error.kind, ErrorKind::RuntimeError { .. }),
                "{source}: {error}"
            );
        }
    }

    #[test]
    fn compiles_first_class_standard_library_globals() {
        let number_cases = [
            ("local abs = math.abs return abs(-5)", 5.0),
            (
                "function f() local abs = math.abs return abs(-5) end return f()",
                5.0,
            ),
            ("local len = string.len return len('abc')", 3.0),
            ("local print = print print('ignored') return 1", 1.0),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm
                .run(&program)
                .unwrap_or_else(|error| panic!("{source}: {error}"));
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }

        let string_cases = [
            (
                "local concat = table.concat return concat({1, 2, 3}, ',')",
                "1,2,3",
            ),
            ("return type(rawget(_G, 'math'))", "table"),
            ("return type(rawget(_G, 'table').concat)", "function"),
            ("return type(rawget(_G, 'string').len)", "function"),
            ("return type(rawget(_G, 'string').format)", "function"),
            ("return type(rawget(_G, 'type'))", "function"),
            ("return type(rawget(_G, 'bit32'))", "nil"),
            ("return type(bit32.extract)", "function"),
            ("return type(rawget(_G, 'vector'))", "nil"),
            ("return type(vector.create)", "function"),
            ("return type(rawget(_G, 'RTTI'))", "nil"),
            ("return type(RTTI)", "table"),
            ("return type(print)", "function"),
            ("return type(xpcall)", "function"),
        ];

        for (source, expected) in string_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(
                vm.string_value(result).unwrap().as_deref(),
                Some(expected),
                "{source}"
            );
        }
    }

    #[test]
    fn compiles_string_format_builtin_subset() {
        let cases = [
            (r#"return string.format("%s = %s", "a", 5)"#, "a = 5"),
            (r#"local fmt = string.format return fmt("%%%d", 10)"#, "%10"),
            (r#"return string.format("%010d", 23)"#, "0000000023"),
            (r#"return string.format("%-5s!", "a")"#, "a    !"),
            (r#"return string.format("%.3s", "abcdef")"#, "abc"),
            (
                r#"return string.format("%o %u %x %X", -1, -1, -1, -1)"#,
                "1777777777777777777777 18446744073709551615 ffffffffffffffff FFFFFFFFFFFFFFFF",
            ),
            (r#"return string.format("%g, %g, %g", 5, 7, 9)"#, "5, 7, 9"),
            (r#"return string.format("%.3g", 12345)"#, "1.23e+04"),
            (r#"return string.format("%.3g", 0.000123456)"#, "0.000123"),
            (r#"return string.format("%.6g", 1000000)"#, "1e+06"),
            (r#"return string.format("%G", 12345)"#, "12345"),
            (r#"return string.format("%+10.3g", 12345)"#, " +1.23e+04"),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm
                .run(&program)
                .unwrap_or_else(|error| panic!("{source}: {error}"));
            assert_eq!(
                vm.string_value(result).unwrap().as_deref(),
                Some(expected),
                "{source}"
            );
        }
    }

    #[test]
    fn compiles_table_getn_and_maxn_builtins() {
        let cases = [
            ("return table.getn({1, 2, 3, nil, nil})", 3.0),
            (
                "local a = {} for j = 1, 40 do a[j] = j end return table.getn(a)",
                40.0,
            ),
            ("return table.maxn({})", 0.0),
            ("return table.maxn({[-100] = 1})", 0.0),
            ("return table.maxn({['1000'] = true})", 0.0),
            ("return table.maxn({['1000'] = true, [24.5] = 3})", 24.5),
            ("return table.maxn({[1000] = true})", 1000.0),
            (
                "local a = {[10] = 1, [20] = 2} a[20] = nil return table.maxn(a)",
                10.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }
    }

    #[test]
    fn compiles_table_find_builtin() {
        let number_cases = [
            (
                "return table.find({'nice', 'nice', 'nice', 'nice', 'nice'}, 'nice')",
                1.0,
            ),
            (
                "return table.find({'nice', 'nice', 'nice', 'nice', 'nice'}, 'nice', 5)",
                5.0,
            ),
            ("return table.find({false, true}, true)", 2.0),
            ("return table.find({[(2)] = true}, true, 2)", 2.0),
            ("return table.find({'nice'}, 'nice', nil)", 1.0),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let nil_cases = [
            "return table.find({'nice', 'nice', 'nice', 'nice', 'nice'}, 'nice', 6)",
            "return table.find({1, nil, 3}, 3)",
            "return table.find({[(2)] = true}, true)",
            "return table.find({[(2)] = true, [(4)] = true}, true, 3)",
        ];

        for source in nil_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.kind(), rustia_vm::ValueKind::Nil);
        }

        let chunk = parse_source("return table.find({}, 42, 0)").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert!(error.to_string().contains("index out of range"));
    }

    #[test]
    fn compiles_next_builtin_for_single_result_contexts() {
        let nil_cases = [
            "return next({})",
            "return next({}, nil)",
            "local t = {} t[1000] = 1000 return next(t, 1000)",
            "local t = { gl = 1 } return next(t, 'gl')",
        ];

        for source in nil_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.kind(), rustia_vm::ValueKind::Nil);
        }

        let numeric_cases = [("local t = {} t[1000] = 1000 return next(t, nil)", 1000.0)];

        for (source, expected) in numeric_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let chunk = parse_source("local t = { gl = 1 } return next(t)").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("gl"));

        let chunk = parse_source("local t = { gl = 1 } return next(t, 'missing')").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert!(error.to_string().contains("invalid key to 'next'"));
    }

    #[test]
    fn compiles_table_create_builtin() {
        let number_cases = [
            ("return #table.create(5)", 0.0),
            ("local t = table.create(5) t[5] = 5 return #t", 5.0),
            ("local t = table.create(10) t[4] = 1 return #t", 0.0),
            ("return #table.create(0)", 0.0),
            ("return #table.create(3, nil)", 0.0),
            ("return #table.create(-0.5)", 0.0),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let chunk = parse_source("return table.concat(table.create(5, 'nice'), '!')").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(
            vm.string_value(result).unwrap().as_deref(),
            Some("nice!nice!nice!nice!nice")
        );

        let chunk = parse_source("return table.concat(table.create(2.9, 'x'), ',')").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("x,x"));

        let chunk = parse_source("return table.concat(table.create('2', 'x'), ',')").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("x,x"));

        let chunk = parse_source("return table.create(-1)").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert!(error.to_string().contains("size out of range"));

        let chunk = parse_source("return table.create(1e9)").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert!(error.to_string().contains("table overflow"));
    }

    #[test]
    fn compiles_table_insert_and_remove_builtins() {
        let number_cases = [
            (
                "local t = table.create(10) table.insert(t, 1) return #t",
                1.0,
            ),
            (
                "local t = table.create(10) table.insert(t, 1) table.insert(t, 1) table.insert(t, 1) table.insert(t, 1) return #t",
                4.0,
            ),
            (
                "local t = table.create(10, 1) table.remove(t) return #t",
                9.0,
            ),
            (
                "local t = table.create(10, 1) table.remove(t) table.remove(t) table.remove(t) return #t",
                7.0,
            ),
            (
                "local a = {} table.insert(a, 10) table.insert(a, 2, 20) table.insert(a, 1, -1) table.insert(a, 40) table.insert(a, table.getn(a) + 1, 50) table.insert(a, 2, -2) return table.remove(a, 1)",
                -1.0,
            ),
            ("local t = {1, 2, 3} return table.remove(t, nil)", 3.0),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let chunk = parse_source("local a = {} table.insert(a, 10) table.insert(a, 2, 20) table.insert(a, 1, -1) table.insert(a, 40) table.insert(a, table.getn(a) + 1, 50) table.insert(a, 2, -2) return table.concat(a, ',')").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(
            vm.string_value(result).unwrap().as_deref(),
            Some("-1,-2,10,20,40,50")
        );

        let chunk = parse_source("local t = {} t[3] = 'c' t[1] = 'a' t[2] = 'b' table.insert(t, 'x') return table.concat(t, ',')").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("a,b,c,x"));

        let chunk =
            parse_source("local t = {} t[3] = 'c' t[1] = 'a' t[2] = 'b' return table.remove(t)")
                .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("c"));

        let nil_cases = ["return table.remove({})", "return table.remove({1}, 2)"];

        for source in nil_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.kind(), rustia_vm::ValueKind::Nil);
        }

        let chunk = parse_source(
            r#"a,b = string.find("123456789", "345") return string.sub("123456789", a, b)"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("345"));
    }

    #[test]
    fn compiles_table_move_builtin() {
        let string_cases = [
            (
                "local a = table.move({10, 20, 30}, 1, 3, 2) return table.concat(a, ',')",
                "10,10,20,30",
            ),
            (
                "local a = table.move({10, 20, 30}, 2, 3, 1) return table.concat(a, ',')",
                "20,30,30",
            ),
            (
                "local a = {} local r = table.move({10, 20, 30}, 1, 3, 1, a) if r == a then return table.concat(a, ',') end return 'bad'",
                "10,20,30",
            ),
        ];

        for (source, expected) in string_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }

        let number_cases = [
            (
                "local a = {} table.move({10, 20, 30}, 1, 0, 3, a) return table.getn(a)",
                0.0,
            ),
            (
                "local a = {1, 2, 3, 4, 5} table.move({10}, 1, 3, 2, a) if a[1] == 1 and a[2] == 10 and a[3] == nil and a[4] == nil and a[5] == 5 then return 1 end return -1",
                1.0,
            ),
            (
                "local ok, err = pcall(table.move, {}, 0, 2147483647, 1) if not ok and err:find('too many') then return 1 end return -1",
                1.0,
            ),
            (
                "local ok, err = pcall(table.move, {}, 1, 2, 2147483647) if not ok and err:find('wrap around') then return 1 end return -1",
                1.0,
            ),
            (
                "local ok, err = pcall(table.move, 1, 2, 3, 4) if not ok and err:find('table expected') then return 1 end return -1",
                1.0,
            ),
            (
                "local ok, err = pcall(table.move, {}, 2, 3, 4, 'foo') if not ok and err:find('table expected') then return 1 end return -1",
                1.0,
            ),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }
    }

    #[test]
    fn compiles_table_freeze_and_isfrozen_builtins() {
        let cases = [
            (
                "local t = {} if table.isfrozen(t) then return -1 end t[1] = 1 t.a = 2 if table.freeze(t) ~= t then return -2 end if not table.isfrozen(t) then return -3 end return t[1] + t.a",
                3.0,
            ),
            (
                "local t = {child = {}} local child = t.child table.freeze(t) child.x = 5 if table.isfrozen(child) then return -1 end return child.x",
                5.0,
            ),
            (
                "local ok, err = pcall(function() local t = table.freeze({}) t.x = 1 end) if not ok and err:find('attempt to modify a readonly table') then return 1 end return -1",
                1.0,
            ),
            (
                "local t = table.freeze({1}) local ok, err = pcall(rawset, t, 1, 2) if not ok and err:find('attempt to modify a readonly table') then return 1 end return -1",
                1.0,
            ),
            (
                "local t = table.freeze({}) local ok, err = pcall(table.freeze, t) if not ok and err == \"invalid argument #1 to 'freeze' (table is already frozen)\" then return 1 end return -1",
                1.0,
            ),
            (
                "local ok, err = pcall(table.insert, table.freeze({1, 2, 3}), 4) if not ok and err:find('attempt to modify a readonly table') then return 1 end return -1",
                1.0,
            ),
            (
                "local ok, err = pcall(table.remove, table.freeze({1})) if not ok and err:find('attempt to modify a readonly table') then return 1 end return -1",
                1.0,
            ),
            (
                "local ok, err = pcall(table.move, table.freeze({}), 1, 1, 1) if not ok and err:find('attempt to modify a readonly table') then return 1 end return -1",
                1.0,
            ),
            (
                "local ok, err = pcall(table.sort, table.freeze({2, 1})) if not ok and err:find('attempt to modify a readonly table') then return 1 end return -1",
                1.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }
    }

    #[test]
    fn compiles_table_clear_builtin() {
        let cases = [
            (
                "local t = {1, 2, 3} t.a = 4 table.clear(t) if next(t) == nil then return #t end return -1",
                0.0,
            ),
            (
                "local t = {} for i = 1, 10 do t[i] = i end table.clear(t) t[10] = true local u = table.create(10) u[10] = true if #t == #u then return #t end return -1",
                10.0,
            ),
            (
                "local ok, err = pcall(table.clear) if not ok and err:find('table expected') then return 1 end return -1",
                1.0,
            ),
            (
                "local ok, err = pcall(table.clear, 1, 2) if not ok and err:find('table expected') then return 1 end return -1",
                1.0,
            ),
            (
                "local ok, err = pcall(table.clear, table.freeze({})) if not ok and err:find('attempt to modify a readonly table') then return 1 end return -1",
                1.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }
    }

    #[test]
    fn compiles_table_sort_builtin() {
        let cases = [
            (
                "local a = {3, 1, 2} table.sort(a) return table.concat(a, ',')",
                "1,2,3",
            ),
            (
                "local a = {'b', 'c', 'a'} table.sort(a) return table.concat(a, ',')",
                "a,b,c",
            ),
            (
                "local a = {10, 9, 8, 4, 19, 23, 0, 0} table.sort(a, function(a, b) return a < b end, 'extra arg') return table.concat(a, ',')",
                "0,0,4,8,9,10,19,23",
            ),
            (
                "local a = {1, 2, 3} table.sort(a, function(a, b) return a > b end) return table.concat(a, ',')",
                "3,2,1",
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }
    }

    #[test]
    fn table_sort_rejects_non_function_comparator_even_when_empty() {
        let chunk = parse_source(
            "local ok_number = pcall(table.sort, {}, 42) local ok_table = pcall(table.sort, {}, {}) if ok_number or ok_table then return -1 end return 1",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(1.0));
    }

    #[test]
    fn gmatch_iterator_charges_captured_text_fuel() {
        let source = r#"local f = string.gmatch("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789", "NOTTHERE") return f()"#;
        let chunk = parse_source(source).unwrap();
        let program = compile(&chunk).unwrap();
        let settings = VmSettings {
            initial_fuel: Some(100),
            ..VmSettings::default()
        };
        let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
        let error = vm.run(&program).unwrap_err();
        assert!(matches!(
            error.kind,
            ErrorKind::FuelExhausted { limit: 100 }
        ));
    }

    #[test]
    fn compiles_table_pack_and_assert_multi_return() {
        let cases = [
            ("local t = table.pack(1, nil, 3) return t.n", 3.0),
            (
                "local t = table.pack(1, nil, 3) if t[1] == 1 and t[2] == nil and t[3] == 3 then return t.n end return -1",
                3.0,
            ),
            ("local t = table.pack(assert(1, 2, 3)) return t.n", 3.0),
            (
                "local t = table.pack(assert(1, nil, 3)) if t[1] == 1 and t[2] == nil and t[3] == 3 then return t.n end return -1",
                3.0,
            ),
            ("local a, b, c = assert(1, 2, 3) return a * 100 + b * 10 + c", 123.0),
            (
                "local function f(...) local t = {...} return table.getn(t) * 100 + t[3] end local r = { f(assert(1, 2, 3)) } return r[1]",
                303.0,
            ),
            (
                "local o = { base = 10 } function o:f(...) local t = {...} return self.base + table.getn(t) * 100 + t[3] end local r = { o:f(assert(1, 2, 3)) } return r[1]",
                313.0,
            ),
            (
                "local o = { base = 10 } function o:f(...) local t = {...} return self.base + table.getn(t) * 100 + t[1] * 10 + t[2] end local r = { o:f(math.modf(1.5)) } return r[1]",
                220.5,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let chunk = parse_source("return table.concat(table.pack(assert(1, 2, 3)), '')").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("123"));

        let chunk = parse_source(
            "local function g() return 1, 2 end local function f(a, b) return a, b, 3 end local t = { f(g()) } return table.concat(t, ',')",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("1,2,3"));
    }

    #[test]
    fn compiles_table_unpack_variable_results() {
        let cases = [
            (
                "local t = table.pack(table.unpack({1, nil, 3}, 1, 3)) if t[1] == 1 and t[2] == nil and t[3] == 3 then return t.n end return -1",
                3.0,
            ),
            ("local t = table.pack(table.unpack({1, 2, 3}, 4, 3)) return t.n", 0.0),
            (
                "local x = {table.unpack({1, 2, 3}, 2, 3)} if x[1] == 2 and x[2] == 3 then return table.getn(x) end return -1",
                2.0,
            ),
            (
                "local a, b, c = table.unpack({1, 2, 3}, 2, 3) if c ~= nil then return -1 end return a * 10 + b",
                23.0,
            ),
            (
                "local t = table.pack(unpack({1, nil, 3}, 1, 3)) if t[1] == 1 and t[2] == nil and t[3] == 3 then return t.n end return -1",
                3.0,
            ),
            ("return select('#', table.unpack(table.create(263, 1)))", 263.0),
            (
                "local t = table.pack(pcall(function() return table.unpack(table.create(100, 'a')) end)) if t[1] == true and t[2] == 'a' and t[101] == 'a' then return t.n end return -1",
                101.0,
            ),
            (
                "local ok, err = pcall(function() return table.unpack(table.create(8000, 1)) end) if not ok and err:find('too many results to unpack') then return 1 end return -1",
                1.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }
    }

    #[test]
    fn compiles_next_with_multiple_results() {
        let chunk = parse_source("local _, value = next({5}) return value").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(5.0));
    }

    #[test]
    fn compiles_select_pcall_error_and_string_methods() {
        let numeric_cases = [
            ("return select('#', assert(1, 2, 3))", 3.0),
            ("return select(-2, 10, 20, 30)", 20.0),
            ("return select('3', 10, 20, 30)", 30.0),
            (
                "local t = table.pack(pcall(function(a, b) return a, b end, 1, 2)) return t.n",
                3.0,
            ),
            (
                "local t = table.pack(xpcall(function(a, b) return a, b, 42 end, error, 1, 2)) return t.n",
                4.0,
            ),
            (
                "local xp = xpcall local t = table.pack(xp(function(a, b) return a, b, 42 end, error, 1, 2)) return t.n",
                4.0,
            ),
        ];

        for (source, expected) in numeric_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let chunk = parse_source(
            "local ok, err = pcall(function() error('oops') end) return err:sub(err:find(': ') + 2, #err)",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("oops"));

        let source = "local ok, err = pcall(function()\n    error('oops')\nend)\nreturn err";
        let chunk = parse_source(source).unwrap();
        let program = compile_with_options(
            &chunk,
            CompileOptions::default().with_source("basic.luau", source),
        )
        .unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(
            vm.string_value(result).unwrap().as_deref(),
            Some("basic.luau:2: oops")
        );

        let source = "function a.x.y ()\na=a+1\nend";
        let chunk = parse_source(source).unwrap();
        let program = compile_with_options(
            &chunk,
            CompileOptions::default().with_source("[string \"function a.x.y ()\"]", source),
        )
        .unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("[string \"function a.x.y ()\"]:1:"),
            "{error}"
        );

        let source = "a = {}\nfunction a.x.y ()\na=a+1\nend";
        let chunk = parse_source(source).unwrap();
        let program = compile_with_options(
            &chunk,
            CompileOptions::default().with_source("[string \"a = {}\"]", source),
        )
        .unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert!(
            error.to_string().starts_with("[string \"a = {}\"]:2:"),
            "{error}"
        );

        let source = r#"
local function lineerror(s)
    local err, msg = pcall(loadstring(s))
    local line = string.match(msg, ":(%d+):")
    return line and line + 0
end
a = {}
return lineerror("function a.x.y ()\na=a+1\nend")
"#;
        let chunk = parse_source(source).unwrap();
        let program = compile_with_options(&chunk, conformance_options()).unwrap();
        let settings = VmSettings {
            loadstring_compiler: Some(test_loadstring_compiler),
            ..VmSettings::default()
        };
        let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(1.0));

        let chunk =
            parse_source("local ok, err = pcall(function() error(nil) end) return err").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.kind(), rustia_vm::ValueKind::Nil);

        let chunk = parse_source(
            "local marker = {} local ok, err = pcall(function() error(marker) end) return not ok and err == marker",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_bool(), Some(true));

        let chunk = parse_source(
            "local ok, msg = xpcall(function() error('oops') end, function(err) return 'bar', 'extra' end) if not ok then return msg end return 'bad'",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("bar"));

        let chunk = parse_source(
            "local ok, msg = xpcall(function() error('oops') end, function(err) error('bar') end) if not ok then return msg end return 'bad'",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(
            vm.string_value(result).unwrap().as_deref(),
            Some("error in error handling")
        );

        let chunk = parse_source(
            "local marker = {} local ok, same = xpcall(function() error(marker) end, function(err) return err == marker end) return not ok and same",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn pcall_enforces_recursion_limit_for_native_callees() {
        let chunk = parse_source(
            "local ok, err = pcall(error, 'ordinary') return not ok and err:find('recursion limit') ~= nil",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let settings = VmSettings {
            recursion_limit: 0,
            ..VmSettings::default()
        };
        let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
        let error = vm.run(&program).unwrap_err();
        assert!(matches!(error.kind, ErrorKind::RecursionLimit { limit: 0 }));
    }

    #[test]
    fn direct_self_recursion_obeys_vm_recursion_limit() {
        for source in [
            r#"
local function deep(n)
    if n > 0 then
        deep(n - 1)
    end
end
local ok, err = pcall(deep, 100)
return not ok and err:find("stack overflow") ~= nil
"#,
            r#"
local function deep(n)
    if n == 0 then
        return 0
    end
    return deep(n - 1)
end
local ok, err = pcall(deep, 100)
return not ok and err:find("stack overflow") ~= nil
"#,
        ] {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let settings = VmSettings {
                recursion_limit: 8,
                ..VmSettings::default()
            };
            let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
            let error = vm.run(&program).unwrap_err();
            assert!(matches!(error.kind, ErrorKind::RecursionLimit { limit: 8 }));
        }
    }

    #[test]
    fn conformance_call_limit_preserves_luau_protected_call_shape() {
        for source in [
            r#"
local function infinite()
    return infinite()
end
local ok, err = pcall(infinite)
return not ok and type(err) == "string"
"#,
            r#"
local function stackover()
    return pcall(stackover)
end
local results = { pcall(stackover) }
return #results == 10000
"#,
        ] {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let settings = VmSettings {
                enable_conformance_luau_call_limit: true,
                ..VmSettings::default()
            };
            let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_bool(), Some(true), "{source}");
        }
    }

    #[test]
    fn generic_iterator_calls_can_yield_and_resume() {
        for loop_source in [
            "for key, value in iterator, nil, nil do output = output .. key .. ':' .. value .. ';' end",
            "for key, value in iterator do output = output .. key .. ':' .. value .. ';' end",
            "for key, value in subject do output = output .. key .. ':' .. value .. ';' end",
        ] {
            let source = format!(
                r#"
local step = 0
local function iterator(_state, _control)
    local resumed = coroutine.yield("pause")
    step += 1
    if step > 2 then
        return nil
    end
    return step, resumed
end
local subject = setmetatable({{}}, {{
    __iter = function()
        return iterator, nil, nil
    end,
}})

local thread = coroutine.create(function()
    local output = ""
    {loop_source}
    return output
end)

local ok1, pause1 = coroutine.resume(thread)
local ok2, pause2 = coroutine.resume(thread, "a")
local ok3, pause3 = coroutine.resume(thread, "b")
local ok4, output = coroutine.resume(thread, "done")
return ok1 and pause1 == "pause"
    and ok2 and pause2 == "pause"
    and ok3 and pause3 == "pause"
    and ok4 and output == "1:a;2:b;"
"#
            );
            let chunk = parse_source(&source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_bool(), Some(true), "{source}");
        }

        let chunk = parse_source(
            r#"
local thread = coroutine.create(function()
    local marker = "before"
    local function iterator()
        marker = "after"
        coroutine.yield("pause")
        return nil
    end
    for _ in iterator do end
    return marker
end)
local ok1, pause = coroutine.resume(thread)
local ok2, marker = coroutine.resume(thread)
return ok1 and pause == "pause" and ok2 and marker == "after"
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn tostring_metamethod_recursion_obeys_vm_recursion_limit() {
        let chunk = parse_source(
            "local t = {} setmetatable(t, { __tostring = tostring }) return tostring(t)",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let settings = VmSettings {
            recursion_limit: 8,
            ..VmSettings::default()
        };
        let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
        let error = vm.run(&program).unwrap_err();
        assert!(matches!(error.kind, ErrorKind::RecursionLimit { limit: 8 }));
    }

    #[test]
    fn table_find_deep_eq_recursion_matches_luau() {
        std::thread::Builder::new()
            .name("table-find-deep-eq-recursion".to_owned())
            .stack_size(128 * 1024 * 1024)
            .spawn(|| {
                let chunk = parse_source(
                    r#"
local function addnrec(n)
    return if n == 1 then 1 else n + addnrec(n - 1)
end

local meta = {
    __eq = function(a, b)
        return addnrec(a.d) == addnrec(b.d)
    end,
}

local function make(d)
    return setmetatable({ d = d }, meta)
end

local o = make(20)
local t = { o, o, o, o, o, o, o, o }
return table.find(t, make(500)) == nil
"#,
                )
                .unwrap();
                let program = compile(&chunk).unwrap();
                let mut vm = Vm::new(Profile::ReproducibleStandard);
                let result = vm.run(&program).unwrap();
                assert_eq!(result.as_bool(), Some(true));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn compiles_table_concat_array_builtin() {
        let cases = [
            ("return table.concat({}, ',')", ""),
            ("return table.concat({1}, ',')", "1"),
            ("return table.concat({1, 2}, ',')", "1,2"),
            ("return table.concat({'a', 'b', 'c'}, ',')", "a,b,c"),
            ("return table.concat({'a', 'b', 'c'}, ',', 2)", "b,c"),
            ("return table.concat({'a', 'b', 'c'}, ',', 1, 2)", "a,b"),
            (
                "local t = {} t[123] = 'a' t[124] = 'b' return table.concat(t, ',', 123, 124)",
                "a,b",
            ),
            (
                "return table.concat({1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17}, ',')",
                "1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17",
            ),
            ("return table.concat({'a', 'b'})", "ab"),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }
    }

    #[test]
    fn compiles_string_sub_builtin() {
        let cases = [
            ("return string.sub('123456789', 2, 4)", "234"),
            ("return string.sub('123456789', 7)", "789"),
            ("return string.sub('123456789', 7, 6)", ""),
            ("return string.sub('123456789', 7, 7)", "7"),
            ("return string.sub('123456789', 0, 0)", ""),
            ("return string.sub('123456789', -10, 10)", "123456789"),
            ("return string.sub('123456789', 1, 9)", "123456789"),
            ("return string.sub('123456789', -10, -20)", ""),
            ("return string.sub('123456789', -1)", "9"),
            ("return string.sub('123456789', -4)", "6789"),
            ("return string.sub('123456789', -6, -4)", "456"),
            ("return string.sub(123456789, 2, 4)", "234"),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }
    }

    #[test]
    fn compiles_string_len_builtin() {
        let cases = [
            ("return string.len('')", 0.0),
            (r#"return string.len("\0\0\0")"#, 3.0),
            ("return string.len('1234567890')", 10.0),
            ("return string.len(123)", 3.0),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }
    }

    #[test]
    fn compiles_string_byte_builtin_single_result() {
        let number_cases = [
            ("return string.byte('a')", 97.0),
            (r#"return string.byte("\0")"#, 0.0),
            (r#"return string.byte("\0\0alo\0x", -1)"#, 120.0),
            ("return string.byte('ba', 2)", 97.0),
            ("return string.byte('\\n\\n', 2, -1)", 10.0),
            ("return string.byte('\\n\\n', 2, 2)", 10.0),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let nil_cases = [
            "return string.byte('')",
            "return string.byte('hi', -3)",
            "return string.byte('hi', 3)",
            "return string.byte('hi', 9, 10)",
            "return string.byte('hi', 2, 1)",
        ];

        for source in nil_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.kind(), rustia_vm::ValueKind::Nil);
        }
    }

    #[test]
    fn compiles_string_ascii_case_builtins() {
        let cases = [
            (r#"return string.upper("ab\0c")"#, "AB\0C"),
            (r#"return string.lower("\0ABCc%$")"#, "\0abcc%$"),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }
    }

    #[test]
    fn compiles_string_reverse_builtin() {
        let cases = [
            (r#"return string.reverse"""#, ""),
            (r#"return string.reverse"\0\1\2\3""#, "\x03\x02\x01\0"),
            (r#"return string.reverse"\0001234""#, "4321\0"),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }
    }

    #[test]
    fn compiles_string_rep_builtin() {
        let cases = [
            (r#"return string.rep("teste", 0)"#, ""),
            (r#"return string.rep("te\0st", 2)"#, "te\0stte\0st"),
            (r#"return string.rep("", 10)"#, ""),
            (r#"return string.rep("", 1e9)"#, ""),
            (r#"return string.rep("a", 3, "ignored")"#, "aaa"),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }
    }

    #[test]
    fn compiles_string_rep_with_runtime_count() {
        let chunk = parse_source(
            "local total = 0 for i = 0, 30 do total += string.len(string.rep('a', i)) end return total",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(465.0));
    }

    #[test]
    fn compiles_string_find_builtin_first_result() {
        let number_cases = [
            (r#"return string.find("123456789", "345")"#, 3.0),
            (
                r#"return string.find("1234567890123456789", "345", 3)"#,
                3.0,
            ),
            (
                r#"return string.find("1234567890123456789", "345", 4)"#,
                13.0,
            ),
            (
                r#"return string.find("1234567890123456789", ".45", -9)"#,
                13.0,
            ),
            (r#"return string.find("", "")"#, 1.0),
            (r#"return string.find("alo(.)alo", "(.)", 1, 1)"#, 4.0),
            (r#"return string.find("123", "2", 0)"#, 2.0),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let nil_cases = [
            r#"return string.find("1234567890123456789", "346", 4)"#,
            r#"return string.find("abcdefg", "\0", 5, 1)"#,
            r#"return string.find("", "aaa", 1)"#,
            r#"return string.find("", "1", 2)"#,
        ];

        for source in nil_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.kind(), rustia_vm::ValueKind::Nil);
        }
    }

    #[test]
    fn compiles_string_split_builtin() {
        let cases = [
            (
                r#"return table.concat(string.split("abc", ""), ",")"#,
                "a,b,c",
            ),
            (
                r#"return table.concat(string.split("abc", "b"), ",")"#,
                "a,c",
            ),
            (
                r#"return table.concat(string.split("abc", "d"), ",")"#,
                "abc",
            ),
            (
                r#"return table.concat(string.split("abc", "c"), ",")"#,
                "ab,",
            ),
            (
                r#"return table.concat(string.split("a,b,c"), "|")"#,
                "a|b|c",
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }

        let chunk =
            parse_source(r#"local t = string.split("abc", "") return #t + string.len(t[3])"#)
                .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(4.0));
    }

    #[test]
    fn compiles_if_then_else_with_luau_truthiness() {
        let cases = [
            ("local a = 1 if a then a = 2 end return a", Some(2.0), None),
            (
                "local a if a then a = 2 end return a",
                None,
                Some(rustia_vm::ValueKind::Nil),
            ),
            (
                "local a = 0 if a then a = 1 else a = 2 end return a",
                Some(1.0),
                None,
            ),
            (
                "local a if a then a = 1 else a = 2 end return a",
                Some(2.0),
                None,
            ),
        ];

        for (source, expected_number, expected_kind) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            if let Some(expected) = expected_number {
                assert_eq!(result.as_number(), Some(expected));
            }
            if let Some(expected) = expected_kind {
                assert_eq!(result.kind(), expected);
            }
        }
    }

    #[test]
    fn compiles_while_repeat_and_break_loops() {
        let cases = [
            (
                "local a = 10 local b = 1 while a > 1 do b = b * 2 a = a - 1 end return b",
                Some(512.0),
                None,
            ),
            (
                "local a = 10 local b = 1 repeat b = b * 2 a = a - 1 until a == 1 return b",
                Some(512.0),
                None,
            ),
            (
                "local a = 10 local b = 1 while true do b = b * 2 a = a - 1 if a == 1 then break end end return b",
                Some(512.0),
                None,
            ),
            (
                "local a = 10 local b = 1 repeat b = b * 2 a = a - 1 if a == 1 then break else end until false return b",
                Some(512.0),
                None,
            ),
            (
                "repeat local a = 5 until a - 4 < 0 or a - 4 >= 0 return nil",
                None,
                Some(rustia_vm::ValueKind::Nil),
            ),
            (
                "local outer = 0 local total = 0 while true do outer = outer + 1 local inner = 0 while true do inner = inner + 1 if inner == 2 then break end end total = total + inner if outer == 3 then break end end return total",
                Some(6.0),
                None,
            ),
            (
                "local a = 0 while true do do break end a = 1 end return a",
                Some(0.0),
                None,
            ),
            (
                "local a = 0 local b = 0 while a < 5 do a += 1 if a == 3 then continue end b += a end return b",
                Some(12.0),
                None,
            ),
            (
                "local a = 0 local b = 0 repeat a += 1 if a == 3 then continue end b += a until a == 5 return b",
                Some(12.0),
                None,
            ),
        ];

        for (source, expected_number, expected_kind) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            if let Some(expected) = expected_number {
                assert_eq!(result.as_number(), Some(expected));
            }
            if let Some(expected) = expected_kind {
                assert_eq!(result.kind(), expected);
            }
        }
    }

    #[test]
    fn compiles_numeric_for_loops_with_luau_step_semantics() {
        let cases = [
            ("local a = 1 for b = 1, 9 do a = a * 2 end return a", 512.0),
            (
                "local a = 1 for b = 1, 9, 2 do a = a * 2 end return a",
                32.0,
            ),
            (
                "local a = 1 for b = 1, 9, -2 do a = a * 2 end return a",
                1.0,
            ),
            (
                "local a = 1 for b = 9, 1, -2 do a = a * 2 end return a",
                32.0,
            ),
            (
                "local a = 1 for b = 1, 9 do a = a * 2 if a == 128 then break end end return a",
                128.0,
            ),
            (
                "local a = 0 for b = 1, 5 do if b == 3 then continue end a += b end return a",
                12.0,
            ),
            (
                "local a = 1 for b = 9, 1, -2 do a = a * 2 b = nil end return a",
                32.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }
    }

    #[test]
    fn numeric_for_loop_variable_is_scoped_to_loop_body() {
        let chunk = parse_source("for i = 1, 1 do end return i").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.kind(), rustia_vm::ValueKind::Nil);
    }

    #[test]
    fn compiles_tonumber_builtin_for_numeric_for_edges() {
        let cases = [
            ("return tonumber('0')", 0.0),
            ("return tonumber(3)", 3.0),
            ("return tonumber('0xa')", 10.0),
            (
                "local c = 0 for i = 1, 10, 0 do c += 1 if c > 10 then break end end return c",
                0.0,
            ),
            (
                "local c = 0 local zero = tonumber('0') for i = 10, 1, zero do c += 1 if c > 10 then break end end return c",
                11.0,
            ),
            (
                "local c = 0 local zero = tonumber('0') for i = 1, 10, zero do c += 1 if c > 10 then break end end return c",
                0.0,
            ),
            (
                "local c = 0 local nan = tonumber('nan') for i = 10, 1, nan do c += 1 end return c",
                1.0,
            ),
            (
                "local c = 0 local nan = tonumber('nan') for i = 1, 10, nan do c += 1 end return c",
                0.0,
            ),
            (
                "local c = 0 local nan = tonumber('nan') for i = 1, nan do c += 1 end return c",
                0.0,
            ),
            (
                "local c = 0 local nan = tonumber('nan') for i = 1, nan, -1 do c += 1 end return c",
                0.0,
            ),
            (
                "local c = 0 for i = -math.huge, 0, math.huge do c += 1 end return c",
                1.0,
            ),
            (
                "local c = 0 for i = math.huge, math.huge, -math.huge do c += 1 end return c",
                1.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }

        let chunk = parse_source("return tonumber('not a number')").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.kind(), rustia_vm::ValueKind::Nil);
    }

    #[test]
    fn compiles_arithmetic_string_to_number_coercion() {
        let cases = [
            ("return 1 + '2'", 3.0),
            ("return 2 * '0xa'", 20.0),
            ("return -'2'", -2.0),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }
    }

    #[test]
    fn compiles_tostring_builtin_for_scalar_values() {
        let cases = [
            ("return tostring(1)", "1"),
            ("return tostring(-1)", "-1"),
            ("return tostring(42)", "42"),
            ("return tostring(-4294967296)", "-4294967296"),
            ("return tostring(9007199254740991)", "9007199254740991"),
            ("return tostring(1.125)", "1.125"),
            ("return tostring(0.5)", "0.5"),
            ("return tostring(0.1)", "0.1"),
            ("return tostring(-0.17)", "-0.17"),
            ("return tostring(math.pi)", "3.141592653589793"),
            ("return tostring(1e+30)", "1e+30"),
            ("return tostring(-1e+24)", "-1e+24"),
            ("return tostring(0/-1)", "-0"),
            ("return tostring(1/0)", "inf"),
            ("return tostring(-1/0)", "-inf"),
            (
                "return tostring(5.4536123983019448e-311)",
                "5.453612398302e-311",
            ),
            (
                "return tostring(5.4834368411298348e-311)",
                "5.48343684113e-311",
            ),
            (
                "return tostring(4.4154895841930002e-305)",
                "4.415489584193e-305",
            ),
            ("return tostring(1125968630513728)", "1125968630513728"),
            (
                "return tostring(3.3951932655938423e-313)",
                "3.3951932656e-313",
            ),
            ("return tostring(1.625)", "1.625"),
            ("return tostring(4.9406564584124654e-324)", "5e-324"),
            ("return tostring(2.0049288280105384)", "2.0049288280105384"),
            ("return tostring(3.0517578125e-05)", "0.000030517578125"),
            ("return tostring(1.383544921875)", "1.383544921875"),
            ("return tostring(3.0053350932691001)", "3.0053350932691"),
            ("return tostring(0.0001373291015625)", "0.0001373291015625"),
            (
                "return tostring(-1.9490628022799998e+289)",
                "-1.94906280228e+289",
            ),
            (
                "return tostring(-0.00610404721867928)",
                "-0.00610404721867928",
            ),
            (
                "return tostring(0.00014495849609375)",
                "0.00014495849609375",
            ),
            ("return tostring(0.453125)", "0.453125"),
            ("return tostring(-4.2375343999999997e+73)", "-4.2375344e+73"),
            (
                "return tostring(1.3202313930270133e-192)",
                "1.3202313930270133e-192",
            ),
            (
                "return tostring(3.6984408976312836e+19)",
                "36984408976312840000",
            ),
            ("return tostring(2.0563000527063302)", "2.05630005270633"),
            (
                "return tostring(4.8970527433648997e-260)",
                "4.8970527433649e-260",
            ),
            ("return tostring(1.62890625)", "1.62890625"),
            (
                "return tostring(1.1295093211933533e+65)",
                "1.1295093211933533e+65",
            ),
            ("return tostring(0)", "0"),
            ("return tostring(-0)", "-0"),
            ("return tostring(true)", "true"),
            ("return tostring(nil)", "nil"),
            ("local a = -0 return tostring(a + 0)", "0"),
            ("local a = -0 return tostring(a - 0)", "-0"),
            ("local a = 0 return tostring(0 - a)", "0"),
            ("local a = 1 / 0 return tostring(a - a)", "nan"),
            ("local a = 0 / 0 return tostring(a * 0)", "nan"),
            ("local a = 2 ^ 1000 return tostring(a / (2 ^ 1000))", "1"),
            ("local a = 2 ^ -1000 return tostring(a / (2 ^ -1000))", "1"),
            ("local a = -2 a = a ^ 0.5 return tostring(a)", "nan"),
            (
                "local c = 0 local function mark() c = 1 end return tostring(1, mark()) .. c",
                "11",
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(
                vm.string_value(result).unwrap().as_deref(),
                Some(expected),
                "{source}"
            );
        }
    }

    #[test]
    fn compiles_string_interpolation() {
        let cases = [
            (r#"return `hello {"world"}`"#, "hello world"),
            (
                r#"return `Escaped brace \{} and backtick \``"#,
                "Escaped brace {} and backtick `",
            ),
            (r#"return `1 {`2 {`3 {4}`}`}`"#, "1 2 3 4"),
            (
                r#"local s = "quoted ` tick" return `x {s}`"#,
                "x quoted ` tick",
            ),
            (
                r#"local s = [[long ` tick]]
-- comment ` tick
return `x {s}`"#,
                "x long ` tick",
            ),
            (
                r#"return `{(function() -- } ` ignored
 return "ok" end)()}`"#,
                "ok",
            ),
            (
                r#"local tostring = function() return "shadowed" end return `{1}`"#,
                "1",
            ),
            (
                r#"local t = setmetatable({}, {__tostring = function() return "meta" end}) return `{t}`"#,
                "meta",
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(
                vm.string_value(result).unwrap().as_deref(),
                Some(expected),
                "{source}"
            );
        }
    }

    #[test]
    fn interpolation_rewrite_keeps_runtime_locations_on_original_lines() {
        let source = "local text = `hello {1 + 2}`\nlocal missing = nil\nreturn missing.field";
        let chunk = parse_source(source).unwrap();
        let program = compile_with_options(
            &chunk,
            CompileOptions::default().with_source("interpolation.luau", source),
        )
        .unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        let location = error
            .location
            .expect("runtime error should have a source location");
        assert_eq!(location.source.as_ref(), "interpolation.luau");
        assert_eq!(location.line, 3);
    }

    #[test]
    fn compiles_math_huge_constant() {
        let chunk = parse_source("return math.huge").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(f64::INFINITY));

        let chunk = parse_source("return -math.huge").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(f64::NEG_INFINITY));
    }

    #[test]
    fn compiles_math_abs_builtin() {
        let cases = [
            ("return math.abs(-5)", 5.0),
            ("return math.abs('-5')", 5.0),
            ("return math.abs(5)", 5.0),
            (
                "local c = 0 local function mark() c = 1 end local r = math.abs(-5, mark()) return r + c",
                6.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let string_cases = [
            (
                "local ok, err = pcall(function() return math.abs() end) return err:match(\"missing argument #1 to 'abs'\")",
                "missing argument #1 to 'abs'",
            ),
            (
                "local ok, err = pcall(function() return math.abs(nil) end) return err:match(\"invalid argument #1 to 'abs'\")",
                "invalid argument #1 to 'abs'",
            ),
            (
                "local ok, err = pcall(function() return math.abs({}) end) return err:match(\"invalid argument #1 to 'abs'\")",
                "invalid argument #1 to 'abs'",
            ),
        ];

        for (source, expected) in string_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(
                vm.string_value(result).unwrap().as_deref(),
                Some(expected),
                "{source}"
            );
        }
    }

    #[test]
    fn compiles_direct_math_scalar_builtins() {
        let cases = [
            ("return math.floor(4.5)", 4.0),
            ("return math.floor('1.5')", 1.0),
            ("return math.ceil(4.5)", 5.0),
            ("return math.ceil('1.5')", 2.0),
            ("return math.round(0.5)", 1.0),
            ("return math.round(3.5)", 4.0),
            ("return math.round(-0.5)", -1.0),
            ("return math.sign(42)", 1.0),
            ("return math.sign(-42)", -1.0),
            ("return math.sign(0 / 0)", 0.0),
            ("return math.clamp(-1, 0, 1)", 0.0),
            ("return math.clamp(0.5, 0, 1)", 0.5),
            ("return math.clamp(2, 0, 1)", 1.0),
            ("return math.clamp('4', 2, 3)", 3.0),
            ("return math.map(0, -1, 1, 0, 2)", 1.0),
            ("return math.map(2.5, 1, 4, 0, 2)", 1.0),
            ("return math.map(-8, 0, 4, 0, 2)", -4.0),
            ("return math.map(16, 0, 4, 0, 2)", 8.0),
            ("return math.lerp(1, 5, 0)", 1.0),
            ("return math.lerp(1, 5, 1)", 5.0),
            ("return math.lerp(1, 5, 0.5)", 3.0),
            ("return math.lerp('1', '5', 0.5)", 3.0),
            ("return math.min(1, -1, 2, -2)", -2.0),
            ("return math.max(1, -1, 2, -2)", 2.0),
            ("return math.max(1, 2, '3')", 3.0),
            ("return math.min(1, 2, '3')", 1.0),
            ("return math.min(1, 0 / 0)", 1.0),
            ("return math.max(1, 0 / 0)", 1.0),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm
                .run(&program)
                .unwrap_or_else(|error| panic!("{source}: {error}"));
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }

        let string_cases = [
            ("return tostring(math.round(-0.4))", "-0"),
            ("return tostring(math.min(0 / 0, 2))", "nan"),
            ("return tostring(math.max(0 / 0, 2))", "nan"),
        ];

        for (source, expected) in string_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }

        let chunk =
            parse_source("local function make() return function() end end return make() == make()")
                .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn compiles_more_direct_math_builtins() {
        let number_cases = [
            ("return math.pi", f64::from_bits(0x4009_21fb_5444_2d18)),
            ("return math.fmod(3, 2)", 1.0),
            ("return math.fmod(-3, 2)", -1.0),
            ("return math.fmod(3, -2)", 1.0),
            ("return math.fmod(-3, -2)", -1.0),
            ("return math.fmod('1.5', 1)", 0.5),
            ("return math.fmod(3, 2, 99)", 1.0),
            ("return math.pow(2, 0)", 1.0),
            ("return math.pow(2, 2)", 4.0),
            ("return math.pow(4, 0.5)", 2.0),
            ("return math.pow(-2, 2)", 4.0),
            ("return math.pow('2', 2)", 4.0),
            ("return math.pow(2, 2, 99)", 4.0),
            ("return math.sqrt(4)", 2.0),
            ("return math.sqrt('4')", 2.0),
            ("return math.sqrt(4, 99)", 2.0),
            ("return math.deg('0')", 0.0),
            ("return math.rad('0')", 0.0),
            ("return math.deg('0', 99)", 0.0),
            ("return math.rad('0', 99)", 0.0),
            ("local math = { pi = 7 }; return math.pi", 7.0),
            ("local math = { sqrt = 1 }; return math.sqrt", 1.0),
            (
                "local m = math m.sqrt = function() return 99 end return math.sqrt(4)",
                99.0,
            ),
            (
                "rawset(math, 'sqrt', function() return 99 end) return math.sqrt(4)",
                99.0,
            ),
            (
                "rawset<<any>>(math, 'sqrt', function() return 99 end) return math.sqrt(4)",
                99.0,
            ),
            (
                "_G<<any>>.assert = function() return 37 end return assert(true)",
                37.0,
            ),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }

        let bool_cases = [
            ("return math.pi > 3", true),
            ("return math.pi < 4", true),
            ("return math.deg(math.pi / 2) > 89", true),
            ("return math.deg(math.pi / 2) < 91", true),
            ("return math.rad(90) > 1.5", true),
            ("return math.rad(90) < 1.6", true),
            ("return math.isnan(0 / 0)", true),
            ("return math.isnan(10)", false),
            ("return math.isnan(math.huge)", false),
            ("return math.isnan('123.45')", false),
            ("return math.isnan('123.45', 99)", false),
            ("return math.isinf(math.huge)", true),
            ("return math.isinf(-math.huge)", true),
            ("return math.isinf(10)", false),
            ("return math.isinf(0 / 0)", false),
            ("return math.isinf('123.45')", false),
            ("return math.isinf('123.45', 99)", false),
            ("return math.isfinite(math.huge)", false),
            ("return math.isfinite(-math.huge)", false),
            ("return math.isfinite(0 / 0)", false),
            ("return math.isfinite(123.45)", true),
            ("return math.isfinite('123.45')", true),
            ("return math.isfinite('123.45', 99)", true),
        ];

        for (source, expected) in bool_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_bool(), Some(expected), "{source}");
        }

        let string_cases = [("return tostring(math.pow(-2, 0.5))", "nan")];

        for (source, expected) in string_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }

        let chunk = parse_source("local function f() return f == f end return f()").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_bool(), Some(true));

        let chunk =
            parse_source("local function f() local t = { [f] = 7 } return t[f] end return f()")
                .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(7.0));
    }

    #[test]
    fn compiles_direct_libm_math_builtins() {
        let number_cases = [
            ("return math.sin('0')", 0.0),
            ("return math.cos('0')", 1.0),
            ("return math.tan('0')", 0.0),
            ("return math.asin('0')", 0.0),
            ("return math.acos('1')", 0.0),
            ("return math.atan('0')", 0.0),
            ("return math.atan2('0', '0')", 0.0),
            ("return math.sinh('0')", 0.0),
            ("return math.cosh('0')", 1.0),
            ("return math.tanh('0')", 0.0),
            ("return math.exp('0')", 1.0),
            ("return math.log10('10')", 1.0),
            ("return math.log('8', 2)", 3.0),
            ("return math.log('10', 10)", 1.0),
            ("return math.log('16', 4)", 2.0),
            ("return math.ldexp('0.75', 1)", 1.5),
            ("return math.ldexp('0.75', 1, 99)", 1.5),
            ("return math.sin('0', 99)", 0.0),
            ("return math.atan2('0', '0', 99)", 0.0),
            ("return math.log('8', 2, 99)", 3.0),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }

        let bool_cases = [
            ("return math.atan2(1, 0) > 1.5", true),
            ("return math.atan2(1, 0) < 1.6", true),
        ];

        for (source, expected) in bool_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_bool(), Some(expected), "{source}");
        }

        let string_cases = [("return tostring(math.log('0'))", "-inf")];

        for (source, expected) in string_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }
    }

    #[test]
    fn loop_control_outside_its_function_loop_is_a_compile_error() {
        for keyword in ["break", "continue"] {
            for source in [
                keyword.to_owned(),
                format!("repeat local function a() {keyword} end until false"),
            ] {
                let chunk = parse_source(&source).unwrap();
                let error = compile(&chunk).unwrap_err();
                let start = source.find(keyword).unwrap();
                assert_eq!(
                    error.span,
                    Some(rustia_lexer::Span::new(start, start + keyword.len()))
                );
                assert_eq!(error.message, format!("{keyword} outside loop"));
            }
        }
    }

    #[test]
    fn compile_errors_keep_source_span() {
        let chunk = parse_source("local function unused() break end").unwrap();
        let error = compile(&chunk).unwrap_err();
        assert!(error.span.is_some());
    }

    #[test]
    fn compiles_assert_builtin_call() {
        let chunk = parse_source("return assert(1)").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(1.0));
    }

    #[test]
    fn assert_builtin_does_not_override_locals() {
        let chunk = parse_source("local assert = 1; assert(true)").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert!(matches!(
            error.kind,
            ErrorKind::RuntimeError { ref message } if message == "attempt to call a number value"
        ));
    }

    #[test]
    fn compiles_equality_for_assert_conditions() {
        let chunk = parse_source("local x = 1; return assert(x == 1)").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn mixed_string_representations_preserve_compatible_eq_metamethods() {
        let chunk = parse_source(
            r#"
                local atom = next({same = true})
                local inline = "same"
                local left = setmetatable({}, {__eq = atom})
                local right = setmetatable({}, {__eq = inline})
                return left == right
            "#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let error = Vm::new(Profile::ReproducibleStandard)
            .run(&program)
            .expect_err("equal mixed-representation metamethod values must be invoked");
        assert!(matches!(
            error.kind,
            ErrorKind::RuntimeError { ref message }
                if message == "attempt to call a string value"
        ));
    }

    #[test]
    fn compiles_relational_operators() {
        let chunk = parse_source("local x = 1; return assert(x <= 2)").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn compiles_and_or_with_luau_value_semantics() {
        let cases = [
            ("local x = 1; return x and 2", Some(2.0), None),
            (
                "local x = nil; return x and 2",
                None,
                Some(rustia_vm::ValueKind::Nil),
            ),
            ("local x = 1; return x or 2", Some(1.0), None),
            ("local x = nil; return x or 2", Some(2.0), None),
        ];

        for (source, expected_number, expected_kind) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            if let Some(expected) = expected_number {
                assert_eq!(result.as_number(), Some(expected));
            }
            if let Some(expected) = expected_kind {
                assert_eq!(result.kind(), expected);
            }
        }
    }

    #[test]
    fn compiles_and_or_short_circuiting() {
        let cases = [
            ("local x = false; return x and assert(false)", Some(false)),
            ("local x = true; return x or assert(false)", Some(true)),
        ];

        for (source, expected_bool) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_bool(), expected_bool);
        }
    }

    #[test]
    fn compiles_not_using_luau_truthiness() {
        let chunk = parse_source("local x = false; return not x").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn assert_builtin_evaluates_all_arguments() {
        let chunk = parse_source("assert(true, assert(false))").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert_assertion_message(error.kind, "assertion failed!");
    }

    #[test]
    fn assert_without_arguments_reports_missing_argument() {
        let chunk = parse_source("assert()").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert_assertion_message(error.kind, "missing argument #1");
    }

    #[test]
    fn assert_failure_uses_string_message_argument() {
        let chunk = parse_source("assert(nil, \"epic fail\")").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert_assertion_message(error.kind, "epic fail");
    }

    #[test]
    fn compiles_call_statements_for_effect() {
        let chunk = parse_source("assert(1); return 2").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(2.0));
    }

    #[test]
    fn compiles_scalar_function_values_and_calls() {
        let number_cases = [
            (
                "local function add1(x) return x + 1 end return add1(41)",
                42.0,
            ),
            ("return (function (x) return x * 2 end)(20)", 40.0),
            (
                "local function fact(n) if n == 0 then return 1 else return n * fact(n - 1) end end return fact(5)",
                120.0,
            ),
            (
                "local function first(a) return a end return first(7, 8)",
                7.0,
            ),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let string_cases = [
            ("local function f() end return type(f)", "function"),
            (
                "local function third(a, b, c) return type(c) end return third(1, 2)",
                "nil",
            ),
        ];

        for (source, expected) in string_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(vm.string_value(result).unwrap().as_deref(), Some(expected));
        }
    }

    #[test]
    fn compiles_fixed_multi_return_function_calls() {
        let number_cases = [
            (
                "local function f() return 1, 2, 30, 4 end local a, b, c, d = f() return a * 1000000 + b * 10000 + c * 100 + d",
                1023004.0,
            ),
            (
                "local function f() return 1, 2 end local a, b = 0, 0 a, b = f() return a * 10 + b",
                12.0,
            ),
            (
                "local function ret2(a, b) return a, b end local a, b = ret2(4, 5) return a * 10 + b",
                45.0,
            ),
            (
                "local function f() return 1, 2 end local a = f() return a",
                1.0,
            ),
            (
                "local function ret2(a, b) return a, b end local a, b, c, d = ret2(1, 2), ret2(3, 4) if d == nil then return a * 100 + b * 10 + c else return 0 end",
                134.0,
            ),
            (
                "local function ret2(a, b) return a, b end local a, b, c, d a, b, c, d = ret2(1, 2), ret2(3, 4) if d == nil then return a * 100 + b * 10 + c else return 0 end",
                134.0,
            ),
            (
                "local function ret2(a, b) return a, b end local a, b, c = ret2(1, 2), 3 if c == nil then return a * 10 + b else return 0 end",
                13.0,
            ),
            (
                "local function tail() return 2, 3 end local function f() return 1, tail() end local a, b, c = f() return a * 100 + b * 10 + c",
                123.0,
            ),
            (
                "local function unlpack(t, i) i = i or 1 if i <= table.getn(t) then return t[i], unlpack(t, i + 1) end end local a, b, c, d = unlpack({1, 2, 3}) if d == nil then return a * 100 + b * 10 + c else return 0 end",
                123.0,
            ),
            (
                "local function f() return 1, 2, 30, 4 end local function ret2(a, b) return a, b end local a, b, c, d = ret2(f()), ret2(f()) if d == nil then return a * 100 + b * 10 + c else return 0 end",
                112.0,
            ),
            (
                "local function f() return 4, 5 end local function ret2(a, b) return a, b end local a, b = ret2(f()) return a * 10 + b",
                45.0,
            ),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }

        let chunk = parse_source("local function f() return 1, 2 end local a, b, c = f() return c")
            .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.kind(), rustia_vm::ValueKind::Nil);

        let chunk = parse_source(
            r#"
local function range(i, j)
    if i <= j then
        return i, range(i + 1, j)
    end
end

local s = string.char(range(0, 255))
return #s * 1000 + string.byte(s, 256)
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(256255.0));
    }

    #[test]
    fn compiles_vararg_functions_and_list_adjustment() {
        let number_cases = [
            (
                "local function f(a, ...) local b, c, d = ... return a * 100 + b * 10 + c + (d or 0) end return f(1, 2, 3)",
                123.0,
            ),
            (
                "local function f(...) return 1, ... end local a, b, c = f(2, 3) return a * 100 + b * 10 + c",
                123.0,
            ),
            (
                "local function f(...) return ... end local function g(a, b, c) return a * 100 + b * 10 + c end return g(1, f(2, 3))",
                123.0,
            ),
            (
                "local t = {10, 20} function t:f(...) local a, b = ... return self[a] + b end return t:f(2, 3)",
                23.0,
            ),
            (
                "local function f(...) local t = { ... } return table.getn(t) end return f(1, 2, 3)",
                3.0,
            ),
            (
                "local function f(...) local t = { 9, ... } return table.getn(t) * 100 + t[1] * 10 + t[4] end return f(1, 2, 3)",
                493.0,
            ),
            (
                "local function f(n, ...) local t = { select(n, ...) } return table.getn(t) * 100 + t[1] * 10 + t[2] end return f(2, 10, 20, 30)",
                430.0,
            ),
            (
                "local function f() return 2, 3 end local function g(a, b) return a, b end local function h(a, b, c) return a * 100 + b * 10 + c end return h(1, g(f()))",
                123.0,
            ),
            (
                "local function outer(...) local function g(a, b) return a, b end local function h(a, b, c) return a * 100 + b * 10 + c end return h(1, g(...)) end return outer(2, 3)",
                123.0,
            ),
            (
                "local function oneless(a, ...) return ... end local function f(a, ...) local b, c, d = ... return a, b, c, d, oneless(oneless(oneless(...))) end local a, b, c, d, e = f(5, 4, 3, 2, 1) return (a or 0) * 10000 + (b or 0) * 1000 + (c or 0) * 100 + (d or 0) * 10 + (e or 0)",
                54321.0,
            ),
            (
                "local t = { base = 100 } local function f() return 2, 3 end function t:m(a, b) return self.base + a * 10 + b end return t:m(f())",
                123.0,
            ),
            (
                "local function f() return 1, 2, 3 end local t = { f() } return table.getn(t) * 100 + t[3]",
                303.0,
            ),
            (
                "local function f<T...>(...: T...): T... return ... end local a, b = f(1, 2) return a * 10 + b",
                12.0,
            ),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm
                .run(&program)
                .unwrap_or_else(|error| panic!("{source}: {error}"));
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }

        let chunk = parse_source("local function f() return ... end").unwrap();
        let error = compile(&chunk).unwrap_err();
        assert!(error
            .to_string()
            .contains("cannot use '...' outside a vararg function"));
    }

    #[test]
    fn compiles_closures_with_captured_locals_and_upvalues() {
        let cases = [
            (
                "local x = 41 local f = function() return x + 1 end return f()",
                42.0,
            ),
            (
                "local x = 1 local function inc() x += 1 return x end inc() return inc()",
                3.0,
            ),
            (
                "local x = 10 local function a() x += 1 return x end local function b() x += 2 return x end a() return b()",
                13.0,
            ),
            (
                "local x = 5 local function outer() local function inner() return x end return inner() end return outer()",
                5.0,
            ),
            (
                "local function make(x) return function() x += 1 return x end end local f = make(9) f() return f()",
                11.0,
            ),
            (
                "local x; x = function() return x end return (x() == x) and 1 or 0",
                1.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }
    }

    #[test]
    fn immutable_capture_analysis_tracks_sibling_and_transitive_mutation() {
        let counter = compile(
            &parse_source(
                "local function make(start, step) local c = start return function() c += step return c end end return make(1, 2)()",
            )
            .unwrap(),
        )
        .unwrap();
        let counter_leaf = counter
            .functions
            .iter()
            .find(|function| function.upvalues.len() == 2)
            .expect("counter leaf");
        assert!(matches!(
            counter_leaf.upvalues[0],
            UpvalueDescriptor::ParentLocal(_)
        ));
        assert!(matches!(
            counter_leaf.upvalues[1],
            UpvalueDescriptor::ParentLocalValue(_)
        ));

        let siblings = compile(
            &parse_source(
                "local x = 1 local function get() return x end local function set(v) x = v end set(4) return get()",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(siblings.functions.len(), 2);
        for function in &siblings.functions {
            assert!(matches!(
                function.upvalues.as_slice(),
                [UpvalueDescriptor::ParentLocal(_)]
            ));
        }

        let transitive = compile(
            &parse_source(
                "local x = 7 local function outer() return function() return x end end return outer()()",
            )
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            transitive.functions[0].upvalues.as_slice(),
            [UpvalueDescriptor::ParentLocalValue(_)]
        ));
        assert!(matches!(
            transitive.functions[1].upvalues.as_slice(),
            [UpvalueDescriptor::ParentUpvalueValue(_)]
        ));

        let transitive_mutation = compile(
            &parse_source(
                "local x=1 local function outer() local function get() return x end local function set(v) x=v end set(9) return get() end return outer()",
            )
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            transitive_mutation.functions[0].upvalues.as_slice(),
            [UpvalueDescriptor::ParentLocal(_)]
        ));
        for function in &transitive_mutation.functions[1..=2] {
            assert!(matches!(
                function.upvalues.as_slice(),
                [UpvalueDescriptor::ParentUpvalue(_)]
            ));
        }

        let immutable_function = compile(
            &parse_source(
                "local function f() return 7 end local function get() return f end return get()()",
            )
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            immutable_function.functions[1].upvalues.as_slice(),
            [UpvalueDescriptor::ParentLocalValue(_)]
        ));

        let nested_self = compile(
            &parse_source(
                "local function f() return function() return f end end return f()()==f and 1 or 0",
            )
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            nested_self.functions[0].upvalues.as_slice(),
            [UpvalueDescriptor::ParentLocal(_)]
        ));
        assert!(matches!(
            nested_self.functions[1].upvalues.as_slice(),
            [UpvalueDescriptor::ParentUpvalue(_)]
        ));

        for (program, expected) in [
            (counter, 3.0),
            (siblings, 4.0),
            (transitive, 7.0),
            (transitive_mutation, 9.0),
            (immutable_function, 7.0),
            (nested_self, 1.0),
        ] {
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            assert_eq!(vm.run(&program).unwrap().as_number(), Some(expected));
        }
    }

    #[test]
    fn capture_discovery_precedes_every_loop_write_form() {
        let cases = [
            (
                "local x=0 local fs={} for i=1,3 do x += 1 fs[i]=function() return x end end return fs[1]()*100+fs[2]()*10+fs[3]()",
                333.0,
            ),
            (
                "local x=0 local fs={} for i=1,3 do x=i fs[i]=function() return x end end return fs[1]()*100+fs[2]()*10+fs[3]()",
                333.0,
            ),
            (
                "local x=0 local f for i=1,3 do x += 1 if i==1 then f=function() return x end end if i==3 then break end end return f()",
                3.0,
            ),
            (
                "local x=0 local f for i=1,3 do x += 1 if i==1 then local function middle() return function() return x end end f=middle() end end return f()",
                3.0,
            ),
            (
                "local x,y=0,0 local fs={} for i=1,3 do x,y=i,i*2 fs[i]=function() return x*10+y end end return fs[1]()*10000+fs[2]()*100+fs[3]()",
                363_636.0,
            ),
            (
                "local function pair(i) return i,i*2 end local x,y=0,0 local fs={} for i=1,3 do x,y=pair(i) fs[i]=function() return x*10+y end end return fs[1]()*10000+fs[2]()*100+fs[3]()",
                363_636.0,
            ),
            (
                "local function outer() local x=0 local fs={} for i=1,3 do x += 1 fs[i]=function() return x end end return fs[1]()*100+fs[2]()*10+fs[3]() end return outer()",
                333.0,
            ),
            (
                "local x=0 local f for outer=1,3 do for i=1,16 do x += 1 end if outer==1 then f=function() return x end end end return f()",
                48.0,
            ),
            (
                "local x=0 local proxy local ys={} for i=1,3 do if proxy then local z=proxy+1 end ys[i]=x proxy=setmetatable({}, {__add=function() x += 1 return 0 end}) end return ys[1]*100+ys[2]*10+ys[3]",
                12.0,
            ),
            (
                "local f=function() return 0 end local fs={} for i=1,3 do function f() return i end fs[i]=function() return f end end local a,b,c=fs[1](),fs[2](),fs[3]() return (a()==3 and b()==3 and c()==3) and 1 or 0",
                1.0,
            ),
            (
                "local fs={} for i=1,3 do local x=i fs[i]=function() return x end end return fs[1]()*100+fs[2]()*10+fs[3]()",
                123.0,
            ),
        ];

        for (source, expected) in cases {
            let program = compile(&parse_source(source).unwrap()).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            assert_eq!(
                vm.run(&program).unwrap().as_number(),
                Some(expected),
                "{source}"
            );
        }

        let source = "local f=(loadstring('local x=0 local fs={} for i=1,3 do x += 1 fs[i]=function() return x end end return fs[1]()*100+fs[2]()*10+fs[3]()')) return f()";
        let program =
            compile_with_options(&parse_source(source).unwrap(), conformance_options()).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&program).unwrap().as_number(), Some(333.0));
    }

    #[test]
    fn captured_loop_variables_use_fresh_cells_per_iteration() {
        let cases = [
            (
                "local a = {} for i = 1, 3 do a[i] = function() return i end end return a[1]() * 100 + a[2]() * 10 + a[3]()",
                123.0,
            ),
            (
                "local a = {} for i = 1, 3 do a[i] = { get = function() return i end, set = function(x) i = x end } end a[1].set(10) return a[2].get()",
                2.0,
            ),
            (
                "local a = {} for i, k in pairs({ 'a', 'b' }) do a[i] = { get_i = function() return i end, set_i = function(x) i = x end, get_k = function() return k end } end a[1].set_i(10) return a[2].get_i()",
                2.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }
    }

    #[test]
    fn captured_loop_body_locals_use_fresh_cells_per_declaration() {
        let cases = [
            (
                "local a = {} for i = 1, 3 do local x = i a[i] = function() return x end end return a[1]() * 100 + a[2]() * 10 + a[3]()",
                123.0,
            ),
            (
                "local a = {} for i = 1, 3 do local x = i a[i] = { get = function() return x end, set = function(v) x = v end } end a[1].set(10) return a[2].get()",
                2.0,
            ),
            (
                "local a = {} local i = 1 repeat local x = i a[i] = function() i = x + 1 return x end until i > 10 or a[i]() ~= x return a[1]() * 100 + a[3]() * 10 + i",
                134.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }
    }

    #[test]
    fn compiles_function_declaration_statements() {
        let number_cases = [
            (
                "local f; function f(x) return x + 1 end return f(41)",
                42.0,
            ),
            (
                "local t = {}; function t.add(x) return x + 1 end return t.add(41)",
                42.0,
            ),
            (
                "local a = { b = { c = {} } }; function a.b.c.f1(x) return x + 1 end return a.b.c.f1(4)",
                5.0,
            ),
            (
                "local t = { value = 0 }; function t:set(x) self.value = x end t:set(12) return t.value",
                12.0,
            ),
            (
                "local t = { value = 37 }; function t:add(x) return self.value + x end return t:add(5)",
                42.0,
            ),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let chunk = parse_source("local f; function f() return f == f end return f()").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn compiles_method_calls_with_implicit_self() {
        let number_cases = [
            (
                "local t = { value = 37, add = function(self, x) return self.value + x end } return t:add(5)",
                42.0,
            ),
            (
                "local t = { value = 0, set = function(self, x) self.value = x end } t:set(12) return t.value",
                12.0,
            ),
            (
                "local t = { first = function(self, xs) return xs[1] end } return t:first{42}",
                42.0,
            ),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected));
        }

        let chunk = parse_source(
            "local t = { prefix = 'a', join = function(self, x) return self.prefix .. x end } return t:join'b'",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("ab"));
    }

    #[test]
    fn default_environment_compiles_print_statement() {
        let chunk = parse_source("print(\"testing asserts\"); return 2").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(2.0));
    }

    #[test]
    fn conformance_environment_compiles_silenced_print_statement() {
        let chunk = parse_source("print(\"testing asserts\"); return 2").unwrap();
        let program = compile_with_options(&chunk, conformance_options()).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(2.0));
    }

    #[test]
    fn silenced_print_still_evaluates_arguments() {
        let chunk = parse_source("print(assert(false))").unwrap();
        let program = compile_with_options(&chunk, conformance_options()).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert_assertion_message(error.kind, "assertion failed!");
    }

    #[test]
    fn silenced_print_evaluates_all_arguments() {
        let chunk = parse_source("print(1, assert(false))").unwrap();
        let program = compile_with_options(&chunk, conformance_options()).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert_assertion_message(error.kind, "assertion failed!");
    }

    #[test]
    fn conformance_print_helper_does_not_override_locals() {
        let chunk = parse_source("local print = 1; print(\"testing asserts\")").unwrap();
        let program = compile_with_options(&chunk, conformance_options()).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert!(matches!(
            error.kind,
            ErrorKind::RuntimeError { ref message } if message == "attempt to call a number value"
        ));
    }

    #[test]
    fn conformance_environment_compiles_concat_helper() {
        let chunk = parse_source("return concat(4, nil, false, 'x')").unwrap();
        let program = compile_with_options(&chunk, conformance_options()).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(
            vm.string_value(result).unwrap().as_deref(),
            Some("4,nil,false,x")
        );
    }

    #[test]
    fn conformance_concat_helper_uses_tostring_metamethod() {
        let chunk =
            parse_source("local t = setmetatable({}, {__tostring = function() return 'mt' end}) return concat(t, 2)")
                .unwrap();
        let program = compile_with_options(&chunk, conformance_options()).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("mt,2"));
    }

    #[test]
    fn compiles_assert_failure_as_structured_error() {
        let chunk = parse_source("assert(false)").unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let error = vm.run(&program).unwrap_err();
        assert_assertion_message(error.kind, "assertion failed!");
    }

    #[test]
    fn lowers_live_dotted_imports_and_keeps_fourth_component_generic() {
        let chunk =
            parse_source("foo = {bar = {baz = {qux = 42}}}; return foo.bar.baz.qux").unwrap();
        let program = compile(&chunk).unwrap();
        let import = program
            .main
            .code
            .iter()
            .find(|instruction| instruction.op == Opcode::GetImport)
            .copied()
            .expect("three-component prefix should use GETIMPORT");
        assert_eq!(import.b, 3);
        assert!(program
            .main
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::GetTableKs));
        let Constant::Import(path) = &program.constants()[import.aux as usize] else {
            panic!("GETIMPORT must reference an import descriptor");
        };
        let names = path
            .iter()
            .map(|index| match &program.constants()[*index as usize] {
                Constant::String(name) => String::from_utf8(name.clone()).unwrap(),
                _ => panic!("import component must reference a string"),
            })
            .collect::<Vec<_>>();
        assert_eq!(names, ["foo", "bar", "baz"]);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&program).unwrap().as_number(), Some(42.0));
    }

    #[test]
    fn lowers_constant_string_table_and_namecall_fastpaths() {
        let program = compile(
            &parse_source(
                r#"
                    local t = {
                        value = 1,
                        ["other"] = 2,
                        add = function(self, x) return self.value + x end,
                    }
                    t.value += 1
                    t["other"] = t.value
                    local key = "other"
                    t[key] = 3
                    local dynamic = t[key]
                    return t:add(dynamic) + 0
                "#,
            )
            .unwrap(),
        )
        .unwrap();
        let code = program
            .main
            .code
            .iter()
            .chain(program.functions.iter().flat_map(|function| &function.code))
            .collect::<Vec<_>>();
        assert!(code
            .iter()
            .any(|instruction| instruction.op == Opcode::GetTableKs));
        assert!(code
            .iter()
            .any(|instruction| instruction.op == Opcode::SetTableKs));
        assert!(code
            .iter()
            .filter(|instruction| instruction.op == Opcode::SetTableKs)
            .all(|instruction| instruction.c != 0));
        let compound_get = code
            .iter()
            .find(|instruction| instruction.op == Opcode::GetTableKs && instruction.c != 0)
            .expect("compound constant-field read should retain its loaded key");
        assert!(code.iter().any(|instruction| {
            instruction.op == Opcode::SetIndex && instruction.b == compound_get.c
        }));
        assert!(code
            .iter()
            .any(|instruction| instruction.op == Opcode::NameCall && instruction.c == 1));
        assert!(code
            .iter()
            .all(|instruction| instruction.op != Opcode::CheckMethod));
        assert!(code
            .iter()
            .any(|instruction| instruction.op == Opcode::GetIndex));
        assert!(code
            .iter()
            .any(|instruction| instruction.op == Opcode::SetIndex));

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&program).unwrap().as_number(), Some(5.0));

        let tail = compile(
            &parse_source("local t = {m = function(self) return 42 end}; return t:m()").unwrap(),
        )
        .unwrap();
        assert!(tail
            .main
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::NameCall && instruction.c == 1));

        let indexed_class = compile(
            &parse_source(
                r#"
                    local Class = {}
                    Class.__index = Class
                    function Class:add(x) return self.value + x end
                    local object = setmetatable({value = 40}, Class)
                    local result = 0
                    for i = 1, 2 do result = object:add(i) end
                    return result
                "#,
            )
            .unwrap(),
        )
        .unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&indexed_class).unwrap().as_number(), Some(42.0));

        let missing =
            compile(&parse_source("local t = {}; return t:missing() + 0").unwrap()).unwrap();
        let error = Vm::new(Profile::ReproducibleStandard)
            .run(&missing)
            .expect_err("checked NAMECALL must retain the missing-method diagnostic");
        assert!(matches!(
            error.kind,
            ErrorKind::RuntimeError { ref message }
                if message == "attempt to call missing method 'missing' of table"
        ));
    }

    #[test]
    fn settableks_preloads_its_logical_key_before_an_intervening_rhs_call() {
        let program = compile(
            &parse_source(
                r#"
                    local t = {}
                    local function rhs() return 42 end
                    t.field = rhs()
                    return t.field
                "#,
            )
            .unwrap(),
        )
        .unwrap();
        let code = &program.main.code;
        let (set_index, set) = code
            .iter()
            .enumerate()
            .find(|(_, instruction)| instruction.op == Opcode::SetTableKs)
            .expect("constant field write should use SETTABLEKS");
        assert_ne!(set.c, 0, "compiler writes must use preloaded-key mode");
        let key_load = code[..set_index]
            .iter()
            .rposition(|instruction| {
                instruction.a == set.c
                    && matches!(instruction.op, Opcode::LoadConst | Opcode::LoadConstWide)
            })
            .expect("SETTABLEKS key register should have a logical constant load");
        let rhs_call = code[..set_index]
            .iter()
            .rposition(|instruction| matches!(instruction.op, Opcode::Call | Opcode::CallMulti))
            .expect("the RHS should contain the ordinary function call");
        assert!(
            key_load < rhs_call && rhs_call < set_index,
            "the key step must remain before RHS side effects and the write after them"
        );

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&program).unwrap().as_number(), Some(42.0));
    }

    #[test]
    fn every_source_write_preloads_the_matching_settableks_key() {
        let program = compile(
            &parse_source(
                r#"
                    local a, b = {}, {}
                    local function rhs(value) return value end
                    a.one, b.two = rhs(1), rhs(2)
                    local c = { three = rhs(3), ["four"] = rhs(4) }
                    function a.five() return 5 end
                    return a.one + b.two + c.three + c.four + a.five()
                "#,
            )
            .unwrap(),
        )
        .unwrap();
        let code = &program.main.code;
        let sets = code
            .iter()
            .enumerate()
            .filter(|(_, instruction)| instruction.op == Opcode::SetTableKs)
            .collect::<Vec<_>>();
        assert_eq!(sets.len(), 5);
        for (set_index, set) in sets {
            assert_ne!(set.c, 0, "source writes must use preloaded-key mode");
            let load = code[..set_index]
                .iter()
                .rfind(|instruction| {
                    instruction.a == set.c
                        && matches!(instruction.op, Opcode::LoadConst | Opcode::LoadConstWide)
                })
                .expect("preloaded key must have a preceding constant load");
            let loaded_constant = match load.op {
                Opcode::LoadConst => i32::from(load.c),
                Opcode::LoadConstWide => load.aux,
                _ => unreachable!(),
            };
            assert_eq!(loaded_constant, set.aux);
        }

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&program).unwrap().as_number(), Some(15.0));
    }

    #[test]
    fn import_lowering_respects_current_and_outer_lexical_bindings() {
        let cases = [
            "local math = {pi = 9}; return math.pi",
            "local function read(math) return math.pi end; return read({pi = 8})",
            "local math = {pi = 7}; local function read() return math.pi end; return read()",
        ];
        for (source, expected) in cases.into_iter().zip([9.0, 8.0, 7.0]) {
            let program = compile(&parse_source(source).unwrap()).unwrap();
            assert!(program
                .main
                .code
                .iter()
                .chain(program.functions.iter().flat_map(|function| &function.code))
                .all(|instruction| instruction.op != Opcode::GetImport));
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            assert_eq!(vm.run(&program).unwrap().as_number(), Some(expected));
        }
    }

    #[test]
    fn import_cache_observes_live_field_mutation() {
        let chunk = parse_source(
            "local result = 0; for i = 1, 2 do result = math.pi; math.pi = 7 end; return result",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(program
            .main
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::GetImport));
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&program).unwrap().as_number(), Some(7.0));
    }

    #[test]
    fn getimport_roots_intermediate_values_across_fused_gc_steps() {
        let source = r#"
            local weak = setmetatable({}, {__mode = "k"})
            local object_meta = {
                __index = function(self, key)
                    return weak[self]
                end,
            }
            foo = setmetatable({}, {
                __index = function(_, key)
                    local object = setmetatable({}, object_meta)
                    weak[object] = 123
                    return object
                end,
            })
            return foo.missing.bar
        "#;
        let program = compile(&parse_source(source).unwrap()).unwrap();
        assert!(program
            .main
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::GetImport && instruction.b == 3));
        let settings = VmSettings {
            deterministic_gc_interval: std::num::NonZeroU64::new(1).unwrap(),
            ..VmSettings::default()
        };
        let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
        assert_eq!(vm.run(&program).unwrap().as_number(), Some(123.0));
    }

    #[test]
    fn generic_fastcall_covers_local_aliases_and_deopts_on_reassignment() {
        let chunk = parse_source(
            "local f = math.floor; local result = 0; for i = 1, 2 do result = f(1.5) + 0; f = function() return 77 end end; return result",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(program
            .main
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::FastCall));
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&program).unwrap().as_number(), Some(77.0));

        let nested = compile(
            &parse_source(
                "local f = math.floor; local function call(x) return f(x) + 0 end; return call(1.5)",
            )
            .unwrap(),
        )
        .unwrap();
        assert!(nested.functions.iter().any(|function| function
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::FastCall)));

        let ordinary =
            compile(&parse_source("local function c() return 1 end; return c() + 0").unwrap())
                .unwrap();
        assert!(ordinary
            .main
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::Call));
        assert!(ordinary
            .main
            .code
            .iter()
            .all(|instruction| instruction.op != Opcode::FastCall));

        let shadowed_parameter = compile(
            &parse_source(
                "local f = math.floor; local function use(f) return f() + 0 end; return use(function() return 1 end)",
            )
            .unwrap(),
        )
        .unwrap();
        assert!(shadowed_parameter
            .functions
            .iter()
            .flat_map(|function| &function.code)
            .all(|instruction| instruction.op != Opcode::FastCall));

        let child_only = compile(
            &parse_source(
                "local function seed() local g = math.floor end; local function g() return 1 end; return g() + 0",
            )
            .unwrap(),
        )
        .unwrap();
        assert!(child_only
            .main
            .code
            .iter()
            .chain(
                child_only
                    .functions
                    .iter()
                    .flat_map(|function| &function.code)
            )
            .all(|instruction| instruction.op != Opcode::FastCall));
    }

    #[test]
    fn fastcall_falls_back_to_callable_table_metamethods() {
        let program = compile(
            &parse_source(
                "local f = math.floor; f = setmetatable({}, {__call = function(_, x) return x + 40 end}); return f(2) + 0",
            )
            .unwrap(),
        )
        .unwrap();
        assert!(program
            .main
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::FastCall));
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&program).unwrap().as_number(), Some(42.0));
    }

    #[test]
    fn fastcall_cold_and_hot_hits_preserve_native_variable_work_fuel() {
        let fast = compile(
            &parse_source(
                r#"
                    local concat = table.concat
                    local values = {"abcdefgh", "ijklmnop"}
                    local result = ""
                    for i = 1, 2 do
                        result = (concat(values))
                    end
                    return result
                "#,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            fast.main
                .code
                .iter()
                .filter(|instruction| instruction.op == Opcode::FastCall)
                .count(),
            1
        );

        let mut ordinary = fast.clone();
        for instruction in &mut ordinary.main.code {
            if instruction.op == Opcode::FastCall {
                instruction.op = Opcode::Call;
            }
        }

        let minimum_fuel = |program: &Program| {
            let succeeds = |limit| {
                let settings = VmSettings {
                    initial_fuel: Some(limit),
                    ..VmSettings::default()
                };
                match Vm::with_settings(Profile::ReproducibleStandard, settings).run(program) {
                    Ok(_) => true,
                    Err(error) if matches!(error.kind, ErrorKind::FuelExhausted { .. }) => false,
                    Err(error) => panic!("unexpected runtime error: {error}"),
                }
            };
            let mut low = 0_u64;
            let mut high = 1_000_u64;
            assert!(succeeds(high), "fuel search upper bound is too small");
            while low < high {
                let middle = low + (high - low) / 2;
                if succeeds(middle) {
                    high = middle;
                } else {
                    low = middle + 1;
                }
            }
            low
        };
        let fast_fuel = minimum_fuel(&fast);
        let ordinary_fuel = minimum_fuel(&ordinary);
        assert_eq!(fast_fuel, ordinary_fuel);
        assert!(fast_fuel > 0);

        let settings = VmSettings {
            initial_fuel: Some(fast_fuel - 1),
            ..VmSettings::default()
        };
        let error = Vm::with_settings(Profile::ReproducibleStandard, settings)
            .run(&fast)
            .expect_err("one unit below the exact CALL fuel must also starve FASTCALL");
        assert!(matches!(error.kind, ErrorKind::FuelExhausted { .. }));
    }

    #[test]
    fn fastcall_preserves_yield_and_write_first_continuation() {
        let chunk = parse_source(
            r#"
                local yield_alias = coroutine.yield
                local function worker()
                    local resumed = yield_alias(5) + 0
                    return resumed
                end
                local thread = coroutine.create(worker)
                local ok, first = coroutine.resume(thread)
                local ok2, second = coroutine.resume(thread, 9)
                assert(ok and first == 5 and ok2)
                return second
            "#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(program.functions.iter().any(|function| function
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::FastCall)));
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&program).unwrap().as_number(), Some(9.0));
    }

    #[test]
    fn getimport_and_fastcall_observe_host_time_builtin_replacement() {
        let program =
            compile(&parse_source("local f = math.floor; return f(1.5)").unwrap()).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let replacement = vm
            .register_host_function(HostFunctionOp::new(
                "replacement_floor",
                1,
                OpCapability::Pure,
                Arc::new(|_, _| Ok(vec![rustia_vm::Value::number(77.0)])),
            ))
            .unwrap();
        let math = vm.get_global_value("math").unwrap();
        let floor = vm.string("floor");
        vm.raw_table_set_value(math, floor, replacement).unwrap();
        assert_eq!(vm.run(&program).unwrap().as_number(), Some(77.0));
    }

    #[test]
    fn invalid_specialized_arities_compile_and_fail_through_native_fallback() {
        for source in [
            "return math.floor()",
            "return math.fmod(1)",
            "return math.log()",
            "return math.min()",
            "return math.clamp(1, 2)",
        ] {
            let program = compile(&parse_source(source).unwrap())
                .unwrap_or_else(|error| panic!("{source} should compile: {error}"));
            assert!(program.main.code.iter().any(|instruction| matches!(
                instruction.op,
                Opcode::FastCall | Opcode::ReturnCall
            )));
            let error = Vm::new(Profile::ReproducibleStandard)
                .run(&program)
                .expect_err("invalid native arity should fail at runtime");
            assert!(matches!(
                error.kind,
                ErrorKind::TypeError { .. } | ErrorKind::RuntimeError { .. }
            ));
        }
    }

    fn conformance_options() -> CompileOptions {
        CompileOptions {
            builtins: BuiltinEnvironment::LuauConformance,
            ..CompileOptions::default()
        }
    }

    fn test_loadstring_compiler(
        source: &[u8],
        source_name: Option<&[u8]>,
        _profile: Profile,
    ) -> std::result::Result<Program, String> {
        let source_name = source_name.map_or_else(
            || {
                let mut preview = String::new();
                for byte in source.iter().copied().take(60) {
                    match byte {
                        b'\\' => preview.push_str("\\\\"),
                        b'"' => preview.push_str("\\\""),
                        b'\n' => break,
                        0x20..=0x7e => preview.push(byte as char),
                        _ => preview.push('?'),
                    }
                }
                if source.len() > 60 {
                    preview.push_str("...");
                }
                format!("[string \"{preview}\"]")
            },
            |name| String::from_utf8_lossy(name).into_owned(),
        );
        let chunk =
            parse_source_bytes(source).map_err(|error| format!("{source_name}:1: {error}"))?;
        compile_with_options(
            &chunk,
            conformance_options().with_source_bytes(source_name.clone(), source),
        )
        .map_err(|error| format!("{source_name}:1: {error}"))
    }

    fn assert_assertion_message(kind: ErrorKind, expected: &str) {
        let ErrorKind::AssertionFailed { message } = kind else {
            panic!("expected assertion failure");
        };
        assert_eq!(message, expected);
    }
}
