//! Language analysis for an embedder-selected returned callback.

use crate::{source_bundle::literal_require_argument, statement_span, CompileError, Result};
use rustia_lexer::{lex, Span, Token, TokenKind};
use rustia_parser::{parse_source, Expr, SourceError, Stmt, TableField};
use std::collections::BTreeSet;

/// Select a module's return function or a callback returned by a factory.
#[derive(Clone, Copy)]
pub enum Selection<'a> {
    Returned,
    Factory {
        export: Option<&'a str>,
        parameters: usize,
    },
}

/// A callback expression and its surrounding lexical bindings. Captures are
/// candidates; the running interpreter still decides which bindings were captured.
pub struct CallbackSource {
    pub source: String,
    pub captures: BTreeSet<String>,
    pub direct_imports: Vec<(String, Vec<u8>)>,
    pub parameter_count: usize,
    pub variadic: bool,
}

/// Analyze frozen source without resolving modules or executing initialization.
pub fn select(source: &str, selection: Selection<'_>) -> Result<CallbackSource> {
    let chunk = parse_source(source).map_err(|error| match error {
        SourceError::Lex(error) => CompileError::at(error.message, error.span),
        SourceError::Parse(error) => CompileError::at(error.message, error.span),
    })?;
    let (module_value, module_span, module_prefix) = returned(&chunk.statements)?;
    let mut captures = BTreeSet::new();
    declarations(module_prefix, &mut captures)?;
    let (callback, direct_imports, isolated_source) = match selection {
        Selection::Returned => {
            let imports = direct_imports(source, module_span.start, module_prefix, &captures);
            (
                module_value,
                imports,
                source[module_span.start..].to_owned(),
            )
        }
        Selection::Factory { export, parameters } => {
            let factory = match (export, module_value) {
                (None, value) => value,
                (Some(export), Expr::Table { fields, span }) => {
                    named_factory(fields, export, *span)?
                }
                (Some(_), _) => {
                    return Err(CompileError::at(
                        "named factory requires a returned table",
                        module_value.span(),
                    ))
                }
            };
            let Expr::Function {
                params,
                is_vararg,
                body,
                span,
                ..
            } = factory
            else {
                return Err(CompileError::at(
                    "factory must be a function",
                    factory.span(),
                ));
            };
            if *is_vararg || params.len() != parameters {
                return Err(CompileError::at(
                    format!("factory must accept exactly {parameters} arguments"),
                    *span,
                ));
            }
            captures.extend(params.iter().map(|parameter| parameter.name.clone()));
            let (value, _, prefix) = returned(body)?;
            declarations(prefix, &mut captures)?;
            let span = value.span();
            (
                value,
                Vec::new(),
                format!("return {}", &source[span.start..span.end]),
            )
        }
    };
    let Expr::Function {
        params,
        is_vararg,
        span,
        ..
    } = callback
    else {
        return Err(CompileError::at(
            "expected a returned callback function",
            callback.span(),
        ));
    };
    let start = match selection {
        Selection::Returned => module_span.start,
        Selection::Factory { .. } => span.start,
    };
    let lines = source[..start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count();
    Ok(CallbackSource {
        source: format!("{}{isolated_source}", "\n".repeat(lines)),
        captures,
        direct_imports,
        parameter_count: params.len(),
        variadic: *is_vararg,
    })
}

fn named_factory<'a>(fields: &'a [TableField], export: &str, span: Span) -> Result<&'a Expr> {
    let mut found = None;
    for field in fields {
        match field {
            TableField::Named {
                name,
                value,
                name_span,
                ..
            } => {
                if name != export {
                    continue;
                }
                if found.replace(value).is_some() {
                    return Err(CompileError::at(
                        format!("duplicate factory export {export:?}"),
                        *name_span,
                    ));
                }
            }
            TableField::Keyed { key, .. } => {
                let cannot_replace = match key {
                    Expr::String(bytes, _) => bytes.as_slice() != export.as_bytes(),
                    Expr::Number(..) | Expr::Bool(..) => true,
                    _ => false,
                };
                if !cannot_replace {
                    return Err(CompileError::at(
                        format!("table key may replace factory export {export:?}"),
                        key.span(),
                    ));
                }
            }
            _ => {}
        }
    }
    found.ok_or_else(|| CompileError::at(format!("missing factory export {export:?}"), span))
}

fn returned(statements: &[Stmt]) -> Result<(&Expr, Span, &[Stmt])> {
    let Some((last, prefix)) = statements.split_last() else {
        return Err(CompileError::new("missing callback return"));
    };
    match last {
        Stmt::Return {
            value: Some(value),
            extra_values,
            span,
        } if extra_values.is_empty() => Ok((value, *span, prefix)),
        _ => Err(CompileError::at(
            "expected exactly one returned value",
            statement_span(last),
        )),
    }
}

fn declarations(statements: &[Stmt], captures: &mut BTreeSet<String>) -> Result<()> {
    for statement in statements {
        let mut add = |name: &str| {
            if captures.insert(name.to_owned()) {
                Ok(())
            } else {
                Err(CompileError::at(
                    format!("shadowed callback binding {name:?}"),
                    statement_span(statement),
                ))
            }
        };
        match statement {
            Stmt::Local { bindings, .. } => {
                for binding in bindings {
                    add(&binding.name)?;
                }
            }
            Stmt::LocalFunction { name, .. } => add(name)?,
            Stmt::TypeAlias { .. } => {}
            _ => {
                return Err(CompileError::at(
                    "callback initialization requires named lexical bindings",
                    statement_span(statement),
                ))
            }
        }
    }
    Ok(())
}

// Function declarations in a module prefix can close over and alter imported
// bindings. Factory callbacks do not specialize imports. A direct callback can
// specialize only an alias that is never used except in a direct call.
fn direct_imports(
    source: &str,
    start: usize,
    prefix: &[Stmt],
    captures: &BTreeSet<String>,
) -> Vec<(String, Vec<u8>)> {
    if captures.contains("require")
        || !lex(&source[..start])
            .is_ok_and(|tokens| tokens.iter().all(|token| token.kind != TokenKind::Function))
    {
        return Vec::new();
    }
    let Ok(tokens) = lex(&source[start..]) else {
        return Vec::new();
    };
    prefix
        .iter()
        .filter_map(|statement| {
            let Stmt::Local { bindings, init, .. } = statement else {
                return None;
            };
            let ([binding], [initializer]) = (bindings.as_slice(), init.as_slice()) else {
                return None;
            };
            let (specifier, _) = literal_require_argument(initializer)?;
            direct_calls_only(&tokens, &binding.name)
                .then(|| (binding.name.clone(), specifier.to_vec()))
        })
        .collect()
}

fn direct_calls_only(tokens: &[Token], alias: &str) -> bool {
    tokens.windows(2).all(|pair| {
        !matches!(&pair[0].kind, TokenKind::Ident(name) if name == alias)
            || pair[1].kind == TokenKind::LParen
    })
}

#[cfg(test)]
mod tests {
    use super::{select, Selection};

    #[test]
    fn refuses_factory_exports_that_may_be_replaced() {
        let factory = "function(api) return function() end end";
        for (second, expected_span) in [
            ("tick", "tick"),
            ("['tick']", "'tick'"),
            ("[alias]", "alias"),
        ] {
            let source = format!(
                "local alias = 'tick'\nreturn {{ tick = {factory}, {second} = {factory} }}"
            );
            let error = select(
                &source,
                Selection::Factory {
                    export: Some("tick"),
                    parameters: 1,
                },
            )
            .err()
            .expect("ambiguous factory must stay interpreted");
            let span = error.span.unwrap();
            assert_eq!(&source[span.start..span.end], expected_span);
        }
        let source = format!("return {{ ['other'] = {factory}, tick = {factory} }}");
        assert!(select(
            &source,
            Selection::Factory {
                export: Some("tick"),
                parameters: 1
            }
        )
        .is_ok());
    }

    #[test]
    fn specializes_only_stable_direct_import_calls() {
        let source =
            "local helper = require<<any>>('./helper')\nreturn function(x) return helper(x) end";
        let callback = select(source, Selection::Returned).unwrap();
        assert_eq!(callback.parameter_count, 1);
        assert_eq!(
            callback.direct_imports,
            [("helper".into(), b"./helper".to_vec())]
        );
        assert_eq!(callback.source, "\nreturn function(x) return helper(x) end");

        for prefix in [
            "local require = replacement\n",
            "local f = function() end\n",
        ] {
            let source = format!("{prefix}{source}");
            assert!(select(&source, Selection::Returned)
                .unwrap()
                .direct_imports
                .is_empty());
        }
        let escaped = source.replace("helper(x)", "helper");
        assert!(select(&escaped, Selection::Returned)
            .unwrap()
            .direct_imports
            .is_empty());
    }

    #[test]
    fn named_factory_keeps_module_and_factory_captures() {
        let source = "local shared = 1\nreturn { tick = function(api)\nlocal count = 0\nreturn function(event) return count + shared end\nend }";
        let callback = select(
            source,
            Selection::Factory {
                export: Some("tick"),
                parameters: 1,
            },
        )
        .unwrap();
        assert_eq!(
            callback.source,
            "\n\n\nreturn function(event) return count + shared end"
        );
        assert_eq!(
            callback.captures.into_iter().collect::<Vec<_>>(),
            ["api", "count", "shared"]
        );
        assert!(callback.direct_imports.is_empty());
    }
}
