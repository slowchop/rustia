use super::{ParseError, SourceParser, Span, Token, TokenKind};

impl SourceParser<'_> {
    pub(super) fn describe_token(&self) -> String {
        self.describe_kind(&self.peek().kind, self.peek().span)
    }

    pub(super) fn diagnostic_span(&self, span: Span) -> Span {
        // Lowering gives the synthetic interpolation name a zero-width span at
        // the opening backtick. The reference lexer reports its first section.
        if span.start == span.end {
            if let Some(string) = self.interpolations.get(&span.start) {
                return string.sections.first().copied().unwrap_or(string.span);
            }
        }
        span
    }

    pub(super) fn describe_kind(&self, kind: &TokenKind, span: Span) -> String {
        let original = self.diagnostic_span(span);
        let raw = self
            .source
            .get(original.start..original.end)
            .filter(|s| !s.is_empty());
        match (kind, raw) {
            (TokenKind::Number(_), Some(raw)) => format!("'{}'", String::from_utf8_lossy(raw)),
            (TokenKind::String(_), Some(raw)) => {
                let contents = raw_string_contents(raw);
                format!("\"{}\"", String::from_utf8_lossy(contents))
            }
            (TokenKind::Ident(_), Some(raw)) if original != span => {
                String::from_utf8_lossy(raw).into_owned()
            }
            _ => token_description(kind),
        }
    }

    pub(super) fn expected_match(&self, open: &Token, close: &str) -> ParseError {
        let line = self.line_feeds.partition_point(|&lf| lf < open.span.start);
        let here = self
            .line_feeds
            .partition_point(|&lf| lf < self.peek().span.start);
        let location = if line == here {
            format!("column {}", self.column(open.span.start) + 1)
        } else {
            format!("line {}", line + 1)
        };
        ParseError {
            unclosed_block: None,
            span: self.diagnostic_span(self.peek().span),
            message: format!(
                "Expected '{close}' (to close {} at {location}), got {}",
                token_description(&open.kind),
                self.describe_token()
            ),
        }
    }

    pub(super) fn column(&self, offset: usize) -> usize {
        let line = self.line_feeds.partition_point(|&lf| lf < offset);
        let start = line.checked_sub(1).map_or(0, |i| self.line_feeds[i] + 1);
        offset - start
    }
}

// A lexer-produced string has matching delimiters. Luau prints its raw
// contents in double quotes, even for single quotes and long bracket strings.
fn raw_string_contents(raw: &[u8]) -> &[u8] {
    let delimiter_len = if raw.starts_with(b"[") {
        2 + raw[1..].iter().take_while(|&&b| b == b'=').count()
    } else {
        1
    };
    &raw[delimiter_len..raw.len() - delimiter_len]
}

fn token_description(kind: &TokenKind) -> String {
    let text = match kind {
        TokenKind::Eof => "<eof>",
        TokenKind::Invalid(description) => return description.clone(),
        TokenKind::Ident(name) | TokenKind::Attribute(name) => return format!("'{name}'"),
        TokenKind::Number(value) => return format!("'{value}'"),
        TokenKind::String(value) => return format!("\"{}\"", String::from_utf8_lossy(value)),
        TokenKind::AttributeOpen => "'@['",
        TokenKind::Local => "'local'",
        TokenKind::Return => "'return'",
        TokenKind::Function => "'function'",
        TokenKind::If => "'if'",
        TokenKind::Then => "'then'",
        TokenKind::ElseIf => "'elseif'",
        TokenKind::Else => "'else'",
        TokenKind::End => "'end'",
        TokenKind::While => "'while'",
        TokenKind::Do => "'do'",
        TokenKind::For => "'for'",
        TokenKind::In => "'in'",
        TokenKind::Repeat => "'repeat'",
        TokenKind::Until => "'until'",
        TokenKind::Break => "'break'",
        TokenKind::Not => "'not'",
        TokenKind::And => "'and'",
        TokenKind::Or => "'or'",
        TokenKind::True => "'true'",
        TokenKind::False => "'false'",
        TokenKind::Nil => "'nil'",
        TokenKind::Type => "'type'",
        TokenKind::Plus => "'+'",
        TokenKind::PlusEqual => "'+='",
        TokenKind::Minus => "'-'",
        TokenKind::MinusEqual => "'-='",
        TokenKind::Star => "'*'",
        TokenKind::StarEqual => "'*='",
        TokenKind::Slash => "'/'",
        TokenKind::SlashEqual => "'/='",
        TokenKind::FloorDiv => "'//'",
        TokenKind::FloorDivEqual => "'//='",
        TokenKind::Percent => "'%'",
        TokenKind::PercentEqual => "'%='",
        TokenKind::Caret => "'^'",
        TokenKind::CaretEqual => "'^='",
        TokenKind::Equal => "'='",
        TokenKind::EqualEqual => "'=='",
        TokenKind::TildeEqual => "'~='",
        TokenKind::LessEqual => "'<='",
        TokenKind::GreaterEqual => "'>='",
        TokenKind::Comma => "','",
        TokenKind::Semicolon => "';'",
        TokenKind::Colon => "':'",
        TokenKind::Dot => "'.'",
        TokenKind::Hash => "'#'",
        TokenKind::Concat => "'..'",
        TokenKind::ConcatEqual => "'..='",
        TokenKind::Ellipsis => "'...'",
        TokenKind::LParen => "'('",
        TokenKind::RParen => "')'",
        TokenKind::LBrace => "'{'",
        TokenKind::RBrace => "'}'",
        TokenKind::LBracket => "'['",
        TokenKind::RBracket => "']'",
        TokenKind::Less => "'<'",
        TokenKind::Greater => "'>'",
        TokenKind::Pipe => "'|'",
        TokenKind::Ampersand => "'&'",
        TokenKind::Question => "'?'",
        TokenKind::Arrow => "'->'",
    };
    text.to_owned()
}
