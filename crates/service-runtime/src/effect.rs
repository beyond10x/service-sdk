//! Restart-safe external-effect contracts.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{BoxFuture, VerifiedAuthContext};

/// Stable exact-plan contract.
pub const EFFECT_PLAN_FORMAT: &str = "service-effect-plan/1";

/// Ordered consequence risk shared with Connector and Harness policy.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectRisk {
    /// Wrong is cheap and visible.
    Low,
    /// Wrong costs work to undo.
    Medium,
    /// Wrong has an external or human-visible cost.
    High,
    /// Wrong cannot be undone.
    Destructive,
}

/// Exact externally dispatched operation derived during preview.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectPlan {
    /// Contract discriminator.
    pub format: String,
    /// Stable generated service identity.
    pub service: String,
    /// Semantic intent being realized.
    pub operation: String,
    /// SHA-256 of normalized caller input.
    pub input_digest: String,
    /// Optional erasable content reference; plaintext is never part of the plan.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_reference: Option<String>,
    /// Digest of implementation bindings selected outside caller input.
    pub binding_digest: String,
    /// Aggregate version against which preview ran.
    pub aggregate_version: u64,
    /// Current downstream resource revision or manifest digest.
    pub resource_revision: String,
    /// Opaque single-operation downstream authority reference.
    pub authority_reference: String,
    /// Grant that admitted delegated execution, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_reference: Option<String>,
    /// Exact evaluated grant revision, present with `grant_reference`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_revision: Option<u64>,
    /// Declared risk.
    pub risk: EffectRisk,
    /// Closed semantic consequences, such as `write_file` or `human_visible`.
    pub consequences: BTreeSet<String>,
}

impl EffectPlan {
    /// Validates and seals this plan into a replay-stable digest and operation identity.
    pub fn prepare(self, idempotency_key: &str) -> Result<PreparedEffect, EffectPlanError> {
        self.validate()?;
        if idempotency_key.trim().is_empty() {
            return Err(EffectPlanError::Empty("idempotency_key"));
        }
        let bytes = serde_json::to_vec(&self).map_err(|_| EffectPlanError::Encoding)?;
        let plan_digest = digest(&[b"service-effect-plan-digest/1", &bytes]);
        let operation_id = digest(&[
            b"service-effect-operation/1",
            self.service.as_bytes(),
            idempotency_key.as_bytes(),
            plan_digest.as_bytes(),
        ]);
        Ok(PreparedEffect {
            plan: self,
            plan_digest: format!("sha256:{plan_digest}"),
            operation_id: format!("effect:sha256:{operation_id}"),
            idempotency_key: idempotency_key.to_owned(),
        })
    }

    fn validate(&self) -> Result<(), EffectPlanError> {
        if self.format != EFFECT_PLAN_FORMAT {
            return Err(EffectPlanError::Format);
        }
        for (name, value) in [
            ("service", self.service.as_str()),
            ("operation", self.operation.as_str()),
            ("resource_revision", self.resource_revision.as_str()),
            ("authority_reference", self.authority_reference.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(EffectPlanError::Empty(name));
            }
        }
        for (name, value) in [
            ("input_digest", self.input_digest.as_str()),
            ("binding_digest", self.binding_digest.as_str()),
        ] {
            if !is_sha256(value) {
                return Err(EffectPlanError::Digest(name));
            }
        }
        if self.grant_reference.is_some() != self.grant_revision.is_some() {
            return Err(EffectPlanError::GrantPair);
        }
        if self.consequences.is_empty()
            || self
                .consequences
                .iter()
                .any(|consequence| consequence.trim().is_empty())
        {
            return Err(EffectPlanError::Consequences);
        }
        Ok(())
    }
}

/// An exact effect accepted for durable preparation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedEffect {
    /// Validated exact plan.
    pub plan: EffectPlan,
    /// Canonical plan SHA-256.
    pub plan_digest: String,
    /// Stable downstream idempotency and recovery identity.
    pub operation_id: String,
    /// Caller-selected service idempotency key.
    pub idempotency_key: String,
}

/// Exact plan validation failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum EffectPlanError {
    /// Unknown plan contract.
    #[error("unsupported effect plan format")]
    Format,
    /// Required coordinate is empty.
    #[error("effect plan coordinate {0} must not be empty")]
    Empty(&'static str),
    /// A digest is not a complete lowercase SHA-256.
    #[error("effect plan coordinate {0} must be a complete lowercase SHA-256")]
    Digest(&'static str),
    /// Grant reference and revision must appear together.
    #[error("effect grant reference and revision must appear together")]
    GrantPair,
    /// Consequence inventory is empty or malformed.
    #[error("effect plan must declare non-empty consequences")]
    Consequences,
    /// Canonical plan encoding failed.
    #[error("effect plan could not be canonically encoded")]
    Encoding,
}

/// One bounded worker claim over a prepared effect.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectClaim {
    /// Receiver-minted lease identity.
    pub lease_id: String,
    /// Worker identity for audit.
    pub worker: String,
    /// RFC 3339 lease expiry.
    pub expires_at: String,
}

/// Terminal downstream result containing references and digests, never raw payloads.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectOutcome {
    /// Downstream committed the exact operation.
    Succeeded {
        /// Opaque downstream result/evidence reference.
        result_reference: String,
        /// Digest of the normalized result.
        result_digest: String,
    },
    /// Downstream policy or expected state refused the operation.
    Refused {
        /// Stable non-secret refusal code.
        code: String,
    },
    /// Downstream conclusively failed without applying the effect.
    Failed {
        /// Stable non-secret failure code.
        code: String,
    },
    /// Outcome cannot be proven and must not be automatically repeated.
    Unknown {
        /// Stable non-secret uncertainty code.
        code: String,
    },
}

/// Durable effect lifecycle state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectState {
    /// Plan was committed before any dispatch.
    Prepared,
    /// One worker owns a bounded dispatch/recovery lease.
    Claimed {
        /// Current bounded worker lease.
        claim: EffectClaim,
    },
    /// A terminal outcome was recorded.
    Completed {
        /// Proven or explicitly unknown terminal result.
        outcome: EffectOutcome,
    },
}

/// Folded effect journal record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectRecord {
    /// Exact prepared effect.
    pub prepared: PreparedEffect,
    /// Current lifecycle state.
    pub state: EffectState,
    /// Gapless journal revision.
    pub revision: u64,
}

/// Result of an atomic journal claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClaimDisposition {
    /// This worker acquired the claim.
    Acquired(EffectRecord),
    /// Another live worker owns the effect.
    Busy(EffectRecord),
    /// A terminal result already exists.
    Terminal(EffectRecord),
}

/// Durable effect journal selected by deployment.
pub trait EffectJournal: Send {
    /// Adapter-specific durable-store error.
    type Error;

    /// Idempotently prepare before dispatch.
    fn prepare<'a>(
        &'a mut self,
        context: &'a VerifiedAuthContext,
        effect: PreparedEffect,
    ) -> BoxFuture<'a, Result<EffectRecord, Self::Error>>;

    /// Atomically claim a prepared or expired-claim record.
    fn claim<'a>(
        &'a mut self,
        context: &'a VerifiedAuthContext,
        operation_id: &'a str,
        claim: EffectClaim,
        now: &'a str,
    ) -> BoxFuture<'a, Result<ClaimDisposition, Self::Error>>;

    /// Commit the exact claim's terminal outcome.
    fn complete<'a>(
        &'a mut self,
        context: &'a VerifiedAuthContext,
        operation_id: &'a str,
        lease_id: &'a str,
        outcome: EffectOutcome,
    ) -> BoxFuture<'a, Result<EffectRecord, Self::Error>>;
}

/// What downstream recovery can prove before dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EffectObservation {
    /// Downstream proves this operation identity has not been applied and is safe to dispatch.
    NotObserved,
    /// Downstream already knows the terminal result.
    Completed(EffectOutcome),
    /// Downstream cannot establish whether the consequence happened.
    Unknown {
        /// Stable non-secret uncertainty code.
        code: String,
    },
}

/// A transport-level dispatch failure with an audit-safe code.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("external effect dispatch failed: {code}")]
pub struct EffectDispatchError {
    /// Stable non-secret error code.
    pub code: String,
}

/// Exact downstream effect adapter selected by deployment.
pub trait EffectAdapter: Send {
    /// Query by stable operation identity before dispatch or after restart.
    fn observe<'a>(
        &'a mut self,
        effect: &'a PreparedEffect,
    ) -> BoxFuture<'a, Result<EffectObservation, EffectDispatchError>>;

    /// Dispatch exactly once under the downstream operation identity.
    fn dispatch<'a>(
        &'a mut self,
        effect: &'a PreparedEffect,
    ) -> BoxFuture<'a, Result<EffectOutcome, EffectDispatchError>>;
}

/// Recover or execute one acquired effect, recording uncertainty instead of blind retry.
pub async fn resume_effect<J: EffectJournal + ?Sized, A: EffectAdapter + ?Sized>(
    journal: &mut J,
    adapter: &mut A,
    context: &VerifiedAuthContext,
    effect: &PreparedEffect,
    claim: &EffectClaim,
) -> Result<EffectRecord, J::Error> {
    let outcome = match adapter.observe(effect).await {
        Ok(EffectObservation::Completed(outcome)) => outcome,
        Ok(EffectObservation::Unknown { code }) => EffectOutcome::Unknown { code },
        Ok(EffectObservation::NotObserved) => match adapter.dispatch(effect).await {
            Ok(outcome) => outcome,
            Err(error) => EffectOutcome::Unknown { code: error.code },
        },
        Err(error) => EffectOutcome::Unknown { code: error.code },
    };
    journal
        .complete(context, &effect.operation_id, &claim.lease_id, outcome)
        .await
}

fn is_sha256(value: &str) -> bool {
    let value = value.strip_prefix("sha256:").unwrap_or(value);
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digest(parts: &[&[u8]]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    let mut output = String::with_capacity(64);
    for byte in hash.finalize() {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> EffectPlan {
        EffectPlan {
            format: EFFECT_PLAN_FORMAT.to_owned(),
            service: "agentide".to_owned(),
            operation: "code_edit".to_owned(),
            input_digest: "a".repeat(64),
            input_reference: Some("content:sha256:body".to_owned()),
            binding_digest: "b".repeat(64),
            aggregate_version: 4,
            resource_revision: "manifest:sha256:workspace".to_owned(),
            authority_reference: "authority:one-shot".to_owned(),
            grant_reference: Some("grant:agentide".to_owned()),
            grant_revision: Some(9),
            risk: EffectRisk::Medium,
            consequences: BTreeSet::from(["write_file".to_owned()]),
        }
    }

    #[test]
    fn exact_plan_digest_and_operation_identity_are_deterministic() {
        let left = plan().prepare("request-1").unwrap();
        let right = plan().prepare("request-1").unwrap();
        assert_eq!(left, right);
        assert!(left.plan_digest.starts_with("sha256:"));
        assert!(left.operation_id.starts_with("effect:sha256:"));
        assert_ne!(
            left.operation_id,
            plan().prepare("request-2").unwrap().operation_id
        );
    }

    #[test]
    fn grant_reference_and_revision_are_one_exact_pair() {
        let mut plan = plan();
        plan.grant_revision = None;
        assert_eq!(plan.prepare("request-1"), Err(EffectPlanError::GrantPair));
    }
}
