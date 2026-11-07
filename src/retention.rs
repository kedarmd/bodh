//! Bounded pruning of immutable local review history.
use crate::{
    ContractError, DraftStore, GeneratedReviewStore, PreparationStore, ReviewPublicationStore,
    load_latest_discussions,
};
use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy)]
pub struct RetentionPolicy {
    pub per_repository_preparations: usize,
    pub global_preparations: usize,
    pub per_repository_bytes: u64,
    pub global_bytes: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            per_repository_preparations: 50,
            global_preparations: 500,
            per_repository_bytes: 64 * 1024 * 1024,
            global_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RetentionReport {
    pub preparations_before: u64,
    pub preparations_after: u64,
    pub estimated_bytes_before: u64,
    pub estimated_bytes_after: u64,
    pub deleted_preparations: u64,
    pub deleted_bytes: u64,
    pub protected_over_budget_preparations: u64,
    pub protected_over_budget_bytes: u64,
    pub protected_over_budget_per_repository_preparations: u64,
    pub protected_over_budget_per_repository_bytes: u64,
    pub unassociated_protected_bytes: u64,
}

static PIN_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Unique owner for one live native window's active-preparation lease.
pub fn new_active_session_id() -> String {
    format!(
        "{}-{}-{}",
        std::process::id(),
        now_ms(),
        PIN_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Update one window's durable active reference. `None` clears the pin.
pub fn set_active_preparation(
    state: &Path,
    session_id: &str,
    preparation_id: Option<&str>,
) -> Result<(), ContractError> {
    let db = Connection::open(state).map_err(db_error)?;
    crate::sqlite::configure(&db).map_err(db_error)?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS active_preparation_pins(session_id TEXT PRIMARY KEY NOT NULL, pid INTEGER NOT NULL, process_start TEXT NOT NULL, preparation_id TEXT NOT NULL, updated_at INTEGER NOT NULL)").map_err(db_error)?;
    let pid = std::process::id() as i64;
    let start = match process_start(pid as u32) {
        ProcessIdentity::Identified(v) => v,
        _ => {
            return Err(ContractError::new(
                "ACTIVE_REFERENCE_UNAVAILABLE",
                "Cannot identify active application process",
                "",
            ));
        }
    };
    db.execute_batch("BEGIN IMMEDIATE").map_err(db_error)?;
    match preparation_id {
        Some(id) => {
            let exists: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM preparations WHERE preparation_id=?1)",
                    [id],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            if !exists {
                let _ = db.execute_batch("ROLLBACK");
                return Err(ContractError::new(
                    "ACTIVE_PREPARATION_NOT_FOUND",
                    "The selected preparation was removed before it could be pinned",
                    "",
                ));
            }
            db.execute("INSERT INTO active_preparation_pins(session_id,pid,process_start,preparation_id,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(session_id) DO UPDATE SET pid=excluded.pid,process_start=excluded.process_start,preparation_id=excluded.preparation_id,updated_at=excluded.updated_at",params![session_id,pid,start,id,now_ms()]).map_err(db_error)?;
        }
        None => {
            db.execute("DELETE FROM active_preparation_pins WHERE session_id=?1 AND pid=?2 AND process_start=?3",params![session_id,pid,start]).map_err(db_error)?;
        }
    }
    db.execute_batch("COMMIT").map_err(db_error)?;
    Ok(())
}

pub fn clear_active_preparation(state: &Path, session_id: &str) -> Result<(), ContractError> {
    set_active_preparation(state, session_id, None)
}

/// Prune old review history. Migrations happen before the short deletion
/// transaction. Active IDs are supplied by the UI; durable recovery and draft
/// references are re-read while holding the write lock.
pub fn enforce_retention(
    state: &Path,
    policy: RetentionPolicy,
    active_preparation_ids: &[String],
) -> Result<RetentionReport, ContractError> {
    drop(PreparationStore::open(state)?);
    drop(DraftStore::open(state)?);
    drop(GeneratedReviewStore::open(state)?);
    drop(ReviewPublicationStore::open(state)?);
    drop(crate::FindingTriageStore::open(state)?);
    // Opens/migrates the discussion cache without performing network I/O.
    let _ = load_latest_discussions(state, "__retention_schema_probe__")?;
    let db = Connection::open(state).map_err(db_error)?;
    crate::sqlite::configure(&db).map_err(db_error)?;
    db.execute_batch("BEGIN IMMEDIATE;").map_err(db_error)?;
    let result = prune_in_transaction(&db, policy, active_preparation_ids);
    match result {
        Ok(report) => {
            db.execute_batch("COMMIT;").map_err(db_error)?;
            Ok(report)
        }
        Err(error) => {
            let _ = db.execute_batch("ROLLBACK;");
            Err(error)
        }
    }
}

#[derive(Clone)]
struct Item {
    id: String,
    preparation: bool,
    context: String,
    host: String,
    owner: String,
    repo: String,
    created: i64,
    bytes: u64,
    protected: bool,
}

fn prune_in_transaction(
    db: &Connection,
    policy: RetentionPolicy,
    active: &[String],
) -> Result<RetentionReport, ContractError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS active_preparation_pins(session_id TEXT PRIMARY KEY NOT NULL, pid INTEGER NOT NULL, process_start TEXT NOT NULL, preparation_id TEXT NOT NULL, updated_at INTEGER NOT NULL)").map_err(db_error)?;
    let now = now_ms();
    if table_exists(db, "generated_review_replacement_approvals")? {
        db.execute(
            "DELETE FROM generated_review_replacement_approvals WHERE expires_at_ms<=?1",
            [now],
        )
        .map_err(db_error)?;
    }
    if table_exists(db, "latest_switch_approvals")? {
        db.execute(
            "DELETE FROM latest_switch_approvals WHERE expires_at_ms<=?1",
            [now],
        )
        .map_err(db_error)?;
    }
    let mut active_ids = active
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let pins: Vec<(String, i64, String, String)> = {
        let mut q = db
            .prepare(
                "SELECT session_id,pid,process_start,preparation_id FROM active_preparation_pins",
            )
            .map_err(db_error)?;
        q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(db_error)?
            .collect::<Result<_, _>>()
            .map_err(db_error)?
    };
    for (session, pid, start, id) in pins {
        match process_start(pid as u32) {
            ProcessIdentity::Identified(current) if current == start => {
                active_ids.insert(id);
            }
            ProcessIdentity::Unknown => {
                active_ids.insert(id);
            }
            ProcessIdentity::Absent | ProcessIdentity::Identified(_) => {
                db.execute(
                    "DELETE FROM active_preparation_pins WHERE session_id=?1",
                    [session],
                )
                .map_err(db_error)?;
            }
        }
    }
    let mut items = Vec::new();
    {
        let sizes = sql_byte_lengths(
            "length(p.title)+length(coalesce(p.description,''))+length(p.checkout)+length(c.context_json)+length(c.changed_files_json)",
        );
        let query = format!(
            "SELECT p.preparation_id,p.context_id,p.host,p.owner,p.repository,p.created_at,{sizes} FROM preparations p JOIN contexts c USING(context_id)"
        );
        let mut stmt = db.prepare(&query).map_err(db_error)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .map_err(db_error)?;
        for row in rows {
            let (id, context, host, owner, repo, created, base_bytes) = row.map_err(db_error)?;
            let generated: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM generated_reviews WHERE context_id=?1)",
                    [&context],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            let draft = exists_context(db, "manual_text_drafts", &context)?
                || exists_context(db, "code_replacement_drafts", &context)?
                || exists_context(db, "reply_drafts", &context)?
                || exists_context(db, "thread_action_drafts", &context)?
                || exists_context(db, "finding_triage_entries", &context)?
                || exists_context(db, "finding_alternative_edits", &context)?
                || context_has_anchor_repair(db, &context)?;
            let recovery = exists_preparation(db, "review_intents", &id)?
                || exists_unresolved_review_operation(db, &id)?
                || exists_preparation(db, "generated_review_replacement_approvals", &id)?
                || exists_switch_approval(db, &id)?
                || exists_open_discussion_plan(db, &id)?;
            let is_active = active_ids.contains(&id);
            let bytes = associated_bytes(db, &id, base_bytes.max(0) as u64)?;
            items.push(Item {
                id,
                preparation: true,
                context,
                host,
                owner,
                repo,
                created,
                bytes,
                protected: generated || draft || recovery || is_active,
            });
        }
    }
    // Contexts without preparations are usually stale source/context cache.
    // Give them a deterministic LRU age (rowid) and include their bytes in the
    // same repository/global byte budgets. Any generated result or draft is a
    // recovery/user payload and therefore pins the context until explicitly
    // resolved by its owning workflow.
    {
        let sizes = sql_byte_lengths("length(c.context_json)+length(c.changed_files_json)");
        let query = format!(
            "SELECT c.context_id,c.context_json,c.rowid,{sizes} FROM contexts c WHERE NOT EXISTS(SELECT 1 FROM preparations p WHERE p.context_id=c.context_id)"
        );
        let mut stmt = db.prepare(&query).map_err(db_error)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(db_error)?;
        for row in rows {
            let (context, json, created, base_bytes) = row.map_err(db_error)?;
            let value: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
            let pr = value.get("pr").unwrap_or(&value);
            let host = pr
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let owner = pr
                .get("owner")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let repo = pr
                .get("repository")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let generated = exists_context(db, "generated_reviews", &context)?;
            let protected = generated
                || exists_context(db, "manual_text_drafts", &context)?
                || exists_context(db, "code_replacement_drafts", &context)?
                || exists_context(db, "reply_drafts", &context)?
                || exists_context(db, "thread_action_drafts", &context)?
                || exists_context(db, "finding_triage_entries", &context)?
                || exists_context(db, "finding_alternative_edits", &context)?
                || context_has_anchor_repair(db, &context)?;
            let bytes = orphan_context_bytes(db, &context, base_bytes.max(0) as u64)?;
            items.push(Item {
                id: format!("context:{context}"),
                preparation: false,
                context,
                host,
                owner,
                repo,
                created,
                bytes,
                protected,
            });
        }
    }
    let before = items.iter().filter(|i| i.preparation).count() as u64;
    let orphan_bytes = items
        .iter()
        .filter(|i| !i.preparation && i.protected)
        .map(|i| i.bytes)
        .sum::<u64>();
    let bytes_before = items.iter().map(|i| i.bytes).sum::<u64>();
    items.sort_by(|a, b| a.created.cmp(&b.created).then(a.id.cmp(&b.id)));
    let mut keep = std::collections::HashSet::new();
    let mut repo_counts = std::collections::HashMap::<(String, String, String), usize>::new();
    let mut repo_bytes = std::collections::HashMap::<(String, String, String), u64>::new();
    let mut protected_repo_counts =
        std::collections::HashMap::<(String, String, String), usize>::new();
    let mut protected_repo_bytes =
        std::collections::HashMap::<(String, String, String), u64>::new();
    let mut total_count = 0usize;
    let mut total_bytes = 0u64;
    // Protected items are floors, even when they already exceed policy.
    for item in &items {
        if item.protected {
            keep.insert(item.id.clone());
            total_count += usize::from(item.preparation);
            total_bytes += item.bytes;
            let key = (item.host.clone(), item.owner.clone(), item.repo.clone());
            *repo_counts.entry(key.clone()).or_default() += usize::from(item.preparation);
            *repo_bytes.entry(key.clone()).or_default() += item.bytes;
            *protected_repo_counts.entry(key.clone()).or_default() += usize::from(item.preparation);
            *protected_repo_bytes.entry(key).or_default() += item.bytes;
        }
    }
    for item in items.iter().filter(|i| !i.protected).rev() {
        let key = (item.host.clone(), item.owner.clone(), item.repo.clone());
        let count_ok = (!item.preparation || total_count < policy.global_preparations)
            && (!item.preparation
                || *repo_counts.get(&key).unwrap_or(&0) < policy.per_repository_preparations);
        let bytes_ok = total_bytes.saturating_add(item.bytes) <= policy.global_bytes
            && repo_bytes
                .get(&key)
                .copied()
                .unwrap_or(0)
                .saturating_add(item.bytes)
                <= policy.per_repository_bytes;
        if count_ok && bytes_ok {
            keep.insert(item.id.clone());
            total_count += usize::from(item.preparation);
            total_bytes += item.bytes;
            *repo_counts.entry(key.clone()).or_default() += usize::from(item.preparation);
            *repo_bytes.entry(key).or_default() += item.bytes;
        }
    }
    let mut report = RetentionReport {
        preparations_before: before,
        preparations_after: items
            .iter()
            .filter(|i| i.preparation && keep.contains(&i.id))
            .count() as u64,
        estimated_bytes_before: bytes_before,
        unassociated_protected_bytes: orphan_bytes,
        ..Default::default()
    };
    let mut contexts_to_check = std::collections::HashSet::new();
    for item in &items {
        if !keep.contains(&item.id) {
            if !item.preparation {
                // Cache-only orphan context selected for eviction. The
                // protected predicate above excludes drafts/results/recovery.
                db.execute("DELETE FROM contexts WHERE context_id=?1", [&item.context])
                    .map_err(db_error)?;
                report.deleted_bytes = report.deleted_bytes.saturating_add(item.bytes);
                continue;
            }
            if table_exists(db, "review_published_drafts")? {
                db.execute(
                    "DELETE FROM review_published_drafts WHERE preparation_id=?1",
                    [&item.id],
                )
                .map_err(db_error)?;
                db.execute(
                    "DELETE FROM review_published_replacements WHERE preparation_id=?1",
                    [&item.id],
                )
                .map_err(db_error)?;
                // Successful history is settled and bounded with its preparation.
                // Delete retry leaves first to respect the self-reference.
                loop {
                    let removed = db.execute("DELETE FROM review_operations WHERE preparation_id=?1 AND state='confirmed' AND NOT EXISTS(SELECT 1 FROM review_operations child WHERE child.retry_of=review_operations.operation_id)", [&item.id]).map_err(db_error)?;
                    if removed == 0 {
                        break;
                    }
                }
            }
            if table_exists(db, "discussion_publication_plans")? {
                db.execute(
                    "DELETE FROM discussion_published_replies WHERE preparation_id=?1",
                    [&item.id],
                )
                .map_err(db_error)?;
                db.execute(
                    "DELETE FROM discussion_published_thread_actions WHERE preparation_id=?1",
                    [&item.id],
                )
                .map_err(db_error)?;
                db.execute("DELETE FROM discussion_publication_steps WHERE plan_id IN (SELECT plan_id FROM discussion_publication_plans WHERE preparation_id=?1)",[&item.id]).map_err(db_error)?;
                db.execute(
                    "DELETE FROM discussion_publication_plans WHERE preparation_id=?1",
                    [&item.id],
                )
                .map_err(db_error)?;
            }
            delete_if_table(db, "freshness_observations", &item.id)?;
            delete_if_table(db, "freshness_baselines", &item.id)?;
            delete_if_table(db, "discussion_observations", &item.id)?;
            db.execute(
                "DELETE FROM preparations WHERE preparation_id=?1",
                [&item.id],
            )
            .map_err(db_error)?;
            contexts_to_check.insert(item.context.clone());
            report.deleted_preparations += 1;
            report.deleted_bytes += item.bytes;
        }
    }
    for context in contexts_to_check {
        let referenced: bool=db.query_row("SELECT EXISTS(SELECT 1 FROM preparations WHERE context_id=?1) OR EXISTS(SELECT 1 FROM generated_reviews WHERE context_id=?1) OR EXISTS(SELECT 1 FROM manual_text_drafts WHERE context_id=?1) OR EXISTS(SELECT 1 FROM code_replacement_drafts WHERE context_id=?1) OR EXISTS(SELECT 1 FROM reply_drafts WHERE context_id=?1) OR EXISTS(SELECT 1 FROM thread_action_drafts WHERE context_id=?1)", [&context], |r| r.get(0)).map_err(db_error)?;
        if !referenced {
            db.execute("DELETE FROM contexts WHERE context_id=?1", [&context])
                .map_err(db_error)?;
        }
    }
    report.estimated_bytes_after = bytes_before.saturating_sub(report.deleted_bytes);
    report.protected_over_budget_preparations =
        total_count.saturating_sub(policy.global_preparations) as u64;
    report.protected_over_budget_bytes = total_bytes.saturating_sub(policy.global_bytes);
    report.protected_over_budget_per_repository_preparations = protected_repo_counts
        .values()
        .map(|n| n.saturating_sub(policy.per_repository_preparations) as u64)
        .sum();
    report.protected_over_budget_per_repository_bytes = protected_repo_bytes
        .values()
        .map(|n| n.saturating_sub(policy.per_repository_bytes))
        .sum();
    Ok(report)
}

fn exists_context(db: &Connection, table: &str, context: &str) -> Result<bool, ContractError> {
    let exists: bool = db
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE context_id=?1)"),
            [context],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    Ok(exists)
}
fn exists_preparation(db: &Connection, table: &str, id: &str) -> Result<bool, ContractError> {
    if !table_exists(db, table)? {
        return Ok(false);
    }
    db.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE preparation_id=?1)"),
        [id],
        |r| r.get(0),
    )
    .map_err(db_error)
}
fn exists_unresolved_review_operation(db: &Connection, id: &str) -> Result<bool, ContractError> {
    if !table_exists(db, "review_operations")? {
        return Ok(false);
    }
    db.query_row("SELECT EXISTS(SELECT 1 FROM review_operations WHERE preparation_id=?1 AND state IN ('planned','sending','failed','unknown'))",[id],|r|r.get(0)).map_err(db_error)
}
fn exists_switch_approval(db: &Connection, id: &str) -> Result<bool, ContractError> {
    if !table_exists(db, "latest_switch_approvals")? {
        return Ok(false);
    }
    db.query_row("SELECT EXISTS(SELECT 1 FROM latest_switch_approvals WHERE current_preparation_id=?1 OR candidate_preparation_id=?1)",[id],|r|r.get(0)).map_err(db_error)
}
fn context_has_anchor_repair(db: &Connection, context: &str) -> Result<bool, ContractError> {
    if !table_exists(db, "draft_anchor_repairs")? {
        return Ok(false);
    }
    db.query_row("SELECT EXISTS(SELECT 1 FROM draft_anchor_repairs WHERE source_context_id=?1 OR candidate_context_id=?1)",[context],|r|r.get(0)).map_err(db_error)
}
fn exists_open_discussion_plan(db: &Connection, id: &str) -> Result<bool, ContractError> {
    if !table_exists(db, "discussion_publication_plans")? {
        return Ok(false);
    }
    db.query_row("SELECT EXISTS(SELECT 1 FROM discussion_publication_plans p WHERE p.preparation_id=?1 AND (p.state IN ('planned','sending','partial','failed','unknown') OR EXISTS(SELECT 1 FROM discussion_publication_steps s WHERE s.plan_id=p.plan_id AND s.state IN ('planned','sending','failed','unknown','unsent'))))",[id],|r|r.get(0)).map_err(db_error)
}
fn table_exists(db: &Connection, table: &str) -> Result<bool, ContractError> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get(0),
    )
    .map_err(db_error)
}
/// SQLite `length(TEXT)` counts Unicode code points. Retention budgets are
/// stored-byte estimates, so turn each operand into a BLOB before measuring.
fn sql_byte_lengths(expr: &str) -> String {
    let bytes = expr.as_bytes();
    let mut out = String::with_capacity(expr.len() + 32);
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"length(") {
            let start = i + b"length(".len();
            let mut j = start;
            let mut depth = 1usize;
            let mut quote = false;
            while j < bytes.len() && depth > 0 {
                match bytes[j] {
                    b'\'' => {
                        if quote && j + 1 < bytes.len() && bytes[j + 1] == b'\'' {
                            j += 1;
                        } else {
                            quote = !quote;
                        }
                    }
                    b'(' if !quote => depth += 1,
                    b')' if !quote => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            if depth == 0 {
                out.push_str("length(CAST(");
                out.push_str(&expr[start..j - 1]);
                out.push_str(" AS BLOB))");
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}
fn orphan_context_bytes(db: &Connection, context: &str, base: u64) -> Result<u64, ContractError> {
    let mut total = base as i64;
    for (table, expr) in [
        (
            "generated_reviews",
            "length(result_json)+length(validation_context_json)+length(prior_concerns_json)",
        ),
        (
            "manual_text_drafts",
            "length(body)+length(source_json)+length(provenance_json)",
        ),
        (
            "code_replacement_drafts",
            "length(source_json)+length(original_text)+length(replacement)+length(provenance_json)",
        ),
        ("reply_drafts", "length(body)"),
        ("thread_action_drafts", "length(action)"),
        (
            "finding_triage_entries",
            "length(coalesce(manual_feedback,''))+length(coalesce(selected_suggestion_id,''))+length(disposition)",
        ),
        (
            "finding_alternative_edits",
            "length(body)+length(source_json)+length(coalesce(original_text,''))+length(coalesce(replacement,''))",
        ),
    ] {
        if table_exists(db, table)? {
            let expr = sql_byte_lengths(expr);
            let sql =
                format!("SELECT coalesce(sum({expr}),0) FROM {table} d WHERE d.context_id=?1");
            total = total.saturating_add(
                db.query_row(&sql, [context], |r| r.get::<_, i64>(0))
                    .map_err(db_error)?,
            );
        }
    }
    Ok(total.max(0) as u64)
}
fn associated_bytes(db: &Connection, id: &str, base: u64) -> Result<u64, ContractError> {
    let context: String = db
        .query_row(
            "SELECT context_id FROM preparations WHERE preparation_id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    let mut bytes = base;
    let queries = [
        (
            "generated_reviews",
            "context_id",
            "length(result_json)+length(validation_context_json)+length(prior_concerns_json)",
        ),
        (
            "discussion_observations",
            "preparation_id",
            "length(coalesce(incomplete_cause,''))+length(coalesce(incomplete_connection,''))",
        ),
        (
            "discussion_threads",
            "run_id",
            "length(thread_json)+length(coalesce(incomplete_cause,''))",
        ),
        (
            "discussion_pages",
            "run_id",
            "length(nodes_json)+length(coalesce(cursor_in,''))+length(coalesce(end_cursor,''))",
        ),
        (
            "manual_text_drafts",
            "context_id",
            "length(body)+length(source_json)+length(provenance_json)",
        ),
        (
            "code_replacement_drafts",
            "context_id",
            "length(source_json)+length(original_text)+length(replacement)+length(provenance_json)",
        ),
        ("reply_drafts", "context_id", "length(body)"),
        ("thread_action_drafts", "context_id", "length(action)"),
        (
            "finding_triage_entries",
            "context_id",
            "length(coalesce(manual_feedback,''))+length(coalesce(selected_suggestion_id,''))+length(disposition)",
        ),
        (
            "finding_alternative_edits",
            "context_id",
            "length(body)+length(source_json)+length(coalesce(original_text,''))+length(coalesce(replacement,''))",
        ),
        ("review_intents", "preparation_id", "length(intent_json)"),
        (
            "review_operations",
            "preparation_id",
            "length(payload_json)+length(coalesce(failure,''))",
        ),
        (
            "discussion_publication_plans",
            "preparation_id",
            "length(fingerprint)",
        ),
        (
            "freshness_baselines",
            "preparation_id",
            "length(base_ref)+length(head_ref)+length(base_sha)+length(head_sha)+length(pull_request_state)",
        ),
        (
            "freshness_observations",
            "preparation_id",
            "length(observation_json)",
        ),
        (
            "generated_review_replacement_approvals",
            "preparation_id",
            "length(work_json)+length(candidate_fingerprint)+length(validation_context_fingerprint)+length(work_fingerprint)",
        ),
    ];
    for (table, key, expr) in queries {
        if !table_exists(db, table)? {
            continue;
        }
        if table == "discussion_threads" || table == "discussion_pages" {
            continue;
        }
        let key_id = if key == "context_id" {
            context.clone()
        } else {
            id.to_owned()
        };
        let expr = sql_byte_lengths(expr);
        let sql = if table == "discussion_observations" {
            format!("SELECT coalesce(sum({expr}),0) FROM {table} WHERE preparation_id=?1")
        } else {
            format!("SELECT coalesce(sum({expr}),0) FROM {table} WHERE {key}=?1")
        };
        let n: i64 = db
            .query_row(&sql, [key_id], |r| r.get(0))
            .map_err(db_error)?;
        bytes = bytes.saturating_add(n.max(0) as u64);
    }
    if table_exists(db, "discussion_observations")? {
        let thread = sql_byte_lengths("length(thread_json)+length(coalesce(incomplete_cause,''))");
        let pages = sql_byte_lengths(
            "length(nodes_json)+length(coalesce(cursor_in,''))+length(coalesce(end_cursor,''))",
        );
        let query = format!(
            "SELECT coalesce(sum((SELECT coalesce(sum({thread}),0) FROM discussion_threads t WHERE t.run_id=o.run_id)+(SELECT coalesce(sum({pages}),0) FROM discussion_pages p WHERE p.run_id=o.run_id)),0) FROM discussion_observations o WHERE o.preparation_id=?1"
        );
        let n: i64 = db.query_row(&query, [id], |r| r.get(0)).map_err(db_error)?;
        bytes = bytes.saturating_add(n.max(0) as u64);
    }
    if table_exists(db, "discussion_publication_plans")? {
        let expr = sql_byte_lengths("length(s.body)+length(coalesce(s.failure,''))");
        let query = format!(
            "SELECT coalesce(sum({expr}),0) FROM discussion_publication_plans p JOIN discussion_publication_steps s USING(plan_id) WHERE p.preparation_id=?1"
        );
        let n: i64 = db.query_row(&query, [id], |r| r.get(0)).map_err(db_error)?;
        bytes = bytes.saturating_add(n.max(0) as u64);
    }
    Ok(bytes)
}
fn delete_if_table(db: &Connection, table: &str, id: &str) -> Result<(), ContractError> {
    let exists = table_exists(db, table)?;
    if exists {
        db.execute(
            &format!("DELETE FROM {table} WHERE preparation_id=?1"),
            [id],
        )
        .map_err(db_error)?;
    }
    Ok(())
}
fn db_error(error: rusqlite::Error) -> ContractError {
    ContractError::new("STATE_DATABASE_ERROR", &error.to_string(), "")
}
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
/// Returns `None` only when the PID is provably absent. Unreadable or malformed
/// process metadata is represented by a stable sentinel so cleanup preserves it.
enum ProcessIdentity {
    Absent,
    Identified(String),
    Unknown,
}
fn process_start(pid: u32) -> ProcessIdentity {
    let contents = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ProcessIdentity::Absent,
        Err(_) => return ProcessIdentity::Unknown,
    };
    let tail = contents
        .rsplit_once(") ")
        .map(|(_, tail)| tail)
        .unwrap_or("");
    tail.split_whitespace()
        .nth(19)
        .map(|v| ProcessIdentity::Identified(v.to_owned()))
        .unwrap_or(ProcessIdentity::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DraftStore, GeneratedReviewStore, PrIdentity, PreparationStore, ReviewContext};
    use rusqlite::{Connection, params};
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db() -> std::path::PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("bodh-retention-{n}.sqlite"))
    }

    fn insert_preparation(db: &Connection, id: &str, context: &str, number: i64, created: i64) {
        db.execute("INSERT OR IGNORE INTO workspaces(host,owner,repository,number) VALUES('github.com','o','r',?1)", [number]).unwrap();
        db.execute(
            "INSERT INTO contexts(context_id,context_json,changed_files_json) VALUES(?1,?2,?3)",
            params![context, json!({"context_id": context}).to_string(), "[]"],
        )
        .unwrap();
        db.execute("INSERT INTO preparations(preparation_id,context_id,host,owner,repository,number,expected_generation,created_at,title,description,checkout) VALUES(?1,?2,'github.com','o','r',?3,0,?4,'title',NULL,'/tmp')", params![id,context,number,created]).unwrap();
    }

    #[test]
    fn repeated_prepare_discussion_publication_restart_cycles_stay_bounded_and_reload_recovery() {
        let path = temp_db();
        let _store = PreparationStore::open(&path).unwrap();
        drop(DraftStore::open(&path).unwrap());
        drop(GeneratedReviewStore::open(&path).unwrap());
        drop(crate::ReviewPublicationStore::open(&path).unwrap());
        drop(crate::DiscussionPublicationStore::open(&path).unwrap());
        let _ = crate::load_latest_discussions(&path, "schema-probe").unwrap();
        let policy = RetentionPolicy {
            per_repository_preparations: 3,
            global_preparations: 4,
            per_repository_bytes: 1024 * 1024,
            global_bytes: 2 * 1024 * 1024,
        };
        let mut protected = Vec::new();
        let mut deleted_total = 0;
        for cycle in 0..12u64 {
            let context = ReviewContext {
                pr: PrIdentity {
                    host: "github.com".into(),
                    owner: "cycles".into(),
                    repository: "repo".into(),
                    number: cycle + 1,
                },
                context_id: format!("cycle-context-{cycle}"),
                base_sha: "a".repeat(40),
                head_sha: "b".repeat(40),
                merge_base_sha: "c".repeat(40),
            };
            let binding = PreparationStore::open(&path)
                .unwrap()
                .save(
                    &context,
                    "/checkout",
                    "cycle title",
                    Some(&format!("source context {cycle}")),
                    &[],
                )
                .unwrap();
            let db = Connection::open(&path).unwrap();
            db.execute("INSERT INTO discussion_observations(run_id,preparation_id,expected_threads,fetched_threads,threads_complete,complete,created_at) VALUES(?1,?2,1,1,1,1,?3)",params![format!("run-{cycle}"),binding.preparation_id,cycle as i64]).unwrap();
            db.execute("INSERT INTO discussion_threads(run_id,ordinal,node_id,thread_json,expected_comments,fetched_comments,comments_complete) VALUES(?1,0,?2,?3,1,1,1)",params![format!("run-{cycle}"),format!("node-{cycle}"),format!("{{\"body\":\"{}\"}}","discussion payload ".repeat(5))]).unwrap();
            if cycle == 8 {
                db.execute("INSERT INTO review_operations(operation_id,preparation_id,fingerprint,state,payload_json,created_at,updated_at) VALUES('cycle-confirmed',?1,'fp','confirmed','{}',8,8)",[&binding.preparation_id]).unwrap();
                db.execute("INSERT INTO review_published_drafts(operation_id,preparation_id,context_id,draft_id,draft_revision) VALUES('cycle-confirmed',?1,?2,'published-draft',1)",params![binding.preparation_id,context.context_id]).unwrap();
            }
            if cycle == 10 {
                db.execute("INSERT INTO generated_reviews VALUES('github.com','cycles','repo',11,?1,'{\"result\":\"current\"}','{}','[]',1,10)",[&context.context_id]).unwrap();
                db.execute("INSERT INTO manual_text_drafts(draft_id,context_id,host,owner,repository,number,body,source_json,provenance_json,revision,created_at,updated_at) VALUES('cycle-draft',?1,'github.com','cycles','repo',11,'keep draft','{}','{}',1,10,10)",[&context.context_id]).unwrap();
                db.execute("INSERT INTO review_operations(operation_id,preparation_id,fingerprint,state,payload_json,created_at,updated_at) VALUES('cycle-unknown',?1,'fp','unknown','{\"body\":\"uncertain\"}',10,10)",[&binding.preparation_id]).unwrap();
                protected.push(binding.preparation_id.clone());
            }
            if cycle == 11 {
                db.execute("INSERT INTO discussion_publication_plans(plan_id,preparation_id,context_id,intent_revision,fingerprint,state,created_at,updated_at) VALUES('cycle-plan',?1,?2,1,'fp','failed',11,11)",params![binding.preparation_id,context.context_id]).unwrap();
                protected.push(binding.preparation_id.clone());
            }
            drop(db);
            deleted_total += enforce_retention(&path, policy, &[])
                .unwrap()
                .deleted_preparations;
            drop(PreparationStore::open(&path).unwrap()); // simulate restart/reopen between cycles
        }
        let report = enforce_retention(&path, policy, &[]).unwrap();
        eprintln!(
            "retention cycle evidence: created=12 deleted_total={deleted_total} retained={} estimated_bytes={}",
            report.preparations_after, report.estimated_bytes_after
        );
        assert!(report.preparations_after <= 4);
        assert!(report.estimated_bytes_after <= 2 * 1024 * 1024);
        let store = PreparationStore::open(&path).unwrap();
        for id in protected {
            store.load(&id).unwrap();
        }
        let db = Connection::open(&path).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM review_operations WHERE operation_id='cycle-unknown'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM discussion_publication_plans WHERE plan_id='cycle-plan'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM manual_text_drafts WHERE draft_id='cycle-draft'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM review_operations WHERE operation_id='cycle-confirmed'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM review_published_drafts WHERE draft_id='published-draft'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert!(deleted_total >= 8);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn prunes_lru_candidates_with_count_budget_and_keeps_current_active_and_draft() {
        let path = temp_db();
        let _store = PreparationStore::open(&path).unwrap();
        drop(DraftStore::open(&path).unwrap());
        drop(GeneratedReviewStore::open(&path).unwrap());
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        insert_preparation(&db, "old", "ctx-old", 10, 1);
        insert_preparation(&db, "draft", "ctx-draft", 11, 2);
        insert_preparation(&db, "current", "ctx-current", 12, 3);
        insert_preparation(&db, "active", "ctx-active", 13, 4);
        db.execute("INSERT INTO generated_reviews VALUES('github.com','o','r',12,'ctx-current','{}','{}','[]',1,3)", []).unwrap();
        db.execute("INSERT INTO manual_text_drafts(draft_id,context_id,host,owner,repository,number,body,source_json,provenance_json,revision,created_at,updated_at) VALUES('d1','ctx-draft','github.com','o','r',11,'draft','{}','{}',1,2,2)", []).unwrap();
        drop(db);

        set_active_preparation(&path, "test-window", Some("active")).unwrap();
        let report = enforce_retention(
            &path,
            RetentionPolicy {
                per_repository_preparations: 3,
                global_preparations: 3,
                per_repository_bytes: u64::MAX,
                global_bytes: u64::MAX,
            },
            &[],
        )
        .unwrap();

        let db = Connection::open(&path).unwrap();
        let retained: Vec<String> = {
            let mut q = db
                .prepare("SELECT preparation_id FROM preparations ORDER BY preparation_id")
                .unwrap();
            q.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(retained, vec!["active", "current", "draft"]);
        assert_eq!(report.deleted_preparations, 1);
        clear_active_preparation(&path, "test-window").unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn byte_budget_evicts_orphan_cache_but_protects_orphan_generated_and_draft_payloads() {
        let path = temp_db();
        let _store = PreparationStore::open(&path).unwrap();
        drop(DraftStore::open(&path).unwrap());
        drop(GeneratedReviewStore::open(&path).unwrap());
        let db = Connection::open(&path).unwrap();
        db.execute("INSERT INTO workspaces(host,owner,repository,number) VALUES('github.com','org','repo',2)", []).unwrap();
        db.execute("INSERT INTO contexts(context_id,context_json,changed_files_json) VALUES('orphan-cache',?1,?2)", params![json!({"pr":{"host":"github.com","owner":"org","repository":"repo","number":1}}).to_string(), "source cache ".repeat(1000)]).unwrap();
        db.execute("INSERT INTO contexts(context_id,context_json,changed_files_json) VALUES('orphan-draft',?1,'[]')", [json!({"pr":{"host":"github.com","owner":"org","repository":"repo","number":2}}).to_string()]).unwrap();
        db.execute("INSERT INTO contexts(context_id,context_json,changed_files_json) VALUES('orphan-result',?1,'[]')", [json!({"pr":{"host":"github.com","owner":"org","repository":"repo","number":3}}).to_string()]).unwrap();
        db.execute("INSERT INTO generated_reviews VALUES('github.com','org','repo',3,'orphan-result','{\"result\":\"keep\"}','{}','[]',1,1)", []).unwrap();
        db.execute("INSERT INTO manual_text_drafts(draft_id,context_id,host,owner,repository,number,body,source_json,provenance_json,revision,created_at,updated_at) VALUES('orphan-draft-row','orphan-draft','github.com','org','repo',2,'keep this draft','{}','{}',1,1,1)", []).unwrap();
        drop(db);

        let report = enforce_retention(
            &path,
            RetentionPolicy {
                per_repository_preparations: 10,
                global_preparations: 10,
                per_repository_bytes: 8,
                global_bytes: 8,
            },
            &[],
        )
        .unwrap();
        let db = Connection::open(&path).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM contexts WHERE context_id='orphan-cache'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0,
            "stale orphan source/context cache should be evicted"
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM contexts WHERE context_id='orphan-draft'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT body FROM manual_text_drafts WHERE draft_id='orphan-draft-row'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "keep this draft"
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM generated_reviews WHERE context_id='orphan-result'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM contexts WHERE context_id='orphan-result'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert!(report.unassociated_protected_bytes > 0);
        assert_eq!(report.estimated_bytes_before, 13_263);
        assert_eq!(report.estimated_bytes_after, 190);
        assert_eq!(report.deleted_bytes, 13_073);
        assert!(report.protected_over_budget_bytes > 0);
        assert!(report.protected_over_budget_per_repository_bytes > 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn publication_recovery_and_failed_cleanup_keep_source_state_atomic() {
        let path = temp_db();
        let _store = PreparationStore::open(&path).unwrap();
        drop(DraftStore::open(&path).unwrap());
        drop(GeneratedReviewStore::open(&path).unwrap());
        drop(crate::ReviewPublicationStore::open(&path).unwrap());
        drop(crate::DiscussionPublicationStore::open(&path).unwrap());
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        insert_preparation(&db, "failed", "ctx-failed", 20, 1);
        insert_preparation(&db, "discussion", "ctx-discussion", 21, 2);
        insert_preparation(&db, "ordinary", "ctx-ordinary", 22, 3);
        db.execute("INSERT INTO review_operations(operation_id,preparation_id,fingerprint,state,payload_json,created_at,updated_at) VALUES('op1','failed','fp','failed','{\"payload\":1}',1,1)",[]).unwrap();
        db.execute("INSERT INTO discussion_publication_plans(plan_id,preparation_id,context_id,intent_revision,fingerprint,state,created_at,updated_at) VALUES('plan1','discussion','ctx-discussion',1,'fp','unknown',2,2)",[]).unwrap();
        db.execute_batch("CREATE TRIGGER prevent_prune BEFORE DELETE ON preparations WHEN OLD.preparation_id='ordinary' BEGIN SELECT RAISE(ABORT,'injected prune failure'); END;").unwrap();
        drop(db);
        let result = enforce_retention(
            &path,
            RetentionPolicy {
                per_repository_preparations: 0,
                global_preparations: 0,
                per_repository_bytes: 0,
                global_bytes: 0,
            },
            &[],
        );
        assert!(result.is_err());
        let db = Connection::open(&path).unwrap();
        let count: i64 = db
            .query_row("SELECT count(*) FROM preparations", [], |r| r.get(0))
            .unwrap();
        let recovery: i64 = db
            .query_row(
                "SELECT count(*) FROM review_operations WHERE operation_id='op1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
        assert_eq!(recovery, 1);
        let discussion_recovery: i64 = db
            .query_row(
                "SELECT count(*) FROM discussion_publication_plans WHERE plan_id='plan1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(discussion_recovery, 1);
        let journal: String = db
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(journal.to_ascii_lowercase(), "wal");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reports_protected_per_repository_floor_and_enforces_byte_budget_on_lru() {
        let path = temp_db();
        let _store = PreparationStore::open(&path).unwrap();
        drop(DraftStore::open(&path).unwrap());
        drop(GeneratedReviewStore::open(&path).unwrap());
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        insert_preparation(&db, "old", "ctx-old", 30, 1);
        insert_preparation(&db, "current", "ctx-current", 31, 2);
        insert_preparation(&db, "draft", "ctx-draft", 32, 3);
        db.execute("INSERT INTO generated_reviews VALUES('github.com','o','r',31,'ctx-current','{\"result\":\"large-current\"}','{}','[]',1,2)",[]).unwrap();
        db.execute("INSERT INTO manual_text_drafts(draft_id,context_id,host,owner,repository,number,body,source_json,provenance_json,revision,created_at,updated_at) VALUES('d1','ctx-draft','github.com','o','r',32,'saved draft','{}','{}',1,3,3)",[]).unwrap();
        drop(db);
        let report = enforce_retention(
            &path,
            RetentionPolicy {
                per_repository_preparations: 1,
                global_preparations: 1,
                per_repository_bytes: 1,
                global_bytes: 1,
            },
            &[],
        )
        .unwrap();
        assert_eq!(report.preparations_after, 2);
        assert_eq!(report.deleted_preparations, 1);
        assert!(report.deleted_bytes > 0);
        assert_eq!(report.protected_over_budget_per_repository_preparations, 1);
        assert!(report.protected_over_budget_per_repository_bytes > 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn concurrent_resume_pin_and_prune_never_leave_a_pin_to_deleted_preparation() {
        let path = temp_db();
        let _store = PreparationStore::open(&path).unwrap();
        let db = Connection::open(&path).unwrap();
        insert_preparation(&db, "race", "ctx-race", 50, 1);
        drop(db);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let pin_path = path.clone();
        let pin_barrier = barrier.clone();
        let pin = std::thread::spawn(move || {
            pin_barrier.wait();
            set_active_preparation(&pin_path, "racing-window", Some("race"))
        });
        let cleanup_path = path.clone();
        let cleanup_barrier = barrier.clone();
        let cleanup = std::thread::spawn(move || {
            cleanup_barrier.wait();
            enforce_retention(
                &cleanup_path,
                RetentionPolicy {
                    per_repository_preparations: 0,
                    global_preparations: 0,
                    per_repository_bytes: 0,
                    global_bytes: 0,
                },
                &[],
            )
        });
        barrier.wait();
        let pin_result = pin.join().unwrap();
        cleanup.join().unwrap().unwrap();
        let db = Connection::open(&path).unwrap();
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM preparations WHERE preparation_id='race')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let pin_exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM active_preparation_pins WHERE session_id='racing-window')",[],|r|r.get(0)).unwrap();
        assert_eq!(exists, pin_result.is_ok());
        assert_eq!(pin_exists, pin_result.is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn byte_estimator_uses_utf8_storage_length_not_character_count() {
        assert_eq!(sql_byte_lengths("length('é')"), "length(CAST('é' AS BLOB))");
    }

    #[test]
    fn consumed_replacement_approval_tombstone_survives_until_expiry() {
        let path = temp_db();
        let _store = PreparationStore::open(&path).unwrap();
        drop(DraftStore::open(&path).unwrap());
        drop(GeneratedReviewStore::open(&path).unwrap());
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        insert_preparation(&db, "approved", "ctx-approved", 60, 1);
        insert_preparation(&db, "ordinary", "ctx-ordinary", 61, 2);
        db.execute("INSERT INTO generated_review_replacement_approvals(approval_id,preparation_id,host,owner,repository,number,context_id,workspace_revision,candidate_fingerprint,validation_context_fingerprint,work_fingerprint,work_json,expires_at_ms,consumed_at_ms) VALUES('used','approved','github.com','o','r',60,'ctx-approved',0,'candidate','context','work','{}',?1,?2)",params![now_ms()+60_000,now_ms()]).unwrap();
        drop(db);
        let _ = enforce_retention(
            &path,
            RetentionPolicy {
                per_repository_preparations: 1,
                global_preparations: 1,
                per_repository_bytes: u64::MAX,
                global_bytes: u64::MAX,
            },
            &[],
        )
        .unwrap();
        let db = Connection::open(&path).unwrap();
        assert_eq!(db.query_row("SELECT count(*) FROM generated_review_replacement_approvals WHERE approval_id='used'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM preparations WHERE preparation_id='approved'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        let _ = std::fs::remove_file(&path);
    }
}
