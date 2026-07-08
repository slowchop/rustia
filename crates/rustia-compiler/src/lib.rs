#![forbid(unsafe_code)]

use rustia_lexer::Span;
use rustia_parser::parse_source_bytes;
use rustia_parser::{
    AssignmentOp, AssignmentTarget, BinaryOp, Chunk, Expr, FunctionParam, GenericForBinding,
    InterpolatedStringPart, Stmt, TableField, UnaryOp,
};
use rustia_vm::bytecode::{
    Constant, Function, Instruction, InstructionSource, Opcode, Program, RuntimeType,
    UpvalueDescriptor, ARRAY_ITERATOR_SKIP_VISIBLE_KEY, ARRAY_ITERATOR_WRITE_VISIBLE_KEY,
    ASSERT_MESSAGE_MISSING_ARGUMENT, ASSERT_MESSAGE_NONE,
};
use rustia_vm::number::{binary, NumericOp};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

const MAX_TABLE_ARRAY_CAPACITY_HINT: usize = 4096;
const MAX_TABLE_HASH_CAPACITY_HINT: usize = 4096;

#[derive(Clone, Copy, Default)]
struct TableCapacityHint {
    array: usize,
    hash: usize,
}

impl TableCapacityHint {
    const fn is_empty(self) -> bool {
        self.array == 0 && self.hash == 0
    }
}

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

pub fn compile(chunk: &Chunk) -> Result<Program> {
    compile_with_options(chunk, CompileOptions::default())
}

pub fn compile_with_options(chunk: &Chunk, options: CompileOptions) -> Result<Program> {
    Compiler::new(options).compile_chunk(chunk)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompileOptions {
    pub builtins: BuiltinEnvironment,
    pub source_name: Option<String>,
    pub source_line_starts: Vec<usize>,
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
    assigned_stdlib_fields: BTreeSet<(String, String)>,
    stdlib_tables_may_be_mutated: bool,
    global_environment_escaped: bool,
    disable_builtin_fastpaths: bool,
    stable_global_builtins: BTreeSet<String>,
    math_builtin_aliases: BTreeMap<u8, MathBuiltinAlias>,
    math_upvalue_aliases: BTreeMap<u8, MathBuiltinAlias>,
    stdlib_builtin_aliases: BTreeMap<u8, StdlibBuiltinAlias>,
    stdlib_upvalue_aliases: BTreeMap<u8, StdlibBuiltinAlias>,
    numeric_registers: BTreeSet<u8>,
    number_constant_locals: BTreeMap<String, f64>,
    fresh_table_registers: BTreeSet<u8>,
    table_reset_capacity_hints: BTreeMap<u8, TableCapacityHint>,
    assigned_local_names: BTreeSet<String>,
    upvalues: Vec<UpvalueDescriptor>,
    captured_registers: BTreeSet<u8>,
    capture_stack: Vec<CaptureScope>,
    source_map: Vec<InstructionSource>,
    current_function_is_vararg: bool,
    next_register: u16,
    max_registers: u16,
    loop_breaks: Vec<Vec<usize>>,
    loop_continues: Vec<Vec<usize>>,
    loop_scope_depths: Vec<usize>,
    scope_register_starts: Vec<u16>,
}

impl Compiler {
    fn new(options: CompileOptions) -> Self {
        Self {
            options,
            constants: Vec::new(),
            functions: Vec::new(),
            code: Vec::new(),
            locals: BTreeMap::new(),
            assigned_globals: BTreeSet::new(),
            assigned_stdlib_fields: BTreeSet::new(),
            stdlib_tables_may_be_mutated: false,
            global_environment_escaped: false,
            disable_builtin_fastpaths: false,
            stable_global_builtins: BTreeSet::new(),
            math_builtin_aliases: BTreeMap::new(),
            math_upvalue_aliases: BTreeMap::new(),
            stdlib_builtin_aliases: BTreeMap::new(),
            stdlib_upvalue_aliases: BTreeMap::new(),
            numeric_registers: BTreeSet::new(),
            number_constant_locals: BTreeMap::new(),
            fresh_table_registers: BTreeSet::new(),
            table_reset_capacity_hints: BTreeMap::new(),
            assigned_local_names: BTreeSet::new(),
            upvalues: Vec::new(),
            captured_registers: BTreeSet::new(),
            capture_stack: Vec::new(),
            source_map: Vec::new(),
            current_function_is_vararg: true,
            next_register: 0,
            max_registers: 0,
            loop_breaks: Vec::new(),
            loop_continues: Vec::new(),
            loop_scope_depths: Vec::new(),
            scope_register_starts: Vec::new(),
        }
    }

    fn compile_chunk(mut self, chunk: &Chunk) -> Result<Program> {
        let global_mutations = global_mutation_summary_in_statements(&chunk.statements);
        self.assigned_globals = global_mutations.assigned_globals.clone();
        self.assigned_stdlib_fields = global_mutations.assigned_stdlib_fields.clone();
        self.stdlib_tables_may_be_mutated = global_mutations.stdlib_tables_may_be_mutated;
        self.global_environment_escaped = global_mutations.environment_escaped;
        if !global_mutations.environment_escaped {
            for name in ["ipairs", "math", "next", "pairs", "string", "table"] {
                if !global_mutations.assigned_globals.contains(name) {
                    self.stable_global_builtins.insert(name.to_owned());
                }
            }
        }
        self.assigned_local_names = assigned_local_names_in_statements(&chunk.statements);
        self.compile_block(&chunk.statements)?;
        self.code.push(Instruction::new(Opcode::Halt, 0, 0, 0));
        let max_registers = usize::from(self.max_registers.max(1));
        let main_source_map = self.source_map;
        Ok(Program::with_functions(
            Function::with_params_and_vararg(self.code, max_registers, 0, true)
                .with_source_map(main_source_map),
            self.constants,
            self.functions,
        )
        .with_source_name(self.options.source_name))
    }

    fn compile_block(&mut self, statements: &[Stmt]) -> Result<()> {
        for (index, statement) in statements.iter().enumerate() {
            let number_constants = self.number_constant_hint_environment();
            let table_capacity_hints = match statement {
                Stmt::Local { bindings, .. } => local_table_array_capacity_hints(
                    bindings,
                    &statements[index + 1..],
                    &number_constants,
                ),
                _ => BTreeMap::new(),
            };
            self.compile_statement_with_table_hints(statement, &table_capacity_hints)?;
        }
        Ok(())
    }

    fn number_constant_hint_environment(&self) -> BTreeMap<String, f64> {
        let mut constants = BTreeMap::new();
        for scope in &self.capture_stack {
            for name in scope.locals.keys() {
                constants.remove(name);
            }
            for (name, value) in &scope.number_constant_locals {
                if !scope.assigned_local_names.contains(name) {
                    constants.insert(name.clone(), *value);
                }
            }
        }
        for name in self.locals.keys() {
            constants.remove(name);
        }
        for (name, value) in &self.number_constant_locals {
            if !self.assigned_local_names.contains(name) {
                constants.insert(name.clone(), *value);
            }
        }
        for name in &self.assigned_local_names {
            constants.remove(name);
        }
        constants
    }

    fn compile_scoped_block(&mut self, statements: &[Stmt]) -> Result<()> {
        let locals = self.locals.clone();
        let math_builtin_aliases = self.math_builtin_aliases.clone();
        let stdlib_builtin_aliases = self.stdlib_builtin_aliases.clone();
        let numeric_registers = self.numeric_registers.clone();
        let number_constant_locals = self.number_constant_locals.clone();
        let fresh_table_registers = self.fresh_table_registers.clone();
        let table_reset_capacity_hints = self.table_reset_capacity_hints.clone();
        let next_register = self.next_register;
        self.scope_register_starts.push(next_register);
        self.compile_block(statements)?;
        self.clear_local_cells_from(next_register)?;
        self.scope_register_starts
            .pop()
            .expect("scope register stack should be balanced");
        self.locals = locals;
        self.math_builtin_aliases =
            restore_scoped_aliases(&self.math_builtin_aliases, math_builtin_aliases);
        self.stdlib_builtin_aliases =
            restore_scoped_aliases(&self.stdlib_builtin_aliases, stdlib_builtin_aliases);
        self.numeric_registers = restore_scoped_set(&self.numeric_registers, numeric_registers);
        self.number_constant_locals =
            restore_scoped_map(&self.number_constant_locals, number_constant_locals);
        self.fresh_table_registers =
            restore_scoped_set(&self.fresh_table_registers, fresh_table_registers);
        self.table_reset_capacity_hints = table_reset_capacity_hints;
        self.next_register = next_register;
        Ok(())
    }

    fn compile_statement_with_table_hints(
        &mut self,
        statement: &Stmt,
        table_capacity_hints: &BTreeMap<String, TableCapacityHint>,
    ) -> Result<()> {
        self.record_source_for_next_instruction(statement_span(statement));
        match statement {
            Stmt::Local {
                bindings,
                init,
                span,
            } => self.compile_local(bindings, init, *span, table_capacity_hints),
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
            Stmt::TypeAlias { .. } => Ok(()),
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
            self.code.push(Instruction::new(Opcode::Return, 0, 0, 0));
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
            self.compile_return_expand(last_expr, prefix_first, prefix_count, span)?;
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
        self.code.push(Instruction::new(
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
        table_capacity_hints: &BTreeMap<String, TableCapacityHint>,
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
            let result_count = registers.len().saturating_sub(last_index);
            if result_count > 1
                && init.len() <= registers.len()
                && self.expr_can_expand(&init[last_index])
            {
                for (index, expr) in init[..last_index].iter().enumerate() {
                    self.compile_expr_to(expr, registers[index])?;
                }
                let expanded = self.compile_expand_multi_to(
                    &init[last_index],
                    registers[last_index],
                    result_count,
                    span,
                )?;
                debug_assert!(expanded);
                handled_init = true;
            }
        }

        if !handled_init {
            for (index, register) in registers.iter().copied().enumerate() {
                if let Some(expr) = init.get(index) {
                    let hint = table_capacity_hints
                        .get(&bindings[index].name)
                        .copied()
                        .unwrap_or_default();
                    match expr_without_parens(expr) {
                        Expr::Table { fields, .. } if fields.is_empty() && !hint.is_empty() => {
                            self.compile_table_to_with_capacity(fields, register, hint)?
                        }
                        _ => self.compile_expr_to(expr, register)?,
                    }
                } else {
                    self.load_constant_to(register, Constant::Nil)?;
                }
            }
            for expr in init.iter().skip(registers.len()) {
                let register = self.compile_expr(expr)?;
                self.free_register(register);
            }
        }
        for (index, (binding, register)) in bindings.iter().zip(registers).enumerate() {
            self.locals
                .insert(binding.name.clone(), Binding::Register(register));
            if !handled_init
                && init
                    .get(index)
                    .is_some_and(|expr| self.expr_is_known_number(expr))
            {
                self.numeric_registers.insert(register);
            } else {
                self.numeric_registers.remove(&register);
            }
            if !handled_init && !self.assigned_local_names.contains(&binding.name) {
                if let Some(number) = init.get(index).and_then(constant_number) {
                    self.number_constant_locals
                        .insert(binding.name.clone(), number);
                } else {
                    self.number_constant_locals.remove(&binding.name);
                }
            } else {
                self.number_constant_locals.remove(&binding.name);
            }
            if !handled_init
                && init
                    .get(index)
                    .is_some_and(|expr| matches!(expr_without_parens(expr), Expr::Table { .. }))
            {
                self.fresh_table_registers.insert(register);
            } else {
                self.fresh_table_registers.remove(&register);
            }
            if !handled_init {
                if let Some(alias) = init
                    .get(index)
                    .and_then(|expr| self.math_builtin_alias_for_expr(expr))
                {
                    self.math_builtin_aliases.insert(register, alias);
                } else if let Some(alias) = init
                    .get(index)
                    .and_then(|expr| self.stdlib_builtin_alias_for_expr(expr))
                {
                    self.stdlib_builtin_aliases.insert(register, alias);
                }
            }
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
        self.locals
            .insert(name.to_owned(), Binding::Register(register));
        self.math_builtin_aliases.remove(&register);
        self.stdlib_builtin_aliases.remove(&register);
        self.numeric_registers.remove(&register);
        self.fresh_table_registers.remove(&register);
        let prototype = self.compile_function_prototype(
            params,
            is_vararg,
            body,
            Some(name),
            Some(name.to_owned()),
            span,
        )?;
        self.code.push(Instruction::with_aux(
            Opcode::LoadFunction,
            register,
            0,
            0,
            i32::try_from(prototype)
                .map_err(|_| CompileError::at("too many function prototypes", span))?,
        ));
        if self.local_register_is_captured(register) {
            self.code.push(Instruction::new(
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
            self.number_constant_locals.remove(name);
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

        let target = self.compile_assignment_target(target)?;
        match target {
            CompiledAssignmentTarget::Local(register) => {
                self.code.push(Instruction::with_aux(
                    Opcode::LoadFunction,
                    register,
                    0,
                    0,
                    prototype,
                ));
            }
            CompiledAssignmentTarget::CapturedLocal(register) => {
                self.code.push(Instruction::with_aux(
                    Opcode::LoadFunction,
                    register,
                    0,
                    0,
                    prototype,
                ));
                self.code.push(Instruction::new(
                    Opcode::SetLocalCell,
                    register,
                    register,
                    0,
                ));
            }
            CompiledAssignmentTarget::Upvalue(index) => {
                let function_register = self.alloc_register_at(span)?;
                self.code.push(Instruction::with_aux(
                    Opcode::LoadFunction,
                    function_register,
                    0,
                    0,
                    prototype,
                ));
                self.code.push(Instruction::with_aux(
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
                may_mutate_stdlib_table,
                ..
            } => {
                let function_register = self.alloc_register_at(span)?;
                self.code.push(Instruction::with_aux(
                    Opcode::LoadFunction,
                    function_register,
                    0,
                    0,
                    prototype,
                ));
                self.emit_source_mapped_instruction(
                    Instruction::new(Opcode::SetIndex, table, key, function_register),
                    span,
                );
                self.mark_table_assignment_mutation(
                    global_field,
                    stdlib_field,
                    may_mutate_stdlib_table,
                );
                self.free_register(function_register);
                self.free_register(key);
                self.free_register(table);
            }
            CompiledAssignmentTarget::Global { key, name } => {
                let function_register = self.alloc_register_at(span)?;
                self.code.push(Instruction::with_aux(
                    Opcode::LoadFunction,
                    function_register,
                    0,
                    0,
                    prototype,
                ));
                self.code.push(Instruction::with_aux(
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
        if targets.is_empty() {
            return Err(CompileError::at("assignment has no targets", span));
        }
        self.invalidate_number_constant_assignment_targets(targets);

        if op == AssignmentOp::Assign
            && targets.len() == 1
            && values.len() == 1
            && self.compile_local_self_increment_assignment(&targets[0], &values[0])?
        {
            return Ok(());
        }

        if op == AssignmentOp::Assign
            && targets.len() == 1
            && values.len() == 1
            && self.compile_upvalue_self_increment_assignment(&targets[0], &values[0])?
        {
            return Ok(());
        }

        if op == AssignmentOp::Assign
            && targets.len() == 1
            && values.len() == 1
            && self.compile_upvalue_self_add_upvalue_assignment(&targets[0], &values[0])?
        {
            return Ok(());
        }

        if op == AssignmentOp::Assign
            && targets.len() == 1
            && values.len() == 1
            && self.compile_local_self_binary_assignment(&targets[0], &values[0])?
        {
            return Ok(());
        }

        if op == AssignmentOp::Assign
            && targets.len() == 1
            && values.len() == 1
            && self.compile_simple_table_assignment(&targets[0], &values[0])?
        {
            return Ok(());
        }

        if op != AssignmentOp::Assign {
            if targets.len() != 1 || values.len() != 1 {
                return Err(CompileError::at(
                    "compound assignment requires one target and one value",
                    span,
                ));
            }
            let target = self.compile_assignment_target(&targets[0])?;
            let result = self.alloc_register_at(span)?;
            match &target {
                CompiledAssignmentTarget::Local(register) => {
                    self.code
                        .push(Instruction::new(Opcode::Move, result, *register, 0));
                }
                CompiledAssignmentTarget::CapturedLocal(register) => {
                    self.code
                        .push(Instruction::new(Opcode::GetLocalCell, result, *register, 0));
                }
                CompiledAssignmentTarget::Upvalue(index) => {
                    self.code.push(Instruction::with_aux(
                        Opcode::GetUpvalue,
                        result,
                        0,
                        0,
                        i32::from(*index),
                    ));
                }
                CompiledAssignmentTarget::Table { table, key, .. } => {
                    let opcode = table_read_opcode_for_assignment_target(&target);
                    self.code
                        .push(Instruction::new(opcode, result, *table, *key));
                }
                CompiledAssignmentTarget::Global { key, .. } => {
                    self.code
                        .push(Instruction::with_aux(Opcode::GetGlobal, result, 0, 0, *key));
                }
            }
            let rhs_register = self.compile_expr(&values[0])?;
            self.code.push(Instruction::new(
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
                        self.code.push(Instruction::new(
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
                    self.code.push(Instruction::with_aux(
                        Opcode::SetUpvalue,
                        result,
                        0,
                        0,
                        i32::from(*index),
                    ));
                }
                CompiledAssignmentTarget::Table { table, key, .. } => {
                    let opcode = table_write_opcode_for_assignment_target(&target);
                    self.code
                        .push(Instruction::new(opcode, *table, *key, result));
                }
                CompiledAssignmentTarget::Global { key, .. } => {
                    self.code
                        .push(Instruction::with_aux(Opcode::SetGlobal, result, 0, 0, *key));
                }
            }
            self.mark_assignment_target_mutations(&target);
            self.free_register(rhs_register);
            self.free_register(result);
            match target {
                CompiledAssignmentTarget::Local(_) => {}
                CompiledAssignmentTarget::CapturedLocal(_) => {}
                CompiledAssignmentTarget::Upvalue(_) => {}
                CompiledAssignmentTarget::Table { table, key, .. } => {
                    self.free_register(key);
                    self.free_register(table);
                }
                CompiledAssignmentTarget::Global { .. } => {}
            }
            return Ok(());
        }

        let mut compiled_targets = Vec::with_capacity(targets.len());
        for target in targets {
            compiled_targets.push(self.compile_assignment_target(target)?);
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
                let expanded = self.compile_expand_multi_to(
                    &values[last_index],
                    first_result,
                    result_count,
                    span,
                )?;
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

        let table_capacity_hint = if compiled_targets.len() == 1 && values.len() == 1 {
            self.assignment_empty_table_capacity_hint(&compiled_targets[0], &values[0])
        } else {
            TableCapacityHint::default()
        };
        let mut value_registers = Vec::with_capacity(values.len());
        for (index, value) in values.iter().enumerate() {
            let hint = if index == 0 {
                table_capacity_hint
            } else {
                TableCapacityHint::default()
            };
            value_registers.push(self.compile_expr_with_table_capacity_hint(value, hint)?);
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

    fn assignment_empty_table_capacity_hint(
        &self,
        target: &CompiledAssignmentTarget,
        value: &Expr,
    ) -> TableCapacityHint {
        if !matches!(expr_without_parens(value), Expr::Table { fields, .. } if fields.is_empty()) {
            return TableCapacityHint::default();
        }
        match target {
            CompiledAssignmentTarget::Local(register)
            | CompiledAssignmentTarget::CapturedLocal(register) => self
                .table_reset_capacity_hints
                .get(register)
                .copied()
                .unwrap_or_default(),
            _ => TableCapacityHint::default(),
        }
    }

    fn invalidate_number_constant_assignment_targets(&mut self, targets: &[AssignmentTarget]) {
        for target in targets {
            if let AssignmentTarget::Local { name, .. } = target {
                self.number_constant_locals.remove(name);
            }
        }
    }

    fn compile_local_self_increment_assignment(
        &mut self,
        target: &AssignmentTarget,
        value: &Expr,
    ) -> Result<bool> {
        let AssignmentTarget::Local { name, .. } = target else {
            return Ok(false);
        };
        if local_self_increment_rhs(name, value).is_none() {
            return Ok(false);
        };
        let Some(Binding::Register(register)) = self.locals.get(name).copied() else {
            return Ok(false);
        };
        if self.local_register_is_captured(register) {
            return Ok(false);
        }
        self.math_builtin_aliases.remove(&register);
        self.stdlib_builtin_aliases.remove(&register);
        self.fresh_table_registers.remove(&register);
        let constant = self.number_constant_operand(1.0)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(Opcode::AddConst, register, register, 0, constant),
            value.span(),
        );
        Ok(true)
    }

    fn compile_upvalue_self_increment_assignment(
        &mut self,
        target: &AssignmentTarget,
        value: &Expr,
    ) -> Result<bool> {
        let AssignmentTarget::Local { name, .. } = target else {
            return Ok(false);
        };
        if local_self_increment_rhs(name, value).is_none() {
            return Ok(false);
        };
        let index = match self.locals.get(name).copied() {
            Some(Binding::Upvalue(index)) => Some(index),
            Some(Binding::Register(_) | Binding::CapturedLocal(_) | Binding::CurrentFunction) => {
                None
            }
            None => self.resolve_upvalue(name)?,
        };
        let Some(index) = index else {
            return Ok(false);
        };

        self.math_upvalue_aliases.remove(&index);
        self.stdlib_upvalue_aliases.remove(&index);
        let register = self.alloc_register_at(value.span())?;
        let constant = self.number_constant_operand(1.0)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(Opcode::AddUpvalueConst, register, index, 0, constant),
            value.span(),
        );
        self.free_register_without_clear(register);
        Ok(true)
    }

    fn compile_upvalue_self_add_upvalue_assignment(
        &mut self,
        target: &AssignmentTarget,
        value: &Expr,
    ) -> Result<bool> {
        let AssignmentTarget::Local { name, .. } = target else {
            return Ok(false);
        };
        let Some(rhs_name) = upvalue_self_add_rhs_name(name, value) else {
            return Ok(false);
        };

        let target_index = match self.locals.get(name).copied() {
            Some(Binding::Upvalue(index)) => Some(index),
            Some(Binding::Register(_) | Binding::CapturedLocal(_) | Binding::CurrentFunction) => {
                None
            }
            None => self.resolve_upvalue(name)?,
        };
        let rhs_index = match self.locals.get(rhs_name).copied() {
            Some(Binding::Upvalue(index)) => Some(index),
            Some(Binding::Register(_) | Binding::CapturedLocal(_) | Binding::CurrentFunction) => {
                None
            }
            None => self.resolve_upvalue(rhs_name)?,
        };
        let (Some(target_index), Some(rhs_index)) = (target_index, rhs_index) else {
            return Ok(false);
        };

        self.math_upvalue_aliases.remove(&target_index);
        self.stdlib_upvalue_aliases.remove(&target_index);
        let register = self.alloc_register_at(value.span())?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::AddUpvalue,
                register,
                rhs_index,
                0,
                i32::from(target_index),
            ),
            value.span(),
        );
        self.free_register_without_clear(register);
        Ok(true)
    }

    fn compile_local_self_binary_assignment(
        &mut self,
        target: &AssignmentTarget,
        value: &Expr,
    ) -> Result<bool> {
        let AssignmentTarget::Local { name, .. } = target else {
            return Ok(false);
        };
        let Some(rhs_operands) = local_self_binary_rhs_operands(name, value) else {
            return Ok(false);
        };
        let Some(Binding::Register(register)) = self.locals.get(name).copied() else {
            return Ok(false);
        };
        if self.local_register_is_captured(register) {
            return Ok(false);
        }

        self.math_builtin_aliases.remove(&register);
        self.stdlib_builtin_aliases.remove(&register);
        self.numeric_registers.remove(&register);
        self.fresh_table_registers.remove(&register);
        for rhs in rhs_operands {
            let rhs_register = self.compile_expr(rhs)?;
            self.emit_source_mapped_instruction(
                Instruction::new(Opcode::Add, register, register, rhs_register),
                value.span(),
            );
            if self.local_register_is_captured(register) {
                self.code.push(Instruction::new(
                    Opcode::SetLocalCell,
                    register,
                    register,
                    0,
                ));
            }
            self.free_register_without_clear(rhs_register);
        }
        Ok(true)
    }

    fn compile_simple_table_assignment(
        &mut self,
        target: &AssignmentTarget,
        value: &Expr,
    ) -> Result<bool> {
        if !self.expr_has_no_effectful_evaluation(value) {
            return Ok(false);
        }

        let (object, table_register, key, global_field, stdlib_field, may_mutate_stdlib_table) =
            match target {
                AssignmentTarget::Field { object, field, .. } => {
                    let Some(table_register) = self.simple_register_expr(object) else {
                        return Ok(false);
                    };
                    (
                        object.as_ref(),
                        table_register,
                        SimpleAssignmentKey::Constant(Constant::String(field.as_bytes().to_vec())),
                        self.global_table_field_assignment(object, field),
                        self.stdlib_table_field_assignment(object, field),
                        self.object_may_mutate_stdlib_table(object),
                    )
                }
                AssignmentTarget::Index { object, index, .. } => {
                    let Some(table_register) = self.simple_register_expr(object) else {
                        return Ok(false);
                    };
                    if let Some((constant, suffix)) = lhs_const_concat_key(index) {
                        if !self.expr_has_no_effectful_evaluation(suffix) {
                            return Ok(false);
                        }
                        if !self.expr_has_no_effectful_evaluation(object) {
                            return Ok(false);
                        }
                        let suffix_register = self.compile_expr(suffix)?;
                        let value_is_scalar_temporary = self.expr_is_known_scalar_temp(value);
                        let value_register = self.compile_expr(value)?;
                        self.mark_table_assignment_mutation(
                            self.global_table_index_assignment(object, index),
                            self.stdlib_table_index_assignment(object, index),
                            self.object_may_mutate_stdlib_table(object),
                        );
                        let constant = i32::try_from(self.constant_index(constant)?)
                            .map_err(|_| CompileError::new("constant pool overflow"))?;
                        self.emit_source_mapped_instruction(
                            Instruction::with_aux(
                                Opcode::SetIndexConcatConst,
                                table_register,
                                suffix_register,
                                value_register,
                                constant,
                            ),
                            value.span(),
                        );
                        if value_is_scalar_temporary {
                            self.free_register_without_clear(value_register);
                        } else {
                            self.free_register(value_register);
                        }
                        self.free_register(suffix_register);
                        return Ok(true);
                    }
                    let key = if let Some(register) = self.simple_register_expr(index) {
                        SimpleAssignmentKey::Register(register)
                    } else if let Some(constant) = const_eval(index) {
                        SimpleAssignmentKey::Constant(constant)
                    } else if self.expr_has_no_effectful_evaluation(index) {
                        SimpleAssignmentKey::Expression(index)
                    } else {
                        return Ok(false);
                    };
                    (
                        object.as_ref(),
                        table_register,
                        key,
                        self.global_table_index_assignment(object, index),
                        self.stdlib_table_index_assignment(object, index),
                        self.object_may_mutate_stdlib_table(object),
                    )
                }
                AssignmentTarget::Local { .. } => return Ok(false),
            };

        if !self.expr_has_no_effectful_evaluation(object) {
            return Ok(false);
        }

        if let SimpleAssignmentKey::Expression(key_expr) = key {
            if let Some((factor_expr, multiplier, modulus)) =
                self.mul_mod_number_const_expr(key_expr)
            {
                if let Some(factor_register) = self.simple_register_expr(factor_expr) {
                    if let Some(value_register) = self.simple_register_expr(value) {
                        if let Some(aux) = self.number_constant_pair_operand(multiplier, modulus)? {
                            self.mark_table_assignment_mutation(
                                global_field,
                                stdlib_field,
                                may_mutate_stdlib_table,
                            );
                            self.emit_source_mapped_instruction(
                                Instruction::with_aux(
                                    Opcode::SetIndexMulModConst,
                                    table_register,
                                    factor_register,
                                    value_register,
                                    aux,
                                ),
                                value.span(),
                            );
                            return Ok(true);
                        }
                    }
                }
            }
        }

        if let SimpleAssignmentKey::Register(key_register) = key {
            if let Some((factor_expr, addend_expr, multiplier, modulus)) =
                self.mul_add_mod_number_const_expr(value)
            {
                if self.numeric_registers.contains(&key_register)
                    && self.simple_register_expr(factor_expr) == Some(key_register)
                {
                    if let Some(addend_register) = self.simple_register_expr(addend_expr) {
                        if let Some(aux) = self.number_constant_pair_operand(multiplier, modulus)? {
                            self.mark_table_assignment_mutation(
                                global_field,
                                stdlib_field,
                                may_mutate_stdlib_table,
                            );
                            self.emit_source_mapped_instruction(
                                Instruction::with_aux(
                                    Opcode::SetIndexMulAddModConst,
                                    table_register,
                                    key_register,
                                    addend_register,
                                    aux,
                                ),
                                value.span(),
                            );
                            return Ok(true);
                        }
                    }
                }
            }
        }

        if let SimpleAssignmentKey::Constant(Constant::String(bytes)) = &key {
            self.mark_table_assignment_mutation(
                global_field,
                stdlib_field,
                may_mutate_stdlib_table,
            );
            let value_is_scalar_temporary = self.expr_is_known_scalar_temp(value);
            let value_register = self.compile_expr(value)?;
            let key = self.string_constant_operand(bytes)?;
            self.emit_source_mapped_instruction(
                Instruction::with_aux(
                    Opcode::SetIndexStringConst,
                    table_register,
                    value_register,
                    0,
                    key,
                ),
                value.span(),
            );
            if value_is_scalar_temporary {
                self.free_register_without_clear(value_register);
            } else {
                self.free_register(value_register);
            }
            return Ok(true);
        }

        if let SimpleAssignmentKey::Constant(Constant::Number(index)) = &key {
            if index.is_finite()
                && index.fract() == 0.0
                && (1.0..=f64::from(i32::MAX)).contains(index)
            {
                self.mark_table_assignment_mutation(
                    global_field,
                    stdlib_field,
                    may_mutate_stdlib_table,
                );
                let value_is_scalar_temporary = self.expr_is_known_scalar_temp(value);
                let value_register = self.compile_expr(value)?;
                self.emit_source_mapped_instruction(
                    Instruction::with_aux(
                        Opcode::SetIndexNumberConst,
                        table_register,
                        value_register,
                        0,
                        *index as i32,
                    ),
                    value.span(),
                );
                if value_is_scalar_temporary {
                    self.free_register_without_clear(value_register);
                } else {
                    self.free_register(value_register);
                }
                return Ok(true);
            }
        }

        let key_register;
        let key_is_temporary;
        let key_is_scalar_temporary;
        let key_is_known_number;
        match key {
            SimpleAssignmentKey::Register(register) => {
                key_register = register;
                key_is_temporary = false;
                key_is_scalar_temporary = false;
                key_is_known_number = self.numeric_registers.contains(&register);
            }
            SimpleAssignmentKey::Constant(constant) => {
                key_is_known_number = matches!(constant, Constant::Number(_));
                key_is_scalar_temporary = constant_is_scalar_temp(&constant);
                key_register = self.load_constant(constant)?;
                key_is_temporary = true;
            }
            SimpleAssignmentKey::Expression(expr) => {
                key_is_known_number = self.expr_is_known_number(expr);
                key_is_scalar_temporary = self.expr_is_known_scalar_temp(expr);
                key_register = self.compile_expr(expr)?;
                key_is_temporary = true;
            }
        }

        self.mark_table_assignment_mutation(global_field, stdlib_field, may_mutate_stdlib_table);
        let value_is_scalar_temporary = self.expr_is_known_scalar_temp(value);
        let value_register = self.compile_expr(value)?;
        let opcode = if key_is_known_number {
            Opcode::SetIndexNumber
        } else {
            Opcode::SetIndex
        };
        self.code.push(Instruction::new(
            opcode,
            table_register,
            key_register,
            value_register,
        ));
        if value_is_scalar_temporary {
            self.free_register_without_clear(value_register);
        } else {
            self.free_register(value_register);
        }
        if key_is_temporary {
            if key_is_scalar_temporary {
                self.free_register_without_clear(key_register);
            } else {
                self.free_register(key_register);
            }
        }
        Ok(true)
    }

    fn compile_rhs_number_const_binary_expr(
        &mut self,
        op: BinaryOp,
        lhs: &Expr,
        rhs: &Expr,
        target: u8,
        span: Span,
    ) -> Result<bool> {
        let Some(opcode) = opcode_for_rhs_number_const_binary(op) else {
            return Ok(false);
        };
        let Some(Constant::Number(rhs)) = const_eval(rhs) else {
            return Ok(false);
        };
        self.compile_expr_to(lhs, target)?;
        let constant = self.number_constant_operand(rhs)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(opcode, target, target, 0, constant),
            span,
        );
        Ok(true)
    }

    fn compile_string_byte_equal_expr(
        &mut self,
        op: BinaryOp,
        lhs: &Expr,
        rhs: &Expr,
        target: u8,
    ) -> Result<bool> {
        if !matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
            return Ok(false);
        }
        let invert = op == BinaryOp::Ne;
        if let Some(byte) = single_byte_string_literal(rhs) {
            if self.compile_string_byte_equal_call(lhs, byte, invert, target)? {
                return Ok(true);
            }
        }
        if let Some(byte) = single_byte_string_literal(lhs) {
            if self.compile_string_byte_equal_call(rhs, byte, invert, target)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn compile_string_byte_equal_call(
        &mut self,
        candidate: &Expr,
        byte: u8,
        invert: bool,
        target: u8,
    ) -> Result<bool> {
        let Expr::Call { callee, args, .. } = expr_without_parens(candidate) else {
            return Ok(false);
        };
        if args.len() != 3 || !self.callee_is_string_sub_builtin(callee)? {
            return Ok(false);
        }
        let Some(source_name) = expr_name_without_parens(&args[0]) else {
            return Ok(false);
        };
        if !self.name_is_local_or_upvalue(source_name)? {
            return Ok(false);
        }
        let Some(index_name) = expr_name_without_parens(&args[1]) else {
            return Ok(false);
        };
        if expr_name_without_parens(&args[2]) != Some(index_name)
            || !self.name_is_local_or_upvalue(index_name)?
        {
            return Ok(false);
        }
        let source_register = self.compile_expr(&args[0])?;
        let index_register = self.compile_expr(&args[1])?;
        let aux = i32::from(byte) | if invert { 0x100 } else { 0 };
        self.code.push(Instruction::with_aux(
            Opcode::StringByteEqual,
            target,
            source_register,
            index_register,
            aux,
        ));
        self.free_register(index_register);
        self.free_register(source_register);
        Ok(true)
    }

    fn callee_is_string_sub_builtin(&mut self, callee: &Expr) -> Result<bool> {
        match expr_without_parens(callee) {
            Expr::Field { object, field, .. }
                if field == "sub"
                    && matches!(object.as_ref(), Expr::Name(name, _) if name == "string")
                    && self.builtin_table_field_is_unmodified("string", "sub") =>
            {
                Ok(true)
            }
            Expr::Name(name, _) => {
                if !self.locals.contains_key(name) {
                    let _ = self.resolve_upvalue(name)?;
                }
                Ok(matches!(
                    self.local_stdlib_builtin_alias(name),
                    Some(StdlibBuiltinAlias::StringSub)
                ))
            }
            _ => Ok(false),
        }
    }

    fn compile_assignment_target(
        &mut self,
        target: &AssignmentTarget,
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
                field_span: _,
                ..
            } => {
                let global_field = self.global_table_field_assignment(object, field);
                let table = self.compile_expr(object)?;
                let key = self.load_constant(Constant::String(field.as_bytes().to_vec()))?;
                Ok(CompiledAssignmentTarget::Table {
                    table,
                    key,
                    key_is_known_number: false,
                    global_field,
                    stdlib_field: self.stdlib_table_field_assignment(object, field),
                    may_mutate_stdlib_table: self.object_may_mutate_stdlib_table(object),
                })
            }
            AssignmentTarget::Index {
                object,
                index,
                span,
            } => {
                if let Some(target) =
                    self.compile_len_plus_one_index_target(object, index, *span)?
                {
                    return Ok(target);
                }
                let global_field = self.global_table_index_assignment(object, index);
                let stdlib_field = self.stdlib_table_index_assignment(object, index);
                let key_is_known_number = self.expr_is_known_number(index);
                let table = self.compile_expr(object)?;
                let key = self.compile_expr(index)?;
                Ok(CompiledAssignmentTarget::Table {
                    table,
                    key,
                    key_is_known_number,
                    global_field,
                    stdlib_field,
                    may_mutate_stdlib_table: self.object_may_mutate_stdlib_table(object),
                })
            }
        }
    }

    fn compile_len_plus_one_index_target(
        &mut self,
        object: &Expr,
        index: &Expr,
        span: Span,
    ) -> Result<Option<CompiledAssignmentTarget>> {
        if !self.index_is_len_plus_one_of_same_name(object, index)? {
            return Ok(None);
        }
        let table = self.compile_expr(object)?;
        let key = self.alloc_register_at(span)?;
        self.code
            .push(Instruction::new(Opcode::LenPlusOne, key, table, 0));
        Ok(Some(CompiledAssignmentTarget::Table {
            table,
            key,
            key_is_known_number: true,
            global_field: None,
            stdlib_field: None,
            may_mutate_stdlib_table: self.object_may_mutate_stdlib_table(object),
        }))
    }

    fn index_is_len_plus_one_of_same_name(&mut self, object: &Expr, index: &Expr) -> Result<bool> {
        let Some(object_name) = expr_name_without_parens(object) else {
            return Ok(false);
        };
        if !self.name_is_local_or_upvalue(object_name)? {
            return Ok(false);
        }
        let Expr::Binary { op, lhs, rhs, .. } = expr_without_parens(index) else {
            return Ok(false);
        };
        if *op != BinaryOp::Add || !expr_is_one(rhs) {
            return Ok(false);
        }
        Ok(len_expr_name(lhs) == Some(object_name))
    }

    fn name_is_local_or_upvalue(&mut self, name: &str) -> Result<bool> {
        if let Some(binding) = self.locals.get(name).copied() {
            return Ok(!matches!(binding, Binding::CurrentFunction));
        }
        self.resolve_upvalue(name).map(|index| index.is_some())
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
        self.code.push(Instruction::with_aux(
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
        self.code.push(Instruction::with_aux(
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
        let next_register = self.next_register;

        let index_register = self.alloc_register_at(span)?;
        let limit_register = self.alloc_register_at(span)?;
        let step_register = if step.is_some() {
            Some(self.alloc_register_at(span)?)
        } else {
            None
        };
        let visible_register = self.alloc_register_at(span)?;

        self.compile_expr_to(start, index_register)?;
        self.compile_expr_to(limit, limit_register)?;
        if let Some((step, step_register)) = step.zip(step_register) {
            self.compile_expr_to(step, step_register)?;
        }
        self.emit_source_mapped_instruction(
            Instruction::new(Opcode::CoerceForNumber, index_register, index_register, 0),
            start.span(),
        );
        self.emit_source_mapped_instruction(
            Instruction::new(Opcode::CoerceForNumber, limit_register, limit_register, 1),
            limit.span(),
        );
        if let Some(step_register) = step_register {
            self.emit_source_mapped_instruction(
                Instruction::new(Opcode::CoerceForNumber, step_register, step_register, 2),
                step.expect("step register exists only with an explicit step")
                    .span(),
            );
        }

        self.locals
            .insert(variable.to_owned(), Binding::Register(visible_register));
        self.numeric_registers.insert(visible_register);

        let prep_jump = self.code.len();
        let prep_opcode = if step_register.is_some() {
            Opcode::ForPrep
        } else {
            Opcode::ForPrepStepOne
        };
        self.emit_source_mapped_instruction(
            Instruction::with_aux(prep_opcode, index_register, 0, 0, 0),
            span,
        );
        let body_start = self.code.len();
        self.loop_breaks.push(Vec::new());
        self.loop_continues.push(Vec::new());
        self.loop_scope_depths
            .push(self.scope_register_starts.len());
        let table_reset_capacity_hints = self.table_reset_capacity_hints.clone();
        let number_constants = self.number_constant_hint_environment();
        for (name, hint) in loop_table_reset_capacity_hints(body, &number_constants) {
            if let Some(Binding::Register(register) | Binding::CapturedLocal(register)) =
                self.locals.get(&name).copied()
            {
                self.table_reset_capacity_hints.insert(register, hint);
            }
        }
        let body_result = self.compile_scoped_block(body);
        self.table_reset_capacity_hints = table_reset_capacity_hints;
        body_result?;
        let continue_target = self.code.len();
        self.patch_loop_continues_to(continue_target, span)?;
        let body_start = self.jump_target(body_start, span)?;
        let loop_opcode = if step_register.is_some() {
            Opcode::ForLoop
        } else {
            Opcode::ForLoopStepOne
        };
        self.emit_source_mapped_instruction(
            Instruction::with_aux(loop_opcode, index_register, 0, 0, body_start),
            span,
        );

        let clear_start = self.code.len();
        self.clear_local_cells_from(next_register)?;
        self.patch_jump_to(prep_jump, clear_start, span)?;
        self.patch_loop_breaks_to(clear_start, span)?;
        self.locals = locals;
        self.numeric_registers.retain(|register| {
            u16::from(*register) < next_register
                && self
                    .locals
                    .values()
                    .any(|binding| matches!(binding, Binding::Register(local) if local == register))
        });
        self.next_register = next_register;
        Ok(())
    }

    fn compile_repeat(&mut self, body: &[Stmt], condition: &Expr, span: Span) -> Result<()> {
        let locals = self.locals.clone();
        let next_register = self.next_register;
        let loop_start = self.code.len();

        self.loop_breaks.push(Vec::new());
        self.loop_continues.push(Vec::new());
        self.loop_scope_depths
            .push(self.scope_register_starts.len());
        self.compile_block(body)?;
        let condition_start = self.code.len();
        self.patch_loop_continues_to(condition_start, span)?;
        let condition_register = self.compile_expr(condition)?;
        let loop_start = self.jump_target(loop_start, span)?;
        self.code.push(Instruction::with_aux(
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

    fn compile_expr_with_table_capacity_hint(
        &mut self,
        expr: &Expr,
        capacity_hint: TableCapacityHint,
    ) -> Result<u8> {
        let target = self.alloc_register_at(expr.span())?;
        self.compile_expr_to_with_table_capacity_hint(expr, target, capacity_hint)?;
        Ok(target)
    }

    fn compile_expr_to_with_table_capacity_hint(
        &mut self,
        expr: &Expr,
        target: u8,
        capacity_hint: TableCapacityHint,
    ) -> Result<()> {
        if !capacity_hint.is_empty() {
            if let Expr::Table { fields, .. } = expr_without_parens(expr) {
                if fields.is_empty() {
                    return self.compile_table_to_with_capacity(fields, target, capacity_hint);
                }
            }
        }
        self.compile_expr_to(expr, target)
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
            self.code.push(Instruction::with_aux(
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
                            if source != target {
                                self.code
                                    .push(Instruction::new(Opcode::Move, target, source, 0));
                            }
                        }
                        Binding::CapturedLocal(source) => {
                            self.code.push(Instruction::new(
                                Opcode::GetLocalCell,
                                target,
                                source,
                                0,
                            ));
                        }
                        Binding::Upvalue(index) => {
                            self.code.push(Instruction::with_aux(
                                Opcode::GetUpvalue,
                                target,
                                0,
                                0,
                                i32::from(index),
                            ));
                        }
                        Binding::CurrentFunction => {
                            self.code.push(Instruction::new(
                                Opcode::LoadCurrentFunction,
                                target,
                                0,
                                0,
                            ));
                        }
                    }
                } else if let Some(index) = self.resolve_upvalue(name)? {
                    self.code.push(Instruction::with_aux(
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
                if self.compile_string_byte_equal_expr(*op, lhs, rhs, target)? {
                    return Ok(());
                }
                if *op == BinaryOp::Mul && self.compile_sin_cos_mul_expr(lhs, rhs, target)? {
                    return Ok(());
                }
                if *op == BinaryOp::Add
                    && self.compile_mod_add_const_expr(lhs, rhs, target, expr.span())?
                {
                    return Ok(());
                }
                if *op == BinaryOp::Concat
                    && self.compile_lhs_const_concat_expr(lhs, rhs, target, expr.span())?
                {
                    return Ok(());
                }
                if *op == BinaryOp::Concat
                    && self.compile_rhs_const_concat_expr(lhs, rhs, target, expr.span())?
                {
                    return Ok(());
                }
                if *op == BinaryOp::Mod
                    && self.compile_mul_mod_const_expr(lhs, rhs, target, expr.span())?
                {
                    return Ok(());
                }
                if self.compile_rhs_number_const_binary_expr(*op, lhs, rhs, target, expr.span())? {
                    return Ok(());
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
                if self.compile_simple_index_to(object, index, target)? {
                    return Ok(());
                }
                self.compile_expr_to(object, target)?;
                let index_register = self.compile_expr(index)?;
                self.code.push(Instruction::new(
                    Opcode::GetIndex,
                    target,
                    target,
                    index_register,
                ));
                self.free_register(index_register);
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
                self.code.push(Instruction::with_aux(
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
            Expr::Paren { expr, .. } => self.compile_expr_to(expr, target),
            Expr::Call { callee, args, .. } => {
                self.compile_call_to(callee, args, target, expr.span())
            }
            Expr::MethodCall {
                object,
                method,
                method_span,
                args,
                ..
            } => {
                self.compile_method_call_to(object, method, *method_span, args, target, expr.span())
            }
        }
    }

    fn compile_sin_cos_mul_expr(&mut self, lhs: &Expr, rhs: &Expr, target: u8) -> Result<bool> {
        if !self.resolve_sin_cos_mul_aliases(lhs)? || !self.resolve_sin_cos_mul_aliases(rhs)? {
            return Ok(false);
        }
        let Some((sin_arg, cos_arg)) = self.sin_cos_mul_args(lhs, rhs) else {
            return Ok(false);
        };
        let Some(name) = self.simple_local_arg_name(sin_arg) else {
            return Ok(false);
        };
        if self.simple_local_arg_name(cos_arg) != Some(name) {
            return Ok(false);
        }

        self.compile_expr_to(sin_arg, target)?;
        self.code.push(Instruction::with_aux(
            Opcode::MathSinCosMul,
            target,
            target,
            0,
            1,
        ));
        Ok(true)
    }

    fn resolve_sin_cos_mul_aliases(&mut self, expr: &Expr) -> Result<bool> {
        let Expr::Call { callee, args, .. } = expr_without_parens(expr) else {
            return Ok(true);
        };
        if args.len() != 1 {
            return Ok(true);
        }
        let Expr::Name(name, _) = expr_without_parens(callee) else {
            return Ok(true);
        };
        if self.assigned_local_names.contains(name) {
            return Ok(false);
        }
        if !self.locals.contains_key(name) {
            let _ = self.resolve_upvalue(name)?;
        }
        Ok(true)
    }

    fn compile_lhs_const_concat_expr(
        &mut self,
        lhs: &Expr,
        rhs: &Expr,
        target: u8,
        span: Span,
    ) -> Result<bool> {
        let Some(constant) = const_eval(lhs) else {
            return Ok(false);
        };
        if !matches!(constant, Constant::String(_) | Constant::Number(_)) {
            return Ok(false);
        }
        let constant = i32::try_from(self.constant_index(constant)?)
            .map_err(|_| CompileError::new("constant pool overflow"))?;
        self.compile_expr_to(rhs, target)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(Opcode::ConcatConst, target, target, 0, constant),
            span,
        );
        Ok(true)
    }

    fn compile_mod_add_const_expr(
        &mut self,
        lhs: &Expr,
        rhs: &Expr,
        target: u8,
        span: Span,
    ) -> Result<bool> {
        let Some((factor_expr, modulus, addend)) = self.mod_add_number_const_operands(lhs, rhs)
        else {
            return Ok(false);
        };
        let Some(aux) = self.number_constant_pair_operand(modulus, addend)? else {
            return Ok(false);
        };
        self.compile_expr_to(factor_expr, target)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(Opcode::ModAddConst, target, target, 0, aux),
            span,
        );
        Ok(true)
    }

    fn mod_add_number_const_expr<'a>(&self, expr: &'a Expr) -> Option<(&'a Expr, f64, f64)> {
        let Expr::Binary {
            op: BinaryOp::Add,
            lhs,
            rhs,
            ..
        } = expr_without_parens(expr)
        else {
            return None;
        };
        self.mod_add_number_const_operands(lhs, rhs)
    }

    fn mod_add_number_const_operands<'a>(
        &self,
        lhs: &'a Expr,
        rhs: &'a Expr,
    ) -> Option<(&'a Expr, f64, f64)> {
        if let Some(Constant::Number(addend)) = const_eval(rhs) {
            if let Some((factor_expr, modulus)) = self.mod_number_const_expr(lhs) {
                return Some((factor_expr, modulus, addend));
            }
        }
        if let Some(Constant::Number(addend)) = const_eval(lhs) {
            if let Some((factor_expr, modulus)) = self.mod_number_const_expr(rhs) {
                return Some((factor_expr, modulus, addend));
            }
        }
        None
    }

    fn mod_number_const_expr<'a>(&self, expr: &'a Expr) -> Option<(&'a Expr, f64)> {
        let Expr::Binary {
            op: BinaryOp::Mod,
            lhs,
            rhs,
            ..
        } = expr_without_parens(expr)
        else {
            return None;
        };

        let Some(Constant::Number(modulus)) = const_eval(rhs) else {
            return None;
        };
        self.expr_is_known_number(lhs).then_some((lhs, modulus))
    }

    fn compile_mul_mod_const_expr(
        &mut self,
        lhs: &Expr,
        rhs: &Expr,
        target: u8,
        span: Span,
    ) -> Result<bool> {
        let Some(Constant::Number(modulus)) = const_eval(rhs) else {
            return Ok(false);
        };
        if let Some((factor_expr, addend_expr, multiplier)) = self.mul_add_number_const_expr(lhs) {
            let Some(aux) = self.number_constant_pair_operand(multiplier, modulus)? else {
                return Ok(false);
            };
            self.compile_expr_to(factor_expr, target)?;
            let addend_register = self.compile_expr(addend_expr)?;
            self.emit_source_mapped_instruction(
                Instruction::with_aux(Opcode::MulAddModConst, target, target, addend_register, aux),
                span,
            );
            self.free_register(addend_register);
            return Ok(true);
        }
        let Some((factor_expr, multiplier)) = self.mul_number_const_expr(lhs) else {
            return Ok(false);
        };
        let Some(aux) = self.number_constant_pair_operand(multiplier, modulus)? else {
            return Ok(false);
        };
        self.compile_expr_to(factor_expr, target)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(Opcode::MulModConst, target, target, 0, aux),
            span,
        );
        Ok(true)
    }

    fn mul_mod_number_const_expr<'a>(&self, expr: &'a Expr) -> Option<(&'a Expr, f64, f64)> {
        let Expr::Binary {
            op: BinaryOp::Mod,
            lhs,
            rhs,
            ..
        } = expr_without_parens(expr)
        else {
            return None;
        };
        let Some(Constant::Number(modulus)) = const_eval(rhs) else {
            return None;
        };
        let Some((factor_expr, multiplier)) = self.mul_number_const_expr(lhs) else {
            return None;
        };
        Some((factor_expr, multiplier, modulus))
    }

    fn mul_add_mod_number_const_expr<'a>(
        &self,
        expr: &'a Expr,
    ) -> Option<(&'a Expr, &'a Expr, f64, f64)> {
        let Expr::Binary {
            op: BinaryOp::Mod,
            lhs,
            rhs,
            ..
        } = expr_without_parens(expr)
        else {
            return None;
        };
        let Some(Constant::Number(modulus)) = const_eval(rhs) else {
            return None;
        };
        let Some((factor_expr, addend_expr, multiplier)) = self.mul_add_number_const_expr(lhs)
        else {
            return None;
        };
        Some((factor_expr, addend_expr, multiplier, modulus))
    }

    fn mul_add_number_const_expr<'a>(&self, expr: &'a Expr) -> Option<(&'a Expr, &'a Expr, f64)> {
        let Expr::Binary {
            op: BinaryOp::Add,
            lhs,
            rhs,
            ..
        } = expr_without_parens(expr)
        else {
            return None;
        };

        if let Some((factor_expr, multiplier)) = self.mul_number_const_expr(lhs) {
            if self.expr_is_known_number(rhs) {
                return Some((factor_expr, rhs, multiplier));
            }
        }
        if let Some((factor_expr, multiplier)) = self.mul_number_const_expr(rhs) {
            if self.expr_is_known_number(lhs) {
                return Some((factor_expr, lhs, multiplier));
            }
        }
        None
    }

    fn mul_number_const_expr<'a>(&self, expr: &'a Expr) -> Option<(&'a Expr, f64)> {
        let Expr::Binary {
            op: BinaryOp::Mul,
            lhs,
            rhs,
            ..
        } = expr_without_parens(expr)
        else {
            return None;
        };

        if let Some(Constant::Number(multiplier)) = const_eval(rhs) {
            if self.expr_is_known_number(lhs) {
                return Some((lhs, multiplier));
            }
        }
        if let Some(Constant::Number(multiplier)) = const_eval(lhs) {
            if self.expr_is_known_number(rhs) {
                return Some((rhs, multiplier));
            }
        }
        None
    }

    fn compile_rhs_const_concat_expr(
        &mut self,
        lhs: &Expr,
        rhs: &Expr,
        target: u8,
        span: Span,
    ) -> Result<bool> {
        let Some(constant) = const_eval(rhs) else {
            return Ok(false);
        };
        if !matches!(constant, Constant::String(_) | Constant::Number(_)) {
            return Ok(false);
        }
        let constant = i32::try_from(self.constant_index(constant)?)
            .map_err(|_| CompileError::new("constant pool overflow"))?;
        self.compile_expr_to(lhs, target)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(Opcode::ConcatRhsConst, target, target, 0, constant),
            span,
        );
        Ok(true)
    }

    fn sin_cos_mul_args<'a>(&self, lhs: &'a Expr, rhs: &'a Expr) -> Option<(&'a Expr, &'a Expr)> {
        let (lhs_opcode, lhs_arg) = self.math_unary_call_arg(lhs)?;
        let (rhs_opcode, rhs_arg) = self.math_unary_call_arg(rhs)?;
        match (lhs_opcode, rhs_opcode) {
            (Opcode::MathSin, Opcode::MathCos) => Some((lhs_arg, rhs_arg)),
            (Opcode::MathCos, Opcode::MathSin) => Some((rhs_arg, lhs_arg)),
            _ => None,
        }
    }

    fn math_unary_call_arg<'a>(&self, expr: &'a Expr) -> Option<(Opcode, &'a Expr)> {
        let Expr::Call { callee, args, .. } = expr_without_parens(expr) else {
            return None;
        };
        if args.len() != 1 {
            return None;
        }
        let opcode = self.math_unary_callee_opcode(callee)?;
        Some((opcode, &args[0]))
    }

    fn math_unary_callee_opcode(&self, callee: &Expr) -> Option<Opcode> {
        match expr_without_parens(callee) {
            Expr::Name(name, _) => match self.local_math_builtin_alias(name)? {
                MathBuiltinAlias::Unary { opcode, .. } => Some(opcode),
                MathBuiltinAlias::Abs | MathBuiltinAlias::Window(_) => None,
            },
            Expr::Field { object, field, .. }
                if matches!(object.as_ref(), Expr::Name(name, _) if name == "math")
                    && self.builtin_table_field_is_unmodified("math", field) =>
            {
                match math_builtin_alias_for_field(field)? {
                    MathBuiltinAlias::Unary { opcode, .. } => Some(opcode),
                    MathBuiltinAlias::Abs | MathBuiltinAlias::Window(_) => None,
                }
            }
            _ => None,
        }
    }

    fn simple_local_arg_name<'a>(&self, expr: &'a Expr) -> Option<&'a str> {
        let Expr::Name(name, _) = expr_without_parens(expr) else {
            return None;
        };
        match self.locals.get(name).copied()? {
            Binding::Register(_) => Some(name),
            Binding::CapturedLocal(_) | Binding::Upvalue(_) | Binding::CurrentFunction => None,
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
            upvalues: std::mem::take(&mut self.upvalues),
            captured_registers: std::mem::take(&mut self.captured_registers),
            math_builtin_aliases: std::mem::take(&mut self.math_builtin_aliases),
            math_upvalue_aliases: std::mem::take(&mut self.math_upvalue_aliases),
            stdlib_builtin_aliases: std::mem::take(&mut self.stdlib_builtin_aliases),
            stdlib_upvalue_aliases: std::mem::take(&mut self.stdlib_upvalue_aliases),
            numeric_registers: std::mem::take(&mut self.numeric_registers),
            number_constant_locals: std::mem::take(&mut self.number_constant_locals),
            fresh_table_registers: std::mem::take(&mut self.fresh_table_registers),
            assigned_local_names: std::mem::take(&mut self.assigned_local_names),
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
        self.current_function_is_vararg = is_vararg;
        self.disable_builtin_fastpaths = true;
        self.assigned_local_names = assigned_local_names_in_statements(body);
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
        self.code.push(Instruction::new(Opcode::Return, 0, 0, 0));

        let function_code = std::mem::take(&mut self.code);
        let function_source_map = std::mem::take(&mut self.source_map);
        let function_max_registers = usize::from(self.max_registers.max(1));
        let upvalues = std::mem::take(&mut self.upvalues);
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
        .with_debug_name(debug_name)
        .with_debug_line_defined(debug_line_defined)
        .with_parameter_types(parameter_types);

        let parent_scope = self
            .capture_stack
            .pop()
            .expect("function compile scope should be balanced");
        let mut parent_math_builtin_aliases = parent_scope.math_builtin_aliases;
        let mut parent_stdlib_builtin_aliases = parent_scope.stdlib_builtin_aliases;
        let mut parent_numeric_registers = parent_scope.numeric_registers;
        let mut parent_fresh_table_registers = parent_scope.fresh_table_registers;
        for register in &parent_scope.captured_registers {
            parent_math_builtin_aliases.remove(register);
            parent_stdlib_builtin_aliases.remove(register);
            parent_numeric_registers.remove(register);
            parent_fresh_table_registers.remove(register);
        }
        self.code = parent_code;
        self.source_map = parent_source_map;
        self.locals = parent_scope.locals;
        self.upvalues = parent_scope.upvalues;
        self.captured_registers = parent_scope.captured_registers;
        self.math_builtin_aliases = parent_math_builtin_aliases;
        self.math_upvalue_aliases = parent_scope.math_upvalue_aliases;
        self.stdlib_builtin_aliases = parent_stdlib_builtin_aliases;
        self.stdlib_upvalue_aliases = parent_scope.stdlib_upvalue_aliases;
        self.numeric_registers = parent_numeric_registers;
        self.number_constant_locals = parent_scope.number_constant_locals;
        self.fresh_table_registers = parent_fresh_table_registers;
        self.assigned_local_names = parent_scope.assigned_local_names;
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
        self.code.push(Instruction::with_aux(
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
        if matches!(object, Expr::Name(name, _) if name == "math")
            && self.builtin_table_field_is_unmodified("math", field)
        {
            match field {
                "huge" => return self.load_constant_to(target, Constant::Number(f64::INFINITY)),
                "pi" => self.load_constant_to(
                    target,
                    Constant::Number(f64::from_bits(0x4009_21fb_5444_2d18)),
                )?,
                _ => {}
            }
        }
        if let Some(object_register) = self.simple_register_expr(object) {
            if target != object_register {
                let key = self.string_constant_operand(field.as_bytes())?;
                self.emit_source_mapped_instruction(
                    Instruction::with_aux(
                        Opcode::GetIndexStringConst,
                        target,
                        object_register,
                        0,
                        key,
                    ),
                    field_span,
                );
                return Ok(());
            }
        }
        self.compile_expr_to(object, target)?;
        let key = self.string_constant_operand(field.as_bytes())?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(Opcode::GetIndexStringConst, target, target, 0, key),
            field_span,
        );
        Ok(())
    }

    fn compile_simple_index_to(&mut self, object: &Expr, index: &Expr, target: u8) -> Result<bool> {
        let Some(object_register) = self.simple_register_expr(object) else {
            return Ok(false);
        };
        if target == object_register {
            return Ok(false);
        }

        if let Some((factor_expr, modulus, addend)) = self.mod_add_number_const_expr(index) {
            if let Some(factor_register) = self.simple_register_expr(factor_expr) {
                let Some(aux) = self.number_constant_pair_operand(modulus, addend)? else {
                    return Ok(false);
                };
                self.emit_source_mapped_instruction(
                    Instruction::with_aux(
                        Opcode::GetIndexModAddConst,
                        target,
                        object_register,
                        factor_register,
                        aux,
                    ),
                    index.span(),
                );
                return Ok(true);
            }
        }

        if let Some((factor_expr, multiplier, modulus)) = self.mul_mod_number_const_expr(index) {
            if let Some(factor_register) = self.simple_register_expr(factor_expr) {
                let Some(aux) = self.number_constant_pair_operand(multiplier, modulus)? else {
                    return Ok(false);
                };
                self.emit_source_mapped_instruction(
                    Instruction::with_aux(
                        Opcode::GetIndexMulModConst,
                        target,
                        object_register,
                        factor_register,
                        aux,
                    ),
                    index.span(),
                );
                return Ok(true);
            }
        }

        if let Some((constant, suffix)) = lhs_const_concat_key(index) {
            let suffix_register = self.compile_expr(suffix)?;
            if target == suffix_register {
                self.free_register(suffix_register);
                return Ok(false);
            }
            let constant = i32::try_from(self.constant_index(constant)?)
                .map_err(|_| CompileError::new("constant pool overflow"))?;
            self.emit_source_mapped_instruction(
                Instruction::with_aux(
                    Opcode::GetIndexConcatConst,
                    target,
                    object_register,
                    suffix_register,
                    constant,
                ),
                index.span(),
            );
            self.free_register(suffix_register);
            return Ok(true);
        }

        if let Some(index_register) = self.simple_register_expr(index) {
            if target == index_register {
                return Ok(false);
            }
            let opcode = if self.numeric_registers.contains(&index_register) {
                Opcode::GetIndexNumber
            } else {
                Opcode::GetIndex
            };
            self.code.push(Instruction::new(
                opcode,
                target,
                object_register,
                index_register,
            ));
            return Ok(true);
        }

        if self.fresh_table_registers.contains(&object_register) {
            if let Some(index_operand) = const_array_index_operand(index) {
                self.emit_source_mapped_instruction(
                    Instruction::with_aux(
                        Opcode::GetIndexNumberConst,
                        target,
                        object_register,
                        0,
                        index_operand,
                    ),
                    index.span(),
                );
                return Ok(true);
            }
        }

        if self.expr_is_known_number(index) {
            let key_register = self.compile_expr(index)?;
            if target == key_register {
                self.free_register_without_clear(key_register);
                return Ok(false);
            }
            self.code.push(Instruction::new(
                Opcode::GetIndexNumber,
                target,
                object_register,
                key_register,
            ));
            self.free_register_without_clear(key_register);
            return Ok(true);
        }

        let Some(key) = const_eval(index) else {
            return Ok(false);
        };
        if let Constant::String(bytes) = key {
            let key = self.string_constant_operand(&bytes)?;
            self.code.push(Instruction::with_aux(
                Opcode::GetIndexStringConst,
                target,
                object_register,
                0,
                key,
            ));
        } else {
            let key_register = self.load_constant(key)?;
            self.code.push(Instruction::new(
                Opcode::GetIndex,
                target,
                object_register,
                key_register,
            ));
            self.free_register(key_register);
        }
        Ok(true)
    }

    fn simple_register_expr(&self, expr: &Expr) -> Option<u8> {
        let Expr::Name(name, _) = expr_without_parens(expr) else {
            return None;
        };
        match self.locals.get(name).copied()? {
            Binding::Register(register) => Some(register),
            Binding::CapturedLocal(_) | Binding::Upvalue(_) | Binding::CurrentFunction => None,
        }
    }

    fn expr_has_no_effectful_evaluation(&self, expr: &Expr) -> bool {
        match expr_without_parens(expr) {
            Expr::Nil(_) | Expr::Bool(_, _) | Expr::Number(_, _) | Expr::String(_, _) => true,
            Expr::Name(_, _) => self.simple_register_expr(expr).is_some(),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                ..
            } => self.expr_is_known_number(expr),
            Expr::Unary {
                op: UnaryOp::Not,
                expr,
                ..
            } => self.expr_has_no_effectful_evaluation(expr),
            Expr::Binary { op, lhs, rhs, .. } => match op {
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::FloorDiv
                | BinaryOp::Mod
                | BinaryOp::Pow => self.expr_is_known_number(lhs) && self.expr_is_known_number(rhs),
                BinaryOp::Concat => {
                    self.expr_is_known_concat_scalar(lhs) && self.expr_is_known_concat_scalar(rhs)
                }
                BinaryOp::And | BinaryOp::Or => {
                    self.expr_has_no_effectful_evaluation(lhs)
                        && self.expr_has_no_effectful_evaluation(rhs)
                }
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => false,
            },
            Expr::InterpolatedString { .. }
            | Expr::Varargs(_)
            | Expr::Unary {
                op: UnaryOp::Len, ..
            }
            | Expr::IfElse { .. }
            | Expr::Field { .. }
            | Expr::Index { .. }
            | Expr::Table { .. }
            | Expr::Function { .. }
            | Expr::Call { .. }
            | Expr::MethodCall { .. } => false,
            Expr::Paren { .. } => unreachable!("expr_without_parens removes parens"),
        }
    }

    fn expr_is_known_number(&self, expr: &Expr) -> bool {
        if matches!(const_eval(expr), Some(Constant::Number(_))) {
            return true;
        }
        match expr_without_parens(expr) {
            Expr::Name(_, _) => self
                .simple_register_expr(expr)
                .is_some_and(|register| self.numeric_registers.contains(&register)),
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                ..
            } => self.expr_is_known_number(expr),
            Expr::Binary { op, lhs, rhs, .. } => match op {
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::FloorDiv
                | BinaryOp::Mod
                | BinaryOp::Pow => self.expr_is_known_number(lhs) && self.expr_is_known_number(rhs),
                _ => false,
            },
            _ => false,
        }
    }

    fn expr_is_known_concat_scalar(&self, expr: &Expr) -> bool {
        if matches!(
            const_eval(expr),
            Some(Constant::String(_) | Constant::Number(_))
        ) {
            return true;
        }
        if self.expr_is_known_number(expr) {
            return true;
        }
        match expr_without_parens(expr) {
            Expr::Binary {
                op: BinaryOp::Concat,
                lhs,
                rhs,
                ..
            } => self.expr_is_known_concat_scalar(lhs) && self.expr_is_known_concat_scalar(rhs),
            _ => false,
        }
    }

    fn expr_is_known_scalar_temp(&self, expr: &Expr) -> bool {
        match const_eval(expr) {
            Some(constant) => return constant_is_scalar_temp(&constant),
            None => {}
        }
        if self.expr_is_known_number(expr) {
            return true;
        }
        match expr_without_parens(expr) {
            Expr::Unary {
                op: UnaryOp::Not,
                expr,
                ..
            } => self.expr_has_no_effectful_evaluation(expr),
            _ => false,
        }
    }

    fn compile_table_to(&mut self, fields: &[TableField], target: u8) -> Result<()> {
        self.compile_table_to_with_capacity(fields, target, TableCapacityHint::default())
    }

    fn compile_table_to_with_capacity(
        &mut self,
        fields: &[TableField],
        target: u8,
        capacity_hint: TableCapacityHint,
    ) -> Result<()> {
        if self.compile_named_record_table_to(fields, target)? {
            return Ok(());
        }
        let mut array_index = 1.0;
        let array_field_count = fields
            .iter()
            .filter(|field| matches!(field, TableField::Array { .. }))
            .count();
        if array_field_count == 0 {
            self.code.push(Instruction::with_aux(
                Opcode::NewArray,
                target,
                0,
                0,
                table_capacity_hint_operand(capacity_hint)?,
            ));
        } else {
            self.load_constant_to(target, Constant::Number(array_field_count as f64))?;
            self.code.push(Instruction::with_aux(
                Opcode::TableCreate,
                target,
                target,
                0,
                1,
            ));
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
                            self.code.push(Instruction::with_aux(
                                Opcode::AppendVarargs,
                                target,
                                0,
                                0,
                                array_index as i32,
                            ));
                            continue;
                        }
                        if expr_is_call(value) && !self.call_requires_scalar_return_lowering(value)
                        {
                            self.compile_append_call_to_table(target, array_index, value)?;
                            continue;
                        }
                    }
                    let key_register = self.load_constant(Constant::Number(array_index))?;
                    let value_register = self.compile_expr(value)?;
                    self.code.push(Instruction::new(
                        Opcode::RawSet,
                        target,
                        key_register,
                        value_register,
                    ));
                    self.free_register(value_register);
                    self.free_register(key_register);
                    array_index += 1.0;
                }
                TableField::Named { name, value, .. } => {
                    let key_register =
                        self.load_constant(Constant::String(name.as_bytes().to_vec()))?;
                    let value_register =
                        self.compile_expr_with_debug_name(value, Some(name.clone()))?;
                    self.code.push(Instruction::new(
                        Opcode::RawSet,
                        target,
                        key_register,
                        value_register,
                    ));
                    self.free_register(value_register);
                    self.free_register(key_register);
                }
                TableField::Keyed { key, value, .. } => {
                    let key_register = self.compile_expr(key)?;
                    let value_register = self.compile_expr(value)?;
                    self.code.push(Instruction::new(
                        Opcode::RawSet,
                        target,
                        key_register,
                        value_register,
                    ));
                    self.free_register(value_register);
                    self.free_register(key_register);
                }
            }
        }
        Ok(())
    }

    fn compile_named_record_table_to(&mut self, fields: &[TableField], target: u8) -> Result<bool> {
        if fields.is_empty() || fields.len() > 8 {
            return Ok(false);
        }
        if !fields
            .iter()
            .all(|field| matches!(field, TableField::Named { .. }))
        {
            return Ok(false);
        }
        let span = match &fields[0] {
            TableField::Named { span, .. } => *span,
            _ => unreachable!("named-only record table was checked above"),
        };
        let window_len = fields
            .len()
            .checked_mul(2)
            .ok_or_else(|| CompileError::at("record table register window overflow", span))?;
        let first_register = self.alloc_register_window_at(window_len, span)?;
        for (index, field) in fields.iter().enumerate() {
            let TableField::Named { name, value, .. } = field else {
                unreachable!("named-only record table was checked above");
            };
            let key_register = checked_register_offset(first_register, index * 2, span)?;
            let value_register = checked_register_offset(first_register, index * 2 + 1, span)?;
            self.load_constant_to(key_register, Constant::String(name.as_bytes().to_vec()))?;
            self.compile_expr_to_with_debug_name(value, value_register, Some(name.clone()))?;
        }
        self.code.push(Instruction::with_aux(
            Opcode::NewRecord,
            target,
            first_register,
            0,
            i32::try_from(fields.len())
                .map_err(|_| CompileError::at("unsupported record table field count", span))?,
        ));
        self.free_registers_back_to(u16::from(first_register));
        Ok(true)
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
        match expr {
            Expr::Call { callee, args, .. } => {
                if args.len() > u8::MAX as usize {
                    return Err(CompileError::at(
                        "function call has too many arguments",
                        expr.span(),
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
                        expr.span(),
                    );
                }
                let mut argc = args.len();
                if self.value_call_has_expandable_trailing_arg(args) {
                    let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
                        CompileError::at(
                            "expanded table call needs a trailing argument",
                            expr.span(),
                        )
                    })?;
                    let trailing = &args[trailing_index];
                    let Some(result_count) = self.fixed_expansion_result_count(trailing) else {
                        return self.compile_append_call_expand_to_table(
                            table_register,
                            array_index,
                            callee,
                            args,
                            original_next_register,
                            expr.span(),
                        );
                    };
                    argc = trailing_index.checked_add(result_count).ok_or_else(|| {
                        CompileError::at("function call has too many arguments", expr.span())
                    })?;
                }
                if argc > u8::MAX as usize {
                    return Err(CompileError::at(
                        "function call has too many arguments",
                        expr.span(),
                    ));
                }
                let callee_register = self.alloc_register_window_at(
                    1usize.checked_add(argc).ok_or_else(|| {
                        CompileError::at("function call register window overflow", expr.span())
                    })?,
                    expr.span(),
                )?;
                self.compile_expr_to(callee, callee_register)?;
                if self.value_call_has_expandable_trailing_arg(args) {
                    let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
                        CompileError::at(
                            "expanded table call needs a trailing argument",
                            expr.span(),
                        )
                    })?;
                    for (index, arg) in args[..trailing_index].iter().enumerate() {
                        let register =
                            checked_register_offset(callee_register, index + 1, expr.span())?;
                        self.compile_expr_to(arg, register)?;
                    }
                    let trailing_register =
                        checked_register_offset(callee_register, trailing_index + 1, expr.span())?;
                    let result_count = argc - trailing_index;
                    self.compile_expand_multi_to(
                        &args[trailing_index],
                        trailing_register,
                        result_count,
                        expr.span(),
                    )?;
                } else {
                    for (index, arg) in args.iter().enumerate() {
                        let register =
                            checked_register_offset(callee_register, index + 1, expr.span())?;
                        self.compile_expr_to(arg, register)?;
                    }
                }
                self.emit_source_mapped_instruction(
                    Instruction::with_aux(
                        Opcode::AppendCall,
                        table_register,
                        callee_register,
                        u8::try_from(argc).map_err(|_| {
                            CompileError::at("function call has too many arguments", expr.span())
                        })?,
                        array_index as i32,
                    ),
                    expr.span(),
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
                    CompileError::at("function call has too many arguments", expr.span())
                })?;
                if argc > u8::MAX as usize {
                    return Err(CompileError::at(
                        "function call has too many arguments",
                        expr.span(),
                    ));
                }
                let original_next_register = self.next_register;
                let mut argc = argc;
                if self.method_call_has_expandable_trailing_arg(args) {
                    let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
                        CompileError::at(
                            "expanded table method call needs a trailing argument",
                            expr.span(),
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
                            CompileError::at("function call has too many arguments", expr.span())
                        })?;
                }
                if argc > u8::MAX as usize {
                    return Err(CompileError::at(
                        "function call has too many arguments",
                        expr.span(),
                    ));
                }
                let callee_register = self.alloc_register_window_at(
                    1usize.checked_add(argc).ok_or_else(|| {
                        CompileError::at("function call register window overflow", expr.span())
                    })?,
                    expr.span(),
                )?;
                let self_register = checked_register_offset(callee_register, 1, expr.span())?;
                self.compile_expr_to(object, self_register)?;
                let key_register =
                    self.load_constant(Constant::String(method.as_bytes().to_vec()))?;
                self.code.push(Instruction::new(
                    Opcode::GetIndex,
                    callee_register,
                    self_register,
                    key_register,
                ));
                self.free_register(key_register);
                if self.method_call_has_expandable_trailing_arg(args) {
                    let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
                        CompileError::at(
                            "expanded table method call needs a trailing argument",
                            expr.span(),
                        )
                    })?;
                    for (index, arg) in args[..trailing_index].iter().enumerate() {
                        let register =
                            checked_register_offset(callee_register, index + 2, expr.span())?;
                        self.compile_expr_to(arg, register)?;
                    }
                    let trailing_register =
                        checked_register_offset(callee_register, trailing_index + 2, expr.span())?;
                    let result_count = argc - trailing_index - 1;
                    self.compile_expand_multi_to(
                        &args[trailing_index],
                        trailing_register,
                        result_count,
                        expr.span(),
                    )?;
                } else {
                    for (index, arg) in args.iter().enumerate() {
                        let register =
                            checked_register_offset(callee_register, index + 2, expr.span())?;
                        self.compile_expr_to(arg, register)?;
                    }
                }
                self.emit_source_mapped_instruction(
                    Instruction::with_aux(
                        Opcode::AppendCall,
                        table_register,
                        callee_register,
                        u8::try_from(argc).map_err(|_| {
                            CompileError::at("function call has too many arguments", expr.span())
                        })?,
                        array_index as i32,
                    ),
                    expr.span(),
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
        self.compile_expr_to(callee, callee_register)?;
        for (index, arg) in args[..prefix_count].iter().enumerate() {
            let register = checked_register_offset(callee_register, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
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
        self.compile_expr_to(callee, callee_register)?;
        for (index, arg) in args[..trailing_index].iter().enumerate() {
            let register = checked_register_offset(callee_register, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
        }
        let trailing_callee = checked_register_offset(callee_register, trailing_index + 1, span)?;
        self.compile_call_for_expanded_args(trailing, trailing_callee, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::AppendCallExpand,
                table_register,
                callee_register,
                u8::try_from(trailing_index)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                pack_append_call_expand_aux(array_index, trailing_argc, span)?,
            ),
            span,
        );
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
        let key_register = self.load_constant(Constant::String(method.as_bytes().to_vec()))?;
        self.code.push(Instruction::new(
            Opcode::GetIndex,
            callee_register,
            self_register,
            key_register,
        ));
        self.free_register(key_register);
        for (index, arg) in args[..trailing_index].iter().enumerate() {
            let register = checked_register_offset(callee_register, index + 2, span)?;
            self.compile_expr_to(arg, register)?;
        }
        let trailing_callee = checked_register_offset(callee_register, prefix_count + 1, span)?;
        self.compile_call_for_expanded_args(trailing, trailing_callee, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::AppendCallExpand,
                table_register,
                callee_register,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                pack_append_call_expand_aux(array_index, trailing_argc, span)?,
            ),
            span,
        );
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
        let Some((iterator, table_expr)) = self.generic_array_iterator(iter, span)? else {
            if let [expr] = iter {
                if !self.expr_can_expand(expr) {
                    return self.compile_generalized_iterator_for(variables, expr, body, span);
                }
            }
            return self.compile_generic_call_for(variables, iter, body, span);
        };

        let locals = self.locals.clone();
        let next_register = self.next_register;

        let table_register = self.alloc_register_at(span)?;
        let _index_register = self.alloc_register_at(span)?;
        let key_register = self.alloc_register_at(span)?;
        let value_register = self.alloc_register_at(span)?;
        let skip_visible_key = !block_mentions_name(body, &variables[0].name);
        let iterator_key_flag = if skip_visible_key {
            ARRAY_ITERATOR_SKIP_VISIBLE_KEY
        } else {
            ARRAY_ITERATOR_WRITE_VISIBLE_KEY
        };
        let mut extra_registers = Vec::new();
        for _ in variables.iter().skip(2) {
            extra_registers.push(self.alloc_register_at(span)?);
        }

        self.compile_expr_to(table_expr, table_register)?;
        if !skip_visible_key {
            self.locals
                .insert(variables[0].name.clone(), Binding::Register(key_register));
        }
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
        self.code.push(Instruction::with_aux(
            iterator.prep_opcode(),
            table_register,
            0,
            iterator_key_flag,
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
        self.code.push(Instruction::with_aux(
            iterator.loop_opcode(),
            table_register,
            0,
            iterator_key_flag,
            body_start,
        ));

        let clear_start = self.code.len();
        self.clear_local_cells_from(next_register)?;
        self.patch_jump_to(prep_jump, clear_start, span)?;
        self.patch_loop_breaks_to(clear_start, span)?;
        self.locals = locals;
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
        if iter.is_empty() {
            return Err(CompileError::at("unsupported generic for iterator", span));
        }
        if variables.len() > u8::MAX as usize {
            return Err(CompileError::at(
                "generic for has too many visible variables",
                span,
            ));
        }

        let locals = self.locals.clone();
        let next_register = self.next_register;
        let window_len = 3usize
            .checked_add(variables.len())
            .ok_or_else(|| CompileError::at("generic for register window overflow", span))?;
        let iterator_register = self.alloc_register_window_at(window_len, span)?;
        let first_visible = checked_register_offset(iterator_register, 3, span)?;

        self.compile_generic_iterator_values(iter, iterator_register, span)?;
        for (index, binding) in variables.iter().enumerate() {
            let register = checked_register_offset(first_visible, index, span)?;
            self.locals
                .insert(binding.name.clone(), Binding::Register(register));
        }

        let visible_count = u8::try_from(variables.len())
            .map_err(|_| CompileError::at("generic for has too many visible variables", span))?;
        let prep_jump = self.code.len();
        self.code.push(Instruction::with_aux(
            Opcode::GenericCallPrep,
            iterator_register,
            0,
            visible_count,
            0,
        ));
        let body_start = self.code.len();
        self.loop_breaks.push(Vec::new());
        self.loop_continues.push(Vec::new());
        self.loop_scope_depths
            .push(self.scope_register_starts.len());
        self.compile_scoped_block(body)?;
        let continue_target = self.code.len();
        self.patch_loop_continues_to(continue_target, span)?;
        let body_start = self.jump_target(body_start, span)?;
        self.code.push(Instruction::with_aux(
            Opcode::GenericCallLoop,
            iterator_register,
            0,
            visible_count,
            body_start,
        ));

        let clear_start = self.code.len();
        self.clear_local_cells_from(next_register)?;
        self.patch_jump_to(prep_jump, clear_start, span)?;
        self.patch_loop_breaks_to(clear_start, span)?;
        self.locals = locals;
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
        let next_register = self.next_register;
        let window_len = 4usize
            .checked_add(variables.len())
            .ok_or_else(|| CompileError::at("generic for register window overflow", span))?;
        let iterator_register = self.alloc_register_window_at(window_len, span)?;
        let first_visible = checked_register_offset(iterator_register, 4, span)?;

        self.compile_expr_to(expr, iterator_register)?;
        for (index, binding) in variables.iter().enumerate() {
            let register = checked_register_offset(first_visible, index, span)?;
            self.locals
                .insert(binding.name.clone(), Binding::Register(register));
        }

        let visible_count = u8::try_from(variables.len())
            .map_err(|_| CompileError::at("generic for has too many visible variables", span))?;
        let prep_jump = self.code.len();
        self.code.push(Instruction::with_aux(
            Opcode::GeneralizedIterPrep,
            iterator_register,
            0,
            visible_count,
            0,
        ));
        let body_start = self.code.len();
        self.loop_breaks.push(Vec::new());
        self.loop_continues.push(Vec::new());
        self.loop_scope_depths
            .push(self.scope_register_starts.len());
        self.compile_scoped_block(body)?;
        let continue_target = self.code.len();
        self.patch_loop_continues_to(continue_target, span)?;
        let body_start = self.jump_target(body_start, span)?;
        self.code.push(Instruction::with_aux(
            Opcode::GeneralizedIterLoop,
            iterator_register,
            0,
            visible_count,
            body_start,
        ));

        let clear_start = self.code.len();
        self.clear_local_cells_from(next_register)?;
        self.patch_jump_to(prep_jump, clear_start, span)?;
        self.patch_loop_breaks_to(clear_start, span)?;
        self.locals = locals;
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
                self.compile_expand_multi_to(&iter[last_index], first_result, result_count, span)?;
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
        span: Span,
    ) -> Result<Option<(GenericArrayIterator, &'a Expr)>> {
        if let [Expr::Call { callee, args, .. }] = iter {
            let Expr::Name(name, _) = callee.as_ref() else {
                return Ok(None);
            };
            let iterator = match name.as_str() {
                "ipairs" if self.builtin_global_is_unmodified("ipairs") => {
                    GenericArrayIterator::Ipairs
                }
                "pairs" if self.builtin_global_is_unmodified("pairs") => {
                    GenericArrayIterator::Pairs
                }
                _ => return Ok(None),
            };
            let [table_expr] = args.as_slice() else {
                return Err(CompileError::at(format!("unsupported {name} arity"), span));
            };
            return Ok(Some((iterator, table_expr)));
        }

        if let [Expr::Name(name, _), table_expr] = iter {
            if name == "next" && self.builtin_global_is_unmodified("next") {
                return Ok(Some((GenericArrayIterator::Pairs, table_expr)));
            }
        }

        if let [table_expr @ Expr::Table { .. }] = iter {
            return Ok(Some((GenericArrayIterator::Pairs, table_expr)));
        }

        Ok(None)
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
        self.mark_global_environment_escape_for_call(callee);
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
                    "concat" => return self.compile_table_concat_call(args, target, span),
                    "getn" => {
                        return self.compile_table_unary_call(
                            args,
                            target,
                            span,
                            Opcode::TableGetN,
                            "table.getn",
                        )
                    }
                    "maxn" => {
                        return self.compile_table_unary_call(
                            args,
                            target,
                            span,
                            Opcode::TableMaxN,
                            "table.maxn",
                        )
                    }
                    "create" => return self.compile_table_create_call(args, target, span),
                    "find" => return self.compile_table_find_call(args, target, span),
                    "clear" | "freeze" | "isfrozen" => {
                        return self.compile_value_call_to(callee, args, target, span);
                    }
                    "insert" => return self.compile_table_insert_call(args, target, span),
                    "remove" => return self.compile_table_remove_call(args, target, span),
                    "move" | "sort" | "foreach" | "foreachi" | "clone" => {
                        return self.compile_value_call_to(callee, args, target, span);
                    }
                    "pack" | "unpack" => {
                        return self.compile_value_call_to(callee, args, target, span);
                    }
                    _ => {
                        return Err(CompileError::at(
                            format!("unsupported table.{field} call"),
                            span,
                        ))
                    }
                }
            }
            if matches!(object.as_ref(), Expr::Name(name, _) if name == "math")
                && self.builtin_global_is_unmodified("math")
            {
                if !self.builtin_table_field_is_unmodified("math", field) {
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
                    _ => {
                        return Err(CompileError::at(
                            format!("unsupported math.{field} call"),
                            span,
                        ))
                    }
                }
            }
            if matches!(object.as_ref(), Expr::Name(name, _) if name == "string")
                && self.builtin_global_is_unmodified("string")
            {
                if !self.builtin_table_field_is_unmodified("string", field) {
                    return self.compile_value_call_to(callee, args, target, span);
                }
                match field.as_str() {
                    "sub" => return self.compile_string_sub_call(args, target, span),
                    "len" => return self.compile_string_len_call(args, target, span),
                    "byte" => return self.compile_string_byte_call(args, target, span),
                    "upper" => {
                        return self.compile_string_unary_call(
                            args,
                            target,
                            span,
                            Opcode::StringUpper,
                            "string.upper",
                        )
                    }
                    "lower" => {
                        return self.compile_string_unary_call(
                            args,
                            target,
                            span,
                            Opcode::StringLower,
                            "string.lower",
                        )
                    }
                    "reverse" => {
                        return self.compile_string_unary_call(
                            args,
                            target,
                            span,
                            Opcode::StringReverse,
                            "string.reverse",
                        )
                    }
                    "rep" => return self.compile_string_rep_call(args, target, span),
                    "find" => return self.compile_string_find_call(args, target, span),
                    "split" => return self.compile_string_split_call(args, target, span),
                    "format" => {
                        if self.compile_string_format_call(args, target, span)? {
                            return Ok(());
                        }
                        return self.compile_value_call_to(callee, args, target, span);
                    }
                    "char" | "match" | "gsub" | "gmatch" | "pack" | "unpack" | "packsize" => {
                        return self.compile_value_call_to(callee, args, target, span)
                    }
                    _ => {
                        return Err(CompileError::at(
                            format!("unsupported string.{field} call"),
                            span,
                        ))
                    }
                }
            }
            return self.compile_value_call_to(callee, args, target, span);
        }

        if let Expr::Name(name, _) = callee {
            if !self.locals.contains_key(name) {
                let _ = self.resolve_upvalue(name)?;
            }
            if let Some(alias) = self.local_math_builtin_alias(name) {
                if self.compile_math_builtin_alias_call_to(alias, args, target, span)? {
                    return Ok(());
                }
            }
            if let Some(alias) = self.local_stdlib_builtin_alias(name) {
                if self.compile_stdlib_builtin_alias_call_to(alias, args, target, span)? {
                    return Ok(());
                }
            }
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
                            return Err(CompileError::at("unsupported tonumber arity", span));
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
                            return Err(CompileError::at("unsupported tostring arity", span));
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
                    "type" | "typeof" => self.compile_typeof_call(args, target, span),
                    "loadstring"
                        if self.options.builtins == BuiltinEnvironment::LuauConformance =>
                    {
                        self.compile_conformance_loadstring_call(callee, args, target, span)
                    }
                    "rawequal" => self.compile_value_call_to(callee, args, target, span),
                    "rawget" => self.compile_rawget_call(args, target, span),
                    "rawset" => self.compile_rawset_call(args, target, span),
                    "rawlen" => self.compile_rawlen_call(args, target, span),
                    "next" => self.compile_next_call(args, target, span),
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
        self.mark_global_environment_escape_for_call(callee);
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
            return self.compile_value_call_expand_to(callee, args, target, 1, span);
        }
        self.compile_expr_to(callee, target)?;
        let mut arg_registers = Vec::with_capacity(args.len());
        for arg in args {
            arg_registers.push(self.compile_expr(arg)?);
        }
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::Call,
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
        let prototype = self.compile_detached_chunk_prototype(&chunk, span)?;
        self.code.push(Instruction::with_aux(
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
        let parent_upvalues = std::mem::take(&mut self.upvalues);
        let parent_captured_registers = std::mem::take(&mut self.captured_registers);
        let parent_capture_stack = std::mem::take(&mut self.capture_stack);
        let parent_assigned_globals = self.assigned_globals.clone();
        let parent_assigned_stdlib_fields = self.assigned_stdlib_fields.clone();
        let parent_stdlib_tables_may_be_mutated = self.stdlib_tables_may_be_mutated;
        let parent_global_environment_escaped = self.global_environment_escaped;
        let parent_disable_builtin_fastpaths = self.disable_builtin_fastpaths;
        let parent_math_builtin_aliases = std::mem::take(&mut self.math_builtin_aliases);
        let parent_math_upvalue_aliases = std::mem::take(&mut self.math_upvalue_aliases);
        let parent_stdlib_builtin_aliases = std::mem::take(&mut self.stdlib_builtin_aliases);
        let parent_stdlib_upvalue_aliases = std::mem::take(&mut self.stdlib_upvalue_aliases);
        let parent_numeric_registers = std::mem::take(&mut self.numeric_registers);
        let parent_number_constant_locals = std::mem::take(&mut self.number_constant_locals);
        let parent_assigned_local_names = std::mem::take(&mut self.assigned_local_names);
        let parent_next_register = self.next_register;
        let parent_max_registers = self.max_registers;
        let parent_loop_breaks = std::mem::take(&mut self.loop_breaks);
        let parent_loop_continues = std::mem::take(&mut self.loop_continues);
        let parent_loop_scope_depths = std::mem::take(&mut self.loop_scope_depths);
        let parent_scope_register_starts = std::mem::take(&mut self.scope_register_starts);
        let parent_current_function_is_vararg = self.current_function_is_vararg;

        self.next_register = 0;
        self.max_registers = 0;
        self.current_function_is_vararg = true;
        self.disable_builtin_fastpaths = true;
        self.assigned_local_names = assigned_local_names_in_statements(&chunk.statements);
        self.compile_block(&chunk.statements)?;
        self.code.push(Instruction::new(Opcode::Return, 0, 0, 0));

        let function_code = std::mem::take(&mut self.code);
        let function_source_map = std::mem::take(&mut self.source_map);
        let function_max_registers = usize::from(self.max_registers.max(1));
        self.functions[prototype] = Function::with_params_vararg_and_upvalues(
            function_code,
            function_max_registers,
            0,
            true,
            Vec::new(),
            Vec::new(),
        )
        .with_source_map(function_source_map);

        self.code = parent_code;
        self.source_map = parent_source_map;
        self.options.source_line_starts = parent_source_line_starts;
        self.locals = parent_locals;
        self.upvalues = parent_upvalues;
        self.captured_registers = parent_captured_registers;
        self.capture_stack = parent_capture_stack;
        self.assigned_globals = parent_assigned_globals;
        self.assigned_stdlib_fields = parent_assigned_stdlib_fields;
        self.stdlib_tables_may_be_mutated = parent_stdlib_tables_may_be_mutated;
        self.global_environment_escaped = parent_global_environment_escaped;
        self.disable_builtin_fastpaths = parent_disable_builtin_fastpaths;
        self.math_builtin_aliases = parent_math_builtin_aliases;
        self.math_upvalue_aliases = parent_math_upvalue_aliases;
        self.stdlib_builtin_aliases = parent_stdlib_builtin_aliases;
        self.stdlib_upvalue_aliases = parent_stdlib_upvalue_aliases;
        self.numeric_registers = parent_numeric_registers;
        self.number_constant_locals = parent_number_constant_locals;
        self.assigned_local_names = parent_assigned_local_names;
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
                self.code.push(Instruction::new(
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
            return self.compile_value_return_call_expand(
                callee,
                args,
                prefix_first,
                prefix_count,
                span,
            );
        }
        let callee_register = self.compile_expr(callee)?;
        let mut arg_registers = Vec::with_capacity(args.len());
        for arg in args {
            arg_registers.push(self.compile_expr(arg)?);
        }
        let args_start = arg_registers.first().copied().unwrap_or(0);
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

    fn compile_value_return_call_expand(
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
            CompileError::at("expanded return call needs a trailing argument", span)
        })?;
        if self.call_expr_has_trailing_varargs(&args[trailing_index]) {
            return self.compile_value_return_call_expand_varargs(
                callee,
                args,
                prefix_first,
                prefix_count,
                span,
            );
        }
        if self.call_expr_has_expandable_trailing_arg(&args[trailing_index]) {
            return self.compile_value_return_call_expand_nested(
                callee,
                args,
                prefix_first,
                prefix_count,
                span,
            );
        }
        let trailing_argc = call_arg_count(&args[trailing_index], span)?;
        if trailing_index > u8::MAX as usize || trailing_argc > u8::MAX as usize {
            return Err(CompileError::at(
                "function call has too many arguments",
                span,
            ));
        }

        let window_len = 1usize
            .checked_add(trailing_index)
            .and_then(|len| len.checked_add(1))
            .and_then(|len| len.checked_add(trailing_argc))
            .ok_or_else(|| {
                CompileError::at("expanded return call register window overflow", span)
            })?;
        let call_base = self.alloc_register_window_at(window_len, span)?;
        self.compile_expr_to(callee, call_base)?;
        for (index, arg) in args[..trailing_index].iter().enumerate() {
            let register = checked_register_offset(call_base, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
        }
        let trailing_callee = checked_register_offset(call_base, trailing_index + 1, span)?;
        self.compile_call_for_expanded_args(&args[trailing_index], trailing_callee, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::ReturnCallExpand,
                prefix_first,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("return list is too large", span))?,
                call_base,
                pack_call_expand_aux(trailing_index, trailing_argc, span)?,
            ),
            span,
        );
        Ok(())
    }

    fn compile_value_return_call_expand_nested(
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
        let outer_prefix_count = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("nested return call needs a trailing argument", span)
        })?;
        let trailing = &args[outer_prefix_count];
        let nested_shape = nested_call_shape(trailing, span)?;
        if self.call_expr_has_trailing_varargs(nested_shape.trailing) {
            return self.compile_value_return_call_expand2_varargs(
                callee,
                args,
                prefix_first,
                prefix_count,
                span,
            );
        }
        let window_len = nested_expand_window_len(outer_prefix_count, nested_shape, span)?;
        let call_base = self.alloc_register_window_at(window_len, span)?;
        self.compile_expr_to(callee, call_base)?;
        for (index, arg) in args[..outer_prefix_count].iter().enumerate() {
            let register = checked_register_offset(call_base, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
        }
        let middle_callee = checked_register_offset(call_base, outer_prefix_count + 1, span)?;
        self.compile_nested_call_prefix(trailing, middle_callee, span)?;
        let inner_callee =
            checked_register_offset(middle_callee, nested_shape.prefix_count + 1, span)?;
        self.compile_call_for_expanded_args(nested_shape.trailing, inner_callee, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::ReturnCallExpand2,
                prefix_first,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("return list is too large", span))?,
                call_base,
                pack_nested_expand_aux(outer_prefix_count, nested_shape, span)?,
            ),
            span,
        );
        Ok(())
    }

    fn compile_value_return_call_expand2_varargs(
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
        let outer_prefix_count = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("nested vararg return call needs a trailing argument", span)
        })?;
        let middle = &args[outer_prefix_count];
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
        self.compile_expr_to(callee, call_base)?;
        for (index, arg) in args[..outer_prefix_count].iter().enumerate() {
            let register = checked_register_offset(call_base, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
        }
        let middle_callee = checked_register_offset(call_base, outer_prefix_count + 1, span)?;
        self.compile_nested_call_prefix(middle, middle_callee, span)?;
        let inner_callee =
            checked_register_offset(middle_callee, middle_shape.prefix_count + 1, span)?;
        self.compile_nested_call_prefix(inner, inner_callee, span)?;
        self.emit_source_mapped_instruction(
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
            span,
        );
        Ok(())
    }

    fn compile_value_return_call_expand_varargs(
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
        let outer_prefix_count = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("vararg return call needs a trailing argument", span)
        })?;
        let trailing = &args[outer_prefix_count];
        let nested_shape = nested_call_shape(trailing, span)?;
        let Expr::Varargs(vararg_span) = nested_shape.trailing else {
            unreachable!("nested vararg expansion requires trailing varargs")
        };
        self.validate_varargs_available(*vararg_span)?;
        let window_len = nested_vararg_window_len(outer_prefix_count, nested_shape, span)?;
        let call_base = self.alloc_register_window_at(window_len, span)?;
        self.compile_expr_to(callee, call_base)?;
        for (index, arg) in args[..outer_prefix_count].iter().enumerate() {
            let register = checked_register_offset(call_base, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
        }
        let middle_callee = checked_register_offset(call_base, outer_prefix_count + 1, span)?;
        self.compile_nested_call_prefix(trailing, middle_callee, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::ReturnCallExpandVarargs,
                prefix_first,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("return list is too large", span))?,
                call_base,
                pack_expand_varargs_aux(outer_prefix_count, nested_shape.prefix_count, span)?,
            ),
            span,
        );
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

        let callee_register = self.compile_expr(callee)?;
        let mut arg_registers = Vec::with_capacity(trailing_index);
        for arg in &args[..trailing_index] {
            arg_registers.push(self.compile_expr(arg)?);
        }
        let args_start = arg_registers.first().copied().unwrap_or(0);
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
        self.compile_expr_to(object, self_register)?;
        let key_register = self.load_constant(Constant::String(method.as_bytes().to_vec()))?;
        self.code.push(Instruction::new(
            Opcode::GetIndex,
            callee_register,
            self_register,
            key_register,
        ));
        self.free_register(key_register);

        for arg in args {
            self.compile_expr(arg)?;
        }
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
        if prefix_count > u8::MAX as usize {
            return Err(CompileError::at("return list is too large", span));
        }
        let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at(
                "expanded return method call needs a trailing argument",
                span,
            )
        })?;
        let method_prefix_count = trailing_index
            .checked_add(1)
            .ok_or_else(|| CompileError::at("function call has too many arguments", span))?;
        let trailing_argc = call_arg_count(&args[trailing_index], span)?;
        let window_len = 1usize
            .checked_add(method_prefix_count)
            .and_then(|len| len.checked_add(1))
            .and_then(|len| len.checked_add(trailing_argc))
            .ok_or_else(|| {
                CompileError::at("expanded return method call register window overflow", span)
            })?;
        let call_base = self.alloc_register_window_at(window_len, span)?;
        let self_register = checked_register_offset(call_base, 1, span)?;
        self.compile_expr_to(object, self_register)?;
        let key_register = self.load_constant(Constant::String(method.as_bytes().to_vec()))?;
        self.code.push(Instruction::new(
            Opcode::GetIndex,
            call_base,
            self_register,
            key_register,
        ));
        self.free_register(key_register);
        for (index, arg) in args[..trailing_index].iter().enumerate() {
            let register = checked_register_offset(call_base, index + 2, span)?;
            self.compile_expr_to(arg, register)?;
        }
        let trailing_callee = checked_register_offset(call_base, method_prefix_count + 1, span)?;
        self.compile_call_for_expanded_args(&args[trailing_index], trailing_callee, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::ReturnCallExpand,
                prefix_first,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("return list is too large", span))?,
                call_base,
                pack_call_expand_aux(method_prefix_count, trailing_argc, span)?,
            ),
            span,
        );
        Ok(())
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
        self.compile_expr_to(object, self_register)?;
        let key_register = self.load_constant(Constant::String(method.as_bytes().to_vec()))?;
        self.code.push(Instruction::new(
            Opcode::GetIndex,
            callee_register,
            self_register,
            key_register,
        ));
        self.free_register(key_register);

        for arg in &args[..trailing_index] {
            self.compile_expr(arg)?;
        }
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
        span: Span,
    ) -> Result<bool> {
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
        self.mark_global_environment_escape_for_call(callee);
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
            return self.compile_value_call_expand_to(
                callee,
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

        self.compile_expr_to(callee, first_register)?;
        for (index, arg) in args.iter().enumerate() {
            let register = checked_register_offset(first_register, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
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

    fn compile_value_call_expand_to(
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
            .ok_or_else(|| CompileError::at("expanded call needs a trailing argument", span))?;
        if self.call_expr_has_trailing_varargs(&args[trailing_index]) {
            return self.compile_value_call_expand_varargs_to(
                callee,
                args,
                first_register,
                result_count,
                span,
            );
        }
        if self.call_expr_has_expandable_trailing_arg(&args[trailing_index]) {
            return self.compile_value_call_expand_nested_to(
                callee,
                args,
                first_register,
                result_count,
                span,
            );
        }
        self.validate_call_multi_shape(trailing_index, result_count, span)?;
        let trailing_argc = call_arg_count(&args[trailing_index], span)?;
        if trailing_argc > u8::MAX as usize {
            return Err(CompileError::at(
                "function call has too many arguments",
                span,
            ));
        }

        let original_next_register = self.next_register;
        let window_len = result_count.max(
            1usize
                .checked_add(trailing_index)
                .and_then(|len| len.checked_add(1))
                .and_then(|len| len.checked_add(trailing_argc))
                .ok_or_else(|| CompileError::at("expanded call register window overflow", span))?,
        );
        self.ensure_register_window_len(first_register, window_len, span)?;

        self.compile_expr_to(callee, first_register)?;
        for (index, arg) in args[..trailing_index].iter().enumerate() {
            let register = checked_register_offset(first_register, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
        }
        let trailing_callee = checked_register_offset(first_register, trailing_index + 1, span)?;
        self.compile_call_for_expanded_args(&args[trailing_index], trailing_callee, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::CallExpand,
                first_register,
                u8::try_from(trailing_index)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                u8::try_from(trailing_argc)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                i32::try_from(result_count)
                    .map_err(|_| CompileError::at("function call has too many results", span))?,
            ),
            span,
        );
        self.free_registers_back_to(original_next_register);
        Ok(())
    }

    fn compile_value_call_expand_nested_to(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        first_register: u8,
        result_count: usize,
        span: Span,
    ) -> Result<()> {
        let outer_prefix_count = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("nested expanded call needs a trailing argument", span)
        })?;
        let trailing = &args[outer_prefix_count];
        let nested_shape = nested_call_shape(trailing, span)?;
        self.validate_call_multi_shape(outer_prefix_count, result_count, span)?;
        if self.call_expr_has_expandable_trailing_arg(nested_shape.trailing) {
            return self.compile_value_call_expand3_to(
                callee,
                args,
                first_register,
                result_count,
                span,
            );
        }

        let original_next_register = self.next_register;
        let window_len = result_count.max(nested_expand_window_len(
            outer_prefix_count,
            nested_shape,
            span,
        )?);
        self.ensure_register_window_len(first_register, window_len, span)?;

        self.compile_expr_to(callee, first_register)?;
        for (index, arg) in args[..outer_prefix_count].iter().enumerate() {
            let register = checked_register_offset(first_register, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
        }
        let middle_callee = checked_register_offset(first_register, outer_prefix_count + 1, span)?;
        self.compile_nested_call_prefix(trailing, middle_callee, span)?;
        let inner_callee =
            checked_register_offset(middle_callee, nested_shape.prefix_count + 1, span)?;
        self.compile_call_for_expanded_args(nested_shape.trailing, inner_callee, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::CallExpand2,
                first_register,
                u8::try_from(outer_prefix_count)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                u8::try_from(result_count)
                    .map_err(|_| CompileError::at("function call has too many results", span))?,
                pack_nested_expand_aux(outer_prefix_count, nested_shape, span)?,
            ),
            span,
        );
        self.free_registers_back_to(original_next_register);
        Ok(())
    }

    fn compile_value_call_expand3_to(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        first_register: u8,
        result_count: usize,
        span: Span,
    ) -> Result<()> {
        let outer_prefix_count = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("nested expanded call needs a trailing argument", span)
        })?;
        let middle = &args[outer_prefix_count];
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

        self.compile_expr_to(callee, first_register)?;
        for (index, arg) in args[..outer_prefix_count].iter().enumerate() {
            let register = checked_register_offset(first_register, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
        }
        let middle_callee = checked_register_offset(first_register, outer_prefix_count + 1, span)?;
        self.compile_nested_call_prefix(middle, middle_callee, span)?;
        let inner_callee =
            checked_register_offset(middle_callee, middle_shape.prefix_count + 1, span)?;
        self.compile_nested_call_prefix(inner, inner_callee, span)?;
        let leaf_callee =
            checked_register_offset(inner_callee, inner_shape.prefix_count + 1, span)?;
        self.compile_call_for_expanded_args(inner_shape.trailing, leaf_callee, span)?;
        self.emit_source_mapped_instruction(
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
            span,
        );
        self.free_registers_back_to(original_next_register);
        Ok(())
    }

    fn compile_value_call_expand_varargs_to(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        first_register: u8,
        result_count: usize,
        span: Span,
    ) -> Result<()> {
        let outer_prefix_count = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("vararg expanded call needs a trailing argument", span)
        })?;
        let trailing = &args[outer_prefix_count];
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

        self.compile_expr_to(callee, first_register)?;
        for (index, arg) in args[..outer_prefix_count].iter().enumerate() {
            let register = checked_register_offset(first_register, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
        }
        let middle_callee = checked_register_offset(first_register, outer_prefix_count + 1, span)?;
        self.compile_nested_call_prefix(trailing, middle_callee, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::CallExpandVarargs,
                first_register,
                u8::try_from(outer_prefix_count)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                u8::try_from(result_count)
                    .map_err(|_| CompileError::at("function call has too many results", span))?,
                pack_expand_varargs_aux(outer_prefix_count, nested_shape.prefix_count, span)?,
            ),
            span,
        );
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

        self.compile_expr_to(callee, first_register)?;
        for (index, arg) in args[..trailing_index].iter().enumerate() {
            let register = checked_register_offset(first_register, index + 1, span)?;
            self.compile_expr_to(arg, register)?;
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
                self.compile_expr_to(callee, callee_register)?;
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
                let key_register =
                    self.load_constant(Constant::String(method.as_bytes().to_vec()))?;
                self.code.push(Instruction::new(
                    Opcode::GetIndex,
                    callee_register,
                    self_register,
                    key_register,
                ));
                self.free_register(key_register);
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
                self.compile_expr_to(callee, callee_register)?;
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
                let key_register =
                    self.load_constant(Constant::String(method.as_bytes().to_vec()))?;
                self.code.push(Instruction::new(
                    Opcode::GetIndex,
                    callee_register,
                    self_register,
                    key_register,
                ));
                self.free_register(key_register);
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
        self.compile_expr_to(object, self_register)?;
        let key_register = self.load_constant(Constant::String(method.as_bytes().to_vec()))?;
        self.code.push(Instruction::new(
            Opcode::GetIndex,
            first_register,
            self_register,
            key_register,
        ));
        self.free_register(key_register);

        for (index, arg) in args.iter().enumerate() {
            let register = checked_register_offset(first_register, index + 2, span)?;
            self.compile_expr_to(arg, register)?;
        }
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
        _method_span: Span,
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
        self.compile_expr_to(object, self_register)?;
        let key_register = self.load_constant(Constant::String(method.as_bytes().to_vec()))?;
        self.code.push(Instruction::new(
            Opcode::GetIndex,
            target,
            self_register,
            key_register,
        ));
        self.emit_source_mapped_instruction(
            Instruction::new(Opcode::CheckMethod, target, self_register, key_register),
            span,
        );
        self.free_register(key_register);

        let mut arg_registers = Vec::with_capacity(args.len());
        for arg in args {
            arg_registers.push(self.compile_expr(arg)?);
        }
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
        let trailing_index = args.len().checked_sub(1).ok_or_else(|| {
            CompileError::at("expanded method call needs a trailing argument", span)
        })?;
        let prefix_count = trailing_index
            .checked_add(1)
            .ok_or_else(|| CompileError::at("function call has too many arguments", span))?;
        self.validate_call_multi_shape(prefix_count, result_count, span)?;
        let trailing_argc = call_arg_count(&args[trailing_index], span)?;

        let original_next_register = self.next_register;
        let window_len = result_count.max(
            1usize
                .checked_add(prefix_count)
                .and_then(|len| len.checked_add(1))
                .and_then(|len| len.checked_add(trailing_argc))
                .ok_or_else(|| {
                    CompileError::at("expanded method call register window overflow", span)
                })?,
        );
        self.ensure_register_window_len(first_register, window_len, span)?;

        let self_register = checked_register_offset(first_register, 1, span)?;
        self.compile_expr_to(object, self_register)?;
        let key_register = self.load_constant(Constant::String(method.as_bytes().to_vec()))?;
        self.code.push(Instruction::new(
            Opcode::GetIndex,
            first_register,
            self_register,
            key_register,
        ));
        self.free_register(key_register);
        for (index, arg) in args[..trailing_index].iter().enumerate() {
            let register = checked_register_offset(first_register, index + 2, span)?;
            self.compile_expr_to(arg, register)?;
        }
        let trailing_callee = checked_register_offset(first_register, prefix_count + 1, span)?;
        self.compile_call_for_expanded_args(&args[trailing_index], trailing_callee, span)?;
        self.emit_source_mapped_instruction(
            Instruction::with_aux(
                Opcode::CallExpand,
                first_register,
                u8::try_from(prefix_count)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                u8::try_from(trailing_argc)
                    .map_err(|_| CompileError::at("function call has too many arguments", span))?,
                i32::try_from(result_count)
                    .map_err(|_| CompileError::at("function call has too many results", span))?,
            ),
            span,
        );
        self.free_registers_back_to(original_next_register);
        Ok(())
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
        self.compile_expr_to(object, self_register)?;
        let key_register = self.load_constant(Constant::String(method.as_bytes().to_vec()))?;
        self.code.push(Instruction::new(
            Opcode::GetIndex,
            first_register,
            self_register,
            key_register,
        ));
        self.free_register(key_register);

        for (index, arg) in args[..trailing_index].iter().enumerate() {
            let register = checked_register_offset(first_register, index + 2, span)?;
            self.compile_expr_to(arg, register)?;
        }
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
        self.code.push(Instruction::with_aux(
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

    fn compile_math_builtin_alias_call_to(
        &mut self,
        alias: MathBuiltinAlias,
        args: &[Expr],
        target: u8,
        span: Span,
    ) -> Result<bool> {
        match alias {
            MathBuiltinAlias::Abs => {
                if args.is_empty() || math_abs_needs_native_error_path(args) {
                    return Ok(false);
                }
                self.compile_math_abs_call(args, target, span)?;
            }
            MathBuiltinAlias::Unary { opcode, name } => {
                if args.is_empty() {
                    return Ok(false);
                }
                self.compile_math_unary_call(args, target, span, opcode, name)?;
            }
            MathBuiltinAlias::Window(spec) => {
                if args.len() < spec.min_arity || args.len() > u8::MAX as usize {
                    return Ok(false);
                }
                self.compile_math_window_call(args, target, span, spec)?;
            }
        }
        Ok(true)
    }

    fn compile_stdlib_builtin_alias_call_to(
        &mut self,
        alias: StdlibBuiltinAlias,
        args: &[Expr],
        target: u8,
        span: Span,
    ) -> Result<bool> {
        match alias {
            StdlibBuiltinAlias::StringSub => {
                if !(2..=3).contains(&args.len()) {
                    return Ok(false);
                }
                self.compile_string_sub_call(args, target, span)?;
            }
            StdlibBuiltinAlias::StringFind => {
                if !(2..=4).contains(&args.len()) {
                    return Ok(false);
                }
                self.compile_string_find_call(args, target, span)?;
            }
            StdlibBuiltinAlias::ToNumber => {
                if args.len() != 1 {
                    return Ok(false);
                }
                self.compile_expr_to(&args[0], target)?;
                self.emit_source_mapped_instruction(
                    Instruction::new(Opcode::ToNumber, target, target, 0),
                    span,
                );
            }
            StdlibBuiltinAlias::StringFormat => {
                if !self.compile_string_format_call(args, target, span)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
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
        self.code.push(Instruction::new(
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
        self.code.push(Instruction::new(
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
        self.code.push(Instruction::new(
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
            Some(self.compile_expr(separator)?)
        } else {
            None
        };
        let mut range_registers = Vec::new();
        for arg in args.iter().skip(2) {
            range_registers.push(self.compile_expr(arg)?);
        }
        self.code.push(Instruction::with_aux(
            Opcode::TableConcat,
            target,
            target,
            separator_register.unwrap_or(0),
            i32::try_from(args.len())
                .map_err(|_| CompileError::at("unsupported table.concat arity", span))?,
        ));
        for register in range_registers.into_iter().rev() {
            self.free_register(register);
        }
        if let Some(separator_register) = separator_register {
            self.free_register(separator_register);
        }
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
        self.code.push(Instruction::new(opcode, target, target, 0));
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
        self.code.push(Instruction::with_aux(
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
        self.code.push(Instruction::with_aux(
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
        self.code.push(Instruction::with_aux(
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
        self.code.push(Instruction::with_aux(
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
        self.code.push(Instruction::with_aux(
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
        self.code.push(Instruction::with_aux(
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
        self.code.push(Instruction::new(opcode, target, target, 0));
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
        self.code.push(Instruction::with_aux(
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
        self.code.push(Instruction::with_aux(
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
        self.code.push(Instruction::with_aux(
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

    fn compile_string_format_call(
        &mut self,
        args: &[Expr],
        target: u8,
        span: Span,
    ) -> Result<bool> {
        if args.is_empty()
            || args.len() > u8::MAX as usize
            || self.value_call_has_trailing_varargs(args)
            || self.value_call_has_expandable_trailing_arg(args)
        {
            return Ok(false);
        }
        let args_start = self.alloc_register_window_at(args.len(), span)?;
        for (index, arg) in args.iter().enumerate() {
            let register = checked_register_offset(args_start, index, span)?;
            self.compile_expr_to(arg, register)?;
        }
        self.code.push(Instruction::with_aux(
            Opcode::StringFormat,
            target,
            args_start,
            0,
            i32::try_from(args.len())
                .map_err(|_| CompileError::at("unsupported string.format arity", span))?,
        ));
        self.free_registers_back_to(u16::from(args_start));
        Ok(true)
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
        self.code.push(Instruction::new(
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
        if let Ok(constant) = u8::try_from(constant) {
            self.code
                .push(Instruction::new(Opcode::LoadConst, register, 0, constant));
        } else {
            let constant =
                i32::try_from(constant).map_err(|_| CompileError::new("constant pool overflow"))?;
            self.code.push(Instruction::with_aux(
                Opcode::LoadConstWide,
                register,
                0,
                0,
                constant,
            ));
        }
        Ok(())
    }

    fn clear_local_cell(&mut self, register: u8) {
        self.code
            .push(Instruction::new(Opcode::ClearLocalCell, register, 0, 0));
    }

    fn clear_local_cells_from(&mut self, start_register: u16) -> Result<()> {
        let mut register = start_register;
        while register < self.next_register {
            let remaining = self.next_register - register;
            let count = remaining.min(u16::from(u8::MAX));
            let start =
                u8::try_from(register).map_err(|_| CompileError::new("register file overflow"))?;
            let count =
                u8::try_from(count).map_err(|_| CompileError::new("register file overflow"))?;
            if count == 1 {
                self.clear_local_cell(start);
            } else {
                self.code
                    .push(Instruction::new(Opcode::ClearLocalCells, start, count, 0));
            }
            register += u16::from(count);
        }
        Ok(())
    }

    fn global_key_index(&mut self, name: &str) -> Result<i32> {
        i32::try_from(self.constant_index(Constant::String(name.as_bytes().to_vec()))?)
            .map_err(|_| CompileError::new("constant pool overflow"))
    }

    fn string_constant_operand(&mut self, bytes: &[u8]) -> Result<i32> {
        i32::try_from(self.constant_index(Constant::String(bytes.to_vec()))?)
            .map_err(|_| CompileError::new("constant pool overflow"))
    }

    fn number_constant_operand(&mut self, value: f64) -> Result<i32> {
        i32::try_from(self.constant_index(Constant::Number(value))?)
            .map_err(|_| CompileError::new("constant pool overflow"))
    }

    fn number_constant_pair_operand(&mut self, first: f64, second: f64) -> Result<Option<i32>> {
        let first = self.constant_index(Constant::Number(first))?;
        let second = self.constant_index(Constant::Number(second))?;
        if first > u16::MAX as usize || second > u16::MAX as usize {
            return Ok(None);
        }
        let packed = ((first as u32) << 16) | second as u32;
        Ok(Some(packed as i32))
    }

    fn builtin_global_is_unmodified(&self, name: &str) -> bool {
        (!self.disable_builtin_fastpaths || self.stable_global_builtins.contains(name))
            && !self.global_environment_escaped
            && !self.locals.contains_key(name)
            && !self.assigned_globals.contains(name)
    }

    fn mark_global_environment_escape_for_call(&mut self, callee: &Expr) {
        if matches!(callee, Expr::Name(name, _) if name == "getfenv" || name == "setfenv") {
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

    fn math_builtin_alias_for_expr(&self, expr: &Expr) -> Option<MathBuiltinAlias> {
        let Expr::Field { object, field, .. } = expr else {
            return None;
        };
        if !matches!(object.as_ref(), Expr::Name(name, _) if name == "math")
            || !self.builtin_table_field_is_unmodified("math", field)
        {
            return None;
        }
        math_builtin_alias_for_field(field)
    }

    fn stdlib_builtin_alias_for_expr(&self, expr: &Expr) -> Option<StdlibBuiltinAlias> {
        match expr {
            Expr::Field { object, field, .. }
                if matches!(object.as_ref(), Expr::Name(name, _) if name == "string")
                    && self.builtin_table_field_is_unmodified("string", field) =>
            {
                match field.as_str() {
                    "sub" => Some(StdlibBuiltinAlias::StringSub),
                    "find" => Some(StdlibBuiltinAlias::StringFind),
                    "format" => Some(StdlibBuiltinAlias::StringFormat),
                    _ => None,
                }
            }
            Expr::Name(name, _)
                if name == "tonumber" && self.builtin_global_is_unmodified(name) =>
            {
                Some(StdlibBuiltinAlias::ToNumber)
            }
            _ => None,
        }
    }

    fn local_math_builtin_alias(&self, name: &str) -> Option<MathBuiltinAlias> {
        match self.locals.get(name).copied()? {
            Binding::Register(register) | Binding::CapturedLocal(register) => {
                self.math_builtin_aliases.get(&register).copied()
            }
            Binding::Upvalue(index) => self.math_upvalue_aliases.get(&index).copied(),
            Binding::CurrentFunction => None,
        }
    }

    fn local_stdlib_builtin_alias(&self, name: &str) -> Option<StdlibBuiltinAlias> {
        match self.locals.get(name).copied()? {
            Binding::Register(register) | Binding::CapturedLocal(register) => {
                self.stdlib_builtin_aliases.get(&register).copied()
            }
            Binding::Upvalue(index) => self.stdlib_upvalue_aliases.get(&index).copied(),
            Binding::CurrentFunction => None,
        }
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
        let Some((descriptor, math_alias, stdlib_alias)) =
            self.resolve_upvalue_from_scope(scope_index, name)
        else {
            return Ok(None);
        };
        let upvalue_index = self.intern_current_upvalue(descriptor, name)?;
        if let Some(alias) = math_alias {
            self.math_upvalue_aliases.insert(upvalue_index, alias);
        }
        if let Some(alias) = stdlib_alias {
            self.stdlib_upvalue_aliases.insert(upvalue_index, alias);
        }
        Ok(Some(upvalue_index))
    }

    fn resolve_upvalue_from_scope(
        &mut self,
        scope_index: usize,
        name: &str,
    ) -> Option<(
        UpvalueDescriptor,
        Option<MathBuiltinAlias>,
        Option<StdlibBuiltinAlias>,
    )> {
        if let Some(binding) = self.capture_stack[scope_index].locals.get(name).copied() {
            match binding {
                Binding::Register(register) | Binding::CapturedLocal(register) => {
                    let (math_alias, stdlib_alias) = if self.capture_stack[scope_index]
                        .assigned_local_names
                        .contains(name)
                    {
                        (None, None)
                    } else {
                        (
                            self.capture_stack[scope_index]
                                .math_builtin_aliases
                                .get(&register)
                                .copied(),
                            self.capture_stack[scope_index]
                                .stdlib_builtin_aliases
                                .get(&register)
                                .copied(),
                        )
                    };
                    self.capture_stack[scope_index]
                        .locals
                        .insert(name.to_owned(), Binding::CapturedLocal(register));
                    self.capture_stack[scope_index]
                        .captured_registers
                        .insert(register);
                    return Some((
                        UpvalueDescriptor::ParentLocal(register),
                        math_alias,
                        stdlib_alias,
                    ));
                }
                Binding::Upvalue(index) => {
                    let (math_alias, stdlib_alias) = if self.capture_stack[scope_index]
                        .assigned_local_names
                        .contains(name)
                    {
                        (None, None)
                    } else {
                        (
                            self.capture_stack[scope_index]
                                .math_upvalue_aliases
                                .get(&index)
                                .copied(),
                            self.capture_stack[scope_index]
                                .stdlib_upvalue_aliases
                                .get(&index)
                                .copied(),
                        )
                    };
                    return Some((
                        UpvalueDescriptor::ParentUpvalue(index),
                        math_alias,
                        stdlib_alias,
                    ));
                }
                Binding::CurrentFunction => {}
            }
        }

        let parent_scope_index = scope_index.checked_sub(1)?;
        let (descriptor, math_alias, stdlib_alias) =
            self.resolve_upvalue_from_scope(parent_scope_index, name)?;
        let upvalue_index = self.intern_scope_upvalue(scope_index, descriptor, name)?;
        let (math_alias, stdlib_alias) = if self.capture_stack[scope_index]
            .assigned_local_names
            .contains(name)
        {
            (None, None)
        } else {
            (math_alias, stdlib_alias)
        };
        if let Some(alias) = math_alias {
            self.capture_stack[scope_index]
                .math_upvalue_aliases
                .insert(upvalue_index, alias);
        }
        if let Some(alias) = stdlib_alias {
            self.capture_stack[scope_index]
                .stdlib_upvalue_aliases
                .insert(upvalue_index, alias);
        }
        Some((
            UpvalueDescriptor::ParentUpvalue(upvalue_index),
            math_alias,
            stdlib_alias,
        ))
    }

    fn intern_current_upvalue(&mut self, descriptor: UpvalueDescriptor, name: &str) -> Result<u8> {
        if let Some(Binding::Upvalue(index)) = self.locals.get(name).copied() {
            return Ok(index);
        }
        let index = u8::try_from(self.upvalues.len())
            .map_err(|_| CompileError::new("too many function upvalues"))?;
        self.upvalues.push(descriptor);
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
        let index = u8::try_from(self.capture_stack[scope_index].upvalues.len()).ok()?;
        self.capture_stack[scope_index].upvalues.push(descriptor);
        self.capture_stack[scope_index]
            .locals
            .insert(name.to_owned(), Binding::Upvalue(index));
        Some(index)
    }

    fn global_table_alias_is_unmodified(&self) -> bool {
        self.builtin_global_is_unmodified("_G")
    }

    fn global_table_field_assignment(&self, object: &Expr, field: &str) -> Option<String> {
        if matches!(object, Expr::Name(name, _) if name == "_G")
            && self.global_table_alias_is_unmodified()
        {
            Some(field.to_owned())
        } else {
            None
        }
    }

    fn global_table_index_assignment(&self, object: &Expr, index: &Expr) -> Option<String> {
        if !matches!(object, Expr::Name(name, _) if name == "_G")
            || !self.global_table_alias_is_unmodified()
        {
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
        if !matches!(object, Expr::Name(name, _) if name == "_G")
            || !self.global_table_alias_is_unmodified()
        {
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
                may_mutate_stdlib_table,
                ..
            } => {
                if *may_mutate_stdlib_table {
                    self.stdlib_tables_may_be_mutated = true;
                }
            }
            CompiledAssignmentTarget::Local(register)
            | CompiledAssignmentTarget::CapturedLocal(register) => {
                self.math_builtin_aliases.remove(register);
                self.stdlib_builtin_aliases.remove(register);
                self.numeric_registers.remove(register);
                self.fresh_table_registers.remove(register);
            }
            CompiledAssignmentTarget::Upvalue(index) => {
                self.math_upvalue_aliases.remove(index);
                self.stdlib_upvalue_aliases.remove(index);
            }
        }
    }

    fn mark_table_assignment_mutation(
        &mut self,
        global_field: Option<String>,
        stdlib_field: Option<(String, String)>,
        may_mutate_stdlib_table: bool,
    ) {
        if let Some(name) = global_field {
            self.assigned_globals.insert(name);
        } else if let Some((table, field)) = stdlib_field {
            self.assigned_stdlib_fields.insert((table, field));
        } else if may_mutate_stdlib_table {
            self.stdlib_tables_may_be_mutated = true;
        }
    }

    fn object_may_mutate_stdlib_table(&self, object: &Expr) -> bool {
        match self.simple_register_expr(object) {
            Some(register) => !self.fresh_table_registers.contains(&register),
            None => true,
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
                self.code.push(Instruction::with_aux(
                    Opcode::SetUpvalue,
                    source,
                    0,
                    0,
                    i32::from(*index),
                ));
            }
            CompiledAssignmentTarget::Table { table, key, .. } => {
                let opcode = table_write_opcode_for_assignment_target(target);
                self.code
                    .push(Instruction::new(opcode, *table, *key, source));
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
                self.free_register(key);
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

    fn free_register_without_clear(&mut self, register: u8) {
        if self.next_register == u16::from(register) + 1 {
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
                return;
            }
        }
        self.source_map.push(InstructionSource { pc, line });
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
        self.code.push(Instruction::new(
            Opcode::LoadVarargs,
            first_register,
            u8::try_from(result_count)
                .map_err(|_| CompileError::at("vararg expression has too many results", span))?,
            0,
        ));
        Ok(())
    }

    fn call_requires_scalar_return_lowering(&self, expr: &Expr) -> bool {
        let Expr::Call { callee, .. } = expr else {
            return false;
        };
        if let Expr::Field { object, field, .. } = callee.as_ref() {
            let Expr::Name(table, _) = object.as_ref() else {
                return false;
            };
            if table == "table" && matches!(field.as_str(), "unpack" | "insert" | "sort") {
                return false;
            }
            if table == "string"
                && matches!(
                    field.as_str(),
                    "byte" | "find" | "gsub" | "match" | "unpack"
                )
            {
                return false;
            }
            if table == "math" && matches!(field.as_str(), "frexp" | "modf" | "randomseed") {
                return false;
            }
            return matches!(table.as_str(), "math" | "string" | "table")
                && self.builtin_table_field_is_unmodified(table, field);
        }
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
            _ => expr_is_call(expr) && !self.call_requires_scalar_return_lowering(expr),
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
        expr_is_call(last) && !self.call_requires_scalar_return_lowering(last)
    }

    fn method_call_has_expandable_trailing_arg(&self, args: &[Expr]) -> bool {
        let Some(last) = args.last() else {
            return false;
        };
        expr_is_call(last) && !self.call_requires_scalar_return_lowering(last)
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

fn const_array_index_operand(expr: &Expr) -> Option<i32> {
    let Some(Constant::Number(value)) = const_eval(expr) else {
        return None;
    };
    if value.is_finite() && value.fract() == 0.0 && (1.0..=f64::from(i32::MAX)).contains(&value) {
        Some(value as i32)
    } else {
        None
    }
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

#[derive(Default)]
struct GlobalMutationSummary {
    assigned_globals: BTreeSet<String>,
    assigned_stdlib_fields: BTreeSet<(String, String)>,
    stdlib_tables_may_be_mutated: bool,
    environment_escaped: bool,
}

#[derive(Default)]
struct MutationScope {
    locals: BTreeSet<String>,
    fresh_table_locals: BTreeSet<String>,
}

#[derive(Default)]
struct GlobalMutationScanner {
    summary: GlobalMutationSummary,
    scopes: Vec<MutationScope>,
}

fn global_mutation_summary_in_statements(statements: &[Stmt]) -> GlobalMutationSummary {
    let mut scanner = GlobalMutationScanner::new();
    scanner.collect_statements(statements);
    scanner.summary
}

impl GlobalMutationScanner {
    fn new() -> Self {
        Self {
            summary: GlobalMutationSummary::default(),
            scopes: vec![MutationScope::default()],
        }
    }

    fn collect_statements(&mut self, statements: &[Stmt]) {
        for statement in statements {
            self.collect_statement(statement);
        }
    }

    fn collect_scoped_statements(&mut self, statements: &[Stmt]) {
        self.scopes.push(MutationScope::default());
        self.collect_statements(statements);
        self.scopes
            .pop()
            .expect("mutation scanner scope stack should be balanced");
    }

    fn collect_function_body(&mut self, params: &[FunctionParam], body: &[Stmt], is_method: bool) {
        self.scopes.push(MutationScope::default());
        if is_method {
            self.declare_local("self", false);
        }
        for param in params {
            self.declare_local(&param.name, false);
        }
        self.collect_statements(body);
        self.scopes
            .pop()
            .expect("mutation scanner function scope should be balanced");
    }

    fn collect_statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Local { bindings, init, .. } => {
                for expr in init {
                    self.collect_expr(expr);
                }
                for (index, binding) in bindings.iter().enumerate() {
                    let is_fresh_table = init.get(index).is_some_and(|expr| {
                        matches!(expr_without_parens(expr), Expr::Table { .. })
                    });
                    self.declare_local(&binding.name, is_fresh_table);
                }
            }
            Stmt::LocalFunction {
                name, params, body, ..
            } => {
                self.declare_local(name, false);
                self.collect_function_body(params, body, false);
            }
            Stmt::Function {
                target,
                is_method,
                params,
                body,
                ..
            } => {
                self.collect_target(target);
                self.collect_function_body(params, body, *is_method);
            }
            Stmt::Assignment {
                targets, values, ..
            } => {
                for target in targets {
                    self.collect_target(target);
                }
                for value in values {
                    self.collect_expr(value);
                }
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                self.collect_expr(condition);
                self.collect_scoped_statements(then_body);
                self.collect_scoped_statements(else_body);
            }
            Stmt::While {
                condition, body, ..
            } => {
                self.collect_expr(condition);
                self.collect_scoped_statements(body);
            }
            Stmt::ForNumeric {
                variable,
                start,
                limit,
                step,
                body,
                ..
            } => {
                self.collect_expr(start);
                self.collect_expr(limit);
                if let Some(step) = step {
                    self.collect_expr(step);
                }
                self.scopes.push(MutationScope::default());
                self.declare_local(variable, false);
                self.collect_statements(body);
                self.scopes
                    .pop()
                    .expect("mutation scanner for scope should be balanced");
            }
            Stmt::ForGeneric {
                variables,
                iter,
                body,
                ..
            } => {
                for expr in iter {
                    self.collect_expr(expr);
                }
                self.scopes.push(MutationScope::default());
                for variable in variables {
                    self.declare_local(&variable.name, false);
                }
                self.collect_statements(body);
                self.scopes
                    .pop()
                    .expect("mutation scanner for scope should be balanced");
            }
            Stmt::Repeat {
                body, condition, ..
            } => {
                self.scopes.push(MutationScope::default());
                self.collect_statements(body);
                self.collect_expr(condition);
                self.scopes
                    .pop()
                    .expect("mutation scanner repeat scope should be balanced");
            }
            Stmt::Do { body, .. } => self.collect_scoped_statements(body),
            Stmt::Return {
                value,
                extra_values,
                ..
            } => {
                if let Some(value) = value {
                    self.collect_expr(value);
                }
                for value in extra_values {
                    self.collect_expr(value);
                }
            }
            Stmt::Expr { expr, .. } => self.collect_expr(expr),
            Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::TypeAlias { .. } => {}
        }
    }

    fn collect_target(&mut self, target: &AssignmentTarget) {
        match target {
            AssignmentTarget::Local { name, .. } => {
                if let Some(scope_index) = self.local_scope_index(name) {
                    self.scopes[scope_index].fresh_table_locals.remove(name);
                } else {
                    self.summary.assigned_globals.insert(name.clone());
                }
            }
            AssignmentTarget::Field { object, field, .. } => {
                self.mark_table_field_assignment(object, Some(field));
                self.collect_expr(object);
            }
            AssignmentTarget::Index { object, index, .. } => {
                let field = const_eval(index).and_then(|constant| match constant {
                    Constant::String(field) => String::from_utf8(field).ok(),
                    _ => None,
                });
                self.mark_table_field_assignment(object, field.as_deref());
                self.collect_expr(object);
                self.collect_expr(index);
            }
        }
    }

    fn collect_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Nil(_) | Expr::Bool(_, _) | Expr::Number(_, _) | Expr::String(_, _) => {}
            Expr::InterpolatedString { expressions, .. } => {
                for expr in expressions {
                    self.collect_expr(expr);
                }
            }
            Expr::Name(_, _) | Expr::Varargs(_) => {}
            Expr::Unary { expr, .. } => self.collect_expr(expr),
            Expr::Binary { lhs, rhs, .. } => {
                self.collect_expr(lhs);
                self.collect_expr(rhs);
            }
            Expr::IfElse {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.collect_expr(condition);
                self.collect_expr(then_expr);
                self.collect_expr(else_expr);
            }
            Expr::Field { object, .. } => self.collect_expr(object),
            Expr::Index { object, index, .. } => {
                self.collect_expr(object);
                self.collect_expr(index);
            }
            Expr::Table { fields, .. } => {
                for field in fields {
                    match field {
                        TableField::Array { value, .. } | TableField::Named { value, .. } => {
                            self.collect_expr(value);
                        }
                        TableField::Keyed { key, value, .. } => {
                            self.collect_expr(key);
                            self.collect_expr(value);
                        }
                    }
                }
            }
            Expr::Function { params, body, .. } => self.collect_function_body(params, body, false),
            Expr::Paren { expr, .. } => self.collect_expr(expr),
            Expr::Call { callee, args, .. } => {
                self.collect_global_call_escape(callee, args);
                self.collect_expr(callee);
                for arg in args {
                    self.collect_expr(arg);
                }
            }
            Expr::MethodCall { object, args, .. } => {
                self.collect_expr(object);
                for arg in args {
                    self.collect_expr(arg);
                }
            }
        }
    }

    fn collect_global_call_escape(&mut self, callee: &Expr, args: &[Expr]) {
        let Expr::Name(name, _) = callee else {
            return;
        };
        if name == "getfenv" || name == "setfenv" {
            self.summary.environment_escaped = true;
        }
        if name != "rawset" {
            return;
        }
        let Some(object) = args.first() else {
            return;
        };
        let Some(index) = args.get(1) else {
            self.summary.environment_escaped = true;
            return;
        };
        if self.object_is_unshadowed_global(object, "_G") {
            match const_eval(index) {
                Some(Constant::String(name)) => match String::from_utf8(name) {
                    Ok(name) => {
                        self.summary.assigned_globals.insert(name);
                    }
                    Err(_) => self.summary.environment_escaped = true,
                },
                _ => self.summary.environment_escaped = true,
            }
            return;
        }
        let field = const_eval(index).and_then(|constant| match constant {
            Constant::String(field) => String::from_utf8(field).ok(),
            _ => None,
        });
        self.mark_table_field_assignment(object, field.as_deref());
    }

    fn mark_table_field_assignment(&mut self, object: &Expr, field: Option<&str>) {
        if self.object_is_unshadowed_global(object, "_G") {
            match field {
                Some(field) => {
                    self.summary.assigned_globals.insert(field.to_owned());
                }
                None => self.summary.environment_escaped = true,
            }
            return;
        }
        if let Some(table) = self.unshadowed_stdlib_table_name(object) {
            if let Some(field) = field {
                self.summary
                    .assigned_stdlib_fields
                    .insert((table.to_owned(), field.to_owned()));
            } else {
                self.summary.stdlib_tables_may_be_mutated = true;
            }
            return;
        }
        if self.object_may_mutate_stdlib_table(object) {
            self.summary.stdlib_tables_may_be_mutated = true;
        }
    }

    fn declare_local(&mut self, name: &str, fresh_table: bool) {
        let scope = self
            .scopes
            .last_mut()
            .expect("mutation scanner should always have a scope");
        scope.locals.insert(name.to_owned());
        if fresh_table {
            scope.fresh_table_locals.insert(name.to_owned());
        } else {
            scope.fresh_table_locals.remove(name);
        }
    }

    fn local_scope_index(&self, name: &str) -> Option<usize> {
        self.scopes
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, scope)| scope.locals.contains(name).then_some(index))
    }

    fn local_is_fresh_table(&self, name: &str) -> bool {
        self.local_scope_index(name)
            .is_some_and(|index| self.scopes[index].fresh_table_locals.contains(name))
    }

    fn object_is_unshadowed_global(&self, object: &Expr, global: &str) -> bool {
        matches!(object, Expr::Name(name, _) if name == global && self.local_scope_index(name).is_none())
    }

    fn unshadowed_stdlib_table_name<'a>(&self, object: &'a Expr) -> Option<&'a str> {
        let Expr::Name(name, _) = object else {
            return None;
        };
        matches!(name.as_str(), "math" | "string" | "table")
            .then_some(name.as_str())
            .filter(|_| self.local_scope_index(name).is_none())
    }

    fn object_may_mutate_stdlib_table(&self, object: &Expr) -> bool {
        match object {
            Expr::Name(name, _) if self.local_is_fresh_table(name) => false,
            _ => true,
        }
    }
}

fn assigned_local_names_in_statements(statements: &[Stmt]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    collect_assigned_local_names_from_statements(statements, &mut names);
    names
}

fn collect_assigned_local_names_from_statements(statements: &[Stmt], names: &mut BTreeSet<String>) {
    for statement in statements {
        collect_assigned_local_names_from_statement(statement, names);
    }
}

fn collect_assigned_local_names_from_statement(statement: &Stmt, names: &mut BTreeSet<String>) {
    match statement {
        Stmt::Local { init, .. } => {
            for expr in init {
                collect_assigned_local_names_from_expr(expr, names);
            }
        }
        Stmt::LocalFunction { body, .. } => {
            collect_assigned_local_names_from_statements(body, names);
        }
        Stmt::Function {
            target,
            body,
            params: _,
            ..
        } => {
            collect_assigned_local_names_from_target(target, names);
            collect_assigned_local_names_from_statements(body, names);
        }
        Stmt::Assignment {
            targets, values, ..
        } => {
            for target in targets {
                collect_assigned_local_names_from_target(target, names);
            }
            for value in values {
                collect_assigned_local_names_from_expr(value, names);
            }
        }
        Stmt::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            collect_assigned_local_names_from_expr(condition, names);
            collect_assigned_local_names_from_statements(then_body, names);
            collect_assigned_local_names_from_statements(else_body, names);
        }
        Stmt::While {
            condition, body, ..
        } => {
            collect_assigned_local_names_from_expr(condition, names);
            collect_assigned_local_names_from_statements(body, names);
        }
        Stmt::ForNumeric {
            start,
            limit,
            step,
            body,
            ..
        } => {
            collect_assigned_local_names_from_expr(start, names);
            collect_assigned_local_names_from_expr(limit, names);
            if let Some(step) = step {
                collect_assigned_local_names_from_expr(step, names);
            }
            collect_assigned_local_names_from_statements(body, names);
        }
        Stmt::ForGeneric { iter, body, .. } => {
            for expr in iter {
                collect_assigned_local_names_from_expr(expr, names);
            }
            collect_assigned_local_names_from_statements(body, names);
        }
        Stmt::Repeat {
            body, condition, ..
        } => {
            collect_assigned_local_names_from_statements(body, names);
            collect_assigned_local_names_from_expr(condition, names);
        }
        Stmt::Do { body, .. } => {
            collect_assigned_local_names_from_statements(body, names);
        }
        Stmt::Return {
            value,
            extra_values,
            ..
        } => {
            if let Some(value) = value {
                collect_assigned_local_names_from_expr(value, names);
            }
            for value in extra_values {
                collect_assigned_local_names_from_expr(value, names);
            }
        }
        Stmt::Expr { expr, .. } => {
            collect_assigned_local_names_from_expr(expr, names);
        }
        Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::TypeAlias { .. } => {}
    }
}

fn collect_assigned_local_names_from_target(
    target: &AssignmentTarget,
    names: &mut BTreeSet<String>,
) {
    match target {
        AssignmentTarget::Local { name, .. } => {
            names.insert(name.clone());
        }
        AssignmentTarget::Field { object, .. } => {
            collect_assigned_local_names_from_expr(object, names);
        }
        AssignmentTarget::Index { object, index, .. } => {
            collect_assigned_local_names_from_expr(object, names);
            collect_assigned_local_names_from_expr(index, names);
        }
    }
}

fn collect_assigned_local_names_from_expr(expr: &Expr, names: &mut BTreeSet<String>) {
    match expr {
        Expr::Nil(_) | Expr::Bool(_, _) | Expr::Number(_, _) | Expr::String(_, _) => {}
        Expr::InterpolatedString { expressions, .. } => {
            for expr in expressions {
                collect_assigned_local_names_from_expr(expr, names);
            }
        }
        Expr::Name(_, _) | Expr::Varargs(_) => {}
        Expr::Unary { expr, .. } => collect_assigned_local_names_from_expr(expr, names),
        Expr::Binary { lhs, rhs, .. } => {
            collect_assigned_local_names_from_expr(lhs, names);
            collect_assigned_local_names_from_expr(rhs, names);
        }
        Expr::IfElse {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            collect_assigned_local_names_from_expr(condition, names);
            collect_assigned_local_names_from_expr(then_expr, names);
            collect_assigned_local_names_from_expr(else_expr, names);
        }
        Expr::Field { object, .. } => collect_assigned_local_names_from_expr(object, names),
        Expr::Index { object, index, .. } => {
            collect_assigned_local_names_from_expr(object, names);
            collect_assigned_local_names_from_expr(index, names);
        }
        Expr::Table { fields, .. } => {
            for field in fields {
                match field {
                    TableField::Array { value, .. } | TableField::Named { value, .. } => {
                        collect_assigned_local_names_from_expr(value, names);
                    }
                    TableField::Keyed { key, value, .. } => {
                        collect_assigned_local_names_from_expr(key, names);
                        collect_assigned_local_names_from_expr(value, names);
                    }
                }
            }
        }
        Expr::Function { body, .. } => {
            collect_assigned_local_names_from_statements(body, names);
        }
        Expr::Paren { expr, .. } => collect_assigned_local_names_from_expr(expr, names),
        Expr::Call { callee, args, .. } => {
            collect_assigned_local_names_from_expr(callee, names);
            for arg in args {
                collect_assigned_local_names_from_expr(arg, names);
            }
        }
        Expr::MethodCall { object, args, .. } => {
            collect_assigned_local_names_from_expr(object, names);
            for arg in args {
                collect_assigned_local_names_from_expr(arg, names);
            }
        }
    }
}

fn expr_is_call(expr: &Expr) -> bool {
    matches!(expr, Expr::Call { .. } | Expr::MethodCall { .. })
}

fn expr_without_parens(expr: &Expr) -> &Expr {
    match expr {
        Expr::Paren { expr, .. } => expr_without_parens(expr),
        _ => expr,
    }
}

fn expr_name_without_parens(expr: &Expr) -> Option<&str> {
    match expr_without_parens(expr) {
        Expr::Name(name, _) => Some(name),
        _ => None,
    }
}

fn local_table_array_capacity_hints(
    bindings: &[rustia_parser::LocalBinding],
    following: &[Stmt],
    number_constants: &BTreeMap<String, f64>,
) -> BTreeMap<String, TableCapacityHint> {
    let mut hints = BTreeMap::new();
    let batch_hints = following_loop_table_reset_capacity_hints(following, number_constants);
    for binding in bindings {
        let mut hint = following
            .first()
            .map(|next| local_table_capacity_hint(&binding.name, next, number_constants))
            .unwrap_or_default();
        if hint.is_empty() {
            hint = batch_hints.get(&binding.name).copied().unwrap_or_default();
        }
        if !hint.is_empty() {
            hints.insert(binding.name.clone(), hint);
        }
    }
    hints
}

fn local_table_capacity_hint(
    table_name: &str,
    next: &Stmt,
    number_constants: &BTreeMap<String, f64>,
) -> TableCapacityHint {
    let Stmt::ForNumeric {
        variable,
        start,
        limit,
        step,
        body,
        ..
    } = next
    else {
        return TableCapacityHint::default();
    };
    let array = numeric_for_table_index_offset(table_name, variable, body)
        .and_then(|offset| {
            numeric_for_array_capacity(start, limit, step.as_ref(), offset, number_constants)
        })
        .unwrap_or(0);
    let hash_writes = numeric_for_table_hash_write_count(
        table_name,
        variable,
        body,
        start,
        limit,
        step.as_ref(),
        number_constants,
    );
    let hash = numeric_for_iteration_count(start, limit, step.as_ref(), number_constants)
        .and_then(|iterations| iterations.checked_mul(hash_writes))
        .unwrap_or(0);
    TableCapacityHint { array, hash }
}

fn following_loop_table_reset_capacity_hints(
    following: &[Stmt],
    number_constants: &BTreeMap<String, f64>,
) -> BTreeMap<String, TableCapacityHint> {
    for statement in following {
        match statement {
            Stmt::Local { .. } | Stmt::TypeAlias { .. } => {}
            Stmt::ForNumeric { body, .. } => {
                return loop_table_reset_capacity_hints(body, number_constants);
            }
            _ => return BTreeMap::new(),
        }
    }
    BTreeMap::new()
}

fn loop_table_reset_capacity_hints(
    body: &[Stmt],
    number_constants: &BTreeMap<String, f64>,
) -> BTreeMap<String, TableCapacityHint> {
    let mut indexed_writes = BTreeMap::new();
    collect_counter_indexed_table_writes(body, &mut indexed_writes);

    let mut hints = BTreeMap::new();
    collect_threshold_reset_table_hints(body, &indexed_writes, number_constants, &mut hints);
    hints
}

fn collect_counter_indexed_table_writes(
    statements: &[Stmt],
    indexed_writes: &mut BTreeMap<String, BTreeSet<String>>,
) {
    for statement in statements {
        match statement {
            Stmt::Assignment {
                op: AssignmentOp::Assign,
                targets,
                ..
            } => {
                for target in targets {
                    let AssignmentTarget::Index { object, index, .. } = target else {
                        continue;
                    };
                    let (Some(table), Some(counter)) = (
                        expr_name_without_parens(object),
                        expr_name_without_parens(index),
                    ) else {
                        continue;
                    };
                    indexed_writes
                        .entry(counter.to_owned())
                        .or_default()
                        .insert(table.to_owned());
                }
            }
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_counter_indexed_table_writes(then_body, indexed_writes);
                collect_counter_indexed_table_writes(else_body, indexed_writes);
            }
            Stmt::While { body, .. }
            | Stmt::ForNumeric { body, .. }
            | Stmt::ForGeneric { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::Do { body, .. } => collect_counter_indexed_table_writes(body, indexed_writes),
            _ => {}
        }
    }
}

fn collect_threshold_reset_table_hints(
    statements: &[Stmt],
    indexed_writes: &BTreeMap<String, BTreeSet<String>>,
    number_constants: &BTreeMap<String, f64>,
    hints: &mut BTreeMap<String, TableCapacityHint>,
) {
    for statement in statements {
        match statement {
            Stmt::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                if let Some((counter, capacity)) =
                    table_reset_threshold_condition(condition, number_constants)
                {
                    if let Some(tables) = indexed_writes.get(&counter) {
                        for reset in empty_table_reset_names(then_body)
                            .into_iter()
                            .chain(empty_table_reset_names(else_body))
                        {
                            if tables.contains(&reset) {
                                let entry = hints.entry(reset).or_default();
                                entry.array = entry.array.max(capacity);
                            }
                        }
                    }
                }
                collect_threshold_reset_table_hints(
                    then_body,
                    indexed_writes,
                    number_constants,
                    hints,
                );
                collect_threshold_reset_table_hints(
                    else_body,
                    indexed_writes,
                    number_constants,
                    hints,
                );
            }
            Stmt::While { body, .. }
            | Stmt::ForNumeric { body, .. }
            | Stmt::ForGeneric { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::Do { body, .. } => {
                collect_threshold_reset_table_hints(body, indexed_writes, number_constants, hints);
            }
            _ => {}
        }
    }
}

fn empty_table_reset_names(statements: &[Stmt]) -> Vec<String> {
    let mut names = Vec::new();
    collect_empty_table_reset_names(statements, &mut names);
    names
}

fn collect_empty_table_reset_names(statements: &[Stmt], names: &mut Vec<String>) {
    for statement in statements {
        match statement {
            Stmt::Assignment {
                op: AssignmentOp::Assign,
                targets,
                values,
                ..
            } if targets.len() == 1 && values.len() == 1 => {
                let AssignmentTarget::Local { name, .. } = &targets[0] else {
                    continue;
                };
                if matches!(expr_without_parens(&values[0]), Expr::Table { fields, .. } if fields.is_empty())
                {
                    names.push(name.clone());
                }
            }
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_empty_table_reset_names(then_body, names);
                collect_empty_table_reset_names(else_body, names);
            }
            Stmt::While { body, .. }
            | Stmt::ForNumeric { body, .. }
            | Stmt::ForGeneric { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::Do { body, .. } => collect_empty_table_reset_names(body, names),
            _ => {}
        }
    }
}

fn table_reset_threshold_condition(
    condition: &Expr,
    number_constants: &BTreeMap<String, f64>,
) -> Option<(String, usize)> {
    let Expr::Binary { op, lhs, rhs, .. } = expr_without_parens(condition) else {
        return None;
    };
    match op {
        BinaryOp::Ge => name_at_least_threshold(lhs, rhs, true, number_constants),
        BinaryOp::Gt => name_at_least_threshold(lhs, rhs, false, number_constants),
        BinaryOp::Le => name_at_least_threshold(rhs, lhs, true, number_constants),
        BinaryOp::Lt => name_at_least_threshold(rhs, lhs, false, number_constants),
        _ => None,
    }
}

fn name_at_least_threshold(
    name_expr: &Expr,
    threshold_expr: &Expr,
    inclusive: bool,
    number_constants: &BTreeMap<String, f64>,
) -> Option<(String, usize)> {
    let name = expr_name_without_parens(name_expr)?.to_owned();
    let mut threshold =
        integer_number(constant_or_local_number(threshold_expr, number_constants)?)?;
    if !inclusive {
        threshold = threshold.checked_add(1)?;
    }
    if threshold <= 0 {
        return None;
    }
    Some((name, usize::try_from(threshold).ok()?))
}

fn numeric_for_table_index_offset(
    table_name: &str,
    loop_variable: &str,
    body: &[Stmt],
) -> Option<f64> {
    body.iter()
        .filter_map(|stmt| stmt_table_index_offset(table_name, loop_variable, stmt))
        .max_by(|left, right| left.total_cmp(right))
}

fn stmt_table_index_offset(table_name: &str, loop_variable: &str, stmt: &Stmt) -> Option<f64> {
    match stmt {
        Stmt::Assignment {
            op: AssignmentOp::Assign,
            targets,
            ..
        } => targets.iter().find_map(|target| {
            let AssignmentTarget::Index { object, index, .. } = target else {
                return None;
            };
            (expr_name_without_parens(object) == Some(table_name))
                .then(|| table_index_loop_offset(index, loop_variable))
                .flatten()
        }),
        Stmt::If {
            then_body,
            else_body,
            ..
        } => numeric_for_table_index_offset(table_name, loop_variable, then_body)
            .or_else(|| numeric_for_table_index_offset(table_name, loop_variable, else_body)),
        Stmt::Do { body, .. } => numeric_for_table_index_offset(table_name, loop_variable, body),
        _ => None,
    }
}

fn numeric_for_table_hash_write_count(
    table_name: &str,
    loop_variable: &str,
    body: &[Stmt],
    start: &Expr,
    limit: &Expr,
    step: Option<&Expr>,
    number_constants: &BTreeMap<String, f64>,
) -> usize {
    body.iter()
        .map(|stmt| {
            stmt_table_hash_write_count(
                table_name,
                loop_variable,
                stmt,
                start,
                limit,
                step,
                number_constants,
            )
        })
        .sum()
}

fn stmt_table_hash_write_count(
    table_name: &str,
    loop_variable: &str,
    stmt: &Stmt,
    start: &Expr,
    limit: &Expr,
    step: Option<&Expr>,
    number_constants: &BTreeMap<String, f64>,
) -> usize {
    match stmt {
        Stmt::Assignment {
            op: AssignmentOp::Assign,
            targets,
            ..
        } => targets
            .iter()
            .filter(|target| {
                let AssignmentTarget::Index { object, index, .. } = target else {
                    return false;
                };
                if expr_name_without_parens(object) != Some(table_name) {
                    return false;
                }
                if table_index_loop_offset(index, loop_variable).is_some() {
                    return false;
                }
                !table_index_is_dense_numeric_for_write(
                    index,
                    loop_variable,
                    start,
                    limit,
                    step,
                    number_constants,
                )
            })
            .count(),
        Stmt::If {
            then_body,
            else_body,
            ..
        } => numeric_for_table_hash_write_count(
            table_name,
            loop_variable,
            then_body,
            start,
            limit,
            step,
            number_constants,
        )
        .max(numeric_for_table_hash_write_count(
            table_name,
            loop_variable,
            else_body,
            start,
            limit,
            step,
            number_constants,
        )),
        Stmt::Do { body, .. } => numeric_for_table_hash_write_count(
            table_name,
            loop_variable,
            body,
            start,
            limit,
            step,
            number_constants,
        ),
        _ => 0,
    }
}

fn table_index_is_dense_numeric_for_write(
    index: &Expr,
    loop_variable: &str,
    start: &Expr,
    limit: &Expr,
    step: Option<&Expr>,
    number_constants: &BTreeMap<String, f64>,
) -> bool {
    table_index_loop_offset(index, loop_variable)
        .and_then(|offset| numeric_for_array_capacity(start, limit, step, offset, number_constants))
        .is_some()
}

fn table_index_loop_offset(index: &Expr, loop_variable: &str) -> Option<f64> {
    match expr_without_parens(index) {
        Expr::Name(name, _) if name == loop_variable => Some(0.0),
        Expr::Binary {
            op: BinaryOp::Add,
            lhs,
            rhs,
            ..
        } => {
            if expr_name_without_parens(lhs) == Some(loop_variable) {
                return constant_number(rhs);
            }
            if expr_name_without_parens(rhs) == Some(loop_variable) {
                return constant_number(lhs);
            }
            None
        }
        Expr::Binary {
            op: BinaryOp::Sub,
            lhs,
            rhs,
            ..
        } if expr_name_without_parens(lhs) == Some(loop_variable) => {
            constant_number(rhs).map(|offset| -offset)
        }
        _ => None,
    }
}

fn numeric_for_array_capacity(
    start: &Expr,
    limit: &Expr,
    step: Option<&Expr>,
    offset: f64,
    number_constants: &BTreeMap<String, f64>,
) -> Option<usize> {
    let start = integer_number(constant_or_local_number(start, number_constants)?)?;
    let limit = integer_number(constant_or_local_number(limit, number_constants)?)?;
    let step = match step {
        Some(step) => integer_number(constant_or_local_number(step, number_constants)?)?,
        None => 1,
    };
    let offset = integer_number(offset)?;
    if step != 1 || limit < start {
        return None;
    }
    let first_index = start.checked_add(offset)?;
    let last_index = limit.checked_add(offset)?;
    if first_index != 1 || last_index < first_index {
        return None;
    }
    usize::try_from(last_index).ok()
}

fn numeric_for_iteration_count(
    start: &Expr,
    limit: &Expr,
    step: Option<&Expr>,
    number_constants: &BTreeMap<String, f64>,
) -> Option<usize> {
    let start = integer_number(constant_or_local_number(start, number_constants)?)?;
    let limit = integer_number(constant_or_local_number(limit, number_constants)?)?;
    let step = match step {
        Some(step) => integer_number(constant_or_local_number(step, number_constants)?)?,
        None => 1,
    };
    if step != 1 || limit < start {
        return None;
    }
    usize::try_from(limit.checked_sub(start)?.checked_add(1)?).ok()
}

fn table_capacity_hint_operand(hint: TableCapacityHint) -> Result<i32> {
    let array = hint.array.min(MAX_TABLE_ARRAY_CAPACITY_HINT);
    let hash = hint.hash.min(MAX_TABLE_HASH_CAPACITY_HINT);
    if hash == 0 {
        return i32::try_from(array).map_err(|_| CompileError::new("table capacity hint overflow"));
    }
    if array == 0 {
        let hash =
            i32::try_from(hash).map_err(|_| CompileError::new("table capacity hint overflow"))?;
        return hash
            .checked_neg()
            .ok_or_else(|| CompileError::new("table capacity hint overflow"));
    }
    let packed = array
        .checked_shl(16)
        .and_then(|array| array.checked_add(hash))
        .ok_or_else(|| CompileError::new("table capacity hint overflow"))?;
    let packed =
        i32::try_from(packed).map_err(|_| CompileError::new("table capacity hint overflow"))?;
    packed
        .checked_neg()
        .ok_or_else(|| CompileError::new("table capacity hint overflow"))
}

fn constant_number(expr: &Expr) -> Option<f64> {
    let Constant::Number(value) = const_eval(expr)? else {
        return None;
    };
    Some(value)
}

fn constant_or_local_number(expr: &Expr, number_constants: &BTreeMap<String, f64>) -> Option<f64> {
    constant_number(expr).or_else(|| {
        expr_name_without_parens(expr).and_then(|name| number_constants.get(name).copied())
    })
}

fn integer_number(value: f64) -> Option<i64> {
    if value.is_finite()
        && value.fract() == 0.0
        && value >= i64::MIN as f64
        && value <= i64::MAX as f64
    {
        Some(value as i64)
    } else {
        None
    }
}

fn len_expr_name(expr: &Expr) -> Option<&str> {
    let Expr::Unary {
        op: UnaryOp::Len,
        expr,
        ..
    } = expr_without_parens(expr)
    else {
        return None;
    };
    expr_name_without_parens(expr)
}

fn expr_is_one(expr: &Expr) -> bool {
    matches!(expr_without_parens(expr), Expr::Number(value, _) if *value == 1.0)
}

fn single_byte_string_literal(expr: &Expr) -> Option<u8> {
    let Expr::String(bytes, _) = expr_without_parens(expr) else {
        return None;
    };
    if bytes.len() == 1 {
        Some(bytes[0])
    } else {
        None
    }
}

fn local_self_increment_rhs<'a>(name: &str, expr: &'a Expr) -> Option<&'a Expr> {
    let Expr::Binary { op, lhs, rhs, .. } = expr_without_parens(expr) else {
        return None;
    };
    if *op == BinaryOp::Add && expr_name_without_parens(lhs) == Some(name) && expr_is_one(rhs) {
        Some(rhs)
    } else {
        None
    }
}

fn upvalue_self_add_rhs_name<'a>(name: &str, expr: &'a Expr) -> Option<&'a str> {
    let Expr::Binary { op, lhs, rhs, .. } = expr_without_parens(expr) else {
        return None;
    };
    if *op == BinaryOp::Add && expr_name_without_parens(lhs) == Some(name) {
        expr_name_without_parens(rhs)
    } else {
        None
    }
}

fn local_self_binary_rhs_operands<'a>(name: &str, expr: &'a Expr) -> Option<Vec<&'a Expr>> {
    let Expr::Binary { op, lhs, rhs, .. } = expr_without_parens(expr) else {
        return None;
    };
    if *op == BinaryOp::Add && expr_name_without_parens(lhs) == Some(name) {
        if local_self_binary_rhs_is_direct_read(rhs) && !expr_contains_function_literal(rhs) {
            return Some(vec![rhs.as_ref()]);
        }
        return None;
    }

    let operands = local_self_add_chain_operands(name, expr)?;
    if operands.len() < 2
        || !operands
            .iter()
            .any(|operand| local_self_binary_rhs_is_read_or_zero(operand))
    {
        return None;
    }
    if operands
        .iter()
        .any(|operand| !local_self_binary_chain_rhs_is_supported(operand))
    {
        return None;
    }
    if operands
        .iter()
        .any(|operand| expr_contains_function_literal(operand) || expr_contains_call(operand))
    {
        return None;
    }
    if operands
        .iter()
        .skip(1)
        .any(|operand| expr_references_name(operand, name))
    {
        return None;
    }
    Some(operands)
}

fn local_self_add_chain_operands<'a>(name: &str, expr: &'a Expr) -> Option<Vec<&'a Expr>> {
    let mut operands = Vec::new();
    let mut current = expr_without_parens(expr);
    loop {
        let Expr::Binary { op, lhs, rhs, .. } = current else {
            return None;
        };
        if *op != BinaryOp::Add {
            return None;
        }
        operands.push(rhs.as_ref());
        if expr_name_without_parens(lhs) == Some(name) {
            operands.reverse();
            return Some(operands);
        }
        current = expr_without_parens(lhs);
    }
}

fn local_self_binary_rhs_is_direct_read(expr: &Expr) -> bool {
    matches!(
        expr_without_parens(expr),
        Expr::Name(_, _)
            | Expr::Field { .. }
            | Expr::Index { .. }
            | Expr::Call { .. }
            | Expr::MethodCall { .. }
    )
}

fn local_self_binary_chain_rhs_is_supported(expr: &Expr) -> bool {
    match expr_without_parens(expr) {
        Expr::Name(_, _) | Expr::Field { .. } | Expr::Index { .. } => true,
        Expr::Binary {
            op: BinaryOp::Or,
            lhs,
            rhs,
            ..
        } => local_self_binary_chain_rhs_is_supported(lhs) && expr_is_zero(rhs),
        _ => false,
    }
}

fn local_self_binary_rhs_is_read_or_zero(expr: &Expr) -> bool {
    let Expr::Binary {
        op: BinaryOp::Or,
        lhs,
        rhs,
        ..
    } = expr_without_parens(expr)
    else {
        return false;
    };
    local_self_binary_chain_rhs_is_supported(lhs) && expr_is_zero(rhs)
}

fn expr_is_zero(expr: &Expr) -> bool {
    matches!(expr_without_parens(expr), Expr::Number(value, _) if *value == 0.0)
}

fn expr_contains_call(expr: &Expr) -> bool {
    match expr_without_parens(expr) {
        Expr::Nil(_) | Expr::Bool(_, _) | Expr::Number(_, _) | Expr::String(_, _) => false,
        Expr::InterpolatedString { expressions, .. } => expressions.iter().any(expr_contains_call),
        Expr::Name(_, _) | Expr::Varargs(_) => false,
        Expr::Unary { expr, .. } => expr_contains_call(expr),
        Expr::Binary { lhs, rhs, .. } => expr_contains_call(lhs) || expr_contains_call(rhs),
        Expr::IfElse {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            expr_contains_call(condition)
                || expr_contains_call(then_expr)
                || expr_contains_call(else_expr)
        }
        Expr::Field { object, .. } => expr_contains_call(object),
        Expr::Index { object, index, .. } => {
            expr_contains_call(object) || expr_contains_call(index)
        }
        Expr::Table { fields, .. } => fields.iter().any(|field| match field {
            TableField::Array { value, .. } | TableField::Named { value, .. } => {
                expr_contains_call(value)
            }
            TableField::Keyed { key, value, .. } => {
                expr_contains_call(key) || expr_contains_call(value)
            }
        }),
        Expr::Function { .. } => true,
        Expr::Call { .. } | Expr::MethodCall { .. } => true,
        Expr::Paren { .. } => unreachable!("expr_without_parens removes parens"),
    }
}

fn expr_references_name(expr: &Expr, needle: &str) -> bool {
    match expr_without_parens(expr) {
        Expr::Nil(_) | Expr::Bool(_, _) | Expr::Number(_, _) | Expr::String(_, _) => false,
        Expr::InterpolatedString { expressions, .. } => expressions
            .iter()
            .any(|expr| expr_references_name(expr, needle)),
        Expr::Name(name, _) => name == needle,
        Expr::Varargs(_) => false,
        Expr::Unary { expr, .. } => expr_references_name(expr, needle),
        Expr::Binary { lhs, rhs, .. } => {
            expr_references_name(lhs, needle) || expr_references_name(rhs, needle)
        }
        Expr::IfElse {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            expr_references_name(condition, needle)
                || expr_references_name(then_expr, needle)
                || expr_references_name(else_expr, needle)
        }
        Expr::Field { object, .. } => expr_references_name(object, needle),
        Expr::Index { object, index, .. } => {
            expr_references_name(object, needle) || expr_references_name(index, needle)
        }
        Expr::Table { fields, .. } => fields.iter().any(|field| match field {
            TableField::Array { value, .. } | TableField::Named { value, .. } => {
                expr_references_name(value, needle)
            }
            TableField::Keyed { key, value, .. } => {
                expr_references_name(key, needle) || expr_references_name(value, needle)
            }
        }),
        Expr::Function { .. } => false,
        Expr::Call { callee, args, .. } => {
            expr_references_name(callee, needle)
                || args.iter().any(|expr| expr_references_name(expr, needle))
        }
        Expr::MethodCall { object, args, .. } => {
            expr_references_name(object, needle)
                || args.iter().any(|expr| expr_references_name(expr, needle))
        }
        Expr::Paren { .. } => unreachable!("expr_without_parens removes parens"),
    }
}

fn block_mentions_name(body: &[Stmt], needle: &str) -> bool {
    body.iter().any(|stmt| stmt_mentions_name(stmt, needle))
}

fn stmt_mentions_name(stmt: &Stmt, needle: &str) -> bool {
    match stmt {
        Stmt::Local { bindings, init, .. } => {
            bindings.iter().any(|binding| binding.name == needle)
                || init.iter().any(|expr| expr_mentions_name(expr, needle))
        }
        Stmt::LocalFunction {
            name, params, body, ..
        } => {
            name == needle
                || params.iter().any(|param| param.name == needle)
                || block_mentions_name(body, needle)
        }
        Stmt::Function {
            target,
            params,
            body,
            ..
        } => {
            assignment_target_mentions_name(target, needle)
                || params.iter().any(|param| param.name == needle)
                || block_mentions_name(body, needle)
        }
        Stmt::Assignment {
            targets, values, ..
        } => {
            targets
                .iter()
                .any(|target| assignment_target_mentions_name(target, needle))
                || values.iter().any(|expr| expr_mentions_name(expr, needle))
        }
        Stmt::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            expr_mentions_name(condition, needle)
                || block_mentions_name(then_body, needle)
                || block_mentions_name(else_body, needle)
        }
        Stmt::While {
            condition, body, ..
        } => expr_mentions_name(condition, needle) || block_mentions_name(body, needle),
        Stmt::ForNumeric {
            variable,
            start,
            limit,
            step,
            body,
            ..
        } => {
            variable == needle
                || expr_mentions_name(start, needle)
                || expr_mentions_name(limit, needle)
                || step
                    .as_ref()
                    .is_some_and(|expr| expr_mentions_name(expr, needle))
                || block_mentions_name(body, needle)
        }
        Stmt::ForGeneric {
            variables,
            iter,
            body,
            ..
        } => {
            variables.iter().any(|binding| binding.name == needle)
                || iter.iter().any(|expr| expr_mentions_name(expr, needle))
                || block_mentions_name(body, needle)
        }
        Stmt::Repeat {
            body, condition, ..
        } => block_mentions_name(body, needle) || expr_mentions_name(condition, needle),
        Stmt::Do { body, .. } => block_mentions_name(body, needle),
        Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::TypeAlias { .. } => false,
        Stmt::Return {
            value,
            extra_values,
            ..
        } => {
            value
                .as_ref()
                .is_some_and(|expr| expr_mentions_name(expr, needle))
                || extra_values
                    .iter()
                    .any(|expr| expr_mentions_name(expr, needle))
        }
        Stmt::Expr { expr, .. } => expr_mentions_name(expr, needle),
    }
}

fn assignment_target_mentions_name(target: &AssignmentTarget, needle: &str) -> bool {
    match target {
        AssignmentTarget::Local { name, .. } => name == needle,
        AssignmentTarget::Field { object, .. } => expr_mentions_name(object, needle),
        AssignmentTarget::Index { object, index, .. } => {
            expr_mentions_name(object, needle) || expr_mentions_name(index, needle)
        }
    }
}

fn expr_mentions_name(expr: &Expr, needle: &str) -> bool {
    match expr_without_parens(expr) {
        Expr::Nil(_) | Expr::Bool(_, _) | Expr::Number(_, _) | Expr::String(_, _) => false,
        Expr::InterpolatedString { expressions, .. } => expressions
            .iter()
            .any(|expr| expr_mentions_name(expr, needle)),
        Expr::Name(name, _) => name == needle,
        Expr::Varargs(_) => false,
        Expr::Unary { expr, .. } => expr_mentions_name(expr, needle),
        Expr::Binary { lhs, rhs, .. } => {
            expr_mentions_name(lhs, needle) || expr_mentions_name(rhs, needle)
        }
        Expr::IfElse {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            expr_mentions_name(condition, needle)
                || expr_mentions_name(then_expr, needle)
                || expr_mentions_name(else_expr, needle)
        }
        Expr::Field { object, .. } => expr_mentions_name(object, needle),
        Expr::Index { object, index, .. } => {
            expr_mentions_name(object, needle) || expr_mentions_name(index, needle)
        }
        Expr::Table { fields, .. } => fields.iter().any(|field| match field {
            TableField::Array { value, .. } | TableField::Named { value, .. } => {
                expr_mentions_name(value, needle)
            }
            TableField::Keyed { key, value, .. } => {
                expr_mentions_name(key, needle) || expr_mentions_name(value, needle)
            }
        }),
        Expr::Function { params, body, .. } => {
            params.iter().any(|param| param.name == needle) || block_mentions_name(body, needle)
        }
        Expr::Call { callee, args, .. } => {
            expr_mentions_name(callee, needle)
                || args.iter().any(|expr| expr_mentions_name(expr, needle))
        }
        Expr::MethodCall { object, args, .. } => {
            expr_mentions_name(object, needle)
                || args.iter().any(|expr| expr_mentions_name(expr, needle))
        }
        Expr::Paren { .. } => unreachable!("expr_without_parens removes parens"),
    }
}

fn expr_contains_function_literal(expr: &Expr) -> bool {
    match expr_without_parens(expr) {
        Expr::Nil(_) | Expr::Bool(_, _) | Expr::Number(_, _) | Expr::String(_, _) => false,
        Expr::InterpolatedString { expressions, .. } => {
            expressions.iter().any(expr_contains_function_literal)
        }
        Expr::Name(_, _) | Expr::Varargs(_) => false,
        Expr::Unary { expr, .. } => expr_contains_function_literal(expr),
        Expr::Binary { lhs, rhs, .. } => {
            expr_contains_function_literal(lhs) || expr_contains_function_literal(rhs)
        }
        Expr::IfElse {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            expr_contains_function_literal(condition)
                || expr_contains_function_literal(then_expr)
                || expr_contains_function_literal(else_expr)
        }
        Expr::Field { object, .. } => expr_contains_function_literal(object),
        Expr::Index { object, index, .. } => {
            expr_contains_function_literal(object) || expr_contains_function_literal(index)
        }
        Expr::Table { fields, .. } => fields.iter().any(|field| match field {
            TableField::Array { value, .. } | TableField::Named { value, .. } => {
                expr_contains_function_literal(value)
            }
            TableField::Keyed { key, value, .. } => {
                expr_contains_function_literal(key) || expr_contains_function_literal(value)
            }
        }),
        Expr::Function { .. } => true,
        Expr::Call { callee, args, .. } => {
            expr_contains_function_literal(callee)
                || args.iter().any(expr_contains_function_literal)
        }
        Expr::MethodCall { object, args, .. } => {
            expr_contains_function_literal(object)
                || args.iter().any(expr_contains_function_literal)
        }
        Expr::Paren { .. } => unreachable!("expr_without_parens removes parens"),
    }
}

#[derive(Clone, Copy)]
enum Binding {
    Register(u8),
    CapturedLocal(u8),
    Upvalue(u8),
    CurrentFunction,
}

enum SimpleAssignmentKey<'a> {
    Register(u8),
    Constant(Constant),
    Expression(&'a Expr),
}

#[derive(Clone)]
struct CaptureScope {
    locals: BTreeMap<String, Binding>,
    upvalues: Vec<UpvalueDescriptor>,
    captured_registers: BTreeSet<u8>,
    math_builtin_aliases: BTreeMap<u8, MathBuiltinAlias>,
    math_upvalue_aliases: BTreeMap<u8, MathBuiltinAlias>,
    stdlib_builtin_aliases: BTreeMap<u8, StdlibBuiltinAlias>,
    stdlib_upvalue_aliases: BTreeMap<u8, StdlibBuiltinAlias>,
    numeric_registers: BTreeSet<u8>,
    number_constant_locals: BTreeMap<String, f64>,
    fresh_table_registers: BTreeSet<u8>,
    assigned_local_names: BTreeSet<String>,
}

#[derive(Clone)]
enum CompiledAssignmentTarget {
    Local(u8),
    CapturedLocal(u8),
    Upvalue(u8),
    Table {
        table: u8,
        key: u8,
        key_is_known_number: bool,
        global_field: Option<String>,
        stdlib_field: Option<(String, String)>,
        may_mutate_stdlib_table: bool,
    },
    Global {
        key: i32,
        name: String,
    },
}

fn table_read_opcode_for_assignment_target(target: &CompiledAssignmentTarget) -> Opcode {
    match target {
        CompiledAssignmentTarget::Table {
            key_is_known_number: true,
            ..
        } => Opcode::GetIndexNumber,
        _ => Opcode::GetIndex,
    }
}

fn table_write_opcode_for_assignment_target(target: &CompiledAssignmentTarget) -> Opcode {
    match target {
        CompiledAssignmentTarget::Table {
            key_is_known_number: true,
            ..
        } => Opcode::SetIndexNumber,
        _ => Opcode::SetIndex,
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

#[derive(Clone, Copy)]
enum MathBuiltinAlias {
    Abs,
    Unary { opcode: Opcode, name: &'static str },
    Window(MathWindowCallSpec),
}

#[derive(Clone, Copy)]
enum StdlibBuiltinAlias {
    StringSub,
    StringFind,
    ToNumber,
    StringFormat,
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

fn math_builtin_alias_for_field(field: &str) -> Option<MathBuiltinAlias> {
    let alias = match field {
        "abs" => MathBuiltinAlias::Abs,
        "floor" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathFloor,
            name: "math.floor",
        },
        "ceil" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathCeil,
            name: "math.ceil",
        },
        "round" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathRound,
            name: "math.round",
        },
        "sign" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathSign,
            name: "math.sign",
        },
        "sqrt" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathSqrt,
            name: "math.sqrt",
        },
        "deg" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathDeg,
            name: "math.deg",
        },
        "rad" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathRad,
            name: "math.rad",
        },
        "isnan" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathIsNan,
            name: "math.isnan",
        },
        "isinf" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathIsInf,
            name: "math.isinf",
        },
        "isfinite" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathIsFinite,
            name: "math.isfinite",
        },
        "sin" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathSin,
            name: "math.sin",
        },
        "cos" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathCos,
            name: "math.cos",
        },
        "tan" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathTan,
            name: "math.tan",
        },
        "asin" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathAsin,
            name: "math.asin",
        },
        "acos" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathAcos,
            name: "math.acos",
        },
        "atan" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathAtan,
            name: "math.atan",
        },
        "sinh" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathSinh,
            name: "math.sinh",
        },
        "cosh" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathCosh,
            name: "math.cosh",
        },
        "tanh" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathTanh,
            name: "math.tanh",
        },
        "exp" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathExp,
            name: "math.exp",
        },
        "log10" => MathBuiltinAlias::Unary {
            opcode: Opcode::MathLog10,
            name: "math.log10",
        },
        "fmod" => {
            MathBuiltinAlias::Window(MathWindowCallSpec::new(Opcode::MathFmod, "math.fmod", 2, 2))
        }
        "pow" => {
            MathBuiltinAlias::Window(MathWindowCallSpec::new(Opcode::MathPow, "math.pow", 2, 2))
        }
        "atan2" => MathBuiltinAlias::Window(MathWindowCallSpec::new(
            Opcode::MathAtan2,
            "math.atan2",
            2,
            2,
        )),
        "log" => {
            MathBuiltinAlias::Window(MathWindowCallSpec::new(Opcode::MathLog, "math.log", 1, 2))
        }
        "ldexp" => MathBuiltinAlias::Window(MathWindowCallSpec::new(
            Opcode::MathLdexp,
            "math.ldexp",
            2,
            2,
        )),
        "clamp" => MathBuiltinAlias::Window(MathWindowCallSpec::new(
            Opcode::MathClamp,
            "math.clamp",
            3,
            3,
        )),
        "map" => {
            MathBuiltinAlias::Window(MathWindowCallSpec::new(Opcode::MathMap, "math.map", 5, 5))
        }
        "lerp" => {
            MathBuiltinAlias::Window(MathWindowCallSpec::new(Opcode::MathLerp, "math.lerp", 3, 3))
        }
        "min" => MathBuiltinAlias::Window(MathWindowCallSpec::new(
            Opcode::MathMin,
            "math.min",
            1,
            u8::MAX as usize,
        )),
        "max" => MathBuiltinAlias::Window(MathWindowCallSpec::new(
            Opcode::MathMax,
            "math.max",
            1,
            u8::MAX as usize,
        )),
        _ => return None,
    };
    Some(alias)
}

fn restore_scoped_aliases<T: Copy>(
    current_aliases: &BTreeMap<u8, T>,
    mut parent_aliases: BTreeMap<u8, T>,
) -> BTreeMap<u8, T> {
    parent_aliases.retain(|register, _| current_aliases.contains_key(register));
    parent_aliases
}

fn restore_scoped_set(
    current_values: &BTreeSet<u8>,
    mut parent_values: BTreeSet<u8>,
) -> BTreeSet<u8> {
    parent_values.retain(|register| current_values.contains(register));
    parent_values
}

fn restore_scoped_map<K: Ord + Clone, V>(
    current_values: &BTreeMap<K, V>,
    mut parent_values: BTreeMap<K, V>,
) -> BTreeMap<K, V> {
    parent_values.retain(|key, _| current_values.contains_key(key));
    parent_values
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
        Expr::Paren { expr, .. } => const_eval(expr),
        Expr::Field { .. }
        | Expr::Index { .. }
        | Expr::Table { .. }
        | Expr::Function { .. }
        | Expr::Call { .. }
        | Expr::MethodCall { .. } => None,
    }
}

fn constant_is_scalar_temp(constant: &Constant) -> bool {
    matches!(
        constant,
        Constant::Nil | Constant::Boolean(_) | Constant::Number(_)
    )
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

fn const_eval_binary(op: BinaryOp, lhs: &Expr, rhs: &Expr) -> Option<Constant> {
    let lhs = const_eval(lhs)?;
    let rhs = const_eval(rhs)?;
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

fn opcode_for_rhs_number_const_binary(op: BinaryOp) -> Option<Opcode> {
    match op {
        BinaryOp::Add => Some(Opcode::AddConst),
        BinaryOp::Sub => Some(Opcode::SubConst),
        BinaryOp::Mul => Some(Opcode::MulConst),
        BinaryOp::Div => Some(Opcode::DivConst),
        BinaryOp::FloorDiv => Some(Opcode::FloorDivConst),
        BinaryOp::Mod => Some(Opcode::ModConst),
        BinaryOp::Pow => Some(Opcode::PowConst),
        _ => None,
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

fn lhs_const_concat_key(index: &Expr) -> Option<(Constant, &Expr)> {
    match index {
        Expr::Binary {
            op: BinaryOp::Concat,
            lhs,
            rhs,
            ..
        } => {
            let constant = const_eval(lhs)?;
            matches!(constant, Constant::String(_) | Constant::Number(_)).then_some((constant, rhs))
        }
        Expr::Paren { expr, .. } => lhs_const_concat_key(expr),
        _ => None,
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
    use rustia_vm::bytecode::{
        Constant, Opcode, Program, ARRAY_ITERATOR_SKIP_VISIBLE_KEY,
        ARRAY_ITERATOR_WRITE_VISIBLE_KEY,
    };
    use rustia_vm::{ErrorKind, Profile, Vm, VmSettings};

    fn opcode_count(program: &Program, opcode: Opcode) -> usize {
        program
            .main
            .code
            .iter()
            .chain(
                program
                    .functions
                    .iter()
                    .flat_map(|function| function.code.iter()),
            )
            .filter(|instruction| instruction.op == opcode)
            .count()
    }

    #[test]
    fn coalesces_scope_cleanup_local_cell_clears() {
        let chunk =
            parse_source("do local a, b, c = 1, 2, 3 local d = a + b + c end return 4").unwrap();
        let program = compile(&chunk).unwrap();
        assert!(
            opcode_count(&program, Opcode::ClearLocalCells) > 0,
            "{:?}",
            program.main.code
        );
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(4.0));
    }

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
            .constants
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
    fn compiles_lhs_constant_concat_to_direct_opcode() {
        let chunk = parse_source("local i = 12 return 'k' .. i").unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::ConcatConst), 1);
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("k12"));

        let chunk = parse_source(
            "local t = setmetatable({}, { __concat = function(a, b) return 'mt' end }) return 'k' .. t",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::ConcatConst), 1);
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("mt"));
    }

    #[test]
    fn compiles_rhs_constant_concat_to_direct_opcode() {
        let chunk = parse_source("local value = 'k' return value .. '!'").unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::ConcatRhsConst), 1);
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("k!"));

        let chunk = parse_source(
            "local t = setmetatable({}, { __concat = function(a, b) return 'mt' end }) return t .. 'k'",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::ConcatRhsConst), 1);
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("mt"));
    }

    #[test]
    fn compiles_lhs_constant_concat_table_index_to_direct_opcodes() {
        let chunk =
            parse_source("local t = {} local i = 12 t['k' .. i] = 5 return t['k' .. i]").unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::SetIndexConcatConst), 1);
        assert_eq!(opcode_count(&program, Opcode::GetIndexConcatConst), 1);
        assert_eq!(opcode_count(&program, Opcode::ConcatConst), 0);
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(5.0));

        let chunk = parse_source(
            "local suffix = setmetatable({}, { __concat = function(a, b) return 'key' end }) local t = {} t['k' .. suffix] = 7 return t['k' .. suffix]",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::SetIndexConcatConst), 1);
        assert_eq!(opcode_count(&program, Opcode::GetIndexConcatConst), 1);
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(7.0));

        let chunk = parse_source(
            "local t = setmetatable({}, { __index = function(_, key) return key end }) local i = 12 return t['k' .. i]",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::GetIndexConcatConst), 1);
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("k12"));

        let chunk = parse_source(
            "local hits = 0 local t = setmetatable({}, { __newindex = function(_, key, value) hits = hits + value end }) local i = 12 t['k' .. i] = 3 return hits",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::SetIndexConcatConst), 1);
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(3.0));
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
            ("return {[nil] = 2}", "table index is nil"),
            ("return {[0/0] = 2}", "table index is NaN"),
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
    fn unused_generic_array_iterator_keys_skip_visible_key_writes() {
        let chunk = parse_source(
            r#"
local acc = 0
for _, v in ipairs({ 1, 2, 3 }) do
    acc = acc + v
end
for _, v in pairs({ a = 4, b = 5 }) do
    acc = acc + v
end
return acc
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(program
            .main
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::IpairsLoop
                && instruction.c == ARRAY_ITERATOR_SKIP_VISIBLE_KEY));
        assert!(program
            .main
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::PairsLoop
                && instruction.c == ARRAY_ITERATOR_SKIP_VISIBLE_KEY));

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(15.0));
    }

    #[test]
    fn referenced_generic_array_iterator_keys_keep_visible_key_writes() {
        let chunk = parse_source(
            r#"
local acc = 0
for k, v in ipairs({ 10, 20 }) do
    acc = acc + k + v
end
for _, v in pairs({ a = 1 }) do
    local function capture()
        return _
    end
    acc = acc + v + (capture() and 1 or 0)
end
return acc
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(program
            .main
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::IpairsLoop
                && instruction.c == ARRAY_ITERATOR_WRITE_VISIBLE_KEY));
        assert!(program
            .main
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::PairsLoop
                && instruction.c == ARRAY_ITERATOR_WRITE_VISIBLE_KEY));

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(35.0));
    }

    #[test]
    fn nested_stable_iterator_builtins_use_specialized_opcodes() {
        let chunk = parse_source(
            r#"
local Coll = {}
Coll.__index = Coll
function Coll.touch() end
local function f()
    local acc = 0
    for _, v in pairs({ a = 1, b = 2 }) do
        acc = acc + v
    end
    for _, v in ipairs({ 3, 4 }) do
        acc = acc + v
    end
    return acc
end
return f()
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(opcode_count(&program, Opcode::PairsLoop) > 0);
        assert!(opcode_count(&program, Opcode::IpairsLoop) > 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(10.0));
    }

    #[test]
    fn nested_iterator_builtin_reassignment_blocks_specialized_opcodes() {
        let chunk = parse_source(
            r#"
local function f()
    local acc = 0
    for _, v in pairs({ 1, 2 }) do
        acc = acc + v
    end
    return acc
end
pairs = function()
    return function()
    end
end
return f
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::PairsLoop), 0);
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

    #[test]
    fn local_self_increment_assignments_write_in_place() {
        let chunk = parse_source("local p = 0 p = p + 1 p = (p) + 1 return p").unwrap();
        let program = compile(&chunk).unwrap();
        let in_place_adds = program
            .main
            .code
            .iter()
            .filter(|instruction| {
                instruction.op == Opcode::AddConst && instruction.a == 0 && instruction.b == 0
            })
            .count();
        assert_eq!(in_place_adds, 2);
        assert!(program
            .main
            .code
            .iter()
            .all(|instruction| instruction.op != Opcode::Add));

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(2.0));
    }

    #[test]
    fn local_self_increment_does_not_reorder_add_operands() {
        let chunk = parse_source("local p = 0 p = 1 + p return p").unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(
            program
                .main
                .code
                .iter()
                .filter(|instruction| {
                    instruction.op == Opcode::Add && instruction.a == 0 && instruction.b == 0
                })
                .count(),
            0
        );

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(1.0));
    }

    #[test]
    fn upvalue_self_increment_assignments_use_fused_opcode() {
        let chunk = parse_source(
            "local function make() local i = 0 return function() i = i + 1 return i end end local f = make() f() return f()",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let add_upvalue_const_count = program
            .functions
            .iter()
            .flat_map(|function| function.code.iter())
            .filter(|instruction| instruction.op == Opcode::AddUpvalueConst)
            .count();
        assert_eq!(add_upvalue_const_count, 1, "{:?}", program.functions);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(2.0));

        let coercion = compile(
            &parse_source(
                "local function make() local i = '1' return function() i = i + 1 return i end end return make()()",
            )
            .unwrap(),
        )
        .unwrap();
        assert!(coercion.functions.iter().any(|function| function
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::AddUpvalueConst)));
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&coercion).unwrap();
        assert_eq!(result.as_number(), Some(2.0));
    }

    #[test]
    fn upvalue_self_increment_does_not_reorder_add_operands() {
        let chunk = parse_source(
            "local function make() local i = 0 return function() i = 1 + i return i end end local f = make() return f()",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        for function in &program.functions {
            let has_set_upvalue = function
                .code
                .iter()
                .any(|instruction| instruction.op == Opcode::SetUpvalue);
            if has_set_upvalue {
                assert!(
                    function
                        .code
                        .iter()
                        .all(|instruction| instruction.op != Opcode::AddUpvalueConst),
                    "{:?}",
                    function.code
                );
                assert_eq!(
                    function
                        .code
                        .iter()
                        .filter(|instruction| instruction.op == Opcode::AddConst)
                        .count(),
                    0,
                    "{:?}",
                    function.code
                );
            }
        }

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(1.0));
    }

    #[test]
    fn upvalue_self_add_upvalue_assignments_use_fused_opcode() {
        let chunk = parse_source(
            "local function make(start, step) local c = start return function() c = c + step return c end end local f = make(2, 3) f() return f()",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let add_upvalue_count = program
            .functions
            .iter()
            .flat_map(|function| function.code.iter())
            .filter(|instruction| instruction.op == Opcode::AddUpvalue)
            .count();
        assert_eq!(add_upvalue_count, 1, "{:?}", program.functions);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(8.0));

        let chunk = parse_source(
            "local function make() local c, step = '1', '2' return function() c = c + step return c end end return make()()",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(program
            .functions
            .iter()
            .flat_map(|function| function.code.iter())
            .any(|instruction| instruction.op == Opcode::AddUpvalue));
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(3.0));
    }

    #[test]
    fn upvalue_self_add_upvalue_does_not_reorder_add_operands() {
        let chunk = parse_source(
            "local function make(start, step) local c = start return function() c = step + c return c end end local f = make(2, 3) return f()",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(
            program
                .functions
                .iter()
                .flat_map(|function| function.code.iter())
                .all(|instruction| instruction.op != Opcode::AddUpvalue),
            "{:?}",
            program.functions
        );

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(5.0));
    }

    #[test]
    fn local_self_binary_assignments_write_in_place() {
        let chunk = parse_source("local p = 2 local q = 5 p = p + q return p").unwrap();
        let program = compile(&chunk).unwrap();
        let in_place_adds = program
            .main
            .code
            .iter()
            .filter(|instruction| {
                instruction.op == Opcode::Add && instruction.a == 0 && instruction.b == 0
            })
            .count();
        assert_eq!(in_place_adds, 1, "{:?}", program.main.code);
        let add_index = program
            .main
            .code
            .iter()
            .position(|instruction| {
                instruction.op == Opcode::Add && instruction.a == 0 && instruction.b == 0
            })
            .unwrap();
        assert_ne!(
            program
                .main
                .code
                .get(add_index + 1)
                .map(|instruction| instruction.op),
            Some(Opcode::ClearLocalCell),
            "{:?}",
            program.main.code
        );

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(7.0));
    }

    #[test]
    fn local_self_binary_assignment_does_not_reorder_add_operands() {
        let chunk = parse_source("local p = 2 local q = 5 p = q + p return p").unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(
            program
                .main
                .code
                .iter()
                .filter(|instruction| {
                    instruction.op == Opcode::Add && instruction.a == 0 && instruction.b == 0
                })
                .count(),
            0,
            "{:?}",
            program.main.code
        );

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(7.0));
    }

    #[test]
    fn local_self_binary_assignment_preserves_rhs_closure_capture() {
        let chunk =
            parse_source("local p = 10 p = p + (function() p = 99 return 1 end)() return p")
                .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(
            program
                .main
                .code
                .iter()
                .filter(|instruction| {
                    instruction.op == Opcode::Add && instruction.a == 0 && instruction.b == 0
                })
                .count(),
            0,
            "{:?}",
            program.main.code
        );

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(11.0));
    }

    #[test]
    fn local_self_binary_assignment_accepts_call_rhs_for_plain_locals() {
        let chunk = parse_source(
            "local p = 2 local function next_value() return 5 end p = p + next_value() return p",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let in_place_adds = program
            .main
            .code
            .iter()
            .filter(|instruction| {
                instruction.op == Opcode::Add && instruction.a == 0 && instruction.b == 0
            })
            .count();
        assert_eq!(in_place_adds, 1, "{:?}", program.main.code);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(7.0));
    }

    #[test]
    fn local_self_binary_assignment_skips_call_rhs_for_captured_locals() {
        let chunk = parse_source(
            "local p = 10 local function bump() p = 99 return 1 end p = p + bump() return p",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let in_place_adds = program
            .main
            .code
            .iter()
            .filter(|instruction| {
                instruction.op == Opcode::Add && instruction.a == 0 && instruction.b == 0
            })
            .count();
        assert_eq!(in_place_adds, 0, "{:?}", program.main.code);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(11.0));
    }

    #[test]
    fn local_self_binary_or_zero_chains_write_in_place() {
        let chunk = parse_source(
            "local t = { a = 4 } local p = 1 p = p + (t.a or 0) + (t.missing or 0) return p",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let in_place_adds = program
            .main
            .code
            .iter()
            .filter(|instruction| {
                instruction.op == Opcode::Add && instruction.a == 1 && instruction.b == 1
            })
            .count();
        assert_eq!(in_place_adds, 2, "{:?}", program.main.code);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(5.0));
    }

    #[test]
    fn local_self_binary_or_zero_chain_keeps_later_self_reads_on_general_path() {
        let chunk =
            parse_source("local t = { 3 } local p = 2 p = p + (t[1] or 0) + p return p").unwrap();
        let program = compile(&chunk).unwrap();
        let in_place_adds = program
            .main
            .code
            .iter()
            .filter(|instruction| {
                instruction.op == Opcode::Add && instruction.a == 1 && instruction.b == 1
            })
            .count();
        assert_eq!(in_place_adds, 0, "{:?}", program.main.code);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(7.0));
    }

    #[test]
    fn local_self_binary_or_zero_chain_preserves_index_metamethod_order() {
        let chunk = parse_source(
            r#"
local p = 1
local hits = {}
local t = setmetatable({}, {
  __index = function()
    hits[#hits + 1] = p
    return 2
  end
})
p = p + (t.a or 0) + (t.b or 0)
return hits[1] * 100 + hits[2] * 10 + p
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(115.0));
    }

    #[test]
    fn local_self_binary_assignment_skips_non_direct_rhs_reads() {
        let chunk = parse_source("local p = 2 local s = 'abcd' p = p + #s return p").unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(
            program
                .main
                .code
                .iter()
                .filter(|instruction| {
                    instruction.op == Opcode::Add && instruction.a == 0 && instruction.b == 0
                })
                .count(),
            0,
            "{:?}",
            program.main.code
        );

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(6.0));
    }

    #[test]
    fn rhs_number_constant_arithmetic_uses_direct_opcodes() {
        let chunk = parse_source(
            "local x = 3 return (x + 2) + (x - 1) + (x * 4) + (x / 2) + (x // 2) + (x % 2) + (x ^ 2)",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        for opcode in [
            Opcode::AddConst,
            Opcode::SubConst,
            Opcode::MulConst,
            Opcode::DivConst,
            Opcode::FloorDivConst,
            Opcode::ModConst,
            Opcode::PowConst,
        ] {
            assert_eq!(opcode_count(&program, opcode), 1, "{opcode:?}");
        }

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(31.5));
    }

    #[test]
    fn rhs_number_constant_arithmetic_preserves_fallbacks() {
        let cases = [
            ("local x = '4' return x + 2", 6.0),
            (
                "local t = setmetatable({}, { __add = function(_, rhs) return rhs + 10 end }) return t + 2",
                12.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            assert!(opcode_count(&program, Opcode::AddConst) > 0, "{source}");

            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }
    }

    #[test]
    fn mul_mod_constant_arithmetic_uses_direct_opcode_for_known_numbers() {
        let chunk = parse_source(
            r#"
local out = 0
for i = 1, 3 do
    out = out + ((i * 7919) % 100003)
end
return out
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::MulModConst), 1);
        assert_eq!(opcode_count(&program, Opcode::MulConst), 0);
        assert_eq!(opcode_count(&program, Opcode::ModConst), 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(47514.0));
    }

    #[test]
    fn mul_mod_constant_table_write_uses_direct_opcode() {
        let chunk = parse_source(
            r#"
local t = {}
for i = 1, 3 do
    t[(i * 7919) % 100003] = i
end
local out = 0
for i = 1, 3 do
    out = out + (t[(i * 7919) % 100003] or 0)
end
return out
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::SetIndexMulModConst), 1);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(6.0));
    }

    #[test]
    fn mul_mod_constant_table_read_uses_direct_opcode() {
        let chunk = parse_source(
            r#"
local t = {}
for i = 1, 3 do
    t[(i * 7919) % 100003] = i
end
local out = 0
for i = 1, 3 do
    out = out + (t[(i * 7919) % 100003] or 0)
end
return out
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::SetIndexMulModConst), 1);
        assert_eq!(opcode_count(&program, Opcode::GetIndexMulModConst), 1);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(6.0));
    }

    #[test]
    fn mul_mod_constant_table_read_preserves_index_fallback() {
        let chunk = parse_source(
            r#"
local t = setmetatable({}, {
    __index = function(_, key)
        return key
    end,
})
local out = 0
for i = 1, 2 do
    out = out + t[(i * 3) % 10]
end
return out
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::GetIndexMulModConst), 1);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(9.0));
    }

    #[test]
    fn mul_mod_constant_arithmetic_skips_unknown_number_factors() {
        let chunk = parse_source(
            r#"
local function value()
    return 2
end
return (value() * 3) % 5
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::MulModConst), 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(1.0));
    }

    #[test]
    fn mul_add_mod_constant_arithmetic_uses_direct_opcode_for_known_numbers() {
        let chunk = parse_source(
            r#"
local out = 0
for it = 1, 2 do
    for i = 1, 3 do
        out = out + ((i * 2654435761 + it) % 1000)
    end
end
return out
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::MulAddModConst), 1);
        assert_eq!(opcode_count(&program, Opcode::MulModConst), 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(3141.0));
    }

    #[test]
    fn mul_add_mod_constant_table_write_uses_direct_opcode() {
        let chunk = parse_source(
            r#"
local t = {}
for it = 1, 2 do
    for i = 1, 3 do
        t[i] = (i * 2654435761 + it) % 1000
    end
end
return t[1] + t[2] + t[3]
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::SetIndexMulAddModConst), 1);
        assert_eq!(opcode_count(&program, Opcode::MulAddModConst), 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(1572.0));
    }

    #[test]
    fn mul_add_mod_constant_arithmetic_skips_unknown_number_factors() {
        let chunk = parse_source(
            r#"
local function value()
    return 2
end
local it = 1
return ((value() * 7 + it) % 5)
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::MulAddModConst), 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(0.0));
    }

    #[test]
    fn mod_add_constant_arithmetic_uses_direct_opcode_for_known_numbers() {
        let chunk = parse_source(
            r#"
local out = 0
for i = 1, 3 do
    out = out + ((i % 8) + 1)
end
return out
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::ModAddConst), 1);
        assert_eq!(opcode_count(&program, Opcode::ModConst), 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(9.0));
    }

    #[test]
    fn mod_add_constant_arithmetic_skips_unknown_number_factors() {
        let chunk = parse_source(
            r#"
local function value()
    return 10
end
return (value() % 8) + 1
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::ModAddConst), 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(3.0));
    }

    #[test]
    fn mod_add_constant_table_read_uses_direct_opcode() {
        let chunk = parse_source(
            r#"
local counters = { 10, 20, 30, 40, 50, 60, 70, 80 }
local acc = 0
for i = 1, 3 do
    acc = acc + counters[(i % 8) + 1]
end
return acc
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::GetIndexModAddConst), 1);
        assert_eq!(opcode_count(&program, Opcode::ModAddConst), 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(90.0));
    }

    #[test]
    fn mod_add_constant_table_read_preserves_index_fallback() {
        let chunk = parse_source(
            r#"
local t = setmetatable({}, {
    __index = function(_, key)
        return key + 10
    end,
})
local acc = 0
for i = 1, 1 do
    acc = t[(i % 8) + 1]
end
return acc
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::GetIndexModAddConst), 1);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(12.0));
    }

    #[test]
    fn simple_table_reads_use_source_registers() {
        let chunk = parse_source(
            "local t = {10, 20} local i = 2 local y = t[i] local z = t[1] return y + z",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let get_indexes = program
            .main
            .code
            .iter()
            .filter(|instruction| instruction.op == Opcode::GetIndexNumber)
            .collect::<Vec<_>>();
        assert_eq!(get_indexes.len(), 1, "{:?}", program.main.code);
        assert_eq!(opcode_count(&program, Opcode::GetIndexNumberConst), 1);
        assert!(
            get_indexes
                .iter()
                .all(|instruction| instruction.a != instruction.b && instruction.a != instruction.c),
            "{:?}",
            program.main.code
        );
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(30.0));
    }

    #[test]
    fn constant_numeric_table_reads_use_direct_opcode_and_preserve_fallbacks() {
        let chunk = parse_source(
            r#"
local t = setmetatable({}, {
    __index = function(_, key)
        return key + 1
    end,
})
return t[3]
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::GetIndexNumberConst), 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(4.0));

        let chunk = parse_source(
            r#"
local t = {}
setmetatable(t, {
    __index = function(_, key)
        return key + 1
    end,
})
return t[3]
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::GetIndexNumberConst), 1);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(4.0));
    }

    #[test]
    fn simple_field_reads_use_source_registers() {
        let chunk = parse_source("local t = { x = 7 } return t.x").unwrap();
        let program = compile(&chunk).unwrap();
        let get_index = program
            .main
            .code
            .iter()
            .find(|instruction| instruction.op == Opcode::GetIndexStringConst)
            .expect("field read should compile to GetIndexStringConst");
        assert_ne!(get_index.a, get_index.b, "{:?}", program.main.code);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(7.0));
    }

    #[test]
    fn static_string_index_reads_use_quickened_opcode() {
        let chunk = parse_source("local t = { x = 7 } return t[\"x\"]").unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::GetIndexStringConst), 1);
        assert_eq!(opcode_count(&program, Opcode::GetIndex), 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(7.0));
    }

    #[test]
    fn quickened_string_index_preserves_metamethod_fallback() {
        let chunk = parse_source(
            r#"
local t = {}
setmetatable(t, {
    __index = function(_, key)
        if key == "missing" then
            return 9
        end
    end,
})
return t.missing
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(opcode_count(&program, Opcode::GetIndexStringConst) >= 1);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(9.0));
    }

    #[test]
    fn static_string_index_writes_use_quickened_opcode() {
        let chunk = parse_source("local t = { x = 0 } t.x = 7 return t.x").unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::SetIndexStringConst), 1);
        assert_eq!(opcode_count(&program, Opcode::SetIndex), 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(7.0));
    }

    #[test]
    fn quickened_string_index_write_preserves_newindex_fallback() {
        let chunk = parse_source(
            r#"
local seen = 0
local t = setmetatable({}, {
    __newindex = function(_, key, value)
        if key == "missing" then
            seen = value
        end
    end,
})
t.missing = 9
return seen
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(opcode_count(&program, Opcode::SetIndexStringConst) >= 1);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(9.0));
    }

    #[test]
    fn quickened_string_index_write_defers_nil_slots_with_newindex() {
        let chunk = parse_source(
            r#"
local seen = 0
local t = { x = 1 }
setmetatable(t, {
    __newindex = function(_, _, value)
        seen = value
    end,
})
t.x = 2
t.x = nil
t.x = 5
return seen + (t.x == nil and 100 or 0)
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(opcode_count(&program, Opcode::SetIndexStringConst) >= 3);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(105.0));
    }

    #[test]
    fn quickened_index_table_cache_reads_updated_delegate_slots() {
        let chunk = parse_source(
            r#"
local proto = { x = 1, y = 2 }
local t = {}
setmetatable(t, { __index = proto })
local a = t.x
proto.x = 3
local b = t.x
proto.y = 4
local c = t.x
return a * 100 + b * 10 + c
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(opcode_count(&program, Opcode::GetIndexStringConst) >= 3);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(133.0));
    }

    #[test]
    fn quickened_index_table_cache_observes_index_table_reassignment() {
        let chunk = parse_source(
            r#"
local first = { x = 1 }
local second = { x = 2 }
local mt = { __index = first }
local t = {}
setmetatable(t, mt)
local a = t.x
mt.__index = second
local b = t.x
return a * 10 + b
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(opcode_count(&program, Opcode::GetIndexStringConst) >= 2);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(12.0));
    }

    #[test]
    fn index_read_lowering_preserves_side_effect_order() {
        let chunk = parse_source(
            r#"
local t = { 11 }
local function replace()
    t = { 22 }
    return 1
end
return t[replace()]
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(11.0));
    }

    #[test]
    fn numeric_loop_table_assignment_uses_source_registers() {
        let chunk = parse_source(
            r#"
local t = {}
for i = 1, 3 do
    t[i] = i * 2
end
return t[3]
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let set_index = program
            .main
            .code
            .iter()
            .position(|instruction| instruction.op == Opcode::SetIndexNumber)
            .expect("table assignment should compile to SetIndexNumber");
        assert_eq!(program.main.code[set_index].a, 0, "{:?}", program.main.code);
        assert_ne!(program.main.code[set_index].b, 0, "{:?}", program.main.code);
        assert_ne!(
            program
                .main
                .code
                .get(set_index + 1)
                .map(|instruction| instruction.op),
            Some(Opcode::ClearLocalCell),
            "{:?}",
            program.main.code
        );

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(6.0));
    }

    #[test]
    fn general_table_assignment_preserves_numeric_key_opcode() {
        let chunk = parse_source(
            r#"
local floor = math.floor
local t = {}
local len = 0
len = len + 1
t[len] = floor(1.5)
return t[1]
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(
            opcode_count(&program, Opcode::SetIndexNumber) > 0,
            "{:?}",
            program.main.code
        );

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(1.0));
    }

    #[test]
    fn constant_numeric_table_writes_use_direct_opcode() {
        let chunk = parse_source("local t = {} t[1] = 7 return t[1]").unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::SetIndexNumberConst), 1);
        assert_eq!(opcode_count(&program, Opcode::SetIndexNumber), 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(7.0));
    }

    #[test]
    fn constant_numeric_table_writes_preserve_newindex_fallback() {
        let chunk = parse_source(
            r#"
local seen = 0
local t = setmetatable({}, {
    __newindex = function(_, k, v)
        seen = k + v
    end,
})
t[3] = 4
return seen
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::SetIndexNumberConst), 1);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(7.0));
    }

    #[test]
    fn numeric_table_index_opcodes_preserve_metamethod_fallbacks() {
        let chunk = parse_source(
            r#"
local seen = 0
local t = setmetatable({}, {
    __newindex = function(_, k, v)
        seen = k + v
    end,
    __index = function(_, k)
        return k + 1
    end,
})
for i = 1, 1 do
    t[i] = 4
    return seen + t[i]
end
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(
            opcode_count(&program, Opcode::SetIndexNumber) > 0,
            "{:?}",
            program.main.code
        );
        assert!(
            opcode_count(&program, Opcode::GetIndexNumber) > 0,
            "{:?}",
            program.main.code
        );
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(7.0));
    }

    #[test]
    fn table_assignment_direct_path_preserves_side_effect_order() {
        let chunk = parse_source(
            r#"
local old = {}
local t = old
local key = 1
local x = setmetatable({}, {
    __add = function()
        t = {}
        return 5
    end,
})
t[key] = x + 1
return old[1] == 5 and t[1] == nil
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn len_plus_one_table_append_uses_specialized_opcode() {
        let chunk = parse_source(
            "local t = {} t[#t + 1] = 'a' t[(#t) + 1] = 'b' return table.concat(t, ',')",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let len_plus_one_count = opcode_count(&program, Opcode::LenPlusOne);
        assert_eq!(len_plus_one_count, 2);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("a,b"));
    }

    #[test]
    fn len_plus_one_table_append_preserves_metamethod_fallbacks() {
        let chunk = parse_source(
            r#"
local log = {}
local proxy = setmetatable({}, {
    __len = function()
        log[#log + 1] = "len"
        return 2
    end,
    __newindex = function(_, key, value)
        log[#log + 1] = tostring(key) .. ":" .. value
    end,
})
proxy[#proxy + 1] = "x"
return table.concat(log, ",")
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(opcode_count(&program, Opcode::LenPlusOne) > 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("len,3:x"));
    }

    #[test]
    fn len_plus_one_table_append_preserves_readonly_errors() {
        let chunk = parse_source(
            r#"
local t = table.freeze({})
local ok, err = pcall(function()
    t[#t + 1] = 1
end)
return tostring(ok) .. "|" .. tostring(string.find(err, "readonly", 1, true) ~= nil)
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(opcode_count(&program, Opcode::LenPlusOne) > 0);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(
            vm.string_value(result).unwrap().as_deref(),
            Some("false|true")
        );
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
    fn table_constructors_use_raw_field_writes() {
        let chunk =
            parse_source("local t = { x = 1, [2] = 3, 4 } return t.x + t[1] + t[2]").unwrap();
        let program = compile(&chunk).unwrap();
        let raw_set_count = program
            .main
            .code
            .iter()
            .filter(|instruction| instruction.op == Opcode::RawSet)
            .count();
        assert_eq!(raw_set_count, 3);
        assert!(program
            .main
            .code
            .iter()
            .all(|instruction| instruction.op != Opcode::SetIndex));

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(8.0));
    }

    #[test]
    fn named_table_literals_use_record_opcode() {
        let chunk = parse_source("local t = { id = 1, x = 2, id = 3 } return t.id + t.x").unwrap();
        let program = compile(&chunk).unwrap();
        assert!(program
            .main
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::NewRecord));
        assert!(program
            .main
            .code
            .iter()
            .all(|instruction| instruction.op != Opcode::RawSet));

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(5.0));

        let chunk =
            parse_source("local t = { f = function() return 'ok' end } return t.f()").unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(
            program
                .functions
                .first()
                .and_then(|function| function.debug_name.as_deref()),
            Some("f")
        );
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("ok"));
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
    fn unmodified_unsupported_stdlib_field_calls_remain_compile_errors() {
        let cases = [
            ("return math.unreal(1)", "unsupported math.unreal call"),
            ("return table.forever({})", "unsupported table.forever call"),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let error = compile(&chunk).unwrap_err();
            assert!(error.message.contains(expected), "{source}: {error}");
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
            (r#"return string.format("%.4f", 1.25)"#, "1.2500"),
            (r#"return string.format("%.4f", -0.0)"#, "-0.0000"),
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
                "local t = table.freeze({}) local ok, err = pcall(table.freeze, t) if not ok and err:find('attempt to modify a readonly table') then return 1 end return -1",
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
                "local t = {} t[0] = 'z' t[1] = 'a' return table.concat(t, ',', 0, 1)",
                "z,a",
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
    fn fresh_array_table_reserves_numeric_for_capacity_hint() {
        let chunk = parse_source(
            "local parts = {} for i = 0, 63 do parts[i + 1] = 'x' end return #table.concat(parts)",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let new_array = program
            .main
            .code
            .iter()
            .find(|instruction| instruction.op == Opcode::NewArray)
            .expect("fresh table allocation");
        assert_eq!(new_array.aux, 64);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(64.0));
    }

    #[test]
    fn fresh_array_table_capacity_hint_skips_zero_based_indices() {
        let chunk = parse_source("local parts = {} for i = 0, 63 do parts[i] = 'x' end").unwrap();
        let program = compile(&chunk).unwrap();
        let new_array = program
            .main
            .code
            .iter()
            .find(|instruction| instruction.op == Opcode::NewArray)
            .expect("fresh table allocation");
        assert_eq!(new_array.aux, 0);
    }

    #[test]
    fn fresh_array_table_uses_captured_numeric_bound_capacity_hint() {
        let chunk = parse_source(
            "local S = 256 local function f() local t = {} for i = 1, S do t[i] = 'x' end return #table.concat(t) end return f()",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let new_array = program
            .functions
            .iter()
            .flat_map(|function| function.code.iter())
            .find(|instruction| instruction.op == Opcode::NewArray)
            .expect("fresh table allocation inside function");
        assert_eq!(new_array.aux, 256);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(256.0));
    }

    #[test]
    fn fresh_array_table_capacity_hint_skips_mutated_numeric_bound() {
        let chunk = parse_source(
            "local S = 256 local function f() local t = {} for i = 1, S do t[i] = 'x' end return t end S = 10 return f()",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let new_array = program
            .functions
            .iter()
            .flat_map(|function| function.code.iter())
            .find(|instruction| instruction.op == Opcode::NewArray)
            .expect("fresh table allocation inside function");
        assert_eq!(new_array.aux, 0);
    }

    #[test]
    fn fresh_table_reserves_numeric_for_hash_capacity_hint() {
        let chunk = parse_source(
            "local t = {} for i = 1, 64 do t['k' .. i] = i t[(i * 7919) % 100003] = i end return t['k1'] + t[7919]",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let new_array = program
            .main
            .code
            .iter()
            .find(|instruction| instruction.op == Opcode::NewArray)
            .expect("fresh table allocation");
        assert_eq!(new_array.aux, -128);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(2.0));
    }

    #[test]
    fn batched_table_reset_reserves_threshold_capacity_hint() {
        let chunk = parse_source(
            r#"
local function f(n)
    local t = {}
    local len = 0
    for i = 1, n do
        len = len + 1
        t[len] = i
        if len >= 64 then
            t = {}
            len = 0
        end
    end
    return t[1] + t[2]
end
return f(130)
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let hinted_allocations = program
            .functions
            .iter()
            .flat_map(|function| function.code.iter())
            .filter(|instruction| instruction.op == Opcode::NewArray)
            .map(|instruction| instruction.aux)
            .collect::<Vec<_>>();
        assert_eq!(hinted_allocations, vec![64, 64]);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(259.0));
    }

    #[test]
    fn batched_table_reset_capacity_hint_requires_indexed_write() {
        let chunk = parse_source(
            r#"
local t = {}
local len = 0
for i = 1, 130 do
    len = len + 1
    if len >= 64 then
        t = {}
        len = 0
    end
end
return len
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let table_allocations = program
            .main
            .code
            .iter()
            .filter(|instruction| instruction.op == Opcode::NewArray)
            .map(|instruction| instruction.aux)
            .collect::<Vec<_>>();
        assert_eq!(table_allocations, vec![0, 0]);

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(2.0));
    }

    #[test]
    fn fresh_local_table_writes_keep_table_concat_fast_path() {
        let chunk =
            parse_source("local t = {} t[1] = 'a' t[2] = 'b' return table.concat(t, ',')").unwrap();
        let program = compile(&chunk).unwrap();
        assert!(
            opcode_count(&program, Opcode::TableConcat) > 0,
            "{:?}",
            program.main.code
        );

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("a,b"));
    }

    #[test]
    fn stdlib_table_alias_writes_disable_table_concat_fast_path() {
        let chunk = parse_source(
            "local alias = table alias.concat = function() return 'mutated' end return table.concat({'a'})",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(
            opcode_count(&program, Opcode::TableConcat),
            0,
            "{:?}",
            program.main.code
        );

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("mutated"));
    }

    #[test]
    fn nested_function_fresh_table_writes_keep_table_concat_fast_path() {
        let chunk = parse_source(
            r#"
local function f(n)
    local parts = {}
    for i = 1, n do
        parts[i] = "a" .. i
    end
    return table.concat(parts, ",")
end
return f(2)
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert!(
            opcode_count(&program, Opcode::TableConcat) > 0,
            "{:?}",
            program.functions
        );

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("a1,a2"));
    }

    #[test]
    fn future_stdlib_table_writes_disable_nested_table_concat_fast_path() {
        let chunk = parse_source(
            r#"
local function f()
    return table.concat({ "a" })
end
table.concat = function()
    return "mutated"
end
return f()
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(
            opcode_count(&program, Opcode::TableConcat),
            0,
            "{:?}",
            program.functions
        );

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("mutated"));
    }

    #[test]
    fn stdlib_alias_writes_disable_nested_table_concat_fast_path() {
        let chunk = parse_source(
            r#"
local alias = table
local function f()
    return table.concat({ "a" })
end
alias.concat = function()
    return "mutated"
end
return f()
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(
            opcode_count(&program, Opcode::TableConcat),
            0,
            "{:?}",
            program.functions
        );

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("mutated"));
    }

    #[test]
    fn table_concat_range_holes_still_raise_type_error() {
        let chunk = parse_source(
            r#"
		local values = { "a", nil, "c" }
local ok, err = pcall(function()
    return table.concat(values, ",", 1, 3)
end)
return tostring(ok) .. "|" .. tostring(string.find(err, "string or number", 1, true) ~= nil)
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(
            vm.string_value(result).unwrap().as_deref(),
            Some("false|true")
        );
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
    fn compiles_string_sub_byte_equality_to_direct_opcode() {
        let cases = [
            (
                r#"local s = "{x" local pos = 1 return string.sub(s, pos, pos) == "{""#,
                true,
            ),
            (
                r#"local sub = string.sub local s = "a;b" local pos = 2 return sub(s, pos, pos) == ";""#,
                true,
            ),
            (
                r#"local sub = string.sub local s = "abc" local pos = 2 return sub(s, pos, pos) ~= "x""#,
                true,
            ),
            (
                r#"local s = 123 local pos = 2 return string.sub(s, pos, pos) == "2""#,
                true,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            assert_eq!(
                opcode_count(&program, Opcode::StringByteEqual),
                1,
                "{source}"
            );
            assert_eq!(opcode_count(&program, Opcode::StringSub), 0, "{source}");

            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_bool(), Some(expected), "{source}");
        }
    }

    #[test]
    fn leaves_non_exact_string_sub_byte_equality_on_general_path() {
        let cases = [
            r#"local sub = string.sub local s = "abc" local pos = 2 return sub(s, pos, pos + 0) == "b""#,
            r#"local s = "abc" local pos = 2 return string.sub(s, pos, pos) ~= """#,
        ];

        for source in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            assert_eq!(opcode_count(&program, Opcode::StringByteEqual), 0);

            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_bool(), Some(true), "{source}");
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
    fn omitted_step_numeric_for_uses_step_one_opcodes() {
        let chunk = parse_source("local a = 0 for i = 1, 4 do a += i end return a").unwrap();
        let program = compile(&chunk).unwrap();
        assert_eq!(opcode_count(&program, Opcode::ForPrepStepOne), 1);
        assert_eq!(opcode_count(&program, Opcode::ForLoopStepOne), 1);
        assert_eq!(opcode_count(&program, Opcode::ForPrep), 0);
        assert_eq!(opcode_count(&program, Opcode::ForLoop), 0);
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(10.0));

        let explicit_step =
            compile(&parse_source("local a = 0 for i = 1, 4, 1 do a += i end return a").unwrap())
                .unwrap();
        assert_eq!(opcode_count(&explicit_step, Opcode::ForPrepStepOne), 0);
        assert_eq!(opcode_count(&explicit_step, Opcode::ForLoopStepOne), 0);
        assert_eq!(opcode_count(&explicit_step, Opcode::ForPrep), 1);
        assert_eq!(opcode_count(&explicit_step, Opcode::ForLoop), 1);
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
    fn compiles_sin_cos_product_to_direct_opcode() {
        let cases = [
            "local sin, cos = math.sin, math.cos local x = 0 return sin(x) * cos(x)",
            "local x = 0 return math.cos(x) * math.sin(x)",
            "local sin, cos = math.sin, math.cos local function f(x) return sin(x) * cos(x) end return f(0)",
        ];

        for source in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let ops = std::iter::once(&program.main)
                .chain(program.functions.iter())
                .flat_map(|function| function.code.iter().map(|instruction| instruction.op))
                .collect::<Vec<_>>();

            assert!(ops.contains(&Opcode::MathSinCosMul), "{ops:?}");
            assert!(!ops.contains(&Opcode::MathSin), "{ops:?}");
            assert!(!ops.contains(&Opcode::MathCos), "{ops:?}");

            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(0.0), "{source}");
        }
    }

    #[test]
    fn sin_cos_product_peephole_preserves_edges() {
        let cases = [
            (
                "local sin, cos = math.sin, math.cos local n = 0 local function x() n = n + 1 return 0 end local r = sin(x()) * cos(x()) return n",
                2.0,
            ),
            (
                "local sin, cos = math.sin, math.cos sin = function(x) return 2 end local x = 0 return sin(x) * cos(x)",
                2.0,
            ),
            (
                "local sin, cos = math.sin, math.cos local function f(x) sin = function() return 2 end return sin(x) * cos(x) end return f(0)",
                2.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let ops = std::iter::once(&program.main)
                .chain(program.functions.iter())
                .flat_map(|function| function.code.iter().map(|instruction| instruction.op))
                .collect::<Vec<_>>();

            assert!(!ops.contains(&Opcode::MathSinCosMul), "{ops:?}");

            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }
    }

    #[test]
    fn compiles_local_math_alias_calls_to_direct_opcodes() {
        let chunk = parse_source(
            "local sin, cos, sqrt, floor = math.sin, math.cos, math.sqrt, math.floor return sin(0) + cos(0) + sqrt(4) + floor(1.9)",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let ops = program
            .main
            .code
            .iter()
            .map(|instruction| instruction.op)
            .collect::<Vec<_>>();

        assert!(ops.contains(&Opcode::MathSin), "{ops:?}");
        assert!(ops.contains(&Opcode::MathCos), "{ops:?}");
        assert!(ops.contains(&Opcode::MathSqrt), "{ops:?}");
        assert!(ops.contains(&Opcode::MathFloor), "{ops:?}");

        let chunk = parse_source(
            "local sin = math.sin local function f(x) return sin(x) + 1 end return f(0)",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let function_ops = program
            .functions
            .iter()
            .flat_map(|function| function.code.iter().map(|instruction| instruction.op))
            .collect::<Vec<_>>();
        assert!(function_ops.contains(&Opcode::MathSin), "{function_ops:?}");
    }

    #[test]
    fn compiles_local_stdlib_alias_calls_to_direct_opcodes() {
        let chunk = parse_source(
            r#"
local sub, find, tonum = string.sub, string.find, tonumber
local function parse(s)
    local comma = find(s, ",", 1, true)
    local number = tonum(sub(s, 1, comma - 1))
    return number
end
return parse("123,456")
"#,
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let function_ops = program
            .functions
            .iter()
            .flat_map(|function| function.code.iter().map(|instruction| instruction.op))
            .collect::<Vec<_>>();

        assert!(
            function_ops.contains(&Opcode::StringFind),
            "{function_ops:?}"
        );
        assert!(
            function_ops.contains(&Opcode::StringSub),
            "{function_ops:?}"
        );
        assert!(function_ops.contains(&Opcode::ToNumber), "{function_ops:?}");

        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number(), Some(123.0));
    }

    #[test]
    fn compiles_string_format_calls_to_direct_opcode() {
        let cases = [
            r#"return string.format("%d:%s", 7, "ok")"#,
            r#"
local fmt = string.format
local function run()
    local text = fmt("%d:%s", 7, "ok")
    return text
end
return run()
"#,
        ];

        for source in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let ops = std::iter::once(&program.main)
                .chain(program.functions.iter())
                .flat_map(|function| function.code.iter().map(|instruction| instruction.op))
                .collect::<Vec<_>>();
            assert!(ops.contains(&Opcode::StringFormat), "{ops:?}");

            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(
                vm.string_value(result).unwrap().as_deref(),
                Some("7:ok"),
                "{source}"
            );
        }
    }

    #[test]
    fn local_math_alias_lowering_preserves_mutation_edges() {
        let cases = [
            (
                "local sin = math.sin sin = function() return 9 end return sin(0)",
                9.0,
            ),
            (
                "math.sin = function() return 9 end local sin = math.sin return sin(0)",
                9.0,
            ),
            (
                "local sin = math.sin math.sin = function() return 9 end return sin(0)",
                0.0,
            ),
            (
                "local sin = math.sin local function f() return sin(0) end sin = function() return 9 end return f()",
                9.0,
            ),
            (
                "local sin = math.sin local function replace() sin = function() return 9 end end replace() return sin(0)",
                9.0,
            ),
            (
                "local sin = math.sin do sin = function() return 9 end end return sin(0)",
                9.0,
            ),
        ];

        for (source, expected) in cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }

        let chunk = parse_source(
            "local abs = math.abs local ok, err = pcall(function() return abs() end) return ok or err:find(\"missing argument #1 to 'abs'\") ~= nil",
        )
        .unwrap();
        let program = compile(&chunk).unwrap();
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn local_stdlib_alias_lowering_preserves_mutation_edges() {
        let number_cases = [
            (
                "local tonum = tonumber tonum = function() return 99 end return tonum('12')",
                99.0,
            ),
            (
                "tonumber = function() return 99 end local tonum = tonumber return tonum('12')",
                99.0,
            ),
            (
                "local tonum = tonumber tonumber = function() return 99 end return tonum('12')",
                12.0,
            ),
            (
                "local find = string.find string.find = function() return 99 end return find('abc', 'b', 1, true)",
                2.0,
            ),
        ];

        for (source, expected) in number_cases {
            let chunk = parse_source(source).unwrap();
            let program = compile(&chunk).unwrap();
            let mut vm = Vm::new(Profile::ReproducibleStandard);
            let result = vm.run(&program).unwrap();
            assert_eq!(result.as_number(), Some(expected), "{source}");
        }

        let string_cases = [
            (
                "local sub = string.sub sub = function() return 'local' end return sub('abc', 1, 1)",
                "local",
            ),
            (
                "string.sub = function() return 'mutated' end local sub = string.sub return sub('abc', 1, 1)",
                "mutated",
            ),
            (
                "local sub = string.sub string.sub = function() return 'mutated' end return sub('abc', 2, 2)",
                "b",
            ),
            (
                "local sub = string.sub local function f() return sub('abc', 2, 2) end sub = function() return 'local' end return f()",
                "local",
            ),
            (
                "local sub = string.sub do sub = function() return 'block' end end return sub('abc', 1, 1)",
                "block",
            ),
            (
                "local fmt = string.format fmt = function() return 'local' end return fmt('%d', 7)",
                "local",
            ),
            (
                "string.format = function() return 'mutated' end local fmt = string.format return fmt('%d', 7)",
                "mutated",
            ),
            (
                "local fmt = string.format string.format = function() return 'mutated' end return fmt('%d', 7)",
                "7",
            ),
            (
                "local fmt = string.format local function f() return fmt('%d', 7) end fmt = function() return 'local' end return f()",
                "local",
            ),
            (
                "string.format = function() return 'mutated' end return string.format('%d', 7)",
                "mutated",
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
    fn break_outside_loop_is_a_compile_error() {
        let chunk = parse_source("break").unwrap();
        let error = compile(&chunk).unwrap_err();
        assert!(error.to_string().contains("break outside loop"));
    }

    #[test]
    fn compile_errors_keep_source_span() {
        let chunk = parse_source("return table.forever({})").unwrap();
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
                "local function make(start, step) local c = start return function() c = c + step return c end end local f = make(1, 2) f() return f()",
                5.0,
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

    fn conformance_options() -> CompileOptions {
        CompileOptions {
            builtins: BuiltinEnvironment::LuauConformance,
            ..CompileOptions::default()
        }
    }

    fn test_loadstring_compiler(
        source: &[u8],
        source_name: Option<&[u8]>,
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
