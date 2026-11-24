//! Experimental generated Rust for one statically numeric Luau closure.
use crate::CompileError;
use rustia_lexer::Span;
use rustia_parser::{
    parse_source, AssignmentOp, AssignmentTarget, BinaryOp, Expr, SourceError, Stmt,
};
use rustia_vm::number::CANON_NAN_BITS;

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
    if source.len() > 4096 {
        return Err(CompileError::at(
            "compiled module exceeds 4096 source bytes",
            module_span,
        ));
    }
    let chunk = parse_source(source).map_err(|error| match error {
        SourceError::Lex(error) => CompileError::at(error.message, error.span),
        SourceError::Parse(error) => CompileError::at(error.message, error.span),
    })?;
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
    let names = Names {
        capture: &binding.name,
        parameter: &parameter.name,
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
            if name != names.capture {
                return Err(CompileError::at(
                    "only the numeric capture can be assigned",
                    *span,
                ));
            }
            let value = emit(value, &names)?;
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
    let result = emit(result, &names)?;
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

struct Names<'a> {
    capture: &'a str,
    parameter: &'a str,
}

fn emit(expr: &Expr, names: &Names<'_>) -> Result<String, CompileError> {
    let unsupported = || CompileError::at("unsupported numeric expression", expr.span());
    match expr {
        Expr::Number(value, _) => Ok(format!("canonical(f64::from_bits({}))", value.to_bits())),
        Expr::Name(name, _) if name == names.capture => Ok("capture".to_owned()),
        Expr::Name(name, _) if name == names.parameter => Ok("parameter".to_owned()),
        Expr::Paren { expr, .. } => emit(expr, names),
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
                emit(lhs, names)?,
                emit(rhs, names)?
            ))
        }
        _ => Err(unsupported()),
    }
}
