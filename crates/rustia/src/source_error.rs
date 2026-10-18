//! Source failures without parser or compiler implementation types.

use crate::SourceSpan;
use std::fmt;

/// Source text that could not be tokenized or parsed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SourceError {
    /// A token, string, or number could not be read.
    Lex(LexError),
    /// The tokens do not form a valid script.
    Parse(ParseError),
}

/// A failure while reading a source token.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct LexError {
    /// Byte range containing the failure.
    pub span: SourceSpan,
    /// Original message, without a source prefix.
    pub message: String,
}

/// A failure while parsing a script.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ParseError {
    /// The construct that still needs its closing keyword, when known.
    pub unclosed_block: Option<Box<UnclosedBlock>>,
    /// Byte range containing the failure.
    pub span: SourceSpan,
    /// Original message, without a source prefix.
    pub message: String,
}

/// A source construct missing its closing keyword.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct UnclosedBlock {
    /// Byte offset of the opening keyword.
    pub start: usize,
    /// The closing keyword expected by the parser.
    pub expected: &'static str,
}

/// Parsed source that cannot be compiled.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CompileError {
    /// Byte range containing the failure, when known.
    pub span: Option<SourceSpan>,
    /// Original message, without a source prefix.
    pub message: String,
}

impl SourceError {
    pub(crate) fn from_core(error: rustia_parser::SourceError) -> Self {
        match error {
            rustia_parser::SourceError::Lex(error) => Self::Lex(LexError {
                span: SourceSpan::from_lexer(error.span),
                message: error.message,
            }),
            rustia_parser::SourceError::Parse(error) => Self::Parse(ParseError {
                unclosed_block: error.unclosed_block.map(|block| {
                    Box::new(UnclosedBlock {
                        start: block.start,
                        expected: block.expected,
                    })
                }),
                span: SourceSpan::from_lexer(error.span),
                message: error.message,
            }),
        }
    }
}

impl CompileError {
    pub(crate) fn from_core(error: rustia_compiler::CompileError) -> Self {
        Self {
            span: error.span.map(SourceSpan::from_lexer),
            message: error.message,
        }
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(error) => error.fmt(formatter),
            Self::Parse(error) => error.fmt(formatter),
        }
    }
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

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "parse error at {}..{}: {}",
            self.span.start, self.span.end, self.message
        )
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(span) = self.span {
            write!(
                formatter,
                "compile error at {}..{}: {}",
                span.start, span.end, self.message
            )
        } else {
            write!(formatter, "compile error: {}", self.message)
        }
    }
}

impl std::error::Error for SourceError {}
impl std::error::Error for LexError {}
impl std::error::Error for ParseError {}
impl std::error::Error for CompileError {}
