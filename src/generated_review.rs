//! Production validation and atomic persistence for one current generated review.
use crate::{
    ContractError, DiscussionFailure, DiscussionObservation, DraftStore, OperationControl,
    PreparationStore, PreparedBinding, PreparedEvidenceContext, PriorConcern, ReviewResult,
    validate_complete, validate_structure,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommittedGeneratedReview {
    pub result: ReviewResult,
    pub context: PreparedEvidenceContext,
    pub generation: u64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GeneratedReviewCommit {
    pub status: String,
    pub generation: u64,
    pub result: ReviewResult,
}

pub struct GeneratedReviewStore {
    path: PathBuf,
}

impl GeneratedReviewStore {
    pub fn open(path: &Path) -> Result<Self, ContractError> {
        drop(PreparationStore::open(path)?);
        drop(DraftStore::open(path)?);
        let db = open_connection(path)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS generated_review_migrations (
                 version INTEGER PRIMARY KEY NOT NULL
             );
             CREATE TABLE IF NOT EXISTS generated_reviews (
                 host TEXT NOT NULL,
                 owner TEXT NOT NULL,
                 repository TEXT NOT NULL,
                 number INTEGER NOT NULL,
                 context_id TEXT NOT NULL,
                 result_json TEXT NOT NULL,
                 validation_context_json TEXT NOT NULL,
                 prior_concerns_json TEXT NOT NULL,
                 generation INTEGER NOT NULL CHECK(generation > 0),
                 updated_at INTEGER NOT NULL,
                 PRIMARY KEY(host,owner,repository,number)
             );
             INSERT OR IGNORE INTO generated_review_migrations(version) VALUES(1);",
        )
        .map_err(db_error)?;
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    /// Fully validates a candidate against the selected preparation and current
    /// durable context without changing the active result or workspace revision.
    pub fn validate(
        &self,
        preparation_id: &str,
        expected_generation: u64,
        input: &[u8],
        control: &OperationControl,
    ) -> Result<ReviewResult, ContractError> {
        let db = open_connection(&self.path)?;
        let binding = load_binding(&db, preparation_id)?;
        require_current_context(&db, &binding)?;
        let generation = workspace_generation(&db, &binding.context.pr)?;
        require_generation(generation, expected_generation)?;
        let prepared = validation_context(&db, &binding)?;
        validate_complete(input, &prepared, Path::new(&binding.checkout), control)
    }

    /// Independently validates and atomically installs the complete result plus
    /// its compact follow-up index. A changed result cannot replace unpublished
    /// reviewer work; exact recommits are read-only no-ops.
    pub fn commit(
        &self,
        preparation_id: &str,
        expected_generation: u64,
        input: &[u8],
        control: &OperationControl,
    ) -> Result<GeneratedReviewCommit, ContractError> {
        let candidate = validate_structure(input)?;
        let mut db = open_connection(&self.path)?;
        let binding = load_binding(&db, preparation_id)?;
        require_current_context(&db, &binding)?;
        if let Some(current) = load_current(&db, &binding.context.pr)?
            && current.context.context == candidate.context
            && current.result == candidate
        {
            return Ok(GeneratedReviewCommit {
                status: "unchanged".into(),
                generation: current.generation,
                result: current.result,
            });
        }

        // Commit never trusts an earlier validate invocation. It takes a fresh
        // preparation, discussion and prior-concern snapshot and validates again.
        let validation_snapshot = validation_context(&db, &binding)?;
        let validated = validate_complete(
            input,
            &validation_snapshot,
            Path::new(&binding.checkout),
            control,
        )?;
        let result_json = serde_json::to_string(&validated).map_err(json_error)?;
        let validation_context_json =
            serde_json::to_string(&validation_snapshot).map_err(json_error)?;
        let prior_concerns = validated
            .findings
            .iter()
            .map(|finding| PriorConcern {
                id: finding.id.clone(),
                title: finding.title.clone(),
                summary: finding.summary.clone(),
            })
            .collect::<Vec<_>>();
        let prior_concerns_json = serde_json::to_string(&prior_concerns).map_err(json_error)?;

        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        require_current_context(&tx, &binding)?;
        if let Some(current) = load_current(&tx, &binding.context.pr)?
            && current.context.context == validated.context
            && current.result == validated
        {
            tx.rollback().map_err(db_error)?;
            return Ok(GeneratedReviewCommit {
                status: "unchanged".into(),
                generation: current.generation,
                result: current.result,
            });
        }

        if unpublished_work_count(&tx, &binding)? > 0 {
            return Err(ContractError::new(
                "APPROVAL_REQUIRED",
                "Changed generated results cannot replace unpublished reviewer work",
                "",
            ));
        }
        let current_generation = workspace_generation(&tx, &binding.context.pr)?;
        require_generation(current_generation, expected_generation)?;
        let current_validation_context = validation_context(&tx, &binding)?;
        if serde_json::to_string(&current_validation_context).map_err(json_error)?
            != validation_context_json
        {
            return Err(ContractError::new(
                "PREPARATION_CONTEXT_STALE",
                "Prepared discussions or prior concerns changed during validation; reload context and validate again",
                "",
            ));
        }
        control.check()?;
        let next_generation = current_generation.checked_add(1).ok_or_else(state_error)?;
        let pr = &binding.context.pr;
        let changed = tx
            .execute(
                "UPDATE workspaces SET generation=?1
                 WHERE host=?2 AND owner=?3 AND repository=?4 AND number=?5 AND generation=?6",
                params![
                    i64::try_from(next_generation).map_err(|_| state_error())?,
                    pr.host,
                    pr.owner,
                    pr.repository,
                    i64::try_from(pr.number).map_err(|_| state_error())?,
                    i64::try_from(current_generation).map_err(|_| state_error())?,
                ],
            )
            .map_err(db_error)?;
        if changed != 1 {
            return Err(stale_generation());
        }
        tx.execute(
            "INSERT INTO generated_reviews(
                 host,owner,repository,number,context_id,result_json,
                 validation_context_json,prior_concerns_json,generation,updated_at
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(host,owner,repository,number) DO UPDATE SET
                 context_id=excluded.context_id,
                 result_json=excluded.result_json,
                 validation_context_json=excluded.validation_context_json,
                 prior_concerns_json=excluded.prior_concerns_json,
                 generation=excluded.generation,
                 updated_at=excluded.updated_at",
            params![
                pr.host,
                pr.owner,
                pr.repository,
                i64::try_from(pr.number).map_err(|_| state_error())?,
                validated.context.context_id,
                result_json,
                validation_context_json,
                prior_concerns_json,
                i64::try_from(next_generation).map_err(|_| state_error())?,
                unix_millis(),
            ],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(GeneratedReviewCommit {
            status: "committed".into(),
            generation: next_generation,
            result: validated,
        })
    }

    pub fn current(
        &self,
        preparation_id: &str,
    ) -> Result<Option<CommittedGeneratedReview>, ContractError> {
        let db = open_connection(&self.path)?;
        let binding = load_binding(&db, preparation_id)?;
        load_current(&db, &binding.context.pr)
    }

    pub fn workspace_generation(&self, preparation_id: &str) -> Result<u64, ContractError> {
        let db = open_connection(&self.path)?;
        let binding = load_binding(&db, preparation_id)?;
        workspace_generation(&db, &binding.context.pr)
    }
}

fn open_connection(path: &Path) -> Result<Connection, ContractError> {
    let db = Connection::open(path).map_err(db_error)?;
    db.busy_timeout(Duration::from_secs(5)).map_err(db_error)?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;")
        .map_err(db_error)?;
    Ok(db)
}

fn load_binding(db: &Connection, preparation_id: &str) -> Result<PreparedBinding, ContractError> {
    let row: Option<(String, String, i64, String, Option<String>, String)> = db
        .query_row(
            "SELECT p.context_id,c.context_json,p.expected_generation,p.title,p.description,p.checkout
             FROM preparations p JOIN contexts c ON c.context_id=p.context_id
             WHERE p.preparation_id=?1",
            [preparation_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?;
    let Some((context_id, context_json, expected_generation, title, description, checkout)) = row
    else {
        return Err(ContractError::new(
            "PREPARATION_NOT_FOUND",
            "Preparation ID was not found",
            "",
        ));
    };
    let files_json: String = db
        .query_row(
            "SELECT changed_files_json FROM contexts WHERE context_id=?1",
            [&context_id],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    Ok(PreparedBinding {
        preparation_id: preparation_id.into(),
        expected_generation: u64::try_from(expected_generation).map_err(|_| state_error())?,
        context: serde_json::from_str(&context_json).map_err(json_error)?,
        title,
        description,
        changed_files: serde_json::from_str(&files_json).map_err(json_error)?,
        checkout,
    })
}

fn require_current_context(
    db: &Connection,
    binding: &PreparedBinding,
) -> Result<(), ContractError> {
    let pr = &binding.context.pr;
    let current: Option<String> = db
        .query_row(
            "SELECT context_id FROM preparations
             WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4
             ORDER BY rowid DESC LIMIT 1",
            params![
                pr.host,
                pr.owner,
                pr.repository,
                i64::try_from(pr.number).map_err(|_| state_error())?
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    if current.as_deref() != Some(binding.context.context_id.as_str()) {
        return Err(ContractError::new(
            "PREPARATION_CONTEXT_STALE",
            "A newer prepared context exists for this pull request",
            "",
        ));
    }
    Ok(())
}

fn workspace_generation(db: &Connection, pr: &crate::PrIdentity) -> Result<u64, ContractError> {
    let generation: i64 = db
        .query_row(
            "SELECT generation FROM workspaces WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4",
            params![pr.host,pr.owner,pr.repository,i64::try_from(pr.number).map_err(|_| state_error())?],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    u64::try_from(generation).map_err(|_| state_error())
}

fn require_generation(actual: u64, expected: u64) -> Result<(), ContractError> {
    if actual != expected {
        return Err(stale_generation());
    }
    Ok(())
}

fn validation_context(
    db: &Connection,
    binding: &PreparedBinding,
) -> Result<PreparedEvidenceContext, ContractError> {
    let discussion = discussion_observation(db, &binding.preparation_id)?;
    let prior_concerns = db
        .query_row(
            "SELECT prior_concerns_json FROM generated_reviews
             WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4",
            params![
                binding.context.pr.host,
                binding.context.pr.owner,
                binding.context.pr.repository,
                i64::try_from(binding.context.pr.number).map_err(|_| state_error())?
            ],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(db_error)?
        .map(|value| serde_json::from_str(&value).map_err(json_error))
        .transpose()?;
    Ok(PreparedEvidenceContext {
        context: binding.context.clone(),
        original_description: binding.description.clone(),
        discussions: discussion,
        prior_concerns,
    })
}

fn discussion_observation(
    db: &Connection,
    preparation_id: &str,
) -> Result<DiscussionObservation, ContractError> {
    if !table_exists(db, "discussion_observations")? {
        return Ok(DiscussionObservation::default());
    }
    type ObservationRow = (String, bool, bool, Option<String>, Option<String>);
    let observation: Option<ObservationRow> = db
        .query_row(
            "SELECT run_id, threads_complete != 0, complete != 0, incomplete_connection, incomplete_cause
             FROM discussion_observations WHERE preparation_id=?1
             ORDER BY rowid DESC LIMIT 1",
            [preparation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()
        .map_err(db_error)?;
    let Some((run_id, threads_complete, observation_complete, connection, cause)) = observation
    else {
        return Ok(DiscussionObservation::default());
    };
    let thread_ids = if table_exists(db, "discussion_threads")? {
        let mut statement = db
            .prepare("SELECT node_id FROM discussion_threads WHERE run_id=?1 ORDER BY ordinal")
            .map_err(db_error)?;
        let rows = statement
            .query_map([run_id.as_str()], |row| row.get::<_, String>(0))
            .map_err(db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?
    } else {
        Vec::new()
    };
    let count_changed = if table_exists(db, "discussion_pages")? {
        db.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM discussion_pages a JOIN discussion_pages b
                   ON a.run_id=b.run_id AND a.connection_name=b.connection_name
                  AND a.thread_key=b.thread_key AND a.expected_count IS NOT NULL
                  AND b.expected_count IS NOT NULL AND a.expected_count<>b.expected_count
                 WHERE a.run_id=?1
             )",
            [run_id.as_str()],
            |row| row.get::<_, bool>(0),
        )
        .map_err(db_error)?
    } else {
        false
    };
    let complete = threads_complete && observation_complete && !count_changed;
    let failures = if complete {
        Vec::new()
    } else {
        vec![DiscussionFailure {
            connection: connection.unwrap_or_else(|| "threads".into()),
            reason: cause.unwrap_or_else(|| {
                if count_changed {
                    "DISCUSSION_COUNT_CHANGED".into()
                } else {
                    "DISCUSSIONS_INCOMPLETE".into()
                }
            }),
        }]
    };
    Ok(DiscussionObservation {
        thread_ids,
        complete,
        failures,
    })
}

fn load_current(
    db: &Connection,
    pr: &crate::PrIdentity,
) -> Result<Option<CommittedGeneratedReview>, ContractError> {
    let row: Option<(String, String, String, i64, i64)> = db
        .query_row(
            "SELECT result_json,validation_context_json,context_id,generation,updated_at
             FROM generated_reviews WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4",
            params![
                pr.host,
                pr.owner,
                pr.repository,
                i64::try_from(pr.number).map_err(|_| state_error())?
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?;
    row.map(|(result, context, context_id, generation, updated_at)| {
        let result: ReviewResult = serde_json::from_str(&result).map_err(json_error)?;
        let context: PreparedEvidenceContext =
            serde_json::from_str(&context).map_err(json_error)?;
        if context.context.context_id != context_id {
            return Err(state_error());
        }
        Ok(CommittedGeneratedReview {
            result,
            context,
            generation: u64::try_from(generation).map_err(|_| state_error())?,
            updated_at,
        })
    })
    .transpose()
}

fn unpublished_work_count(
    db: &Connection,
    binding: &PreparedBinding,
) -> Result<u64, ContractError> {
    let pr = &binding.context.pr;
    let mut total = 0u64;
    for table in [
        "manual_text_drafts",
        "code_replacement_drafts",
        "reply_drafts",
        "thread_action_drafts",
    ] {
        if !table_exists(db, table)? {
            continue;
        }
        let query = if table_exists(db, "review_published_drafts")? {
            format!(
                "SELECT count(*) FROM {table} d
                 WHERE d.host=?1 AND d.owner=?2 AND d.repository=?3 AND d.number=?4
                   AND NOT EXISTS (
                     SELECT 1 FROM review_published_drafts p
                     WHERE p.context_id=d.context_id AND p.draft_id=d.draft_id
                       AND p.draft_revision=d.revision
                   )"
            )
        } else {
            format!(
                "SELECT count(*) FROM {table}
                 WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4"
            )
        };
        let count: i64 = db
            .query_row(
                &query,
                params![
                    pr.host,
                    pr.owner,
                    pr.repository,
                    i64::try_from(pr.number).map_err(|_| state_error())?
                ],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        total = total
            .checked_add(u64::try_from(count).map_err(|_| state_error())?)
            .ok_or_else(state_error)?;
    }
    if table_exists(db, "review_intents")? {
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM review_intents WHERE preparation_id=?1",
                [&binding.preparation_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        total = total
            .checked_add(u64::try_from(count).map_err(|_| state_error())?)
            .ok_or_else(state_error)?;
    }
    if table_exists(db, "review_operations")? {
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM review_operations
                 WHERE preparation_id=?1 AND state IN ('planned','sending','unknown')",
                [&binding.preparation_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        total = total
            .checked_add(u64::try_from(count).map_err(|_| state_error())?)
            .ok_or_else(state_error)?;
    }
    Ok(total)
}

fn table_exists(db: &Connection, table: &str) -> Result<bool, ContractError> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
    .map_err(db_error)
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn stale_generation() -> ContractError {
    ContractError::new(
        "WORKSPACE_REVISION_STALE",
        "Workspace content changed; reload the manifest and validate again",
        "",
    )
}
fn state_error() -> ContractError {
    ContractError::new("STATE_UNAVAILABLE", "Review state is unavailable", "")
}
fn db_error(_: rusqlite::Error) -> ContractError {
    ContractError::new("STATE_UNAVAILABLE", "Review state is unavailable", "")
}
fn json_error(_: serde_json::Error) -> ContractError {
    ContractError::new("STATE_UNAVAILABLE", "Review state is unavailable", "")
}
