//! Bounded plain-text presentation for script failures.

use crate::Error;
use std::fmt::{self, Write};

pub use rustia_vm::diagnostic::DiagnosticReason;
pub use rustia_vm::diagnostic::{Diagnostic, DiagnosticFrame, DiagnosticLimits, DiagnosticSpan};

/// An error formatted with source context and a call trace when available.
///
/// Create one with [`Error::report`] and print it with `eprintln!("{report}")`.
/// Formatting is bounded by the configured [`DiagnosticLimits`] and never
/// runs script code.
pub struct ErrorReport<'a> {
    error: &'a Error,
}

impl Error {
    /// Returns the source location, call trace, and other details captured
    /// when this error occurred.
    ///
    /// These details remain usable after the VM is dropped or reset. Use
    /// [`report`](Self::report) to format them for a person.
    pub fn diagnostic(&self) -> Option<&Diagnostic> {
        self.diagnostic.as_deref().or_else(|| {
            self.as_vm_error()
                .and_then(|error| error.diagnostic.as_deref())
        })
    }

    /// Formats this error for a console or log, with source context when available.
    ///
    /// ```
    /// # use rustia::Rustia;
    /// let mut vm = Rustia::builder().build()?;
    /// if let Err(error) = vm.load("return missing.health")
    ///     .source_name("combat.luau")
    ///     .exec()
    /// {
    ///     eprintln!("{}", error.report());
    /// }
    /// # Ok::<(), rustia::Error>(())
    /// ```
    ///
    /// Printing the error itself gives a shorter message. Use
    /// [`kind`](Self::kind) to decide how your application should respond.
    pub fn report(&self) -> ErrorReport<'_> {
        ErrorReport { error: self }
    }
}

impl fmt::Display for ErrorReport<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut output = BoundedWriter {
            formatter,
            remaining: self
                .error
                .diagnostic()
                .map_or(16 * 1024, |diagnostic| diagnostic.report_bytes),
            truncated: false,
        };
        let reason = self
            .error
            .diagnostic()
            .and_then(|diagnostic| diagnostic.reason.as_ref());
        if let Some(reason) = reason {
            match reason {
                DiagnosticReason::Index {
                    object_type,
                    key: Some(key),
                } => writeln!(
                    output,
                    "error: cannot access `{key}`: the indexed value is {object_type}"
                )?,
                DiagnosticReason::Index {
                    object_type,
                    key: None,
                } => writeln!(
                    output,
                    "error: cannot index a {object_type} value with this key"
                )?,
                DiagnosticReason::Call { value_type } => {
                    writeln!(output, "error: this {value_type} value is not callable")?
                }
                DiagnosticReason::Arithmetic {
                    operation,
                    left_type,
                    right_type,
                } => writeln!(
                    output,
                    "error: cannot perform {operation} on {left_type} and {right_type}"
                )?,
            }
        } else if let Some(error) = self.error.as_vm_error().filter(|_| {
            self.error
                .diagnostic()
                .is_some_and(|diagnostic| diagnostic.source.is_some())
        }) {
            writeln!(output, "error: {}", error.message())?;
        } else {
            writeln!(output, "error: {}", self.error.kind())?;
        }
        for context in self.error.contexts().rev() {
            writeln!(output, "note: {context}")?;
        }
        if let Some(diagnostic) = self.error.diagnostic() {
            if let Some((line, expected)) = diagnostic.unclosed_block {
                writeln!(
                    output,
                    "help: the block opened on line {line} still needs `{expected}`"
                )?;
            }
            if let Some(path) = &diagnostic.data_path {
                writeln!(output, "field: {path}")?;
            }
            if diagnostic.generated_source {
                writeln!(output, "note: the failure is in generated wrapper code")?;
            }
            if let (Some(source), Some(line)) = (&diagnostic.source, diagnostic.line) {
                let column = diagnostic.snapshot.as_ref().and_then(|snapshot| {
                    let span = diagnostic.span?;
                    let prefix = std::str::from_utf8(snapshot.bytes().get(..span.start)?).ok()?;
                    let actual_line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
                    (actual_line == line as usize)
                        .then(|| prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1)
                });
                if let Some(column) = column {
                    writeln!(output, "  --> {source}:{line}:{column}")?;
                } else {
                    writeln!(output, "  --> {source}:{line}")?;
                }
            } else if let Some(source) = &diagnostic.source {
                writeln!(output, "  --> {source}")?;
            }
            if let (Some(snapshot), Some(line)) = (&diagnostic.snapshot, diagnostic.line) {
                // Source is byte-oriented. Invalid UTF-8 is not guessed into
                // character columns; retain a location-only report instead.
                if let Ok(source) = std::str::from_utf8(snapshot.bytes()) {
                    let line_index = line.saturating_sub(1) as usize;
                    let start = if line_index == 0 {
                        Some(0)
                    } else {
                        source
                            .bytes()
                            .enumerate()
                            .filter(|(_, byte)| *byte == b'\n')
                            .map(|(index, _)| index + 1)
                            .nth(line_index - 1)
                    };
                    if let Some(start) = start {
                        let text = source[start..]
                            .split('\n')
                            .next()
                            .unwrap_or("")
                            .trim_end_matches('\r');
                        writeln!(output, "\n {line} | {text}")?;
                        if let Some(span) = diagnostic.span {
                            let end = start.saturating_add(text.len());
                            if span.start >= start
                                && span.start <= end
                                && span.end >= span.start
                                && source.is_char_boundary(span.start)
                            {
                                let prefix = &source[start..span.start];
                                let marked_end = span.end.min(end);
                                if source.is_char_boundary(marked_end) {
                                    let width =
                                        source[span.start..marked_end].chars().count().max(1);
                                    // Keep alignment honest for characters whose terminal
                                    // width is not known to this ASCII renderer.
                                    if text.is_ascii() && !text.chars().any(char::is_control) {
                                        write!(
                                            output,
                                            " {} | ",
                                            " ".repeat(line.to_string().len())
                                        )?;
                                        for _ in 0..prefix.len().min(16 * 1024) {
                                            output.write_char(' ')?;
                                        }
                                        for _ in 0..width.min(16 * 1024) {
                                            output.write_char('^')?;
                                        }
                                        writeln!(output)?;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if reason.is_some() {
                if let Some(error) = self.error.as_vm_error() {
                    writeln!(output, "original: {}", error.message())?;
                }
            }
            if !diagnostic.frames.is_empty() {
                writeln!(output, "\ncalled from:")?;
                let mut index = 0;
                while let Some(frame) = diagnostic.frames.get(index) {
                    let repeated = diagnostic.frames[index..]
                        .iter()
                        .take_while(|other| *other == frame)
                        .count();
                    writeln!(
                        output,
                        "  {}    {}:{}",
                        frame.function.as_deref().unwrap_or("<script>"),
                        frame.source,
                        frame.line
                    )?;
                    if repeated > 1 {
                        writeln!(output, "  ... {} repeated calls ...", repeated - 1)?;
                    }
                    index += repeated;
                }
            }
            if diagnostic.frames_truncated {
                writeln!(output, "  ... additional frames omitted ...")?;
            }
        }
        if let Some(error) = self.error.as_vm_error() {
            match error.kind {
                crate::VmErrorKind::FuelExhausted { .. } => writeln!(output, "note: this is where the instruction budget ran out, not necessarily where most work happened")?,
                crate::VmErrorKind::OutOfMemory => writeln!(output, "note: this is where execution stopped, not necessarily the main source of memory use")?,
                crate::VmErrorKind::RecursionLimit { .. } => writeln!(output, "help: check recursion stopping conditions and whether the input can contain cycles")?,
                _ => {}
            }
        }
        // Host causes may contain useful I/O or schema context. Traverse a
        // bounded chain (including pathological cycles), never guest values.
        let mut cause = match self.error.kind() {
            crate::ErrorKind::Vm(error) => std::error::Error::source(error.as_ref()),
            crate::ErrorKind::External(error) => error.source(),
            _ => None,
        };
        for _ in 0..8 {
            let Some(current) = cause else { break };
            writeln!(output, "caused by: {current}")?;
            cause = current.source();
        }
        if cause.is_some() {
            writeln!(output, "... additional causes omitted ...")?;
        }
        if output.truncated && output.remaining >= "\n... report truncated ...".len() {
            output.formatter.write_str("\n... report truncated ...")?;
        }
        Ok(())
    }
}

// Stream directly into the formatter: never build an unbounded intermediate
// string from a guest error message. Reserve space for the truncation marker.
struct BoundedWriter<'a, 'b> {
    formatter: &'a mut fmt::Formatter<'b>,
    remaining: usize,
    truncated: bool,
}

impl Write for BoundedWriter<'_, '_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.truncated {
            return Ok(());
        }
        for character in text.chars() {
            let mut encoded = [0; 4];
            let text = if character.is_control() && character != '\n' {
                // All control characters except structural newlines are
                // escaped, including tabs, carriage returns and ANSI ESC.
                let escaped = character.escape_default();
                for character in escaped {
                    let text = character.encode_utf8(&mut encoded);
                    if self.remaining < text.len() + 32 {
                        self.truncated = true;
                        return Ok(());
                    }
                    self.formatter.write_str(text)?;
                    self.remaining -= text.len();
                }
                continue;
            } else {
                character.encode_utf8(&mut encoded)
            };
            if self.remaining < text.len() + 32 {
                self.truncated = true;
                return Ok(());
            }
            self.formatter.write_str(text)?;
            self.remaining -= text.len();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{Error, Rustia};

    #[test]
    fn report_does_not_change_original_error() {
        let error = Error::embed("bad\u{1b}[2Jvalue\r\t".to_owned());
        let original = error.to_string();
        let report = error.report().to_string();
        assert!(report.contains("bad\\u{1b}[2Jvalue\\r\\t"));
        assert_eq!(error.to_string(), original);
    }

    #[test]
    fn report_output_is_bounded_on_utf8_boundary() {
        let error = Error::embed("🦀".repeat(20_000));
        let report = error.report().to_string();
        assert!(report.len() <= 16 * 1024);
        assert!(report.ends_with("... report truncated ..."));
    }

    #[test]
    fn diagnostic_outlives_vm_and_reset() {
        let mut vm = Rustia::new(crate::Profile::LuauCompat);
        let error = vm.load("local function inner() error('broken') end\nlocal function outer() inner() end\nouter()")
            .source_name("trace.luau").exec().unwrap_err();
        let report = error.report().to_string();
        assert!(report.contains("trace.luau"));
        assert!(report.contains("called from:"), "{report}");
        assert!(report.contains("local function inner()"), "{report}");
        vm.reset_decision().unwrap();
        drop(vm);
        assert_eq!(error.report().to_string(), report);
    }

    #[test]
    fn explicitly_suppressed_location_has_no_host_trace() {
        let mut vm = Rustia::new(crate::Profile::LuauCompat);
        let error = vm
            .load("error('private', 0)")
            .source_name("hidden.luau")
            .exec()
            .unwrap_err();
        assert!(error.diagnostic().is_none());
        assert!(!error.report().to_string().contains("hidden.luau"));
    }

    #[test]
    fn source_budget_is_retained_by_errors_and_released_on_drop() {
        let mut vm = Rustia::new(crate::Profile::LuauCompat);
        let error = vm
            .load("error('broken')")
            .source_name("budget.luau")
            .exec()
            .unwrap_err();
        assert!(vm.diagnostic_source_bytes() > 0);
        vm.reset_decision().unwrap();
        assert!(vm.diagnostic_source_bytes() > 0);
        drop(error);
        assert_eq!(vm.diagnostic_source_bytes(), 0);
    }

    #[test]
    fn syntax_failure_has_named_source_and_snippet() {
        let error = Rustia::compile_module("syntax.luau", "local = 1")
            .err()
            .expect("invalid syntax");
        let report = error.report().to_string();
        assert!(report.contains("syntax.luau:1"), "{report}");
        assert!(report.contains("local = 1"), "{report}");
    }

    #[test]
    fn zero_source_budget_keeps_location_and_execution_behavior() {
        let settings = crate::VmSettings {
            diagnostic_limits: super::DiagnosticLimits {
                retained_source_bytes: 0,
                report_bytes: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut vm = crate::RustiaBuilder::standard()
            .settings(settings)
            .build()
            .unwrap();
        let error = vm
            .load("error('broken')")
            .source_name("limited.luau")
            .exec()
            .unwrap_err();
        assert!(error.diagnostic().unwrap().snapshot.is_none());
        assert_eq!(error.source_location().unwrap().source, "limited.luau");
        assert_eq!(error.report().to_string(), "");
        assert_eq!(vm.diagnostic_source_bytes(), 0);
    }

    #[test]
    fn serde_paths_use_lua_array_positions_and_preserve_messages() {
        #[derive(serde::Deserialize)]
        struct Enemy {
            #[serde(rename = "health")]
            _health: u32,
        }
        #[derive(serde::Deserialize)]
        struct Scenario {
            #[serde(rename = "enemies")]
            _enemies: Vec<Enemy>,
        }
        let mut vm = Rustia::new(crate::Profile::LuauCompat);
        let error = vm
            .load("return {enemies = {{health=1}, {health=2}, {health='bad'}}}")
            .eval_serde::<Scenario>()
            .err()
            .expect("invalid health");
        assert_eq!(
            error.diagnostic().unwrap().data_path.as_deref(),
            Some("enemies[3].health")
        );
        assert!(error
            .report()
            .to_string()
            .contains("field: enemies[3].health"));
    }

    #[test]
    fn host_context_preserves_resource_classification() {
        let error = Error::from(crate::VmError::new(crate::VmErrorKind::OutOfMemory))
            .with_context("installing scripts");
        assert!(matches!(
            error.as_vm_error().unwrap().kind,
            crate::VmErrorKind::OutOfMemory
        ));
    }

    #[test]
    fn console_prefix_maps_runtime_spans_and_retained_source() {
        let mut vm = Rustia::new(crate::Profile::LuauCompat);
        let error = vm
            .load("return missing.health")
            .source_name("console")
            .source_map(crate::SourceMap::generated_prefix(7))
            .exec()
            .unwrap_err();
        let report = error.report().to_string();
        assert!(report.contains("1 | missing.health"), "{report}");
        assert!(!report.contains("1 | return"), "{report}");
        assert_eq!(
            error.source_span(),
            Some(crate::SourceSpan { start: 0, end: 14 })
        );
    }

    #[test]
    fn generated_wrapper_failure_is_not_attributed_to_user_input() {
        let mut vm = Rustia::new(crate::Profile::LuauCompat);
        let error = vm
            .load("not_valid local x = 1")
            .source_map(crate::SourceMap::generated_prefix(10))
            .exec()
            .unwrap_err();
        assert!(error.diagnostic().unwrap().generated_source);
        assert!(error
            .report()
            .to_string()
            .contains("generated wrapper code"));
    }

    #[test]
    fn missing_end_points_to_the_innermost_unclosed_block() {
        let error = Rustia::compile_module(
            "blocks.luau",
            "local function outer()\n  if true then\n    local x = 1",
        )
        .err()
        .unwrap();
        assert_eq!(error.diagnostic().unwrap().unclosed_block, Some((2, "end")));
        assert!(error
            .report()
            .to_string()
            .contains("block opened on line 2 still needs `end`"));
    }

    #[test]
    fn reused_names_keep_the_exact_executed_source() {
        let mut vm = Rustia::default();
        vm.load("function old() return missing.old_field end")
            .source_name("shared.luau")
            .exec()
            .unwrap();
        let error = vm
            .load("-- replacement version\nold()")
            .source_name("shared.luau")
            .exec()
            .unwrap_err();
        let report = error.report().to_string();
        assert!(
            report.contains("1 | function old() return missing.old_field end"),
            "{report}"
        );
        assert!(!report.contains("replacement version"), "{report}");
    }

    #[test]
    fn unicode_source_uses_honest_line_fallback() {
        let mut vm = Rustia::default();
        let error = vm
            .load("local label = '🦀'; return missing.health")
            .source_name("unicode.luau")
            .exec()
            .unwrap_err();
        let report = error.report().to_string();
        assert!(report.contains("🦀"), "{report}");
        assert!(!report.contains('^'), "{report}");
    }

    #[test]
    fn deep_calls_are_bounded_and_explicitly_truncated() {
        let mut settings = crate::VmSettings::default();
        settings.diagnostic_limits.frames = 3;
        let mut vm = crate::RustiaBuilder::standard()
            .settings(settings)
            .build()
            .unwrap();
        let error = vm
            .load(
                "local function f(n) if n == 0 then error('stop') end return 1 + f(n-1) end\nf(20)",
            )
            .source_name("recursion.luau")
            .exec()
            .unwrap_err();
        let diagnostic = error.diagnostic().unwrap();
        assert!(diagnostic.frames.len() <= 3);
        assert!(diagnostic.frames_truncated);
        assert!(error
            .report()
            .to_string()
            .contains("additional frames omitted"));
    }

    #[test]
    fn host_errors_keep_typed_causes_and_guest_call_sites() {
        use std::error::Error as _;
        let mut vm = Rustia::default();
        let callback = vm
            .create_function(|| -> crate::Result<()> {
                Err(Error::external(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "asset missing",
                )))
            })
            .unwrap();
        vm.globals()
            .unwrap()
            .set_live(&mut vm, "asset", &callback.as_value())
            .unwrap();
        let error = vm
            .load("asset()")
            .source_name("assets.luau")
            .exec()
            .unwrap_err();
        let mut cause = error.source();
        let mut found = false;
        while let Some(current) = cause {
            found |= current.downcast_ref::<std::io::Error>().is_some();
            cause = current.source();
        }
        assert!(found);
        assert!(error.report().to_string().contains("assets.luau:1"));
    }

    #[test]
    fn failed_install_reports_only_the_guaranteed_rollback() {
        let mut vm = Rustia::default();
        let error = vm
            .load("_G.changed = 1; error('failed')")
            .source_name("install.luau")
            .install()
            .unwrap_err();
        let report = error.report().to_string();
        assert!(report.contains("guest state was restored"));
        assert!(report.contains("external host effects are not rolled back"));
        assert!(vm.eval::<bool>("return changed == nil").unwrap());
    }

    #[test]
    fn host_panic_reports_script_trigger_without_changing_raw_location_policy() {
        let mut vm = Rustia::default();
        let callback = vm
            .create_function(|| -> () { panic!("test host failure") })
            .unwrap();
        vm.globals()
            .unwrap()
            .set_live(&mut vm, "broken_host", &callback.as_value())
            .unwrap();
        let error = vm
            .load("broken_host()")
            .source_name("host-trigger.luau")
            .exec()
            .unwrap_err();
        assert!(error.as_vm_error().unwrap().location_suppressed);
        assert!(error.as_vm_error().unwrap().location.is_none());
        assert!(error.report().to_string().contains("host-trigger.luau:1"));
        assert!(matches!(
            error.as_vm_error().unwrap().kind,
            crate::VmErrorKind::HostPanicked { .. }
        ));
    }

    #[test]
    fn diagnostic_limits_do_not_change_guest_messages_clock_or_live_memory() {
        fn run(limits: crate::DiagnosticLimits) -> ((bool, String, f64), usize) {
            let settings = crate::VmSettings {
                diagnostic_limits: limits,
                ..Default::default()
            };
            let mut vm = crate::RustiaBuilder::standard()
                .settings(settings)
                .build()
                .unwrap();
            let result = vm.load("local ok, message = pcall(function() return missing.health end)\nreturn {ok, message, os.clock()}")
                .source_name("protected.luau").eval_serde().unwrap();
            (result, vm.memory_stats().live_bytes)
        }
        assert_eq!(
            run(crate::DiagnosticLimits::default()),
            run(crate::DiagnosticLimits {
                retained_source_bytes: 0,
                source_bytes: 0,
                frames: 0,
                report_bytes: 0,
            })
        );
    }
}
