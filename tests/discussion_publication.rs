use bodh::{
    DiscussionPublicationStore, DiscussionPublisher, DiscussionReconciliationOutcome,
    DiscussionSelection, DiscussionSendOutcome, DraftStore, OperationControl, PrIdentity,
    PreparationStore, RemoteReviewState, ReviewContext, ReviewEvent, ReviewPublicationStore,
    ReviewPublisher, SelectedDraft, SendOutcome, StageReviewRequest, ThreadAction, ThreadState,
    acquire_discussions_with_gh,
};
use rusqlite::{Connection, params};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> (PathBuf, PathBuf, String) {
    fixture_with_resolved(false)
}

fn fixture_with_resolved(resolved: bool) -> (PathBuf, PathBuf, String) {
    let root = std::env::temp_dir().join(format!(
        "bodh-discussion-publication-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let state = root.join("state.sqlite");
    let binding = PreparationStore::open(&state)
        .unwrap()
        .save(
            &ReviewContext {
                pr: PrIdentity {
                    host: "github.com".into(),
                    owner: "acme".into(),
                    repository: "widget".into(),
                    number: 7,
                },
                context_id: "discussion-publication-context".into(),
                base_sha: "a".repeat(40),
                head_sha: "b".repeat(40),
                merge_base_sha: "c".repeat(40),
            },
            "/checkout",
            "PR",
            None,
            &[],
        )
        .unwrap();
    let gh = root.join("gh");
    let script = r##"#!/bin/sh
case "$*" in
  *node\(id:*) printf '%s' '{"data":{"node":{"comments":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"c1"},"nodes":[{"id":"123","databaseId":123,"body":"root text","replyTo":null}]}}}}' ;;
  *) printf '%s' '{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"t1"},"nodes":[{"id":"thread-node-1","path":"src/lib.rs","line":12,"originalLine":8,"diffSide":"RIGHT","isResolved":__RESOLVED__,"isOutdated":false,"viewerCanReply":true,"viewerCanResolve":true,"viewerCanUnresolve":true,"comments":{"totalCount":1}}]}}}}}' ;;
esac
"##.replace("__RESOLVED__", if resolved { "true" } else { "false" });
    std::fs::write(&gh, script).unwrap();
    let mut permissions = std::fs::metadata(&gh).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&gh, permissions).unwrap();
    let binding = PreparationStore::open(&state)
        .unwrap()
        .load(&binding.preparation_id)
        .unwrap();
    acquire_discussions_with_gh(&binding, &state, &OperationControl::default(), &gh).unwrap();
    (root, state, binding.preparation_id)
}

#[derive(Clone)]
struct ControlledPublisher {
    state: ThreadState,
    outcomes: Arc<Mutex<Vec<DiscussionSendOutcome>>>,
    reconciliations: Arc<Mutex<Vec<DiscussionReconciliationOutcome>>>,
    calls: Arc<Mutex<Vec<String>>>,
}
impl DiscussionPublisher for ControlledPublisher {
    fn inspect(
        &self,
        _: &str,
        _: &str,
        _: &OperationControl,
    ) -> Result<ThreadState, bodh::ContractError> {
        Ok(self.state.clone())
    }
    fn reply(
        &self,
        _: &str,
        root: &str,
        body: &str,
        _: &OperationControl,
    ) -> DiscussionSendOutcome {
        self.calls
            .lock()
            .unwrap()
            .push(format!("reply:{root}:{body}"));
        self.outcomes.lock().unwrap().remove(0)
    }
    fn thread_action(
        &self,
        _: &str,
        thread: &str,
        action: ThreadAction,
        _: &OperationControl,
    ) -> DiscussionSendOutcome {
        self.calls
            .lock()
            .unwrap()
            .push(format!("action:{thread}:{action:?}"));
        self.outcomes.lock().unwrap().remove(0)
    }

    fn reconcile_unknown(
        &self,
        _: &str,
        _: &bodh::DiscussionStep,
        _: i64,
        _: &OperationControl,
    ) -> Result<DiscussionReconciliationOutcome, bodh::ContractError> {
        Ok(self.reconciliations.lock().unwrap().remove(0))
    }
}
fn publisher(
    resolved: bool,
    can_reply: bool,
    can_resolve: bool,
    outcomes: Vec<DiscussionSendOutcome>,
) -> ControlledPublisher {
    ControlledPublisher {
        state: ThreadState {
            pull_request_open: true,
            head_sha: "b".repeat(40),
            base_sha: "a".repeat(40),
            resolved,
            can_reply,
            can_resolve,
            can_reopen: true,
        },
        outcomes: Arc::new(Mutex::new(outcomes)),
        reconciliations: Arc::new(Mutex::new(Vec::new())),
        calls: Arc::new(Mutex::new(Vec::new())),
    }
}

fn controlled_publisher(
    events: Arc<Mutex<Vec<String>>>,
    outcomes: Vec<DiscussionSendOutcome>,
) -> ControlledPublisher {
    ControlledPublisher {
        state: ThreadState {
            pull_request_open: true,
            head_sha: "b".repeat(40),
            base_sha: "a".repeat(40),
            resolved: false,
            can_reply: true,
            can_resolve: true,
            can_reopen: true,
        },
        outcomes: Arc::new(Mutex::new(outcomes)),
        reconciliations: Arc::new(Mutex::new(Vec::new())),
        calls: events,
    }
}

struct OrderedReviewPublisher {
    events: Arc<Mutex<Vec<String>>>,
    outcome: SendOutcome,
}

impl ReviewPublisher for OrderedReviewPublisher {
    fn publish(&self, _: &bodh::ReviewPayload, _: &OperationControl) -> SendOutcome {
        self.events.lock().unwrap().push("grouped-review".into());
        self.outcome.clone()
    }
}

fn grouped_fixture() -> (PathBuf, PathBuf, String, RemoteReviewState) {
    use std::process::Command;

    let root = std::env::temp_dir().join(format!(
        "bodh-grouped-discussion-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let repo = root.join("repo");
    let state = root.join("state.sqlite");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "-b", "main"]);
    std::fs::write(repo.join("src.rs"), "fn value() {\n    1\n}\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "base"]);
    let base_sha = git(&["rev-parse", "HEAD"]);
    std::fs::write(repo.join("src.rs"), "fn value() {\n    2\n}\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "head"]);
    let head_sha = git(&["rev-parse", "HEAD"]);
    let context = ReviewContext {
        pr: PrIdentity {
            host: "github.com".into(),
            owner: "acme".into(),
            repository: "widget".into(),
            number: 7,
        },
        context_id: "grouped-discussion-context".into(),
        base_sha: base_sha.clone(),
        head_sha: head_sha.clone(),
        merge_base_sha: base_sha.clone(),
    };
    let binding = PreparationStore::open(&state)
        .unwrap()
        .save(&context, repo.to_str().unwrap(), "PR", None, &[])
        .unwrap();
    let gh = root.join("gh");
    std::fs::write(
        &gh,
        r##"#!/bin/sh
case "$*" in
  *node\(id:*) printf '%s' '{"data":{"node":{"comments":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"c1"},"nodes":[{"id":"123","databaseId":123,"body":"root text","replyTo":null}]}}}}' ;;
  *) printf '%s' '{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"t1"},"nodes":[{"id":"thread-node-1","path":"src.rs","line":2,"originalLine":2,"diffSide":"RIGHT","isResolved":false,"isOutdated":false,"viewerCanReply":true,"viewerCanResolve":true,"viewerCanUnresolve":true,"comments":{"totalCount":1}}]}}}}}' ;;
esac
"##,
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&gh).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&gh, permissions).unwrap();
    let binding = PreparationStore::open(&state)
        .unwrap()
        .load(&binding.preparation_id)
        .unwrap();
    acquire_discussions_with_gh(&binding, &state, &OperationControl::default(), &gh).unwrap();
    let remote = RemoteReviewState {
        pull_request_id: "PR_node".into(),
        head_sha,
        base_sha,
        state: "OPEN".into(),
        viewer_permission: "WRITE".into(),
    };
    (root, state, binding.preparation_id, remote)
}

fn stage_grouped_intent(state: &Path, prep: &str) -> (String, String, u64) {
    let mut drafts = DraftStore::open(state).unwrap();
    let snapshot = drafts.list(prep).unwrap();
    let reply = drafts
        .save_reply(
            prep,
            None,
            "thread-node-1",
            "123",
            "first reply",
            snapshot.workspace_revision,
            0,
        )
        .unwrap();
    let snapshot = drafts.list(prep).unwrap();
    let action = drafts
        .stage_thread_action(
            prep,
            None,
            "thread-node-1",
            ThreadAction::Resolve,
            snapshot.workspace_revision,
            0,
        )
        .unwrap();
    let mut store = ReviewPublicationStore::open(state).unwrap();
    let intent = store
        .stage(StageReviewRequest {
            state: state.to_path_buf(),
            preparation_id: prep.into(),
            expected_workspace_revision: action.workspace_revision,
            expected_intent_revision: store.intent(prep).unwrap().revision,
            selected: vec![],
            selected_replacements: vec![],
            selected_replies: vec![SelectedDraft {
                id: reply.reply.id.clone(),
                revision: reply.reply.revision,
            }],
            selected_thread_actions: vec![SelectedDraft {
                id: action.action.id.clone(),
                revision: action.action.revision,
            }],
            summary: Some("Review summary".into()),
            event: Some(ReviewEvent::Comment),
        })
        .unwrap();
    (reply.reply.id, action.action.id, intent.revision)
}
fn seed_work(state: &Path, prep: &str) -> (u64, String, String) {
    let mut drafts = DraftStore::open(state).unwrap();
    let mut snapshot = drafts.list(prep).unwrap();
    let reply = drafts
        .save_reply(
            prep,
            None,
            "thread-node-1",
            "123",
            "first reply",
            snapshot.workspace_revision,
            0,
        )
        .unwrap();
    snapshot = drafts.list(prep).unwrap();
    let action = drafts
        .stage_thread_action(
            prep,
            None,
            "thread-node-1",
            ThreadAction::Resolve,
            snapshot.workspace_revision,
            0,
        )
        .unwrap();
    (action.workspace_revision, reply.reply.id, action.action.id)
}

fn stage_intent(state: &Path, prep: &str, reply_ids: &[&str], action_ids: &[&str]) -> u64 {
    let snapshot = DraftStore::open(state).unwrap().list(prep).unwrap();
    let mut store = ReviewPublicationStore::open(state).unwrap();
    let expected_intent_revision = store.intent(prep).unwrap().revision;
    store
        .stage(StageReviewRequest {
            state: state.to_path_buf(),
            preparation_id: prep.to_owned(),
            expected_workspace_revision: snapshot.workspace_revision,
            expected_intent_revision,
            selected: vec![],
            selected_replacements: vec![],
            selected_replies: reply_ids
                .iter()
                .map(|id| SelectedDraft {
                    id: (*id).into(),
                    revision: 1,
                })
                .collect(),
            selected_thread_actions: action_ids
                .iter()
                .map(|id| SelectedDraft {
                    id: (*id).into(),
                    revision: 1,
                })
                .collect(),
            summary: None,
            event: None,
        })
        .unwrap()
        .revision
}

#[test]
fn independent_outcomes_confirm_only_accepted_steps_and_keep_reply_payload_exact() {
    let (root, state, prep) = fixture();
    let (_, reply_id, action_id) = seed_work(&state, &prep);
    let intent_revision = stage_intent(&state, &prep, &[&reply_id], &[&action_id]);
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = store
        .confirm(
            &prep,
            intent_revision,
            None,
            &[DiscussionSelection {
                id: reply_id.clone(),
                revision: 1,
            }],
            &[DiscussionSelection {
                id: action_id.clone(),
                revision: 1,
            }],
        )
        .unwrap();
    assert_eq!(
        plan.steps
            .iter()
            .map(|s| s.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["reply", "thread_action"]
    );
    let sender = publisher(
        false,
        true,
        true,
        vec![
            DiscussionSendOutcome::Rejected {
                message: "temporary rejection".into(),
            },
            DiscussionSendOutcome::Confirmed {
                remote_id: Some("thread-node-1".into()),
            },
        ],
    );
    let result = store
        .publish(&plan.id, &sender, &OperationControl::default())
        .unwrap();
    assert_eq!(result.state, "partial");
    assert_eq!(
        sender.calls.lock().unwrap().as_slice(),
        ["reply:123:first reply", "action:thread-node-1:Resolve"]
    );
    let conn = Connection::open(&state).unwrap();
    let replies: i64 = conn
        .query_row(
            "SELECT count(*) FROM discussion_published_replies WHERE draft_id=?1",
            [reply_id],
            |r| r.get(0),
        )
        .unwrap();
    let actions: i64 = conn
        .query_row(
            "SELECT count(*) FROM discussion_published_thread_actions WHERE draft_id=?1",
            [action_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!((replies, actions), (0, 1));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn reply_only_and_thread_only_plans_confirm_only_real_selected_operations() {
    let (reply_root, reply_state, reply_prep) = fixture();
    let (_, reply_id, unselected_action_id) = seed_work(&reply_state, &reply_prep);
    let reply_intent = stage_intent(&reply_state, &reply_prep, &[&reply_id], &[]);
    let mut reply_store = DiscussionPublicationStore::open(&reply_state).unwrap();
    let reply_plan = reply_store
        .confirm(
            &reply_prep,
            reply_intent,
            None,
            &[DiscussionSelection {
                id: reply_id.clone(),
                revision: 1,
            }],
            &[],
        )
        .unwrap();
    assert_eq!(reply_plan.review_operation_id, None);
    assert_eq!(reply_plan.steps.len(), 1);
    assert_eq!(reply_plan.steps[0].kind, "reply");
    assert_eq!(reply_plan.steps[0].draft_id, reply_id);
    assert_ne!(reply_plan.steps[0].draft_id, unselected_action_id);
    let reply_sender = publisher(
        false,
        true,
        true,
        vec![DiscussionSendOutcome::Confirmed {
            remote_id: Some("reply-comment-456".into()),
        }],
    );
    let sent_reply = reply_store
        .publish(&reply_plan.id, &reply_sender, &OperationControl::default())
        .unwrap();
    assert_eq!(sent_reply.state, "confirmed");
    assert_eq!(
        reply_sender.calls.lock().unwrap().as_slice(),
        ["reply:123:first reply"]
    );
    let _ = std::fs::remove_dir_all(reply_root);

    let (action_root, action_state, action_prep) = fixture();
    let (_, unselected_reply_id, action_id) = seed_work(&action_state, &action_prep);
    let action_intent = stage_intent(&action_state, &action_prep, &[], &[&action_id]);
    let mut action_store = DiscussionPublicationStore::open(&action_state).unwrap();
    let action_plan = action_store
        .confirm(
            &action_prep,
            action_intent,
            None,
            &[],
            &[DiscussionSelection {
                id: action_id.clone(),
                revision: 1,
            }],
        )
        .unwrap();
    assert_eq!(action_plan.review_operation_id, None);
    assert_eq!(action_plan.steps.len(), 1);
    assert_eq!(action_plan.steps[0].kind, "thread_action");
    assert_eq!(action_plan.steps[0].draft_id, action_id);
    assert_ne!(action_plan.steps[0].draft_id, unselected_reply_id);
    let action_sender = publisher(
        false,
        true,
        true,
        vec![DiscussionSendOutcome::Confirmed {
            remote_id: Some("thread-node-1".into()),
        }],
    );
    let sent_action = action_store
        .publish(
            &action_plan.id,
            &action_sender,
            &OperationControl::default(),
        )
        .unwrap();
    assert_eq!(sent_action.state, "confirmed");
    assert_eq!(
        action_sender.calls.lock().unwrap().as_slice(),
        ["action:thread-node-1:Resolve"]
    );
    let _ = std::fs::remove_dir_all(action_root);
}

#[test]
fn permission_loss_and_already_achieved_action_send_nothing_and_never_claim_publication() {
    let (root, state, prep) = fixture();
    let (_, reply_id, action_id) = seed_work(&state, &prep);
    let intent_revision = stage_intent(&state, &prep, &[&reply_id], &[&action_id]);
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = store
        .confirm(
            &prep,
            intent_revision,
            None,
            &[DiscussionSelection {
                id: reply_id.clone(),
                revision: 1,
            }],
            &[DiscussionSelection {
                id: action_id.clone(),
                revision: 1,
            }],
        )
        .unwrap();
    let sender = publisher(true, false, false, vec![]);
    let result = store
        .publish(&plan.id, &sender, &OperationControl::default())
        .unwrap();
    assert_eq!(result.state, "partial");
    assert!(sender.calls.lock().unwrap().is_empty());
    assert_eq!(
        result
            .steps
            .iter()
            .map(|s| s.state.as_str())
            .collect::<Vec<_>>(),
        vec!["unsent", "already_achieved"]
    );
    let conn = Connection::open(&state).unwrap();
    let marker_count:i64=conn.query_row("SELECT (SELECT count(*) FROM discussion_published_replies)+(SELECT count(*) FROM discussion_published_thread_actions)",[],|r|r.get(0)).unwrap();
    assert_eq!(marker_count, 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn changed_draft_revision_is_detected_after_confirmation_before_any_write() {
    let (root, state, prep) = fixture();
    let (_, reply_id, _) = seed_work(&state, &prep);
    let intent_revision = stage_intent(&state, &prep, &[&reply_id], &[]);
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = store
        .confirm(
            &prep,
            intent_revision,
            None,
            &[DiscussionSelection {
                id: reply_id.clone(),
                revision: 1,
            }],
            &[],
        )
        .unwrap();
    let conn = Connection::open(&state).unwrap();
    conn.execute(
        "UPDATE reply_drafts SET revision=2,body='changed' WHERE draft_id=?1",
        params![reply_id],
    )
    .unwrap();
    let sender = publisher(false, true, true, vec![]);
    let result = store
        .publish(&plan.id, &sender, &OperationControl::default())
        .unwrap();
    assert_eq!(result.steps[0].state, "unsent");
    assert!(sender.calls.lock().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn interrupted_sending_step_becomes_unknown_and_cannot_be_retried() {
    let (root, state, prep) = fixture();
    let (_, reply_id, _) = seed_work(&state, &prep);
    let intent_revision = stage_intent(&state, &prep, &[&reply_id], &[]);
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = store
        .confirm(
            &prep,
            intent_revision,
            None,
            &[DiscussionSelection {
                id: reply_id,
                revision: 1,
            }],
            &[],
        )
        .unwrap();
    let conn = Connection::open(&state).unwrap();
    conn.execute(
        "UPDATE discussion_publication_plans SET state='sending' WHERE plan_id=?1",
        [&plan.id],
    )
    .unwrap();
    conn.execute(
        "UPDATE discussion_publication_steps SET state='sending' WHERE plan_id=?1",
        [&plan.id],
    )
    .unwrap();
    store.recover_interrupted(&prep).unwrap();
    let recovered = store.plan(&plan.id).unwrap();
    assert_eq!(recovered.state, "unknown");
    assert_eq!(recovered.steps[0].state, "unknown");
    let sender = publisher(false, true, true, vec![]);
    let error = store
        .publish(&plan.id, &sender, &OperationControl::default())
        .unwrap_err();
    assert_eq!(error.code, "DISCUSSION_SEND_NOT_ALLOWED");
    assert!(sender.calls.lock().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn grouped_review_is_confirmed_before_dependent_operations_and_order_survives_reopen() {
    let (root, state, prep, remote) = grouped_fixture();
    let (reply_id, action_id, intent_revision) = stage_grouped_intent(&state, &prep);
    let mut review_store = ReviewPublicationStore::open(&state).unwrap();
    let review = review_store
        .confirm(
            &prep,
            &state,
            intent_revision,
            &remote,
            &OperationControl::default(),
        )
        .unwrap();
    let mut discussion_store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = discussion_store
        .confirm(
            &prep,
            intent_revision,
            Some(&review.id),
            &[DiscussionSelection {
                id: reply_id.clone(),
                revision: 1,
            }],
            &[DiscussionSelection {
                id: action_id.clone(),
                revision: 1,
            }],
        )
        .unwrap();
    assert_eq!(
        plan.review_operation_id.as_deref(),
        Some(review.id.as_str())
    );
    assert_eq!(
        plan.steps
            .iter()
            .map(|step| step.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["reply", "thread_action"]
    );
    assert_eq!(plan.steps[0].draft_id, reply_id);
    assert_eq!(plan.steps[1].draft_id, action_id);

    let events = Arc::new(Mutex::new(Vec::new()));
    let premature_sender = controlled_publisher(events.clone(), vec![]);
    assert_eq!(
        discussion_store
            .publish(&plan.id, &premature_sender, &OperationControl::default())
            .unwrap_err()
            .code,
        "REVIEW_NOT_CONFIRMED"
    );
    assert!(events.lock().unwrap().is_empty());
    assert_eq!(discussion_store.plan(&plan.id).unwrap().state, "planned");

    let review_sender = OrderedReviewPublisher {
        events: events.clone(),
        outcome: SendOutcome::Confirmed {
            remote_id: "review-1".into(),
            url: Some("https://example.invalid/review/1".into()),
        },
    };
    let published_review = review_store
        .publish_confirmed(
            &review.id,
            &remote,
            &review_sender,
            &OperationControl::default(),
        )
        .unwrap();
    assert_eq!(published_review.state, "confirmed");

    let mut sender = controlled_publisher(
        events.clone(),
        vec![
            DiscussionSendOutcome::Confirmed {
                remote_id: Some("reply-1".into()),
            },
            DiscussionSendOutcome::Confirmed {
                remote_id: Some("thread-node-1".into()),
            },
        ],
    );
    sender.state.head_sha = remote.head_sha.clone();
    sender.state.base_sha = remote.base_sha.clone();
    let published_discussion = discussion_store
        .publish(&plan.id, &sender, &OperationControl::default())
        .unwrap();
    assert_eq!(
        published_discussion.state,
        "confirmed",
        "steps: {:?}; events: {:?}",
        published_discussion.steps,
        events.lock().unwrap()
    );
    assert_eq!(
        events.lock().unwrap().as_slice(),
        [
            "grouped-review",
            "reply:123:first reply",
            "action:thread-node-1:Resolve"
        ]
    );

    drop(discussion_store);
    let resumed = DiscussionPublicationStore::open(&state)
        .unwrap()
        .plan(&plan.id)
        .unwrap();
    assert_eq!(resumed.state, "confirmed");
    assert_eq!(
        resumed
            .steps
            .iter()
            .map(|step| step.state.as_str())
            .collect::<Vec<_>>(),
        vec!["confirmed", "confirmed"]
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rejected_or_uncertain_grouped_review_keeps_dependent_steps_unsent_after_restart() {
    for (outcome, review_state, plan_state) in [
        (
            SendOutcome::Rejected {
                message: "permission denied".into(),
            },
            "failed",
            "failed",
        ),
        (
            SendOutcome::Uncertain {
                message: "timed out after request".into(),
            },
            "unknown",
            "unknown",
        ),
    ] {
        let (root, state, prep, remote) = grouped_fixture();
        let (reply_id, action_id, intent_revision) = stage_grouped_intent(&state, &prep);
        let mut review_store = ReviewPublicationStore::open(&state).unwrap();
        let review = review_store
            .confirm(
                &prep,
                &state,
                intent_revision,
                &remote,
                &OperationControl::default(),
            )
            .unwrap();
        let mut discussion_store = DiscussionPublicationStore::open(&state).unwrap();
        let plan = discussion_store
            .confirm(
                &prep,
                intent_revision,
                Some(&review.id),
                &[DiscussionSelection {
                    id: reply_id,
                    revision: 1,
                }],
                &[DiscussionSelection {
                    id: action_id,
                    revision: 1,
                }],
            )
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let review_sender = OrderedReviewPublisher {
            events: events.clone(),
            outcome,
        };
        let failed_review = review_store
            .publish_confirmed(
                &review.id,
                &remote,
                &review_sender,
                &OperationControl::default(),
            )
            .unwrap();
        assert_eq!(failed_review.state, review_state);

        let sender = controlled_publisher(events.clone(), vec![]);
        let dependent = discussion_store
            .publish(&plan.id, &sender, &OperationControl::default())
            .unwrap();
        assert_eq!(dependent.state, plan_state);
        assert_eq!(
            dependent
                .steps
                .iter()
                .map(|step| step.state.as_str())
                .collect::<Vec<_>>(),
            vec!["unsent", "unsent"]
        );
        assert_eq!(events.lock().unwrap().as_slice(), ["grouped-review"]);

        drop(discussion_store);
        let mut resumed = DiscussionPublicationStore::open(&state).unwrap();
        let recovered = resumed.plan(&plan.id).unwrap();
        assert_eq!(recovered.state, plan_state);
        assert_eq!(
            recovered
                .steps
                .iter()
                .map(|step| step.state.as_str())
                .collect::<Vec<_>>(),
            vec!["unsent", "unsent"]
        );
        assert_eq!(
            resumed
                .publish(&plan.id, &sender, &OperationControl::default())
                .unwrap_err()
                .code,
            "DISCUSSION_SEND_NOT_ALLOWED"
        );
        assert_eq!(events.lock().unwrap().as_slice(), ["grouped-review"]);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn unique_read_only_reply_match_confirms_only_the_exact_unknown_step() {
    let (root, state, prep) = fixture();
    let (_, reply_id, _) = seed_work(&state, &prep);
    let intent_revision = stage_intent(&state, &prep, &[&reply_id], &[]);
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = store
        .confirm(
            &prep,
            intent_revision,
            None,
            &[DiscussionSelection {
                id: reply_id.clone(),
                revision: 1,
            }],
            &[],
        )
        .unwrap();
    let sender = publisher(
        false,
        true,
        true,
        vec![DiscussionSendOutcome::Uncertain {
            message: "connection lost after request".into(),
        }],
    );
    let sent = store
        .publish(&plan.id, &sender, &OperationControl::default())
        .unwrap();
    assert_eq!(sent.state, "unknown");
    assert!(sent.steps[0].attempt_started_at.is_some());
    sender
        .reconciliations
        .lock()
        .unwrap()
        .push(DiscussionReconciliationOutcome::Confirmed {
            remote_id: "456".into(),
        });

    let reconciled = store
        .reconcile_unknown(&plan.id, &sender, &OperationControl::default())
        .unwrap();

    assert_eq!(reconciled.state, "confirmed");
    assert_eq!(reconciled.steps[0].state, "confirmed");
    assert_eq!(reconciled.steps[0].remote_id.as_deref(), Some("456"));
    let conn = Connection::open(&state).unwrap();
    let published: i64 = conn
        .query_row(
            "SELECT count(*) FROM discussion_published_replies WHERE draft_id=?1",
            [reply_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(published, 1);
    assert!(
        sender
            .calls
            .lock()
            .unwrap()
            .as_slice()
            .eq(&["reply:123:first reply"])
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn desired_thread_state_is_recorded_as_already_achieved_not_published_by_reviewer() {
    let (root, state, prep) = fixture();
    let (_, _, action_id) = seed_work(&state, &prep);
    let intent_revision = stage_intent(&state, &prep, &[], &[&action_id]);
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = store
        .confirm(
            &prep,
            intent_revision,
            None,
            &[],
            &[DiscussionSelection {
                id: action_id.clone(),
                revision: 1,
            }],
        )
        .unwrap();
    let sender = publisher(
        false,
        true,
        true,
        vec![DiscussionSendOutcome::Uncertain {
            message: "connection lost after request".into(),
        }],
    );
    let sent = store
        .publish(&plan.id, &sender, &OperationControl::default())
        .unwrap();
    assert_eq!(sent.steps[0].state, "unknown");
    sender
        .reconciliations
        .lock()
        .unwrap()
        .push(DiscussionReconciliationOutcome::AlreadyAchieved);

    let reconciled = store
        .reconcile_unknown(&plan.id, &sender, &OperationControl::default())
        .unwrap();

    assert_eq!(reconciled.state, "partial");
    assert_eq!(reconciled.steps[0].state, "already_achieved");
    assert!(reconciled.steps[0].remote_id.is_none());
    let conn = Connection::open(&state).unwrap();
    let published: i64 = conn
        .query_row(
            "SELECT count(*) FROM discussion_published_thread_actions WHERE draft_id=?1",
            [action_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(published, 0);
    assert_eq!(
        sender.calls.lock().unwrap().as_slice(),
        ["action:thread-node-1:Resolve"]
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn ambiguous_or_unavailable_read_only_evidence_keeps_the_step_unknown() {
    let (root, state, prep) = fixture();
    let (_, reply_id, _) = seed_work(&state, &prep);
    let intent_revision = stage_intent(&state, &prep, &[&reply_id], &[]);
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = store
        .confirm(
            &prep,
            intent_revision,
            None,
            &[DiscussionSelection {
                id: reply_id,
                revision: 1,
            }],
            &[],
        )
        .unwrap();
    let sender = publisher(
        false,
        true,
        true,
        vec![DiscussionSendOutcome::Uncertain {
            message: "connection lost after request".into(),
        }],
    );
    store
        .publish(&plan.id, &sender, &OperationControl::default())
        .unwrap();
    sender
        .reconciliations
        .lock()
        .unwrap()
        .push(DiscussionReconciliationOutcome::Unknown {
            message:
                "Two matching comments were observed; authorship cannot be established uniquely"
                    .into(),
        });

    let reconciled = store
        .reconcile_unknown(&plan.id, &sender, &OperationControl::default())
        .unwrap();

    assert_eq!(reconciled.state, "unknown");
    assert_eq!(reconciled.steps[0].state, "unknown");
    assert!(
        reconciled.steps[0]
            .failure
            .as_deref()
            .unwrap()
            .contains("cannot be established uniquely")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_partial_plan_can_retry_only_its_failed_steps_as_a_new_plan() {
    let (root, state, prep) = fixture();
    let (workspace_revision, reply_id, action_id) = seed_work(&state, &prep);
    let intent_revision = stage_intent(&state, &prep, &[&reply_id], &[&action_id]);
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = store
        .confirm(
            &prep,
            intent_revision,
            None,
            &[DiscussionSelection {
                id: reply_id,
                revision: 1,
            }],
            &[DiscussionSelection {
                id: action_id.clone(),
                revision: 1,
            }],
        )
        .unwrap();
    let first_sender = controlled_publisher(
        Arc::new(Mutex::new(Vec::new())),
        vec![
            DiscussionSendOutcome::Confirmed {
                remote_id: Some("reply-1".into()),
            },
            DiscussionSendOutcome::Rejected {
                message: "thread action was rejected".into(),
            },
        ],
    );
    let first = store
        .publish(&plan.id, &first_sender, &OperationControl::default())
        .unwrap();
    assert_eq!(first.state, "partial");
    assert_eq!(
        first
            .steps
            .iter()
            .map(|step| step.state.as_str())
            .collect::<Vec<_>>(),
        ["confirmed", "failed"]
    );

    let retry_sender = controlled_publisher(
        Arc::new(Mutex::new(Vec::new())),
        vec![DiscussionSendOutcome::Confirmed {
            remote_id: Some("thread-node-1".into()),
        }],
    );
    let retry = store
        .retry_failed_plan(&first.id, &retry_sender, &OperationControl::default())
        .unwrap();
    assert_ne!(retry.id, first.id);
    assert_eq!(retry.state, "planned");
    assert_eq!(retry.steps.len(), 1);
    assert_eq!(retry.steps[0].kind, "thread_action");
    assert_eq!(retry.steps[0].draft_id, action_id);
    assert_eq!(retry.steps[0].state, "planned");
    assert_eq!(retry.review_operation_id, first.review_operation_id);

    let history = DiscussionPublicationStore::open(&state)
        .unwrap()
        .plan_history(&prep)
        .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0], first);
    assert_eq!(history[1], retry);
    assert_eq!(
        workspace_revision,
        DraftStore::open(&state)
            .unwrap()
            .list(&prep)
            .unwrap()
            .workspace_revision
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn retry_includes_eligible_unsent_steps_and_never_repeats_confirmed_steps() {
    let (root, state, prep) = fixture();
    let (_, reply_id, action_id) = seed_work(&state, &prep);
    let intent_revision = stage_intent(&state, &prep, &[&reply_id], &[&action_id]);
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let original = store
        .confirm(
            &prep,
            intent_revision,
            None,
            &[DiscussionSelection {
                id: reply_id,
                revision: 1,
            }],
            &[DiscussionSelection {
                id: action_id.clone(),
                revision: 1,
            }],
        )
        .unwrap();
    let mut first_sender = controlled_publisher(
        Arc::new(Mutex::new(Vec::new())),
        vec![DiscussionSendOutcome::Confirmed {
            remote_id: Some("reply-1".into()),
        }],
    );
    first_sender.state.can_resolve = false;
    let first = store
        .publish(&original.id, &first_sender, &OperationControl::default())
        .unwrap();
    assert_eq!(first.state, "partial");
    assert_eq!(
        first
            .steps
            .iter()
            .map(|step| step.state.as_str())
            .collect::<Vec<_>>(),
        ["confirmed", "unsent"]
    );
    drop(store);
    Connection::open(&state)
        .unwrap()
        .execute(
            "UPDATE discussion_publication_plans SET state='confirmed' WHERE plan_id=?1",
            [&first.id],
        )
        .unwrap();
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    assert_eq!(store.plan(&first.id).unwrap().state, "partial");
    let retry_sender = controlled_publisher(
        Arc::new(Mutex::new(Vec::new())),
        vec![DiscussionSendOutcome::Confirmed {
            remote_id: Some("thread-node-1".into()),
        }],
    );
    let retry = store
        .retry_failed_plan(&first.id, &retry_sender, &OperationControl::default())
        .unwrap();
    assert_eq!(retry.steps.len(), 1);
    assert_eq!(retry.steps[0].state, "planned");
    assert_eq!(retry.steps[0].kind, "thread_action");
    assert_eq!(retry.steps[0].draft_id, action_id);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn failed_replies_can_be_retried_after_restart_and_keep_the_exact_payload() {
    let (root, state, prep) = fixture();
    let (_, reply_id, _) = seed_work(&state, &prep);
    let intent_revision = stage_intent(&state, &prep, &[&reply_id], &[]);
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let original = store
        .confirm(
            &prep,
            intent_revision,
            None,
            &[DiscussionSelection {
                id: reply_id.clone(),
                revision: 1,
            }],
            &[],
        )
        .unwrap();
    let reject = controlled_publisher(
        Arc::new(Mutex::new(Vec::new())),
        vec![DiscussionSendOutcome::Rejected {
            message: "reply rejected".into(),
        }],
    );
    let failed = store
        .publish(&original.id, &reject, &OperationControl::default())
        .unwrap();
    assert_eq!(failed.state, "failed");

    let sender = controlled_publisher(
        Arc::new(Mutex::new(Vec::new())),
        vec![DiscussionSendOutcome::Confirmed {
            remote_id: Some("reply-2".into()),
        }],
    );
    let retry = store
        .retry_failed_plan(&failed.id, &sender, &OperationControl::default())
        .unwrap();
    assert_eq!(retry.retry_of.as_deref(), Some(failed.id.as_str()));
    assert_eq!(retry.steps.len(), 1);
    assert_eq!(retry.steps[0].kind, "reply");
    assert_eq!(retry.steps[0].root_comment_id.as_deref(), Some("123"));
    assert_eq!(retry.steps[0].body.as_deref(), Some("first reply"));

    let duplicate = store
        .retry_failed_plan(&failed.id, &sender, &OperationControl::default())
        .unwrap_err();
    assert_eq!(duplicate.code, "DISCUSSION_RETRY_NOT_ALLOWED");

    drop(store);
    let mut reopened = DiscussionPublicationStore::open(&state).unwrap();
    let history = reopened.plan_history(&prep).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].state, "failed");
    assert_eq!(history[1].retry_of.as_deref(), Some(failed.id.as_str()));
    let published = reopened
        .publish(&retry.id, &sender, &OperationControl::default())
        .unwrap();
    assert_eq!(published.state, "confirmed");
    assert_eq!(published.steps[0].body.as_deref(), Some("first reply"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unknown_or_stale_discussion_work_cannot_be_retried() {
    let (root, state, prep) = fixture();
    let (_, reply_id, _) = seed_work(&state, &prep);
    let intent_revision = stage_intent(&state, &prep, &[&reply_id], &[]);
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = store
        .confirm(
            &prep,
            intent_revision,
            None,
            &[DiscussionSelection {
                id: reply_id.clone(),
                revision: 1,
            }],
            &[],
        )
        .unwrap();
    let unknown_sender = controlled_publisher(
        Arc::new(Mutex::new(Vec::new())),
        vec![DiscussionSendOutcome::Uncertain {
            message: "reply acknowledgment was lost".into(),
        }],
    );
    let unknown = store
        .publish(&plan.id, &unknown_sender, &OperationControl::default())
        .unwrap();
    assert_eq!(unknown.state, "unknown");
    let fresh = controlled_publisher(Arc::new(Mutex::new(Vec::new())), vec![]);
    assert_eq!(
        store
            .retry_failed_plan(&unknown.id, &fresh, &OperationControl::default())
            .unwrap_err()
            .code,
        "DISCUSSION_RETRY_NOT_ALLOWED"
    );

    let (root2, state2, prep2) = fixture();
    let (_, reply_id2, _) = seed_work(&state2, &prep2);
    let intent_revision2 = stage_intent(&state2, &prep2, &[&reply_id2], &[]);
    let mut stale_store = DiscussionPublicationStore::open(&state2).unwrap();
    let plan2 = stale_store
        .confirm(
            &prep2,
            intent_revision2,
            None,
            &[DiscussionSelection {
                id: reply_id2,
                revision: 1,
            }],
            &[],
        )
        .unwrap();
    let reject = controlled_publisher(
        Arc::new(Mutex::new(Vec::new())),
        vec![DiscussionSendOutcome::Rejected {
            message: "reply rejected".into(),
        }],
    );
    let failed = stale_store
        .publish(&plan2.id, &reject, &OperationControl::default())
        .unwrap();
    let mut stale_head = controlled_publisher(Arc::new(Mutex::new(Vec::new())), vec![]);
    stale_head.state.head_sha = "f".repeat(40);
    assert_eq!(
        stale_store
            .retry_failed_plan(&failed.id, &stale_head, &OperationControl::default())
            .unwrap_err()
            .code,
        "DISCUSSION_RETRY_STALE"
    );
    let mut no_permission = controlled_publisher(Arc::new(Mutex::new(Vec::new())), vec![]);
    no_permission.state.can_reply = false;
    assert_eq!(
        stale_store
            .retry_failed_plan(&failed.id, &no_permission, &OperationControl::default())
            .unwrap_err()
            .code,
        "DISCUSSION_RETRY_STALE"
    );
    let snapshot = DraftStore::open(&state2).unwrap().list(&prep2).unwrap();
    DraftStore::open(&state2)
        .unwrap()
        .save_reply(
            &prep2,
            Some(&failed.steps[0].draft_id),
            &failed.steps[0].thread_id,
            failed.steps[0].root_comment_id.as_deref().unwrap(),
            "edited after rejection",
            snapshot.workspace_revision,
            failed.steps[0].draft_revision,
        )
        .unwrap();
    assert_eq!(
        stale_store
            .retry_failed_plan(&failed.id, &fresh, &OperationControl::default())
            .unwrap_err()
            .code,
        "DISCUSSION_RETRY_STALE"
    );
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
}

#[test]
fn a_failed_reopen_action_can_be_retried_after_a_fresh_permission_check() {
    let (root, state, prep) = fixture_with_resolved(true);
    let mut drafts = DraftStore::open(&state).unwrap();
    let snapshot = drafts.list(&prep).unwrap();
    let action = drafts
        .stage_thread_action(
            &prep,
            None,
            "thread-node-1",
            ThreadAction::Reopen,
            snapshot.workspace_revision,
            0,
        )
        .unwrap();
    let intent_revision = stage_intent(&state, &prep, &[], &[&action.action.id]);
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = store
        .confirm(
            &prep,
            intent_revision,
            None,
            &[],
            &[DiscussionSelection {
                id: action.action.id,
                revision: 1,
            }],
        )
        .unwrap();
    let mut reject = controlled_publisher(
        Arc::new(Mutex::new(Vec::new())),
        vec![DiscussionSendOutcome::Rejected {
            message: "reopen rejected".into(),
        }],
    );
    reject.state.resolved = true;
    let failed = store
        .publish(&plan.id, &reject, &OperationControl::default())
        .unwrap();
    assert_eq!(failed.steps[0].action, Some(ThreadAction::Reopen));
    let retry_sender = ControlledPublisher {
        state: ThreadState {
            pull_request_open: true,
            head_sha: "b".repeat(40),
            base_sha: "a".repeat(40),
            resolved: true,
            can_reply: true,
            can_resolve: true,
            can_reopen: true,
        },
        outcomes: Arc::new(Mutex::new(vec![DiscussionSendOutcome::Confirmed {
            remote_id: Some("thread-node-1".into()),
        }])),
        reconciliations: Arc::new(Mutex::new(Vec::new())),
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let retry = store
        .retry_failed_plan(&failed.id, &retry_sender, &OperationControl::default())
        .unwrap();
    assert_eq!(retry.steps[0].action, Some(ThreadAction::Reopen));
    let _ = std::fs::remove_dir_all(root);
}
