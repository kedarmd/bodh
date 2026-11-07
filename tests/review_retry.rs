use bodh::{
    DraftStore, OperationControl, PrIdentity, PreparationStore, RemoteReviewState, ReviewContext,
    ReviewEvent, ReviewPublicationStore, Revision, SelectedDraft, SourceRef, StageReviewRequest,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    state: PathBuf,
    preparation_id: String,
    draft_id: String,
    remote: RemoteReviewState,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "bodh-review-retry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = root.join("repo");
        let state = root.join("state.sqlite");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("a.rs"), "fn value() {\n    1\n}\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "base"]);
        let base = git(&repo, &["rev-parse", "HEAD"]);
        std::fs::write(repo.join("a.rs"), "fn value() {\n    2\n}\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "head"]);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        let context = ReviewContext {
            pr: PrIdentity {
                host: "github.com".into(),
                owner: "acme".into(),
                repository: "widget".into(),
                number: 7,
            },
            context_id: "retry-context".into(),
            base_sha: base.clone(),
            head_sha: head.clone(),
            merge_base_sha: base.clone(),
        };
        let index = bodh::diff_index(&repo, &context, &OperationControl::default()).unwrap();
        let binding = PreparationStore::open(&state)
            .unwrap()
            .save(
                &context,
                repo.to_str().unwrap(),
                "Fixture",
                None,
                &index.files,
            )
            .unwrap();
        let draft = DraftStore::open(&state)
            .unwrap()
            .save(
                &binding.preparation_id,
                None,
                "Please preserve this check.",
                &SourceRef {
                    path: "a.rs".into(),
                    revision: Revision::Head,
                    start_line: 2,
                    end_line: 2,
                },
                0,
                0,
                &OperationControl::default(),
            )
            .unwrap()
            .draft;
        Self {
            root,
            state,
            preparation_id: binding.preparation_id,
            draft_id: draft.id,
            remote: RemoteReviewState {
                pull_request_id: "PR_node".into(),
                head_sha: head,
                base_sha: base,
                state: "OPEN".into(),
                viewer_permission: "WRITE".into(),
            },
        }
    }

    fn failed_operation(&self) -> bodh::ReviewOperation {
        let snapshot = DraftStore::open(&self.state)
            .unwrap()
            .list(&self.preparation_id)
            .unwrap();
        let intent = ReviewPublicationStore::open(&self.state)
            .unwrap()
            .stage(StageReviewRequest {
                state: self.state.clone(),
                preparation_id: self.preparation_id.clone(),
                expected_workspace_revision: snapshot.workspace_revision,
                expected_intent_revision: 0,
                selected: vec![SelectedDraft {
                    id: self.draft_id.clone(),
                    revision: 1,
                }],
                selected_replacements: vec![],
                selected_replies: vec![],
                selected_thread_actions: vec![],
                summary: Some("Please address this.".into()),
                event: Some(ReviewEvent::RequestChanges),
            })
            .unwrap();
        let mut store = ReviewPublicationStore::open(&self.state).unwrap();
        let operation = store
            .confirm(
                &self.preparation_id,
                &self.state,
                intent.revision,
                &self.remote,
                &OperationControl::default(),
            )
            .unwrap();
        store.begin_send(&operation.id, &self.remote).unwrap();
        store
            .finish_send(
                &operation.id,
                bodh::SendOutcome::Rejected {
                    message: "GitHub rejected the review".into(),
                },
            )
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[test]
fn a_definitive_rejection_can_be_retried_as_a_new_immutable_operation() {
    let fixture = Fixture::new();
    let rejected = fixture.failed_operation();
    let mut reopened = ReviewPublicationStore::open(&fixture.state).unwrap();
    let retry = reopened
        .retry_failed(
            &rejected.id,
            &fixture.state,
            &fixture.remote,
            &OperationControl::default(),
        )
        .unwrap();

    assert_ne!(retry.id, rejected.id);
    assert_eq!(retry.state, "planned");
    assert_eq!(retry.fingerprint, rejected.fingerprint);
    assert_eq!(retry.payload, rejected.payload);

    let history = ReviewPublicationStore::open(&fixture.state)
        .unwrap()
        .operation_history(&fixture.preparation_id)
        .unwrap();
    assert_eq!(history.len(), 2);
    assert!(
        history
            .iter()
            .any(|op| op.id == rejected.id && op.state == "failed")
    );
    assert!(
        history
            .iter()
            .any(|op| op.id == retry.id && op.state == "planned")
    );
}

#[test]
fn unknown_operations_cannot_be_retried_and_cannot_become_duplicate_attempts() {
    let fixture = Fixture::new();
    let failed = fixture.failed_operation();
    let mut store = ReviewPublicationStore::open(&fixture.state).unwrap();
    let retry = store
        .retry_failed(
            &failed.id,
            &fixture.state,
            &fixture.remote,
            &OperationControl::default(),
        )
        .unwrap();

    let duplicate = store
        .retry_failed(
            &failed.id,
            &fixture.state,
            &fixture.remote,
            &OperationControl::default(),
        )
        .unwrap_err();
    assert_eq!(duplicate.code, "REVIEW_RETRY_ALREADY_CREATED");

    store.begin_send(&retry.id, &fixture.remote).unwrap();
    store
        .finish_send(
            &retry.id,
            bodh::SendOutcome::Uncertain {
                message: "response was lost".into(),
            },
        )
        .unwrap();
    let unknown = store
        .retry_failed(
            &retry.id,
            &fixture.state,
            &fixture.remote,
            &OperationControl::default(),
        )
        .unwrap_err();
    assert_eq!(unknown.code, "REVIEW_RETRY_NOT_ALLOWED");
}

#[test]
fn retry_rechecks_current_pr_and_draft_revisions() {
    let fixture = Fixture::new();
    let failed = fixture.failed_operation();
    let mut moved = fixture.remote.clone();
    moved.head_sha = "moved-head".into();
    let moved_head = ReviewPublicationStore::open(&fixture.state)
        .unwrap()
        .retry_failed(
            &failed.id,
            &fixture.state,
            &moved,
            &OperationControl::default(),
        )
        .unwrap_err();
    assert_eq!(moved_head.code, "PR_HEAD_MOVED");

    let mut denied = fixture.remote.clone();
    denied.viewer_permission = "READ".into();
    let permission_denied = ReviewPublicationStore::open(&fixture.state)
        .unwrap()
        .retry_failed(
            &failed.id,
            &fixture.state,
            &denied,
            &OperationControl::default(),
        )
        .unwrap_err();
    assert_eq!(permission_denied.code, "REVIEW_PERMISSION_DENIED");

    DraftStore::open(&fixture.state)
        .unwrap()
        .save(
            &fixture.preparation_id,
            Some(&fixture.draft_id),
            "Edited after rejection.",
            &SourceRef {
                path: "a.rs".into(),
                revision: Revision::Head,
                start_line: 2,
                end_line: 2,
            },
            1,
            1,
            &OperationControl::default(),
        )
        .unwrap();
    let stale = ReviewPublicationStore::open(&fixture.state)
        .unwrap()
        .retry_failed(
            &failed.id,
            &fixture.state,
            &fixture.remote,
            &OperationControl::default(),
        )
        .unwrap_err();
    assert_eq!(stale.code, "DRAFT_REVISION_STALE");
}

#[test]
fn retry_excludes_draft_revisions_already_confirmed_by_another_operation() {
    let fixture = Fixture::new();
    let rejected = fixture.failed_operation();
    let snapshot = DraftStore::open(&fixture.state)
        .unwrap()
        .list(&fixture.preparation_id)
        .unwrap();
    let mut store = ReviewPublicationStore::open(&fixture.state).unwrap();
    let next_intent = store
        .stage(StageReviewRequest {
            state: fixture.state.clone(),
            preparation_id: fixture.preparation_id.clone(),
            expected_workspace_revision: snapshot.workspace_revision,
            expected_intent_revision: 1,
            selected: vec![SelectedDraft {
                id: fixture.draft_id.clone(),
                revision: 1,
            }],
            selected_replacements: vec![],
            selected_replies: vec![],
            selected_thread_actions: vec![],
            summary: Some("A separate exact review.".into()),
            event: Some(ReviewEvent::Comment),
        })
        .unwrap();
    let confirmed = store
        .confirm(
            &fixture.preparation_id,
            &fixture.state,
            next_intent.revision,
            &fixture.remote,
            &OperationControl::default(),
        )
        .unwrap();
    store.begin_send(&confirmed.id, &fixture.remote).unwrap();
    store
        .finish_send(
            &confirmed.id,
            bodh::SendOutcome::Confirmed {
                remote_id: "PRR_1".into(),
                url: Some("https://github.com/acme/widget/pull/7".into()),
            },
        )
        .unwrap();
    let already_confirmed = store
        .retry_failed(
            &rejected.id,
            &fixture.state,
            &fixture.remote,
            &OperationControl::default(),
        )
        .unwrap_err();
    assert_eq!(already_confirmed.code, "REVIEW_DRAFT_ALREADY_CONFIRMED");
}

#[test]
fn a_rejected_replacement_retry_keeps_its_exact_suggestion_and_anchor() {
    let fixture = Fixture::new();
    let source = SourceRef {
        path: "a.rs".into(),
        revision: Revision::Head,
        start_line: 2,
        end_line: 2,
    };
    let snapshot = DraftStore::open(&fixture.state)
        .unwrap()
        .list(&fixture.preparation_id)
        .unwrap();
    let replacement = DraftStore::open(&fixture.state)
        .unwrap()
        .save_replacement(
            &fixture.preparation_id,
            None,
            &source,
            "    2\n",
            "    3\n",
            snapshot.workspace_revision,
            0,
            &OperationControl::default(),
        )
        .unwrap()
        .draft;
    let snapshot = DraftStore::open(&fixture.state)
        .unwrap()
        .list(&fixture.preparation_id)
        .unwrap();
    let intent = ReviewPublicationStore::open(&fixture.state)
        .unwrap()
        .stage(StageReviewRequest {
            state: fixture.state.clone(),
            preparation_id: fixture.preparation_id.clone(),
            expected_workspace_revision: snapshot.workspace_revision,
            expected_intent_revision: 0,
            selected: vec![],
            selected_replacements: vec![SelectedDraft {
                id: replacement.id,
                revision: 1,
            }],
            selected_replies: vec![],
            selected_thread_actions: vec![],
            summary: Some("Please use this replacement.".into()),
            event: Some(ReviewEvent::RequestChanges),
        })
        .unwrap();
    let mut store = ReviewPublicationStore::open(&fixture.state).unwrap();
    let operation = store
        .confirm(
            &fixture.preparation_id,
            &fixture.state,
            intent.revision,
            &fixture.remote,
            &OperationControl::default(),
        )
        .unwrap();
    store.begin_send(&operation.id, &fixture.remote).unwrap();
    let rejected = store
        .finish_send(
            &operation.id,
            bodh::SendOutcome::Rejected {
                message: "GitHub rejected the review".into(),
            },
        )
        .unwrap();
    let retry = ReviewPublicationStore::open(&fixture.state)
        .unwrap()
        .retry_failed(
            &rejected.id,
            &fixture.state,
            &fixture.remote,
            &OperationControl::default(),
        )
        .unwrap();

    assert_eq!(retry.payload, rejected.payload);
    assert_eq!(retry.payload.suggestions.len(), 1);
    assert_eq!(retry.payload.suggestions[0].source, source);
    assert_eq!(retry.payload.suggestions[0].original_text, "    2\n");
    assert_eq!(retry.payload.suggestions[0].replacement, "    3\n");
    let _ = std::fs::remove_dir_all(&fixture.root);
}
