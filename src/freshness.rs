//! Read-only observations of newer pull-request metadata.
use crate::{
    ContractError, DraftStore, OperationControl, PreparationStore, PreparedBinding,
    ReviewPublicationStore,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessStatus {
    Current,
    Changed,
    Closed,
    Unknown,
}

/// A stored observation is separate from the immutable prepared binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreshnessObservation {
    pub preparation_id: String,
    pub status: FreshnessStatus,
    pub pinned_base_sha: String,
    pub pinned_head_sha: String,
    pub observed_base_ref: Option<String>,
    pub observed_head_ref: Option<String>,
    pub observed_base_sha: Option<String>,
    pub observed_head_sha: Option<String>,
    pub head_moved: bool,
    pub base_moved: bool,
    pub target_changed: Option<bool>,
    pub head_branch_changed: Option<bool>,
    pub pull_request_state: Option<String>,
    pub checked_at_ms: i64,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedWorkSnapshot {
    pub workspace_revision: u64,
    pub unpublished_draft_count: u64,
    pub unresolved_publication_count: u64,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LatestSwitchPreview {
    pub current_preparation_id: String,
    pub candidate: PreparedBinding,
    pub protected_work: ProtectedWorkSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LatestSwitchApproval {
    pub approval_id: String,
    pub current_preparation_id: String,
    pub candidate_preparation_id: String,
    pub expires_at_ms: i64,
}

const SWITCH_APPROVAL_TTL_MS: i64 = 5 * 60 * 1000;
type ApprovalRow = (
    String,
    String,
    String,
    String,
    i64,
    String,
    i64,
    Option<i64>,
);

#[derive(Debug, Deserialize)]
struct RemoteSnapshot {
    state: String,
    base: RemoteBranch,
    head: RemoteBranch,
}

#[derive(Debug, Deserialize)]
struct RemoteBranch {
    #[serde(rename = "ref")]
    reference: String,
    sha: String,
}

/// Saves the metadata observed while this immutable preparation was acquired.
/// Reopening or rechecking never replaces this baseline.
pub fn record_initial_remote_snapshot(
    state: &Path,
    preparation_id: &str,
    base_ref: &str,
    head_ref: &str,
    base_sha: &str,
    head_sha: &str,
    pull_request_state: &str,
) -> Result<(), ContractError> {
    let db = open_db(state)?;
    db.execute(
        "INSERT OR IGNORE INTO freshness_baselines(
             preparation_id, base_ref, head_ref, base_sha, head_sha, pull_request_state, captured_at_ms
         ) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            preparation_id,
            base_ref,
            head_ref,
            base_sha,
            head_sha,
            pull_request_state,
            now_ms()
        ],
    )
    .map_err(db_error)?;
    Ok(())
}

/// Checks current PR metadata through `gh` and persists the observation without
/// changing the pinned source, description, or prepared context.
pub fn refresh_freshness_with_gh(
    prepared: &PreparedBinding,
    state: &Path,
    control: &OperationControl,
    gh: &Path,
) -> Result<FreshnessObservation, ContractError> {
    let baseline = load_baseline(state, &prepared.preparation_id)?;
    let checked_at_ms = now_ms();
    let remote = match fetch_remote(prepared, control, gh) {
        Ok(remote) => remote,
        Err(error) => {
            let observation = FreshnessObservation {
                preparation_id: prepared.preparation_id.clone(),
                status: FreshnessStatus::Unknown,
                pinned_base_sha: prepared.context.base_sha.clone(),
                pinned_head_sha: prepared.context.head_sha.clone(),
                observed_base_ref: None,
                observed_head_ref: None,
                observed_base_sha: None,
                observed_head_sha: None,
                head_moved: false,
                base_moved: false,
                target_changed: None,
                head_branch_changed: None,
                pull_request_state: None,
                checked_at_ms,
                error_code: Some(error.code),
            };
            store_observation(state, &observation)?;
            return Ok(observation);
        }
    };

    let Some((baseline_base_ref, baseline_head_ref)) = baseline else {
        let observation = FreshnessObservation {
            preparation_id: prepared.preparation_id.clone(),
            status: FreshnessStatus::Unknown,
            pinned_base_sha: prepared.context.base_sha.clone(),
            pinned_head_sha: prepared.context.head_sha.clone(),
            observed_base_ref: Some(remote.base.reference),
            observed_head_ref: Some(remote.head.reference),
            observed_base_sha: Some(remote.base.sha),
            observed_head_sha: Some(remote.head.sha),
            head_moved: false,
            base_moved: false,
            target_changed: None,
            head_branch_changed: None,
            pull_request_state: Some(remote.state),
            checked_at_ms,
            error_code: Some("FRESHNESS_BASELINE_UNAVAILABLE".into()),
        };
        store_observation(state, &observation)?;
        return Ok(observation);
    };

    let head_moved = remote.head.sha != prepared.context.head_sha;
    let base_moved = remote.base.sha != prepared.context.base_sha;
    let target_changed = remote.base.reference != baseline_base_ref;
    let head_branch_changed = remote.head.reference != baseline_head_ref;
    let is_closed = !remote.state.eq_ignore_ascii_case("open");
    let changed = head_moved || base_moved || target_changed || head_branch_changed;
    let observation = FreshnessObservation {
        preparation_id: prepared.preparation_id.clone(),
        status: if is_closed {
            FreshnessStatus::Closed
        } else if changed {
            FreshnessStatus::Changed
        } else {
            FreshnessStatus::Current
        },
        pinned_base_sha: prepared.context.base_sha.clone(),
        pinned_head_sha: prepared.context.head_sha.clone(),
        observed_base_ref: Some(remote.base.reference),
        observed_head_ref: Some(remote.head.reference),
        observed_base_sha: Some(remote.base.sha),
        observed_head_sha: Some(remote.head.sha),
        head_moved,
        base_moved,
        target_changed: Some(target_changed),
        head_branch_changed: Some(head_branch_changed),
        pull_request_state: Some(remote.state),
        checked_at_ms,
        error_code: None,
    };
    store_observation(state, &observation)?;
    Ok(observation)
}

pub fn latest_freshness_observation(
    state: &Path,
    preparation_id: &str,
) -> Result<Option<FreshnessObservation>, ContractError> {
    let db = open_db(state)?;
    let stored: Option<String> = db
        .query_row(
            "SELECT observation_json FROM freshness_observations WHERE preparation_id=?1",
            [preparation_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    stored
        .map(|value| serde_json::from_str(&value).map_err(json_error))
        .transpose()
}

/// Acquires a complete candidate binding while keeping the currently selected
/// preparation active. The returned preview fingerprints all unpublished work.
pub fn prepare_latest_candidate_with_gh(
    current_preparation_id: &str,
    state: &Path,
    control: &OperationControl,
    gh: &Path,
) -> Result<LatestSwitchPreview, ContractError> {
    let current = PreparationStore::open(state)?.load(current_preparation_id)?;
    let manifest = crate::prepare_local_pr_with_gh(
        Path::new(&current.checkout),
        current.context.pr.number,
        None,
        state,
        control,
        gh,
    )?;
    let candidate = PreparationStore::open(state)?.load(&manifest.preparation_id)?;
    if candidate.context.pr != current.context.pr {
        return Err(ContractError::new(
            "PR_IDENTITY_CHANGED",
            "The latest preparation does not match the selected pull request",
            "",
        ));
    }
    let protected_work = protected_work_snapshot(state, &current)?;
    Ok(LatestSwitchPreview {
        current_preparation_id: current.preparation_id,
        candidate,
        protected_work,
    })
}

/// Rechecks GitHub immediately before an explicit switch so the user cannot
/// accidentally load a candidate that became stale while the preview was open.
pub fn verify_latest_candidate_with_gh(
    candidate: &PreparedBinding,
    state: &Path,
    control: &OperationControl,
    gh: &Path,
) -> Result<(), ContractError> {
    let remote = fetch_remote(candidate, control, gh)?;
    let Some((base_ref, head_ref)) = load_baseline(state, &candidate.preparation_id)? else {
        return Err(ContractError::new(
            "FRESHNESS_BASELINE_UNAVAILABLE",
            "The latest preparation has no saved GitHub metadata baseline",
            "",
        ));
    };
    if !remote.state.eq_ignore_ascii_case("open")
        || remote.base.sha != candidate.context.base_sha
        || remote.head.sha != candidate.context.head_sha
        || remote.base.reference != base_ref
        || remote.head.reference != head_ref
    {
        return Err(ContractError::new(
            "LATEST_CANDIDATE_STALE",
            "The pull request changed after the latest preparation was acquired; check again before loading it",
            "",
        ));
    }
    Ok(())
}

/// Issues a short lived, one use approval after verifying that the work shown
/// in the preview still has the exact same revisions and review intent.
pub fn authorize_latest_switch(
    state: &Path,
    current_preparation_id: &str,
    candidate_preparation_id: &str,
    expected_fingerprint: &str,
) -> Result<LatestSwitchApproval, ContractError> {
    ensure_switch_tables(state)?;
    let current = PreparationStore::open(state)?.load(current_preparation_id)?;
    let candidate = PreparationStore::open(state)?.load(candidate_preparation_id)?;
    validate_switch_pair(&current, &candidate)?;
    let mut db = open_db(state)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    let protected = protected_work_in(&tx, &current)?;
    if protected.fingerprint != expected_fingerprint {
        return Err(stale_protected_work());
    }
    let expires_at_ms = now_ms().saturating_add(SWITCH_APPROVAL_TTL_MS);
    let approval_id = new_approval_id(&current, &candidate, expected_fingerprint);
    tx.execute(
        "INSERT INTO latest_switch_approvals(
             approval_id,current_preparation_id,candidate_preparation_id,current_context_id,
             candidate_context_id,workspace_revision,work_fingerprint,expires_at_ms,consumed_at_ms
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,NULL)",
        params![
            approval_id,
            current.preparation_id,
            candidate.preparation_id,
            current.context.context_id,
            candidate.context.context_id,
            i64::try_from(protected.workspace_revision).map_err(|_| state_error())?,
            protected.fingerprint,
            expires_at_ms
        ],
    )
    .map_err(db_error)?;
    tx.commit().map_err(db_error)?;
    Ok(LatestSwitchApproval {
        approval_id,
        current_preparation_id: current_preparation_id.into(),
        candidate_preparation_id: candidate_preparation_id.into(),
        expires_at_ms,
    })
}

/// Consumes a human issued approval and returns the fully acquired candidate.
/// Drafts and publication operations stay attached to the old preparation.
pub fn apply_latest_switch_approval(
    state: &Path,
    approval_id: &str,
) -> Result<PreparedBinding, ContractError> {
    ensure_switch_tables(state)?;
    // Load the immutable preparation bindings before taking the write lock.
    // PreparationStore::open may configure WAL on a separate SQLite handle;
    // doing that while this connection holds BEGIN IMMEDIATE can block until
    // the busy timeout and hide the more useful stale-approval result.
    let db = open_db(state)?;
    let row: Option<ApprovalRow> = db
        .query_row(
            "SELECT current_preparation_id,candidate_preparation_id,current_context_id,
                    candidate_context_id,workspace_revision,work_fingerprint,expires_at_ms,consumed_at_ms
             FROM latest_switch_approvals WHERE approval_id=?1",
            [approval_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?;
    drop(db);
    let Some(initial_row) = row else {
        return Err(approval_invalid());
    };
    let (
        current_id,
        candidate_id,
        current_context,
        candidate_context,
        revision,
        fingerprint,
        expires_at,
        consumed_at,
    ) = initial_row.clone();
    if consumed_at.is_some() || expires_at <= now_ms() {
        return Err(approval_invalid());
    }
    let current = PreparationStore::open(state)?.load(&current_id)?;
    let candidate = PreparationStore::open(state)?.load(&candidate_id)?;

    let mut db = open_db(state)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    let row: Option<ApprovalRow> = tx
        .query_row(
            "SELECT current_preparation_id,candidate_preparation_id,current_context_id,
                    candidate_context_id,workspace_revision,work_fingerprint,expires_at_ms,consumed_at_ms
             FROM latest_switch_approvals WHERE approval_id=?1",
            [approval_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?;
    if row.as_ref() != Some(&initial_row) {
        return Err(approval_invalid());
    }

    if current.context.context_id != current_context
        || candidate.context.context_id != candidate_context
        || i64::try_from(current.expected_generation).unwrap_or(-1) < 0
    {
        return Err(approval_invalid());
    }
    validate_switch_pair(&current, &candidate)?;
    let protected = protected_work_in(&tx, &current)?;
    if protected.workspace_revision != u64::try_from(revision).map_err(|_| state_error())?
        || protected.fingerprint != fingerprint
    {
        return Err(stale_protected_work());
    }
    let changed = tx
        .execute(
            "UPDATE latest_switch_approvals SET consumed_at_ms=?1
             WHERE approval_id=?2 AND consumed_at_ms IS NULL AND expires_at_ms>?1",
            params![now_ms(), approval_id],
        )
        .map_err(db_error)?;
    if changed != 1 {
        return Err(approval_invalid());
    }
    tx.commit().map_err(db_error)?;
    PreparationStore::open(state)?.load(&candidate_id)
}

pub fn protected_work_snapshot(
    state: &Path,
    prepared: &PreparedBinding,
) -> Result<ProtectedWorkSnapshot, ContractError> {
    ensure_switch_tables(state)?;
    let mut db = open_db(state)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .map_err(db_error)?;
    protected_work_in(&tx, prepared)
}

fn validate_switch_pair(
    current: &PreparedBinding,
    candidate: &PreparedBinding,
) -> Result<(), ContractError> {
    if current.context.pr != candidate.context.pr {
        return Err(ContractError::new(
            "PR_IDENTITY_CHANGED",
            "Load latest candidate belongs to a different pull request",
            "",
        ));
    }
    if current.preparation_id == candidate.preparation_id {
        return Err(ContractError::new(
            "LATEST_CANDIDATE_INVALID",
            "Load latest requires a separate fully acquired preparation",
            "",
        ));
    }
    Ok(())
}

fn ensure_switch_tables(state: &Path) -> Result<(), ContractError> {
    drop(DraftStore::open(state)?);
    drop(ReviewPublicationStore::open(state)?);
    let db = open_db(state)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS latest_switch_approvals(
             approval_id TEXT PRIMARY KEY NOT NULL,
             current_preparation_id TEXT NOT NULL REFERENCES preparations(preparation_id),
             candidate_preparation_id TEXT NOT NULL REFERENCES preparations(preparation_id),
             current_context_id TEXT NOT NULL,
             candidate_context_id TEXT NOT NULL,
             workspace_revision INTEGER NOT NULL,
             work_fingerprint TEXT NOT NULL,
             expires_at_ms INTEGER NOT NULL,
             consumed_at_ms INTEGER
         );",
    )
    .map_err(db_error)
}

fn protected_work_in(
    db: &Connection,
    prepared: &PreparedBinding,
) -> Result<ProtectedWorkSnapshot, ContractError> {
    let pr = &prepared.context.pr;
    let workspace_revision: i64 = db
        .query_row(
            "SELECT generation FROM workspaces WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4",
            params![pr.host,pr.owner,pr.repository,i64::try_from(pr.number).map_err(|_|state_error())?],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    let mut digest = Sha256::new();
    let mut pending_draft_count = 0_u64;
    let mut add = |tag: &str, value: &str| {
        digest.update((tag.len() as u64).to_be_bytes());
        digest.update(tag.as_bytes());
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    };
    add("preparation", &prepared.preparation_id);
    add("context", &prepared.context.context_id);
    add("workspace_revision", &workspace_revision.to_string());

    let mut collect =
        |kind: &str, sql: &str, args: &[&dyn rusqlite::ToSql]| -> Result<(), ContractError> {
            let mut statement = db.prepare(sql).map_err(db_error)?;
            let rows = statement
                .query_map(args, |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(db_error)?;
            for row in rows {
                let (id, revision) = row.map_err(db_error)?;
                add(kind, &id);
                add(&format!("{kind}_revision"), &revision.to_string());
                pending_draft_count = pending_draft_count.saturating_add(1);
            }
            Ok(())
        };
    let number = i64::try_from(pr.number).map_err(|_| state_error())?;
    let context = prepared.context.context_id.as_str();
    collect(
        "inline",
        "SELECT d.draft_id,d.revision FROM manual_text_drafts d
         WHERE d.host=?1 AND d.owner=?2 AND d.repository=?3 AND d.number=?4 AND d.context_id=?5
           AND NOT EXISTS(SELECT 1 FROM review_published_drafts p WHERE p.context_id=?6 AND p.draft_id=d.draft_id AND p.draft_revision=d.revision)
         ORDER BY d.draft_id",
        &[&pr.host,&pr.owner,&pr.repository,&number,&context,&context],
    )?;
    for (kind, table) in [
        ("reply", "reply_drafts"),
        ("thread_action", "thread_action_drafts"),
    ] {
        collect(
            kind,
            &format!(
                "SELECT draft_id,revision FROM {table} WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4 AND context_id=?5 ORDER BY draft_id"
            ),
            &[&pr.host, &pr.owner, &pr.repository, &number, &context],
        )?;
    }
    let replacement_markers_exist: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='review_published_replacements')",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    let replacement_sql = if replacement_markers_exist {
        "SELECT d.draft_id,d.revision FROM code_replacement_drafts d
         WHERE d.host=?1 AND d.owner=?2 AND d.repository=?3 AND d.number=?4 AND d.context_id=?5
           AND NOT EXISTS(SELECT 1 FROM review_published_replacements p WHERE p.draft_id=d.draft_id AND p.draft_revision=d.revision)
         ORDER BY d.draft_id"
    } else {
        "SELECT draft_id,revision FROM code_replacement_drafts
         WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4 AND context_id=?5
         ORDER BY draft_id"
    };
    collect(
        "replacement",
        replacement_sql,
        &[&pr.host, &pr.owner, &pr.repository, &number, &context],
    )?;

    let intent: Option<(i64, String)> = db
        .query_row(
            "SELECT revision,intent_json FROM review_intents WHERE preparation_id=?1",
            [&prepared.preparation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(db_error)?;
    let mut has_unpublished_summary = false;
    if let Some((revision, intent_json)) = intent {
        add("review_intent_revision", &revision.to_string());
        add("review_intent", &intent_json);
        let intent_value: serde_json::Value =
            serde_json::from_str(&intent_json).map_err(json_error)?;
        let summary = intent_value
            .get("summary")
            .and_then(serde_json::Value::as_str);
        if let Some(summary) = summary.filter(|summary| !summary.trim().is_empty()) {
            let mut statement = db
                .prepare("SELECT payload_json FROM review_operations WHERE preparation_id=?1 AND state='confirmed'")
                .map_err(db_error)?;
            let rows = statement
                .query_map([&prepared.preparation_id], |row| row.get::<_, String>(0))
                .map_err(db_error)?;
            let mut published = false;
            for row in rows {
                let payload: serde_json::Value =
                    serde_json::from_str(&row.map_err(db_error)?).map_err(json_error)?;
                if payload
                    .get("intent_revision")
                    .and_then(serde_json::Value::as_i64)
                    == Some(revision)
                    && payload.get("summary").and_then(serde_json::Value::as_str) == Some(summary)
                {
                    published = true;
                    break;
                }
            }
            has_unpublished_summary = !published;
            if has_unpublished_summary {
                pending_draft_count = pending_draft_count.saturating_add(1);
            }
        }
    } else {
        add("review_intent", "none");
    }
    add(
        "unpublished_summary",
        if has_unpublished_summary { "1" } else { "0" },
    );

    let mut unresolved_count = 0_u64;
    let mut statement = db
        .prepare("SELECT operation_id,state,fingerprint FROM review_operations WHERE preparation_id=?1 AND state IN ('planned','sending','unknown') ORDER BY operation_id")
        .map_err(db_error)?;
    let rows = statement
        .query_map([&prepared.preparation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(db_error)?;
    for row in rows {
        let (id, status, fingerprint) = row.map_err(db_error)?;
        add("publication_operation", &id);
        add("publication_status", &status);
        add("publication_fingerprint", &fingerprint);
        unresolved_count = unresolved_count.saturating_add(1);
    }
    let fingerprint = format!("v1-{:x}", digest.finalize());
    Ok(ProtectedWorkSnapshot {
        workspace_revision: u64::try_from(workspace_revision).map_err(|_| state_error())?,
        unpublished_draft_count: pending_draft_count,
        unresolved_publication_count: unresolved_count,
        fingerprint,
    })
}

fn new_approval_id(
    current: &PreparedBinding,
    candidate: &PreparedBinding,
    fingerprint: &str,
) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let now = now_ms();
    let mut digest = Sha256::new();
    digest.update(b"bodh-latest-switch-approval-v1\0");
    digest.update(std::process::id().to_be_bytes());
    digest.update(now.to_be_bytes());
    digest.update(NEXT_ID.fetch_add(1, Ordering::Relaxed).to_be_bytes());
    digest.update(current.preparation_id.as_bytes());
    digest.update(candidate.preparation_id.as_bytes());
    digest.update(fingerprint.as_bytes());
    format!("approval-{:x}", digest.finalize())
}

fn stale_protected_work() -> ContractError {
    ContractError::new(
        "DRAFT_REVISION_STALE",
        "Unpublished review work changed after the load-latest preview; refresh before approving",
        "",
    )
}

fn approval_invalid() -> ContractError {
    ContractError::new(
        "LOAD_LATEST_APPROVAL_INVALID",
        "Load-latest approval is missing, expired, or already used",
        "",
    )
}

fn fetch_remote(
    prepared: &PreparedBinding,
    control: &OperationControl,
    gh: &Path,
) -> Result<RemoteSnapshot, ContractError> {
    let host = &prepared.context.pr.host;
    let endpoint = format!(
        "repos/{}/{}/pulls/{}",
        prepared.context.pr.owner, prepared.context.pr.repository, prepared.context.pr.number
    );
    let output = crate::preparation::run_gh(gh, &["api", "--hostname", host, &endpoint], control)?;
    serde_json::from_slice::<RemoteSnapshot>(&output).map_err(|_| {
        ContractError::new(
            "GH_RESPONSE_INVALID",
            "GitHub returned invalid pull request metadata during freshness check",
            "",
        )
    })
}

fn load_baseline(
    state: &Path,
    preparation_id: &str,
) -> Result<Option<(String, String)>, ContractError> {
    let db = open_db(state)?;
    db.query_row(
        "SELECT base_ref, head_ref FROM freshness_baselines WHERE preparation_id=?1",
        [preparation_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(db_error)
}

fn store_observation(
    state: &Path,
    observation: &FreshnessObservation,
) -> Result<(), ContractError> {
    let db = open_db(state)?;
    let value = serde_json::to_string(observation).map_err(json_error)?;
    db.execute(
        "INSERT INTO freshness_observations(preparation_id, checked_at_ms, observation_json)
         VALUES(?1,?2,?3)
         ON CONFLICT(preparation_id) DO UPDATE SET
             checked_at_ms=excluded.checked_at_ms,
             observation_json=excluded.observation_json",
        params![observation.preparation_id, observation.checked_at_ms, value],
    )
    .map_err(db_error)?;
    Ok(())
}

fn open_db(path: &Path) -> Result<Connection, ContractError> {
    drop(crate::PreparationStore::open(path)?);
    let db = Connection::open(path).map_err(db_error)?;
    crate::sqlite::configure(&db).map_err(db_error)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS freshness_baselines(
             preparation_id TEXT PRIMARY KEY NOT NULL REFERENCES preparations(preparation_id),
             base_ref TEXT NOT NULL,
             head_ref TEXT NOT NULL,
             base_sha TEXT NOT NULL,
             head_sha TEXT NOT NULL,
             pull_request_state TEXT NOT NULL,
             captured_at_ms INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS freshness_observations(
             preparation_id TEXT PRIMARY KEY NOT NULL REFERENCES preparations(preparation_id),
             checked_at_ms INTEGER NOT NULL,
             observation_json TEXT NOT NULL
         );",
    )
    .map_err(db_error)?;
    Ok(db)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn db_error(_: rusqlite::Error) -> ContractError {
    ContractError::new(
        "STATE_STORAGE_FAILED",
        "Could not store freshness observation",
        "",
    )
}

fn json_error(_: serde_json::Error) -> ContractError {
    ContractError::new(
        "STATE_STORAGE_FAILED",
        "Could not encode freshness state",
        "",
    )
}

fn state_error() -> ContractError {
    ContractError::new(
        "STATE_STORAGE_FAILED",
        "Could not store freshness state",
        "",
    )
}
