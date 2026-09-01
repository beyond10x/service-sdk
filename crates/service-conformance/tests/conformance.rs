//! End-to-end and mutation proofs for the cross-artifact contract.

use std::collections::BTreeMap;

use service_builder::client::{ClientInput, ClientInputSource, ClientOperationKind, ClientResult};
use service_builder::ess::EssSources;
use service_builder::{ServiceBuild, build_service};
use service_conformance::{
    CONFORMANCE_REPORT_FORMAT, ConformanceReport, ViolationCode, check, validate,
};
use service_connectors::{
    ConnectorServiceFactoryDescriptor, ContributionError, OperationEffect, OperationKind,
};
use service_definition::{RealmPolicy, ServiceDefinition};

const ESS: &str = r"
format: ess/1
system: demo
version: v1
domains: [demo.todo]
domain: demo.todo
types:
  - name: demo.todo.ItemId
    kind: newtype
    of: Uuid
  - name: demo.todo.ContentRef
    kind: newtype
    of: String
  - name: demo.todo.OwnerRef
    kind: newtype
    of: String
  - name: demo.todo.ItemRow
    kind: struct
    fields:
      - { name: item_id, type: demo.todo.ItemId }
      - { name: content_ref, type: demo.todo.ContentRef }
      - { name: owner, type: demo.todo.OwnerRef }
entities:
  - name: demo.todo.Item
    identity: { name: item_id, type: demo.todo.ItemId }
    fields:
      - { name: content_ref, type: demo.todo.ContentRef }
      - { name: owner, type: demo.todo.OwnerRef }
    lifecycle:
      initial: Active
      states: [Active]
      terminal: [Active]
commands:
  - name: demo.todo.AddItem
    input:
      - { name: item_id, type: demo.todo.ItemId }
      - { name: content_ref, type: demo.todo.ContentRef }
    outcomes:
      - name: added
        creates: demo.todo.Item
        instance: item_id
        emits: [demo.todo.ItemAdded]
        payload:
          demo.todo.ItemAdded:
            item_id: input.item_id
            content_ref: input.content_ref
events:
  - name: demo.todo.ItemAdded
    fields:
      - { name: item_id, type: demo.todo.ItemId }
      - { name: content_ref, type: demo.todo.ContentRef }
      - { name: owner, type: demo.todo.OwnerRef }
views:
  - name: demo.todo.ItemById
    source: demo.todo.Item
    shape: demo.todo.ItemRow
    consistency: read_your_writes
";

const DEFINITION: &str = r"
format: service-definition/2
service: demo_todo
realm: optional
content:
  - name: item_content
    reference_type: demo.todo.ContentRef
    media_types: [text/plain]
    max_bytes: 65536
    custody: external_erasable
obligations:
  - name: bind_owner
    provider: sdk.derive.inherit-parent-authority/v1
    bindings:
      parent: demo.todo.Item
      child: demo.todo.Item
      parent_owner: demo.todo.Item.owner
      parent_scopes: demo.todo.Item.owner
      child_owner: demo.todo.Item.owner
      child_scopes: demo.todo.Item.owner
    description: Bind owner from current authenticated authority.
  - name: aggregate
    provider: sdk.aggregate.event-sourced/v1
    bindings: { aggregate: demo.todo.Item, identity: item_id }
    description: Execute one guarded event-sourced aggregate.
  - name: content_lifecycle
    provider: sdk.content.external-erasable/v1
    bindings: { content: item_content }
    description: Own the external content lifecycle around append.
  - name: visibility
    provider: sdk.projection.auth-partitioned-visibility/v1
    bindings: { owner: owner, scopes: owner }
    description: Partition projection access by authenticated authority.
projections:
  - name: item_by_id
    view: demo.todo.ItemById
    delivery: inline_transactional
    obligations: [bind_owner, visibility]
intents:
  - name: add_item
    command: demo.todo.AddItem
    stream_id: { kind: command_field, field: item_id }
    expected_version: { kind: operation_field, field: expected_version }
    idempotency: { kind: operation_field, field: idempotency_key }
    content:
      - content: item_content
        input_field: body
        command_reference_field: content_ref
    event_bindings:
      - event: demo.todo.ItemAdded
        field: owner
        source: { kind: context, value: current_authority }
    projections: [item_by_id]
    obligations: [aggregate, bind_owner, content_lifecycle]
queries:
  - name: get_item
    view: demo.todo.ItemById
    projection: item_by_id
    selectors:
      - { parameter: item_id, view_field: item_id }
    sort:
      - { view_field: item_id, direction: ascending }
    delivery: read_your_writes
    obligations: [visibility]
";

fn fixture() -> ServiceBuild {
    let sources = EssSources::new(BTreeMap::from([("system.yaml".to_owned(), ESS.to_owned())]))
        .expect("fixture ESS sources validate");
    let definition =
        ServiceDefinition::from_yaml(DEFINITION).expect("fixture service definition validates");
    build_service(&sources, &definition).expect("fixture builds through official ESS")
}

fn codes(report: &ConformanceReport) -> Vec<ViolationCode> {
    report
        .violations()
        .iter()
        .map(|violation| violation.code)
        .collect()
}

#[test]
fn builder_output_is_conformant_deterministic_and_inert() {
    let build = fixture();
    let first = validate(
        &build.runtime_ir,
        &build.client_plan,
        &build.connector_descriptor,
    );
    let second = validate(
        &build.runtime_ir,
        &build.client_plan,
        &build.connector_descriptor,
    );

    assert!(first.is_conformant(), "{first}");
    assert_eq!(first, second);
    assert_eq!(first.format(), CONFORMANCE_REPORT_FORMAT);
    assert_eq!(first.to_canonical_json(), second.to_canonical_json());
    check(
        &build.runtime_ir,
        &build.client_plan,
        &build.connector_descriptor,
    )
    .expect("the complete generated contract conforms");

    assert_eq!(build.client_plan.realm_policy, RealmPolicy::Optional);
    for operation in &build.client_plan.operations {
        assert!(
            operation
                .inputs
                .iter()
                .all(|input| input.name != "realm" && input.name != "realm_id")
        );
    }
    let connector = build.connector_descriptor.to_canonical_json();
    for deployment_key in [
        "endpoint",
        "credential",
        "grants",
        "exposure",
        "deployment",
        "route",
    ] {
        assert!(!connector.contains(&format!("\"{deployment_key}\"")));
    }
}

#[test]
fn metadata_mutations_have_precise_typed_violations() {
    let build = fixture();
    let mut client = build.client_plan.clone();
    client.service = "other".to_owned();
    client.ess_source_digest = "b".repeat(64);
    client.realm_policy = RealmPolicy::Required;

    let report = validate(&build.runtime_ir, &client, &build.connector_descriptor);
    assert_eq!(
        codes(&report),
        [
            ViolationCode::ServiceIdentity,
            ViolationCode::SourceDigest,
            ViolationCode::RealmPolicy,
        ]
    );
    assert_eq!(
        report
            .violations()
            .iter()
            .map(|violation| violation.path.as_str())
            .collect::<Vec<_>>(),
        [
            "client.service",
            "client.ess_source_digest",
            "client.realm_policy",
        ]
    );

    let mut contribution = build.connector_descriptor.contribution().clone();
    contribution.service = "other".to_owned();
    contribution.ess_source_digest = "c".repeat(64);
    let connector =
        ConnectorServiceFactoryDescriptor::new(contribution).expect("mutation remains structural");
    let report = validate(&build.runtime_ir, &build.client_plan, &connector);
    assert_eq!(
        codes(&report),
        [ViolationCode::ServiceIdentity, ViolationCode::SourceDigest,]
    );
}

#[test]
fn client_operation_mutations_cover_order_semantics_kind_inputs_and_result() {
    let build = fixture();
    let mut client = build.client_plan.clone();
    client.operations.reverse();
    let intent = client
        .operations
        .iter_mut()
        .find(|operation| operation.operation == "add_item")
        .expect("intent exists");
    intent.semantic_ref = "demo.todo.Other".to_owned();
    intent.kind = ClientOperationKind::Query;
    intent.inputs[0].type_ref = "String".to_owned();
    intent.result = ClientResult::Query {
        fields: BTreeMap::new(),
    };

    let report = validate(&build.runtime_ir, &client, &build.connector_descriptor);
    assert_eq!(
        codes(&report),
        [
            ViolationCode::OperationOrder,
            ViolationCode::SemanticReference,
            ViolationCode::OperationKind,
            ViolationCode::InputInventory,
            ViolationCode::ResultContract,
        ]
    );
    assert_eq!(
        report,
        validate(&build.runtime_ir, &client, &build.connector_descriptor)
    );
}

#[test]
fn connector_mutations_cover_order_semantics_kind_effect_and_inputs() {
    let build = fixture();
    let mut contribution = build.connector_descriptor.contribution().clone();
    contribution.operations.reverse();
    let intent = contribution
        .operations
        .iter_mut()
        .find(|operation| operation.operation == "add_item")
        .expect("intent exists");
    intent.semantic_ref = "demo.todo.Other".to_owned();
    intent.kind = OperationKind::Query;
    intent.effect = OperationEffect::Read;
    intent.inputs[0].type_ref = "String".to_owned();
    let connector =
        ConnectorServiceFactoryDescriptor::new(contribution).expect("mutation remains structural");

    let report = validate(&build.runtime_ir, &build.client_plan, &connector);
    assert_eq!(
        codes(&report),
        [
            ViolationCode::OperationOrder,
            ViolationCode::SemanticReference,
            ViolationCode::OperationKind,
            ViolationCode::ConnectorEffect,
            ViolationCode::InputInventory,
        ]
    );
}

#[test]
fn caller_authentication_coordinates_are_refused_and_never_connector_descriptors() {
    let build = fixture();
    let mut client = build.client_plan.clone();
    let operation = &mut client.operations[0];
    operation.inputs.push(ClientInput {
        name: "principal_id".to_owned(),
        type_ref: "String".to_owned(),
        optional: false,
        source: ClientInputSource::Command,
    });

    let report = validate(&build.runtime_ir, &client, &build.connector_descriptor);
    assert_eq!(
        codes(&report),
        [
            ViolationCode::InputInventory,
            ViolationCode::AuthenticationCoordinate,
        ]
    );

    let mut contribution = build.connector_descriptor.contribution().clone();
    contribution.operations[0].inputs[0].name = "realm_id".to_owned();
    assert!(matches!(
        ConnectorServiceFactoryDescriptor::new(contribution),
        Err(ContributionError::AuthenticationCoordinate { input, .. }) if input == "realm_id"
    ));
}

#[test]
fn missing_and_unexpected_operations_are_not_hidden_by_set_comparison() {
    let build = fixture();
    let mut client = build.client_plan.clone();
    let removed = client.operations.pop().expect("two fixture operations");
    let mut unexpected = client.operations[0].clone();
    unexpected.operation = "unexpected".to_owned();
    client.operations.push(unexpected);

    let report = validate(&build.runtime_ir, &client, &build.connector_descriptor);
    assert_eq!(
        codes(&report),
        [
            ViolationCode::OperationOrder,
            ViolationCode::MissingOperation,
            ViolationCode::UnexpectedOperation,
        ]
    );
    assert!(
        report
            .violations()
            .iter()
            .any(|violation| violation.path.contains(&removed.operation))
    );
}
