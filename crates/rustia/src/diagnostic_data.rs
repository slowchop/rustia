//! Owned diagnostic data; engine storage and budget leases stay private.

use std::sync::Arc;

/// Host-side reporting budgets, separate from script fuel and memory limits.
///
/// Start with [`default`](Self::default) and adjust the limits your application
/// needs. Defaults are 4 MiB of retained-source accounting, 256 KiB per source,
/// 32 captured frames, and 16 KiB per formatted report. These limits do not
/// constrain script execution or change the error value received by `pcall`.
/// A zero source budget omits nonempty source text while retaining available
/// locations; zero frames disables frame storage, and zero report bytes produces
/// an empty report for captured diagnostics. Errors without captured diagnostics
/// use the formatter's fallback budget instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DiagnosticLimits {
    /// Total budget for retained source snapshots, including accounting overhead,
    /// shared by compiled scripts and outstanding errors. Measured in bytes.
    pub retained_source_bytes: usize,
    /// Maximum source length in bytes retained for one script.
    /// A larger source is omitted rather than stored as a partial snapshot.
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
    /// The zero-based offset of the first byte in the range.
    pub start: usize,
    /// The zero-based offset immediately after the range's final byte.
    pub end: usize,
}

/// Facts captured by a failing script operation.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum DiagnosticReason {
    /// A value could not be indexed.
    Index {
        /// The runtime type of the value being indexed.
        object_type: &'static str,
        /// The field name when captured; `None` means no field name is available.
        key: Option<String>,
    },
    /// A value could not be called.
    Call {
        /// The runtime type of the value used as a function.
        value_type: &'static str,
    },
    /// An arithmetic operation received incompatible values.
    Arithmetic {
        /// The arithmetic operation's name.
        operation: &'static str,
        /// The left operand's runtime type.
        left_type: &'static str,
        /// The right operand's runtime type.
        right_type: &'static str,
    },
}

/// A script call frame captured before execution state is discarded.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DiagnosticFrame {
    /// The source label recorded for this call site; not necessarily a filesystem path.
    pub source: Arc<str>,
    /// The one-based source line of the call site.
    pub line: u32,
    /// The function's debug name, when available. Long names may be truncated.
    pub function: Option<String>,
}

/// Source details and call frames that remain available after reset or drop.
///
/// This data is separate from the script's error value and never changes what
/// `pcall` receives. Source and frames may be absent when their budgets are zero.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Diagnostic {
    /// Whether runtime call-trace capture has been performed.
    /// This can be true even when no frames were retained.
    pub trace_captured: bool,
    /// Structured facts from the failing operation, when they were captured.
    pub reason: Option<DiagnosticReason>,
    /// The field or array path associated with a data-conversion failure, when known.
    pub data_path: Option<String>,
    /// Whether the failure falls in generated wrapper text rather than mapped input.
    pub generated_source: bool,
    /// The original typed host error, when a host cause was preserved.
    pub host_cause: Option<Arc<dyn std::error::Error + Send + Sync>>,
    /// The source label, when known; it need not name a file on disk.
    pub source: Option<Arc<str>>,
    /// The one-based source line, when known.
    pub line: Option<u32>,
    /// The one-based opening line and expected closing keyword for an unclosed block.
    pub unclosed_block: Option<(u32, &'static str)>,
    /// The half-open source byte range, when known. A source map may adjust it
    /// to refer to the retained input rather than generated wrapper text.
    pub span: Option<DiagnosticSpan>,
    /// Retained source text for this failure. It may be absent when the source
    /// or total retention budget is exceeded, allocation fails, or source is unavailable.
    pub snapshot: Option<Arc<SourceSnapshot>>,
    /// The maximum UTF-8 byte length of [`crate::Error::report`] output.
    /// This is a formatting budget, not the current report's length; zero emits nothing.
    pub report_bytes: usize,
    /// Captured caller frames, starting with the caller nearest the failure.
    /// An empty list can mean no callers, disabled capture, or unavailable frame data.
    pub frames: Vec<DiagnosticFrame>,
    /// Whether additional call frames were omitted when capture reached its bound.
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
