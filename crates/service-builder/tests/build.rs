//! End-to-end proofs for deterministic service construction and CLI drift checking.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use service_builder::client::{ClientInputSource, ClientOperationKind};
use service_builder::ess::EssSources;
use service_builder::{
    CLIENT_PLAN_PATH, CONNECTOR_CONTRIBUTION_PATH, RUNTIME_IR_PATH, build_service,
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
format: service-definition/1
service: demo_todo
realm: optional
content:
  - name: item_content
    reference_type: demo.todo.ContentRef
    media_types: [text/plain, text/markdown]
    max_bytes: 65536
    custody: external_erasable
obligations:
  - name: bind_owner
    kind: derivation
    description: Bind owner from current authenticated authority.
projections:
  - name: item_by_id
    view: demo.todo.ItemById
    delivery: inline_transactional
    obligations: [bind_owner]
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
    obligations: [bind_owner]
queries:
  - name: get_item
    view: demo.todo.ItemById
    projection: item_by_id
    selectors:
      - { parameter: item_id, view_field: item_id }
    guards:
      - name: may_read
        refusal_code: forbidden
        reads:
          - { kind: view_field, field: owner }
          - { kind: context, value: current_authority }
    sort:
      - { view_field: item_id, direction: ascending }
    delivery: read_your_writes
    obligations: [bind_owner]
";

fn inputs() -> (EssSources, ServiceDefinition) {
    let sources = EssSources::new(BTreeMap::from([("system.yaml".to_owned(), ESS.to_owned())]))
        .expect("fixture sources are valid");
    let definition = ServiceDefinition::from_yaml(DEFINITION).expect("fixture definition is valid");
    (sources, definition)
}

#[test]
fn one_runtime_ir_derives_identical_client_and_connector_surfaces() {
    let (sources, definition) = inputs();
    let first = build_service(&sources, &definition).expect("service builds");
    let second = build_service(&sources, &definition).expect("same service builds again");

    assert_eq!(
        first.artifacts.iter().collect::<Vec<_>>(),
        second.artifacts.iter().collect::<Vec<_>>()
    );
    assert_expected_artifacts(&first);
    assert_client_and_connector_surfaces(&first);
}

fn assert_expected_artifacts(build: &service_builder::ServiceBuild) {
    let paths = build
        .artifacts
        .iter()
        .map(|(path, _)| path)
        .collect::<Vec<_>>();
    assert!(paths.contains(&RUNTIME_IR_PATH));
    assert!(paths.contains(&CLIENT_PLAN_PATH));
    assert!(paths.contains(&CONNECTOR_CONTRIBUTION_PATH));
    assert!(paths.contains(&"ess/synthesis/plan.json"));
    assert!(
        paths
            .iter()
            .any(|path| path.starts_with("ess/projections/"))
    );
    assert_eq!(build.client_plan.realm_policy, RealmPolicy::Optional);
    assert_eq!(
        build
            .runtime_ir
            .projections()
            .values()
            .next()
            .expect("projection resolves")
            .shape
            .as_deref(),
        Some("demo.todo.ItemRow")
    );
}

fn assert_client_and_connector_surfaces(build: &service_builder::ServiceBuild) {
    let intent = build
        .client_plan
        .operations
        .iter()
        .find(|operation| operation.operation == "add_item")
        .expect("intent is present");
    assert_eq!(intent.kind, ClientOperationKind::Intent);
    assert_eq!(intent.semantic_ref, "demo.todo.AddItem");
    assert_eq!(
        intent
            .inputs
            .iter()
            .map(|input| input.name.as_str())
            .collect::<Vec<_>>(),
        ["body", "expected_version", "idempotency_key", "item_id"]
    );
    assert!(intent.inputs.iter().any(|input| {
        input.name == "body"
            && matches!(
                &input.source,
                ClientInputSource::Content { policy } if policy == "item_content"
            )
    }));
    assert!(
        !intent
            .inputs
            .iter()
            .any(|input| input.name == "content_ref")
    );

    let query = build
        .client_plan
        .operations
        .iter()
        .find(|operation| operation.operation == "get_item")
        .expect("query is present");
    assert_eq!(query.kind, ClientOperationKind::Query);
    assert_eq!(query.semantic_ref, "demo.todo.ItemById");
    assert_eq!(query.inputs.len(), 1);
    assert_eq!(query.inputs[0].name, "item_id");

    let contribution = build.connector_descriptor.contribution();
    assert_eq!(
        contribution.operations.len(),
        build.client_plan.operations.len()
    );
    for (client, connector) in build
        .client_plan
        .operations
        .iter()
        .zip(&contribution.operations)
    {
        assert_eq!(client.operation, connector.operation);
        assert_eq!(client.semantic_ref, connector.semantic_ref);
        assert_eq!(
            client
                .inputs
                .iter()
                .map(|input| (&input.name, &input.type_ref, input.optional))
                .collect::<Vec<_>>(),
            connector
                .inputs
                .iter()
                .map(|input| (&input.name, &input.type_ref, input.optional))
                .collect::<Vec<_>>()
        );
    }

    let reserved = [
        "realm",
        "realm_id",
        "tenant_id",
        "user_id",
        "principal_id",
        "authority",
        "executor",
    ];
    for operation in &build.client_plan.operations {
        for input in &operation.inputs {
            assert!(
                !reserved.contains(&input.name.as_str()),
                "{} exposes reserved input {}",
                operation.operation,
                input.name
            );
        }
    }
    let connector_json = build.connector_descriptor.to_canonical_json();
    for forbidden in reserved {
        assert!(
            !connector_json.contains(&format!("\"name\": \"{forbidden}\"")),
            "generated Connector surface contains reserved input {forbidden}: {connector_json}"
        );
    }
}

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "service-builder-integration-{}-{sequence}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale test directory");
        }
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if self.0.exists() {
            fs::remove_dir_all(&self.0).expect("remove test directory");
        }
    }
}

#[test]
fn cli_generate_then_check_detects_byte_drift() {
    let temporary = TestDirectory::new();
    let ess = temporary.path().join("ess");
    fs::create_dir_all(&ess).expect("create ESS directory");
    fs::write(ess.join("system.yaml"), ESS).expect("write ESS fixture");
    let definition = temporary.path().join("service.yaml");
    fs::write(&definition, DEFINITION).expect("write definition fixture");
    let output = temporary.path().join("generated");

    let run = |subcommand: &str| {
        Command::new(env!("CARGO_BIN_EXE_service-builder"))
            .arg(subcommand)
            .arg("--ess")
            .arg(&ess)
            .arg("--definition")
            .arg(&definition)
            .arg("--output")
            .arg(&output)
            .output()
            .expect("run service-builder")
    };

    let generated = run("generate");
    assert!(
        generated.status.success(),
        "generation failed: {}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let current = run("check");
    assert!(
        current.status.success(),
        "clean check failed: {}",
        String::from_utf8_lossy(&current.stderr)
    );

    fs::write(output.join(CLIENT_PLAN_PATH), "{}\n").expect("introduce client-plan drift");
    let drifted = run("check");
    assert!(!drifted.status.success());
    let error = String::from_utf8_lossy(&drifted.stderr);
    assert!(error.contains("changed client/plan.json"), "{error}");
}
