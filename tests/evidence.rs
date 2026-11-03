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

#[test]
fn intent_quotes_are_exact_contiguous_original_claim_occurrences() {
    let f = Fixture::new();
    let mut v = serde_json::to_value(f.result()).unwrap();
    v["overview"]["intent_assessments"] = json!([{"quote":"Preserve drafts.","occurrence":2,"status":"partial","explanation":"Head retains the branch","evidence":[{"source":{"path":"source.txt","revision":"head","start_line":2,"end_line":2},"explanation":"Pinned implementation"}],"finding_ids":[]}]);
    validate_evidence(
        &serde_json::to_vec(&v).unwrap(),
        &f.prepared,
        &f.repo,
        &OperationControl::default(),
    )
    .unwrap();
    for (field, value, code) in [
        (
            "quote",
            json!("preserve drafts."),
            "DESCRIPTION_QUOTE_INVALID",
        ),
        ("occurrence", json!(3), "DESCRIPTION_QUOTE_INVALID"),
        ("occurrence", json!(0), "DESCRIPTION_QUOTE_INVALID"),
        ("quote", json!(""), "DESCRIPTION_QUOTE_INVALID"),
    ] {
        let mut bad = v.clone();
        bad["overview"]["intent_assessments"][0][field] = value;
        assert_eq!(
            validate_evidence(
                &serde_json::to_vec(&bad).unwrap(),
                &f.prepared,
                &f.repo,
                &OperationControl::default()
            )
            .unwrap_err()
            .code,
            code
        );
    }
    let mut prepared = f.prepared.clone();
    prepared.original_description = None;
    assert_eq!(
        validate_evidence(
            &serde_json::to_vec(&v).unwrap(),
            &prepared,
            &f.repo,
            &OperationControl::default()
        )
        .unwrap_err()
        .code,
        "DESCRIPTION_UNAVAILABLE"
    );
    let mut bad = v.clone();
    bad["overview"]["intent_assessments"][0]["evidence"][0]["source"]["end_line"] = json!(99);
    assert_eq!(
        validate_evidence(
            &serde_json::to_vec(&bad).unwrap(),
            &f.prepared,
            &f.repo,
            &OperationControl::default()
        )
        .unwrap_err()
        .pointer,
        "/overview/intent_assessments/0/evidence/0/source/end_line"
    );
}

#[test]
fn cancellation_timeout_and_blob_limits_fail_without_checkout_mutation() {
    use std::time::Duration;
    let f = Fixture::new();
    let before = f.snapshot();
    let control = OperationControl::default();
    control.cancel();
    assert_eq!(
        validate_evidence(
            &serde_json::to_vec(&f.result()).unwrap(),
            &f.prepared,
            &f.repo,
            &control
        )
        .unwrap_err()
        .code,
        "CANCELLED"
    );
    let timeout = OperationControl::with_timeout(Duration::ZERO);
    assert_eq!(
        read_source(
            &f.repo,
            &f.prepared.context,
            &SourceRef {
                path: "source.txt".into(),
                revision: Revision::Head,
                start_line: 1,
                end_line: 1
            },
            &timeout
        )
        .unwrap_err()
        .code,
        "OPERATION_TIMEOUT"
    );
    assert_eq!(f.snapshot(), before);
}

#[test]
fn oversized_committed_blobs_have_an_explicit_bound() {
    let f = Fixture::new();
    std::fs::write(
        f.repo.join("large.txt"),
        vec![b'x'; bodh::MAX_SOURCE_BYTES + 1],
    )
    .unwrap();
    git(&f.repo, &["add", "large.txt"]);
    git(&f.repo, &["commit", "-m", "large fixture"]);
    let mut context = f.prepared.context.clone();
    context.head_sha = git(&f.repo, &["rev-parse", "HEAD"]);
    let before = f.snapshot();
    let error = read_source(
        &f.repo,
        &context,
        &SourceRef {
            path: "large.txt".into(),
            revision: Revision::Head,
            start_line: 1,
            end_line: 1,
        },
        &OperationControl::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, "SOURCE_TOO_LARGE");
    assert_eq!(f.snapshot(), before);
}

#[test]
fn key_changes_and_coverage_require_explained_nonempty_content() {
    let f = Fixture::new();
    let good = serde_json::to_value(f.result()).unwrap();
    for (pointer, value) in [
        ("/overview/key_changes/0/evidence", json!([])),
        ("/overview/key_changes/0/evidence/0/explanation", json!(" ")),
        (
            "/coverage/checks",
            json!([{"name":"tests","status":"not_run","details":""}]),
        ),
        (
            "/coverage/excluded_areas",
            json!([{"area":"UI","reason":" "}]),
        ),
        ("/coverage/limitations", json!([""])),
    ] {
        let mut bad = good.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            validate_evidence(
                &serde_json::to_vec(&bad).unwrap(),
                &f.prepared,
                &f.repo,
                &OperationControl::default()
            )
            .unwrap_err()
            .code,
            "EVIDENCE_CONTENT_INVALID"
        );
    }
}
