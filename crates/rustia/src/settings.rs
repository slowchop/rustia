//! Application-facing settings and memory measurements.

use crate::{DiagnosticLimits, GuestSurface, Profile, RustiaExecutionProfile};

/// A host provider's declared reproducibility contract.
///
/// Rustia trusts this declaration; it cannot inspect the provider's captured
/// state or prove that its answers repeat.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceDeterminism {
    /// The application promises repeatable answers for the same execution.
    ReplayStable,
    /// Answers may depend on clocks, external state, or fresh entropy.
    Live,
}

impl ServiceDeterminism {
    const fn from_deterministic(deterministic: bool) -> Self {
        if deterministic {
            Self::ReplayStable
        } else {
            Self::Live
        }
    }
}

/// An immutable description of a builder's or interpreter's configuration.
///
/// Read this through [`crate::RustiaBuilder::settings`] before construction or
/// [`crate::Rustia::settings`] afterward. Configure the interpreter through its
/// builder.
///
/// [`identity_bytes`](Self::identity_bytes) also covers internal settings that
/// applications cannot configure. It is useful when binding a replay or a
/// script package to an exact configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    profile: Profile,
    guest_surface: GuestSurface,
    fuel: Option<u64>,
    recursion_limit: usize,
    memory_limit_bytes: Option<usize>,
    memory_tracking: bool,
    diagnostic_limits: DiagnosticLimits,
    runtime_compilation: bool,
    stdout: bool,
    wall_clock: Option<ServiceDeterminism>,
    monotonic_clock: Option<ServiceDeterminism>,
    entropy: Option<ServiceDeterminism>,
    module_resolver: Option<ServiceDeterminism>,
    identity: Vec<u8>,
}

impl Settings {
    /// Returns the selected language compatibility and host capability policy.
    pub const fn profile(&self) -> Profile {
        self.profile
    }

    /// Returns the script environment selected by the execution preset.
    pub const fn guest_surface(&self) -> GuestSurface {
        self.guest_surface
    }

    /// Returns the instruction-fuel ceiling, or `None` when unlimited.
    pub const fn fuel(&self) -> Option<u64> {
        self.fuel
    }

    /// Returns the maximum script call depth.
    pub const fn recursion_limit(&self) -> usize {
        self.recursion_limit
    }

    /// Returns the logical-live script memory limit, or `None` when unlimited.
    pub const fn memory_limit_bytes(&self) -> Option<usize> {
        self.memory_limit_bytes
    }

    /// Returns whether instruction-boundary memory peaks are recorded.
    pub const fn memory_tracking(&self) -> bool {
        self.memory_tracking
    }

    /// Returns the separate host-side error reporting budgets.
    pub const fn diagnostic_limits(&self) -> DiagnosticLimits {
        self.diagnostic_limits
    }

    /// Returns whether scripts may compile more code through `loadstring`.
    pub const fn runtime_compilation(&self) -> bool {
        self.runtime_compilation
    }

    /// Returns whether `print` has a host output sink.
    pub const fn stdout(&self) -> bool {
        self.stdout
    }

    /// Returns the wall-clock provider's declaration, or `None` for the default.
    pub const fn wall_clock(&self) -> Option<ServiceDeterminism> {
        self.wall_clock
    }

    /// Returns the monotonic-clock declaration, or `None` for the logical clock.
    pub const fn monotonic_clock(&self) -> Option<ServiceDeterminism> {
        self.monotonic_clock
    }

    /// Returns the seed provider's declaration, or `None` for the zero seed.
    pub const fn entropy(&self) -> Option<ServiceDeterminism> {
        self.entropy
    }

    /// Returns the module resolver's declaration, or `None` when absent.
    pub const fn module_resolver(&self) -> Option<ServiceDeterminism> {
        self.module_resolver
    }

    /// Returns canonical, versioned bytes describing the effective settings.
    ///
    /// The bytes cover the profile, guest surface, every resource and reporting
    /// limit, internal execution options, runtime-compiler presence, and host
    /// provider presence and reproducibility declarations. Integers have fixed
    /// widths and little-endian encoding, independent of the machine's pointer
    /// width. Treat the result as opaque; a new encoding version may change it.
    ///
    /// This does not identify source code, the Rustia build, callback or provider
    /// implementations, captured data, resolved modules, or returned clock and
    /// entropy values. Bind those separately when constructing a replay or
    /// package identity. Reading these bytes never calls a host provider.
    pub fn identity_bytes(&self) -> &[u8] {
        &self.identity
    }

    pub(crate) fn from_core(profile: Profile, settings: &rustia_vm::VmSettings) -> Self {
        // Keep this exhaustive: adding an engine option requires an explicit
        // identity decision instead of silently omitting it from attestations.
        let rustia_vm::VmSettings {
            diagnostic_limits,
            initial_fuel,
            recursion_limit,
            deterministic_gc_interval,
            loadstring_compiler,
            module_resolver,
            enable_conformance_coverage,
            enable_conformance_top_level_yield,
            enable_conformance_int64_userdata,
            enable_conformance_native_userdata,
            enable_conformance_native_type_checks,
            enable_conformance_pcall_helpers,
            enable_conformance_luau_call_limit,
            enable_memory_tracking,
            enable_opcode_census,
            memory_limit_bytes,
            guest_surface,
            services,
        } = settings;
        let rustia_vm::VmServices {
            stdout,
            wall_clock,
            monotonic_clock,
            entropy,
        } = services;
        let rustia_vm::diagnostic::DiagnosticLimits {
            retained_source_bytes,
            source_bytes,
            frames,
            report_bytes,
        } = diagnostic_limits;
        let guest_surface = GuestSurface::from_core(*guest_surface);
        let wall_clock = wall_clock
            .as_ref()
            .map(|provider| ServiceDeterminism::from_deterministic(provider.deterministic()));
        let monotonic_clock = monotonic_clock
            .as_ref()
            .map(|provider| ServiceDeterminism::from_deterministic(provider.deterministic()));
        let entropy = entropy
            .as_ref()
            .map(|provider| ServiceDeterminism::from_deterministic(provider.deterministic()));
        let module_resolver = module_resolver
            .as_ref()
            .map(|provider| ServiceDeterminism::from_deterministic(provider.deterministic()));

        let mut identity = Vec::with_capacity(128);
        identity.extend_from_slice(b"rustia-settings\0");
        identity.extend_from_slice(&1u32.to_le_bytes());
        identity.extend_from_slice(
            &RustiaExecutionProfile::new(profile, guest_surface).identity_bytes(),
        );
        for limit in [retained_source_bytes, source_bytes, frames, report_bytes] {
            identity.extend_from_slice(&(*limit as u64).to_le_bytes());
        }
        encode_optional_u64(&mut identity, *initial_fuel);
        identity.extend_from_slice(&(*recursion_limit as u64).to_le_bytes());
        identity.extend_from_slice(&deterministic_gc_interval.get().to_le_bytes());
        identity.push(u8::from(loadstring_compiler.is_some()));
        encode_provider(&mut identity, module_resolver);
        for enabled in [
            enable_conformance_coverage,
            enable_conformance_top_level_yield,
            enable_conformance_int64_userdata,
            enable_conformance_native_userdata,
            enable_conformance_native_type_checks,
            enable_conformance_pcall_helpers,
            enable_conformance_luau_call_limit,
            enable_memory_tracking,
            enable_opcode_census,
        ] {
            identity.push(u8::from(*enabled));
        }
        encode_optional_u64(&mut identity, memory_limit_bytes.map(|limit| limit as u64));
        identity.push(u8::from(stdout.is_some()));
        encode_provider(&mut identity, wall_clock);
        encode_provider(&mut identity, monotonic_clock);
        encode_provider(&mut identity, entropy);

        Self {
            profile,
            guest_surface,
            fuel: *initial_fuel,
            recursion_limit: *recursion_limit,
            memory_limit_bytes: *memory_limit_bytes,
            memory_tracking: *enable_memory_tracking,
            diagnostic_limits: DiagnosticLimits::from_core(*diagnostic_limits),
            runtime_compilation: loadstring_compiler.is_some(),
            stdout: stdout.is_some(),
            wall_clock,
            monotonic_clock,
            entropy,
            module_resolver,
            identity,
        }
    }
}

fn encode_optional_u64(output: &mut Vec<u8>, value: Option<u64>) {
    output.push(u8::from(value.is_some()));
    output.extend_from_slice(&value.unwrap_or(0).to_le_bytes());
}

fn encode_provider(output: &mut Vec<u8>, provider: Option<ServiceDeterminism>) {
    output.push(match provider {
        None => 0,
        Some(ServiceDeterminism::ReplayStable) => 1,
        Some(ServiceDeterminism::Live) => 2,
    });
}

/// A sample of VM-owned script memory, measured in bytes.
///
/// The logical live total counts object contents; the reserved total includes
/// retained allocation capacity. Rust callback captures, object payloads, and
/// process allocator overhead are outside these totals. Only logical live
/// bytes count toward the script memory limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VmMemoryStats {
    live_bytes: usize,
    reserved_bytes: usize,
    peak_live_bytes: Option<usize>,
    peak_reserved_bytes: Option<usize>,
}

impl VmMemoryStats {
    /// Returns the current logical live total.
    pub const fn live_bytes(&self) -> usize {
        self.live_bytes
    }

    /// Returns current reserved allocation capacity.
    pub const fn reserved_bytes(&self) -> usize {
        self.reserved_bytes
    }

    /// Returns the recorded live peak, or `None` when tracking is disabled.
    pub const fn peak_live_bytes(&self) -> Option<usize> {
        self.peak_live_bytes
    }

    /// Returns the recorded reserved peak, or `None` when tracking is disabled.
    pub const fn peak_reserved_bytes(&self) -> Option<usize> {
        self.peak_reserved_bytes
    }

    pub(crate) const fn from_core(stats: rustia_vm::VmMemoryStats) -> Self {
        Self {
            live_bytes: stats.live_bytes,
            reserved_bytes: stats.reserved_bytes,
            peak_live_bytes: stats.peak_live_bytes,
            peak_reserved_bytes: stats.peak_reserved_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Settings, VmMemoryStats};
    use crate::Profile;
    use rustia_vm::{ClockProvider, EntropyProvider, ModuleResolver, StdoutSink, VmSettings};
    use std::num::NonZeroU64;

    #[test]
    fn settings_identity_v1_has_a_fixed_portable_encoding() {
        let core = VmSettings {
            diagnostic_limits: rustia_vm::diagnostic::DiagnosticLimits {
                retained_source_bytes: 1,
                source_bytes: 2,
                frames: 3,
                report_bytes: 4,
            },
            initial_fuel: Some(5),
            recursion_limit: 6,
            deterministic_gc_interval: NonZeroU64::new(7).unwrap(),
            loadstring_compiler: Some(|_, _, _| Err("unused compiler".to_owned())),
            module_resolver: Some(ModuleResolver::new(|_, _| unreachable!(), false)),
            enable_conformance_coverage: true,
            enable_conformance_top_level_yield: false,
            enable_conformance_int64_userdata: true,
            enable_conformance_native_userdata: false,
            enable_conformance_native_type_checks: true,
            enable_conformance_pcall_helpers: false,
            enable_conformance_luau_call_limit: true,
            enable_memory_tracking: true,
            enable_opcode_census: false,
            memory_limit_bytes: Some(8),
            guest_surface: rustia_vm::GuestSurface::HardenedLeaf,
            services: rustia_vm::VmServices {
                stdout: Some(StdoutSink::new(|_| {})),
                wall_clock: Some(ClockProvider::new(|| unreachable!(), true)),
                monotonic_clock: Some(ClockProvider::new(|| unreachable!(), false)),
                entropy: Some(EntropyProvider::new(|| unreachable!(), true)),
            },
        };
        let settings = Settings::from_core(Profile::DeterministicSim, &core);
        let actual = settings
            .identity_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            actual,
            concat!(
                "7275737469612d73657474696e67730001000000010000000300000002000000",
                "0100000000000000020000000000000003000000000000000400000000000000",
                "01050000000000000006000000000000000700000000000000",
                "010201000100010001010001080000000000000001010201"
            )
        );
    }

    #[test]
    fn every_engine_option_changes_the_settings_identity() {
        type Change = (&'static str, fn(&mut VmSettings));
        let changes: &[Change] = &[
            ("retained source", |s| {
                s.diagnostic_limits.retained_source_bytes += 1
            }),
            ("source size", |s| s.diagnostic_limits.source_bytes += 1),
            ("frames", |s| s.diagnostic_limits.frames += 1),
            ("report size", |s| s.diagnostic_limits.report_bytes += 1),
            ("fuel", |s| s.initial_fuel = Some(0)),
            ("recursion", |s| s.recursion_limit += 1),
            ("collection cadence", |s| {
                s.deterministic_gc_interval = NonZeroU64::new(1).unwrap();
            }),
            ("runtime compiler", |s| {
                s.loadstring_compiler = Some(|_, _, _| Err("unused".to_owned()));
            }),
            ("module resolver", |s| {
                s.module_resolver = Some(ModuleResolver::new(|_, _| unreachable!(), true));
            }),
            ("coverage", |s| s.enable_conformance_coverage = true),
            ("top-level yield", |s| {
                s.enable_conformance_top_level_yield = true
            }),
            ("int64 userdata", |s| {
                s.enable_conformance_int64_userdata = true
            }),
            ("native userdata", |s| {
                s.enable_conformance_native_userdata = true
            }),
            ("native type checks", |s| {
                s.enable_conformance_native_type_checks = true
            }),
            ("pcall helpers", |s| {
                s.enable_conformance_pcall_helpers = true
            }),
            ("Luau call limit", |s| {
                s.enable_conformance_luau_call_limit = true
            }),
            ("memory peaks", |s| s.enable_memory_tracking = true),
            ("opcode census", |s| s.enable_opcode_census = true),
            ("memory limit", |s| s.memory_limit_bytes = Some(0)),
            ("guest surface", |s| {
                s.guest_surface = rustia_vm::GuestSurface::HardenedLeaf
            }),
            ("stdout", |s| {
                s.services.stdout = Some(StdoutSink::new(|_| {}))
            }),
            ("wall clock", |s| {
                s.services.wall_clock = Some(ClockProvider::new(|| unreachable!(), true));
            }),
            ("monotonic clock", |s| {
                s.services.monotonic_clock = Some(ClockProvider::new(|| unreachable!(), true));
            }),
            ("entropy", |s| {
                s.services.entropy = Some(EntropyProvider::new(|| unreachable!(), true));
            }),
        ];
        let original = Settings::from_core(Profile::ReproducibleStandard, &VmSettings::default());
        for (name, change) in changes {
            let mut core = VmSettings::default();
            change(&mut core);
            let changed = Settings::from_core(Profile::ReproducibleStandard, &core);
            assert_ne!(
                original.identity_bytes(),
                changed.identity_bytes(),
                "{name}"
            );
        }
        let different_profile = Settings::from_core(Profile::LuauCompat, &VmSettings::default());
        assert_ne!(
            original.identity_bytes(),
            different_profile.identity_bytes()
        );
    }

    #[test]
    fn service_declarations_are_identity_inputs_but_provider_contents_are_not() {
        let stable = VmSettings {
            services: rustia_vm::VmServices {
                stdout: Some(StdoutSink::new(|_| unreachable!())),
                wall_clock: Some(ClockProvider::new(|| unreachable!(), true)),
                monotonic_clock: Some(ClockProvider::new(|| unreachable!(), true)),
                entropy: Some(EntropyProvider::new(|| unreachable!(), true)),
            },
            module_resolver: Some(ModuleResolver::new(|_, _| unreachable!(), true)),
            ..VmSettings::default()
        };
        let baseline = Settings::from_core(Profile::ReproducibleStandard, &stable);
        let mut different_implementation = stable.clone();
        different_implementation.services.wall_clock = Some(ClockProvider::new(|| 42.0, true));
        different_implementation.services.monotonic_clock = Some(ClockProvider::new(|| 99.0, true));
        different_implementation.services.entropy = Some(EntropyProvider::new(|| 123, true));
        different_implementation.services.stdout = Some(StdoutSink::new(|_| {}));
        different_implementation.module_resolver = Some(ModuleResolver::new(
            |_, _| Err(rustia_vm::Error::new(rustia_vm::ErrorKind::OutOfMemory)),
            true,
        ));
        assert_eq!(
            baseline,
            Settings::from_core(Profile::ReproducibleStandard, &different_implementation)
        );

        let changes: &[fn(&mut VmSettings)] = &[
            |s| s.services.wall_clock = Some(ClockProvider::new(|| unreachable!(), false)),
            |s| s.services.monotonic_clock = Some(ClockProvider::new(|| unreachable!(), false)),
            |s| s.services.entropy = Some(EntropyProvider::new(|| unreachable!(), false)),
            |s| s.module_resolver = Some(ModuleResolver::new(|_, _| unreachable!(), false)),
        ];
        for change in changes {
            let mut live = stable.clone();
            change(&mut live);
            assert_ne!(
                baseline.identity_bytes(),
                Settings::from_core(Profile::ReproducibleStandard, &live).identity_bytes()
            );
        }
    }

    #[test]
    fn memory_stats_keep_absent_peaks_distinct_from_zero() {
        let without_tracking = VmMemoryStats::from_core(rustia_vm::VmMemoryStats {
            live_bytes: 1,
            reserved_bytes: 2,
            peak_live_bytes: None,
            peak_reserved_bytes: None,
        });
        let tracked_zero = VmMemoryStats::from_core(rustia_vm::VmMemoryStats {
            live_bytes: 1,
            reserved_bytes: 2,
            peak_live_bytes: Some(0),
            peak_reserved_bytes: Some(0),
        });
        assert_eq!(without_tracking.live_bytes(), 1);
        assert_eq!(without_tracking.reserved_bytes(), 2);
        assert_eq!(without_tracking.peak_live_bytes(), None);
        assert_eq!(without_tracking.peak_reserved_bytes(), None);
        assert_eq!(tracked_zero.peak_live_bytes(), Some(0));
        assert_eq!(tracked_zero.peak_reserved_bytes(), Some(0));
        assert_ne!(without_tracking, tracked_zero);
    }
}
