//! End-to-end proofs for deterministic service construction and CLI drift checking.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use service_builder::client::{ClientInputSource, ClientOperationKind};
use service_builder::ess::EssSources;
use service_builder::package::SdkLock;
use service_builder::realization::RealizationArtifacts;
use service_builder::{
    CLIENT_PLAN_PATH, CONNECTOR_CONTRIBUTION_PATH, ESS_IR_PATH, HTTP_OPENAPI_PATH,
    REALIZATION_PLAN_PATH, RUNTIME_IR_PATH, SERVICE_CATALOG_PATH, build_service,
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
components:
  - component: demo-service
    summary: Owns demo todo items.
    owns:
      domains: [demo.todo]
    accepts:
      commands: [demo.todo.AddItem]
    publishes:
      events: [demo.todo.ItemAdded]
    reached_by: network
";

const DEFINITION: &str = r"
format: service-definition/3
service: demo_todo
delivery: { kind: identity_http, audience: urn:b10x:demo-todo }
realm: optional
content:
  - name: item_content
    reference_type: demo.todo.ContentRef
    media_types: [text/plain, text/markdown]
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
    scope: items.manage
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
    scope: items.read
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
    obligations: [visibility]
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

#[test]
fn realization_plan_carries_optional_projection_fields_from_ess() {
    let ess = ESS
        .replace(
            "      - { name: owner, type: demo.todo.OwnerRef }\nentities:",
            "      - { name: owner, type: demo.todo.OwnerRef }\n      - { name: active_revision_id, type: Optional<demo.todo.ItemId> }\nentities:",
        )
        .replace(
            "      - { name: owner, type: demo.todo.OwnerRef }\n    lifecycle:",
            "      - { name: owner, type: demo.todo.OwnerRef }\n      - { name: active_revision_id, type: Optional<demo.todo.ItemId> }\n    lifecycle:",
        );
    let sources = EssSources::new(BTreeMap::from([("system.yaml".to_owned(), ess)]))
        .expect("fixture sources are valid");
    let definition = ServiceDefinition::from_yaml(DEFINITION).expect("fixture definition is valid");

    let build = build_service(&sources, &definition).expect("service builds");
    let view = build
        .realization_plan
        .views
        .get("demo.todo.ItemById")
        .expect("the projection view is realized");

    assert_eq!(build.realization_plan.format, "service-realization-plan/3");
    assert!(view.fields.contains(&"active_revision_id".to_owned()));
    assert_eq!(
        view.field_types["active_revision_id"],
        "Optional<demo.todo.ItemId>"
    );
    assert_eq!(
        view.optional_fields,
        ["active_revision_id".to_owned()].into_iter().collect()
    );
    let mut masquerading_legacy_plan = serde_json::to_value(&build.realization_plan)
        .expect("the current realization plan serializes");
    masquerading_legacy_plan["format"] =
        serde_json::Value::String("service-realization-plan/2".to_owned());
    assert!(
        service_engine::ServicePlan::from_json(
            &serde_json::to_string(&masquerading_legacy_plan)
                .expect("the altered realization plan serializes"),
        )
        .is_err(),
        "new field metadata must not masquerade under the legacy format"
    );
}

#[test]
fn realization_plan_only_marks_outer_optional_projection_fields_as_absentable() {
    let ess = ESS
        .replace(
            "      - { name: owner, type: demo.todo.OwnerRef }\nentities:",
            "      - { name: owner, type: demo.todo.OwnerRef }\n      - { name: maybe_labels, type: Optional<List<String>> }\n      - { name: labels_with_gaps, type: List<Optional<String>> }\nentities:",
        )
        .replace(
            "      - { name: owner, type: demo.todo.OwnerRef }\n    lifecycle:",
            "      - { name: owner, type: demo.todo.OwnerRef }\n      - { name: maybe_labels, type: Optional<List<String>> }\n      - { name: labels_with_gaps, type: List<Optional<String>> }\n    lifecycle:",
        );
    let sources = EssSources::new(BTreeMap::from([("system.yaml".to_owned(), ess)]))
        .expect("fixture sources are valid");
    let definition = ServiceDefinition::from_yaml(DEFINITION).expect("fixture definition is valid");

    let build = build_service(&sources, &definition).expect("service builds");
    let view = build
        .realization_plan
        .views
        .get("demo.todo.ItemById")
        .expect("the projection view is realized");

    assert_eq!(
        view.optional_fields,
        ["maybe_labels".to_owned()].into_iter().collect()
    );

    let generated = RealizationArtifacts::generate(
        &build.realization_plan,
        &build.client_plan,
        &SdkLock {
            repository: "https://github.com/beyond10x/service-sdk.git".to_owned(),
            revision: "a".repeat(40),
        },
        [],
    );
    let source = &generated.files["rust/src/lib.rs"];
    assert!(
        source.contains("pub maybe_labels: Option<Vec<String>>"),
        "the generated HTTP client must decode an omitted outer optional collection"
    );
    assert!(
        source.contains("pub labels_with_gaps: Vec<Option<String>>"),
        "nested Optional values must remain optional inside generated collection types"
    );
}

fn assert_expected_artifacts(build: &service_builder::ServiceBuild) {
    let paths = build
        .artifacts
        .iter()
        .map(|(path, _)| path)
        .collect::<Vec<_>>();
    assert!(paths.contains(&RUNTIME_IR_PATH));
    assert!(paths.contains(&ESS_IR_PATH));
    assert!(paths.contains(&REALIZATION_PLAN_PATH));
    assert!(paths.contains(&CLIENT_PLAN_PATH));
    assert!(paths.contains(&CONNECTOR_CONTRIBUTION_PATH));
    assert!(paths.contains(&SERVICE_CATALOG_PATH));
    assert!(paths.contains(&HTTP_OPENAPI_PATH));
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
    assert_eq!(intent.scope, "items.manage");
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
    assert_eq!(query.scope, "items.read");
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
        assert_catalog_operation(build, client, connector);
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

fn assert_catalog_operation(
    build: &service_builder::ServiceBuild,
    client: &service_builder::client::ClientOperation,
    connector: &service_connectors::OperationContribution,
) {
    let catalog = build
        .service_catalog
        .operations
        .iter()
        .find(|catalog| catalog.name == client.operation)
        .expect("catalog operation is present");
    let (input_schema, output_schema) =
        service_connectors::operation_schemas(&build.realization_plan, connector);
    assert_eq!(catalog.input_schema, input_schema);
    assert_eq!(catalog.output_schema, output_schema);
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

#[test]
fn composed_connector_package_does_not_emit_a_standalone_http_host() {
    let temporary = TestDirectory::new();
    let ess = temporary.path().join("ess");
    fs::create_dir_all(&ess).expect("create ESS directory");
    fs::write(ess.join("system.yaml"), ESS).expect("write ESS fixture");
    fs::write(
        temporary.path().join("runtime.yaml"),
        DEFINITION
            .replace("service: demo_todo", "service: demo")
            .replace(
                "delivery: { kind: identity_http, audience: urn:b10x:demo-todo }",
                "delivery: { kind: composed_connector }",
            ),
    )
    .expect("write runtime fixture");
    fs::write(
        temporary.path().join("service.yaml"),
        r"format: service/1
service: demo
sdk:
  repository: https://github.com/beyond10x/service-sdk.git
  revision: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
semantic:
  root: ess
  sources: [system.yaml]
runtime: runtime.yaml
",
    )
    .expect("write package fixture");
    let output = temporary.path().join("generated");
    let generated = Command::new(env!("CARGO_BIN_EXE_service-builder"))
        .arg("generate")
        .arg("--package")
        .arg(temporary.path().join("service.yaml"))
        .arg("--output")
        .arg(&output)
        .output()
        .expect("run package generation");
    assert!(
        generated.status.success(),
        "package generation failed: {}",
        String::from_utf8_lossy(&generated.stderr)
    );

    let cargo = fs::read_to_string(output.join("rust/Cargo.toml")).unwrap();
    assert!(!cargo.contains("service-host"));
    assert!(!output.join("rust/src/main.rs").exists());
}

#[test]
#[allow(clippy::too_many_lines)]
fn unified_package_emits_compilable_service_and_connector_factory_sources() {
    let temporary = TestDirectory::new();
    let ess = temporary.path().join("ess");
    fs::create_dir_all(&ess).expect("create ESS directory");
    fs::write(ess.join("system.yaml"), ESS).expect("write ESS fixture");
    fs::write(
        temporary.path().join("runtime.yaml"),
        DEFINITION.replace("service: demo_todo", "service: demo"),
    )
    .expect("write runtime fixture");
    fs::write(
        temporary.path().join("scenario.yaml"),
        r"format: service-scenarios/1
service: demo
scenarios:
  - name: reads-an-item
    given:
      auth: { tenant: tenant-a, realm: null, authority: person-a, user: person-a }
    then:
      - query: get_item
        input: { item_id: item-a }
        count: 1
",
    )
    .expect("write scenario fixture");
    fs::write(
        temporary.path().join("service.yaml"),
        r"format: service/1
service: demo
sdk:
  repository: https://github.com/beyond10x/service-sdk.git
  revision: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
semantic:
  root: ess
  sources: [system.yaml]
runtime: runtime.yaml
scenarios: [scenario.yaml]
release:
  image_repository: ghcr.io/example/demo
  version: 0.1.0
  build_base:
    repository: docker.io/library/rust
    digest: sha256:0000000000000000000000000000000000000000000000000000000000000000
  runtime_base:
    repository: gcr.io/distroless/cc-debian12
    digest: sha256:1111111111111111111111111111111111111111111111111111111111111111
",
    )
    .expect("write package fixture");
    let output = temporary.path().join("generated");
    let generated = Command::new(env!("CARGO_BIN_EXE_service-builder"))
        .arg("generate")
        .arg("--package")
        .arg(temporary.path().join("service.yaml"))
        .arg("--output")
        .arg(&output)
        .output()
        .expect("run package generation");
    assert!(
        generated.status.success(),
        "package generation failed: {}",
        String::from_utf8_lossy(&generated.stderr)
    );

    let cargo = fs::read_to_string(output.join("rust/Cargo.toml")).unwrap();
    let rust = fs::read_to_string(output.join("rust/src/lib.rs")).unwrap();
    assert!(cargo.contains("rev = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\""));
    assert!(cargo.contains("service-connectors"));
    assert!(cargo.contains("service-catalog"));
    assert!(cargo.contains("service-http"));
    assert!(cargo.contains("service-host = { git ="));
    assert!(cargo.contains("optional = true"));
    assert!(cargo.contains("default = [\"standalone-host\"]"));
    assert!(cargo.contains("required-features = [\"standalone-host\"]"));
    assert!(cargo.contains("service-conformance"));
    assert!(rust.contains("service_connectors::GeneratedConnectorFactory"));
    assert!(rust.contains("service_connectors::DurableEventStore"));
    assert!(!rust.contains("dyn connectors_service::ConnectorBackend"));
    assert!(rust.contains("service_engine::ServiceEngine"));
    assert!(rust.contains("pub fn service_catalog()"));
    assert!(rust.contains("pub struct DemoClient"));
    assert!(rust.contains("pub async fn add_item"));
    assert!(rust.contains("pub async fn get_item"));
    assert!(rust.contains("pub async fn http_router"));
    let main = fs::read_to_string(output.join("rust/src/main.rs")).unwrap();
    assert!(main.contains("service_host::run_sqlite"));
    assert!(main.contains("\"DEMO\""));
    assert!(main.contains("/var/lib/demo/demo.sqlite3"));
    assert!(!rust.contains("Unimplemented"));
    assert!(!rust.contains("realm_id:"));
    assert!(output.join(ESS_IR_PATH).is_file());
    assert!(output.join(REALIZATION_PLAN_PATH).is_file());
    assert!(output.join(SERVICE_CATALOG_PATH).is_file());
    let http_openapi = fs::read_to_string(output.join(HTTP_OPENAPI_PATH)).unwrap();
    assert!(http_openapi.contains("urn:b10x:demo-todo"));
    assert!(http_openapi.contains("items.manage"));
    assert!(http_openapi.contains("items.read"));
    assert!(output.join("docs/src/App.vue").is_file());
    let docs = fs::read_to_string(output.join("docs/src/App.vue")).unwrap();
    assert!(docs.contains("createDemoServiceBinding"));
    assert!(!docs.contains("realm_id"));
    let docs_package = fs::read_to_string(output.join("docs/package.json")).unwrap();
    assert!(docs_package.contains("\"typescript\": \"^5.9.0\""));
    assert_eq!(
        fs::read_to_string(output.join("docs/pnpm-workspace.yaml")).unwrap(),
        "allowBuilds:\n  esbuild: true\n"
    );
    assert!(output.join("docs/tsconfig.json").is_file());
    assert!(output.join("conformance/scenario.yaml").is_file());
    assert!(output.join("deployment/component.ir.json").is_file());
    assert!(output.join("deployment/build.ir.json").is_file());
    assert!(output.join("deployment/runtime.ir.json").is_file());
    assert!(output.join("deployment/buildkit/Dockerfile.ess").is_file());
    let dockerfile = fs::read_to_string(output.join("deployment/buildkit/Dockerfile.ess")).unwrap();
    assert!(dockerfile.contains("FROM gcr.io/distroless/cc-debian12@sha256:"));
    assert!(dockerfile.contains("/usr/local/bin/demo-generated-service"));
    assert!(dockerfile.contains("WORKDIR /src\nRUN --network=none [\"tar\""));
    assert!(output.join("deployment/chart/Chart.yaml").is_file());
    let scenario_test =
        fs::read_to_string(output.join("rust/tests/generated_scenarios.rs")).unwrap();
    assert!(scenario_test.contains("run_connector_scenarios"));
    assert!(scenario_test.contains("conformance/scenario.yaml"));
    let formatted = Command::new("rustfmt")
        .arg("--edition")
        .arg("2024")
        .arg("--check")
        .arg(output.join("rust/src/lib.rs"))
        .arg(output.join("rust/src/main.rs"))
        .arg(output.join("rust/tests/generated_scenarios.rs"))
        .output()
        .expect("run rustfmt against generated scenario test");
    assert!(
        formatted.status.success(),
        "generated Rust is not rustfmt-clean:\n{}{}",
        String::from_utf8_lossy(&formatted.stdout),
        String::from_utf8_lossy(&formatted.stderr)
    );
    assert!(!output.join("ess/synthesis/Cargo.toml").exists());
}
