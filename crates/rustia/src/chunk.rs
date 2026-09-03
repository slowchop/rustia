use crate::serde_value::from_live;
use crate::value::snapshot_value;
use crate::{CompiledModule, Error, Exports, FromRustia, HostSchema, Result, Rustia, Table, Value};

/// A borrowed source chunk configured for one execution or installation.
///
/// Create this with [`Rustia::load`], optionally attach a source name or exact
/// lexical environment, then choose one terminal operation.
#[must_use = "a chunk does nothing until eval, eval_live, eval_serde, exec, or install is called"]
pub struct Chunk<'rustia, 'source, C = ()>
where
    C: HostSchema,
{
    rustia: &'rustia mut Rustia<C>,
    source: &'source [u8],
    source_name: Option<String>,
    environment: Option<Table>,
}

impl<'rustia, 'source, C> Chunk<'rustia, 'source, C>
where
    C: HostSchema,
{
    pub(crate) fn new(rustia: &'rustia mut Rustia<C>, source: &'source [u8]) -> Self {
        Self {
            rustia,
            source,
            source_name: None,
            environment: None,
        }
    }

    /// Sets the source name used by compile/runtime diagnostics and as the
    /// referrer passed to the host module resolver for `require` calls.
    pub fn source_name(mut self, source_name: impl Into<String>) -> Self {
        self.source_name = Some(source_name.into());
        self
    }

    /// Selects the exact table used as this chunk's lexical environment.
    ///
    /// Cross-VM and reset-stale handles are rejected before parsing or
    /// compilation. For [`install`](Self::install), mutations to this table
    /// commit only when the complete installation succeeds.
    pub fn environment(mut self, environment: impl Into<Table>) -> Self {
        self.environment = Some(environment.into());
        self
    }

    /// Evaluates the chunk and converts its first result through the owned
    /// typed facade. Ordinary evaluation is stateful and does not roll back
    /// guest mutations when execution fails.
    pub fn eval<T>(self) -> Result<T>
    where
        T: FromRustia,
    {
        let (rustia, value) = self.execute_raw()?;
        let value = snapshot_value(rustia.vm_mut(), value)?;
        T::from_rustia(value)
    }

    /// Evaluates the chunk and retains its first result without snapshotting.
    pub fn eval_live(self) -> Result<Value> {
        let (rustia, value) = self.execute_raw()?;
        Value::retain(rustia.vm_mut(), value)
    }

    /// Evaluates and deserializes the first result directly from the live VM
    /// graph without constructing an intermediate [`crate::OwnedValue`].
    pub fn eval_serde<T>(self) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let (rustia, value) = self.execute_raw()?;
        from_live(rustia.vm_mut(), value)
    }

    /// Executes the chunk and discards its returned value.
    pub fn exec(self) -> Result<()> {
        self.execute_raw().map(|_| ())
    }

    /// Atomically installs this chunk and returns its rooted exports value.
    ///
    /// Without [`environment`](Self::environment), installation creates a
    /// fresh module environment that inherits VM globals and binds `_G` to
    /// itself. Compile, runtime, quota, and result-retention failures leave
    /// prior guest-visible state unchanged.
    pub fn install(self) -> Result<Exports> {
        let (rustia, module, environment) = self.prepare()?;
        let raw_environment = environment
            .as_ref()
            .map(|environment| environment.raw(rustia.vm(), "chunk environment handle"))
            .transpose()?;
        rustia.install_with_environment(&module, raw_environment)
    }

    fn execute_raw(self) -> Result<(&'rustia mut Rustia<C>, rustia_vm::Value)> {
        let (rustia, module, environment) = self.prepare()?;
        let raw_environment = environment
            .as_ref()
            .map(|environment| environment.raw(rustia.vm(), "chunk environment handle"))
            .transpose()?;
        let value = match raw_environment {
            Some(environment) => rustia
                .vm_mut()
                .run_with_environment(&module.program, environment)
                .map_err(Error::Vm)?,
            None => rustia.vm_mut().run(&module.program).map_err(Error::Vm)?,
        };
        Ok((rustia, value))
    }

    fn prepare(self) -> Result<(&'rustia mut Rustia<C>, CompiledModule, Option<Table>)> {
        let Self {
            rustia,
            source,
            source_name,
            environment,
        } = self;
        rustia.ensure_dynamic_source_loading_allowed()?;
        environment
            .as_ref()
            .map(|environment| environment.raw(rustia.vm(), "chunk environment handle"))
            .transpose()?;
        let module = Rustia::compile_module_bytes_with_constructor_order(
            source_name.unwrap_or_else(|| "chunk".to_owned()),
            source,
            rustia.vm().profile() == crate::Profile::Puc51Compat,
        )?;
        Ok((rustia, module, environment))
    }
}

impl From<&Table> for Table {
    fn from(table: &Table) -> Self {
        table.clone()
    }
}

impl<C> std::fmt::Debug for Chunk<'_, '_, C>
where
    C: HostSchema,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Chunk")
            .field("source_bytes", &self.source.len())
            .field("source_name", &self.source_name)
            .field("has_environment", &self.environment.is_some())
            .finish_non_exhaustive()
    }
}
