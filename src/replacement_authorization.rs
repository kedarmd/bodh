//! Human-approved, one-use grants for replacing a generated review with local
//! unpublished work. The grant fingerprints the complete protected snapshot;
//! consumption and draft deletion occur inside the result replacement transaction.
use crate::{ContractError, PrIdentity, PreparedBinding, ReviewIntent};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const APPROVAL_TTL_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReplacementApprovalPreview {
    pub preparation_id: String,
    pub host: String,
    pub owner: String,
    pub repository: String,
    pub pull_request_number: u64,
    pub context_id: String,
    pub workspace_revision: u64,
    pub inline_comments: u64,
    pub code_replacements: u64,
    pub replies: u64,
    pub thread_actions: u64,
    pub review_intents: u64,
    pub total_unpublished_work: u64,
    pub candidate_fingerprint: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplacementApproval {
    pub id: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DraftRef {
    table: String,
    id: String,
    context_id: String,
    revision: u64,
    row_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct IntentRef {
    preparation_id: String,
    revision: u64,
    intent_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AuthorizedWork {
    drafts: Vec<DraftRef>,
    intents: Vec<IntentRef>,
}

#[derive(Debug, Clone, Serialize)]
struct DraftMaterial {
    table: String,
    id: String,
    context_id: String,
    revision: i64,
    row: Value,
}

#[derive(Debug, Clone, Serialize)]
struct IntentMaterial {
    preparation_id: String,
    revision: i64,
    intent_json: String,
}

#[derive(Debug, Clone, Serialize)]
struct OperationMaterial {
    preparation_id: String,
    operation_id: String,
    state: String,
    fingerprint: String,
    payload_json: String,
}

#[derive(Debug, Clone, Serialize)]
struct SnapshotMaterial {
    pr: PrIdentity,
    preparation_id: String,
    context_id: String,
    workspace_revision: u64,
    candidate_fingerprint: String,
    validation_context_fingerprint: String,
    drafts: Vec<DraftMaterial>,
    intents: Vec<IntentMaterial>,
    operations: Vec<OperationMaterial>,
}

#[derive(Debug)]
pub(crate) struct ProtectedSnapshot {
    pub preview: ReplacementApprovalPreview,
    pub fingerprint: String,
    pub unresolved_publications: u64,
    work: AuthorizedWork,
}

pub(crate) fn ensure_schema(db: &Connection) -> Result<(), ContractError> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS generated_review_replacement_approvals(
             approval_id TEXT PRIMARY KEY NOT NULL,
             preparation_id TEXT NOT NULL REFERENCES preparations(preparation_id),
             host TEXT NOT NULL,
             owner TEXT NOT NULL,
             repository TEXT NOT NULL,
             number INTEGER NOT NULL,
             context_id TEXT NOT NULL,
             workspace_revision INTEGER NOT NULL,
             candidate_fingerprint TEXT NOT NULL,
             validation_context_fingerprint TEXT NOT NULL,
             work_fingerprint TEXT NOT NULL,
             work_json TEXT NOT NULL,
             expires_at_ms INTEGER NOT NULL,
             consumed_at_ms INTEGER
         );
         CREATE INDEX IF NOT EXISTS generated_review_replacement_approvals_expiry
             ON generated_review_replacement_approvals(expires_at_ms, consumed_at_ms);",
    )
    .map_err(db_error)
}

pub(crate) fn protected_snapshot(
    db: &Connection,
    prepared: &PreparedBinding,
    workspace_revision: u64,
    candidate_fingerprint: &str,
    validation_context_fingerprint: &str,
) -> Result<ProtectedSnapshot, ContractError> {
    let pr = &prepared.context.pr;
    let number = i64::try_from(pr.number).map_err(|_| state_error())?;
    let mut drafts = Vec::new();
    let mut material_drafts = Vec::new();
    let mut inline = 0u64;
    let mut replacements = 0u64;
    let mut replies = 0u64;
    let mut thread_actions = 0u64;

    for (table, published_table, category) in [
        (
            "manual_text_drafts",
            Some("review_published_drafts"),
            "inline",
        ),
        (
            "code_replacement_drafts",
            Some("review_published_replacements"),
            "replacement",
        ),
        (
            "reply_drafts",
            Some("discussion_published_replies"),
            "reply",
        ),
        (
            "thread_action_drafts",
            Some("discussion_published_thread_actions"),
            "thread_action",
        ),
    ] {
        let mut rows = collect_drafts(db, pr, number, table, published_table)?;
        match category {
            "inline" => inline = u64::try_from(rows.len()).map_err(|_| state_error())?,
            "replacement" => replacements = u64::try_from(rows.len()).map_err(|_| state_error())?,
            "reply" => replies = u64::try_from(rows.len()).map_err(|_| state_error())?,
            "thread_action" => {
                thread_actions = u64::try_from(rows.len()).map_err(|_| state_error())?
            }
            _ => return Err(state_error()),
        }
        for (draft_ref, row) in rows.drain(..) {
            material_drafts.push(row);
            drafts.push(draft_ref);
        }
    }
    drafts.sort_by(|a, b| (&a.table, &a.context_id, &a.id).cmp(&(&b.table, &b.context_id, &b.id)));
    material_drafts
        .sort_by(|a, b| (&a.table, &a.context_id, &a.id).cmp(&(&b.table, &b.context_id, &b.id)));

    let (intents, material_intents, review_intents) = collect_intents(db, pr, &drafts)?;
    let mut operations = collect_operations(db, pr, number)?;
    let discussion_unresolved = collect_discussion_journal(db, pr, number, &mut operations)?;
    let unresolved_publications = u64::try_from(
        operations
            .iter()
            .filter(|operation| {
                matches!(operation.state.as_str(), "planned" | "sending" | "unknown")
            })
            .count(),
    )
    .map_err(|_| state_error())?;
    let unresolved_publications = unresolved_publications
        .checked_add(discussion_unresolved)
        .ok_or_else(state_error)?;

    let material = SnapshotMaterial {
        pr: pr.clone(),
        preparation_id: prepared.preparation_id.clone(),
        context_id: prepared.context.context_id.clone(),
        workspace_revision,
        candidate_fingerprint: candidate_fingerprint.to_owned(),
        validation_context_fingerprint: validation_context_fingerprint.to_owned(),
        drafts: material_drafts,
        intents: material_intents,
        operations: operations.clone(),
    };
    let encoded = serde_json::to_vec(&material).map_err(json_error)?;
    let fingerprint = format!("v1-{:x}", Sha256::digest(encoded));
    let total_unpublished_work = u64::try_from(drafts.len())
        .map_err(|_| state_error())?
        .checked_add(review_intents)
        .ok_or_else(state_error)?;
    let preview = ReplacementApprovalPreview {
        preparation_id: prepared.preparation_id.clone(),
        host: pr.host.clone(),
        owner: pr.owner.clone(),
        repository: pr.repository.clone(),
        pull_request_number: pr.number,
        context_id: prepared.context.context_id.clone(),
        workspace_revision,
        inline_comments: inline,
        code_replacements: replacements,
        replies,
        thread_actions,
        review_intents,
        total_unpublished_work,
        candidate_fingerprint: candidate_fingerprint.to_owned(),
        expires_at_ms: now_ms()
            .checked_add(APPROVAL_TTL_MS)
            .ok_or_else(state_error)?,
    };
    Ok(ProtectedSnapshot {
        preview,
        fingerprint,
        unresolved_publications,
        work: AuthorizedWork { drafts, intents },
    })
}

pub(crate) fn persist_approval<F>(
    state: &Path,
    prepared: &PreparedBinding,
    expected_workspace_revision: u64,
    candidate_fingerprint: &str,
    validation_context_fingerprint: &str,
    initial: &ProtectedSnapshot,
    confirm: F,
) -> Result<ReplacementApproval, ContractError>
where
    F: FnOnce(&ReplacementApprovalPreview) -> Result<bool, ContractError>,
{
    if initial.unresolved_publications > 0 {
        return Err(publication_recovery_required());
    }
    if initial.preview.total_unpublished_work == 0 {
        return Err(ContractError::new(
            "APPROVAL_NOT_REQUIRED",
            "No unpublished reviewer work needs approval before replacing this result",
            "",
        ));
    }
    if !confirm(&initial.preview)? {
        return Err(ContractError::new(
            "APPROVAL_DECLINED",
            "Generated review replacement was not approved",
            "",
        ));
    }

    let mut db = open_connection(state)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    require_current_context(&tx, prepared)?;
    require_workspace_revision(&tx, &prepared.context.pr, expected_workspace_revision)?;
    let fresh = protected_snapshot(
        &tx,
        prepared,
        expected_workspace_revision,
        candidate_fingerprint,
        validation_context_fingerprint,
    )?;
    if fresh.unresolved_publications > 0 {
        return Err(publication_recovery_required());
    }
    if fresh.fingerprint != initial.fingerprint || fresh.preview.total_unpublished_work == 0 {
        return Err(approval_stale());
    }

    let approval_id = new_approval_id()?;
    let expires_at_ms = now_ms()
        .checked_add(APPROVAL_TTL_MS)
        .ok_or_else(state_error)?;
    tx.execute(
        "INSERT INTO generated_review_replacement_approvals(
             approval_id,preparation_id,host,owner,repository,number,context_id,
             workspace_revision,candidate_fingerprint,validation_context_fingerprint,
             work_fingerprint,work_json,expires_at_ms,consumed_at_ms
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,NULL)",
        params![
            approval_id,
            prepared.preparation_id,
            prepared.context.pr.host,
            prepared.context.pr.owner,
            prepared.context.pr.repository,
            i64::try_from(prepared.context.pr.number).map_err(|_| state_error())?,
            prepared.context.context_id,
            i64::try_from(expected_workspace_revision).map_err(|_| state_error())?,
            candidate_fingerprint,
            validation_context_fingerprint,
            fresh.fingerprint,
            serde_json::to_string(&fresh.work).map_err(json_error)?,
            expires_at_ms,
        ],
    )
    .map_err(db_error)?;
    tx.commit().map_err(db_error)?;
    Ok(ReplacementApproval {
        id: approval_id,
        expires_at_ms,
    })
}

/// Revalidates and consumes a grant inside the generated-result transaction.
pub(crate) fn consume_approval(
    tx: &Transaction<'_>,
    prepared: &PreparedBinding,
    expected_workspace_revision: u64,
    candidate_fingerprint: &str,
    validation_context_fingerprint: &str,
    approval_id: &str,
) -> Result<AuthorizedWork, ContractError> {
    type ApprovalRow = (
        String,
        String,
        String,
        String,
        i64,
        String,
        i64,
        String,
        String,
        String,
        String,
        i64,
        Option<i64>,
    );
    let row: Option<ApprovalRow> = tx
        .query_row(
            "SELECT preparation_id,host,owner,repository,number,context_id,
                    workspace_revision,candidate_fingerprint,
                    validation_context_fingerprint,work_fingerprint,work_json,
                    expires_at_ms,consumed_at_ms
             FROM generated_review_replacement_approvals WHERE approval_id=?1",
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
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?;
    let Some((
        preparation_id,
        host,
        owner,
        repository,
        number,
        context_id,
        workspace_revision,
        stored_candidate,
        stored_validation,
        stored_fingerprint,
        work_json,
        expires_at_ms,
        consumed_at_ms,
    )) = row
    else {
        return Err(approval_invalid());
    };
    if consumed_at_ms.is_some() {
        return Err(ContractError::new(
            "APPROVAL_ALREADY_USED",
            "This generated review replacement approval was already consumed",
            "",
        ));
    }
    if expires_at_ms <= now_ms() {
        return Err(ContractError::new(
            "APPROVAL_EXPIRED",
            "This generated review replacement approval has expired",
            "",
        ));
    }
    if preparation_id != prepared.preparation_id
        || host != prepared.context.pr.host
        || owner != prepared.context.pr.owner
        || repository != prepared.context.pr.repository
        || number != i64::try_from(prepared.context.pr.number).map_err(|_| state_error())?
        || context_id != prepared.context.context_id
        || stored_candidate != candidate_fingerprint
        || stored_validation != validation_context_fingerprint
    {
        return Err(approval_invalid());
    }
    if workspace_revision
        != i64::try_from(expected_workspace_revision).map_err(|_| state_error())?
    {
        return Err(approval_stale());
    }
    let fresh = protected_snapshot(
        tx,
        prepared,
        expected_workspace_revision,
        candidate_fingerprint,
        validation_context_fingerprint,
    )?;
    if fresh.unresolved_publications > 0 {
        return Err(publication_recovery_required());
    }
    if fresh.fingerprint != stored_fingerprint {
        return Err(approval_stale());
    }
    let authorized: AuthorizedWork = serde_json::from_str(&work_json).map_err(json_error)?;
    if authorized != fresh.work {
        return Err(approval_stale());
    }
    let changed = tx
        .execute(
            "UPDATE generated_review_replacement_approvals SET consumed_at_ms=?1
             WHERE approval_id=?2 AND consumed_at_ms IS NULL AND expires_at_ms>?1",
            params![now_ms(), approval_id],
        )
        .map_err(db_error)?;
    if changed != 1 {
        return Err(approval_stale());
    }
    Ok(authorized)
}

/// Deletes exactly the draft revisions authorized by the grant and clears the
/// corresponding staged intent without touching publication journals/markers.
pub(crate) fn clear_authorized_work(
    tx: &Transaction<'_>,
    prepared: &PreparedBinding,
    work: &AuthorizedWork,
) -> Result<(), ContractError> {
    let pr = &prepared.context.pr;
    for draft in &work.drafts {
        let sql = match draft.table.as_str() {
            "manual_text_drafts" => {
                "DELETE FROM manual_text_drafts WHERE draft_id=?1 AND context_id=?2 AND revision=?3 AND host=?4 AND owner=?5 AND repository=?6 AND number=?7"
            }
            "code_replacement_drafts" => {
                "DELETE FROM code_replacement_drafts WHERE draft_id=?1 AND context_id=?2 AND revision=?3 AND host=?4 AND owner=?5 AND repository=?6 AND number=?7"
            }
            "reply_drafts" => {
                "DELETE FROM reply_drafts WHERE draft_id=?1 AND context_id=?2 AND revision=?3 AND host=?4 AND owner=?5 AND repository=?6 AND number=?7"
            }
            "thread_action_drafts" => {
                "DELETE FROM thread_action_drafts WHERE draft_id=?1 AND context_id=?2 AND revision=?3 AND host=?4 AND owner=?5 AND repository=?6 AND number=?7"
            }
            _ => return Err(state_error()),
        };
        let changed = tx
            .execute(
                sql,
                params![
                    draft.id,
                    draft.context_id,
                    i64::try_from(draft.revision).map_err(|_| state_error())?,
                    pr.host,
                    pr.owner,
                    pr.repository,
                    i64::try_from(pr.number).map_err(|_| state_error())?,
                ],
            )
            .map_err(db_error)?;
        if changed != 1 {
            return Err(approval_stale());
        }
    }
    for intent in &work.intents {
        let row: Option<String> = tx
            .query_row(
                "SELECT intent_json FROM review_intents WHERE preparation_id=?1 AND revision=?2",
                params![
                    intent.preparation_id,
                    i64::try_from(intent.revision).map_err(|_| state_error())?
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if row.as_deref() != Some(intent.intent_json.as_str()) {
            return Err(approval_stale());
        }
        let mut value: Value = serde_json::from_str(&intent.intent_json).map_err(json_error)?;
        let next_revision = intent.revision.checked_add(1).ok_or_else(state_error)?;
        value["revision"] = json!(next_revision);
        value["selected"] = json!([]);
        value["selected_replacements"] = json!([]);
        value["summary"] = Value::Null;
        value["event"] = Value::Null;
        let changed = tx
            .execute(
                "UPDATE review_intents SET revision=?1,intent_json=?2,updated_at=?3
                 WHERE preparation_id=?4 AND revision=?5 AND intent_json=?6",
                params![
                    i64::try_from(next_revision).map_err(|_| state_error())?,
                    serde_json::to_string(&value).map_err(json_error)?,
                    now_ms(),
                    intent.preparation_id,
                    i64::try_from(intent.revision).map_err(|_| state_error())?,
                    intent.intent_json,
                ],
            )
            .map_err(db_error)?;
        if changed != 1 {
            return Err(approval_stale());
        }
    }
    Ok(())
}

fn collect_drafts(
    db: &Connection,
    pr: &PrIdentity,
    number: i64,
    table: &str,
    marker_table: Option<&str>,
) -> Result<Vec<(DraftRef, DraftMaterial)>, ContractError> {
    if !table_exists(db, table)? {
        return Ok(Vec::new());
    }
    let marker = match marker_table {
        Some(marker) if table_exists(db, marker)? => Some(marker),
        _ => None,
    };
    let predicate = marker.map_or_else(String::new, |marker| {
        format!(
            "AND NOT EXISTS(SELECT 1 FROM {marker} p WHERE p.context_id=d.context_id AND p.draft_id=d.draft_id AND p.draft_revision=d.revision)"
        )
    });
    let (source, qualifier) = if marker.is_some() {
        (format!("{table} d"), "d.")
    } else {
        (table.to_owned(), "")
    };
    let sql = format!(
        "SELECT {qualifier}* FROM {source} WHERE {qualifier}host=?1 AND {qualifier}owner=?2 AND {qualifier}repository=?3 AND {qualifier}number=?4 {predicate} ORDER BY {qualifier}context_id,{qualifier}draft_id"
    );
    let mut statement = db.prepare(&sql).map_err(db_error)?;
    let columns = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let id_index = columns
        .iter()
        .position(|column| column == "draft_id")
        .ok_or_else(state_error)?;
    let context_index = columns
        .iter()
        .position(|column| column == "context_id")
        .ok_or_else(state_error)?;
    let revision_index = columns
        .iter()
        .position(|column| column == "revision")
        .ok_or_else(state_error)?;
    let rows = statement
        .query_map(params![pr.host, pr.owner, pr.repository, number], |row| {
            let mut object = serde_json::Map::new();
            for (index, column) in columns.iter().enumerate() {
                object.insert(column.clone(), sql_value(row.get_ref(index)?));
            }
            let id: String = row.get(id_index)?;
            let context_id: String = row.get(context_index)?;
            let revision: i64 = row.get(revision_index)?;
            Ok((id, context_id, revision, Value::Object(object)))
        })
        .map_err(db_error)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, context_id, revision, row_value) = row.map_err(db_error)?;
        let row_json = serde_json::to_string(&row_value).map_err(json_error)?;
        let row_fingerprint = format!("{:x}", Sha256::digest(row_json.as_bytes()));
        out.push((
            DraftRef {
                table: table.to_owned(),
                id: id.clone(),
                context_id: context_id.clone(),
                revision: u64::try_from(revision).map_err(|_| state_error())?,
                row_fingerprint,
            },
            DraftMaterial {
                table: table.to_owned(),
                id,
                context_id,
                revision,
                row: row_value,
            },
        ));
    }
    Ok(out)
}

fn collect_intents(
    db: &Connection,
    pr: &PrIdentity,
    drafts: &[DraftRef],
) -> Result<(Vec<IntentRef>, Vec<IntentMaterial>, u64), ContractError> {
    if !table_exists(db, "review_intents")? {
        return Ok((Vec::new(), Vec::new(), 0));
    }
    let mut statement = db
        .prepare(
            "SELECT i.preparation_id,i.revision,i.intent_json
             FROM review_intents i JOIN preparations p ON p.preparation_id=i.preparation_id
             WHERE p.host=?1 AND p.owner=?2 AND p.repository=?3 AND p.number=?4
             ORDER BY i.preparation_id",
        )
        .map_err(db_error)?;
    let rows = statement
        .query_map(
            params![
                pr.host,
                pr.owner,
                pr.repository,
                i64::try_from(pr.number).map_err(|_| state_error())?
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(db_error)?;
    let mut refs = Vec::new();
    let mut material = Vec::new();
    let mut unpublished = 0u64;
    for row in rows {
        let (preparation_id, revision, intent_json) = row.map_err(db_error)?;
        let intent: ReviewIntent = serde_json::from_str(&intent_json).map_err(json_error)?;
        let current_revision = u64::try_from(revision).map_err(|_| state_error())?;
        let published = intent_revision_published(db, &preparation_id, current_revision)?;
        let references_unpublished_drafts =
            intent.selected.iter().any(|selected| {
                drafts
                    .iter()
                    .any(|draft| draft.id == selected.id && draft.revision == selected.revision)
            }) || intent.selected_replacements.iter().any(|selected| {
                drafts
                    .iter()
                    .any(|draft| draft.id == selected.id && draft.revision == selected.revision)
            });
        let active = references_unpublished_drafts
            || (!published
                && (intent
                    .summary
                    .as_deref()
                    .is_some_and(|summary| !summary.trim().is_empty())
                    || intent.event.is_some()
                    || !intent.selected.is_empty()
                    || !intent.selected_replacements.is_empty()));
        let material_row = IntentMaterial {
            preparation_id: preparation_id.clone(),
            revision,
            intent_json: intent_json.clone(),
        };
        material.push(material_row);
        if active {
            unpublished = unpublished.checked_add(1).ok_or_else(state_error)?;
            refs.push(IntentRef {
                preparation_id,
                revision: current_revision,
                intent_json,
            });
        }
    }
    Ok((refs, material, unpublished))
}

fn intent_revision_published(
    db: &Connection,
    preparation_id: &str,
    revision: u64,
) -> Result<bool, ContractError> {
    if !table_exists(db, "review_operations")? {
        return Ok(false);
    }
    let mut statement = db
        .prepare("SELECT payload_json FROM review_operations WHERE preparation_id=?1 AND state='confirmed'")
        .map_err(db_error)?;
    let rows = statement
        .query_map([preparation_id], |row| row.get::<_, String>(0))
        .map_err(db_error)?;
    for row in rows {
        let payload: Value = serde_json::from_str(&row.map_err(db_error)?).map_err(json_error)?;
        if payload.get("intent_revision").and_then(Value::as_u64) == Some(revision) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn collect_operations(
    db: &Connection,
    pr: &PrIdentity,
    number: i64,
) -> Result<Vec<OperationMaterial>, ContractError> {
    if !table_exists(db, "review_operations")? {
        return Ok(Vec::new());
    }
    let mut statement = db
        .prepare(
            "SELECT o.preparation_id,o.operation_id,o.state,o.fingerprint,o.payload_json
             FROM review_operations o JOIN preparations p ON p.preparation_id=o.preparation_id
             WHERE p.host=?1 AND p.owner=?2 AND p.repository=?3 AND p.number=?4
             ORDER BY o.preparation_id,o.operation_id",
        )
        .map_err(db_error)?;
    let rows = statement
        .query_map(params![pr.host, pr.owner, pr.repository, number], |row| {
            Ok(OperationMaterial {
                preparation_id: row.get(0)?,
                operation_id: row.get(1)?,
                state: row.get(2)?,
                fingerprint: row.get(3)?,
                payload_json: row.get(4)?,
            })
        })
        .map_err(db_error)?;
    rows.map(|row| row.map_err(db_error)).collect()
}

fn collect_discussion_journal(
    db: &Connection,
    pr: &PrIdentity,
    number: i64,
    material: &mut Vec<OperationMaterial>,
) -> Result<u64, ContractError> {
    let plans_exist = table_exists(db, "discussion_publication_plans")?;
    let steps_exist = table_exists(db, "discussion_publication_steps")?;
    if !plans_exist && !steps_exist {
        return Ok(0);
    }
    if plans_exist != steps_exist {
        return Err(ContractError::new(
            "STATE_UNAVAILABLE",
            "Discussion publication journal is incomplete; replacement is blocked",
            "",
        ));
    }
    let mut statement = db.prepare(
        "SELECT p.plan_id,p.preparation_id,p.state,p.fingerprint,
                COALESCE((SELECT json_group_array(json(step)) FROM (SELECT json_object(
                    'step_id',s.step_id,'ordinal',s.ordinal,'kind',s.kind,
                    'draft_id',s.draft_id,'draft_revision',s.draft_revision,
                    'thread_id',s.thread_id,'root_comment_id',s.root_comment_id,
                    'body',s.body,'action',s.action,'observed_resolved',s.observed_resolved,
                    'state',s.state,'remote_id',s.remote_id,'failure',s.failure
                ) AS step FROM discussion_publication_steps s WHERE s.plan_id=p.plan_id ORDER BY s.ordinal)),'[]')
         FROM discussion_publication_plans p JOIN preparations r ON r.preparation_id=p.preparation_id
         WHERE r.host=?1 AND r.owner=?2 AND r.repository=?3 AND r.number=?4
         ORDER BY p.plan_id",
    ).map_err(db_error)?;
    let rows = statement
        .query_map(params![pr.host, pr.owner, pr.repository, number], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(db_error)?;
    let mut unresolved = 0u64;
    for row in rows {
        let (plan_id, preparation_id, state, fingerprint, payload_json) = row.map_err(db_error)?;
        let steps: Value = serde_json::from_str(&payload_json).map_err(json_error)?;
        if let Some(values) = steps.as_array() {
            unresolved = unresolved
                .checked_add(
                    values
                        .iter()
                        .filter(|step| {
                            step.get("state")
                                .and_then(Value::as_str)
                                .is_some_and(|state| {
                                    matches!(state, "planned" | "sending" | "unknown")
                                })
                        })
                        .count() as u64,
                )
                .ok_or_else(state_error)?;
        }
        material.push(OperationMaterial {
            preparation_id,
            operation_id: format!("discussion:{plan_id}"),
            state: format!("journal-{state}"),
            fingerprint,
            payload_json,
        });
    }
    Ok(unresolved)
}

fn require_current_context(
    db: &Connection,
    prepared: &PreparedBinding,
) -> Result<(), ContractError> {
    let current: Option<String> = db
        .query_row(
            "SELECT context_id FROM preparations WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4 ORDER BY rowid DESC LIMIT 1",
            params![prepared.context.pr.host,prepared.context.pr.owner,prepared.context.pr.repository,i64::try_from(prepared.context.pr.number).map_err(|_|state_error())?],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    if current.as_deref() != Some(prepared.context.context_id.as_str()) {
        return Err(ContractError::new(
            "PREPARATION_CONTEXT_STALE",
            "A newer prepared context exists for this pull request",
            "",
        ));
    }
    Ok(())
}

fn require_workspace_revision(
    db: &Connection,
    pr: &PrIdentity,
    expected: u64,
) -> Result<(), ContractError> {
    let generation: i64 = db
        .query_row(
            "SELECT generation FROM workspaces WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4",
            params![pr.host,pr.owner,pr.repository,i64::try_from(pr.number).map_err(|_|state_error())?],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    if u64::try_from(generation).map_err(|_| state_error())? != expected {
        return Err(ContractError::new(
            "WORKSPACE_REVISION_STALE",
            "Workspace changed while replacement approval was being considered",
            "",
        ));
    }
    Ok(())
}

fn table_exists(db: &Connection, table: &str) -> Result<bool, ContractError> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
    .map_err(db_error)
}

fn sql_value(value: rusqlite::types::ValueRef<'_>) -> Value {
    use rusqlite::types::ValueRef;
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(number) => json!(number),
        ValueRef::Real(number) => json!(number),
        ValueRef::Text(text) => json!(String::from_utf8_lossy(text).into_owned()),
        ValueRef::Blob(bytes) => json!(
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
    }
}

fn open_connection(path: &Path) -> Result<Connection, ContractError> {
    let db = Connection::open(path).map_err(db_error)?;
    crate::sqlite::configure(&db).map_err(db_error)?;
    ensure_schema(&db)?;
    Ok(db)
}

fn new_approval_id() -> Result<String, ContractError> {
    let mut random = [0u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut random))
        .map_err(|_| {
            ContractError::new(
                "AUTH_UNAVAILABLE",
                "Secure approval IDs are unavailable",
                "",
            )
        })?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
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
        "STATE_UNAVAILABLE",
        "Review state could not be read or saved",
        "",
    )
}
fn json_error(_: serde_json::Error) -> ContractError {
    ContractError::new("STATE_UNAVAILABLE", "Review state could not be decoded", "")
}
fn state_error() -> ContractError {
    ContractError::new(
        "STATE_UNAVAILABLE",
        "Review state is invalid or out of range",
        "",
    )
}
fn approval_stale() -> ContractError {
    ContractError::new(
        "APPROVAL_STALE",
        "Unpublished work or review context changed after approval; approve the replacement again",
        "",
    )
}
fn approval_invalid() -> ContractError {
    ContractError::new(
        "APPROVAL_INVALID",
        "This approval is not valid for the requested generated review replacement",
        "",
    )
}
fn publication_recovery_required() -> ContractError {
    ContractError::new(
        "PUBLICATION_RECOVERY_REQUIRED",
        "Resolve the in-flight or uncertain GitHub review operation before replacing this result",
        "",
    )
}
