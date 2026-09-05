//! ESS-first standalone service construction.
//!
//! The builder compiles through official [`ess_compiler::EssIr`], consumes ESS's own structural
//! [`ess_synth::SynthesisPlan`], and adds only production-service realization artifacts. It never
//! reconstructs semantic handles or emits a second copy of ESS-owned types.

pub mod client;
pub mod ess;
pub mod http;
pub mod package;
pub mod realization;
pub mod release;
pub mod tree;

use std::collections::BTreeMap;

use anyhow::{Context as _, Result};
use ess_compiler::EssIr;
use ess_gen::Artifact;
use service_catalog::{
    CatalogAuthentication, CatalogOperation, CatalogOperationEffect, CatalogOperationKind,
    RealmPolicy as CatalogRealmPolicy, SERVICE_CATALOG_FORMAT, ServiceCatalog,
};
use service_connectors::ConnectorServiceFactoryDescriptor;
use service_definition::ServiceDefinition;
use service_runtime_ir::ServiceRuntimeIr;

use crate::client::ClientPlan;
use crate::package::ServicePackage;
use crate::realization::RealizationArtifacts;
use crate::tree::ArtifactTree;

/// Canonical generated runtime IR path.
pub const RUNTIME_IR_PATH: &str = "runtime/ir.json";

/// Canonical compiler-minted ESS IR path used by composition tooling.
pub const ESS_IR_PATH: &str = "ess/ir.json";

/// Canonical generated client plan path.
pub const CLIENT_PLAN_PATH: &str = "client/plan.json";

/// Canonical generated inert Connector contribution path.
pub const CONNECTOR_CONTRIBUTION_PATH: &str = "connectors/contribution.json";

/// Canonical generated catalog consumed by docs, its external factory, and application widgets.
pub const SERVICE_CATALOG_PATH: &str = "catalog/service-catalog.json";

/// Canonical generated executable realization-plan path.
pub const REALIZATION_PLAN_PATH: &str = "runtime/realization-plan.json";

/// Canonical generated Identity HTTP `OpenAPI` path.
pub const HTTP_OPENAPI_PATH: &str = "http/openapi.json";

/// ESS-owned structural synthesis and projections for one compiled model.
pub struct EssBuild {
    /// Compiler-minted resolved semantics.
    pub ir: EssIr,
    /// The language-neutral ESS synthesis plan.
    pub plan: ess_synth::SynthesisPlan,
    /// ESS-owned synthesis artifacts, keyed by relative path.
    pub synthesis: BTreeMap<String, Artifact>,
    /// ESS-owned contract/document projections, keyed by relative path.
    pub projections: BTreeMap<String, Artifact>,
}

/// Complete result of compiling one standalone service definition.
pub struct ServiceBuild {
    /// ESS compiler, plan, and official generator results.
    pub ess: EssBuild,
    /// Closed service-runtime realization IR.
    pub runtime_ir: ServiceRuntimeIr,
    /// Realm-free transport-neutral client plan.
    pub client_plan: ClientPlan,
    /// Inert descriptor a generated service adapter uses to implement `ConnectorServiceFactory`.
    ///
    /// The composing product supplies only its explicit `ServiceDeployment` binding.
    pub connector_descriptor: ConnectorServiceFactoryDescriptor,
    /// Versioned application catalog derived from ESS and the exact executable operation schemas.
    pub service_catalog: ServiceCatalog,
    /// SDK-executable realization plan derived from ESS and the runtime IR.
    pub realization_plan: service_engine::ServicePlan,
    /// Complete exclusively owned generated output tree.
    pub artifacts: ArtifactTree,
}

/// Runs the compiler and both established ESS artifact pipelines.
pub fn build_ess(sources: &ess::EssSources) -> Result<EssBuild> {
    let ir = sources.compile()?;
    let synthesized = ess_synth::synthesize(&ir).into_synthesis_result()?;
    admit_rust_target(synthesized.target.as_ref())?;
    let projections = ess_gen::generate_all(&ir)?;
    Ok(EssBuild {
        ir,
        plan: synthesized.plan,
        synthesis: synthesized.artifacts,
        projections,
    })
}

// Admit both the pinned direct return and the checked ESS facade without changing SDK's API.
trait IntoSynthesisResult {
    fn into_synthesis_result(self) -> Result<ess_synth::Synthesis>;
}

impl IntoSynthesisResult for ess_synth::Synthesis {
    fn into_synthesis_result(self) -> Result<ess_synth::Synthesis> {
        Ok(self)
    }
}

impl<E: std::error::Error + Send + Sync + 'static> IntoSynthesisResult
    for std::result::Result<ess_synth::Synthesis, E>
{
    fn into_synthesis_result(self) -> Result<ess_synth::Synthesis> {
        self.map_err(anyhow::Error::new)
    }
}

fn admit_rust_target(report: Option<&ess_synth::TargetReport>) -> Result<()> {
    let Some(report) = report else {
        return Ok(());
    };
    // The SDK has no accounting path for weakened guarantees or ungenerated capabilities.
    if report.target != "rust" || !report.is_empty() {
        anyhow::bail!(
            "ESS synthesis target `{}` is not admitted: expected `rust` without refusals or \
             weakenings.\n{}",
            report.target,
            report.to_markdown(),
        );
    }
    Ok(())
}

/// Compiles ESS and runtime annotations into the complete deterministic generated tree.
pub fn build_service(
    sources: &ess::EssSources,
    definition: &ServiceDefinition,
) -> Result<ServiceBuild> {
    let ess = build_ess(sources)?;
    let runtime_ir = service_runtime_ir::compile(&ess.ir, &ess.plan, definition)
        .context("compiling service runtime IR")?;
    let client_plan = ClientPlan::from_runtime(&runtime_ir).context("deriving client plan")?;
    let connector_descriptor = client_plan
        .connector_descriptor()
        .context("deriving inert Connector contribution")?;
    let realization_plan = realization::compile(&ess.ir, &runtime_ir, &client_plan)
        .context("compiling executable service realization plan")?;
    let service_catalog =
        build_service_catalog(&ess, &client_plan, &realization_plan, &connector_descriptor)
            .context("deriving service catalog")?;

    let mut artifacts = ArtifactTree::new();
    artifacts.insert(ESS_IR_PATH, ess.ir.to_canonical_json())?;
    for path in ["PLAN.md", "plan.json"] {
        if let Some(artifact) = ess.synthesis.get(path) {
            artifacts.insert(format!("ess/synthesis/{path}"), artifact.contents.clone())?;
        }
    }
    artifacts.extend_ess("ess/projections", &ess.projections)?;
    artifacts.insert(RUNTIME_IR_PATH, runtime_ir.to_canonical_json())?;
    artifacts.insert(CLIENT_PLAN_PATH, client_plan.to_canonical_json())?;
    artifacts.insert(REALIZATION_PLAN_PATH, realization_plan.to_canonical_json())?;
    artifacts.insert(
        CONNECTOR_CONTRIBUTION_PATH,
        connector_descriptor.to_canonical_json(),
    )?;
    artifacts.insert(SERVICE_CATALOG_PATH, service_catalog.to_canonical_json())?;
    if let Some(openapi) = http::openapi(&client_plan) {
        artifacts.insert(HTTP_OPENAPI_PATH, openapi)?;
    }

    Ok(ServiceBuild {
        ess,
        runtime_ir,
        client_plan,
        connector_descriptor,
        service_catalog,
        realization_plan,
        artifacts,
    })
}

/// Compiles one unified package and emits its complete compilable Rust and Connector factory.
pub fn build_package(package: &ServicePackage) -> Result<ServiceBuild> {
    let mut build = build_service(&package.sources, &package.definition)?;
    package.validate_scenarios(&build.client_plan)?;
    let generated = RealizationArtifacts::generate(
        &build.realization_plan,
        &build.client_plan,
        &package.manifest.sdk,
        package
            .scenarios
            .iter()
            .map(|scenario| scenario.path.as_str()),
    );
    if let Some(release) = release::ReleaseArtifacts::generate(
        package,
        &build.ess,
        &build.realization_plan,
        &generated.files,
    )? {
        for (path, contents) in release.files {
            build.artifacts.insert(path, contents)?;
        }
    }
    for (path, contents) in generated.files {
        build.artifacts.insert(path, contents)?;
    }
    for scenario in &package.scenarios {
        build.artifacts.insert(
            format!("conformance/{}", scenario.path),
            scenario.contents.clone(),
        )?;
    }
    Ok(build)
}

fn build_service_catalog(
    ess: &EssBuild,
    client: &ClientPlan,
    plan: &service_engine::ServicePlan,
    descriptor: &ConnectorServiceFactoryDescriptor,
) -> Result<ServiceCatalog> {
    let ess_catalog = ess_synth::web::browser_catalog(&ess.ir, &ess.plan);
    let semantic_catalog = serde_json::from_str(ess_catalog.as_json())
        .context("ESS browser catalog is canonical JSON")?;
    let operations = client
        .operations
        .iter()
        .map(|operation| {
            let (kind, effect) = match operation.kind {
                client::ClientOperationKind::Intent => {
                    (CatalogOperationKind::Intent, CatalogOperationEffect::Write)
                }
                client::ClientOperationKind::Query => {
                    (CatalogOperationKind::Query, CatalogOperationEffect::Read)
                }
            };
            let contribution = descriptor
                .contribution()
                .operations
                .iter()
                .find(|candidate| candidate.operation == operation.operation)
                .ok_or_else(|| anyhow::anyhow!("catalog operation lost Connector binding"))?;
            let (input_schema, output_schema) =
                service_connectors::operation_schemas(plan, contribution);
            Ok(CatalogOperation {
                name: operation.operation.clone(),
                operation_ref: format!("{}.{}", client.service, operation.operation),
                semantic_ref: operation.semantic_ref.clone(),
                kind,
                effect,
                input_schema,
                output_schema,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    ServiceCatalog::new(ServiceCatalog {
        format: SERVICE_CATALOG_FORMAT.to_owned(),
        service_ref: format!("service:{}", client.service),
        display_name: client.service.replace('_', " "),
        description: format!("Generated {} service", client.service.replace('_', " ")),
        semantic_catalog,
        authentication: CatalogAuthentication {
            source: "session".to_owned(),
            realm_policy: match client.realm_policy {
                service_definition::RealmPolicy::Required => CatalogRealmPolicy::Required,
                service_definition::RealmPolicy::Optional => CatalogRealmPolicy::Optional,
                service_definition::RealmPolicy::Forbidden => CatalogRealmPolicy::Forbidden,
            },
        },
        operations,
    })
    .context("service catalog is valid")
}

#[cfg(test)]
mod synthesis_outcome_admission {
    use std::collections::BTreeMap;

    use ess_gen::Artifact;
    use ess_synth::{Synthesis, SynthesisPlan, TargetReport};

    use super::IntoSynthesisResult;
    use crate::ess::EssSources;

    fn synthesis() -> Synthesis {
        let sources = EssSources::new(BTreeMap::from([(
            "system.yaml".to_owned(),
            "format: ess/1\nsystem: admission\nversion: v1\ndomains: [admission.model]\ndomain: admission.model\ntypes:\n  - name: admission.model.ItemId\n    kind: newtype\n    of: String\n"
                .to_owned(),
        )]))
        .expect("valid typed control source");
        let ir = sources.compile().expect("control compiles");
        let plan = SynthesisPlan::of(&ir);
        let target = Some(TargetReport {
            provenance: plan.provenance.clone(),
            target: "rust",
            weakenings: Vec::new(),
            refusals: Vec::new(),
        });
        Synthesis {
            plan,
            artifacts: BTreeMap::from([(
                "control.txt".to_owned(),
                Artifact::new("control.txt", "original bytes\n".to_owned()),
            )]),
            target,
        }
    }

    fn assert_original_synthesis(actual: &Synthesis) {
        let expected = synthesis();
        assert_eq!(actual.plan, expected.plan);
        assert_eq!(actual.artifacts, expected.artifacts);
        assert_eq!(actual.target, expected.target);
    }

    #[derive(Debug, PartialEq, Eq, thiserror::Error)]
    #[error("Rust target failure at demo.lib: {0}")]
    struct FixtureTargetFailure(&'static str);

    #[test]
    fn historical_rust_success_preserves_every_ess_artifact() {
        let sources = EssSources::new(BTreeMap::from([(
            "system.yaml".to_owned(),
            include_str!("../tests/fixtures/service.ess.yaml").to_owned(),
        )]))
        .expect("fixture sources are valid");
        let ir = sources.compile().expect("fixture compiles through ESS");
        let synthesized = ess_synth::synthesize(&ir)
            .into_synthesis_result()
            .expect("valid Rust synthesis succeeds");
        assert!(
            synthesized.target.is_none(),
            "the current pinned Rust producer represents success with no report"
        );
        let projections = ess_gen::generate_all(&ir).expect("fixture projects through ESS");

        let built = super::build_ess(&sources).expect("historical Rust success builds");
        assert_eq!(built.ir.to_canonical_json(), ir.to_canonical_json());
        assert_eq!(built.plan, synthesized.plan);
        assert_eq!(built.synthesis, synthesized.artifacts);
        assert_eq!(built.projections, projections);
    }

    #[test]
    fn direct_synthesis_keeps_plan_artifacts_and_target() {
        assert_original_synthesis(
            &synthesis()
                .into_synthesis_result()
                .expect("the existing direct return is admitted"),
        );
    }

    #[test]
    fn fallible_success_keeps_plan_artifacts_and_target() {
        assert_original_synthesis(
            &Ok::<_, FixtureTargetFailure>(synthesis())
                .into_synthesis_result()
                .expect("a future successful result is admitted"),
        );
    }

    #[test]
    fn fallible_failure_preserves_original_typed_cause_without_a_capability() {
        let error = Err::<Synthesis, _>(FixtureTargetFailure(
            "output lib.rs collides although the domain has zero capabilities",
        ))
        .into_synthesis_result()
        .err()
        .expect("a failed synthesis cannot become a successful builder input");
        assert_eq!(
            error.to_string(),
            "Rust target failure at demo.lib: output lib.rs collides although the domain has zero capabilities"
        );
        assert_eq!(
            error.downcast_ref::<FixtureTargetFailure>(),
            Some(&FixtureTargetFailure(
                "output lib.rs collides although the domain has zero capabilities"
            )),
            "the original typed cause remains available to callers"
        );
    }
}

#[cfg(test)]
mod rust_target_admission {
    use ess_gen::Provenance;
    use ess_synth::{Capability, CapabilityKind, TargetRefusal, TargetReport, TargetWeakening};

    use super::admit_rust_target;

    fn report(target: &'static str) -> TargetReport {
        TargetReport {
            provenance: Provenance {
                system: "admission-fixture".to_owned(),
                specification_version: "v1".to_owned(),
                source_digest: "source-digest".to_owned(),
                contract_digest: "contract-digest".to_owned(),
            },
            target,
            weakenings: Vec::new(),
            refusals: Vec::new(),
        }
    }

    fn refusal(kind: CapabilityKind, source: &str, detail: &str) -> TargetRefusal {
        TargetRefusal {
            capability: Capability {
                kind,
                source: source.to_owned(),
            },
            detail: detail.to_owned(),
        }
    }

    fn weakening() -> TargetWeakening {
        TargetWeakening {
            guarantee: "exhaustive lifecycle transitions".to_owned(),
            instead: "runtime checks only; preserve this original explanation".to_owned(),
            affects: vec![
                CapabilityKind::EntityLifecycle,
                CapabilityKind::CommandContract,
            ],
        }
    }

    #[test]
    fn historical_absent_report_is_admitted() {
        admit_rust_target(None).expect("historical Rust success is admitted");
    }

    #[test]
    fn empty_rust_report_is_admitted() {
        admit_rust_target(Some(&report("rust"))).expect("complete Rust output is admitted");
    }

    #[test]
    fn mismatched_target_is_rejected_even_without_notes() {
        for target in ["go", "web", "clap", "Rust", "rust ", ""] {
            let error = admit_rust_target(Some(&report(target)))
                .expect_err("an empty report for another target is not Rust success")
                .to_string();
            assert!(error.contains(&format!("`{target}`")), "{error}");
            assert!(error.contains("expected `rust`"), "{error}");
        }
    }

    #[test]
    fn every_refusal_keeps_its_capability_source_and_original_reason() {
        let mut report = report("rust");
        report.refusals = vec![
            refusal(
                CapabilityKind::DomainType,
                "demo.domain.Foo_Bar",
                "normalizes to FooBar; collides with demo.domain.FooBar",
            ),
            refusal(
                CapabilityKind::EntityLifecycle,
                "demo.domain.Node",
                "optional self-reference has infinite size\nsource: domain.yaml:17 — use indirection",
            ),
        ];
        let error = admit_rust_target(Some(&report))
            .expect_err("refused Rust capabilities cannot become a successful service")
            .to_string();
        assert!(error.contains("`rust`"), "{error}");
        for refusal in &report.refusals {
            assert!(
                error.contains(refusal.capability.kind.describes()),
                "{error}"
            );
            assert!(error.contains(&refusal.capability.source), "{error}");
            assert!(error.contains(&refusal.detail), "{error}");
        }
        assert_eq!(
            error,
            admit_rust_target(Some(&report)).unwrap_err().to_string(),
            "the same report produces identical diagnostics"
        );
    }

    #[test]
    fn weakening_without_refusals_keeps_guarantee_replacement_and_affected_capabilities() {
        let mut report = report("rust");
        report.weakenings.push(weakening());
        let error = admit_rust_target(Some(&report))
            .expect_err("SDK cannot silently drop weaker guarantees")
            .to_string();
        let weakening = &report.weakenings[0];
        assert!(error.contains("`rust`"), "{error}");
        assert!(error.contains(&weakening.guarantee), "{error}");
        assert!(error.contains(&weakening.instead), "{error}");
        for kind in &weakening.affects {
            assert!(error.contains(kind.describes()), "{error}");
        }
    }

    #[test]
    fn mismatched_target_preserves_both_refusals_and_weakenings() {
        let mut report = report("web");
        report.refusals.push(refusal(
            CapabilityKind::DomainType,
            "demo.domain.Refused",
            "original refusal must not disappear behind target mismatch",
        ));
        report.weakenings.push(weakening());
        let error = admit_rust_target(Some(&report))
            .expect_err("all accounting from the wrong target must remain visible")
            .to_string();
        assert!(error.contains("`web`"), "{error}");
        assert!(error.contains("expected `rust`"), "{error}");
        assert!(error.contains("demo.domain.Refused"), "{error}");
        assert!(error.contains(&report.refusals[0].detail), "{error}");
        assert!(error.contains(&report.weakenings[0].guarantee), "{error}");
        assert!(error.contains(&report.weakenings[0].instead), "{error}");
    }
}

#[cfg(test)]
#[path = "../tests/support/rust_target_adversary.rs"]
mod rust_target_adversary;
