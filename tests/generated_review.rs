use bodh::{
    DraftStore, GeneratedReviewStore, OperationControl, PrIdentity, PreparationStore,
    ReviewContext, ReviewResult, Revision, SourceRef, diff_index,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

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
