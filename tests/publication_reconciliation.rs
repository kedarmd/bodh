use bodh::{
    DraftStore, GhReviewOutcomeReader, OperationControl, PrIdentity, PreparationStore,
    RemoteReviewState, ReviewContext, ReviewEvent, ReviewPublicationStore, Revision, SelectedDraft,
    SendOutcome, SourceRef, StageReviewRequest,
};
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    state: PathBuf,
    prep: String,
    draft_id: String,
    head: String,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "bodh-review-reconciliation-{}-{}",
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
            context_id: "review-reconciliation-context".into(),
            base_sha: base.clone(),
            head_sha: head.clone(),
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
        let draft = DraftStore::open(&state)
            .unwrap()
            .save(
                &binding.preparation_id,
                None,
                "Please use a named constant.",
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
            repo,
            state,
            prep: binding.preparation_id,
            draft_id: draft.id,
            head,
        }
    }

    fn unknown_review(&self) -> bodh::ReviewOperation {
        let mut store = ReviewPublicationStore::open(&self.state).unwrap();
        let intent = store
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
                selected_replies: vec![],
                selected_thread_actions: vec![],
                summary: Some("Please address this before merge.".into()),
                event: Some(ReviewEvent::RequestChanges),
            })
            .unwrap();
        let remote = RemoteReviewState {
            pull_request_id: "PR_node".into(),
            head_sha: self.head.clone(),
            base_sha: git(&self.repo, &["rev-parse", "HEAD~1"]),
            state: "OPEN".into(),
            viewer_permission: "WRITE".into(),
        };
        let planned = store
            .confirm(
                &self.prep,
                &self.state,
                intent.revision,
                &remote,
                &OperationControl::default(),
            )
            .unwrap();
        store.begin_send(&planned.id, &remote).unwrap();
        store
            .finish_send(
                &planned.id,
                SendOutcome::Uncertain {
                    message: "Connection ended after the request started".into(),
                },
            )
            .unwrap()
    }

    fn review_snapshot(
        &self,
        operation: &bodh::ReviewOperation,
        login: &str,
        repository: &str,
        reviews: Vec<Value>,
    ) -> Value {
        let submitted_at = rfc3339_utc(operation.send_started_at.unwrap() + 5_000);
        let thread_nodes: Vec<_> = reviews
            .iter()
            .map(|review| {
                json!({
                    "path":"a.rs", "line":2, "startLine":null,
                    "diffSide":"RIGHT", "startDiffSide":null,
                    "comments": {
                        "pageInfo":{"hasNextPage":false},
                        "nodes":[{
                            "id":"comment-node-1",
                            "body":"Please use a named constant.",
                            "pullRequestReview":{"id":review["id"]}
                        }]
                    }
                })
            })
            .collect();
        let review_nodes: Vec<_> = reviews
            .into_iter()
            .map(|mut review| {
                review["submittedAt"] = json!(submitted_at);
                review["commit"] = json!({"oid":self.head});
                review
            })
            .collect();
        json!({"data": {
            "viewer": {"login": login},
            "repository": {
                "nameWithOwner": repository,
                "pullRequest": {
                    "number": 7,
                    "reviews": {
                        "pageInfo": {"hasNextPage": false},
                        "nodes": review_nodes
                    },
                    "reviewThreads": {
                        "pageInfo": {"hasNextPage": false},
                    "nodes": thread_nodes
                    }
                }
            }
        }})
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn one_exact_authenticated_review_match_confirms_and_keeps_payload_provenance() {
    let f = Fixture::new();
    let operation = f.unknown_review();
    let response = f.review_snapshot(
        &operation,
        "reviewer",
        "acme/widget",
        vec![review("REVIEW_NODE_1", "reviewer")],
    );
    let gh = fake_gh(&f.root.join("gh"), &response);
    let reader = GhReviewOutcomeReader {
        executable: gh,
        checkout: f.repo.clone(),
        timeout: std::time::Duration::from_secs(2),
    };

    let mut store = ReviewPublicationStore::open(&f.state).unwrap();
    let reconciled = store
        .reconcile_unknown(&operation.id, &reader, &OperationControl::default())
        .unwrap();

    assert_eq!(reconciled.state, "confirmed");
    assert_eq!(
        reconciled.remote_review_id.as_deref(),
        Some("REVIEW_NODE_1")
    );
    assert_eq!(
        reconciled.remote_url.as_deref(),
        Some("https://github.com/acme/widget/pull/7#pullrequestreview-1")
    );
    assert_eq!(reconciled.payload, operation.payload);
    assert_eq!(reconciled.payload.comments[0].draft_id, f.draft_id);
    assert!(reconciled.failure.is_none());
    assert!(store.pending_operation(&f.prep).unwrap().is_none());
}

#[test]
fn no_remote_match_leaves_the_payload_unknown_and_unpublished() {
    let f = Fixture::new();
    let operation = f.unknown_review();
    let response = f.review_snapshot(&operation, "reviewer", "acme/widget", vec![]);
    let gh = fake_gh(&f.root.join("gh"), &response);
    let reader = reader(gh, &f.repo);

    let mut store = ReviewPublicationStore::open(&f.state).unwrap();
    let reconciled = store
        .reconcile_unknown(&operation.id, &reader, &OperationControl::default())
        .unwrap();

    assert_eq!(reconciled.state, "unknown");
    assert!(
        reconciled
            .failure
            .as_deref()
            .unwrap()
            .to_lowercase()
            .contains("no unique")
    );
    assert_eq!(reconciled.payload, operation.payload);
    assert!(store.pending_operation(&f.prep).unwrap().is_some());
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
fn duplicate_exact_remote_reviews_remain_unknown() {
    let f = Fixture::new();
    let operation = f.unknown_review();
    let response = f.review_snapshot(
        &operation,
        "reviewer",
        "acme/widget",
        vec![
            review("REVIEW_NODE_1", "reviewer"),
            review("REVIEW_NODE_2", "reviewer"),
        ],
    );
    let gh = fake_gh(&f.root.join("gh"), &response);
    let mut store = ReviewPublicationStore::open(&f.state).unwrap();
    let reconciled = store
        .reconcile_unknown(
            &operation.id,
            &reader(gh, &f.repo),
            &OperationControl::default(),
        )
        .unwrap();
    assert_eq!(reconciled.state, "unknown");
    assert!(
        reconciled
            .failure
            .as_deref()
            .unwrap()
            .to_lowercase()
            .contains("ambiguous")
    );
    assert!(reconciled.remote_review_id.is_none());
}

#[test]
fn wrong_actor_or_repository_never_confirms_an_exact_looking_review() {
    for (login, repository) in [("other-user", "acme/widget"), ("reviewer", "other/repo")] {
        let f = Fixture::new();
        let operation = f.unknown_review();
        let response = f.review_snapshot(
            &operation,
            login,
            repository,
            vec![review("REVIEW_NODE_1", "reviewer")],
        );
        let gh = fake_gh(&f.root.join("gh"), &response);
        let mut store = ReviewPublicationStore::open(&f.state).unwrap();
        let reconciled = store
            .reconcile_unknown(
                &operation.id,
                &reader(gh, &f.repo),
                &OperationControl::default(),
            )
            .unwrap();
        assert_eq!(reconciled.state, "unknown");
        assert!(reconciled.remote_review_id.is_none());
    }
}

#[test]
fn wrong_diff_side_is_not_an_exact_review_match() {
    let f = Fixture::new();
    let operation = f.unknown_review();
    let mut response = f.review_snapshot(
        &operation,
        "reviewer",
        "acme/widget",
        vec![review("REVIEW_NODE_1", "reviewer")],
    );
    response["data"]["repository"]["pullRequest"]["reviewThreads"]["nodes"][0]["diffSide"] =
        json!("LEFT");
    let gh = fake_gh(&f.root.join("gh-wrong-side"), &response);
    let mut store = ReviewPublicationStore::open(&f.state).unwrap();
    let reconciled = store
        .reconcile_unknown(
            &operation.id,
            &reader(gh, &f.repo),
            &OperationControl::default(),
        )
        .unwrap();
    assert_eq!(reconciled.state, "unknown");
}

#[test]
fn incomplete_remote_listing_and_unavailable_gh_never_resolve_unknown() {
    let f = Fixture::new();
    let operation = f.unknown_review();
    let mut response = f.review_snapshot(
        &operation,
        "reviewer",
        "acme/widget",
        vec![review("REVIEW_NODE_1", "reviewer")],
    );
    response["data"]["repository"]["pullRequest"]["reviews"]["pageInfo"]["hasNextPage"] =
        json!(true);
    let gh = fake_gh(&f.root.join("gh-incomplete"), &response);
    let mut store = ReviewPublicationStore::open(&f.state).unwrap();
    let partial = store
        .reconcile_unknown(
            &operation.id,
            &reader(gh, &f.repo),
            &OperationControl::default(),
        )
        .unwrap();
    assert_eq!(partial.state, "unknown");
    assert!(partial.failure.as_deref().unwrap().contains("incomplete"));

    let unavailable = fake_gh(
        &f.root.join("gh-unavailable"),
        &json!({"errors":[{"message":"offline"}]}),
    );
    let error = store
        .reconcile_unknown(
            &operation.id,
            &reader(unavailable, &f.repo),
            &OperationControl::default(),
        )
        .unwrap_err();
    assert_eq!(error.code, "REVIEW_RECONCILIATION_UNAVAILABLE");
    assert_eq!(store.operation(&operation.id).unwrap().state, "unknown");
}

fn review(id: &str, login: &str) -> Value {
    json!({
        "id":id,
        "url":format!("https://github.com/acme/widget/pull/7#pullrequestreview-{}", id.trim_start_matches("REVIEW_NODE_")),
        "body":"Please address this before merge.",
        "state":"CHANGES_REQUESTED",
        "submittedAt":null,
        "author":{"login":login},
        "commit":{"oid":""}
    })
}

fn reader(executable: PathBuf, checkout: &Path) -> GhReviewOutcomeReader {
    GhReviewOutcomeReader {
        executable,
        checkout: checkout.to_path_buf(),
        timeout: std::time::Duration::from_secs(2),
    }
}

fn fake_gh(path: &Path, response: &Value) -> PathBuf {
    let response = serde_json::to_string(response).unwrap();
    let quoted = response.replace('\'', "'\"'\"'");
    // The production client sends the GraphQL request on stdin. Consume it
    // before exiting so the fixture cannot race a fast shell startup and close
    // the pipe while the parent is still writing a larger request.
    std::fs::write(
        path,
        format!("#!/bin/sh\ncat >/dev/null\nprintf '%s' '{quoted}'\n"),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
    path.to_path_buf()
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn rfc3339_utc(unix_ms: i64) -> String {
    let seconds = unix_ms.div_euclid(1_000);
    let millis = unix_ms.rem_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}
