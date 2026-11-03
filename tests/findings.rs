use bodh::{OperationControl, PreparedEvidenceContext, validate_findings};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
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
struct Fixture {
    repo: PathBuf,
    prepared: PreparedEvidenceContext,
}
impl Fixture {
    fn new() -> Self {
        let repo = std::env::temp_dir().join(format!(
            "bodh-findings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        let old = (1..=40).map(|i| format!("old {i}\n")).collect::<String>();
        std::fs::write(repo.join("source.txt"), &old).unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "base"]);
        let base = git(&repo, &["rev-parse", "HEAD"]);
        std::fs::write(repo.join("source.txt"), old.replace("old 2\n", "new 2\n")).unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "head"]);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        std::fs::write(repo.join("source.txt"), "DIRTY\n").unwrap();
        std::fs::write(repo.join("untracked.txt"), "keep\n").unwrap();
        let prepared=serde_json::from_value(json!({"context":{"pr":{"host":"github.com","owner":"example","repository":"demo","number":7},"context_id":"fixture","base_sha":base,"head_sha":head,"merge_base_sha":base},"original_description":"Preserve drafts."})).unwrap();
        Self { repo, prepared }
    }
    fn result(&self) -> Value {
        let mut v: Value =
            serde_json::from_str(include_str!("../contracts/v1/fixtures/empty.json")).unwrap();
        v["context"] = serde_json::to_value(&self.prepared.context).unwrap();
        v["findings"] = json!([{"id":"F1","title":"Retain uncertain drafts","severity":"high","confidence":"medium","summary":"Drafts can disappear","reason":"A timeout is not proof of failure","impact":"Reviewer feedback can be lost","evidence":[{"source":source("source.txt","head",2,2),"explanation":"This changed line clears drafts"}],"locations":[{"source":source("source.txt","head",2,2),"primary":true}],"suggestions":[],"related_thread_ids":[]}]);
        v
    }
    fn validate(&self, v: &Value) -> Result<bodh::ReviewResult, bodh::ContractError> {
        validate_findings(
            &serde_json::to_vec(v).unwrap(),
            &self.prepared,
            &self.repo,
            &OperationControl::default(),
        )
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
fn source(path: &str, revision: &str, start: u32, end: u32) -> Value {
    json!({"path":path,"revision":revision,"start_line":start,"end_line":end})
}
#[test]
fn findings_need_explained_source_and_one_primary_without_mutating_checkout() {
    let f = Fixture::new();
    let before = f.snapshot();
    let v = f.result();
    f.validate(&v).unwrap();
    for (pointer, value, code) in [
        ("/findings/0/id", json!(" "), "ID_INVALID"),
        ("/findings/0/impact", json!(""), "FINDING_CONTENT_INVALID"),
        (
            "/findings/0/evidence",
            json!([]),
            "FINDING_EVIDENCE_REQUIRED",
        ),
        (
            "/findings/0/evidence/0/explanation",
            json!(" "),
            "FINDING_CONTENT_INVALID",
        ),
        (
            "/findings/0/evidence/0/source/end_line",
            json!(99),
            "SOURCE_RANGE_INVALID",
        ),
        (
            "/findings/0/locations/0/primary",
            json!(false),
            "PRIMARY_LOCATION_INVALID",
        ),
    ] {
        let mut bad = v.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert_eq!(f.validate(&bad).unwrap_err().code, code, "{pointer}");
    }
    assert_eq!(f.snapshot(), before);
}
#[test]
fn scoped_finding_and_suggestion_ids_and_overview_relationships_resolve_before_use() {
    let f = Fixture::new();
    let v = f.result();
    let mut bad = v.clone();
    bad["findings"]
        .as_array_mut()
        .unwrap()
        .push(v["findings"][0].clone());
    assert_eq!(f.validate(&bad).unwrap_err().code, "ID_DUPLICATE");
    let mut bad = v.clone();
    bad["findings"][0]["locations"]
        .as_array_mut()
        .unwrap()
        .push(v["findings"][0]["locations"][0].clone());
    assert_eq!(
        f.validate(&bad).unwrap_err().code,
        "PRIMARY_LOCATION_INVALID"
    );
    let mut linked = v.clone();
    linked["overview"]["intent_assessments"] = json!([{"quote":"Preserve drafts.","occurrence":1,"status":"mismatch","explanation":"A draft may be lost","evidence":[],"finding_ids":["F1"]}]);
    f.validate(&linked).unwrap();
    linked["overview"]["intent_assessments"][0]["finding_ids"] = json!(["missing"]);
    assert_eq!(
        f.validate(&linked).unwrap_err().code,
        "FINDING_REFERENCE_INVALID"
    );
    let mut unchanged = v.clone();
    unchanged["findings"][0]["locations"][0]["source"] = source("source.txt", "head", 20, 20);
    f.validate(&unchanged).unwrap();
    let comment = json!({"kind":"text_comment","id":"choice","body":"Keep the local draft","source":source("source.txt","head",2,2)});
    let mut alternatives = v.clone();
    alternatives["findings"][0]["suggestions"] = json!([comment.clone(), comment]);
    assert_eq!(f.validate(&alternatives).unwrap_err().code, "ID_DUPLICATE");
}
