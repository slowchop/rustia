use crate::serde_value::from_live;
use crate::value::snapshot_value;
use crate::{CompiledModule, Error, Exports, FromRustia, HostSchema, Result, Rustia, Table, Value};

/// Luau source prepared for one execution or module installation.
///
/// Created by [`Rustia::load`]. Give it a [`source_name`](Self::source_name) for
/// useful diagnostics, then choose how to run it:
///
/// - [`eval`](Self::eval) returns ordinary Rust data.
/// - [`eval_serde`](Self::eval_serde) deserializes into a Rust structure.
/// - [`eval_live`](Self::eval_live) keeps a script value alive without copying it.
/// - [`exec`](Self::exec) discards the result.
/// - [`install`](Self::install) returns exports and rolls back guest state on failure.
///
/// Loading alone does not parse or execute the source. Each operation consumes
/// the chunk; use [`Rustia::compile_module`] to compile once and install repeatedly.
#[must_use = "a chunk does nothing until eval, eval_live, eval_serde, exec, or install is called"]
pub struct Chunk<'rustia, 'source, C = ()>
where
    C: HostSchema,
{
    rustia: &'rustia mut Rustia<C>,
    source: &'source [u8],
    source_name: Option<String>,
    source_map: crate::SourceMap,
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
            source_map: crate::SourceMap::default(),
            environment: None,
        }
    }

    /// Labels the source in errors and module resolution.
    ///
    /// Use a filename or another meaningful label, such as `"rules.luau"`.
    /// The default is `"chunk"`. This name is also the referrer passed to the
    /// module resolver for `require`; setting it does not read a file.
    pub fn source_name(mut self, source_name: impl Into<String>) -> Self {
        self.source_name = Some(source_name.into());
        self
    }

    /// Maps generated wrapper text out of human-facing diagnostics.
    ///
    /// This only changes error presentation; it does not change script execution
    /// or the source names and lines visible to scripts. See [`crate::SourceMap`].
    pub fn source_map(mut self, mapping: crate::SourceMap) -> Self {
        self.source_map = mapping;
        self
    }

    /// Selects the exact table used as this chunk's lexical environment.
    ///
    /// Cross-VM and reset-stale handles are rejected before parsing or
    /// compilation. For [`install`](Self::install), mutations to this table
    /// commit only when the complete installation succeeds.
    /// This replaces the default environment; globals are inherited only if the
    /// supplied table's metatable provides that behavior.
    pub fn environment(mut self, environment: impl Into<Table>) -> Self {
        self.environment = Some(environment.into());
        self
    }

    /// Runs the script and converts its first result to Rust data.
    ///
    /// Conversion uses [`FromRustia`] over an owned snapshot. No returned value
    /// means Luau `nil`; extra results are ignored. Use [`eval_live`](Self::eval_live)
    /// for functions and cyclic tables, or to preserve shared table identity.
    ///
    /// Invalid source, execution failures, and incompatible result types return
    /// [`Error`]. Script mutations remain in place even when execution or result
    /// conversion fails; use [`install`](Self::install) for guest-state rollback.
    ///
    /// ```
    /// # use rustia::Rustia;
    /// # let mut vm = Rustia::builder().build()?;
    /// let scores: Vec<i32> = vm.load("return {10, 20, 30}").eval()?;
    /// assert_eq!(scores, vec![10, 20, 30]);
    /// # Ok::<(), rustia::Error>(())
    /// ```
    pub fn eval<T>(self) -> Result<T>
    where
        T: FromRustia,
    {
        let name = self
            .source_name
            .clone()
            .unwrap_or_else(|| "chunk".to_owned());
        let limits = self.rustia.vm().settings().diagnostic_limits;
        let (rustia, value) = self.execute_raw()?;
        snapshot_value(rustia.vm_mut(), value)
            .and_then(T::from_rustia)
            .map_err(|error| error.with_result_source(&name, limits))
    }

    /// Runs the script and returns a live handle to its first result.
    ///
    /// The handle keeps the value alive in this interpreter and preserves shared
    /// and cyclic objects. It remains usable until [`Rustia::reset_decision`],
    /// and cannot be used with another interpreter. No returned value becomes
    /// a handle to `nil`. As with [`eval`](Self::eval), errors do not undo mutations.
    pub fn eval_live(self) -> Result<Value> {
        let (rustia, value) = self.execute_raw()?;
        Value::retain(rustia.vm_mut(), value)
    }

    /// Runs the script and deserializes its first result into Rust data.
    ///
    /// Reads the live VM graph without an intermediate [`crate::OwnedValue`].
    /// The destination still owns its strings and collections. Invalid source,
    /// execution failures, and data that does not match `T` return [`Error`];
    /// mutations remain in place on failure.
    ///
    /// ```
    /// # use rustia::Rustia;
    /// # let mut vm = Rustia::builder().build()?;
    /// #[derive(serde::Deserialize)]
    /// struct Settings {
    ///     volume: f64,
    /// }
    /// let settings: Settings = vm.load("return { volume = 0.5 }").eval_serde()?;
    /// assert_eq!(settings.volume, 0.5);
    /// # Ok::<(), rustia::Error>(())
    /// ```
    pub fn eval_serde<T>(self) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let name = self
            .source_name
            .clone()
            .unwrap_or_else(|| "chunk".to_owned());
        let limits = self.rustia.vm().settings().diagnostic_limits;
        let (rustia, value) = self.execute_raw()?;
        from_live(rustia.vm_mut(), value).map_err(|error| error.with_result_source(&name, limits))
    }

    /// Executes the chunk and discards its returned value.
    ///
    /// Parse, compilation, and execution errors are returned as [`Error`].
    /// Changes made before an error are retained, as with [`eval`](Self::eval).
    pub fn exec(self) -> Result<()> {
        self.execute_raw().map(|_| ())
    }

    /// Runs the script as a module and keeps its returned exports alive.
    ///
    /// Without [`environment`](Self::environment), installation creates a
    /// fresh module environment that inherits VM globals and binds `_G` to
    /// itself. Compile, runtime, quota, and result-retention failures leave
    /// prior guest-visible state unchanged.
    /// External effects from Rust callbacks or the module resolver cannot be
    /// undone. A successful installation does not invalidate earlier exports;
    /// the application chooses when to replace its active handle.
    ///
    /// ```
    /// # use rustia::Rustia;
    /// # let mut vm = Rustia::builder().build()?;
    /// let exports = vm.load("return { double = function(n) return n * 2 end }")
    ///     .source_name("math.luau")
    ///     .install()?;
    /// let double = exports.function(&mut vm, "double")?;
    /// let answer: i32 = double.call(&mut vm, (21,))?;
    /// assert_eq!(answer, 42);
    /// # Ok::<(), rustia::Error>(())
    /// ```
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
                .map_err(Error::from)?,
            None => rustia.vm_mut().run(&module.program).map_err(Error::from)?,
        };
        Ok((rustia, value))
    }

    fn prepare(self) -> Result<(&'rustia mut Rustia<C>, CompiledModule, Option<Table>)> {
        let Self {
            rustia,
            source,
            source_name,
            source_map,
            environment,
        } = self;
        rustia.ensure_dynamic_source_loading_allowed()?;
        environment
            .as_ref()
            .map(|environment| environment.raw(rustia.vm(), "chunk environment handle"))
            .transpose()?;
        source_map.validate(source)?;
        let limits = rustia.vm().settings().diagnostic_limits;
        let mut module = Rustia::compile_module_bytes_with_constructor_order(
            source_name.unwrap_or_else(|| "chunk".to_owned()),
            source,
            rustia.vm().profile() == crate::Profile::Puc51Compat,
            rustia.vm().settings().diagnostic_limits,
        )
        .map_err(|error| {
            source_map
                .apply_error(error, source, limits)
                .admit_diagnostic(rustia.vm())
        })?;
        source_map.apply(&mut module, source, limits);
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
