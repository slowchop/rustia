//! Conservative admission for function modules that may be instantiated in a
//! compiled guest without transferring persistent module state.

use crate::{CompileError, Result};
use rustia_parser::{
    parse_source_bytes, AssignmentTarget, Expr, SourceError, Stmt, TableField, UnaryOp,
};
use std::collections::BTreeSet;

fn pure_builtin(name: &str) -> bool {
    matches!(name, "type" | "assert" | "tonumber" | "tostring" | "select")
}

fn can_bind(name: &str) -> bool {
    !matches!(name, "luvy" | "require" | "math" | "table") && !pure_builtin(name)
}

fn literal(expression: &Expr) -> bool {
    matches!(
        expression,
        Expr::Nil(_) | Expr::Bool(..) | Expr::Number(..) | Expr::String(..)
    ) || matches!(expression, Expr::Unary { op: UnaryOp::Neg, expr, .. } if matches!(**expr, Expr::Number(..)))
}

fn import(statement: &Stmt) -> Option<&str> {
    let Stmt::Local { bindings, init, .. } = statement else {
        return None;
    };
    let ([binding], [Expr::Call { callee, args, .. }]) = (bindings.as_slice(), init.as_slice())
    else {
        return None;
    };
    let [Expr::String(..)] = args.as_slice() else {
        return None;
    };
    matches!(callee.as_ref(), Expr::Name(name, _) if name == "require")
        .then_some(binding.name.as_str())
}

fn constant(statement: &Stmt) -> Option<(&str, rustia_lexer::Span)> {
    let Stmt::Local {
        bindings,
        init,
        span,
    } = statement
    else {
        return None;
    };
    let ([binding], [value]) = (bindings.as_slice(), init.as_slice()) else {
        return None;
    };
    literal(value).then_some((binding.name.as_str(), *span))
}

struct Helper<'a> {
    functions: BTreeSet<&'a str>,
    constants: BTreeSet<&'a str>,
}

impl Helper<'_> {
    fn expression(&self, expression: &Expr, locals: &BTreeSet<&str>) -> bool {
        let valid = |expression: &Expr| self.expression(expression, locals);
        match expression {
            Expr::Nil(_)
            | Expr::Bool(..)
            | Expr::String(..)
            | Expr::Number(..)
            | Expr::Varargs(_) => true,
            Expr::Name(name, _) => {
                locals.contains(name.as_str()) || self.constants.contains(name.as_str())
            }
            Expr::Field { object, field, .. } => {
                valid(object)
                    || matches!(object.as_ref(), Expr::Name(name, _) if name == "math" && field == "pi")
            }
            Expr::Index { object, index, .. } => valid(object) && valid(index),
            Expr::InterpolatedString { expressions, .. } => expressions.iter().all(valid),
            Expr::Binary { lhs, rhs, .. } => valid(lhs) && valid(rhs),
            Expr::Unary { expr, .. } | Expr::Paren { expr, .. } => valid(expr),
            Expr::IfElse {
                condition,
                then_expr,
                else_expr,
                ..
            } => valid(condition) && valid(then_expr) && valid(else_expr),
            Expr::Table { fields, .. } => fields.iter().all(|field| match field {
                TableField::Named { value, .. } | TableField::Array { value, .. } => valid(value),
                TableField::Keyed { key, value, .. } => valid(key) && valid(value),
            }),
            Expr::Call { callee, args, .. } => {
                let allowed = match callee.as_ref() {
                    Expr::Name(name, _) => {
                        !locals.contains(name.as_str())
                            && (pure_builtin(name) || self.functions.contains(name.as_str()))
                    }
                    Expr::Field { object, field, .. } => matches!(object.as_ref(),
                        Expr::Name(name, _) if name == "math" && !locals.contains("math")
                            && matches!(field.as_str(), "abs" | "sqrt" | "floor" | "ceil" | "sin" | "cos" | "min" | "max" | "clamp")),
                    _ => false,
                };
                allowed && args.iter().all(valid)
            }
            Expr::MethodCall { .. } | Expr::Function { .. } | Expr::TypeInstantiation { .. } => {
                false
            }
        }
    }

    fn block<'a>(&self, statements: &'a [Stmt], mut locals: BTreeSet<&'a str>) -> bool {
        statements.iter().all(|statement| match statement {
            Stmt::Local { bindings, init, .. } => {
                let valid = init
                    .iter()
                    .all(|expression| self.expression(expression, &locals));
                locals.extend(bindings.iter().map(|binding| binding.name.as_str()));
                valid
            }
            Stmt::Assignment {
                targets, values, ..
            } => {
                targets.iter().all(|target| {
                    matches!(target, AssignmentTarget::Local { name, .. } if locals.contains(name.as_str()))
                }) && values
                    .iter()
                    .all(|expression| self.expression(expression, &locals))
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                self.expression(condition, &locals)
                    && self.block(then_body, locals.clone())
                    && self.block(else_body, locals.clone())
            }
            Stmt::While {
                condition, body, ..
            } => self.expression(condition, &locals) && self.block(body, locals.clone()),
            Stmt::ForNumeric {
                variable,
                start,
                limit,
                step,
                body,
                ..
            } => {
                let mut inner = locals.clone();
                inner.insert(variable);
                self.expression(start, &locals)
                    && self.expression(limit, &locals)
                    && step
                        .as_ref()
                        .is_none_or(|expression| self.expression(expression, &locals))
                    && self.block(body, inner)
            }
            Stmt::Do { body, .. } => self.block(body, locals.clone()),
            Stmt::Return {
                value, extra_values, ..
            } => value
                .iter()
                .chain(extra_values)
                .all(|expression| self.expression(expression, &locals)),
            Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::TypeAlias { .. } => true,
            _ => false,
        })
    }
}

pub(crate) fn validate(source: &[u8]) -> Result<()> {
    let source_span = rustia_lexer::Span::new(0, source.len());
    let chunk = parse_source_bytes(source).map_err(|error| match error {
        SourceError::Lex(error) => CompileError::at(error.message, error.span),
        SourceError::Parse(error) => CompileError::at(error.message, error.span),
    })?;
    let Some((
        Stmt::Return {
            value:
                Some(Expr::Function {
                    params,
                    body,
                    span: function_span,
                    ..
                }),
            extra_values,
            ..
        },
        definitions,
    )) = chunk.statements.split_last()
    else {
        return Err(CompileError::at(
            "a stateless helper module must return a function",
            source_span,
        ));
    };

    let mut helper = Helper {
        functions: BTreeSet::new(),
        constants: BTreeSet::new(),
    };
    let mut names = BTreeSet::new();
    for statement in definitions {
        if matches!(statement, Stmt::TypeAlias { .. }) {
            continue;
        }
        let (binding, span) = if let Some(alias) = import(statement) {
            helper.functions.insert(alias);
            (alias, statement_span(statement))
        } else if let Some((binding, span)) = constant(statement) {
            helper.constants.insert(binding);
            (binding, span)
        } else {
            return Err(CompileError::at(
                "stateless helper initialization accepts imports and literal scalars only",
                statement_span(statement),
            ));
        };
        if !can_bind(binding) || !names.insert(binding) {
            return Err(CompileError::at(
                format!("overlapping stateless helper binding {binding:?}"),
                span,
            ));
        }
    }
    if !extra_values.is_empty()
        || !helper.block(
            body,
            params.iter().map(|param| param.name.as_str()).collect(),
        )
    {
        return Err(CompileError::at(
            "stateless helper must use pure operations; host effects, mutable captures and indirect calls are unsupported",
            *function_span,
        ));
    }
    Ok(())
}

fn statement_span(statement: &Stmt) -> rustia_lexer::Span {
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
        | Stmt::TypeFunction { span, .. }
        | Stmt::TypeAlias { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::Expr { span, .. } => *span,
    }
}
