use bodh::{
    DraftStore, OperationControl, PrIdentity, PreparationStore, RemoteReviewState, ReviewContext,
    ReviewEvent, ReviewIntent, ReviewPublicationStore, ReviewPublisher, Revision, SelectedDraft,
    SourceRef, StageReviewRequest, suggestion_markdown,
};
use std::{
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn github_suggestion_markdown_is_built_from_structured_replacement_text() {
    assert_eq!(
        suggestion_markdown("    let value = 3;\n\n").unwrap(),
        "```suggestion\n    let value = 3;\n\n```"
    );
}

#[test]
fn confirmed_replacement_publishes_exact_suggestion_and_only_then_removes_draft() {
    let f = Fixture::new();
    let source = SourceRef {
        path: "a.rs".into(),
        revision: Revision::Head,
        start_line: 2,
        end_line: 2,
    };
    let replacement = DraftStore::open(&f.state)
        .unwrap()
        .save_replacement(
            &f.prep,
            None,
            &source,
            "    2\n",
            "    3\n",
            1,
            0,
            &OperationControl::default(),
        )
        .unwrap()
        .draft;
    let snapshot = DraftStore::open(&f.state).unwrap().list(&f.prep).unwrap();
    let intent = ReviewPublicationStore::open(&f.state)
        .unwrap()
        .stage(StageReviewRequest {
            state: f.state.clone(),
            preparation_id: f.prep.clone(),
            expected_workspace_revision: snapshot.workspace_revision,
            expected_intent_revision: 0,
            selected: vec![],
            selected_replacements: vec![SelectedDraft {
                id: replacement.id.clone(),
                revision: replacement.revision,
            }],
            summary: None,
            event: Some(ReviewEvent::Comment),
        })
        .unwrap();
    let remote = RemoteReviewState {
        pull_request_id: "PR_node".into(),
        head_sha: git(&f.repo, &["rev-parse", "HEAD"]),
        base_sha: git(&f.repo, &["rev-parse", "HEAD~1"]),
        state: "OPEN".into(),
        viewer_permission: "WRITE".into(),
    };
    let plan = ReviewPublicationStore::open(&f.state)
        .unwrap()
        .confirm(
            &f.prep,
            &f.state,
            intent.revision,
            &remote,
            &OperationControl::default(),
        )
        .unwrap();
    assert_eq!(
        plan.payload.suggestions[0].body,
        "```suggestion\n    3\n```"
    );
    assert_eq!(plan.payload.suggestions[0].source, source);
    let capture = f.root.join("suggestion-request.json");
    let fake = fake_gh(
        &f.root.join("suggestion-gh"),
        &format!(
            "cat > '{}'\nprintf '%s' '{{\"data\":{{\"addPullRequestReview\":{{\"pullRequestReview\":{{\"id\":\"PRR_SUGGEST\",\"url\":\"https://example.invalid/review/2\"}}}}}}}}'",
            capture.display()
        ),
    );
    let publisher = bodh::GhReviewPublisher {
        executable: fake,
        checkout: f.repo.clone(),
        timeout: std::time::Duration::from_secs(2),
    };
    let result = ReviewPublicationStore::open(&f.state)
        .unwrap()
        .publish_confirmed(&plan.id, &remote, &publisher, &OperationControl::default())
        .unwrap();
    assert_eq!(result.state, "confirmed");
    let sent: serde_json::Value = serde_json::from_slice(&std::fs::read(capture).unwrap()).unwrap();
    assert_eq!(
        sent["variables"]["threads"][0]["body"],
        "```suggestion\n    3\n```"
    );
    assert_eq!(sent["variables"]["threads"][0]["line"], 2);
    assert_eq!(sent["variables"]["threads"][0]["side"], "RIGHT");
    let remaining = DraftStore::open(&f.state)
        .unwrap()
        .list_replacements(&f.prep)
        .unwrap();
    assert!(remaining.drafts.is_empty());
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
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
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    state: PathBuf,
    prep: String,
    draft_id: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "bodh-manual-review-{}-{}",
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
            context_id: "review-ctx".into(),
            base_sha: base.clone(),
            head_sha: head,
            merge_base_sha: base,
        };
        let index = bodh::diff_index(&repo, &context, &OperationControl::default()).unwrap();
        let binding = PreparationStore::open(&state)
            .unwrap()
            .save(
                &context,
                repo.to_str().unwrap(),
                "Fixture PR",
                None,
                &index.files,
            )
            .unwrap();
        let source = SourceRef {
            path: "a.rs".into(),
            revision: Revision::Head,
            start_line: 2,
            end_line: 2,
        };
        let draft = DraftStore::open(&state)
            .unwrap()
            .save(
                &binding.preparation_id,
                None,
                "Please use a named constant.",
                &source,
                0,
                0,
                &OperationControl::default(),
            )
            .unwrap()
            .draft;
        Self {
            root,
            repo,
            state,
            prep: binding.preparation_id,
            draft_id: draft.id,
        }
    }
    fn stage(&self, event: Option<ReviewEvent>) -> ReviewIntent {
        ReviewPublicationStore::open(&self.state)
            .unwrap()
            .stage(StageReviewRequest {
                state: self.state.clone(),
                preparation_id: self.prep.clone(),
                expected_workspace_revision: 1,
                expected_intent_revision: 0,
                selected: vec![SelectedDraft {
                    id: self.draft_id.clone(),
                    revision: 1,
                }],
                selected_replacements: vec![],
                summary: Some("Please address this before merge.".into()),
                event,
            })
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn fake_gh(path: &Path, script: &str) -> PathBuf {
    std::fs::write(path, format!("#!/bin/sh\n{script}\n")).unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
    path.to_owned()
}
fn cli(args: &[&str], input: Option<&serde_json::Value>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bodh"))
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(input).unwrap())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

#[test]
fn event_must_be_chosen_and_confirmed_payload_binds_exact_draft_revision_and_head() {
    let f = Fixture::new();
    let intent = f.stage(None);
    assert_eq!(intent.event, None);
    let mut store = ReviewPublicationStore::open(&f.state).unwrap();
    let remote = RemoteReviewState {
        pull_request_id: "PR_node".into(),
        head_sha: git(&f.repo, &["rev-parse", "HEAD"]),
        base_sha: git(&f.repo, &["rev-parse", "HEAD~1"]),
        state: "OPEN".into(),
        viewer_permission: "WRITE".into(),
    };
    let error = store
        .confirm(
            &f.prep,
            &f.state,
            intent.revision,
            &remote,
            &OperationControl::default(),
        )
        .unwrap_err();
    assert_eq!(error.code, "REVIEW_PLAN_INVALID");
    let intent = store
        .stage(StageReviewRequest {
            state: f.state.clone(),
            preparation_id: f.prep.clone(),
            expected_workspace_revision: 1,
            expected_intent_revision: intent.revision,
            selected: vec![SelectedDraft {
                id: f.draft_id.clone(),
                revision: 1,
            }],
            selected_replacements: vec![],
            summary: Some("Please address this before merge.".into()),
            event: Some(ReviewEvent::RequestChanges),
        })
        .unwrap();
    let operation = store
        .confirm(
            &f.prep,
            &f.state,
            intent.revision,
            &remote,
            &OperationControl::default(),
        )
        .unwrap();
    assert_eq!(operation.state, "planned");
    assert_eq!(operation.payload.comments[0].draft_id, f.draft_id);
    assert_eq!(operation.payload.comments[0].draft_revision, 1);
    assert_eq!(operation.payload.head_sha, remote.head_sha);
}

#[test]
fn interrupted_send_becomes_unknown_and_cannot_be_started_again() {
    let f = Fixture::new();
    let mut store = ReviewPublicationStore::open(&f.state).unwrap();
    let intent = store
        .stage(StageReviewRequest {
            state: f.state.clone(),
            preparation_id: f.prep.clone(),
            expected_workspace_revision: 1,
            expected_intent_revision: 0,
            selected: vec![SelectedDraft {
                id: f.draft_id.clone(),
                revision: 1,
            }],
            selected_replacements: vec![],
            summary: Some("A summary".into()),
            event: Some(ReviewEvent::Comment),
        })
        .unwrap();
    let remote = RemoteReviewState {
        pull_request_id: "PR_node".into(),
        head_sha: git(&f.repo, &["rev-parse", "HEAD"]),
        base_sha: git(&f.repo, &["rev-parse", "HEAD~1"]),
        state: "OPEN".into(),
        viewer_permission: "WRITE".into(),
    };
    let plan = store
        .confirm(
            &f.prep,
            &f.state,
            intent.revision,
            &remote,
            &OperationControl::default(),
        )
        .unwrap();
    let sending = store.begin_send(&plan.id, &remote).unwrap();
    let accepted = fake_gh(
        &f.root.join("accepted-before-crash"),
        "cat >/dev/null\nprintf '%s' '{\"data\":{\"addPullRequestReview\":{\"pullRequestReview\":{\"id\":\"PRR_ACCEPTED\",\"url\":\"https://example.invalid/review/1\"}}}}'",
    );
    let publisher = bodh::GhReviewPublisher {
        executable: accepted,
        checkout: f.repo.clone(),
        timeout: std::time::Duration::from_secs(1),
    };
    assert!(matches!(
        publisher.publish(&sending.payload, &OperationControl::default()),
        bodh::SendOutcome::Confirmed { .. }
    ));
    drop(store);
    let mut reopened = ReviewPublicationStore::open(&f.state).unwrap();
    let interrupted = reopened.recover_interrupted(&f.prep).unwrap().unwrap();
    assert_eq!(interrupted.state, "unknown");
    assert_eq!(
        reopened.pending_operation(&f.prep).unwrap().unwrap().id,
        plan.id
    );
    assert_eq!(
        reopened.begin_send(&plan.id, &remote).unwrap_err().code,
        "REVIEW_SEND_NOT_ALLOWED"
    );
}

#[test]
fn changed_head_or_permission_after_confirmation_blocks_send_without_starting_journal() {
    let f = Fixture::new();
    let mut store = ReviewPublicationStore::open(&f.state).unwrap();
    let intent = store
        .stage(StageReviewRequest {
            state: f.state.clone(),
            preparation_id: f.prep.clone(),
            expected_workspace_revision: 1,
            expected_intent_revision: 0,
            selected: vec![SelectedDraft {
                id: f.draft_id.clone(),
                revision: 1,
            }],
            selected_replacements: vec![],
            summary: Some("A summary".into()),
            event: Some(ReviewEvent::Comment),
        })
        .unwrap();
    let remote = RemoteReviewState {
        pull_request_id: "PR_node".into(),
        head_sha: git(&f.repo, &["rev-parse", "HEAD"]),
        base_sha: git(&f.repo, &["rev-parse", "HEAD~1"]),
        state: "OPEN".into(),
        viewer_permission: "WRITE".into(),
    };
    let plan = store
        .confirm(
            &f.prep,
            &f.state,
            intent.revision,
            &remote,
            &OperationControl::default(),
        )
        .unwrap();
    let mut changed = remote.clone();
    changed.head_sha = "f".repeat(40);
    assert_eq!(
        store.begin_send(&plan.id, &changed).unwrap_err().code,
        "PR_HEAD_MOVED"
    );
    assert_eq!(store.operation(&plan.id).unwrap().state, "planned");
    let mut denied = remote;
    denied.viewer_permission = "READ".into();
    assert_eq!(
        store.begin_send(&plan.id, &denied).unwrap_err().code,
        "REVIEW_PERMISSION_DENIED"
    );
}

#[test]
fn controlled_gh_receives_exact_confirmed_group_and_returns_persisted_review_link() {
    let f = Fixture::new();
    let capture = f.root.join("request.json");
    let fake = fake_gh(
        &f.root.join("fake-gh"),
        &format!(
            "cat > '{}'\nprintf '%s' '{{\"data\":{{\"addPullRequestReview\":{{\"pullRequestReview\":{{\"id\":\"PRR_42\",\"url\":\"https://github.com/acme/widget/pull/7#pullrequestreview-42\",\"state\":\"SUBMITTED\"}}}}}}}}'",
            capture.display()
        ),
    );
    let mut store = ReviewPublicationStore::open(&f.state).unwrap();
    let intent = store
        .stage(StageReviewRequest {
            state: f.state.clone(),
            preparation_id: f.prep.clone(),
            expected_workspace_revision: 1,
            expected_intent_revision: 0,
            selected: vec![SelectedDraft {
                id: f.draft_id.clone(),
                revision: 1,
            }],
            selected_replacements: vec![],
            summary: Some("A summary".into()),
            event: Some(ReviewEvent::Comment),
        })
        .unwrap();
    let remote = RemoteReviewState {
        pull_request_id: "PR_node".into(),
        head_sha: git(&f.repo, &["rev-parse", "HEAD"]),
        base_sha: git(&f.repo, &["rev-parse", "HEAD~1"]),
        state: "OPEN".into(),
        viewer_permission: "WRITE".into(),
    };
    let plan = store
        .confirm(
            &f.prep,
            &f.state,
            intent.revision,
            &remote,
            &OperationControl::default(),
        )
        .unwrap();
    let publisher = bodh::GhReviewPublisher {
        executable: fake,
        checkout: f.repo.clone(),
        timeout: std::time::Duration::from_secs(2),
    };
    let result = store
        .publish_confirmed(&plan.id, &remote, &publisher, &OperationControl::default())
        .unwrap();
    assert_eq!(result.state, "confirmed");
    assert_eq!(result.remote_review_id.as_deref(), Some("PRR_42"));
    assert!(
        result
            .remote_url
            .as_deref()
            .unwrap()
            .contains("pullrequestreview-42")
    );
    let sent: serde_json::Value = serde_json::from_slice(&std::fs::read(capture).unwrap()).unwrap();
    assert_eq!(sent["variables"]["pullRequestId"], "PR_node");
    assert_eq!(sent["variables"]["commitOID"], remote.head_sha);
    assert_eq!(sent["variables"]["event"], "COMMENT");
    assert_eq!(sent["variables"]["threads"][0]["path"], "a.rs");
    assert_eq!(sent["variables"]["threads"][0]["side"], "RIGHT");
    assert_eq!(
        ReviewPublicationStore::open(&f.state)
            .unwrap()
            .operation(&plan.id)
            .unwrap(),
        result
    );
    let mut drafts = DraftStore::open(&f.state).unwrap();
    let published = drafts.list(&f.prep).unwrap();
    assert!(published.drafts.is_empty());
    assert_eq!(published.counts.inline_comments, 0);
    assert_eq!(published.counts.review_comments, 0);
    let revised = drafts
        .save(
            &f.prep,
            Some(&f.draft_id),
            "Follow-up edit after publication.",
            &SourceRef {
                path: "a.rs".into(),
                revision: Revision::Head,
                start_line: 2,
                end_line: 2,
            },
            published.workspace_revision,
            1,
            &OperationControl::default(),
        )
        .unwrap();
    assert_eq!(revised.draft.revision, 2);
    assert_eq!(drafts.list(&f.prep).unwrap().drafts, [revised.draft]);
    let current = drafts.list(&f.prep).unwrap();
    let intent = ReviewPublicationStore::open(&f.state)
        .unwrap()
        .intent(&f.prep)
        .unwrap();
    let mut publication = ReviewPublicationStore::open(&f.state).unwrap();
    publication
        .stage(StageReviewRequest {
            state: f.state.clone(),
            preparation_id: f.prep.clone(),
            expected_workspace_revision: current.workspace_revision,
            expected_intent_revision: intent.revision,
            selected: vec![SelectedDraft {
                id: f.draft_id.clone(),
                revision: 2,
            }],
            selected_replacements: vec![],
            summary: Some("A revised summary.".into()),
            event: Some(ReviewEvent::Comment),
        })
        .unwrap();
    assert_eq!(
        DraftStore::open(&f.state)
            .unwrap()
            .list(&f.prep)
            .unwrap()
            .counts
            .review_comments,
        1
    );
}

#[test]
fn rejection_is_definitive_but_timeout_after_request_is_unknown_and_not_retryable() {
    let f = Fixture::new();
    let mut store = ReviewPublicationStore::open(&f.state).unwrap();
    let intent = store
        .stage(StageReviewRequest {
            state: f.state.clone(),
            preparation_id: f.prep.clone(),
            expected_workspace_revision: 1,
            expected_intent_revision: 0,
            selected: vec![SelectedDraft {
                id: f.draft_id.clone(),
                revision: 1,
            }],
            selected_replacements: vec![],
            summary: Some("Summary".into()),
            event: Some(ReviewEvent::Comment),
        })
        .unwrap();
    let remote = RemoteReviewState {
        pull_request_id: "PR_node".into(),
        head_sha: git(&f.repo, &["rev-parse", "HEAD"]),
        base_sha: git(&f.repo, &["rev-parse", "HEAD~1"]),
        state: "OPEN".into(),
        viewer_permission: "WRITE".into(),
    };
    let rejected = store
        .confirm(
            &f.prep,
            &f.state,
            intent.revision,
            &remote,
            &OperationControl::default(),
        )
        .unwrap();
    let fake = fake_gh(
        &f.root.join("reject-gh"),
        "cat >/dev/null\necho 'GraphQL: Resource not accessible by integration' >&2\nexit 1",
    );
    let publisher = bodh::GhReviewPublisher {
        executable: fake,
        checkout: f.repo.clone(),
        timeout: std::time::Duration::from_secs(1),
    };
    let operation = store
        .publish_confirmed(
            &rejected.id,
            &remote,
            &publisher,
            &OperationControl::default(),
        )
        .unwrap();
    assert_eq!(operation.state, "failed");

    let intent = store
        .stage(StageReviewRequest {
            state: f.state.clone(),
            preparation_id: f.prep.clone(),
            expected_workspace_revision: 1,
            expected_intent_revision: intent.revision,
            selected: vec![SelectedDraft {
                id: f.draft_id.clone(),
                revision: 1,
            }],
            selected_replacements: vec![],
            summary: Some("Summary updated".into()),
            event: Some(ReviewEvent::Comment),
        })
        .unwrap();
    let timed = store
        .confirm(
            &f.prep,
            &f.state,
            intent.revision,
            &remote,
            &OperationControl::default(),
        )
        .unwrap();
    let fake = fake_gh(
        &f.root.join("timeout-gh"),
        "cat >/dev/null\nsleep 2\nprintf '%s' '{\"data\":{\"addPullRequestReview\":{\"pullRequestReview\":{\"id\":\"accepted-but-not-observed\"}}}}'",
    );
    let publisher = bodh::GhReviewPublisher {
        executable: fake,
        checkout: f.repo.clone(),
        timeout: std::time::Duration::from_millis(80),
    };
    let operation = store
        .publish_confirmed(&timed.id, &remote, &publisher, &OperationControl::default())
        .unwrap();
    assert_eq!(operation.state, "unknown");
    assert_eq!(
        store
            .publish_confirmed(&timed.id, &remote, &publisher, &OperationControl::default())
            .unwrap_err()
            .code,
        "REVIEW_SEND_NOT_ALLOWED"
    );
}

#[test]
fn remote_preflight_uses_json_fields_and_retains_current_permission_observation() {
    let f = Fixture::new();
    let binding = PreparationStore::open(&f.state)
        .unwrap()
        .load(&f.prep)
        .unwrap();
    let fake = fake_gh(
        &f.root.join("preflight-gh"),
        "printf '%s' '{\"id\":\"PR_node\",\"headRefOid\":\"head\",\"baseRefOid\":\"base\",\"state\":\"OPEN\",\"viewerPermission\":\"WRITE\"}'",
    );
    let remote =
        bodh::inspect_remote_review_with_gh(&binding, &fake, &OperationControl::default()).unwrap();
    assert_eq!(remote.pull_request_id, "PR_node");
    assert_eq!(remote.viewer_permission, "WRITE");
    assert_eq!(remote.state, "OPEN");
}

#[test]
fn cli_stage_and_intent_round_trip_preserve_explicit_missing_event() {
    let f = Fixture::new();
    let state = f.state.to_str().unwrap();
    let prep = f.prep.as_str();
    let initial = cli(
        &["review", "intent", "--prepared", prep, "--state", state],
        None,
    );
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    let initial: serde_json::Value = serde_json::from_slice(&initial.stdout).unwrap();
    assert_eq!(initial["data"]["event"], serde_json::Value::Null);
    let request = serde_json::json!({"expected_workspace_revision":1,"expected_intent_revision":0,"selected":[{"id":f.draft_id,"revision":1}],"summary":"Review this carefully.","event":null});
    let staged = cli(
        &[
            "review",
            "stage",
            "--prepared",
            prep,
            "--state",
            state,
            "--input",
            "-",
        ],
        Some(&request),
    );
    assert!(
        staged.status.success(),
        "{}",
        String::from_utf8_lossy(&staged.stderr)
    );
    let staged: serde_json::Value = serde_json::from_slice(&staged.stdout).unwrap();
    assert_eq!(staged["data"]["revision"], 1);
    assert_eq!(staged["data"]["event"], serde_json::Value::Null);
    let loaded = cli(
        &["review", "intent", "--prepared", prep, "--state", state],
        None,
    );
    let loaded: serde_json::Value = serde_json::from_slice(&loaded.stdout).unwrap();
    assert_eq!(loaded["data"]["selected"][0]["revision"], 1);
    assert_eq!(loaded["data"]["summary"], "Review this carefully.");
    assert_eq!(
        DraftStore::open(&f.state)
            .unwrap()
            .list(&f.prep)
            .unwrap()
            .counts
            .review_comments,
        1
    );
    let mut publication = ReviewPublicationStore::open(&f.state).unwrap();
    publication
        .stage(StageReviewRequest {
            state: f.state.clone(),
            preparation_id: f.prep.clone(),
            expected_workspace_revision: 1,
            expected_intent_revision: 1,
            selected: vec![SelectedDraft {
                id: f.draft_id.clone(),
                revision: 1,
            }],
            selected_replacements: vec![],
            summary: None,
            event: None,
        })
        .unwrap();
    assert_eq!(
        DraftStore::open(&f.state)
            .unwrap()
            .list(&f.prep)
            .unwrap()
            .counts
            .review_comments,
        0
    );
}
