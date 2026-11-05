//! Durable, independently journaled publication of review-thread replies and
//! resolve/reopen requests. Every mutation is recorded as `sending` before the
//! transport is called; a restart never retries an uncertain request.
use crate::{ContractError, DraftStore, OperationControl, PreparationStore, ThreadAction};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionSelection {
    pub id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionStep {
    pub id: String,
    pub ordinal: u64,
    pub kind: String,
    pub draft_id: String,
    pub draft_revision: u64,
    pub thread_id: String,
    pub root_comment_id: Option<String>,
    pub body: Option<String>,
    pub action: Option<ThreadAction>,
    pub observed_resolved: Option<bool>,
    pub state: String,
    pub remote_id: Option<String>,
    pub failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionPlan {
    pub id: String,
    pub fingerprint: String,
    pub preparation_id: String,
    pub context_id: String,
    /// An optional grouped review operation. It must be confirmed before these
    /// dependent discussion operations can be sent.
    pub review_operation_id: Option<String>,
    pub state: String,
    pub steps: Vec<DiscussionStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadState {
    pub pull_request_open: bool,
    pub head_sha: String,
    pub base_sha: String,
    pub resolved: bool,
    pub can_reply: bool,
    pub can_resolve: bool,
    pub can_reopen: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscussionSendOutcome {
    Confirmed { remote_id: Option<String> },
    Rejected { message: String },
    Uncertain { message: String },
}

/// The transport exposes fresh observations separately from writes. Tests can
/// supply deterministic implementations and never touch GitHub.
pub trait DiscussionPublisher {
    fn inspect(
        &self,
        preparation_id: &str,
        thread_id: &str,
        control: &OperationControl,
    ) -> Result<ThreadState, ContractError>;
    fn reply(
        &self,
        preparation_id: &str,
        root_comment_id: &str,
        body: &str,
        control: &OperationControl,
    ) -> DiscussionSendOutcome;
    fn thread_action(
        &self,
        preparation_id: &str,
        thread_id: &str,
        action: ThreadAction,
        control: &OperationControl,
    ) -> DiscussionSendOutcome;
}

/// GitHub CLI transport. All inspections are read-only; write methods are only
/// called after the journal has durably moved the corresponding step to
/// `sending`.
pub struct GhDiscussionPublisher {
    pub executable: PathBuf,
    pub state: PathBuf,
}

impl DiscussionPublisher for GhDiscussionPublisher {
    fn inspect(
        &self,
        preparation_id: &str,
        thread_id: &str,
        control: &OperationControl,
    ) -> Result<ThreadState, ContractError> {
        let binding = PreparationStore::open(&self.state)?.load(preparation_id)?;
        let pr_query = "query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){state headRefOid baseRefOid viewerPermission}}}";
        let scope = crate::discussion_acquisition::GraphQlScope {
            host: &binding.context.pr.host,
            owner: &binding.context.pr.owner,
            repository: &binding.context.pr.repository,
            number: binding.context.pr.number,
            thread_id: None,
            after: None,
        };
        let pr =
            crate::discussion_acquisition::gh_graphql(&self.executable, pr_query, scope, control)?;
        let pr = pr
            .get("data")
            .and_then(|v| v.get("repository"))
            .and_then(|v| v.get("pullRequest"))
            .ok_or_else(remote_invalid)?;
        let query = "query($id:ID!){node(id:$id){... on PullRequestReviewThread{id isResolved viewerCanReply viewerCanResolve viewerCanUnresolve}}}";
        let scope = crate::discussion_acquisition::GraphQlScope {
            host: &binding.context.pr.host,
            owner: &binding.context.pr.owner,
            repository: &binding.context.pr.repository,
            number: binding.context.pr.number,
            thread_id: Some(thread_id),
            after: None,
        };
        let thread =
            crate::discussion_acquisition::gh_graphql(&self.executable, query, scope, control)?;
        let thread = thread
            .get("data")
            .and_then(|v| v.get("node"))
            .filter(|v| !v.is_null())
            .ok_or_else(remote_invalid)?;
        let text = |value: &Value, key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(remote_invalid)
        };
        Ok(ThreadState {
            pull_request_open: pr.get("state").and_then(Value::as_str) == Some("OPEN"),
            head_sha: text(pr, "headRefOid")?,
            base_sha: text(pr, "baseRefOid")?,
            resolved: thread
                .get("isResolved")
                .and_then(Value::as_bool)
                .ok_or_else(remote_invalid)?,
            can_reply: thread
                .get("viewerCanReply")
                .and_then(Value::as_bool)
                .ok_or_else(remote_invalid)?,
            can_resolve: thread
                .get("viewerCanResolve")
                .and_then(Value::as_bool)
                .ok_or_else(remote_invalid)?,
            can_reopen: thread
                .get("viewerCanUnresolve")
                .and_then(Value::as_bool)
                .ok_or_else(remote_invalid)?,
        })
    }

    fn reply(
        &self,
        preparation_id: &str,
        root_comment_id: &str,
        body: &str,
        control: &OperationControl,
    ) -> DiscussionSendOutcome {
        let result = (|| {
            let binding = PreparationStore::open(&self.state)?.load(preparation_id)?;
            let endpoint = format!(
                "repos/{}/{}/pulls/{}/comments/{}/replies",
                binding.context.pr.owner,
                binding.context.pr.repository,
                binding.context.pr.number,
                root_comment_id
            );
            let args = vec![
                "api".into(),
                "--hostname".into(),
                binding.context.pr.host,
                "--method".into(),
                "POST".into(),
                endpoint,
                "-f".into(),
                format!("body={body}"),
            ];
            let bytes =
                crate::discussion_acquisition::run_process(&self.executable, &args, control)?;
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| remote_invalid())?;
            value
                .get("id")
                .and_then(Value::as_u64)
                .map(|id| Some(id.to_string()))
                .ok_or_else(remote_invalid)
        })();
        match result {
            Ok(remote_id) => DiscussionSendOutcome::Confirmed { remote_id },
            Err(error) => classify_write_error(error),
        }
    }

    fn thread_action(
        &self,
        preparation_id: &str,
        thread_id: &str,
        action: ThreadAction,
        control: &OperationControl,
    ) -> DiscussionSendOutcome {
        let result = (|| {
            let binding = PreparationStore::open(&self.state)?.load(preparation_id)?;
            let (mutation, field) = match action {
                ThreadAction::Resolve => ("resolveReviewThread", "resolveReviewThread"),
                ThreadAction::Reopen => ("unresolveReviewThread", "unresolveReviewThread"),
            };
            let query = format!(
                "mutation($id:ID!){{{field}(input:{{threadId:$id}}){{thread{{id isResolved}}}}}}"
            );
            let scope = crate::discussion_acquisition::GraphQlScope {
                host: &binding.context.pr.host,
                owner: &binding.context.pr.owner,
                repository: &binding.context.pr.repository,
                number: binding.context.pr.number,
                thread_id: Some(thread_id),
                after: None,
            };
            let value = crate::discussion_acquisition::gh_graphql(
                &self.executable,
                &query,
                scope,
                control,
            )?;
            if let Some(errors) = value
                .get("errors")
                .and_then(Value::as_array)
                .filter(|errors| !errors.is_empty())
            {
                let message = errors
                    .iter()
                    .filter_map(|error| error.get("message").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join(" ");
                if ["permission", "forbidden", "not authorized", "access denied"]
                    .iter()
                    .any(|needle| message.to_ascii_lowercase().contains(needle))
                {
                    return Err(ContractError::new(
                        "AUTH_ACCESS",
                        "GitHub rejected the thread action because current permission changed",
                        "",
                    ));
                }
                return Err(remote_invalid());
            }
            if !value
                .get("data")
                .and_then(|v| v.get(mutation))
                .and_then(|v| v.get("thread"))
                .is_some_and(|thread| !thread.is_null())
            {
                return Err(remote_invalid());
            }
            Ok(thread_id.to_owned())
        })();
        match result {
            Ok(remote_id) => DiscussionSendOutcome::Confirmed {
                remote_id: Some(remote_id),
            },
            Err(error) => classify_write_error(error),
        }
    }
}

fn classify_write_error(error: ContractError) -> DiscussionSendOutcome {
    if error.code == "AUTH_ACCESS" {
        DiscussionSendOutcome::Rejected {
            message: error.message,
        }
    } else {
        DiscussionSendOutcome::Uncertain {
            message: format!("{}: {}", error.code, error.message),
        }
    }
}
fn remote_invalid() -> ContractError {
    ContractError::new(
        "GH_RESPONSE_INVALID",
        "GitHub returned an incomplete discussion response",
        "",
    )
}

pub struct DiscussionPublicationStore {
    db: Connection,
    path: PathBuf,
}

impl DiscussionPublicationStore {
    pub fn open(path: &Path) -> Result<Self, ContractError> {
        let db = Connection::open(path).map_err(db_error)?;
        db.busy_timeout(Duration::from_secs(5)).map_err(db_error)?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS discussion_publication_plans(
                plan_id TEXT PRIMARY KEY NOT NULL, preparation_id TEXT NOT NULL REFERENCES preparations(preparation_id),
                context_id TEXT NOT NULL, fingerprint TEXT NOT NULL,
                review_operation_id TEXT, state TEXT NOT NULL CHECK(state IN ('planned','sending','partial','confirmed','failed','unknown')),
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
            CREATE UNIQUE INDEX IF NOT EXISTS one_unresolved_discussion_plan
                ON discussion_publication_plans(preparation_id) WHERE state IN ('planned','sending','unknown');
            CREATE TABLE IF NOT EXISTS discussion_publication_steps(
                step_id TEXT PRIMARY KEY NOT NULL, plan_id TEXT NOT NULL REFERENCES discussion_publication_plans(plan_id),
                ordinal INTEGER NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('reply','thread_action')),
                draft_id TEXT NOT NULL, draft_revision INTEGER NOT NULL, thread_id TEXT NOT NULL,
                root_comment_id TEXT, body TEXT, action TEXT, observed_resolved INTEGER,
                state TEXT NOT NULL CHECK(state IN ('planned','sending','confirmed','failed','unknown','already_achieved','unsent')),
                remote_id TEXT, failure TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                UNIQUE(plan_id,ordinal), UNIQUE(plan_id,draft_id,draft_revision));
            CREATE TABLE IF NOT EXISTS discussion_published_replies(
                plan_id TEXT NOT NULL REFERENCES discussion_publication_plans(plan_id), preparation_id TEXT NOT NULL,
                context_id TEXT NOT NULL, draft_id TEXT NOT NULL, draft_revision INTEGER NOT NULL, step_id TEXT NOT NULL,
                PRIMARY KEY(plan_id,draft_id,draft_revision));
            CREATE TABLE IF NOT EXISTS discussion_published_thread_actions(
                plan_id TEXT NOT NULL REFERENCES discussion_publication_plans(plan_id), preparation_id TEXT NOT NULL,
                context_id TEXT NOT NULL, draft_id TEXT NOT NULL, draft_revision INTEGER NOT NULL, step_id TEXT NOT NULL,
                PRIMARY KEY(plan_id,draft_id,draft_revision));")
            .map_err(db_error)?;
        Ok(Self {
            db,
            path: path.to_path_buf(),
        })
    }

    /// Saves a fixed operation sequence after checking every selected draft
    /// revision. Replies are ordered first, then thread actions, in selection
    /// order within each group.
    pub fn confirm(
        &mut self,
        preparation_id: &str,
        review_operation_id: Option<&str>,
        replies: &[DiscussionSelection],
        actions: &[DiscussionSelection],
    ) -> Result<DiscussionPlan, ContractError> {
        if replies.is_empty() && actions.is_empty() {
            return Err(invalid("Select at least one reply or thread action", ""));
        }
        let binding = PreparationStore::open(&self.path)?.load(preparation_id)?;
        let snapshot = DraftStore::open(&self.path)?.list(preparation_id)?;
        let mut seen = std::collections::HashSet::new();
        let mut prepared = Vec::new();
        for selection in replies {
            if !seen.insert(selection.id.as_str()) {
                return Err(invalid("Selected draft IDs must be unique", "/replies"));
            }
            let draft = snapshot
                .reply_drafts
                .iter()
                .find(|item| item.id == selection.id)
                .ok_or_else(|| invalid("Selected reply is no longer available", "/replies"))?;
            if draft.revision != selection.revision {
                return Err(stale());
            }
            if draft
                .root_comment_id
                .parse::<u64>()
                .map_or(true, |id| id == 0)
            {
                return Err(invalid(
                    "A reply must target the numeric ID of a top-level review comment",
                    "/replies/root_comment_id",
                ));
            }
            prepared.push((
                "reply",
                draft.id.clone(),
                draft.revision,
                draft.thread_id.clone(),
                Some(draft.root_comment_id.clone()),
                Some(draft.body.clone()),
                None,
                None,
            ));
        }
        for selection in actions {
            if !seen.insert(selection.id.as_str()) {
                return Err(invalid(
                    "Selected draft IDs must be unique",
                    "/thread_actions",
                ));
            }
            let draft = snapshot
                .thread_actions
                .iter()
                .find(|item| item.id == selection.id)
                .ok_or_else(|| {
                    invalid(
                        "Selected thread action is no longer available",
                        "/thread_actions",
                    )
                })?;
            if draft.revision != selection.revision {
                return Err(stale());
            }
            prepared.push((
                "thread_action",
                draft.id.clone(),
                draft.revision,
                draft.thread_id.clone(),
                None,
                None,
                Some(draft.action),
                Some(draft.observed_resolved),
            ));
        }
        let canonical = serde_json::to_vec(&(
            preparation_id,
            &binding.context.context_id,
            review_operation_id,
            &prepared,
        ))
        .map_err(json_error)?;
        let fingerprint = format!("{:x}", Sha256::digest(&canonical));
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let unresolved:Option<String>=tx.query_row("SELECT plan_id FROM discussion_publication_plans WHERE preparation_id=?1 AND state IN ('planned','sending','unknown')",[preparation_id],|r|r.get(0)).optional().map_err(db_error)?;
        if unresolved.is_some() {
            return Err(ContractError::new(
                "DISCUSSION_PLAN_UNRESOLVED",
                "Another discussion publication must be reconciled first",
                "",
            ));
        }
        let attempt: i64 = tx
            .query_row(
                "SELECT count(*) FROM discussion_publication_plans WHERE fingerprint=?1",
                [&fingerprint],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        let id = if attempt == 0 {
            format!("discussion-{}", &fingerprint[..24])
        } else {
            format!("discussion-{}-retry{attempt}", &fingerprint[..24])
        };
        let now = now_ms();
        tx.execute("INSERT INTO discussion_publication_plans(plan_id,preparation_id,context_id,fingerprint,review_operation_id,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'planned',?6,?6)", params![id,preparation_id,binding.context.context_id,fingerprint,review_operation_id,now]).map_err(db_error)?;
        for (ordinal, step) in prepared.iter().enumerate() {
            let step_id = format!("{}-{ordinal:04}", &fingerprint[..16]);
            tx.execute("INSERT INTO discussion_publication_steps(step_id,plan_id,ordinal,kind,draft_id,draft_revision,thread_id,root_comment_id,body,action,observed_resolved,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'planned',?12,?12)", params![step_id,id,ordinal as i64,step.0,step.1,i64::try_from(step.2).map_err(|_|state_error())?,step.3,step.4,step.5,step.6.map(action_name),step.7.map(i64::from),now]).map_err(db_error)?;
        }
        tx.commit().map_err(db_error)?;
        self.plan(&id)
    }

    pub fn plan(&self, id: &str) -> Result<DiscussionPlan, ContractError> {
        let header: Option<(String,String,String,Option<String>,String)> = self.db.query_row("SELECT preparation_id,context_id,fingerprint,review_operation_id,state FROM discussion_publication_plans WHERE plan_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(db_error)?;
        let Some((preparation_id, context_id, fingerprint, review_operation_id, state)) = header
        else {
            return Err(invalid("Discussion plan not found", "/id"));
        };
        let mut q=self.db.prepare("SELECT step_id,ordinal,kind,draft_id,draft_revision,thread_id,root_comment_id,body,action,observed_resolved,state,remote_id,failure FROM discussion_publication_steps WHERE plan_id=?1 ORDER BY ordinal").map_err(db_error)?;
        let rows = q
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, Option<i64>>(9)?,
                    r.get::<_, String>(10)?,
                    r.get::<_, Option<String>>(11)?,
                    r.get::<_, Option<String>>(12)?,
                ))
            })
            .map_err(db_error)?;
        let mut steps = Vec::new();
        for row in rows {
            let (
                id,
                ordinal,
                kind,
                draft_id,
                revision,
                thread_id,
                root,
                body,
                action,
                resolved,
                state,
                remote_id,
                failure,
            ) = row.map_err(db_error)?;
            steps.push(DiscussionStep {
                id,
                ordinal: u64::try_from(ordinal).map_err(|_| state_error())?,
                kind,
                draft_id,
                draft_revision: u64::try_from(revision).map_err(|_| state_error())?,
                thread_id,
                root_comment_id: root,
                body,
                action: action.map(|s| {
                    if s == "resolve" {
                        ThreadAction::Resolve
                    } else {
                        ThreadAction::Reopen
                    }
                }),
                observed_resolved: resolved.map(|v| v != 0),
                state,
                remote_id,
                failure,
            });
        }
        Ok(DiscussionPlan {
            id: id.into(),
            fingerprint,
            preparation_id,
            context_id,
            review_operation_id,
            state,
            steps,
        })
    }

    pub fn latest_plan(
        &self,
        preparation_id: &str,
    ) -> Result<Option<DiscussionPlan>, ContractError> {
        let id:Option<String>=self.db.query_row("SELECT plan_id FROM discussion_publication_plans WHERE preparation_id=?1 ORDER BY created_at DESC LIMIT 1",[preparation_id],|r|r.get(0)).optional().map_err(db_error)?;
        id.map(|id| self.plan(&id)).transpose()
    }

    /// Runs all independently safe operations. If a dependent grouped review
    /// has not been confirmed, discussion steps remain explicitly unsent.
    pub fn publish(
        &mut self,
        plan_id: &str,
        publisher: &dyn DiscussionPublisher,
        control: &OperationControl,
    ) -> Result<DiscussionPlan, ContractError> {
        let plan = self.plan(plan_id)?;
        if plan.state != "planned" {
            return Err(ContractError::new(
                "DISCUSSION_SEND_NOT_ALLOWED",
                "Only a new discussion plan can be sent; reconcile sending or unknown work first",
                "",
            ));
        }
        if let Some(review_id) = plan.review_operation_id.as_deref() {
            let review = crate::ReviewPublicationStore::open(&self.path)?.operation(review_id)?;
            if review.state != "confirmed" {
                if matches!(review.state.as_str(), "failed" | "unknown") {
                    let dependent_state = if review.state == "unknown" {
                        "unknown"
                    } else {
                        "failed"
                    };
                    let tx = self
                        .db
                        .transaction_with_behavior(TransactionBehavior::Immediate)
                        .map_err(db_error)?;
                    tx.execute("UPDATE discussion_publication_steps SET state='unsent',failure=?1,updated_at=?2 WHERE plan_id=?3 AND state='planned'",params![format!("No discussion request was sent because grouped review {} is {}",review_id,review.state),now_ms(),plan_id]).map_err(db_error)?;
                    tx.execute("UPDATE discussion_publication_plans SET state=?1,updated_at=?2 WHERE plan_id=?3 AND state='planned'",params![dependent_state,now_ms(),plan_id]).map_err(db_error)?;
                    tx.commit().map_err(db_error)?;
                    return self.plan(plan_id);
                }
                return Err(ContractError::new(
                    "REVIEW_NOT_CONFIRMED",
                    "Grouped review must be confirmed before replies or thread actions are sent",
                    "",
                ));
            }
        }
        let begin = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let changed=begin.execute("UPDATE discussion_publication_plans SET state='sending',updated_at=?1 WHERE plan_id=?2 AND state='planned'",params![now_ms(),plan_id]).map_err(db_error)?;
        if changed != 1 {
            return Err(ContractError::new(
                "DISCUSSION_SEND_NOT_ALLOWED",
                "Another publisher already started or completed this plan",
                "",
            ));
        }
        begin.commit().map_err(db_error)?;
        let binding = PreparationStore::open(&self.path)?.load(&plan.preparation_id)?;
        for planned in &plan.steps {
            if planned.state != "planned" {
                continue;
            }
            if let Err(error) = control.check() {
                self.set_step(plan_id, &planned.id, "unsent", None, Some(&error.message))?;
                continue;
            }
            let fresh = match publisher.inspect(&plan.preparation_id, &planned.thread_id, control) {
                Ok(state) => state,
                Err(error) => {
                    self.set_step(plan_id, &planned.id, "unsent", None, Some(&error.message))?;
                    continue;
                }
            };
            let already = planned.action.is_some_and(|action| {
                matches!(
                    (action, fresh.resolved),
                    (ThreadAction::Resolve, true) | (ThreadAction::Reopen, false)
                )
            });
            if already {
                self.set_step(plan_id,&planned.id,"already_achieved",None,Some("Action is already achieved remotely; no request was sent and draft remains unpublished"))?;
                continue;
            }
            if !fresh.pull_request_open
                || fresh.head_sha != binding.context.head_sha
                || fresh.base_sha != binding.context.base_sha
            {
                self.set_step(
                    plan_id,
                    &planned.id,
                    "unsent",
                    None,
                    Some("Remote PR state changed; no request was sent"),
                )?;
                continue;
            }
            let allowed = if planned.kind == "reply" {
                fresh.can_reply
            } else {
                match planned.action {
                    Some(ThreadAction::Resolve) => fresh.can_resolve && !fresh.resolved,
                    Some(ThreadAction::Reopen) => fresh.can_reopen && fresh.resolved,
                    None => false,
                }
            };
            if !allowed {
                self.set_step(plan_id,&planned.id,"unsent",None,Some("Current thread permission or state does not allow this operation; no request was sent"))?;
                continue;
            }
            match self.begin_step(plan_id, &planned) {
                Ok(true) => (),
                Ok(false) => continue,
                Err(error) => {
                    self.set_step(
                        plan_id,
                        &planned.id,
                        "unsent",
                        None,
                        Some(&format!("No GitHub request was sent. {}", error.message)),
                    )?;
                    continue;
                }
            }
            let outcome = match planned.kind.as_str() {
                "reply" => publisher.reply(
                    &plan.preparation_id,
                    planned.root_comment_id.as_deref().unwrap_or(""),
                    planned.body.as_deref().unwrap_or(""),
                    control,
                ),
                _ => publisher.thread_action(
                    &plan.preparation_id,
                    &planned.thread_id,
                    planned.action.expect("validated action"),
                    control,
                ),
            };
            match outcome {
                DiscussionSendOutcome::Confirmed { remote_id } => {
                    self.confirm_step(&plan, planned, remote_id.as_deref())?
                }
                DiscussionSendOutcome::Rejected { message } => {
                    self.set_step(plan_id, &planned.id, "failed", None, Some(&message))?
                }
                DiscussionSendOutcome::Uncertain { message } => {
                    self.set_step(plan_id, &planned.id, "unknown", None, Some(&message))?
                }
            }
        }
        self.recompute_plan(plan_id)?;
        self.plan(plan_id)
    }

    fn begin_step(&mut self, plan: &str, step: &DiscussionStep) -> Result<bool, ContractError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let row: Option<(String, String, i64)> = if step.kind == "reply" {
            tx.query_row(
                "SELECT thread_id,root_comment_id,revision FROM reply_drafts WHERE draft_id=?1",
                [&step.draft_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(db_error)?
        } else {
            tx.query_row(
                "SELECT thread_id,action,revision FROM thread_action_drafts WHERE draft_id=?1",
                [&step.draft_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(db_error)?
        };
        let Some((thread, target, revision)) = row else {
            return Err(stale());
        };
        let target_matches = if step.kind == "reply" {
            Some(target) == step.root_comment_id
        } else {
            Some(target) == step.action.map(action_name).map(str::to_owned)
        };
        if thread != step.thread_id
            || !target_matches
            || u64::try_from(revision).map_err(|_| state_error())? != step.draft_revision
        {
            return Err(stale());
        }
        let already_marked: bool = if step.kind == "reply" {
            tx.query_row("SELECT EXISTS(SELECT 1 FROM discussion_published_replies WHERE preparation_id=(SELECT preparation_id FROM discussion_publication_plans WHERE plan_id=?1) AND context_id=(SELECT context_id FROM discussion_publication_plans WHERE plan_id=?1) AND draft_id=?2 AND draft_revision=?3)",params![plan,step.draft_id,revision],|r|r.get(0)).map_err(db_error)?
        } else {
            tx.query_row("SELECT EXISTS(SELECT 1 FROM discussion_published_thread_actions WHERE preparation_id=(SELECT preparation_id FROM discussion_publication_plans WHERE plan_id=?1) AND context_id=(SELECT context_id FROM discussion_publication_plans WHERE plan_id=?1) AND draft_id=?2 AND draft_revision=?3)",params![plan,step.draft_id,revision],|r|r.get(0)).map_err(db_error)?
        };
        if already_marked {
            return Err(stale());
        }
        let changed=tx.execute("UPDATE discussion_publication_steps SET state='sending',updated_at=?1 WHERE plan_id=?2 AND step_id=?3 AND state='planned'",params![now_ms(),plan,step.id]).map_err(db_error)?;
        if changed == 1 {
            tx.execute(
                "UPDATE discussion_publication_plans SET updated_at=?1 WHERE plan_id=?2",
                params![now_ms(), plan],
            )
            .map_err(db_error)?;
        }
        tx.commit().map_err(db_error)?;
        Ok(changed == 1)
    }
    fn set_step(
        &mut self,
        plan: &str,
        step: &str,
        state: &str,
        remote: Option<&str>,
        failure: Option<&str>,
    ) -> Result<(), ContractError> {
        self.db.execute("UPDATE discussion_publication_steps SET state=?1,remote_id=?2,failure=?3,updated_at=?4 WHERE plan_id=?5 AND step_id=?6 AND state IN ('planned','sending')",params![state,remote,failure.map(bound),now_ms(),plan,step]).map_err(db_error)?;
        Ok(())
    }
    fn confirm_step(
        &mut self,
        plan: &DiscussionPlan,
        step: &DiscussionStep,
        remote: Option<&str>,
    ) -> Result<(), ContractError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let changed=tx.execute("UPDATE discussion_publication_steps SET state='confirmed',remote_id=?1,updated_at=?2 WHERE plan_id=?3 AND step_id=?4 AND state='sending'",params![remote,now_ms(),plan.id,step.id]).map_err(db_error)?;
        if changed != 1 {
            return Err(ContractError::new(
                "DISCUSSION_SEND_STATE_INVALID",
                "Step is not awaiting acknowledgment",
                "",
            ));
        }
        let table = if step.kind == "reply" {
            "discussion_published_replies"
        } else {
            "discussion_published_thread_actions"
        };
        let sql = format!(
            "INSERT OR IGNORE INTO {table}(plan_id,preparation_id,context_id,draft_id,draft_revision,step_id) VALUES(?1,?2,?3,?4,?5,?6)"
        );
        tx.execute(
            &sql,
            params![
                plan.id,
                plan.preparation_id,
                plan.context_id,
                step.draft_id,
                i64::try_from(step.draft_revision).map_err(|_| state_error())?,
                step.id
            ],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(())
    }
    fn recompute_plan(&self, id: &str) -> Result<(), ContractError> {
        let counts:(i64,i64,i64,i64)=self.db.query_row("SELECT sum(state='confirmed'),sum(state='failed'),sum(state='unknown'),sum(state IN ('planned','sending')) FROM discussion_publication_steps WHERE plan_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(db_error)?;
        let state = if counts.2 > 0 {
            "unknown"
        } else if counts.3 > 0 {
            "partial"
        } else if counts.1 > 0 && counts.0 == 0 {
            "failed"
        } else if counts.1 > 0 || counts.0 == 0 {
            "partial"
        } else {
            "confirmed"
        };
        self.db
            .execute(
                "UPDATE discussion_publication_plans SET state=?1,updated_at=?2 WHERE plan_id=?3",
                params![state, now_ms(), id],
            )
            .map_err(db_error)?;
        Ok(())
    }

    /// Interrupted requests become unknown, never silently retried.
    pub fn recover_interrupted(&mut self, preparation_id: &str) -> Result<(), ContractError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        tx.execute("UPDATE discussion_publication_steps SET state='unknown',failure='Process ended while awaiting GitHub acknowledgment; inspect remote state before proceeding',updated_at=?1 WHERE state='sending' AND plan_id IN (SELECT plan_id FROM discussion_publication_plans WHERE preparation_id=?2)",params![now_ms(),preparation_id]).map_err(db_error)?;
        tx.execute("UPDATE discussion_publication_plans SET state='unknown',updated_at=?1 WHERE preparation_id=?2 AND state='sending'",params![now_ms(),preparation_id]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(())
    }
}

fn action_name(action: ThreadAction) -> &'static str {
    match action {
        ThreadAction::Resolve => "resolve",
        ThreadAction::Reopen => "reopen",
    }
}
fn stale() -> ContractError {
    ContractError::new(
        "DRAFT_REVISION_STALE",
        "A selected discussion draft changed; reopen Finish Review",
        "/selected_discussions",
    )
}
fn invalid(message: &str, pointer: &str) -> ContractError {
    ContractError::new("DISCUSSION_PLAN_INVALID", message, pointer)
}
fn bound(message: &str) -> String {
    message.chars().take(1024).collect()
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn state_error() -> ContractError {
    ContractError::new(
        "STATE_INVALID",
        "Stored discussion publication state is invalid",
        "",
    )
}
fn db_error(_: rusqlite::Error) -> ContractError {
    ContractError::new(
        "STATE_UNAVAILABLE",
        "Cannot read or write local discussion publication state",
        "",
    )
}
fn json_error(_: serde_json::Error) -> ContractError {
    ContractError::new(
        "STATE_INVALID",
        "Stored discussion publication data is invalid",
        "",
    )
}
