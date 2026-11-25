//! Experimental Rust lowering for a bounded, capture-backed host callback.
use crate::{CompileError, Result};
use rustia_lexer::{lex_for_parser, Span};
use rustia_parser::{BinaryOp, Expr, Parser, Stmt};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};

pub const MAX_SOURCE_BYTES: usize = 4096;
const MAX_TOKENS: usize = 128;

const INTERFACE: &str = r#"pub trait Host {
    type Value;
    type Error;
    fn call(&mut self, capture: &str, args: &[&str]) -> Result<Self::Value, Self::Error>;
    fn method(&mut self, capture: &str, method: &str, args: &[&str]) -> Result<Self::Value, Self::Error>;
    fn truthy(&self, value: &Self::Value) -> bool;
    fn equals_text(&self, value: &Self::Value, text: &str) -> bool;
}

pub fn compiled_host_call<H: Host>(host: &mut H) -> Result<(), H::Error> {
"#;

/// Compile the body of a zero-argument callback. The interpreter still
/// evaluates setup and owns captures, values and invocation policy.
/// The host must bind the output to the exact source and capture declaration.
/// Generated control flow is not metered as interpreter bytecode.
pub fn generate(source: &str) -> Result<String> {
    let span = Span::new(0, source.len());
    if source.len() > MAX_SOURCE_BYTES {
        return Err(CompileError::at(
            "host callback source exceeds its byte limit",
            span,
        ));
    }
    let tokens = lex_for_parser(source.as_bytes())
        .map_err(|error| CompileError::at(error.message, error.span))?;
    if let Some(token) = tokens.get(MAX_TOKENS) {
        return Err(CompileError::at(
            "host callback source exceeds its token limit",
            token.span,
        ));
    }
    let chunk = Parser::new(tokens)
        .parse_chunk()
        .map_err(|error| CompileError::at(error.message, error.span))?;
    let Some((
        Stmt::Return {
            value:
                Some(Expr::Function {
                    attributes,
                    params,
                    is_vararg: false,
                    body,
                    span: callback_span,
                }),
            extra_values,
            ..
        },
        setup,
    )) = chunk.statements.split_last()
    else {
        return Err(CompileError::at("expected a returned host callback", span));
    };
    if !attributes.is_empty() || !params.is_empty() || !extra_values.is_empty() {
        return Err(CompileError::at(
            "expected an unadorned zero-argument callback",
            span,
        ));
    }

    let mut captures = BTreeSet::new();
    for statement in setup {
        let Stmt::Local {
            bindings,
            init,
            span,
        } = statement
        else {
            return Err(CompileError::at("unsupported host callback setup", span));
        };
        let ([binding], [Expr::Call { callee, args, .. }]) = (bindings.as_slice(), init.as_slice())
        else {
            return Err(CompileError::at(
                "expected one setup capture per call",
                *span,
            ));
        };
        if !matches!(callee.as_ref(), Expr::Field { object, .. } if matches!(object.as_ref(), Expr::Name(..)))
            || !matches!(args.as_slice(), [Expr::String(..)])
            || !captures.insert(binding.name.as_str())
        {
            return Err(CompileError::at("unsupported host callback capture", *span));
        }
    }
    let mut emitted = INTERFACE.to_owned();
    Emitter {
        captures,
        callback_span: *callback_span,
    }
    .block(body, &mut emitted, BTreeMap::new(), 1)?;
    emitted.push_str("    Ok(())\n}\n");
    Ok(emitted)
}

struct Emitter<'a> {
    captures: BTreeSet<&'a str>,
    callback_span: Span,
}

impl Emitter<'_> {
    fn block<'a>(
        &self,
        statements: &'a [Stmt],
        out: &mut String,
        mut locals: BTreeMap<&'a str, String>,
        depth: usize,
    ) -> Result<()> {
        let indent = "    ".repeat(depth);
        for statement in statements {
            match statement {
                Stmt::Local {
                    bindings,
                    init,
                    span,
                } => {
                    let ([binding], [value]) = (bindings.as_slice(), init.as_slice()) else {
                        return Err(CompileError::at("expected one callback local", *span));
                    };
                    if self.captures.contains(binding.name.as_str())
                        || locals.contains_key(binding.name.as_str())
                    {
                        return Err(CompileError::at(
                            "shadowing is unsupported in compiled callbacks",
                            *span,
                        ));
                    }
                    let value = self.call(value)?;
                    let local = format!("value_{}", locals.len());
                    writeln!(out, "{indent}let {local} = {value};").unwrap();
                    locals.insert(binding.name.as_str(), local);
                }
                Stmt::Expr { expr, .. } => {
                    let call = self.call(expr)?;
                    writeln!(out, "{indent}{call};").unwrap();
                }
                Stmt::If {
                    condition,
                    then_body,
                    else_body,
                    ..
                } => {
                    let condition = self.condition(condition, &locals)?;
                    let condition = condition
                        .strip_prefix('(')
                        .and_then(|inside| inside.strip_suffix(')'))
                        .unwrap_or(&condition);
                    writeln!(out, "{indent}if {condition} {{").unwrap();
                    self.block(then_body, out, locals.clone(), depth + 1)?;
                    if !else_body.is_empty() {
                        writeln!(out, "{indent}}} else {{").unwrap();
                        self.block(else_body, out, locals.clone(), depth + 1)?;
                    }
                    writeln!(out, "{indent}}}").unwrap();
                }
                _ => {
                    return Err(CompileError::at(
                        "unsupported compiled host callback statement",
                        self.callback_span,
                    ))
                }
            }
        }
        Ok(())
    }

    fn call(&self, expr: &Expr) -> Result<String> {
        let (capture, method, args) = match expr {
            Expr::Call { callee, args, .. } => (callee.as_ref(), None, args),
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => (object.as_ref(), Some(method), args),
            _ => {
                return Err(CompileError::at(
                    "expected a captured host call",
                    expr.span(),
                ))
            }
        };
        let Expr::Name(name, _) = capture else {
            return Err(CompileError::at("expected a named capture", capture.span()));
        };
        if !self.captures.contains(name.as_str()) {
            return Err(CompileError::at(
                "call target is not a setup capture",
                capture.span(),
            ));
        }
        let args = args
            .iter()
            .map(text_literal)
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        Ok(match method {
            Some(method) => format!("host.method({name:?}, {method:?}, &[{args}])?"),
            None => format!("host.call({name:?}, &[{args}])?"),
        })
    }

    fn condition(&self, expr: &Expr, locals: &BTreeMap<&str, String>) -> Result<String> {
        match expr {
            Expr::Paren { expr, .. } => self.condition(expr, locals),
            Expr::Binary {
                op: op @ (BinaryOp::And | BinaryOp::Or),
                lhs,
                rhs,
                ..
            } => {
                let operator = if *op == BinaryOp::And { "&&" } else { "||" };
                Ok(format!(
                    "({} {operator} {})",
                    self.condition(lhs, locals)?,
                    self.condition(rhs, locals)?
                ))
            }
            Expr::Binary {
                op: BinaryOp::Eq,
                lhs,
                rhs,
                ..
            } => {
                let Expr::Name(name, _) = lhs.as_ref() else {
                    return Err(CompileError::at(
                        "expected a callback local in comparison",
                        lhs.span(),
                    ));
                };
                let local = locals
                    .get(name.as_str())
                    .ok_or_else(|| CompileError::at("unknown callback local", lhs.span()))?;
                Ok(format!(
                    "host.equals_text(&{local}, {})",
                    text_literal(rhs)?
                ))
            }
            Expr::Call { .. } | Expr::MethodCall { .. } => Ok(format!(
                "{{ let value = {}; host.truthy(&value) }}",
                self.call(expr)?
            )),
            _ => Err(CompileError::at(
                "unsupported compiled host condition",
                expr.span(),
            )),
        }
    }
}

fn text_literal(expr: &Expr) -> Result<String> {
    let Expr::String(bytes, span) = expr else {
        return Err(CompileError::at("expected a text literal", expr.span()));
    };
    let text = std::str::from_utf8(bytes)
        .map_err(|_| CompileError::at("compiled host arguments require UTF-8 text", *span))?;
    Ok(format!("{text:?}"))
}

#[cfg(test)]
mod tests {
    use super::generate;

    #[test]
    fn emits_capture_calls_and_short_circuits_conditions() {
        let rust = generate(
            r#"
            local screen = host.state("Screen")
            local pressed = host.native("key")
            return function()
                local current = screen:get()
                if current == "Title" and pressed("Enter") then screen:set("Playing") end
            end
        "#,
        )
        .unwrap();
        assert!(rust.contains(r#"host.method("screen", "get", &[])"#));
        assert!(rust.contains(r#"&& { let value = host.call("pressed", &["Enter"])"#));
        assert!(rust.contains(r#"host.method("screen", "set", &["Playing"])"#));
    }

    #[test]
    fn rejects_unsupported_syntax_without_eliding_effects() {
        let source =
            r#"local action = host.native("fire") return function() while action() do end end"#;
        let error = generate(source).unwrap_err();
        assert!(error.span.is_some());
        assert!(error.message.contains("unsupported"));
        for body in [
            "other()",
            "action(other())",
            "return action()",
            "local action = action()",
        ] {
            let source =
                format!(r#"local action = host.native("fire") return function() {body} end"#);
            assert!(generate(&source).unwrap_err().span.is_some(), "{body}");
        }
    }
}
