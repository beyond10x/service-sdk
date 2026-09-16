//! Acceptance for strict `/4` fixture compilation and reload.

use service_builder::ess::EssSources;
use service_builder::package::ServicePackageV4;
use service_definition::{
    ServiceDefinition,
    v4::{SERVICE_DEFINITION_FORMAT_V4, ServiceDefinitionV4},
};
use service_engine::v4::{REALIZATION_PLAN_FORMAT_V4, ServicePlanV4};
use service_runtime_ir::v4::{RUNTIME_IR_FORMAT_V4, ServiceRuntimeIrV4};
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
