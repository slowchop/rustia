use crate::function::IntoHostFunction;
use crate::{Error, Profile, Result, Rustia, VmError, VmErrorKind, VmSettings};
use rustia_compiler::{compile_with_options, CompileOptions};
use rustia_vm::{ClockProvider, HostFunctionOp, OpCapability, Program, StdoutSink};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Fluent builder for [`Rustia`]: start from a preset ([`standard`](Self::standard),
/// [`deterministic`](Self::deterministic), [`live`](Self::live)), then override profile,
/// settings, fuel, services, and extensions before [`build`](Self::build).
pub struct RustiaBuilder {
    profile: Profile,
    settings: VmSettings,
    extensions: Vec<RustiaExtension>,
    live_requested: bool,
}

impl RustiaBuilder {
    pub(crate) fn new() -> Self {
        Self {
            profile: Profile::ReproducibleStandard,
            settings: VmSettings::default(),
            extensions: Vec::new(),
            live_requested: false,
        }
    }

    /// Reproducible-by-default preset: `Profile::ReproducibleStandard` plus a real stdout
    /// `print` sink on native targets (no sink on wasm32, where `print` stays a no-op).
    /// Clocks keep the VM's built-in deterministic defaults — real time is always an explicit
    /// `.wall_clock(ServiceClock::system())` choice, never a default.
    /// [`Rustia::builder()`](crate::Rustia::builder) delegates here.
    pub fn standard() -> Self {
        let builder = Self::new();
        #[cfg(not(target_arch = "wasm32"))]
        let builder = builder.stdout(|text| print!("{text}"));
        builder
    }

    /// Sealed-box preset: `Profile::DeterministicSim`, no services. Deterministic providers
    /// can still be plugged explicitly via the service sockets.
    pub fn deterministic() -> Self {
        Self::new().profile(Profile::DeterministicSim)
    }

    /// "Normal Luau" preset with `loadstring` enabled (runtime codegen — attack surface for
    /// hosts running untrusted scripts).
    ///
    /// `Profile::ReproducibleStandard` + real stdout + `ServiceClock::system()` on both clock
    /// sockets + the stock `loadstring` compiler. The language core stays deterministic
    /// (insertion order, pinned PCG32, Schubfach, libm); only the services go live. On wasm32
    /// there is no system clock or stdout, so [`build`](Self::build) fails with a typed error
    /// telling the host to plug providers explicitly.
    pub fn live() -> Self {
        let builder = Self::standard();
        #[cfg(not(target_arch = "wasm32"))]
        let builder = builder
            .wall_clock(ServiceClock::system())
            .monotonic_clock(ServiceClock::system());
        let mut builder = builder;
        builder.live_requested = true;
        builder.settings.loadstring_compiler = Some(stock_loadstring_compiler);
        builder
    }

    pub fn profile(mut self, profile: Profile) -> Self {
        self.profile = profile;
        self
    }

    pub fn settings(mut self, settings: VmSettings) -> Self {
        self.settings = settings;
        self
    }

    pub fn fuel(mut self, fuel: u64) -> Self {
        self.settings.initial_fuel = Some(fuel);
        self
    }

    pub fn extension(mut self, extension: RustiaExtension) -> Self {
        self.extensions.push(extension);
        self
    }

    /// Installs the sink that receives composed `print` output (arguments tab-joined, trailing
    /// newline). Closure only — sink errors are the host closure's business. `print` is an
    /// output service: sink-attachable on every profile, including `DeterministicSim`.
    pub fn stdout(mut self, sink: impl FnMut(&str) + 'static) -> Self {
        self.settings.services.stdout = Some(StdoutSink::new(sink));
        self
    }

    /// Plugs the wall-clock provider backing no-arg `os.time()` and the default `os.date`
    /// timestamp. A nondeterministic provider is rejected at [`build`](Self::build) on
    /// `DeterministicSim`.
    pub fn wall_clock(mut self, clock: ServiceClock) -> Self {
        self.settings.services.wall_clock = Some(clock.into_provider());
        self
    }

    /// Plugs the monotonic-clock provider backing `os.clock()`, overriding the VM's built-in
    /// logical clock. Same `DeterministicSim` validation as [`wall_clock`](Self::wall_clock).
    pub fn monotonic_clock(mut self, clock: ServiceClock) -> Self {
        self.settings.services.monotonic_clock = Some(clock.into_provider());
        self
    }

    pub fn build(self) -> Result<Rustia> {
        if cfg!(target_arch = "wasm32") && self.live_requested {
            return Err(Error::Vm(VmError::new(VmErrorKind::Unsupported {
                feature: "live() preset on wasm32 (no system clock or stdout exists; plug \
                          providers explicitly via .stdout/.wall_clock/.monotonic_clock)",
            })));
        }
        if self.profile.is_deterministic_sim() {
            for (feature, clock) in [
                (
                    "non-deterministic wall clock provider",
                    &self.settings.services.wall_clock,
                ),
                (
                    "non-deterministic monotonic clock provider",
                    &self.settings.services.monotonic_clock,
                ),
            ] {
                if clock.as_ref().is_some_and(|clock| !clock.deterministic()) {
                    return Err(Error::Vm(VmError::new(VmErrorKind::CapabilityRemoved {
                        feature,
                        profile: self.profile,
                    })));
                }
            }
        }
        let mut rustia = Rustia::with_settings(self.profile, self.settings);
        for extension in self.extensions {
            rustia.install_extension(extension)?;
        }
        Ok(rustia)
    }
}

/// Host clock provider plugged into a builder clock socket (design/72-host-services.md).
///
/// Every constructor carries a determinism label: `DeterministicSim` rejects a
/// nondeterministic provider at [`RustiaBuilder::build`]; other profiles accept any provider.
/// Cloning shares the underlying provider.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct ServiceClock {
    provider: ClockProvider,
}

impl ServiceClock {
    /// Real system time in seconds since the Unix epoch, anchored to a monotonic instant at
    /// construction so readings never move backwards. Labeled nondeterministic: usable on
    /// `ReproducibleStandard`/`LuauCompat`, rejected on `DeterministicSim`. Not available on
    /// wasm32, where no system clock exists — plug [`ServiceClock::fixed`] or
    /// [`ServiceClock::from_fn`] explicitly.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn system() -> Self {
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |duration| duration.as_secs_f64());
        let start = Instant::now();
        Self {
            provider: ClockProvider::new(move || epoch + start.elapsed().as_secs_f64(), false),
        }
    }

    /// Constant clock that always reads `t` seconds. Labeled deterministic.
    pub fn fixed(t: f64) -> Self {
        Self {
            provider: ClockProvider::new(move || t, true),
        }
    }

    /// Host-supplied clock; the host asserts the readings are deterministic (replay-stable).
    /// The assertion is trusted, not verified — the label stays greppable in host code.
    pub fn from_fn(clock: impl FnMut() -> f64 + 'static) -> Self {
        Self {
            provider: ClockProvider::new(clock, true),
        }
    }

    pub(crate) fn into_provider(self) -> ClockProvider {
        self.provider
    }
}

/// Stock `loadstring` compiler installed by [`RustiaBuilder::live`]: the same parse + compile
/// pipeline the facade uses for ahead-of-time source (public builtin environment), with
/// Luau's `[string "..."]` chunk naming when the script passes no chunk name.
fn stock_loadstring_compiler(
    source: &[u8],
    source_name: Option<&[u8]>,
) -> std::result::Result<Program, String> {
    let source_name = source_name.map_or_else(
        || loadstring_chunk_name(source),
        |name| String::from_utf8_lossy(name).into_owned(),
    );
    let chunk = rustia_parser::parse_source_bytes(source)
        .map_err(|error| format!("{source_name}:1: {error}"))?;
    let options = CompileOptions::default().with_source_bytes(source_name.clone(), source);
    compile_with_options(&chunk, options).map_err(|error| format!("{source_name}:1: {error}"))
}

fn loadstring_chunk_name(source: &[u8]) -> String {
    let mut preview = String::new();
    for byte in source.iter().copied().take(60) {
        match byte {
            b'\\' => preview.push_str("\\\\"),
            b'"' => preview.push_str("\\\""),
            b'\n' => break,
            0x20..=0x7e => preview.push(byte as char),
            _ => preview.push('?'),
        }
    }
    if source.len() > 60 {
        preview.push_str("...");
    }
    format!("[string \"{preview}\"]")
}

#[derive(Clone, Debug)]
pub struct RustiaExtension {
    namespace: Option<String>,
    functions: Vec<HostFunctionOp>,
}

/// Version for the public host-function fuel-cost schedule.
///
/// Bump this when a rustia release changes the meaning of declared host fuel costs.
pub const RUSTIA_HOST_FUEL_ABI_VERSION: u32 = 1;

/// Default fuel cost assigned by capability-specific host-function helpers.
pub const RUSTIA_DEFAULT_HOST_FUNCTION_FUEL: u32 = 1;

/// Minimum accepted host-function fuel cost.
pub const RUSTIA_MIN_HOST_FUNCTION_FUEL: u32 = 1;

/// Public host-function fuel policy for extension metadata.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RustiaHostFuelPolicy {
    /// ABI version for interpreting host-function fuel costs.
    pub abi_version: u32,
    /// Fuel cost used when the helper API does not take an explicit cost.
    pub default_function_fuel: u32,
    /// Smallest fuel cost accepted by VM registration.
    pub minimum_function_fuel: u32,
}

pub const RUSTIA_HOST_FUEL_POLICY: RustiaHostFuelPolicy = RustiaHostFuelPolicy {
    abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
    default_function_fuel: RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
    minimum_function_fuel: RUSTIA_MIN_HOST_FUNCTION_FUEL,
};

/// Public pre-install metadata for a registered host function.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RustiaHostFunctionInfo<'a> {
    pub name: &'a str,
    /// ABI version used to interpret `fuel_cost`.
    pub fuel_abi_version: u32,
    pub fuel_cost: u32,
    pub capability: OpCapability,
    pub parameter_count: u8,
    pub parameter_types: &'a [&'static str],
    pub return_types: &'a [&'static str],
    pub fallible: bool,
}

impl RustiaExtension {
    pub fn new(namespace: impl Into<String>) -> Self {
        let namespace = namespace.into();
        Self {
            namespace: (!namespace.is_empty()).then_some(namespace),
            functions: Vec::new(),
        }
    }

    pub fn global() -> Self {
        Self {
            namespace: None,
            functions: Vec::new(),
        }
    }

    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    pub fn fuel_policy(&self) -> RustiaHostFuelPolicy {
        RUSTIA_HOST_FUEL_POLICY
    }

    pub fn functions(&self) -> impl Iterator<Item = RustiaHostFunctionInfo<'_>> + '_ {
        self.functions
            .iter()
            .map(|function| RustiaHostFunctionInfo {
                name: &function.name,
                fuel_abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
                fuel_cost: function.fuel_cost,
                capability: function.capability,
                parameter_count: function.parameter_count,
                parameter_types: &function.parameter_types,
                return_types: &function.return_types,
                fallible: function.fallible,
            })
    }

    pub fn fn_pure<Func, Args, Return>(self, name: impl Into<String>, function: Func) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.fn_with(
            name,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
            OpCapability::Pure,
            function,
        )
    }

    pub fn fn_input<Func, Args, Return>(self, name: impl Into<String>, function: Func) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.fn_with(
            name,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
            OpCapability::ReadsDeterministicInput,
            function,
        )
    }

    pub fn fn_slot_read<Func, Args, Return>(self, name: impl Into<String>, function: Func) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.fn_with(
            name,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
            OpCapability::ReadsSlotState,
            function,
        )
    }

    pub fn fn_slot_write<Func, Args, Return>(self, name: impl Into<String>, function: Func) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.fn_with(
            name,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
            OpCapability::WritesSlotEffect,
            function,
        )
    }

    pub fn fn_effect<Func, Args, Return>(self, name: impl Into<String>, function: Func) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.fn_with(
            name,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
            OpCapability::EmitsDeterministicEffect,
            function,
        )
    }

    pub fn fn_external_state<Func, Args, Return>(
        self,
        name: impl Into<String>,
        function: Func,
    ) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.fn_with(
            name,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
            OpCapability::ReadsExternalMutableState,
            function,
        )
    }

    pub fn fn_nondeterministic<Func, Args, Return>(
        self,
        name: impl Into<String>,
        function: Func,
    ) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.fn_with(
            name,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
            OpCapability::NondeterministicEffect,
            function,
        )
    }

    pub fn fn_with<Func, Args, Return>(
        mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        capability: OpCapability,
        function: Func,
    ) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.functions
            .push(function.into_host_function(name.into(), fuel_cost, capability));
        self
    }

    pub(crate) fn install(self, rustia: &mut Rustia) -> Result<()> {
        let Some(namespace) = self.namespace else {
            for function in self.functions {
                rustia
                    .vm_mut()
                    .install_host_function(function)
                    .map_err(Error::Vm)?;
            }
            return Ok(());
        };

        let table = rustia.vm_mut().empty_table();
        for function in self.functions {
            let name = function.name.clone();
            let value = rustia
                .vm_mut()
                .register_host_function(function)
                .map_err(Error::Vm)?;
            let key = rustia.vm_mut().string(&name);
            rustia
                .vm_mut()
                .raw_table_set_value(table, key, value)
                .map_err(Error::Vm)?;
        }
        rustia
            .vm_mut()
            .set_global_value(namespace, table)
            .map_err(Error::Vm)
    }
}
