//! Public-API adversarial cases for the frozen SDK persistence contract.

use std::collections::BTreeSet;
use std::sync::Arc;

use eventlog_core::{EventStore, TenantId};
use eventlog_sqlite::SqliteEventStore;
use service_engine::RequestMetadata;
use service_eventlog::{AuthorityFacts, EventlogService};
use service_runtime::{
    AuthorityId, ExecutorId, TenantId as ServiceTenantId, UserId, VerifiedAuthContext,
    VerifiedIdentity,
};

fn verified(authority: &str, user: &str, executor: Option<&str>) -> VerifiedAuthContext {
    VerifiedAuthContext::from_verified(VerifiedIdentity::after_verification(
        ServiceTenantId::new("adversary-tenant").unwrap(),
        AuthorityId::new(authority).unwrap(),
        UserId::new(user).unwrap(),
        executor.map(|value| ExecutorId::new(value).unwrap()),
        None,
    ))
}

fn current_facts() -> AuthorityFacts {
    AuthorityFacts {
        teams: BTreeSet::from(["review-team".to_owned()]),
        ..AuthorityFacts::default()
    }
}

fn create_input(key: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "key": key,
        "content": {"media_type": "text/plain", "text": "retained fixture content"},
        "scopes": {"team": "review-team"}
    }))
    .unwrap()
}

#[tokio::test]
async fn retry_binds_verified_identity_but_not_transport_or_json_member_order() {
    // ADR 0043 and README: the original input, user, authority and exact optional
    // executor belong to retry identity. Transport request IDs and current grants do not.
    let store: Arc<dyn EventStore> = Arc::new(
        SqliteEventStore::in_memory("sdk_adversary_identity")
            .await
            .unwrap(),
    );
    let service = EventlogService::initialize(
        store.clone(),
        persistence_http_generated_service::service().unwrap(),
    )
    .await
    .unwrap();
    let caller = verified("owner-a", "user-a", Some("executor-a"));
    let original = br#"{"key":"identity-key","content":{"media_type":"text/plain","text":"original retained body"},"scopes":{"team":"review-team"}}"#;
    let reordered = br#"{ "scopes": {"team": "review-team"}, "content": {"text": "original retained body", "media_type": "text/plain"}, "key": "identity-key" }"#;
    let first = service
        .intent(
            &caller,
            current_facts(),
            RequestMetadata {
                request_id: Some("transport-first"),
            },
            "create",
            original,
        )
        .await
        .unwrap();
    assert!(!first.replayed);
    assert_eq!(first.through_version, 1);
    let tenant = TenantId::new("adversary-tenant").unwrap();
    let original_events = store.read_feed(&tenant, 0, 100).await.unwrap().events;
    assert_eq!(original_events.len(), 1);

    for (label, changed) in [
        (
            "authority",
            verified("owner-b", "user-a", Some("executor-a")),
        ),
        ("user", verified("owner-a", "user-b", Some("executor-a"))),
        (
            "executor",
            verified("owner-a", "user-a", Some("executor-b")),
        ),
        ("absent executor", verified("owner-a", "user-a", None)),
    ] {
        let result = service
            .intent(
                &changed,
                current_facts(),
                RequestMetadata::default(),
                "create",
                original,
            )
            .await;
        assert!(
            result.is_err(),
            "changed {label} recovered a different identity's receipt"
        );
        assert_eq!(
            store.read_feed(&tenant, 0, 100).await.unwrap().events,
            original_events,
            "changed {label} committed another generated UUID"
        );
    }

    let mut refreshed = current_facts();
    refreshed
        .teams
        .insert("additional-current-grant".to_owned());
    let replay = service
        .intent(
            &caller,
            refreshed,
            RequestMetadata {
                request_id: Some("transport-retry"),
            },
            "create",
            reordered,
        )
        .await
        .expect("transport IDs, JSON member order and unrelated current grants must not change the original intent");
    assert!(replay.replayed);
    assert_eq!(replay.events, first.events);
    assert_eq!(replay.through_version, first.through_version);
    assert_eq!(
        store.read_feed(&tenant, 0, 100).await.unwrap().events,
        original_events
    );
    let digest = first.events[0].fields["content_ref"]
        .as_str()
        .unwrap()
        .strip_prefix("content:")
        .unwrap();
    assert_eq!(
        store.get_blob(&tenant, digest).await.unwrap().unwrap(),
        b"original retained body"
    );
}

#[tokio::test]
async fn operation_key_collision_refuses_and_old_receipt_survives_a_later_transition() {
    let store: Arc<dyn EventStore> = Arc::new(
        SqliteEventStore::in_memory("sdk_adversary_operation")
            .await
            .unwrap(),
    );
    let service = EventlogService::initialize(
        store.clone(),
        persistence_factory_generated_service::service().unwrap(),
    )
    .await
    .unwrap();
    let caller = verified("owner-a", "user-a", None);
    let created = service
        .intent(
            &caller,
            current_facts(),
            RequestMetadata::default(),
            "create",
            &create_input("create-key"),
        )
        .await
        .unwrap();
    let revision_input = serde_json::json!({
        "id": created.events[0].fields["id"], "key": "one-decision", "version": 1
    });
    let revision_body = serde_json::to_vec(&revision_input).unwrap();
    let revised = service
        .intent(
            &caller,
            current_facts(),
            RequestMetadata::default(),
            "revise",
            &revision_body,
        )
        .await
        .unwrap();
    assert_eq!(revised.through_version, 2);
    let tenant = TenantId::new("adversary-tenant").unwrap();
    let before_collision = store.read_feed(&tenant, 0, 100).await.unwrap().events;
    assert_eq!(before_collision.len(), 2);
    assert!(
        service
            .intent(
                &caller,
                current_facts(),
                RequestMetadata::default(),
                "close",
                &revision_body
            )
            .await
            .is_err(),
        "a different operation must not inherit an earlier receipt with identical input"
    );
    assert_eq!(
        store.read_feed(&tenant, 0, 100).await.unwrap().events,
        before_collision
    );

    let closing = serde_json::to_vec(&serde_json::json!({
        "id": created.events[0].fields["id"], "key": "new-close-key", "version": 2
    }))
    .unwrap();
    let closed = service
        .intent(
            &caller,
            current_facts(),
            RequestMetadata::default(),
            "close",
            &closing,
        )
        .await
        .unwrap();
    assert_eq!(closed.through_version, 3);
    let after_close = store.read_feed(&tenant, 0, 100).await.unwrap().events;
    let replay = service
        .intent(
            &caller,
            current_facts(),
            RequestMetadata::default(),
            "revise",
            &revision_body,
        )
        .await
        .expect("retry must recover its original decision without rerunning the Open precondition");
    assert!(replay.replayed);
    assert_eq!(replay.events, revised.events);
    assert_eq!(replay.outcome, revised.outcome);
    assert_eq!(replay.through_version, 2);
    assert_eq!(
        store.read_feed(&tenant, 0, 100).await.unwrap().events,
        after_close
    );
}

#[tokio::test]
async fn actual_redaction_refuses_original_receipt_without_new_generated_identity() {
    let store: Arc<dyn EventStore> = Arc::new(
        SqliteEventStore::in_memory("sdk_adversary_redaction")
            .await
            .unwrap(),
    );
    let service = EventlogService::initialize(
        store.clone(),
        persistence_http_generated_service::service().unwrap(),
    )
    .await
    .unwrap();
    let caller = verified("owner-a", "user-a", None);
    let body = create_input("redacted-original");
    service
        .intent(
            &caller,
            current_facts(),
            RequestMetadata::default(),
            "create",
            &body,
        )
        .await
        .unwrap();
    let tenant = TenantId::new("adversary-tenant").unwrap();
    let original = store.read_feed(&tenant, 0, 100).await.unwrap().events;
    assert_eq!(original.len(), 1);
    // A real owner-port redaction in this test's private in-memory database,
    // rather than a fabricated malformed receipt or modified implementation.
    let tombstone = store
        .redact(
            &original[0].stream().unwrap(),
            1,
            "synthetic review redaction",
        )
        .await
        .unwrap();
    assert!(tombstone.is_redacted());
    let after_redaction = store.read_feed(&tenant, 0, 100).await.unwrap().events;
    assert!(
        service
            .intent(
                &caller,
                current_facts(),
                RequestMetadata::default(),
                "create",
                &body
            )
            .await
            .is_err(),
        "a retained claim with redacted original evidence must not become a fresh Create"
    );
    assert_eq!(
        store.read_feed(&tenant, 0, 100).await.unwrap().events,
        after_redaction
    );
}

#[tokio::test]
async fn drain_closes_existing_service_and_store_handles_and_cannot_be_resealed() {
    let persistence = service_host::Persistence::open_sqlite(
        ":memory:",
        "sdk_adversary_drain",
        &["persistence_http"],
    )
    .await
    .unwrap();
    let store = persistence.store();
    let service = EventlogService::initialize(
        store.clone(),
        persistence_http_generated_service::service().unwrap(),
    )
    .await
    .unwrap();
    persistence.seal().await.unwrap();
    service.readiness().await.unwrap();
    let caller = verified("owner-a", "user-a", None);
    let body = create_input("before-drain");
    service
        .intent(
            &caller,
            current_facts(),
            RequestMetadata::default(),
            "create",
            &body,
        )
        .await
        .unwrap();
    let retained_service = service.clone();
    persistence.begin_drain();
    assert!(persistence.readiness().await.is_err());
    assert!(retained_service.readiness().await.is_err());
    assert!(persistence.seal().await.is_err());
    assert!(
        retained_service
            .intent(
                &caller,
                current_facts(),
                RequestMetadata::default(),
                "create",
                &body
            )
            .await
            .is_err(),
        "retained service handles must not replay after storage admission closes"
    );
    assert!(
        store
            .read_feed(&TenantId::new("adversary-tenant").unwrap(), 0, 100)
            .await
            .is_err()
    );
    persistence.shutdown().await.unwrap();
    assert!(persistence.seal().await.is_err());
    assert!(retained_service.readiness().await.is_err());
}
