//! Typed event-log boundary with exact expected-version and idempotency guards.

use std::fmt;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{BoxFuture, RealmId, TenantId};

/// Stable format marker for persisted stream partition keys.
pub const STREAM_ID_ENCODING: &str = "service-stream/1";

/// Stable identity of the standalone service that owns a stream.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ServiceId(String);

impl ServiceId {
    /// Creates a stable service identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidServiceId> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(InvalidServiceId);
        }
        Ok(Self(value))
    }

    /// Returns the stable service identifier as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServiceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ServiceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// A service identifier cannot be empty.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("service identifier must not be empty")]
pub struct InvalidServiceId;

/// A service-, tenant-, and exact-optional-realm-scoped aggregate stream identity.
///
/// Service is deployment-stable, tenant and realm come from verified authentication, and aggregate
/// category and key are domain coordinates rather than transport routes. `None` and
/// `Some("default")` are different partitions.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StreamId {
    service: ServiceId,
    tenant: TenantId,
    realm: Option<RealmId>,
    category: String,
    key: String,
}

impl StreamId {
    /// Creates a typed stream identity.
    pub fn new(
        service: ServiceId,
        tenant: TenantId,
        realm: Option<RealmId>,
        category: impl Into<String>,
        key: impl Into<String>,
    ) -> Result<Self, InvalidStreamId> {
        let category = category.into();
        let key = key.into();
        if category.trim().is_empty() {
            return Err(InvalidStreamId::EmptyCategory);
        }
        if key.trim().is_empty() {
            return Err(InvalidStreamId::EmptyKey);
        }
        Ok(Self {
            service,
            tenant,
            realm,
            category,
            key,
        })
    }

    /// Returns the stable service that owns the stream.
    pub fn service(&self) -> &ServiceId {
        &self.service
    }

    /// Returns the tenant that owns the stream.
    pub fn tenant(&self) -> &TenantId {
        &self.tenant
    }

    /// Returns the exact optional authenticated realm partition.
    pub fn realm(&self) -> Option<&RealmId> {
        self.realm.as_ref()
    }

    /// Returns the aggregate category.
    pub fn category(&self) -> &str {
        &self.category
    }

    /// Returns the aggregate key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns the versioned, length-delimited persistence partition key.
    ///
    /// Each string length is its UTF-8 byte length. Realm carries an explicit `0`/`1` presence tag,
    /// so absence cannot collide with any realm spelling, including `"default"`. Length prefixes
    /// make separators inside an identifier data rather than structure.
    pub fn partition_key(&self) -> String {
        let mut output = String::from(STREAM_ID_ENCODING);
        push_part(&mut output, self.service.as_str());
        push_part(&mut output, self.tenant.as_str());
        match &self.realm {
            None => output.push_str("|0"),
            Some(realm) => {
                output.push_str("|1");
                push_part(&mut output, realm.as_str());
            }
        }
        push_part(&mut output, &self.category);
        push_part(&mut output, &self.key);
        output
    }
}

impl fmt::Display for StreamId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.partition_key())
    }
}

impl Serialize for StreamId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.partition_key())
    }
}

fn push_part(output: &mut String, value: &str) {
    let _ = write!(output, "|{}:{value}", value.len());
}

/// Stream identity validation failed.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum InvalidStreamId {
    /// Aggregate category cannot be empty.
    #[error("stream category must not be empty")]
    EmptyCategory,
    /// Aggregate key cannot be empty.
    #[error("stream key must not be empty")]
    EmptyKey,
}

/// The committed version of a stream (`0` means no events).
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct StreamVersion(u64);

impl StreamVersion {
    /// Version of an empty stream.
    pub const EMPTY: Self = Self(0);

    /// Creates a version from its numeric representation.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the numeric representation.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next version, or `None` at numeric exhaustion.
    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

impl fmt::Display for StreamVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Caller-declared optimistic-concurrency guard.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ExpectedVersion {
    /// The stream must not exist yet.
    NoStream,
    /// The stream must be at this exact existing version.
    Exact(StreamVersion),
}

impl ExpectedVersion {
    /// Requires that the stream has no committed events.
    pub const fn no_stream() -> Self {
        Self::NoStream
    }

    /// Expects the supplied current stream version.
    pub const fn exact(version: StreamVersion) -> Self {
        Self::Exact(version)
    }

    /// Returns whether the actual stream version satisfies this caller declaration.
    pub const fn matches(self, actual: StreamVersion) -> bool {
        match self {
            Self::NoStream => actual.get() == StreamVersion::EMPTY.get(),
            Self::Exact(expected) => expected.get() == actual.get(),
        }
    }

    /// Returns the stream version represented by this guard.
    pub const fn stream_version(self) -> StreamVersion {
        match self {
            Self::NoStream => StreamVersion::EMPTY,
            Self::Exact(version) => version,
        }
    }
}

impl fmt::Display for ExpectedVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoStream => formatter.write_str("no stream"),
            Self::Exact(version) => write!(formatter, "exact version {version}"),
        }
    }
}

/// A validated mutation idempotency key.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Creates an idempotency key.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidIdempotencyKey> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(InvalidIdempotencyKey);
        }
        Ok(Self(value))
    }

    /// Returns the idempotency key as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// An idempotency key cannot be empty.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("idempotency key must not be empty")]
pub struct InvalidIdempotencyKey;

/// An event identity assigned by the event log.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct EventId(String);

impl EventId {
    /// Creates an event identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidEventId> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(InvalidEventId);
        }
        Ok(Self(value))
    }

    /// Returns the event identifier as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An event identifier cannot be empty.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("event identifier must not be empty")]
pub struct InvalidEventId;

/// A committed, versioned event.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EventEnvelope<E> {
    event_id: EventId,
    stream: StreamId,
    version: StreamVersion,
    idempotency_key: IdempotencyKey,
    event: E,
}

impl<E> EventEnvelope<E> {
    /// Creates an envelope after an event log has committed an event.
    pub fn committed(
        event_id: EventId,
        stream: StreamId,
        version: StreamVersion,
        idempotency_key: IdempotencyKey,
        event: E,
    ) -> Self {
        Self {
            event_id,
            stream,
            version,
            idempotency_key,
            event,
        }
    }

    /// Returns the event identity.
    pub fn event_id(&self) -> &EventId {
        &self.event_id
    }

    /// Returns the stream identity.
    pub fn stream(&self) -> &StreamId {
        &self.stream
    }

    /// Returns the committed stream version.
    pub const fn version(&self) -> StreamVersion {
        self.version
    }

    /// Returns the mutation idempotency key.
    pub fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    /// Returns the domain event.
    pub const fn event(&self) -> &E {
        &self.event
    }
}

/// Loaded stream history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventHistory<E> {
    stream: StreamId,
    events: Vec<EventEnvelope<E>>,
}

impl<E> EventHistory<E> {
    /// Creates loaded history. Runtime folding validates its stream and version invariants.
    pub fn loaded(stream: StreamId, events: Vec<EventEnvelope<E>>) -> Self {
        Self { stream, events }
    }

    /// Returns the loaded stream.
    pub fn stream(&self) -> &StreamId {
        &self.stream
    }

    /// Returns the loaded events.
    pub fn events(&self) -> &[EventEnvelope<E>] {
        &self.events
    }

    /// Returns the current version implied by the final event.
    pub fn current_version(&self) -> StreamVersion {
        self.events
            .last()
            .map_or(StreamVersion::EMPTY, EventEnvelope::version)
    }

    /// Consumes the history into its events.
    pub fn into_events(self) -> Vec<EventEnvelope<E>> {
        self.events
    }

    /// Checks that every envelope belongs to this stream and versions are contiguous from one.
    pub fn validate(&self) -> Result<(), HistoryInvariantError> {
        let mut expected = StreamVersion::new(1);
        for envelope in &self.events {
            if envelope.stream() != &self.stream {
                return Err(HistoryInvariantError::WrongStream {
                    expected: Box::new(self.stream.clone()),
                    actual: Box::new(envelope.stream().clone()),
                });
            }
            if envelope.version() != expected {
                return Err(HistoryInvariantError::NonContiguousVersion {
                    expected,
                    actual: envelope.version(),
                });
            }
            expected = expected
                .checked_next()
                .ok_or(HistoryInvariantError::VersionExhausted)?;
        }
        Ok(())
    }
}

/// Loaded history violated the event-log contract.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum HistoryInvariantError {
    /// An envelope belongs to another stream.
    #[error("history for {expected} contained an event from {actual}")]
    WrongStream {
        /// Stream that was loaded.
        expected: Box<StreamId>,
        /// Stream found on an envelope.
        actual: Box<StreamId>,
    },
    /// Event versions are not contiguous and one-based.
    #[error("expected event version {expected}, found {actual}")]
    NonContiguousVersion {
        /// Required next version.
        expected: StreamVersion,
        /// Envelope version that was found.
        actual: StreamVersion,
    },
    /// The version counter cannot advance.
    #[error("stream version exhausted")]
    VersionExhausted,
}

/// An atomic append guarded by exact stream version and idempotency key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardedAppend<E> {
    stream: StreamId,
    expected: ExpectedVersion,
    idempotency_key: IdempotencyKey,
    events: Vec<E>,
}

impl<E> GuardedAppend<E> {
    /// Creates an append request.
    pub fn new(
        stream: StreamId,
        expected: ExpectedVersion,
        idempotency_key: IdempotencyKey,
        events: Vec<E>,
    ) -> Self {
        Self {
            stream,
            expected,
            idempotency_key,
            events,
        }
    }

    /// Returns the target stream.
    pub fn stream(&self) -> &StreamId {
        &self.stream
    }

    /// Returns the exact expected version.
    pub const fn expected(&self) -> ExpectedVersion {
        self.expected
    }

    /// Returns the idempotency key.
    pub fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    /// Returns the proposed domain events.
    pub fn events(&self) -> &[E] {
        &self.events
    }

    /// Consumes the request into its parts.
    pub fn into_parts(self) -> (StreamId, ExpectedVersion, IdempotencyKey, Vec<E>) {
        (
            self.stream,
            self.expected,
            self.idempotency_key,
            self.events,
        )
    }
}

/// Whether an append committed events or replayed an earlier idempotent result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppendDisposition {
    /// Events were committed by this call.
    Committed,
    /// Identical events from an earlier call were returned.
    Replayed,
}

/// Successful guarded append result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppendReceipt<E> {
    disposition: AppendDisposition,
    events: Vec<EventEnvelope<E>>,
}

impl<E> AppendReceipt<E> {
    /// Creates a successful append receipt.
    pub fn new(disposition: AppendDisposition, events: Vec<EventEnvelope<E>>) -> Self {
        Self {
            disposition,
            events,
        }
    }

    /// Returns whether this call committed or replayed.
    pub const fn disposition(&self) -> AppendDisposition {
        self.disposition
    }

    /// Returns the committed event envelopes.
    pub fn events(&self) -> &[EventEnvelope<E>] {
        &self.events
    }

    /// Returns the final committed version, if the receipt contains events.
    pub fn through_version(&self) -> Option<StreamVersion> {
        self.events.last().map(EventEnvelope::version)
    }
}

/// An exact expected-version guard failed.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("expected {expected}, found stream version {actual}")]
pub struct VersionConflict {
    /// Caller-declared expected version.
    pub expected: ExpectedVersion,
    /// Actual current version.
    pub actual: StreamVersion,
}

/// An idempotency key was reused for a different request.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("idempotency key {key} was reused with different append content")]
pub struct IdempotencyConflict {
    /// Conflicting key.
    pub key: IdempotencyKey,
}

/// A guarded append failed.
#[derive(Debug, Error)]
pub enum AppendError<E> {
    /// Optimistic concurrency guard failed.
    #[error(transparent)]
    Version(#[from] VersionConflict),
    /// Idempotency key was reused inconsistently.
    #[error(transparent)]
    Idempotency(#[from] IdempotencyConflict),
    /// Persistence adapter failed.
    #[error("event-log adapter failed")]
    Adapter(E),
}

/// Asynchronous persistence port required by aggregate execution.
///
/// The boxed-future shape keeps the port object-safe and lets SQLite, PostgreSQL, or other
/// adapters use their existing asynchronous execution model without coupling this crate to an
/// executor or an event-log implementation.
pub trait EventLog<E>: Send {
    /// Adapter-specific failure.
    type Error;

    /// Loads the complete history for one typed stream.
    fn load<'a>(
        &'a mut self,
        stream: &'a StreamId,
    ) -> BoxFuture<'a, Result<EventHistory<E>, Self::Error>>;

    /// Atomically appends events under expected-version and idempotency guards.
    ///
    /// Implementations must check an existing idempotency record before the current-version guard.
    /// Reusing a key with the same events returns the original envelopes even though the stream has
    /// advanced since the first call; reusing it with different events is an idempotency conflict.
    fn append_guarded(
        &mut self,
        request: GuardedAppend<E>,
    ) -> BoxFuture<'_, Result<AppendReceipt<E>, AppendError<Self::Error>>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(realm: Option<&str>) -> StreamId {
        StreamId::new(
            ServiceId::new("todo").unwrap(),
            TenantId::new("tenant-a").unwrap(),
            realm.map(|value| RealmId::new(value).unwrap()),
            "list",
            "one",
        )
        .unwrap()
    }

    #[test]
    fn partition_encoding_is_versioned_length_delimited_and_realm_exact() {
        let absent = stream(None);
        let default = stream(Some("default"));

        assert_eq!(
            absent.partition_key(),
            "service-stream/1|4:todo|8:tenant-a|0|4:list|3:one"
        );
        assert_eq!(
            default.partition_key(),
            "service-stream/1|4:todo|8:tenant-a|1|7:default|4:list|3:one"
        );
        assert_ne!(absent, default);
        assert_ne!(absent.partition_key(), default.partition_key());
        assert_eq!(absent.to_string(), absent.partition_key());
        assert_eq!(
            serde_json::to_string(&absent).unwrap(),
            r#""service-stream/1|4:todo|8:tenant-a|0|4:list|3:one""#
        );
    }

    #[test]
    fn length_prefixes_disambiguate_delimiters_inside_coordinates() {
        let left = StreamId::new(
            ServiceId::new("a|1:b").unwrap(),
            TenantId::new("c").unwrap(),
            None,
            "d",
            "e",
        )
        .unwrap();
        let right = StreamId::new(
            ServiceId::new("a").unwrap(),
            TenantId::new("1:b|c").unwrap(),
            None,
            "d",
            "e",
        )
        .unwrap();

        assert_ne!(left.partition_key(), right.partition_key());
        assert!(left.partition_key().starts_with("service-stream/1|5:a|1:b"));
        assert!(
            right
                .partition_key()
                .starts_with("service-stream/1|1:a|5:1:b|c")
        );
    }

    #[test]
    fn no_stream_and_exact_versions_remain_distinct_caller_declarations() {
        assert!(ExpectedVersion::no_stream().matches(StreamVersion::EMPTY));
        assert!(!ExpectedVersion::no_stream().matches(StreamVersion::new(1)));
        assert_eq!(
            ExpectedVersion::exact(StreamVersion::new(3)).stream_version(),
            StreamVersion::new(3)
        );
        assert_ne!(
            ExpectedVersion::no_stream(),
            ExpectedVersion::exact(StreamVersion::EMPTY)
        );
    }
}
