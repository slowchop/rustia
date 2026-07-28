#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Profile {
    /// Luau 0.709 surface compatibility (Luau hash-order table iteration) with the same
    /// host-service rules as [`ReproducibleStandard`](Profile::ReproducibleStandard). This is
    /// **not** the "normal Luau" preset — hosts looking for live stdout, system clocks, and
    /// `loadstring` want `RustiaBuilder::live()` from the `rustia` facade crate.
    LuauCompat,
    #[default]
    ReproducibleStandard,
    DeterministicSim,
}

impl Profile {
    #[inline]
    pub const fn is_deterministic_sim(self) -> bool {
        matches!(self, Self::DeterministicSim)
    }
}

/// Guest-visible language surface installed into one VM.
///
/// This is separate from [`Profile`]: the profile selects deterministic
/// semantics and host capabilities, while the guest surface selects how much
/// of the compatible standard environment a particular embedding exposes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GuestSurface {
    /// The profile's ordinary Luau-compatible global and standard-library
    /// surface.
    #[default]
    Standard,
    /// A stateless authoritative-leaf surface layered on
    /// [`Profile::DeterministicSim`].
    ///
    /// Guest globals and host-module namespaces are frozen before the first
    /// guest instruction. Environment/debug/raw mutation, output and clocks,
    /// VM RNG, buffers, vectors, coroutines, and captured upvalues are
    /// unavailable.
    HardenedLeaf,
}

impl GuestSurface {
    #[inline]
    pub const fn is_hardened_leaf(self) -> bool {
        matches!(self, Self::HardenedLeaf)
    }
}
