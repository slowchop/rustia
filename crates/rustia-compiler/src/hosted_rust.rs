//! Experimental Rust source backend for a host-supplied value and operation adapter.
use crate::source_bundle::{validate_module_key, SourceBundle};
use rustia_parser::{
    parse_source_bytes, AssignmentOp, AssignmentTarget, BinaryOp, Expr, FunctionParam, SourceError,
    Stmt, TableField, UnaryOp,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::{self, Write},
};

#[derive(Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            output,
            "{}:{}:{}: {}",
            self.file, self.line, self.column, self.message
        )
    }
}

impl std::error::Error for Diagnostic {}

type Result<T> = std::result::Result<T, Diagnostic>;

/// Capture and builtin lookup policy for the caller's runtime adapter.
pub struct Options<'a> {
    /// Capture names in an isolated callback. Use `None` for a module.
    pub captures: Option<&'a BTreeSet<&'a str>>,
    /// Whether the runtime adapter exposes a named builtin to a callback.
    pub supports_builtin: fn(&str) -> bool,
}

/// The part of a builtin available to a closed module.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Builtin {
    Missing,
    Value,
    Namespace,
}

type CheckCall<'a> = dyn Fn(&str, Option<&[u8]>) -> std::result::Result<(), String> + 'a;

/// Build-time restrictions for modules linked to a closed host interface.
pub struct ClosedHost<'a> {
    pub namespace: &'a str,
    pub builtin: fn(&str, Option<&str>) -> Builtin,
    /// Receives a dot-separated declaration and, when present, its single
    /// literal string argument. The embedder defines its own declarations.
    pub check_call: &'a CheckCall<'a>,
}

/// Emit a standalone module for the value adapter named by `runtime_crate`.
/// The adapter provides standalone operators and retains module state.
pub fn emit_module(
    file: &str,
    source: &[u8],
    function_name: &str,
    runtime_crate: &str,
    host: Option<&ClosedHost<'_>>,
) -> Result<String> {
    emit_source(
        file,
        source,
        function_name,
        Options {
            captures: None,
            supports_builtin: |_| true,
        },
        runtime_crate,
        host,
        true,
    )
}

/// Emit selected standalone modules and a deterministic entry lookup from frozen bytes.
/// The embedder owns file discovery, module execution and the runtime adapter.
pub fn emit_module_bundle(
    bundle: &SourceBundle,
    modules: &[&str],
    runtime_crate: &str,
    host: Option<&ClosedHost<'_>>,
) -> Result<String> {
    let diagnostic = |file: &str, message: &str| Diagnostic {
        file: file.to_owned(),
        line: 1,
        column: 1,
        message: message.to_owned(),
    };
    if !rust_identifier(runtime_crate) {
        return Err(diagnostic(
            "<bundle>",
            "invalid generated Rust runtime crate name",
        ));
    }
    let mut names = BTreeSet::new();
    for &name in modules {
        validate_module_key(name).map_err(|message| diagnostic(name, message))?;
        if !names.insert(name) {
            return Err(diagnostic(name, "duplicate Luau module"));
        }
    }

    let mut output = String::new();
    let mut arms = String::new();
    for (index, name) in names.into_iter().enumerate() {
        let source = bundle
            .source(name)
            .ok_or_else(|| diagnostic(name, "missing Luau module"))?;
        let code = emit_module(name, source, "run", runtime_crate, host)?;
        writeln!(output, "mod script_{index} {{\n{code}\n}}").unwrap();
        writeln!(arms, "        {name:?} => Some(script_{index}::run),").unwrap();
    }
    writeln!(
        output,
        "pub type AotEntry = fn(&mut dyn {runtime_crate}::Host, Vec<{runtime_crate}::Value>) -> Result<{runtime_crate}::Value, {runtime_crate}::Error>;"
    )
    .unwrap();
    if arms.is_empty() {
        output.push_str("pub fn aot_module(_: &str) -> Option<AotEntry> { None }\n");
    } else {
        output.push_str("pub fn aot_module(path: &str) -> Option<AotEntry> {\n    match path {\n");
        output.push_str(&arms);
        output.push_str("        _ => None,\n    }\n}\n");
    }
    Ok(output)
}

/// Lower a module or isolated callback to Rust against a caller-supplied
/// `compiled_runtime` adapter. Unsupported syntax returns a source diagnostic.
/// Selection, capture binding, VM state and execution remain caller-owned.
/// Hosted loops use `Host::loop_checkpoint` and `Host::number`. Interpolation
/// uses `Host::interpolation_value` and the adapter's concatenation operator.
pub fn emit(
    file: &str,
    source: &[u8],
    function_name: &str,
    options: Options<'_>,
) -> Result<String> {
    emit_source(
        file,
        source,
        function_name,
        options,
        "compiled_runtime",
        None,
        false,
    )
}

fn emit_source<'a>(
    file: &'a str,
    source: &'a [u8],
    function_name: &str,
    options: Options<'a>,
    runtime: &'a str,
    closed: Option<&'a ClosedHost<'a>>,
    standalone: bool,
) -> Result<String> {
    let mut emitter = Emitter {
        file,
        runtime,
        closed,
        standalone,
        external: options.captures,
        supports_builtin: options.supports_builtin,
        source,
        scopes: vec![BTreeMap::new()],
        written_globals: BTreeSet::new(),
        captures: Vec::new(),
        next_id: 0,
        variadic: true,
        loop_depth: 0,
        repeat_depth: None,
    };
    if !rust_identifier(function_name) {
        return Err(emitter.error(0, "invalid generated Rust module function name"));
    }
    if !rust_identifier(runtime) {
        return Err(emitter.error(0, "invalid generated Rust runtime crate name"));
    }
    let program = parse_source_bytes(source).map_err(|error| {
        let offset = match &error {
            SourceError::Lex(error) => error.span.start,
            SourceError::Parse(error) => error.span.start,
        };
        emitter.error(offset, error.to_string())
    })?;
    let body = emitter.function_body(&program.statements)?;
    let binary = if standalone { "binary, " } else { "" };
    // No module-specific imports at file scope, so generated items concatenate.
    Ok(format!(
        "#[allow(clippy::all, dead_code, unreachable_code, unused_imports, unused_variables)]\n\
         pub fn {function_name}(host: &mut dyn {runtime}::Host, _args: Vec<{runtime}::Value>) \
         -> Result<{runtime}::Value, {runtime}::Error> {{\n\
         use {runtime}::{{Error, LocalValue, Value, {binary}call, call_method, cell, get, iterate, set, unary}};\n\
         let varargs = _args;\n\
         let result: Result<Value, Error> = (|| {{\n{body}}})();\n\
         Ok(result?.into_first())\n}}\n"
    ))
}

fn rust_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
        && !RUST_KEYWORDS.contains(&name)
}

const RUST_KEYWORDS: &[&str] = &[
    "_", "abstract", "as", "async", "await", "become", "box", "break", "const", "continue",
    "crate", "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if",
    "impl", "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "try", "type",
    "typeof", "union", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

#[derive(Default)]
struct Captures {
    self_binding: Option<usize>,
    first_local: usize,
    used: BTreeSet<usize>,
}

struct Emitter<'a> {
    file: &'a str,
    runtime: &'a str,
    closed: Option<&'a ClosedHost<'a>>,
    standalone: bool,
    external: Option<&'a BTreeSet<&'a str>>,
    supports_builtin: fn(&str) -> bool,
    source: &'a [u8],
    scopes: Vec<BTreeMap<String, usize>>,
    written_globals: BTreeSet<String>,
    captures: Vec<Captures>,
    next_id: usize,
    variadic: bool,
    loop_depth: usize,
    repeat_depth: Option<usize>,
}

impl Emitter<'_> {
    fn error(&self, offset: usize, message: impl Into<String>) -> Diagnostic {
        let prefix = &self.source[..offset.min(self.source.len())];
        Diagnostic {
            file: self.file.to_owned(),
            line: prefix.iter().filter(|&&b| b == b'\n').count() + 1,
            column: prefix
                .rsplit(|&b| b == b'\n')
                .next()
                .unwrap_or_default()
                .len()
                + 1,
            message: message.into(),
        }
    }

    fn fresh(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn declare(&mut self, name: &str) -> usize {
        let id = self.fresh();
        self.scopes.last_mut().unwrap().insert(name.to_owned(), id);
        id
    }

    fn local(&self, name: &str) -> Option<usize> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn reference(&mut self, id: usize) {
        for capture in &mut self.captures {
            if id < capture.first_local {
                capture.used.insert(id);
            }
        }
    }

    fn block(&mut self, statements: &[Stmt]) -> Result<String> {
        self.scopes.push(BTreeMap::new());
        let block = self.statements(statements);
        self.scopes.pop();
        block
    }

    fn statements(&mut self, statements: &[Stmt]) -> Result<String> {
        let mut output = String::new();
        for statement in statements {
            self.statement(statement, &mut output)?;
        }
        Ok(output)
    }

    fn function_body(&mut self, statements: &[Stmt]) -> Result<String> {
        let mut output = self.statements(statements)?;
        if !ends_flow(statements) {
            output.push_str("Ok(Value::Multi(Vec::new()))\n");
        }
        Ok(output)
    }

    fn function(
        &mut self,
        params: &[FunctionParam],
        is_method: bool,
        is_vararg: bool,
        body: &[Stmt],
        binding: Option<usize>,
        offset: usize,
    ) -> Result<String> {
        if self.external.is_some() && !self.captures.is_empty() {
            return Err(self.error(
                offset,
                "callback adapter cannot transfer guest-created closures",
            ));
        }
        let outer_variadic = std::mem::replace(&mut self.variadic, is_vararg);
        let outer_loops = std::mem::take(&mut self.loop_depth);
        let outer_repeat = self.repeat_depth.take();
        self.scopes.push(BTreeMap::new());
        self.captures.push(Captures {
            self_binding: binding,
            first_local: self.next_id,
            used: BTreeSet::new(),
        });
        let mut parameters = String::new();
        let parameter_names = is_method
            .then_some("self")
            .into_iter()
            .chain(params.iter().map(|param| param.name.as_str()));
        for name in parameter_names {
            let id = self.declare(name);
            writeln!(
                parameters,
                "let l{id} = cell(arguments.next().unwrap_or(Value::Nil));"
            )
            .unwrap();
        }
        if is_vararg {
            parameters.push_str("let varargs: Vec<Value> = arguments.collect();\n");
        }
        let body = self.function_body(body)?;
        let mut captures = self.captures.pop().unwrap();
        self.scopes.pop();
        self.variadic = outer_variadic;
        self.loop_depth = outer_loops;
        self.repeat_depth = outer_repeat;

        let mut output = String::from("{\n");
        let recursive = binding.filter(|id| captures.used.remove(id));
        let constructor = match (binding, recursive) {
            (_, Some(id)) => {
                format!(
                    "{}::recursive_local(move |host, l{id}, arguments|",
                    self.runtime
                )
            }
            (Some(_), None) => "cell(Value::function(move |host, arguments|".into(),
            (None, None) => "Value::function(move |host, arguments|".into(),
        };
        let close = if binding.is_some() && recursive.is_none() {
            "))"
        } else {
            ")"
        };
        for id in captures.used {
            writeln!(output, "let l{id} = l{id}.clone();").unwrap();
        }
        writeln!(
            output,
            "{constructor} {{\n\
             #[allow(unused_mut)] let mut arguments = arguments.into_iter();\n\
             {parameters}{body}}}{close}\n}}"
        )
        .unwrap();
        Ok(output)
    }

    fn statement(&mut self, statement: &Stmt, output: &mut String) -> Result<()> {
        let runtime = self.runtime;
        let checkpoint = if self.standalone {
            ""
        } else {
            "host.loop_checkpoint()?;\n"
        };
        match statement {
            Stmt::Local { bindings, init, .. } => {
                let init = self.values(init)?;
                let id = self.fresh();
                writeln!(output, "let mut values{id} = ({init}).into_iter();").unwrap();
                for binding in bindings {
                    let binding = self.declare(&binding.name);
                    writeln!(
                        output,
                        "let l{binding} = cell(values{id}.next().unwrap_or(Value::Nil));"
                    )
                    .unwrap();
                }
            }
            Stmt::LocalFunction {
                name,
                params,
                is_vararg,
                body,
                span,
                ..
            } => {
                let id = self.declare(name);
                let function =
                    self.function(params, false, *is_vararg, body, Some(id), span.start)?;
                writeln!(output, "let l{id} = {function};").unwrap();
            }
            Stmt::Function {
                target,
                is_method,
                params,
                is_vararg,
                body,
                span,
                ..
            } => {
                let target = self.target(target, output)?;
                let function =
                    self.function(params, *is_method, *is_vararg, body, None, span.start)?;
                writeln!(output, "{}", target.write(&function)).unwrap();
            }
            Stmt::Assignment {
                targets,
                op,
                values,
                span,
            } => {
                self.assignment(targets, *op, values, span.start, output)?;
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                let condition = self.expr(condition)?;
                let then_body = self.block(then_body)?;
                writeln!(output, "if ({condition}).truthy() {{\n{then_body}}}").unwrap();
                if !else_body.is_empty() {
                    let else_body = self.block(else_body)?;
                    writeln!(output, "else {{\n{else_body}}}").unwrap();
                }
            }
            Stmt::While {
                condition, body, ..
            } => {
                let condition = self.expr(condition)?;
                let body = self.loop_body(body)?;
                writeln!(
                    output,
                    "while ({condition}).truthy() {{\n{checkpoint}{body}}}"
                )
                .unwrap();
            }
            Stmt::ForNumeric {
                variable,
                start,
                limit,
                step,
                body,
                ..
            } => {
                let start = self.expr(start)?;
                let limit = self.expr(limit)?;
                let step = step
                    .as_ref()
                    .map(|step| self.expr(step))
                    .transpose()?
                    .unwrap_or_else(|| "Value::Number(1.0)".to_owned());
                self.scopes.push(BTreeMap::new());
                let variable = self.declare(variable);
                let body = self.loop_body(body)?;
                self.scopes.pop();
                let bounds = if self.standalone {
                    format!(
                        "let mut current = ({start}).number()?;\n\
                         let limit = ({limit}).number()?;\n\
                         let step = ({step}).number()?;\n"
                    )
                } else {
                    format!(
                        "let start_value = {start};\n\
                         let limit_value = {limit};\n\
                         let step_value = {step};\n\
                         let mut current = host.number(&start_value)?;\n\
                         let limit = host.number(&limit_value)?;\n\
                         let step = host.number(&step_value)?;\n"
                    )
                };
                writeln!(
                    output,
                    "{{\n{bounds}while if step > 0.0 {{ current <= limit }} else {{ current >= limit }} {{\n\
                     {checkpoint}let l{variable} = cell(Value::Number(current));\n\
                     let next = current + step;\n\
                     current = next;\n{body}}}\n}}"
                )
                .unwrap();
            }
            Stmt::ForGeneric {
                variables,
                iter,
                body,
                ..
            } => {
                let iter = self.values(iter)?;
                self.scopes.push(BTreeMap::new());
                let mut bindings = String::new();
                for variable in variables {
                    let id = self.declare(&variable.name);
                    writeln!(
                        bindings,
                        "let l{id} = cell(row.next().unwrap_or(Value::Nil));"
                    )
                    .unwrap();
                }
                let body = self.loop_body(body)?;
                self.scopes.pop();
                writeln!(
                    output,
                    "{{\nlet input = {iter};\n\
                     let mut rows = {runtime}::LoopRows::new(host, input)?;\n\
                     while let Some(row) = rows.next_row(host)? {{\n\
                     {checkpoint}let mut row = row.into_iter();\n{bindings}{body}}}\n}}"
                )
                .unwrap();
            }
            Stmt::Do { body, .. } => writeln!(output, "{{\n{}}}", self.block(body)?).unwrap(),
            Stmt::Break { span } | Stmt::Continue { span } => {
                if self.loop_depth == 0 {
                    return Err(self.error(span.start, "loop control outside a loop"));
                }
                let keyword = if matches!(statement, Stmt::Break { .. }) {
                    "break"
                } else {
                    if self.repeat_depth == Some(self.loop_depth) {
                        return Err(
                            self.error(span.start, "continue in a repeat loop is unsupported")
                        );
                    }
                    "continue"
                };
                writeln!(output, "{keyword};").unwrap();
            }
            Stmt::Return {
                value,
                extra_values,
                ..
            } => {
                let values: Vec<_> = value.iter().chain(extra_values).collect();
                let values = self.value_list(&values)?;
                writeln!(output, "return Ok(Value::Multi({values}));").unwrap();
            }
            Stmt::Expr { expr, .. } => {
                writeln!(output, "let _ = {};", self.raw_expr(expr)?).unwrap();
            }
            Stmt::TypeAlias { .. } | Stmt::TypeFunction { .. } => {}
            Stmt::Repeat {
                body, condition, ..
            } => {
                self.loop_depth += 1;
                let outer_repeat = self.repeat_depth.replace(self.loop_depth);
                self.scopes.push(BTreeMap::new());
                let body = self.statements(body)?;
                let condition = self.expr(condition)?;
                self.scopes.pop();
                self.repeat_depth = outer_repeat;
                self.loop_depth -= 1;
                writeln!(
                    output,
                    "loop {{\n{checkpoint}{body}if ({condition}).truthy() {{ break; }}\n}}"
                )
                .unwrap();
            }
        }
        Ok(())
    }

    fn loop_body(&mut self, statements: &[Stmt]) -> Result<String> {
        self.loop_depth += 1;
        let body = self.block(statements);
        self.loop_depth -= 1;
        body
    }

    fn assignment(
        &mut self,
        targets: &[AssignmentTarget],
        op: AssignmentOp,
        values: &[Expr],
        offset: usize,
        output: &mut String,
    ) -> Result<()> {
        if op == AssignmentOp::Assign {
            let targets = targets
                .iter()
                .map(|target| self.target(target, output))
                .collect::<Result<Vec<_>>>()?;
            let values = self.values(values)?;
            let id = self.fresh();
            writeln!(output, "let mut values{id} = ({values}).into_iter();").unwrap();
            for target in targets {
                let value = format!("values{id}.next().unwrap_or(Value::Nil)");
                writeln!(output, "{}", target.write(&value)).unwrap();
            }
        } else {
            let ([target], [value]) = (targets, values) else {
                return Err(self.error(offset, "compound assignment needs one target and value"));
            };
            let target = self.target(target, output)?;
            let left = target.read();
            let right = self.expr(value)?;
            let op = match op {
                AssignmentOp::Assign => unreachable!(),
                AssignmentOp::Add => "+",
                AssignmentOp::Sub => "-",
                AssignmentOp::Mul => "*",
                AssignmentOp::Div => "/",
                AssignmentOp::FloorDiv => "//",
                AssignmentOp::Mod => "%",
                AssignmentOp::Pow => "^",
                AssignmentOp::Concat => "..",
            };
            let binary = if self.standalone {
                "binary"
            } else {
                "host.binary"
            };
            let calc = format!(
                "{{ let left = {left}; let right = {right}; {binary}({op:?}, left, right)? }}"
            );
            writeln!(output, "{}", target.write(&calc)).unwrap();
        }
        Ok(())
    }

    fn target(&mut self, target: &AssignmentTarget, output: &mut String) -> Result<Target> {
        let (object, key) = match target {
            AssignmentTarget::Local { name, span } => {
                let Some(id) = self.local(name) else {
                    if let Some(external) = self.external {
                        if external.contains(name.as_str()) {
                            return Ok(Target::Capture(name.to_owned()));
                        }
                        return Err(
                            self.error(span.start, format!("cannot assign live global {name:?}"))
                        );
                    }
                    if self.closed.is_some() {
                        return Err(self.error(
                            span.start,
                            format!(
                                "global assignment to {name:?} is unsupported by this closed host"
                            ),
                        ));
                    }
                    self.written_globals.insert(name.to_owned());
                    return Ok(Target::Global(name.to_owned()));
                };
                if self
                    .captures
                    .last()
                    .is_some_and(|scope| scope.self_binding == Some(id))
                {
                    return Err(self.error(span.start, "cannot assign to function self binding"));
                }
                self.reference(id);
                return Ok(Target::Local(id));
            }
            AssignmentTarget::Field { object, field, .. } => (object, string(field)),
            AssignmentTarget::Index { object, index, .. } => (object, self.expr(index)?),
        };
        let object = self.expr(object)?;
        let id = self.fresh();
        writeln!(output, "let object{id} = {object};\nlet key{id} = {key};").unwrap();
        Ok(Target::Table(id))
    }

    fn values(&mut self, expressions: &[Expr]) -> Result<String> {
        self.value_list(&expressions.iter().collect::<Vec<_>>())
    }

    fn value_list(&mut self, expressions: &[&Expr]) -> Result<String> {
        let Some((last, prefix)) = expressions.split_last() else {
            return Ok("Vec::new()".to_owned());
        };
        let prefix = prefix
            .iter()
            .map(|expr| self.expr(expr))
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        if expands(last) {
            let last = self.raw_expr(last)?;
            Ok(format!(
                "{{ let mut values = vec![{prefix}]; values.extend(({last}).into_values()); values }}"
            ))
        } else {
            let last = self.expr(last)?;
            let comma = if prefix.is_empty() { "" } else { ", " };
            Ok(format!("vec![{prefix}{comma}{last}]"))
        }
    }

    fn expr(&mut self, expression: &Expr) -> Result<String> {
        let source = self.raw_expr(expression)?;
        if expands(expression) {
            Ok(format!("({source}).into_first()"))
        } else {
            Ok(source)
        }
    }

    fn raw_expr(&mut self, expression: &Expr) -> Result<String> {
        let runtime = self.runtime;
        Ok(match expression {
            Expr::Nil(_) => "Value::Nil".to_owned(),
            Expr::Bool(value, _) => format!("Value::Bool({value})"),
            Expr::Number(value, _) => number(*value),
            Expr::String(value, _) => match std::str::from_utf8(value) {
                Ok(text) => string(text),
                Err(_) => format!("Value::from_bytes(vec!{value:?})"),
            },
            Expr::InterpolatedString {
                parts, expressions, ..
            } => {
                if !self.standalone {
                    let mut code = String::from("{ ");
                    let mut first = true;
                    for part in parts {
                        let part = match part {
                            rustia_parser::InterpolatedStringPart::Text(bytes) => {
                                if bytes.is_empty() {
                                    continue;
                                }
                                format!("Value::from_bytes(vec!{bytes:?})")
                            }
                            rustia_parser::InterpolatedStringPart::Expr(index) => {
                                let value = self.expr(&expressions[*index])?;
                                format!(
                                    "{{ let value = {value}; host.interpolation_value(value)? }}"
                                )
                            }
                        };
                        if first {
                            writeln!(code, "#[allow(unused_mut)] let mut text = {part};").unwrap();
                            first = false;
                        } else {
                            writeln!(
                                code,
                                "let part = {part}; text = host.binary(\"..\", text, part)?;"
                            )
                            .unwrap();
                        }
                    }
                    code.push_str(if first {
                        "Value::String(String::new()) }"
                    } else {
                        "text }"
                    });
                    return Ok(code);
                }
                let mut code = String::from("{ let mut text = Vec::<u8>::new();");
                for part in parts {
                    match part {
                        rustia_parser::InterpolatedStringPart::Text(bytes) => {
                            writeln!(code, "text.extend_from_slice(&{bytes:?});").unwrap();
                        }
                        rustia_parser::InterpolatedStringPart::Expr(index) => {
                            let value = self.expr(&expressions[*index])?;
                            writeln!(code, "text.extend({runtime}::string_bytes(&({value})));")
                                .unwrap();
                        }
                    }
                }
                code.push_str("Value::from_bytes(text) }");
                code
            }
            Expr::Name(name, span) => match self.local(name) {
                Some(id) => {
                    self.reference(id);
                    if self
                        .captures
                        .last()
                        .is_some_and(|scope| scope.self_binding == Some(id))
                    {
                        format!("l{id}.original()")
                    } else {
                        Target::Local(id).read()
                    }
                }
                None if self
                    .external
                    .is_some_and(|bindings| bindings.contains(name.as_str())) =>
                {
                    format!("host.capture({name:?})?")
                }
                None if self.external.is_some() && !(self.supports_builtin)(name) => {
                    return Err(self.error(span.start, format!("unsupported live global {name:?}")));
                }
                None if let Some(closed) = self.closed => {
                    if name == closed.namespace {
                        return Err(self.error(
                            span.start,
                            format!(
                                "the closed host API requires a direct {} declaration call",
                                closed.namespace
                            ),
                        ));
                    }
                    match (closed.builtin)(name, None) {
                        Builtin::Namespace => {
                            return Err(self.error(
                                span.start,
                                format!("closed host requires a named field of {name}"),
                            ))
                        }
                        Builtin::Missing => {
                            return Err(
                                self.error(span.start, format!("unsupported host global {name:?}"))
                            )
                        }
                        Builtin::Value => format!("host.global({name:?})?"),
                    }
                }
                None if name == "require" => format!("host.require_function({:?})?", self.file),
                None => format!("host.global({name:?})?"),
            },
            Expr::Varargs(_) if self.variadic => "Value::Multi(varargs.clone())".to_owned(),
            Expr::Varargs(span) => {
                return Err(self.error(span.start, "varargs used outside variadic function"));
            }
            Expr::Unary { op, expr, .. } => {
                let expr = self.expr(expr)?;
                let op = match op {
                    UnaryOp::Neg => "-",
                    UnaryOp::Not => "not",
                    UnaryOp::Len => "#",
                };
                format!("{{ let value = {expr}; host.unary({op:?}, value)? }}")
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                let lhs = self.expr(lhs)?;
                let rhs = self.expr(rhs)?;
                match op {
                    BinaryOp::And => {
                        format!(
                            "{{ let left = {lhs}; if left.truthy() {{ {rhs} }} else {{ left }} }}"
                        )
                    }
                    BinaryOp::Or => {
                        format!(
                            "{{ let left = {lhs}; if left.truthy() {{ left }} else {{ {rhs} }} }}"
                        )
                    }
                    BinaryOp::Eq | BinaryOp::Ne => {
                        let negate = *op == BinaryOp::Ne;
                        format!(
                            "{{ let left = {lhs}; let right = {rhs}; {runtime}::equals(host, left, right, {negate})? }}"
                        )
                    }
                    _ => {
                        let operator = binary_operator(*op);
                        if self.standalone {
                            format!("binary({operator:?}, {lhs}, {rhs})?")
                        } else {
                            format!("{{ let left = {lhs}; let right = {rhs}; host.binary({operator:?}, left, right)? }}")
                        }
                    }
                }
            }
            Expr::IfElse {
                condition,
                then_expr,
                else_expr,
                ..
            } => format!(
                "if ({}).truthy() {{ {} }} else {{ {} }}",
                self.expr(condition)?,
                self.expr(then_expr)?,
                self.expr(else_expr)?
            ),
            Expr::Field { object, field, .. } => {
                let object = self.field_object(object, field)?;
                let key = string(field);
                format!("{{ let object = {object}; get(host, object, {key})? }}")
            }
            Expr::Index { object, index, .. } => {
                let object = match index.as_ref() {
                    Expr::String(field, _) if let Ok(field) = std::str::from_utf8(field) => {
                        self.field_object(object, field)?
                    }
                    _ => self.expr(object)?,
                };
                let key = self.expr(index)?;
                format!("{{ let object = {object}; let key = {key}; get(host, object, key)? }}")
            }
            Expr::Table { fields, .. } => self.table(fields)?,
            Expr::Function {
                params,
                is_vararg,
                body,
                ..
            } => self.function(
                params,
                false,
                *is_vararg,
                body,
                None,
                expression.span().start,
            )?,
            Expr::Paren { expr, .. } | Expr::TypeInstantiation { expr, .. } => self.expr(expr)?,
            Expr::Call { callee, args, span } => {
                let mut target = callee.as_ref();
                while let Expr::TypeInstantiation { expr, .. } = target {
                    target = expr;
                }
                if self.closed.is_some()
                    && matches!(target, Expr::Name(name, _) if name == "require" && self.local(name).is_none())
                {
                    let [Expr::String(specifier, _)] = args.as_slice() else {
                        return Err(self.error(
                            span.start,
                            "require expects a literal module specifier for this host",
                        ));
                    };
                    let specifier = std::str::from_utf8(specifier)
                        .map_err(|_| self.error(span.start, "require specifier must be UTF-8"))?;
                    return Ok(format!("host.require({:?}, {specifier:?})?", self.file));
                }
                let deferred = self.defer_builtin(callee, args);
                let declaration = self.closed.and_then(|host| {
                    if self.local(host.namespace).is_some() {
                        return None;
                    }
                    host_fields(target, host.namespace).map(|fields| (host, fields))
                });
                let callee = if let Some((host, fields)) = declaration {
                    let literal = match args.as_slice() {
                        [Expr::String(raw, _)] => Some(raw.as_slice()),
                        _ => None,
                    };
                    (host.check_call)(&fields.join("."), literal)
                        .map_err(|message| self.error(span.start, message))?;
                    let namespace = host.namespace;
                    let mut lookup = format!("host.global({namespace:?})?");
                    for field in fields {
                        let key = string(field);
                        lookup = format!("{{ let parent = {lookup}; get(host, parent, {key})? }}");
                    }
                    lookup
                } else {
                    self.expr(callee)?
                };
                let arguments = self.values(args)?;
                if deferred {
                    format!(
                        "{{ let arguments = {arguments}; let function = {callee}; call(host, function, arguments)? }}"
                    )
                } else {
                    format!(
                        "{{ let function = {callee}; let arguments = {arguments}; call(host, function, arguments)? }}"
                    )
                }
            }
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => {
                let object = self.expr(object)?;
                let arguments = self.values(args)?;
                format!(
                    "{{ let object = {object}; let arguments = {arguments}; call_method(host, object, {method:?}, arguments)? }}"
                )
            }
        })
    }

    // The pinned Luau builtin list changes callee lookup order even when the
    // table's function has been replaced. Parentheses and lexical aliases
    // deliberately take the ordinary evaluate-callee-first path.
    fn defer_builtin(&self, callee: &Expr, args: &[Expr]) -> bool {
        let root = match callee {
            Expr::Name(name, _)
                if crate::defers_builtin_lookup(
                    name,
                    None,
                    matches!(args, [_, Expr::Varargs(_)]),
                ) =>
            {
                name
            }
            Expr::Field { object, field, .. }
                if let Expr::Name(table, _) = object.as_ref()
                    && crate::defers_builtin_lookup(table, Some(field), false) =>
            {
                table
            }
            _ => return false,
        };
        self.local(root).is_none()
            && !self.written_globals.contains(root)
            && !self
                .external
                .is_some_and(|names| names.contains(root.as_str()))
    }

    fn field_object(&mut self, object: &Expr, field: &str) -> Result<String> {
        let (Some(host), Expr::Name(name, span)) = (self.closed, object) else {
            return self.expr(object);
        };
        if self.local(name).is_some() || (host.builtin)(name, None) != Builtin::Namespace {
            return self.expr(object);
        }
        if (host.builtin)(name, Some(field)) == Builtin::Missing {
            return Err(self.error(span.start, format!("unsupported builtin {name}.{field}")));
        }
        Ok(format!("host.global({name:?})?"))
    }

    fn table(&mut self, fields: &[TableField]) -> Result<String> {
        let mut init = Vec::new();
        let mut array_index = 0;
        let mut expanded = None;
        for (index, field) in fields.iter().enumerate() {
            let (key, value) = match field {
                TableField::Array { value, .. } => {
                    array_index += 1;
                    if index + 1 == fields.len() && expands(value) {
                        expanded = Some(self.raw_expr(value)?);
                        continue;
                    }
                    (format!("Value::Number({array_index}.0)"), value)
                }
                TableField::Named { name, value, .. } => (string(name), value),
                TableField::Keyed { key, value, .. } => (self.expr(key)?, value),
            };
            let value = self.expr(value)?;
            init.push(format!("({key}, {value})"));
        }
        let init = init.join(", ");
        if let Some(values) = expanded {
            Ok(format!(
                "{{ let mut fields = vec![{init}];\n\
                 for (index, value) in ({values}).into_values().into_iter().enumerate() {{\n\
                 fields.push((Value::Number(({array_index} + index) as f64), value));\n}}\n\
                 host.make_table(fields)? }}"
            ))
        } else {
            Ok(format!(
                "{{ let fields = vec![{init}]; host.make_table(fields)? }}"
            ))
        }
    }
}

fn host_fields<'a>(mut callee: &'a Expr, namespace: &str) -> Option<Vec<&'a str>> {
    let mut path = Vec::new();
    while let Expr::Field { object, field, .. } = callee {
        path.push(field.as_str());
        callee = object;
    }
    if path.is_empty() || !matches!(callee, Expr::Name(name, _) if name == namespace) {
        return None;
    }
    path.reverse();
    Some(path)
}

fn binary_operator(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::And | BinaryOp::Or => unreachable!(),
        BinaryOp::Eq => "==",
        BinaryOp::Ne => "~=",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        BinaryOp::Concat => "..",
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::FloorDiv => "//",
        BinaryOp::Mod => "%",
        BinaryOp::Pow => "^",
    }
}

fn string(value: &str) -> String {
    format!("Value::String(String::from({value:?}))")
}

fn number(value: f64) -> String {
    format!("Value::Number(f64::from_bits(0x{:016x}))", value.to_bits())
}

fn expands(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Call { .. } | Expr::MethodCall { .. } | Expr::Varargs(_)
    )
}

fn ends_flow(statements: &[Stmt]) -> bool {
    match statements.last() {
        Some(Stmt::Return { .. } | Stmt::Break { .. } | Stmt::Continue { .. }) => true,
        Some(Stmt::Do { body, .. }) => ends_flow(body),
        Some(Stmt::If {
            then_body,
            else_body,
            ..
        }) => ends_flow(then_body) && ends_flow(else_body),
        _ => false,
    }
}

enum Target {
    Capture(String),
    Global(String),
    Local(usize),
    Table(usize),
}

impl Target {
    fn read(&self) -> String {
        match self {
            Self::Capture(name) => format!("host.capture({name:?})?"),
            Self::Global(name) => format!("host.global({name:?})?"),
            Self::Local(id) => format!("l{id}.read_local()"),
            Self::Table(id) => format!("get(host, object{id}.clone(), key{id}.clone())?"),
        }
    }

    fn write(&self, expression: &str) -> String {
        // Evaluate the value before admitting either the host or the local
        // lock. A callback on the right may read or modify that same cell.
        match self {
            Self::Capture(name) => {
                format!("{{ let value = {expression}; host.set_capture({name:?}, value)?; }}")
            }
            Self::Global(name) => {
                format!("{{ let value = {expression}; host.set_global({name:?}, value)?; }}")
            }
            Self::Local(id) => format!("{{ let value = {expression}; l{id}.write_local(value); }}"),
            Self::Table(id) => {
                format!("{{ let value = {expression}; set(host, object{id}, key{id}, value)?; }}")
            }
        }
    }
}

#[cfg(test)]
mod bundle_tests {
    use super::*;

    #[test]
    fn selected_modules_are_lowered_from_the_snapshot_in_sorted_order() {
        let bundle = SourceBundle::new([
            ("z.luau", "return 2"),
            ("unused.luau", "invalid Luau syntax"),
            ("nested/a.luau", "return 1"),
        ])
        .unwrap();
        let emit = |names: &[&str]| emit_module_bundle(&bundle, names, "adapter", None).unwrap();
        let first = emit(&["z.luau", "nested/a.luau"]);
        assert_eq!(first, emit(&["nested/a.luau", "z.luau"]));
        assert!(first.contains("\"nested/a.luau\" => Some(script_0::run)"));
        assert!(first.contains("\"z.luau\" => Some(script_1::run)"));
        assert!(!first.contains("unused.luau"));
        assert!(emit(&[]).contains("fn aot_module(_: &str) -> Option<AotEntry> { None }"));
    }

    #[test]
    fn reports_invalid_selection_and_preserves_source_diagnostics() {
        let bundle = SourceBundle::new([("broken.luau", "-- header\nlocal = 3")]).unwrap();
        let emit =
            |names: &[&str]| emit_module_bundle(&bundle, names, "adapter", None).unwrap_err();
        assert!(emit(&["broken.luau", "broken.luau"])
            .message
            .contains("duplicate"));
        assert!(emit(&["../broken.luau"])
            .message
            .contains("project-relative"));
        assert!(emit(&["missing.luau"]).message.contains("missing"));
        let error = emit(&["broken.luau"]);
        assert_eq!(error.file, "broken.luau");
        assert_eq!(error.line, 2);
    }
}
