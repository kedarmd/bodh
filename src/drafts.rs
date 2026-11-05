//! Durable local manual text-comment drafts with optimistic workspace revisions.
use crate::{
    ContractError, DraftCounts, OperationControl, PreparationStore, PreparedBinding, Revision,
    SourceRef, diff_index, load_latest_discussions, read_source,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

fn require_same_pull_request(
    source: &PreparedBinding,
    candidate: &PreparedBinding,
) -> Result<(), ContractError> {
    let old = &source.context.pr;
    let new = &candidate.context.pr;
    if old.host != new.host
        || old.owner != new.owner
        || old.repository != new.repository
        || old.number != new.number
    {
        return Err(anchor_repair_invalid(
            "Draft repair must stay within the same pull request",
        ));
    }
    Ok(())
}

fn locate_exact_anchor(
    checkout: &Path,
    context: &crate::ReviewContext,
    old_source: &SourceRef,
    exact_text: &str,
    index: &crate::DiffIndex,
    control: &OperationControl,
) -> Result<(AnchorRepairStatus, Option<SourceRef>, String), ContractError> {
    if exact_text.is_empty() {
        return Ok((
            AnchorRepairStatus::Changed,
            None,
            "The original anchor has no source text to compare.".into(),
        ));
    }
    if index.validate_anchor(old_source).is_ok()
        && read_source(checkout, context, old_source, control).is_ok_and(|text| text == exact_text)
    {
        return Ok((
            AnchorRepairStatus::Current,
            Some(old_source.clone()),
            "The exact source still occupies its original eligible lines.".into(),
        ));
    }

    let needle = exact_text.split_inclusive('\n').collect::<Vec<_>>();
    let mut mapped_files = Vec::new();
    let mut changed_files = Vec::new();
    let mut saw_removed = false;
    let mut unsupported = false;
    for file in &index.files {
        let matches_path = file.old_path.as_deref() == Some(old_source.path.as_str())
            || file.new_path.as_deref() == Some(old_source.path.as_str());
        let path = match old_source.revision {
            Revision::Base => file.old_path.as_deref(),
            Revision::Head => file.new_path.as_deref(),
        };
        let Some(path) = path else {
            saw_removed |= matches_path;
            continue;
        };
        if file.unsupported_reason.is_some()
            || file.hunks.iter().all(|hunk| match old_source.revision {
                Revision::Base => hunk.old_lines == 0,
                Revision::Head => hunk.new_lines == 0,
            })
        {
            if matches_path && file.unsupported_reason.is_some() {
                unsupported = true;
            }
            continue;
        }
        let candidate = (file, path.to_owned());
        changed_files.push(candidate.clone());
        if matches_path {
            mapped_files.push(candidate);
        }
    }
    // Git does not always identify a move as a rename when enough surrounding
    // text changed. In that case search all eligible changed ranges, still
    // requiring a unique exact match and explicit reviewer confirmation.
    let matching_files = if mapped_files.is_empty() {
        changed_files
    } else {
        mapped_files
    };

    let mut blobs: HashMap<String, Vec<String>> = HashMap::new();
    let mut proposed = Vec::new();
    for (file, path) in matching_files {
        if file.unsupported_reason.is_some() {
            unsupported = true;
            continue;
        }
        let (oid, path_is_present) = match old_source.revision {
            Revision::Base => (&file.old_oid, file.old_path.is_some()),
            Revision::Head => (&file.new_oid, file.new_path.is_some()),
        };
        if !path_is_present {
            saw_removed = true;
            continue;
        }
        if !blobs.contains_key(oid) {
            let bytes = crate::evidence::git(checkout, control, &["cat-file", "blob", oid])?;
            let Ok(text) = std::str::from_utf8(&bytes) else {
                unsupported = true;
                continue;
            };
            blobs.insert(
                oid.clone(),
                text.split_inclusive('\n').map(str::to_owned).collect(),
            );
        }
        let Some(lines) = blobs.get(oid) else {
            unsupported = true;
            continue;
        };
        for hunk in &file.hunks {
            control.check()?;
            let (start, count) = match old_source.revision {
                Revision::Base => (hunk.old_start, hunk.old_lines),
                Revision::Head => (hunk.new_start, hunk.new_lines),
            };
            if start == 0 || count == 0 || count < needle.len() as u32 {
                continue;
            }
            let start_index = usize::try_from(start - 1).map_err(|_| state_error())?;
            let end_index = start_index
                .checked_add(usize::try_from(count).map_err(|_| state_error())?)
                .ok_or_else(state_error)?;
            if end_index > lines.len() {
                return Err(anchor_repair_invalid(
                    "The candidate diff hunk exceeds its pinned source blob",
                ));
            }
            for offset in 0..=usize::try_from(count).map_err(|_| state_error())? - needle.len() {
                if lines[start_index + offset..start_index + offset + needle.len()]
                    .iter()
                    .map(String::as_str)
                    .eq(needle.iter().copied())
                {
                    let first =
                        u32::try_from(start_index + offset + 1).map_err(|_| state_error())?;
                    let last = first
                        .checked_add(u32::try_from(needle.len() - 1).map_err(|_| state_error())?)
                        .ok_or_else(state_error)?;
                    proposed.push(SourceRef {
                        path: path.clone(),
                        revision: old_source.revision,
                        start_line: first,
                        end_line: last,
                    });
                    if proposed.len() > 1 {
                        return Ok((
                            AnchorRepairStatus::Ambiguous,
                            None,
                            "The exact source occurs in multiple eligible ranges; choose a location manually.".into(),
                        ));
                    }
                }
            }
        }
    }
    if let Some(source) = proposed.pop() {
        return Ok((
            AnchorRepairStatus::RelocationProposed,
            Some(source),
            "The exact text has one eligible new location. Confirm the proposed relocation after reviewing its context.".into(),
        ));
    }
    if unsupported {
        return Ok((
            AnchorRepairStatus::Unsupported,
            None,
            "The candidate file has content that cannot be safely searched for source anchors."
                .into(),
        ));
    }
    if saw_removed {
        return Ok((
            AnchorRepairStatus::Removed,
            None,
            "The file was removed on the selected review side.".into(),
        ));
    }
    Ok((
        AnchorRepairStatus::Changed,
        None,
        "The exact source is absent from eligible changed lines. Review and attach a new location manually.".into(),
    ))
}

fn anchor_repair_invalid(message: &str) -> ContractError {
    ContractError::new("ANCHOR_REPAIR_INVALID", message, "")
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
    pub reply_drafts: Vec<ReplyDraft>,
    pub thread_actions: Vec<ThreadActionDraft>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftSaveResult {
    pub workspace_revision: u64,
    pub counts: DraftCounts,
    pub draft: ManualTextDraft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadAction {
    Resolve,
    Reopen,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyDraft {
    pub id: String,
    pub thread_id: String,
    pub root_comment_id: String,
    pub body: String,
    pub revision: u64,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadActionDraft {
    pub id: String,
    pub thread_id: String,
    pub action: ThreadAction,
    pub observed_resolved: bool,
    pub revision: u64,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplySaveResult {
    pub workspace_revision: u64,
    pub counts: DraftCounts,
    pub reply: ReplyDraft,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThreadActionSaveResult {
    pub workspace_revision: u64,
    pub counts: DraftCounts,
    pub action: ThreadActionDraft,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorRepairStatus {
    Current,
    RelocationProposed,
    Reattached,
    Changed,
    Removed,
    Unsupported,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorRepairPreview {
    pub draft_id: String,
    pub draft_revision: u64,
    pub kind: String,
    pub source: SourceRef,
    pub status: AnchorRepairStatus,
    pub proposed_source: Option<SourceRef>,
    pub reattached_draft_id: Option<String>,
    pub exact_text: Option<String>,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodeReplacementSaveResult {
    pub workspace_revision: u64,
    pub draft: CodeReplacementDraft,
}

pub struct DraftStore {
    db: Connection,
    path: std::path::PathBuf,
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
                     INSERT INTO draft_schema_migrations(version) VALUES(1);
                     COMMIT;",
                )
                .map_err(db_error)?;
            }
            1 | 2 => {}
            _ => {
                return Err(ContractError::new(
                    "DRAFT_SCHEMA_UNSUPPORTED",
                    "Draft database schema is newer than this application",
                    "",
                ));
            }
        }
        db.execute_batch("BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS code_replacement_drafts (draft_id TEXT PRIMARY KEY NOT NULL, context_id TEXT NOT NULL REFERENCES contexts(context_id), host TEXT NOT NULL, owner TEXT NOT NULL, repository TEXT NOT NULL, number INTEGER NOT NULL, source_json TEXT NOT NULL, original_text TEXT NOT NULL, replacement TEXT NOT NULL, provenance_json TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision>0), created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, FOREIGN KEY(host,owner,repository,number) REFERENCES workspaces(host,owner,repository,number));
            CREATE INDEX IF NOT EXISTS code_replacement_drafts_context ON code_replacement_drafts(host,owner,repository,number,context_id,updated_at);
            CREATE TABLE IF NOT EXISTS reply_drafts (draft_id TEXT PRIMARY KEY NOT NULL, context_id TEXT NOT NULL REFERENCES contexts(context_id), host TEXT NOT NULL, owner TEXT NOT NULL, repository TEXT NOT NULL, number INTEGER NOT NULL, thread_id TEXT NOT NULL, root_comment_id TEXT NOT NULL, body TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision>0), created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, FOREIGN KEY(host,owner,repository,number) REFERENCES workspaces(host,owner,repository,number));
            CREATE INDEX IF NOT EXISTS reply_drafts_workspace ON reply_drafts(host,owner,repository,number,context_id,updated_at);
            CREATE TABLE IF NOT EXISTS thread_action_drafts (draft_id TEXT PRIMARY KEY NOT NULL, context_id TEXT NOT NULL REFERENCES contexts(context_id), host TEXT NOT NULL, owner TEXT NOT NULL, repository TEXT NOT NULL, number INTEGER NOT NULL, thread_id TEXT NOT NULL, action TEXT NOT NULL CHECK(action IN ('resolve','reopen')), observed_resolved INTEGER NOT NULL, revision INTEGER NOT NULL CHECK(revision>0), created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, FOREIGN KEY(host,owner,repository,number) REFERENCES workspaces(host,owner,repository,number));
            CREATE INDEX IF NOT EXISTS thread_actions_workspace ON thread_action_drafts(host,owner,repository,number,context_id,updated_at);
            INSERT OR IGNORE INTO draft_schema_migrations(version) VALUES(2); COMMIT;").map_err(db_error)?;
        drop(crate::ReviewPublicationStore::open(path)?);
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS draft_anchor_repairs(
                source_context_id TEXT NOT NULL,
                source_draft_id TEXT NOT NULL,
                source_revision INTEGER NOT NULL,
                candidate_context_id TEXT NOT NULL,
                candidate_draft_id TEXT NOT NULL,
                kind TEXT NOT NULL CHECK(kind IN ('text_comment','code_replacement')),
                created_at INTEGER NOT NULL,
                PRIMARY KEY(source_context_id,source_draft_id,source_revision,candidate_context_id)
            );",
        )
        .map_err(db_error)?;
        Ok(Self {
            db,
            path: path.to_path_buf(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_reply(
        &mut self,
        preparation_id: &str,
        draft_id: Option<&str>,
        thread_id: &str,
        root_comment_id: &str,
        body: &str,
        expected_workspace_revision: u64,
        expected_draft_revision: u64,
    ) -> Result<ReplySaveResult, ContractError> {
        validate_body(body)?;
        let _prepared = load_binding(&self.db, preparation_id)?;
        let discussions = load_latest_discussions(&self.path, preparation_id)?;
        let thread = find_thread(&discussions, thread_id)?;
        if thread["viewerCanReply"] != true || !root_comment_exists(&thread, root_comment_id) {
            return Err(ContractError::new(
                "REPLY_NOT_ALLOWED",
                "Reply target is unavailable or reply permission was not observed",
                "/thread_id",
            ));
        }
        self.save_discussion_draft(
            preparation_id,
            draft_id,
            thread_id,
            Some(root_comment_id),
            body,
            None,
            expected_workspace_revision,
            expected_draft_revision,
        )
    }

    pub fn stage_thread_action(
        &mut self,
        preparation_id: &str,
        draft_id: Option<&str>,
        thread_id: &str,
        action: ThreadAction,
        expected_workspace_revision: u64,
        expected_draft_revision: u64,
    ) -> Result<ThreadActionSaveResult, ContractError> {
        let prepared = load_binding(&self.db, preparation_id)?;
        let discussions = load_latest_discussions(&self.path, preparation_id)?;
        let thread = find_thread(&discussions, thread_id)?;
        let allowed = match action {
            ThreadAction::Resolve => {
                thread["viewerCanResolve"] == true && thread["isResolved"] == false
            }
            ThreadAction::Reopen => {
                thread["viewerCanUnresolve"] == true && thread["isResolved"] == true
            }
        };
        if !allowed {
            return Err(ContractError::new(
                "THREAD_ACTION_NOT_ALLOWED",
                "Thread action is unavailable or permission was not observed",
                "/action",
            ));
        }
        let observed_resolved = thread["isResolved"].as_bool().unwrap_or(false);
        let (workspace_revision, counts, row, _) = self.save_discussion_draft_inner(
            &prepared,
            draft_id,
            thread_id,
            None,
            "",
            Some((action, observed_resolved)),
            expected_workspace_revision,
            expected_draft_revision,
        )?;
        Ok(ThreadActionSaveResult {
            workspace_revision,
            counts,
            action: row.expect("thread action saved"),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn save_discussion_draft(
        &mut self,
        preparation_id: &str,
        draft_id: Option<&str>,
        thread_id: &str,
        root_comment_id: Option<&str>,
        body: &str,
        action: Option<(ThreadAction, bool)>,
        expected_workspace_revision: u64,
        expected_draft_revision: u64,
    ) -> Result<ReplySaveResult, ContractError> {
        let prepared = load_binding(&self.db, preparation_id)?;
        let (workspace_revision, counts, _, id) = self.save_discussion_draft_inner(
            &prepared,
            draft_id,
            thread_id,
            root_comment_id,
            body,
            action,
            expected_workspace_revision,
            expected_draft_revision,
        )?;
        let reply = self
            .list(preparation_id)?
            .reply_drafts
            .into_iter()
            .find(|item| item.id == id)
            .ok_or_else(draft_not_found)?;
        Ok(ReplySaveResult {
            workspace_revision,
            counts,
            reply,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn save_discussion_draft_inner(
        &mut self,
        prepared: &PreparedBinding,
        draft_id: Option<&str>,
        thread_id: &str,
        root_comment_id: Option<&str>,
        body: &str,
        action: Option<(ThreadAction, bool)>,
        expected_workspace_revision: u64,
        expected_draft_revision: u64,
    ) -> Result<(u64, DraftCounts, Option<ThreadActionDraft>, String), ContractError> {
        let pr = &prepared.context.pr;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let current = workspace_revision(&tx, pr)?;
        require_revision(
            current,
            expected_workspace_revision,
            "WORKSPACE_REVISION_STALE",
            "Workspace content changed; reload before saving this draft",
        )?;
        let now = unix_millis();
        let id = draft_id.map(str::to_owned).unwrap_or_else(new_id);
        if let Some((action, observed)) = action {
            let action_text = match action {
                ThreadAction::Resolve => "resolve",
                ThreadAction::Reopen => "reopen",
            };
            let old:Option<(String,String,i64,i64)>=tx.query_row("SELECT context_id,thread_id,revision,created_at FROM thread_action_drafts WHERE draft_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(db_error)?;
            if draft_id.is_some() && old.is_none() {
                return Err(draft_not_found());
            }
            let (revision, created) = if let Some((context, old_thread, rev, created)) = old {
                if context != prepared.context.context_id {
                    return Err(draft_not_found());
                }
                if old_thread != thread_id {
                    return Err(ContractError::new(
                        "DRAFT_CONTEXT_MISMATCH",
                        "Thread intent belongs to a different thread",
                        "/thread_id",
                    ));
                }
                require_revision(
                    rev as u64,
                    expected_draft_revision,
                    "DRAFT_REVISION_STALE",
                    "Draft changed elsewhere; reload it before saving",
                )?;
                let n = rev + 1;
                tx.execute("UPDATE thread_action_drafts SET action=?1,observed_resolved=?2,revision=?3,updated_at=?4 WHERE draft_id=?5",params![action_text,observed as i64,n,now,id]).map_err(db_error)?;
                (n, created)
            } else {
                if expected_draft_revision != 0 {
                    return Err(ContractError::new(
                        "DRAFT_REVISION_STALE",
                        "New drafts must start at revision zero",
                        "/expected_draft_revision",
                    ));
                }
                tx.execute("INSERT INTO thread_action_drafts(draft_id,context_id,host,owner,repository,number,thread_id,action,observed_resolved,revision,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10,?10)",params![id,prepared.context.context_id,pr.host,pr.owner,pr.repository,pr.number as i64,thread_id,action_text,observed as i64,now]).map_err(db_error)?;
                (1, now)
            };
            let next = advance_workspace(&tx, pr, current)?;
            let counts = count_all(
                &tx,
                pr,
                &prepared.context.context_id,
                &prepared.preparation_id,
            )?;
            tx.commit().map_err(db_error)?;
            Ok((
                next,
                counts,
                Some(ThreadActionDraft {
                    id: id.clone(),
                    thread_id: thread_id.into(),
                    action,
                    observed_resolved: observed,
                    revision: revision as u64,
                    created_at: created,
                    updated_at: now,
                }),
                id,
            ))
        } else {
            let root = root_comment_id.unwrap_or_default();
            let old: Option<(String, String, String, i64, i64)> = if let Some(id) = draft_id {
                tx.query_row("SELECT context_id,thread_id,root_comment_id,revision,created_at FROM reply_drafts WHERE draft_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(db_error)?
            } else {
                None
            };
            if draft_id.is_some() && old.is_none() {
                return Err(draft_not_found());
            }
            let (id, revision, created) = if let Some((
                context,
                old_thread,
                old_root,
                rev,
                created,
            )) = old
            {
                if context != prepared.context.context_id {
                    return Err(draft_not_found());
                }
                if old_thread != thread_id || old_root != root {
                    return Err(ContractError::new(
                        "REPLY_TARGET_MISMATCH",
                        "Reply draft is bound to a different thread root",
                        "/root_comment_id",
                    ));
                }
                require_revision(
                    rev as u64,
                    expected_draft_revision,
                    "DRAFT_REVISION_STALE",
                    "Draft changed elsewhere; reload it before saving",
                )?;
                let n = rev + 1;
                tx.execute(
                    "UPDATE reply_drafts SET body=?1,revision=?2,updated_at=?3 WHERE draft_id=?4",
                    params![body, n, now, draft_id],
                )
                .map_err(db_error)?;
                (draft_id.unwrap().into(), n, created)
            } else {
                if expected_draft_revision != 0 {
                    return Err(ContractError::new(
                        "DRAFT_REVISION_STALE",
                        "New drafts must start at revision zero",
                        "/expected_draft_revision",
                    ));
                }
                let id = new_id();
                tx.execute("INSERT INTO reply_drafts(draft_id,context_id,host,owner,repository,number,thread_id,root_comment_id,body,revision,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10,?10)",params![id,prepared.context.context_id,pr.host,pr.owner,pr.repository,pr.number as i64,thread_id,root,body,now]).map_err(db_error)?;
                (id, 1, now)
            };
            let next = advance_workspace(&tx, pr, current)?;
            let counts = count_all(
                &tx,
                pr,
                &prepared.context.context_id,
                &prepared.preparation_id,
            )?;
            tx.commit().map_err(db_error)?;
            let _ = (revision, created);
            Ok((next, counts, None, id))
        }
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
        let reply_query = if table_exists(&self.db, "discussion_published_replies")? {
            "SELECT draft_id,thread_id,root_comment_id,body,revision,created_at,updated_at FROM reply_drafts d WHERE d.host=?1 AND d.owner=?2 AND d.repository=?3 AND d.number=?4 AND d.context_id=?5 AND NOT EXISTS (SELECT 1 FROM discussion_published_replies p WHERE p.context_id=d.context_id AND p.draft_id=d.draft_id AND p.draft_revision=d.revision) ORDER BY d.created_at,d.draft_id"
        } else {
            "SELECT draft_id,thread_id,root_comment_id,body,revision,created_at,updated_at FROM reply_drafts WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4 AND context_id=?5 ORDER BY created_at,draft_id"
        };
        let mut reply_statement = self.db.prepare(reply_query).map_err(db_error)?;
        let reply_rows = reply_statement
            .query_map(
                params![
                    pr.host,
                    pr.owner,
                    pr.repository,
                    i64::try_from(pr.number).map_err(|_| state_error())?,
                    prepared.context.context_id
                ],
                |r| {
                    Ok(ReplyDraft {
                        id: r.get(0)?,
                        thread_id: r.get(1)?,
                        root_comment_id: r.get(2)?,
                        body: r.get(3)?,
                        revision: u64::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
                        created_at: r.get(5)?,
                        updated_at: r.get(6)?,
                    })
                },
            )
            .map_err(db_error)?;
        let reply_drafts = reply_rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let action_query = if table_exists(&self.db, "discussion_published_thread_actions")? {
            "SELECT draft_id,thread_id,action,observed_resolved,revision,created_at,updated_at FROM thread_action_drafts d WHERE d.host=?1 AND d.owner=?2 AND d.repository=?3 AND d.number=?4 AND d.context_id=?5 AND NOT EXISTS (SELECT 1 FROM discussion_published_thread_actions p WHERE p.context_id=d.context_id AND p.draft_id=d.draft_id AND p.draft_revision=d.revision) ORDER BY d.created_at,d.draft_id"
        } else {
            "SELECT draft_id,thread_id,action,observed_resolved,revision,created_at,updated_at FROM thread_action_drafts WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4 AND context_id=?5 ORDER BY created_at,draft_id"
        };
        let mut action_statement = self.db.prepare(action_query).map_err(db_error)?;
        let action_rows = action_statement
            .query_map(
                params![
                    pr.host,
                    pr.owner,
                    pr.repository,
                    i64::try_from(pr.number).map_err(|_| state_error())?,
                    prepared.context.context_id
                ],
                |r| {
                    let action: String = r.get(2)?;
                    Ok(ThreadActionDraft {
                        id: r.get(0)?,
                        thread_id: r.get(1)?,
                        action: if action == "resolve" {
                            ThreadAction::Resolve
                        } else {
                            ThreadAction::Reopen
                        },
                        observed_resolved: r.get::<_, i64>(3)? != 0,
                        revision: u64::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
                        created_at: r.get(5)?,
                        updated_at: r.get(6)?,
                    })
                },
            )
            .map_err(db_error)?;
        let thread_actions = action_rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let counts = DraftCounts {
            inline_comments: drafts.len() as u64,
            replies: reply_drafts.len() as u64,
            thread_intents: thread_actions.len() as u64,
            code_replacements: self.list_replacements(preparation_id)?.drafts.len() as u64,
            review_comments: review_summary_count(&self.db, preparation_id)?,
        };
        Ok(DraftSnapshot {
            workspace_revision,
            counts,
            drafts,
            reply_drafts,
            thread_actions,
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
             FROM code_replacement_drafts d WHERE d.host=?1 AND d.owner=?2 AND d.repository=?3 AND d.number=?4 AND d.context_id=?5
             AND NOT EXISTS (SELECT 1 FROM review_published_replacements p WHERE p.context_id=?6 AND p.draft_id=d.draft_id AND p.draft_revision=d.revision)
             ORDER BY created_at,draft_id"
        ).map_err(db_error)?;
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

    /// Compares unpublished anchors from an older preparation with eligible lines
    /// in a newer preparation. Relocations are proposals only; callers must get
    /// explicit reviewer confirmation before calling either reattach method.
    pub fn preview_anchor_repairs(
        &self,
        source_preparation_id: &str,
        candidate_preparation_id: &str,
        control: &OperationControl,
    ) -> Result<Vec<AnchorRepairPreview>, ContractError> {
        let preparations = PreparationStore::open(&self.path)?;
        let source_binding = preparations.load(source_preparation_id)?;
        let candidate = preparations.load(candidate_preparation_id)?;
        require_same_pull_request(&source_binding, &candidate)?;
        let index = diff_index(Path::new(&candidate.checkout), &candidate.context, control)?;
        let old = self.list(source_preparation_id)?;
        let replacements = self.list_replacements(source_preparation_id)?;
        let mut previews = Vec::with_capacity(old.drafts.len() + replacements.drafts.len());
        for draft in old.drafts {
            let reattached_draft_id = repaired_draft_id(
                &self.db,
                &source_binding.context.context_id,
                &draft.id,
                draft.revision,
                &candidate.context.context_id,
            )?;
            let exact_text = read_source(
                Path::new(&source_binding.checkout),
                &source_binding.context,
                &draft.source,
                control,
            )
            .ok();
            let (status, proposed_source, explanation) = if reattached_draft_id.is_some() {
                (
                    AnchorRepairStatus::Reattached,
                    None,
                    "A copy is already attached to this preparation; the original remains available on the older review.".into(),
                )
            } else {
                match exact_text.as_deref() {
                    Some(text) => locate_exact_anchor(
                        Path::new(&candidate.checkout),
                        &candidate.context,
                        &draft.source,
                        text,
                        &index,
                        control,
                    )?,
                    None => (
                        AnchorRepairStatus::Unsupported,
                        None,
                        "The original pinned source is unavailable for safe comparison.".into(),
                    ),
                }
            };
            previews.push(AnchorRepairPreview {
                draft_id: draft.id,
                draft_revision: draft.revision,
                kind: "text_comment".into(),
                source: draft.source,
                status,
                proposed_source,
                reattached_draft_id,
                exact_text,
                explanation,
            });
        }
        for draft in replacements.drafts {
            let reattached_draft_id = repaired_draft_id(
                &self.db,
                &source_binding.context.context_id,
                &draft.id,
                draft.revision,
                &candidate.context.context_id,
            )?;
            let (status, proposed_source, explanation) = if reattached_draft_id.is_some() {
                (
                    AnchorRepairStatus::Reattached,
                    None,
                    "A replacement copy is already attached to this preparation; the original remains available on the older review.".into(),
                )
            } else {
                locate_exact_anchor(
                    Path::new(&candidate.checkout),
                    &candidate.context,
                    &draft.source,
                    &draft.original_text,
                    &index,
                    control,
                )?
            };
            previews.push(AnchorRepairPreview {
                draft_id: draft.id,
                draft_revision: draft.revision,
                kind: "code_replacement".into(),
                source: draft.source,
                status,
                proposed_source,
                reattached_draft_id,
                exact_text: Some(draft.original_text),
                explanation,
            });
        }
        Ok(previews)
    }

    /// Copies a human text comment to a reviewer-selected anchor in a newer
    /// preparation. The original draft remains on its old preparation until the
    /// reviewer explicitly discards or defers it.
    #[allow(clippy::too_many_arguments)]
    pub fn reattach_text_comment(
        &mut self,
        source_preparation_id: &str,
        candidate_preparation_id: &str,
        draft_id: &str,
        expected_source_draft_revision: u64,
        expected_candidate_workspace_revision: u64,
        new_source: &SourceRef,
        control: &OperationControl,
    ) -> Result<DraftSaveResult, ContractError> {
        let preparations = PreparationStore::open(&self.path)?;
        let source_binding = preparations.load(source_preparation_id)?;
        let candidate = preparations.load(candidate_preparation_id)?;
        require_same_pull_request(&source_binding, &candidate)?;
        if source_binding.context.context_id == candidate.context.context_id {
            return Err(anchor_repair_invalid(
                "Choose a newer preparation to repair this stale anchor",
            ));
        }
        let copy = repaired_draft_id(
            &self.db,
            &source_binding.context.context_id,
            draft_id,
            expected_source_draft_revision,
            &candidate.context.context_id,
        )?;
        if let Some(copy) = copy {
            let snapshot = self.list(candidate_preparation_id)?;
            let draft = snapshot
                .drafts
                .into_iter()
                .find(|draft| draft.id == copy)
                .ok_or_else(draft_not_found)?;
            return Ok(DraftSaveResult {
                workspace_revision: snapshot.workspace_revision,
                counts: snapshot.counts,
                draft,
            });
        }
        diff_index(Path::new(&candidate.checkout), &candidate.context, control)?
            .validate_anchor(new_source)?;
        read_source(
            Path::new(&candidate.checkout),
            &candidate.context,
            new_source,
            control,
        )?;
        control.check()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let current = workspace_revision(&tx, &candidate.context.pr)?;
        require_revision(
            current,
            expected_candidate_workspace_revision,
            "WORKSPACE_REVISION_STALE",
            "Workspace content changed; reload before attaching this comment",
        )?;
        let row: Option<(String, i64)> = tx
            .query_row(
                "SELECT body,revision FROM manual_text_drafts WHERE draft_id=?1 AND context_id=?2",
                params![draft_id, source_binding.context.context_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        let Some((body, revision)) = row else {
            return Err(draft_not_found());
        };
        validate_body(&body)?;
        require_revision(
            u64::try_from(revision).map_err(|_| state_error())?,
            expected_source_draft_revision,
            "DRAFT_REVISION_STALE",
            "The source draft changed; reload it before reattaching",
        )?;
        let published: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM review_published_drafts WHERE context_id=?1 AND draft_id=?2 AND draft_revision=?3)",
                params![source_binding.context.context_id, draft_id, revision],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if published {
            return Err(draft_not_found());
        }
        let now = unix_millis();
        let id = new_id();
        let provenance = provenance_for(candidate_preparation_id, &candidate);
        tx.execute(
            "INSERT INTO manual_text_drafts(draft_id,context_id,host,owner,repository,number,body,source_json,provenance_json,revision,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10,?10)",
            params![id,candidate.context.context_id,candidate.context.pr.host,candidate.context.pr.owner,candidate.context.pr.repository,i64::try_from(candidate.context.pr.number).map_err(|_|state_error())?,body,serde_json::to_string(new_source).map_err(json_error)?,serde_json::to_string(&provenance).map_err(json_error)?,now],
        )
        .map_err(db_error)?;
        tx.execute(
            "INSERT INTO draft_anchor_repairs(source_context_id,source_draft_id,source_revision,candidate_context_id,candidate_draft_id,kind,created_at) VALUES(?1,?2,?3,?4,?5,'text_comment',?6)",
            params![source_binding.context.context_id,draft_id,revision,candidate.context.context_id,id,now],
        )
        .map_err(db_error)?;
        advance_workspace(&tx, &candidate.context.pr, current)?;
        tx.commit().map_err(db_error)?;
        let snapshot = self.list(candidate_preparation_id)?;
        let draft = snapshot
            .drafts
            .iter()
            .find(|draft| draft.id == id)
            .cloned()
            .ok_or_else(draft_not_found)?;
        Ok(DraftSaveResult {
            workspace_revision: snapshot.workspace_revision,
            counts: snapshot.counts,
            draft,
        })
    }

    /// Copies a replacement to a reviewer-selected anchor in a newer
    /// preparation. The replacement text is kept; the new source excerpt is
    /// captured from the candidate's pinned commit and must be reviewed anew.
    #[allow(clippy::too_many_arguments)]
    pub fn reattach_code_replacement(
        &mut self,
        source_preparation_id: &str,
        candidate_preparation_id: &str,
        draft_id: &str,
        expected_source_draft_revision: u64,
        expected_candidate_workspace_revision: u64,
        new_source: &SourceRef,
        control: &OperationControl,
    ) -> Result<CodeReplacementSaveResult, ContractError> {
        let preparations = PreparationStore::open(&self.path)?;
        let source_binding = preparations.load(source_preparation_id)?;
        let candidate = preparations.load(candidate_preparation_id)?;
        require_same_pull_request(&source_binding, &candidate)?;
        if source_binding.context.context_id == candidate.context.context_id {
            return Err(anchor_repair_invalid(
                "Choose a newer preparation to repair this stale anchor",
            ));
        }
        if let Some(copy) = repaired_draft_id(
            &self.db,
            &source_binding.context.context_id,
            draft_id,
            expected_source_draft_revision,
            &candidate.context.context_id,
        )? {
            let snapshot = self.list_replacements(candidate_preparation_id)?;
            let draft = snapshot
                .drafts
                .into_iter()
                .find(|draft| draft.id == copy)
                .ok_or_else(draft_not_found)?;
            return Ok(CodeReplacementSaveResult {
                workspace_revision: snapshot.workspace_revision,
                draft,
            });
        }
        let new_original = read_source(
            Path::new(&candidate.checkout),
            &candidate.context,
            new_source,
            control,
        )?;
        let index = diff_index(Path::new(&candidate.checkout), &candidate.context, control)?;
        if new_source.revision != Revision::Head {
            return Err(ContractError::new(
                "REPLACEMENT_HEAD_ONLY",
                "Code replacements can target only the pinned head side",
                "/source/revision",
            ));
        }
        index.validate_anchor(new_source)?;
        control.check()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let current = workspace_revision(&tx, &candidate.context.pr)?;
        require_revision(
            current,
            expected_candidate_workspace_revision,
            "WORKSPACE_REVISION_STALE",
            "Workspace content changed; reload before attaching this replacement",
        )?;
        let row: Option<(String, String, i64)> = tx
            .query_row(
                "SELECT original_text,replacement,revision FROM code_replacement_drafts WHERE draft_id=?1 AND context_id=?2",
                params![draft_id, source_binding.context.context_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(db_error)?;
        let Some((_, replacement, revision)) = row else {
            return Err(draft_not_found());
        };
        require_revision(
            u64::try_from(revision).map_err(|_| state_error())?,
            expected_source_draft_revision,
            "DRAFT_REVISION_STALE",
            "The replacement draft changed; reload it before reattaching",
        )?;
        validate_replacement_text(&replacement)?;
        let published: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM review_published_replacements WHERE context_id=?1 AND draft_id=?2 AND draft_revision=?3)",
                params![source_binding.context.context_id, draft_id, revision],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if published {
            return Err(draft_not_found());
        }
        let now = unix_millis();
        let id = new_id();
        let provenance = provenance_for(candidate_preparation_id, &candidate);
        tx.execute(
            "INSERT INTO code_replacement_drafts(draft_id,context_id,host,owner,repository,number,source_json,original_text,replacement,provenance_json,revision,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,1,?11,?11)",
            params![id,candidate.context.context_id,candidate.context.pr.host,candidate.context.pr.owner,candidate.context.pr.repository,i64::try_from(candidate.context.pr.number).map_err(|_|state_error())?,serde_json::to_string(new_source).map_err(json_error)?,new_original,replacement,serde_json::to_string(&provenance).map_err(json_error)?,now],
        )
        .map_err(db_error)?;
        tx.execute(
            "INSERT INTO draft_anchor_repairs(source_context_id,source_draft_id,source_revision,candidate_context_id,candidate_draft_id,kind,created_at) VALUES(?1,?2,?3,?4,?5,'code_replacement',?6)",
            params![source_binding.context.context_id,draft_id,revision,candidate.context.context_id,id,now],
        )
        .map_err(db_error)?;
        advance_workspace(&tx, &candidate.context.pr, current)?;
        tx.commit().map_err(db_error)?;
        let snapshot = self.list_replacements(candidate_preparation_id)?;
        let draft = snapshot
            .drafts
            .iter()
            .find(|draft| draft.id == id)
            .cloned()
            .ok_or_else(draft_not_found)?;
        Ok(CodeReplacementSaveResult {
            workspace_revision: snapshot.workspace_revision,
            draft,
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

    pub fn discard_reply(
        &mut self,
        preparation_id: &str,
        draft_id: &str,
        expected_workspace_revision: u64,
        expected_draft_revision: u64,
    ) -> Result<DraftSnapshot, ContractError> {
        self.discard_discussion_draft(
            preparation_id,
            draft_id,
            "reply_drafts",
            expected_workspace_revision,
            expected_draft_revision,
        )
    }
    pub fn discard_thread_action(
        &mut self,
        preparation_id: &str,
        draft_id: &str,
        expected_workspace_revision: u64,
        expected_draft_revision: u64,
    ) -> Result<DraftSnapshot, ContractError> {
        self.discard_discussion_draft(
            preparation_id,
            draft_id,
            "thread_action_drafts",
            expected_workspace_revision,
            expected_draft_revision,
        )
    }
    fn discard_discussion_draft(
        &mut self,
        preparation_id: &str,
        draft_id: &str,
        table: &str,
        expected_workspace_revision: u64,
        expected_draft_revision: u64,
    ) -> Result<DraftSnapshot, ContractError> {
        let prepared = load_binding(&self.db, preparation_id)?;
        let pr = &prepared.context.pr;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let current = workspace_revision(&tx, pr)?;
        require_revision(
            current,
            expected_workspace_revision,
            "WORKSPACE_REVISION_STALE",
            "Workspace content changed; reload before discarding this draft",
        )?;
        let query = format!(
            "SELECT context_id,revision FROM {table} WHERE draft_id=?1 AND host=?2 AND owner=?3 AND repository=?4 AND number=?5"
        );
        let row: Option<(String, i64)> = tx
            .query_row(
                &query,
                params![draft_id, pr.host, pr.owner, pr.repository, pr.number as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        let Some((context, revision)) = row else {
            return Err(draft_not_found());
        };
        if context != prepared.context.context_id {
            return Err(draft_not_found());
        };
        require_revision(
            revision as u64,
            expected_draft_revision,
            "DRAFT_REVISION_STALE",
            "Draft changed elsewhere; reload it before discarding",
        )?;
        tx.execute(
            &format!("DELETE FROM {table} WHERE draft_id=?1"),
            [draft_id],
        )
        .map_err(db_error)?;
        advance_workspace(&tx, pr, current)?;
        tx.commit().map_err(db_error)?;
        self.list(preparation_id)
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
        let counts = count_all(
            &tx,
            pr,
            &prepared.context.context_id,
            &prepared.preparation_id,
        )?;
        tx.commit().map_err(db_error)?;
        Ok(DraftSaveResult {
            workspace_revision,
            counts,
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
        let _drafts = count_all(
            &tx,
            pr,
            &prepared.context.context_id,
            &prepared.preparation_id,
        )?;
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

fn count_all(
    tx: &Transaction<'_>,
    pr: &crate::PrIdentity,
    context_id: &str,
    preparation_id: &str,
) -> Result<DraftCounts, ContractError> {
    let inline = tx.query_row(
        "SELECT count(*) FROM manual_text_drafts d WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4 AND context_id=?5 AND NOT EXISTS (SELECT 1 FROM review_published_drafts p WHERE p.context_id=?6 AND p.draft_id=d.draft_id AND p.draft_revision=d.revision)",
        params![pr.host,pr.owner,pr.repository,i64::try_from(pr.number).map_err(|_|state_error())?,context_id,context_id],
        |row|row.get::<_,i64>(0)
    ).map_err(db_error).and_then(|value|u64::try_from(value).map_err(|_|state_error()))?;
    let count = |table: &str| -> Result<u64, ContractError> {
        tx.query_row(&format!("SELECT count(*) FROM {table} WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4 AND context_id=?5"),params![pr.host,pr.owner,pr.repository,i64::try_from(pr.number).map_err(|_|state_error())?,context_id],|r|r.get::<_,i64>(0)).map_err(db_error).and_then(|value|u64::try_from(value).map_err(|_|state_error()))
    };
    let code_replacements = tx
        .query_row(
            "SELECT count(*) FROM code_replacement_drafts d
             WHERE d.host=?1 AND d.owner=?2 AND d.repository=?3 AND d.number=?4 AND d.context_id=?5
               AND NOT EXISTS (SELECT 1 FROM review_published_replacements p
                               WHERE p.context_id=?6 AND p.draft_id=d.draft_id AND p.draft_revision=d.revision)",
            params![pr.host,pr.owner,pr.repository,i64::try_from(pr.number).map_err(|_|state_error())?,context_id,context_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(db_error)
        .and_then(|value| u64::try_from(value).map_err(|_| state_error()))?;
    Ok(DraftCounts {
        inline_comments: inline,
        replies: count("reply_drafts")?,
        thread_intents: count("thread_action_drafts")?,
        code_replacements,
        review_comments: review_summary_count(tx, preparation_id)?,
    })
}

fn review_summary_count(db: &Connection, preparation_id: &str) -> Result<u64, ContractError> {
    let json: Option<String> = db
        .query_row(
            "SELECT intent_json FROM review_intents WHERE preparation_id=?1",
            [preparation_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    let Some(json) = json else { return Ok(0) };
    let intent: crate::ReviewIntent = serde_json::from_str(&json).map_err(json_error)?;
    if !intent
        .summary
        .as_deref()
        .is_some_and(|summary| !summary.trim().is_empty())
    {
        return Ok(0);
    }
    let mut statement = db
        .prepare("SELECT payload_json FROM review_operations WHERE preparation_id=?1 AND state='confirmed'")
        .map_err(db_error)?;
    let rows = statement
        .query_map([preparation_id], |row| row.get::<_, String>(0))
        .map_err(db_error)?;
    for row in rows {
        let payload: crate::ReviewPayload =
            serde_json::from_str(&row.map_err(db_error)?).map_err(json_error)?;
        if payload.intent_revision == intent.revision && payload.summary == intent.summary {
            return Ok(0);
        }
    }
    Ok(1)
}

fn find_thread(
    discussions: &serde_json::Value,
    id: &str,
) -> Result<serde_json::Value, ContractError> {
    discussions["threads"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|thread| thread["id"].as_str() == Some(id))
        .cloned()
        .ok_or_else(|| {
            ContractError::new(
                "DISCUSSION_THREAD_NOT_FOUND",
                "Thread is not present in the latest discussion observation",
                "/thread_id",
            )
        })
}
fn root_comment_exists(thread: &serde_json::Value, root_id: &str) -> bool {
    thread["comments"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|comment| comment["id"].as_str() == Some(root_id) && comment["replyTo"].is_null())
}

fn repaired_draft_id(
    db: &Connection,
    source_context_id: &str,
    source_draft_id: &str,
    source_revision: u64,
    candidate_context_id: &str,
) -> Result<Option<String>, ContractError> {
    let revision = i64::try_from(source_revision).map_err(|_| state_error())?;
    db.query_row(
        "SELECT candidate_draft_id FROM draft_anchor_repairs WHERE source_context_id=?1 AND source_draft_id=?2 AND source_revision=?3 AND candidate_context_id=?4",
        params![source_context_id, source_draft_id, revision, candidate_context_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_error)
}

fn provenance_for(preparation_id: &str, binding: &PreparedBinding) -> DraftProvenance {
    DraftProvenance {
        preparation_id: preparation_id.into(),
        context_id: binding.context.context_id.clone(),
        base_sha: binding.context.base_sha.clone(),
        head_sha: binding.context.head_sha.clone(),
        merge_base_sha: binding.context.merge_base_sha.clone(),
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

fn table_exists(db: &Connection, table: &str) -> Result<bool, ContractError> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
    .map_err(db_error)
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
