//! Acceptance for strict `/4` fixture compilation and reload.

use service_builder::ess::EssSources;
use service_builder::package::ServicePackageV4;
use service_definition::{
    ServiceDefinition,
    v4::{OperationFieldPolicy, SERVICE_DEFINITION_FORMAT_V4, ServiceDefinitionV4},
};
use service_engine::v4::{REALIZATION_PLAN_FORMAT_V4, ServicePlanV4};
use service_runtime_ir::v4::{RUNTIME_IR_FORMAT_V4, ServiceRuntimeIrV4};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../service-host/tests/fixtures/er-v4")
        .join(name)
}

fn sources(name: &str) -> EssSources {
    let base = fixture(name).join("ess");
    let mut pending = vec![base.clone()];
    let mut paths = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).expect("fixture directory") {
            let path = entry.expect("fixture entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "yaml") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    EssSources::new(
        paths
            .into_iter()
            .map(|path| {
                let label = path
                    .strip_prefix(&base)
                    .expect("fixture child")
                    .display()
                    .to_string();
                let text = fs::read_to_string(path).expect("fixture source");
                (label, text)
            })
            .collect::<BTreeMap<_, _>>(),
    )
    .expect("explicit source inventory")
}

fn definition(name: &str) -> ServiceDefinitionV4 {
    ServiceDefinitionV4::from_yaml(
        &fs::read_to_string(fixture(name).join("runtime.yaml")).expect("runtime fixture"),
    )
    .expect("strict v4 definition")
}

#[test]
fn billing_and_gatepass_compile_to_closed_er_delegated_artifacts() {
    for name in ["billing", "gatepass"] {
        let sources = sources(name);
        let definition = definition(name);
        let build = service_builder::build_service_v4(&sources, &definition)
            .expect("complete v4 fixture compiles");
        assert_eq!(definition.format, SERVICE_DEFINITION_FORMAT_V4);
        let runtime = build.runtime_ir.to_canonical_json();
        let realization = build.realization_plan.to_canonical_json();
        assert!(runtime.contains(RUNTIME_IR_FORMAT_V4));
        assert!(realization.contains(REALIZATION_PLAN_FORMAT_V4));
        assert!(!realization.contains("__entity_runtime_selected__"));
        assert!(!realization.contains("\"reducers\""));
        assert_eq!(
            ServiceRuntimeIrV4::from_json_bound(&runtime, &build.ess.ir, &build.ess.plan)
                .expect("strict runtime reload"),
            build.runtime_ir
        );
        assert_eq!(
            ServicePlanV4::from_json(&realization).expect("strict plan reload"),
            build.realization_plan
        );
        assert!(
            ServiceDefinition::from_yaml(
                &fs::read_to_string(fixture(name).join("runtime.yaml")).unwrap()
            )
            .is_err(),
            "the /3 reader rejects /4"
        );
    }
}

#[test]
fn complete_fixture_coordinates_and_absence_actions_survive_compilation() {
    let billing = service_builder::build_service_v4(&sources("billing"), &definition("billing"))
        .expect("billing compiles");
    assert_eq!(
        billing.runtime_ir.entity_runtime().bindings.commands.len(),
        4
    );
    assert_eq!(billing.runtime_ir.slots().len(), 11);
    assert_eq!(billing.runtime_ir.operation_fields().len(), 36);
    let billing_json = billing.realization_plan.to_canonical_json();
    assert!(billing_json.contains("notify-on-invoice-created"));
    assert!(billing_json.contains("\"kind\": \"absent\""));
    assert!(billing_json.contains("\"name\": \"issued_clock\""));

    let gatepass = service_builder::build_service_v4(&sources("gatepass"), &definition("gatepass"))
        .expect("gatepass compiles");
    assert_eq!(
        gatepass.runtime_ir.entity_runtime().bindings.commands.len(),
        3
    );
    assert_eq!(gatepass.runtime_ir.slots().len(), 11);
    assert_eq!(gatepass.runtime_ir.operation_fields().len(), 20);
    let plan = gatepass.realization_plan.to_canonical_json();
    assert!(plan.contains("gatepass.visit.AdmitVisitor"));
    assert!(!plan.contains("\"kind\": \"remove\""));
    assert!(plan.contains("\"field\": \"badge\""));
}

#[test]
fn strict_reloads_refuse_changed_definition_binding_and_target_revision() {
    let build = service_builder::build_service_v4(&sources("billing"), &definition("billing"))
        .expect("billing compiles");
    let runtime = build.runtime_ir.to_canonical_json();
    for changed in [
        runtime.replacen("\"service\": \"billing\"", "\"service\": \"other\"", 1),
        runtime.replacen(
            "7fd93ef43d4a91c460c7305f9e3be90d7b0a4c11",
            "0000000000000000000000000000000000000000",
            1,
        ),
        runtime.replacen(
            "\"kind\": \"absent\"",
            "\"kind\": \"literal\", \"value\": null",
            1,
        ),
    ] {
        assert!(
            ServiceRuntimeIrV4::from_json_bound(&changed, &build.ess.ir, &build.ess.plan).is_err()
        );
    }
}

#[test]
fn operation_field_command_sources_must_match_the_lowerer_field_type() {
    let mut definition = definition("billing");
    let issued_at = definition
        .operation_fields
        .iter_mut()
        .find(|binding| {
            binding.coordinate.command.to_string() == "billing.invoice.IssueInvoice"
                && binding.coordinate.outcome.as_str() == "issued"
                && binding.coordinate.field == "issued_at"
        })
        .expect("billing binds IssueInvoice.issued.issued_at");
    issued_at.policy = OperationFieldPolicy::CommandField {
        field: "invoice_id".to_owned(),
    };

    assert!(
        service_builder::build_service_v4(&sources("billing"), &definition).is_err(),
        "a UUID command field cannot fulfill the lowerer's Timestamp operation field"
    );
}

#[test]
fn realization_reader_refuses_a_self_consistent_incompatible_er_target() {
    let mut plan = service_builder::build_service_v4(&sources("billing"), &definition("billing"))
        .expect("billing compiles")
        .realization_plan;
    plan.er.target_revision = "0000000000000000000000000000000000000000".to_owned();
    plan.plan_digest.clear();
    plan.plan_digest = hex::encode(Sha256::digest(serde_json::to_vec(&plan).unwrap()));

    assert!(
        ServicePlanV4::from_json(&plan.to_canonical_json()).is_err(),
        "the executable reader cannot admit a plan for another ER semantic target"
    );
}

#[test]
fn billing_and_gatepass_packages_emit_strict_generated_http_services() {
    for name in ["billing", "gatepass"] {
        let package = ServicePackageV4::read(&fixture(name).join("package.yaml"))
            .expect("strict v4 package reads");
        let build =
            service_builder::build_package_v4(&package).expect("strict v4 package generates");
        let generated = build.artifacts.iter().collect::<BTreeMap<_, _>>();
        let cargo = generated["rust/Cargo.toml"];
        let source = generated["rust/src/lib.rs"];
        assert!(cargo.contains("service-engine"));
        assert!(source.contains("ServicePlanV4::from_json"));
        assert!(source.contains("IdentityHttpServiceV4::initialize"));
        assert!(source.contains("intent_v4"));
        assert!(source.contains("pub async fn get_") || source.contains("pub async fn list_"));
    }
}

#[test]
fn generated_v4_openapi_describes_the_v4_mutation_contract() {
    let package = ServicePackageV4::read(&fixture("billing").join("package.yaml"))
        .expect("strict v4 package reads");
    let build = service_builder::build_package_v4(&package).expect("strict v4 package generates");
    let artifacts = build.artifacts.iter().collect::<BTreeMap<_, _>>();
    let openapi: serde_json::Value =
        serde_json::from_str(artifacts["http/openapi.json"]).expect("generated OpenAPI is JSON");
    let receipt = &openapi["components"]["schemas"]["MutationReceipt"];
    let properties = receipt["properties"]
        .as_object()
        .expect("mutation receipt properties");
    assert!(properties.contains_key("response"));
    assert!(properties.contains_key("commit"));
    assert!(
        openapi["paths"]["/v1/intents/issue_invoice"]["post"]["responses"]
            .get("202")
            .is_some(),
        "committed aftercare is a declared HTTP result"
    );
}
