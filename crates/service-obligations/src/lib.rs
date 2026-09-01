//! Closed, versioned obligation implementations supplied by the SDK.
//!
//! A service definition selects entries from this catalog and binds their typed parameters to its
//! ESS model. Application repositories never implement an obligation. Unknown providers,
//! incomplete bindings, incompatible operation coverage, and unused declarations are refused
//! before runtime IR or Rust can be generated.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use service_definition::{
    DefinitionId, EventBindingSource, ObligationDefinition, ObligationProviderId, ServiceDefinition,
};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

/// Where one catalog implementation may be applied.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationSurface {
    /// Authenticated mutation execution.
    Intent,
    /// Materialized projection updates.
    Projection,
    /// Authenticated projection reads.
    Query,
    /// Derivation of an otherwise undetermined event field.
    EventBinding,
}

/// Static contract of one SDK-provided obligation implementation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CatalogEntry {
    /// Exact versioned provider identity.
    pub provider: &'static str,
    /// Bindings every use must supply.
    pub required_bindings: &'static [&'static str],
    /// Additional reviewed bindings accepted by the provider.
    pub optional_bindings: &'static [&'static str],
    /// Runtime surfaces this implementation can cover.
    pub surfaces: &'static [ObligationSurface],
}

const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        provider: "sdk.aggregate.event-sourced/v1",
        required_bindings: &["aggregate", "identity"],
        optional_bindings: &["category"],
        surfaces: &[ObligationSurface::Intent],
    },
    CatalogEntry {
        provider: "sdk.auth.owner-and-conjunctive-scopes/v1",
        required_bindings: &["owner", "scopes"],
        optional_bindings: &["requested_scopes"],
        surfaces: &[ObligationSurface::Intent, ObligationSurface::Query],
    },
    CatalogEntry {
        provider: "sdk.auth.requested-scopes/v1",
        required_bindings: &["scopes"],
        optional_bindings: &[],
        surfaces: &[ObligationSurface::Intent],
    },
    CatalogEntry {
        provider: "sdk.auth.same-partition-owner-transfer/v1",
        required_bindings: &["owner"],
        optional_bindings: &["new_owner"],
        surfaces: &[ObligationSurface::Intent],
    },
    CatalogEntry {
        provider: "sdk.auth.trusted-scheduler/v1",
        required_bindings: &["capability"],
        optional_bindings: &[],
        surfaces: &[ObligationSurface::Intent],
    },
    CatalogEntry {
        provider: "sdk.lifecycle.expiring-parent-child/v1",
        required_bindings: &["parent", "parent_lifetime", "child_lifetime"],
        optional_bindings: &["parent_state", "child_state"],
        surfaces: &[ObligationSurface::Intent],
    },
    CatalogEntry {
        provider: "sdk.lifecycle.bounded-future/v1",
        required_bindings: &["lifetime"],
        optional_bindings: &[],
        surfaces: &[ObligationSurface::Intent],
    },
    CatalogEntry {
        provider: "sdk.lifecycle.expiry-due/v1",
        required_bindings: &["entity", "lifetime"],
        optional_bindings: &["identity"],
        surfaces: &[ObligationSurface::Intent],
    },
    CatalogEntry {
        provider: "sdk.lifecycle.require-state/v1",
        required_bindings: &["entity", "allowed"],
        optional_bindings: &["identity"],
        surfaces: &[ObligationSurface::Intent],
    },
    CatalogEntry {
        provider: "sdk.aggregate.nested-entity/v1",
        required_bindings: &["parent", "child", "parent_identity"],
        optional_bindings: &["child_identity"],
        surfaces: &[ObligationSurface::Intent],
    },
    CatalogEntry {
        provider: "sdk.derive.inherit-parent-authority/v1",
        required_bindings: &[
            "parent_owner",
            "parent_scopes",
            "child_owner",
            "child_scopes",
        ],
        optional_bindings: &["parent", "child"],
        surfaces: &[
            ObligationSurface::Intent,
            ObligationSurface::Projection,
            ObligationSurface::EventBinding,
        ],
    },
    CatalogEntry {
        provider: "sdk.content.external-erasable/v1",
        required_bindings: &["content"],
        optional_bindings: &[],
        surfaces: &[ObligationSurface::Intent],
    },
    CatalogEntry {
        provider: "sdk.projection.auth-partitioned-visibility/v1",
        required_bindings: &["owner", "scopes"],
        optional_bindings: &["parent", "terminal_states"],
        surfaces: &[ObligationSurface::Projection, ObligationSurface::Query],
    },
    CatalogEntry {
        provider: "sdk.projection.hide-terminal-parent/v1",
        required_bindings: &["parent", "parent_identity"],
        optional_bindings: &["terminal_states"],
        surfaces: &[ObligationSurface::Projection, ObligationSurface::Query],
    },
];

/// Returns the complete built-in catalog in canonical provider order.
pub fn catalog() -> &'static [CatalogEntry] {
    CATALOG
}

/// Returns the exact implementation contract for a provider identity.
pub fn resolve(provider: &ObligationProviderId) -> Option<&'static CatalogEntry> {
    CATALOG
        .iter()
        .find(|entry| entry.provider == provider.as_str())
}

/// Digest binding generated runtime IR to the exact catalog semantics.
pub fn catalog_digest() -> String {
    use std::fmt::Write as _;

    let bytes = serde_json::to_vec(CATALOG).expect("the static obligation catalog serializes");
    let hash = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in hash {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

/// A completely resolved obligation use embedded in runtime IR.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedObligation {
    /// Definition-local identity referenced by operations.
    pub name: DefinitionId,
    /// Exact SDK implementation identity.
    pub provider: ObligationProviderId,
    /// Complete provider-validated bindings.
    pub bindings: BTreeMap<DefinitionId, String>,
}

/// Stable category of obligation compilation refusal.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationCode {
    /// The definition selected no SDK implementation.
    #[error("unknown SDK obligation provider")]
    UnknownProvider,
    /// A provider-required parameter is absent.
    #[error("required obligation binding is absent")]
    MissingBinding,
    /// A binding is not part of the provider contract.
    #[error("obligation binding is not accepted by its provider")]
    UnexpectedBinding,
    /// An operation used an obligation on an unsupported surface.
    #[error("obligation provider does not cover this runtime surface")]
    WrongSurface,
    /// A declaration is never referenced by an operation or derivation.
    #[error("obligation declaration is unused")]
    Unused,
    /// An operation has no SDK-provided implementation covering its required surface.
    #[error("runtime surface has no SDK-provided obligation")]
    Uncovered,
    /// Content staging lacks the SDK content lifecycle implementation.
    #[error("content binding has no SDK content obligation")]
    ContentUncovered,
    /// Event derivation names an obligation that cannot derive event fields.
    #[error("event derivation obligation has no derivation implementation")]
    EventBindingUncovered,
}

/// One repair-oriented catalog diagnostic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ObligationDiagnostic {
    /// Stable refusal category.
    pub code: ObligationCode,
    /// Definition-local location.
    pub path: String,
    /// Concrete repair guidance.
    pub message: String,
}

/// All obligation compilation refusals from one service definition.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("service obligation compilation was refused")]
pub struct ObligationDiagnostics(Vec<ObligationDiagnostic>);

impl ObligationDiagnostics {
    /// Every diagnostic in deterministic validation order.
    pub fn diagnostics(&self) -> &[ObligationDiagnostic] {
        &self.0
    }
}

/// Resolves and validates every obligation and proves complete runtime-surface coverage.
#[allow(clippy::too_many_lines)]
pub fn compile(
    definition: &ServiceDefinition,
) -> Result<BTreeMap<DefinitionId, ResolvedObligation>, ObligationDiagnostics> {
    let declarations = definition
        .obligations
        .iter()
        .map(|obligation| (obligation.name.clone(), obligation))
        .collect::<BTreeMap<_, _>>();
    let mut diagnostics = Vec::new();
    let mut resolved = BTreeMap::new();

    for (index, obligation) in definition.obligations.iter().enumerate() {
        let Some(entry) = resolve(&obligation.provider) else {
            diagnostics.push(diagnostic(
                ObligationCode::UnknownProvider,
                format!("obligations[{index}].provider"),
                format!(
                    "{} is not supplied by this SDK catalog",
                    obligation.provider
                ),
            ));
            continue;
        };
        let required = entry
            .required_bindings
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let accepted = required
            .iter()
            .copied()
            .chain(entry.optional_bindings.iter().copied())
            .collect::<BTreeSet<_>>();
        for missing in required.iter().filter(|name| {
            !obligation
                .bindings
                .keys()
                .any(|binding| binding.as_str() == **name)
        }) {
            diagnostics.push(diagnostic(
                ObligationCode::MissingBinding,
                format!("obligations[{index}].bindings.{missing}"),
                format!("{} requires binding {missing:?}", obligation.provider),
            ));
        }
        for binding in obligation
            .bindings
            .keys()
            .filter(|binding| !accepted.contains(binding.as_str()))
        {
            diagnostics.push(diagnostic(
                ObligationCode::UnexpectedBinding,
                format!("obligations[{index}].bindings.{binding}"),
                format!(
                    "{} does not accept binding {binding:?}",
                    obligation.provider
                ),
            ));
        }
        resolved.insert(
            obligation.name.clone(),
            ResolvedObligation {
                name: obligation.name.clone(),
                provider: obligation.provider.clone(),
                bindings: obligation.bindings.clone(),
            },
        );
    }

    let mut used = BTreeSet::new();
    for (index, intent) in definition.intents.iter().enumerate() {
        validate_surface(
            &declarations,
            &intent.obligations,
            ObligationSurface::Intent,
            &format!("intents[{index}].obligations"),
            &mut used,
            &mut diagnostics,
        );
        if !contains_provider(
            &declarations,
            &intent.obligations,
            "sdk.aggregate.event-sourced/v1",
        ) {
            diagnostics.push(diagnostic(
                ObligationCode::Uncovered,
                format!("intents[{index}].obligations"),
                "every mutation requires sdk.aggregate.event-sourced/v1",
            ));
        }
        if !intent.content.is_empty()
            && !contains_provider(
                &declarations,
                &intent.obligations,
                "sdk.content.external-erasable/v1",
            )
        {
            diagnostics.push(diagnostic(
                ObligationCode::ContentUncovered,
                format!("intents[{index}].content"),
                "plaintext staging requires sdk.content.external-erasable/v1",
            ));
        }
        for (binding_index, binding) in intent.event_bindings.iter().enumerate() {
            if let EventBindingSource::Obligation { name } = &binding.source {
                used.insert(name.clone());
                let covered = declarations.get(name).is_some_and(|obligation| {
                    resolve(&obligation.provider).is_some_and(|entry| {
                        entry.surfaces.contains(&ObligationSurface::EventBinding)
                    })
                });
                if !covered {
                    diagnostics.push(diagnostic(
                        ObligationCode::EventBindingUncovered,
                        format!("intents[{index}].event_bindings[{binding_index}].source"),
                        format!("{name} cannot derive an event field"),
                    ));
                }
            }
        }
    }
    for (index, projection) in definition.projections.iter().enumerate() {
        validate_surface(
            &declarations,
            &projection.obligations,
            ObligationSurface::Projection,
            &format!("projections[{index}].obligations"),
            &mut used,
            &mut diagnostics,
        );
    }
    for (index, query) in definition.queries.iter().enumerate() {
        validate_surface(
            &declarations,
            &query.obligations,
            ObligationSurface::Query,
            &format!("queries[{index}].obligations"),
            &mut used,
            &mut diagnostics,
        );
    }
    for (index, obligation) in definition.obligations.iter().enumerate() {
        if !used.contains(&obligation.name) {
            diagnostics.push(diagnostic(
                ObligationCode::Unused,
                format!("obligations[{index}]"),
                format!("{} is never applied", obligation.name),
            ));
        }
    }

    if diagnostics.is_empty() {
        Ok(resolved)
    } else {
        Err(ObligationDiagnostics(diagnostics))
    }
}

fn validate_surface(
    declarations: &BTreeMap<DefinitionId, &ObligationDefinition>,
    references: &[DefinitionId],
    surface: ObligationSurface,
    path: &str,
    used: &mut BTreeSet<DefinitionId>,
    diagnostics: &mut Vec<ObligationDiagnostic>,
) {
    if references.is_empty() {
        diagnostics.push(diagnostic(
            ObligationCode::Uncovered,
            path,
            format!("{surface:?} has no SDK-provided obligation"),
        ));
        return;
    }
    for (index, reference) in references.iter().enumerate() {
        used.insert(reference.clone());
        let supported = declarations.get(reference).is_some_and(|obligation| {
            resolve(&obligation.provider).is_some_and(|entry| entry.surfaces.contains(&surface))
        });
        if !supported {
            diagnostics.push(diagnostic(
                ObligationCode::WrongSurface,
                format!("{path}[{index}]"),
                format!("{reference} does not provide {surface:?} behavior"),
            ));
        }
    }
}

fn contains_provider(
    declarations: &BTreeMap<DefinitionId, &ObligationDefinition>,
    references: &[DefinitionId],
    provider: &str,
) -> bool {
    references.iter().any(|reference| {
        declarations
            .get(reference)
            .is_some_and(|obligation| obligation.provider.as_str() == provider)
    })
}

fn diagnostic(
    code: ObligationCode,
    path: impl Into<String>,
    message: impl Into<String>,
) -> ObligationDiagnostic {
    ObligationDiagnostic {
        code,
        path: path.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use service_definition::ServiceDefinition;

    use super::*;

    const COMPLETE: &str = r"
format: service-definition/2
service: notes
realm: optional
obligations:
  - name: aggregate
    provider: sdk.aggregate.event-sourced/v1
    bindings: { aggregate: notes.note.Note, identity: note_id }
    description: Persist Note as one guarded aggregate.
  - name: visibility
    provider: sdk.projection.auth-partitioned-visibility/v1
    bindings: { owner: owner, scopes: scopes }
    description: Partition and filter the read model by verified authentication.
projections:
  - name: note_by_id
    view: notes.note.NoteById
    delivery: inline_transactional
    obligations: [visibility]
intents:
  - name: create_note
    command: notes.note.CreateNote
    stream_id: { kind: generated_uuid_v7 }
    expected_version: { kind: no_stream }
    idempotency: { kind: operation_field, field: idempotency_key }
    projections: [note_by_id]
    obligations: [aggregate]
queries:
  - name: get_note
    view: notes.note.NoteById
    projection: note_by_id
    selectors: [{ parameter: note_id, view_field: note_id }]
    sort: [{ view_field: note_id, direction: ascending }]
    delivery: read_your_writes
    obligations: [visibility]
";

    #[test]
    fn complete_catalog_binding_resolves_and_has_stable_digest() {
        let definition = ServiceDefinition::from_yaml(COMPLETE).unwrap();
        let resolved = compile(&definition).unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(catalog_digest().len(), 64);
        assert_eq!(catalog_digest(), catalog_digest());
    }

    #[test]
    fn unknown_missing_and_uncovered_behavior_is_refused_together() {
        let invalid = COMPLETE
            .replace("sdk.aggregate.event-sourced/v1", "sdk.unknown/v1")
            .replace(
                "bindings: { owner: owner, scopes: scopes }",
                "bindings: { owner: owner }",
            )
            .replace("obligations: [visibility]", "obligations: []");
        let definition = ServiceDefinition::from_yaml(&invalid).unwrap();
        let errors = compile(&definition).unwrap_err();
        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|item| item.code == ObligationCode::UnknownProvider)
        );
        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|item| item.code == ObligationCode::MissingBinding)
        );
        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|item| item.code == ObligationCode::Uncovered)
        );
    }
}
