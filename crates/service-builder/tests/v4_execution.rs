//! Native acceptance for Entity Runtime delegated mutation ordering and recovery.

use entity_core::{Decision, DecisionRecord, EntityInstance};
use entity_store::{
    Recording,
    asynchronous::{
        AppendOutcome, AppendRequest, AppendScript, AsyncRecordedReader, AsyncRecordedWriter,
        AsyncStateReader, BatchKey, CommitReceipt, HistoryOrigin, MemoryRecordedStore, StoredBatch,
        Subject,
    },
};
use futures::executor::block_on;
use serde_json::{Map, Value, json};
use service_definition::{
    DefinitionId, ObligationDefinition, ObligationProviderId, v4::ServiceDefinitionV4,
};
use service_engine::v4::{
    AuthenticatedPartition, EngineV4, ExecutionErrorV4, IntentPlanV4, MutationResultV4,
    ResourcesV4, StagedContentV4, project_query_v4,
};
use service_engine::{AuthorityCheck, ContentPolicyPlan, InputSource};
use service_runtime::{
    AuthorityId, RealmId, TenantId, UserId, VerifiedAuthContext, VerifiedIdentity,
};
use sha2::{Digest as _, Sha256};

mod support;

#[derive(Default)]
struct Calls {
    authorize: usize,
    bind: usize,
    lookup: usize,
    load: usize,
    history: usize,
    append: usize,
    clock: usize,
    uuid: usize,
    slot: usize,
    field: usize,
    project: usize,
    effects: usize,
    stage_content: usize,
    accept_content: usize,
    accept_recorded_content: usize,
}

struct Resources {
    store: MemoryRecordedStore,
    partition: AuthenticatedPartition,
    calls: Calls,
    deny: bool,
    fail_projection: bool,
    fail_accept_content: bool,
    recorded_content: Vec<(String, String, String)>,
    obligation_snapshot: Vec<EntityInstance>,
}

impl Resources {
    fn new(service: &str, context: &VerifiedAuthContext) -> Self {
        Self {
            store: MemoryRecordedStore::new(),
            partition: AuthenticatedPartition::derive(service, context),
            calls: Calls::default(),
            deny: false,
            fail_projection: false,
            fail_accept_content: false,
            recorded_content: Vec::new(),
            obligation_snapshot: Vec::new(),
        }
    }
}

impl ResourcesV4 for Resources {
    fn authorize(&mut self, _: &VerifiedAuthContext, _: &str) -> Result<(), String> {
        self.calls.authorize += 1;
        if self.deny {
            Err("denied".into())
        } else {
            Ok(())
        }
    }

    fn bind_partition(&mut self, partition: &AuthenticatedPartition) -> Result<(), String> {
        self.calls.bind += 1;
        if partition == &self.partition {
            Ok(())
        } else {
            Err("wrong partition".into())
        }
    }

    fn lookup_batch(&mut self, key: &BatchKey) -> Result<Option<StoredBatch>, String> {
        self.calls.lookup += 1;
        block_on(self.store.lookup_batch(key)).map_err(|error| error.to_string())
    }

    fn load(&mut self, subject: &Subject) -> Result<Option<EntityInstance>, String> {
        self.calls.load += 1;
        block_on(self.store.load(subject)).map_err(|error| error.to_string())
    }

    fn history(&mut self, subject: &Subject) -> Result<Vec<DecisionRecord>, String> {
        self.calls.history += 1;
        let history = block_on(self.store.history(subject)).map_err(|error| error.to_string())?;
        if history.origin != HistoryOrigin::Genesis {
            return Err("expected genesis history".into());
        }
        Ok(history
            .records
            .into_iter()
            .filter_map(|record| match record.entry {
                entity_store::asynchronous::RecordedEntry::Decision(commit) => {
                    Some(commit.envelope.record)
                }
                entity_store::asynchronous::RecordedEntry::Observation(_) => None,
            })
            .collect())
    }

    fn obligation_instances(&mut self) -> Result<Vec<EntityInstance>, String> {
        Ok(self.obligation_snapshot.clone())
    }

    fn obligation_authority(
        &mut self,
        context: &VerifiedAuthContext,
        check: AuthorityCheck,
    ) -> Result<bool, String> {
        Ok(matches!(
            check,
            AuthorityCheck::OwnerAndScopes { owner, scopes }
                if owner.as_str() == Some(context.authority().as_str())
                    && scopes.as_object().is_some_and(serde_json::Map::is_empty)
        ))
    }

    fn append(&mut self, request: AppendRequest) -> Result<AppendOutcome, String> {
        self.calls.append += 1;
        block_on(self.store.append(request)).map_err(|error| error.to_string())
    }

    fn trusted_clock(&mut self) -> Result<Value, String> {
        self.calls.clock += 1;
        Ok(json!("2026-09-16T12:00:00Z"))
    }

    fn uuid_v7(&mut self) -> Result<Value, String> {
        self.calls.uuid += 1;
        Ok(json!("018f7f4c-9b68-7abc-8def-0123456789ab"))
    }

    fn slot_obligation(
        &mut self,
        name: &str,
        input: &Map<String, Value>,
        _: &VerifiedAuthContext,
    ) -> Result<Option<Value>, String> {
        self.calls.slot += 1;
        match name {
            "payee_from_email" => Ok(Some(json!({
                "kind": "person",
                "value": input.get("customer_email").cloned().ok_or("email")?,
            }))),
            other => Err(format!("unknown slot obligation {other}")),
        }
    }

    fn operation_field_obligation(
        &mut self,
        name: &str,
        _: &str,
        _: &Map<String, Value>,
        _: &EntityInstance,
        _: &VerifiedAuthContext,
    ) -> Result<Value, String> {
        self.calls.field += 1;
        match name {
            "issued_clock" => Ok(json!("2026-09-16T12:00:00Z")),
            other => Err(format!("unknown field obligation {other}")),
        }
    }

    fn stage_content(
        &mut self,
        _: &VerifiedAuthContext,
        _: &str,
        _: &str,
        _: &str,
        _: &[u8],
    ) -> Result<StagedContentV4, String> {
        self.calls.stage_content += 1;
        Ok(StagedContentV4 {
            reference: "content-reference".into(),
            token: "content-token".into(),
        })
    }

    fn accept_content(&mut self, _: &VerifiedAuthContext, _: String) -> Result<(), String> {
        self.calls.accept_content += 1;
        if self.fail_accept_content {
            Err("content acceptance unavailable".to_owned())
        } else {
            Ok(())
        }
    }

    fn accept_recorded_content(
        &mut self,
        _: &VerifiedAuthContext,
        policy: &str,
        idempotency_key: &str,
        reference: &str,
    ) -> Result<(), String> {
        self.calls.accept_recorded_content += 1;
        self.recorded_content.push((
            policy.to_owned(),
            idempotency_key.to_owned(),
            reference.to_owned(),
        ));
        Ok(())
    }

    fn abandon_content(&mut self, _: &VerifiedAuthContext, _: String) -> Result<(), String> {
        Ok(())
    }

    fn project(
        &mut self,
        _: &VerifiedAuthContext,
        _: &IntentPlanV4,
        _: &Decision,
        _: &CommitReceipt,
    ) -> Result<(), String> {
        self.calls.project += 1;
        if self.fail_projection {
            Err("projection unavailable".into())
        } else {
            Ok(())
        }
    }

    fn effects(
        &mut self,
        _: &VerifiedAuthContext,
        _: &IntentPlanV4,
        _: &Decision,
        _: &CommitReceipt,
    ) -> Result<(), String> {
        self.calls.effects += 1;
        Ok(())
    }
}

fn context(realm: Option<&str>) -> VerifiedAuthContext {
    context_for_authority("account-a", realm)
}

fn context_for_authority(authority: &str, realm: Option<&str>) -> VerifiedAuthContext {
    VerifiedAuthContext::from_verified(VerifiedIdentity::after_verification(
        TenantId::new("tenant-a").unwrap(),
        AuthorityId::new(authority).unwrap(),
        UserId::new("user-a").unwrap(),
        None,
        realm.map(|value| RealmId::new(value).unwrap()),
    ))
}

#[test]
fn owner_obligation_checks_the_loaded_subject_not_an_unrelated_partition_instance() {
    const OWNER: &str = "018f7f4c-9b68-7abc-8def-111111111111";
    const OTHER: &str = "018f7f4c-9b68-7abc-8def-222222222222";
    let root = support::fixture("billing");
    let mut definition = ServiceDefinitionV4::from_yaml(
        &std::fs::read_to_string(root.join("runtime.yaml")).unwrap(),
    )
    .unwrap();
    let obligation = DefinitionId::new("invoice_owner").unwrap();
    definition.obligations.push(ObligationDefinition {
        name: obligation.clone(),
        provider: ObligationProviderId::new("sdk.auth.owner-and-conjunctive-scopes/v1").unwrap(),
        bindings: [
            (
                DefinitionId::new("owner").unwrap(),
                "billing.invoice.Invoice.account_id".to_owned(),
            ),
            (
                DefinitionId::new("scopes").unwrap(),
                "billing.invoice.Invoice.metadata".to_owned(),
            ),
        ]
        .into_iter()
        .collect(),
        description: "Only the invoice owner may pay it.".to_owned(),
    });
    definition
        .intents
        .iter_mut()
        .find(|intent| intent.name.as_str() == "pay_invoice")
        .unwrap()
        .obligations
        .push(obligation);
    let build = billing_with_definition(&definition);
    let engine = EngineV4::new(&build.realization_plan).unwrap();
    let owner = context_for_authority(OWNER, None);
    let other = context_for_authority(OTHER, None);
    let mut resources = Resources::new("billing", &owner);

    let (created, _) = committed(
        engine
            .execute_json(
                &owner,
                "create_invoice",
                &serde_json::to_vec(&json!({
                    "account_id": OWNER,
                    "customer_email": "owner@example.test",
                    "amount": {"amount": 12.5, "currency": "EUR"}
                }))
                .unwrap(),
                0,
                "owner-create",
                recording("owner-create"),
                &mut resources,
            )
            .unwrap(),
    );
    let invoice_id = created.instance.fields["invoice_id"].clone();
    let (issued, _) = committed(
        engine
            .execute_json(
                &owner,
                "issue_invoice",
                &serde_json::to_vec(&json!({"invoice_id": invoice_id})).unwrap(),
                1,
                "owner-issue",
                recording("owner-issue"),
                &mut resources,
            )
            .unwrap(),
    );
    resources.obligation_snapshot = vec![invoice_owned_by(&issued, OTHER)];
    let appends_before = resources.calls.append;

    let result = engine.execute_public_json(
        &other,
        "pay_invoice",
        &serde_json::to_vec(&json!({
            "invoice_id": issued.instance.fields["invoice_id"],
            "amount": {"amount": 12.5, "currency": "EUR"},
            "expected_version": 2,
            "idempotency_key": "unauthorized-pay"
        }))
        .unwrap(),
        recording("unauthorized-pay"),
        &mut resources,
    );
    let outcome = match &result {
        Ok(MutationResultV4::Committed { .. }) => "committed",
        Ok(MutationResultV4::Refused(_)) => "domain_refused",
        Err(ExecutionErrorV4::ObligationRefused(_)) => "obligation_refused",
        Err(_) => "other_error",
    };
    assert!(
        matches!(result, Err(ExecutionErrorV4::ObligationRefused(_))),
        "the owner check must use the selected invoice, not an unrelated invoice in the same partition; observed {outcome}"
    );
    assert_eq!(
        resources.calls.append, appends_before,
        "a refused payment cannot append"
    );

    assert_selected_owner_can_pay(&engine, &issued, &owner, &mut resources);
}

fn assert_selected_owner_can_pay(
    engine: &EngineV4<'_>,
    issued: &Decision,
    owner: &VerifiedAuthContext,
    resources: &mut Resources,
) {
    let authorized = engine.execute_public_json(
        owner,
        "pay_invoice",
        &serde_json::to_vec(&json!({
            "invoice_id": issued.instance.fields["invoice_id"],
            "amount": {"amount": 12.5, "currency": "EUR"},
            "expected_version": 2,
            "idempotency_key": "authorized-pay"
        }))
        .unwrap(),
        recording("authorized-pay"),
        resources,
    );
    assert!(
        matches!(authorized, Ok(MutationResultV4::Committed { .. })),
        "the selected invoice owner must remain authorized: {authorized:?}"
    );
}

fn invoice_owned_by(issued: &Decision, owner: &str) -> EntityInstance {
    let mut invoice = issued.instance.clone();
    invoice.id = format!("s:{owner}");
    invoice.fields.insert("invoice_id".to_owned(), json!(owner));
    invoice.fields.insert("account_id".to_owned(), json!(owner));
    invoice
}

#[test]
fn lifecycle_obligation_accepts_the_selected_logical_identity_in_an_allowed_state() {
    let root = support::fixture("billing");
    let mut definition = ServiceDefinitionV4::from_yaml(
        &std::fs::read_to_string(root.join("runtime.yaml")).unwrap(),
    )
    .unwrap();
    let obligation = DefinitionId::new("must_be_issued").unwrap();
    definition.obligations.push(ObligationDefinition {
        name: obligation.clone(),
        provider: ObligationProviderId::new("sdk.lifecycle.require-state/v1").unwrap(),
        bindings: [
            (
                DefinitionId::new("entity").unwrap(),
                "billing.invoice.Invoice".to_owned(),
            ),
            (DefinitionId::new("allowed").unwrap(), "Issued".to_owned()),
            (
                DefinitionId::new("identity").unwrap(),
                "invoice_id".to_owned(),
            ),
        ]
        .into_iter()
        .collect(),
        description: "An issued invoice may be paid.".to_owned(),
    });
    definition
        .intents
        .iter_mut()
        .find(|intent| intent.name.as_str() == "pay_invoice")
        .unwrap()
        .obligations
        .push(obligation);
    let build = billing_with_definition(&definition);
    let engine = EngineV4::new(&build.realization_plan).unwrap();
    let context = context(None);
    let mut resources = Resources::new("billing", &context);
    let (created, _) = committed(
        engine
            .execute_json(
                &context,
                "create_invoice",
                &serde_json::to_vec(&json!({
                    "account_id": "018f7f4c-9b68-7abc-8def-111111111111",
                    "customer_email": "person@example.test",
                    "amount": {"amount": 12.5, "currency": "EUR"}
                }))
                .unwrap(),
                0,
                "allowed-create",
                recording("allowed-create"),
                &mut resources,
            )
            .unwrap(),
    );
    let invoice_id = created.instance.fields["invoice_id"].clone();
    let (issued, _) = committed(
        engine
            .execute_json(
                &context,
                "issue_invoice",
                &serde_json::to_vec(&json!({"invoice_id": invoice_id})).unwrap(),
                1,
                "allowed-issue",
                recording("allowed-issue"),
                &mut resources,
            )
            .unwrap(),
    );
    assert_eq!(issued.instance.lifecycle_state, "Issued");
    assert_eq!(
        issued.instance.id,
        format!(
            "s:{}",
            issued.instance.fields["invoice_id"].as_str().unwrap()
        )
    );

    let result = engine.execute_public_json(
        &context,
        "pay_invoice",
        &serde_json::to_vec(&json!({
            "invoice_id": issued.instance.fields["invoice_id"],
            "amount": {"amount": 12.5, "currency": "EUR"},
            "expected_version": 2,
            "idempotency_key": "allowed-pay"
        }))
        .unwrap(),
        recording("allowed-pay"),
        &mut resources,
    );
    assert!(
        matches!(result, Ok(MutationResultV4::Committed { .. })),
        "the lifecycle obligation must resolve the selected logical identity to its storage address: {result:?}"
    );
}

#[test]
fn owned_revision_resolves_public_logical_identity_and_state() {
    let build = billing_with_owned_revision();
    let engine = EngineV4::new(&build.realization_plan).unwrap();
    let context = context(None);
    let mut resources = Resources::new("billing", &context);
    let (created, _) = committed(
        engine
            .execute_json(
                &context,
                "create_invoice",
                &serde_json::to_vec(&json!({
                    "account_id": "018f7f4c-9b68-7abc-8def-111111111111",
                    "customer_email": "revision@example.test",
                    "amount": {"amount": 12.5, "currency": "EUR"}
                }))
                .unwrap(),
                0,
                "revision-create",
                recording("revision-create"),
                &mut resources,
            )
            .unwrap(),
    );
    let invoice_id = created.instance.fields["invoice_id"].clone();
    let draft_pay = serde_json::to_vec(&json!({
        "invoice_id": invoice_id,
        "amount": {"amount": 12.5, "currency": "EUR"},
        "expected_version": 1,
        "idempotency_key": "revision-draft-pay"
    }))
    .unwrap();
    let appends_before = resources.calls.append;
    let draft_result = engine.execute_public_json(
        &context,
        "pay_invoice",
        &draft_pay,
        recording("revision-draft-pay"),
        &mut resources,
    );
    assert!(
        matches!(
            draft_result,
            Err(ExecutionErrorV4::ObligationRefused(ref reason)) if reason == "revision_not_publishable"
        ),
        "a present Draft revision must be refused by its state, observed {draft_result:?}"
    );
    assert_eq!(resources.calls.append, appends_before);

    let (issued, _) = committed(
        engine
            .execute_json(
                &context,
                "issue_invoice",
                &serde_json::to_vec(&json!({"invoice_id": invoice_id})).unwrap(),
                1,
                "revision-issue",
                recording("revision-issue"),
                &mut resources,
            )
            .unwrap(),
    );
    let issued_pay = serde_json::to_vec(&json!({
        "invoice_id": issued.instance.fields["invoice_id"],
        "amount": {"amount": 12.5, "currency": "EUR"},
        "expected_version": 2,
        "idempotency_key": "revision-issued-pay"
    }))
    .unwrap();
    let result = engine.execute_public_json(
        &context,
        "pay_invoice",
        &issued_pay,
        recording("revision-issued-pay"),
        &mut resources,
    );
    assert!(
        matches!(result, Ok(MutationResultV4::Committed { .. })),
        "the issued logical revision must resolve to its ER address: {result:?}"
    );
}

fn billing_with_owned_revision() -> service_builder::ServiceBuildV4 {
    let root = support::fixture("billing");
    let mut definition = ServiceDefinitionV4::from_yaml(
        &std::fs::read_to_string(root.join("runtime.yaml")).unwrap(),
    )
    .unwrap();
    let obligation = DefinitionId::new("issued_revision").unwrap();
    definition.obligations.push(ObligationDefinition {
        name: obligation.clone(),
        provider: ObligationProviderId::new("sdk.aggregate.owned-revision/v1").unwrap(),
        bindings: [
            (
                DefinitionId::new("revision").unwrap(),
                "billing.invoice.Invoice".to_owned(),
            ),
            (
                DefinitionId::new("revision_identity").unwrap(),
                "invoice_id".to_owned(),
            ),
            (DefinitionId::new("allowed").unwrap(), "Issued".to_owned()),
        ]
        .into_iter()
        .collect(),
        description: "Only an issued invoice revision may be paid.".to_owned(),
    });
    definition
        .intents
        .iter_mut()
        .find(|intent| intent.name.as_str() == "pay_invoice")
        .unwrap()
        .obligations
        .push(obligation);
    billing_with_definition(&definition)
}

fn recording(label: &str) -> Recording {
    Recording {
        record_id: format!("record-{label}"),
        recorded_at: "2026-09-16T12:00:00Z".into(),
        correlation: Some("flow-native-v4".into()),
        causation: None,
        actor: Some("user-a".into()),
    }
}

fn billing() -> service_builder::ServiceBuildV4 {
    let root = support::fixture("billing");
    let definition = ServiceDefinitionV4::from_yaml(
        &std::fs::read_to_string(root.join("runtime.yaml")).unwrap(),
    )
    .unwrap();
    service_builder::build_service_v4(&support::sources("billing"), &definition).unwrap()
}

fn billing_with_definition(definition: &ServiceDefinitionV4) -> service_builder::ServiceBuildV4 {
    service_builder::build_service_v4(&support::sources("billing"), definition).unwrap()
}

fn gatepass() -> service_builder::ServiceBuildV4 {
    let root = support::fixture("gatepass");
    let definition = ServiceDefinitionV4::from_yaml(
        &std::fs::read_to_string(root.join("runtime.yaml")).unwrap(),
    )
    .unwrap();
    service_builder::build_service_v4(&support::sources("gatepass"), &definition).unwrap()
}

fn committed(result: MutationResultV4) -> (Decision, bool) {
    match result {
        MutationResultV4::Committed {
            decision, replayed, ..
        } => (*decision, replayed),
        MutationResultV4::Refused(refusal) => panic!("unexpected refusal: {refusal:?}"),
    }
}

#[test]
fn billing_projection_uses_source_filter_selectors_visibility_sort_and_page() {
    let build = billing();
    let invoice = |id: &str, state: &str, revision: u64| EntityInstance {
        entity: "billing.invoice.Invoice".into(),
        version: 1,
        id: id.into(),
        lifecycle_state: state.into(),
        revision,
        fields: serde_json::from_value(json!({
            "invoice_id": id,
            "total": {"amount": 12.5, "currency": "EUR"},
            "reminder_count": 0,
            "issued_at": "2026-09-16T12:00:00Z"
        }))
        .unwrap(),
    };
    let instances = vec![
        invoice("invoice-b", "Issued", 2),
        invoice("invoice-c", "Draft", 1),
        invoice("invoice-a", "Issued", 3),
    ];
    let first = project_query_v4(
        &build.realization_plan,
        "list_outstanding",
        br"{}",
        instances.clone(),
        None,
        1,
        |instance, obligations| {
            assert_eq!(obligations.len(), 1);
            Ok(instance.id != "invoice-b")
        },
    )
    .unwrap();
    assert_eq!(
        first.items,
        vec![json!({
            "invoice_id": "invoice-a",
            "total": {"amount": 12.5, "currency": "EUR"},
            "issued_at": "2026-09-16T12:00:00Z"
        })]
    );
    assert_eq!(first.through_version, Some(3));
    assert!(first.partial);
    let second = project_query_v4(
        &build.realization_plan,
        "list_outstanding",
        br"{}",
        instances.clone(),
        first.next_cursor.as_deref(),
        1,
        |_, _| Ok(true),
    )
    .unwrap();
    assert_eq!(second.items[0]["invoice_id"], "invoice-b");
    assert!(!second.partial);
    assert!(second.next_cursor.is_none());
    let selected = project_query_v4(
        &build.realization_plan,
        "get_invoice",
        br#"{"invoice_id":"invoice-c"}"#,
        instances,
        None,
        100,
        |_, _| Ok(true),
    )
    .unwrap();
    assert_eq!(selected.items.len(), 1);
    assert_eq!(selected.items[0]["invoice_id"], "invoice-c");
}

#[test]
fn gatepass_preserves_badge_fulfillment_and_expected_view_transition() {
    let build = gatepass();
    let engine = EngineV4::new(&build.realization_plan).unwrap();
    let context = context(None);
    let mut resources = Resources::new("gatepass", &context);
    let register = serde_json::to_vec(&json!({
        "visitor": "Visitor One",
        "building": "North",
        "host": {"kind": "employee", "value": "account-a"},
        "expected_minutes": 30,
        "expected_stay": "PT30M",
        "deposit": {"amount": 20, "currency": "EUR"},
        "escorts": [],
        "notes": {},
        "on_watchlist": false
    }))
    .unwrap();
    let (registered, _) = committed(
        engine
            .execute_json(
                &context,
                "register_visit",
                &register,
                0,
                "register-1",
                recording("register"),
                &mut resources,
            )
            .unwrap(),
    );
    let expected = project_query_v4(
        &build.realization_plan,
        "list_expected",
        br"{}",
        [registered.instance.clone()],
        None,
        100,
        |_, _| Ok(true),
    )
    .unwrap();
    assert_eq!(expected.items.len(), 1);
    let visit_id = registered.instance.fields["visit_id"].clone();
    let admit = serde_json::to_vec(&json!({
        "visit_id": visit_id,
        "badge": {"serial": "badge-007", "signature": "AA=="}
    }))
    .unwrap();
    let (admitted, _) = committed(
        engine
            .execute_json(
                &context,
                "admit_visitor",
                &admit,
                1,
                "admit-1",
                recording("admit"),
                &mut resources,
            )
            .unwrap(),
    );
    assert_eq!(admitted.instance.fields["badge"]["serial"], "badge-007");
    assert_eq!(admitted.record.outcome.as_deref(), Some("admitted"));
    let no_longer_expected = project_query_v4(
        &build.realization_plan,
        "list_expected",
        br"{}",
        [admitted.instance],
        None,
        100,
        |_, _| Ok(true),
    )
    .unwrap();
    assert!(no_longer_expected.items.is_empty());
}

#[test]
#[allow(clippy::too_many_lines)]
fn billing_delegates_refusal_fulfillment_atomic_retry_uncertainty_and_replay() {
    let build = billing();
    let engine = EngineV4::new(&build.realization_plan).unwrap();
    let context = context(None);
    let mut resources = Resources::new("billing", &context);
    let create = json!({
        "account_id": "018f7f4c-9b68-7abc-8def-111111111111",
        "customer_email": "person@example.test",
        "amount": {"amount": 12.5, "currency": "EUR"}
    });
    let bytes = serde_json::to_vec(&create).unwrap();

    let (created, replayed) = committed(
        engine
            .execute_json(
                &context,
                "create_invoice",
                &bytes,
                0,
                "create-1",
                recording("create"),
                &mut resources,
            )
            .unwrap(),
    );
    assert!(!replayed);
    assert_eq!(created.record.outcome.as_deref(), Some("accepted"));
    assert_eq!(created.instance.revision, 1);
    assert_eq!(resources.calls.uuid, 1);
    assert_eq!(resources.calls.slot, 1);
    assert_eq!(resources.calls.append, 1);
    assert_eq!(resources.calls.project, 1);
    assert_eq!(resources.calls.effects, 1);
    let invoice_id = created.instance.fields["invoice_id"].clone();

    let (retried, replayed) = committed(
        engine
            .execute_json(
                &context,
                "create_invoice",
                &bytes,
                0,
                "create-1",
                recording("retry"),
                &mut resources,
            )
            .unwrap(),
    );
    assert!(replayed);
    assert_eq!(retried, created);
    assert_eq!(
        resources.calls.uuid, 1,
        "retry cannot mint another identity"
    );
    assert_eq!(resources.calls.slot, 1, "retry cannot rerun a provider");
    assert_eq!(
        resources.calls.append, 1,
        "retry reads the original atomic batch"
    );
    assert_eq!(
        resources.calls.effects, 1,
        "retry cannot repeat an external effect"
    );

    let changed = serde_json::to_vec(&json!({
        "account_id": "018f7f4c-9b68-7abc-8def-111111111111",
        "customer_email": "changed@example.test",
        "amount": {"amount": 12.5, "currency": "EUR"}
    }))
    .unwrap();
    assert!(matches!(
        engine.execute_json(
            &context,
            "create_invoice",
            &changed,
            0,
            "create-1",
            recording("changed"),
            &mut resources,
        ),
        Err(ExecutionErrorV4::IdempotencyConflict)
    ));

    let loads = resources.calls.load;
    let fields = resources.calls.field;
    let zero = serde_json::to_vec(&json!({
        "invoice_id": "018f7f4c-9b68-7abc-8def-222222222222",
        "amount": {"amount": 0, "currency": "EUR"}
    }))
    .unwrap();
    assert!(matches!(
        engine
            .execute_json(
                &context,
                "pay_invoice",
                &zero,
                1,
                "pay-zero",
                recording("pay-zero"),
                &mut resources,
            )
            .unwrap(),
        MutationResultV4::Refused(_)
    ));
    assert_eq!(
        resources.calls.load, loads,
        "pre-load refusal cannot read a subject"
    );
    assert_eq!(
        resources.calls.field, fields,
        "pre-load refusal cannot fulfill a field"
    );

    let issue = serde_json::to_vec(&json!({"invoice_id": invoice_id})).unwrap();
    let (issued, _) = committed(
        engine
            .execute_json(
                &context,
                "issue_invoice",
                &issue,
                1,
                "issue-1",
                recording("issue"),
                &mut resources,
            )
            .unwrap(),
    );
    assert_eq!(issued.record.outcome.as_deref(), Some("issued"));
    assert_eq!(issued.instance.revision, 2);
    assert_eq!(
        resources.calls.field,
        fields + 1,
        "selected issued_at runs once"
    );

    let pay = serde_json::to_vec(&json!({
        "invoice_id": issued.instance.fields["invoice_id"],
        "amount": {"amount": 12.5, "currency": "EUR"}
    }))
    .unwrap();
    resources
        .store
        .script_next_append(AppendScript::CommitThenUncertain);
    let (paid, replayed) = committed(
        engine
            .execute_json(
                &context,
                "pay_invoice",
                &pay,
                2,
                "pay-1",
                recording("pay"),
                &mut resources,
            )
            .unwrap(),
    );
    assert!(
        replayed,
        "lost append response is recovered from the atomic batch"
    );
    assert_eq!(paid.record.outcome.as_deref(), Some("settled"));
    assert_eq!(paid.instance.revision, 3);
    assert!(
        resources.calls.history >= 2,
        "recovery verifies complete ER history"
    );

    let calls = (
        resources.calls.uuid,
        resources.calls.slot,
        resources.calls.field,
        resources.calls.append,
        resources.calls.effects,
    );
    let (retried_create, replayed) = committed(
        engine
            .execute_json(
                &context,
                "create_invoice",
                &bytes,
                0,
                "create-1",
                recording("create-after-payment"),
                &mut resources,
            )
            .unwrap(),
    );
    assert!(replayed);
    assert_eq!(retried_create, created);
    let (retried_issue, replayed) = committed(
        engine
            .execute_json(
                &context,
                "issue_invoice",
                &issue,
                1,
                "issue-1",
                recording("issue-after-payment"),
                &mut resources,
            )
            .unwrap(),
    );
    assert!(replayed);
    assert_eq!(retried_issue, issued);
    assert_eq!(
        (
            resources.calls.uuid,
            resources.calls.slot,
            resources.calls.field,
            resources.calls.append,
            resources.calls.effects,
        ),
        calls,
        "historical retries return original evidence without rerunning fulfillment, append, or effects"
    );
}

#[test]
fn same_idempotency_key_with_a_changed_expected_version_is_not_an_exact_retry() {
    let build = billing();
    let engine = EngineV4::new(&build.realization_plan).unwrap();
    let context = context(None);
    let mut resources = Resources::new("billing", &context);
    let create = serde_json::to_vec(&json!({
        "account_id": "018f7f4c-9b68-7abc-8def-111111111111",
        "customer_email": "person@example.test",
        "amount": {"amount": 12.5, "currency": "EUR"},
        "idempotency_key": "create-expected-version"
    }))
    .unwrap();
    let (created, _) = committed(
        engine
            .execute_public_json(
                &context,
                "create_invoice",
                &create,
                recording("create-expected-version"),
                &mut resources,
            )
            .unwrap(),
    );
    let issue = serde_json::to_vec(&json!({
        "invoice_id": created.instance.fields["invoice_id"],
        "expected_version": 1,
        "idempotency_key": "issue-expected-version"
    }))
    .unwrap();
    let _ = committed(
        engine
            .execute_public_json(
                &context,
                "issue_invoice",
                &issue,
                recording("issue-expected-version"),
                &mut resources,
            )
            .unwrap(),
    );
    let changed_precondition = serde_json::to_vec(&json!({
        "invoice_id": created.instance.fields["invoice_id"],
        "expected_version": 2,
        "idempotency_key": "issue-expected-version"
    }))
    .unwrap();

    assert!(matches!(
        engine.execute_public_json(
            &context,
            "issue_invoice",
            &changed_precondition,
            recording("issue-changed-expected-version"),
            &mut resources,
        ),
        Err(ExecutionErrorV4::IdempotencyConflict)
    ));
}

#[test]
fn v4_executes_selected_sdk_intent_obligations() {
    let root = support::fixture("billing");
    let mut definition = ServiceDefinitionV4::from_yaml(
        &std::fs::read_to_string(root.join("runtime.yaml")).unwrap(),
    )
    .unwrap();
    let obligation = DefinitionId::new("must_be_draft").unwrap();
    definition.obligations.push(ObligationDefinition {
        name: obligation.clone(),
        provider: ObligationProviderId::new("sdk.lifecycle.require-state/v1").unwrap(),
        bindings: [
            (
                DefinitionId::new("entity").unwrap(),
                "billing.invoice.Invoice".to_owned(),
            ),
            (DefinitionId::new("allowed").unwrap(), "Draft".to_owned()),
            (
                DefinitionId::new("identity").unwrap(),
                "invoice_id".to_owned(),
            ),
        ]
        .into_iter()
        .collect(),
        description: "Only a draft invoice may be paid.".to_owned(),
    });
    definition
        .intents
        .iter_mut()
        .find(|intent| intent.name.as_str() == "pay_invoice")
        .unwrap()
        .obligations
        .push(obligation);
    let build = billing_with_definition(&definition);
    let pay = &build.realization_plan.intents["pay_invoice"];
    assert!(
        pay.obligations
            .iter()
            .any(|item| item.provider == "sdk.lifecycle.require-state/v1"),
        "the compiler retained the selected SDK lifecycle implementation"
    );

    let engine = EngineV4::new(&build.realization_plan).unwrap();
    let context = context(None);
    let mut resources = Resources::new("billing", &context);
    let create = serde_json::to_vec(&json!({
        "account_id": "018f7f4c-9b68-7abc-8def-111111111111",
        "customer_email": "person@example.test",
        "amount": {"amount": 12.5, "currency": "EUR"}
    }))
    .unwrap();
    let (created, _) = committed(
        engine
            .execute_json(
                &context,
                "create_invoice",
                &create,
                0,
                "obligation-create",
                recording("obligation-create"),
                &mut resources,
            )
            .unwrap(),
    );
    let invoice_id = created.instance.fields["invoice_id"].clone();
    let issue = serde_json::to_vec(&json!({"invoice_id": invoice_id})).unwrap();
    let (issued, _) = committed(
        engine
            .execute_json(
                &context,
                "issue_invoice",
                &issue,
                1,
                "obligation-issue",
                recording("obligation-issue"),
                &mut resources,
            )
            .unwrap(),
    );
    let pay = serde_json::to_vec(&json!({
        "invoice_id": issued.instance.fields["invoice_id"],
        "amount": {"amount": 12.5, "currency": "EUR"}
    }))
    .unwrap();

    let refused = engine.execute_json(
        &context,
        "pay_invoice",
        &pay,
        2,
        "obligation-pay",
        recording("obligation-pay"),
        &mut resources,
    );
    assert!(
        refused.is_err(),
        "sdk.lifecycle.require-state/v1 must refuse paying the issued invoice because only Draft was allowed"
    );
    assert!(
        matches!(refused, Err(ExecutionErrorV4::ObligationRefused(ref reason)) if reason == "wrong_state"),
        "the disallowed selected invoice must fail its state check"
    );
}

#[test]
fn admission_precedes_decode_and_committed_projection_failure_keeps_receipt() {
    let build = billing();
    let engine = EngineV4::new(&build.realization_plan).unwrap();
    let context = context(Some("realm-a"));
    let mut resources = Resources::new("billing", &context);
    resources.deny = true;
    assert!(matches!(
        engine.execute_json(
            &context,
            "create_invoice",
            b"not-json",
            0,
            "denied",
            recording("denied"),
            &mut resources,
        ),
        Err(ExecutionErrorV4::Admission(_))
    ));
    assert_eq!(resources.calls.authorize, 1);
    assert_eq!(resources.calls.bind, 0);

    resources.deny = false;
    resources.fail_projection = true;
    let input = serde_json::to_vec(&json!({
        "account_id": "018f7f4c-9b68-7abc-8def-111111111111",
        "customer_email": "person@example.test",
        "amount": {"amount": 12.5, "currency": "EUR"},
        "idempotency_key": "aftercare"
    }))
    .unwrap();
    let error = engine
        .execute_public_json(
            &context,
            "create_invoice",
            &input,
            recording("aftercare"),
            &mut resources,
        )
        .unwrap_err();
    let ExecutionErrorV4::CommittedAftercare {
        receipt,
        repair_token,
        ..
    } = error
    else {
        panic!("expected committed-aftercare result: {error:?}");
    };
    assert!(matches!(&*receipt, CommitReceipt::Batch(batch) if batch.members.len() == 2));
    assert!(repair_token.starts_with("sdk-er-repair-"));
    assert_eq!(resources.calls.append, 1);
    assert_eq!(resources.calls.effects, 0);

    let retried = engine
        .execute_public_json(
            &context,
            "create_invoice",
            &input,
            recording("aftercare-retry"),
            &mut resources,
        )
        .unwrap();
    assert!(matches!(
        retried,
        MutationResultV4::Committed { replayed: true, .. }
    ));
    assert_eq!(resources.calls.project, 1, "ordinary retry does not repair");
    assert_eq!(
        resources.calls.effects, 0,
        "ordinary retry does not deliver"
    );

    resources.fail_projection = false;
    let repaired = engine
        .repair(&context, &repair_token, &mut resources)
        .unwrap();
    let MutationResultV4::Committed {
        receipt: repaired_receipt,
        replayed,
        ..
    } = repaired
    else {
        panic!("repair did not recover a committed decision");
    };
    assert!(replayed);
    assert_eq!(repaired_receipt, *receipt);
    assert_eq!(resources.calls.append, 1, "repair cannot append");
    assert_eq!(resources.calls.uuid, 1, "repair cannot rerun fulfillment");
    assert_eq!(resources.calls.slot, 1, "repair cannot rerun fulfillment");
    assert_eq!(resources.calls.project, 2, "repair replays projection once");
    assert_eq!(resources.calls.effects, 1, "repair resumes effects once");
}

#[test]
fn committed_content_acceptance_repairs_from_recorded_reference() {
    let mut build = billing();
    build.realization_plan.content.insert(
        "proof-content".to_owned(),
        ContentPolicyPlan {
            media_types: std::collections::BTreeSet::from(["text/plain".to_owned()]),
            max_bytes: 1024,
        },
    );
    let create = build
        .realization_plan
        .intents
        .get_mut("create_invoice")
        .unwrap();
    create
        .inputs
        .iter_mut()
        .find(|field| field.name == "customer_email")
        .unwrap()
        .source = InputSource::Content {
        policy: "proof-content".to_owned(),
        command_field: "customer_email".to_owned(),
    };
    build.realization_plan.plan_digest.clear();
    build.realization_plan.plan_digest = hex::encode(Sha256::digest(
        serde_json::to_vec(&build.realization_plan).unwrap(),
    ));
    let engine = EngineV4::new(&build.realization_plan).unwrap();
    let context = context(None);
    let mut resources = Resources::new("billing", &context);
    resources.fail_accept_content = true;
    let input = serde_json::to_vec(&json!({
        "account_id": "018f7f4c-9b68-7abc-8def-111111111111",
        "customer_email": {"media_type": "text/plain", "text": "person@example.test"},
        "amount": {"amount": 12.5, "currency": "EUR"},
        "idempotency_key": "content-1"
    }))
    .unwrap();

    let error = engine
        .execute_public_json(
            &context,
            "create_invoice",
            &input,
            recording("content"),
            &mut resources,
        )
        .unwrap_err();
    let ExecutionErrorV4::CommittedAftercare {
        receipt,
        repair_token,
        ..
    } = error
    else {
        panic!("content acceptance failure did not retain committed evidence");
    };
    assert_eq!(resources.calls.stage_content, 1);
    assert_eq!(resources.calls.accept_content, 1);
    assert_eq!(resources.calls.accept_recorded_content, 0);

    let retried = engine
        .execute_public_json(
            &context,
            "create_invoice",
            &input,
            recording("content-retry"),
            &mut resources,
        )
        .unwrap();
    assert!(matches!(
        retried,
        MutationResultV4::Committed { replayed: true, .. }
    ));
    assert_eq!(resources.calls.stage_content, 1);
    assert_eq!(resources.calls.accept_content, 1);

    resources.fail_accept_content = false;
    let repaired = engine
        .repair(&context, &repair_token, &mut resources)
        .unwrap();
    assert!(matches!(
        repaired,
        MutationResultV4::Committed {
            receipt: repaired,
            replayed: true,
            ..
        } if repaired == *receipt
    ));
    assert_eq!(resources.calls.stage_content, 1, "repair cannot restage");
    assert_eq!(
        resources.calls.accept_content, 1,
        "repair has no staging token"
    );
    assert_eq!(resources.calls.accept_recorded_content, 1);
    assert_eq!(
        resources.recorded_content,
        vec![(
            "proof-content".to_owned(),
            "content-1".to_owned(),
            "content-reference".to_owned()
        )]
    );
}
