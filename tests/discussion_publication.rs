use bodh::{
    DiscussionPublicationStore, DiscussionPublisher, DiscussionSelection, DiscussionSendOutcome,
    DraftStore, OperationControl, PrIdentity, PreparationStore, ReviewContext, ThreadAction,
    ThreadState, acquire_discussions_with_gh,
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

#[test]
fn independent_outcomes_confirm_only_accepted_steps_and_keep_reply_payload_exact() {
    let (root, state, prep) = fixture();
    let (_, reply_id, action_id) = seed_work(&state, &prep);
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = store
        .confirm(
            &prep,
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
fn permission_loss_and_already_achieved_action_send_nothing_and_never_claim_publication() {
    let (root, state, prep) = fixture();
    let (_, reply_id, action_id) = seed_work(&state, &prep);
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = store
        .confirm(
            &prep,
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
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = store
        .confirm(
            &prep,
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
    let mut store = DiscussionPublicationStore::open(&state).unwrap();
    let plan = store
        .confirm(
            &prep,
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
