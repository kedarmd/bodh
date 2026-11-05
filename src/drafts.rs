//! Durable local manual text-comment drafts with optimistic workspace revisions.
use crate::{
    ContractError, DraftCounts, OperationControl, PreparationStore, PreparedBinding, Revision,
    SourceRef, diff_index, read_source,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_DRAFT_CHARS: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftProvenance {
    pub preparation_id: String,
    pub context_id: String,
    pub base_sha: String,
    pub head_sha: String,
    pub merge_base_sha: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManualTextDraft {
    pub id: String,
    pub body: String,
    pub source: SourceRef,
    pub revision: u64,
    pub provenance: DraftProvenance,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftSnapshot {
    pub workspace_revision: u64,
    pub counts: DraftCounts,
    pub drafts: Vec<ManualTextDraft>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftSaveResult {
    pub workspace_revision: u64,
    pub counts: DraftCounts,
    pub draft: ManualTextDraft,
}

/// A local code replacement proposal. It is deliberately stored separately from
/// model-authored suggestions and never applied to the checkout.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodeReplacementDraft {
    pub id: String,
    pub source: SourceRef,
    pub original_text: String,
    pub replacement: String,
    pub revision: u64,
    pub provenance: DraftProvenance,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodeReplacementSnapshot {
    pub workspace_revision: u64,
    pub drafts: Vec<CodeReplacementDraft>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodeReplacementSaveResult {
    pub workspace_revision: u64,
    pub draft: CodeReplacementDraft,
}

pub struct DraftStore {
    db: Connection,
}

impl DraftStore {
    /// Opens the preparation database and applies this module's additive migration.
    /// Draft tables use a namespaced migration ledger so independent workspace
    /// features do not race over SQLite's single `user_version` value.
    pub fn open(path: &Path) -> Result<Self, ContractError> {
        drop(PreparationStore::open(path)?);
        let db = Connection::open(path).map_err(db_error)?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(db_error)?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;")
            .map_err(db_error)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS draft_schema_migrations (
                 version INTEGER PRIMARY KEY NOT NULL
             );",
        )
        .map_err(db_error)?;
        let version: Option<i64> = db
            .query_row(
                "SELECT max(version) FROM draft_schema_migrations",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?
            .flatten();
        match version.unwrap_or(0) {
            0 => {
                db.execute_batch(
                    "BEGIN IMMEDIATE;
                     CREATE TABLE IF NOT EXISTS draft_schema_migrations (
                         version INTEGER PRIMARY KEY NOT NULL
                     );
                     CREATE TABLE manual_text_drafts (
                         draft_id TEXT PRIMARY KEY NOT NULL,
                         context_id TEXT NOT NULL REFERENCES contexts(context_id),
                         host TEXT NOT NULL,
                         owner TEXT NOT NULL,
                         repository TEXT NOT NULL,
                         number INTEGER NOT NULL,
                         body TEXT NOT NULL,
                         source_json TEXT NOT NULL,
                         provenance_json TEXT NOT NULL,
                         revision INTEGER NOT NULL CHECK (revision > 0),
                         created_at INTEGER NOT NULL,
                         updated_at INTEGER NOT NULL,
                         FOREIGN KEY(host, owner, repository, number)
                             REFERENCES workspaces(host, owner, repository, number)
                     );
                     CREATE INDEX manual_text_drafts_context
                         ON manual_text_drafts(host, owner, repository, number, context_id, updated_at);
                     CREATE TABLE code_replacement_drafts (
                         draft_id TEXT PRIMARY KEY NOT NULL,
                         context_id TEXT NOT NULL REFERENCES contexts(context_id),
                         host TEXT NOT NULL, owner TEXT NOT NULL, repository TEXT NOT NULL,
                         number INTEGER NOT NULL, source_json TEXT NOT NULL,
                         original_text TEXT NOT NULL, replacement TEXT NOT NULL,
                         provenance_json TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision > 0),
                         created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                         FOREIGN KEY(host, owner, repository, number)
                             REFERENCES workspaces(host, owner, repository, number)
                     );
                     CREATE INDEX code_replacement_drafts_context
                         ON code_replacement_drafts(host,owner,repository,number,context_id,updated_at);
                     INSERT INTO draft_schema_migrations(version) VALUES(2);
                     COMMIT;",
                )
                .map_err(db_error)?;
            }
            1 => {
                db.execute_batch(
                    "BEGIN IMMEDIATE;
                     CREATE TABLE code_replacement_drafts (
                         draft_id TEXT PRIMARY KEY NOT NULL,
                         context_id TEXT NOT NULL REFERENCES contexts(context_id),
                         host TEXT NOT NULL, owner TEXT NOT NULL, repository TEXT NOT NULL,
                         number INTEGER NOT NULL, source_json TEXT NOT NULL,
                         original_text TEXT NOT NULL, replacement TEXT NOT NULL,
                         provenance_json TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision > 0),
                         created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                         FOREIGN KEY(host, owner, repository, number)
                             REFERENCES workspaces(host, owner, repository, number)
                     );
                     CREATE INDEX code_replacement_drafts_context
                         ON code_replacement_drafts(host,owner,repository,number,context_id,updated_at);
                     INSERT INTO draft_schema_migrations(version) VALUES(2);
                     COMMIT;",
                ).map_err(db_error)?;
            }
            2 => {}
            _ => {
                return Err(ContractError::new(
                    "DRAFT_SCHEMA_UNSUPPORTED",
                    "Draft database schema is newer than this application",
                    "",
                ));
            }
        }
        drop(crate::ReviewPublicationStore::open(path)?);
        Ok(Self { db })
    }

    /// Returns the current content revision and drafts for the immutable prepared context.
    pub fn list(&self, preparation_id: &str) -> Result<DraftSnapshot, ContractError> {
        let prepared = load_binding(&self.db, preparation_id)?;
        let pr = &prepared.context.pr;
        let workspace_revision = workspace_revision(&self.db, pr)?;
        let mut statement = self
            .db
            .prepare(
                "SELECT draft_id, body, source_json, provenance_json, revision, created_at, updated_at
                 FROM manual_text_drafts
                 WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4 AND context_id=?5
                   AND NOT EXISTS (
                       SELECT 1 FROM review_published_drafts p
                       WHERE p.context_id=?6 AND p.draft_id=manual_text_drafts.draft_id
                         AND p.draft_revision=manual_text_drafts.revision
                   )
                 ORDER BY created_at, draft_id",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map(
                params![
                    pr.host,
                    pr.owner,
                    pr.repository,
                    i64::try_from(pr.number).map_err(|_| state_error())?,
                    prepared.context.context_id,
                    prepared.context.context_id
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, i64>(6)?,
                    ))
                },
            )
            .map_err(db_error)?;
        let mut drafts = Vec::new();
        for row in rows {
            let (id, body, source, provenance, revision, created_at, updated_at) =
                row.map_err(db_error)?;
            drafts.push(ManualTextDraft {
                id,
                body,
                source: serde_json::from_str(&source).map_err(json_error)?,
                revision: u64::try_from(revision).map_err(|_| state_error())?,
                provenance: serde_json::from_str(&provenance).map_err(json_error)?,
                created_at,
                updated_at,
            });
        }
        let counts = draft_counts(drafts.len());
        Ok(DraftSnapshot {
            workspace_revision,
            counts,
            drafts,
        })
    }

    /// Lists human-authored replacement proposals for this pinned context.
    pub fn list_replacements(
        &self,
        preparation_id: &str,
    ) -> Result<CodeReplacementSnapshot, ContractError> {
        let prepared = load_binding(&self.db, preparation_id)?;
        let pr = &prepared.context.pr;
        let mut statement = self.db.prepare(
            "SELECT draft_id,source_json,original_text,replacement,provenance_json,revision,created_at,updated_at
             FROM code_replacement_drafts WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4 AND context_id=?5
             ORDER BY created_at,draft_id"
        ).map_err(db_error)?;
        let rows = statement
            .query_map(
                params![
                    pr.host,
                    pr.owner,
                    pr.repository,
                    i64::try_from(pr.number).map_err(|_| state_error())?,
                    prepared.context.context_id
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, i64>(6)?,
                        r.get::<_, i64>(7)?,
                    ))
                },
            )
            .map_err(db_error)?;
        let mut drafts = Vec::new();
        for row in rows {
            let (
                id,
                source,
                original_text,
                replacement,
                provenance,
                revision,
                created_at,
                updated_at,
            ) = row.map_err(db_error)?;
            drafts.push(CodeReplacementDraft {
                id,
                source: serde_json::from_str(&source).map_err(json_error)?,
                original_text,
                replacement,
                revision: u64::try_from(revision).map_err(|_| state_error())?,
                provenance: serde_json::from_str(&provenance).map_err(json_error)?,
                created_at,
                updated_at,
            });
        }
        Ok(CodeReplacementSnapshot {
            workspace_revision: workspace_revision(&self.db, pr)?,
            drafts,
        })
    }

    /// Saves an editable replacement after checking it still names exact pinned head source.
    #[allow(clippy::too_many_arguments)]
    pub fn save_replacement(
        &mut self,
        preparation_id: &str,
        draft_id: Option<&str>,
        source: &SourceRef,
        original_text: &str,
        replacement: &str,
        expected_workspace_revision: u64,
        expected_draft_revision: u64,
        control: &OperationControl,
    ) -> Result<CodeReplacementSaveResult, ContractError> {
        if source.revision != Revision::Head {
            return Err(ContractError::new(
                "REPLACEMENT_HEAD_ONLY",
                "Code replacements can target only the pinned head side",
                "/source/revision",
            ));
        }
        validate_replacement_text(replacement)?;
        if original_text.is_empty() || original_text.chars().count() > MAX_DRAFT_CHARS {
            return Err(ContractError::new(
                "REPLACEMENT_SOURCE_INVALID",
                "Original source must be nonempty and no longer than 65,536 characters",
                "/original_text",
            ));
        }
        let prepared = load_binding(&self.db, preparation_id)?;
        let index = diff_index(Path::new(&prepared.checkout), &prepared.context, control)?;
        index.validate_anchor(source)?;
        let exact = read_source(
            Path::new(&prepared.checkout),
            &prepared.context,
            source,
            control,
        )?;
        if exact != original_text {
            return Err(ContractError::new(
                "REPLACEMENT_SOURCE_CHANGED",
                "The selected text differs from the exact pinned head source; reload the source before saving",
                "/original_text",
            ));
        }
        control.check()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let pr = &prepared.context.pr;
        let current = workspace_revision(&tx, pr)?;
        require_revision(
            current,
            expected_workspace_revision,
            "WORKSPACE_REVISION_STALE",
            "Workspace content changed; reload before saving this replacement",
        )?;
        let provenance = DraftProvenance {
            preparation_id: preparation_id.into(),
            context_id: prepared.context.context_id.clone(),
            base_sha: prepared.context.base_sha.clone(),
            head_sha: prepared.context.head_sha.clone(),
            merge_base_sha: prepared.context.merge_base_sha.clone(),
        };
        let source_json = serde_json::to_string(source).map_err(json_error)?;
        let provenance_json = serde_json::to_string(&provenance).map_err(json_error)?;
        let now = unix_millis();
        let (id, revision, created_at) = if let Some(id) = draft_id {
            let row: Option<(String,i64,i64)> = tx.query_row("SELECT context_id,revision,created_at FROM code_replacement_drafts WHERE draft_id=?1 AND host=?2 AND owner=?3 AND repository=?4 AND number=?5", params![id,pr.host,pr.owner,pr.repository,i64::try_from(pr.number).map_err(|_| state_error())?], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(db_error)?;
            let Some((context, old_revision, created)) = row else {
                return Err(draft_not_found());
            };
            if context != prepared.context.context_id {
                return Err(draft_not_found());
            }
            require_revision(
                u64::try_from(old_revision).map_err(|_| state_error())?,
                expected_draft_revision,
                "DRAFT_REVISION_STALE",
                "Replacement changed elsewhere; reload it before saving",
            )?;
            let next = old_revision.checked_add(1).ok_or_else(state_error)?;
            tx.execute("UPDATE code_replacement_drafts SET source_json=?1,original_text=?2,replacement=?3,provenance_json=?4,revision=?5,updated_at=?6 WHERE draft_id=?7", params![source_json,original_text,replacement,provenance_json,next,now,id]).map_err(db_error)?;
            (id.to_owned(), next, created)
        } else {
            if expected_draft_revision != 0 {
                return Err(ContractError::new(
                    "DRAFT_REVISION_STALE",
                    "New drafts must start at revision zero",
                    "/expected_draft_revision",
                ));
            }
            let id = new_id();
            tx.execute("INSERT INTO code_replacement_drafts(draft_id,context_id,host,owner,repository,number,source_json,original_text,replacement,provenance_json,revision,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,1,?11,?11)", params![id,prepared.context.context_id,pr.host,pr.owner,pr.repository,i64::try_from(pr.number).map_err(|_| state_error())?,source_json,original_text,replacement,provenance_json,now]).map_err(db_error)?;
            (id, 1, now)
        };
        let workspace_revision = advance_workspace(&tx, pr, current)?;
        tx.commit().map_err(db_error)?;
        Ok(CodeReplacementSaveResult {
            workspace_revision,
            draft: CodeReplacementDraft {
                id,
                source: source.clone(),
                original_text: original_text.into(),
                replacement: replacement.into(),
                revision: u64::try_from(revision).map_err(|_| state_error())?,
                provenance,
                created_at,
                updated_at: now,
            },
        })
    }

    /// Explicitly removes one replacement draft using optimistic revisions.
    pub fn discard_replacement(
        &mut self,
        preparation_id: &str,
        draft_id: &str,
        expected_workspace_revision: u64,
        expected_draft_revision: u64,
    ) -> Result<CodeReplacementSnapshot, ContractError> {
        let prepared = load_binding(&self.db, preparation_id)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let pr = &prepared.context.pr;
        let current = workspace_revision(&tx, pr)?;
        require_revision(
            current,
            expected_workspace_revision,
            "WORKSPACE_REVISION_STALE",
            "Workspace content changed; reload before discarding this replacement",
        )?;
        let row: Option<(String,i64)> = tx.query_row("SELECT context_id,revision FROM code_replacement_drafts WHERE draft_id=?1 AND host=?2 AND owner=?3 AND repository=?4 AND number=?5", params![draft_id,pr.host,pr.owner,pr.repository,i64::try_from(pr.number).map_err(|_| state_error())?], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)?;
        let Some((context, revision)) = row else {
            return Err(draft_not_found());
        };
        if context != prepared.context.context_id {
            return Err(draft_not_found());
        }
        require_revision(
            u64::try_from(revision).map_err(|_| state_error())?,
            expected_draft_revision,
            "DRAFT_REVISION_STALE",
            "Replacement changed elsewhere; reload it before discarding",
        )?;
        tx.execute(
            "DELETE FROM code_replacement_drafts WHERE draft_id=?1",
            [draft_id],
        )
        .map_err(db_error)?;
        advance_workspace(&tx, pr, current)?;
        tx.commit().map_err(db_error)?;
        self.list_replacements(preparation_id)
    }

    /// Validates the exact pinned diff anchor before a short optimistic write transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn save(
        &mut self,
        preparation_id: &str,
        draft_id: Option<&str>,
        body: &str,
        source: &SourceRef,
        expected_workspace_revision: u64,
        expected_draft_revision: u64,
        control: &OperationControl,
    ) -> Result<DraftSaveResult, ContractError> {
        validate_body(body)?;
        let prepared = load_binding(&self.db, preparation_id)?;
        diff_index(Path::new(&prepared.checkout), &prepared.context, control)?
            .validate_anchor(source)?;
        control.check()?;

        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let pr = &prepared.context.pr;
        let current_workspace = workspace_revision(&tx, pr)?;
        require_revision(
            current_workspace,
            expected_workspace_revision,
            "WORKSPACE_REVISION_STALE",
            "Workspace content changed; reload before saving this draft",
        )?;
        let now = unix_millis();
        let provenance = DraftProvenance {
            preparation_id: preparation_id.to_owned(),
            context_id: prepared.context.context_id.clone(),
            base_sha: prepared.context.base_sha.clone(),
            head_sha: prepared.context.head_sha.clone(),
            merge_base_sha: prepared.context.merge_base_sha.clone(),
        };
        let source_json = serde_json::to_string(source).map_err(json_error)?;
        let provenance_json = serde_json::to_string(&provenance).map_err(json_error)?;
        let (id, revision, created_at) = if let Some(id) = draft_id {
            let old: Option<(String, i64, i64)> = tx
                .query_row(
                    "SELECT context_id, revision, created_at FROM manual_text_drafts
                     WHERE draft_id=?1 AND host=?2 AND owner=?3 AND repository=?4 AND number=?5",
                    params![
                        id,
                        pr.host,
                        pr.owner,
                        pr.repository,
                        i64::try_from(pr.number).map_err(|_| state_error())?
                    ],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(db_error)?;
            let Some((context_id, revision, created_at)) = old else {
                return Err(draft_not_found());
            };
            if context_id != prepared.context.context_id {
                return Err(ContractError::new(
                    "DRAFT_CONTEXT_MISMATCH",
                    "Draft belongs to a different pinned review context",
                    "/draft_id",
                ));
            }
            require_revision(
                u64::try_from(revision).map_err(|_| state_error())?,
                expected_draft_revision,
                "DRAFT_REVISION_STALE",
                "Draft changed elsewhere; reload it before saving",
            )?;
            let next = revision.checked_add(1).ok_or_else(state_error)?;
            tx.execute(
                "UPDATE manual_text_drafts SET body=?1, source_json=?2, provenance_json=?3,
                     revision=?4, updated_at=?5 WHERE draft_id=?6",
                params![body, source_json, provenance_json, next, now, id],
            )
            .map_err(db_error)?;
            (id.to_owned(), next, created_at)
        } else {
            if expected_draft_revision != 0 {
                return Err(ContractError::new(
                    "DRAFT_REVISION_STALE",
                    "New drafts must start at revision zero",
                    "/expected_draft_revision",
                ));
            }
            let id = new_id();
            tx.execute(
                "INSERT INTO manual_text_drafts(
                     draft_id, context_id, host, owner, repository, number, body,
                     source_json, provenance_json, revision, created_at, updated_at
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10,?10)",
                params![
                    id,
                    prepared.context.context_id,
                    pr.host,
                    pr.owner,
                    pr.repository,
                    i64::try_from(pr.number).map_err(|_| state_error())?,
                    body,
                    source_json,
                    provenance_json,
                    now
                ],
            )
            .map_err(db_error)?;
            (id, 1, now)
        };
        let workspace_revision = advance_workspace(&tx, pr, current_workspace)?;
        let drafts = count_drafts(&tx, pr, &prepared.context.context_id)?;
        tx.commit().map_err(db_error)?;
        Ok(DraftSaveResult {
            workspace_revision,
            counts: draft_counts(drafts),
            draft: ManualTextDraft {
                id,
                body: body.to_owned(),
                source: source.clone(),
                revision: u64::try_from(revision).map_err(|_| state_error())?,
                provenance,
                created_at,
                updated_at: now,
            },
        })
    }

    /// Explicitly discards one draft if both workspace and draft revisions still match.
    pub fn discard(
        &mut self,
        preparation_id: &str,
        draft_id: &str,
        expected_workspace_revision: u64,
        expected_draft_revision: u64,
    ) -> Result<DraftSnapshot, ContractError> {
        let prepared = load_binding(&self.db, preparation_id)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let pr = &prepared.context.pr;
        let current_workspace = workspace_revision(&tx, pr)?;
        require_revision(
            current_workspace,
            expected_workspace_revision,
            "WORKSPACE_REVISION_STALE",
            "Workspace content changed; reload before discarding this draft",
        )?;
        let current_draft: Option<(String, i64)> = tx
            .query_row(
                "SELECT context_id, revision FROM manual_text_drafts
                 WHERE draft_id=?1 AND host=?2 AND owner=?3 AND repository=?4 AND number=?5",
                params![
                    draft_id,
                    pr.host,
                    pr.owner,
                    pr.repository,
                    i64::try_from(pr.number).map_err(|_| state_error())?
                ],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        let Some((context_id, revision)) = current_draft else {
            return Err(draft_not_found());
        };
        if context_id != prepared.context.context_id {
            return Err(draft_not_found());
        }
        require_revision(
            u64::try_from(revision).map_err(|_| state_error())?,
            expected_draft_revision,
            "DRAFT_REVISION_STALE",
            "Draft changed elsewhere; reload it before discarding",
        )?;
        tx.execute(
            "DELETE FROM manual_text_drafts WHERE draft_id=?1",
            [draft_id],
        )
        .map_err(db_error)?;
        let _workspace_revision = advance_workspace(&tx, pr, current_workspace)?;
        let _drafts = count_drafts(&tx, pr, &prepared.context.context_id)?;
        tx.commit().map_err(db_error)?;
        self.list(preparation_id)
    }
}

fn load_binding(db: &Connection, id: &str) -> Result<PreparedBinding, ContractError> {
    let row: Option<(String, String, u64, String, Option<String>, String)> = db
        .query_row(
            "SELECT p.context_id,c.context_json,p.expected_generation,p.title,p.description,p.checkout
             FROM preparations p JOIN contexts c ON c.context_id=p.context_id
             WHERE p.preparation_id=?1",
            [id],
            |r| {
                let generation: i64 = r.get(2)?;
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    u64::try_from(generation).unwrap_or(u64::MAX),
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
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
            |r| r.get(0),
        )
        .map_err(db_error)?;
    Ok(PreparedBinding {
        preparation_id: id.into(),
        expected_generation,
        context: serde_json::from_str(&context_json).map_err(json_error)?,
        title,
        description,
        changed_files: serde_json::from_str(&files_json).map_err(json_error)?,
        checkout,
    })
}

fn workspace_revision(db: &Connection, pr: &crate::PrIdentity) -> Result<u64, ContractError> {
    let generation: i64 = db
        .query_row(
            "SELECT generation FROM workspaces WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4",
            params![pr.host,pr.owner,pr.repository,i64::try_from(pr.number).map_err(|_| state_error())?],
            |r| r.get(0),
        )
        .map_err(db_error)?;
    u64::try_from(generation).map_err(|_| state_error())
}

fn advance_workspace(
    tx: &Transaction<'_>,
    pr: &crate::PrIdentity,
    current: u64,
) -> Result<u64, ContractError> {
    let next = current.checked_add(1).ok_or_else(state_error)?;
    let changed = tx
        .execute(
            "UPDATE workspaces SET generation=?1
             WHERE host=?2 AND owner=?3 AND repository=?4 AND number=?5 AND generation=?6",
            params![
                i64::try_from(next).map_err(|_| state_error())?,
                pr.host,
                pr.owner,
                pr.repository,
                i64::try_from(pr.number).map_err(|_| state_error())?,
                i64::try_from(current).map_err(|_| state_error())?
            ],
        )
        .map_err(db_error)?;
    if changed != 1 {
        return Err(stale_workspace());
    }
    Ok(next)
}

fn count_drafts(
    tx: &Transaction<'_>,
    pr: &crate::PrIdentity,
    context_id: &str,
) -> Result<usize, ContractError> {
    tx.query_row(
        "SELECT count(*) FROM manual_text_drafts d
         WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4 AND context_id=?5
           AND NOT EXISTS (SELECT 1 FROM review_published_drafts p
               WHERE p.context_id=?6 AND p.draft_id=d.draft_id AND p.draft_revision=d.revision)",
        params![
            pr.host,
            pr.owner,
            pr.repository,
            i64::try_from(pr.number).map_err(|_| state_error())?,
            context_id,
            context_id
        ],
        |r| r.get::<_, i64>(0),
    )
    .map_err(db_error)
    .and_then(|count| usize::try_from(count).map_err(|_| state_error()))
}

fn draft_counts(inline_comments: usize) -> DraftCounts {
    DraftCounts {
        inline_comments: u64::try_from(inline_comments).unwrap_or(u64::MAX),
        replies: 0,
        thread_intents: 0,
        review_comments: 0,
    }
}

fn validate_body(body: &str) -> Result<(), ContractError> {
    if body.trim().is_empty() || body.chars().count() > MAX_DRAFT_CHARS {
        return Err(ContractError::new(
            "DRAFT_BODY_INVALID",
            "Draft text must be nonempty and no longer than 65,536 characters",
            "/body",
        ));
    }
    Ok(())
}

fn validate_replacement_text(text: &str) -> Result<(), ContractError> {
    if text.chars().count() > MAX_DRAFT_CHARS {
        return Err(ContractError::new(
            "REPLACEMENT_TEXT_INVALID",
            "Replacement text must be no longer than 65,536 characters",
            "/replacement",
        ));
    }
    Ok(())
}

fn require_revision(
    actual: u64,
    expected: u64,
    code: &str,
    message: &str,
) -> Result<(), ContractError> {
    if actual == expected {
        Ok(())
    } else {
        Err(ContractError::new(code, message, ""))
    }
}

fn draft_not_found() -> ContractError {
    ContractError::new(
        "DRAFT_NOT_FOUND",
        "Draft was not found in this review context",
        "",
    )
}

fn stale_workspace() -> ContractError {
    ContractError::new(
        "WORKSPACE_REVISION_STALE",
        "Workspace content changed; reload before saving this draft",
        "",
    )
}

fn new_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "d-{:x}-{:x}-{:x}",
        std::process::id(),
        time,
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn db_error(_: rusqlite::Error) -> ContractError {
    ContractError::new(
        "STATE_DATABASE_ERROR",
        "Cannot read or write review state",
        "",
    )
}

fn json_error(_: serde_json::Error) -> ContractError {
    ContractError::new("STATE_DATABASE_ERROR", "Stored review state is invalid", "")
}

fn state_error() -> ContractError {
    ContractError::new(
        "STATE_DATABASE_ERROR",
        "Review state revision is invalid",
        "",
    )
}
