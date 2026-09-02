//! Transactional loading of one modular `service/1` package.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use service_definition::ServiceDefinition;

use crate::client::{ClientOperationKind, ClientPlan};
use crate::ess::EssSources;

/// The only service-package format understood by the builder.
pub const SERVICE_PACKAGE_FORMAT: &str = "service/1";

/// Exact SDK source lock emitted into generated Rust dependencies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SdkLock {
    /// Canonical Git repository URL.
    pub repository: String,
    /// Exact lowercase Git commit object name.
    pub revision: String,
}

/// ESS fragment inventory relative to one package directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticPackage {
    /// Directory beneath the package containing the ESS source tree.
    pub root: String,
    /// Explicit ESS files relative to `root`; one must be `system.yaml`.
    pub sources: Vec<String>,
}

/// Human-authored, transactional service package manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServicePackageManifest {
    /// Format discriminator.
    pub format: String,
    /// Stable service identity, checked against the runtime definition and ESS system.
    pub service: String,
    /// Exact SDK revision that generated code consumes.
    pub sdk: SdkLock,
    /// Modular ESS source inventory.
    pub semantic: SemanticPackage,
    /// Runtime-definition document relative to the package directory.
    pub runtime: String,
    /// Declarative scenario documents relative to the package directory.
    #[serde(default)]
    pub scenarios: Vec<String>,
}

/// Complete package input loaded before any output is produced.
pub struct ServicePackage {
    /// Validated human-authored manifest.
    pub manifest: ServicePackageManifest,
    /// Exact ESS fragment set.
    pub sources: EssSources,
    /// Strict runtime definition.
    pub definition: ServiceDefinition,
    /// Validated scenario source bytes in manifest order.
    pub(crate) scenarios: Vec<ScenarioSource>,
}

#[derive(Debug)]
pub(crate) struct ScenarioSource {
    pub(crate) path: String,
    pub(crate) contents: String,
    document: ScenarioDocument,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioDocument {
    format: String,
    service: String,
    scenarios: Vec<ScenarioCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioCase {
    name: String,
    given: ScenarioGiven,
    #[serde(default)]
    when: Vec<ScenarioIntent>,
    then: Vec<ScenarioAssertion>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioGiven {
    auth: ScenarioAuth,
    #[serde(default)]
    other_auth: Option<ScenarioAuth>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioAuth {
    tenant: String,
    realm: Option<String>,
    authority: String,
    user: String,
    #[serde(default)]
    executor: Option<String>,
    #[serde(default)]
    facts: ScenarioFacts,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioFacts {
    #[serde(default)]
    principals: BTreeSet<String>,
    #[serde(default)]
    teams: BTreeSet<String>,
    #[serde(default)]
    projects: BTreeSet<String>,
    #[serde(default)]
    extensions: BTreeSet<String>,
    #[serde(default)]
    capabilities: BTreeSet<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioIntent {
    intent: String,
    input: BTreeMap<String, Value>,
    #[serde(default = "default_auth_fixture")]
    using: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ScenarioAssertion {
    Query(ScenarioQuery),
    Partitions(ScenarioPartitions),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioQuery {
    query: String,
    input: BTreeMap<String, Value>,
    count: usize,
    #[serde(default = "default_auth_fixture")]
    using: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioPartitions {
    partitions_are_distinct: [String; 2],
}

fn default_auth_fixture() -> String {
    "auth".to_owned()
}

impl ServicePackage {
    /// Reads every package input and validates cross-document identity atomically.
    pub fn read(manifest_path: &Path) -> Result<Self> {
        let text = fs::read_to_string(manifest_path)
            .with_context(|| format!("reading service package {}", manifest_path.display()))?;
        let manifest: ServicePackageManifest =
            match manifest_path.extension().and_then(std::ffi::OsStr::to_str) {
                Some("yaml" | "yml") => serde_yaml::from_str(&text).with_context(|| {
                    format!("parsing YAML service package {}", manifest_path.display())
                })?,
                Some("json") => serde_json::from_str(&text).with_context(|| {
                    format!("parsing JSON service package {}", manifest_path.display())
                })?,
                _ => bail!("service package manifest must be YAML or JSON"),
            };
        validate_manifest(&manifest)?;

        let package_root = manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()
            .with_context(|| format!("resolving package root for {}", manifest_path.display()))?;
        let semantic_root = resolve_directory(&package_root, &manifest.semantic.root)?;
        let mut source_files = BTreeMap::new();
        for source in &manifest.semantic.sources {
            let path = resolve_file(&semantic_root, source)?;
            source_files.insert(
                source.clone(),
                fs::read_to_string(&path)
                    .with_context(|| format!("reading ESS fragment {}", path.display()))?,
            );
        }
        let sources = EssSources::new(source_files)?;

        let runtime_path = resolve_file(&package_root, &manifest.runtime)?;
        let runtime_text = fs::read_to_string(&runtime_path)
            .with_context(|| format!("reading runtime definition {}", runtime_path.display()))?;
        let definition = match runtime_path.extension().and_then(std::ffi::OsStr::to_str) {
            Some("yaml" | "yml") => ServiceDefinition::from_yaml(&runtime_text),
            Some("json") => ServiceDefinition::from_json(&runtime_text),
            _ => bail!("runtime definition must be YAML or JSON"),
        }
        .with_context(|| format!("validating runtime definition {}", runtime_path.display()))?;

        let ir = sources.compile()?;
        if definition.service.as_str() != manifest.service {
            bail!(
                "package service {:?} differs from runtime service {:?}",
                manifest.service,
                definition.service.as_str()
            );
        }
        if ir.system().to_string() != manifest.service {
            bail!(
                "package service {:?} differs from ESS system {:?}",
                manifest.service,
                ir.system().to_string()
            );
        }

        let mut scenarios = Vec::with_capacity(manifest.scenarios.len());
        for scenario in &manifest.scenarios {
            let path = resolve_file(&package_root, scenario)?;
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("reading scenario {}", path.display()))?;
            let document: ScenarioDocument = serde_yaml::from_str(&contents)
                .with_context(|| format!("parsing scenario {}", path.display()))?;
            document.validate_header(&manifest.service, scenario)?;
            scenarios.push(ScenarioSource {
                path: scenario.clone(),
                contents,
                document,
            });
        }

        Ok(Self {
            manifest,
            sources,
            definition,
            scenarios,
        })
    }

    /// Validates every declarative scenario against the generated public operation surface.
    pub(crate) fn validate_scenarios(&self, client: &ClientPlan) -> Result<()> {
        for source in &self.scenarios {
            source.document.validate_operations(client, &source.path)?;
        }
        Ok(())
    }
}

impl ScenarioDocument {
    fn validate_header(&self, service: &str, path: &str) -> Result<()> {
        if self.format != "service-scenarios/1" {
            bail!("scenario {path:?} has unsupported format {:?}", self.format);
        }
        if self.service != service {
            bail!(
                "scenario {path:?} service {:?} differs from package service {service:?}",
                self.service
            );
        }
        if self.scenarios.is_empty() {
            bail!("scenario {path:?} must contain at least one case");
        }
        let mut names = BTreeSet::new();
        for scenario in &self.scenarios {
            if scenario.name.trim().is_empty() || !names.insert(&scenario.name) {
                bail!("scenario {path:?} has an empty or duplicate case name");
            }
            validate_auth(&scenario.given.auth, path, &scenario.name)?;
            if let Some(auth) = &scenario.given.other_auth {
                validate_auth(auth, path, &scenario.name)?;
            }
            if scenario.then.is_empty() {
                bail!(
                    "scenario {path:?} case {:?} has no assertions",
                    scenario.name
                );
            }
        }
        Ok(())
    }

    fn validate_operations(&self, client: &ClientPlan, path: &str) -> Result<()> {
        for scenario in &self.scenarios {
            for intent in &scenario.when {
                validate_auth_fixture(&scenario.given, path, &scenario.name, &intent.using)?;
                validate_operation(
                    client,
                    path,
                    &scenario.name,
                    &intent.intent,
                    ClientOperationKind::Intent,
                    &intent.input,
                )?;
            }
            for assertion in &scenario.then {
                match assertion {
                    ScenarioAssertion::Query(query) => {
                        validate_auth_fixture(&scenario.given, path, &scenario.name, &query.using)?;
                        validate_operation(
                            client,
                            path,
                            &scenario.name,
                            &query.query,
                            ClientOperationKind::Query,
                            &query.input,
                        )?;
                        let _ = query.count;
                    }
                    ScenarioAssertion::Partitions(partitions) => {
                        let [left, right] = &partitions.partitions_are_distinct;
                        if left == right
                            || !auth_fixture_exists(&scenario.given, left)
                            || !auth_fixture_exists(&scenario.given, right)
                        {
                            bail!(
                                "scenario {path:?} case {:?} names invalid partition fixtures",
                                scenario.name
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn validate_auth(auth: &ScenarioAuth, path: &str, scenario: &str) -> Result<()> {
    if [&auth.tenant, &auth.authority, &auth.user]
        .into_iter()
        .any(|value| value.trim().is_empty())
        || auth
            .realm
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        || auth
            .executor
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
    {
        bail!("scenario {path:?} case {scenario:?} has an empty authentication coordinate");
    }
    if auth
        .facts
        .principals
        .iter()
        .chain(&auth.facts.teams)
        .chain(&auth.facts.projects)
        .chain(&auth.facts.extensions)
        .chain(&auth.facts.capabilities)
        .any(|value| value.trim().is_empty())
    {
        bail!("scenario {path:?} case {scenario:?} has an empty authority fact");
    }
    Ok(())
}

fn validate_auth_fixture(
    given: &ScenarioGiven,
    path: &str,
    scenario: &str,
    fixture: &str,
) -> Result<()> {
    if auth_fixture_exists(given, fixture) {
        Ok(())
    } else {
        bail!("scenario {path:?} case {scenario:?} names unknown auth fixture {fixture:?}")
    }
}

fn auth_fixture_exists(given: &ScenarioGiven, name: &str) -> bool {
    name == "auth" || (name == "other_auth" && given.other_auth.is_some())
}

fn validate_operation(
    client: &ClientPlan,
    path: &str,
    scenario: &str,
    name: &str,
    kind: ClientOperationKind,
    input: &BTreeMap<String, Value>,
) -> Result<()> {
    let operation = client
        .operations
        .iter()
        .find(|candidate| candidate.operation == name && candidate.kind == kind)
        .ok_or_else(|| {
            anyhow::anyhow!("scenario {path:?} case {scenario:?} names unknown {kind:?} {name:?}")
        })?;
    let declared = operation
        .inputs
        .iter()
        .map(|field| field.name.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(field) = input
        .keys()
        .find(|field| !declared.contains(field.as_str()))
    {
        bail!(
            "scenario {path:?} case {scenario:?} supplies undeclared input {field:?} to {name:?}"
        );
    }
    if let Some(field) = operation
        .inputs
        .iter()
        .find(|field| !field.optional && !input.contains_key(&field.name))
    {
        bail!(
            "scenario {path:?} case {scenario:?} omits required input {:?} from {name:?}",
            field.name
        );
    }
    Ok(())
}

fn validate_manifest(manifest: &ServicePackageManifest) -> Result<()> {
    if manifest.format != SERVICE_PACKAGE_FORMAT {
        bail!(
            "unsupported service package format {:?}; expected {SERVICE_PACKAGE_FORMAT:?}",
            manifest.format
        );
    }
    if manifest.service.is_empty()
        || !manifest
            .service
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        bail!("package service must be a lowercase stable identifier");
    }
    if !manifest.sdk.repository.starts_with("https://")
        || !Path::new(&manifest.sdk.repository)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("git"))
    {
        bail!("sdk.repository must be a canonical HTTPS Git URL ending in .git");
    }
    if manifest.sdk.revision.len() != 40
        || !manifest
            .sdk
            .revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("sdk.revision must be an exact lowercase 40-character Git object name");
    }
    validate_relative(&manifest.semantic.root)?;
    validate_relative(&manifest.runtime)?;
    if manifest.semantic.sources.is_empty() {
        bail!("semantic.sources must not be empty");
    }
    if !manifest
        .semantic
        .sources
        .iter()
        .any(|source| source == "system.yaml")
    {
        bail!("semantic.sources must explicitly contain system.yaml");
    }
    let mut paths = BTreeSet::new();
    for path in manifest
        .semantic
        .sources
        .iter()
        .chain(manifest.scenarios.iter())
    {
        validate_relative(path)?;
        if !paths.insert(path) {
            bail!("package path {path:?} is declared more than once");
        }
    }
    Ok(())
}

fn validate_relative(value: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("package path {value:?} is not a safe relative path");
    }
    Ok(())
}

fn resolve_directory(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_relative(relative)?;
    let path = root.join(relative).canonicalize().with_context(|| {
        format!(
            "resolving package directory {}",
            root.join(relative).display()
        )
    })?;
    if !path.starts_with(root) || !path.is_dir() {
        bail!("package directory {relative:?} escapes the package or is not a directory");
    }
    Ok(path)
}

fn resolve_file(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_relative(relative)?;
    let path = root
        .join(relative)
        .canonicalize()
        .with_context(|| format!("resolving package file {}", root.join(relative).display()))?;
    if !path.starts_with(root) || !path.is_file() {
        bail!("package file {relative:?} escapes the package or is not a regular file");
    }
    Ok(path)
}
