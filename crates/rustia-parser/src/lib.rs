use rustia_lexer::{lex_bytes, LexError, Span, Token, TokenKind};
use std::collections::BTreeMap;
use std::fmt;

const MAX_PARSE_DEPTH: usize = 512;

#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub statements: Vec<Stmt>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Local {
        bindings: Vec<LocalBinding>,
        init: Vec<Expr>,
        span: Span,
    },
    LocalFunction {
        name: String,
        name_span: Span,
        params: Vec<FunctionParam>,
        is_vararg: bool,
        body: Vec<Stmt>,
        span: Span,
    },
    Function {
        target: AssignmentTarget,
        is_method: bool,
        params: Vec<FunctionParam>,
        is_vararg: bool,
        body: Vec<Stmt>,
        span: Span,
    },
    Assignment {
        targets: Vec<AssignmentTarget>,
        op: AssignmentOp,
        values: Vec<Expr>,
        span: Span,
    },
    If {
        condition: Expr,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
        span: Span,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
    ForNumeric {
        variable: String,
        variable_span: Span,
        start: Expr,
        limit: Expr,
        step: Option<Expr>,
        body: Vec<Stmt>,
        span: Span,
    },
    ForGeneric {
        variables: Vec<GenericForBinding>,
        iter: Vec<Expr>,
        body: Vec<Stmt>,
        span: Span,
    },
    Repeat {
        body: Vec<Stmt>,
        condition: Expr,
        span: Span,
    },
    Do {
        body: Vec<Stmt>,
        span: Span,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
    TypeFunction {
        name: String,
        exported: bool,
        span: Span,
    },
    TypeAlias {
        name: String,
        generics: Vec<String>,
        value: TypeExpr,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        extra_values: Vec<Expr>,
        span: Span,
    },
    Expr {
        expr: Expr,
        span: Span,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalBinding {
    pub name: String,
    pub annotation: Option<TypeExpr>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GenericForBinding {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FunctionParam {
    pub name: String,
    pub annotation: Option<TypeExpr>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AssignmentTarget {
    Local {
        name: String,
        span: Span,
    },
    Field {
        object: Box<Expr>,
        field: String,
        field_span: Span,
        span: Span,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssignmentOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    Concat,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Nil(Span),
    Bool(bool, Span),
    Number(f64, Span),
    String(Vec<u8>, Span),
    InterpolatedString {
        parts: Vec<InterpolatedStringPart>,
        expressions: Vec<Expr>,
        span: Span,
    },
    Name(String, Span),
    Varargs(Span),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    IfElse {
        condition: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
        span: Span,
    },
    Field {
        object: Box<Expr>,
        field: String,
        field_span: Span,
        span: Span,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    Table {
        fields: Vec<TableField>,
        span: Span,
    },
    Function {
        params: Vec<FunctionParam>,
        is_vararg: bool,
        body: Vec<Stmt>,
        span: Span,
    },
    Paren {
        expr: Box<Expr>,
        span: Span,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    MethodCall {
        object: Box<Expr>,
        method: String,
        method_span: Span,
        args: Vec<Expr>,
        span: Span,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum InterpolatedStringPart {
    Text(Vec<u8>),
    Expr(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TableField {
    Array {
        value: Expr,
        span: Span,
    },
    Named {
        name: String,
        name_span: Span,
        value: Expr,
        span: Span,
    },
    Keyed {
        key: Expr,
        value: Expr,
        span: Span,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
    Len,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Concat,
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TypeExpr {
    Named {
        name: String,
        args: Vec<TypeExpr>,
        span: Span,
    },
    Nil(Span),
    Boolean(bool, Span),
    StringSingleton(Vec<u8>, Span),
    Optional {
        inner: Box<TypeExpr>,
        span: Span,
    },
    Union {
        types: Vec<TypeExpr>,
        span: Span,
    },
    Intersection {
        types: Vec<TypeExpr>,
        span: Span,
    },
    Function {
        params: Vec<TypeExpr>,
        result: Box<TypeExpr>,
        span: Span,
    },
    TypePack {
        types: Vec<TypeExpr>,
        span: Span,
    },
    Table {
        span: Span,
    },
    Typeof {
        span: Span,
    },
    Variadic {
        inner: Box<TypeExpr>,
        span: Span,
    },
}

/// The parser knows which construct was still open at the point of failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnclosedBlock {
    pub start: usize,
    pub expected: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseError {
    pub unclosed_block: Option<Box<UnclosedBlock>>,
    pub span: Span,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "parse error at {}..{}: {}",
            self.span.start, self.span.end, self.message
        )
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug)]
pub enum SourceError {
    Lex(LexError),
    Parse(ParseError),
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(error) => error.fmt(formatter),
            Self::Parse(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SourceError {}

pub type Result<T> = std::result::Result<T, ParseError>;

pub fn parse_source(source: &str) -> std::result::Result<Chunk, SourceError> {
    parse_source_bytes(source.as_bytes())
}

pub fn parse_source_bytes(source: &[u8]) -> std::result::Result<Chunk, SourceError> {
    let lowered = lower_interpolated_strings(source).map_err(SourceError::Lex)?;
    if let Some(mut lowered) = lowered {
        let mut tokens = lex_bytes(&lowered.source)
            .map_err(|error| SourceError::Lex(lowered.remap_lex_error(error)))?;
        let newlines_before = token_newlines_before(&tokens, &lowered.source);
        for token in &mut tokens {
            token.span = lowered.remap_span(token.span);
        }
        let pending_interpolations = std::mem::take(&mut lowered.interpolations);
        let interpolations = pending_interpolations
            .into_iter()
            .map(|(offset, interpolation)| (lowered.remap_start(offset), interpolation))
            .collect();
        Parser::new_with_newlines_and_interpolations(tokens, newlines_before, interpolations)
            .parse_chunk()
            .map_err(SourceError::Parse)
    } else {
        let tokens = lex_bytes(source).map_err(SourceError::Lex)?;
        Parser::new_with_source_and_interpolations(tokens, source, BTreeMap::new())
            .parse_chunk()
            .map_err(SourceError::Parse)
    }
}

#[derive(Clone, Debug)]
struct LoweredSource {
    source: Vec<u8>,
    origins: Vec<Span>,
    original_len: usize,
    interpolations: BTreeMap<usize, PendingInterpolatedString>,
}

impl LoweredSource {
    fn remap_start(&self, offset: usize) -> usize {
        self.origins
            .get(offset)
            .map_or(self.original_len, |origin| origin.start)
    }

    fn remap_span(&self, span: Span) -> Span {
        let start = self.remap_start(span.start);
        let end = if span.end > span.start {
            self.origins
                .get(span.end.saturating_sub(1))
                .map_or(self.original_len, |origin| origin.end)
        } else {
            start
        };
        Span::new(start, end.max(start))
    }

    fn remap_lex_error(&self, mut error: LexError) -> LexError {
        error.span = self.remap_span(error.span);
        error
    }
}

#[derive(Clone, Debug)]
struct LoweredOutput {
    source: Vec<u8>,
    origins: Vec<Span>,
}

impl LoweredOutput {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            source: Vec::with_capacity(capacity),
            origins: Vec::with_capacity(capacity),
        }
    }

    fn len(&self) -> usize {
        self.source.len()
    }

    fn push_original(&mut self, byte: u8, offset: usize) {
        self.source.push(byte);
        self.origins.push(Span::new(offset, offset + 1));
    }

    fn extend_original(&mut self, source: &[u8], start: usize, end: usize) {
        for (offset, byte) in source[start..end].iter().copied().enumerate() {
            self.push_original(byte, start + offset);
        }
    }

    fn push_synthetic(&mut self, byte: u8, origin: Span) {
        self.source.push(byte);
        self.origins.push(origin);
    }

    fn extend_synthetic(&mut self, bytes: &[u8], origin: Span) {
        self.source.extend_from_slice(bytes);
        self.origins
            .extend(std::iter::repeat_n(origin, bytes.len()));
    }
}

#[derive(Clone, Debug)]
struct PendingInterpolatedString {
    parts: Vec<InterpolatedStringPart>,
    expression_count: usize,
    span: Span,
}

fn lower_interpolated_strings(
    source: &[u8],
) -> std::result::Result<Option<LoweredSource>, LexError> {
    if !source.contains(&b'`') {
        return Ok(None);
    }
    let mut cursor = 0;
    let mut output = LoweredOutput::with_capacity(source.len());
    let mut interpolations = BTreeMap::new();
    let mut next_interpolation_id = 0usize;
    while cursor < source.len() {
        if source[cursor] == b'`' {
            cursor = lower_interpolated_string(
                source,
                cursor,
                &mut output,
                &mut interpolations,
                &mut next_interpolation_id,
            )?;
        } else if matches!(source[cursor], b'\'' | b'"') {
            copy_short_string(source, &mut cursor, &mut output)?;
        } else if source[cursor] == b'[' && copy_long_bracket(source, &mut cursor, &mut output)? {
        } else if source[cursor] == b'-' && source.get(cursor + 1) == Some(&b'-') {
            copy_comment(source, &mut cursor, &mut output)?;
        } else {
            output.push_original(source[cursor], cursor);
            cursor += 1;
        }
    }
    Ok(Some(LoweredSource {
        source: output.source,
        origins: output.origins,
        original_len: source.len(),
        interpolations,
    }))
}

fn lower_interpolated_string(
    source: &[u8],
    start: usize,
    output: &mut LoweredOutput,
    interpolations: &mut BTreeMap<usize, PendingInterpolatedString>,
    next_interpolation_id: &mut usize,
) -> std::result::Result<usize, LexError> {
    output.push_synthetic(b' ', Span::new(start, start));
    let placeholder_start = output.len();
    output.extend_synthetic(
        format!("__rustia_interp_{next_interpolation_id}(").as_bytes(),
        Span::new(start, start),
    );
    *next_interpolation_id += 1;
    let mut expression_count = 0usize;
    let mut cursor = start + 1;
    let mut parts = Vec::new();
    let mut text = Vec::new();
    while cursor < source.len() {
        match source[cursor] {
            b'`' => {
                cursor += 1;
                if !text.is_empty() {
                    parts.push(InterpolatedStringPart::Text(std::mem::take(&mut text)));
                }
                if parts.is_empty() {
                    parts.push(InterpolatedStringPart::Text(Vec::new()));
                }
                output.push_synthetic(b')', Span::new(cursor - 1, cursor));
                interpolations.insert(
                    placeholder_start,
                    PendingInterpolatedString {
                        parts,
                        expression_count,
                        span: Span::new(start, cursor),
                    },
                );
                return Ok(cursor);
            }
            b'{' => {
                if source.get(cursor + 1) == Some(&b'{') {
                    return Err(LexError {
                        span: Span::new(cursor, cursor + 2),
                        message: "double '{{' is not valid in interpolated strings".to_owned(),
                    });
                }
                if !text.is_empty() {
                    parts.push(InterpolatedStringPart::Text(std::mem::take(&mut text)));
                }
                if expression_count > 0 {
                    output.push_synthetic(b',', Span::new(cursor, cursor));
                }
                output.push_synthetic(b'(', Span::new(cursor, cursor + 1));
                cursor = lower_interpolated_expression(
                    source,
                    cursor + 1,
                    output,
                    interpolations,
                    next_interpolation_id,
                )?;
                output.push_synthetic(b')', Span::new(cursor - 1, cursor));
                parts.push(InterpolatedStringPart::Expr(expression_count));
                expression_count += 1;
            }
            b'\\' => {
                cursor += 1;
                let escaped = source.get(cursor).copied().ok_or_else(|| LexError {
                    span: Span::new(start, cursor),
                    message: "unterminated interpolation escape".to_owned(),
                })?;
                match escaped {
                    b'\n' | b'\r' => {
                        take_interpolation_newline(source, &mut cursor);
                        text.push(b'\n');
                    }
                    b'a' => {
                        cursor += 1;
                        text.push(0x07);
                    }
                    b'b' => {
                        cursor += 1;
                        text.push(0x08);
                    }
                    b'f' => {
                        cursor += 1;
                        text.push(0x0c);
                    }
                    b'n' => {
                        cursor += 1;
                        text.push(b'\n');
                    }
                    b't' => {
                        cursor += 1;
                        text.push(b'\t');
                    }
                    b'r' => {
                        cursor += 1;
                        text.push(b'\r');
                    }
                    b'v' => {
                        cursor += 1;
                        text.push(0x0b);
                    }
                    b'x' => {
                        cursor += 1;
                        text.push(interpolation_hex_escape(source, &mut cursor, start)?);
                    }
                    b'u' => {
                        cursor += 1;
                        text.extend(interpolation_unicode_escape(source, &mut cursor, start)?);
                    }
                    b'z' => {
                        cursor += 1;
                        while source.get(cursor).is_some_and(|byte| {
                            matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c)
                        }) {
                            cursor += 1;
                        }
                    }
                    escaped if escaped.is_ascii_digit() => {
                        text.push(interpolation_decimal_escape(source, &mut cursor, start)?);
                    }
                    b'\\' | b'"' | b'\'' | b'`' | b'{' => {
                        cursor += 1;
                        text.push(escaped);
                    }
                    other if other.is_ascii_alphanumeric() || other == b'_' => {
                        return Err(LexError {
                            span: Span::new(start, cursor + 1),
                            message: format!(
                                "invalid interpolation escape \\{}",
                                char::from(other)
                            ),
                        });
                    }
                    other => {
                        cursor += 1;
                        text.push(other);
                    }
                }
            }
            b'\n' | b'\r' => {
                return Err(LexError {
                    span: Span::new(start, cursor),
                    message: "unfinished interpolated string near newline".to_owned(),
                });
            }
            byte => {
                text.push(byte);
                cursor += 1;
            }
        }
    }
    Err(LexError {
        span: Span::new(start, cursor),
        message: "unterminated interpolated string".to_owned(),
    })
}

fn lower_interpolated_expression(
    source: &[u8],
    mut cursor: usize,
    output: &mut LoweredOutput,
    interpolations: &mut BTreeMap<usize, PendingInterpolatedString>,
    next_interpolation_id: &mut usize,
) -> std::result::Result<usize, LexError> {
    let start = cursor - 1;
    let mut depth = 1usize;
    while cursor < source.len() {
        match source[cursor] {
            b'`' => {
                cursor = lower_interpolated_string(
                    source,
                    cursor,
                    output,
                    interpolations,
                    next_interpolation_id,
                )?;
            }
            b'\'' | b'"' => {
                copy_short_string(source, &mut cursor, output)?;
            }
            b'[' if copy_long_bracket(source, &mut cursor, output)? => {}
            b'-' if source.get(cursor + 1) == Some(&b'-') => {
                copy_comment(source, &mut cursor, output)?;
            }
            b'{' => {
                depth += 1;
                output.push_original(b'{', cursor);
                cursor += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(cursor + 1);
                }
                output.push_original(b'}', cursor);
                cursor += 1;
            }
            byte => {
                output.push_original(byte, cursor);
                cursor += 1;
            }
        }
    }
    Err(LexError {
        span: Span::new(start, cursor),
        message: "unterminated interpolation expression".to_owned(),
    })
}

fn take_interpolation_newline(source: &[u8], cursor: &mut usize) {
    if source.get(*cursor) == Some(&b'\r') && source.get(*cursor + 1) == Some(&b'\n') {
        *cursor += 2;
    } else {
        *cursor += 1;
    }
}

fn interpolation_hex_escape(
    source: &[u8],
    cursor: &mut usize,
    start: usize,
) -> std::result::Result<u8, LexError> {
    let hi = source.get(*cursor).copied().ok_or_else(|| LexError {
        span: Span::new(start, *cursor),
        message: "incomplete hexadecimal escape".to_owned(),
    })?;
    let lo = source.get(*cursor + 1).copied().ok_or_else(|| LexError {
        span: Span::new(start, *cursor + 1),
        message: "incomplete hexadecimal escape".to_owned(),
    })?;
    let Some(hi) = hex_value(hi) else {
        return Err(LexError {
            span: Span::new(start, *cursor + 1),
            message: "invalid hexadecimal escape".to_owned(),
        });
    };
    let Some(lo) = hex_value(lo) else {
        return Err(LexError {
            span: Span::new(start, *cursor + 2),
            message: "invalid hexadecimal escape".to_owned(),
        });
    };
    *cursor += 2;
    Ok((hi << 4) | lo)
}

fn interpolation_decimal_escape(
    source: &[u8],
    cursor: &mut usize,
    start: usize,
) -> std::result::Result<u8, LexError> {
    let mut value = 0u32;
    let mut digits = 0usize;
    while digits < 3 {
        let Some(byte) = source.get(*cursor).copied() else {
            break;
        };
        if !byte.is_ascii_digit() {
            break;
        }
        value = value * 10 + u32::from(byte - b'0');
        *cursor += 1;
        digits += 1;
    }
    if value > 0xff {
        return Err(LexError {
            span: Span::new(start, *cursor),
            message: "decimal interpolation escape exceeds 255".to_owned(),
        });
    }
    Ok(value as u8)
}

fn interpolation_unicode_escape(
    source: &[u8],
    cursor: &mut usize,
    start: usize,
) -> std::result::Result<Vec<u8>, LexError> {
    if source.get(*cursor) != Some(&b'{') {
        return Err(LexError {
            span: Span::new(start, *cursor),
            message: "expected '{' after unicode escape".to_owned(),
        });
    }
    *cursor += 1;
    let digits_start = *cursor;
    let mut value = 0u32;
    while let Some(byte) = source.get(*cursor).copied() {
        if byte == b'}' {
            if *cursor == digits_start {
                return Err(LexError {
                    span: Span::new(start, *cursor),
                    message: "empty unicode escape".to_owned(),
                });
            }
            *cursor += 1;
            if value > 0x10ffff {
                return Err(LexError {
                    span: Span::new(start, *cursor),
                    message: "unicode escape exceeds 0x10FFFF".to_owned(),
                });
            }
            return Ok(encode_interpolation_unicode_escape(value));
        }
        let Some(digit) = hex_value(byte) else {
            return Err(LexError {
                span: Span::new(start, *cursor + 1),
                message: "invalid unicode escape".to_owned(),
            });
        };
        value = value
            .checked_mul(16)
            .and_then(|value| value.checked_add(u32::from(digit)))
            .ok_or_else(|| LexError {
                span: Span::new(start, *cursor + 1),
                message: "invalid unicode escape".to_owned(),
            })?;
        *cursor += 1;
    }
    Err(LexError {
        span: Span::new(start, *cursor),
        message: "unterminated unicode escape".to_owned(),
    })
}

fn encode_interpolation_unicode_escape(value: u32) -> Vec<u8> {
    match value {
        0x0000..=0x007f => vec![value as u8],
        0x0080..=0x07ff => vec![0xc0 | ((value >> 6) as u8), 0x80 | ((value & 0x3f) as u8)],
        0x0800..=0xffff => vec![
            0xe0 | ((value >> 12) as u8),
            0x80 | (((value >> 6) & 0x3f) as u8),
            0x80 | ((value & 0x3f) as u8),
        ],
        _ => vec![
            0xf0 | ((value >> 18) as u8),
            0x80 | (((value >> 12) & 0x3f) as u8),
            0x80 | (((value >> 6) & 0x3f) as u8),
            0x80 | ((value & 0x3f) as u8),
        ],
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn copy_short_string(
    source: &[u8],
    cursor: &mut usize,
    output: &mut LoweredOutput,
) -> std::result::Result<(), LexError> {
    let start = *cursor;
    let quote = source[*cursor];
    output.push_original(quote, *cursor);
    *cursor += 1;
    while let Some(byte) = source.get(*cursor).copied() {
        output.push_original(byte, *cursor);
        *cursor += 1;
        if byte == b'\\' {
            let Some(escaped) = source.get(*cursor).copied() else {
                return Err(LexError {
                    span: Span::new(start, *cursor),
                    message: "unterminated escape sequence".to_owned(),
                });
            };
            output.push_original(escaped, *cursor);
            *cursor += 1;
        } else if byte == quote {
            return Ok(());
        }
    }
    Err(LexError {
        span: Span::new(start, *cursor),
        message: "unterminated string literal".to_owned(),
    })
}

fn copy_comment(
    source: &[u8],
    cursor: &mut usize,
    output: &mut LoweredOutput,
) -> std::result::Result<(), LexError> {
    let start = *cursor;
    debug_assert_eq!(source.get(*cursor), Some(&b'-'));
    debug_assert_eq!(source.get(*cursor + 1), Some(&b'-'));
    output.extend_original(source, *cursor, *cursor + 2);
    *cursor += 2;
    if copy_long_bracket(source, cursor, output)? {
        return Ok(());
    }
    while let Some(byte) = source.get(*cursor).copied() {
        if matches!(byte, b'\n' | b'\r') {
            break;
        }
        output.push_original(byte, *cursor);
        *cursor += 1;
    }
    debug_assert!(*cursor >= start + 2);
    Ok(())
}

fn copy_long_bracket(
    source: &[u8],
    cursor: &mut usize,
    output: &mut LoweredOutput,
) -> std::result::Result<bool, LexError> {
    let start = *cursor;
    let Some((level, after_opener)) = long_bracket_opener_at(source, start) else {
        return Ok(false);
    };
    output.extend_original(source, start, after_opener);
    *cursor = after_opener;
    while *cursor < source.len() {
        if long_bracket_closer_matches(source, *cursor, level) {
            let end = *cursor + level + 2;
            output.extend_original(source, *cursor, end);
            *cursor = end;
            return Ok(true);
        }
        output.push_original(source[*cursor], *cursor);
        *cursor += 1;
    }
    Err(LexError {
        span: Span::new(start, *cursor),
        message: "unterminated long string".to_owned(),
    })
}

fn long_bracket_opener_at(source: &[u8], start: usize) -> Option<(usize, usize)> {
    if source.get(start).copied() != Some(b'[') {
        return None;
    }
    let mut cursor = start + 1;
    while source.get(cursor).copied() == Some(b'=') {
        cursor += 1;
    }
    if source.get(cursor).copied() == Some(b'[') {
        Some((cursor - start - 1, cursor + 1))
    } else {
        None
    }
}

fn long_bracket_closer_matches(source: &[u8], cursor: usize, level: usize) -> bool {
    if source.get(cursor).copied() != Some(b']') {
        return false;
    }
    let mut cursor = cursor + 1;
    for _ in 0..level {
        if source.get(cursor).copied() != Some(b'=') {
            return false;
        }
        cursor += 1;
    }
    source.get(cursor).copied() == Some(b']')
}

/// Tracks lexical bindings only in chunks that contain the declaration syntax.
/// A type function can use globals and its own locals, including from nested
/// closures, but cannot capture a local from before its declaration.
#[derive(Default)]
struct TypeFunctionLocals {
    enabled: bool,
    visible: BTreeMap<String, usize>,
    bindings: Vec<(String, Option<usize>)>,
    boundary: Option<usize>,
}

impl TypeFunctionLocals {
    fn new(tokens: &[Token]) -> Self {
        Self {
            enabled: tokens.windows(2).any(|pair| {
                matches!(pair[0].kind, TokenKind::Type)
                    && matches!(pair[1].kind, TokenKind::Function)
            }),
            ..Self::default()
        }
    }

    fn bind(&mut self, name: &str) {
        if self.enabled {
            let previous = self.visible.insert(name.to_owned(), self.bindings.len());
            self.bindings.push((name.to_owned(), previous));
        }
    }

    fn restore(&mut self, mark: usize) {
        for (name, shadowed) in self.bindings.drain(mark..).rev() {
            if let Some(previous) = shadowed {
                self.visible.insert(name, previous);
            } else {
                self.visible.remove(&name);
            }
        }
    }

    fn is_outer_local(&self, name: &str) -> bool {
        self.boundary
            .is_some_and(|start| self.visible.get(name).is_some_and(|index| *index < start))
    }
}

#[derive(Clone, Copy)]
enum FunctionBinding<'a> {
    Plain,
    Local(&'a str),
    Method,
}

pub struct Parser {
    tokens: Vec<Token>,
    newlines_before: Vec<bool>,
    interpolations: BTreeMap<usize, PendingInterpolatedString>,
    cursor: usize,
    active_parse_frames: usize,
    type_function_locals: TypeFunctionLocals,
    function_is_vararg: bool,
    loop_depth: usize,
    in_erased_code: bool,
    #[cfg(test)]
    type_primary_visits: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        let newlines_before = vec![false; tokens.len()];
        Self::new_with_newlines_and_interpolations(tokens, newlines_before, BTreeMap::new())
    }

    fn new_with_source_and_interpolations(
        tokens: Vec<Token>,
        source: &[u8],
        interpolations: BTreeMap<usize, PendingInterpolatedString>,
    ) -> Self {
        let newlines_before = token_newlines_before(&tokens, source);
        Self::new_with_newlines_and_interpolations(tokens, newlines_before, interpolations)
    }

    fn new_with_newlines_and_interpolations(
        tokens: Vec<Token>,
        newlines_before: Vec<bool>,
        interpolations: BTreeMap<usize, PendingInterpolatedString>,
    ) -> Self {
        Self {
            type_function_locals: TypeFunctionLocals::new(&tokens),
            tokens,
            newlines_before,
            interpolations,
            cursor: 0,
            active_parse_frames: 0,
            function_is_vararg: true,
            loop_depth: 0,
            in_erased_code: false,
            #[cfg(test)]
            type_primary_visits: 0,
        }
    }

    pub fn parse_chunk(mut self) -> Result<Chunk> {
        let statements = self.parse_block_until(&[SimpleToken::Eof])?;
        Ok(Chunk { statements })
    }

    fn parse_block_until(&mut self, terminators: &[SimpleToken]) -> Result<Vec<Stmt>> {
        let mut statements = Vec::new();
        while !self.at_any(terminators) {
            let statement = self.parse_statement()?;
            let final_error = match statement {
                Stmt::Return { .. } => Some("return must be the final statement in a block"),
                Stmt::Break { .. } => Some("break must be the final statement in a block"),
                Stmt::Continue { .. } => Some("continue must be the final statement in a block"),
                _ => None,
            };
            statements.push(statement);
            if let Some(message) = final_error {
                if !self.at_any(terminators) {
                    return Err(self.error_here(message));
                }
            }
        }
        Ok(statements)
    }

    fn parse_delimited_block(
        &mut self,
        terminators: &[SimpleToken],
        start: usize,
        expected: &'static str,
    ) -> Result<Vec<Stmt>> {
        self.with_parse_frame(|parser| {
            parser.parse_delimited_block_inner(terminators, start, expected)
        })
    }

    fn parse_delimited_block_inner(
        &mut self,
        terminators: &[SimpleToken],
        start: usize,
        expected: &'static str,
    ) -> Result<Vec<Stmt>> {
        self.parse_block_until(terminators).map_err(|mut error| {
            if self.at_simple(SimpleToken::Eof) && error.unclosed_block.is_none() {
                error.unclosed_block = Some(Box::new(UnclosedBlock { start, expected }));
            }
            error
        })
    }

    fn parse_statement(&mut self) -> Result<Stmt> {
        self.with_parse_frame(|parser| parser.parse_statement_inner())
    }

    fn parse_statement_inner(&mut self) -> Result<Stmt> {
        if let Some(if_token) = self.eat_simple(SimpleToken::If) {
            return self.parse_if(if_token.span.start);
        }
        if let Some(while_token) = self.eat_simple(SimpleToken::While) {
            return self.with_local_scope(|parser| parser.parse_while(while_token.span.start));
        }
        if let Some(for_token) = self.eat_simple(SimpleToken::For) {
            return self.with_local_scope(|parser| parser.parse_for(for_token.span.start));
        }
        if let Some(repeat_token) = self.eat_simple(SimpleToken::Repeat) {
            return self.with_local_scope(|parser| parser.parse_repeat(repeat_token.span.start));
        }
        if let Some(do_token) = self.eat_simple(SimpleToken::Do) {
            return self.with_local_scope(|parser| parser.parse_do(do_token.span.start));
        }
        self.parse_non_control_statement()
    }

    fn parse_non_control_statement(&mut self) -> Result<Stmt> {
        if let Some(local_token) = self.eat_simple(SimpleToken::Local) {
            if self.eat_simple(SimpleToken::Function).is_some() {
                return self.parse_local_function(local_token.span.start);
            }
            return self.parse_local(local_token.span.start);
        }
        if let Some(function_token) = self.eat_simple(SimpleToken::Function) {
            return self.parse_function_statement(function_token.span.start);
        }
        // `export` is contextual: ordinary variables with this name remain valid.
        if matches!(&self.peek().kind, TokenKind::Ident(name) if name == "export")
            && matches!(
                self.tokens.get(self.peek_index() + 1).map(|t| &t.kind),
                Some(TokenKind::Type)
            )
        {
            let start = self.advance().span.start;
            self.check_value_name("export")?;
            self.advance();
            return self.parse_type_declaration(start, true);
        }
        if self.at_simple(SimpleToken::Type)
            && matches!(
                self.tokens
                    .get(self.peek_index() + 1)
                    .map(|token| &token.kind),
                Some(TokenKind::Ident(_) | TokenKind::Function)
            )
        {
            let start = self.advance().span.start;
            self.check_value_name("type")?;
            return self.parse_type_declaration(start, false);
        }
        if let Some(break_token) = self.eat_simple(SimpleToken::Break) {
            if self.in_erased_code && self.loop_depth == 0 {
                return Err(ParseError {
                    unclosed_block: None,
                    span: break_token.span,
                    message: "break statement must be inside a loop".to_owned(),
                });
            }
            self.eat_simple(SimpleToken::Semicolon);
            return Ok(Stmt::Break {
                span: break_token.span,
            });
        }
        if self.at_ident("continue") && self.next_starts_statement_boundary() {
            let continue_token = self.advance().clone();
            self.check_value_name("continue")?;
            if self.in_erased_code && self.loop_depth == 0 {
                return Err(ParseError {
                    unclosed_block: None,
                    span: continue_token.span,
                    message: "continue statement must be inside a loop".to_owned(),
                });
            }
            self.eat_simple(SimpleToken::Semicolon);
            return Ok(Stmt::Continue {
                span: continue_token.span,
            });
        }
        if let Some(return_token) = self.eat_simple(SimpleToken::Return) {
            let value = if self.at_statement_boundary() {
                None
            } else {
                Some(self.parse_expr(0, 0)?)
            };
            let mut extra_values = Vec::new();
            while value.is_some() && self.eat_simple(SimpleToken::Comma).is_some() {
                extra_values.push(self.parse_expr(0, 0)?);
            }
            let end = extra_values.last().map_or_else(
                || value.as_ref().map_or(return_token.span.end, Expr::span_end),
                Expr::span_end,
            );
            self.eat_simple(SimpleToken::Semicolon);
            return Ok(Stmt::Return {
                value,
                extra_values,
                span: Span::new(return_token.span.start, end),
            });
        }
        let expr = self.parse_expr(0, 0)?;
        if self.at_simple(SimpleToken::Equal)
            || self.at_simple(SimpleToken::Comma)
            || self.peek_compound_assignment_op().is_some()
        {
            return self.parse_assignment(expr);
        }
        if !matches!(expr, Expr::Call { .. } | Expr::MethodCall { .. }) {
            return Err(ParseError {
                unclosed_block: None,
                span: expr.span(),
                message: "expected statement".to_owned(),
            });
        }
        let span = expr.span();
        self.eat_simple(SimpleToken::Semicolon);
        Ok(Stmt::Expr { expr, span })
    }

    fn parse_local(&mut self, start: usize) -> Result<Stmt> {
        let first = self.parse_local_binding()?;
        let mut bindings = vec![first];
        while self.eat_simple(SimpleToken::Comma).is_some() {
            bindings.push(self.parse_local_binding()?);
        }
        let init = if self.eat_simple(SimpleToken::Equal).is_some() {
            self.parse_expr_list()?
        } else {
            Vec::new()
        };
        for binding in &bindings {
            self.type_function_locals.bind(&binding.name);
        }
        let end = init.last().map_or_else(
            || bindings.last().map_or(start, |binding| binding.span.end),
            Expr::span_end,
        );
        self.eat_simple(SimpleToken::Semicolon);
        Ok(Stmt::Local {
            bindings,
            init,
            span: Span::new(start, end),
        })
    }

    fn parse_local_function(&mut self, start: usize) -> Result<Stmt> {
        let name = self.expect_name("expected local function name")?;
        let (params, is_vararg, body, end) =
            self.parse_function_body(start, FunctionBinding::Local(&name.text))?;
        self.eat_simple(SimpleToken::Semicolon);
        Ok(Stmt::LocalFunction {
            name: name.text,
            name_span: name.span,
            params,
            is_vararg,
            body,
            span: Span::new(start, end),
        })
    }

    fn parse_function_statement(&mut self, start: usize) -> Result<Stmt> {
        let (target, is_method) = self.parse_function_statement_target()?;
        let binding = if is_method {
            FunctionBinding::Method
        } else {
            FunctionBinding::Plain
        };
        let (params, is_vararg, body, end) = self.parse_function_body(start, binding)?;
        self.eat_simple(SimpleToken::Semicolon);
        Ok(Stmt::Function {
            target,
            is_method,
            params,
            is_vararg,
            body,
            span: Span::new(start, end),
        })
    }

    fn parse_function_statement_target(&mut self) -> Result<(AssignmentTarget, bool)> {
        let name = self.expect_name("expected function name")?;
        self.check_value_name(&name.text)?;
        let mut expr = Expr::Name(name.text, name.span);
        while self.eat_simple(SimpleToken::Dot).is_some() {
            let field = self.expect_name("expected field name after '.'")?;
            let span = Span::new(expr.span_start(), field.span.end);
            expr = Expr::Field {
                object: Box::new(expr),
                field: field.text,
                field_span: field.span,
                span,
            };
        }
        let is_method = if self.eat_simple(SimpleToken::Colon).is_some() {
            let method = self.expect_name("expected method name after ':'")?;
            let span = Span::new(expr.span_start(), method.span.end);
            expr = Expr::Field {
                object: Box::new(expr),
                field: method.text,
                field_span: method.span,
                span,
            };
            true
        } else {
            false
        };
        Ok((assignment_target_from_expr(expr)?, is_method))
    }

    fn parse_binding_annotation(&mut self) -> Result<Option<TypeExpr>> {
        if self.eat_simple(SimpleToken::Colon).is_some() {
            Ok(Some(self.parse_single_type(0)?))
        } else {
            Ok(None)
        }
    }

    fn parse_local_binding(&mut self) -> Result<LocalBinding> {
        let name = self.expect_name("expected local name")?;
        let annotation = self.parse_binding_annotation()?;
        let end = annotation
            .as_ref()
            .map_or(name.span.end, TypeExpr::span_end);
        Ok(LocalBinding {
            name: name.text,
            annotation,
            span: Span::new(name.span.start, end),
        })
    }

    fn parse_assignment(&mut self, first: Expr) -> Result<Stmt> {
        let mut targets = vec![assignment_target_from_expr(first)?];
        if let Some(op) = self.eat_compound_assignment_op() {
            let value = self.parse_expr(0, 0)?;
            let start = targets[0].span_start();
            let end = value.span_end();
            self.eat_simple(SimpleToken::Semicolon);
            return Ok(Stmt::Assignment {
                targets,
                op,
                values: vec![value],
                span: Span::new(start, end),
            });
        }

        while self.eat_simple(SimpleToken::Comma).is_some() {
            let target = self.parse_expr(0, 0)?;
            targets.push(assignment_target_from_expr(target)?);
        }
        self.expect_simple(SimpleToken::Equal, "expected '=' in assignment")?;
        let values = self.parse_expr_list()?;
        let start = targets.first().map_or(0, AssignmentTarget::span_start);
        let end = values.last().map_or_else(
            || targets.last().map_or(start, AssignmentTarget::span_end),
            Expr::span_end,
        );
        self.eat_simple(SimpleToken::Semicolon);
        Ok(Stmt::Assignment {
            targets,
            op: AssignmentOp::Assign,
            values,
            span: Span::new(start, end),
        })
    }

    fn parse_if(&mut self, start: usize) -> Result<Stmt> {
        let (statement, _) = self.parse_if_chain(start)?;
        self.eat_simple(SimpleToken::Semicolon);
        Ok(statement)
    }

    fn parse_if_chain(&mut self, start: usize) -> Result<(Stmt, usize)> {
        self.with_parse_frame(|parser| parser.parse_if_chain_inner(start))
    }

    fn parse_if_chain_inner(&mut self, start: usize) -> Result<(Stmt, usize)> {
        let condition = self.parse_expr(0, 0)?;
        self.expect_simple(SimpleToken::Then, "expected 'then' after if condition")?;
        let then_body = self.with_local_scope(|parser| {
            parser.parse_delimited_block(
                &[SimpleToken::ElseIf, SimpleToken::Else, SimpleToken::End],
                start,
                "end",
            )
        })?;
        let (else_body, end) = if let Some(elseif_token) = self.eat_simple(SimpleToken::ElseIf) {
            let (elseif_stmt, end) = self.parse_if_chain(elseif_token.span.start)?;
            (vec![elseif_stmt], end)
        } else if self.eat_simple(SimpleToken::Else).is_some() {
            let else_body = self.with_local_scope(|parser| {
                parser.parse_delimited_block(&[SimpleToken::End], start, "end")
            })?;
            let end_token =
                self.expect_simple(SimpleToken::End, "expected 'end' after if block")?;
            (else_body, end_token.span.end)
        } else {
            let end_token =
                self.expect_simple(SimpleToken::End, "expected 'end' after if block")?;
            (Vec::new(), end_token.span.end)
        };
        Ok((
            Stmt::If {
                condition,
                then_body,
                else_body,
                span: Span::new(start, end),
            },
            end,
        ))
    }

    fn parse_while(&mut self, start: usize) -> Result<Stmt> {
        let condition = self.parse_expr(0, 0)?;
        self.expect_simple(SimpleToken::Do, "expected 'do' after while condition")?;
        let body = self.parse_loop_body(&[SimpleToken::End], start, "end")?;
        let end_token = self.expect_simple(SimpleToken::End, "expected 'end' after while block")?;
        self.eat_simple(SimpleToken::Semicolon);
        Ok(Stmt::While {
            condition,
            body,
            span: Span::new(start, end_token.span.end),
        })
    }

    fn parse_for_binding(&mut self, expected: &'static str) -> Result<IdentToken> {
        let name = self.expect_name(expected)?;
        self.parse_binding_annotation()?;
        Ok(name)
    }

    fn parse_for(&mut self, start: usize) -> Result<Stmt> {
        let first = self.parse_for_binding("expected for loop variable")?;
        if self.eat_simple(SimpleToken::Equal).is_some() {
            return self.parse_numeric_for(start, first);
        }

        let mut variables = vec![GenericForBinding {
            name: first.text,
            span: first.span,
        }];
        while self.eat_simple(SimpleToken::Comma).is_some() {
            let variable = self.parse_for_binding("expected generic for loop variable")?;
            variables.push(GenericForBinding {
                name: variable.text,
                span: variable.span,
            });
        }
        self.expect_simple(SimpleToken::In, "expected 'in' after generic for variables")?;
        let iter = self.parse_expr_list()?;
        self.expect_simple(SimpleToken::Do, "expected 'do' after generic for iterator")?;
        for variable in &variables {
            self.type_function_locals.bind(&variable.name);
        }
        let body = self.parse_loop_body(&[SimpleToken::End], start, "end")?;
        let end_token = self.expect_simple(SimpleToken::End, "expected 'end' after for loop")?;
        self.eat_simple(SimpleToken::Semicolon);
        Ok(Stmt::ForGeneric {
            variables,
            iter,
            body,
            span: Span::new(start, end_token.span.end),
        })
    }

    fn parse_numeric_for(&mut self, start: usize, variable: IdentToken) -> Result<Stmt> {
        let start_expr = self.parse_expr(0, 0)?;
        self.expect_simple(SimpleToken::Comma, "expected ',' after for loop start")?;
        let limit = self.parse_expr(0, 0)?;
        let step = if self.eat_simple(SimpleToken::Comma).is_some() {
            Some(self.parse_expr(0, 0)?)
        } else {
            None
        };
        self.expect_simple(SimpleToken::Do, "expected 'do' after for loop range")?;
        self.type_function_locals.bind(&variable.text);
        let body = self.parse_loop_body(&[SimpleToken::End], start, "end")?;
        let end_token = self.expect_simple(SimpleToken::End, "expected 'end' after for loop")?;
        self.eat_simple(SimpleToken::Semicolon);
        Ok(Stmt::ForNumeric {
            variable: variable.text,
            variable_span: variable.span,
            start: start_expr,
            limit,
            step,
            body,
            span: Span::new(start, end_token.span.end),
        })
    }

    fn parse_repeat(&mut self, start: usize) -> Result<Stmt> {
        let body = self.parse_loop_body(&[SimpleToken::Until], start, "until")?;
        self.expect_simple(SimpleToken::Until, "expected 'until' after repeat block")?;
        let condition = self.parse_expr(0, 0)?;
        let end = condition.span_end();
        self.eat_simple(SimpleToken::Semicolon);
        Ok(Stmt::Repeat {
            body,
            condition,
            span: Span::new(start, end),
        })
    }

    fn parse_do(&mut self, start: usize) -> Result<Stmt> {
        let body = self.parse_delimited_block(&[SimpleToken::End], start, "end")?;
        let end_token = self.expect_simple(SimpleToken::End, "expected 'end' after do block")?;
        self.eat_simple(SimpleToken::Semicolon);
        Ok(Stmt::Do {
            body,
            span: Span::new(start, end_token.span.end),
        })
    }

    fn parse_expr_list(&mut self) -> Result<Vec<Expr>> {
        let mut values = vec![self.parse_expr(0, 0)?];
        while self.eat_simple(SimpleToken::Comma).is_some() {
            values.push(self.parse_expr(0, 0)?);
        }
        Ok(values)
    }

    fn parse_type_declaration(&mut self, start: usize, exported: bool) -> Result<Stmt> {
        let Some(function) = self.eat_simple(SimpleToken::Function) else {
            return self.parse_type_alias();
        };
        let name = self.expect_name("expected type function name")?;
        let mark = self.type_function_locals.bindings.len();
        let old_boundary = self.type_function_locals.boundary.replace(mark);
        let old_erasure = std::mem::replace(&mut self.in_erased_code, true);
        let result = self.parse_function_body(function.span.start, FunctionBinding::Plain);
        self.in_erased_code = old_erasure;
        self.type_function_locals.boundary = old_boundary;
        let (_, _, _, end) = result?;
        self.eat_simple(SimpleToken::Semicolon);
        Ok(Stmt::TypeFunction {
            name: name.text,
            exported,
            span: Span::new(start, end),
        })
    }

    fn parse_type_alias(&mut self) -> Result<Stmt> {
        let name = self.expect_name("expected type alias name")?;
        let start = name.span.start;
        let mut generics = Vec::new();
        if self.eat_simple(SimpleToken::Less).is_some() {
            let mut seen_pack = false;
            let mut seen_default = false;
            loop {
                generics.push(self.expect_name("expected generic type parameter")?.text);
                let pack = self.eat_simple(SimpleToken::Ellipsis).is_some();
                if seen_pack && !pack {
                    return Err(self.error_here("generic types must precede generic type packs"));
                }
                seen_pack |= pack;
                if self.eat_simple(SimpleToken::Equal).is_some() {
                    seen_default = true;
                    if pack {
                        self.parse_generic_pack_default(0)?;
                    } else {
                        self.parse_single_type(0)?;
                    }
                } else if seen_default {
                    return Err(self.error_here("expected a default after generic parameter"));
                }
                if self.eat_simple(SimpleToken::Comma).is_none() {
                    break;
                }
            }
            self.expect_simple(
                SimpleToken::Greater,
                "expected '>' after generic parameters",
            )?;
        }
        self.expect_simple(SimpleToken::Equal, "expected '=' in type alias")?;
        let value = self.parse_single_type(0)?;
        let span = Span::new(start, value.span().end);
        self.eat_simple(SimpleToken::Semicolon);
        Ok(Stmt::TypeAlias {
            name: name.text,
            generics,
            value,
            span,
        })
    }

    fn parse_expr(&mut self, min_bp: u8, depth: usize) -> Result<Expr> {
        self.with_parse_frame(|parser| parser.parse_expr_inner(min_bp, depth))
    }

    fn parse_expr_inner(&mut self, min_bp: u8, depth: usize) -> Result<Expr> {
        self.check_depth(depth)?;
        let lhs = self.parse_expr_prefix(depth)?;
        self.parse_expr_suffix(lhs, min_bp, depth)
    }

    fn parse_expr_prefix(&mut self, depth: usize) -> Result<Expr> {
        self.with_parse_frame(|parser| parser.parse_expr_prefix_inner(depth))
    }

    fn parse_expr_prefix_inner(&mut self, depth: usize) -> Result<Expr> {
        self.check_depth(depth + 1)?;
        if let Some(function) = self.eat_simple(SimpleToken::Function) {
            return self.parse_function_expression(function.span.start);
        }
        self.parse_non_function_prefix(depth)
    }

    fn parse_non_function_prefix(&mut self, depth: usize) -> Result<Expr> {
        let expr = if let Some(token) = self.eat_simple(SimpleToken::Minus) {
            let expr = self.parse_expr(7, depth + 1)?;
            let span = Span::new(token.span.start, expr.span_end());
            Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(expr),
                span,
            }
        } else if let Some(token) = self.eat_simple(SimpleToken::Not) {
            let expr = self.parse_expr(7, depth + 1)?;
            let span = Span::new(token.span.start, expr.span_end());
            Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(expr),
                span,
            }
        } else if let Some(token) = self.eat_simple(SimpleToken::Hash) {
            let expr = self.parse_expr(7, depth + 1)?;
            let span = Span::new(token.span.start, expr.span_end());
            Expr::Unary {
                op: UnaryOp::Len,
                expr: Box::new(expr),
                span,
            }
        } else {
            self.parse_primary(depth + 1)?
        };
        Ok(expr)
    }

    fn parse_expr_suffix(&mut self, mut lhs: Expr, min_bp: u8, depth: usize) -> Result<Expr> {
        lhs = self.parse_postfix(lhs, depth + 1)?;
        if self.at_type_assertion() {
            self.advance();
            self.advance();
            let annotation = self.parse_single_type(depth + 1)?;
            // Like parentheses, assertions produce one value and are not lvalues.
            lhs = Expr::Paren {
                span: Span::new(lhs.span_start(), annotation.span_end()),
                expr: Box::new(lhs),
            };
        }

        while let Some((op, left_bp, right_bp)) = self.peek_binary_op() {
            if left_bp < min_bp {
                break;
            }
            self.cursor += 1;
            let rhs = self.parse_expr(right_bp, depth + 1)?;
            let span = Span::new(lhs.span_start(), rhs.span_end());
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }

        Ok(lhs)
    }

    fn parse_function_expression(&mut self, start: usize) -> Result<Expr> {
        self.with_parse_frame(|parser| parser.parse_function_expression_inner(start))
    }

    fn parse_function_expression_inner(&mut self, start: usize) -> Result<Expr> {
        let (params, is_vararg, body, end) =
            self.parse_function_body(start, FunctionBinding::Plain)?;
        Ok(Expr::Function {
            params,
            is_vararg,
            body,
            span: Span::new(start, end),
        })
    }

    fn at_type_assertion(&self) -> bool {
        let index = self.peek_index();
        self.at_simple(SimpleToken::Colon)
            && self.tokens.get(index + 1).is_some_and(|next| {
                matches!(next.kind, TokenKind::Colon)
                    && self.tokens[index].span.end == next.span.start
            })
    }

    fn parse_postfix(&mut self, mut expr: Expr, depth: usize) -> Result<Expr> {
        self.check_depth(depth)?;
        loop {
            if self.at_type_assertion() || !expr_accepts_postfix(&expr) {
                break;
            }
            if let Some((args, end)) = self.parse_call_suffix(depth + 1)? {
                let span = Span::new(expr.span_start(), end);
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
                    span,
                };
            } else if self.eat_simple(SimpleToken::Colon).is_some() {
                let method = self.expect_name("expected method name after ':'")?;
                let (args, end) = self
                    .parse_call_suffix(depth + 1)?
                    .ok_or_else(|| ParseError {
                        unclosed_block: None,
                        span: self.peek().span,
                        message: "expected arguments after method name".to_owned(),
                    })?;
                let span = Span::new(expr.span_start(), end);
                expr = Expr::MethodCall {
                    object: Box::new(expr),
                    method: method.text,
                    method_span: method.span,
                    args,
                    span,
                };
            } else if let Some(dot) = self.eat_simple(SimpleToken::Dot) {
                let field = self.expect_name("expected field name after '.'")?;
                let span = Span::new(expr.span_start().min(dot.span.start), field.span.end);
                expr = Expr::Field {
                    object: Box::new(expr),
                    field: field.text,
                    field_span: field.span,
                    span,
                };
            } else if self.eat_simple(SimpleToken::LBracket).is_some() {
                let index = self.parse_expr(0, depth + 1)?;
                let close =
                    self.expect_simple(SimpleToken::RBracket, "expected ']' after index")?;
                let span = Span::new(expr.span_start(), close.span.end);
                expr = Expr::Index {
                    object: Box::new(expr),
                    index: Box::new(index),
                    span,
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_call_suffix(&mut self, depth: usize) -> Result<Option<(Vec<Expr>, usize)>> {
        if self.at_explicit_type_instantiation_open() {
            self.parse_explicit_type_instantiation(depth + 1)?;
            let Some(args) = self.parse_call_args_suffix(depth + 1)? else {
                return Err(ParseError {
                    unclosed_block: None,
                    span: self.peek().span,
                    message: "expected arguments after explicit type instantiation".to_owned(),
                });
            };
            return Ok(Some(args));
        }
        self.parse_call_args_suffix(depth)
    }

    fn parse_call_args_suffix(&mut self, depth: usize) -> Result<Option<(Vec<Expr>, usize)>> {
        if self.newline_before_current_token() {
            return Ok(None);
        }
        if self.at_simple(SimpleToken::LParen) {
            self.advance();
            let mut args = Vec::new();
            if !self.at_simple(SimpleToken::RParen) {
                loop {
                    args.push(self.parse_expr(0, depth + 1)?);
                    if self.eat_simple(SimpleToken::Comma).is_none() {
                        break;
                    }
                }
            }
            let close = self.expect_simple(SimpleToken::RParen, "expected ')' after arguments")?;
            return Ok(Some((args, close.span.end)));
        }
        if matches!(self.peek().kind, TokenKind::String(_)) {
            let token = self.advance().clone();
            let TokenKind::String(value) = token.kind else {
                unreachable!("matched string token");
            };
            let end = token.span.end;
            return Ok(Some((vec![Expr::String(value, token.span)], end)));
        }
        if let Some(open) = self.eat_simple(SimpleToken::LBrace) {
            let table = self.parse_table_literal(open.span, depth + 1)?;
            let end = table.span_end();
            return Ok(Some((vec![table], end)));
        }
        Ok(None)
    }

    fn parse_explicit_type_instantiation(&mut self, depth: usize) -> Result<()> {
        self.check_depth(depth)?;
        self.expect_simple(SimpleToken::Less, "expected '<<' before type arguments")?;
        self.expect_simple(SimpleToken::Less, "expected '<<' before type arguments")?;
        if self.at_explicit_type_instantiation_close() {
            return Err(self.error_here("expected type argument"));
        }
        loop {
            self.parse_explicit_type_instantiation_arg(depth + 1)?;
            if self.eat_simple(SimpleToken::Comma).is_none() {
                break;
            }
        }
        self.expect_simple(SimpleToken::Greater, "expected '>>' after type arguments")?;
        self.expect_simple(SimpleToken::Greater, "expected '>>' after type arguments")?;
        Ok(())
    }

    fn parse_explicit_type_instantiation_arg(&mut self, depth: usize) -> Result<()> {
        self.check_depth(depth)?;
        if self.at_simple(SimpleToken::LParen) {
            self.advance();
            if !self.at_simple(SimpleToken::RParen) {
                loop {
                    self.parse_type(depth + 1)?;
                    if self.eat_simple(SimpleToken::Comma).is_none() {
                        break;
                    }
                }
            }
            self.expect_simple(SimpleToken::RParen, "expected ')' after type pack")?;
        } else {
            self.parse_type(depth + 1)?;
        }
        Ok(())
    }

    fn parse_primary(&mut self, depth: usize) -> Result<Expr> {
        self.check_depth(depth)?;
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Nil => Ok(Expr::Nil(token.span)),
            TokenKind::True => Ok(Expr::Bool(true, token.span)),
            TokenKind::False => Ok(Expr::Bool(false, token.span)),
            TokenKind::Number(value) => Ok(Expr::Number(value, token.span)),
            TokenKind::String(value) => Ok(Expr::String(value, token.span)),
            TokenKind::Ident(name) => {
                if let Some(interpolation) = self.interpolations.remove(&token.span.start) {
                    return self.parse_interpolated_string(interpolation, depth + 1);
                }
                self.check_value_name(&name)?;
                Ok(Expr::Name(name, token.span))
            }
            TokenKind::Type => {
                self.check_value_name("type")?;
                Ok(Expr::Name("type".to_owned(), token.span))
            }
            TokenKind::Ellipsis if self.in_erased_code && !self.function_is_vararg => {
                Err(ParseError {
                    unclosed_block: None,
                    span: token.span,
                    message: "Cannot use '...' outside of a vararg function".to_owned(),
                })
            }
            TokenKind::Ellipsis => Ok(Expr::Varargs(token.span)),
            TokenKind::If => self.parse_if_expression(token.span.start, depth + 1),
            TokenKind::LParen => {
                let expr = self.parse_expr(0, depth + 1)?;
                let close =
                    self.expect_simple(SimpleToken::RParen, "expected ')' after expression")?;
                Ok(Expr::Paren {
                    span: Span::new(token.span.start, close.span.end),
                    expr: Box::new(expr),
                })
            }
            TokenKind::Function => self.parse_function_expression(token.span.start),
            TokenKind::LBrace => self.parse_table_literal(token.span, depth + 1),
            _ => Err(ParseError {
                unclosed_block: None,
                span: token.span,
                message: "expected expression".to_owned(),
            }),
        }
    }

    fn parse_interpolated_string(
        &mut self,
        interpolation: PendingInterpolatedString,
        depth: usize,
    ) -> Result<Expr> {
        self.check_depth(depth)?;
        self.expect_simple(
            SimpleToken::LParen,
            "expected '(' after interpolated string placeholder",
        )?;
        let mut expressions = Vec::new();
        if !self.at_simple(SimpleToken::RParen) {
            loop {
                expressions.push(self.parse_expr(0, depth + 1)?);
                if self.eat_simple(SimpleToken::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect_simple(
            SimpleToken::RParen,
            "expected ')' after interpolated string placeholder",
        )?;
        if expressions.len() != interpolation.expression_count {
            return Err(ParseError {
                unclosed_block: None,
                span: interpolation.span,
                message: "interpolated string expression count mismatch".to_owned(),
            });
        }
        Ok(Expr::InterpolatedString {
            parts: interpolation.parts,
            expressions,
            span: interpolation.span,
        })
    }

    fn parse_if_expression(&mut self, start: usize, depth: usize) -> Result<Expr> {
        self.with_parse_frame(|parser| parser.parse_if_expression_inner(start, depth))
    }

    fn parse_if_expression_inner(&mut self, start: usize, depth: usize) -> Result<Expr> {
        self.check_depth(depth)?;
        let condition = self.parse_expr(0, depth + 1)?;
        self.expect_simple(
            SimpleToken::Then,
            "expected 'then' after if expression condition",
        )?;
        let then_expr = self.parse_expr(0, depth + 1)?;
        let else_expr = if let Some(elseif_token) = self.eat_simple(SimpleToken::ElseIf) {
            self.parse_if_expression(elseif_token.span.start, depth + 1)?
        } else {
            self.expect_simple(SimpleToken::Else, "expected 'else' in if expression")?;
            self.parse_expr(0, depth + 1)?
        };
        let span = Span::new(start, else_expr.span_end());
        Ok(Expr::IfElse {
            condition: Box::new(condition),
            then_expr: Box::new(then_expr),
            else_expr: Box::new(else_expr),
            span,
        })
    }

    fn parse_function_body(
        &mut self,
        start: usize,
        binding: FunctionBinding<'_>,
    ) -> Result<(Vec<FunctionParam>, bool, Vec<Stmt>, usize)> {
        self.with_parse_frame(|parser| parser.parse_function_body_inner(start, binding))
    }

    fn parse_function_body_inner(
        &mut self,
        start: usize,
        binding: FunctionBinding<'_>,
    ) -> Result<(Vec<FunctionParam>, bool, Vec<Stmt>, usize)> {
        self.parse_optional_function_generics()?;
        self.expect_simple(SimpleToken::LParen, "expected '(' after function name")?;
        let mut params = Vec::new();
        let mut is_vararg = false;
        if !self.at_simple(SimpleToken::RParen) {
            loop {
                if self.eat_simple(SimpleToken::Ellipsis).is_some() {
                    is_vararg = true;
                    if self.eat_simple(SimpleToken::Colon).is_some() {
                        if self.at_named_type_pack() {
                            self.parse_type_pack_tail(0)?;
                        } else {
                            self.parse_single_type(0)?;
                        }
                    }
                    break;
                }
                params.push(self.parse_function_param()?);
                if self.eat_simple(SimpleToken::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect_simple(
            SimpleToken::RParen,
            "expected ')' after function parameters",
        )?;
        if self.eat_simple(SimpleToken::Colon).is_some() {
            self.parse_function_return_annotation(0)?;
        }
        // A recursive local is visible in its body and subsequent statements;
        // its own signature still sees the preceding binding.
        if let FunctionBinding::Local(name) = binding {
            self.type_function_locals.bind(name);
        }
        let enclosing_vararg = self.function_is_vararg;
        let enclosing_loops = self.loop_depth;
        self.function_is_vararg = is_vararg;
        self.loop_depth = 0;
        let body = self.with_local_scope(|parser| {
            if matches!(binding, FunctionBinding::Method) {
                parser.type_function_locals.bind("self");
            }
            for param in &params {
                parser.type_function_locals.bind(&param.name);
            }
            parser.parse_delimited_block(&[SimpleToken::End], start, "end")
        });
        self.function_is_vararg = enclosing_vararg;
        self.loop_depth = enclosing_loops;
        let body = body?;
        let end = self
            .expect_simple(SimpleToken::End, "expected 'end' after function body")?
            .span
            .end;
        Ok((params, is_vararg, body, end.max(start)))
    }

    fn parse_function_return_annotation(&mut self, depth: usize) -> Result<()> {
        self.with_parse_frame(|parser| parser.parse_function_return_annotation_inner(depth))
    }

    fn parse_function_return_annotation_inner(&mut self, depth: usize) -> Result<()> {
        self.parse_return_type(depth).map(|_| ())
    }

    fn parse_return_type(&mut self, depth: usize) -> Result<TypeExpr> {
        self.with_parse_frame(|parser| parser.parse_return_type_inner(depth))
    }

    fn parse_return_type_inner(&mut self, depth: usize) -> Result<TypeExpr> {
        self.check_depth(depth)?;
        if self.at_type_pack_tail() {
            self.parse_type_pack_tail(depth + 1)
        } else if self.at_simple(SimpleToken::LParen) {
            let primary = self.parse_type_primary(depth + 1)?;
            // Luau 0.709 Parser::parseReturnType folds suffixes only when the
            // list contains exactly one fixed type, even if it also has a tail
            // (the upstream CLI-140667 compatibility quirk). A forced function
            // signature returns immediately; a suffix needs an outer group.
            let accepts_suffix = match &primary {
                TypeExpr::TypePack { types, .. } => {
                    types
                        .iter()
                        .filter(|ty| !matches!(ty, TypeExpr::Variadic { .. }))
                        .count()
                        == 1
                }
                _ => false,
            };
            if accepts_suffix {
                let start = primary.span_start();
                let ty = self.parse_type_postfix_suffix(primary)?;
                self.parse_type_combinators(ty, None, start, depth + 1)
            } else {
                Ok(primary)
            }
        } else {
            self.parse_single_type(depth + 1)
        }
    }

    fn parse_optional_function_generics(&mut self) -> Result<()> {
        if self.eat_simple(SimpleToken::Less).is_none() {
            return Ok(());
        }

        let mut seen_pack = false;
        loop {
            self.expect_name("expected generic type parameter")?;
            let pack = self.eat_simple(SimpleToken::Ellipsis).is_some();
            if seen_pack && !pack {
                return Err(self.error_here("generic types must precede generic type packs"));
            }
            seen_pack |= pack;
            if self.eat_simple(SimpleToken::Comma).is_none() {
                break;
            }
        }
        self.expect_simple(
            SimpleToken::Greater,
            "expected '>' after generic function parameters",
        )?;
        Ok(())
    }

    fn parse_function_param(&mut self) -> Result<FunctionParam> {
        let name = self.expect_name("expected function parameter name")?;
        let annotation = self.parse_binding_annotation()?;
        let end = annotation
            .as_ref()
            .map_or(name.span.end, TypeExpr::span_end);
        Ok(FunctionParam {
            name: name.text,
            annotation,
            span: Span::new(name.span.start, end),
        })
    }

    fn parse_table_literal(&mut self, open_span: Span, depth: usize) -> Result<Expr> {
        self.check_depth(depth)?;
        let mut fields = Vec::new();
        if !self.at_simple(SimpleToken::RBrace) {
            loop {
                fields.push(self.parse_table_field(depth + 1)?);
                if self.eat_simple(SimpleToken::Comma).is_some()
                    || self.eat_simple(SimpleToken::Semicolon).is_some()
                {
                    if self.at_simple(SimpleToken::RBrace) {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
        let close = self.expect_simple(SimpleToken::RBrace, "expected '}' after table literal")?;
        Ok(Expr::Table {
            fields,
            span: Span::new(open_span.start, close.span.end),
        })
    }

    fn parse_table_field(&mut self, depth: usize) -> Result<TableField> {
        self.check_depth(depth)?;
        if let Some(open) = self.eat_simple(SimpleToken::LBracket) {
            let key = self.parse_expr(0, depth + 1)?;
            self.expect_simple(SimpleToken::RBracket, "expected ']' after table key")?;
            self.expect_simple(SimpleToken::Equal, "expected '=' after table key")?;
            let value = self.parse_expr(0, depth + 1)?;
            return Ok(TableField::Keyed {
                span: Span::new(open.span.start, value.span_end()),
                key,
                value,
            });
        }

        if let TokenKind::Ident(name) = &self.peek().kind {
            if matches!(
                self.tokens.get(self.cursor + 1).map(|token| &token.kind),
                Some(TokenKind::Equal)
            ) {
                let name = name.clone();
                let name_span = self.advance().span;
                self.expect_simple(SimpleToken::Equal, "expected '=' after table field name")?;
                let value = self.parse_expr(0, depth + 1)?;
                return Ok(TableField::Named {
                    span: Span::new(name_span.start, value.span_end()),
                    name,
                    name_span,
                    value,
                });
            }
        }

        let value = self.parse_expr(0, depth + 1)?;
        Ok(TableField::Array {
            span: value.span(),
            value,
        })
    }

    // Assertions and regular alias defaults take a type, not a type pack.
    // Function results and named generic arguments retain their pack grammar.
    fn parse_single_type(&mut self, depth: usize) -> Result<TypeExpr> {
        self.with_parse_frame(|parser| parser.parse_single_type_inner(depth))
    }

    fn parse_single_type_inner(&mut self, depth: usize) -> Result<TypeExpr> {
        let ty = self.parse_type(depth)?;
        if ty.is_single_type() {
            Ok(ty)
        } else {
            Err(ParseError {
                unclosed_block: None,
                span: ty.span(),
                message: "expected a type, not a type pack".to_owned(),
            })
        }
    }

    fn at_named_type_pack(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Ident(_) | TokenKind::Type)
            && matches!(
                self.tokens.get(self.cursor + 1).map(|token| &token.kind),
                Some(TokenKind::Ellipsis)
            )
    }

    fn at_type_pack_tail(&self) -> bool {
        self.at_simple(SimpleToken::Ellipsis) || self.at_named_type_pack()
    }

    fn parse_type_pack_tail(&mut self, depth: usize) -> Result<TypeExpr> {
        self.check_depth(depth)?;
        let start = self.peek().span.start;
        if self.eat_simple(SimpleToken::Ellipsis).is_some() {
            let inner = self.parse_single_type(depth + 1)?;
            Ok(TypeExpr::Variadic {
                span: Span::new(start, inner.span_end()),
                inner: Box::new(inner),
            })
        } else {
            let name = self.expect_name("expected generic type pack name")?;
            let end = self
                .expect_simple(SimpleToken::Ellipsis, "expected '...' after type pack name")?
                .span
                .end;
            Ok(TypeExpr::Variadic {
                span: Span::new(start, end),
                inner: Box::new(TypeExpr::Named {
                    name: name.text,
                    args: Vec::new(),
                    span: name.span,
                }),
            })
        }
    }

    fn parse_generic_pack_default(&mut self, depth: usize) -> Result<()> {
        self.check_depth(depth)?;
        if self.at_type_pack_tail() {
            return self.parse_type_pack_tail(depth + 1).map(|_| ());
        }
        self.expect_simple(SimpleToken::LParen, "expected type pack default")?;
        if !self.at_simple(SimpleToken::RParen) {
            loop {
                if self.at_type_pack_tail() {
                    self.parse_type_pack_tail(depth + 1)?;
                    break;
                }
                self.parse_single_type(depth + 1)?;
                if self.eat_simple(SimpleToken::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect_simple(SimpleToken::RParen, "expected ')' after type pack default")?;
        Ok(())
    }

    fn parse_type(&mut self, depth: usize) -> Result<TypeExpr> {
        self.with_parse_frame(|parser| parser.parse_type_inner(depth))
    }

    fn parse_type_inner(&mut self, depth: usize) -> Result<TypeExpr> {
        if self.interpolations.contains_key(&self.peek().span.start) {
            return Err(ParseError {
                unclosed_block: None,
                span: self.peek().span,
                message: "interpolated strings are not valid in types".to_owned(),
            });
        }
        self.parse_type_union(depth)
    }

    fn parse_type_union(&mut self, depth: usize) -> Result<TypeExpr> {
        self.with_parse_frame(|parser| parser.parse_type_union_inner(depth))
    }

    fn parse_type_union_inner(&mut self, depth: usize) -> Result<TypeExpr> {
        self.check_depth(depth)?;
        let start = self.peek().span.start;
        let separator = if self.eat_simple(SimpleToken::Pipe).is_some() {
            Some(SimpleToken::Pipe)
        } else if self.eat_simple(SimpleToken::Ampersand).is_some() {
            Some(SimpleToken::Ampersand)
        } else {
            None
        };
        let first = self.parse_type_postfix(depth + 1)?;
        self.parse_type_combinators(first, separator, start, depth)
    }

    fn parse_type_combinators(
        &mut self,
        first: TypeExpr,
        mut separator: Option<SimpleToken>,
        start: usize,
        depth: usize,
    ) -> Result<TypeExpr> {
        self.check_depth(depth)?;
        let mut types = vec![first];
        loop {
            let next = if self.at_simple(SimpleToken::Pipe) {
                Some(SimpleToken::Pipe)
            } else if self.at_simple(SimpleToken::Ampersand) {
                Some(SimpleToken::Ampersand)
            } else {
                None
            };
            let Some(next) = next else { break };
            if separator.is_some_and(|previous| {
                std::mem::discriminant(&previous) != std::mem::discriminant(&next)
            }) {
                return Err(
                    self.error_here("mixed union and intersection types require parentheses")
                );
            }
            separator = Some(next);
            self.advance();
            types.push(self.parse_type_postfix(depth + 1)?);
        }
        if matches!(separator, Some(SimpleToken::Ampersand))
            && types
                .iter()
                .any(|ty| matches!(ty, TypeExpr::Optional { .. }))
        {
            return Err(self.error_here("optional intersection members require parentheses"));
        }
        let span = Span::new(
            start,
            types.last().expect("type list is nonempty").span_end(),
        );
        match separator {
            Some(SimpleToken::Pipe) => Ok(TypeExpr::Union { types, span }),
            Some(SimpleToken::Ampersand) => Ok(TypeExpr::Intersection { types, span }),
            _ => Ok(types.pop().expect("type list is nonempty")),
        }
    }

    fn parse_type_postfix(&mut self, depth: usize) -> Result<TypeExpr> {
        self.with_parse_frame(|parser| parser.parse_type_postfix_inner(depth))
    }

    fn parse_type_postfix_inner(&mut self, depth: usize) -> Result<TypeExpr> {
        self.check_depth(depth)?;
        let ty = self.parse_type_primary(depth + 1)?;
        self.parse_type_postfix_suffix(ty)
    }

    fn parse_type_postfix_suffix(&mut self, mut ty: TypeExpr) -> Result<TypeExpr> {
        loop {
            if let Some(question) = self.eat_simple(SimpleToken::Question) {
                let span = Span::new(ty.span_start(), question.span.end);
                ty = TypeExpr::Optional {
                    inner: Box::new(ty),
                    span,
                };
            } else if let Some(ellipsis) = self.eat_simple(SimpleToken::Ellipsis) {
                let span = Span::new(ty.span_start(), ellipsis.span.end);
                ty = TypeExpr::Variadic {
                    inner: Box::new(ty),
                    span,
                };
            } else {
                break;
            }
        }

        Ok(ty)
    }

    fn at_named_type_entry(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Ident(_) | TokenKind::Type)
            && matches!(
                self.tokens.get(self.cursor + 1).map(|token| &token.kind),
                Some(TokenKind::Colon)
            )
    }

    fn parse_table_type(&mut self, open: Span, depth: usize) -> Result<TypeExpr> {
        self.check_depth(depth)?;
        let mut has_property = false;
        let mut has_indexer = false;
        while !self.at_simple(SimpleToken::RBrace) {
            // read/write are contextual: `{read: T}` is an ordinary property.
            if (self.at_ident("read") || self.at_ident("write")) && !self.at_named_type_entry() {
                self.advance();
            }
            if self.eat_simple(SimpleToken::LBracket).is_some() {
                if self.interpolations.contains_key(&self.peek().span.start) {
                    return Err(self.error_here("interpolated strings are not valid in types"));
                }
                let string_property = matches!(self.peek().kind, TokenKind::String(_))
                    && matches!(
                        self.tokens.get(self.cursor + 1).map(|token| &token.kind),
                        Some(TokenKind::RBracket)
                    );
                if string_property {
                    let token = self.advance();
                    if let TokenKind::String(value) = &token.kind {
                        if value.contains(&0) {
                            return Err(
                                self.error_here("table type property cannot contain a null byte")
                            );
                        }
                    }
                    has_property = true;
                } else {
                    if has_indexer {
                        return Err(self.error_here("cannot have more than one table indexer"));
                    }
                    self.parse_single_type(depth + 1)?;
                    has_indexer = true;
                }
                self.expect_simple(SimpleToken::RBracket, "expected ']' after table type key")?;
                self.expect_simple(SimpleToken::Colon, "expected ':' after table type key")?;
                self.parse_single_type(depth + 1)?;
            } else if !has_property && !has_indexer && !self.at_named_type_entry() {
                // Array shorthand is the entire table type, not another field.
                self.parse_single_type(depth + 1)?;
                break;
            } else {
                self.expect_name("expected table type property name")?;
                self.expect_simple(SimpleToken::Colon, "expected ':' after table type property")?;
                self.parse_single_type(depth + 1)?;
                has_property = true;
            }
            if self.eat_simple(SimpleToken::Comma).is_none()
                && self.eat_simple(SimpleToken::Semicolon).is_none()
            {
                break;
            }
        }
        let close = self.expect_simple(SimpleToken::RBrace, "expected '}' after table type")?;
        Ok(TypeExpr::Table {
            span: Span::new(open.start, close.span.end),
        })
    }

    fn parse_type_primary(&mut self, depth: usize) -> Result<TypeExpr> {
        self.with_parse_frame(|parser| parser.parse_type_primary_inner(depth))
    }

    fn parse_type_primary_inner(&mut self, depth: usize) -> Result<TypeExpr> {
        self.check_depth(depth)?;
        #[cfg(test)]
        {
            self.type_primary_visits += 1;
        }
        if self.at_ident("typeof") {
            return self.parse_typeof_expression(depth);
        }
        self.parse_non_typeof_primary(depth)
    }

    fn parse_typeof_expression(&mut self, depth: usize) -> Result<TypeExpr> {
        self.with_parse_frame(|parser| parser.parse_typeof_expression_inner(depth))
    }

    fn parse_typeof_expression_inner(&mut self, depth: usize) -> Result<TypeExpr> {
        let start = self.advance().span.start;
        self.expect_simple(SimpleToken::LParen, "expected '(' after typeof")?;
        let enclosing_erasure = self.in_erased_code;
        self.in_erased_code = true;
        let expression = self.parse_expr(0, depth + 1);
        self.in_erased_code = enclosing_erasure;
        expression?;
        let close =
            self.expect_simple(SimpleToken::RParen, "expected ')' after typeof expression")?;
        Ok(TypeExpr::Typeof {
            span: Span::new(start, close.span.end),
        })
    }

    fn parse_non_typeof_primary(&mut self, depth: usize) -> Result<TypeExpr> {
        if let Some(ellipsis) = self.eat_simple(SimpleToken::Ellipsis) {
            let inner = self.parse_type_postfix(depth + 1)?;
            return Ok(TypeExpr::Variadic {
                span: Span::new(ellipsis.span.start, inner.span().end),
                inner: Box::new(inner),
            });
        }

        // A generic binder forces function syntax, even for one parameter.
        // Bindings are erased, as they are on value-level function declarations.
        let generic_start = self
            .at_simple(SimpleToken::Less)
            .then(|| self.peek().span.start);
        if generic_start.is_some() {
            self.parse_optional_function_generics()?;
        }
        if let Some(open) = self.eat_simple(SimpleToken::LParen) {
            let start = generic_start.unwrap_or(open.span.start);
            let mut force_function = generic_start.is_some();
            let mut types = Vec::new();
            if !self.at_simple(SimpleToken::RParen) {
                loop {
                    if self.at_type_pack_tail() {
                        types.push(self.parse_type_pack_tail(depth + 1)?);
                        // A tail occupies the rest of the parameter/return pack.
                        if self.at_simple(SimpleToken::Comma) {
                            return Err(self.error_here("type pack tail must be last"));
                        }
                        break;
                    }
                    if matches!(self.peek().kind, TokenKind::Ident(_) | TokenKind::Type)
                        && matches!(
                            self.tokens.get(self.cursor + 1).map(|token| &token.kind),
                            Some(TokenKind::Colon)
                        )
                    {
                        self.expect_name("expected function type parameter name")?;
                        self.expect_simple(
                            SimpleToken::Colon,
                            "expected ':' after parameter name",
                        )?;
                        force_function = true;
                    }
                    types.push(self.parse_single_type(depth + 1)?);
                    if self.eat_simple(SimpleToken::Comma).is_none() {
                        break;
                    }
                }
            }
            let close = self.expect_simple(SimpleToken::RParen, "expected ')' in type pack")?;
            if self.eat_simple(SimpleToken::Arrow).is_some() {
                let result = self.parse_return_type(depth + 1)?;
                let span = Span::new(start, result.span_end());
                return Ok(TypeExpr::Function {
                    params: types,
                    result: Box::new(result),
                    span,
                });
            }
            if force_function {
                return Err(self.error_here("expected '->' after function type parameters"));
            }
            let span = Span::new(start, close.span.end);
            return Ok(TypeExpr::TypePack { types, span });
        }
        if generic_start.is_some() {
            return Err(self.error_here("expected '(' after function type generics"));
        }

        if let Some(open) = self.eat_simple(SimpleToken::LBrace) {
            return self.parse_table_type(open.span, depth + 1);
        }

        let token = self.advance().clone();
        let kind = match token.kind {
            TokenKind::Type => TokenKind::Ident("type".to_owned()),
            kind => kind,
        };
        match kind {
            TokenKind::Nil => Ok(TypeExpr::Nil(token.span)),
            TokenKind::True => Ok(TypeExpr::Boolean(true, token.span)),
            TokenKind::False => Ok(TypeExpr::Boolean(false, token.span)),
            TokenKind::String(value) => Ok(TypeExpr::StringSingleton(value, token.span)),
            TokenKind::Ident(mut name) => {
                let mut end = token.span.end;
                if self.eat_simple(SimpleToken::Dot).is_some() {
                    let member = self.expect_name("expected type name after '.'")?;
                    name.push('.');
                    name.push_str(&member.text);
                    end = member.span.end;
                }
                let mut args = Vec::new();
                if self.eat_simple(SimpleToken::Less).is_some() {
                    loop {
                        args.push(self.parse_type(depth + 1)?);
                        if self.eat_simple(SimpleToken::Comma).is_none() {
                            break;
                        }
                    }
                    end = self
                        .expect_simple(SimpleToken::Greater, "expected '>' after type arguments")?
                        .span
                        .end;
                }
                Ok(TypeExpr::Named {
                    name,
                    args,
                    span: Span::new(token.span.start, end),
                })
            }
            _ => Err(ParseError {
                unclosed_block: None,
                span: token.span,
                message: "expected type".to_owned(),
            }),
        }
    }

    fn peek_binary_op(&self) -> Option<(BinaryOp, u8, u8)> {
        match self.peek().kind {
            TokenKind::Or => Some((BinaryOp::Or, 0, 1)),
            TokenKind::And => Some((BinaryOp::And, 1, 2)),
            TokenKind::EqualEqual => Some((BinaryOp::Eq, 2, 3)),
            TokenKind::TildeEqual => Some((BinaryOp::Ne, 2, 3)),
            TokenKind::Less => Some((BinaryOp::Lt, 2, 3)),
            TokenKind::LessEqual => Some((BinaryOp::Le, 2, 3)),
            TokenKind::Greater => Some((BinaryOp::Gt, 2, 3)),
            TokenKind::GreaterEqual => Some((BinaryOp::Ge, 2, 3)),
            TokenKind::Concat => Some((BinaryOp::Concat, 3, 3)),
            TokenKind::Plus => Some((BinaryOp::Add, 3, 4)),
            TokenKind::Minus => Some((BinaryOp::Sub, 3, 4)),
            TokenKind::Star => Some((BinaryOp::Mul, 5, 6)),
            TokenKind::Slash => Some((BinaryOp::Div, 5, 6)),
            TokenKind::FloorDiv => Some((BinaryOp::FloorDiv, 5, 6)),
            TokenKind::Percent => Some((BinaryOp::Mod, 5, 6)),
            TokenKind::Caret => Some((BinaryOp::Pow, 9, 8)),
            _ => None,
        }
    }

    fn expect_name(&mut self, message: &'static str) -> Result<IdentToken> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Ident(text) => Ok(IdentToken {
                text,
                span: token.span,
            }),
            TokenKind::Type => Ok(IdentToken {
                text: "type".to_owned(),
                span: token.span,
            }),
            _ => Err(ParseError {
                unclosed_block: None,
                span: token.span,
                message: message.to_owned(),
            }),
        }
    }

    fn expect_simple(&mut self, expected: SimpleToken, message: &'static str) -> Result<Token> {
        self.eat_simple(expected).ok_or_else(|| ParseError {
            unclosed_block: None,
            span: self.peek().span,
            message: message.to_owned(),
        })
    }

    fn eat_simple(&mut self, expected: SimpleToken) -> Option<Token> {
        if self.at_simple(expected) {
            let token = self.peek().clone();
            self.cursor += 1;
            Some(token)
        } else {
            None
        }
    }

    fn at_simple(&self, expected: SimpleToken) -> bool {
        token_matches(&self.peek().kind, expected)
    }

    fn at_explicit_type_instantiation_open(&self) -> bool {
        self.at_simple(SimpleToken::Less)
            && self
                .tokens
                .get(self.peek_index().saturating_add(1))
                .is_some_and(|token| token_matches(&token.kind, SimpleToken::Less))
    }

    fn at_explicit_type_instantiation_close(&self) -> bool {
        self.at_simple(SimpleToken::Greater)
            && self
                .tokens
                .get(self.peek_index().saturating_add(1))
                .is_some_and(|token| token_matches(&token.kind, SimpleToken::Greater))
    }

    fn at_any(&self, expected: &[SimpleToken]) -> bool {
        expected
            .iter()
            .any(|expected| token_matches(&self.peek().kind, *expected))
    }

    fn at_statement_boundary(&self) -> bool {
        self.at_any(&[
            SimpleToken::Eof,
            SimpleToken::Semicolon,
            SimpleToken::Else,
            SimpleToken::ElseIf,
            SimpleToken::End,
            SimpleToken::Until,
        ])
    }

    fn next_starts_statement_boundary(&self) -> bool {
        self.tokens.get(self.peek_index() + 1).is_some_and(|token| {
            matches!(
                token.kind,
                TokenKind::Eof
                    | TokenKind::Semicolon
                    | TokenKind::Else
                    | TokenKind::ElseIf
                    | TokenKind::End
                    | TokenKind::Until
            )
        })
    }

    fn newline_before_current_token(&self) -> bool {
        self.newlines_before
            .get(self.peek_index())
            .copied()
            .unwrap_or(false)
    }

    fn eat_compound_assignment_op(&mut self) -> Option<AssignmentOp> {
        let op = self.peek_compound_assignment_op()?;
        self.cursor += 1;
        Some(op)
    }

    fn peek_compound_assignment_op(&self) -> Option<AssignmentOp> {
        match self.peek().kind {
            TokenKind::PlusEqual => Some(AssignmentOp::Add),
            TokenKind::MinusEqual => Some(AssignmentOp::Sub),
            TokenKind::StarEqual => Some(AssignmentOp::Mul),
            TokenKind::SlashEqual => Some(AssignmentOp::Div),
            TokenKind::FloorDivEqual => Some(AssignmentOp::FloorDiv),
            TokenKind::PercentEqual => Some(AssignmentOp::Mod),
            TokenKind::CaretEqual => Some(AssignmentOp::Pow),
            TokenKind::ConcatEqual => Some(AssignmentOp::Concat),
            _ => None,
        }
    }

    fn at_ident(&self, expected: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Ident(name) if name == expected)
    }

    fn advance(&mut self) -> &Token {
        let token = self.peek_index();
        if !matches!(self.tokens[token].kind, TokenKind::Eof) {
            self.cursor += 1;
        }
        &self.tokens[token]
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.peek_index()]
    }

    fn peek_index(&self) -> usize {
        self.cursor.min(self.tokens.len().saturating_sub(1))
    }

    fn error_here(&self, message: &'static str) -> ParseError {
        ParseError {
            unclosed_block: None,
            span: self.peek().span,
            message: message.to_owned(),
        }
    }

    // Luau resolves statement-leading contextual names (type, export, continue)
    // as value references before recognizing the declaration or statement.
    fn check_value_name(&self, name: &str) -> Result<()> {
        if self.type_function_locals.is_outer_local(name) {
            return Err(ParseError {
                unclosed_block: None,
                // Luau reports the current token after consuming the name.
                span: self.peek().span,
                message: format!("Type function cannot reference outer local '{name}'"),
            });
        }
        Ok(())
    }

    fn with_local_scope<T>(&mut self, parse: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if !self.type_function_locals.enabled {
            return parse(self);
        }
        let mark = self.type_function_locals.bindings.len();
        let result = parse(self);
        self.type_function_locals.restore(mark);
        result
    }

    fn with_parse_frame<T>(&mut self, parse: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.check_depth(self.active_parse_frames + 1)?;
        self.active_parse_frames += 1;
        let result = parse(self);
        self.active_parse_frames -= 1;
        result
    }

    fn parse_loop_body(
        &mut self,
        terminators: &[SimpleToken],
        start: usize,
        expected: &'static str,
    ) -> Result<Vec<Stmt>> {
        self.loop_depth += 1;
        let result = self.parse_delimited_block(terminators, start, expected);
        self.loop_depth -= 1;
        result
    }

    fn check_depth(&self, depth: usize) -> Result<()> {
        if depth > MAX_PARSE_DEPTH {
            Err(ParseError {
                unclosed_block: None,
                span: self.peek().span,
                message: "parser recursion limit exceeded".to_owned(),
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IdentToken {
    text: String,
    span: Span,
}

#[derive(Clone, Copy)]
enum SimpleToken {
    Eof,
    Local,
    Function,
    Return,
    Type,
    If,
    Then,
    ElseIf,
    Else,
    End,
    While,
    Do,
    For,
    In,
    Repeat,
    Until,
    Break,
    Minus,
    Not,
    Equal,
    Comma,
    Semicolon,
    Colon,
    Dot,
    Hash,
    Ellipsis,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Less,
    Greater,
    Pipe,
    Ampersand,
    Question,
    Arrow,
}

fn token_newlines_before(tokens: &[Token], source: &[u8]) -> Vec<bool> {
    let mut newlines_before = Vec::with_capacity(tokens.len());
    let mut previous_end = 0;
    for token in tokens {
        newlines_before.push(source_contains_newline(
            source,
            previous_end,
            token.span.start,
        ));
        previous_end = token.span.end;
    }
    newlines_before
}

fn source_contains_newline(source: &[u8], start: usize, end: usize) -> bool {
    let start = start.min(source.len());
    let end = end.min(source.len());
    source[start..end]
        .iter()
        .any(|byte| matches!(byte, b'\n' | b'\r'))
}

fn token_matches(kind: &TokenKind, expected: SimpleToken) -> bool {
    matches!(
        (kind, expected),
        (TokenKind::Eof, SimpleToken::Eof)
            | (TokenKind::Local, SimpleToken::Local)
            | (TokenKind::Function, SimpleToken::Function)
            | (TokenKind::Return, SimpleToken::Return)
            | (TokenKind::Type, SimpleToken::Type)
            | (TokenKind::If, SimpleToken::If)
            | (TokenKind::Then, SimpleToken::Then)
            | (TokenKind::ElseIf, SimpleToken::ElseIf)
            | (TokenKind::Else, SimpleToken::Else)
            | (TokenKind::End, SimpleToken::End)
            | (TokenKind::While, SimpleToken::While)
            | (TokenKind::Do, SimpleToken::Do)
            | (TokenKind::For, SimpleToken::For)
            | (TokenKind::In, SimpleToken::In)
            | (TokenKind::Repeat, SimpleToken::Repeat)
            | (TokenKind::Until, SimpleToken::Until)
            | (TokenKind::Break, SimpleToken::Break)
            | (TokenKind::Minus, SimpleToken::Minus)
            | (TokenKind::Not, SimpleToken::Not)
            | (TokenKind::Equal, SimpleToken::Equal)
            | (TokenKind::Comma, SimpleToken::Comma)
            | (TokenKind::Semicolon, SimpleToken::Semicolon)
            | (TokenKind::Colon, SimpleToken::Colon)
            | (TokenKind::Dot, SimpleToken::Dot)
            | (TokenKind::Hash, SimpleToken::Hash)
            | (TokenKind::Ellipsis, SimpleToken::Ellipsis)
            | (TokenKind::LParen, SimpleToken::LParen)
            | (TokenKind::RParen, SimpleToken::RParen)
            | (TokenKind::LBrace, SimpleToken::LBrace)
            | (TokenKind::RBrace, SimpleToken::RBrace)
            | (TokenKind::LBracket, SimpleToken::LBracket)
            | (TokenKind::RBracket, SimpleToken::RBracket)
            | (TokenKind::Less, SimpleToken::Less)
            | (TokenKind::Greater, SimpleToken::Greater)
            | (TokenKind::Pipe, SimpleToken::Pipe)
            | (TokenKind::Ampersand, SimpleToken::Ampersand)
            | (TokenKind::Question, SimpleToken::Question)
            | (TokenKind::Arrow, SimpleToken::Arrow)
    )
}

fn assignment_target_from_expr(expr: Expr) -> Result<AssignmentTarget> {
    match expr {
        Expr::Name(name, span) => Ok(AssignmentTarget::Local { name, span }),
        Expr::Field {
            object,
            field,
            field_span,
            span,
        } => Ok(AssignmentTarget::Field {
            object,
            field,
            field_span,
            span,
        }),
        Expr::Index {
            object,
            index,
            span,
        } => Ok(AssignmentTarget::Index {
            object,
            index,
            span,
        }),
        other => Err(ParseError {
            unclosed_block: None,
            span: other.span(),
            message: "unsupported assignment target".to_owned(),
        }),
    }
}

impl AssignmentTarget {
    fn span_start(&self) -> usize {
        match self {
            Self::Local { span, .. } | Self::Field { span, .. } | Self::Index { span, .. } => {
                span.start
            }
        }
    }

    fn span_end(&self) -> usize {
        match self {
            Self::Local { span, .. } | Self::Field { span, .. } | Self::Index { span, .. } => {
                span.end
            }
        }
    }
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Self::Nil(span)
            | Self::Bool(_, span)
            | Self::Number(_, span)
            | Self::String(_, span)
            | Self::InterpolatedString { span, .. }
            | Self::Name(_, span)
            | Self::Varargs(span)
            | Self::Unary { span, .. }
            | Self::Binary { span, .. }
            | Self::IfElse { span, .. }
            | Self::Field { span, .. }
            | Self::Index { span, .. }
            | Self::Table { span, .. }
            | Self::Function { span, .. }
            | Self::Paren { span, .. }
            | Self::Call { span, .. }
            | Self::MethodCall { span, .. } => *span,
        }
    }

    fn span_start(&self) -> usize {
        match self {
            Self::Nil(span)
            | Self::Bool(_, span)
            | Self::Number(_, span)
            | Self::String(_, span)
            | Self::InterpolatedString { span, .. }
            | Self::Name(_, span)
            | Self::Varargs(span)
            | Self::Unary { span, .. }
            | Self::Binary { span, .. }
            | Self::IfElse { span, .. }
            | Self::Field { span, .. }
            | Self::Index { span, .. }
            | Self::Table { span, .. }
            | Self::Function { span, .. }
            | Self::Paren { span, .. }
            | Self::Call { span, .. }
            | Self::MethodCall { span, .. } => span.start,
        }
    }

    fn span_end(&self) -> usize {
        match self {
            Self::Nil(span)
            | Self::Bool(_, span)
            | Self::Number(_, span)
            | Self::String(_, span)
            | Self::InterpolatedString { span, .. }
            | Self::Name(_, span)
            | Self::Varargs(span)
            | Self::Unary { span, .. }
            | Self::Binary { span, .. }
            | Self::IfElse { span, .. }
            | Self::Field { span, .. }
            | Self::Index { span, .. }
            | Self::Table { span, .. }
            | Self::Function { span, .. }
            | Self::Paren { span, .. }
            | Self::Call { span, .. }
            | Self::MethodCall { span, .. } => span.end,
        }
    }
}

fn expr_accepts_postfix(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Name(_, _)
            | Expr::Field { .. }
            | Expr::Index { .. }
            | Expr::Call { .. }
            | Expr::MethodCall { .. }
            | Expr::Paren { .. }
    )
}

impl TypeExpr {
    fn is_single_type(&self) -> bool {
        match self {
            Self::TypePack { types, .. } => types.len() == 1 && types[0].is_single_type(),
            Self::Variadic { .. } => false,
            Self::Optional { inner, .. } => inner.is_single_type(),
            Self::Union { types, .. } | Self::Intersection { types, .. } => {
                types.iter().all(Self::is_single_type)
            }
            // Function signatures and generic arguments have their own pack
            // positions; a pack inside them does not make the outer type a pack.
            _ => true,
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Self::Named { span, .. }
            | Self::Nil(span)
            | Self::Boolean(_, span)
            | Self::StringSingleton(_, span)
            | Self::Optional { span, .. }
            | Self::Union { span, .. }
            | Self::Intersection { span, .. }
            | Self::Function { span, .. }
            | Self::TypePack { span, .. }
            | Self::Table { span }
            | Self::Typeof { span }
            | Self::Variadic { span, .. } => *span,
        }
    }

    fn span_start(&self) -> usize {
        match self {
            Self::Named { span, .. }
            | Self::Nil(span)
            | Self::Boolean(_, span)
            | Self::StringSingleton(_, span)
            | Self::Optional { span, .. }
            | Self::Union { span, .. }
            | Self::Intersection { span, .. }
            | Self::Function { span, .. }
            | Self::TypePack { span, .. }
            | Self::Table { span }
            | Self::Typeof { span }
            | Self::Variadic { span, .. } => span.start,
        }
    }

    fn span_end(&self) -> usize {
        match self {
            Self::Named { span, .. }
            | Self::Nil(span)
            | Self::Boolean(_, span)
            | Self::StringSingleton(_, span)
            | Self::Optional { span, .. }
            | Self::Union { span, .. }
            | Self::Intersection { span, .. }
            | Self::Function { span, .. }
            | Self::TypePack { span, .. }
            | Self::Table { span }
            | Self::Typeof { span }
            | Self::Variadic { span, .. } => span.end,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_source, parse_source_bytes, AssignmentOp, AssignmentTarget, BinaryOp, Expr, Parser,
        SimpleToken, Stmt, TableField, TypeExpr, UnaryOp,
    };

    #[test]
    fn parses_plain_exported_and_nested_type_function_declarations() {
        let source = "type function first() return types.number end;\n\
            export type function other<T, P...>(x: T, ...: P...): T\n\
                type function nested() return types.string end\n\
                return x\n\
            end";
        let chunk = parse_source(source).unwrap();
        let [Stmt::TypeFunction {
            name: first,
            exported: false,
            span: first_span,
        }, Stmt::TypeFunction {
            name: other,
            exported: true,
            span: export_span,
        }] = chunk.statements.as_slice()
        else {
            panic!("expected two type function declarations");
        };
        assert_eq!(first, "first");
        assert_eq!(other, "other");
        assert_eq!(
            &source[first_span.start..first_span.end],
            "type function first() return types.number end"
        );
        assert_eq!(export_span.start, source.find("export").unwrap());
        assert_eq!(export_span.end, source.len());
    }

    #[test]
    fn type_functions_reject_captures_across_their_lexical_boundary() {
        let cases = [
            ("local x; type function f() return x end", "x"),
            ("local x; type function f() x = 1 end", "x"),
            (
                "local x; type function f() function x.method() end end",
                "x",
            ),
            (
                "local x; type function f() return function() return x end end",
                "x",
            ),
            ("local x; type function f(x: typeof(x)) end", "x"),
            (
                "local x; type function f() local function x(): typeof(x) end end",
                "x",
            ),
            ("local x; type function f() local x = x end", "x"),
            ("local x; type function f() for x in x do end end", "x"),
            ("local x; type function f() return `value {x}` end", "x"),
            (
                "function obj:method() type function f() return self end end",
                "self",
            ),
            ("for x=1,2 do type function f() return x end end", "x"),
            (
                "for k,v in pairs(t) do type function f() return v end end",
                "v",
            ),
            (
                "repeat local x=1 until (function() type function f() return x end end)()",
                "x",
            ),
            ("type function f(x) type function g() return x end end", "x"),
            ("local type; type function f() return type end", "type"),
            ("local type; type function f() type T=number end", "type"),
            (
                "local export; type function f() export type T=number end",
                "export",
            ),
            (
                "local continue; type function f() for i=1,2 do continue end end",
                "continue",
            ),
        ];
        for (source, name) in cases {
            let Err(super::SourceError::Parse(error)) = parse_source(source) else {
                panic!("expected a parse error for {source}");
            };
            assert_eq!(
                error.message,
                format!("Type function cannot reference outer local '{name}'"),
                "{source}"
            );
        }
        let source = "local x; type function f() return x\nend";
        let Err(super::SourceError::Parse(error)) = parse_source(source) else {
            panic!("expected a capture error");
        };
        assert_eq!(&source[error.span.start..error.span.end], "end");
    }

    #[test]
    fn type_function_value_names_follow_block_scope_and_declaration_order() {
        for source in [
            "type function f(x) local function g() x = x end return g end",
            "local x; type function f(x) if x then local x = x else return x end end",
            "local x; type function f() local function x() return x end end",
            "local self; type function f() function t:m() return self end end",
            "local x; type function f() return ({x=1}).x end",
            "local x; type function f() repeat local x=true until x end",
            "local x; type function f() for x=1,3 do print(x) end end",
            "local x; type function f() for x in next,t do print(x) end end",
            "type function f(x) type function g(x) return x end return x end",
            "local x = (function() type function f() return x end end)()",
            "if true then local x=1 else type function f() return x end end",
            "do local x end; while false do local y end; for z=1,1 do end; type function f() return x,y,z end",
            "local type; type function f() export type T=number end",
            "local continue; type function f() local continue; for i=1,2 do continue end end",
            "type function f() end; local function g() return ... end",
        ] {
            assert!(parse_source(source).is_ok(), "{source}");
        }
    }

    #[test]
    fn local_annotations_require_a_single_type_in_value_and_type_functions() {
        for declaration in ["local function", "type function"] {
            for ty in [
                "number",
                "(number)",
                "(A...) -> (string,number)",
                "Box<A...>",
            ] {
                let source = format!("{declaration} f() local x:{ty} end");
                assert!(parse_source(&source).is_ok(), "{source}");
            }
            for ty in ["(number,string)", "...number", "A...", "(A...)"] {
                let source = format!("{declaration} f() local x:{ty} end");
                assert!(parse_source(&source).is_err(), "{source}");
            }
        }
    }

    #[test]
    fn value_and_type_function_parameters_use_the_pinned_type_pack_grammar() {
        for declaration in ["local function", "type function"] {
            for annotation in [
                "x: (number)",
                "x: (A...) -> (string, number)",
                "...: number",
                "...: (number)",
                "...: A...",
                "...: type...",
                "...: <T>(T) -> T",
                "...: Box<A...>",
                "...: (number, string) -> ()",
            ] {
                let source = format!("{declaration} f({annotation}) end");
                assert!(parse_source(&source).is_ok(), "{source}");
            }
            for annotation in [
                "x: (number, string)",
                "x: ...number",
                "x: A...",
                "...: ...number",
                "...: (number, string)",
                "...: Mod.A...",
                "...: Box<A>...",
                "...: (A)...",
                "...: A...?",
                "...: A... | number",
                "...: A?...",
            ] {
                let source = format!("{declaration} f({annotation}) end");
                assert!(parse_source(&source).is_err(), "{source}");
            }
        }
    }

    #[test]
    fn type_function_bodies_validate_syntax_before_erasure() {
        for source in [
            "type function () end",
            "type function f.x() end",
            "export type function f:x() end",
            "type function f<T=number>() end",
            "type function f() return ... end",
            "while true do type function f() break end end",
            "type function f() continue end",
            "type function f() if true then return 1 end",
        ] {
            assert!(parse_source(source).is_err(), "{source}");
        }
    }

    #[test]
    fn exported_aliases_and_assertions_require_complete_types() {
        for source in [
            "export type = number",
            "export type A =",
            "local x = 5 ::",
            "local x = 5 :: Module.",
            "local x = 5 :: Module.Type.Inner",
            "local x = 5 :: number :: any",
            "local x = 5 :: {value: number",
            "local x = 5 : : number",
        ] {
            assert!(parse_source(source).is_err(), "{source}");
        }
        assert!(parse_source("local export = 1; export += 1; return export").is_ok());
        assert!(parse_source("export type A<T> = {value:T}; return 5 :: A<number>").is_ok());
    }

    #[test]
    fn parses_local_annotation_without_typechecking() {
        let chunk = parse_source("local x: Option<number>? = 1 + 2 * 3; return x").unwrap();
        assert_eq!(chunk.statements.len(), 2);
        let Stmt::Local { bindings, .. } = &chunk.statements[0] else {
            panic!("expected annotated local");
        };
        let Some(annotation) = &bindings[0].annotation else {
            panic!("expected local annotation");
        };
        assert!(matches!(annotation, TypeExpr::Optional { .. }));
    }

    #[test]
    fn parses_raw_non_utf8_source_bytes() {
        let chunk = parse_source_bytes(b"return 'a\xffb'").unwrap();
        assert!(matches!(
            chunk.statements.as_slice(),
            [Stmt::Return { value, extra_values, .. }]
                if extra_values.is_empty()
                    && matches!(value, Some(Expr::String(value, _)) if value == b"a\xffb")
        ));
    }

    #[test]
    fn parses_interpolated_strings_without_scanning_ignored_regions() {
        parse_source(
            "local s = 'literal ` tick'\nlocal l = [[long ` tick]]\n-- comment ` tick\nreturn `x {s} {l}`",
        )
        .unwrap();
        parse_source("return `{(function() -- } ` ignored\n return 'ok' end)()}`").unwrap();
    }

    #[test]
    fn interpolation_lowering_preserves_original_expression_and_following_spans() {
        let source = "local first = `x {foo + 1}`\nlocal after = bar";
        let chunk = parse_source(source).unwrap();
        let [Stmt::Local {
            init: first_init, ..
        }, Stmt::Local {
            bindings,
            init: after_init,
            ..
        }] = chunk.statements.as_slice()
        else {
            panic!("expected two local statements");
        };

        let [Expr::InterpolatedString {
            expressions, span, ..
        }] = first_init.as_slice()
        else {
            panic!("expected interpolated string initializer");
        };
        assert_eq!(*span, rustia_lexer::Span::new(14, 27));
        let [Expr::Paren { expr, .. }] = expressions.as_slice() else {
            panic!("expected lowered interpolation expression wrapper");
        };
        let Expr::Binary { lhs, .. } = expr.as_ref() else {
            panic!("expected binary interpolation expression");
        };
        assert!(matches!(
            lhs.as_ref(),
            Expr::Name(name, span)
                if name == "foo"
                    && *span == rustia_lexer::Span::new(
                        source.find("foo").unwrap(),
                        source.find("foo").unwrap() + 3,
                    )
        ));

        assert_eq!(
            bindings[0].span,
            rustia_lexer::Span::new(
                source.find("after").unwrap(),
                source.find("after").unwrap() + 5,
            )
        );
        assert!(matches!(
            after_init.as_slice(),
            [Expr::Name(name, span)]
                if name == "bar"
                    && *span == rustia_lexer::Span::new(
                        source.rfind("bar").unwrap(),
                        source.rfind("bar").unwrap() + 3,
                    )
        ));
    }

    #[test]
    fn nested_interpolation_preserves_spans_after_the_outer_rewrite() {
        let source = "local outer = `outer {`inner {needle}`}`\nlocal after = value";
        let chunk = parse_source(source).unwrap();
        let [Stmt::Local { init: outer, .. }, Stmt::Local { bindings, init, .. }] =
            chunk.statements.as_slice()
        else {
            panic!("expected two locals");
        };
        let [Expr::InterpolatedString { expressions, .. }] = outer.as_slice() else {
            panic!("expected outer interpolation");
        };
        let [Expr::Paren { expr, .. }] = expressions.as_slice() else {
            panic!("expected outer expression wrapper");
        };
        let Expr::InterpolatedString { expressions, .. } = expr.as_ref() else {
            panic!("expected nested interpolation");
        };
        let [Expr::Paren { expr, .. }] = expressions.as_slice() else {
            panic!("expected nested expression wrapper");
        };
        assert!(matches!(
            expr.as_ref(),
            Expr::Name(name, span)
                if name == "needle"
                    && *span == rustia_lexer::Span::new(
                        source.find("needle").unwrap(),
                        source.find("needle").unwrap() + 6,
                    )
        ));
        assert_eq!(bindings[0].span.start, source.find("after").unwrap());
        assert!(matches!(
            init.as_slice(),
            [Expr::Name(name, span)]
                if name == "value" && span.start == source.rfind("value").unwrap()
        ));
    }

    #[test]
    fn rejects_invalid_interpolated_string_forms() {
        let cases = [
            "f`x`",
            "return `{{}}`",
            "return `\\q`",
            "return `line\nbreak`",
        ];
        for source in cases {
            assert!(parse_source(source).is_err(), "{source}");
        }
    }

    #[test]
    fn shared_recursion_budget_covers_type_expression_function_cycles() {
        let mut ty = "number".to_owned();
        for _ in 0..1200 {
            ty = format!("typeof(function(): {ty} end)");
        }
        let error = parse_source(&format!("type T = {ty}")).unwrap_err();
        assert!(error
            .to_string()
            .contains("parser recursion limit exceeded"));
    }

    #[test]
    fn shared_recursion_budget_covers_nested_statement_blocks() {
        let blocks = format!("{}{}", "do ".repeat(1200), "end ".repeat(1200));
        assert!(parse_source(&blocks)
            .unwrap_err()
            .to_string()
            .contains("parser recursion limit exceeded"));
    }

    #[test]
    fn typeof_function_bodies_check_context_without_changing_ordinary_error_phase() {
        for source in [
            "type T = typeof(function(...) return ... end)",
            "type T = typeof(function() while true do break end end)",
            "type T = typeof(function() for i=1,1 do continue end end)",
            "type T = typeof(function(x:typeof(...)):typeof(...) end)",
            "local function outer(...) type T = typeof(function(x:typeof(...)) end) end",
        ] {
            assert!(parse_source(source).is_ok(), "{source}");
        }
        for source in [
            "type T = typeof(function() break end)",
            "type T = typeof(function() continue end)",
            "type T = typeof(function() return ... end)",
            "type T = typeof(function() local x=... end)",
            "type T = typeof(function(...) return function() return ... end end)",
            "type T = typeof(function() while true do local f=function() break end end end)",
            "local function outer() type T = typeof(function(x:typeof(...)) end) end",
        ] {
            assert!(parse_source(source).is_err(), "{source}");
        }
        assert!(parse_source("local function f() break end").is_ok());
        assert!(parse_source("local function f() return ... end").is_ok());
    }

    #[test]
    fn typeof_annotations_validate_expression_syntax() {
        for source in [
            "type T = typeof(1)",
            "local x: typeof(`x`) = 1",
            "type T = typeof(error('never execute'))",
            "type T = typeof(`value {error('never execute')}`)",
            "type T = typeof(function(x:number):number return x end)",
            "type T = typeof((function(...):...number return ... end)(1,2))",
        ] {
            assert!(parse_source(source).is_ok(), "{source}");
        }
        for source in [
            "type T = typeof()",
            "type T = typeof(1+)",
            "type T = typeof(this is not an expression)",
            "type T = typeof(local x = 1)",
            "type T = typeof(return 1)",
            "type T = typeof(1,2)",
            "type T = typeof(function( end)",
            "type T = typeof((1)",
        ] {
            assert!(parse_source(source).is_err(), "{source}");
        }
    }

    #[test]
    fn nested_function_result_types_are_parsed_once() {
        for depth in [1, 8, 24] {
            let mut source = "number".to_owned();
            for _ in 0..depth {
                source = format!("() -> ({source})");
            }
            let mut parser = Parser::new(rustia_lexer::lex_bytes(source.as_bytes()).unwrap());
            parser.parse_type(0).unwrap();
            assert!(parser.at_simple(SimpleToken::Eof));
            assert_eq!(parser.type_primary_visits, 2 * depth + 1);
        }
    }

    #[test]
    fn table_types_validate_fields_indexers_and_array_shorthand() {
        for ty in [
            "{}",
            "{number}",
            "{{number}}",
            "{x:number,y:string;}",
            "{[string]:number,x:boolean}",
            "{x:number;[number]:string}",
            "{[\"a\"]:number,[\"b\"]:string}",
            "{[\"a\" | \"b\"]:number}",
            "{read x:number,write [string]:boolean}",
            "{read:number,write:string}",
            "{id:<T>(T)->T}",
            "{read number}",
            "{x: | number | string}",
            "{x: & A & B}",
            "{| number | string}",
            "{[| number | string]:number}",
            "{read | number | string}",
            "{x: A & (B | C)}",
        ] {
            let source = format!("type T = {ty}");
            assert!(parse_source(&source).is_ok(), "{source}");
        }
        for ty in [
            "{x:}",
            "{this is not a type}",
            "{x:number y:string}",
            "{number,}",
            "{number,x:string}",
            "{T,U}",
            "{[string]:number,[number]:string}",
            "{[string]number}",
            "{x:(number,string)}",
            "{[T...]:number}",
            "{[\"a\\0b\"]:number}",
            "{read}",
            "{[`field{1}`]:number}",
            "{[`plain`]:number}",
            "{read [`field{1}`]:number}",
            "{x:number;;}",
            "{x: A | B & C}",
            "{x: A & B | C}",
            "{x: A? & B}",
            "{x: A & B?}",
        ] {
            let source = format!("type T = {ty}");
            assert!(parse_source(&source).is_err(), "{source}");
        }
    }

    #[test]
    fn return_annotations_distinguish_single_types_and_packs() {
        for annotation in [
            "(number)?",
            "(number) | nil",
            "(number,T...)?",
            "(number,...string) | nil",
            "((number))?",
            "(number,U...) & nil",
            "()",
            "(number,string)",
            "(...number)",
            "T...",
            "...number | nil",
            "(x:number)->number",
            "<T>(T)->T",
            "((number,string)->number)",
            "((number,string)->()) | nil",
        ] {
            let source = format!("local function f(): {annotation} return nil end");
            assert!(parse_source(&source).is_ok(), "{source}");
        }
        for annotation in [
            "((number,string))",
            "T...?",
            "(number,string)?",
            "() | nil",
            "(...number)?",
            "(number,)",
            "(T...,number)",
            "(x:number)",
            "(number,string)->() | nil",
            "(number,string)->(number,string)?",
        ] {
            let source = format!("local function f(): {annotation} return nil end");
            assert!(parse_source(&source).is_err(), "{source}");
        }
    }

    #[test]
    fn parses_generic_function_types_and_named_parameters() {
        for ty in [
            "<T>(T) -> T",
            "<T>(value:T) -> T",
            "<T>(T,value:T) -> T",
            "<T,U...>(T,U...) -> U...",
            "<T>(T) -> <U>(U) -> T",
            "<type>(type:type) -> type",
            "<T,T>(T) -> T",
            "<T>(T) -> () | nil",
            "<T>() -> (number,string)?",
            "(value:number, string) -> number",
        ] {
            let source = format!("type F = {ty}");
            assert!(parse_source(&source).is_ok(), "{source}");
            assert!(
                parse_source(&format!("local value = nil :: ({ty})")).is_ok(),
                "{ty}"
            );
        }
        assert!(parse_source("type Box<F = <T>(T)->T> = F").is_ok());
        assert!(parse_source("type A = C<B<<T>() -> T>>").is_ok());
        assert!(parse_source(
            "local function f(): (x:number)->number return function(x) return x end end"
        )
        .is_ok());
        let source = "type Identity = <T>(value:T) -> T";
        let chunk = parse_source(source).unwrap();
        let Stmt::TypeAlias {
            value: TypeExpr::Function { params, span, .. },
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected function type");
        };
        assert_eq!(params.len(), 1);
        assert_eq!(span.start, source.find('<').unwrap());
        assert_eq!(span.end, source.len());
    }

    #[test]
    fn rejects_malformed_generic_function_types() {
        for ty in [
            "<>(number) -> number",
            "<T=number>(T) -> T",
            "<P...=()>() -> ()",
            "<P...,T>(P...) -> T",
            "<T,>(T) -> T",
            "<T>(T)",
            "<T>()",
            "<T>number",
            "<T>(x:T...) -> T",
            "<T>(x:...T) -> T",
            "<T>(T...,number) -> T",
            "<T>((T...)) -> T",
            "<T>(()) -> T",
            "<T>((number,string)) -> T",
            "<T>(T) -> (value:T)",
        ] {
            assert!(parse_source(&format!("type F = {ty}")).is_err(), "{ty}");
        }
        for annotation in ["(number,string)?", "(number,string) | nil", "() | nil"] {
            assert!(
                parse_source(&format!("local function f(): {annotation} return nil end")).is_err(),
                "{annotation}"
            );
        }
        // Source excerpts from Luau 0.709 tests/Parser.test.cpp, MIT licensed:
        // function_type_matching_parenthesis, empty_function_type_error_recovery,
        // and function_type_named_arguments; permission terms in bundled LICENSE-MIT.
        // Copyright (c) 2019-2025 Roblox Corporation; (c) 1994-2019 Lua.org, PUC-Rio.
        for source in [
            "local a: <T>(number -> string",
            "type Fn = (any, string | number | <a>()) -> any",
            "type Fn = (any, string | number | <a...>()) -> any",
            "type MyFunc = (a: number, b: string, c: number) -> (d: number, e: string, f: number)",
        ] {
            assert!(parse_source(source).is_err(), "{source}");
        }
    }

    #[test]
    fn parses_defaulted_alias_generics() {
        for source in [
            "type Box<T = number> = { value: T }",
            "export type Map<K, V = string> = { [K]: V }",
            "type Pair<T = number, U = string> = { first: T, second: U }",
            "type Callback<T = () -> (number, string)> = T",
            "type Grouped<T = (number)> = T",
            "type Packs<T, P... = (number, string)> = T",
            "type Packs<P... = ()> = number",
            "type Packs<P... = (number)> = number",
            "type Packs<P... = (number, ...string)> = number",
            "type Packs<P... = U...> = number",
            "type Packs<P... = ...number?> = number",
        ] {
            assert!(parse_source(source).is_ok(), "{source}");
        }
    }

    #[test]
    fn rejects_invalid_alias_generic_defaults() {
        for source in [
            "type Box<T = > = T",
            "type Box<T = number, U> = T",
            "type Box<T = number, P...> = T",
            "type Box<P..., T> = T",
            "type Box<T = number,> = T",
            "type Box<T = (number, string)> = T",
            "type Box<T = ...number> = T",
            "type Box<P... = number> = number",
            "type Box<P... = (number), Q...> = number",
            "type Box<P... = (U..., number)> = number",
            "type Box<P... = (...number, string)> = number",
            "type Box<P... = ((number,string))> = number",
            "local function f<T = number>() end",
        ] {
            assert!(parse_source(source).is_err(), "{source}");
        }
    }

    #[test]
    fn assertions_require_one_type_but_preserve_function_and_generic_packs() {
        for annotation in [
            "number",
            "(number)",
            "((number))",
            "(number)?",
            "() -> (number, string)",
            "(number) -> ()",
            "(number...) -> number",
            "Foo<(number, string)>",
            "Foo<...number>",
        ] {
            let source = format!("local x = 1 :: {annotation}");
            assert!(parse_source(&source).is_ok(), "{source}");
        }
        for annotation in [
            "()",
            "(number, string)",
            "((number, string))",
            "(...number)",
            "number...",
            "...number",
            "(number,string)?",
            "(number,string) | number",
        ] {
            let source = format!("local x = 1 :: {annotation}");
            assert!(parse_source(&source).is_err(), "{source}");
        }
    }

    #[test]
    fn parses_type_aliases_and_luau_type_operators() {
        let chunk = parse_source(
            "type Alias<T> = ({foo: T}) -> T | nil\nlocal x: typeof(foo) & 'ready' = 1",
        )
        .unwrap();
        assert_eq!(chunk.statements.len(), 2);
        assert!(matches!(
            &chunk.statements[0],
            Stmt::TypeAlias {
                value: TypeExpr::Function { .. },
                ..
            }
        ));
    }

    #[test]
    fn parses_erased_function_type_packs() {
        let chunk = parse_source("local function f(callback: () -> ()): () end").unwrap();
        let [Stmt::LocalFunction { params, .. }] = chunk.statements.as_slice() else {
            panic!("expected local function");
        };
        let Some(TypeExpr::Function { result, .. }) = &params[0].annotation else {
            panic!("expected function type annotation");
        };
        assert!(matches!(result.as_ref(), TypeExpr::TypePack { types, .. } if types.is_empty()));
    }

    #[test]
    fn rejects_statements_after_return() {
        let error = parse_source("return 1\nlocal x = 2").unwrap_err();
        assert!(error.to_string().contains("return must be the final"));
    }

    #[test]
    fn rejects_empty_semicolon_statements() {
        let error = parse_source(";").unwrap_err();
        assert!(error.to_string().contains("expected expression"));

        let error = parse_source("local x = 1;;").unwrap_err();
        assert!(error.to_string().contains("expected expression"));

        let error = parse_source("return;;").unwrap_err();
        assert!(error.to_string().contains("return must be the final"));
    }

    #[test]
    fn accepts_single_statement_terminating_semicolons() {
        parse_source("local x = 1; return x;").unwrap();
    }

    #[test]
    fn rejects_call_parentheses_after_newline() {
        let error = parse_source("a = math.sin\n(3)").unwrap_err();
        assert!(error.to_string().contains("expected statement"));
    }

    #[test]
    fn parses_call_expressions() {
        let chunk = parse_source("return assert(assert(1))").unwrap();
        let Stmt::Return {
            value: Some(Expr::Call { args, .. }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected call return");
        };
        assert_eq!(args.len(), 1);
        assert!(matches!(args[0], Expr::Call { .. }));
    }

    #[test]
    fn parses_return_expression_lists() {
        let chunk = parse_source("return 1, 2, 3").unwrap();
        let Stmt::Return {
            value: Some(Expr::Number(first, _)),
            extra_values,
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected return list");
        };
        assert_eq!(*first, 1.0);
        assert_eq!(extra_values.len(), 2);
        assert!(matches!(extra_values[0], Expr::Number(2.0, _)));
        assert!(matches!(extra_values[1], Expr::Number(3.0, _)));
    }

    #[test]
    fn parses_method_call_expressions_and_statements() {
        let chunk = parse_source("return t:m(1, 2)").unwrap();
        let Stmt::Return {
            value:
                Some(Expr::MethodCall {
                    method,
                    args,
                    method_span,
                    ..
                }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected method call return");
        };
        assert_eq!(method, "m");
        assert_eq!(args.len(), 2);
        assert_eq!(method_span.start, 9);

        let chunk = parse_source("t:m\"x\"; t:n{}").unwrap();
        assert!(matches!(
            &chunk.statements[0],
            Stmt::Expr {
                expr: Expr::MethodCall { args, .. },
                ..
            } if matches!(args.as_slice(), [Expr::String(value, _)] if value == b"x")
        ));
        assert!(matches!(
            &chunk.statements[1],
            Stmt::Expr {
                expr: Expr::MethodCall { args, .. },
                ..
            } if matches!(args.as_slice(), [Expr::Table { .. }])
        ));
    }

    #[test]
    fn parses_erased_explicit_type_instantiation_calls() {
        let chunk = parse_source(
            "local function multipleReturns<T>(x: T): (T, T) return x, x end\nlocal function typePacks<T...>(...: T...): T... return ... end\nlocal a = multipleReturns<<number>>(1)\nlocal b, c = typePacks<<(string, number)>>(1, 'a')\nt:methodTypePacks<<(string, number)>>(1, 'a')",
        )
        .unwrap();
        let Stmt::Local { init, .. } = &chunk.statements[2] else {
            panic!("expected local call");
        };
        assert!(matches!(init.as_slice(), [Expr::Call { args, .. }] if args.len() == 1));

        let Stmt::Local { init, .. } = &chunk.statements[3] else {
            panic!("expected type-pack local call");
        };
        assert!(matches!(init.as_slice(), [Expr::Call { args, .. }] if args.len() == 2));

        assert!(matches!(
            &chunk.statements[4],
            Stmt::Expr {
                expr: Expr::MethodCall { method, args, .. },
                ..
            } if method == "methodTypePacks" && args.len() == 2
        ));
    }

    #[test]
    fn parses_local_function_declarations() {
        let chunk =
            parse_source("local function f(x: number, y) return x + y end return f(1, 2)").unwrap();
        let Stmt::LocalFunction {
            name, params, body, ..
        } = &chunk.statements[0]
        else {
            panic!("expected local function declaration");
        };
        assert_eq!(name, "f");
        assert_eq!(params.len(), 2);
        assert!(params[0].annotation.is_some());
        assert!(matches!(body.first(), Some(Stmt::Return { .. })));
    }

    #[test]
    fn parses_function_declaration_statements() {
        let chunk =
            parse_source("function f(x) return x end\nfunction a.b.c:f2(x, y) self[x] = y end")
                .unwrap();

        let Stmt::Function {
            target,
            is_method,
            params,
            body,
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected function declaration");
        };
        assert!(matches!(target, AssignmentTarget::Local { name, .. } if name == "f"));
        assert!(!is_method);
        assert_eq!(params.len(), 1);
        assert!(matches!(body.first(), Some(Stmt::Return { .. })));

        let Stmt::Function {
            target,
            is_method,
            params,
            body,
            ..
        } = &chunk.statements[1]
        else {
            panic!("expected method declaration");
        };
        assert!(matches!(target, AssignmentTarget::Field { field, .. } if field == "f2"));
        assert!(is_method);
        assert_eq!(params.len(), 2);
        assert!(matches!(body.first(), Some(Stmt::Assignment { .. })));
    }

    #[test]
    fn parses_contextual_type_name_in_fields_and_function_targets() {
        let chunk =
            parse_source("function t.type<T>(x: T): T return x end\nt:type(2)\nreturn t.type(1)")
                .unwrap();

        let Stmt::Function {
            target,
            is_method,
            params,
            body,
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected function declaration");
        };
        assert!(matches!(target, AssignmentTarget::Field { field, .. } if field == "type"));
        assert!(!is_method);
        assert_eq!(params.len(), 1);
        assert!(params[0].annotation.is_some());
        assert!(matches!(body.first(), Some(Stmt::Return { .. })));

        assert!(matches!(
            &chunk.statements[1],
            Stmt::Expr {
                expr: Expr::MethodCall { method, .. },
                ..
            } if method == "type"
        ));

        let Stmt::Return {
            value: Some(Expr::Call { callee, .. }),
            ..
        } = &chunk.statements[2]
        else {
            panic!("expected field call return");
        };
        assert!(matches!(
            callee.as_ref(),
            Expr::Field { field, .. } if field == "type"
        ));
    }

    #[test]
    fn parses_contextual_type_name_in_bindings() {
        let chunk = parse_source(
            "local type = 1\nlocal function f(type) return type end\nfor type in next, {1} do return f(type) end",
        )
        .unwrap();
        assert!(
            matches!(&chunk.statements[0], Stmt::Local { bindings, .. } if bindings[0].name == "type")
        );
        assert!(
            matches!(&chunk.statements[1], Stmt::LocalFunction { params, .. } if params[0].name == "type")
        );
        assert!(
            matches!(&chunk.statements[2], Stmt::ForGeneric { variables, .. } if variables[0].name == "type")
        );
    }

    #[test]
    fn parses_anonymous_function_expressions() {
        let chunk = parse_source("return (function (x) return x * 2 end)(20)").unwrap();
        let Stmt::Return {
            value: Some(Expr::Call { callee, args, .. }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected call return");
        };
        assert!(matches!(
            callee.as_ref(),
            Expr::Paren {
                expr,
                ..
            } if matches!(expr.as_ref(), Expr::Function { .. })
        ));
        assert_eq!(args.len(), 1);
    }

    #[test]
    fn rejects_postfix_suffixes_on_non_prefix_expressions() {
        let error = parse_source("1()").unwrap_err();
        assert!(error.to_string().contains("expected statement"));

        let error = parse_source("1:m()").unwrap_err();
        assert!(error.to_string().contains("expected statement"));
    }

    #[test]
    fn parses_vararg_function_parameters_and_expressions() {
        let chunk = parse_source("local function f(a, ...) return a, ... end").unwrap();
        let Stmt::LocalFunction {
            params,
            is_vararg,
            body,
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected local vararg function");
        };
        assert_eq!(params.len(), 1);
        assert!(*is_vararg);
        let Some(Stmt::Return { extra_values, .. }) = body.first() else {
            panic!("expected vararg return");
        };
        assert!(matches!(extra_values.as_slice(), [Expr::Varargs(_)]));

        let chunk = parse_source("local function f<T...>(...: T...): T... return ... end").unwrap();
        let Stmt::LocalFunction {
            params,
            is_vararg,
            body,
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected typed vararg function");
        };
        assert!(params.is_empty());
        assert!(*is_vararg);
        assert!(matches!(
            body.first(),
            Some(Stmt::Return {
                value: Some(Expr::Varargs(_)),
                ..
            })
        ));
    }

    #[test]
    fn parses_string_literal_call_arguments() {
        let chunk = parse_source(r#"return string.reverse"\0abc""#).unwrap();
        let Stmt::Return {
            value: Some(Expr::Call { args, .. }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected call return");
        };
        assert_eq!(args.len(), 1);
        assert!(matches!(&args[0], Expr::String(value, _) if value.as_slice() == b"\0abc"));
    }

    #[test]
    fn parses_table_constructor_call_arguments() {
        let chunk = parse_source("return table.getn{1, 2, nil}").unwrap();
        let Stmt::Return {
            value: Some(Expr::Call { args, .. }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected call return");
        };
        assert_eq!(args.len(), 1);
        assert!(matches!(&args[0], Expr::Table { fields, .. } if fields.len() == 3));
    }

    #[test]
    fn parses_type_keyword_as_expression_name() {
        let chunk = parse_source("return type(nil)").unwrap();
        let Stmt::Return {
            value: Some(Expr::Call { callee, .. }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected type call return");
        };
        assert!(matches!(callee.as_ref(), Expr::Name(name, _) if name == "type"));
    }

    #[test]
    fn parses_type_keyword_call_statement() {
        let chunk = parse_source("type(nil); return nil").unwrap();
        assert!(matches!(
            &chunk.statements[0],
            Stmt::Expr {
                expr: Expr::Call { callee, .. },
                ..
            } if matches!(callee.as_ref(), Expr::Name(name, _) if name == "type")
        ));
    }

    #[test]
    fn parses_call_statements_but_rejects_bare_expression_statements() {
        let chunk = parse_source("assert(1); return nil").unwrap();
        assert!(matches!(chunk.statements[0], Stmt::Expr { .. }));

        let error = parse_source("1 + 2").unwrap_err();
        assert!(error.to_string().contains("expected statement"));
    }

    #[test]
    fn parses_multi_name_local_declarations() {
        let chunk = parse_source("local a, b: number, c = 1, 2\nreturn c").unwrap();
        let Stmt::Local { bindings, init, .. } = &chunk.statements[0] else {
            panic!("expected local declaration");
        };
        assert_eq!(bindings.len(), 3);
        assert_eq!(init.len(), 2);
        assert!(bindings[1].annotation.is_some());
    }

    #[test]
    fn parses_assignment_statements() {
        let chunk = parse_source("local a = 1\nlocal b = 2\na, b = b, a\nreturn a").unwrap();
        let Stmt::Assignment {
            targets, values, ..
        } = &chunk.statements[2]
        else {
            panic!("expected assignment");
        };
        assert_eq!(targets.len(), 2);
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn parses_table_assignment_targets() {
        let chunk = parse_source("t[2] = 10\nt.a = 11").unwrap();
        let Stmt::Assignment { targets, .. } = &chunk.statements[0] else {
            panic!("expected index assignment");
        };
        assert!(matches!(
            &targets[0],
            AssignmentTarget::Index {
                object,
                index,
                ..
            } if matches!(object.as_ref(), Expr::Name(name, _) if name == "t")
                && matches!(index.as_ref(), Expr::Number(2.0, _))
        ));

        let Stmt::Assignment { targets, .. } = &chunk.statements[1] else {
            panic!("expected field assignment");
        };
        assert!(matches!(
            &targets[0],
            AssignmentTarget::Field {
                object,
                field,
                ..
            } if matches!(object.as_ref(), Expr::Name(name, _) if name == "t") && field == "a"
        ));
    }

    #[test]
    fn parses_compound_assignment_statements() {
        let chunk = parse_source("local a = 1\na += 2").unwrap();
        let Stmt::Assignment {
            targets,
            op,
            values,
            ..
        } = &chunk.statements[1]
        else {
            panic!("expected assignment");
        };
        assert_eq!(targets.len(), 1);
        assert_eq!(*op, AssignmentOp::Add);
        assert_eq!(values.len(), 1);
    }

    #[test]
    fn parses_if_then_else_blocks() {
        let chunk = parse_source("local a = 0\nif a then a = 1 else a = 2 end\nreturn a").unwrap();
        let Stmt::If {
            then_body,
            else_body,
            ..
        } = &chunk.statements[1]
        else {
            panic!("expected if statement");
        };
        assert_eq!(then_body.len(), 1);
        assert_eq!(else_body.len(), 1);
    }

    #[test]
    fn parses_elseif_as_nested_else_branch() {
        let chunk = parse_source("if a then x = 1 elseif b then x = 2 else x = 3 end").unwrap();
        let Stmt::If {
            then_body,
            else_body,
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected if statement");
        };
        assert_eq!(then_body.len(), 1);
        assert_eq!(else_body.len(), 1);
        let Stmt::If {
            then_body,
            else_body,
            ..
        } = &else_body[0]
        else {
            panic!("expected elseif to lower to nested if");
        };
        assert_eq!(then_body.len(), 1);
        assert_eq!(else_body.len(), 1);
    }

    #[test]
    fn parses_if_else_expressions() {
        let chunk = parse_source("local x = if a then 1 else 2").unwrap();
        let Stmt::Local { init, .. } = &chunk.statements[0] else {
            panic!("expected local statement");
        };
        let Expr::IfElse {
            condition,
            then_expr,
            else_expr,
            ..
        } = &init[0]
        else {
            panic!("expected if expression");
        };
        assert!(matches!(condition.as_ref(), Expr::Name(name, _) if name == "a"));
        assert!(matches!(then_expr.as_ref(), Expr::Number(1.0, _)));
        assert!(matches!(else_expr.as_ref(), Expr::Number(2.0, _)));
    }

    #[test]
    fn parses_if_expression_elseif_as_nested_else_expression() {
        let chunk = parse_source("return if a then 1 elseif b then 2 else 3").unwrap();
        let Stmt::Return { value, .. } = &chunk.statements[0] else {
            panic!("expected return statement");
        };
        let Expr::IfElse { else_expr, .. } = value.as_ref().expect("expected value") else {
            panic!("expected if expression");
        };
        assert!(matches!(else_expr.as_ref(), Expr::IfElse { .. }));
    }

    #[test]
    fn rejects_if_expression_without_else() {
        let error = parse_source("local x = if true then 1").unwrap_err();
        assert!(error.to_string().contains("expected 'else'"));
    }

    #[test]
    fn return_is_final_inside_if_blocks() {
        let error = parse_source("if true then return 1 local x = 2 end").unwrap_err();
        assert!(error.to_string().contains("return must be the final"));
    }

    #[test]
    fn parses_while_repeat_and_break_blocks() {
        let chunk = parse_source(
            "while true do if false then break else end end\nrepeat local a = 1 until a == 1",
        )
        .unwrap();
        let Stmt::While { body, .. } = &chunk.statements[0] else {
            panic!("expected while statement");
        };
        assert!(matches!(body[0], Stmt::If { .. }));
        let Stmt::Repeat {
            body, condition, ..
        } = &chunk.statements[1]
        else {
            panic!("expected repeat statement");
        };
        assert!(matches!(body[0], Stmt::Local { .. }));
        assert!(matches!(
            condition,
            Expr::Binary {
                op: BinaryOp::Eq,
                ..
            }
        ));
    }

    #[test]
    fn parses_continue_statement_contextually() {
        let chunk = parse_source("while true do continue end").unwrap();
        let Stmt::While { body, .. } = &chunk.statements[0] else {
            panic!("expected while statement");
        };
        assert!(matches!(body[0], Stmt::Continue { .. }));

        let chunk = parse_source("local continue = 1\ncontinue = continue + 1").unwrap();
        assert!(matches!(chunk.statements[0], Stmt::Local { .. }));
        assert!(matches!(chunk.statements[1], Stmt::Assignment { .. }));
    }

    #[test]
    fn break_is_final_inside_blocks() {
        let error = parse_source("while true do break local x = 2 end").unwrap_err();
        assert!(error.to_string().contains("break must be the final"));
    }

    #[test]
    fn parses_scoped_do_blocks() {
        let chunk = parse_source("while true do do break end end").unwrap();
        let Stmt::While { body, .. } = &chunk.statements[0] else {
            panic!("expected while statement");
        };
        let Stmt::Do { body, .. } = &body[0] else {
            panic!("expected do block");
        };
        assert!(matches!(body[0], Stmt::Break { .. }));
    }

    #[test]
    fn typed_for_bindings_validate_annotations_before_entering_scope() {
        for source in [
            "for i:(number)=1,2 do end",
            "for i:typeof(unknown()),v: number in ipairs(t) do end",
            "type function f(y, xs) for i:typeof(y)=1,2 do y=i end; for k:string,v:typeof(y) in pairs(xs) do y=v end end",
        ] {
            assert!(parse_source(source).is_ok(), "{source}");
        }
        for source in [
            "for i:(number,string)=1,1 do end",
            "for i:...number=1,1 do end",
            "for i:number,v:(string,number) in pairs({}) do end",
        ] {
            assert!(parse_source(source).is_err(), "{source}");
        }
        for source in [
            "local outer; type function f() for i:typeof(outer)=1,2 do end end",
            "local outer; type function f() for outer:number,v:typeof(outer) in pairs({}) do end end",
        ] {
            let Err(super::SourceError::Parse(error)) = parse_source(source) else {
                panic!("expected a capture error for {source}");
            };
            assert_eq!(error.message, "Type function cannot reference outer local 'outer'", "{source}");
        }
    }

    #[test]
    fn parses_numeric_for_loops() {
        let chunk = parse_source("for i = 1, 9, 2 do break end").unwrap();
        let Stmt::ForNumeric {
            variable,
            start,
            limit,
            step: Some(step),
            body,
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected numeric for statement");
        };
        assert_eq!(variable, "i");
        assert!(matches!(start, Expr::Number(1.0, _)));
        assert!(matches!(limit, Expr::Number(9.0, _)));
        assert!(matches!(step, Expr::Number(2.0, _)));
        assert!(matches!(body[0], Stmt::Break { .. }));

        let chunk = parse_source("for i = 1, 9 do end").unwrap();
        assert!(matches!(
            &chunk.statements[0],
            Stmt::ForNumeric { step: None, .. }
        ));
    }

    #[test]
    fn parses_generic_for_loops_and_array_table_literals() {
        let chunk = parse_source("for k, v in ipairs({5, 6, nil}) do local x = v end").unwrap();
        let Stmt::ForGeneric {
            variables,
            iter,
            body,
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected generic for statement");
        };
        assert_eq!(variables.len(), 2);
        assert_eq!(variables[0].name, "k");
        assert_eq!(variables[1].name, "v");
        assert_eq!(iter.len(), 1);
        let Expr::Call { args, .. } = &iter[0] else {
            panic!("expected ipairs call");
        };
        assert_eq!(args.len(), 1);
        assert!(matches!(&args[0], Expr::Table { fields, .. } if fields.len() == 3));
        assert!(matches!(body[0], Stmt::Local { .. }));
    }

    #[test]
    fn parses_named_and_computed_table_fields() {
        let chunk = parse_source("return {data = 4, [1 + 2] = 5, 6}").unwrap();
        let Stmt::Return {
            value: Some(Expr::Table { fields, .. }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected table literal return");
        };
        assert!(matches!(
            &fields[0],
            TableField::Named {
                name,
                value: Expr::Number(4.0, _),
                ..
            } if name == "data"
        ));
        assert!(matches!(
            &fields[1],
            TableField::Keyed {
                key: Expr::Binary {
                    op: BinaryOp::Add,
                    ..
                },
                value: Expr::Number(5.0, _),
                ..
            }
        ));
        assert!(matches!(
            &fields[2],
            TableField::Array {
                value: Expr::Number(6.0, _),
                ..
            }
        ));
    }

    #[test]
    fn parses_field_access_expressions() {
        let chunk = parse_source("return math.huge").unwrap();
        let Stmt::Return {
            value: Some(Expr::Field { object, field, .. }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected field access return");
        };
        assert!(matches!(object.as_ref(), Expr::Name(name, _) if name == "math"));
        assert_eq!(field, "huge");
    }

    #[test]
    fn parses_equality_below_arithmetic_precedence() {
        let chunk = parse_source("return 1 + 2 == 3").unwrap();
        let Stmt::Return {
            value:
                Some(Expr::Binary {
                    op: BinaryOp::Eq,
                    lhs,
                    rhs,
                    ..
                }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected equality return");
        };
        assert!(matches!(
            **lhs,
            Expr::Binary {
                op: BinaryOp::Add,
                ..
            }
        ));
        assert!(matches!(**rhs, Expr::Number(3.0, _)));
    }

    #[test]
    fn parses_relational_operators_below_arithmetic_precedence() {
        let chunk = parse_source("return 1 + 1 <= 2").unwrap();
        let Stmt::Return {
            value:
                Some(Expr::Binary {
                    op: BinaryOp::Le,
                    lhs,
                    rhs,
                    ..
                }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected relational return");
        };
        assert!(matches!(
            **lhs,
            Expr::Binary {
                op: BinaryOp::Add,
                ..
            }
        ));
        assert!(matches!(**rhs, Expr::Number(2.0, _)));
    }

    #[test]
    fn parses_and_or_below_comparison_precedence() {
        let chunk = parse_source("return 1 < 2 and 3 or 4").unwrap();
        let Stmt::Return {
            value:
                Some(Expr::Binary {
                    op: BinaryOp::Or,
                    lhs,
                    rhs,
                    ..
                }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected logical or return");
        };
        assert!(matches!(
            **lhs,
            Expr::Binary {
                op: BinaryOp::And,
                ..
            }
        ));
        assert!(matches!(**rhs, Expr::Number(4.0, _)));
    }

    #[test]
    fn parses_floor_division_with_multiplicative_precedence() {
        let chunk = parse_source("return 1 + 3 // 2").unwrap();
        let Stmt::Return {
            value:
                Some(Expr::Binary {
                    op: BinaryOp::Add,
                    rhs,
                    ..
                }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected additive return");
        };
        assert!(matches!(
            **rhs,
            Expr::Binary {
                op: BinaryOp::FloorDiv,
                ..
            }
        ));
    }

    #[test]
    fn parses_concat_between_comparison_and_addition_precedence() {
        let chunk = parse_source("return 'a' .. 1 + 2 == 'a3'").unwrap();
        let Stmt::Return {
            value:
                Some(Expr::Binary {
                    op: BinaryOp::Eq,
                    lhs,
                    ..
                }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected equality return");
        };
        let Expr::Binary {
            op: BinaryOp::Concat,
            rhs,
            ..
        } = lhs.as_ref()
        else {
            panic!("expected concat expression");
        };
        assert!(matches!(
            rhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::Add,
                ..
            }
        ));
    }

    #[test]
    fn parses_concat_as_right_associative() {
        let chunk = parse_source("return 'a' .. 'b' .. 'c'").unwrap();
        let Stmt::Return {
            value:
                Some(Expr::Binary {
                    op: BinaryOp::Concat,
                    rhs,
                    ..
                }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected concat return");
        };
        assert!(matches!(
            rhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::Concat,
                ..
            }
        ));
    }

    #[test]
    fn parses_power_as_right_associative_above_unary() {
        let chunk = parse_source("return -2 ^ 2 ^ 3").unwrap();
        let Stmt::Return {
            value:
                Some(Expr::Unary {
                    op: UnaryOp::Neg,
                    expr,
                    ..
                }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected unary negated power expression");
        };
        let Expr::Binary {
            op: BinaryOp::Pow,
            rhs,
            ..
        } = expr.as_ref()
        else {
            panic!("expected power expression under unary minus");
        };
        assert!(matches!(
            rhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::Pow,
                ..
            }
        ));
    }

    #[test]
    fn parses_not_as_unary_expression() {
        let chunk = parse_source("return not false").unwrap();
        let Stmt::Return {
            value:
                Some(Expr::Unary {
                    op: UnaryOp::Not,
                    expr,
                    ..
                }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected unary not return");
        };
        assert!(matches!(**expr, Expr::Bool(false, _)));
    }

    #[test]
    fn parses_length_and_index_expressions() {
        let chunk = parse_source("return #t[1]").unwrap();
        let Stmt::Return {
            value:
                Some(Expr::Unary {
                    op: UnaryOp::Len,
                    expr,
                    ..
                }),
            ..
        } = &chunk.statements[0]
        else {
            panic!("expected unary length return");
        };
        assert!(matches!(
            expr.as_ref(),
            Expr::Index {
                object,
                index,
                ..
            } if matches!(object.as_ref(), Expr::Name(name, _) if name == "t")
                && matches!(index.as_ref(), Expr::Number(1.0, _))
        ));
    }

    #[test]
    fn returns_error_instead_of_recursing_forever() {
        let mut source = String::from("local x: ");
        for _ in 0..600 {
            source.push_str("Box<");
        }
        source.push_str("number");
        for _ in 0..600 {
            source.push('>');
        }
        let error = parse_source(&source).unwrap_err();
        assert!(error.to_string().contains("recursion limit"));
    }
}
