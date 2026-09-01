use crate::value::snapshot_value;
use crate::{Error, Exports, FromRustia, Result, Rustia, Table, Value};
use rustia_compiler::{compile_with_options, CompileOptions};

/// A source chunk configured for one execution.
///
/// Create this with [`Rustia::load`], then optionally attach a source name or
/// lexical environment before choosing the result shape. The source is
/// borrowed until the chunk is evaluated or executed; no VM allocation or
/// compilation happens while the builder is being configured.
#[must_use = "a chunk does nothing until eval, eval_value, eval_exports, or exec is called"]
pub struct Chunk<'rustia, 'source> {
    rustia: &'rustia mut Rustia,
    source: &'source [u8],
    source_name: Option<String>,
    environment: Option<Table>,
}

impl<'rustia, 'source> Chunk<'rustia, 'source> {
    pub(crate) fn new(rustia: &'rustia mut Rustia, source: &'source [u8]) -> Self {
        Self {
            rustia,
            source,
            source_name: None,
            environment: None,
        }
    }

    /// Sets the source name used by compile and runtime diagnostics.
    pub fn name(mut self, source_name: impl Into<String>) -> Self {
        self.source_name = Some(source_name.into());
        self
    }

    /// Selects the table used as this chunk's lexical global environment.
    ///
    /// Reads, writes, and functions created by the chunk use this exact table.
    /// Pass either an owned [`Table`] handle or `&Table`; the builder keeps a
    /// rooted clone until execution finishes. Cross-VM and reset-stale handles
    /// are rejected before source parsing or compilation.
    pub fn environment(mut self, environment: impl Into<Table>) -> Self {
        self.environment = Some(environment.into());
        self
    }

    /// Evaluates the chunk and converts its first result through the owned
    /// typed facade.
    pub fn eval<T>(self) -> Result<T>
    where
        T: FromRustia,
    {
        let (rustia, value) = self.execute_raw()?;
        let value = snapshot_value(rustia.vm_mut(), value)?;
        T::from_rustia(value)
    }

    /// Evaluates the chunk and retains its first result as a rooted live value.
    pub fn eval_value(self) -> Result<Value> {
        let (rustia, value) = self.execute_raw()?;
        let generation = rustia.generation();
        Value::retain(rustia.vm_mut(), value, generation)
    }

    /// Evaluates the chunk and exposes its first result through the retained
    /// exports facade.
    pub fn eval_exports(self) -> Result<Exports> {
        let (rustia, value) = self.execute_raw()?;
        let value = rustia.vm_mut().retain_value(value).map_err(Error::Vm)?;
        Ok(Exports::new(
            value,
            rustia.generation(),
            rustia.decision_epoch(),
        ))
    }

    /// Executes the chunk and discards its returned value.
    pub fn exec(self) -> Result<()> {
        self.execute_raw().map(|_| ())
    }

    fn execute_raw(self) -> Result<(&'rustia mut Rustia, rustia_vm::Value)> {
        let Self {
            rustia,
            source,
            source_name,
            environment,
        } = self;

        rustia.ensure_dynamic_source_loading_allowed()?;
        let environment = environment
            .as_ref()
            .map(|environment| environment.raw(rustia.vm(), "chunk environment handle"))
            .transpose()?;

        let chunk = rustia_parser::parse_source_bytes(source).map_err(Error::Source)?;
        let mut options = CompileOptions {
            puc51_constructor_order: rustia.vm().profile() == crate::Profile::Puc51Compat,
            ..CompileOptions::default()
        };
        if let Some(source_name) = source_name {
            options = options.with_source_bytes(source_name, source);
        }
        let program = compile_with_options(&chunk, options).map_err(Error::Compile)?;
        let value = match environment {
            Some(environment) => rustia
                .vm_mut()
                .run_with_environment(&program, environment)
                .map_err(Error::Vm)?,
            None => rustia.vm_mut().run(&program).map_err(Error::Vm)?,
        };
        rustia.advance_generation();
        Ok((rustia, value))
    }
}

impl From<&Table> for Table {
    fn from(table: &Table) -> Self {
        table.clone()
    }
}

impl std::fmt::Debug for Chunk<'_, '_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Chunk")
            .field("source_bytes", &self.source.len())
            .field("source_name", &self.source_name)
            .field("has_environment", &self.environment.is_some())
            .finish_non_exhaustive()
    }
}
