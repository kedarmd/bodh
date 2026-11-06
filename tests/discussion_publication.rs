use bodh::{
    DiscussionPublicationStore, DiscussionPublisher, DiscussionReconciliationOutcome,
    DiscussionSelection, DiscussionSendOutcome, DraftStore, OperationControl, PrIdentity,
    PreparationStore, ReviewContext, ReviewPublicationStore, SelectedDraft, StageReviewRequest,
    ThreadAction, ThreadState, acquire_discussions_with_gh,
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
    std::fs::write(&gh,r##"#!/bin/sh
case "$*" in
  *node\(id:*) printf '%s' '{"data":{"node":{"comments":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"c1"},"nodes":[{"id":"123","databaseId":123,"body":"root text","replyTo":null}]}}}}' ;;
  *) printf '%s' '{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"t1"},"nodes":[{"id":"thread-node-1","path":"src/lib.rs","line":12,"originalLine":8,"diffSide":"RIGHT","isResolved":false,"isOutdated":false,"viewerCanReply":true,"viewerCanResolve":true,"viewerCanUnresolve":true,"comments":{"totalCount":1}}]}}}}}' ;;
esac
"##).unwrap();
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
