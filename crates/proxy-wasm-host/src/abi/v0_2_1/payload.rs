//! The value the crate serves to a guest for the time of one callback.
//!
//! A callback that carries a result, such as `proxy_on_http_call_response`,
//! lends the guest maps and a buffer that exist only while it runs.
//! The crate holds that value here and answers the map and the buffer
//! functions from it, so neither the stream state nor the service is asked.
//! The two predicates of this module are the one list of the map types and
//! the buffer types a delivery serves.

use std::borrow::Cow;

use crate::abi::v0_2_1::types::{BufferType, MapType};
use crate::abi::v0_2_1::{CalloutId, GrpcStatus, HeaderPairs, HttpCallResponse};
use crate::header_map::VecHeaderMap;

/// A header map that a delivery answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveredMap {
    HttpCallResponseHeaders,
    HttpCallResponseTrailers,
    GrpcCallInitialMetadata,
    GrpcCallTrailingMetadata,
}

/// A buffer that a delivery answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveredBuffer {
    HttpCallResponseBody,
    GrpcCallMessage,
    ForeignFunctionArguments,
}

/// The map types a delivery serves, which no implementation of the embedder
/// sees.
pub(crate) fn serves_map(map_type: MapType) -> Option<DeliveredMap> {
    match map_type {
        MapType::HttpCallResponseHeaders => Some(DeliveredMap::HttpCallResponseHeaders),
        MapType::HttpCallResponseTrailers => Some(DeliveredMap::HttpCallResponseTrailers),
        MapType::GrpcCallInitialMetadata => Some(DeliveredMap::GrpcCallInitialMetadata),
        MapType::GrpcCallTrailingMetadata => Some(DeliveredMap::GrpcCallTrailingMetadata),
        _ => None,
    }
}

/// The buffer types a delivery serves, which no implementation of the
/// embedder sees.
pub(crate) fn serves_buffer(buffer_type: BufferType) -> Option<DeliveredBuffer> {
    match buffer_type {
        BufferType::HttpCallResponseBody => Some(DeliveredBuffer::HttpCallResponseBody),
        BufferType::GrpcCallMessage => Some(DeliveredBuffer::GrpcCallMessage),
        BufferType::ForeignFunctionArguments => Some(DeliveredBuffer::ForeignFunctionArguments),
        _ => None,
    }
}

fn owned_map(pairs: HeaderPairs<'_>) -> VecHeaderMap {
    pairs
        .into_iter()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

/// What the running callback lends to the guest.
///
/// The callout is absent for a callback that names none.
#[derive(Debug)]
pub(crate) struct Delivery {
    callout: Option<CalloutId>,
    payload: Payload,
}

/// The one value of a delivery, by the callback that carries it.
#[derive(Debug)]
enum Payload {
    HttpCallResponse {
        headers: VecHeaderMap,
        trailers: VecHeaderMap,
        body: Vec<u8>,
    },
    GrpcInitialMetadata(VecHeaderMap),
    GrpcMessage(Vec<u8>),
    GrpcTrailingMetadata(VecHeaderMap),
    GrpcClose(GrpcStatus),
    ForeignArguments(Vec<u8>),
}

impl Delivery {
    /// Takes the response apart, so a value that owns its bytes is moved and
    /// not copied.
    pub(crate) fn http_call_response(callout: CalloutId, response: HttpCallResponse<'_>) -> Self {
        let (headers, body, trailers) = response.into_parts();
        Self {
            callout: Some(callout),
            payload: Payload::HttpCallResponse {
                headers: owned_map(headers),
                trailers: owned_map(trailers),
                body: body.into_owned(),
            },
        }
    }

    pub(crate) fn grpc_initial_metadata(callout: CalloutId, metadata: HeaderPairs<'_>) -> Self {
        Self {
            callout: Some(callout),
            payload: Payload::GrpcInitialMetadata(owned_map(metadata)),
        }
    }

    pub(crate) fn grpc_message(callout: CalloutId, message: Cow<'_, [u8]>) -> Self {
        Self {
            callout: Some(callout),
            payload: Payload::GrpcMessage(message.into_owned()),
        }
    }

    pub(crate) fn grpc_trailing_metadata(callout: CalloutId, metadata: HeaderPairs<'_>) -> Self {
        Self {
            callout: Some(callout),
            payload: Payload::GrpcTrailingMetadata(owned_map(metadata)),
        }
    }

    pub(crate) fn grpc_close(callout: CalloutId, status: GrpcStatus) -> Self {
        Self {
            callout: Some(callout),
            payload: Payload::GrpcClose(status),
        }
    }

    /// The arguments of a foreign function call, which names no callout.
    pub(crate) fn foreign_arguments(arguments: Cow<'_, [u8]>) -> Self {
        Self {
            callout: None,
            payload: Payload::ForeignArguments(arguments.into_owned()),
        }
    }

    pub(crate) fn callout(&self) -> Option<CalloutId> {
        self.callout
    }

    /// The map this delivery holds, which a read reaches and a write does
    /// not.
    pub(crate) fn map_mut(&mut self, which: DeliveredMap) -> Option<&mut VecHeaderMap> {
        match (&mut self.payload, which) {
            (Payload::HttpCallResponse { headers, .. }, DeliveredMap::HttpCallResponseHeaders) => {
                Some(headers)
            }
            (
                Payload::HttpCallResponse { trailers, .. },
                DeliveredMap::HttpCallResponseTrailers,
            ) => Some(trailers),
            (Payload::GrpcInitialMetadata(map), DeliveredMap::GrpcCallInitialMetadata) => Some(map),
            (Payload::GrpcTrailingMetadata(map), DeliveredMap::GrpcCallTrailingMetadata) => {
                Some(map)
            }
            _ => None,
        }
    }

    /// The buffer this delivery holds.
    pub(crate) fn buffer(&self, which: DeliveredBuffer) -> Option<&[u8]> {
        match (&self.payload, which) {
            (Payload::HttpCallResponse { body, .. }, DeliveredBuffer::HttpCallResponseBody) => {
                Some(body)
            }
            (Payload::GrpcMessage(message), DeliveredBuffer::GrpcCallMessage) => Some(message),
            (Payload::ForeignArguments(arguments), DeliveredBuffer::ForeignFunctionArguments) => {
                Some(arguments)
            }
            _ => None,
        }
    }

    /// The code and the message `proxy_get_status` answers, or `None` for a
    /// delivery that has no status.
    ///
    /// The ABI gives the status of an HTTP call and of a gRPC message no
    /// meaning, so a callout delivery that is not a gRPC close answers code
    /// zero and an empty message.
    /// A foreign function call is no callout and answers no status.
    pub(crate) fn status(&self) -> Option<(u32, &str)> {
        match &self.payload {
            Payload::GrpcClose(status) => Some((status.code, &status.message)),
            Payload::ForeignArguments(_) => None,
            _ => Some((0, "")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header_map::HeaderMap;

    fn callout() -> CalloutId {
        CalloutId::try_from(1_u32).unwrap()
    }

    fn pairs(key: &'static [u8], value: &'static [u8]) -> HeaderPairs<'static> {
        vec![(Cow::Borrowed(key), Cow::Borrowed(value))]
    }

    #[test]
    fn the_two_predicates_name_the_four_maps_and_the_three_buffers_a_delivery_serves() {
        // Arrange
        let maps = [
            MapType::HttpRequestHeaders,
            MapType::GrpcCallInitialMetadata,
            MapType::GrpcCallTrailingMetadata,
            MapType::HttpCallResponseHeaders,
            MapType::HttpCallResponseTrailers,
        ];
        let buffers = [
            BufferType::HttpRequestBody,
            BufferType::GrpcCallMessage,
            BufferType::HttpCallResponseBody,
            BufferType::ForeignFunctionArguments,
            BufferType::VmConfiguration,
        ];

        // Act
        let served = (maps.map(serves_map), buffers.map(serves_buffer));

        // Assert
        assert_eq!(
            served.0,
            [
                None,
                Some(DeliveredMap::GrpcCallInitialMetadata),
                Some(DeliveredMap::GrpcCallTrailingMetadata),
                Some(DeliveredMap::HttpCallResponseHeaders),
                Some(DeliveredMap::HttpCallResponseTrailers),
            ]
        );
        assert_eq!(
            served.1,
            [
                None,
                Some(DeliveredBuffer::GrpcCallMessage),
                Some(DeliveredBuffer::HttpCallResponseBody),
                Some(DeliveredBuffer::ForeignFunctionArguments),
                None,
            ]
        );
    }

    #[test]
    fn a_delivery_answers_the_map_it_holds_and_no_other() {
        // Arrange
        let mut delivery = Delivery::grpc_initial_metadata(callout(), pairs(b"k", b"v"));

        // Act
        let answers = [
            delivery
                .map_mut(DeliveredMap::GrpcCallInitialMetadata)
                .map(|map| map.len()),
            delivery
                .map_mut(DeliveredMap::GrpcCallTrailingMetadata)
                .map(|map| map.len()),
            delivery
                .map_mut(DeliveredMap::HttpCallResponseHeaders)
                .map(|map| map.len()),
        ];

        // Assert
        assert_eq!(answers, [Some(1), None, None]);
        assert_eq!(delivery.callout(), Some(callout()));
    }

    #[test]
    fn a_delivery_answers_the_buffer_it_holds_and_no_other() {
        // Arrange
        let delivery = Delivery::grpc_message(callout(), Cow::Borrowed(b"hello"));

        // Act
        let answers = [
            delivery.buffer(DeliveredBuffer::GrpcCallMessage),
            delivery.buffer(DeliveredBuffer::HttpCallResponseBody),
        ];

        // Assert
        assert_eq!(answers, [Some(b"hello".as_slice()), None]);
        assert_eq!(delivery.status(), Some((0, "")));
    }

    #[test]
    fn a_close_answers_its_code_and_its_message_and_no_map_or_buffer() {
        // Arrange
        let mut delivery = Delivery::grpc_close(callout(), GrpcStatus::new(14, "unavailable"));

        // Act
        let status = delivery.status();

        // Assert
        assert_eq!(status, Some((14, "unavailable")));
        assert!(delivery.buffer(DeliveredBuffer::GrpcCallMessage).is_none());
        assert!(
            delivery
                .map_mut(DeliveredMap::GrpcCallTrailingMetadata)
                .is_none()
        );
    }

    #[test]
    fn a_delivered_response_holds_its_two_maps_and_its_body() {
        // Arrange
        let response = HttpCallResponse::received(pairs(b":status", b"200"))
            .with_trailers(pairs(b"grpc-status", b"0"))
            .with_body(Cow::Borrowed(b"body"));
        let mut delivery = Delivery::http_call_response(callout(), response);

        // Act
        let headers = delivery
            .map_mut(DeliveredMap::HttpCallResponseHeaders)
            .map(|map| map.get(b":status").map(|value| value.to_vec()));

        // Assert
        assert_eq!(headers, Some(Some(b"200".to_vec())));
        assert_eq!(
            delivery.buffer(DeliveredBuffer::HttpCallResponseBody),
            Some(b"body".as_slice())
        );
        assert_eq!(
            delivery
                .map_mut(DeliveredMap::HttpCallResponseTrailers)
                .map(|map| map.len()),
            Some(1)
        );
        assert_eq!(delivery.status(), Some((0, "")));
    }

    #[test]
    fn a_foreign_function_delivery_holds_its_arguments_and_no_callout() {
        // Arrange
        let delivery = Delivery::foreign_arguments(Cow::Borrowed(b"hello"));

        // Act
        let arguments = delivery.buffer(DeliveredBuffer::ForeignFunctionArguments);

        // Assert
        assert_eq!(arguments, Some(b"hello".as_slice()));
        assert_eq!(delivery.callout(), None);
        assert_eq!(delivery.status(), None, "the ABI gives it no status");
        assert!(delivery.buffer(DeliveredBuffer::GrpcCallMessage).is_none());
    }
}
