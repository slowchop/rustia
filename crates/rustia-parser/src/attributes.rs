use super::{
    Expr, ParseError, Result, SimpleToken, SourceParser, Span, TableField, Token, TokenKind,
};

/// A function annotation accepted by the pinned Luau parser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttributeKind {
    Checked,
    Native,
    Deprecated,
}

/// Attribute arguments are literal expressions. The interpreter never evaluates them.
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionAttribute {
    pub kind: AttributeKind,
    pub arguments: Vec<Expr>,
    pub span: Span,
}

fn error(span: Span, message: impl Into<String>) -> ParseError {
    ParseError {
        unclosed_block: None,
        span,
        message: message.into(),
    }
}

impl SourceParser<'_> {
    pub(super) fn at_attribute(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenKind::Attribute(_) | TokenKind::AttributeOpen
        )
    }

    pub(super) fn parse_attributed_statement(&mut self) -> Result<super::Stmt> {
        let attributes = self.parse_attributes()?;
        let start = attributes[0].span.start;
        if let Some(function) = self.eat_simple(SimpleToken::Function) {
            return self.parse_function_statement(function.span.start, attributes);
        }
        if self.eat_simple(SimpleToken::Local).is_some() {
            if self.eat_simple(SimpleToken::Function).is_some() {
                return self.parse_local_function(start, attributes);
            }
            return Err(error(
                self.peek().span,
                format!(
                "Expected 'function' after local declaration with attribute, but got {} instead",
                self.describe_token()
            ),
            ));
        }
        Err(error(self.peek().span, format!(
            "Expected 'function', 'local function', 'declare function' or a function type declaration after attribute, but got {} instead",
            self.describe_token()
        )))
    }

    pub(super) fn parse_attributed_expression(&mut self) -> Result<Expr> {
        let first_token = self.peek().span;
        let attributes = self.parse_attributes()?;
        let Some(function) = self.eat_simple(SimpleToken::Function) else {
            return Err(error(
                first_token,
                format!(
                    "Expected 'function' declaration after attribute, but got {} instead",
                    self.describe_token()
                ),
            ));
        };
        self.parse_function_expression(function.span.start, attributes)
    }

    fn parse_attributes(&mut self) -> Result<Vec<FunctionAttribute>> {
        let mut attributes = Vec::new();
        while self.at_attribute() {
            let token = self.advance().clone();
            if let TokenKind::Attribute(name) = token.kind {
                append_attribute(&mut attributes, &name, token.span, token.span, Vec::new())?;
                continue;
            }
            if let Some(close) = self.eat_simple(SimpleToken::RBracket) {
                return Err(error(
                    Span::new(token.span.start, close.span.end),
                    "Attribute list cannot be empty",
                ));
            }
            loop {
                if !matches!(self.peek().kind, TokenKind::Ident(_) | TokenKind::Type) {
                    return Err(error(
                        self.peek().span,
                        format!(
                            "Expected identifier when parsing attribute name, got {}",
                            self.describe_token()
                        ),
                    ));
                }
                let name = self.expect_name("expected attribute name")?;
                let (args, span) = self.attribute_arguments(name.span)?;
                append_attribute(&mut attributes, &name.text, name.span, span, args)?;
                if self.eat_simple(SimpleToken::Comma).is_none() {
                    break;
                }
            }
            self.close_attribute_delimiter(&token, SimpleToken::RBracket, "]")?;
        }
        Ok(attributes)
    }

    fn close_attribute_delimiter(
        &mut self,
        open: &Token,
        close: SimpleToken,
        spelling: &str,
    ) -> Result<Token> {
        self.eat_simple(close)
            .ok_or_else(|| self.expected_match(open, spelling))
    }

    fn attribute_arguments(&mut self, name: Span) -> Result<(Vec<Expr>, Span)> {
        let first = self.peek().span;
        let (arguments, argument_span) = match self.peek().kind {
            TokenKind::LParen => {
                let open = self.advance().clone();
                let args = if self.at_simple(SimpleToken::RParen) {
                    Vec::new()
                } else {
                    self.parse_expr_list()?
                };
                let close = self.close_attribute_delimiter(&open, SimpleToken::RParen, ")")?;
                (args, Span::new(open.span.end, close.span.end))
            }
            TokenKind::LBrace => {
                self.advance();
                let table = self.parse_table_literal(first, 0)?;
                let span = Span::new(first.end, table.span_end());
                (vec![table], span)
            }
            TokenKind::String(_) | TokenKind::MalformedStringEscape(_) => {
                let expr = self.parse_primary(0)?;
                (vec![expr], first)
            }
            _ => return Ok((Vec::new(), name)),
        };
        if !arguments.iter().all(is_attribute_literal) {
            return Err(error(
                argument_span,
                "Only literals can be passed as arguments for attributes",
            ));
        }
        Ok((arguments, Span::new(name.start, argument_span.end)))
    }
}

fn is_string_literal(expr: &Expr) -> bool {
    matches!(expr, Expr::String(_, _))
        || matches!(expr, Expr::InterpolatedString { expressions, .. } if expressions.is_empty())
}

fn is_attribute_literal(expr: &Expr) -> bool {
    match expr {
        Expr::Nil(_) | Expr::Bool(_, _) | Expr::Number(_, _) => true,
        Expr::Table { fields, .. } => fields.iter().all(|field| match field {
            TableField::Array { value, .. } | TableField::Named { value, .. } => {
                is_attribute_literal(value)
            }
            TableField::Keyed { .. } => false,
        }),
        other => is_string_literal(other),
    }
}

fn append_attribute(
    attributes: &mut Vec<FunctionAttribute>,
    name: &str,
    name_span: Span,
    span: Span,
    arguments: Vec<Expr>,
) -> Result<()> {
    let kind = match name {
        "checked" => AttributeKind::Checked,
        "native" => AttributeKind::Native,
        "deprecated" => AttributeKind::Deprecated,
        "" => return Err(error(name_span, "Attribute name is missing")),
        _ => return Err(error(name_span, format!("Invalid attribute '@{name}'"))),
    };
    if attributes.iter().any(|attr| attr.kind == kind) {
        return Err(error(
            name_span,
            format!("Cannot duplicate attribute '@{name}'"),
        ));
    }
    if kind == AttributeKind::Deprecated {
        validate_deprecated(name_span, &arguments)?;
    }
    attributes.push(FunctionAttribute {
        kind,
        arguments,
        span,
    });
    Ok(())
}

fn validate_deprecated(name_span: Span, args: &[Expr]) -> Result<()> {
    let arg = match args {
        [] => return Ok(()),
        [arg] => arg,
        _ => {
            return Err(error(
                name_span,
                "@deprecated can be parametrized only by 1 argument",
            ))
        }
    };
    let Expr::Table { fields, .. } = arg else {
        return Err(error(arg.span(), "Unknown argument type for @deprecated"));
    };
    for field in fields {
        match field {
            TableField::Named {
                name,
                name_span,
                value,
                ..
            } => {
                if name != "use" && name != "reason" {
                    return Err(error(*name_span, format!("Unknown argument '{name}' for @deprecated. Only string constants for 'use' and 'reason' are allowed")));
                }
                if !is_string_literal(value) {
                    return Err(error(
                        value.span(),
                        format!("Only constant string allowed as value for '{name}'"),
                    ));
                }
            }
            TableField::Array { value, .. } | TableField::Keyed { value, .. } => {
                return Err(error(
                    value.span(),
                    "Only constants keys 'use' and 'reason' are allowed for @deprecated attribute",
                ));
            }
        }
    }
    Ok(())
}
