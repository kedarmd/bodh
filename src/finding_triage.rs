//! Durable reviewer decisions and edits for findings in a committed result.
use crate::{
    ContractError, GeneratedReviewStore, OperationControl, PreparationStore, Revision, SourceRef,
    Suggestion, diff_index, read_source,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingDisposition {
    Unreviewed,
    Staged,
    Dismissed,
    Deferred,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FindingTriageEntry {
    pub finding_id: String,
    pub disposition: FindingDisposition,
    pub selected_suggestion_id: Option<String>,
    pub manual_feedback: Option<String>,
    pub revision: u64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FindingAlternativeEdit {
    pub finding_id: String,
    pub suggestion_id: String,
    pub draft_id: String,
    pub body: String,
    pub source: SourceRef,
    pub original_text: Option<String>,
    pub replacement: Option<String>,
    pub revision: u64,
    pub updated_at: i64,
}

#[derive(Clone, Debug)]
pub struct FindingAlternativeEditInput<'a> {
    pub body: &'a str,
    pub replacement: Option<&'a str>,
    pub expected_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FindingTriageSnapshot {
    pub context_id: String,
    pub entries: Vec<FindingTriageEntry>,
    pub alternative_edits: Vec<FindingAlternativeEdit>,
}

pub struct FindingTriageStore {
    db: Connection,
    path: PathBuf,
}

impl FindingTriageStore {
    pub fn open(path: &Path) -> Result<Self, ContractError> {
        drop(PreparationStore::open(path)?);
        drop(GeneratedReviewStore::open(path)?);
        let db = Connection::open(path).map_err(db_error)?;
        db.busy_timeout(Duration::from_secs(5)).map_err(db_error)?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;")
            .map_err(db_error)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS finding_triage_migrations(version INTEGER PRIMARY KEY NOT NULL);
             CREATE TABLE IF NOT EXISTS finding_triage_entries(
                context_id TEXT NOT NULL REFERENCES contexts(context_id), finding_id TEXT NOT NULL,
                disposition TEXT NOT NULL CHECK(disposition IN ('unreviewed','staged','dismissed','deferred')),
                selected_suggestion_id TEXT, manual_feedback TEXT,
                revision INTEGER NOT NULL CHECK(revision>0), updated_at INTEGER NOT NULL,
                PRIMARY KEY(context_id,finding_id));
             CREATE TABLE IF NOT EXISTS finding_alternative_edits(
                context_id TEXT NOT NULL REFERENCES contexts(context_id), finding_id TEXT NOT NULL,
                suggestion_id TEXT NOT NULL, body TEXT NOT NULL, source_json TEXT NOT NULL,
                original_text TEXT, replacement TEXT, draft_id TEXT NOT NULL,
                revision INTEGER NOT NULL CHECK(revision>0),
                updated_at INTEGER NOT NULL, PRIMARY KEY(context_id,finding_id,suggestion_id));
             INSERT OR IGNORE INTO finding_triage_migrations(version) VALUES(1);",
        ).map_err(db_error)?;
        Ok(Self {
            db,
            path: path.to_path_buf(),
        })
    }

    pub fn list(&self, preparation_id: &str) -> Result<FindingTriageSnapshot, ContractError> {
        let binding = PreparationStore::open(&self.path)?.load(preparation_id)?;
        let context_id = binding.context.context_id;
        let drafts = crate::DraftStore::open(&self.path)?.list(preparation_id)?;
        let replacements =
            crate::DraftStore::open(&self.path)?.list_replacements(preparation_id)?;
        let mut entries = Vec::new();
        let mut statement = self.db.prepare("SELECT finding_id,disposition,selected_suggestion_id,manual_feedback,revision,updated_at FROM finding_triage_entries WHERE context_id=?1 ORDER BY finding_id").map_err(db_error)?;
        let rows = statement
            .query_map([&context_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })
            .map_err(db_error)?;
        for row in rows {
            let (
                finding_id,
                disposition,
                selected_suggestion_id,
                manual_feedback,
                revision,
                updated_at,
            ) = row.map_err(db_error)?;
            entries.push(FindingTriageEntry {
                finding_id,
                disposition: parse_disposition(&disposition)?,
                selected_suggestion_id,
                manual_feedback,
                revision: to_u64(revision)?,
                updated_at,
            });
        }
        let mut alternative_edits = Vec::new();
        let mut statement = self.db.prepare("SELECT finding_id,suggestion_id,body,source_json,original_text,replacement,draft_id,revision,updated_at FROM finding_alternative_edits WHERE context_id=?1 ORDER BY finding_id,suggestion_id").map_err(db_error)?;
        let rows = statement
            .query_map([&context_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, i64>(8)?,
                ))
            })
            .map_err(db_error)?;
        for row in rows {
            let (
                finding_id,
                suggestion_id,
                body,
                source_json,
                original_text,
                replacement,
                draft_id,
                revision,
                updated_at,
            ) = row.map_err(db_error)?;
            let mut edit = FindingAlternativeEdit {
                finding_id,
                suggestion_id,
                draft_id,
                body,
                source: serde_json::from_str(&source_json).map_err(json_error)?,
                original_text,
                replacement,
                revision: to_u64(revision)?,
                updated_at,
            };
            if let Some(draft) = drafts.drafts.iter().find(|draft| draft.id == edit.draft_id) {
                edit.body = draft.body.clone();
            }
            if let Some(draft) = replacements
                .drafts
                .iter()
                .find(|draft| draft.id == edit.draft_id)
            {
                edit.replacement = Some(draft.replacement.clone());
            }
            alternative_edits.push(edit);
        }
        Ok(FindingTriageSnapshot {
            context_id,
            entries,
            alternative_edits,
        })
    }

    pub fn set_disposition(
        &mut self,
        preparation_id: &str,
        finding_id: &str,
        disposition: FindingDisposition,
        selected_suggestion_id: Option<&str>,
        manual_feedback: Option<&str>,
        expected_revision: u64,
    ) -> Result<FindingTriageEntry, ContractError> {
        validate_text(manual_feedback)?;
        if disposition == FindingDisposition::Staged
            && selected_suggestion_id.is_none()
            && manual_feedback.is_none()
        {
            return Err(invalid(
                "Staged findings need exactly one selected alternative or independent manual feedback",
                "/disposition",
            ));
        }
        let binding = PreparationStore::open(&self.path)?.load(preparation_id)?;
        let review = GeneratedReviewStore::open(&self.path)?
            .current(preparation_id)?
            .filter(|r| r.context.context.context_id == binding.context.context_id)
            .ok_or_else(|| {
                ContractError::new(
                    "GENERATED_REVIEW_MISSING",
                    "No committed finding result exists for this pinned context",
                    "",
                )
            })?;
        let finding = review
            .result
            .findings
            .iter()
            .find(|f| f.id == finding_id)
            .ok_or_else(|| {
                ContractError::new(
                    "FINDING_NOT_FOUND",
                    "Finding is not part of the committed result",
                    "/finding_id",
                )
            })?;
        if let Some(id) = selected_suggestion_id
            && !finding.suggestions.iter().any(|s| suggestion_id(s) == id)
        {
            return Err(ContractError::new(
                "SUGGESTION_NOT_FOUND",
                "Alternative is not part of this finding",
                "/selected_suggestion_id",
            ));
        }
        let current = self
            .list(preparation_id)?
            .entries
            .into_iter()
            .find(|entry| entry.finding_id == finding_id);
        let previous_revision = current.as_ref().map(|entry| entry.revision).unwrap_or(0);
        let effective_feedback = manual_feedback.map(str::to_owned).or_else(|| {
            current
                .as_ref()
                .and_then(|entry| entry.manual_feedback.clone())
        });
        if previous_revision != expected_revision {
            return Err(ContractError::new(
                "FINDING_TRIAGE_STALE",
                "Finding triage changed elsewhere; reload before saving",
                "/expected_revision",
            ));
        }
        if let Some(previous_suggestion_id) = current
            .as_ref()
            .filter(|entry| entry.disposition == FindingDisposition::Staged)
            .and_then(|entry| entry.selected_suggestion_id.as_deref())
            .filter(|previous| {
                disposition != FindingDisposition::Staged
                    || Some(*previous) != selected_suggestion_id
            })
            && let Some(edit) = self
                .list(preparation_id)?
                .alternative_edits
                .into_iter()
                .find(|edit| {
                    edit.finding_id == finding_id && edit.suggestion_id == previous_suggestion_id
                })
        {
            let mut publication = crate::ReviewPublicationStore::open(&self.path)?;
            let mut intent = publication.intent(preparation_id)?;
            let before = intent.selected.len() + intent.selected_replacements.len();
            intent
                .selected
                .retain(|selected| selected.id != edit.draft_id);
            intent
                .selected_replacements
                .retain(|selected| selected.id != edit.draft_id);
            if before != intent.selected.len() + intent.selected_replacements.len() {
                let drafts = crate::DraftStore::open(&self.path)?.list(preparation_id)?;
                publication.stage(crate::StageReviewRequest {
                    state: self.path.clone(),
                    preparation_id: preparation_id.into(),
                    expected_workspace_revision: drafts.workspace_revision,
                    expected_intent_revision: intent.revision,
                    selected: intent.selected,
                    selected_replacements: intent.selected_replacements,
                    selected_replies: intent.selected_replies,
                    selected_thread_actions: intent.selected_thread_actions,
                    summary: intent.summary,
                    event: intent.event,
                })?;
            }
        }
        let actual: Option<i64> = self
            .db
            .query_row(
                "SELECT revision FROM finding_triage_entries WHERE context_id=?1 AND finding_id=?2",
                params![binding.context.context_id, finding_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let actual = actual.map(to_u64).transpose()?.unwrap_or(0);
        if actual != expected_revision {
            return Err(ContractError::new(
                "FINDING_TRIAGE_STALE",
                "Finding triage changed elsewhere; reload before saving",
                "/expected_revision",
            ));
        }
        let updated_at = unix_millis();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let locked_revision: Option<i64> = tx
            .query_row(
                "SELECT revision FROM finding_triage_entries WHERE context_id=?1 AND finding_id=?2",
                params![binding.context.context_id, finding_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let locked_revision = locked_revision.map(to_u64).transpose()?.unwrap_or(0);
        if locked_revision != expected_revision {
            return Err(ContractError::new(
                "FINDING_TRIAGE_STALE",
                "Finding triage changed elsewhere; reload before saving",
                "/expected_revision",
            ));
        }
        let revision = locked_revision.checked_add(1).ok_or_else(state_error)?;
        tx.execute("INSERT INTO finding_triage_entries(context_id,finding_id,disposition,selected_suggestion_id,manual_feedback,revision,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(context_id,finding_id) DO UPDATE SET disposition=excluded.disposition,selected_suggestion_id=excluded.selected_suggestion_id,manual_feedback=excluded.manual_feedback,revision=excluded.revision,updated_at=excluded.updated_at", params![binding.context.context_id,finding_id,disposition_name(disposition),selected_suggestion_id,effective_feedback,to_i64(revision)?,updated_at]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(FindingTriageEntry {
            finding_id: finding_id.into(),
            disposition,
            selected_suggestion_id: selected_suggestion_id.map(str::to_owned),
            manual_feedback: effective_feedback,
            revision,
            updated_at,
        })
    }

    /// Stages one editable suggestion and removes every other alternative for
    /// the same finding from the publication selection. Saved edits remain in
    /// local draft storage when another alternative is selected later.
    pub fn stage_alternative(
        &mut self,
        preparation_id: &str,
        finding_id: &str,
        suggestion_id_value: &str,
        expected_triage_revision: u64,
    ) -> Result<(crate::ReviewIntent, FindingTriageEntry), ContractError> {
        let binding = PreparationStore::open(&self.path)?.load(preparation_id)?;
        let review = GeneratedReviewStore::open(&self.path)?
            .current(preparation_id)?
            .filter(|r| r.context.context.context_id == binding.context.context_id)
            .ok_or_else(|| {
                ContractError::new(
                    "GENERATED_REVIEW_MISSING",
                    "No committed finding result exists for this pinned context",
                    "",
                )
            })?;
        let finding = review
            .result
            .findings
            .iter()
            .find(|f| f.id == finding_id)
            .ok_or_else(|| {
                ContractError::new(
                    "FINDING_NOT_FOUND",
                    "Finding is not part of the committed result",
                    "/finding_id",
                )
            })?;
        let suggestion = finding
            .suggestions
            .iter()
            .find(|s| suggestion_id(s) == suggestion_id_value)
            .ok_or_else(|| {
                ContractError::new(
                    "SUGGESTION_NOT_FOUND",
                    "Alternative is not part of this finding",
                    "/suggestion_id",
                )
            })?;
        let triage = self.list(preparation_id)?;
        let edit = if let Some(edit) = triage
            .alternative_edits
            .iter()
            .find(|e| e.finding_id == finding_id && e.suggestion_id == suggestion_id_value)
        {
            edit.clone()
        } else {
            let (body, replacement) = match suggestion {
                Suggestion::TextComment { body, .. } => (body.as_str(), None),
                Suggestion::CodeReplacement {
                    body, replacement, ..
                } => (body.as_str(), Some(replacement.as_str())),
            };
            self.save_alternative_edit(
                preparation_id,
                finding_id,
                suggestion_id_value,
                FindingAlternativeEditInput {
                    body,
                    replacement,
                    expected_revision: 0,
                },
                &OperationControl::default(),
            )?
        };
        let entry_revision = triage
            .entries
            .iter()
            .find(|entry| entry.finding_id == finding_id)
            .map(|entry| entry.revision)
            .unwrap_or(0);
        if entry_revision != expected_triage_revision {
            return Err(ContractError::new(
                "FINDING_TRIAGE_STALE",
                "Finding triage changed elsewhere; reload before staging",
                "/expected_revision",
            ));
        }
        let mut publication = crate::ReviewPublicationStore::open(&self.path)?;
        let mut intent = publication.intent(preparation_id)?;
        let snapshot = crate::DraftStore::open(&self.path)?.list(preparation_id)?;
        let replacement_snapshot =
            crate::DraftStore::open(&self.path)?.list_replacements(preparation_id)?;
        for alternative in triage
            .alternative_edits
            .iter()
            .filter(|edit| edit.finding_id == finding_id)
        {
            intent
                .selected
                .retain(|selected| selected.id != alternative.draft_id);
            intent
                .selected_replacements
                .retain(|selected| selected.id != alternative.draft_id);
        }
        match suggestion {
            Suggestion::TextComment { .. } => {
                let draft = snapshot
                    .drafts
                    .iter()
                    .find(|draft| draft.id == edit.draft_id)
                    .ok_or_else(|| {
                        ContractError::new(
                            "FINDING_DRAFT_MISSING",
                            "Saved alternative draft is no longer available",
                            "/suggestion_id",
                        )
                    })?;
                intent.selected.push(crate::SelectedDraft {
                    id: draft.id.clone(),
                    revision: draft.revision,
                });
            }
            Suggestion::CodeReplacement { .. } => {
                let draft = replacement_snapshot
                    .drafts
                    .iter()
                    .find(|draft| draft.id == edit.draft_id)
                    .ok_or_else(|| {
                        ContractError::new(
                            "FINDING_DRAFT_MISSING",
                            "Saved replacement draft is no longer available",
                            "/suggestion_id",
                        )
                    })?;
                intent.selected_replacements.push(crate::SelectedDraft {
                    id: draft.id.clone(),
                    revision: draft.revision,
                });
            }
        }
        let staged = publication.stage(crate::StageReviewRequest {
            state: self.path.clone(),
            preparation_id: preparation_id.into(),
            expected_workspace_revision: snapshot.workspace_revision,
            expected_intent_revision: intent.revision,
            selected: intent.selected,
            selected_replacements: intent.selected_replacements,
            selected_replies: intent.selected_replies,
            selected_thread_actions: intent.selected_thread_actions,
            summary: intent.summary,
            event: intent.event,
        })?;
        let entry = self.set_disposition(
            preparation_id,
            finding_id,
            FindingDisposition::Staged,
            Some(suggestion_id_value),
            None,
            expected_triage_revision,
        )?;
        Ok((staged, entry))
    }

    pub fn save_alternative_edit(
        &mut self,
        preparation_id: &str,
        finding_id: &str,
        suggestion_id_value: &str,
        input: FindingAlternativeEditInput<'_>,
        control: &OperationControl,
    ) -> Result<FindingAlternativeEdit, ContractError> {
        let FindingAlternativeEditInput {
            body,
            replacement,
            expected_revision,
        } = input;
        validate_text(Some(body))?;
        validate_text(replacement)?;
        let binding = PreparationStore::open(&self.path)?.load(preparation_id)?;
        let review = GeneratedReviewStore::open(&self.path)?
            .current(preparation_id)?
            .filter(|r| r.context.context.context_id == binding.context.context_id)
            .ok_or_else(|| {
                ContractError::new(
                    "GENERATED_REVIEW_MISSING",
                    "No committed finding result exists for this pinned context",
                    "",
                )
            })?;
        let finding = review
            .result
            .findings
            .iter()
            .find(|f| f.id == finding_id)
            .ok_or_else(|| {
                ContractError::new(
                    "FINDING_NOT_FOUND",
                    "Finding is not part of the committed result",
                    "/finding_id",
                )
            })?;
        let suggestion = finding
            .suggestions
            .iter()
            .find(|s| suggestion_id(s) == suggestion_id_value)
            .ok_or_else(|| {
                ContractError::new(
                    "SUGGESTION_NOT_FOUND",
                    "Alternative is not part of this finding",
                    "/suggestion_id",
                )
            })?;
        let (source, original, initial_replacement) = match suggestion {
            Suggestion::TextComment { source, .. } => (source, None, None),
            Suggestion::CodeReplacement {
                source,
                original_text,
                replacement,
                ..
            } => (
                source,
                Some(original_text.as_str()),
                Some(replacement.as_str()),
            ),
        };
        diff_index(Path::new(&binding.checkout), &binding.context, control)?
            .validate_anchor(source)?;
        if let Suggestion::CodeReplacement { .. } = suggestion {
            if source.revision != Revision::Head {
                return Err(ContractError::new(
                    "REPLACEMENT_HEAD_ONLY",
                    "Code replacements can target only pinned head source",
                    "/source/revision",
                ));
            }
            if read_source(
                Path::new(&binding.checkout),
                &binding.context,
                source,
                control,
            )? != original.unwrap_or_default()
            {
                return Err(ContractError::new(
                    "REPLACEMENT_SOURCE_CHANGED",
                    "Alternative differs from pinned source; refresh before editing",
                    "/source",
                ));
            }
        }
        let existing: Option<(i64, String)> = self.db.query_row("SELECT revision,draft_id FROM finding_alternative_edits WHERE context_id=?1 AND finding_id=?2 AND suggestion_id=?3", params![binding.context.context_id,finding_id,suggestion_id_value], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)?;
        let actual = existing
            .as_ref()
            .map(|(revision, _)| *revision)
            .map(to_u64)
            .transpose()?
            .unwrap_or(0);
        if actual != expected_revision {
            return Err(ContractError::new(
                "FINDING_ALTERNATIVE_STALE",
                "Alternative draft changed elsewhere; reload before saving",
                "/expected_revision",
            ));
        }
        let updated_at = unix_millis();
        let source_json = serde_json::to_string(source).map_err(json_error)?;
        let replacement = replacement.or(initial_replacement);
        let mut drafts = crate::DraftStore::open(&self.path)?;
        let draft_snapshot = drafts.list(preparation_id)?;
        let draft_id = match suggestion {
            Suggestion::TextComment { .. } => {
                let mapped = existing.as_ref().map(|(_, id)| id.as_str());
                let expected_draft_revision = mapped
                    .and_then(|id| {
                        draft_snapshot
                            .drafts
                            .iter()
                            .find(|draft| draft.id == id)
                            .map(|draft| draft.revision)
                    })
                    .unwrap_or(0);
                drafts
                    .save(
                        preparation_id,
                        mapped,
                        body,
                        source,
                        draft_snapshot.workspace_revision,
                        expected_draft_revision,
                        control,
                    )?
                    .draft
                    .id
            }
            Suggestion::CodeReplacement { .. } => {
                let replacement_snapshot = drafts.list_replacements(preparation_id)?;
                let mapped = existing.as_ref().map(|(_, id)| id.as_str());
                let expected_draft_revision = mapped
                    .and_then(|id| {
                        replacement_snapshot
                            .drafts
                            .iter()
                            .find(|draft| draft.id == id)
                            .map(|draft| draft.revision)
                    })
                    .unwrap_or(0);
                drafts
                    .save_replacement(
                        preparation_id,
                        mapped,
                        source,
                        original.unwrap_or_default(),
                        replacement.unwrap_or_default(),
                        replacement_snapshot.workspace_revision,
                        expected_draft_revision,
                        control,
                    )?
                    .draft
                    .id
            }
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let locked_revision: Option<i64> = tx
            .query_row(
                "SELECT revision FROM finding_alternative_edits WHERE context_id=?1 AND finding_id=?2 AND suggestion_id=?3",
                params![binding.context.context_id,finding_id,suggestion_id_value],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let locked_revision = locked_revision.map(to_u64).transpose()?.unwrap_or(0);
        if locked_revision != expected_revision {
            return Err(ContractError::new(
                "FINDING_ALTERNATIVE_STALE",
                "Alternative draft changed elsewhere; reload before saving",
                "/expected_revision",
            ));
        }
        let revision = locked_revision.checked_add(1).ok_or_else(state_error)?;
        tx.execute("INSERT INTO finding_alternative_edits(context_id,finding_id,suggestion_id,body,source_json,original_text,replacement,draft_id,revision,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(context_id,finding_id,suggestion_id) DO UPDATE SET body=excluded.body,source_json=excluded.source_json,original_text=excluded.original_text,replacement=excluded.replacement,draft_id=excluded.draft_id,revision=excluded.revision,updated_at=excluded.updated_at", params![binding.context.context_id,finding_id,suggestion_id_value,body,source_json,original,replacement,draft_id,to_i64(revision)?,updated_at]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(FindingAlternativeEdit {
            finding_id: finding_id.into(),
            suggestion_id: suggestion_id_value.into(),
            draft_id,
            body: body.into(),
            source: source.clone(),
            original_text: original.map(str::to_owned),
            replacement: replacement.map(str::to_owned),
            revision,
            updated_at,
        })
    }
}

fn suggestion_id(s: &Suggestion) -> &str {
    match s {
        Suggestion::TextComment { id, .. } | Suggestion::CodeReplacement { id, .. } => id,
    }
}
fn validate_text(v: Option<&str>) -> Result<(), ContractError> {
    if v.is_some_and(|s| s.trim().is_empty() || s.chars().count() > 65_536) {
        Err(invalid(
            "Text must be nonempty and no longer than 65,536 characters",
            "/body",
        ))
    } else {
        Ok(())
    }
}
fn disposition_name(v: FindingDisposition) -> &'static str {
    match v {
        FindingDisposition::Unreviewed => "unreviewed",
        FindingDisposition::Staged => "staged",
        FindingDisposition::Dismissed => "dismissed",
        FindingDisposition::Deferred => "deferred",
    }
}
fn parse_disposition(v: &str) -> Result<FindingDisposition, ContractError> {
    match v {
        "unreviewed" => Ok(FindingDisposition::Unreviewed),
        "staged" => Ok(FindingDisposition::Staged),
        "dismissed" => Ok(FindingDisposition::Dismissed),
        "deferred" => Ok(FindingDisposition::Deferred),
        _ => Err(state_error()),
    }
}
fn db_error(_: rusqlite::Error) -> ContractError {
    ContractError::new(
        "STATE_UNAVAILABLE",
        "Finding triage state is unavailable",
        "",
    )
}
fn json_error(_: serde_json::Error) -> ContractError {
    ContractError::new("STATE_INVALID", "Persisted finding triage is invalid", "")
}
fn invalid(message: &str, pointer: &str) -> ContractError {
    ContractError::new("FINDING_TRIAGE_INVALID", message, pointer)
}
fn state_error() -> ContractError {
    ContractError::new("STATE_INVALID", "Finding triage revision is invalid", "")
}
fn to_u64(v: i64) -> Result<u64, ContractError> {
    u64::try_from(v).map_err(|_| state_error())
}
fn to_i64(v: u64) -> Result<i64, ContractError> {
    i64::try_from(v).map_err(|_| state_error())
}
fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}
