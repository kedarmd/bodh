use bodh::{
    DiscussionPublisher, DiscussionReconciliationOutcome, DiscussionStep, GhDiscussionPublisher,
    OperationControl, PrIdentity, PreparationStore, ReviewContext,
};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn lost_reply_acknowledgment_stays_unknown_after_reply_permission_is_lost() {
    let (root, state, preparation_id, head, base) = fixture();
    let calls = root.join("gh-calls");
    let script = r#"case "$*" in
  *viewerPermission*) printf '%s' '{"data":{"repository":{"pullRequest":{"state":"OPEN","headRefOid":"HEAD_SHA","baseRefOid":"BASE_SHA","viewerPermission":"READ"}}}}' ;;
  *viewerCanReply*) printf '%s' '{"data":{"node":{"id":"thread-1","isResolved":false,"viewerCanReply":false,"viewerCanResolve":false,"viewerCanUnresolve":false}}}' ;;
  *comments\(first:100\)*) printf '%s' '{"data":{"viewer":{"login":"reviewer"},"node":{"comments":{"pageInfo":{"hasNextPage":false},"nodes":[{"databaseId":456,"body":"exact reply","createdAt":"2026-11-06T00:00:00Z","author":{"login":"reviewer"},"replyTo":{"databaseId":123}}]}}}' ;;
  *) echo 'unexpected query' >&2; exit 2 ;;
esac"#
        .replace("HEAD_SHA", &head)
        .replace("BASE_SHA", &base);
    let gh = fake_gh(&root.join("gh"), &calls, &script);
    let publisher = GhDiscussionPublisher {
        executable: gh,
        state,
    };
    let step = DiscussionStep {
        id: "step-1".into(),
        ordinal: 0,
        kind: "reply".into(),
        draft_id: "draft-1".into(),
        draft_revision: 1,
        thread_id: "thread-1".into(),
        root_comment_id: Some("123".into()),
        body: Some("exact reply".into()),
        action: None,
        observed_resolved: None,
        state: "unknown".into(),
        remote_id: None,
        failure: None,
        attempt_started_at: Some(0),
    };

    let result = publisher
        .reconcile_unknown(&preparation_id, &step, 0, &OperationControl::default())
        .unwrap();
    assert!(matches!(
        result,
        DiscussionReconciliationOutcome::Unknown { .. }
    ));
    let calls = std::fs::read_to_string(calls).unwrap();
    assert_eq!(
        calls.lines().count(),
        2,
        "comments are not read after permission loss"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn gh_reader_confirms_only_one_exact_reply_by_the_authenticated_actor() {
    let (root, state, preparation_id, head, base) = fixture();
    let publisher = reader_fixture(
        &root,
        &state,
        "reviewer",
        true,
        vec![comment(456, "reviewer")],
        &head,
        &base,
    );
    let result = publisher
        .reconcile_unknown(
            &preparation_id,
            &reply_step(),
            0,
            &OperationControl::default(),
        )
        .unwrap();
    assert_eq!(
        result,
        DiscussionReconciliationOutcome::Confirmed {
            remote_id: "456".into()
        }
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn gh_reader_keeps_wrong_actor_and_duplicate_exact_replies_unknown() {
    for (viewer, comments) in [
        ("reviewer", vec![comment(456, "another-user")]),
        (
            "reviewer",
            vec![comment(456, "reviewer"), comment(457, "reviewer")],
        ),
    ] {
        let (root, state, preparation_id, head, base) = fixture();
        let publisher = reader_fixture(&root, &state, viewer, true, comments, &head, &base);
        let result = publisher
            .reconcile_unknown(
                &preparation_id,
                &reply_step(),
                0,
                &OperationControl::default(),
            )
            .unwrap();
        assert!(matches!(
            result,
            DiscussionReconciliationOutcome::Unknown { .. }
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn gh_reader_keeps_stale_head_and_unavailable_reads_unknown() {
    let (root, state, preparation_id, _head, base) = fixture();
    let moved_head = "c".repeat(40);
    let stale_publisher = reader_fixture(
        &root,
        &state,
        "reviewer",
        true,
        vec![comment(456, "reviewer")],
        &moved_head,
        &base,
    );
    let stale = stale_publisher
        .reconcile_unknown(
            &preparation_id,
            &reply_step(),
            0,
            &OperationControl::default(),
        )
        .unwrap();
    assert!(matches!(
        stale,
        DiscussionReconciliationOutcome::Unknown { .. }
    ));

    let unavailable = fake_gh(
        &root.join("gh-unavailable"),
        &root.join("unavailable-calls"),
        "echo 'HTTP 503 unavailable' >&2; exit 1",
    );
    let unavailable_publisher = GhDiscussionPublisher {
        executable: unavailable,
        state: state.clone(),
    };
    assert!(
        unavailable_publisher
            .reconcile_unknown(
                &preparation_id,
                &reply_step(),
                0,
                &OperationControl::default(),
            )
            .is_err()
    );
    let _ = std::fs::remove_dir_all(root);
}

fn fixture() -> (PathBuf, PathBuf, String, String, String) {
    let root = std::env::temp_dir().join(format!(
        "bodh-discussion-reconcile-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let state = root.join("state.sqlite");
    let base = "a".repeat(40);
    let head = "b".repeat(40);
    let context = ReviewContext {
        pr: PrIdentity {
            host: "github.com".into(),
            owner: "acme".into(),
            repository: "widget".into(),
            number: 17,
        },
        context_id: format!("ctx-{}", NEXT.load(Ordering::Relaxed)),
        base_sha: base.clone(),
        head_sha: head.clone(),
        merge_base_sha: base.clone(),
    };
    let binding = PreparationStore::open(&state)
        .unwrap()
        .save(&context, "/checkout", "PR title", None, &[])
        .unwrap();
    (root, state, binding.preparation_id, head, base)
}

fn fake_gh(path: &Path, calls: &Path, script: &str) -> PathBuf {
    let body = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n{}\n",
        calls.display(),
        script
    );
    std::fs::write(path, body).unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
    path.to_path_buf()
}

fn reply_step() -> DiscussionStep {
    DiscussionStep {
        id: "step-1".into(),
        ordinal: 0,
        kind: "reply".into(),
        draft_id: "draft-1".into(),
        draft_revision: 1,
        thread_id: "thread-1".into(),
        root_comment_id: Some("123".into()),
        body: Some("exact reply".into()),
        action: None,
        observed_resolved: None,
        state: "unknown".into(),
        remote_id: None,
        failure: None,
        attempt_started_at: Some(0),
    }
}

fn comment(id: u64, author: &str) -> serde_json::Value {
    serde_json::json!({
        "databaseId": id,
        "body": "exact reply",
        "createdAt": "2026-11-06T00:00:00Z",
        "author": {"login": author},
        "replyTo": {"databaseId": 123}
    })
}

fn reader_fixture(
    root: &Path,
    state: &Path,
    viewer: &str,
    can_reply: bool,
    comments: Vec<serde_json::Value>,
    head: &str,
    base: &str,
) -> GhDiscussionPublisher {
    let pr = serde_json::json!({
        "data": {"repository": {"pullRequest": {
            "state": "OPEN", "headRefOid": head, "baseRefOid": base,
            "viewerPermission": "WRITE"
        }}}
    });
    let thread = serde_json::json!({
        "data": {"node": {
            "id": "thread-1", "isResolved": false,
            "viewerCanReply": can_reply, "viewerCanResolve": true,
            "viewerCanUnresolve": true
        }}
    });
    let comments = serde_json::json!({
        "data": {"viewer": {"login": viewer}, "node": {
            "comments": {"pageInfo": {"hasNextPage": false}, "nodes": comments}
        }}
    });
    let pr_path = root.join("pr.json");
    let thread_path = root.join("thread.json");
    let comments_path = root.join("comments.json");
    std::fs::write(&pr_path, serde_json::to_vec(&pr).unwrap()).unwrap();
    std::fs::write(&thread_path, serde_json::to_vec(&thread).unwrap()).unwrap();
    std::fs::write(&comments_path, serde_json::to_vec(&comments).unwrap()).unwrap();
    let script = format!(
        "case \"$*\" in\n  *viewerPermission*) cat '{}' ;;\n  *viewerCanReply*) cat '{}' ;;\n  *comments*) cat '{}' ;;\n  *) echo 'unexpected query' >&2; exit 2 ;;\nesac",
        pr_path.display(),
        thread_path.display(),
        comments_path.display()
    );
    let gh = fake_gh(&root.join("gh-reader"), &root.join("reader-calls"), &script);
    GhDiscussionPublisher {
        executable: gh,
        state: state.to_path_buf(),
    }
}
