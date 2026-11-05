use bodh::{ContractError, MAX_INPUT_BYTES, PROTOCOL_VERSION};
use serde_json::{Value, json};
use std::io::Read;

fn read_input(path: &str) -> Result<Vec<u8>, ContractError> {
    let reader: Box<dyn Read> =
        if path == "-" {
            Box::new(std::io::stdin())
        } else {
            Box::new(std::fs::File::open(path).map_err(|_| {
                ContractError::new("INPUT_UNREADABLE", "Cannot open input file", "")
            })?)
        };
    let mut input = Vec::new();
    reader
        .take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut input)
        .map_err(|_| ContractError::new("INPUT_UNREADABLE", "Cannot read input", ""))?;
    Ok(input)
}
fn run() -> Result<Value, ContractError> {
    let mut args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.into_string().map_err(|_| arguments()))
        .collect::<Result<_, _>>()?;
    if args.iter().filter(|a| *a == "--json").count() > 1 {
        return Err(arguments());
    }
    args.retain(|a| a != "--json");
    if args.first().is_some_and(|a| a == "validate")
        && args.iter().any(|a| {
            matches!(
                a.as_str(),
                "--evidence-only" | "--findings-only" | "--complete-fixture"
            )
        })
    {
        return evidence_validation(&args[1..]);
    }
    if args.first().is_some_and(|a| a == "prepare") {
        return prepare_command(&args[1..]);
    }
    if args.first().is_some_and(|a| a == "draft") {
        return draft_command(&args[1..]);
    }
    if args.first().is_some_and(|a| a == "review") {
        return review_command(&args[1..]);
    }
    if args.first().is_some_and(|a| a == "inspect") {
        return inspect_command(&args[1..]);
    }
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["contract", "describe"] | ["--help"] => Ok(bodh::describe()),
        ["contract", "schema"] => Ok(bodh::schema()),
        ["validate", "--input", path, "--structural"]
        | ["validate", "--structural", "--input", path] => {
            bodh::validate_structure(&read_input(path)?)?;
            Ok(
                json!({"valid":true,"validation":"structural_only","semantic_validation_available":false}),
            )
        }
        _ => Err(arguments()),
    }
}
fn options(
    args: &[String],
    allowed: &[&str],
) -> Result<std::collections::BTreeMap<String, String>, ContractError> {
    let mut out = std::collections::BTreeMap::new();
    let mut it = args.iter();
    while let Some(k) = it.next() {
        if !allowed.contains(&k.as_str()) {
            return Err(arguments());
        }
        let v = it.next().ok_or_else(arguments)?;
        if out.insert(k.clone(), v.clone()).is_some() {
            return Err(arguments());
        }
    }
    Ok(out)
}
fn prepare_command(args: &[String]) -> Result<Value, ContractError> {
    let o = options(
        args,
        &["--repo", "--pr", "--remote", "--state", "--timeout-ms"],
    )?;
    let repo = o.get("--repo").ok_or_else(arguments)?;
    let pr = o
        .get("--pr")
        .ok_or_else(arguments)?
        .parse::<u64>()
        .map_err(|_| arguments())?;
    let state = o.get("--state").ok_or_else(arguments)?;
    let ms = o
        .get("--timeout-ms")
        .map(|s| s.parse::<u64>().map_err(|_| arguments()))
        .transpose()?
        .unwrap_or(30_000);
    if ms > 120_000 {
        return Err(arguments());
    }
    let control = bodh::OperationControl::with_timeout(std::time::Duration::from_millis(ms));
    let manifest = bodh::prepare_local_pr(
        std::path::Path::new(repo),
        pr,
        o.get("--remote").map(String::as_str),
        std::path::Path::new(state),
        &control,
    )?;
    with_discussion_summary(
        serde_json::to_value(manifest).expect("manifest serializes"),
        state,
    )
}
fn inspect_command(args: &[String]) -> Result<Value, ContractError> {
    let o = options(
        args,
        &[
            "--prepared",
            "--kind",
            "--state",
            "--path",
            "--revision",
            "--start-line",
            "--end-line",
            "--cursor",
            "--limit",
            "--timeout-ms",
        ],
    )?;
    let id = o.get("--prepared").ok_or_else(arguments)?;
    let kind = o.get("--kind").ok_or_else(arguments)?;
    let state = o.get("--state").ok_or_else(arguments)?;
    let p = bodh::PreparationStore::open(std::path::Path::new(state))?.load(id)?;
    match kind.as_str() {
        "manifest" => with_discussion_summary(
            serde_json::to_value(bodh::PreparationManifest::from(p)).unwrap(),
            state,
        ),
        "description" => Ok(json!({"available":p.description.is_some(),"text":p.description})),
        "files" => bodh::inspect_files(
            &p,
            o.get("--cursor")
                .map(|s| s.parse())
                .transpose()
                .map_err(|_| arguments())?
                .unwrap_or(0),
            o.get("--limit")
                .map(|s| s.parse())
                .transpose()
                .map_err(|_| arguments())?
                .unwrap_or(100),
        ),
        "discussions" => {
            bodh::acquire_discussions(&p, std::path::Path::new(state), &inspect_control(&o)?)
        }
        "previous" => Ok(json!({"available":false,"reason":"NO_PRIOR_RESULT"})),
        "source" => {
            let path = o.get("--path").ok_or_else(arguments)?;
            let revision = match o.get("--revision").map(String::as_str) {
                Some("head") => bodh::Revision::Head,
                Some("base") => bodh::Revision::Base,
                _ => return Err(arguments()),
            };
            let start = o
                .get("--start-line")
                .ok_or_else(arguments)?
                .parse()
                .map_err(|_| arguments())?;
            let end = o
                .get("--end-line")
                .ok_or_else(arguments)?
                .parse()
                .map_err(|_| arguments())?;
            let control = inspect_control(&o)?;
            let source = bodh::SourceRef {
                path: path.clone(),
                revision,
                start_line: start,
                end_line: end,
            };
            let text = bodh::read_source(
                std::path::Path::new(&p.checkout),
                &p.context,
                &source,
                &control,
            )?;
            Ok(json!({"source":source,"text":text,"truncated":false}))
        }
        "diff" => {
            let path = o.get("--path").ok_or_else(arguments)?;
            let cursor = o
                .get("--cursor")
                .map(|s| s.parse())
                .transpose()
                .map_err(|_| arguments())?
                .unwrap_or(0);
            bodh::inspect_diff(&p, path, cursor, &inspect_control(&o)?)
        }
        _ => Err(arguments()),
    }
}
fn with_discussion_summary(mut manifest: Value, state: &str) -> Result<Value, ContractError> {
    let preparation_id = manifest
        .get("preparation_id")
        .and_then(Value::as_str)
        .ok_or_else(arguments)?
        .to_owned();
    let summary = bodh::latest_discussion_summary(std::path::Path::new(state), &preparation_id)?;
    if let Some(object) = manifest.as_object_mut() {
        object.insert(
            "discussions_complete".into(),
            summary
                .get("complete")
                .cloned()
                .unwrap_or(Value::Bool(false)),
        );
        object.insert("discussions".into(), summary);
        let drafts = bodh::DraftStore::open(std::path::Path::new(state))?.list(&preparation_id)?;
        object.insert(
            "expected_generation".into(),
            json!(drafts.workspace_revision),
        );
        object.insert("draft_counts".into(), json!(drafts.counts));
    }
    Ok(manifest)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftSaveInput {
    draft_id: Option<String>,
    body: String,
    source: bodh::SourceRef,
    expected_workspace_revision: u64,
    expected_draft_revision: u64,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftDiscardInput {
    draft_id: String,
    expected_workspace_revision: u64,
    expected_draft_revision: u64,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplacementSaveInput {
    draft_id: Option<String>,
    source: bodh::SourceRef,
    original_text: String,
    replacement: String,
    expected_workspace_revision: u64,
    expected_draft_revision: u64,
}

fn draft_command(args: &[String]) -> Result<Value, ContractError> {
    match args.first().map(String::as_str) {
        Some("list") => {
            let o = options(&args[1..], &["--prepared", "--state"])?;
            let prepared = o.get("--prepared").ok_or_else(arguments)?;
            let state = o.get("--state").ok_or_else(arguments)?;
            let snapshot = bodh::DraftStore::open(std::path::Path::new(state))?.list(prepared)?;
            serde_json::to_value(snapshot).map_err(|_| {
                ContractError::new("SERIALIZATION_ERROR", "Cannot encode draft state", "")
            })
        }
        Some("replacement-list") => {
            let o = options(&args[1..], &["--prepared", "--state"])?;
            let prepared = o.get("--prepared").ok_or_else(arguments)?;
            let state = o.get("--state").ok_or_else(arguments)?;
            let snapshot =
                bodh::DraftStore::open(std::path::Path::new(state))?.list_replacements(prepared)?;
            serde_json::to_value(snapshot).map_err(|_| {
                ContractError::new(
                    "SERIALIZATION_ERROR",
                    "Cannot encode replacement draft state",
                    "",
                )
            })
        }
        Some("replacement-save") => {
            let o = options(
                &args[1..],
                &["--prepared", "--state", "--input", "--timeout-ms"],
            )?;
            let prepared = o.get("--prepared").ok_or_else(arguments)?;
            let state = o.get("--state").ok_or_else(arguments)?;
            let input = o.get("--input").ok_or_else(arguments)?;
            let request: ReplacementSaveInput = serde_json::from_slice(&read_input(input)?)
                .map_err(|_| {
                    ContractError::new(
                        "DRAFT_INPUT_INVALID",
                        "Expected a valid code replacement save request",
                        "",
                    )
                })?;
            let mut store = bodh::DraftStore::open(std::path::Path::new(state))?;
            let saved = store.save_replacement(
                prepared,
                request.draft_id.as_deref(),
                &request.source,
                &request.original_text,
                &request.replacement,
                request.expected_workspace_revision,
                request.expected_draft_revision,
                &inspect_control(&o)?,
            )?;
            serde_json::to_value(saved).map_err(|_| {
                ContractError::new(
                    "SERIALIZATION_ERROR",
                    "Cannot encode saved replacement draft",
                    "",
                )
            })
        }
        Some("replacement-discard") => {
            let o = options(&args[1..], &["--prepared", "--state", "--input"])?;
            let prepared = o.get("--prepared").ok_or_else(arguments)?;
            let state = o.get("--state").ok_or_else(arguments)?;
            let input = o.get("--input").ok_or_else(arguments)?;
            let request: DraftDiscardInput =
                serde_json::from_slice(&read_input(input)?).map_err(|_| {
                    ContractError::new(
                        "DRAFT_INPUT_INVALID",
                        "Expected a valid replacement draft discard request",
                        "",
                    )
                })?;
            let mut store = bodh::DraftStore::open(std::path::Path::new(state))?;
            let snapshot = store.discard_replacement(
                prepared,
                &request.draft_id,
                request.expected_workspace_revision,
                request.expected_draft_revision,
            )?;
            serde_json::to_value(snapshot).map_err(|_| {
                ContractError::new(
                    "SERIALIZATION_ERROR",
                    "Cannot encode discarded replacement state",
                    "",
                )
            })
        }
        Some("save") => {
            let o = options(
                &args[1..],
                &["--prepared", "--state", "--input", "--timeout-ms"],
            )?;
            let prepared = o.get("--prepared").ok_or_else(arguments)?;
            let state = o.get("--state").ok_or_else(arguments)?;
            let input = o.get("--input").ok_or_else(arguments)?;
            let request: DraftSaveInput =
                serde_json::from_slice(&read_input(input)?).map_err(|_| {
                    ContractError::new(
                        "DRAFT_INPUT_INVALID",
                        "Expected a valid draft save request",
                        "",
                    )
                })?;
            let mut store = bodh::DraftStore::open(std::path::Path::new(state))?;
            let saved = store.save(
                prepared,
                request.draft_id.as_deref(),
                &request.body,
                &request.source,
                request.expected_workspace_revision,
                request.expected_draft_revision,
                &inspect_control(&o)?,
            )?;
            serde_json::to_value(saved).map_err(|_| {
                ContractError::new("SERIALIZATION_ERROR", "Cannot encode saved draft", "")
            })
        }
        Some("discard") => {
            let o = options(&args[1..], &["--prepared", "--state", "--input"])?;
            let prepared = o.get("--prepared").ok_or_else(arguments)?;
            let state = o.get("--state").ok_or_else(arguments)?;
            let input = o.get("--input").ok_or_else(arguments)?;
            let request: DraftDiscardInput =
                serde_json::from_slice(&read_input(input)?).map_err(|_| {
                    ContractError::new(
                        "DRAFT_INPUT_INVALID",
                        "Expected a valid draft discard request",
                        "",
                    )
                })?;
            let mut store = bodh::DraftStore::open(std::path::Path::new(state))?;
            let snapshot = store.discard(
                prepared,
                &request.draft_id,
                request.expected_workspace_revision,
                request.expected_draft_revision,
            )?;
            serde_json::to_value(snapshot).map_err(|_| {
                ContractError::new(
                    "SERIALIZATION_ERROR",
                    "Cannot encode discarded draft state",
                    "",
                )
            })
        }
        _ => Err(arguments()),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewStageInput {
    expected_workspace_revision: u64,
    expected_intent_revision: u64,
    #[serde(default)]
    selected: Vec<bodh::SelectedDraft>,
    summary: Option<String>,
    event: Option<bodh::ReviewEvent>,
}

fn review_command(args: &[String]) -> Result<Value, ContractError> {
    match args.first().map(String::as_str) {
        Some("intent") => {
            let o = options(&args[1..], &["--prepared", "--state"])?;
            let prepared = o.get("--prepared").ok_or_else(arguments)?;
            let state = o.get("--state").ok_or_else(arguments)?;
            let intent = bodh::ReviewPublicationStore::open(std::path::Path::new(state))?
                .intent(prepared)?;
            serde_json::to_value(intent).map_err(|_| {
                ContractError::new("SERIALIZATION_ERROR", "Cannot encode review intent", "")
            })
        }
        Some("stage") => {
            let o = options(&args[1..], &["--prepared", "--state", "--input"])?;
            let prepared = o.get("--prepared").ok_or_else(arguments)?;
            let state = o.get("--state").ok_or_else(arguments)?;
            let input = o.get("--input").ok_or_else(arguments)?;
            let request: ReviewStageInput =
                serde_json::from_slice(&read_input(input)?).map_err(|_| {
                    ContractError::new(
                        "REVIEW_INPUT_INVALID",
                        "Expected a valid review staging request",
                        "",
                    )
                })?;
            let mut store = bodh::ReviewPublicationStore::open(std::path::Path::new(state))?;
            let intent = store.stage(bodh::StageReviewRequest {
                state: state.into(),
                preparation_id: prepared.clone(),
                expected_workspace_revision: request.expected_workspace_revision,
                expected_intent_revision: request.expected_intent_revision,
                selected: request.selected,
                summary: request.summary,
                event: request.event,
            })?;
            serde_json::to_value(intent).map_err(|_| {
                ContractError::new("SERIALIZATION_ERROR", "Cannot encode review intent", "")
            })
        }
        Some("confirm") => {
            let o = options(
                &args[1..],
                &[
                    "--prepared",
                    "--state",
                    "--intent-revision",
                    "--timeout-ms",
                    "--gh",
                ],
            )?;
            let prepared = o.get("--prepared").ok_or_else(arguments)?;
            let state = o.get("--state").ok_or_else(arguments)?;
            let expected_intent_revision = o
                .get("--intent-revision")
                .ok_or_else(arguments)?
                .parse::<u64>()
                .map_err(|_| arguments())?;
            let binding =
                bodh::PreparationStore::open(std::path::Path::new(state))?.load(prepared)?;
            let control = inspect_control(&o)?;
            let remote = bodh::inspect_remote_review_with_gh(
                &binding,
                std::path::Path::new(o.get("--gh").map(String::as_str).unwrap_or("gh")),
                &control,
            )?;
            let mut store = bodh::ReviewPublicationStore::open(std::path::Path::new(state))?;
            let operation = store.confirm(
                prepared,
                std::path::Path::new(state),
                expected_intent_revision,
                &remote,
                &control,
            )?;
            serde_json::to_value(operation).map_err(|_| {
                ContractError::new("SERIALIZATION_ERROR", "Cannot encode review plan", "")
            })
        }
        Some("operation") => {
            let o = options(&args[1..], &["--state", "--id"])?;
            let state = o.get("--state").ok_or_else(arguments)?;
            let id = o.get("--id").ok_or_else(arguments)?;
            let operation =
                bodh::ReviewPublicationStore::open(std::path::Path::new(state))?.operation(id)?;
            serde_json::to_value(operation).map_err(|_| {
                ContractError::new("SERIALIZATION_ERROR", "Cannot encode review operation", "")
            })
        }
        _ => Err(arguments()),
    }
}
fn inspect_control(
    o: &std::collections::BTreeMap<String, String>,
) -> Result<bodh::OperationControl, ContractError> {
    let ms = o
        .get("--timeout-ms")
        .map(|s| s.parse::<u64>().map_err(|_| arguments()))
        .transpose()?
        .unwrap_or(30_000);
    if ms > 120_000 {
        return Err(arguments());
    }
    Ok(bodh::OperationControl::with_timeout(
        std::time::Duration::from_millis(ms),
    ))
}
fn evidence_validation(args: &[String]) -> Result<Value, ContractError> {
    let mut options = std::collections::BTreeMap::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if matches!(
            arg.as_str(),
            "--evidence-only" | "--findings-only" | "--complete-fixture"
        ) {
            if options.insert(arg.as_str(), "").is_some() {
                return Err(arguments());
            }
            continue;
        }
        if !matches!(
            arg.as_str(),
            "--input" | "--context" | "--repository" | "--timeout-ms"
        ) {
            return Err(arguments());
        }
        let value = iter.next().ok_or_else(arguments)?;
        if options.insert(arg, value).is_some() {
            return Err(arguments());
        }
    }
    let modes = ["--findings-only", "--evidence-only", "--complete-fixture"];
    if modes
        .iter()
        .filter(|mode| options.contains_key(**mode))
        .count()
        != 1
    {
        return Err(arguments());
    }
    let findings = options.contains_key("--findings-only");
    let complete = options.contains_key("--complete-fixture");
    let input = options.get("--input").ok_or_else(arguments)?;
    let context = options.get("--context").ok_or_else(arguments)?;
    if *input == "-" && *context == "-" {
        return Err(arguments());
    }
    let repository = options.get("--repository").ok_or_else(arguments)?;
    let timeout = options
        .get("--timeout-ms")
        .map(|s| s.parse::<u64>().map_err(|_| arguments()))
        .transpose()?
        .unwrap_or(30_000);
    if timeout > 120_000 {
        return Err(arguments());
    }
    let prepared = bodh::parse_prepared_evidence(&read_input(context)?)?;
    let control = bodh::OperationControl::with_timeout(std::time::Duration::from_millis(timeout));
    let bytes = read_input(input)?;
    if complete {
        bodh::validate_complete(
            &bytes,
            &prepared,
            std::path::Path::new(repository),
            &control,
        )?;
        Ok(
            json!({"valid":true,"validation":"complete_fixture_context","semantic_validation_complete":true,"preparation_authority":"fixture_only"}),
        )
    } else if findings {
        bodh::validate_findings(
            &bytes,
            &prepared,
            std::path::Path::new(repository),
            &control,
        )?;
        Ok(
            json!({"valid":true,"validation":"findings_and_evidence","semantic_validation_complete":false,"remaining_checks":["diagrams_and_follow_up"]}),
        )
    } else {
        bodh::validate_evidence(
            &bytes,
            &prepared,
            std::path::Path::new(repository),
            &control,
        )?;
        Ok(
            json!({"valid":true,"validation":"evidence_only","semantic_validation_complete":false,"remaining_checks":["findings_and_anchors","diagrams_and_follow_up"]}),
        )
    }
}
fn exit_category(error: &ContractError) -> i32 {
    match error.code.as_str() {
        "CONTEXT_MISMATCH"
        | "WORKSPACE_REVISION_STALE"
        | "DRAFT_REVISION_STALE"
        | "REVIEW_INTENT_STALE"
        | "PR_HEAD_MOVED" => 3,
        "AUTH_ACCESS" | "GIT_UNAVAILABLE" | "GH_UNAVAILABLE" | "GIT_OBJECT_UNAVAILABLE" => 4,
        "OPERATION_TIMEOUT" | "CANCELLED" | "NETWORK_UNCERTAIN" => 5,
        "GIT_FAILED" | "STATE_UNAVAILABLE" | "STATE_INVALID" | "GH_RESPONSE_INVALID" => 10,
        _ => 2,
    }
}
fn arguments() -> ContractError {
    ContractError::new(
        "INVALID_ARGUMENTS",
        "Use contract describe/schema, prepare --repo <checkout> --pr <number> --state <database> [--remote <name>], inspect --prepared <id> --kind <manifest|description|files|diff|source|discussions|previous> --state <database>, draft list|save|discard, review intent|stage|confirm|operation, or a validate command; optional --json",
        "",
    )
}
fn main() {
    let (envelope, exit) = match run() {
        Ok(data) => (
            json!({"protocol_version":PROTOCOL_VERSION,"ok":true,"data":data}),
            0,
        ),
        Err(error) => {
            eprintln!("{}: {}", error.code, error.message);
            let exit = exit_category(&error);
            (
                json!({"protocol_version":PROTOCOL_VERSION,"ok":false,"error":error}),
                exit,
            )
        }
    };
    println!("{}", envelope);
    std::process::exit(exit);
}
