use crate::function::IntoHostFunction;
use crate::{
    Error, GuestSurface, HostSchema, OpCapability, Profile, Result, Rustia, Settings, VmError,
    VmErrorKind,
};
use rustia_compiler::{compile_with_options, CompileOptions};
use rustia_vm::{
    ClockProvider, EntropyProvider, HostFunctionOp, ModuleResolver, Program, ResolvedModule,
    StdoutSink, VmSettings,
};
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use std::time::{Instant, SystemTime, UNIX_EPOCH};
#[cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "web-live"))]
use wasm_bindgen::{prelude::wasm_bindgen, JsCast, JsValue};

#[cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "web-live"))]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = log)]
    fn web_console_log(text: &str);
}

/// Chooses a script environment, resource limits, and host services.
///
/// Start with [`Rustia::builder`] for reproducible defaults, or use
/// [`live`](Self::live) for real clocks, entropy, and `loadstring`.
/// [`deterministic`](Self::deterministic) rejects nondeterministic input providers
/// and host capabilities. None of these presets sets a fuel or memory limit.
///
/// ```
/// use rustia::Rustia;
///
/// let mut lua = Rustia::builder()
///     .fuel(100_000)
///     .memory_limit_bytes(4 * 1024 * 1024)
///     .build()?;
/// assert_eq!(lua.load("return 6 * 7").eval::<i64>()?, 42);
/// # Ok::<(), rustia::Error>(())
/// ```
///
/// Use [`build_for_host`](Self::build_for_host) when callbacks need borrowed
/// per-invocation state. The more restricted
/// [`deterministic_leaf`](Self::deterministic_leaf) preset is for stateless
/// decisions and requires a typed host plus an explicit memory limit.
pub struct RustiaBuilder {
    profile: Profile,
    settings: VmSettings,
    extensions: Vec<RustiaExtension>,
    live_requested: bool,
}

/// Stable, typed description of the core profile and guest surface selected by
/// a builder.
///
/// The descriptor deliberately covers the core [`Profile`] and guest-visible
/// [`GuestSurface`] together. Its canonical bytes are one component of an
/// embedder's package or replay identity; settings, budgets, host ABI, and any
/// permitted services must be bound separately. Unlike a caller-maintained
/// label, the value can also be read back from the built [`Rustia`] VM.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RustiaExecutionProfile {
    profile: Profile,
    guest_surface: GuestSurface,
}

impl RustiaExecutionProfile {
    /// Ordinary sealed deterministic execution.
    pub const DETERMINISTIC_STANDARD: Self =
        Self::new(Profile::DeterministicSim, GuestSurface::Standard);

    /// Stateless authoritative-leaf execution.
    pub const DETERMINISTIC_HARDENED_LEAF: Self =
        Self::new(Profile::DeterministicSim, GuestSurface::HardenedLeaf);

    pub(crate) const fn new(profile: Profile, guest_surface: GuestSurface) -> Self {
        Self {
            profile,
            guest_surface,
        }
    }

    pub(crate) const fn from_vm(
        profile: rustia_vm::Profile,
        guest_surface: rustia_vm::GuestSurface,
    ) -> Self {
        Self::new(
            Profile::from_core(profile),
            GuestSurface::from_core(guest_surface),
        )
    }

    /// Constructs a builder whose core VM semantics exactly match this
    /// descriptor. Further builder methods may add budgets or services; the
    /// final descriptor remains readable through
    /// [`RustiaBuilder::execution_profile`].
    pub fn builder(self) -> RustiaBuilder {
        RustiaBuilder::from_execution_profile(self)
    }

    /// Stable compact code for fixed-width host protocol fields.
    ///
    /// Codes name the complete `(Profile, GuestSurface)` pair rather than
    /// relying on Rust enum discriminants.
    pub const fn compatibility_id(self) -> u32 {
        match (self.profile, self.guest_surface) {
            (Profile::DeterministicSim, GuestSurface::HardenedLeaf) => 1,
            (Profile::LuauCompat, GuestSurface::Standard) => 2,
            (Profile::ReproducibleStandard, GuestSurface::Standard) => 3,
            (Profile::DeterministicSim, GuestSurface::Standard) => 4,
            (Profile::LuauCompat, GuestSurface::HardenedLeaf) => 5,
            (Profile::ReproducibleStandard, GuestSurface::HardenedLeaf) => 6,
            (Profile::Puc51Compat, GuestSurface::Standard) => 7,
            (Profile::Puc51Compat, GuestSurface::HardenedLeaf) => 8,
        }
    }

    /// Canonical versioned bytes for package and replay identity components.
    ///
    /// Each field is an explicitly assigned little-endian `u32`: encoding
    /// version, core-profile tag, then guest-surface tag.
    pub const fn identity_bytes(self) -> [u8; 12] {
        let encoding = 1u32.to_le_bytes();
        let profile = match self.profile {
            Profile::LuauCompat => 1u32,
            Profile::ReproducibleStandard => 2,
            Profile::DeterministicSim => 3,
            Profile::Puc51Compat => 4,
        }
        .to_le_bytes();
        let surface = match self.guest_surface {
            GuestSurface::Standard => 1u32,
            GuestSurface::HardenedLeaf => 2,
        }
        .to_le_bytes();
        [
            encoding[0],
            encoding[1],
            encoding[2],
            encoding[3],
            profile[0],
            profile[1],
            profile[2],
            profile[3],
            surface[0],
            surface[1],
            surface[2],
            surface[3],
        ]
    }
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

    fn from_execution_profile(execution_profile: RustiaExecutionProfile) -> Self {
        let mut builder = Self::new();
        builder.profile = execution_profile.profile;
        builder.settings.guest_surface = execution_profile.guest_surface.to_core();
        builder
    }

    /// Returns the typed core execution semantics currently selected by this
    /// builder.
    pub const fn execution_profile(&self) -> RustiaExecutionProfile {
        RustiaExecutionProfile::new(
            self.profile,
            GuestSurface::from_core(self.settings.guest_surface),
        )
    }

    /// Returns the effective limits and service declarations selected so far.
    ///
    /// Use [`Settings::identity_bytes`] when configuration is part of a replay
    /// or package identity. Provider data and application code remain separate
    /// identity inputs.
    pub fn settings(&self) -> Settings {
        Settings::from_core(self.profile, &self.settings)
    }

    #[cfg(test)]
    pub(crate) const fn vm_settings(&self) -> &VmSettings {
        &self.settings
    }

    /// Starts with reproducible inputs and the standard script environment.
    ///
    /// This uses [`Profile::ReproducibleStandard`]: clocks use deterministic
    /// defaults, the random seed is zero, and `loadstring` is disabled. Script
    /// `print` writes to stdout on native/WASI targets and is silent on browser
    /// Wasm until a sink is supplied. Real clocks and other nondeterministic
    /// providers remain available when explicitly configured.
    /// [`Rustia::builder`] starts with this preset.
    pub fn standard() -> Self {
        let builder = Self::new();
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        let builder = builder.stdout(|text| print!("{text}"));
        builder
    }

    /// Starts with [`Profile::DeterministicSim`] and no host services.
    ///
    /// Nondeterministic input providers and host capabilities are rejected.
    /// Replay-stable providers and a `print` sink can still be added explicitly.
    pub fn deterministic() -> Self {
        RustiaExecutionProfile::DETERMINISTIC_STANDARD.builder()
    }

    /// Starts a restricted environment for independent, stateless decisions.
    ///
    /// Uses [`GuestSurface::HardenedLeaf`] with [`Profile::DeterministicSim`].
    /// Set an explicit [`memory_limit_bytes`](Self::memory_limit_bytes) and use
    /// [`build_for_host`](Self::build_for_host). Execute host-owned decision
    /// bodies with [`Rustia::compile_entry`] and [`Rustia::invoke_entry_into`].
    pub fn deterministic_leaf() -> Self {
        RustiaExecutionProfile::DETERMINISTIC_HARDENED_LEAF.builder()
    }

    /// Starts with real clocks, a fresh random seed, output, and `loadstring`.
    ///
    /// Uses [`Profile::ReproducibleStandard`] with nondeterministic services;
    /// results are not guaranteed to repeat. `loadstring` lets scripts compile
    /// additional code at runtime, so choose this preset deliberately for
    /// untrusted scripts.
    ///
    /// Native and WASI targets use their system services. With the default `web-live` feature,
    /// `wasm32-unknown-unknown` uses the JavaScript host (`console`, `Date`, `performance`, and
    /// `crypto`). Bare-Wasm builds can disable default features and plug the three input
    /// providers explicitly; [`build`](Self::build) then verifies that none are missing.
    pub fn live() -> Self {
        let builder = Self::standard();
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        let builder = builder
            .wall_clock(ServiceClock::system())
            .monotonic_clock(ServiceClock::monotonic())
            .entropy(ServiceEntropy::system());
        #[cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "web-live"))]
        let builder = builder
            .stdout(web_console_log)
            .wall_clock(ServiceClock::system())
            .monotonic_clock(ServiceClock::monotonic())
            .entropy(ServiceEntropy::system());
        let mut builder = builder;
        builder.live_requested = true;
        builder.settings.loadstring_compiler = Some(stock_loadstring_compiler);
        builder
    }

    /// Starts a Lua 5.1 compatibility environment with `loadstring` enabled.
    ///
    /// The profile alone reproduces PUC 5.1.5 table layout, number
    /// formatting, and `table.sort`; this preset adds `loadstring` so PUC-era
    /// code that compiles chunks at runtime works out of the box. Runtime
    /// compilation is an extra capability for untrusted scripts; enable it
    /// where Lua 5.1 source compatibility requires it. Host services otherwise
    /// use the defaults from [`standard`](Self::standard).
    pub fn puc51_compat() -> Self {
        let mut builder = Self::standard().profile(Profile::Puc51Compat);
        builder.settings.loadstring_compiler = Some(stock_loadstring_compiler);
        builder
    }

    /// Selects the core compatibility and capability policy.
    /// Existing settings and services are retained and checked when building.
    pub fn profile(mut self, profile: Profile) -> Self {
        self.profile = profile;
        self
    }

    #[cfg(test)]
    pub(crate) fn raw_settings(mut self, settings: VmSettings) -> Self {
        self.settings = settings;
        self
    }

    /// Configures host-side reports independently of guest heap and fuel limits.
    pub fn diagnostic_limits(mut self, limits: crate::DiagnosticLimits) -> Self {
        self.settings.diagnostic_limits = limits.to_core();
        self
    }

    /// Limits the instruction work allowed during execution.
    ///
    /// Direct `Rustia<()>` entry points use this as their top-level budget. A
    /// typed invocation uses the lower of this ceiling and its explicit
    /// [`crate::InvocationLimits`] fuel, across every guest call in that
    /// invocation. Fuel measures deterministic work rather than elapsed time;
    /// script `pcall` cannot catch fuel exhaustion. The default is unlimited.
    pub fn fuel(mut self, fuel: u64) -> Self {
        self.settings.initial_fuel = Some(fuel);
        self
    }

    /// Limits nested script calls. The default and maximum are 512.
    ///
    /// Larger values are clamped to the engine maximum. Exhaustion returns a
    /// distinct error that script `pcall` cannot catch.
    pub fn recursion_limit(mut self, limit: usize) -> Self {
        self.settings.recursion_limit = limit.min(rustia_vm::MAX_RECURSION_LIMIT);
        self
    }

    /// Limits the VM's accounted live memory. The default is unlimited.
    ///
    /// Accounting is deterministic across native and Wasm targets. It does not
    /// bound total process memory, allocator capacity, or memory captured by
    /// Rust callbacks. Allocations can fail once the limit is reached.
    pub fn memory_limit_bytes(mut self, bytes: usize) -> Self {
        self.settings.memory_limit_bytes = Some(bytes);
        self
    }

    /// Enables tracking of live and reserved memory peaks at instruction boundaries.
    pub fn memory_tracking(mut self, enabled: bool) -> Self {
        self.settings.enable_memory_tracking = enabled;
        self
    }

    /// Adds a group of Rust callbacks to install when the VM is built.
    /// For a typed host, use [`crate::HostModule`] after building instead.
    pub fn extension(mut self, extension: RustiaExtension) -> Self {
        self.extensions.push(extension);
        self
    }

    /// Chooses where script `print` output goes.
    ///
    /// Each call sends one string, with arguments joined by tabs and a trailing
    /// newline. The closure cannot return an error. Output sinks are allowed in
    /// every ordinary profile, including [`Profile::DeterministicSim`].
    pub fn stdout(mut self, sink: impl FnMut(&str) + 'static) -> Self {
        self.settings.services.stdout = Some(StdoutSink::new(sink));
        self
    }

    /// Discards script `print` output by removing the configured sink.
    pub fn without_stdout(mut self) -> Self {
        self.settings.services.stdout = None;
        self
    }

    /// Installs the host-owned resolver behind script-visible `require`.
    ///
    /// The resolver chooses a canonical module identity and source for every
    /// request. Successful modules execute once per decision epoch and are
    /// cached by that identity; [`Rustia::reset_decision`] clears the cache.
    pub fn module_resolver(mut self, resolver: ServiceModuleResolver) -> Self {
        self.settings.module_resolver = Some(resolver.into_provider());
        self
    }

    /// Supplies the clock for no-argument `os.time()` and the default `os.date`
    /// timestamp. A nondeterministic provider is rejected at [`build`](Self::build) on
    /// `DeterministicSim`.
    pub fn wall_clock(mut self, clock: ServiceClock) -> Self {
        self.settings.services.wall_clock = Some(clock.into_provider());
        self
    }

    /// Supplies the clock for `os.clock()`, overriding the VM's built-in
    /// logical clock. Same `DeterministicSim` validation as [`wall_clock`](Self::wall_clock).
    pub fn monotonic_clock(mut self, clock: ServiceClock) -> Self {
        self.settings.services.monotonic_clock = Some(clock.into_provider());
        self
    }

    /// Supplies the seed read once at VM construction for the initial `math.random`
    /// sequence. Calling `math.randomseed` later still replaces that state. A nondeterministic
    /// provider is rejected by [`deterministic`](Self::deterministic).
    pub fn entropy(mut self, entropy: ServiceEntropy) -> Self {
        self.settings.services.entropy = Some(entropy.into_provider());
        self
    }

    /// Validates the configuration and builds a VM without a typed host context.
    ///
    /// Returns an error for incompatible services, invalid extensions, or a
    /// hardened-leaf preset, which requires [`build_for_host`](Self::build_for_host).
    pub fn build(self) -> Result<Rustia> {
        self.validate()?;
        if self.settings.guest_surface.is_hardened_leaf() {
            return Err(Error::from(VmError::new(
                VmErrorKind::HardenedLeafViolation {
                    feature: "context-free facade",
                },
            )));
        }
        let mut rustia = Rustia::from_settings(self.profile, self.settings);
        for extension in self.extensions {
            rustia.install_extension(extension)?;
        }
        Ok(rustia)
    }

    /// Builds a typed VM while preserving this builder's profile, settings,
    /// and host services.
    ///
    /// Legacy [`RustiaExtension`] values deliberately do not cross into the
    /// typed capability surface; install a [`crate::HostModule<H>`] after
    /// building instead.
    pub fn build_for_host<H>(self) -> Result<Rustia<H>>
    where
        H: HostSchema,
    {
        self.validate()?;
        if !self.extensions.is_empty() {
            return Err(Error::embed(
                "legacy RustiaExtension values cannot be installed into a typed VM; \
                 use HostModule instead"
                    .to_owned(),
            ));
        }
        Ok(Rustia::<H>::from_settings(self.profile, self.settings))
    }

    fn validate(&self) -> Result<()> {
        if self.live_requested
            && (self.settings.services.wall_clock.is_none()
                || self.settings.services.monotonic_clock.is_none()
                || self.settings.services.entropy.is_none())
        {
            return Err(Error::from(VmError::new(VmErrorKind::Unsupported {
                feature: "live() without a complete platform adapter (plug \
                          .wall_clock/.monotonic_clock/.entropy providers, or enable the \
                          default web-live feature for browser/JavaScript wasm hosts)",
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
                    return Err(Error::from(VmError::new(VmErrorKind::CapabilityRemoved {
                        feature,
                        operation: None,
                        profile: self.profile,
                    })));
                }
            }
            if self
                .settings
                .services
                .entropy
                .as_ref()
                .is_some_and(|entropy| !entropy.deterministic())
            {
                return Err(Error::from(VmError::new(VmErrorKind::CapabilityRemoved {
                    feature: "non-deterministic entropy provider",
                    operation: None,
                    profile: self.profile,
                })));
            }
            if self
                .settings
                .module_resolver
                .as_ref()
                .is_some_and(|resolver| !resolver.deterministic())
            {
                return Err(Error::from(VmError::new(VmErrorKind::CapabilityRemoved {
                    feature: "non-deterministic module resolver",
                    operation: Some("require".to_owned()),
                    profile: self.profile,
                })));
            }
        }
        if self.settings.guest_surface.is_hardened_leaf() {
            if !self.profile.is_deterministic_sim() {
                return Err(Error::from(VmError::new(
                    VmErrorKind::HardenedLeafViolation {
                        feature: "non-DeterministicSim profile",
                    },
                )));
            }
            if self.settings.memory_limit_bytes.is_none() {
                return Err(Error::from(VmError::new(
                    VmErrorKind::HardenedLeafViolation {
                        feature: "missing logical-live memory limit",
                    },
                )));
            }
            if self.settings.services.stdout.is_some()
                || self.settings.services.wall_clock.is_some()
                || self.settings.services.monotonic_clock.is_some()
                || self.settings.services.entropy.is_some()
            {
                return Err(Error::from(VmError::new(
                    VmErrorKind::HardenedLeafViolation {
                        feature: "host output, clock, or entropy services",
                    },
                )));
            }
            if self.settings.loadstring_compiler.is_some()
                || self.settings.module_resolver.is_some()
                || self.settings.enable_conformance_coverage
                || self.settings.enable_opcode_census
                || self.settings.enable_conformance_top_level_yield
                || self.settings.enable_conformance_int64_userdata
                || self.settings.enable_conformance_native_userdata
                || self.settings.enable_conformance_native_type_checks
                || self.settings.enable_conformance_pcall_helpers
            {
                return Err(Error::from(VmError::new(
                    VmErrorKind::HardenedLeafViolation {
                        feature: "dynamic-code or conformance-only facilities",
                    },
                )));
            }
        }
        Ok(())
    }
}

/// Host-supplied source returned by a [`ServiceModuleResolver`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleSource {
    identity: String,
    source_name: String,
    source: Vec<u8>,
}

impl ModuleSource {
    /// Creates a module whose diagnostic and resolver-referrer source name
    /// matches its canonical cache identity.
    pub fn new(identity: impl Into<String>, source: impl AsRef<[u8]>) -> Self {
        let identity = identity.into();
        Self {
            source_name: identity.clone(),
            identity,
            source: source.as_ref().to_vec(),
        }
    }

    /// Creates a module with a separate canonical cache identity and source
    /// name. The source name is used for diagnostics and nested `require`
    /// resolver referrers.
    pub fn with_source_name(
        identity: impl Into<String>,
        source_name: impl Into<String>,
        source: impl AsRef<[u8]>,
    ) -> Self {
        Self {
            identity: identity.into(),
            source_name: source_name.into(),
            source: source.as_ref().to_vec(),
        }
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }

    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    pub fn source(&self) -> &[u8] {
        &self.source
    }
}

/// Host-owned resolver for script-visible `require`.
///
/// Resolution is performed on demand with the requested path and the current
/// chunk's source name. The host must return a stable canonical identity when
/// it labels the resolver deterministic. Because the canonical identity is a
/// resolver result, the resolver is called before every cache lookup; a host
/// with expensive I/O or compilation should keep its own source cache. A cache
/// hit skips module execution and discards the newly resolved program.
///
/// A module may return zero or one value. Successful results are cached within
/// the calling function's module cohort; failures can be retried. Module code
/// running in a guest coroutine may yield and resume before its result is
/// cached. Recursive loading is an error, as is requesting an identity that a
/// different coroutine is still loading.
///
/// Ordinary `require` keeps completed writes when a module fails. An enclosing
/// [`Chunk::install`](crate::Chunk::install) still rolls back guest state when
/// the installation fails; external Rust effects remain outside that rollback.
///
/// ```
/// use rustia::{ModuleSource, RustiaBuilder, ServiceModuleResolver};
///
/// # fn main() -> rustia::Result<()> {
/// let resolver = ServiceModuleResolver::from_fn(|name, _referrer| {
///     match name {
///         "answer" => Ok(ModuleSource::new("answer", "return 42")),
///         _ => Err(rustia::Error::embed(format!("unknown module {name}"))),
///     }
/// });
/// let mut lua = RustiaBuilder::deterministic().module_resolver(resolver).build()?;
/// assert_eq!(lua.load("return require('answer')").eval::<i64>()?, 42);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct ServiceModuleResolver {
    provider: ModuleResolver,
}

impl ServiceModuleResolver {
    /// Host-supplied replay-stable resolver. The determinism assertion is
    /// trusted, not verified.
    pub fn from_fn(
        resolver: impl FnMut(&str, Option<&str>) -> Result<ModuleSource> + 'static,
    ) -> Self {
        Self::from_fn_with_determinism(resolver, true)
    }

    /// Host-supplied live resolver explicitly labeled nondeterministic.
    pub fn from_nondeterministic_fn(
        resolver: impl FnMut(&str, Option<&str>) -> Result<ModuleSource> + 'static,
    ) -> Self {
        Self::from_fn_with_determinism(resolver, false)
    }

    fn from_fn_with_determinism(
        mut resolver: impl FnMut(&str, Option<&str>) -> Result<ModuleSource> + 'static,
        deterministic: bool,
    ) -> Self {
        let provider = ModuleResolver::new_with_profile(
            move |requested, referrer, profile| {
                let module =
                    resolver(requested, referrer).map_err(crate::value::embed_to_vm_error)?;
                compile_module_source(module, profile)
            },
            deterministic,
        );
        Self { provider }
    }

    fn into_provider(self) -> ModuleResolver {
        self.provider
    }
}

fn compile_module_source(
    module: ModuleSource,
    profile: rustia_vm::Profile,
) -> rustia_vm::Result<ResolvedModule> {
    if module.identity.is_empty() {
        return Err(crate::value::embed_to_vm_error(Error::embed(
            "module identity must not be empty",
        )));
    }
    let chunk = rustia_parser::parse_source_bytes(&module.source).map_err(|error| {
        module_compile_error(
            Error::from_engine(error),
            &module.source_name,
            &module.source,
        )
    })?;
    let options = CompileOptions {
        puc51_constructor_order: profile == rustia_vm::Profile::Puc51Compat,
        ..CompileOptions::default()
    }
    .with_source_bytes(module.source_name.clone(), &module.source);
    let mut program = compile_with_options(&chunk, options).map_err(|error| {
        module_compile_error(
            Error::from_engine(error),
            &module.source_name,
            &module.source,
        )
    })?;
    program.diagnostic_source = rustia_vm::diagnostic::SourceSnapshot::copy_with_limit(
        &module.source,
        crate::DiagnosticLimits::default().source_bytes,
    );
    Ok(ResolvedModule::new(module.identity, program))
}

fn module_compile_error(error: Error, name: &str, source: &[u8]) -> rustia_vm::Error {
    let message = format!("{name}:1: {error}");
    let mut error = error.with_source(name, source, crate::DiagnosticLimits::default());
    // Keep a single snapshot lease at the outer diagnostic, not an unbudgeted
    // copy hidden in the typed cause chain.
    let mut diagnostic = error.diagnostic.take();
    let mut error = crate::value::embed_to_vm_error(error);
    if let Some(diagnostic) = &mut diagnostic {
        std::sync::Arc::make_mut(diagnostic).host_cause = error
            .diagnostic
            .as_ref()
            .and_then(|diagnostic| diagnostic.host_cause.clone());
    }
    error.diagnostic = diagnostic.map(|diagnostic| std::sync::Arc::new(diagnostic.to_core()));
    if let rustia_vm::ErrorKind::RuntimeError { message: raw } = &mut error.kind {
        *raw = message;
    }
    error
}

/// A clock supplied to [`RustiaBuilder::wall_clock`] or
/// [`RustiaBuilder::monotonic_clock`].
///
/// Every constructor declares whether readings repeat during replay.
/// [`Profile::DeterministicSim`] rejects nondeterministic providers when built;
/// other profiles accept either kind.
/// Cloning shares the underlying provider.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct ServiceClock {
    provider: ClockProvider,
}

impl ServiceClock {
    /// Real system time in seconds since the Unix epoch, anchored to a monotonic instant at
    /// construction so readings never move backwards. Labeled nondeterministic: usable on
    /// `ReproducibleStandard`/`LuauCompat`, rejected on `DeterministicSim`.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    pub fn system() -> Self {
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |duration| duration.as_secs_f64());
        let start = Instant::now();
        Self {
            provider: ClockProvider::new(move || epoch + start.elapsed().as_secs_f64(), false),
        }
    }

    /// Monotonic elapsed time in seconds. Labeled nondeterministic.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    pub fn monotonic() -> Self {
        let start = Instant::now();
        Self {
            provider: ClockProvider::new(move || start.elapsed().as_secs_f64(), false),
        }
    }

    /// JavaScript host wall time in seconds since the Unix epoch. Labeled nondeterministic.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "web-live"))]
    pub fn system() -> Self {
        Self {
            provider: ClockProvider::new(|| js_sys::Date::now() / 1_000.0, false),
        }
    }

    /// JavaScript host monotonic time in seconds. Uses `performance.now()` when available and
    /// falls back to `Date.now()`. Labeled nondeterministic.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "web-live"))]
    pub fn monotonic() -> Self {
        Self {
            provider: ClockProvider::new(web_monotonic_seconds, false),
        }
    }

    /// Constant clock that always reads `t` seconds. Labeled deterministic.
    pub fn fixed(t: f64) -> Self {
        Self {
            provider: ClockProvider::new(move || t, true),
        }
    }

    /// Supplies a clock whose readings the host promises to reproduce during replay.
    /// Rustia trusts this declaration and does not verify it.
    pub fn from_fn(clock: impl FnMut() -> f64 + 'static) -> Self {
        Self {
            provider: ClockProvider::new(clock, true),
        }
    }

    /// Host-supplied live clock. The provider is explicitly labeled nondeterministic.
    pub fn from_nondeterministic_fn(clock: impl FnMut() -> f64 + 'static) -> Self {
        Self {
            provider: ClockProvider::new(clock, false),
        }
    }

    pub(crate) fn into_provider(self) -> ClockProvider {
        self.provider
    }
}

/// Host entropy used to select the VM's initial `math.random` sequence.
///
/// Like [`ServiceClock`], every constructor carries a determinism label. The provider is read
/// once while the VM is built; `math.randomseed` remains available afterward.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct ServiceEntropy {
    provider: EntropyProvider,
}

impl ServiceEntropy {
    /// System entropy where Rustia has a default platform adapter. If the platform random
    /// source is temporarily unavailable, a time-varying fallback still preserves `live()`'s
    /// no-replay-guarantee contract. Labeled nondeterministic.
    #[cfg(any(
        not(all(target_arch = "wasm32", target_os = "unknown")),
        all(target_arch = "wasm32", target_os = "unknown", feature = "web-live")
    ))]
    pub fn system() -> Self {
        let mut fallback_counter = 0_u64;
        Self {
            provider: EntropyProvider::new(
                move || {
                    system_entropy().unwrap_or_else(|| {
                        fallback_counter = fallback_counter.wrapping_add(1);
                        fallback_entropy(fallback_counter)
                    })
                },
                false,
            ),
        }
    }

    /// Constant replay-stable seed.
    pub fn fixed(seed: u64) -> Self {
        Self {
            provider: EntropyProvider::new(move || seed, true),
        }
    }

    /// Host-supplied replay-stable seed provider.
    pub fn from_fn(entropy: impl FnMut() -> u64 + 'static) -> Self {
        Self {
            provider: EntropyProvider::new(entropy, true),
        }
    }

    /// Host-supplied live seed provider, explicitly labeled nondeterministic.
    pub fn from_nondeterministic_fn(entropy: impl FnMut() -> u64 + 'static) -> Self {
        Self {
            provider: EntropyProvider::new(entropy, false),
        }
    }

    pub(crate) fn into_provider(self) -> EntropyProvider {
        self.provider
    }
}

#[cfg(feature = "web-live")]
fn system_entropy() -> Option<u64> {
    getrandom::u64().ok()
}

#[cfg(all(
    not(feature = "web-live"),
    not(all(target_arch = "wasm32", target_os = "unknown"))
))]
fn system_entropy() -> Option<u64> {
    None
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn fallback_entropy(counter: u64) -> u64 {
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0_u128, |duration| duration.as_nanos());
    (wall as u64) ^ ((wall >> 64) as u64) ^ counter.rotate_left(29)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "web-live"))]
fn fallback_entropy(counter: u64) -> u64 {
    js_sys::Date::now().to_bits() ^ web_monotonic_seconds().to_bits() ^ counter.rotate_left(29)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "web-live"))]
fn web_monotonic_seconds() -> f64 {
    let global = js_sys::global();
    let performance = js_sys::Reflect::get(&global, &JsValue::from_str("performance"));
    let Ok(performance) = performance else {
        return js_sys::Date::now() / 1_000.0;
    };
    let now = js_sys::Reflect::get(&performance, &JsValue::from_str("now"))
        .ok()
        .and_then(|value| value.dyn_into::<js_sys::Function>().ok())
        .and_then(|now| now.call0(&performance).ok())
        .and_then(|value| value.as_f64());
    now.unwrap_or_else(js_sys::Date::now) / 1_000.0
}

/// Stock `loadstring` compiler installed by [`RustiaBuilder::live`]: the same parse + compile
/// pipeline the facade uses for ahead-of-time source (public builtin environment), with
/// Luau's `[string "..."]` chunk naming when the script passes no chunk name.
///
/// The VM supplies its active [`Profile`] so profile-dependent codegen stays
/// consistent between ahead-of-time and runtime-compiled chunks (currently the
/// PUC 5.1 table-constructor write order under `Profile::Puc51Compat`).
fn stock_loadstring_compiler(
    source: &[u8],
    source_name: Option<&[u8]>,
    profile: rustia_vm::Profile,
) -> std::result::Result<Program, String> {
    let source_name = source_name.map_or_else(
        || loadstring_chunk_name(source),
        |name| String::from_utf8_lossy(name).into_owned(),
    );
    let chunk = rustia_parser::parse_source_bytes(source)
        .map_err(|error| format!("{source_name}:1: {error}"))?;
    let options = CompileOptions {
        puc51_constructor_order: profile == rustia_vm::Profile::Puc51Compat,
        ..CompileOptions::default()
    }
    .with_source_bytes(source_name.clone(), source);
    let mut program = compile_with_options(&chunk, options)
        .map_err(|error| format!("{source_name}:1: {error}"))?;
    program.diagnostic_source = rustia_vm::diagnostic::SourceSnapshot::copy_with_limit(
        source,
        crate::DiagnosticLimits::default().source_bytes,
    );
    Ok(program)
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

/// A group of Rust callbacks installed together, optionally in a named table.
///
/// Add it with [`RustiaBuilder::extension`] or [`Rustia::install_extension`].
/// For callbacks that borrow per-invocation host state, use [`crate::HostModule`]
/// instead. The capability labels here describe captured host state; they do
/// not provide a borrowed host context or make captured mutations transactional.
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

/// The fuel policy used by extension registration helpers.
pub const RUSTIA_HOST_FUEL_POLICY: RustiaHostFuelPolicy = RustiaHostFuelPolicy {
    abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
    default_function_fuel: RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
    minimum_function_fuel: RUSTIA_MIN_HOST_FUNCTION_FUEL,
};

/// Public pre-install metadata for a registered host function.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RustiaHostFunctionInfo<'a> {
    /// The binding name inside the extension's namespace, or a global name.
    pub name: &'a str,
    /// ABI version used to interpret `fuel_cost`.
    pub fuel_abi_version: u32,
    /// Fuel charged per call.
    pub fuel_cost: u32,
    /// The callback's declared access to host state and effects.
    pub capability: OpCapability,
    /// The number of typed Rust parameters.
    pub parameter_count: u8,
    /// Rust type names for the callback parameters.
    pub parameter_types: &'a [&'static str],
    /// Rust type names for the callback results.
    pub return_types: &'a [&'static str],
    /// Whether the callback returns a Rustia `Result`.
    pub fallible: bool,
}

impl RustiaExtension {
    /// Creates callbacks to install under a named table. An empty name selects globals.
    pub fn new(namespace: impl Into<String>) -> Self {
        let namespace = namespace.into();
        Self {
            namespace: (!namespace.is_empty()).then_some(namespace),
            functions: Vec::new(),
        }
    }

    /// Creates callbacks to install directly as globals.
    pub fn global() -> Self {
        Self {
            namespace: None,
            functions: Vec::new(),
        }
    }

    /// Returns the table name, or `None` for global callbacks.
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// Returns the fuel policy used by these registration helpers.
    pub fn fuel_policy(&self) -> RustiaHostFuelPolicy {
        RUSTIA_HOST_FUEL_POLICY
    }

    /// Describes the callbacks before installation.
    pub fn functions(&self) -> impl Iterator<Item = RustiaHostFunctionInfo<'_>> + '_ {
        self.functions
            .iter()
            .map(|function| RustiaHostFunctionInfo {
                name: &function.name,
                fuel_abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
                fuel_cost: function.fuel_cost,
                capability: OpCapability::from_core(function.capability),
                parameter_count: function.parameter_count,
                parameter_types: &function.parameter_types,
                return_types: &function.return_types,
                fallible: function.fallible,
            })
    }

    /// Adds a callback declared free of external mutable state, at the default fuel cost.
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

    /// Adds a callback declared to read deterministic input, at the default fuel cost.
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

    /// Adds a callback declared to read slot state, at the default fuel cost.
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

    /// Adds a callback declared to write slot effects, at the default fuel cost.
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

    /// Adds a callback declared to emit deterministic effects, at the default fuel cost.
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

    /// Adds a callback that reads external mutable state. Deterministic profiles reject it.
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

    /// Adds a nondeterministic callback. Deterministic profiles reject it.
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

    /// Adds a callback with an explicit capability and nonzero fuel cost.
    /// Invalid costs and disallowed capabilities are rejected during installation.
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
        self.functions.push(
            crate::HostFunction::<()>::with_capability(capability, function)
                .fuel_cost(fuel_cost)
                .into_op(name.into()),
        );
        self
    }

    pub(crate) fn install(self, rustia: &mut Rustia) -> Result<()> {
        let Some(namespace) = self.namespace else {
            return rustia
                .vm_mut()
                .install_host_functions(self.functions)
                .map_err(Error::from_engine);
        };

        rustia
            .vm_mut()
            .install_host_module(namespace, self.functions)
            .map_err(Error::from_engine)
    }
}
