use bodh::{OperationControl, PreparedEvidenceContext, validate_complete};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
type DiagramMutation = (fn(&mut Value), &'static str);

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
            "pub fn handle() {}\nprior\nfn parse() {}\nfn note() {}\n",
        )
        .unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "base source"]);
        let base = git(&repo, &["rev-parse", "HEAD"]);
        std::fs::write(
            repo.join("src/service.rs"),
            "pub fn handle() {}\nold\nfn parse() {}\nfn note() {}\n",
        )
        .unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "head source"]);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        let prepared: PreparedEvidenceContext = serde_json::from_value(json!({
            "context": {
                "pr": {"host":"github.com","owner":"example","repository":"demo","number":7},
                "context_id":"fixture",
                "base_sha":base,
                "head_sha":head,
                "merge_base_sha":base
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
                {"kind":"return","id":"parse_return","from":"service","to":"service","label":"Finished parsing","in_reply_to":"parse","source":source},
                {"kind":"note","id":"note","participants":["service"],"text":"The request body is read-only"},
                {"kind":"alt","id":"outcome","label":"Cache outcome","branches":[
                    {"label":"Hit","steps":[
                        {"kind":"message","id":"hit","from":"service","to":"database","label":"Read cached value","source":source},
                        {"kind":"return","id":"hit_return","from":"database","to":"service","label":"Cached value","in_reply_to":"hit","source":source}
                    ]},
                    {"label":"Miss","steps":[
                        {"kind":"message","id":"miss","from":"service","to":"database","label":"Load value","source":source},
                        {"kind":"return","id":"miss_return","from":"database","to":"service","label":"Loaded value","in_reply_to":"miss","source":source}
                    ]}
                ]},
                {"kind":"opt","id":"audit","label":"Audit request","steps":[
                    {"kind":"message","id":"audit_call","from":"service","to":"database","label":"Record request","source":source}
                ]}
            ]
        })
    }

    fn valid_finding(&self) -> Value {
        let source = self.source(1);
        json!({"id":"F_current","title":"Current behavior","severity":"medium","confidence":"high","summary":"A current issue","reason":"The operation has a reliability concern","impact":"A caller may receive a stale result","evidence":[{"source":source,"explanation":"The pinned handler has the relevant path"}],"locations":[{"source":source,"primary":true}],"suggestions":[],"related_thread_ids":[]})
    }

    fn prepare_with_prior(&self) -> PreparedEvidenceContext {
        let mut prepared = self.prepared.clone();
        let mut value = serde_json::to_value(&prepared).unwrap();
        value["prior_concerns"] = json!([
            {"id":"prior_resolved","title":"Resolved concern","summary":"The old implementation could drop data"},
            {"id":"prior_unresolved","title":"Open concern","summary":"The old implementation could drop data"},
            {"id":"prior_regressed","title":"Regressed concern","summary":"The old implementation could drop data"},
            {"id":"prior_uncertain","title":"Uncertain concern","summary":"The old implementation could drop data"}
        ]);
        prepared = serde_json::from_value(value).unwrap();
        prepared
    }

    fn follow_up(&self, prior_id: &str, status: &str, finding_id: Option<&str>) -> Value {
        let mut item = json!({
            "prior_concern_id":prior_id,
            "status":status,
            "explanation":"The current implementation was checked against the old concern.",
            "evidence":[{"source":self.source(1),"explanation":"The pinned implementation supplies current evidence."}]
        });
        if let Some(finding_id) = finding_id {
            item["finding_id"] = json!(finding_id);
        }
        item
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

#[test]
fn diagrams_reject_broken_ids_participants_returns_branches_and_code_links() {
    let f = Fixture::new();
    let mut result = f.result.clone();
    result["diagrams"] = json!([f.valid_diagram()]);
    f.validate(&result).unwrap();

    let invalid_cases = [
        (
            "/steps/0/to",
            json!("missing"),
            "PARTICIPANT_REFERENCE_INVALID",
        ),
        (
            "/steps/1/in_reply_to",
            json!("missing"),
            "STEP_REFERENCE_INVALID",
        ),
        ("/steps/1/from", json!("caller"), "RETURN_ENDPOINTS_INVALID"),
        ("/steps/3/id", json!("service"), "ID_DUPLICATE"),
        (
            "/steps/5/branches/0/label",
            json!(" "),
            "DIAGRAM_CONTENT_INVALID",
        ),
        (
            "/steps/5/branches/0/steps/1/in_reply_to",
            json!("request"),
            "RETURN_ENDPOINTS_INVALID",
        ),
        (
            "/steps/5/branches/1/steps/1/in_reply_to",
            json!("hit"),
            "STEP_REFERENCE_INVALID",
        ),
    ];
    for (pointer, value, code) in invalid_cases {
        let mut bad = result.clone();
        *bad.pointer_mut(&format!("/diagrams/0{pointer}"))
            .unwrap_or_else(|| panic!("missing pointer {pointer}")) = value;
        assert_eq!(f.validate(&bad).unwrap_err().code, code, "{pointer}");
    }

    let invalid_mutations: [DiagramMutation; 5] = [
        (
            |diagram: &mut Value| {
                diagram["steps"][5]["branches"]
                    .as_array_mut()
                    .unwrap()
                    .truncate(1);
            },
            "ALT_BRANCHES_INVALID",
        ),
        (
            |diagram: &mut Value| diagram["steps"][5]["branches"][0]["steps"] = json!([]),
            "ALT_BRANCHES_INVALID",
        ),
        (
            |diagram: &mut Value| {
                diagram["steps"][5]["branches"][1]["steps"][0]["id"] = json!("hit")
            },
            "ID_DUPLICATE",
        ),
        (
            |diagram: &mut Value| {
                diagram["steps"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("source");
            },
            "STEP_SOURCE_REQUIRED",
        ),
        (
            |diagram: &mut Value| diagram["steps"][0]["source"]["path"] = json!("src/missing.rs"),
            "SOURCE_UNAVAILABLE",
        ),
    ];
    for (mutate, code) in invalid_mutations {
        let mut bad = result.clone();
        mutate(&mut bad["diagrams"][0]);
        assert_eq!(f.validate(&bad).unwrap_err().code, code);
    }
}

#[test]
fn follow_up_requires_real_prior_concerns_and_current_actionable_findings() {
    let f = Fixture::new();
    let prepared = f.prepare_with_prior();
    let mut result = f.result.clone();
    result["findings"] = json!([f.valid_finding()]);
    result["follow_up"] = json!([
        f.follow_up("prior_resolved", "resolved", None),
        f.follow_up("prior_unresolved", "unresolved", Some("F_current")),
        f.follow_up("prior_regressed", "regressed", Some("F_current")),
        f.follow_up("prior_uncertain", "uncertain", Some("F_current"))
    ]);
    validate_complete(
        &serde_json::to_vec(&result).unwrap(),
        &prepared,
        &f.repo,
        &OperationControl::default(),
    )
    .unwrap();

    let mut stale = result.clone();
    stale["follow_up"][0]["prior_concern_id"] = json!("unknown");
    let error = validate_complete(
        &serde_json::to_vec(&stale).unwrap(),
        &prepared,
        &f.repo,
        &OperationControl::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, "PRIOR_CONCERN_REFERENCE_INVALID");

    let mut claims_fixed = result.clone();
    claims_fixed["follow_up"][0]["finding_id"] = json!("F_current");
    let error = validate_complete(
        &serde_json::to_vec(&claims_fixed).unwrap(),
        &prepared,
        &f.repo,
        &OperationControl::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, "FOLLOW_UP_RESOLUTION_INVALID");

    let mut hides_open_issue = result.clone();
    hides_open_issue["follow_up"][1]
        .as_object_mut()
        .unwrap()
        .remove("finding_id");
    let error = validate_complete(
        &serde_json::to_vec(&hides_open_issue).unwrap(),
        &prepared,
        &f.repo,
        &OperationControl::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, "FOLLOW_UP_FINDING_REQUIRED");

    let mut no_history = f.prepared.clone();
    no_history.prior_concerns = None;
    let error = validate_complete(
        &serde_json::to_vec(&result).unwrap(),
        &no_history,
        &f.repo,
        &OperationControl::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, "PRIOR_RESULT_UNAVAILABLE");

    let mut duplicate_assessment = result.clone();
    duplicate_assessment["follow_up"][3] = duplicate_assessment["follow_up"][2].clone();
    let error = validate_complete(
        &serde_json::to_vec(&duplicate_assessment).unwrap(),
        &prepared,
        &f.repo,
        &OperationControl::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, "FOLLOW_UP_DUPLICATE");

    let mut missing_finding = result.clone();
    missing_finding["follow_up"][1]["finding_id"] = json!("F_missing");
    let error = validate_complete(
        &serde_json::to_vec(&missing_finding).unwrap(),
        &prepared,
        &f.repo,
        &OperationControl::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, "FOLLOW_UP_FINDING_INVALID");

    let mut empty_prior = f.prepared.clone();
    empty_prior.prior_concerns = Some(vec![]);
    let mut no_assessments = f.result.clone();
    validate_complete(
        &serde_json::to_vec(&no_assessments).unwrap(),
        &empty_prior,
        &f.repo,
        &OperationControl::default(),
    )
    .unwrap();
    no_assessments["follow_up"] = json!([f.follow_up("absent", "uncertain", None)]);
    let error = validate_complete(
        &serde_json::to_vec(&no_assessments).unwrap(),
        &empty_prior,
        &f.repo,
        &OperationControl::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, "PRIOR_CONCERN_REFERENCE_INVALID");
}

#[test]
fn cli_complete_fixture_reports_full_semantic_validation_and_rejects_bad_steps() {
    let f = Fixture::new();
    let mut prepared = f.prepared.clone();
    prepared.original_description = Some("Responds synchronously".into());
    prepared.discussions.thread_ids = vec!["PRRT_fixture".into()];
    prepared.prior_concerns = Some(
        ["resolved", "unresolved", "regressed", "uncertain"]
            .into_iter()
            .map(|status| bodh::PriorConcern {
                id: format!("prior-{status}"),
                title: format!("Prior {status} concern"),
                summary: format!("Prior {status} explanation"),
                evidence: Vec::new(),
            })
            .collect(),
    );
    let dir = std::env::temp_dir().join(format!("bodh-cli-complete-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let context_path = dir.join("context.json");
    let result_path = dir.join("result.json");
    std::fs::write(&context_path, serde_json::to_vec(&prepared).unwrap()).unwrap();
    let mut result: Value =
        serde_json::from_str(include_str!("../contracts/v1/fixtures/all-variants.json")).unwrap();
    result["context"] = serde_json::to_value(&prepared.context).unwrap();
    result["findings"][0]["suggestions"][1]["source"]["start_line"] = json!(2);
    result["findings"][0]["suggestions"][1]["source"]["end_line"] = json!(2);
    std::fs::write(&result_path, serde_json::to_vec(&result).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_bodh"))
        .args(["--json", "validate", "--input"])
        .arg(&result_path)
        .args(["--complete-fixture", "--context"])
        .arg(&context_path)
        .args(["--repository"])
        .arg(&f.repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["data"]["validation"], "complete_fixture_context");
    assert_eq!(envelope["data"]["semantic_validation_complete"], true);
    assert_eq!(envelope["data"]["preparation_authority"], "fixture_only");

    result["diagrams"][0]["steps"][0]
        .as_object_mut()
        .unwrap()
        .remove("source");
    std::fs::write(&result_path, serde_json::to_vec(&result).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_bodh"))
        .args(["validate", "--input"])
        .arg(&result_path)
        .args(["--complete-fixture", "--context"])
        .arg(&context_path)
        .args(["--repository"])
        .arg(&f.repo)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], "STEP_SOURCE_REQUIRED");
    let _ = std::fs::remove_dir_all(dir);
}
