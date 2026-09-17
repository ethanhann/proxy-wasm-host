//! The `wasi_*_t` enumerations.
//!
//! The ABI document defines the subset of WASI values that a host must
//! understand.
//! This module holds only that subset.
//! The split from the proxy enumerations mirrors the `proxy_` and `wasi_`
//! prefixes in the ABI document.

abi_enum! {
    /// The `wasi_errno_t` type.
    WasiErrno {
        /// `SUCCESS` = 0.
        Success = 0,
        /// `BADF` = 8.
        Badf = 8,
        /// `FAULT` = 21.
        Fault = 21,
        /// `INVAL` = 28.
        Inval = 28,
        /// `NOTSUP` = 58.
        Notsup = 58,
    }
}

abi_enum! {
    /// The `wasi_fd_id_t` type.
    WasiFdId {
        /// `STDOUT` = 1.
        Stdout = 1,
        /// `STDERR` = 2.
        Stderr = 2,
    }
}

abi_enum! {
    /// The `wasi_clock_id_t` type.
    WasiClockId {
        /// `REALTIME` = 0.
        Realtime = 0,
        /// `MONOTONIC` = 1.
        Monotonic = 1,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{covers, expected, round_trip, unknown};
    use super::*;

    #[test]
    fn wasi_errno_matches_the_abi_document() {
        // Arrange
        let table = [
            (WasiErrno::Success, 0),
            (WasiErrno::Badf, 8),
            (WasiErrno::Fault, 21),
            (WasiErrno::Inval, 28),
            (WasiErrno::Notsup, 58),
        ];

        // Act
        let results = round_trip(&table);

        // Assert
        assert_eq!(results, expected(&table));
        assert!(covers(&table, WasiErrno::ALL));
    }

    #[test]
    fn wasi_errno_rejects_unknown_values() {
        // Arrange
        let values = [-1, 1, 59];

        // Act
        let results: Vec<_> = values.iter().map(|&v| WasiErrno::try_from(v)).collect();

        // Assert
        assert_eq!(results, unknown("WasiErrno", &values));
    }

    #[test]
    fn wasi_fd_id_matches_the_abi_document() {
        // Arrange
        let table = [(WasiFdId::Stdout, 1), (WasiFdId::Stderr, 2)];

        // Act
        let results = round_trip(&table);

        // Assert
        assert_eq!(results, expected(&table));
        assert!(covers(&table, WasiFdId::ALL));
    }

    #[test]
    fn wasi_fd_id_rejects_unknown_values() {
        // Arrange
        let values = [-1, 0, 3];

        // Act
        let results: Vec<_> = values.iter().map(|&v| WasiFdId::try_from(v)).collect();

        // Assert
        assert_eq!(results, unknown("WasiFdId", &values));
    }

    #[test]
    fn wasi_clock_id_matches_the_abi_document() {
        // Arrange
        let table = [(WasiClockId::Realtime, 0), (WasiClockId::Monotonic, 1)];

        // Act
        let results = round_trip(&table);

        // Assert
        assert_eq!(results, expected(&table));
        assert!(covers(&table, WasiClockId::ALL));
    }

    #[test]
    fn wasi_clock_id_rejects_unknown_values() {
        // Arrange
        let values = [-1, 2];

        // Act
        let results: Vec<_> = values.iter().map(|&v| WasiClockId::try_from(v)).collect();

        // Assert
        assert_eq!(results, unknown("WasiClockId", &values));
    }
}
