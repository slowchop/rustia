//! Experimental generated Rust for one statically numeric Luau closure.
use crate::CompileError;
use rustia_lexer::{lex_for_parser, Span};
use rustia_parser::{AssignmentOp, AssignmentTarget, BinaryOp, Expr, Parser, Stmt};
use rustia_vm::number::CANON_NAN_BITS;

/// Maximum source size accepted by the experimental compiled-call driver.
pub const MAX_SOURCE_BYTES: usize = 4096;
// This small grammar needs few tokens. Bound the token stream before entering
// the recursive parser, including ineligible syntax and long flat expressions.
const MAX_TOKENS: usize = 128;

/// Build input for an experimental compiled call. The host owns the capture
/// storage and must use the interpreter for calls subject to VM budgets.
#[derive(Debug)]
pub struct GeneratedCall {
    pub rust: String,
    pub capture_name: String,
    pub initial_capture: f64,
}

/// Lower a single source module defining one numeric capture and returning a
/// unary closure. The closure may update that capture once before returning.
pub fn generate(source: &str) -> Result<GeneratedCall, CompileError> {
    let module_span = Span::new(0, source.len());
    if source.len() > MAX_SOURCE_BYTES {
        return Err(CompileError::at(
            format!("compiled module exceeds {MAX_SOURCE_BYTES} source bytes"),
            module_span,
        ));
    }
    let tokens = lex_for_parser(source.as_bytes())
        .map_err(|error| CompileError::at(error.message, error.span))?;
    if let Some(token) = tokens.get(MAX_TOKENS) {
        return Err(CompileError::at(
            "compiled module exceeds its token budget",
            token.span,
        ));
    }
    let chunk = Parser::new(tokens)
        .parse_chunk()
        .map_err(|error| CompileError::at(error.message, error.span))?;
    let [Stmt::Local {
        bindings,
        init,
        span,
    }, Stmt::Return {
        value:
            Some(Expr::Function {
                attributes,
                params,
                is_vararg: false,
                body,
                span: function_span,
            }),
        extra_values,
        ..
    }] = chunk.statements.as_slice()
    else {
        return Err(CompileError::at(
            "expected one numeric local and a returned closure",
            module_span,
        ));
    };
    let ([binding], [Expr::Number(initial_capture, _)]) = (bindings.as_slice(), init.as_slice())
    else {
        return Err(CompileError::at(
            "expected one literal numeric capture",
            *span,
        ));
    };
    let [parameter] = params.as_ref() else {
        return Err(CompileError::at(
            "expected one numeric parameter",
            *function_span,
        ));
    };
    if !attributes.is_empty() || !extra_values.is_empty() || binding.name == parameter.name {
        return Err(CompileError::at(
            "unsupported compiled closure signature",
            *function_span,
        ));
    }
    let mut emitter = Emitter {
        capture: &binding.name,
        parameter: &parameter.name,
        uses_capture: false,
    };
    let (update, result) = match body.as_ref() {
        [Stmt::Return {
            value: Some(result),
            extra_values,
            ..
        }] if extra_values.is_empty() => (String::new(), result),
        [Stmt::Assignment {
            targets,
            op,
            values,
            span,
        }, Stmt::Return {
            value: Some(result),
            extra_values,
            ..
        }] if extra_values.is_empty() => {
            let ([AssignmentTarget::Local { name, .. }], [value]) =
                (targets.as_slice(), values.as_slice())
            else {
                return Err(CompileError::at(
                    "expected one numeric capture assignment",
                    *span,
                ));
            };
            if name != emitter.capture {
                return Err(CompileError::at(
                    "only the numeric capture can be assigned",
                    *span,
                ));
            }
            emitter.uses_capture = true;
            let value = emitter.emit(value)?;
            let rhs = match op {
                AssignmentOp::Assign => value,
                AssignmentOp::Add => format!("canonical(capture + {value})"),
                AssignmentOp::Sub => format!("canonical(capture - {value})"),
                AssignmentOp::Mul => format!("canonical(capture * {value})"),
                AssignmentOp::Div => format!("canonical(capture / {value})"),
                _ => return Err(CompileError::at("unsupported numeric assignment", *span)),
            };
            (format!("    let capture = {rhs};\n"), result)
        }
        _ => {
            return Err(CompileError::at(
                "unsupported compiled closure body",
                *function_span,
            ))
        }
    };
    let result = emitter.emit(result)?;
    if !emitter.uses_capture {
        return Err(CompileError::at(
            "closure must capture the declared numeric local",
            *function_span,
        ));
    }
    let rust = format!(
        "#[inline]\nfn canonical(value: f64) -> f64 {{\n    if value.is_nan() {{ f64::from_bits({CANON_NAN_BITS}) }} else {{ value }}\n}}\n\n\
         pub fn compiled_call(capture: f64, parameter: f64) -> (f64, f64) {{\n\
             let capture = canonical(capture);\n    let parameter = canonical(parameter);\n{update}    (capture, {result})\n}}\n"
    );
    Ok(GeneratedCall {
        rust,
        capture_name: binding.name.clone(),
        initial_capture: *initial_capture,
    })
}

struct Emitter<'a> {
    capture: &'a str,
    parameter: &'a str,
    uses_capture: bool,
}

impl Emitter<'_> {
    fn emit(&mut self, expr: &Expr) -> Result<String, CompileError> {
        let unsupported = || CompileError::at("unsupported numeric expression", expr.span());
        match expr {
            Expr::Number(value, _) => Ok(format!("canonical(f64::from_bits({}))", value.to_bits())),
            Expr::Name(name, _) if name == self.capture => {
                self.uses_capture = true;
                Ok("capture".to_owned())
            }
            Expr::Name(name, _) if name == self.parameter => Ok("parameter".to_owned()),
            Expr::Paren { expr, .. } => self.emit(expr),
            Expr::Binary { op, lhs, rhs, .. } => {
                let op = match op {
                    BinaryOp::Add => '+',
                    BinaryOp::Sub => '-',
                    BinaryOp::Mul => '*',
                    BinaryOp::Div => '/',
                    _ => return Err(unsupported()),
                };
                Ok(format!(
                    "canonical({} {op} {})",
                    self.emit(lhs)?,
                    self.emit(rhs)?
                ))
            }
            _ => Err(unsupported()),
        }
    }
}
