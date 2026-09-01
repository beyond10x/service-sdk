//! ESS-first standalone service construction.
//!
//! The builder compiles through official [`ess_compiler::EssIr`], consumes ESS's own structural
//! [`ess_synth::SynthesisPlan`], and adds only production-service realization artifacts. It never
//! reconstructs semantic handles or emits a second copy of ESS-owned types.

pub mod client;
pub mod ess;
pub mod tree;

use std::collections::BTreeMap;

use anyhow::{Context as _, Result};
use ess_compiler::EssIr;
use ess_gen::Artifact;
use service_connectors::ConnectorServiceFactoryDescriptor;
use service_definition::ServiceDefinition;
use service_runtime_ir::ServiceRuntimeIr;

use crate::client::ClientPlan;
use crate::tree::ArtifactTree;

/// Canonical generated runtime IR path.
pub const RUNTIME_IR_PATH: &str = "runtime/ir.json";

/// Canonical generated client plan path.
pub const CLIENT_PLAN_PATH: &str = "client/plan.json";

/// Canonical generated inert Connector contribution path.
pub const CONNECTOR_CONTRIBUTION_PATH: &str = "connectors/contribution.json";

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
    /// Complete exclusively owned generated output tree.
    pub artifacts: ArtifactTree,
}

/// Runs the compiler and both established ESS artifact pipelines.
pub fn build_ess(sources: &ess::EssSources) -> Result<EssBuild> {
    let ir = sources.compile()?;
    let synthesized = ess_synth::synthesize(&ir);
    let projections = ess_gen::generate_all(&ir)?;
    Ok(EssBuild {
        ir,
        plan: synthesized.plan,
        synthesis: synthesized.artifacts,
        projections,
    })
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

    let mut artifacts = ArtifactTree::new();
    artifacts.extend_ess("ess/synthesis", &ess.synthesis)?;
    artifacts.extend_ess("ess/projections", &ess.projections)?;
    artifacts.insert(RUNTIME_IR_PATH, runtime_ir.to_canonical_json())?;
    artifacts.insert(CLIENT_PLAN_PATH, client_plan.to_canonical_json())?;
    artifacts.insert(
        CONNECTOR_CONTRIBUTION_PATH,
        connector_descriptor.to_canonical_json(),
    )?;

    Ok(ServiceBuild {
        ess,
        runtime_ir,
        client_plan,
        connector_descriptor,
        artifacts,
    })
}
