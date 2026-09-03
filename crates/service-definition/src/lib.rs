//! Strict, human-authored runtime annotations for one ESS-defined service.
//!
//! ESS owns domain meaning. This document adds only runtime decisions needed to realize that
//! meaning as a standalone service. Parsing is fail-closed: unknown fields, malformed identifiers,
//! duplicate declarations, dangling local references, and caller-controlled authority coordinates
//! are refused before a runtime IR can be minted.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use ess_compiler::refs::{CommandRef, DeclaredTypeRef, EventRef, ViewRef};

/// The only service-definition format understood by this crate.
pub const SERVICE_DEFINITION_FORMAT: &str = "service-definition/3";

/// A stable lowercase identifier within one service definition.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct DefinitionId(String);

impl DefinitionId {
    /// Parses a stable identifier.
    pub fn new(value: impl AsRef<str>) -> Result<Self, DefinitionIdError> {
        let value = value.as_ref();
        let mut previous_separator = false;
        let valid = (1..=80).contains(&value.len())
            && value.is_ascii()
            && value
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase())
            && value.bytes().all(|byte| {
                let separator = matches!(byte, b'-' | b'_');
                let accepted = byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || (separator && !previous_separator);
                previous_separator = separator;
                accepted
            })
            && !previous_separator;
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(DefinitionIdError(value.to_owned()))
        }
    }

    /// Returns the validated identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DefinitionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for DefinitionId {
    type Err = DefinitionIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl<'de> serde::Deserialize<'de> for DefinitionId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// An invalid [`DefinitionId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionIdError(String);

impl fmt::Display for DefinitionIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid definition identifier {:?}: expected 1-80 lowercase ASCII letters or digits separated by single `-` or `_` characters",
            self.0
        )
    }
}

impl std::error::Error for DefinitionIdError {}

/// Stable versioned identity of an obligation implementation supplied by this SDK.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct ObligationProviderId(String);

impl ObligationProviderId {
    /// Parses an SDK obligation identity such as `sdk.aggregate.event-sourced/v1`.
    pub fn new(value: impl AsRef<str>) -> Result<Self, ObligationProviderIdError> {
        let value = value.as_ref();
        let Some((name, version)) = value.rsplit_once('/') else {
            return Err(ObligationProviderIdError(value.to_owned()));
        };
        let valid_name = name.starts_with("sdk.")
            && name.len() <= 120
            && name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
            });
        let valid_version = version
            .strip_prefix('v')
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()));
        if valid_name && valid_version {
            Ok(Self(value.to_owned()))
        } else {
            Err(ObligationProviderIdError(value.to_owned()))
        }
    }

    /// Returns the exact versioned provider identity.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ObligationProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for ObligationProviderId {
    type Err = ObligationProviderIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl<'de> serde::Deserialize<'de> for ObligationProviderId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// An invalid [`ObligationProviderId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObligationProviderIdError(String);

impl fmt::Display for ObligationProviderIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid SDK obligation provider {:?}: expected sdk.<lowercase-name>/v<positive-format-version>",
            self.0
        )
    }
}

impl std::error::Error for ObligationProviderIdError {}

/// Admission policy for the optional realm carried by verified authentication.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RealmPolicy {
    /// A realm must be present in the verified authority.
    Required,
    /// Both absence and presence are accepted without normalization.
    Optional,
    /// A realm must be absent from the verified authority.
    Forbidden,
}

/// Supported public delivery boundary for one generated service.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServiceDelivery {
    /// Expose the generated operations through an Identity-authenticated HTTP service.
    IdentityHttp {
        /// Exact resource audience accepted by the generated server and requested by clients.
        audience: String,
    },
    /// Expose the generated operations only through a product-composed Connector factory.
    ComposedConnector,
}

/// Where an aggregate stream identity comes from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StreamIdSource {
    /// A semantic ESS command field carries the aggregate identity.
    CommandField {
        /// Command input field name.
        field: String,
    },
    /// The runtime mints a `UUIDv7` before checking the no-stream precondition.
    GeneratedUuidV7,
}

/// Optimistic-concurrency input for an intent.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExpectedVersionSource {
    /// Creation requires that no aggregate stream exists.
    NoStream,
    /// An operation-envelope field carries the exact expected version.
    OperationField {
        /// Transport-neutral operation field name.
        field: String,
    },
}

/// Idempotency identity for one mutation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IdempotencySource {
    /// An operation-envelope field carries the idempotency key.
    OperationField {
        /// Transport-neutral operation field name.
        field: String,
    },
    /// The trusted request context supplies its request identifier.
    RequestId,
}

/// A verified authority fact usable by guards and event bindings.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ContextValue {
    /// Required authenticated tenant.
    TenantId,
    /// Exact optional realm; `None` is never normalized to `"default"`.
    RealmIdOptional,
    /// Authenticated user.
    UserId,
    /// Current authority or principal.
    CurrentAuthority,
    /// Optional executor acting for the authority.
    ExecutorOptional,
}

/// One fact a guard is permitted to inspect.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GuardRead {
    /// A field of the intent's ESS command input.
    CommandField {
        /// Command field name.
        field: String,
    },
    /// A field in the query's ESS view row.
    ViewField {
        /// View field name.
        field: String,
    },
    /// A fact bound by verified authentication.
    Context {
        /// The verified context value.
        value: ContextValue,
    },
}

/// A named, stable refusal guard attached to an operation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardDefinition {
    /// Guard identity within the operation.
    pub name: DefinitionId,
    /// Stable public refusal code.
    pub refusal_code: DefinitionId,
    /// Closed set of values this guard may read.
    pub reads: Vec<GuardRead>,
}

/// Source for one emitted-event field ESS deliberately leaves undetermined.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventBindingSource {
    /// The resolved aggregate stream identity.
    StreamId,
    /// A semantic command field.
    CommandField {
        /// Command field name.
        field: String,
    },
    /// A verified authority value.
    Context {
        /// The verified context value.
        value: ContextValue,
    },
    /// A fresh `UUIDv7`.
    GeneratedUuidV7,
    /// A named realization obligation derives the value.
    Obligation {
        /// Top-level obligation identity.
        name: DefinitionId,
    },
}

/// Binding for one field of an event emitted by the intent's command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventBinding {
    /// ESS event name.
    pub event: EventRef,
    /// Event payload field.
    pub field: String,
    /// Runtime source for that field.
    pub source: EventBindingSource,
}

/// One plaintext operation field staged into an opaque content reference.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentBinding {
    /// Top-level content policy identity.
    pub content: DefinitionId,
    /// Plaintext operation field consumed before command construction.
    pub input_field: String,
    /// ESS command field receiving only the opaque reference.
    pub command_reference_field: String,
}

/// An authenticated mutation mapped to exactly one ESS command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentDefinition {
    /// Stable operation identity.
    pub name: DefinitionId,
    /// Exact OAuth scope required before this operation's body is decoded.
    pub scope: String,
    /// ESS semantic command.
    pub command: CommandRef,
    /// Aggregate stream identity source.
    pub stream_id: StreamIdSource,
    /// Optimistic-concurrency source.
    pub expected_version: ExpectedVersionSource,
    /// Idempotency source.
    pub idempotency: IdempotencySource,
    /// Guards evaluated after fold and before decision.
    #[serde(default)]
    pub guards: Vec<GuardDefinition>,
    /// Plaintext-to-reference staging bindings.
    #[serde(default)]
    pub content: Vec<ContentBinding>,
    /// Bindings for otherwise undetermined event fields.
    #[serde(default)]
    pub event_bindings: Vec<EventBinding>,
    /// Projections driven after a successful append.
    #[serde(default)]
    pub projections: Vec<DefinitionId>,
    /// Named realization obligations this intent owes.
    #[serde(default)]
    pub obligations: Vec<DefinitionId>,
}

/// One caller selector bound to an ESS view field.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuerySelector {
    /// Operation parameter name.
    pub parameter: String,
    /// ESS view field selected by the parameter.
    pub view_field: String,
}

/// Stable query sort direction.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    /// Ascending order.
    Ascending,
    /// Descending order.
    Descending,
}

/// One stable query ordering key.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuerySort {
    /// ESS view field.
    pub view_field: String,
    /// Sort direction.
    pub direction: SortDirection,
}

/// Delivery guarantee exposed by a query.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum QueryDelivery {
    /// A successful mutation is visible to its authenticated caller before it returns.
    ReadYourWrites,
    /// The view catches up asynchronously.
    Eventual,
}

/// An authenticated query mapped to exactly one ESS view.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryDefinition {
    /// Stable operation identity.
    pub name: DefinitionId,
    /// Exact OAuth scope required before this operation's body is decoded.
    pub scope: String,
    /// ESS semantic view.
    pub view: ViewRef,
    /// Projection that materializes the view.
    pub projection: DefinitionId,
    /// Explicit caller selectors. Authority coordinates are forbidden here.
    #[serde(default)]
    pub selectors: Vec<QuerySelector>,
    /// Guards evaluated before projection access.
    #[serde(default)]
    pub guards: Vec<GuardDefinition>,
    /// Non-empty stable ordering used for deterministic reads and cursors.
    pub sort: Vec<QuerySort>,
    /// Declared visibility guarantee.
    pub delivery: QueryDelivery,
    /// Named realization obligations this query owes.
    #[serde(default)]
    pub obligations: Vec<DefinitionId>,
}

/// How a projection is updated from accepted events.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionDelivery {
    /// Projection and append commit atomically.
    InlineTransactional,
    /// Projection consumes a durable event feed.
    CatchUp,
}

/// Runtime materialization policy for one ESS view.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionDefinition {
    /// Stable projection identity.
    pub name: DefinitionId,
    /// ESS semantic view.
    pub view: ViewRef,
    /// Projection delivery guarantee.
    pub delivery: ProjectionDelivery,
    /// Named realization obligations this projection owes.
    #[serde(default)]
    pub obligations: Vec<DefinitionId>,
}

/// Custody contract for free-form, erasable content.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ContentCustody {
    /// Plaintext remains outside Eventlog; events carry only an erasable opaque reference.
    ExternalErasable,
}

/// Reusable staging policy for externally custodied content.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentDefinition {
    /// Stable content policy identity.
    pub name: DefinitionId,
    /// ESS newtype carried by commands and events as an opaque reference.
    pub reference_type: DeclaredTypeRef,
    /// Accepted media types.
    pub media_types: Vec<String>,
    /// Maximum plaintext size admitted before staging.
    pub max_bytes: u64,
    /// Storage and erasure contract.
    pub custody: ContentCustody,
}

/// One binding to a versioned obligation implementation supplied by the SDK.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObligationDefinition {
    /// Stable obligation identity.
    pub name: DefinitionId,
    /// Exact SDK catalog entry that implements this obligation.
    pub provider: ObligationProviderId,
    /// Provider-defined parameters bound to ESS handles, field paths, or reviewed constants.
    #[serde(default)]
    pub bindings: BTreeMap<DefinitionId, String>,
    /// Non-empty human-readable contract.
    pub description: String,
}

/// Strict runtime annotations attached to one resolved ESS model.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceDefinition {
    /// Format discriminator; must equal [`SERVICE_DEFINITION_FORMAT`].
    pub format: String,
    /// Stable service identity.
    pub service: DefinitionId,
    /// Explicit public delivery boundary; there is no implicit transport.
    pub delivery: ServiceDelivery,
    /// Admission policy applied to authentication's optional realm.
    pub realm: RealmPolicy,
    /// Mutation operations.
    #[serde(default)]
    pub intents: Vec<IntentDefinition>,
    /// Projection materializations.
    #[serde(default)]
    pub projections: Vec<ProjectionDefinition>,
    /// Query operations.
    #[serde(default)]
    pub queries: Vec<QueryDefinition>,
    /// External content policies.
    #[serde(default)]
    pub content: Vec<ContentDefinition>,
    /// Versioned SDK-provided obligation bindings.
    #[serde(default)]
    pub obligations: Vec<ObligationDefinition>,
}

impl ServiceDefinition {
    /// Reads and validates strict YAML.
    pub fn from_yaml(text: &str) -> Result<Self, DefinitionDiagnostics> {
        let definition: Self = serde_yaml::from_str(text)
            .map_err(|error| DefinitionDiagnostics::syntax(error.to_string()))?;
        definition.validate()?;
        Ok(definition)
    }

    /// Reads and validates strict JSON.
    pub fn from_json(text: &str) -> Result<Self, DefinitionDiagnostics> {
        let definition: Self = serde_json::from_str(text)
            .map_err(|error| DefinitionDiagnostics::syntax(error.to_string()))?;
        definition.validate()?;
        Ok(definition)
    }

    /// Validates programmatically constructed definitions with the same rules as parsing.
    #[allow(clippy::too_many_lines)]
    pub fn validate(&self) -> Result<(), DefinitionDiagnostics> {
        let mut diagnostics = Vec::new();
        if self.format != SERVICE_DEFINITION_FORMAT {
            diagnostics.push(DefinitionDiagnostic::new(
                DefinitionCode::UnsupportedFormat,
                "format",
                format!(
                    "expected {SERVICE_DEFINITION_FORMAT:?}, found {:?}",
                    self.format
                ),
            ));
        }
        if self.intents.is_empty() && self.queries.is_empty() {
            diagnostics.push(DefinitionDiagnostic::new(
                DefinitionCode::Empty,
                "operations",
                "a service must declare at least one intent or query",
            ));
        }
        validate_delivery(&mut diagnostics, &self.delivery);

        let mut declarations = BTreeMap::<DefinitionId, &'static str>::new();
        let declaration_groups: [(&str, Vec<&DefinitionId>); 5] = [
            (
                "intent",
                self.intents.iter().map(|item| &item.name).collect(),
            ),
            (
                "projection",
                self.projections.iter().map(|item| &item.name).collect(),
            ),
            (
                "query",
                self.queries.iter().map(|item| &item.name).collect(),
            ),
            (
                "content",
                self.content.iter().map(|item| &item.name).collect(),
            ),
            (
                "obligation",
                self.obligations.iter().map(|item| &item.name).collect(),
            ),
        ];
        for (kind, names) in declaration_groups {
            for name in names {
                if let Some(previous) = declarations.insert(name.clone(), kind) {
                    diagnostics.push(DefinitionDiagnostic::new(
                        DefinitionCode::Duplicate,
                        format!("{kind}.{name}"),
                        format!("name is already used by a {previous}"),
                    ));
                }
            }
        }

        let projections: BTreeMap<_, _> = self
            .projections
            .iter()
            .map(|projection| (&projection.name, projection))
            .collect();
        let contents: BTreeSet<_> = self.content.iter().map(|content| &content.name).collect();
        let obligations: BTreeSet<_> = self
            .obligations
            .iter()
            .map(|obligation| &obligation.name)
            .collect();

        validate_obligation_definitions(&mut diagnostics, &self.obligations);
        for (index, content) in self.content.iter().enumerate() {
            validate_content(&mut diagnostics, index, content);
        }
        for (index, projection) in self.projections.iter().enumerate() {
            validate_obligation_refs(
                &mut diagnostics,
                &format!("projections[{index}].obligations"),
                &projection.obligations,
                &obligations,
            );
        }
        for (index, intent) in self.intents.iter().enumerate() {
            validate_scope(
                &mut diagnostics,
                &format!("intents[{index}].scope"),
                &intent.scope,
            );
            validate_intent(
                &mut diagnostics,
                index,
                intent,
                &projections,
                &contents,
                &obligations,
            );
        }
        for (index, query) in self.queries.iter().enumerate() {
            validate_scope(
                &mut diagnostics,
                &format!("queries[{index}].scope"),
                &query.scope,
            );
            validate_query(&mut diagnostics, index, query, &projections, &obligations);
        }

        if diagnostics.is_empty() {
            Ok(())
        } else {
            Err(DefinitionDiagnostics(diagnostics))
        }
    }

    /// Canonical pretty JSON with stable field order and a trailing newline.
    pub fn to_canonical_json(&self) -> String {
        let mut output = serde_json::to_string_pretty(self)
            .unwrap_or_else(|error| panic!("validated service definition serializes: {error}"));
        output.push('\n');
        output
    }
}

fn validate_delivery(diagnostics: &mut Vec<DefinitionDiagnostic>, delivery: &ServiceDelivery) {
    if let ServiceDelivery::IdentityHttp { audience } = delivery {
        let valid = (3..=256).contains(&audience.len())
            && audience.trim() == audience
            && audience.is_ascii()
            && audience
                .bytes()
                .all(|byte| !byte.is_ascii_whitespace() && !byte.is_ascii_control());
        if !valid {
            diagnostics.push(DefinitionDiagnostic::new(
                DefinitionCode::InvalidValue,
                "delivery.audience",
                "Identity HTTP audience must be 3-256 visible ASCII characters without whitespace",
            ));
        }
    }
}

fn validate_scope(diagnostics: &mut Vec<DefinitionDiagnostic>, path: &str, scope: &str) {
    let valid = (3..=128).contains(&scope.len())
        && scope
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
        && scope.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b':' | b'-' | b'_')
        });
    if !valid {
        diagnostics.push(DefinitionDiagnostic::new(
            DefinitionCode::InvalidValue,
            path,
            "scope must be a 3-128 character lowercase OAuth scope token",
        ));
    }
}

fn validate_obligation_definitions(
    diagnostics: &mut Vec<DefinitionDiagnostic>,
    obligations: &[ObligationDefinition],
) {
    for (index, obligation) in obligations.iter().enumerate() {
        nonempty(
            diagnostics,
            &format!("obligations[{index}].description"),
            &obligation.description,
        );
        for (binding, value) in &obligation.bindings {
            nonempty(
                diagnostics,
                &format!("obligations[{index}].bindings.{binding}"),
                value,
            );
        }
    }
}

fn validate_content(
    diagnostics: &mut Vec<DefinitionDiagnostic>,
    index: usize,
    content: &ContentDefinition,
) {
    let path = format!("content[{index}]");
    if content.max_bytes == 0 {
        diagnostics.push(DefinitionDiagnostic::new(
            DefinitionCode::InvalidValue,
            format!("{path}.max_bytes"),
            "must be greater than zero",
        ));
    }
    if content.media_types.is_empty() {
        diagnostics.push(DefinitionDiagnostic::new(
            DefinitionCode::Empty,
            format!("{path}.media_types"),
            "at least one media type is required",
        ));
    }
    let mut seen = BTreeSet::new();
    for (media_index, media_type) in content.media_types.iter().enumerate() {
        if !valid_media_type(media_type) {
            diagnostics.push(DefinitionDiagnostic::new(
                DefinitionCode::InvalidValue,
                format!("{path}.media_types[{media_index}]"),
                "expected a lowercase type/subtype without whitespace",
            ));
        }
        if !seen.insert(media_type) {
            diagnostics.push(DefinitionDiagnostic::new(
                DefinitionCode::Duplicate,
                format!("{path}.media_types[{media_index}]"),
                "media type is declared more than once",
            ));
        }
    }
}

fn validate_intent(
    diagnostics: &mut Vec<DefinitionDiagnostic>,
    index: usize,
    intent: &IntentDefinition,
    projections: &BTreeMap<&DefinitionId, &ProjectionDefinition>,
    contents: &BTreeSet<&DefinitionId>,
    obligations: &BTreeSet<&DefinitionId>,
) {
    let path = format!("intents[{index}]");
    if let StreamIdSource::CommandField { field } = &intent.stream_id {
        operation_field(diagnostics, &format!("{path}.stream_id.field"), field);
    }
    let mut envelope_fields = BTreeSet::new();
    if let ExpectedVersionSource::OperationField { field } = &intent.expected_version {
        operation_field(
            diagnostics,
            &format!("{path}.expected_version.field"),
            field,
        );
        envelope_fields.insert(field.as_str());
    }
    if let IdempotencySource::OperationField { field } = &intent.idempotency {
        operation_field(diagnostics, &format!("{path}.idempotency.field"), field);
        if !envelope_fields.insert(field) {
            duplicate_field(diagnostics, &format!("{path}.idempotency.field"));
        }
    }

    validate_guards(diagnostics, &format!("{path}.guards"), &intent.guards);
    let mut bindings = BTreeSet::new();
    for (binding_index, binding) in intent.content.iter().enumerate() {
        let binding_path = format!("{path}.content[{binding_index}]");
        if !contents.contains(&binding.content) {
            dangling(
                diagnostics,
                &format!("{binding_path}.content"),
                "content policy",
                &binding.content,
            );
        }
        operation_field(
            diagnostics,
            &format!("{binding_path}.input_field"),
            &binding.input_field,
        );
        operation_field(
            diagnostics,
            &format!("{binding_path}.command_reference_field"),
            &binding.command_reference_field,
        );
        if !envelope_fields.insert(binding.input_field.as_str()) {
            duplicate_field(diagnostics, &format!("{binding_path}.input_field"));
        }
        if !bindings.insert(binding.command_reference_field.as_str()) {
            duplicate_field(
                diagnostics,
                &format!("{binding_path}.command_reference_field"),
            );
        }
    }

    let mut event_fields = BTreeSet::new();
    for (binding_index, binding) in intent.event_bindings.iter().enumerate() {
        let binding_path = format!("{path}.event_bindings[{binding_index}]");
        nonempty(
            diagnostics,
            &format!("{binding_path}.field"),
            &binding.field,
        );
        let key = (binding.event.to_string(), binding.field.as_str());
        if !event_fields.insert(key) {
            duplicate_field(diagnostics, &binding_path);
        }
        if let EventBindingSource::CommandField { field } = &binding.source {
            operation_field(diagnostics, &format!("{binding_path}.source.field"), field);
        }
        if let EventBindingSource::Obligation { name } = &binding.source
            && !obligations.contains(name)
        {
            dangling(
                diagnostics,
                &format!("{binding_path}.source.name"),
                "obligation",
                name,
            );
        }
    }

    let mut seen_projections = BTreeSet::new();
    for (projection_index, projection) in intent.projections.iter().enumerate() {
        let projection_path = format!("{path}.projections[{projection_index}]");
        if !projections.contains_key(projection) {
            dangling(diagnostics, &projection_path, "projection", projection);
        }
        if !seen_projections.insert(projection) {
            duplicate_field(diagnostics, &projection_path);
        }
    }
    validate_obligation_refs(
        diagnostics,
        &format!("{path}.obligations"),
        &intent.obligations,
        obligations,
    );
}

fn validate_query(
    diagnostics: &mut Vec<DefinitionDiagnostic>,
    index: usize,
    query: &QueryDefinition,
    projections: &BTreeMap<&DefinitionId, &ProjectionDefinition>,
    obligations: &BTreeSet<&DefinitionId>,
) {
    let path = format!("queries[{index}]");
    match projections.get(&query.projection) {
        None => dangling(
            diagnostics,
            &format!("{path}.projection"),
            "projection",
            &query.projection,
        ),
        Some(projection) if projection.view != query.view => {
            diagnostics.push(DefinitionDiagnostic::new(
                DefinitionCode::InvalidReference,
                format!("{path}.projection"),
                format!(
                    "projection {} materializes {}, not {}",
                    query.projection, projection.view, query.view
                ),
            ));
        }
        Some(_) => {}
    }

    let mut parameters = BTreeSet::new();
    let mut selected_fields = BTreeSet::new();
    for (selector_index, selector) in query.selectors.iter().enumerate() {
        let selector_path = format!("{path}.selectors[{selector_index}]");
        operation_field(
            diagnostics,
            &format!("{selector_path}.parameter"),
            &selector.parameter,
        );
        nonempty(
            diagnostics,
            &format!("{selector_path}.view_field"),
            &selector.view_field,
        );
        if !parameters.insert(selector.parameter.as_str()) {
            duplicate_field(diagnostics, &format!("{selector_path}.parameter"));
        }
        if !selected_fields.insert(selector.view_field.as_str()) {
            duplicate_field(diagnostics, &format!("{selector_path}.view_field"));
        }
    }
    validate_guards(diagnostics, &format!("{path}.guards"), &query.guards);
    if query.sort.is_empty() {
        diagnostics.push(DefinitionDiagnostic::new(
            DefinitionCode::Empty,
            format!("{path}.sort"),
            "at least one stable sort field is required",
        ));
    }
    let mut sort_fields = BTreeSet::new();
    for (sort_index, sort) in query.sort.iter().enumerate() {
        let sort_path = format!("{path}.sort[{sort_index}].view_field");
        nonempty(diagnostics, &sort_path, &sort.view_field);
        if !sort_fields.insert(sort.view_field.as_str()) {
            duplicate_field(diagnostics, &sort_path);
        }
    }
    validate_obligation_refs(
        diagnostics,
        &format!("{path}.obligations"),
        &query.obligations,
        obligations,
    );
}

fn validate_guards(
    diagnostics: &mut Vec<DefinitionDiagnostic>,
    path: &str,
    guards: &[GuardDefinition],
) {
    let mut names = BTreeSet::new();
    let mut refusal_codes = BTreeSet::new();
    for (index, guard) in guards.iter().enumerate() {
        let guard_path = format!("{path}[{index}]");
        if !names.insert(&guard.name) {
            duplicate_field(diagnostics, &format!("{guard_path}.name"));
        }
        if !refusal_codes.insert(&guard.refusal_code) {
            duplicate_field(diagnostics, &format!("{guard_path}.refusal_code"));
        }
        if guard.reads.is_empty() {
            diagnostics.push(DefinitionDiagnostic::new(
                DefinitionCode::Empty,
                format!("{guard_path}.reads"),
                "a guard must declare every value it reads",
            ));
        }
        let mut reads = BTreeSet::new();
        for (read_index, read) in guard.reads.iter().enumerate() {
            let encoded = serde_json::to_string(read)
                .unwrap_or_else(|error| panic!("guard read serializes: {error}"));
            if !reads.insert(encoded) {
                duplicate_field(diagnostics, &format!("{guard_path}.reads[{read_index}]"));
            }
            match read {
                GuardRead::CommandField { field } => operation_field(
                    diagnostics,
                    &format!("{guard_path}.reads[{read_index}].field"),
                    field,
                ),
                GuardRead::ViewField { field } => nonempty(
                    diagnostics,
                    &format!("{guard_path}.reads[{read_index}].field"),
                    field,
                ),
                GuardRead::Context { .. } => {}
            }
        }
    }
}

fn validate_obligation_refs(
    diagnostics: &mut Vec<DefinitionDiagnostic>,
    path: &str,
    references: &[DefinitionId],
    obligations: &BTreeSet<&DefinitionId>,
) {
    let mut seen = BTreeSet::new();
    for (index, reference) in references.iter().enumerate() {
        let reference_path = format!("{path}[{index}]");
        if !obligations.contains(reference) {
            dangling(diagnostics, &reference_path, "obligation", reference);
        }
        if !seen.insert(reference) {
            duplicate_field(diagnostics, &reference_path);
        }
    }
}

fn nonempty(diagnostics: &mut Vec<DefinitionDiagnostic>, path: &str, value: &str) {
    if value.trim().is_empty() {
        diagnostics.push(DefinitionDiagnostic::new(
            DefinitionCode::Empty,
            path,
            "must not be empty",
        ));
    }
}

fn operation_field(diagnostics: &mut Vec<DefinitionDiagnostic>, path: &str, value: &str) {
    nonempty(diagnostics, path, value);
    if authority_coordinate(value) {
        diagnostics.push(DefinitionDiagnostic::new(
            DefinitionCode::AuthorityCoordinate,
            path,
            format!(
                "{value:?} is authentication-derived and must not be encoded as an operation field"
            ),
        ));
    }
}

fn authority_coordinate(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "realm"
            | "realm_id"
            | "realmid"
            | "tenant"
            | "tenant_id"
            | "tenantid"
            | "user"
            | "user_id"
            | "userid"
            | "current_user"
            | "authority"
            | "authority_id"
            | "authorityid"
            | "current_authority"
            | "principal"
            | "principal_id"
            | "principalid"
            | "executor"
            | "executor_id"
            | "executorid"
    )
}

fn valid_media_type(value: &str) -> bool {
    let Some((kind, subtype)) = value.split_once('/') else {
        return false;
    };
    !kind.is_empty()
        && !subtype.is_empty()
        && !subtype.contains('/')
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'/' | b'+' | b'-' | b'.')
        })
}

fn dangling(
    diagnostics: &mut Vec<DefinitionDiagnostic>,
    path: &str,
    kind: &str,
    name: &DefinitionId,
) {
    diagnostics.push(DefinitionDiagnostic::new(
        DefinitionCode::InvalidReference,
        path,
        format!("{kind} {name:?} is not declared"),
    ));
}

fn duplicate_field(diagnostics: &mut Vec<DefinitionDiagnostic>, path: &str) {
    diagnostics.push(DefinitionDiagnostic::new(
        DefinitionCode::Duplicate,
        path,
        "value is declared more than once",
    ));
}

/// Stable machine-readable definition diagnostic code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DefinitionCode {
    /// JSON or YAML could not be decoded under the strict schema.
    Syntax,
    /// The document format is not supported.
    UnsupportedFormat,
    /// A required collection or string is empty.
    Empty,
    /// A declaration or field is repeated.
    Duplicate,
    /// A value is malformed or outside its allowed bounds.
    InvalidValue,
    /// A local reference does not resolve or resolves to the wrong declaration.
    InvalidReference,
    /// An authentication-derived coordinate was exposed as caller input.
    AuthorityCoordinate,
}

/// One structured service-definition diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DefinitionDiagnostic {
    /// Stable diagnostic code.
    pub code: DefinitionCode,
    /// Definition path that needs repair.
    pub path: String,
    /// Human-readable repair guidance.
    pub message: String,
}

impl DefinitionDiagnostic {
    fn new(code: DefinitionCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
}

/// All definition errors found in one validation pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionDiagnostics(Vec<DefinitionDiagnostic>);

impl DefinitionDiagnostics {
    fn syntax(message: String) -> Self {
        Self(vec![DefinitionDiagnostic::new(
            DefinitionCode::Syntax,
            "document",
            message,
        )])
    }

    /// Structured diagnostics in deterministic discovery order.
    pub fn diagnostics(&self) -> &[DefinitionDiagnostic] {
        &self.0
    }
}

impl fmt::Display for DefinitionDiagnostics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("service definition was refused")?;
        for diagnostic in &self.0 {
            write!(
                formatter,
                "\n- {:?} at {}: {}",
                diagnostic.code, diagnostic.path, diagnostic.message
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for DefinitionDiagnostics {}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPLETE: &str = r"
format: service-definition/3
service: todo
delivery: { kind: composed_connector }
realm: optional
content:
  - name: item_content
    reference_type: todo.list.ContentRef
    media_types: [text/plain, text/markdown]
    max_bytes: 65536
    custody: external_erasable
obligations:
  - name: inherit_scope
    provider: sdk.derive.inherit-parent-authority/v1
    bindings:
      parent: todo.list.TodoList
      child: todo.list.Item
      parent_owner: todo.list.TodoList.owner
      parent_scopes: todo.list.TodoList.scopes
      child_owner: todo.list.Item.owner
      child_scopes: todo.list.Item.scopes
    description: Inherit the authenticated list scope without accepting it from the caller.
projections:
  - name: item_by_id
    view: todo.list.ItemById
    delivery: inline_transactional
    obligations: [inherit_scope]
intents:
  - name: create_item
    scope: todo.manage
    command: todo.list.CreateItem
    stream_id: { kind: command_field, field: item_id }
    expected_version: { kind: no_stream }
    idempotency: { kind: operation_field, field: idempotency_key }
    guards:
      - name: list_visible
        refusal_code: list_not_visible
        reads:
          - { kind: command_field, field: list_id }
          - { kind: context, value: current_authority }
    content:
      - content: item_content
        input_field: content
        command_reference_field: content_ref
    event_bindings:
      - event: todo.list.ItemCreated
        field: owner
        source: { kind: context, value: current_authority }
      - event: todo.list.ItemCreated
        field: scopes
        source: { kind: obligation, name: inherit_scope }
    projections: [item_by_id]
    obligations: [inherit_scope]
queries:
  - name: get_item
    scope: todo.read
    view: todo.list.ItemById
    projection: item_by_id
    selectors:
      - { parameter: item_id, view_field: item_id }
    guards:
      - name: visible
        refusal_code: not_visible
        reads:
          - { kind: view_field, field: owner }
          - { kind: context, value: current_authority }
    sort:
      - { view_field: item_id, direction: ascending }
    delivery: read_your_writes
    obligations: [inherit_scope]
";

    #[test]
    fn strict_roundtrip_preserves_every_annotation() {
        let definition = ServiceDefinition::from_yaml(COMPLETE).expect("valid definition");
        let canonical = definition.to_canonical_json();
        let reparsed = ServiceDefinition::from_json(&canonical).expect("canonical JSON reads");
        assert_eq!(reparsed, definition);
        assert_eq!(canonical, reparsed.to_canonical_json());
        for expected in [
            "item_content",
            "text/markdown",
            "65536",
            "external_erasable",
            "item_id",
            "idempotency_key",
            "list_visible",
            "list_not_visible",
            "content_ref",
            "ItemCreated",
            "inherit_scope",
            "sdk.derive.inherit-parent-authority/v1",
            "item_by_id",
            "inline_transactional",
            "get_item",
            "read_your_writes",
        ] {
            assert!(
                canonical.contains(expected),
                "missing {expected:?}: {canonical}"
            );
        }
    }

    #[test]
    fn unknown_fields_are_refused() {
        let text = COMPLETE.replace(
            "realm: optional",
            "realm: optional\nroute: /realms/default/todo",
        );
        let errors = ServiceDefinition::from_yaml(&text).expect_err("unknown route must fail");
        assert_eq!(errors.diagnostics()[0].code, DefinitionCode::Syntax);
        assert!(errors.to_string().contains("unknown field `route`"));
    }

    #[test]
    fn superseded_formats_and_invalid_http_authority_are_refused() {
        let legacy = COMPLETE.replace("service-definition/3", "service-definition/2");
        let errors = ServiceDefinition::from_yaml(&legacy).expect_err("v2 is not compatible");
        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == DefinitionCode::UnsupportedFormat)
        );

        let invalid_scope = COMPLETE.replacen("scope: todo.manage", "scope: Todo Manage", 1);
        let errors = ServiceDefinition::from_yaml(&invalid_scope).expect_err("scope is closed");
        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.path == "intents[0].scope")
        );

        let invalid_audience = COMPLETE.replace(
            "delivery: { kind: composed_connector }",
            "delivery: { kind: identity_http, audience: 'urn: invalid' }",
        );
        let errors =
            ServiceDefinition::from_yaml(&invalid_audience).expect_err("audience is closed");
        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.path == "delivery.audience")
        );
    }

    #[test]
    fn authority_coordinates_are_never_operation_fields() {
        for (from, to) in [
            ("field: item_id", "field: realm_id"),
            ("field: idempotency_key", "field: tenant_id"),
            ("parameter: item_id", "parameter: user_id"),
            ("field: idempotency_key", "field: authority_id"),
            ("field: idempotency_key", "field: current_authority"),
            ("parameter: item_id", "parameter: principal_id"),
            ("parameter: item_id", "parameter: executor_id"),
        ] {
            let text = COMPLETE.replacen(from, to, 1);
            let errors =
                ServiceDefinition::from_yaml(&text).expect_err("authority input must fail");
            assert!(
                errors
                    .diagnostics()
                    .iter()
                    .any(|diagnostic| diagnostic.code == DefinitionCode::AuthorityCoordinate)
            );
        }
    }

    #[test]
    fn duplicates_and_dangling_obligations_are_refused_together() {
        let text = COMPLETE
            .replace(
                "obligations: [inherit_scope]\nqueries:",
                "obligations: [missing, missing]\nqueries:",
            )
            .replace(
                "media_types: [text/plain, text/markdown]",
                "media_types: [text/plain, text/plain]",
            );
        let errors = ServiceDefinition::from_yaml(&text).expect_err("invalid references must fail");
        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|item| item.code == DefinitionCode::Duplicate)
        );
        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|item| item.code == DefinitionCode::InvalidReference)
        );
    }

    #[test]
    fn authority_named_response_fields_are_not_mistaken_for_caller_coordinates() {
        let text = COMPLETE.replace(
            "{ kind: view_field, field: owner }",
            "{ kind: view_field, field: authority_id }",
        );
        ServiceDefinition::from_yaml(&text)
            .expect("a projection field is service output, not caller-controlled authority input");
    }
}
