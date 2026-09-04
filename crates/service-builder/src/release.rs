//! ESS-native release artifacts for independently buildable generated services.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result, bail};
use ess_deployment::{
    BuildSpec, ComponentSpec, RuntimeSpec, compile_build, compile_component, compile_runtime,
    project_build_mermaid, project_buildkit, project_helm,
};
use serde_json::{Value, json};
use service_engine::{PlanDelivery, ServicePlan};
use sha2::{Digest as _, Sha256};

use crate::EssBuild;
use crate::package::{ReleasePackage, ServicePackage};

/// Generated release files, all beneath the service-builder output root.
pub struct ReleaseArtifacts {
    /// Relative artifact path to UTF-8 bytes.
    pub files: BTreeMap<String, String>,
}

impl ReleaseArtifacts {
    /// Generates and validates the complete component → build → runtime → chart chain.
    pub fn generate(
        package: &ServicePackage,
        ess: &EssBuild,
        plan: &ServicePlan,
        generated: &BTreeMap<String, String>,
    ) -> Result<Option<Self>> {
        let Some(release) = &package.manifest.release else {
            return Ok(None);
        };
        let service = &package.manifest.service;
        let component_names = ess
            .ir
            .components()
            .keys()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if component_names.is_empty() {
            bail!("an independently released service must realize at least one ESS component");
        }

        let realization_yaml = realization_yaml(service, ess, generated, &component_names)?;
        let realization_spec = ess_realization::RealizationSpec::from_yaml(&realization_yaml)
            .context("generated ESS realization does not parse")?;
        let realization = ess_realization::compile(&realization_spec, &ess.ir)
            .context("generated ESS realization was refused")?;

        let build_yaml = build_yaml(service, release);
        let build_spec = BuildSpec::from_yaml(&build_yaml)
            .context("generated ESS build description does not parse")?;
        let build = compile_build(&build_spec).context("generated ESS build was refused")?;

        let runtime_yaml = runtime_yaml(
            service,
            release,
            ess,
            plan,
            &realization,
            &build,
            &component_names,
        )?;
        let runtime_spec = RuntimeSpec::from_yaml(&runtime_yaml)
            .context("generated ESS runtime description does not parse")?;
        let runtime = compile_runtime(&runtime_spec, &ess.ir, &realization, &build)
            .context("generated ESS runtime was refused")?;

        let component_yaml = component_yaml(service, package, release);
        let component_spec = ComponentSpec::from_yaml(&component_yaml)
            .context("generated ESS component description does not parse")?;
        let component =
            compile_component(&component_spec).context("generated ESS component was refused")?;

        let mut files = BTreeMap::from([
            ("deployment/component.yaml".to_owned(), component_yaml),
            (
                "deployment/component.ir.json".to_owned(),
                component.to_canonical_json(),
            ),
            ("deployment/build.yaml".to_owned(), build_yaml),
            (
                "deployment/build.ir.json".to_owned(),
                build.to_canonical_json(),
            ),
            (
                "deployment/build.mmd".to_owned(),
                project_build_mermaid(&build),
            ),
            ("deployment/realization.yaml".to_owned(), realization_yaml),
            (
                "deployment/realization.ir.json".to_owned(),
                realization.to_canonical_json(),
            ),
            ("deployment/runtime.yaml".to_owned(), runtime_yaml),
            (
                "deployment/runtime.ir.json".to_owned(),
                runtime.to_canonical_json(),
            ),
        ]);
        for (path, contents) in project_buildkit(&build).files() {
            files.insert(format!("deployment/buildkit/{path}"), contents.clone());
        }
        let chart_name = format!("{service}-chart")
            .parse()
            .expect("validated service names are ESS identifiers");
        let chart_version = release
            .version
            .parse()
            .expect("release versions are validated while reading the package");
        for (path, contents) in project_helm(&runtime, &chart_name, &chart_version).files() {
            files.insert(format!("deployment/chart/{path}"), contents.clone());
        }
        Ok(Some(Self { files }))
    }
}

fn realization_yaml(
    service: &str,
    ess: &EssBuild,
    generated: &BTreeMap<String, String>,
    components: &[String],
) -> Result<String> {
    let source_identity = generated_source_identity(generated);
    let mut surfaces = ess
        .ir
        .commands()
        .keys()
        .map(|name| json!({"kind": "command", "name": name.to_string()}))
        .chain(
            ess.ir
                .views()
                .keys()
                .map(|name| json!({"kind": "view", "name": name.to_string()})),
        )
        .collect::<Vec<_>>();
    surfaces.sort_by_key(Value::to_string);
    render_yaml(&json!({
        "type": "ess-realization/1",
        "id": format!("{service}-generated"),
        "specification": {
            "system": ess.ir.system().to_string(),
            "version": ess.ir.version().to_string(),
            "source_digest": format!("sha256:{}", ess.ir.source_digest()),
        },
        "synthesis": {
            "target": "rust-linux-amd64/1",
            "generator": "service-builder/1",
        },
        "components": components,
        "actors": [],
        "implementations": [{
            "id": format!("{service}-binary"),
            "components": components,
            "artifact": {
                "kind": "source",
                "locator": "generated/rust",
                "identity": source_identity,
            },
        }],
        "entrypoints": [{
            "id": "http-api",
            "title": format!("{service} HTTP API"),
            "summary": format!("Invoke the generated {service} service."),
            "primary": true,
            "interaction": "invoke",
            "attachment": "network",
            "availability": "internal",
            "support": "preview",
            "implementation": format!("{service}-binary"),
            "actors": [],
            "surfaces": surfaces,
            "invocation": {"kind": "url", "url": "http://127.0.0.1:8080"},
            "requires": [
                {"kind": "environment_variable", "name": environment_name(service, "IDENTITY_ORIGIN"), "summary": "Identity service origin."},
                {"kind": "filesystem", "name": format!("/var/lib/{service}"), "summary": "Durable Eventlog storage."},
            ],
        }],
    }))
}

fn build_yaml(service: &str, release: &ReleasePackage) -> String {
    let crate_name = format!("{service}-generated-service");
    let chart_path = format!("{}/deployment/chart", release.generated_root);
    let chart_archive = format!("/tmp/{service}-chart.tgz");
    let executable = format!("/build/target/release/{crate_name}");
    let mut nodes = vec![
        json!({"id": "build-base", "kind": "oci_base", "reference": release.build_base.repository, "digest": release.build_base.digest}),
        json!({"id": "repository", "kind": "source", "path": ".", "destination": "/src"}),
        json!({"id": "build-source", "kind": "copy", "base": "build-base", "from": "repository", "source": "/src", "destination": "/src"}),
        json!({
            "id": "compile",
            "kind": "run",
            "base": "build-source",
            "argv": ["cargo", "build", "--release", "--manifest-path", format!("/src/{}/rust/Cargo.toml", release.generated_root)],
            "workdir": "/src",
            "environment": {"CARGO_TARGET_DIR": "/build/target"},
            "network": "sandbox",
        }),
    ];
    let runtime_root = if let Some(base) = &release.runtime_base {
        nodes.push(json!({"id": "runtime-base", "kind": "oci_base", "reference": base.repository, "digest": base.digest}));
        nodes.push(json!({
            "id": "runtime-root",
            "kind": "copy",
            "base": "runtime-base",
            "from": "compile",
            "source": executable,
            "destination": format!("/usr/local/bin/{crate_name}"),
        }));
        "runtime-root"
    } else {
        "compile"
    };
    let entrypoint = if release.runtime_base.is_some() {
        format!("/usr/local/bin/{crate_name}")
    } else {
        executable
    };
    nodes.extend([
        json!({
            "id": "runtime-image",
            "kind": "image",
            "rootfs": runtime_root,
            "config": {"entrypoint": [entrypoint], "workdir": "/var/lib/service"},
        }),
        json!({
            "id": "chart-package",
            "kind": "run",
            "base": "build-source",
            "argv": ["tar", "--sort=name", "--mtime=@0", "--owner=0", "--group=0", "-czf", chart_archive, "-C", chart_path, "."],
        }),
        json!({"id": "chart-archive", "kind": "artifact", "from": "chart-package", "path": chart_archive}),
    ]);
    render_yaml(&json!({
        "format": "ess-build/1",
        "build": format!("{service}-build"),
        "platforms": [{"os": "linux", "architecture": "amd64"}],
        "nodes": nodes,
        "outputs": [
            {"name": "app", "release_unit": format!("{service}-runtime"), "node": "runtime-image", "kind": "oci_image", "repository": release.image_repository},
            {"name": "chart", "release_unit": format!("{service}-chart"), "node": "chart-archive", "kind": "helm_chart"},
        ],
    }))
    .expect("generated build documents always serialize")
}

#[allow(clippy::too_many_arguments)]
fn runtime_yaml(
    service: &str,
    release: &ReleasePackage,
    ess: &EssBuild,
    plan: &ServicePlan,
    realization: &ess_realization::RealizationIr,
    build: &ess_deployment::BuildIr,
    components: &[String],
) -> Result<String> {
    let PlanDelivery::IdentityHttp { audience } = &plan.delivery else {
        bail!("independent release generation currently requires Identity HTTP delivery");
    };
    render_yaml(&json!({
        "format": "ess-runtime/1",
        "runtime": format!("{service}-runtime"),
        "semantic_digest": format!("sha256:{}", ess.ir.source_digest()),
        "realization_digest": realization.realization_digest().to_string(),
        "build_digest": build.digest().to_string(),
        "processes": [{"name": "server", "image": "app"}],
        "containers": [{
            "name": "server",
            "process": "server",
            "http_port": 8080,
            "readiness_path": "/readyz",
            "liveness_path": "/healthz",
            "config": [{
                "name": "database-path",
                "environment": environment_name(service, "DATABASE_PATH"),
                "kind": "literal",
                "value": format!("/var/lib/{service}/{service}.sqlite3"),
            }],
            "endpoints": [{
                "name": "identity",
                "environment": environment_name(service, "IDENTITY_ORIGIN"),
                "system": "identity",
                "endpoint": "api",
            }],
            "volume_mounts": [{"volume": "data", "mount_path": format!("/var/lib/{service}")}],
            "audiences": [audience],
        }],
        "workloads": [{
            "name": service,
            "components": components,
            "containers": ["server"],
            "replicas": 1,
            "volumes": [{"name": "data", "size": release.storage_size}],
        }],
        "provided_endpoints": [{"name": "api", "workload": service, "container": "server", "scheme": "http"}],
    }))
}

fn component_yaml(service: &str, package: &ServicePackage, release: &ReleasePackage) -> String {
    render_yaml(&json!({
        "format": "ess-component/1",
        "component": service,
        "system": service,
        "semantic_version": package.sources.compile().expect("package ESS already compiled").version().to_string(),
        "inputs": {
            "specification": package.manifest.semantic.root,
            "realization": format!("{}/deployment/realization.yaml", release.generated_root),
            "build": format!("{}/deployment/build.yaml", release.generated_root),
            "runtime": format!("{}/deployment/runtime.yaml", release.generated_root),
        },
        "release_units": {"runtime": format!("{service}-runtime"), "chart": format!("{service}-chart")},
    }))
    .expect("generated component documents always serialize")
}

fn generated_source_identity(generated: &BTreeMap<String, String>) -> String {
    let mut digest = Sha256::new();
    for (path, contents) in generated
        .iter()
        .filter(|(path, _)| path.starts_with("rust/"))
    {
        digest.update((path.len() as u64).to_be_bytes());
        digest.update(path.as_bytes());
        digest.update((contents.len() as u64).to_be_bytes());
        digest.update(contents.as_bytes());
    }
    format!("sha256:{}", hex::encode(digest.finalize()))
}

fn environment_name(service: &str, suffix: &str) -> String {
    let prefix = service
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("{prefix}_{suffix}")
}

fn render_yaml(value: &Value) -> Result<String> {
    serde_yaml::to_string(value).context("serializing generated ESS YAML")
}
