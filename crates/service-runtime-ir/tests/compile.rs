//! Cross-crate proofs for exact ESS/synthesis binding and fail-closed annotation resolution.

use ess_compiler::EssIr;
use ess_compiler::source::SourceMap;
use ess_domain::spec::{RawSpecFile, Specification};
use ess_domain::system::Source;
use ess_synth::SynthesisPlan;
use service_definition::ServiceDefinition;
use service_runtime_ir::{
    INTENT_PIPELINE, RuntimeCode, ServiceRuntimeIr, compile as compile_runtime,
};

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
      - { name: authority_id, type: String }
entities:
  - name: demo.todo.Item
    identity: { name: item_id, type: demo.todo.ItemId }
    fields:
      - { name: content_ref, type: demo.todo.ContentRef }
      - { name: owner, type: demo.todo.OwnerRef }
      - { name: authority_id, type: String }
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
      - { name: authority_id, type: String }
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
    expected_version: { kind: no_stream }
    idempotency: { kind: operation_field, field: idempotency_key }
    guards:
      - name: may_add
        refusal_code: forbidden
        reads:
          - { kind: command_field, field: item_id }
          - { kind: context, value: current_authority }
    content:
      - content: item_content
        input_field: body
        command_reference_field: content_ref
    event_bindings:
      - event: demo.todo.ItemAdded
        field: owner
        source: { kind: context, value: current_authority }
      - event: demo.todo.ItemAdded
        field: authority_id
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

fn ess(source: &str) -> EssIr {
    let raw = RawSpecFile::parse(source).expect("fixture is well formed");
    let specification =
        Specification::assemble([(Source::new("demo.yaml"), raw)]).expect("fixture validates");
    let mut sources = SourceMap::new();
    sources.insert("demo.yaml", source);
    ess_compiler::compile(&specification, &sources).expect("fixture resolves")
}

fn inputs() -> (EssIr, SynthesisPlan, ServiceDefinition) {
    let ir = ess(ESS);
    let plan = SynthesisPlan::of(&ir);
    let definition = ServiceDefinition::from_yaml(DEFINITION).expect("definition validates");
    (ir, plan, definition)
}

#[test]
fn canonical_roundtrip_binds_exact_ess_and_synthesis_and_loses_no_annotations() {
    let (ir, plan, definition) = inputs();
    let first = compile_runtime(&ir, &plan, &definition).expect("runtime annotations resolve");
    let second = compile_runtime(&ir, &plan, &definition).expect("same compilation resolves");
    let canonical = first.to_canonical_json();

    assert_eq!(canonical, second.to_canonical_json());
    assert_eq!(first.ess_source_digest(), ir.source_digest());
    assert_eq!(
        first.intents()[&"add_item".parse().unwrap()].pipeline,
        INTENT_PIPELINE
    );
    assert_eq!(
        ServiceRuntimeIr::from_json_bound(&canonical, &ir, &plan).expect("bound document reads"),
        first
    );
    assert_eq!(first.definition(), &definition);

    for annotation in [
        "service-definition/1",
        "demo_todo",
        "optional",
        "item_content",
        "demo.todo.ContentRef",
        "text/markdown",
        "65536",
        "external_erasable",
        "bind_owner",
        "Bind owner from current authenticated authority.",
        "item_by_id",
        "inline_transactional",
        "add_item",
        "demo.todo.AddItem",
        "item_id",
        "no_stream",
        "idempotency_key",
        "may_add",
        "forbidden",
        "current_authority",
        "authority_id",
        "body",
        "content_ref",
        "demo.todo.ItemAdded",
        "get_item",
        "owner",
        "ascending",
        "read_your_writes",
    ] {
        assert!(
            canonical.contains(annotation),
            "runtime IR dropped annotation {annotation:?}: {canonical}"
        );
    }
}

#[test]
fn persisted_ir_is_closed_and_recompiled_before_acceptance() {
    let (ir, plan, definition) = inputs();
    let runtime = compile_runtime(&ir, &plan, &definition).expect("runtime annotations resolve");
    let canonical = runtime.to_canonical_json();

    let unknown = canonical.replacen(
        "\"format\": \"service-runtime-ir/1\"",
        "\"format\": \"service-runtime-ir/1\",\n  \"route\": \"/realms/default\"",
        1,
    );
    let error = ServiceRuntimeIr::from_json_bound(&unknown, &ir, &plan)
        .expect_err("unknown persisted fields must fail");
    assert_eq!(error.diagnostics()[0].code, RuntimeCode::InvalidPersistedIr);

    let tampered = canonical.replacen(
        "\"command\": \"demo.todo.AddItem\"",
        "\"command\": \"demo.todo.Missing\"",
        1,
    );
    let error = ServiceRuntimeIr::from_json_bound(&tampered, &ir, &plan)
        .expect_err("tampered semantics must fail recompilation");
    assert!(error.diagnostics().iter().any(|item| {
        matches!(
            item.code,
            RuntimeCode::MissingSemantic | RuntimeCode::PersistedIrMismatch
        )
    }));
}

#[test]
fn every_unresolved_or_contextually_unsupported_annotation_is_refused() {
    let (ir, plan, _) = inputs();

    let missing_command = ServiceDefinition::from_yaml(
        &DEFINITION.replace("command: demo.todo.AddItem", "command: demo.todo.Missing"),
    )
    .expect("the definition layer validates names structurally");
    let error = compile_runtime(&ir, &plan, &missing_command)
        .expect_err("missing semantic command must fail");
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|item| item.code == RuntimeCode::MissingSemantic)
    );

    let wrong_guard = ServiceDefinition::from_yaml(&DEFINITION.replacen(
        "reads:\n          - { kind: command_field, field: item_id }",
        "reads:\n          - { kind: view_field, field: item_id }",
        1,
    ))
    .expect("the definition layer permits either closed guard-read shape");
    let error =
        compile_runtime(&ir, &plan, &wrong_guard).expect_err("view read in intent must fail");
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|item| item.code == RuntimeCode::UnsupportedAnnotation)
    );

    let mismatched_content = ServiceDefinition::from_yaml(&DEFINITION.replace(
        "reference_type: demo.todo.ContentRef",
        "reference_type: demo.todo.ItemId",
    ))
    .expect("both type names are structurally valid");
    let error = compile_runtime(&ir, &plan, &mismatched_content)
        .expect_err("content reference mismatch must fail");
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|item| item.code == RuntimeCode::ContentTypeMismatch)
    );
}

#[test]
fn a_plan_for_any_other_model_is_refused_even_when_its_shape_was_cloned() {
    let (ir, mut plan, definition) = inputs();
    plan.provenance.source_digest = "0".repeat(64);
    let error = compile_runtime(&ir, &plan, &definition).expect_err("wrong plan must fail");
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|item| item.code == RuntimeCode::SynthesisModelMismatch)
    );
}

#[test]
fn auth_coordinates_in_ess_command_inputs_are_refused() {
    for coordinate in [
        "tenant_id",
        "realm_id",
        "user_id",
        "authority",
        "authority_id",
        "authorityid",
        "current_authority",
        "principal",
        "principal_id",
        "principalid",
        "executor",
        "executor_id",
        "executorid",
    ] {
        let source = ESS.replace(
            "- { name: content_ref, type: demo.todo.ContentRef }\n    outcomes:",
            &format!(
                "- {{ name: content_ref, type: demo.todo.ContentRef }}\n      - {{ name: {coordinate}, type: String }}\n    outcomes:"
            ),
        );
        let ir = ess(&source);
        let plan = SynthesisPlan::of(&ir);
        let definition = ServiceDefinition::from_yaml(DEFINITION).expect("definition validates");
        let Err(error) = compile_runtime(&ir, &plan, &definition) else {
            panic!("{coordinate} must not become caller input");
        };
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|item| item.code == RuntimeCode::AuthorityCoordinate),
            "{coordinate} was refused for the wrong reason: {error}"
        );
    }
}
