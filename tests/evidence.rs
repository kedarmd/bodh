use bodh::{
    OperationControl, PreparedEvidenceContext, ReviewResult, Revision, SourceRef, read_source,
    validate_evidence,
};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    repo: PathBuf,
    prepared: PreparedEvidenceContext,
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
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}
impl Fixture {
    fn new() -> Self {
        let repo = std::env::temp_dir().join(format!(
            "bodh-evidence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("source.txt"), b"one\r\nbase\r\nlast").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "root"]);
        let merge_base = git(&repo, &["rev-parse", "HEAD"]);
        git(&repo, &["checkout", "-b", "feature"]);
        std::fs::write(repo.join("source.txt"), b"one\r\nhead\r\nlast").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "head"]);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        git(&repo, &["checkout", "main"]);
        std::fs::write(repo.join("target.txt"), "target advance\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "target"]);
        let base = git(&repo, &["rev-parse", "HEAD"]);
        std::fs::write(repo.join("source.txt"), "DIRTY\n").unwrap();
        std::fs::write(repo.join("untracked.txt"), "keep me\n").unwrap();
        let prepared = serde_json::from_value(json!({"context":{"pr":{"host":"github.com","owner":"example","repository":"demo","number":7},"context_id":"fixture-immutable","base_sha":base,"head_sha":head,"merge_base_sha":merge_base},"original_description":"Preserve drafts.\r\nPreserve drafts."})).unwrap();
        Self { repo, prepared }
    }
    fn result(&self) -> ReviewResult {
        let mut v: serde_json::Value =
            serde_json::from_str(include_str!("../contracts/v1/fixtures/empty.json")).unwrap();
        v["context"] = serde_json::to_value(&self.prepared.context).unwrap();
        v["overview"]["key_changes"] = json!([{"summary":"Reads pinned source","evidence":[{"source":{"path":"source.txt","revision":"base","start_line":2,"end_line":2},"explanation":"Original text"}]}]);
        serde_json::from_value(v).unwrap()
    }
    fn snapshot(&self) -> Vec<Vec<u8>> {
        vec![
            git(&self.repo, &["rev-parse", "HEAD"]).into_bytes(),
            git(&self.repo, &["symbolic-ref", "HEAD"]).into_bytes(),
            std::fs::read(self.repo.join(".git/index")).unwrap(),
            std::fs::read(self.repo.join("source.txt")).unwrap(),
            std::fs::read(self.repo.join("untracked.txt")).unwrap(),
        ]
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.repo);
    }
}
#[test]
fn review_evidence_reads_pinned_merge_base_and_head_without_touching_dirty_checkout() {
    let f = Fixture::new();
    let before = f.snapshot();
    let bytes = serde_json::to_vec(&f.result()).unwrap();
    validate_evidence(&bytes, &f.prepared, &f.repo, &OperationControl::default()).unwrap();
    let source = SourceRef {
        path: "source.txt".into(),
        revision: Revision::Base,
        start_line: 2,
        end_line: 2,
    };
    assert_eq!(
        read_source(
            &f.repo,
            &f.prepared.context,
            &source,
            &OperationControl::default()
        )
        .unwrap(),
        "base\r\n"
    );
    assert_eq!(
        read_source(
            &f.repo,
            &f.prepared.context,
            &SourceRef {
                revision: Revision::Head,
                start_line: 2,
                end_line: 3,
                ..source
            },
            &OperationControl::default()
        )
        .unwrap(),
        "head\r\nlast"
    );
    assert_eq!(f.snapshot(), before);
}

#[test]
fn evidence_requires_an_exact_valid_preparation_and_real_commit_graph() {
    let f = Fixture::new();
    let mut result = f.result();
    result.context.pr.number = 8;
    assert_eq!(
        validate_evidence(
            &serde_json::to_vec(&result).unwrap(),
            &f.prepared,
            &f.repo,
            &OperationControl::default()
        )
        .unwrap_err()
        .code,
        "CONTEXT_MISMATCH"
    );
    let mut prepared = f.prepared.clone();
    prepared.context.pr.host = "GITHUB.COM".into();
    result.context = prepared.context.clone();
    assert_eq!(
        validate_evidence(
            &serde_json::to_vec(&result).unwrap(),
            &prepared,
            &f.repo,
            &OperationControl::default()
        )
        .unwrap_err()
        .code,
        "PR_IDENTITY_INVALID"
    );
    let mut prepared = f.prepared.clone();
    prepared.context.head_sha = "f".repeat(40);
    result.context = prepared.context.clone();
    result.overview.key_changes.clear();
    assert_eq!(
        validate_evidence(
            &serde_json::to_vec(&result).unwrap(),
            &prepared,
            &f.repo,
            &OperationControl::default()
        )
        .unwrap_err()
        .code,
        "GIT_OBJECT_UNAVAILABLE"
    );
    let mut prepared = f.prepared.clone();
    prepared.context.merge_base_sha = prepared.context.base_sha.clone();
    result.context = prepared.context.clone();
    assert_eq!(
        validate_evidence(
            &serde_json::to_vec(&result).unwrap(),
            &prepared,
            &f.repo,
            &OperationControl::default()
        )
        .unwrap_err()
        .code,
        "PREPARED_CONTEXT_INVALID"
    );
}
