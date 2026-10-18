use crate::{CompiledModule, DiagnosticLimits, Error, Result};

/// Keeps error locations aligned with input when a console adds wrapper text.
///
/// For example, a console may turn `1 + 2` into `return 1 + 2` before running
/// it. Pass `SourceMap::generated_prefix(7)` to
/// [`Chunk::source_map`](crate::Chunk::source_map) to hide the added `return `
/// from source excerpts and positions. Wrappers cannot contain line breaks.
/// Script-visible strings are unchanged.
#[derive(Clone, Copy, Debug, Default)]
pub struct SourceMap {
    prefix_bytes: usize,
    suffix_bytes: usize,
}

impl SourceMap {
    /// Marks this many leading bytes as generated text on the first line.
    pub const fn generated_prefix(prefix_bytes: usize) -> Self {
        Self {
            prefix_bytes,
            suffix_bytes: 0,
        }
    }

    /// Removes same-line generated text on both sides of the original input.
    pub const fn generated_wrapper(prefix_bytes: usize, suffix_bytes: usize) -> Self {
        Self {
            prefix_bytes,
            suffix_bytes,
        }
    }

    pub(crate) fn validate(self, source: &[u8]) -> Result<()> {
        let end = source
            .len()
            .checked_sub(self.suffix_bytes)
            .filter(|end| *end >= self.prefix_bytes)
            .ok_or_else(|| Error::embed("source-map wrapper exceeds the chunk length"))?;
        let prefix = source
            .get(..self.prefix_bytes)
            .ok_or_else(|| Error::embed("source-map prefix exceeds the chunk length"))?;
        let suffix = &source[end..];
        if prefix.contains(&b'\n')
            || prefix.contains(&b'\r')
            || suffix.contains(&b'\n')
            || suffix.contains(&b'\r')
        {
            return Err(Error::embed(
                "source-map wrappers must not contain line breaks",
            ));
        }
        Ok(())
    }

    pub(crate) fn apply(
        self,
        module: &mut CompiledModule,
        source: &[u8],
        limits: DiagnosticLimits,
    ) {
        if self.prefix_bytes == 0 && self.suffix_bytes == 0 {
            return;
        }
        module.program.diagnostic_prefix_bytes = self.prefix_bytes;
        module.program.diagnostic_mapped_length =
            Some(source.len() - self.prefix_bytes - self.suffix_bytes);
        module.program.diagnostic_source = rustia_vm::diagnostic::SourceSnapshot::copy_with_limit(
            &source[self.prefix_bytes..source.len() - self.suffix_bytes],
            limits.source_bytes,
        );
    }

    pub(crate) fn apply_error(
        self,
        mut error: Error,
        source: &[u8],
        limits: DiagnosticLimits,
    ) -> Error {
        if self.prefix_bytes == 0 && self.suffix_bytes == 0 {
            return error;
        }
        if let Some(diagnostic) = &mut error.diagnostic {
            let diagnostic = std::sync::Arc::make_mut(diagnostic);
            diagnostic.generated_source = diagnostic.span.is_some_and(|span| {
                span.start < self.prefix_bytes || span.end > source.len() - self.suffix_bytes
            });
            diagnostic.span = diagnostic
                .span
                .filter(|_| !diagnostic.generated_source)
                .and_then(|span| {
                    Some(crate::DiagnosticSpan {
                        start: span.start.checked_sub(self.prefix_bytes)?,
                        end: span.end.checked_sub(self.prefix_bytes)?,
                    })
                });
            diagnostic.snapshot = if diagnostic.generated_source {
                None
            } else {
                rustia_vm::diagnostic::SourceSnapshot::copy_with_limit(
                    &source[self.prefix_bytes..source.len() - self.suffix_bytes],
                    limits.source_bytes,
                )
            };
        }
        error
    }
}
