//! Immutable, handle-free metadata for host-side error reports.
//!
//! This metadata never changes the guest error object or its display text.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Host-side diagnostic limits, independent of guest fuel and heap quotas.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiagnosticLimits {
    pub retained_source_bytes: usize,
    pub source_bytes: usize,
    pub frames: usize,
    pub report_bytes: usize,
}

impl Default for DiagnosticLimits {
    fn default() -> Self {
        Self {
            retained_source_bytes: 4 * 1024 * 1024,
            source_bytes: 256 * 1024,
            frames: 32,
            report_bytes: 16 * 1024,
        }
    }
}

/// Immutable executed source. VM-admitted snapshots carry a shared budget
/// lease, released only when both compiled code and outstanding errors drop it.
#[derive(Debug)]
pub struct SourceSnapshot {
    bytes: Arc<[u8]>,
    budget: Option<Arc<AtomicUsize>>,
    charge: usize,
}

impl SourceSnapshot {
    pub fn copy_with_limit(bytes: &[u8], limit: usize) -> Option<Arc<Self>> {
        if bytes.len() > limit {
            return None;
        }
        let mut copy = Vec::new();
        copy.try_reserve_exact(bytes.len()).ok()?;
        copy.extend_from_slice(bytes);
        Some(Arc::new(Self {
            bytes: copy.into(),
            budget: None,
            charge: 0,
        }))
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn admit(
        self: &Arc<Self>,
        budget: &Arc<AtomicUsize>,
        limits: DiagnosticLimits,
    ) -> Option<Arc<Self>> {
        if self.bytes.len() > limits.source_bytes {
            return None;
        }
        if self
            .budget
            .as_ref()
            .is_some_and(|owner| Arc::ptr_eq(owner, budget))
        {
            return Some(self.clone());
        }
        let charge = self.bytes.len().checked_add(std::mem::size_of::<Self>())?;
        budget
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(charge)
                    .filter(|next| *next <= limits.retained_source_bytes)
            })
            .ok()?;
        Some(Arc::new(Self {
            bytes: self.bytes.clone(),
            budget: Some(budget.clone()),
            charge,
        }))
    }
}

impl Drop for SourceSnapshot {
    fn drop(&mut self) {
        if let Some(budget) = &self.budget {
            budget.fetch_sub(self.charge, Ordering::Relaxed);
        }
    }
}

/// Byte offsets into the exact source supplied to the compiler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiagnosticSpan {
    pub start: usize,
    pub end: usize,
}

/// Facts recorded by a failing VM operation, not inferred from error text.
#[derive(Clone, Debug)]
pub enum DiagnosticReason {
    Index {
        object_type: &'static str,
        key: Option<String>,
    },
    Call {
        value_type: &'static str,
    },
    Arithmetic {
        operation: &'static str,
        left_type: &'static str,
        right_type: &'static str,
    },
}

/// A script frame resolved while the VM's execution state is still available.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticFrame {
    pub source: Arc<str>,
    pub line: u32,
    pub function: Option<String>,
}

/// Diagnostic information that can outlive the VM without rooting guest values.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub trace_captured: bool,
    pub reason: Option<DiagnosticReason>,
    pub data_path: Option<String>,
    pub generated_source: bool,
    pub host_cause: Option<Arc<dyn std::error::Error + Send + Sync>>,
    pub source: Option<Arc<str>>,
    pub line: Option<u32>,
    /// Opening line and expected closing keyword, established by the parser.
    pub unclosed_block: Option<(u32, &'static str)>,
    pub span: Option<DiagnosticSpan>,
    pub snapshot: Option<Arc<SourceSnapshot>>,
    pub report_bytes: usize,
    pub frames: Vec<DiagnosticFrame>,
    /// True when capture stopped at its frame limit.
    pub frames_truncated: bool,
}

impl Default for Diagnostic {
    fn default() -> Self {
        Self {
            host_cause: None,
            trace_captured: false,
            reason: None,
            data_path: None,
            generated_source: false,
            source: None,
            line: None,
            unclosed_block: None,
            span: None,
            snapshot: None,
            report_bytes: DiagnosticLimits::default().report_bytes,
            frames: Vec::new(),
            frames_truncated: false,
        }
    }
}
