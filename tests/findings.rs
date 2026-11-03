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
#[test]
fn inline_comments_use_exact_old_new_hunks_while_unchanged_source_remains_readable() {
    let f = Fixture::new();
    let v = f.result();
    let before = f.snapshot();
    for revision in ["head", "base"] {
        let mut good = v.clone();
        good["findings"][0]["suggestions"] = json!([{"kind":"text_comment","id":"comment","body":"Keep the draft","source":source("source.txt",revision,1,3)}]);
        f.validate(&good).unwrap();
    }
    let mut bad = v.clone();
    bad["findings"][0]["suggestions"] = json!([{"kind":"text_comment","id":"comment","body":"Keep the draft","source":source("source.txt","head",20,20)}]);
    assert_eq!(f.validate(&bad).unwrap_err().code, "INLINE_ANCHOR_INVALID");
    let index =
        bodh::diff_index(&f.repo, &f.prepared.context, &OperationControl::default()).unwrap();
    assert_eq!(index.files.len(), 1);
    assert_eq!(index.files[0].old_path.as_deref(), Some("source.txt"));
    assert_eq!(index.files[0].new_path.as_deref(), Some("source.txt"));
    assert_eq!(
        index.files[0].hunks,
        vec![bodh::DiffHunk {
            old_start: 1,
            old_lines: 5,
            new_start: 1,
            new_lines: 5
        }]
    );
    assert_eq!(f.snapshot(), before);
}
#[test]
fn replacements_require_head_hunk_spans_and_byte_exact_original_lines() {
    let f = Fixture::new();
    let v = f.result();
    let mut good = v.clone();
    good["findings"][0]["suggestions"] = json!([{"kind":"code_replacement","id":"fix","body":"Retain drafts until confirmed","source":source("source.txt","head",1,3),"original_text":"old 1\nnew 2\nold 3\n","replacement":"replacement\n"}]);
    f.validate(&good).unwrap();
    for (pointer, value, code) in [
        (
            "/findings/0/suggestions/0/source/revision",
            json!("base"),
            "REPLACEMENT_SIDE_INVALID",
        ),
        (
            "/findings/0/suggestions/0/original_text",
            json!("dirty original\n"),
            "REPLACEMENT_ORIGINAL_MISMATCH",
        ),
        (
            "/findings/0/suggestions/0/source/end_line",
            json!(20),
            "INLINE_ANCHOR_INVALID",
        ),
    ] {
        let mut bad = good.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert_eq!(f.validate(&bad).unwrap_err().code, code);
    }
    good["findings"][0]["suggestions"][0]["replacement"] = json!("");
    f.validate(&good).unwrap();
}
#[test]
fn additions_deletions_edited_renames_and_exact_crlf_ranges_have_literal_anchors() {
    use std::os::unix::fs::symlink;
    let mut f = Fixture::new();
    let long = (1..=20)
        .map(|i| format!("rename {i}\n"))
        .collect::<String>();
    let pure = (1..=20).map(|i| format!("pure {i}\n")).collect::<String>();
    let split = (1..=40).map(|i| format!("line {i}\n")).collect::<String>();
    for (name, bytes) in [
        ("old name.txt", long.as_bytes()),
        ("pure old.txt", pure.as_bytes()),
        ("deleted.txt", b"gone one\ngone two"),
        ("crlf.txt", b"one\r\nold\r\nlast"),
        ("binary.bin", b"old\0binary"),
        (
            "pointer.txt",
            b"version https://git-lfs.github.com/spec/v1\noid sha256:abcd\nsize 9\n",
        ),
        ("split.txt", split.as_bytes()),
    ] {
        std::fs::write(f.repo.join(name), bytes).unwrap();
    }
    symlink("old name.txt", f.repo.join("link.txt")).unwrap();
    git(
        &f.repo,
        &[
            "add",
            "old name.txt",
            "pure old.txt",
            "deleted.txt",
            "crlf.txt",
            "binary.bin",
            "pointer.txt",
            "split.txt",
            "link.txt",
        ],
    );
    git(
        &f.repo,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &f.prepared.context.head_sha,
            "submodule",
        ],
    );
    git(&f.repo, &["commit", "-m", "variant base"]);
    let base = git(&f.repo, &["rev-parse", "HEAD"]);
    git(&f.repo, &["mv", "old name.txt", "new name.txt"]);
    git(&f.repo, &["mv", "pure old.txt", "pure new.txt"]);
    git(&f.repo, &["rm", "deleted.txt"]);
    std::fs::write(
        f.repo.join("new name.txt"),
        long.replace("rename 2\n", "edited 2\n"),
    )
    .unwrap();
    std::fs::write(f.repo.join("added.txt"), "new one\nnew two\n").unwrap();
    std::fs::write(f.repo.join("crlf.txt"), "one\r\nnew\r\nlast").unwrap();
    std::fs::write(f.repo.join("binary.bin"), b"new\0binary").unwrap();
    std::fs::write(
        f.repo.join("pointer.txt"),
        b"version https://git-lfs.github.com/spec/v1\r\noid sha256:efgh\r\nsize 9\r\n",
    )
    .unwrap();
    std::fs::write(
        f.repo.join("split.txt"),
        split
            .replace("line 2\n", "edit 2\n")
            .replace("line 35\n", "edit 35\n"),
    )
    .unwrap();
    std::fs::remove_file(f.repo.join("link.txt")).unwrap();
    symlink("new name.txt", f.repo.join("link.txt")).unwrap();
    git(
        &f.repo,
        &[
            "add",
            "new name.txt",
            "added.txt",
            "crlf.txt",
            "binary.bin",
            "pointer.txt",
            "split.txt",
            "link.txt",
        ],
    );
    git(
        &f.repo,
        &["update-index", "--cacheinfo", "160000", &base, "submodule"],
    );
    git(&f.repo, &["commit", "-m", "variant head"]);
    let head = git(&f.repo, &["rev-parse", "HEAD"]);
    f.prepared.context.base_sha = base.clone();
    f.prepared.context.merge_base_sha = base;
    f.prepared.context.head_sha = head;
    let marker = f.repo.join("HELPER_RAN");
    let helper = f.repo.join("helper.sh");
    std::fs::write(
        &helper,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    git(
        &f.repo,
        &["config", "diff.external", helper.to_str().unwrap()],
    );
    git(
        &f.repo,
        &["config", "diff.owned.textconv", helper.to_str().unwrap()],
    );
    std::fs::write(f.repo.join(".gitattributes"), "* diff=owned\n").unwrap();
    let before = f.snapshot();
    let v = f.result();
    for (path, side, start, end) in [
        ("new name.txt", "head", 1, 3),
        ("old name.txt", "base", 1, 3),
        ("added.txt", "head", 1, 2),
        ("deleted.txt", "base", 1, 2),
    ] {
        let mut good = v.clone();
        good["findings"][0]["suggestions"] = json!([{"kind":"text_comment","id":"choice","body":"Review this span","source":source(path,side,start,end)}]);
        f.validate(&good).unwrap();
    }
    let mut crlf = v.clone();
    crlf["findings"][0]["suggestions"] = json!([{"kind":"code_replacement","id":"fix","body":"Keep CRLF","source":source("crlf.txt","head",2,3),"original_text":"new\r\nlast","replacement":""}]);
    f.validate(&crlf).unwrap();
    crlf["findings"][0]["suggestions"][0]["original_text"] = json!("new\nlast");
    assert_eq!(
        f.validate(&crlf).unwrap_err().code,
        "REPLACEMENT_ORIGINAL_MISMATCH"
    );
    for (path, side, start, end, code) in [
        ("pure new.txt", "head", 1, 1, "INLINE_ANCHOR_INVALID"),
        ("split.txt", "head", 2, 35, "INLINE_ANCHOR_INVALID"),
        ("binary.bin", "head", 1, 1, "SOURCE_CONTENT_UNSUPPORTED"),
        ("pointer.txt", "head", 1, 1, "SOURCE_CONTENT_UNSUPPORTED"),
        ("link.txt", "head", 1, 1, "SOURCE_CONTENT_UNSUPPORTED"),
        ("submodule", "head", 1, 1, "SOURCE_CONTENT_UNSUPPORTED"),
        ("old name.txt", "head", 1, 1, "SOURCE_UNAVAILABLE"),
        ("new name.txt", "base", 1, 1, "SOURCE_UNAVAILABLE"),
    ] {
        let mut bad = v.clone();
        bad["findings"][0]["suggestions"] = json!([{"kind":"text_comment","id":"choice","body":"Review this span","source":source(path,side,start,end)}]);
        assert_eq!(f.validate(&bad).unwrap_err().code, code, "{path}");
    }
    let index =
        bodh::diff_index(&f.repo, &f.prepared.context, &OperationControl::default()).unwrap();
    let renamed = index
        .files
        .iter()
        .find(|file| file.new_path.as_deref() == Some("new name.txt"))
        .unwrap();
    assert_eq!(renamed.old_path.as_deref(), Some("old name.txt"));
    assert!(renamed.status.starts_with('R'));
    assert_eq!(
        renamed.hunks,
        vec![bodh::DiffHunk {
            old_start: 1,
            old_lines: 5,
            new_start: 1,
            new_lines: 5
        }]
    );
    assert_eq!(
        index
            .files
            .iter()
            .find(|file| file.new_path.as_deref() == Some("added.txt"))
            .unwrap()
            .hunks,
        vec![bodh::DiffHunk {
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: 2
        }]
    );
    assert_eq!(
        index
            .files
            .iter()
            .find(|file| file.old_path.as_deref() == Some("deleted.txt"))
            .unwrap()
            .hunks,
        vec![bodh::DiffHunk {
            old_start: 1,
            old_lines: 2,
            new_start: 0,
            new_lines: 0
        }]
    );
    assert_eq!(
        index
            .files
            .iter()
            .find(|file| file.new_path.as_deref() == Some("submodule"))
            .unwrap()
            .unsupported_reason
            .as_deref(),
        Some("non_regular_entry")
    );
    assert_eq!(f.snapshot(), before);
    assert!(!marker.exists());
}
#[test]
fn related_threads_must_be_observed_and_incomplete_absence_is_explicit() {
    let mut f = Fixture::new();
    let mut v = f.result();
    v["findings"][0]["related_thread_ids"] = json!(["PRRT_actual"]);
    assert_eq!(f.validate(&v).unwrap_err().code, "DISCUSSIONS_INCOMPLETE");
    f.prepared=serde_json::from_value(json!({"context":f.prepared.context,"discussions":{"thread_ids":["PRRT_actual"],"complete":false,"failures":[{"connection":"threads","reason":"page 2 unavailable"}]}})).unwrap();
    f.validate(&v).unwrap();
    v["findings"][0]["related_thread_ids"] = json!(["PRRT_missing"]);
    assert_eq!(f.validate(&v).unwrap_err().code, "DISCUSSIONS_INCOMPLETE");
    f.prepared=serde_json::from_value(json!({"context":f.prepared.context,"discussions":{"thread_ids":["PRRT_actual"],"complete":true,"failures":[]}})).unwrap();
    assert_eq!(f.validate(&v).unwrap_err().code, "THREAD_REFERENCE_INVALID");
    v["findings"][0]["related_thread_ids"] = json!(["PRRT_actual", "PRRT_actual"]);
    assert_eq!(
        f.validate(&v).unwrap_err().code,
        "THREAD_REFERENCE_DUPLICATE"
    );
    for observation in [
        json!({"thread_ids":["PRRT_actual","PRRT_actual"],"complete":true,"failures":[]}),
        json!({"thread_ids":[],"complete":false,"failures":[]}),
        json!({"thread_ids":[],"complete":true,"failures":[{"connection":"threads","reason":"failed"}]}),
    ] {
        f.prepared =
            serde_json::from_value(json!({"context":f.prepared.context,"discussions":observation}))
                .unwrap();
        assert_eq!(
            f.validate(&f.result()).unwrap_err().code,
            "PREPARED_CONTEXT_INVALID"
        );
    }
}
#[test]
fn cli_reports_shared_finding_errors_without_claiming_complete_semantic_validation() {
    use std::{io::Write, process::Stdio};
    let f = Fixture::new();
    let context = f.repo.join("prepared.json");
    std::fs::write(&context, serde_json::to_vec(&f.prepared).unwrap()).unwrap();
    let invoke = |value: &Value, flags: &[&str]| {
        let mut process = Command::new(env!("CARGO_BIN_EXE_bodh"))
            .args([
                "validate",
                "--json",
                "--input",
                "-",
                "--findings-only",
                "--context",
            ])
            .arg(&context)
            .arg("--repository")
            .arg(&f.repo)
            .args(flags)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        process
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(value).unwrap())
            .unwrap();
        let out = process.wait_with_output().unwrap();
        (
            out.status.code().unwrap(),
            serde_json::from_slice::<Value>(&out.stdout).unwrap(),
        )
    };
    let good = f.result();
    let (exit, envelope) = invoke(&good, &[]);
    assert_eq!(exit, 0, "{envelope}");
    assert_eq!(envelope["protocol_version"], "1.0");
    assert_eq!(envelope["data"]["semantic_validation_complete"], false);
    assert_eq!(envelope["data"]["validation"], "findings_and_evidence");
    let mut bad = good.clone();
    bad["findings"][0]["impact"] = json!("");
    let (exit, envelope) = invoke(&bad, &[]);
    assert_eq!(exit, 2);
    assert_eq!(envelope["error"]["code"], "FINDING_CONTENT_INVALID");
    assert_eq!(envelope["error"]["pointer"], "/findings/0/impact");
    let (exit, envelope) = invoke(&good, &["--evidence-only"]);
    assert_eq!(exit, 2);
    assert_eq!(envelope["error"]["code"], "INVALID_ARGUMENTS");
    let mut stale = good.clone();
    stale["context"]["context_id"] = json!("old");
    let (exit, envelope) = invoke(&stale, &[]);
    assert_eq!(exit, 3);
    assert_eq!(envelope["error"]["code"], "CONTEXT_MISMATCH");
}
