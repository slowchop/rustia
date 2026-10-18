//! Owned diagnostic data; engine storage and budget leases stay private.

use std::sync::Arc;

/// Host-side reporting budgets, separate from script fuel and memory limits.
///
/// Start with [`default`](Self::default) and adjust the limits your application
/// needs. A zero source budget retains locations without copying source text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DiagnosticLimits {
    /// Total bytes retained by compiled scripts and outstanding errors.
    pub retained_source_bytes: usize,
    /// Maximum source size retained for a single script.
    pub source_bytes: usize,
    /// Maximum number of captured script frames.
    pub frames: usize,
    /// Maximum bytes written by a formatted error report.
    pub report_bytes: usize,
}

impl DiagnosticLimits {
    /// Uses the default source, frame, and report budgets.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the total retained-source budget.
    pub const fn retained_source_bytes(&self) -> usize {
        self.retained_source_bytes
    }

    /// Returns the largest source retained for one script.
    pub const fn source_bytes(&self) -> usize {
        self.source_bytes
    }

    /// Returns the maximum captured frame count.
    pub const fn frames(&self) -> usize {
        self.frames
    }

    /// Returns the maximum formatted report size.
    pub const fn report_bytes(&self) -> usize {
        self.report_bytes
    }

    /// Sets the total retained-source budget.
    pub const fn with_retained_source_bytes(mut self, bytes: usize) -> Self {
        self.retained_source_bytes = bytes;
        self
    }

    /// Sets the maximum source size retained for one script.
    pub const fn with_source_bytes(mut self, bytes: usize) -> Self {
        self.source_bytes = bytes;
        self
    }

    /// Sets the number of frames captured for one error.
    pub const fn with_frames(mut self, frames: usize) -> Self {
        self.frames = frames;
        self
    }

    /// Sets the maximum formatted report size.
    pub const fn with_report_bytes(mut self, bytes: usize) -> Self {
        self.report_bytes = bytes;
        self
    }

    pub(crate) const fn from_core(limits: rustia_vm::diagnostic::DiagnosticLimits) -> Self {
        Self {
            retained_source_bytes: limits.retained_source_bytes,
            source_bytes: limits.source_bytes,
            frames: limits.frames,
            report_bytes: limits.report_bytes,
        }
    }

    pub(crate) const fn to_core(self) -> rustia_vm::diagnostic::DiagnosticLimits {
        rustia_vm::diagnostic::DiagnosticLimits {
            retained_source_bytes: self.retained_source_bytes,
            source_bytes: self.source_bytes,
            frames: self.frames,
            report_bytes: self.report_bytes,
        }
    }
}

impl Default for DiagnosticLimits {
    fn default() -> Self {
        Self::from_core(rustia_vm::diagnostic::DiagnosticLimits::default())
    }
}

/// A retained copy of the exact source used for an error report.
///
/// Keeping a snapshot alive retains its reporting-budget charge. It does not
/// retain script objects or keep the interpreter alive.
#[derive(Debug)]
pub struct SourceSnapshot {
    core: Arc<rustia_vm::diagnostic::SourceSnapshot>,
}

impl SourceSnapshot {
    /// Returns the original bytes; source is not required to be valid UTF-8.
    pub fn bytes(&self) -> &[u8] {
        self.core.bytes()
    }

    pub(crate) fn copy_with_limit(bytes: &[u8], limit: usize) -> Option<Arc<Self>> {
        rustia_vm::diagnostic::SourceSnapshot::copy_with_limit(bytes, limit).map(Self::from_core)
    }

    fn from_core(core: Arc<rustia_vm::diagnostic::SourceSnapshot>) -> Arc<Self> {
        Arc::new(Self { core })
    }
}

/// A half-open byte range in the exact input source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiagnosticSpan {
    pub start: usize,
    pub end: usize,
}

/// Facts captured by a failing script operation.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum DiagnosticReason {
    /// A value could not be indexed.
    Index {
        object_type: &'static str,
        key: Option<String>,
    },
    /// A value could not be called.
    Call { value_type: &'static str },
    /// An arithmetic operation received incompatible values.
    Arithmetic {
        operation: &'static str,
        left_type: &'static str,
        right_type: &'static str,
    },
}

/// A script call frame captured before execution state is discarded.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DiagnosticFrame {
    pub source: Arc<str>,
    pub line: u32,
    pub function: Option<String>,
}

/// Source details and call frames that remain available after reset or drop.
///
/// This data is separate from the script's error value and never changes what
/// `pcall` receives. Source and frames may be absent when their budgets are zero.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Diagnostic {
    pub trace_captured: bool,
    pub reason: Option<DiagnosticReason>,
    pub data_path: Option<String>,
    pub generated_source: bool,
    pub host_cause: Option<Arc<dyn std::error::Error + Send + Sync>>,
    pub source: Option<Arc<str>>,
    pub line: Option<u32>,
    /// Opening line and the expected closing keyword for an unclosed block.
    pub unclosed_block: Option<(u32, &'static str)>,
    pub span: Option<DiagnosticSpan>,
    pub snapshot: Option<Arc<SourceSnapshot>>,
    pub report_bytes: usize,
    pub frames: Vec<DiagnosticFrame>,
    pub frames_truncated: bool,
}

impl Diagnostic {
    pub(crate) fn from_core(diagnostic: &rustia_vm::diagnostic::Diagnostic) -> Self {
        Self {
            trace_captured: diagnostic.trace_captured,
            reason: diagnostic.reason.as_ref().map(|reason| match reason {
                rustia_vm::diagnostic::DiagnosticReason::Index { object_type, key } => {
                    DiagnosticReason::Index {
                        object_type,
                        key: key.clone(),
                    }
                }
                rustia_vm::diagnostic::DiagnosticReason::Call { value_type } => {
                    DiagnosticReason::Call { value_type }
                }
                rustia_vm::diagnostic::DiagnosticReason::Arithmetic {
                    operation,
                    left_type,
                    right_type,
                } => DiagnosticReason::Arithmetic {
                    operation,
                    left_type,
                    right_type,
                },
            }),
            data_path: diagnostic.data_path.clone(),
            generated_source: diagnostic.generated_source,
            host_cause: diagnostic.host_cause.clone(),
            source: diagnostic.source.clone(),
            line: diagnostic.line,
            unclosed_block: diagnostic.unclosed_block,
            span: diagnostic.span.map(|span| DiagnosticSpan {
                start: span.start,
                end: span.end,
            }),
            snapshot: diagnostic.snapshot.clone().map(SourceSnapshot::from_core),
            report_bytes: diagnostic.report_bytes,
            frames: diagnostic
                .frames
                .iter()
                .map(|frame| DiagnosticFrame {
                    source: frame.source.clone(),
                    line: frame.line,
                    function: frame.function.clone(),
                })
                .collect(),
            frames_truncated: diagnostic.frames_truncated,
        }
    }

    pub(crate) fn to_core(&self) -> rustia_vm::diagnostic::Diagnostic {
        rustia_vm::diagnostic::Diagnostic {
            trace_captured: self.trace_captured,
            reason: self.reason.as_ref().map(|reason| match reason {
                DiagnosticReason::Index { object_type, key } => {
                    rustia_vm::diagnostic::DiagnosticReason::Index {
                        object_type,
                        key: key.clone(),
                    }
                }
                DiagnosticReason::Call { value_type } => {
                    rustia_vm::diagnostic::DiagnosticReason::Call { value_type }
                }
                DiagnosticReason::Arithmetic {
                    operation,
                    left_type,
                    right_type,
                } => rustia_vm::diagnostic::DiagnosticReason::Arithmetic {
                    operation,
                    left_type,
                    right_type,
                },
            }),
            data_path: self.data_path.clone(),
            generated_source: self.generated_source,
            host_cause: self.host_cause.clone(),
            source: self.source.clone(),
            line: self.line,
            unclosed_block: self.unclosed_block,
            span: self.span.map(|span| rustia_vm::diagnostic::DiagnosticSpan {
                start: span.start,
                end: span.end,
            }),
            snapshot: self.snapshot.as_ref().map(|snapshot| snapshot.core.clone()),
            report_bytes: self.report_bytes,
            frames: self
                .frames
                .iter()
                .map(|frame| rustia_vm::diagnostic::DiagnosticFrame {
                    source: frame.source.clone(),
                    line: frame.line,
                    function: frame.function.clone(),
                })
                .collect(),
            frames_truncated: self.frames_truncated,
        }
    }
}

impl Default for Diagnostic {
    fn default() -> Self {
        Self::from_core(&rustia_vm::diagnostic::Diagnostic::default())
    }
}
