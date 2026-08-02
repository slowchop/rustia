use crate::function::IntoHostFunction;
use crate::{
    Error, GuestSurface, HostSchema, Profile, Result, Rustia, VmError, VmErrorKind, VmSettings,
};
use rustia_compiler::{compile_with_options, CompileOptions};
use rustia_vm::{
    ClockProvider, EntropyProvider, HostFunctionOp, ModuleResolver, OpCapability, Program,
    ResolvedModule, StdoutSink,
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

/// Fluent builder for [`Rustia`]: start from a preset ([`standard`](Self::standard),
/// [`deterministic`](Self::deterministic), [`deterministic_leaf`](Self::deterministic_leaf),
/// [`live`](Self::live)), then override profile, settings, fuel, services, and extensions before
/// [`build`](Self::build).
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

    pub(crate) const fn from_vm(profile: Profile, guest_surface: GuestSurface) -> Self {
        Self::new(profile, guest_surface)
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
        builder.settings.guest_surface = execution_profile.guest_surface;
        builder
    }

    /// Returns the typed core execution semantics currently selected by this
    /// builder.
    pub const fn execution_profile(&self) -> RustiaExecutionProfile {
        RustiaExecutionProfile::new(self.profile, self.settings.guest_surface)
    }

    /// Returns the complete VM settings currently held by this builder.
    ///
    /// Embedders can use this readback to verify that separately versioned
    /// package budgets and services match the values that will reach the VM.
    /// The core profile and guest surface remain available as the stable,
    /// compact [`RustiaExecutionProfile`] identity component.
    pub const fn vm_settings(&self) -> &VmSettings {
        &self.settings
    }

    /// Reproducible-by-default preset: `Profile::ReproducibleStandard` plus a real stdout
    /// `print` sink on native targets (no sink on wasm32, where `print` stays a no-op).
    /// Clocks keep the VM's built-in deterministic defaults — real time is always an explicit
    /// `.wall_clock(ServiceClock::system())` choice, never a default.
    /// [`Rustia::builder()`](crate::Rustia::builder) delegates here.
    pub fn standard() -> Self {
        let builder = Self::new();
        #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
        let builder = builder.stdout(|text| print!("{text}"));
        builder
    }

    /// Sealed-box preset: `Profile::DeterministicSim`, no services. Deterministic providers
    /// can still be plugged explicitly via the service sockets.
    pub fn deterministic() -> Self {
        RustiaExecutionProfile::DETERMINISTIC_STANDARD.builder()
    }

    /// Authoritative stateless-leaf preset.
    ///
    /// This layers [`GuestSurface::HardenedLeaf`] on
    /// [`Profile::DeterministicSim`]. A logical-live memory limit remains
    /// mandatory and must be selected explicitly with
    /// [`memory_limit_bytes`](Self::memory_limit_bytes).
    pub fn deterministic_leaf() -> Self {
        RustiaExecutionProfile::DETERMINISTIC_HARDENED_LEAF.builder()
    }

    /// "Normal Luau" preset with `loadstring` enabled (runtime codegen — attack surface for
    /// hosts running untrusted scripts).
    ///
    /// `Profile::ReproducibleStandard` + host stdout + real wall/monotonic clocks + system
    /// entropy for the initial `math.random` seed + the stock `loadstring` compiler.
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

    /// PUC Lua 5.1 compatibility preset: `Profile::Puc51Compat` plus the stock
    /// `loadstring` compiler, on the same service base as
    /// [`standard`](Self::standard).
    ///
    /// The profile alone reproduces PUC 5.1.5 table layout, number
    /// formatting, and `table.sort`; this preset adds `loadstring` so PUC-era
    /// code that compiles chunks at runtime works out of the box. Runtime
    /// codegen is attack surface for hosts running untrusted scripts — add it
    /// only where PUC source compatibility requires it.
    pub fn puc51_compat() -> Self {
        let mut builder = Self::standard().profile(Profile::Puc51Compat);
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

    /// Sets the VM-wide execution fuel ceiling.
    ///
    /// Direct `Rustia<()>` entry points use this as their top-level budget. A
    /// typed invocation uses the lower of this ceiling and its explicit
    /// [`crate::InvocationLimits`] fuel, across every guest call in that
    /// invocation.
    pub fn fuel(mut self, fuel: u64) -> Self {
        self.settings.initial_fuel = Some(fuel);
        self
    }

    /// Caps VM-owned logical live bytes. The same value has deterministic
    /// semantics on native and wasm32; process RSS and allocator capacity
    /// remain separate host-level measurements.
    pub fn memory_limit_bytes(mut self, bytes: usize) -> Self {
        self.settings.memory_limit_bytes = Some(bytes);
        self
    }

    /// Enables exact instruction-boundary live/reserved peak telemetry.
    pub fn memory_tracking(mut self, enabled: bool) -> Self {
        self.settings.enable_memory_tracking = enabled;
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

    /// Installs the host-owned resolver behind script-visible `require`.
    ///
    /// The resolver chooses a canonical module identity and source for every
    /// request. Successful modules execute once per decision epoch and are
    /// cached by that identity; [`Rustia::reset_decision`] clears the cache.
    pub fn module_resolver(mut self, resolver: ServiceModuleResolver) -> Self {
        self.settings.module_resolver = Some(resolver.into_provider());
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

    /// Plugs the provider used once at VM construction to choose the initial `math.random`
    /// sequence. Calling `math.randomseed` later still replaces that state. A nondeterministic
    /// provider is rejected by [`deterministic`](Self::deterministic).
    pub fn entropy(mut self, entropy: ServiceEntropy) -> Self {
        self.settings.services.entropy = Some(entropy.into_provider());
        self
    }

    pub fn build(self) -> Result<Rustia> {
        self.validate()?;
        if self.settings.guest_surface.is_hardened_leaf() {
            return Err(Error::Vm(VmError::new(
                VmErrorKind::HardenedLeafViolation {
                    feature: "context-free facade",
                },
            )));
        }
        let mut rustia = Rustia::with_settings(self.profile, self.settings);
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
            return Err(Error::Embed(
                "legacy RustiaExtension values cannot be installed into a typed VM; \
                 use HostModule instead"
                    .to_owned(),
            ));
        }
        Ok(Rustia::for_host_with_settings::<H>(
            self.profile,
            self.settings,
        ))
    }

    fn validate(&self) -> Result<()> {
        if self.live_requested
            && (self.settings.services.wall_clock.is_none()
                || self.settings.services.monotonic_clock.is_none()
                || self.settings.services.entropy.is_none())
        {
            return Err(Error::Vm(VmError::new(VmErrorKind::Unsupported {
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
                    return Err(Error::Vm(VmError::new(VmErrorKind::CapabilityRemoved {
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
                return Err(Error::Vm(VmError::new(VmErrorKind::CapabilityRemoved {
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
                return Err(Error::Vm(VmError::new(VmErrorKind::CapabilityRemoved {
                    feature: "non-deterministic module resolver",
                    operation: Some("require".to_owned()),
                    profile: self.profile,
                })));
            }
        }
        if self.settings.guest_surface.is_hardened_leaf() {
            if !self.profile.is_deterministic_sim() {
                return Err(Error::Vm(VmError::new(
                    VmErrorKind::HardenedLeafViolation {
                        feature: "non-DeterministicSim profile",
                    },
                )));
            }
            if self.settings.memory_limit_bytes.is_none() {
                return Err(Error::Vm(VmError::new(
                    VmErrorKind::HardenedLeafViolation {
                        feature: "missing logical-live memory limit",
                    },
                )));
            }
            if self.settings.services.stdout.is_some()
                || self.settings.services.wall_clock.is_some()
                || self.settings.services.monotonic_clock.is_some()
            {
                return Err(Error::Vm(VmError::new(
                    VmErrorKind::HardenedLeafViolation {
                        feature: "host output or clock services",
                    },
                )));
            }
            if self.settings.loadstring_compiler.is_some()
                || self.settings.module_resolver.is_some()
                || self.settings.enable_conformance_coverage
                || self.settings.enable_conformance_top_level_yield
                || self.settings.enable_conformance_int64_userdata
                || self.settings.enable_conformance_native_userdata
                || self.settings.enable_conformance_native_type_checks
                || self.settings.enable_conformance_pcall_helpers
            {
                return Err(Error::Vm(VmError::new(
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
    /// Creates a module whose diagnostic source name matches its canonical
    /// cache identity.
    pub fn new(identity: impl Into<String>, source: impl AsRef<[u8]>) -> Self {
        let identity = identity.into();
        Self {
            source_name: identity.clone(),
            identity,
            source: source.as_ref().to_vec(),
        }
    }

    /// Creates a module with a separate canonical cache identity and
    /// diagnostic source name.
    pub fn named(
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
#[derive(Clone, Debug)]
pub struct ServiceModuleResolver {
    provider: ModuleResolver,
}

impl ServiceModuleResolver {
    /// Host-supplied replay-stable resolver. The determinism assertion is
    /// trusted, not verified.
    pub fn from_fn(
        resolver: impl FnMut(&str, Option<&str>) -> std::result::Result<ModuleSource, String> + 'static,
    ) -> Self {
        Self::from_fn_with_determinism(resolver, true)
    }

    /// Host-supplied live resolver explicitly labeled nondeterministic.
    pub fn from_nondeterministic_fn(
        resolver: impl FnMut(&str, Option<&str>) -> std::result::Result<ModuleSource, String> + 'static,
    ) -> Self {
        Self::from_fn_with_determinism(resolver, false)
    }

    fn from_fn_with_determinism(
        mut resolver: impl FnMut(&str, Option<&str>) -> std::result::Result<ModuleSource, String>
            + 'static,
        deterministic: bool,
    ) -> Self {
        let provider = ModuleResolver::new_with_profile(
            move |requested, referrer, profile| {
                let module = resolver(requested, referrer)?;
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
    profile: Profile,
) -> std::result::Result<ResolvedModule, String> {
    if module.identity.is_empty() {
        return Err("module identity must not be empty".to_owned());
    }
    let chunk = rustia_parser::parse_source_bytes(&module.source)
        .map_err(|error| format!("{}:1: {error}", module.source_name))?;
    let options = CompileOptions {
        puc51_constructor_order: profile == Profile::Puc51Compat,
        ..CompileOptions::default()
    }
    .with_source_bytes(module.source_name.clone(), &module.source);
    let program = compile_with_options(&chunk, options)
        .map_err(|error| format!("{}:1: {error}", module.source_name))?;
    Ok(ResolvedModule::new(module.identity, program))
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

    /// Host-supplied clock; the host asserts the readings are deterministic (replay-stable).
    /// The assertion is trusted, not verified — the label stays greppable in host code.
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
    profile: Profile,
) -> std::result::Result<Program, String> {
    let source_name = source_name.map_or_else(
        || loadstring_chunk_name(source),
        |name| String::from_utf8_lossy(name).into_owned(),
    );
    let chunk = rustia_parser::parse_source_bytes(source)
        .map_err(|error| format!("{source_name}:1: {error}"))?;
    let options = CompileOptions {
        puc51_constructor_order: profile == Profile::Puc51Compat,
        ..CompileOptions::default()
    }
    .with_source_bytes(source_name.clone(), source);
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

        rustia
            .vm_mut()
            .install_host_module(namespace, self.functions)
            .map_err(Error::Vm)
    }
}
