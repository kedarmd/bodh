use bodh::{
    DiscussionPublicationStore, PrIdentity, PreparationStore, ReviewContext, ReviewPublicationStore,
};
use rusqlite::{Connection, params};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn state_path() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "bodh-publication-history-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    root.join("state.sqlite")
}

fn create_preparation(state: &PathBuf) -> String {
    PreparationStore::open(state)
        .unwrap()
        .save(
            &ReviewContext {
                pr: PrIdentity {
                    host: "github.com".into(),
                    owner: "acme".into(),
                    repository: "widget".into(),
                    number: 17,
                },
                context_id: "history-context".into(),
                base_sha: "a".repeat(40),
                head_sha: "b".repeat(40),
                merge_base_sha: "c".repeat(40),
            },
            "/checkout",
            "PR title",
            None,
            &[],
        )
        .unwrap()
        .preparation_id
}

#[test]
fn history_readers_return_empty_for_preparations_without_publications() {
    let state = state_path();
    let reviews = ReviewPublicationStore::open(&state).unwrap();
    let discussions = DiscussionPublicationStore::open(&state).unwrap();

    assert!(
        reviews
            .operation_history("missing-preparation")
            .unwrap()
            .is_empty()
    );
    assert!(
        discussions
            .plan_history("missing-preparation")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn history_readers_restore_persisted_operation_and_plan_after_reopen() {
    let state = state_path();
    let preparation_id = create_preparation(&state);
    let _reviews = ReviewPublicationStore::open(&state).unwrap();
    let _discussions = DiscussionPublicationStore::open(&state).unwrap();
    let payload = serde_json::json!({
        "preparation_id": preparation_id,
        "context_id": "history-context",
        "pull_request_id": "PR_node",
        "workspace_revision": 1,
        "intent_revision": 2,
        "head_sha": "b".repeat(40),
        "base_sha": "a".repeat(40),
        "event": "COMMENT",
        "summary": "Persisted summary",
        "comments": [],
        "suggestions": []
    });
    let db = Connection::open(&state).unwrap();
    db.execute(
        "INSERT INTO review_operations(operation_id,preparation_id,fingerprint,state,payload_json,created_at,updated_at) VALUES(?1,?2,?3,'failed',?4,100,100)",
        params!["review-1", preparation_id, "fingerprint-1", payload.to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO discussion_publication_plans(plan_id,preparation_id,context_id,intent_revision,fingerprint,state,created_at,updated_at) VALUES('plan-1',?1,'history-context',2,'fingerprint-plan','failed',101,101)",
        [&preparation_id],
    )
    .unwrap();
    db.execute(
        "INSERT INTO discussion_publication_steps(step_id,plan_id,ordinal,kind,draft_id,draft_revision,thread_id,body,state,created_at,updated_at) VALUES('step-1','plan-1',0,'reply','reply-1',3,'thread-1','reply payload','failed',101,101)",
        [],
    )
    .unwrap();
    drop(db);

    let review_history = ReviewPublicationStore::open(&state)
        .unwrap()
        .operation_history(&preparation_id)
        .unwrap();
    let discussion_history = DiscussionPublicationStore::open(&state)
        .unwrap()
        .plan_history(&preparation_id)
        .unwrap();

    assert_eq!(review_history.len(), 1);
    assert_eq!(review_history[0].id, "review-1");
    assert_eq!(review_history[0].state, "failed");
    assert_eq!(
        review_history[0].payload.summary.as_deref(),
        Some("Persisted summary")
    );
    assert_eq!(discussion_history.len(), 1);
    assert_eq!(discussion_history[0].id, "plan-1");
    assert_eq!(discussion_history[0].state, "failed");
    assert_eq!(
        discussion_history[0].steps[0].body.as_deref(),
        Some("reply payload")
    );
}
