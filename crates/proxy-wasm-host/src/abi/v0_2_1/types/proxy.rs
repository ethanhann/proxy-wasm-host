//! The `proxy_*_t` enumerations.
//!
//! The split from the WASI enumerations mirrors the `proxy_` and `wasi_`
//! prefixes in the ABI document.

abi_enum! {
    /// The `proxy_log_level_t` type.
    ///
    /// The variants order from the least to the most severe, so you can
    /// compare two levels directly.
    #[derive(PartialOrd, Ord)]
    LogLevel {
        /// `TRACE` = 0.
        Trace = 0,
        /// `DEBUG` = 1.
        Debug = 1,
        /// `INFO` = 2.
        Info = 2,
        /// `WARN` = 3.
        Warn = 3,
        /// `ERROR` = 4.
        Error = 4,
        /// `CRITICAL` = 5.
        Critical = 5,
    }
}

abi_enum! {
    /// The `proxy_status_t` type.
    ///
    /// Every host function returns one of these to the guest.
    /// The values 5, 9, and 11 are gaps in the ABI document.
    /// The enum is non exhaustive, because the vNEXT draft adds values.
    #[non_exhaustive]
    Status {
        /// `OK` = 0.
        Ok = 0,
        /// `NOT_FOUND` = 1.
        NotFound = 1,
        /// `BAD_ARGUMENT` = 2.
        BadArgument = 2,
        /// `SERIALIZATION_FAILURE` = 3.
        SerializationFailure = 3,
        /// `PARSE_FAILURE` = 4.
        ParseFailure = 4,
        /// `INVALID_MEMORY_ACCESS` = 6.
        InvalidMemoryAccess = 6,
        /// `EMPTY` = 7.
        Empty = 7,
        /// `CAS_MISMATCH` = 8.
        CasMismatch = 8,
        /// `INTERNAL_FAILURE` = 10.
        InternalFailure = 10,
        /// `UNIMPLEMENTED` = 12.
        Unimplemented = 12,
    }
}

abi_enum! {
    /// The `proxy_action_t` type.
    Action {
        /// `CONTINUE` = 0.
        Continue = 0,
        /// `PAUSE` = 1.
        Pause = 1,
    }
}

abi_enum! {
    /// The `proxy_buffer_type_t` type.
    BufferType {
        /// `HTTP_REQUEST_BODY` = 0.
        HttpRequestBody = 0,
        /// `HTTP_RESPONSE_BODY` = 1.
        HttpResponseBody = 1,
        /// `DOWNSTREAM_DATA` = 2.
        DownstreamData = 2,
        /// `UPSTREAM_DATA` = 3.
        UpstreamData = 3,
        /// `HTTP_CALL_RESPONSE_BODY` = 4.
        HttpCallResponseBody = 4,
        /// `GRPC_CALL_MESSAGE` = 5.
        GrpcCallMessage = 5,
        /// `VM_CONFIGURATION` = 6.
        VmConfiguration = 6,
        /// `PLUGIN_CONFIGURATION` = 7.
        PluginConfiguration = 7,
        /// `FOREIGN_FUNCTION_ARGUMENTS` = 8.
        ForeignFunctionArguments = 8,
    }
}

abi_enum! {
    /// The `proxy_map_type_t` type.
    MapType {
        /// `HTTP_REQUEST_HEADERS` = 0.
        HttpRequestHeaders = 0,
        /// `HTTP_REQUEST_TRAILERS` = 1.
        HttpRequestTrailers = 1,
        /// `HTTP_RESPONSE_HEADERS` = 2.
        HttpResponseHeaders = 2,
        /// `HTTP_RESPONSE_TRAILERS` = 3.
        HttpResponseTrailers = 3,
        /// `GRPC_CALL_INITIAL_METADATA` = 4.
        GrpcCallInitialMetadata = 4,
        /// `GRPC_CALL_TRAILING_METADATA` = 5.
        GrpcCallTrailingMetadata = 5,
        /// `HTTP_CALL_RESPONSE_HEADERS` = 6.
        HttpCallResponseHeaders = 6,
        /// `HTTP_CALL_RESPONSE_TRAILERS` = 7.
        HttpCallResponseTrailers = 7,
    }
}

abi_enum! {
    /// The `proxy_peer_type_t` type.
    PeerType {
        /// `UNKNOWN` = 0.
        Unknown = 0,
        /// `LOCAL` = 1.
        Local = 1,
        /// `REMOTE` = 2.
        Remote = 2,
    }
}

abi_enum! {
    /// The `proxy_stream_type_t` type.
    StreamType {
        /// `HTTP_REQUEST` = 0.
        HttpRequest = 0,
        /// `HTTP_RESPONSE` = 1.
        HttpResponse = 1,
        /// `DOWNSTREAM` = 2.
        Downstream = 2,
        /// `UPSTREAM` = 3.
        Upstream = 3,
    }
}

abi_enum! {
    /// The `proxy_metric_type_t` type.
    MetricType {
        /// `COUNTER` = 0.
        Counter = 0,
        /// `GAUGE` = 1.
        Gauge = 1,
        /// `HISTOGRAM` = 2.
        Histogram = 2,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{covers, expected, round_trip, unknown};
    use super::*;

    #[test]
    fn log_level_matches_the_abi_document() {
        // Arrange
        let table = [
            (LogLevel::Trace, 0),
            (LogLevel::Debug, 1),
            (LogLevel::Info, 2),
            (LogLevel::Warn, 3),
            (LogLevel::Error, 4),
            (LogLevel::Critical, 5),
        ];

        // Act
        let results = round_trip(&table);

        // Assert
        assert_eq!(results, expected(&table));
        assert!(covers(&table, LogLevel::ALL));
    }

    #[test]
    fn log_level_rejects_unknown_values() {
        // Arrange
        let values = [-1, 6];

        // Act
        let results: Vec<_> = values.iter().map(|&v| LogLevel::try_from(v)).collect();

        // Assert
        assert_eq!(results, unknown("LogLevel", &values));
    }

    #[test]
    fn status_matches_the_abi_document() {
        // Arrange
        let table = [
            (Status::Ok, 0),
            (Status::NotFound, 1),
            (Status::BadArgument, 2),
            (Status::SerializationFailure, 3),
            (Status::ParseFailure, 4),
            (Status::InvalidMemoryAccess, 6),
            (Status::Empty, 7),
            (Status::CasMismatch, 8),
            (Status::InternalFailure, 10),
            (Status::Unimplemented, 12),
        ];

        // Act
        let results = round_trip(&table);

        // Assert
        assert_eq!(results, expected(&table));
        assert!(covers(&table, Status::ALL));
    }

    #[test]
    fn status_rejects_the_gaps_and_out_of_range_values() {
        // Arrange
        let values = [-1, 5, 9, 11, 13];

        // Act
        let results: Vec<_> = values.iter().map(|&v| Status::try_from(v)).collect();

        // Assert
        assert_eq!(results, unknown("Status", &values));
    }

    #[test]
    fn action_matches_the_abi_document() {
        // Arrange
        let table = [(Action::Continue, 0), (Action::Pause, 1)];

        // Act
        let results = round_trip(&table);

        // Assert
        assert_eq!(results, expected(&table));
        assert!(covers(&table, Action::ALL));
    }

    #[test]
    fn action_rejects_unknown_values() {
        // Arrange
        let values = [-1, 2];

        // Act
        let results: Vec<_> = values.iter().map(|&v| Action::try_from(v)).collect();

        // Assert
        assert_eq!(results, unknown("Action", &values));
    }

    #[test]
    fn buffer_type_matches_the_abi_document() {
        // Arrange
        let table = [
            (BufferType::HttpRequestBody, 0),
            (BufferType::HttpResponseBody, 1),
            (BufferType::DownstreamData, 2),
            (BufferType::UpstreamData, 3),
            (BufferType::HttpCallResponseBody, 4),
            (BufferType::GrpcCallMessage, 5),
            (BufferType::VmConfiguration, 6),
            (BufferType::PluginConfiguration, 7),
            (BufferType::ForeignFunctionArguments, 8),
        ];

        // Act
        let results = round_trip(&table);

        // Assert
        assert_eq!(results, expected(&table));
        assert!(covers(&table, BufferType::ALL));
    }

    #[test]
    fn buffer_type_rejects_unknown_values() {
        // Arrange
        let values = [-1, 9];

        // Act
        let results: Vec<_> = values.iter().map(|&v| BufferType::try_from(v)).collect();

        // Assert
        assert_eq!(results, unknown("BufferType", &values));
    }

    #[test]
    fn map_type_matches_the_abi_document() {
        // Arrange
        let table = [
            (MapType::HttpRequestHeaders, 0),
            (MapType::HttpRequestTrailers, 1),
            (MapType::HttpResponseHeaders, 2),
            (MapType::HttpResponseTrailers, 3),
            (MapType::GrpcCallInitialMetadata, 4),
            (MapType::GrpcCallTrailingMetadata, 5),
            (MapType::HttpCallResponseHeaders, 6),
            (MapType::HttpCallResponseTrailers, 7),
        ];

        // Act
        let results = round_trip(&table);

        // Assert
        assert_eq!(results, expected(&table));
        assert!(covers(&table, MapType::ALL));
    }

    #[test]
    fn map_type_rejects_unknown_values() {
        // Arrange
        let values = [-1, 8];

        // Act
        let results: Vec<_> = values.iter().map(|&v| MapType::try_from(v)).collect();

        // Assert
        assert_eq!(results, unknown("MapType", &values));
    }

    #[test]
    fn peer_type_matches_the_abi_document() {
        // Arrange
        let table = [
            (PeerType::Unknown, 0),
            (PeerType::Local, 1),
            (PeerType::Remote, 2),
        ];

        // Act
        let results = round_trip(&table);

        // Assert
        assert_eq!(results, expected(&table));
        assert!(covers(&table, PeerType::ALL));
    }

    #[test]
    fn peer_type_rejects_unknown_values() {
        // Arrange
        let values = [-1, 3];

        // Act
        let results: Vec<_> = values.iter().map(|&v| PeerType::try_from(v)).collect();

        // Assert
        assert_eq!(results, unknown("PeerType", &values));
    }

    #[test]
    fn stream_type_matches_the_abi_document() {
        // Arrange
        let table = [
            (StreamType::HttpRequest, 0),
            (StreamType::HttpResponse, 1),
            (StreamType::Downstream, 2),
            (StreamType::Upstream, 3),
        ];

        // Act
        let results = round_trip(&table);

        // Assert
        assert_eq!(results, expected(&table));
        assert!(covers(&table, StreamType::ALL));
    }

    #[test]
    fn stream_type_rejects_unknown_values() {
        // Arrange
        let values = [-1, 4];

        // Act
        let results: Vec<_> = values.iter().map(|&v| StreamType::try_from(v)).collect();

        // Assert
        assert_eq!(results, unknown("StreamType", &values));
    }

    #[test]
    fn metric_type_matches_the_abi_document() {
        // Arrange
        let table = [
            (MetricType::Counter, 0),
            (MetricType::Gauge, 1),
            (MetricType::Histogram, 2),
        ];

        // Act
        let results = round_trip(&table);

        // Assert
        assert_eq!(results, expected(&table));
        assert!(covers(&table, MetricType::ALL));
    }

    #[test]
    fn metric_type_rejects_unknown_values() {
        // Arrange
        let values = [-1, 3];

        // Act
        let results: Vec<_> = values.iter().map(|&v| MetricType::try_from(v)).collect();

        // Assert
        assert_eq!(results, unknown("MetricType", &values));
    }
}

impl std::fmt::Display for Status {
    /// The ABI's own name for the status.
    ///
    /// The match names every variant, so a version that adds one stops the
    /// build rather than printing something the ABI does not use.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Status::Ok => "OK",
            Status::NotFound => "NOT_FOUND",
            Status::BadArgument => "BAD_ARGUMENT",
            Status::SerializationFailure => "SERIALIZATION_FAILURE",
            Status::ParseFailure => "PARSE_FAILURE",
            Status::InvalidMemoryAccess => "INVALID_MEMORY_ACCESS",
            Status::Empty => "EMPTY",
            Status::CasMismatch => "CAS_MISMATCH",
            Status::InternalFailure => "INTERNAL_FAILURE",
            Status::Unimplemented => "UNIMPLEMENTED",
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn every_status_prints_the_name_the_abi_uses() {
        // Arrange
        let statuses = Status::ALL;

        // Act
        let printed: Vec<String> = statuses.iter().map(ToString::to_string).collect();

        // Assert
        assert_eq!(
            printed,
            [
                "OK",
                "NOT_FOUND",
                "BAD_ARGUMENT",
                "SERIALIZATION_FAILURE",
                "PARSE_FAILURE",
                "INVALID_MEMORY_ACCESS",
                "EMPTY",
                "CAS_MISMATCH",
                "INTERNAL_FAILURE",
                "UNIMPLEMENTED",
            ]
        );
    }
}
