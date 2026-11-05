use bodh::{
    DraftStore, OperationControl, PrIdentity, PreparationStore, ReviewContext, Revision, SourceRef,
    diff_index,
};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

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

fn invoke_cli(args: &[&str], input: Option<&serde_json::Value>) -> Output {
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

struct Fixture {
    repo: PathBuf,
    state: PathBuf,
    preparation_id: String,
    source: SourceRef,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "bodh-manual-drafts-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = root.join("repo");
        let state = root.join("state.sqlite");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(
            repo.join("source.rs"),
            "fn main() {\n    let count = 1;\n}\n",
        )
        .unwrap();
        std::fs::write(repo.join("unchanged.rs"), "fn stable() {}\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "base"]);
        let base = git(&repo, &["rev-parse", "HEAD"]);
        std::fs::write(
            repo.join("source.rs"),
            "fn main() {\n    let count = 2;\n}\n",
        )
        .unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "head"]);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        // Draft validation and storage must leave dirty checkout state alone.
        std::fs::write(repo.join("source.rs"), "uncommitted edit\n").unwrap();
        std::fs::write(repo.join("untracked.txt"), "leave me\n").unwrap();
        let context = ReviewContext {
            pr: PrIdentity {
                host: "github.com".into(),
                owner: "example".into(),
                repository: "demo".into(),
                number: 18,
            },
            context_id: "manual-draft-context".into(),
            base_sha: base.clone(),
            head_sha: head,
            merge_base_sha: base,
        };
        let control = OperationControl::default();
        let files = diff_index(&repo, &context, &control).unwrap().files;
        let binding = PreparationStore::open(&state)
            .unwrap()
            .save(
                &context,
                repo.to_str().unwrap(),
                "Draft fixture",
                Some("Description"),
                &files,
            )
            .unwrap();
        Self {
            repo,
            state,
            preparation_id: binding.preparation_id,
            source: SourceRef {
                path: "source.rs".into(),
                revision: Revision::Head,
                start_line: 2,
                end_line: 2,
            },
        }
    }

    fn snapshot(&self) -> Vec<Vec<u8>> {
        vec![
            git(&self.repo, &["rev-parse", "HEAD"]).into_bytes(),
            git(&self.repo, &["symbolic-ref", "HEAD"]).into_bytes(),
            std::fs::read(self.repo.join(".git/index")).unwrap(),
            std::fs::read(self.repo.join("source.rs")).unwrap(),
            std::fs::read(self.repo.join("untracked.txt")).unwrap(),
        ]
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(root) = self.repo.parent() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

#[test]
fn saved_comment_draft_reopens_with_pinned_provenance_and_revision() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    let mut first = DraftStore::open(&fixture.state).unwrap();
    let empty = first.list(&fixture.preparation_id).unwrap();
    assert_eq!(empty.workspace_revision, 0);
    assert!(empty.drafts.is_empty());

    let saved = first
        .save(
            &fixture.preparation_id,
            None,
            "Please preserve this check.",
            &fixture.source,
            0,
            0,
            &OperationControl::default(),
        )
        .unwrap();
    assert_eq!(saved.workspace_revision, 1);
    assert_eq!(saved.draft.revision, 1);
    assert_eq!(saved.draft.body, "Please preserve this check.");
    assert_eq!(saved.draft.source, fixture.source);
    assert_eq!(saved.draft.provenance.context_id, "manual-draft-context");

    let reopened = DraftStore::open(&fixture.state).unwrap();
    let loaded = reopened.list(&fixture.preparation_id).unwrap();
    assert_eq!(loaded.workspace_revision, 1);
    assert_eq!(loaded.drafts, [saved.draft]);
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn readable_unchanged_source_cannot_become_an_inline_comment_draft() {
    let fixture = Fixture::new();
    let mut store = DraftStore::open(&fixture.state).unwrap();
    let error = store
        .save(
            &fixture.preparation_id,
            None,
            "This is readable but not an inline anchor.",
            &SourceRef {
                path: "unchanged.rs".into(),
                revision: Revision::Head,
                start_line: 1,
                end_line: 1,
            },
            0,
            0,
            &OperationControl::default(),
        )
        .unwrap_err();
    assert_eq!(error.code, "INLINE_ANCHOR_INVALID");
    assert!(
        store
            .list(&fixture.preparation_id)
            .unwrap()
            .drafts
            .is_empty()
    );
}

#[test]
fn concurrent_workspace_and_draft_revisions_reject_stale_writes_without_losing_text() {
    let fixture = Fixture::new();
    let mut first = DraftStore::open(&fixture.state).unwrap();
    let mut second = DraftStore::open(&fixture.state).unwrap();
    let initial = first.list(&fixture.preparation_id).unwrap();
    let created = first
        .save(
            &fixture.preparation_id,
            None,
            "First version.",
            &fixture.source,
            initial.workspace_revision,
            0,
            &OperationControl::default(),
        )
        .unwrap();
    let stale_create = second
        .save(
            &fixture.preparation_id,
            None,
            "Keep this newer editor text locally.",
            &fixture.source,
            initial.workspace_revision,
            0,
            &OperationControl::default(),
        )
        .unwrap_err();
    assert_eq!(stale_create.code, "WORKSPACE_REVISION_STALE");

    let stale_edit = second
        .save(
            &fixture.preparation_id,
            Some(&created.draft.id),
            "A stale edit must not replace persisted text.",
            &fixture.source,
            created.workspace_revision,
            0,
            &OperationControl::default(),
        )
        .unwrap_err();
    assert_eq!(stale_edit.code, "DRAFT_REVISION_STALE");
    let latest = second.list(&fixture.preparation_id).unwrap();
    assert_eq!(latest.drafts[0].body, "First version.");
}

#[test]
fn cli_save_list_and_discard_use_json_envelopes_without_publishing() {
    let fixture = Fixture::new();
    let state = fixture.state.to_str().unwrap();
    let prepared = fixture.preparation_id.as_str();
    let listed = invoke_cli(
        &[
            "--json",
            "draft",
            "list",
            "--prepared",
            prepared,
            "--state",
            state,
        ],
        None,
    );
    assert!(listed.status.success());
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(listed["protocol_version"], "1.0");
    assert_eq!(listed["data"]["workspace_revision"], 0);
    assert_eq!(listed["data"]["drafts"].as_array().unwrap().len(), 0);

    let request = serde_json::json!({
        "draft_id": null,
        "body": "Keep this local until I choose to publish.",
        "source": fixture.source.clone(),
        "expected_workspace_revision": 0,
        "expected_draft_revision": 0
    });
    let saved = invoke_cli(
        &[
            "--json",
            "draft",
            "save",
            "--prepared",
            prepared,
            "--state",
            state,
            "--input",
            "-",
        ],
        Some(&request),
    );
    assert!(saved.status.success());
    let saved: serde_json::Value = serde_json::from_slice(&saved.stdout).unwrap();
    assert_eq!(saved["data"]["workspace_revision"], 1);
    assert_eq!(saved["data"]["counts"]["inline_comments"], 1);
    let draft_id = saved["data"]["draft"]["id"].as_str().unwrap();
    assert_eq!(saved["data"]["draft"]["body"], request["body"]);

    let stale_request = serde_json::json!({
        "draft_id": draft_id,
        "body": "A stale cli writer retains this text locally.",
        "source": fixture.source.clone(),
        "expected_workspace_revision": 0,
        "expected_draft_revision": 1
    });
    let stale = invoke_cli(
        &[
            "--json",
            "draft",
            "save",
            "--prepared",
            prepared,
            "--state",
            state,
            "--input",
            "-",
        ],
        Some(&stale_request),
    );
    assert_eq!(stale.status.code(), Some(3));
    let stale: serde_json::Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(stale["error"]["code"], "WORKSPACE_REVISION_STALE");

    let discard_request = serde_json::json!({
        "draft_id": draft_id,
        "expected_workspace_revision": 1,
        "expected_draft_revision": 1
    });
    let discarded = invoke_cli(
        &[
            "--json",
            "draft",
            "discard",
            "--prepared",
            prepared,
            "--state",
            state,
            "--input",
            "-",
        ],
        Some(&discard_request),
    );
    assert!(discarded.status.success());
    let discarded: serde_json::Value = serde_json::from_slice(&discarded.stdout).unwrap();
    assert_eq!(discarded["data"]["workspace_revision"], 2);
    assert_eq!(discarded["data"]["counts"]["inline_comments"], 0);
    assert!(discarded["data"]["drafts"].as_array().unwrap().is_empty());
}
