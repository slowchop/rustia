use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    #[inline]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Eof,
    Ident(String),
    Number(f64),
    String(Vec<u8>),
    Local,
    Return,
    Function,
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
    Not,
    And,
    Or,
    True,
    False,
    Nil,
    Type,
    Plus,
    PlusEqual,
    Minus,
    MinusEqual,
    Star,
    StarEqual,
    Slash,
    SlashEqual,
    FloorDiv,
    FloorDivEqual,
    Percent,
    PercentEqual,
    Caret,
    CaretEqual,
    Equal,
    EqualEqual,
    TildeEqual,
    LessEqual,
    GreaterEqual,
    Comma,
    Semicolon,
    Colon,
    Dot,
    Hash,
    Concat,
    ConcatEqual,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LexError {
    pub span: Span,
    pub message: String,
}

impl fmt::Display for LexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "lex error at {}..{}: {}",
            self.span.start, self.span.end, self.message
        )
    }
}

impl std::error::Error for LexError {}

pub type Result<T> = std::result::Result<T, LexError>;

pub fn lex(source: &str) -> Result<Vec<Token>> {
    lex_bytes(source.as_bytes())
}

pub fn lex_bytes(source: &[u8]) -> Result<Vec<Token>> {
    Lexer::new(source).lex_all()
}

struct Lexer<'a> {
    bytes: &'a [u8],
    cursor: usize,
    tokens: Vec<Token>,
}

impl<'a> Lexer<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            cursor: 0,
            tokens: Vec::new(),
        }
    }

    fn lex_all(mut self) -> Result<Vec<Token>> {
        while let Some(byte) = self.peek() {
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c => {
                    self.cursor += 1;
                }
                b'0'..=b'9' => self.lex_number()?,
                b'.' if self.peek_next().is_some_and(|b| b.is_ascii_digit()) => {
                    self.lex_number()?
                }
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.lex_ident_or_keyword(),
                b'"' | b'\'' => self.lex_string(byte)?,
                b'+' => self.single_or_equal(TokenKind::Plus, TokenKind::PlusEqual),
                b'*' => self.single_or_equal(TokenKind::Star, TokenKind::StarEqual),
                b'/' => self.lex_slash(),
                b'%' => self.single_or_equal(TokenKind::Percent, TokenKind::PercentEqual),
                b'^' => self.single_or_equal(TokenKind::Caret, TokenKind::CaretEqual),
                b'=' => self.lex_equal(),
                b'~' => self.lex_tilde_equal()?,
                b',' => self.single(TokenKind::Comma),
                b';' => self.single(TokenKind::Semicolon),
                b':' => self.single(TokenKind::Colon),
                b'.' => self.lex_dot(),
                b'#' => self.single(TokenKind::Hash),
                b'(' => self.single(TokenKind::LParen),
                b')' => self.single(TokenKind::RParen),
                b'{' => self.single(TokenKind::LBrace),
                b'}' => self.single(TokenKind::RBrace),
                b'[' => self.lex_left_bracket()?,
                b']' => self.single(TokenKind::RBracket),
                b'<' => self.lex_less(),
                b'>' => self.lex_greater(),
                b'|' => self.single(TokenKind::Pipe),
                b'&' => self.single(TokenKind::Ampersand),
                b'?' => self.single(TokenKind::Question),
                b'-' => self.lex_minus_or_comment()?,
                _ => {
                    let span = Span::new(self.cursor, self.cursor + 1);
                    return Err(LexError {
                        span,
                        message: format!("unexpected byte 0x{byte:02x}"),
                    });
                }
            }
        }
        self.tokens.push(Token {
            kind: TokenKind::Eof,
            span: Span::new(self.cursor, self.cursor),
        });
        Ok(self.tokens)
    }

    fn lex_number(&mut self) -> Result<()> {
        let start = self.cursor;
        if self.peek() == Some(b'0') && matches!(self.peek_next(), Some(b'x' | b'X')) {
            self.cursor += 2;
            let digits_start = self.cursor;
            let digit_count = self.take_radix_digits(|byte| byte.is_ascii_hexdigit());
            if digit_count == 0 {
                return Err(LexError {
                    span: Span::new(start, self.cursor),
                    message: "expected hexadecimal digits".to_owned(),
                });
            }
            let text = &self.bytes[digits_start..self.cursor];
            self.reject_radix_number_continuation(start, "hexadecimal")?;
            let value = wrapping_radix_value(text, 16) as f64;
            self.tokens.push(Token {
                kind: TokenKind::Number(value),
                span: Span::new(start, self.cursor),
            });
            return Ok(());
        }
        if self.peek() == Some(b'0') && matches!(self.peek_next(), Some(b'b' | b'B')) {
            self.cursor += 2;
            let digits_start = self.cursor;
            let digit_count = self.take_radix_digits(|byte| matches!(byte, b'0' | b'1'));
            if digit_count == 0 {
                return Err(LexError {
                    span: Span::new(start, self.cursor),
                    message: "expected binary digits".to_owned(),
                });
            }
            let text = &self.bytes[digits_start..self.cursor];
            self.reject_radix_number_continuation(start, "binary")?;
            let value = wrapping_radix_value(text, 2) as f64;
            self.tokens.push(Token {
                kind: TokenKind::Number(value),
                span: Span::new(start, self.cursor),
            });
            return Ok(());
        }

        let mut fraction_digits = 0;
        if self.peek() == Some(b'.') {
            self.cursor += 1;
            fraction_digits = self.take_radix_digits(|byte| byte.is_ascii_digit());
        } else {
            self.take_radix_digits(|byte| byte.is_ascii_digit());
            if self.peek() == Some(b'.') {
                self.cursor += 1;
                fraction_digits = self.take_radix_digits(|byte| byte.is_ascii_digit());
            }
        }

        let mut has_exponent = false;
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let exponent = self.cursor;
            self.cursor += 1;
            has_exponent = true;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.cursor += 1;
            }
            let digits_start = self.cursor;
            let digit_count = self.take_radix_digits(|byte| byte.is_ascii_digit());
            if digit_count == 0 {
                return Err(LexError {
                    span: Span::new(exponent, self.cursor),
                    message: "expected exponent digits".to_owned(),
                });
            }
            debug_assert!(self.cursor > digits_start);
        }

        self.reject_decimal_number_continuation(start, fraction_digits > 0, has_exponent)?;
        let text = &self.bytes[start..self.cursor];
        let digits = number_text_without_separators(text);
        let value = digits.parse::<f64>().map_err(|_| LexError {
            span: Span::new(start, self.cursor),
            message: format!("invalid number literal {text:?}"),
        })?;
        self.tokens.push(Token {
            kind: TokenKind::Number(value),
            span: Span::new(start, self.cursor),
        });
        Ok(())
    }

    fn reject_radix_number_continuation(&self, start: usize, radix_name: &str) -> Result<()> {
        let Some(byte) = self.peek() else {
            return Ok(());
        };
        let invalid = byte.is_ascii_alphanumeric()
            || byte == b'_'
            || (byte == b'.' && self.peek_next() != Some(b'.'));
        if invalid {
            return Err(LexError {
                span: Span::new(start, self.cursor + 1),
                message: format!("invalid {radix_name} number literal"),
            });
        }
        Ok(())
    }

    fn reject_decimal_number_continuation(
        &self,
        start: usize,
        has_fraction_digits: bool,
        has_exponent: bool,
    ) -> Result<()> {
        let Some(byte) = self.peek() else {
            return Ok(());
        };
        let invalid = byte.is_ascii_alphabetic()
            || byte == b'_'
            || (byte == b'.'
                && self.peek_next().is_some_and(|next| next.is_ascii_digit())
                && (has_fraction_digits || has_exponent));
        if invalid {
            return Err(LexError {
                span: Span::new(start, self.cursor + 1),
                message: "invalid number literal".to_owned(),
            });
        }
        Ok(())
    }

    fn lex_ident_or_keyword(&mut self) {
        let start = self.cursor;
        self.take_while(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        let text = &self.bytes[start..self.cursor];
        let kind = match text {
            b"local" => TokenKind::Local,
            b"return" => TokenKind::Return,
            b"function" => TokenKind::Function,
            b"if" => TokenKind::If,
            b"then" => TokenKind::Then,
            b"elseif" => TokenKind::ElseIf,
            b"else" => TokenKind::Else,
            b"end" => TokenKind::End,
            b"while" => TokenKind::While,
            b"do" => TokenKind::Do,
            b"for" => TokenKind::For,
            b"in" => TokenKind::In,
            b"repeat" => TokenKind::Repeat,
            b"until" => TokenKind::Until,
            b"break" => TokenKind::Break,
            b"not" => TokenKind::Not,
            b"and" => TokenKind::And,
            b"or" => TokenKind::Or,
            b"true" => TokenKind::True,
            b"false" => TokenKind::False,
            b"nil" => TokenKind::Nil,
            b"type" => TokenKind::Type,
            _ => TokenKind::Ident(ascii_identifier(text)),
        };
        self.tokens.push(Token {
            kind,
            span: Span::new(start, self.cursor),
        });
    }

    fn lex_string(&mut self, quote: u8) -> Result<()> {
        let start = self.cursor;
        self.cursor += 1;
        let mut value = Vec::new();
        while let Some(byte) = self.peek() {
            match byte {
                b if b == quote => {
                    self.cursor += 1;
                    self.tokens.push(Token {
                        kind: TokenKind::String(value),
                        span: Span::new(start, self.cursor),
                    });
                    return Ok(());
                }
                b'\\' => {
                    self.cursor += 1;
                    let escaped = self.peek().ok_or_else(|| LexError {
                        span: Span::new(start, self.cursor),
                        message: "unterminated escape sequence".to_owned(),
                    })?;
                    if escaped.is_ascii_digit() {
                        value.push(self.lex_decimal_string_escape(start)?);
                    } else {
                        match escaped {
                            b'\n' | b'\r' => {
                                self.take_newline();
                                value.push(b'\n');
                            }
                            b'a' => {
                                self.cursor += 1;
                                value.push(0x07);
                            }
                            b'b' => {
                                self.cursor += 1;
                                value.push(0x08);
                            }
                            b'f' => {
                                self.cursor += 1;
                                value.push(0x0c);
                            }
                            b'n' => {
                                self.cursor += 1;
                                value.push(b'\n');
                            }
                            b't' => {
                                self.cursor += 1;
                                value.push(b'\t');
                            }
                            b'r' => {
                                self.cursor += 1;
                                value.push(b'\r');
                            }
                            b'v' => {
                                self.cursor += 1;
                                value.push(0x0b);
                            }
                            b'\\' => {
                                self.cursor += 1;
                                value.push(b'\\');
                            }
                            b'"' => {
                                self.cursor += 1;
                                value.push(b'"');
                            }
                            b'\'' => {
                                self.cursor += 1;
                                value.push(b'\'');
                            }
                            b'`' => {
                                self.cursor += 1;
                                value.push(b'`');
                            }
                            b'x' => {
                                self.cursor += 1;
                                value.push(self.lex_hex_string_escape(start)?);
                            }
                            b'u' => {
                                self.cursor += 1;
                                value.extend_from_slice(&self.lex_unicode_string_escape(start)?);
                            }
                            b'z' => {
                                self.cursor += 1;
                                self.take_while(|byte| {
                                    matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c)
                                });
                            }
                            other => {
                                return Err(LexError {
                                    span: Span::new(start, self.cursor),
                                    message: format!(
                                        "invalid escape sequence \\{}",
                                        char::from(other)
                                    ),
                                });
                            }
                        }
                    }
                }
                b'\n' | b'\r' => {
                    return Err(LexError {
                        span: Span::new(start, self.cursor),
                        message: "unfinished string near newline".to_owned(),
                    });
                }
                byte => {
                    value.push(byte);
                    self.cursor += 1;
                }
            }
        }
        Err(LexError {
            span: Span::new(start, self.cursor),
            message: "unterminated string literal".to_owned(),
        })
    }

    fn lex_decimal_string_escape(&mut self, string_start: usize) -> Result<u8> {
        let mut value = 0_u32;
        let mut digits = 0;
        while digits < 3 {
            let Some(byte) = self.peek() else {
                break;
            };
            if !byte.is_ascii_digit() {
                break;
            }
            value = value * 10 + u32::from(byte - b'0');
            self.cursor += 1;
            digits += 1;
        }
        if value > 0xff {
            return Err(LexError {
                span: Span::new(string_start, self.cursor),
                message: "decimal string escape exceeds 255".to_owned(),
            });
        }
        Ok(value as u8)
    }

    fn lex_hex_string_escape(&mut self, string_start: usize) -> Result<u8> {
        let high = self.peek().ok_or_else(|| LexError {
            span: Span::new(string_start, self.cursor),
            message: "expected two hexadecimal digits after \\x".to_owned(),
        })?;
        self.cursor += 1;
        let low = self.peek().ok_or_else(|| LexError {
            span: Span::new(string_start, self.cursor),
            message: "expected two hexadecimal digits after \\x".to_owned(),
        })?;
        self.cursor += 1;
        let Some(high) = hex_value(high) else {
            return Err(LexError {
                span: Span::new(string_start, self.cursor),
                message: "expected two hexadecimal digits after \\x".to_owned(),
            });
        };
        let Some(low) = hex_value(low) else {
            return Err(LexError {
                span: Span::new(string_start, self.cursor),
                message: "expected two hexadecimal digits after \\x".to_owned(),
            });
        };
        Ok((high << 4) | low)
    }

    fn lex_unicode_string_escape(&mut self, string_start: usize) -> Result<Vec<u8>> {
        if self.peek() != Some(b'{') {
            return Err(LexError {
                span: Span::new(string_start, self.cursor),
                message: "expected '{' after \\u".to_owned(),
            });
        }
        self.cursor += 1;

        let mut value = 0_u128;
        let mut digits = 0_usize;
        while digits < 16 {
            let Some(byte) = self.peek() else {
                break;
            };
            let Some(digit) = hex_value(byte) else {
                break;
            };
            value = value * 16 + u128::from(digit);
            self.cursor += 1;
            digits += 1;
        }

        if digits == 0 || self.peek() != Some(b'}') {
            return Err(LexError {
                span: Span::new(string_start, self.cursor),
                message: "expected hexadecimal codepoint in \\u{...}".to_owned(),
            });
        }
        self.cursor += 1;

        if value > 0x10ffff {
            return Err(LexError {
                span: Span::new(string_start, self.cursor),
                message: "unicode escape exceeds 0x10FFFF".to_owned(),
            });
        }
        Ok(encode_unicode_escape(value as u32))
    }

    fn take_newline(&mut self) -> bool {
        match self.peek() {
            Some(b'\n') => {
                self.cursor += 1;
                if self.peek() == Some(b'\r') {
                    self.cursor += 1;
                }
                true
            }
            Some(b'\r') => {
                self.cursor += 1;
                if self.peek() == Some(b'\n') {
                    self.cursor += 1;
                }
                true
            }
            _ => false,
        }
    }

    fn lex_left_bracket(&mut self) -> Result<()> {
        let start = self.cursor;
        if let Some((level, content_start)) = self.long_bracket_opener_at(start) {
            let (content_start, value) = self.read_long_bracket(start, level, content_start)?;
            self.tokens.push(Token {
                kind: TokenKind::String(value),
                span: Span::new(start, self.cursor),
            });
            debug_assert!(content_start <= self.cursor);
        } else {
            self.single(TokenKind::LBracket);
        }
        Ok(())
    }

    fn read_long_bracket(
        &mut self,
        opener_start: usize,
        level: usize,
        mut content_start: usize,
    ) -> Result<(usize, Vec<u8>)> {
        self.cursor = content_start;
        if let Some(newline_end) = self.newline_end_at(self.cursor) {
            self.cursor = newline_end;
            content_start = newline_end;
        }
        let content_start = content_start;
        while self.cursor < self.bytes.len() {
            if self.long_bracket_closer_matches(level) {
                let content_end = self.cursor;
                self.cursor += level + 2;
                return Ok((
                    content_start,
                    self.bytes[content_start..content_end].to_vec(),
                ));
            }
            self.cursor += 1;
        }
        Err(LexError {
            span: Span::new(opener_start, self.cursor),
            message: "unterminated long string".to_owned(),
        })
    }

    fn lex_dot(&mut self) {
        let start = self.cursor;
        self.cursor += 1;
        if self.peek() == Some(b'.') {
            self.cursor += 1;
            let kind = if self.peek() == Some(b'.') {
                self.cursor += 1;
                TokenKind::Ellipsis
            } else if self.peek() == Some(b'=') {
                self.cursor += 1;
                TokenKind::ConcatEqual
            } else {
                TokenKind::Concat
            };
            self.tokens.push(Token {
                kind,
                span: Span::new(start, self.cursor),
            });
        } else {
            self.tokens.push(Token {
                kind: TokenKind::Dot,
                span: Span::new(start, self.cursor),
            });
        }
    }

    fn lex_equal(&mut self) {
        let start = self.cursor;
        self.cursor += 1;
        if self.peek() == Some(b'=') {
            self.cursor += 1;
            self.tokens.push(Token {
                kind: TokenKind::EqualEqual,
                span: Span::new(start, self.cursor),
            });
        } else {
            self.tokens.push(Token {
                kind: TokenKind::Equal,
                span: Span::new(start, self.cursor),
            });
        }
    }

    fn lex_tilde_equal(&mut self) -> Result<()> {
        let start = self.cursor;
        self.cursor += 1;
        if self.peek() == Some(b'=') {
            self.cursor += 1;
            self.tokens.push(Token {
                kind: TokenKind::TildeEqual,
                span: Span::new(start, self.cursor),
            });
            Ok(())
        } else {
            Err(LexError {
                span: Span::new(start, self.cursor),
                message: "expected '=' after '~'".to_owned(),
            })
        }
    }

    fn lex_slash(&mut self) {
        let start = self.cursor;
        self.cursor += 1;
        if self.peek() == Some(b'/') {
            self.cursor += 1;
            let kind = if self.peek() == Some(b'=') {
                self.cursor += 1;
                TokenKind::FloorDivEqual
            } else {
                TokenKind::FloorDiv
            };
            self.tokens.push(Token {
                kind,
                span: Span::new(start, self.cursor),
            });
        } else if self.peek() == Some(b'=') {
            self.cursor += 1;
            self.tokens.push(Token {
                kind: TokenKind::SlashEqual,
                span: Span::new(start, self.cursor),
            });
        } else {
            self.tokens.push(Token {
                kind: TokenKind::Slash,
                span: Span::new(start, self.cursor),
            });
        }
    }

    fn lex_less(&mut self) {
        let start = self.cursor;
        self.cursor += 1;
        let kind = if self.peek() == Some(b'=') {
            self.cursor += 1;
            TokenKind::LessEqual
        } else {
            TokenKind::Less
        };
        self.tokens.push(Token {
            kind,
            span: Span::new(start, self.cursor),
        });
    }

    fn lex_greater(&mut self) {
        let start = self.cursor;
        self.cursor += 1;
        let kind = if self.peek() == Some(b'=') {
            self.cursor += 1;
            TokenKind::GreaterEqual
        } else {
            TokenKind::Greater
        };
        self.tokens.push(Token {
            kind,
            span: Span::new(start, self.cursor),
        });
    }

    fn lex_minus_or_comment(&mut self) -> Result<()> {
        let start = self.cursor;
        self.cursor += 1;
        if self.peek() == Some(b'-') {
            self.cursor += 1;
            if let Some((level, content_start)) = self.long_bracket_opener_at(self.cursor) {
                self.read_long_bracket(start, level, content_start)?;
            } else {
                while let Some(byte) = self.peek() {
                    if matches!(byte, b'\n' | b'\r') {
                        break;
                    }
                    self.cursor += 1;
                }
            }
        } else if self.peek() == Some(b'=') {
            self.cursor += 1;
            self.tokens.push(Token {
                kind: TokenKind::MinusEqual,
                span: Span::new(start, self.cursor),
            });
        } else if self.peek() == Some(b'>') {
            self.cursor += 1;
            self.tokens.push(Token {
                kind: TokenKind::Arrow,
                span: Span::new(start, self.cursor),
            });
        } else {
            self.tokens.push(Token {
                kind: TokenKind::Minus,
                span: Span::new(start, self.cursor),
            });
        }
        Ok(())
    }

    fn single_or_equal(&mut self, bare: TokenKind, with_equal: TokenKind) {
        let start = self.cursor;
        self.cursor += 1;
        let kind = if self.peek() == Some(b'=') {
            self.cursor += 1;
            with_equal
        } else {
            bare
        };
        self.tokens.push(Token {
            kind,
            span: Span::new(start, self.cursor),
        });
    }

    fn single(&mut self, kind: TokenKind) {
        let start = self.cursor;
        self.cursor += 1;
        self.tokens.push(Token {
            kind,
            span: Span::new(start, self.cursor),
        });
    }

    fn take_while(&mut self, mut predicate: impl FnMut(u8) -> bool) {
        while let Some(byte) = self.peek() {
            if !predicate(byte) {
                break;
            }
            self.cursor += 1;
        }
    }

    fn take_radix_digits(&mut self, mut is_digit: impl FnMut(u8) -> bool) -> usize {
        let mut digits = 0;
        while let Some(byte) = self.peek() {
            if is_digit(byte) {
                self.cursor += 1;
                digits += 1;
            } else if byte == b'_' {
                self.cursor += 1;
            } else {
                break;
            }
        }
        digits
    }

    #[inline]
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.cursor).copied()
    }

    #[inline]
    fn peek_next(&self) -> Option<u8> {
        self.bytes.get(self.cursor + 1).copied()
    }

    fn long_bracket_opener_at(&self, start: usize) -> Option<(usize, usize)> {
        if self.bytes.get(start).copied() != Some(b'[') {
            return None;
        }
        let mut cursor = start + 1;
        while self.bytes.get(cursor).copied() == Some(b'=') {
            cursor += 1;
        }
        if self.bytes.get(cursor).copied() == Some(b'[') {
            Some((cursor - start - 1, cursor + 1))
        } else {
            None
        }
    }

    fn long_bracket_closer_matches(&self, level: usize) -> bool {
        if self.peek() != Some(b']') {
            return false;
        }
        let mut cursor = self.cursor + 1;
        for _ in 0..level {
            if self.bytes.get(cursor).copied() != Some(b'=') {
                return false;
            }
            cursor += 1;
        }
        self.bytes.get(cursor).copied() == Some(b']')
    }

    fn newline_end_at(&self, start: usize) -> Option<usize> {
        match (
            self.bytes.get(start).copied(),
            self.bytes.get(start + 1).copied(),
        ) {
            (Some(b'\r'), Some(b'\n')) | (Some(b'\n'), Some(b'\r')) => Some(start + 2),
            (Some(b'\r' | b'\n'), _) => Some(start + 1),
            _ => None,
        }
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

fn encode_unicode_escape(value: u32) -> Vec<u8> {
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

fn number_text_without_separators(text: &[u8]) -> String {
    text.iter()
        .copied()
        .filter(|byte| *byte != b'_')
        .map(char::from)
        .collect()
}

fn wrapping_radix_value(text: &[u8], radix: u32) -> u64 {
    let mut value = 0_u64;
    for byte in text.iter().copied().filter(|byte| *byte != b'_') {
        let digit = u64::from(
            char::from(byte)
                .to_digit(radix)
                .expect("scanner only accepts digits for this radix"),
        );
        value = value.wrapping_mul(u64::from(radix)).wrapping_add(digit);
    }
    value
}

fn ascii_identifier(text: &[u8]) -> String {
    text.iter().copied().map(char::from).collect()
}

#[cfg(test)]
mod tests {
    use super::{lex, lex_bytes, TokenKind};

    #[test]
    fn skips_comments_and_keeps_type_tokens() {
        let tokens = lex("local x: Foo<number>? = 1 -- comment\nreturn x").unwrap();
        let kinds: Vec<_> = tokens.into_iter().map(|token| token.kind).collect();
        assert!(matches!(kinds[0], TokenKind::Local));
        assert!(matches!(kinds[3], TokenKind::Ident(_)));
        assert!(matches!(kinds[4], TokenKind::Less));
        assert!(matches!(kinds[6], TokenKind::Greater));
        assert!(matches!(kinds[7], TokenKind::Question));
        assert!(matches!(kinds[10], TokenKind::Return));
    }

    #[test]
    fn lexes_common_luau_number_forms_and_utf8_strings() {
        let tokens = lex("0x10 1e3 .5 1. 'é'").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Number(16.0)));
        assert!(matches!(tokens[1].kind, TokenKind::Number(1000.0)));
        assert!(matches!(tokens[2].kind, TokenKind::Number(0.5)));
        assert!(matches!(tokens[3].kind, TokenKind::Number(1.0)));
        assert!(matches!(&tokens[4].kind, TokenKind::String(value) if value == "é".as_bytes()));
    }

    #[test]
    fn lexes_raw_non_utf8_string_bytes() {
        let tokens = lex_bytes(b"'a\xffb' [[c\xe1d]]").unwrap();
        assert!(matches!(&tokens[0].kind, TokenKind::String(value) if value == b"a\xffb"));
        assert!(matches!(&tokens[1].kind, TokenKind::String(value) if value == b"c\xe1d"));
    }

    #[test]
    fn lexes_luau_number_separators_and_binary_literals() {
        let tokens = lex("0x42_1234_5678 0b1010_0101 0b_0101 1_000.5_25 1e1_0").unwrap();
        assert!(
            matches!(tokens[0].kind, TokenKind::Number(value) if value == 0x42_1234_5678_u64 as f64)
        );
        assert!(matches!(tokens[1].kind, TokenKind::Number(165.0)));
        assert!(matches!(tokens[2].kind, TokenKind::Number(5.0)));
        assert!(matches!(tokens[3].kind, TokenKind::Number(1000.525)));
        assert!(matches!(
            tokens[4].kind,
            TokenKind::Number(10_000_000_000.0)
        ));
    }

    #[test]
    fn lexes_wrapping_radix_integer_literals() {
        let tokens = lex("0xffff_ffff_ffff_ffff 0x1_0000_0000_0000_0000 0b1_0000000000000000000000000000000000000000000000000000000000000000").unwrap();
        assert!(matches!(
            tokens[0].kind,
            TokenKind::Number(value) if value == u64::MAX as f64
        ));
        assert!(matches!(tokens[1].kind, TokenKind::Number(0.0)));
        assert!(matches!(tokens[2].kind, TokenKind::Number(0.0)));
    }

    #[test]
    fn rejects_malformed_number_literals() {
        for source in ["0b102", "0x1G", "0x1.8p4", "1.2.3", "1e"] {
            assert!(lex(source).is_err(), "{source:?} should reject");
        }
    }

    #[test]
    fn lexes_decimal_hex_and_z_string_escapes_as_bytes() {
        let tokens = lex("\"\\0\\000A\\65\\255\\xff\\z\n\tB\"").unwrap();
        assert!(
            matches!(&tokens[0].kind, TokenKind::String(value) if value.as_slice() == b"\0\0AA\xff\xffB")
        );

        let error = lex(r#""\256""#).unwrap_err();
        assert!(error.to_string().contains("exceeds 255"));
    }

    #[test]
    fn lexes_unicode_backtick_and_newline_string_escapes() {
        let tokens = lex("\"\\`\\u{41}\\u{1f600}\\u{d800}\\u{dfff}a\\\r\nb\"").unwrap();
        assert!(
            matches!(&tokens[0].kind, TokenKind::String(value) if value.as_slice() == b"`A\xf0\x9f\x98\x80\xed\xa0\x80\xed\xbf\xbfa\nb")
        );
    }

    #[test]
    fn rejects_invalid_short_string_escapes_and_raw_newlines() {
        for source in [
            "\"a\nb\"",
            "\"a\rb\"",
            r#""\q""#,
            r#""\u{}""#,
            r#""\u{110000}""#,
            r#""\xG0""#,
        ] {
            assert!(lex(source).is_err(), "{source:?} should reject");
        }
    }

    #[test]
    fn lexes_long_bracket_strings() {
        let tokens = lex("[[hello]] [=[a [[ nested-looking ]] value]=]").unwrap();
        assert!(matches!(&tokens[0].kind, TokenKind::String(value) if value == b"hello"));
        assert!(
            matches!(&tokens[1].kind, TokenKind::String(value) if value == b"a [[ nested-looking ]] value")
        );
    }

    #[test]
    fn lexes_long_bracket_string_with_first_newline_skip() {
        let tokens = lex("[[\nhello\n]]").unwrap();
        assert!(matches!(&tokens[0].kind, TokenKind::String(value) if value == b"hello\n"));
    }

    #[test]
    fn skips_long_comments_and_falls_back_for_malformed_openers() {
        let tokens = lex("local a = 1 --[[ hidden ]]\nreturn a --[== still line comment").unwrap();
        let kinds: Vec<_> = tokens.into_iter().map(|token| token.kind).collect();
        assert!(matches!(kinds[0], TokenKind::Local));
        assert!(matches!(kinds[4], TokenKind::Return));
        assert!(matches!(kinds[6], TokenKind::Eof));
    }

    #[test]
    fn lexes_equality_operators() {
        let tokens = lex("a == b ~= c").unwrap();
        assert!(matches!(tokens[1].kind, TokenKind::EqualEqual));
        assert!(matches!(tokens[3].kind, TokenKind::TildeEqual));
    }

    #[test]
    fn lexes_relational_operators() {
        let tokens = lex("a < b <= c > d >= e").unwrap();
        assert!(matches!(tokens[1].kind, TokenKind::Less));
        assert!(matches!(tokens[3].kind, TokenKind::LessEqual));
        assert!(matches!(tokens[5].kind, TokenKind::Greater));
        assert!(matches!(tokens[7].kind, TokenKind::GreaterEqual));
    }

    #[test]
    fn lexes_floor_division_and_power() {
        let tokens = lex("a // b ^ c / d").unwrap();
        assert!(matches!(tokens[1].kind, TokenKind::FloorDiv));
        assert!(matches!(tokens[3].kind, TokenKind::Caret));
        assert!(matches!(tokens[5].kind, TokenKind::Slash));
    }

    #[test]
    fn lexes_concat_and_ellipsis() {
        let tokens = lex("a .. b ... c").unwrap();
        assert!(matches!(tokens[1].kind, TokenKind::Concat));
        assert!(matches!(tokens[3].kind, TokenKind::Ellipsis));
    }

    #[test]
    fn lexes_compound_assignment_operators() {
        let tokens = lex("a += b -= c *= d /= e //= f %= g ^= h ..= i").unwrap();
        assert!(matches!(tokens[1].kind, TokenKind::PlusEqual));
        assert!(matches!(tokens[3].kind, TokenKind::MinusEqual));
        assert!(matches!(tokens[5].kind, TokenKind::StarEqual));
        assert!(matches!(tokens[7].kind, TokenKind::SlashEqual));
        assert!(matches!(tokens[9].kind, TokenKind::FloorDivEqual));
        assert!(matches!(tokens[11].kind, TokenKind::PercentEqual));
        assert!(matches!(tokens[13].kind, TokenKind::CaretEqual));
        assert!(matches!(tokens[15].kind, TokenKind::ConcatEqual));
    }

    #[test]
    fn lexes_not_keyword() {
        let tokens = lex("not notable").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Not));
        assert!(matches!(&tokens[1].kind, TokenKind::Ident(value) if value == "notable"));
    }

    #[test]
    fn lexes_and_or_keywords() {
        let tokens = lex("and or orderly").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::And));
        assert!(matches!(tokens[1].kind, TokenKind::Or));
        assert!(matches!(&tokens[2].kind, TokenKind::Ident(value) if value == "orderly"));
    }

    #[test]
    fn lexes_if_block_keywords() {
        let tokens = lex("if x then y elseif a then b else z end").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::If));
        assert!(matches!(tokens[2].kind, TokenKind::Then));
        assert!(matches!(tokens[4].kind, TokenKind::ElseIf));
        assert!(matches!(tokens[6].kind, TokenKind::Then));
        assert!(matches!(tokens[8].kind, TokenKind::Else));
        assert!(matches!(tokens[10].kind, TokenKind::End));
    }

    #[test]
    fn lexes_loop_keywords() {
        let tokens = lex("while x do break end repeat y until z for i in").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::While));
        assert!(matches!(tokens[2].kind, TokenKind::Do));
        assert!(matches!(tokens[3].kind, TokenKind::Break));
        assert!(matches!(tokens[4].kind, TokenKind::End));
        assert!(matches!(tokens[5].kind, TokenKind::Repeat));
        assert!(matches!(tokens[7].kind, TokenKind::Until));
        assert!(matches!(tokens[9].kind, TokenKind::For));
        assert!(matches!(tokens[11].kind, TokenKind::In));
    }

    #[test]
    fn lexes_length_operator_and_brackets() {
        let tokens = lex("#t[1]").unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Hash));
        assert!(matches!(tokens[2].kind, TokenKind::LBracket));
        assert!(matches!(tokens[4].kind, TokenKind::RBracket));
    }
}
