use bodh::{
    DraftStore, FindingAlternativeEditInput, FindingDisposition, FindingTriageStore,
    GeneratedReviewStore, OperationControl, PrIdentity, PreparationStore, RemoteReviewState,
    RetentionPolicy, ReviewContext, ReviewEvent, ReviewPublicationStore, ReviewPublisher,
    ReviewResult, Revision, SendOutcome, SourceRef, StageReviewRequest, diff_index,
    enforce_retention,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct ConfirmPublication;
impl ReviewPublisher for ConfirmPublication {
    fn publish(&self, _payload: &bodh::ReviewPayload, _control: &OperationControl) -> SendOutcome {
        SendOutcome::Confirmed {
            remote_id: "controlled-review-id".into(),
            url: None,
        }
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
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

struct Fixture {
    root: PathBuf,
    state: PathBuf,
    preparation_id: String,
    result: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "bodh-generated-review-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = root.join("repo");
        let state = root.join("state.sqlite");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("src.rs"), "before\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "base"]);
        let base = git(&repo, &["rev-parse", "HEAD"]);
        std::fs::write(repo.join("src.rs"), "after\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "head"]);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        let context = ReviewContext {
            pr: PrIdentity {
                host: "github.com".into(),
                owner: "acme".into(),
                repository: "widget".into(),
                number: 27,
            },
            context_id: "generated-review-context".into(),
            base_sha: base.clone(),
            head_sha: head,
            merge_base_sha: base,
        };
        let files = diff_index(&repo, &context, &OperationControl::default())
            .unwrap()
            .files;
        let binding = PreparationStore::open(&state)
            .unwrap()
            .save(
                &context,
                repo.to_str().unwrap(),
                "Generated review fixture",
                Some("An immutable original description."),
                &files,
            )
            .unwrap();
        DraftStore::open(&state).unwrap();
        let mut result: Value =
            serde_json::from_str(include_str!("../contracts/v1/fixtures/empty.json")).unwrap();
        result["context"] = serde_json::to_value(&binding.context).unwrap();
        Self {
            root,
            state,
            preparation_id: binding.preparation_id,
            result: serde_json::to_vec(&result).unwrap(),
        }
    }

    fn result(&self) -> Vec<u8> {
        self.result.clone()
    }

    fn cli(&self, command: &str, payload: &[u8], expected_generation: u64) -> (bool, Value) {
        let input = self.root.join("candidate.json");
        std::fs::write(&input, payload).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_bodh"))
            .args([
                command,
                "--prepared",
                &self.preparation_id,
                "--input",
                input.to_str().unwrap(),
                "--state",
                self.state.to_str().unwrap(),
                "--expected-generation",
                &expected_generation.to_string(),
            ])
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let value = if output.status.success() {
            value["data"].clone()
        } else {
            value["error"].clone()
        };
        (output.status.success(), value)
    }
}

#[test]
fn stale_generation_and_invalid_result_leave_the_current_review_untouched() {
    let fixture = Fixture::new();
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    let control = OperationControl::default();
    let first = store
        .commit(&fixture.preparation_id, 0, &fixture.result(), &control)
        .unwrap();

    let mut changed: Value = serde_json::from_slice(&fixture.result()).unwrap();
    changed["coverage"]["limitations"] = json!(["New generation"]);
    let stale = store
        .commit(
            &fixture.preparation_id,
            0,
            &serde_json::to_vec(&changed).unwrap(),
            &control,
        )
        .unwrap_err();
    assert_eq!(stale.code, "WORKSPACE_REVISION_STALE");

    changed["findings"] = json!("not an array");
    let invalid = store
        .commit(
            &fixture.preparation_id,
            1,
            &serde_json::to_vec(&changed).unwrap(),
            &control,
        )
        .unwrap_err();
    assert_eq!(invalid.code, "CONTRACT_INVALID");
    let current = store.current(&fixture.preparation_id).unwrap().unwrap();
    assert_eq!(current.result, first.result);
    assert_eq!(current.generation, 1);
    assert_eq!(
        store.workspace_generation(&fixture.preparation_id).unwrap(),
        1
    );
}

#[test]
fn finding_triage_is_bound_to_a_committed_finding_and_survives_reopen() {
    let fixture = Fixture::new();
    let mut result: Value = serde_json::from_slice(&fixture.result()).unwrap();
    result["findings"] = json!([{
        "id":"F1","title":"Handle empty response","severity":"high","confidence":"medium",
        "summary":"The response is not checked.","reason":"A consumer can receive an empty value.",
        "impact":"The caller may fail later with less context.",
        "evidence":[{"source":{"path":"src.rs","revision":"head","start_line":1,"end_line":1},"explanation":"The changed line returns the unchecked value."}],
        "locations":[{"source":{"path":"src.rs","revision":"head","start_line":1,"end_line":1},"primary":true}],
        "suggestions":[{"kind":"text_comment","id":"ask-check","body":"Could this response be checked?","source":{"path":"src.rs","revision":"head","start_line":1,"end_line":1}}],
        "related_thread_ids":[]
    }]);
    GeneratedReviewStore::open(&fixture.state)
        .unwrap()
        .commit(
            &fixture.preparation_id,
            0,
            &serde_json::to_vec(&result).unwrap(),
            &OperationControl::default(),
        )
        .unwrap();

    let mut triage = FindingTriageStore::open(&fixture.state).unwrap();
    let edit = triage
        .save_alternative_edit(
            &fixture.preparation_id,
            "F1",
            "ask-check",
            FindingAlternativeEditInput {
                body: "Please validate the response before returning it.",
                replacement: None,
                expected_revision: 0,
            },
            &OperationControl::default(),
        )
        .unwrap();
    assert_eq!(edit.revision, 1);
    assert_eq!(edit.source.path, "src.rs");
    let (intent, selected) = triage
        .stage_alternative(&fixture.preparation_id, "F1", "ask-check", 0)
        .unwrap();
    assert_eq!(intent.selected.len(), 1);
    assert_eq!(intent.selected[0].id, edit.draft_id);
    assert_eq!(selected.revision, 1);
    assert_eq!(
        selected.selected_suggestion_id.as_deref(),
        Some("ask-check")
    );
    assert_eq!(
        triage
            .set_disposition(
                &fixture.preparation_id,
                "F1",
                FindingDisposition::Deferred,
                None,
                None,
                0,
            )
            .unwrap_err()
            .code,
        "FINDING_TRIAGE_STALE"
    );

    let reopened = FindingTriageStore::open(&fixture.state).unwrap();
    let snapshot = reopened.list(&fixture.preparation_id).unwrap();
    assert_eq!(snapshot.entries, [selected]);
    assert_eq!(snapshot.alternative_edits, [edit]);
}

#[test]
fn replacement_keeps_only_compact_prior_concern_text_and_exact_source_metadata() {
    let fixture = Fixture::new();
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    let mut first: Value = serde_json::from_slice(&fixture.result()).unwrap();
    first["findings"] = json!([{
        "id":"F1","title":"Handle empty response","severity":"high","confidence":"medium",
        "summary":"The response is not checked.","reason":"A consumer can receive an empty value.",
        "impact":"The caller may fail later with less context.",
        "evidence":[
            {"source":{"path":"src.rs","revision":"head","start_line":1,"end_line":1},"explanation":"The changed line returns the unchecked value."},
            {"source":{"path":"src.rs","revision":"base","start_line":1,"end_line":1},"explanation":"Original implementation at merge base."}
        ],
        "locations":[{"source":{"path":"src.rs","revision":"head","start_line":1,"end_line":1},"primary":true}],
        "suggestions":[],"related_thread_ids":[]
    }]);
    store
        .commit(
            &fixture.preparation_id,
            0,
            &serde_json::to_vec(&first).unwrap(),
            &OperationControl::default(),
        )
        .unwrap();

    let mut next: Value = serde_json::from_slice(&fixture.result()).unwrap();
    next["follow_up"] = json!([{
        "prior_concern_id":"F1","status":"resolved","explanation":"The unchecked response path was removed.",
        "evidence":[{"source":{"path":"src.rs","revision":"head","start_line":1,"end_line":1},"explanation":"Current source returns the checked value."}],
        "finding_id":null
    }]);
    let committed = store
        .commit(
            &fixture.preparation_id,
            1,
            &serde_json::to_vec(&next).unwrap(),
            &OperationControl::default(),
        )
        .unwrap();
    let prior = store
        .current(&fixture.preparation_id)
        .unwrap()
        .unwrap()
        .context
        .prior_concerns
        .unwrap();
    assert_eq!(committed.generation, 2);
    assert_eq!(prior.len(), 1);
    assert_eq!(prior[0].id, "F1");
    assert_eq!(prior[0].summary, "The response is not checked.");
    assert_eq!(prior[0].evidence.len(), 2);
    assert_eq!(
        prior[0].evidence[0].commit_sha,
        first["context"]["head_sha"]
    );
    assert_eq!(prior[0].evidence[0].source.path, "src.rs");
    assert_eq!(
        prior[0].evidence[1].commit_sha,
        first["context"]["merge_base_sha"]
    );
    let db = rusqlite::Connection::open(&fixture.state).unwrap();
    let (count, current_result): (u32, String) = db
        .query_row(
            "SELECT COUNT(*), MAX(result_json) FROM generated_reviews",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert!(!current_result.contains("Handle empty response"));
}

#[test]
fn generated_import_follow_up_publication_restart_and_failed_prune_cycles() {
    let fixture = Fixture::new();
    let repo = fixture.root.join("repo");
    let policy = RetentionPolicy {
        per_repository_preparations: 2,
        global_preparations: 2,
        per_repository_bytes: 1024 * 1024,
        global_bytes: 1024 * 1024,
    };
    let mut prep_ids = Vec::new();
    for cycle in 0..3 {
        let prior = PreparationStore::open(&fixture.state)
            .unwrap()
            .load(&fixture.preparation_id)
            .unwrap();
        let context = ReviewContext {
            context_id: format!("generated-cycle-{cycle}"),
            ..prior.context
        };
        let files = diff_index(&repo, &context, &OperationControl::default())
            .unwrap()
            .files;
        let binding = PreparationStore::open(&fixture.state)
            .unwrap()
            .save(
                &context,
                repo.to_str().unwrap(),
                "cycle",
                Some("repeat"),
                &files,
            )
            .unwrap();
        let mut candidate: Value = serde_json::from_slice(&fixture.result()).unwrap();
        candidate["context"] = serde_json::to_value(&binding.context).unwrap();
        if cycle == 0 {
            candidate["findings"] = json!([{
                "id":"F_cycle","title":"Check the response","severity":"high","confidence":"medium",
                "summary":"The response is not checked.","reason":"The consumer receives unchecked data.","impact":"Later failures lose context.",
                "evidence":[{"source":{"path":"src.rs","revision":"head","start_line":1,"end_line":1},"explanation":"The changed line returns the unchecked value."}],
                "locations":[{"source":{"path":"src.rs","revision":"head","start_line":1,"end_line":1},"primary":true}],"suggestions":[],"related_thread_ids":[]
            }]);
        } else if cycle == 1 {
            candidate["follow_up"] = json!([{
                "prior_concern_id":"F_cycle","status":"resolved","explanation":"The response check was added.",
                "evidence":[{"source":{"path":"src.rs","revision":"head","start_line":1,"end_line":1},"explanation":"Current source returns a checked response."}],"finding_id":null
            }]);
        }
        let commit = GeneratedReviewStore::open(&fixture.state)
            .unwrap()
            .commit(
                &binding.preparation_id,
                binding.expected_generation,
                &serde_json::to_vec(&candidate).unwrap(),
                &OperationControl::default(),
            )
            .unwrap();
        assert_eq!(commit.generation, binding.expected_generation + 1);
        prep_ids.push(binding.preparation_id.clone());

        if cycle == 0 {
            let mut publication = ReviewPublicationStore::open(&fixture.state).unwrap();
            let snapshot = DraftStore::open(&fixture.state)
                .unwrap()
                .list(&binding.preparation_id)
                .unwrap();
            let intent = publication
                .stage(StageReviewRequest {
                    state: fixture.state.clone(),
                    preparation_id: binding.preparation_id.clone(),
                    expected_workspace_revision: snapshot.workspace_revision,
                    expected_intent_revision: 0,
                    selected: vec![],
                    selected_replacements: vec![],
                    selected_replies: vec![],
                    selected_thread_actions: vec![],
                    summary: Some("Cycle publication".into()),
                    event: Some(ReviewEvent::Comment),
                })
                .unwrap();
            let remote = RemoteReviewState {
                pull_request_id: "PR_controlled".into(),
                head_sha: context.head_sha.clone(),
                base_sha: context.base_sha.clone(),
                state: "OPEN".into(),
                viewer_permission: "WRITE".into(),
            };
            let operation = publication
                .confirm(
                    &binding.preparation_id,
                    &fixture.state,
                    intent.revision,
                    &remote,
                    &OperationControl::default(),
                )
                .unwrap();
            let sent = publication
                .publish_confirmed(
                    &operation.id,
                    &remote,
                    &ConfirmPublication,
                    &OperationControl::default(),
                )
                .unwrap();
            assert_eq!(sent.state, "confirmed");
        }
        drop(GeneratedReviewStore::open(&fixture.state).unwrap()); // restart/reopen after each import
        if cycle == 2 {
            let db = rusqlite::Connection::open(&fixture.state).unwrap();
            db.execute_batch("CREATE TRIGGER retention_cycle_failure BEFORE DELETE ON preparations BEGIN SELECT RAISE(ABORT,'injected cycle failure'); END;").unwrap();
            assert!(enforce_retention(&fixture.state, policy, &[]).is_err());
            db.execute_batch("DROP TRIGGER retention_cycle_failure;")
                .unwrap();
            assert_eq!(
                db.query_row("SELECT count(*) FROM preparations", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                4
            );
            drop(db);
            let reopened = GeneratedReviewStore::open(&fixture.state).unwrap();
            assert!(reopened.current(&binding.preparation_id).unwrap().is_some());
            let report = enforce_retention(&fixture.state, policy, &[]).unwrap();
            assert_eq!(report.preparations_after, 2);
            assert!(report.estimated_bytes_after <= policy.global_bytes);
            eprintln!(
                "generated retention cycles: preparations={} bytes={} deleted_preparations={} deleted_bytes={}",
                report.preparations_after,
                report.estimated_bytes_after,
                report.deleted_preparations,
                report.deleted_bytes
            );
        }
    }
    let store = PreparationStore::open(&fixture.state).unwrap();
    assert!(
        store.load(&prep_ids[0]).is_ok(),
        "publication intent remains a protected recovery floor"
    );
    assert!(
        store.load(&prep_ids[2]).is_ok(),
        "current generated result remains protected after restart"
    );
}

#[test]
fn independent_manual_finding_feedback_is_persisted_without_publication() {
    let fixture = Fixture::new();
    let mut result: Value = serde_json::from_slice(&fixture.result()).unwrap();
    let source = json!({"path":"src.rs","revision":"head","start_line":1,"end_line":1});
    result["findings"] = json!([{
        "id":"F1","title":"Handle empty response","severity":"high","confidence":"medium",
        "summary":"The response is not checked.","reason":"A consumer can receive an empty value.",
        "impact":"The caller may fail later with less context.",
        "evidence":[{"source":source.clone(),"explanation":"The changed line returns the unchecked value."}],
        "locations":[{"source":source,"primary":true}],"suggestions":[],"related_thread_ids":[]
    }]);
    GeneratedReviewStore::open(&fixture.state)
        .unwrap()
        .commit(
            &fixture.preparation_id,
            0,
            &serde_json::to_vec(&result).unwrap(),
            &OperationControl::default(),
        )
        .unwrap();

    let entry = FindingTriageStore::open(&fixture.state)
        .unwrap()
        .set_disposition(
            &fixture.preparation_id,
            "F1",
            FindingDisposition::Staged,
            None,
            Some("Please verify the empty-response case manually."),
            0,
        )
        .unwrap();
    assert_eq!(
        entry.manual_feedback.as_deref(),
        Some("Please verify the empty-response case manually.")
    );
    assert!(
        ReviewPublicationStore::open(&fixture.state)
            .unwrap()
            .intent(&fixture.preparation_id)
            .unwrap()
            .selected
            .is_empty()
    );
}

#[test]
fn switching_a_finding_alternative_preserves_other_edits_and_unstaging_keeps_drafts() {
    let fixture = Fixture::new();
    let mut result: Value = serde_json::from_slice(&fixture.result()).unwrap();
    let source = json!({"path":"src.rs","revision":"head","start_line":1,"end_line":1});
    result["findings"] = json!([{
        "id":"F1","title":"Handle empty response","severity":"high","confidence":"medium",
        "summary":"The response is not checked.","reason":"A consumer can receive an empty value.",
        "impact":"The caller may fail later with less context.",
        "evidence":[{"source":source.clone(),"explanation":"The changed line returns the unchecked value."}],
        "locations":[{"source":source.clone(),"primary":true}],
        "suggestions":[
            {"kind":"text_comment","id":"choice-a","body":"Check the response.","source":source.clone()},
            {"kind":"text_comment","id":"choice-b","body":"Handle an empty response.","source":source}
        ],
        "related_thread_ids":[]
    }]);
    GeneratedReviewStore::open(&fixture.state)
        .unwrap()
        .commit(
            &fixture.preparation_id,
            0,
            &serde_json::to_vec(&result).unwrap(),
            &OperationControl::default(),
        )
        .unwrap();

    let mut triage = FindingTriageStore::open(&fixture.state).unwrap();
    let first = triage
        .save_alternative_edit(
            &fixture.preparation_id,
            "F1",
            "choice-a",
            FindingAlternativeEditInput {
                body: "Reviewer edit A",
                replacement: None,
                expected_revision: 0,
            },
            &OperationControl::default(),
        )
        .unwrap();
    triage
        .stage_alternative(&fixture.preparation_id, "F1", "choice-a", 0)
        .unwrap();
    let second = triage
        .save_alternative_edit(
            &fixture.preparation_id,
            "F1",
            "choice-b",
            FindingAlternativeEditInput {
                body: "Reviewer edit B",
                replacement: None,
                expected_revision: 0,
            },
            &OperationControl::default(),
        )
        .unwrap();
    let (intent, staged) = triage
        .stage_alternative(&fixture.preparation_id, "F1", "choice-b", 1)
        .unwrap();

    assert_eq!(intent.selected.len(), 1);
    assert_eq!(intent.selected[0].id, second.draft_id);
    assert_ne!(first.draft_id, second.draft_id);
    assert_eq!(staged.selected_suggestion_id.as_deref(), Some("choice-b"));
    let drafts = DraftStore::open(&fixture.state)
        .unwrap()
        .list(&fixture.preparation_id)
        .unwrap();
    assert_eq!(drafts.drafts.len(), 2);
    assert!(
        drafts
            .drafts
            .iter()
            .any(|draft| draft.id == first.draft_id && draft.body == "Reviewer edit A")
    );
    assert!(
        drafts
            .drafts
            .iter()
            .any(|draft| draft.id == second.draft_id && draft.body == "Reviewer edit B")
    );

    let mut triage = FindingTriageStore::open(&fixture.state).unwrap();
    triage
        .set_disposition(
            &fixture.preparation_id,
            "F1",
            FindingDisposition::Deferred,
            None,
            None,
            2,
        )
        .unwrap();
    let intent = ReviewPublicationStore::open(&fixture.state)
        .unwrap()
        .intent(&fixture.preparation_id)
        .unwrap();
    assert!(intent.selected.is_empty());
    assert_eq!(
        DraftStore::open(&fixture.state)
            .unwrap()
            .list(&fixture.preparation_id)
            .unwrap()
            .drafts
            .len(),
        2
    );
}

#[test]
fn a_newer_preparation_makes_an_older_candidate_stale() {
    let fixture = Fixture::new();
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    let old = PreparationStore::open(&fixture.state)
        .unwrap()
        .load(&fixture.preparation_id)
        .unwrap();
    let mut newer_context = old.context.clone();
    newer_context.context_id = "newer-pinned-context".into();
    PreparationStore::open(&fixture.state)
        .unwrap()
        .save(
            &newer_context,
            &old.checkout,
            "Newer fixture",
            old.description.as_deref(),
            &old.changed_files,
        )
        .unwrap();
    let stale = store
        .validate(
            &fixture.preparation_id,
            0,
            &fixture.result(),
            &OperationControl::default(),
        )
        .unwrap_err();
    assert_eq!(stale.code, "PREPARATION_CONTEXT_STALE");
    assert!(store.current(&fixture.preparation_id).unwrap().is_none());
}

#[test]
fn a_sql_failure_rolls_back_generation_and_result_as_one_transaction() {
    let fixture = Fixture::new();
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    let db = rusqlite::Connection::open(&fixture.state).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_generated_review BEFORE INSERT ON generated_reviews
         BEGIN SELECT RAISE(ABORT, 'fixture rollback'); END;",
    )
    .unwrap();
    let failure = store
        .commit(
            &fixture.preparation_id,
            0,
            &fixture.result(),
            &OperationControl::default(),
        )
        .unwrap_err();
    assert_eq!(failure.code, "STATE_UNAVAILABLE");
    assert!(store.current(&fixture.preparation_id).unwrap().is_none());
    assert_eq!(
        store.workspace_generation(&fixture.preparation_id).unwrap(),
        0
    );
}

#[test]
fn a_failed_replacement_keeps_the_previous_complete_result_and_generation() {
    let fixture = Fixture::new();
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    let first = store
        .commit(
            &fixture.preparation_id,
            0,
            &fixture.result(),
            &OperationControl::default(),
        )
        .unwrap();
    let db = rusqlite::Connection::open(&fixture.state).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_generated_review_update BEFORE UPDATE ON generated_reviews
         BEGIN SELECT RAISE(ABORT, 'fixture replacement rollback'); END;",
    )
    .unwrap();
    let mut changed: Value = serde_json::from_slice(&fixture.result()).unwrap();
    changed["coverage"]["limitations"] = json!(["Replacement candidate"]);
    let failure = store
        .commit(
            &fixture.preparation_id,
            1,
            &serde_json::to_vec(&changed).unwrap(),
            &OperationControl::default(),
        )
        .unwrap_err();
    assert_eq!(failure.code, "STATE_UNAVAILABLE");
    let current = store.current(&fixture.preparation_id).unwrap().unwrap();
    assert_eq!(current.result, first.result);
    assert_eq!(current.generation, 1);
    assert_eq!(
        store.workspace_generation(&fixture.preparation_id).unwrap(),
        1
    );
}

#[test]
fn an_unpublished_replacement_blocks_changed_results() {
    let fixture = Fixture::new();
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    let control = OperationControl::default();
    let original = store
        .commit(&fixture.preparation_id, 0, &fixture.result(), &control)
        .unwrap();
    let mut drafts = DraftStore::open(&fixture.state).unwrap();
    drafts
        .save_replacement(
            &fixture.preparation_id,
            None,
            &SourceRef {
                path: "src.rs".into(),
                revision: Revision::Head,
                start_line: 1,
                end_line: 1,
            },
            "after\n",
            "proposed\n",
            1,
            0,
            &control,
        )
        .unwrap();

    let mut changed: Value = serde_json::from_slice(&fixture.result()).unwrap();
    changed["coverage"]["limitations"] = json!(["Changed generated review"]);
    let failure = store
        .commit(
            &fixture.preparation_id,
            2,
            &serde_json::to_vec(&changed).unwrap(),
            &control,
        )
        .unwrap_err();
    assert_eq!(failure.code, "APPROVAL_REQUIRED");
    assert_eq!(
        store
            .current(&fixture.preparation_id)
            .unwrap()
            .unwrap()
            .result,
        original.result
    );
}

#[test]
fn an_explicit_replacement_approval_is_bound_to_the_candidate_and_clears_only_its_drafts() {
    let fixture = Fixture::new();
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    let control = OperationControl::default();
    store
        .commit(&fixture.preparation_id, 0, &fixture.result(), &control)
        .unwrap();
    let mut drafts = DraftStore::open(&fixture.state).unwrap();
    drafts
        .save_replacement(
            &fixture.preparation_id,
            None,
            &SourceRef {
                path: "src.rs".into(),
                revision: Revision::Head,
                start_line: 1,
                end_line: 1,
            },
            "after\n",
            "proposed\n",
            1,
            0,
            &control,
        )
        .unwrap();
    let mut changed: Value = serde_json::from_slice(&fixture.result()).unwrap();
    changed["coverage"]["limitations"] = json!(["Approved replacement"]);
    let input = serde_json::to_vec(&changed).unwrap();

    let approval = store
        .authorize_replacement(&fixture.preparation_id, 2, &input, &control, |preview| {
            assert_eq!(preview.pull_request_number, 27);
            assert_eq!(preview.code_replacements, 1);
            assert_eq!(preview.workspace_revision, 2);
            Ok(true)
        })
        .unwrap();
    let committed = store
        .commit_authorized(&fixture.preparation_id, 2, &input, &approval.id, &control)
        .unwrap();
    assert_eq!(committed.status, "committed");
    assert_eq!(committed.generation, 3);
    assert!(
        drafts
            .list_replacements(&fixture.preparation_id)
            .unwrap()
            .drafts
            .is_empty()
    );
    assert!(store.current(&fixture.preparation_id).unwrap().is_some());
}

#[test]
fn replacement_approval_is_one_use_and_rejects_a_different_candidate() {
    let fixture = Fixture::new();
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    let control = OperationControl::default();
    store
        .commit(&fixture.preparation_id, 0, &fixture.result(), &control)
        .unwrap();
    let mut drafts = DraftStore::open(&fixture.state).unwrap();
    let snapshot = drafts.list(&fixture.preparation_id).unwrap();
    drafts
        .save(
            &fixture.preparation_id,
            None,
            "Keep pending reviewer work.",
            &SourceRef {
                path: "src.rs".into(),
                revision: Revision::Head,
                start_line: 1,
                end_line: 1,
            },
            snapshot.workspace_revision,
            0,
            &control,
        )
        .unwrap();
    let mut first: Value = serde_json::from_slice(&fixture.result()).unwrap();
    first["coverage"]["limitations"] = json!(["first candidate"]);
    let first = serde_json::to_vec(&first).unwrap();
    let approval = store
        .authorize_replacement(&fixture.preparation_id, 2, &first, &control, |_| Ok(true))
        .unwrap();
    let mut other: Value = serde_json::from_slice(&fixture.result()).unwrap();
    other["coverage"]["limitations"] = json!(["different candidate"]);
    let invalid = store
        .commit_authorized(
            &fixture.preparation_id,
            2,
            &serde_json::to_vec(&other).unwrap(),
            &approval.id,
            &control,
        )
        .unwrap_err();
    assert_eq!(invalid.code, "APPROVAL_INVALID");
    assert_eq!(
        drafts.list(&fixture.preparation_id).unwrap().drafts.len(),
        1
    );
    store
        .commit_authorized(&fixture.preparation_id, 2, &first, &approval.id, &control)
        .unwrap();

    let after = drafts.list(&fixture.preparation_id).unwrap();
    drafts
        .save(
            &fixture.preparation_id,
            None,
            "A later reviewer note.",
            &SourceRef {
                path: "src.rs".into(),
                revision: Revision::Head,
                start_line: 1,
                end_line: 1,
            },
            after.workspace_revision,
            0,
            &control,
        )
        .unwrap();
    let mut later: Value = serde_json::from_slice(&fixture.result()).unwrap();
    later["coverage"]["limitations"] = json!(["later candidate"]);
    let spent = store
        .commit_authorized(
            &fixture.preparation_id,
            4,
            &serde_json::to_vec(&later).unwrap(),
            &approval.id,
            &control,
        )
        .unwrap_err();
    assert_eq!(spent.code, "APPROVAL_ALREADY_USED");
    assert_eq!(
        drafts.list(&fixture.preparation_id).unwrap().drafts.len(),
        1
    );
}

#[test]
fn declined_replacement_approval_does_not_change_result_or_work() {
    let fixture = Fixture::new();
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    let control = OperationControl::default();
    let original = store
        .commit(&fixture.preparation_id, 0, &fixture.result(), &control)
        .unwrap();
    let mut drafts = DraftStore::open(&fixture.state).unwrap();
    let initial = drafts.list(&fixture.preparation_id).unwrap();
    drafts
        .save(
            &fixture.preparation_id,
            None,
            "Do not discard this note.",
            &SourceRef {
                path: "src.rs".into(),
                revision: Revision::Head,
                start_line: 1,
                end_line: 1,
            },
            initial.workspace_revision,
            0,
            &control,
        )
        .unwrap();
    let mut changed: Value = serde_json::from_slice(&fixture.result()).unwrap();
    changed["coverage"]["limitations"] = json!(["declined candidate"]);
    let declined = store
        .authorize_replacement(
            &fixture.preparation_id,
            2,
            &serde_json::to_vec(&changed).unwrap(),
            &control,
            |_| Ok(false),
        )
        .unwrap_err();
    assert_eq!(declined.code, "APPROVAL_DECLINED");
    assert_eq!(
        store
            .current(&fixture.preparation_id)
            .unwrap()
            .unwrap()
            .result,
        original.result
    );
    assert_eq!(
        drafts.list(&fixture.preparation_id).unwrap().drafts.len(),
        1
    );
}

#[test]
fn replacement_approval_expires_when_a_protected_draft_revision_changes() {
    let fixture = Fixture::new();
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    let control = OperationControl::default();
    store
        .commit(&fixture.preparation_id, 0, &fixture.result(), &control)
        .unwrap();
    let mut drafts = DraftStore::open(&fixture.state).unwrap();
    let initial = drafts.list(&fixture.preparation_id).unwrap();
    let draft = drafts
        .save(
            &fixture.preparation_id,
            None,
            "Original note.",
            &SourceRef {
                path: "src.rs".into(),
                revision: Revision::Head,
                start_line: 1,
                end_line: 1,
            },
            initial.workspace_revision,
            0,
            &control,
        )
        .unwrap();
    let mut changed: Value = serde_json::from_slice(&fixture.result()).unwrap();
    changed["coverage"]["limitations"] = json!(["candidate"]);
    let candidate = serde_json::to_vec(&changed).unwrap();
    let approval = store
        .authorize_replacement(&fixture.preparation_id, 2, &candidate, &control, |_| {
            Ok(true)
        })
        .unwrap();
    drafts
        .save(
            &fixture.preparation_id,
            Some(&draft.draft.id),
            "Edited after approval.",
            &draft.draft.source,
            2,
            draft.draft.revision,
            &control,
        )
        .unwrap();
    let stale = store
        .commit_authorized(
            &fixture.preparation_id,
            3,
            &candidate,
            &approval.id,
            &control,
        )
        .unwrap_err();
    assert_eq!(stale.code, "APPROVAL_STALE");
    assert_eq!(
        drafts.list(&fixture.preparation_id).unwrap().drafts[0].body,
        "Edited after approval."
    );
}

#[test]
fn unresolved_publication_payload_blocks_replacement_authorization() {
    let fixture = Fixture::new();
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    let control = OperationControl::default();
    store
        .commit(&fixture.preparation_id, 0, &fixture.result(), &control)
        .unwrap();
    let db = rusqlite::Connection::open(&fixture.state).unwrap();
    db.execute(
        "INSERT INTO review_operations(operation_id,preparation_id,fingerprint,state,payload_json,created_at,updated_at)
         VALUES('unknown-op',?1,'fp','unknown','{\"immutable\":\"payload\"}',1,1)",
        [&fixture.preparation_id],
    )
    .unwrap();
    let mut changed: Value = serde_json::from_slice(&fixture.result()).unwrap();
    changed["coverage"]["limitations"] = json!(["candidate"]);
    let blocked = store
        .authorize_replacement(
            &fixture.preparation_id,
            1,
            &serde_json::to_vec(&changed).unwrap(),
            &control,
            |_| panic!("must not prompt while publication outcome is unknown"),
        )
        .unwrap_err();
    assert_eq!(blocked.code, "PUBLICATION_RECOVERY_REQUIRED");
    let retained: String = db
        .query_row(
            "SELECT payload_json FROM review_operations WHERE operation_id='unknown-op'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained, "{\"immutable\":\"payload\"}");
}

#[test]
fn a_staged_grouped_review_intent_blocks_changed_results() {
    let fixture = Fixture::new();
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    let control = OperationControl::default();
    let original = store
        .commit(&fixture.preparation_id, 0, &fixture.result(), &control)
        .unwrap();
    let mut publication = bodh::ReviewPublicationStore::open(&fixture.state).unwrap();
    let intent = publication
        .stage(bodh::StageReviewRequest {
            state: fixture.state.clone(),
            preparation_id: fixture.preparation_id.clone(),
            expected_workspace_revision: 1,
            expected_intent_revision: 0,
            selected: vec![],
            selected_replacements: vec![],
            selected_replies: vec![],
            selected_thread_actions: vec![],
            summary: Some("Preserve this staged review summary.".into()),
            event: None,
        })
        .unwrap();
    assert_eq!(intent.revision, 1);

    let mut changed: Value = serde_json::from_slice(&fixture.result()).unwrap();
    changed["coverage"]["limitations"] = json!(["Changed generated review"]);
    let failure = store
        .commit(
            &fixture.preparation_id,
            1,
            &serde_json::to_vec(&changed).unwrap(),
            &control,
        )
        .unwrap_err();
    assert_eq!(failure.code, "APPROVAL_REQUIRED");
    assert_eq!(
        store
            .current(&fixture.preparation_id)
            .unwrap()
            .unwrap()
            .result,
        original.result
    );
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn validation_is_read_only_and_commit_is_generation_checked_with_exact_retry_noop() {
    let fixture = Fixture::new();
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    let control = OperationControl::default();

    let validated = store
        .validate(&fixture.preparation_id, 0, &fixture.result(), &control)
        .unwrap();
    assert_eq!(validated.context.context_id, "generated-review-context");
    assert!(store.current(&fixture.preparation_id).unwrap().is_none());
    assert_eq!(
        store.workspace_generation(&fixture.preparation_id).unwrap(),
        0
    );

    let committed = store
        .commit(&fixture.preparation_id, 0, &fixture.result(), &control)
        .unwrap();
    assert_eq!(committed.status, "committed");
    assert_eq!(committed.generation, 1);
    let active = store.current(&fixture.preparation_id).unwrap().unwrap();
    assert_eq!(
        active.result,
        serde_json::from_slice::<ReviewResult>(&fixture.result()).unwrap()
    );
    assert_eq!(active.generation, 1);

    let mut drafts = DraftStore::open(&fixture.state).unwrap();
    let draft_snapshot = drafts.list(&fixture.preparation_id).unwrap();
    drafts
        .save(
            &fixture.preparation_id,
            None,
            "Keep this reviewer note through an exact recommit.",
            &SourceRef {
                path: "src.rs".into(),
                revision: Revision::Head,
                start_line: 1,
                end_line: 1,
            },
            draft_snapshot.workspace_revision,
            0,
            &control,
        )
        .unwrap();

    let repeated = store
        .commit(&fixture.preparation_id, 1, &fixture.result(), &control)
        .unwrap();
    assert_eq!(repeated.status, "unchanged");
    assert_eq!(repeated.generation, 1);
    let after_retry = drafts.list(&fixture.preparation_id).unwrap();
    assert_eq!(after_retry.workspace_revision, 2);
    assert_eq!(after_retry.drafts.len(), 1);

    let mut changed: Value = serde_json::from_slice(&fixture.result()).unwrap();
    changed["coverage"]["limitations"] = json!(["Changed payload"]);
    let blocked = store
        .commit(
            &fixture.preparation_id,
            2,
            &serde_json::to_vec(&changed).unwrap(),
            &control,
        )
        .unwrap_err();
    assert_eq!(blocked.code, "APPROVAL_REQUIRED");
    let still_active = store.current(&fixture.preparation_id).unwrap().unwrap();
    assert_eq!(still_active.result, active.result);
    assert_eq!(
        store.workspace_generation(&fixture.preparation_id).unwrap(),
        2
    );
}

#[test]
fn cli_validate_then_commit_reports_complete_validation_and_persists_result() {
    let fixture = Fixture::new();
    let (valid, validation) = fixture.cli("validate", &fixture.result(), 0);
    assert!(valid, "{validation}");
    assert_eq!(validation["validation"], "complete");
    assert_eq!(validation["expected_generation"], 0);
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    assert!(store.current(&fixture.preparation_id).unwrap().is_none());

    let (committed, output) = fixture.cli("commit", &fixture.result(), 0);
    assert!(committed, "{output}");
    assert_eq!(output["status"], "committed");
    assert_eq!(output["generation"], 1);
    assert_eq!(
        store
            .current(&fixture.preparation_id)
            .unwrap()
            .unwrap()
            .generation,
        1
    );
}

#[test]
fn cli_rejects_invalid_full_result_without_writing() {
    let fixture = Fixture::new();
    let mut invalid: Value = serde_json::from_slice(&fixture.result()).unwrap();
    invalid["findings"] = json!("not an array");
    let (valid, output) = fixture.cli("validate", &serde_json::to_vec(&invalid).unwrap(), 0);
    assert!(!valid);
    assert_eq!(output["code"], "CONTRACT_INVALID");
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    assert!(store.current(&fixture.preparation_id).unwrap().is_none());
    assert_eq!(
        store.workspace_generation(&fixture.preparation_id).unwrap(),
        0
    );
}

#[test]
fn cli_failed_validation_correction_rounds_preserve_committed_result_and_drafts() {
    let fixture = Fixture::new();
    let control = OperationControl::default();
    let store = GeneratedReviewStore::open(&fixture.state).unwrap();
    let committed = store
        .commit(&fixture.preparation_id, 0, &fixture.result(), &control)
        .unwrap();
    assert_eq!(committed.generation, 1);

    let mut drafts = DraftStore::open(&fixture.state).unwrap();
    let before_draft = drafts.list(&fixture.preparation_id).unwrap();
    let saved = drafts
        .save(
            &fixture.preparation_id,
            None,
            "Keep this reviewer draft while analysis is corrected.",
            &SourceRef {
                path: "src.rs".into(),
                revision: Revision::Head,
                start_line: 1,
                end_line: 1,
            },
            before_draft.workspace_revision,
            0,
            &control,
        )
        .unwrap();
    let validation_generation = drafts
        .list(&fixture.preparation_id)
        .unwrap()
        .workspace_revision;

    // Model the initial validation and all three permitted correction rounds.
    // Each candidate fixes the previous error but retains one independent
    // contract error, so the public CLI must reject all four without commit.
    let mut candidates = Vec::new();
    let mut candidate: Value = serde_json::from_slice(&fixture.result()).unwrap();
    candidate["findings"] = json!("not an array");
    candidates.push(candidate.clone());
    candidate["findings"] = json!([]);
    candidate["diagrams"] = json!("not an array");
    candidates.push(candidate.clone());
    candidate["diagrams"] = json!([]);
    candidate["coverage"]["limitations"] = json!("not an array");
    candidates.push(candidate.clone());
    candidate["coverage"]["limitations"] = json!([]);
    candidate["follow_up"] = json!("not an array");
    candidates.push(candidate);

    for (round, candidate) in candidates.iter().enumerate() {
        let (valid, output) = fixture.cli(
            "validate",
            &serde_json::to_vec(candidate).unwrap(),
            validation_generation,
        );
        assert!(!valid, "round {round} unexpectedly validated: {output}");
        assert_eq!(
            output["code"], "CONTRACT_INVALID",
            "round {round}: {output}"
        );
        assert!(
            output["pointer"]
                .as_str()
                .is_some_and(|pointer| !pointer.is_empty()),
            "round {round} did not report an actionable contract pointer: {output}"
        );
    }

    let current = store.current(&fixture.preparation_id).unwrap().unwrap();
    assert_eq!(current.generation, committed.generation);
    assert_eq!(current.result, committed.result);
    let after_draft = drafts.list(&fixture.preparation_id).unwrap();
    assert_eq!(after_draft.drafts.len(), 1);
    assert_eq!(after_draft.drafts[0].id, saved.draft.id);
    assert_eq!(
        after_draft.drafts[0].body,
        "Keep this reviewer draft while analysis is corrected."
    );
}

#[test]
fn cli_replacement_authorization_requires_a_terminal() {
    let fixture = Fixture::new();
    let (authorized, output) = fixture.cli("authorize-replacement", &fixture.result(), 0);
    assert!(!authorized);
    assert_eq!(output["code"], "APPROVAL_TTY_REQUIRED");
}
