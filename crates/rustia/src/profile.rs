/// Selects language compatibility and the allowed host capabilities.
///
/// Use builder presets for ordinary setup. `live()` uses
/// [`ReproducibleStandard`](Self::ReproducibleStandard) with real host services.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Profile {
    /// Standalone Luau compatibility, including Luau-style table iteration.
    LuauCompat,
    /// Standalone Luau with reproducible defaults and insertion-ordered tables.
    #[default]
    ReproducibleStandard,
    /// Replay-oriented execution that rejects external mutable-state capabilities.
    DeterministicSim,
    /// PUC Lua 5.1.5 table layout, sorting, and number formatting.
    Puc51Compat,
}

impl Profile {
    pub const fn is_deterministic_sim(self) -> bool {
        matches!(self, Self::DeterministicSim)
    }

    pub const fn is_luau_compat(self) -> bool {
        matches!(self, Self::LuauCompat)
    }

    pub const fn is_puc51_compat(self) -> bool {
        matches!(self, Self::Puc51Compat)
    }

    pub(crate) const fn to_core(self) -> rustia_vm::Profile {
        match self {
            Self::LuauCompat => rustia_vm::Profile::LuauCompat,
            Self::ReproducibleStandard => rustia_vm::Profile::ReproducibleStandard,
            Self::DeterministicSim => rustia_vm::Profile::DeterministicSim,
            Self::Puc51Compat => rustia_vm::Profile::Puc51Compat,
        }
    }

    pub(crate) const fn from_core(profile: rustia_vm::Profile) -> Self {
        match profile {
            rustia_vm::Profile::LuauCompat => Self::LuauCompat,
            rustia_vm::Profile::ReproducibleStandard => Self::ReproducibleStandard,
            rustia_vm::Profile::DeterministicSim => Self::DeterministicSim,
            rustia_vm::Profile::Puc51Compat => Self::Puc51Compat,
        }
    }
}

/// The script environment selected by an execution preset.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GuestSurface {
    /// The selected profile's ordinary globals and libraries.
    #[default]
    Standard,
    /// The frozen, restricted environment used for stateless decisions.
    HardenedLeaf,
}

impl GuestSurface {
    pub const fn is_hardened_leaf(self) -> bool {
        matches!(self, Self::HardenedLeaf)
    }

    pub(crate) const fn to_core(self) -> rustia_vm::GuestSurface {
        match self {
            Self::Standard => rustia_vm::GuestSurface::Standard,
            Self::HardenedLeaf => rustia_vm::GuestSurface::HardenedLeaf,
        }
    }

    pub(crate) const fn from_core(surface: rustia_vm::GuestSurface) -> Self {
        match surface {
            rustia_vm::GuestSurface::Standard => Self::Standard,
            rustia_vm::GuestSurface::HardenedLeaf => Self::HardenedLeaf,
        }
    }
}

/// A Rust callback's declared access to application state and effects.
///
/// Rustia checks the declaration against the selected profile. Captured state
/// remains the application's responsibility.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpCapability {
    Pure,
    ReadsDeterministicInput,
    ReadsSlotState,
    EmitsDeterministicEffect,
    WritesSlotEffect,
    ReadsExternalMutableState,
    NondeterministicEffect,
}

impl OpCapability {
    /// Stable tag for replay or package identities; independent of enum layout.
    pub const fn identity_tag(self) -> u32 {
        match self {
            Self::Pure => 1,
            Self::ReadsDeterministicInput => 2,
            Self::ReadsSlotState => 3,
            Self::EmitsDeterministicEffect => 4,
            Self::WritesSlotEffect => 5,
            Self::ReadsExternalMutableState => 6,
            Self::NondeterministicEffect => 7,
        }
    }

    pub(crate) const fn to_core(self) -> rustia_vm::OpCapability {
        match self {
            Self::Pure => rustia_vm::OpCapability::Pure,
            Self::ReadsDeterministicInput => rustia_vm::OpCapability::ReadsDeterministicInput,
            Self::ReadsSlotState => rustia_vm::OpCapability::ReadsSlotState,
            Self::EmitsDeterministicEffect => rustia_vm::OpCapability::EmitsDeterministicEffect,
            Self::WritesSlotEffect => rustia_vm::OpCapability::WritesSlotEffect,
            Self::ReadsExternalMutableState => rustia_vm::OpCapability::ReadsExternalMutableState,
            Self::NondeterministicEffect => rustia_vm::OpCapability::NondeterministicEffect,
        }
    }

    pub(crate) const fn from_core(capability: rustia_vm::OpCapability) -> Self {
        match capability {
            rustia_vm::OpCapability::Pure => Self::Pure,
            rustia_vm::OpCapability::ReadsDeterministicInput => Self::ReadsDeterministicInput,
            rustia_vm::OpCapability::ReadsSlotState => Self::ReadsSlotState,
            rustia_vm::OpCapability::EmitsDeterministicEffect => Self::EmitsDeterministicEffect,
            rustia_vm::OpCapability::WritesSlotEffect => Self::WritesSlotEffect,
            rustia_vm::OpCapability::ReadsExternalMutableState => Self::ReadsExternalMutableState,
            rustia_vm::OpCapability::NondeterministicEffect => Self::NondeterministicEffect,
        }
    }
}
