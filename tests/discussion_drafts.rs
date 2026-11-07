use bodh::{
    DraftStore, OperationControl, PrIdentity, PreparationStore, ReviewContext,
    acquire_discussions_with_gh,
};
use std::{
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn seed() -> (PathBuf, PathBuf, String) {
    let root = std::env::temp_dir().join(format!(
        "bodh-thread-drafts-{}-{}",
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
                context_id: "thread-context".into(),
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
    (root, state, binding.preparation_id)
}

fn fake_gh(root: &Path, reply_allowed: bool, resolve_allowed: bool) -> PathBuf {
    let path = root.join("gh");
    let body = format!(
        r#"#!/bin/sh
case "$*" in
  *node\(id:*) printf '%s' '{{"data":{{"node":{{"comments":{{"totalCount":1,"pageInfo":{{"hasNextPage":false,"endCursor":"c1"}},"nodes":[{{"id":"comment-root","databaseId":"101","body":"root text","replyTo":null}}]}}}}}}}}' ;;
  *) printf '%s' '{{"data":{{"repository":{{"pullRequest":{{"reviewThreads":{{"totalCount":1,"pageInfo":{{"hasNextPage":false,"endCursor":"t1"}},"nodes":[{{"id":"thread-1","path":"src/lib.rs","line":12,"originalLine":8,"diffSide":"RIGHT","isResolved":false,"isOutdated":false,"viewerCanReply":{},"viewerCanResolve":{},"viewerCanUnresolve":true,"comments":{{"totalCount":1}}}}]}}}}}}}}}}' ;;
esac
"#,
        reply_allowed, resolve_allowed
    );
    std::fs::write(&path, body).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

fn populated() -> (PathBuf, PathBuf, String) {
    let (root, state, prep) = seed();
    let binding = PreparationStore::open(&state).unwrap().load(&prep).unwrap();
    let gh = fake_gh(&root, true, true);
    acquire_discussions_with_gh(&binding, &state, &OperationControl::default(), &gh).unwrap();
    (root, state, prep)
}

fn invoke_cli(
    preparation_id: &str,
    state: &Path,
    command: &str,
    input: &serde_json::Value,
) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bodh"))
        .args([
            "draft",
            command,
            "--prepared",
            preparation_id,
            "--state",
            state.to_str().unwrap(),
            "--input",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(input).unwrap())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn reply_and_thread_intent_persist_as_distinct_unpublished_work() {
    let (_root, state, prep) = populated();
    let mut store = DraftStore::open(&state).unwrap();
    let initial = store.list(&prep).unwrap();
    let reply = store
        .save_reply(
            &prep,
            None,
            "thread-1",
            "comment-root",
            "Could you retain this check?",
            initial.workspace_revision,
            0,
        )
        .unwrap();
    assert_eq!(reply.reply.thread_id, "thread-1");
    assert_eq!(reply.reply.root_comment_id, "101");
    assert_eq!(reply.counts.replies, 1);
    let moved_root = store
        .save_reply(
            &prep,
            Some(&reply.reply.id),
            "thread-1",
            "some-other-comment",
            "A root cannot silently change",
            reply.workspace_revision,
            reply.reply.revision,
        )
        .unwrap_err();
    assert_eq!(moved_root.code, "REPLY_NOT_ALLOWED");
    let edited = store
        .save_reply(
            &prep,
            Some(&reply.reply.id),
            "thread-1",
            "comment-root",
            "Revised reply",
            reply.workspace_revision,
            reply.reply.revision,
        )
        .unwrap();
    assert_eq!(edited.reply.revision, 2);
    let staged = store
        .stage_thread_action(
            &prep,
            None,
            "thread-1",
            bodh::ThreadAction::Resolve,
            edited.workspace_revision,
            0,
        )
        .unwrap();
    assert_eq!(staged.action.action, bodh::ThreadAction::Resolve);
    assert_eq!(staged.counts.thread_intents, 1);
    let reopened = DraftStore::open(&state).unwrap().list(&prep).unwrap();
    assert_eq!(reopened.workspace_revision, staged.workspace_revision);
    assert_eq!(reopened.reply_drafts.len(), 1);
    assert_eq!(reopened.thread_actions.len(), 1);
    assert_eq!(reopened.reply_drafts[0].body, "Revised reply");
    let after_reply = DraftStore::open(&state)
        .unwrap()
        .discard_reply(
            &prep,
            &reopened.reply_drafts[0].id,
            reopened.workspace_revision,
            reopened.reply_drafts[0].revision,
        )
        .unwrap();
    let cleared = DraftStore::open(&state)
        .unwrap()
        .discard_thread_action(
            &prep,
            &after_reply.thread_actions[0].id,
            after_reply.workspace_revision,
            after_reply.thread_actions[0].revision,
        )
        .unwrap();
    assert!(cleared.reply_drafts.is_empty());
    assert!(cleared.thread_actions.is_empty());
}

#[test]
fn confirmed_discussion_marker_hides_only_its_exact_draft_revision() {
    let (_root, state, prep) = populated();
    let mut store = DraftStore::open(&state).unwrap();
    let initial = store.list(&prep).unwrap();
    let reply = store
        .save_reply(
            &prep,
            None,
            "thread-1",
            "comment-root",
            "Published reply",
            initial.workspace_revision,
            0,
        )
        .unwrap();
    let action = store
        .stage_thread_action(
            &prep,
            None,
            "thread-1",
            bodh::ThreadAction::Resolve,
            reply.workspace_revision,
            0,
        )
        .unwrap();
    let db = rusqlite::Connection::open(&state).unwrap();
    let context_id: String = db
        .query_row(
            "SELECT context_id FROM preparations WHERE preparation_id=?1",
            [&prep],
            |row| row.get(0),
        )
        .unwrap();
    db
        .execute_batch(
            "CREATE TABLE discussion_published_replies(plan_id TEXT,preparation_id TEXT,context_id TEXT,draft_id TEXT,draft_revision INTEGER,step_id TEXT);
             CREATE TABLE discussion_published_thread_actions(plan_id TEXT,preparation_id TEXT,context_id TEXT,draft_id TEXT,draft_revision INTEGER,step_id TEXT);",
        )
        .unwrap();
    db.execute(
        "INSERT INTO discussion_published_replies VALUES('plan',?1,?2,?3,?4,'reply-step')",
        rusqlite::params![
            prep,
            context_id,
            reply.reply.id,
            reply.reply.revision as i64
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO discussion_published_thread_actions VALUES('plan',?1,?2,?3,?4,'action-step')",
        rusqlite::params![
            prep,
            context_id,
            action.action.id,
            action.action.revision as i64
        ],
    )
    .unwrap();
    let published = store.list(&prep).unwrap();
    assert!(published.reply_drafts.is_empty());
    assert!(published.thread_actions.is_empty());
    assert_eq!(published.counts.replies, 0);
    assert_eq!(published.counts.thread_intents, 0);
    let edited = store
        .save_reply(
            &prep,
            Some(&reply.reply.id),
            "thread-1",
            "comment-root",
            "New revision remains unpublished",
            published.workspace_revision,
            reply.reply.revision,
        )
        .unwrap();
    let visible = store.list(&prep).unwrap();
    assert_eq!(edited.reply.revision, 2);
    assert_eq!(visible.reply_drafts.len(), 1);
    assert_eq!(visible.reply_drafts[0].revision, 2);
    assert!(visible.thread_actions.is_empty());
}

#[test]
fn stale_revision_and_remote_permission_loss_reject_without_mutating_pending_work() {
    let (root, state, prep) = populated();
    let mut first = DraftStore::open(&state).unwrap();
    let mut second = DraftStore::open(&state).unwrap();
    let initial = first.list(&prep).unwrap();
    let saved = first
        .save_reply(
            &prep,
            None,
            "thread-1",
            "comment-root",
            "Keep this editor text.",
            initial.workspace_revision,
            0,
        )
        .unwrap();
    let stale = second
        .save_reply(
            &prep,
            None,
            "thread-1",
            "comment-root",
            "stale overwrite",
            initial.workspace_revision,
            0,
        )
        .unwrap_err();
    assert_eq!(stale.code, "WORKSPACE_REVISION_STALE");
    let gh = fake_gh(&root, false, false);
    let binding = PreparationStore::open(&state).unwrap().load(&prep).unwrap();
    acquire_discussions_with_gh(&binding, &state, &OperationControl::default(), &gh).unwrap();
    let denied = first
        .stage_thread_action(
            &prep,
            None,
            "thread-1",
            bodh::ThreadAction::Resolve,
            saved.workspace_revision,
            0,
        )
        .unwrap_err();
    assert_eq!(denied.code, "THREAD_ACTION_NOT_ALLOWED");
    let latest = DraftStore::open(&state).unwrap().list(&prep).unwrap();
    assert_eq!(latest.reply_drafts[0].body, "Keep this editor text.");
    assert!(latest.thread_actions.is_empty());
}

#[test]
fn cli_reply_and_action_commands_return_revision_checked_json_without_remote_writes() {
    let (_root, state, prep) = populated();
    // The native UI selects the GraphQL node ID; the stored draft must contain
    // GitHub's numeric REST comment ID for the eventual reply endpoint.
    let input = serde_json::json!({"thread_id":"thread-1","root_comment_id":"comment-root","body":"CLI local reply","expected_workspace_revision":0,"expected_draft_revision":0});
    let reply = invoke_cli(&prep, &state, "reply-save", &input);
    assert!(
        reply.status.success(),
        "{}",
        String::from_utf8_lossy(&reply.stderr)
    );
    let envelope: serde_json::Value = serde_json::from_slice(&reply.stdout).unwrap();
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["data"]["reply"]["root_comment_id"], "101");
    let input = serde_json::json!({"thread_id":"thread-1","action":"resolve","expected_workspace_revision":1,"expected_draft_revision":0});
    let action = invoke_cli(&prep, &state, "thread-action-stage", &input);
    assert!(
        action.status.success(),
        "{}",
        String::from_utf8_lossy(&action.stderr)
    );
    let envelope: serde_json::Value = serde_json::from_slice(&action.stdout).unwrap();
    assert_eq!(envelope["data"]["action"]["action"], "resolve");
    assert_eq!(envelope["data"]["counts"]["replies"], 1);
    assert_eq!(envelope["data"]["counts"]["thread_intents"], 1);
}

#[test]
fn reply_save_accepts_numeric_rest_comment_id_as_well_as_graphql_node_id() {
    let (_root, state, prep) = populated();
    let saved = DraftStore::open(&state)
        .unwrap()
        .save_reply(
            &prep,
            None,
            "thread-1",
            "101",
            "Numeric REST selection",
            0,
            0,
        )
        .unwrap();
    assert_eq!(saved.reply.root_comment_id, "101");
}

#[test]
fn shared_version_two_migration_repairs_tables_from_independent_ticket_orderings() {
    let (_root, state, prep) = seed();
    drop(DraftStore::open(&state).unwrap());
    let db = rusqlite::Connection::open(&state).unwrap();
    db.execute_batch("DROP TABLE code_replacement_drafts; DROP TABLE reply_drafts; DROP TABLE thread_action_drafts;").unwrap();
    drop(db);
    let snapshot = DraftStore::open(&state).unwrap().list(&prep).unwrap();
    assert_eq!(snapshot.counts.replies, 0);
    assert_eq!(snapshot.counts.thread_intents, 0);
    let db = rusqlite::Connection::open(&state).unwrap();
    for table in [
        "code_replacement_drafts",
        "reply_drafts",
        "thread_action_drafts",
    ] {
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists, "missing additive v2 table {table}");
    }
}
