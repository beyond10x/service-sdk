//! Cross-artifact conformance for one ESS-built standalone service.
//!
//! The runtime IR is authoritative. [validate] independently derives the expected client and
//! inert Connector surfaces, then compares stable identities, digests, operation semantics,
//! inputs, results, kinds, and effects. Authentication context remains outside every caller input;
//! realm policy survives only as client-generation metadata.

use std::collections::{BTreeMap, btree_map::Entry};
use std::fmt;

use serde::Serialize;
use service_builder::client::{
    CLIENT_PLAN_FORMAT, ClientAuthentication, ClientOperation, ClientPlan,
};
use service_connectors::{
    ConnectorServiceFactoryDescriptor, OperationContribution, OperationEffect, OperationKind,
};
use service_runtime_ir::ServiceRuntimeIr;

/// Canonical serialized report format.
pub const CONFORMANCE_REPORT_FORMAT: &str = "service-conformance-report/1";

/// A stable category of cross-artifact contract violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationCode {
    /// The runtime IR could not derive its expected public artifact surface.
    DerivationFailure,
    /// A client plan uses an unsupported format.
    ClientFormat,
    /// Two artifacts disagree on the stable service identity.
    ServiceIdentity,
    /// Two artifacts disagree on the compiler-minted ESS source digest.
    SourceDigest,
    /// The client failed to retain the runtime realm policy as metadata.
    RealmPolicy,
    /// The client authentication mode differs from the generated contract.
    AuthenticationMode,
    /// Operations are not in the deterministic generated order.
    OperationOrder,
    /// An expected operation is absent.
    MissingOperation,
    /// An operation exists outside the runtime contract.
    UnexpectedOperation,
    /// An operation identity occurs more than once.
    DuplicateOperation,
    /// An operation names the wrong ESS command or view.
    SemanticReference,
    /// An operation is classified as the wrong intent/query role.
    OperationKind,
    /// Caller-input names, types, optionality, sources, or order differ.
    InputInventory,
    /// A client result differs from the runtime-derived result contract.
    ResultContract,
    /// An intent/query exposes a trusted authentication coordinate as caller input.
    AuthenticationCoordinate,
    /// A Connector operation exposes the wrong read/write effect.
    ConnectorEffect,
    /// A Connector descriptor digest is not reproducible from its contribution.
    ConnectorDescriptorDigest,
    /// A Connector contribution contains deployment, endpoint, credential, grant, or route data.
    ConnectorDeploymentBinding,
}

/// One deterministic, machine-readable conformance violation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Violation {
    /// Stable violation category.
    pub code: ViolationCode,
    /// Artifact-local path to the divergent value.
    pub path: String,
    /// Human-readable repair guidance.
    pub message: String,
}

/// Complete result of one cross-artifact validation pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConformanceReport {
    format: String,
    violations: Vec<Violation>,
}

impl ConformanceReport {
    fn new() -> Self {
        Self {
            format: CONFORMANCE_REPORT_FORMAT.to_owned(),
            violations: Vec::new(),
        }
    }

    fn push(&mut self, code: ViolationCode, path: impl Into<String>, message: impl Into<String>) {
        self.violations.push(Violation {
            code,
            path: path.into(),
            message: message.into(),
        });
    }

    /// Report-format discriminator.
    pub fn format(&self) -> &str {
        &self.format
    }

    /// Violations in deterministic validation order.
    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }

    /// Whether every supplied artifact is the exact runtime-derived contract.
    pub fn is_conformant(&self) -> bool {
        self.violations.is_empty()
    }

    /// Converts a report into the conventional success/error shape.
    pub fn into_result(self) -> Result<(), Self> {
        if self.is_conformant() {
            Ok(())
        } else {
            Err(self)
        }
    }

    /// Canonical pretty JSON with a trailing newline.
    pub fn to_canonical_json(&self) -> String {
        let mut output = serde_json::to_string_pretty(self)
            .unwrap_or_else(|error| panic!("a conformance report serializes: {error}"));
        output.push('\n');
        output
    }
}

impl fmt::Display for ConformanceReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_conformant() {
            return formatter.write_str("service artifacts conform");
        }
        formatter.write_str("service artifact conformance was refused")?;
        for violation in &self.violations {
            write!(
                formatter,
                "\n- {:?} at {}: {}",
                violation.code, violation.path, violation.message
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for ConformanceReport {}

/// Validates runtime IR, client plan, and Connector descriptor as one exact contract.
pub fn validate(
    runtime: &ServiceRuntimeIr,
    client: &ClientPlan,
    connector: &ConnectorServiceFactoryDescriptor,
) -> ConformanceReport {
    let mut report = ConformanceReport::new();
    compare_metadata(runtime, client, connector, &mut report);

    let expected_client = match ClientPlan::from_runtime(runtime) {
        Ok(expected) => Some(expected),
        Err(error) => {
            report.push(
                ViolationCode::DerivationFailure,
                "runtime",
                format!("runtime IR cannot derive a safe client plan: {error}"),
            );
            None
        }
    };

    if let Some(expected) = &expected_client {
        compare_client_operations(expected, client, &mut report);
        match expected.connector_descriptor() {
            Ok(expected_connector) => {
                compare_connector_operations(&expected_connector, connector, &mut report);
            }
            Err(error) => report.push(
                ViolationCode::DerivationFailure,
                "runtime",
                format!(
                    "runtime-derived client cannot mint an inert Connector descriptor: {error}"
                ),
            ),
        }
    }

    scan_authentication_inputs(client, connector, &mut report);
    validate_descriptor_digest(connector, &mut report);
    validate_inert_connector(connector, &mut report);
    report
}

/// Requires complete cross-artifact conformance.
pub fn check(
    runtime: &ServiceRuntimeIr,
    client: &ClientPlan,
    connector: &ConnectorServiceFactoryDescriptor,
) -> Result<(), ConformanceReport> {
    validate(runtime, client, connector).into_result()
}

fn compare_metadata(
    runtime: &ServiceRuntimeIr,
    client: &ClientPlan,
    connector: &ConnectorServiceFactoryDescriptor,
    report: &mut ConformanceReport,
) {
    if client.format != CLIENT_PLAN_FORMAT {
        report.push(
            ViolationCode::ClientFormat,
            "client.format",
            format!("expected {CLIENT_PLAN_FORMAT:?}, found {:?}", client.format),
        );
    }
    let service = runtime.definition().service.to_string();
    compare_string(
        &service,
        &client.service,
        ViolationCode::ServiceIdentity,
        "client.service",
        report,
    );
    compare_string(
        &service,
        &connector.contribution().service,
        ViolationCode::ServiceIdentity,
        "connector.service",
        report,
    );
    compare_string(
        runtime.ess_source_digest(),
        &client.ess_source_digest,
        ViolationCode::SourceDigest,
        "client.ess_source_digest",
        report,
    );
    compare_string(
        runtime.ess_source_digest(),
        &connector.contribution().ess_source_digest,
        ViolationCode::SourceDigest,
        "connector.ess_source_digest",
        report,
    );
    if client.realm_policy != runtime.definition().realm {
        report.push(
            ViolationCode::RealmPolicy,
            "client.realm_policy",
            format!(
                "expected {:?} from runtime admission metadata, found {:?}",
                runtime.definition().realm,
                client.realm_policy
            ),
        );
    }
    if client.authentication != ClientAuthentication::Session {
        report.push(
            ViolationCode::AuthenticationMode,
            "client.authentication",
            "generated clients must receive an authenticated session",
        );
    }
}

fn compare_string(
    expected: &str,
    actual: &str,
    code: ViolationCode,
    path: &str,
    report: &mut ConformanceReport,
) {
    if expected != actual {
        report.push(
            code,
            path,
            format!("expected {expected:?}, found {actual:?}"),
        );
    }
}

fn compare_client_operations(
    expected: &ClientPlan,
    actual: &ClientPlan,
    report: &mut ConformanceReport,
) {
    compare_operation_order(
        "client.operations",
        expected
            .operations
            .iter()
            .map(|operation| operation.operation.as_str()),
        actual
            .operations
            .iter()
            .map(|operation| operation.operation.as_str()),
        report,
    );
    let expected = index_client_operations(&expected.operations, "expected", report);
    let actual = index_client_operations(&actual.operations, "client", report);

    for (name, expected) in &expected {
        let path = format!("client.operations[{name}]");
        let Some(actual) = actual.get(name) else {
            report.push(
                ViolationCode::MissingOperation,
                path,
                "runtime-derived client operation is absent",
            );
            continue;
        };
        compare_string(
            &expected.semantic_ref,
            &actual.semantic_ref,
            ViolationCode::SemanticReference,
            &format!("{path}.semantic_ref"),
            report,
        );
        if expected.kind != actual.kind {
            report.push(
                ViolationCode::OperationKind,
                format!("{path}.kind"),
                format!("expected {:?}, found {:?}", expected.kind, actual.kind),
            );
        }
        if expected.inputs != actual.inputs {
            report.push(
                ViolationCode::InputInventory,
                format!("{path}.inputs"),
                "names, types, optionality, sources, or deterministic order differ",
            );
        }
        if expected.result != actual.result {
            report.push(
                ViolationCode::ResultContract,
                format!("{path}.result"),
                "client result does not equal the runtime-derived result contract",
            );
        }
    }
    for name in actual.keys().filter(|name| !expected.contains_key(*name)) {
        report.push(
            ViolationCode::UnexpectedOperation,
            format!("client.operations[{name}]"),
            "operation is not declared by runtime IR",
        );
    }
}

fn index_client_operations<'a>(
    operations: &'a [ClientOperation],
    artifact: &str,
    report: &mut ConformanceReport,
) -> BTreeMap<&'a str, &'a ClientOperation> {
    let mut indexed = BTreeMap::new();
    for operation in operations {
        match indexed.entry(operation.operation.as_str()) {
            Entry::Vacant(entry) => {
                entry.insert(operation);
            }
            Entry::Occupied(_) => report.push(
                ViolationCode::DuplicateOperation,
                format!("{artifact}.operations[{}]", operation.operation),
                "operation identity occurs more than once",
            ),
        }
    }
    indexed
}

fn compare_connector_operations(
    expected: &ConnectorServiceFactoryDescriptor,
    actual: &ConnectorServiceFactoryDescriptor,
    report: &mut ConformanceReport,
) {
    let expected = &expected.contribution().operations;
    let actual = &actual.contribution().operations;
    compare_operation_order(
        "connector.operations",
        expected
            .iter()
            .map(|operation| operation.operation.as_str()),
        actual.iter().map(|operation| operation.operation.as_str()),
        report,
    );
    let expected = index_connector_operations(expected, "expected_connector", report);
    let actual = index_connector_operations(actual, "connector", report);

    for (name, expected) in &expected {
        let path = format!("connector.operations[{name}]");
        let Some(actual) = actual.get(name) else {
            report.push(
                ViolationCode::MissingOperation,
                path,
                "runtime-derived Connector operation is absent",
            );
            continue;
        };
        compare_string(
            &expected.semantic_ref,
            &actual.semantic_ref,
            ViolationCode::SemanticReference,
            &format!("{path}.semantic_ref"),
            report,
        );
        if expected.kind != actual.kind {
            report.push(
                ViolationCode::OperationKind,
                format!("{path}.kind"),
                format!("expected {:?}, found {:?}", expected.kind, actual.kind),
            );
        }
        if expected.effect != actual.effect {
            report.push(
                ViolationCode::ConnectorEffect,
                format!("{path}.effect"),
                format!(
                    "expected {:?} for {:?}, found {:?}",
                    expected.effect, expected.kind, actual.effect
                ),
            );
        }
        if expected.inputs != actual.inputs {
            report.push(
                ViolationCode::InputInventory,
                format!("{path}.inputs"),
                "names, types, optionality, or deterministic order differ",
            );
        }
    }
    for name in actual.keys().filter(|name| !expected.contains_key(*name)) {
        report.push(
            ViolationCode::UnexpectedOperation,
            format!("connector.operations[{name}]"),
            "operation is not declared by runtime IR",
        );
    }

    for operation in actual.values() {
        let correct = matches!(
            (operation.kind, operation.effect),
            (OperationKind::Intent, OperationEffect::Write)
                | (OperationKind::Query, OperationEffect::Read)
        );
        if !correct {
            report.push(
                ViolationCode::ConnectorEffect,
                format!("connector.operations[{}].effect", operation.operation),
                format!(
                    "{:?} operations cannot expose {:?} effect",
                    operation.kind, operation.effect
                ),
            );
        }
    }
}

fn index_connector_operations<'a>(
    operations: &'a [OperationContribution],
    artifact: &str,
    report: &mut ConformanceReport,
) -> BTreeMap<&'a str, &'a OperationContribution> {
    let mut indexed = BTreeMap::new();
    for operation in operations {
        match indexed.entry(operation.operation.as_str()) {
            Entry::Vacant(entry) => {
                entry.insert(operation);
            }
            Entry::Occupied(_) => report.push(
                ViolationCode::DuplicateOperation,
                format!("{artifact}.operations[{}]", operation.operation),
                "operation identity occurs more than once",
            ),
        }
    }
    indexed
}

fn compare_operation_order<'a>(
    path: &str,
    expected: impl Iterator<Item = &'a str>,
    actual: impl Iterator<Item = &'a str>,
    report: &mut ConformanceReport,
) {
    let expected = expected.collect::<Vec<_>>();
    let actual = actual.collect::<Vec<_>>();
    if expected != actual {
        report.push(
            ViolationCode::OperationOrder,
            path,
            format!("expected {expected:?}, found {actual:?}"),
        );
    }
}

fn scan_authentication_inputs(
    client: &ClientPlan,
    connector: &ConnectorServiceFactoryDescriptor,
    report: &mut ConformanceReport,
) {
    for operation in &client.operations {
        for input in &operation.inputs {
            if is_authentication_coordinate(&input.name) {
                report.push(
                    ViolationCode::AuthenticationCoordinate,
                    format!(
                        "client.operations[{}].inputs[{}]",
                        operation.operation, input.name
                    ),
                    "trusted login context must not be caller controlled",
                );
            }
        }
    }
    for operation in &connector.contribution().operations {
        for input in &operation.inputs {
            if is_authentication_coordinate(&input.name) {
                report.push(
                    ViolationCode::AuthenticationCoordinate,
                    format!(
                        "connector.operations[{}].inputs[{}]",
                        operation.operation, input.name
                    ),
                    "trusted login context must not be caller controlled",
                );
            }
        }
    }
}

fn is_authentication_coordinate(input: &str) -> bool {
    matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "tenant"
            | "tenant_id"
            | "tenantid"
            | "realm"
            | "realm_id"
            | "realmid"
            | "user"
            | "user_id"
            | "userid"
            | "current_user"
            | "principal"
            | "principal_id"
            | "principalid"
            | "authority"
            | "authority_id"
            | "authorityid"
            | "current_authority"
            | "executor"
            | "executor_id"
            | "executorid"
    )
}

fn validate_descriptor_digest(
    connector: &ConnectorServiceFactoryDescriptor,
    report: &mut ConformanceReport,
) {
    match ConnectorServiceFactoryDescriptor::new(connector.contribution().clone()) {
        Ok(reminted) if reminted.digest() != connector.digest() => report.push(
            ViolationCode::ConnectorDescriptorDigest,
            "connector.digest",
            "descriptor digest is not reproducible from its contribution",
        ),
        Ok(_) => {}
        Err(error) => report.push(
            ViolationCode::DerivationFailure,
            "connector",
            format!("validated descriptor contribution no longer validates: {error}"),
        ),
    }
}

fn validate_inert_connector(
    connector: &ConnectorServiceFactoryDescriptor,
    report: &mut ConformanceReport,
) {
    let value = serde_json::to_value(connector.contribution())
        .unwrap_or_else(|error| panic!("a Connector contribution serializes: {error}"));
    let mut bindings = Vec::new();
    find_deployment_keys(&value, "connector", &mut bindings);
    bindings.sort();
    for (path, key) in bindings {
        report.push(
            ViolationCode::ConnectorDeploymentBinding,
            path,
            format!("deployment key {key:?} belongs to product-supplied ServiceDeployment"),
        );
    }
}

fn find_deployment_keys(value: &serde_json::Value, path: &str, found: &mut Vec<(String, String)>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                let child = format!("{path}.{key}");
                if is_deployment_key(key) {
                    found.push((child.clone(), key.clone()));
                }
                find_deployment_keys(value, &child, found);
            }
        }
        serde_json::Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                find_deployment_keys(item, &format!("{path}[{index}]"), found);
            }
        }
        _ => {}
    }
}

fn is_deployment_key(key: &str) -> bool {
    matches!(
        key,
        "endpoint"
            | "base_url"
            | "url"
            | "credential"
            | "credentials"
            | "grant"
            | "grants"
            | "exposure"
            | "deployment"
            | "route"
            | "realm"
            | "realm_id"
    )
}
