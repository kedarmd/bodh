use bodh::{OperationControl, PreparedEvidenceContext, validate_complete};
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
    repo: PathBuf,
    prepared: PreparedEvidenceContext,
    result: Value,
}

impl Fixture {
    fn new() -> Self {
        let repo = std::env::temp_dir().join(format!(
            "bodh-complete-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(
            repo.join("src/service.rs"),
            "pub fn handle() {}\nasync fn request() {}\nfn parse() {}\nfn note() {}\n",
        )
        .unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "pinned source"]);
        let commit = git(&repo, &["rev-parse", "HEAD"]);
        let prepared: PreparedEvidenceContext = serde_json::from_value(json!({
            "context": {
                "pr": {"host":"github.com","owner":"example","repository":"demo","number":7},
                "context_id":"fixture",
                "base_sha":commit,
                "head_sha":commit,
                "merge_base_sha":commit
            },
            "original_description":"Claim one. Claim two.",
            "discussions":{"thread_ids":["PRRT_known"],"complete":true,"failures":[]}
        }))
        .unwrap();
        let mut result: Value =
            serde_json::from_str(include_str!("../contracts/v1/fixtures/empty.json")).unwrap();
        result["context"] = serde_json::to_value(&prepared.context).unwrap();
        Self {
            repo,
            prepared,
            result,
        }
    }

    fn source(&self, line: u32) -> Value {
        json!({"path":"src/service.rs","revision":"head","start_line":line,"end_line":line})
    }

    fn valid_diagram(&self) -> Value {
        let source = self.source(1);
        json!({
            "id":"request_flow",
            "title":"Request flow",
            "description":"The caller invokes the service and receives a response.",
            "participants":[
                {"id":"caller","label":"Caller","category":"actor"},
                {"id":"service","label":"Service","category":"service"},
                {"id":"database","label":"Database","category":"database"}
            ],
            "steps":[
                {"kind":"message","id":"request","from":"caller","to":"service","label":"Handle request","source":source},
                {"kind":"return","id":"response","from":"service","to":"caller","label":"Response","in_reply_to":"request","source":source},
                {"kind":"self_call","id":"parse","participant":"service","label":"Parse request","source":source},
                {"kind":"note","id":"note","participants":["service"],"text":"The request body is read-only"},
                {"kind":"alt","id":"outcome","label":"Cache outcome","branches":[
                    {"label":"Hit","steps":[
                        {"kind":"message","id":"hit","from":"service","to":"database","label":"Read cached value","source":source},
                        {"kind":"return","id":"hit_return","from":"database","to":"service","label":"Cached value","in_reply_to":"hit"}
                    ]},
                    {"label":"Miss","steps":[
                        {"kind":"message","id":"miss","from":"service","to":"database","label":"Load value","source":source}
                    ]}
                ]},
                {"kind":"opt","id":"audit","label":"Audit request","steps":[
                    {"kind":"message","id":"audit_call","from":"service","to":"database","label":"Record request"}
                ]}
            ]
        })
    }

    fn valid_finding(&self) -> Value {
        let source = self.source(1);
        json!({"id":"F_current","title":"Current behavior","severity":"medium","confidence":"high","summary":"A current issue","reason":"The operation has a reliability concern","impact":"A caller may receive a stale result","evidence":[{"source":source,"explanation":"The pinned handler has the relevant path"}],"locations":[{"source":source,"primary":true}],"suggestions":[],"related_thread_ids":[]})
    }

    fn validate(&self, result: &Value) -> Result<bodh::ReviewResult, bodh::ContractError> {
        validate_complete(
            &serde_json::to_vec(result).unwrap(),
            &self.prepared,
            &self.repo,
            &OperationControl::default(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.repo);
    }
}

#[test]
fn complete_validator_accepts_empty_results_and_all_supported_diagram_steps() {
    let f = Fixture::new();
    f.validate(&f.result).unwrap();

    let mut result = f.result.clone();
    result["diagrams"] = json!([f.valid_diagram()]);
    f.validate(&result).unwrap();
}
