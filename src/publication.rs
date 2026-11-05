//! Human-confirmed grouped publication of manual inline review comments.
//!
//! Intent and operation payloads are stored separately from drafts. Network I/O
//! always happens after the journal leaves SQLite; interrupted sends are unknown
//! and deliberately cannot be retried through this API.
use crate::{ContractError, DraftStore, OperationControl, PreparationStore, SourceRef, diff_index};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewEvent {
    Comment,
    Approve,
    RequestChanges,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedDraft {
    pub id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewIntent {
    pub preparation_id: String,
    pub revision: u64,
    pub selected: Vec<SelectedDraft>,
    pub summary: Option<String>,
    /// `None` is a real state; callers must never infer APPROVE.
    pub event: Option<ReviewEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteReviewState {
    pub pull_request_id: String,
    pub head_sha: String,
    pub base_sha: String,
    pub state: String,
    pub viewer_permission: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewPayload {
    pub preparation_id: String,
    pub context_id: String,
    pub pull_request_id: String,
    pub workspace_revision: u64,
    pub intent_revision: u64,
    pub head_sha: String,
    pub base_sha: String,
    pub event: ReviewEvent,
    pub summary: Option<String>,
    pub comments: Vec<ReviewPayloadComment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewPayloadComment {
    pub draft_id: String,
    pub draft_revision: u64,
    pub body: String,
    pub source: SourceRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewOperation {
    pub id: String,
    pub fingerprint: String,
    pub state: String,
    pub payload: ReviewPayload,
    pub remote_review_id: Option<String>,
    pub remote_url: Option<String>,
    pub failure: Option<String>,
}

pub struct ReviewPublicationStore {
    db: Connection,
}

impl ReviewPublicationStore {
    pub fn open(path: &Path) -> Result<Self, ContractError> {
        let db = Connection::open(path).map_err(db_error)?;
        db.busy_timeout(Duration::from_secs(5)).map_err(db_error)?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;")
            .map_err(db_error)?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS review_publication_migrations(version INTEGER PRIMARY KEY); ").map_err(db_error)?;
        let version: Option<i64> = db
            .query_row(
                "SELECT max(version) FROM review_publication_migrations",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?
            .flatten();
        match version.unwrap_or(0) {
            0 => db.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE review_intents(preparation_id TEXT PRIMARY KEY NOT NULL REFERENCES preparations(preparation_id), revision INTEGER NOT NULL, intent_json TEXT NOT NULL, updated_at INTEGER NOT NULL);
                CREATE TABLE review_operations(operation_id TEXT PRIMARY KEY NOT NULL, preparation_id TEXT NOT NULL REFERENCES preparations(preparation_id), fingerprint TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('planned','sending','confirmed','failed','unknown')), payload_json TEXT NOT NULL, remote_review_id TEXT, remote_url TEXT, failure TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
                CREATE TABLE review_published_drafts(operation_id TEXT NOT NULL REFERENCES review_operations(operation_id), preparation_id TEXT NOT NULL REFERENCES preparations(preparation_id), context_id TEXT NOT NULL, draft_id TEXT NOT NULL, draft_revision INTEGER NOT NULL, PRIMARY KEY(operation_id,draft_id,draft_revision));
                CREATE UNIQUE INDEX one_unresolved_review_operation ON review_operations(preparation_id) WHERE state IN ('planned','sending','unknown');
                INSERT INTO review_publication_migrations(version) VALUES(1); COMMIT;").map_err(db_error)?,
            1 => {},
            _ => return Err(ContractError::new("REVIEW_SCHEMA_UNSUPPORTED", "Review publication schema is newer than this application", "")),
        }
        Ok(Self { db })
    }

    pub fn intent(&self, preparation_id: &str) -> Result<ReviewIntent, ContractError> {
        let row: Option<String> = self
            .db
            .query_row(
                "SELECT intent_json FROM review_intents WHERE preparation_id=?1",
                [preparation_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        match row {
            Some(value) => serde_json::from_str(&value).map_err(json_error),
            None => Ok(ReviewIntent {
                preparation_id: preparation_id.into(),
                revision: 0,
                selected: vec![],
                summary: None,
                event: None,
            }),
        }
    }

    /// Saves explicit selection/summary/event with an optimistic content revision.
    pub fn stage(&mut self, request: StageReviewRequest) -> Result<ReviewIntent, ContractError> {
        if request
            .summary
            .as_ref()
            .is_some_and(|s| s.chars().count() > 65_536)
        {
            return Err(invalid(
                "Review summary exceeds the supported length",
                "/summary",
            ));
        }
        let drafts = DraftStore::open(&request.state)?.list(&request.preparation_id)?;
        if drafts.workspace_revision != request.expected_workspace_revision {
            return Err(ContractError::new(
                "WORKSPACE_REVISION_STALE",
                "Workspace content changed; reload before staging the review",
                "",
            ));
        }
        let mut ids = std::collections::HashSet::new();
        for selected in &request.selected {
            if !ids.insert(selected.id.as_str()) {
                return Err(invalid("Selected draft IDs must be unique", "/selected"));
            }
            let Some(draft) = drafts.drafts.iter().find(|d| d.id == selected.id) else {
                return Err(invalid(
                    "Selected draft is no longer available",
                    "/selected",
                ));
            };
            if draft.revision != selected.revision {
                return Err(ContractError::new(
                    "DRAFT_REVISION_STALE",
                    "A selected draft changed; reload before staging the review",
                    "/selected",
                ));
            }
        }
        let current = self.intent(&request.preparation_id)?;
        if current.revision != request.expected_intent_revision {
            return Err(ContractError::new(
                "REVIEW_INTENT_STALE",
                "Review selection changed elsewhere; reload before saving",
                "",
            ));
        }
        let next = current.revision.checked_add(1).ok_or_else(state_error)?;
        let intent = ReviewIntent {
            preparation_id: request.preparation_id,
            revision: next,
            selected: request.selected,
            summary: request.summary,
            event: request.event,
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let now = now_ms();
        tx.execute("INSERT INTO review_intents(preparation_id,revision,intent_json,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(preparation_id) DO UPDATE SET revision=excluded.revision,intent_json=excluded.intent_json,updated_at=excluded.updated_at", params![intent.preparation_id, i64::try_from(next).map_err(|_| state_error())?, serde_json::to_string(&intent).map_err(json_error)?, now]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(intent)
    }

    /// Creates the immutable plan after fresh remote observations and anchor checks.
    /// This operation only journals intent; `send` is a separate explicit action.
    pub fn confirm(
        &mut self,
        preparation_id: &str,
        state: &Path,
        expected_intent_revision: u64,
        remote: &RemoteReviewState,
        control: &OperationControl,
    ) -> Result<ReviewOperation, ContractError> {
        let binding = PreparationStore::open(state)?.load(preparation_id)?;
        let intent = self.intent(preparation_id)?;
        if intent.revision != expected_intent_revision {
            return Err(ContractError::new(
                "REVIEW_INTENT_STALE",
                "Review selection changed; reopen Finish Review",
                "",
            ));
        }
        let event = intent.event.ok_or_else(|| {
            invalid(
                "Choose a review event explicitly before confirmation",
                "/event",
            )
        })?;
        if !matches!(remote.state.as_str(), "OPEN") {
            return Err(ContractError::new(
                "PR_NOT_OPEN",
                "The pull request is not open",
                "",
            ));
        }
        if !matches!(
            remote.viewer_permission.as_str(),
            "WRITE" | "MAINTAIN" | "ADMIN"
        ) {
            return Err(ContractError::new(
                "REVIEW_PERMISSION_DENIED",
                "Current account cannot submit a review",
                "",
            ));
        }
        if remote.head_sha != binding.context.head_sha
            || remote.base_sha != binding.context.base_sha
        {
            return Err(ContractError::new(
                "PR_HEAD_MOVED",
                "The remote PR head or target changed; reload before publishing",
                "",
            ));
        }
        if remote.pull_request_id.trim().is_empty() {
            return Err(invalid("GitHub did not return a pull request node ID", ""));
        }
        if intent.selected.is_empty() && intent.summary.as_deref().unwrap_or("").trim().is_empty() {
            return Err(invalid(
                "Select at least one comment or write a review summary",
                "",
            ));
        }
        if event == ReviewEvent::RequestChanges
            && intent.summary.as_deref().unwrap_or("").trim().is_empty()
        {
            return Err(invalid(
                "Request changes requires a review summary",
                "/summary",
            ));
        }
        let draft_snapshot = DraftStore::open(state)?.list(preparation_id)?;
        let index = diff_index(Path::new(&binding.checkout), &binding.context, control)?;
        let mut comments = Vec::with_capacity(intent.selected.len());
        for selected in &intent.selected {
            let draft = draft_snapshot
                .drafts
                .iter()
                .find(|d| d.id == selected.id)
                .ok_or_else(|| invalid("Selected draft no longer exists", "/selected"))?;
            if draft.revision != selected.revision {
                return Err(ContractError::new(
                    "DRAFT_REVISION_STALE",
                    "A selected draft changed; reopen Finish Review",
                    "/selected",
                ));
            }
            index.validate_anchor(&draft.source)?;
            comments.push(ReviewPayloadComment {
                draft_id: draft.id.clone(),
                draft_revision: draft.revision,
                body: draft.body.clone(),
                source: draft.source.clone(),
            });
        }
        let payload = ReviewPayload {
            preparation_id: preparation_id.into(),
            context_id: binding.context.context_id,
            pull_request_id: remote.pull_request_id.clone(),
            workspace_revision: draft_snapshot.workspace_revision,
            intent_revision: intent.revision,
            head_sha: binding.context.head_sha,
            base_sha: binding.context.base_sha,
            event,
            summary: intent.summary,
            comments,
        };
        let payload_json = serde_json::to_string(&payload).map_err(json_error)?;
        let fingerprint = format!("{:x}", Sha256::digest(payload_json.as_bytes()));
        let id = format!("review-{}", &fingerprint[..24]);
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let now = now_ms();
        let inserted = tx.execute("INSERT OR IGNORE INTO review_operations(operation_id,preparation_id,fingerprint,state,payload_json,created_at,updated_at) VALUES(?1,?2,?3,'planned',?4,?5,?5)", params![id, preparation_id, fingerprint, payload_json, now]).map_err(db_error)?;
        if inserted == 0 {
            let unresolved: Option<String> = tx.query_row("SELECT operation_id FROM review_operations WHERE preparation_id=?1 AND state IN ('planned','sending','unknown')", [preparation_id], |r| r.get(0)).optional().map_err(db_error)?;
            return Err(ContractError::new(
                if unresolved.is_some() {
                    "REVIEW_OPERATION_UNRESOLVED"
                } else {
                    "REVIEW_ALREADY_PLANNED"
                },
                "This exact review plan already exists or another send needs reconciliation",
                "",
            ));
        }
        tx.commit().map_err(db_error)?;
        Ok(ReviewOperation {
            id,
            fingerprint,
            state: "planned".into(),
            payload,
            remote_review_id: None,
            remote_url: None,
            failure: None,
        })
    }

    /// Begins the only permitted send transition before leaving SQLite.
    pub fn begin_send(
        &mut self,
        operation_id: &str,
        remote: &RemoteReviewState,
    ) -> Result<ReviewOperation, ContractError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let operation = load_operation(&tx, operation_id)?;
        if operation.state != "planned" {
            return Err(ContractError::new(
                "REVIEW_SEND_NOT_ALLOWED",
                "Only a newly planned review can be sent; reconcile sending or unknown operations first",
                "",
            ));
        }
        verify_remote_state(&operation.payload, remote)?;
        tx.execute("UPDATE review_operations SET state='sending',updated_at=?1 WHERE operation_id=?2 AND state='planned'", params![now_ms(), operation_id]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        let mut updated = operation;
        updated.state = "sending".into();
        Ok(updated)
    }

    pub fn finish_send(
        &mut self,
        operation_id: &str,
        outcome: SendOutcome,
    ) -> Result<ReviewOperation, ContractError> {
        let (state, remote_id, url, failure) = match outcome {
            SendOutcome::Confirmed { remote_id, url } if !remote_id.trim().is_empty() => {
                ("confirmed", Some(remote_id), url, None)
            }
            SendOutcome::Rejected { message } => ("failed", None, None, Some(bound(&message))),
            SendOutcome::Uncertain { message } => ("unknown", None, None, Some(bound(&message))),
            SendOutcome::Confirmed { .. } => (
                "unknown",
                None,
                None,
                Some("GitHub accepted a response without a review ID".into()),
            ),
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let payload_json:Option<String>=tx.query_row("SELECT payload_json FROM review_operations WHERE operation_id=?1 AND state='sending'",[operation_id],|row|row.get(0)).optional().map_err(db_error)?;
        let Some(payload_json) = payload_json else {
            return Err(ContractError::new(
                "REVIEW_SEND_STATE_INVALID",
                "Review operation is not awaiting an acknowledgment",
                "",
            ));
        };
        let payload: ReviewPayload = serde_json::from_str(&payload_json).map_err(json_error)?;
        let changed = tx.execute("UPDATE review_operations SET state=?1,remote_review_id=?2,remote_url=?3,failure=?4,updated_at=?5 WHERE operation_id=?6 AND state='sending'", params![state, remote_id, url, failure, now_ms(), operation_id]).map_err(db_error)?;
        if changed != 1 {
            return Err(ContractError::new(
                "REVIEW_SEND_STATE_INVALID",
                "Review operation is not awaiting an acknowledgment",
                "",
            ));
        }
        if state == "confirmed" {
            for comment in payload.comments {
                tx.execute("INSERT OR IGNORE INTO review_published_drafts(operation_id,preparation_id,context_id,draft_id,draft_revision) VALUES(?1,?2,?3,?4,?5)",params![operation_id,payload.preparation_id,payload.context_id,comment.draft_id,i64::try_from(comment.draft_revision).map_err(|_|state_error())?]).map_err(db_error)?;
            }
        }
        tx.commit().map_err(db_error)?;
        self.operation(operation_id)
    }

    pub fn operation(&self, operation_id: &str) -> Result<ReviewOperation, ContractError> {
        load_operation(&self.db, operation_id)
    }

    /// Returns an unresolved operation for the review so a restart can present
    /// the immutable plan instead of silently dropping or resending it.
    pub fn pending_operation(
        &self,
        preparation_id: &str,
    ) -> Result<Option<ReviewOperation>, ContractError> {
        let id:Option<String>=self.db.query_row("SELECT operation_id FROM review_operations WHERE preparation_id=?1 AND state IN ('planned','sending','unknown') ORDER BY created_at DESC LIMIT 1",[preparation_id],|row|row.get(0)).optional().map_err(db_error)?;
        id.map(|id| self.operation(&id)).transpose()
    }

    /// A process restart turns a persisted `sending` operation into `unknown`.
    pub fn recover_interrupted(
        &mut self,
        preparation_id: &str,
    ) -> Result<Option<ReviewOperation>, ContractError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let id: Option<String> = tx.query_row("SELECT operation_id FROM review_operations WHERE preparation_id=?1 AND state='sending'", [preparation_id], |r| r.get(0)).optional().map_err(db_error)?;
        if let Some(id) = &id {
            tx.execute("UPDATE review_operations SET state='unknown',failure='Process ended while awaiting GitHub acknowledgment; inspect remote state before proceeding',updated_at=?1 WHERE operation_id=?2", params![now_ms(), id]).map_err(db_error)?;
        }
        tx.commit().map_err(db_error)?;
        id.map(|id| self.operation(&id)).transpose()
    }

    /// Runs the grouped mutation only after the durable `sending` transition.
    pub fn publish_confirmed(
        &mut self,
        operation_id: &str,
        remote: &RemoteReviewState,
        publisher: &dyn ReviewPublisher,
        control: &OperationControl,
    ) -> Result<ReviewOperation, ContractError> {
        let sending = self.begin_send(operation_id, remote)?;
        let outcome = publisher.publish(&sending.payload, control);
        self.finish_send(operation_id, outcome)
    }
}

#[derive(Debug, Clone)]
pub struct StageReviewRequest {
    pub state: PathBuf,
    pub preparation_id: String,
    pub expected_workspace_revision: u64,
    pub expected_intent_revision: u64,
    pub selected: Vec<SelectedDraft>,
    pub summary: Option<String>,
    pub event: Option<ReviewEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    Confirmed {
        remote_id: String,
        url: Option<String>,
    },
    Rejected {
        message: String,
    },
    Uncertain {
        message: String,
    },
}

/// Minimal transport boundary makes controlled `gh` fixtures replace publication safely.
pub trait ReviewPublisher {
    fn publish(&self, payload: &ReviewPayload, control: &OperationControl) -> SendOutcome;
}

pub struct GhReviewPublisher {
    pub executable: PathBuf,
    pub checkout: PathBuf,
    pub timeout: Duration,
}
impl ReviewPublisher for GhReviewPublisher {
    fn publish(&self, payload: &ReviewPayload, control: &OperationControl) -> SendOutcome {
        let comments: Vec<Value> = payload
            .comments
            .iter()
            .map(|c| {
                let mut item = json!({"path": c.source.path, "body": c.body});
                if let Some(map) = item.as_object_mut() {
                    let revision = format!("{:?}", c.source.revision).to_ascii_uppercase();
                    map.insert(
                        "side".into(),
                        Value::String(if revision == "HEAD" { "RIGHT" } else { "LEFT" }.into()),
                    );
                    map.insert("line".into(), json!(c.source.end_line));
                    if c.source.start_line != c.source.end_line {
                        map.insert("startLine".into(), json!(c.source.start_line));
                        map.insert(
                            "startSide".into(),
                            Value::String(if revision == "HEAD" { "RIGHT" } else { "LEFT" }.into()),
                        );
                    }
                }
                item
            })
            .collect();
        let mutation = "mutation($pullRequestId:ID!,$event:PullRequestReviewEvent!,$body:String,$commitOID:GitObjectID!,$threads:[DraftPullRequestReviewThread!]){addPullRequestReview(input:{pullRequestId:$pullRequestId,event:$event,body:$body,commitOID:$commitOID,threads:$threads}){pullRequestReview{id url state}}}";
        let event = match payload.event {
            ReviewEvent::Comment => "COMMENT",
            ReviewEvent::Approve => "APPROVE",
            ReviewEvent::RequestChanges => "REQUEST_CHANGES",
        };
        let input = json!({"query":mutation,"variables":{"pullRequestId":payload.pull_request_id,"event":event,"body":payload.summary,"commitOID":payload.head_sha,"threads":comments}});
        let output = match run_gh_json(
            &self.executable,
            &self.checkout,
            &input,
            control,
            self.timeout,
        ) {
            Ok(output) => output,
            Err(ProcessFailure::NotStarted(message)) => return SendOutcome::Rejected { message },
            Err(ProcessFailure::Uncertain(message)) => return SendOutcome::Uncertain { message },
        };
        if output.status.success() {
            match serde_json::from_slice::<Value>(&output.stdout)
                .ok()
                .and_then(|v| {
                    v.pointer("/data/addPullRequestReview/pullRequestReview")
                        .cloned()
                }) {
                Some(review) if review.get("id").and_then(Value::as_str).is_some() => {
                    SendOutcome::Confirmed {
                        remote_id: review["id"].as_str().unwrap_or_default().to_owned(),
                        url: review.get("url").and_then(Value::as_str).map(str::to_owned),
                    }
                }
                _ => SendOutcome::Uncertain {
                    message: "GitHub returned no identifiable review acknowledgment".into(),
                },
            }
        } else {
            let message = String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(2048)
                .collect::<String>();
            if message.contains("GraphQL:") || message.contains("HTTP 4") {
                SendOutcome::Rejected { message }
            } else {
                SendOutcome::Uncertain {
                    message: if message.is_empty() {
                        "gh exited without a definitive GitHub response".into()
                    } else {
                        message
                    },
                }
            }
        }
    }
}

enum ProcessFailure {
    NotStarted(String),
    Uncertain(String),
}

fn run_gh_json(
    program: &Path,
    cwd: &Path,
    input: &Value,
    control: &OperationControl,
    timeout: Duration,
) -> Result<std::process::Output, ProcessFailure> {
    run_gh_process(
        program,
        cwd,
        &["api", "graphql", "--input", "-"],
        Some(input),
        control,
        timeout,
    )
}
fn run_gh_process(
    program: &Path,
    cwd: &Path,
    args: &[&str],
    input: Option<&Value>,
    control: &OperationControl,
    timeout: Duration,
) -> Result<std::process::Output, ProcessFailure> {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| ProcessFailure::NotStarted(format!("Cannot start gh: {e}")))?;
    let out = child.stdout.take().expect("piped stdout");
    let err = child.stderr.take().expect("piped stderr");
    let out_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = out.take(1_048_577).read_to_end(&mut bytes);
        (result, bytes)
    });
    let err_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = err.take(65_537).read_to_end(&mut bytes);
        (result, bytes)
    });
    if let Some(input) = input
        && let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(input.to_string().as_bytes()).is_err()
    {
        kill_process_group(&mut child);
        let _ = child.wait();
        let _ = out_reader.join();
        let _ = err_reader.join();
        return Err(ProcessFailure::Uncertain(
            "Could not deliver the prepared request after starting gh".into(),
        ));
    }
    let started = std::time::Instant::now();
    let status = loop {
        if control.check().is_err() || started.elapsed() >= timeout {
            kill_process_group(&mut child);
            let _ = child.wait();
            let _ = out_reader.join();
            let _ = err_reader.join();
            return Err(ProcessFailure::Uncertain(
                "Timed out or cancelled after starting GitHub publication; inspect remote state"
                    .into(),
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                kill_process_group(&mut child);
                let _ = child.wait();
                let _ = out_reader.join();
                let _ = err_reader.join();
                return Err(ProcessFailure::Uncertain(format!(
                    "Could not determine publication outcome: {e}"
                )));
            }
        }
    };
    let (_, stdout) = out_reader
        .join()
        .map_err(|_| ProcessFailure::Uncertain("Could not read GitHub response".into()))?;
    let (_, stderr) = err_reader
        .join()
        .map_err(|_| ProcessFailure::Uncertain("Could not read GitHub diagnostic".into()))?;
    if stdout.len() > 1_048_576 || stderr.len() > 65_536 {
        return Err(ProcessFailure::Uncertain(
            "GitHub response exceeded the supported size".into(),
        ));
    }
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}
fn kill_process_group(child: &mut std::process::Child) {
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
}

/// Obtains the mutable PR lifecycle/head/target/permission observation immediately
/// before confirmation. It is deliberately separate from pinned local preparation.
pub fn inspect_remote_review_with_gh(
    binding: &crate::PreparedBinding,
    gh: &Path,
    control: &OperationControl,
) -> Result<RemoteReviewState, ContractError> {
    let number = binding.context.pr.number.to_string();
    let args = [
        "pr",
        "view",
        number.as_str(),
        "--json",
        "id,headRefOid,baseRefOid,state,viewerPermission",
    ];
    let output = run_gh_process(
        gh,
        Path::new(&binding.checkout),
        &args,
        None,
        control,
        Duration::from_secs(15),
    )
    .map_err(|e| match e {
        ProcessFailure::NotStarted(message) => ContractError::new("GH_UNAVAILABLE", &message, ""),
        ProcessFailure::Uncertain(message) => ContractError::new("NETWORK_UNCERTAIN", &message, ""),
    })?;
    if !output.status.success() {
        return Err(ContractError::new(
            "GH_PREFLIGHT_FAILED",
            &String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(2048)
                .collect::<String>(),
            "",
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        ContractError::new(
            "GH_RESPONSE_INVALID",
            "GitHub PR preflight returned invalid JSON",
            "",
        )
    })?;
    let get = |key| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let state = get("state").ok_or_else(|| {
        ContractError::new("GH_RESPONSE_INVALID", "PR preflight omitted state", "")
    })?;
    let permission = get("viewerPermission").ok_or_else(|| {
        ContractError::new(
            "GH_RESPONSE_INVALID",
            "PR preflight omitted viewer permission",
            "",
        )
    })?;
    let head_sha = get("headRefOid").ok_or_else(|| {
        ContractError::new("GH_RESPONSE_INVALID", "PR preflight omitted head SHA", "")
    })?;
    let base_sha = get("baseRefOid").ok_or_else(|| {
        ContractError::new("GH_RESPONSE_INVALID", "PR preflight omitted base SHA", "")
    })?;
    let pull_request_id = get("id").ok_or_else(|| {
        ContractError::new("GH_RESPONSE_INVALID", "PR preflight omitted node ID", "")
    })?;
    Ok(RemoteReviewState {
        pull_request_id,
        head_sha,
        base_sha,
        state,
        viewer_permission: permission,
    })
}

type OperationRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn load_operation(db: &Connection, id: &str) -> Result<ReviewOperation, ContractError> {
    let row: Option<OperationRow> = db.query_row("SELECT fingerprint,state,payload_json,remote_review_id,remote_url,failure FROM review_operations WHERE operation_id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(db_error)?;
    let Some((fingerprint, state, payload, remote_id, url, failure)) = row else {
        return Err(ContractError::new(
            "REVIEW_OPERATION_NOT_FOUND",
            "Review operation was not found",
            "",
        ));
    };
    Ok(ReviewOperation {
        id: id.into(),
        fingerprint,
        state,
        payload: serde_json::from_str(&payload).map_err(json_error)?,
        remote_review_id: remote_id,
        remote_url: url,
        failure,
    })
}
fn verify_remote_state(
    payload: &ReviewPayload,
    remote: &RemoteReviewState,
) -> Result<(), ContractError> {
    if remote.pull_request_id != payload.pull_request_id
        || remote.head_sha != payload.head_sha
        || remote.base_sha != payload.base_sha
    {
        return Err(ContractError::new(
            "PR_HEAD_MOVED",
            "The pull request changed after confirmation; return to Finish Review and inspect again",
            "",
        ));
    }
    if remote.state != "OPEN" {
        return Err(ContractError::new(
            "PR_NOT_OPEN",
            "The pull request is not open",
            "",
        ));
    }
    if !matches!(
        remote.viewer_permission.as_str(),
        "WRITE" | "MAINTAIN" | "ADMIN"
    ) {
        return Err(ContractError::new(
            "REVIEW_PERMISSION_DENIED",
            "Current account cannot submit a review",
            "",
        ));
    }
    Ok(())
}
fn invalid(message: &str, pointer: &str) -> ContractError {
    ContractError::new("REVIEW_PLAN_INVALID", message, pointer)
}
fn state_error() -> ContractError {
    ContractError::new(
        "STATE_INVALID",
        "Review state could not be persisted safely",
        "",
    )
}
fn json_error(_: serde_json::Error) -> ContractError {
    state_error()
}
fn db_error(_: rusqlite::Error) -> ContractError {
    state_error()
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn bound(s: &str) -> String {
    s.chars().take(2048).collect()
}
