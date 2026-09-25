//! How hard the compiler optimizes the code of a guest.

/// How hard Cranelift optimizes the machine code it makes from a guest.
///
/// Set it with [`EngineConfig::with_opt_level`](crate::EngineConfig::with_opt_level).
/// It applies to every module the engine compiles, and the default is
/// [`OptLevel::Speed`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum OptLevel {
    /// Makes the fastest code it can, even when the code grows larger.
    #[default]
    Speed,
    /// Makes fast code, and also applies the transformations that keep the
    /// code small.
    SpeedAndSize,
}

impl From<OptLevel> for wasmtime::OptLevel {
    fn from(level: OptLevel) -> Self {
        match level {
            OptLevel::Speed => Self::Speed,
            OptLevel::SpeedAndSize => Self::SpeedAndSize,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_level_maps_to_the_wasmtime_level_of_its_name() {
        // Arrange
        let levels = [OptLevel::Speed, OptLevel::SpeedAndSize];

        // Act
        let mapped = levels.map(wasmtime::OptLevel::from);

        // Assert
        assert_eq!(
            mapped,
            [wasmtime::OptLevel::Speed, wasmtime::OptLevel::SpeedAndSize]
        );
    }
}
