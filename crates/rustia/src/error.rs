use crate::{Diagnostic, DiagnosticLimits, DiagnosticSpan, SourceLocation, SourceSpan, VmError};
use rustia_compiler::CompileError;
use rustia_parser::SourceError;
use std::{fmt, sync::Arc};

/// A typed failure with independently renderable, handle-free diagnostics.
#[derive(Debug)]
pub struct Error {
    pub(crate) kind: ErrorKind,
    pub(crate) diagnostic: Option<Arc<Diagnostic>>,
    contexts: Vec<String>,
}

/// The original failure category. Match this instead of parsing error text.
///
/// New categories can be added as the language and embedding API grow. Include
/// a fallback arm when matching an error's [`kind`](Error::kind).
#[derive(Debug)]
#[non_exhaustive]
pub enum ErrorKind {
    Source(SourceError),
    Compile(CompileError),
    Vm(Box<VmError>),
    Embed(String),
    External(Arc<dyn std::error::Error + Send + Sync>),
}

impl Error {
    pub fn embed(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Embed(message.into()),
            diagnostic: None,
            contexts: Vec::new(),
        }
    }

    /// Returns this failure's original category without consuming its diagnostics.
    ///
    /// ```
    /// use rustia::{Error, ErrorKind};
    ///
    /// let error = Error::embed("configuration is missing");
    /// match error.kind() {
    ///     ErrorKind::Embed(message) => assert_eq!(message, "configuration is missing"),
    ///     _ => unreachable!("this example constructs an embedding error"),
    /// }
    /// ```
    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }

    /// Preserves a host's typed error for downcasting and standard cause traversal.
    pub fn external(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self {
            kind: ErrorKind::External(Arc::new(error)),
            diagnostic: None,
            contexts: Vec::new(),
        }
    }

    /// Adds host context without changing the typed failure classification.
    pub fn with_context(mut self, context: impl fmt::Display) -> Self {
        self.contexts.push(context.to_string());
        self
    }

    pub fn contexts(&self) -> impl DoubleEndedIterator<Item = &str> {
        self.contexts.iter().map(String::as_str)
    }

    pub(crate) fn with_data_path(mut self, path: String) -> Self {
        let diagnostic = self
            .diagnostic
            .get_or_insert_with(|| Arc::new(Diagnostic::default()));
        Arc::make_mut(diagnostic).data_path = Some(path);
        self
    }

    pub(crate) fn with_result_source(mut self, name: &str, limits: DiagnosticLimits) -> Self {
        let diagnostic = self
            .diagnostic
            .get_or_insert_with(|| Arc::new(Diagnostic::default()));
        let diagnostic = Arc::make_mut(diagnostic);
        if diagnostic.source.is_none() {
            diagnostic.source = Some(Arc::from(name));
        }
        diagnostic.report_bytes = limits.report_bytes;
        self.with_context("converting the script's returned value")
    }

    pub fn into_kind(self) -> ErrorKind {
        self.kind
    }

    pub(crate) fn installation_rolled_back(self) -> Self {
        self.with_context("installation failed; guest state was restored; external host effects are not rolled back")
    }

    pub fn as_vm_error(&self) -> Option<&VmError> {
        match &self.kind {
            ErrorKind::Vm(error) => Some(error),
            _ => None,
        }
    }

    pub fn source_span(&self) -> Option<SourceSpan> {
        if let Some(diagnostic) = self.diagnostic() {
            return diagnostic.span.map(|span| SourceSpan {
                start: span.start,
                end: span.end,
            });
        }
        match &self.kind {
            ErrorKind::Source(SourceError::Lex(error)) => Some(error.span.into()),
            ErrorKind::Source(SourceError::Parse(error)) => Some(error.span.into()),
            ErrorKind::Compile(error) => error.span.map(Into::into),
            ErrorKind::Vm(error) => error.span.map(|span| SourceSpan {
                start: span.0 as usize,
                end: span.1 as usize,
            }),
            ErrorKind::Embed(_) | ErrorKind::External(_) => None,
        }
    }

    pub fn source_location(&self) -> Option<SourceLocation<'_>> {
        if let Some(diagnostic) = self.diagnostic() {
            if let (Some(source), Some(line)) = (&diagnostic.source, diagnostic.line) {
                return Some(SourceLocation { source, line });
            }
        }
        self.as_vm_error()?
            .location
            .as_ref()
            .map(|location| SourceLocation {
                source: &location.source,
                line: location.line,
            })
    }

    pub(crate) fn with_source(
        mut self,
        name: &str,
        source: &[u8],
        limits: DiagnosticLimits,
    ) -> Self {
        let span = self.source_span().map(|span| DiagnosticSpan {
            start: span.start,
            end: span.end,
        });
        let line = span.and_then(|span| {
            source.get(..span.start).and_then(|prefix| {
                u32::try_from(prefix.iter().filter(|byte| **byte == b'\n').count() + 1).ok()
            })
        });
        let unclosed_block = match &self.kind {
            ErrorKind::Source(SourceError::Parse(error)) => {
                error.unclosed_block.as_ref().and_then(|block| {
                    let prefix = source.get(..block.start)?;
                    let line =
                        u32::try_from(prefix.iter().filter(|byte| **byte == b'\n').count() + 1)
                            .ok()?;
                    Some((line, block.expected))
                })
            }
            _ => None,
        };
        self.diagnostic = Some(Arc::new(Diagnostic {
            unclosed_block,
            source: Some(Arc::from(name)),
            line,
            span,
            snapshot: rustia_vm::diagnostic::SourceSnapshot::copy_with_limit(
                source,
                limits.source_bytes,
            ),
            report_bytes: limits.report_bytes,
            ..Diagnostic::default()
        }));
        self
    }

    pub(crate) fn admit_diagnostic(mut self, vm: &rustia_vm::Vm) -> Self {
        if let Some(diagnostic) = &mut self.diagnostic {
            vm.admit_host_diagnostic(Arc::make_mut(diagnostic));
        }
        self
    }
}

impl From<SourceError> for Error {
    fn from(error: SourceError) -> Self {
        Self {
            kind: ErrorKind::Source(error),
            diagnostic: None,
            contexts: Vec::new(),
        }
    }
}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self::embed(message)
    }
}

impl From<CompileError> for Error {
    fn from(error: CompileError) -> Self {
        Self {
            kind: ErrorKind::Compile(error),
            diagnostic: None,
            contexts: Vec::new(),
        }
    }
}

impl From<VmError> for Error {
    fn from(error: VmError) -> Self {
        let diagnostic = error.diagnostic.clone();
        Self {
            kind: ErrorKind::Vm(Box::new(error)),
            diagnostic,
            contexts: Vec::new(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for context in self.contexts().rev() {
            write!(formatter, "{context}: ")?;
        }
        self.kind.fmt(formatter)
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::Source(error) => error.fmt(formatter),
            ErrorKind::Compile(error) => error.fmt(formatter),
            ErrorKind::Vm(error) => error.fmt(formatter),
            ErrorKind::Embed(message) => formatter.write_str(message),
            ErrorKind::External(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            ErrorKind::Source(error) => Some(error),
            ErrorKind::Compile(error) => Some(error),
            ErrorKind::Vm(error) => Some(error.as_ref()),
            ErrorKind::Embed(_) => None,
            ErrorKind::External(error) => Some(error.as_ref()),
        }
    }
}
