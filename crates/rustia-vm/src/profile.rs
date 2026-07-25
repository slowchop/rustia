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
