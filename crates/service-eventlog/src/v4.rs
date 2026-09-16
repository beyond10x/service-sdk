//! Exact `/4` binding from the synchronous recorded adapter to SDK host resources.

use entity_core::{Decision, DecisionRecord, EntityInstance};
use entity_eventlog::sync::{CallWait, RecordedEventlogBridge};
use entity_eventlog::{Authority, EventlogOperationContext};
use entity_store::asynchronous::{
    AppendOutcome, AppendRequest, BatchKey, CommitReceipt, HistoryOrigin, RecordedEntry,
    StoredBatch, Subject,
};
use serde_json::{Map, Value};
use service_engine::{
    ObligationUse,
    v4::{AuthenticatedPartition, ResourcesV4},
};
use service_runtime::VerifiedAuthContext;

/// SDK-owned behavior surrounding the immutable Entity Runtime/Eventlog boundary.
pub trait HostResourcesV4 {
    /// Enforces the operation scope before application input decoding.
    fn authorize(&mut self, context: &VerifiedAuthContext, scope: &str) -> Result<(), String>;
    /// Supplies a trusted clock value.
    fn trusted_clock(&mut self) -> Result<Value, String>;
    /// Supplies one `UUIDv7` logical value.
    fn uuid_v7(&mut self) -> Result<Value, String>;
    /// Runs one declared ordinary-slot obligation once.
    fn slot_obligation(
        &mut self,
        name: &str,
        input: &Map<String, Value>,
        context: &VerifiedAuthContext,
    ) -> Result<Option<Value>, String>;
    /// Runs one selected operation-field obligation once.
    fn operation_field_obligation(
        &mut self,
        name: &str,
        field: &str,
        input: &Map<String, Value>,
        loaded: &EntityInstance,
        context: &VerifiedAuthContext,
    ) -> Result<Value, String>;
    /// Stages validated plaintext outside Eventlog.
    fn stage_content(
        &mut self,
        context: &VerifiedAuthContext,
        policy: &str,
        idempotency_key: &str,
        media_type: &str,
        bytes: &[u8],
    ) -> Result<service_engine::v4::StagedContentV4, String>;
    /// Accepts one staged object after durable aftercare succeeds.
    fn accept_content(
        &mut self,
        context: &VerifiedAuthContext,
        token: String,
    ) -> Result<(), String>;
    /// Accepts committed staged content after repair reconstructs its durable coordinates.
    fn accept_recorded_content(
        &mut self,
        context: &VerifiedAuthContext,
        policy: &str,
        idempotency_key: &str,
        reference: &str,
    ) -> Result<(), String>;
    /// Abandons one conclusively unreferenced staged object.
    fn abandon_content(
        &mut self,
        context: &VerifiedAuthContext,
        token: String,
    ) -> Result<(), String>;
    /// Updates declared SDK projections from durable authority.
    fn project(
        &mut self,
        context: &VerifiedAuthContext,
        intent: &service_engine::v4::IntentPlanV4,
        decision: &Decision,
        receipt: &CommitReceipt,
    ) -> Result<(), String>;
    /// Runs declared SDK/provider effects from the durable selected result.
    fn effects(
        &mut self,
        context: &VerifiedAuthContext,
        intent: &service_engine::v4::IntentPlanV4,
        decision: &Decision,
        receipt: &CommitReceipt,
    ) -> Result<(), String>;
    /// Applies declared row-level visibility obligations to one authoritative instance.
    fn visible(
        &mut self,
        instance: &EntityInstance,
        obligations: &[ObligationUse],
        context: &VerifiedAuthContext,
        facts: &crate::AuthorityFacts,
    ) -> Result<bool, String>;
}

/// One request-scoped binding to an already opened, provisioned recorded adapter.
pub struct EventlogResourcesV4<'a, H> {
    bridge: &'a RecordedEventlogBridge,
    authority: &'a Authority,
    operation: EventlogOperationContext,
    wait: CallWait,
    host: &'a mut H,
}

impl<'a, H> EventlogResourcesV4<'a, H> {
    /// Binds one operation without provisioning or changing provider administration.
    pub const fn new(
        bridge: &'a RecordedEventlogBridge,
        authority: &'a Authority,
        operation: EventlogOperationContext,
        wait: CallWait,
        host: &'a mut H,
    ) -> Self {
        Self {
            bridge,
            authority,
            operation,
            wait,
            host,
        }
    }
}

impl<H: HostResourcesV4> ResourcesV4 for EventlogResourcesV4<'_, H> {
    fn authorize(&mut self, context: &VerifiedAuthContext, scope: &str) -> Result<(), String> {
        self.host.authorize(context, scope)
    }

    fn bind_partition(&mut self, partition: &AuthenticatedPartition) -> Result<(), String> {
        if self.authority.logical_scope != partition.logical_scope
            || self.authority.tenant != partition.physical_tenant
        {
            return Err("opened Eventlog authority does not match authenticated partition".into());
        }
        Ok(())
    }

    fn lookup_batch(&mut self, key: &BatchKey) -> Result<Option<StoredBatch>, String> {
        self.bridge
            .lookup_batch(key, self.wait)
            .map_err(|error| format!("{error:?}"))
    }

    fn load(&mut self, subject: &Subject) -> Result<Option<EntityInstance>, String> {
        self.bridge
            .load(subject, self.wait)
            .map_err(|error| format!("{error:?}"))
    }

    fn history(&mut self, subject: &Subject) -> Result<Vec<DecisionRecord>, String> {
        let history = self
            .bridge
            .history(subject, self.wait)
            .map_err(|error| format!("{error:?}"))?;
        if history.origin != HistoryOrigin::Genesis {
            return Err("/4 replay requires a complete ER genesis history".into());
        }
        Ok(history
            .records
            .into_iter()
            .filter_map(|record| match record.entry {
                RecordedEntry::Decision(commit) => Some(commit.envelope.record),
                RecordedEntry::Observation(_) => None,
            })
            .collect())
    }

    fn append(&mut self, request: AppendRequest) -> Result<AppendOutcome, String> {
        self.bridge
            .operation(self.operation.clone())
            .append(request, self.wait)
            .map_err(|error| format!("{error:?}"))
    }

    fn trusted_clock(&mut self) -> Result<Value, String> {
        self.host.trusted_clock()
    }

    fn uuid_v7(&mut self) -> Result<Value, String> {
        self.host.uuid_v7()
    }

    fn slot_obligation(
        &mut self,
        name: &str,
        input: &Map<String, Value>,
        context: &VerifiedAuthContext,
    ) -> Result<Option<Value>, String> {
        self.host.slot_obligation(name, input, context)
    }

    fn operation_field_obligation(
        &mut self,
        name: &str,
        field: &str,
        input: &Map<String, Value>,
        loaded: &EntityInstance,
        context: &VerifiedAuthContext,
    ) -> Result<Value, String> {
        self.host
            .operation_field_obligation(name, field, input, loaded, context)
    }

    fn stage_content(
        &mut self,
        context: &VerifiedAuthContext,
        policy: &str,
        idempotency_key: &str,
        media_type: &str,
        bytes: &[u8],
    ) -> Result<service_engine::v4::StagedContentV4, String> {
        self.host
            .stage_content(context, policy, idempotency_key, media_type, bytes)
    }

    fn accept_content(
        &mut self,
        context: &VerifiedAuthContext,
        token: String,
    ) -> Result<(), String> {
        self.host.accept_content(context, token)
    }

    fn accept_recorded_content(
        &mut self,
        context: &VerifiedAuthContext,
        policy: &str,
        idempotency_key: &str,
        reference: &str,
    ) -> Result<(), String> {
        self.host
            .accept_recorded_content(context, policy, idempotency_key, reference)
    }

    fn abandon_content(
        &mut self,
        context: &VerifiedAuthContext,
        token: String,
    ) -> Result<(), String> {
        self.host.abandon_content(context, token)
    }

    fn project(
        &mut self,
        context: &VerifiedAuthContext,
        intent: &service_engine::v4::IntentPlanV4,
        decision: &Decision,
        receipt: &CommitReceipt,
    ) -> Result<(), String> {
        self.host.project(context, intent, decision, receipt)
    }

    fn effects(
        &mut self,
        context: &VerifiedAuthContext,
        intent: &service_engine::v4::IntentPlanV4,
        decision: &Decision,
        receipt: &CommitReceipt,
    ) -> Result<(), String> {
        self.host.effects(context, intent, decision, receipt)
    }
}
