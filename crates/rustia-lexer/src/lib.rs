mod confusables;

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
    Invalid(String),
    Ident(String),
    Attribute(String),
    AttributeOpen,
    Number(f64),
    /// A later malformed numeral retained while resolving an earlier string error.
    MalformedNumber(String),
    String(Vec<u8>),
    /// Parser-facing token for a closed quote whose contents failed to decode.
    MalformedStringEscape(Vec<u8>),
    /// Parser-facing token for an unfinished quoted or long-bracket string.
    UnfinishedString,
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
    Lexer::new(source).lex_all(false)
}

/// Keeps malformed string, punctuation and Unicode tokens for contextual parser diagnostics.
/// A malformed string retains its source extent and any following tokens through EOF.
pub fn lex_for_parser(source: &[u8]) -> Result<Vec<Token>> {
    Lexer::new(source).lex_all(true)
}

struct Lexer<'a> {
    bytes: &'a [u8],
    cursor: usize,
    tokens: Vec<Token>,
    has_invalid_string: bool,
}

impl<'a> Lexer<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            cursor: 0,
            tokens: Vec::new(),
            has_invalid_string: false,
        }
    }

    fn lex_all(mut self, keep_invalid: bool) -> Result<Vec<Token>> {
        while let Some(byte) = self.peek() {
            if let Err(error) = self.lex_token(byte, keep_invalid) {
                if !self.has_invalid_string {
                    return Err(error);
                }
                // A previous string failure is pending. Keep both tokens so the
                // grammar can determine which error to report first.
                let raw = &self.bytes[error.span.start..error.span.end];
                let kind = if matches!(byte, b'0'..=b'9' | b'.') {
                    TokenKind::MalformedNumber(String::from_utf8_lossy(raw).into_owned())
                } else {
                    TokenKind::Invalid(format!("'{}'", String::from_utf8_lossy(raw)))
                };
                self.tokens.push(Token {
                    kind,
                    span: error.span,
                });
            }
        }
        self.tokens.push(Token {
            kind: TokenKind::Eof,
            span: Span::new(self.cursor, self.cursor),
        });
        Ok(self.tokens)
    }

    fn lex_token(&mut self, byte: u8, keep_invalid: bool) -> Result<()> {
        match byte {
            b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c => {
                self.cursor += 1;
            }
            b'0'..=b'9' => self.lex_number()?,
            b'.' if self.peek_next().is_some_and(|b| b.is_ascii_digit()) => self.lex_number()?,
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.lex_ident_or_keyword(),
            b'"' | b'\'' => {
                let start = self.cursor;
                if let Err(error) = self.lex_string(byte) {
                    if !keep_invalid {
                        return Err(error);
                    }
                    let token = self.recover_short_string(start, byte);
                    self.cursor = token.span.end;
                    self.has_invalid_string = true;
                    self.tokens.push(token);
                }
            }
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
            b'@' => self.lex_attribute(),
            b'(' => self.single(TokenKind::LParen),
            b')' => self.single(TokenKind::RParen),
            b'{' => self.single(TokenKind::LBrace),
            b'}' => self.single(TokenKind::RBrace),
            b'[' => {
                if let Err(error) = self.lex_left_bracket() {
                    if !keep_invalid {
                        return Err(error);
                    }
                    self.has_invalid_string = true;
                    self.tokens.push(Token {
                        kind: TokenKind::UnfinishedString,
                        span: error.span,
                    });
                }
            }
            b']' => self.single(TokenKind::RBracket),
            b'<' => self.lex_less(),
            b'>' => self.lex_greater(),
            b'|' => self.single(TokenKind::Pipe),
            b'&' => self.single(TokenKind::Ampersand),
            b'?' => self.single(TokenKind::Question),
            b'-' => self.lex_minus_or_comment(keep_invalid)?,
            _ if keep_invalid => self.lex_invalid(),
            _ => {
                let span = Span::new(self.cursor, self.cursor + 1);
                return Err(LexError {
                    span,
                    message: format!("unexpected byte 0x{byte:02x}"),
                });
            }
        }
        Ok(())
    }

    fn lex_invalid(&mut self) {
        let start = self.cursor;
        let byte = self.bytes[start];
        self.cursor += 1;
        let description = if byte.is_ascii() {
            format!("'{}'", char::from(byte))
        } else {
            let width = byte.leading_ones() as usize;
            let mut codepoint = u32::from(byte & (0x7f >> width.min(7)));
            let mut valid = (2..=4).contains(&width);
            if valid {
                for _ in 1..width {
                    let Some(next) = self.peek().filter(|next| next & 0xc0 == 0x80) else {
                        valid = false;
                        break;
                    };
                    codepoint = (codepoint << 6) | u32::from(next & 0x3f);
                    self.cursor += 1;
                }
            }
            if !valid || codepoint == 0 {
                "invalid UTF-8 sequence".to_owned()
            } else if let Some(suggestion) = confusables::ascii_lookalike(codepoint) {
                format!("Unicode character U+{codepoint:x} (did you mean '{suggestion}'?)")
            } else {
                format!("Unicode character U+{codepoint:x}")
            }
        };
        self.tokens.push(Token {
            kind: TokenKind::Invalid(description),
            span: Span::new(start, self.cursor),
        });
    }

    fn lex_number(&mut self) -> Result<()> {
        let start = self.cursor;
        // The reference lexer first consumes the complete number-like token.
        // Verification comes afterwards, so diagnostics name the whole literal.
        self.take_while(|b| b.is_ascii_digit() || matches!(b, b'.' | b'_'));
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.cursor += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.cursor += 1;
            }
        }
        self.take_while(|b| b.is_ascii_alphanumeric() || b == b'_');
        let span = Span::new(start, self.cursor);
        let digits = number_text_without_separators(&self.bytes[start..self.cursor]);
        let value = if let Some(text) = digits
            .strip_prefix("0x")
            .or_else(|| digits.strip_prefix("0X"))
        {
            radix_literal_value(text, 16)
        } else if let Some(text) = digits
            .strip_prefix("0b")
            .or_else(|| digits.strip_prefix("0B"))
        {
            radix_literal_value(text, 2)
        } else {
            digits.parse::<f64>().ok()
        }
        .ok_or_else(|| LexError {
            span,
            message: "Malformed number".to_owned(),
        })?;
        self.tokens.push(Token {
            kind: TokenKind::Number(value),
            span,
        });
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

    /// Recover the complete raw token only when decoding has failed. A
    /// closing quote distinguishes a bad escape from an unfinished literal.
    fn recover_short_string(&self, start: usize, quote: u8) -> Token {
        let mut end = start + 1;
        let mut kind = TokenKind::UnfinishedString;
        while let Some(&byte) = self.bytes.get(end) {
            match byte {
                b'\r' | b'\n' => break,
                b if b == quote => {
                    end += 1;
                    kind =
                        TokenKind::MalformedStringEscape(self.bytes[start + 1..end - 1].to_vec());
                    break;
                }
                b'\\' => {
                    end += 1;
                    match self.bytes.get(end) {
                        Some(b'z') => {
                            end += 1;
                            while self
                                .bytes
                                .get(end)
                                .is_some_and(|b| b.is_ascii_whitespace() || *b == 0x0b)
                            {
                                end += 1;
                            }
                        }
                        Some(b'\r') if self.bytes.get(end + 1) == Some(&b'\n') => end += 2,
                        Some(_) => end += 1,
                        None => break,
                    }
                }
                _ => end += 1,
            }
        }
        Token {
            span: Span::new(start, end),
            kind,
        }
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
                                self.cursor += 1;
                                value.push(other);
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
            let range = self.read_long_bracket(
                start,
                level,
                content_start,
                "Malformed string; did you forget to finish it?",
            )?;
            let raw = &self.bytes[range];
            // 0.709 ignores one initial LF or CRLF, and converts later CRLF pairs to LF.
            let raw = raw
                .strip_prefix(b"\r\n")
                .or_else(|| raw.strip_prefix(b"\n"))
                .unwrap_or(raw);
            let mut value = Vec::with_capacity(raw.len());
            for (index, &byte) in raw.iter().enumerate() {
                if byte != b'\r' || raw.get(index + 1) != Some(&b'\n') {
                    value.push(byte);
                }
            }
            self.tokens.push(Token {
                kind: TokenKind::String(value),
                span: Span::new(start, self.cursor),
            });
        } else {
            self.single(TokenKind::LBracket);
        }
        Ok(())
    }

    fn read_long_bracket(
        &mut self,
        opener_start: usize,
        level: usize,
        content_start: usize,
        unfinished: &'static str,
    ) -> Result<std::ops::Range<usize>> {
        self.cursor = content_start;
        while self.cursor < self.bytes.len() {
            if self.long_bracket_closer_matches(level) {
                let content_end = self.cursor;
                self.cursor += level + 2;
                return Ok(content_start..content_end);
            }
            self.cursor += 1;
        }
        Err(LexError {
            span: Span::new(opener_start, self.cursor),
            message: unfinished.to_owned(),
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

    fn lex_attribute(&mut self) {
        let start = self.cursor;
        self.cursor += 1;
        let kind = if self.peek() == Some(b'[') {
            self.cursor += 1;
            TokenKind::AttributeOpen
        } else {
            let name_start = self.cursor;
            if self
                .peek()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            {
                self.take_while(|b| b.is_ascii_alphanumeric() || b == b'_');
            }
            TokenKind::Attribute(ascii_identifier(&self.bytes[name_start..self.cursor]))
        };
        self.tokens.push(Token {
            kind,
            span: Span::new(start, self.cursor),
        });
    }

    fn lex_minus_or_comment(&mut self, keep_invalid: bool) -> Result<()> {
        let start = self.cursor;
        self.cursor += 1;
        if self.peek() == Some(b'-') {
            self.cursor += 1;
            if let Some((level, content_start)) = self.long_bracket_opener_at(self.cursor) {
                if let Err(error) =
                    self.read_long_bracket(start, level, content_start, "unfinished comment")
                {
                    if keep_invalid {
                        self.tokens.push(Token {
                            kind: TokenKind::Invalid(error.message),
                            span: error.span,
                        });
                    } else {
                        return Err(error);
                    }
                }
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

fn radix_literal_value(text: &str, radix: u32) -> Option<f64> {
    if text.is_empty() {
        return None;
    }
    // Luau 0.709 converts through strtoull. Out-of-range magnitudes
    // saturate, but a trailing invalid digit still makes the literal malformed.
    let mut value = 0_u64;
    for digit in text.chars() {
        let digit = u64::from(digit.to_digit(radix)?);
        value = value.saturating_mul(u64::from(radix)).saturating_add(digit);
    }
    Some(value as f64)
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
    fn radix_literals_round_to_nearest_and_saturate_at_u64_max() {
        // Expected IEEE-754 bit patterns from Luau 0.709 AstExprConstantNumber.
        // 2^53 + 1 and + 3 round in opposite directions at the nearest-even ties.
        let cases = [
            ("0x20000000000001", 0x4340_0000_0000_0000),
            ("0x20000000000003", 0x4340_0000_0000_0002),
            ("0xFFFFFFFFFFFFFBFF", 0x43ef_ffff_ffff_ffff),
            ("0xFFFFFFFFFFFFFC00", 0x43f0_0000_0000_0000),
            ("0x0000_ffff_ffff_ffff_ffff", 0x43f0_0000_0000_0000),
            ("0x1_0000_0000_0000_0000", 0x43f0_0000_0000_0000),
            ("0X2_0000_0000_0000_0000", 0x43f0_0000_0000_0000),
            (
                "0B1_00000000_00000000_00000000_00000000_00000000_00000000_00000000_00000000",
                0x43f0_0000_0000_0000,
            ),
        ];
        for (literal, expected) in cases {
            let tokens = lex(literal).unwrap();
            let TokenKind::Number(value) = tokens[0].kind else {
                panic!("{literal}");
            };
            assert_eq!(value.to_bits(), expected, "{literal}");
        }
        for malformed in [
            "0X2_0000_0000_0000_0000g",
            "0B111111111111111111111111111111111111111111111111111111111111111112",
        ] {
            assert_eq!(lex(malformed).unwrap_err().message, "Malformed number");
        }
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
