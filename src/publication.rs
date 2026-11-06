//! Human-confirmed grouped publication of manual inline review comments.
//!
//! Intent and operation payloads are stored separately from drafts. Network I/O
//! always happens after the journal leaves SQLite; interrupted sends are unknown
//! and deliberately cannot be retried through this API.
use crate::{
    ContractError, DraftStore, OperationControl, PreparationStore, Revision, SourceRef, diff_index,
    read_source,
};
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
    #[serde(default)]
    pub selected_replacements: Vec<SelectedDraft>,
    #[serde(default)]
    pub selected_replies: Vec<SelectedDraft>,
    #[serde(default)]
    pub selected_thread_actions: Vec<SelectedDraft>,
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
    #[serde(default)]
    pub suggestions: Vec<ReviewPayloadSuggestion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewPayloadComment {
    pub draft_id: String,
    pub draft_revision: u64,
    pub body: String,
    pub source: SourceRef,
}

/// A code suggestion is constructed from a structured human replacement and
/// binds the exact source text and draft revision used to build the body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewPayloadSuggestion {
    pub draft_id: String,
    pub draft_revision: u64,
    pub source: SourceRef,
    pub original_text: String,
    pub replacement: String,
    pub body: String,
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
    /// UTC Unix milliseconds when the durable send transition began.
    #[serde(default)]
    pub send_started_at: Option<i64>,
}

pub struct ReviewPublicationStore {
    db: Connection,
    path: PathBuf,
}

/// Encodes replacement text as GitHub's suggestion block. A longer fence is
/// used when the replacement itself contains a backtick fence.
pub fn suggestion_markdown(replacement: &str) -> Result<String, ContractError> {
    if replacement.chars().count() > 65_536 {
        return Err(invalid(
            "Replacement exceeds the supported length",
            "/replacement",
        ));
    }
    let mut longest_run = 0usize;
    let mut current_run = 0usize;
    for ch in replacement.chars() {
        if ch == '`' {
            current_run += 1;
            longest_run = longest_run.max(current_run);
        } else {
            current_run = 0;
        }
    }
    let fence = "`".repeat(3usize.max(longest_run.saturating_add(1)));
    let mut body = format!("{fence}suggestion\n{replacement}");
    if !replacement.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(&fence);
    Ok(body)
}

impl ReviewPublicationStore {
    pub fn open(path: &Path) -> Result<Self, ContractError> {
        let db = Connection::open(path).map_err(db_error)?;
        crate::sqlite::configure(&db).map_err(db_error)?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS review_publication_migrations(version INTEGER PRIMARY KEY); ").map_err(db_error)?;
        db.execute_batch("BEGIN IMMEDIATE;").map_err(db_error)?;
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
            0 => db.execute_batch("
                CREATE TABLE review_intents(preparation_id TEXT PRIMARY KEY NOT NULL REFERENCES preparations(preparation_id), revision INTEGER NOT NULL, intent_json TEXT NOT NULL, updated_at INTEGER NOT NULL);
                CREATE TABLE review_operations(operation_id TEXT PRIMARY KEY NOT NULL, preparation_id TEXT NOT NULL REFERENCES preparations(preparation_id), fingerprint TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('planned','sending','confirmed','failed','unknown')), payload_json TEXT NOT NULL, remote_review_id TEXT, remote_url TEXT, failure TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, send_started_at INTEGER);
                CREATE TABLE review_published_drafts(operation_id TEXT NOT NULL REFERENCES review_operations(operation_id), preparation_id TEXT NOT NULL REFERENCES preparations(preparation_id), context_id TEXT NOT NULL, draft_id TEXT NOT NULL, draft_revision INTEGER NOT NULL, PRIMARY KEY(operation_id,draft_id,draft_revision));
                CREATE UNIQUE INDEX one_unresolved_review_operation ON review_operations(preparation_id) WHERE state IN ('planned','sending','unknown');
                INSERT INTO review_publication_migrations(version) VALUES(2);").map_err(db_error)?,
            1 => {
                db.execute_batch("ALTER TABLE review_operations ADD COLUMN send_started_at INTEGER; INSERT INTO review_publication_migrations(version) VALUES(2);").map_err(db_error)?;
            },
            2 => {},
            _ => {
                let _ = db.execute_batch("ROLLBACK;");
                return Err(ContractError::new("REVIEW_SCHEMA_UNSUPPORTED", "Review publication schema is newer than this application", ""));
            },
        }
        db.execute_batch("COMMIT;").map_err(db_error)?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS review_published_replacements(operation_id TEXT NOT NULL REFERENCES review_operations(operation_id), preparation_id TEXT NOT NULL REFERENCES preparations(preparation_id), context_id TEXT NOT NULL, draft_id TEXT NOT NULL, draft_revision INTEGER NOT NULL, PRIMARY KEY(operation_id,draft_id,draft_revision));").map_err(db_error)?;
        Ok(Self {
            db,
            path: path.to_path_buf(),
        })
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
                selected_replacements: vec![],
                selected_replies: vec![],
                selected_thread_actions: vec![],
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
        let replacements =
            DraftStore::open(&request.state)?.list_replacements(&request.preparation_id)?;
        let discussion_drafts = &drafts;
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
        for selected in &request.selected_replacements {
            if !ids.insert(selected.id.as_str()) {
                return Err(invalid(
                    "Selected draft IDs must be unique across comments and suggestions",
                    "/selected_replacements",
                ));
            }
            let Some(draft) = replacements.drafts.iter().find(|d| d.id == selected.id) else {
                return Err(invalid(
                    "Selected replacement is no longer available",
                    "/selected_replacements",
                ));
            };
            if draft.revision != selected.revision {
                return Err(ContractError::new(
                    "DRAFT_REVISION_STALE",
                    "A selected replacement changed; reload before staging the review",
                    "/selected_replacements",
                ));
            }
        }
        for selected in &request.selected_replies {
            if !ids.insert(selected.id.as_str()) {
                return Err(invalid(
                    "Selected draft IDs must be unique across all operations",
                    "/selected_replies",
                ));
            }
            let Some(draft) = discussion_drafts
                .reply_drafts
                .iter()
                .find(|draft| draft.id == selected.id)
            else {
                return Err(invalid(
                    "Selected reply is no longer available",
                    "/selected_replies",
                ));
            };
            if draft.revision != selected.revision {
                return Err(ContractError::new(
                    "DRAFT_REVISION_STALE",
                    "A selected reply changed; reload before staging the review",
                    "/selected_replies",
                ));
            }
        }
        for selected in &request.selected_thread_actions {
            if !ids.insert(selected.id.as_str()) {
                return Err(invalid(
                    "Selected draft IDs must be unique across all operations",
                    "/selected_thread_actions",
                ));
            }
            let Some(draft) = discussion_drafts
                .thread_actions
                .iter()
                .find(|draft| draft.id == selected.id)
            else {
                return Err(invalid(
                    "Selected thread action is no longer available",
                    "/selected_thread_actions",
                ));
            };
            if draft.revision != selected.revision {
                return Err(ContractError::new(
                    "DRAFT_REVISION_STALE",
                    "A selected thread action changed; reload before staging the review",
                    "/selected_thread_actions",
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
            selected_replacements: request.selected_replacements,
            selected_replies: request.selected_replies,
            selected_thread_actions: request.selected_thread_actions,
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
        if intent.selected.is_empty()
            && intent.selected_replacements.is_empty()
            && intent.summary.as_deref().unwrap_or("").trim().is_empty()
        {
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
        let replacement_snapshot = DraftStore::open(state)?.list_replacements(preparation_id)?;
        if replacement_snapshot.workspace_revision != draft_snapshot.workspace_revision {
            return Err(ContractError::new(
                "WORKSPACE_REVISION_STALE",
                "Workspace content changed; reload before confirming the review",
                "",
            ));
        }
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
        let mut suggestions = Vec::with_capacity(intent.selected_replacements.len());
        for selected in &intent.selected_replacements {
            let draft = replacement_snapshot
                .drafts
                .iter()
                .find(|draft| draft.id == selected.id)
                .ok_or_else(|| {
                    invalid(
                        "Selected replacement no longer exists",
                        "/selected_replacements",
                    )
                })?;
            if draft.revision != selected.revision {
                return Err(ContractError::new(
                    "DRAFT_REVISION_STALE",
                    "A selected replacement changed; reopen Finish Review",
                    "/selected_replacements",
                ));
            }
            validate_replacement_target(
                Path::new(&binding.checkout),
                &binding.context,
                &draft.source,
                &draft.original_text,
                control,
                &index,
            )?;
            suggestions.push(ReviewPayloadSuggestion {
                draft_id: draft.id.clone(),
                draft_revision: draft.revision,
                source: draft.source.clone(),
                original_text: draft.original_text.clone(),
                replacement: draft.replacement.clone(),
                body: suggestion_markdown(&draft.replacement)?,
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
            suggestions,
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
            send_started_at: None,
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
        verify_replacement_revisions(&tx, &operation.payload)?;
        let started_at = now_ms();
        tx.execute("UPDATE review_operations SET state='sending',send_started_at=?1,updated_at=?1 WHERE operation_id=?2 AND state='planned'", params![started_at, operation_id]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        let mut updated = operation;
        updated.state = "sending".into();
        updated.send_started_at = Some(started_at);
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
        let record: Option<(String, Option<i64>)> = tx.query_row("SELECT payload_json,send_started_at FROM review_operations WHERE operation_id=?1 AND state='sending'",[operation_id],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(db_error)?;
        let Some((payload_json, _send_started_at)) = record else {
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
            record_published_payload(&tx, operation_id, &payload)?;
        }
        tx.commit().map_err(db_error)?;
        self.operation(operation_id)
    }

    pub fn operation(&self, operation_id: &str) -> Result<ReviewOperation, ContractError> {
        load_operation(&self.db, operation_id)
    }

    /// Reconciles an uncertain outcome using a complete, read-only GitHub snapshot.
    /// The immutable request payload remains unchanged on every path.
    pub fn reconcile_unknown(
        &mut self,
        operation_id: &str,
        reader: &dyn ReviewOutcomeReader,
        control: &OperationControl,
    ) -> Result<ReviewOperation, ContractError> {
        let operation = self.operation(operation_id)?;
        if operation.state != "unknown" {
            return Err(ContractError::new(
                "REVIEW_RECONCILIATION_NOT_ALLOWED",
                "Only an unknown review operation can be reconciled",
                "",
            ));
        }
        let started_at = operation.send_started_at.ok_or_else(|| {
            ContractError::new(
                "REVIEW_RECONCILIATION_INCOMPLETE",
                "The send attempt has no recorded start time",
                "",
            )
        })?;
        let binding =
            PreparationStore::open(&self.path)?.load(&operation.payload.preparation_id)?;
        let snapshot = reader.read(&binding.context.pr, control)?;
        let Some(data) = snapshot.pointer("/data") else {
            return Err(reconciliation_unavailable(
                "GitHub returned no complete data object",
            ));
        };
        if data.get("errors").is_some() {
            return Err(reconciliation_unavailable("GitHub returned GraphQL errors"));
        }
        let pr = data.pointer("/repository/pullRequest").ok_or_else(|| {
            reconciliation_unavailable("GitHub did not return the requested pull request")
        })?;
        let expected_repo = format!(
            "{}/{}",
            binding.context.pr.owner, binding.context.pr.repository
        );
        if data
            .pointer("/repository/nameWithOwner")
            .and_then(Value::as_str)
            != Some(expected_repo.as_str())
            || pr.get("number").and_then(Value::as_u64) != Some(binding.context.pr.number as u64)
        {
            return self.store_reconciliation_unknown(operation_id, "GitHub evidence does not match this repository and pull request; outcome remains unknown");
        }
        let Some(viewer) = data.pointer("/viewer/login").and_then(Value::as_str) else {
            return Err(reconciliation_unavailable(
                "GitHub did not identify the authenticated viewer",
            ));
        };
        let reviews = pr
            .pointer("/reviews/nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| reconciliation_unavailable("GitHub review listing is incomplete"))?;
        let threads = pr
            .pointer("/reviewThreads/nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| reconciliation_unavailable("GitHub thread listing is incomplete"))?;
        let complete = pr
            .pointer("/reviews/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
            == Some(false)
            && pr
                .pointer("/reviewThreads/pageInfo/hasNextPage")
                .and_then(Value::as_bool)
                == Some(false)
            && threads.iter().all(|t| {
                t.pointer("/comments/pageInfo/hasNextPage")
                    .and_then(Value::as_bool)
                    == Some(false)
            });
        if !complete {
            return self.store_reconciliation_unknown(
                operation_id,
                "GitHub returned an incomplete review listing; outcome remains unknown",
            );
        }
        let lower = started_at.saturating_sub(2_000);
        let upper = now_ms().saturating_add(30_000);
        let expected_event = match operation.payload.event {
            ReviewEvent::Comment => "COMMENTED",
            ReviewEvent::Approve => "APPROVED",
            ReviewEvent::RequestChanges => "CHANGES_REQUESTED",
        };
        let mut matches = Vec::new();
        for review in reviews {
            if review.pointer("/author/login").and_then(Value::as_str) != Some(viewer)
                || review.pointer("/state").and_then(Value::as_str) != Some(expected_event)
                || review
                    .pointer("/body")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    != operation.payload.summary.as_deref().unwrap_or("")
                || review.pointer("/commit/oid").and_then(Value::as_str)
                    != Some(operation.payload.head_sha.as_str())
                || review
                    .pointer("/submittedAt")
                    .and_then(Value::as_str)
                    .and_then(parse_rfc3339_ms)
                    .is_none_or(|t| t < lower || t > upper)
            {
                continue;
            }
            let Some(review_id) = review.get("id").and_then(Value::as_str) else {
                continue;
            };
            if remote_review_comments(threads, review_id)
                == Some(expected_review_comments(&operation.payload))
            {
                matches.push(review);
            }
        }
        match matches.as_slice() {
            [review] => {
                let id = review
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let url = review.get("url").and_then(Value::as_str).map(str::to_owned);
                let tx = self
                    .db
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(db_error)?;
                let changed = tx.execute("UPDATE review_operations SET state='confirmed',remote_review_id=?1,remote_url=?2,failure=NULL,updated_at=?3 WHERE operation_id=?4 AND state='unknown'", params![id,url,now_ms(),operation_id]).map_err(db_error)?;
                if changed != 1 {
                    return Err(ContractError::new(
                        "REVIEW_RECONCILIATION_NOT_ALLOWED",
                        "Review operation changed while reconciliation was running",
                        "",
                    ));
                }
                record_published_payload(&tx, operation_id, &operation.payload)?;
                tx.commit().map_err(db_error)?;
                self.operation(operation_id)
            }
            [] => self.store_reconciliation_unknown(
                operation_id,
                "No unique matching GitHub review was found; outcome remains unknown",
            ),
            _ => self.store_reconciliation_unknown(
                operation_id,
                "Ambiguous: multiple matching GitHub reviews were found; outcome remains unknown",
            ),
        }
    }

    fn store_reconciliation_unknown(
        &self,
        operation_id: &str,
        message: &str,
    ) -> Result<ReviewOperation, ContractError> {
        self.db.execute("UPDATE review_operations SET failure=?1,updated_at=?2 WHERE operation_id=?3 AND state='unknown'", params![bound(message),now_ms(),operation_id]).map_err(db_error)?;
        self.operation(operation_id)
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

    /// Returns the complete local publication history for one preparation,
    /// oldest first. This is a read-only view for recovery surfaces; it never
    /// changes operation state or makes a failed/unknown operation retryable.
    pub fn operation_history(
        &self,
        preparation_id: &str,
    ) -> Result<Vec<ReviewOperation>, ContractError> {
        let mut statement = self
            .db
            .prepare(
                "SELECT operation_id FROM review_operations
                 WHERE preparation_id=?1 ORDER BY created_at ASC,operation_id ASC",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([preparation_id], |row| row.get::<_, String>(0))
            .map_err(db_error)?;
        let mut operations = Vec::new();
        for row in rows {
            operations.push(self.operation(&row.map_err(db_error)?)?);
        }
        Ok(operations)
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
        let plan = self.operation(operation_id)?;
        let binding = PreparationStore::open(&self.path)?.load(&plan.payload.preparation_id)?;
        let index = diff_index(Path::new(&binding.checkout), &binding.context, control)?;
        for suggestion in &plan.payload.suggestions {
            if suggestion.body != suggestion_markdown(&suggestion.replacement)? {
                let failure = ContractError::new(
                    "REVIEW_PLAN_INVALID",
                    "The confirmed suggestion body no longer matches its structured replacement",
                    "/suggestions",
                );
                return self.fail_unstarted(operation_id, &failure.message);
            }
            if let Err(error) = validate_replacement_target(
                Path::new(&binding.checkout),
                &binding.context,
                &suggestion.source,
                &suggestion.original_text,
                control,
                &index,
            ) {
                return self.fail_unstarted(operation_id, &error.message);
            }
        }
        // `begin_send` checks replacement IDs/revisions again inside the same
        // immediate transaction that records the sending state. This closes the
        // local edit race between the pinned Git read above and network I/O.
        let sending = match self.begin_send(operation_id, remote) {
            Ok(sending) => sending,
            Err(error) if error.code == "DRAFT_REVISION_STALE" => {
                return self.fail_unstarted(operation_id, &error.message);
            }
            Err(error) => return Err(error),
        };
        let outcome = publisher.publish(&sending.payload, control);
        self.finish_send(operation_id, outcome)
    }

    fn fail_unstarted(
        &mut self,
        operation_id: &str,
        message: &str,
    ) -> Result<ReviewOperation, ContractError> {
        let failure = format!("No GitHub request was sent. {message}");
        self.db.execute(
            "UPDATE review_operations SET state='failed',failure=?1,updated_at=?2 WHERE operation_id=?3 AND state='planned'",
            params![bound(&failure), now_ms(), operation_id],
        ).map_err(db_error)?;
        self.operation(operation_id)
    }
}

#[derive(Debug, Clone)]
pub struct StageReviewRequest {
    pub state: PathBuf,
    pub preparation_id: String,
    pub expected_workspace_revision: u64,
    pub expected_intent_revision: u64,
    pub selected: Vec<SelectedDraft>,
    pub selected_replacements: Vec<SelectedDraft>,
    pub selected_replies: Vec<SelectedDraft>,
    pub selected_thread_actions: Vec<SelectedDraft>,
    pub summary: Option<String>,
    pub event: Option<ReviewEvent>,
}

fn verify_replacement_revisions(
    tx: &rusqlite::Transaction<'_>,
    payload: &ReviewPayload,
) -> Result<(), ContractError> {
    for suggestion in &payload.suggestions {
        let row: Option<(String, String, String, i64)> = tx
            .query_row(
                "SELECT source_json,original_text,replacement,revision
                 FROM code_replacement_drafts WHERE draft_id=?1 AND context_id=?2",
                params![suggestion.draft_id, payload.context_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(db_error)?;
        let Some((source, original, replacement, revision)) = row else {
            return Err(ContractError::new(
                "DRAFT_REVISION_STALE",
                "A selected replacement was removed after confirmation; no GitHub request was sent",
                "/suggestions",
            ));
        };
        if source != serde_json::to_string(&suggestion.source).map_err(json_error)?
            || original != suggestion.original_text
            || replacement != suggestion.replacement
            || u64::try_from(revision).map_err(|_| state_error())? != suggestion.draft_revision
        {
            return Err(ContractError::new(
                "DRAFT_REVISION_STALE",
                "A selected replacement changed after confirmation; no GitHub request was sent",
                "/suggestions",
            ));
        }
    }
    Ok(())
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

/// Read-only boundary used to inspect an uncertain grouped review outcome.
pub trait ReviewOutcomeReader {
    fn read(
        &self,
        pr: &crate::PrIdentity,
        control: &OperationControl,
    ) -> Result<Value, ContractError>;
}

/// Reads reviews, threads, and the authenticated viewer through a read-only GraphQL query.
pub struct GhReviewOutcomeReader {
    pub executable: PathBuf,
    pub checkout: PathBuf,
    pub timeout: Duration,
}

impl ReviewOutcomeReader for GhReviewOutcomeReader {
    fn read(
        &self,
        pr: &crate::PrIdentity,
        control: &OperationControl,
    ) -> Result<Value, ContractError> {
        let query = "query($owner:String!,$name:String!,$number:Int!){viewer{login} repository(owner:$owner,name:$name){nameWithOwner pullRequest(number:$number){number reviews(first:100){pageInfo{hasNextPage} nodes{id url body state submittedAt author{login} commit{oid}}} reviewThreads(first:100){pageInfo{hasNextPage} nodes{path line startLine diffSide startDiffSide comments(first:100){pageInfo{hasNextPage} nodes{id body pullRequestReview{id}}}}}}}}}";
        let input = json!({"query":query,"variables":{"owner":pr.owner,"name":pr.repository,"number":pr.number}});
        let output = run_gh_json(
            &self.executable,
            &self.checkout,
            &input,
            control,
            self.timeout,
        )
        .map_err(|e| {
            reconciliation_unavailable(&match e {
                ProcessFailure::NotStarted(m) | ProcessFailure::Uncertain(m) => m,
            })
        })?;
        if !output.status.success() {
            return Err(reconciliation_unavailable(&String::from_utf8_lossy(
                &output.stderr,
            )));
        }
        let value: Value = serde_json::from_slice(&output.stdout)
            .map_err(|_| reconciliation_unavailable("GitHub returned invalid JSON"))?;
        if value.get("errors").is_some() {
            return Err(reconciliation_unavailable("GitHub returned GraphQL errors"));
        }
        Ok(value)
    }
}

fn reconciliation_unavailable(message: &str) -> ContractError {
    ContractError::new("REVIEW_RECONCILIATION_UNAVAILABLE", &bound(message), "")
}

fn expected_review_comments(payload: &ReviewPayload) -> Vec<(String, i64, i64, String, String)> {
    let mut expected: Vec<_> = payload
        .comments
        .iter()
        .map(|c| {
            (
                c.source.path.clone(),
                c.source.start_line as i64,
                c.source.end_line as i64,
                c.body.clone(),
                if c.source.revision == Revision::Head {
                    "RIGHT".into()
                } else {
                    "LEFT".into()
                },
            )
        })
        .collect();
    expected.extend(payload.suggestions.iter().map(|c| {
        (
            c.source.path.clone(),
            c.source.start_line as i64,
            c.source.end_line as i64,
            c.body.clone(),
            if c.source.revision == Revision::Head {
                "RIGHT".into()
            } else {
                "LEFT".into()
            },
        )
    }));
    expected.sort();
    expected
}

/// Missing fields or unsupported diff-side metadata fail closed.
fn remote_review_comments(
    threads: &[Value],
    review_id: &str,
) -> Option<Vec<(String, i64, i64, String, String)>> {
    let mut comments = Vec::new();
    for thread in threads {
        let path = thread.get("path")?.as_str()?.to_owned();
        let line = thread.get("line")?.as_i64()?;
        let start = thread
            .get("startLine")
            .filter(|v| !v.is_null())
            .map(Value::as_i64)
            .flatten()
            .unwrap_or(line);
        let side = thread.get("diffSide")?.as_str()?.to_owned();
        let values = thread.pointer("/comments/nodes")?.as_array()?;
        for comment in values {
            if comment
                .pointer("/pullRequestReview/id")
                .and_then(Value::as_str)
                == Some(review_id)
            {
                comments.push((
                    path.clone(),
                    start,
                    line,
                    comment.get("body")?.as_str()?.to_owned(),
                    side.clone(),
                ));
            }
        }
    }
    comments.sort();
    Some(comments)
}

/// Parse RFC3339 timestamps to Unix milliseconds without accepting malformed dates.
fn parse_rfc3339_ms(value: &str) -> Option<i64> {
    let (date, clock) = value.split_once('T')?;
    let mut d = date.split('-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    if d.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let tz_pos = clock.find(['Z', '+', '-'])?;
    let time = &clock[..tz_pos];
    let zone = &clock[tz_pos..];
    let mut t = time.split(':');
    let hour: i64 = t.next()?.parse().ok()?;
    let minute: i64 = t.next()?.parse().ok()?;
    let sec_fraction = t.next()?;
    if t.next().is_some() || hour > 23 || minute > 59 {
        return None;
    }
    let (sec, frac) = sec_fraction.split_once('.').unwrap_or((sec_fraction, ""));
    let second: i64 = sec.parse().ok()?;
    if second > 59 || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let millis: i64 = if frac.is_empty() {
        0
    } else {
        format!("{:0<3}", &frac[..frac.len().min(3)]).parse().ok()?
    };
    let offset = if zone == "Z" {
        0
    } else {
        if zone.len() != 6 || &zone[3..4] != ":" {
            return None;
        }
        let sign = if zone.starts_with('+') {
            1
        } else if zone.starts_with('-') {
            -1
        } else {
            return None;
        };
        let oh: i64 = zone[1..3].parse().ok()?;
        let om: i64 = zone[4..6].parse().ok()?;
        if oh > 23 || om > 59 {
            return None;
        }
        sign * (oh * 3600 + om * 60)
    };
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some((days * 86400 + hour * 3600 + minute * 60 + second - offset) * 1000 + millis)
}

pub struct GhReviewPublisher {
    pub executable: PathBuf,
    pub checkout: PathBuf,
    pub timeout: Duration,
}
impl ReviewPublisher for GhReviewPublisher {
    fn publish(&self, payload: &ReviewPayload, control: &OperationControl) -> SendOutcome {
        let mut comments: Vec<Value> = payload
            .comments
            .iter()
            .map(|comment| review_thread(comment.body.as_str(), &comment.source))
            .collect();
        comments.extend(
            payload
                .suggestions
                .iter()
                .map(|suggestion| review_thread(&suggestion.body, &suggestion.source)),
        );
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

fn review_thread(body: &str, source: &SourceRef) -> Value {
    let side = if source.revision == Revision::Head {
        "RIGHT"
    } else {
        "LEFT"
    };
    let mut thread = json!({
        "path": source.path,
        "body": body,
        "side": side,
        "line": source.end_line,
    });
    if source.start_line != source.end_line {
        thread["startLine"] = json!(source.start_line);
        thread["startSide"] = Value::String(side.into());
    }
    thread
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
    Option<i64>,
);

fn load_operation(db: &Connection, id: &str) -> Result<ReviewOperation, ContractError> {
    let row: Option<OperationRow> = db.query_row("SELECT fingerprint,state,payload_json,remote_review_id,remote_url,failure,send_started_at FROM review_operations WHERE operation_id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(db_error)?;
    let Some((fingerprint, state, payload, remote_id, url, failure, send_started_at)) = row else {
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
        send_started_at,
    })
}
fn record_published_payload(
    tx: &rusqlite::Transaction<'_>,
    operation_id: &str,
    payload: &ReviewPayload,
) -> Result<(), ContractError> {
    for comment in &payload.comments {
        tx.execute("INSERT OR IGNORE INTO review_published_drafts(operation_id,preparation_id,context_id,draft_id,draft_revision) VALUES(?1,?2,?3,?4,?5)", params![operation_id,payload.preparation_id,payload.context_id,comment.draft_id,i64::try_from(comment.draft_revision).map_err(|_|state_error())?]).map_err(db_error)?;
    }
    for suggestion in &payload.suggestions {
        tx.execute("INSERT OR IGNORE INTO review_published_replacements(operation_id,preparation_id,context_id,draft_id,draft_revision) VALUES(?1,?2,?3,?4,?5)", params![operation_id,payload.preparation_id,payload.context_id,suggestion.draft_id,i64::try_from(suggestion.draft_revision).map_err(|_|state_error())?]).map_err(db_error)?;
    }
    Ok(())
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

fn validate_replacement_target(
    checkout: &Path,
    context: &crate::ReviewContext,
    source: &SourceRef,
    expected: &str,
    control: &OperationControl,
    index: &crate::DiffIndex,
) -> Result<(), ContractError> {
    if source.revision != Revision::Head {
        return Err(ContractError::new(
            "REPLACEMENT_HEAD_ONLY",
            "Code suggestions must target the reviewed head side",
            "/source/revision",
        ));
    }
    index.validate_anchor(source)?;
    let exact = read_source(checkout, context, source, control)?;
    if exact != expected {
        return Err(ContractError::new(
            "REPLACEMENT_SOURCE_CHANGED",
            "The exact pinned head text changed; revalidate this replacement before sending",
            "/original_text",
        ));
    }
    Ok(())
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
