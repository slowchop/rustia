#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Profile {
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
