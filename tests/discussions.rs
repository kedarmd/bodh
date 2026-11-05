use bodh::{
    OperationControl, PrIdentity, PreparationStore, ReviewContext, acquire_discussions_with_gh,
    latest_discussion_summary, load_latest_discussions,
};
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> (
    std::path::PathBuf,
    std::path::PathBuf,
    bodh::PreparedBinding,
) {
    let root = std::env::temp_dir().join(format!(
        "bodh-discussions-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let state = root.join("state.sqlite");
    let context = ReviewContext {
        pr: PrIdentity {
            host: "github.com".into(),
            owner: "acme".into(),
            repository: "widget".into(),
            number: 17,
        },
        context_id: format!("ctx-{}", NEXT.load(Ordering::Relaxed)),
        base_sha: "a".repeat(40),
        head_sha: "b".repeat(40),
        merge_base_sha: "c".repeat(40),
    };
    let binding = PreparationStore::open(&state)
        .unwrap()
        .save(
            &context,
            "/checkout",
            "PR title",
            Some("original body"),
            &[],
        )
        .unwrap();
    (root, state, binding)
}

fn fake_gh(root: &Path, script: &str) -> std::path::PathBuf {
    let path = root.join(format!("gh-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

fn run_gh(binding: &bodh::PreparedBinding, state: &Path, gh: &Path) -> serde_json::Value {
    acquire_discussions_with_gh(binding, state, &OperationControl::default(), gh).unwrap()
}

#[test]
fn fetches_outer_threads_and_each_nested_reply_connection_and_survives_restart() {
    let (root, state, binding) = fixture();
    let gh = fake_gh(
        &root,
        r#"case "$*" in
  *node\(id:*thread-1*after=reply-2*) printf '%s' '{"data":{"node":{"comments":{"totalCount":2,"pageInfo":{"hasNextPage":false,"endCursor":"reply-2"},"nodes":[{"id":"comment-2","databaseId":"102","body":"reply two","replyTo":{"databaseId":"101"}}]}}}}' ;;
  *node\(id:*thread-1*) printf '%s' '{"data":{"node":{"comments":{"totalCount":2,"pageInfo":{"hasNextPage":true,"endCursor":"reply-2"},"nodes":[{"id":"comment-1","databaseId":"101","body":"root","replyTo":null}]}}}}' ;;
  *node\(id:*thread-2*) printf '%s' '{"data":{"node":{"comments":{"totalCount":0,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]}}}}' ;;
  *after=outer-2*) printf '%s' '{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":2,"pageInfo":{"hasNextPage":false,"endCursor":"outer-2"},"nodes":[{"id":"thread-2","path":"src/b.rs","line":9,"originalLine":8,"diffSide":"RIGHT","startLine":7,"originalStartLine":6,"startDiffSide":"RIGHT","originalStartDiffSide":"LEFT","isResolved":true,"isOutdated":false,"viewerCanReply":false,"comments":{"totalCount":0}}]}}}}}' ;;
  *) printf '%s' '{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":2,"pageInfo":{"hasNextPage":true,"endCursor":"outer-2"},"nodes":[{"id":"thread-1","path":"src/a.rs","line":3,"originalLine":2,"diffSide":"RIGHT","startLine":null,"originalStartLine":null,"startDiffSide":null,"originalStartDiffSide":null,"isResolved":false,"isOutdated":true,"viewerCanReply":true,"comments":{"totalCount":2}}]}}}}}' ;;
esac"#,
    );
    let result = run_gh(&binding, &state, &gh);
    assert_eq!(result["complete"], true);
    assert_eq!(result["expected_thread_count"], 2);
    assert_eq!(result["fetched_thread_count"], 2);
    assert_eq!(result["threads"].as_array().unwrap().len(), 2);
    assert_eq!(result["threads"][0]["id"], "thread-1");
    assert_eq!(
        result["threads"][0]["comments"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        result["threads"][0]["comments"][1]["replyTo"]["databaseId"],
        "101"
    );
    assert_eq!(result["threads"][1]["isResolved"], true);
    assert_eq!(result["threads"][0]["diffSide"], "RIGHT");
    assert_eq!(result["threads"][0]["originalLine"], 2);
    assert_eq!(
        result["threads"][0]["originalDiffSide"],
        serde_json::Value::Null
    );
    assert_eq!(
        result["threads"][0]["original_side_availability"],
        "not_exposed_by_graphql"
    );
    let summary = latest_discussion_summary(&state, &binding.preparation_id).unwrap();
    assert_eq!(summary["expected_comment_count"], 2);
    assert_eq!(summary["fetched_comment_count"], 2);
    let reopened = load_latest_discussions(&state, &binding.preparation_id).unwrap();
    assert_eq!(reopened, result);
    let prepared_again = PreparationStore::open(&state)
        .unwrap()
        .load(&binding.preparation_id)
        .unwrap();
    assert_eq!(prepared_again.description.as_deref(), Some("original body"));
    let db = rusqlite::Connection::open(&state).unwrap();
    let user_version: i64 = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    let discussion_version: i64 = db
        .query_row(
            "SELECT MAX(version) FROM discussion_schema_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(user_version, 1);
    assert_eq!(discussion_version, 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn later_page_permission_failure_preserves_acquired_outer_and_reply_pages() {
    let (root, state, binding) = fixture();
    let gh = fake_gh(
        &root,
        r#"case "$*" in
  *node\(id:*thread-1*after=reply-2*) echo 'HTTP 403 permission denied' >&2; exit 1 ;;
  *node\(id:*thread-1*) printf '%s' '{"data":{"node":{"comments":{"totalCount":2,"pageInfo":{"hasNextPage":true,"endCursor":"reply-2"},"nodes":[{"id":"comment-1","databaseId":101,"body":"root","replyTo":null}]}}}}' ;;
  *) printf '%s' '{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"outer-1"},"nodes":[{"id":"thread-1","path":"src/a.rs","line":3,"originalLine":2,"diffSide":"RIGHT","isResolved":false,"isOutdated":false,"viewerCanReply":true,"comments":{"totalCount":2}}]}}}}}' ;;
esac"#,
    );
    let result = run_gh(&binding, &state, &gh);
    assert_eq!(result["complete"], false);
    assert_eq!(result["fetched_thread_count"], 1);
    assert_eq!(
        result["threads"][0]["comments"].as_array().unwrap().len(),
        1
    );
    assert_eq!(result["threads"][0]["comments_complete"], false);
    assert_eq!(
        result["threads"][0]["comments_incomplete_cause"],
        "AUTH_ACCESS"
    );
    assert_eq!(
        load_latest_discussions(&state, &binding.preparation_id).unwrap(),
        result
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn discussion_cache_keeps_the_latest_attempt_and_one_complete_fallback() {
    let (root, state, binding) = fixture();
    let complete = fake_gh(
        &root,
        r#"printf '%s' '{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":0,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]}}}}}'"#,
    );
    let success = run_gh(&binding, &state, &complete);
    assert_eq!(success["complete"], true);

    let failure = fake_gh(&root, "echo 'HTTP 503 unavailable' >&2; exit 1");
    for _ in 0..3 {
        let partial = run_gh(&binding, &state, &failure);
        assert_eq!(partial["complete"], false);
    }
    let db = rusqlite::Connection::open(&state).unwrap();
    let retained: i64 = db
        .query_row(
            "SELECT count(*) FROM discussion_observations WHERE preparation_id=?1",
            [&binding.preparation_id],
            |row| row.get(0),
        )
        .unwrap();
    let complete_count: i64 = db
        .query_row(
            "SELECT count(*) FROM discussion_observations WHERE preparation_id=?1 AND complete=1",
            [&binding.preparation_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained, 2, "keep current partial plus last complete cache");
    assert_eq!(complete_count, 1);
    assert_eq!(
        load_latest_discussions(&state, &binding.preparation_id).unwrap()["complete"],
        false
    );

    let refreshed = run_gh(&binding, &state, &complete);
    assert_eq!(refreshed["complete"], true);
    let retained: i64 = db
        .query_row(
            "SELECT count(*) FROM discussion_observations WHERE preparation_id=?1",
            [&binding.preparation_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        retained, 1,
        "successful refresh replaces transient cache history"
    );
}

#[test]
fn repeated_cursors_duplicates_null_nodes_and_count_changes_are_incomplete() {
    let (root, state, binding) = fixture();
    let gh = fake_gh(
        &root,
        r#"case "$*" in
  *node\(id:*thread-1*after=same*) printf '%s' '{"data":{"node":{"comments":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"same"},"nodes":[null,{"id":"comment-1","body":"duplicate root"}]}}}}' ;;
  *node\(id:*thread-1*) printf '%s' '{"data":{"node":{"comments":{"totalCount":1,"pageInfo":{"hasNextPage":true,"endCursor":"same"},"nodes":[{"id":"comment-1","body":"root"}]}}}}' ;;
  *after=same*) printf '%s' '{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":3,"pageInfo":{"hasNextPage":true,"endCursor":"same"},"nodes":[]}}}}}' ;;
  *) printf '%s' '{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":2,"pageInfo":{"hasNextPage":true,"endCursor":"same"},"nodes":[{"id":"thread-1","path":"src/a.rs","line":3,"isResolved":false,"isOutdated":false,"viewerCanReply":true,"comments":{"totalCount":1}}]}}}}}' ;;
esac"#,
    );
    let result = run_gh(&binding, &state, &gh);
    assert_eq!(result["complete"], false);
    assert_eq!(result["incomplete_connection"], "threads");
    assert_eq!(result["incomplete_cause"], "REPEATED_CURSOR");
    assert_eq!(result["count_changed"], true);
    assert!(result["page_count"].as_u64().unwrap() >= 3);
    assert_eq!(result["threads"][0]["comments_complete"], false);
    assert_eq!(
        result["threads"][0]["comments_incomplete_cause"],
        "NULL_COMMENT_NODE"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn inspection_cli_returns_persisted_observation_and_manifest_summary() {
    let (root, state, binding) = fixture();
    let gh = fake_gh(
        &root,
        r#"printf '%s' '{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":null,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]}}}}}'"#,
    );
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::copy(&gh, bin.join("gh")).unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_bodh"))
        .args([
            "inspect",
            "--prepared",
            &binding.preparation_id,
            "--kind",
            "discussions",
            "--state",
            state.to_str().unwrap(),
        ])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["data"]["available"], true);
    assert_eq!(envelope["data"]["complete"], true);
    assert_eq!(
        envelope["data"]["expected_thread_count"],
        serde_json::Value::Null
    );
    assert_eq!(envelope["data"]["expected_comment_count"], 0);
    let manifest = Command::new(env!("CARGO_BIN_EXE_bodh"))
        .args([
            "inspect",
            "--prepared",
            &binding.preparation_id,
            "--kind",
            "manifest",
            "--state",
            state.to_str().unwrap(),
        ])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(manifest.status.success());
    let manifest: serde_json::Value = serde_json::from_slice(&manifest.stdout).unwrap();
    assert_eq!(manifest["data"]["discussions_complete"], true);
    assert_eq!(manifest["data"]["discussions"]["fetched_thread_count"], 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn nested_page_timeout_keeps_the_prior_page_durable() {
    let (root, state, binding) = fixture();
    let gh = fake_gh(
        &root,
        r#"case "$*" in
  *node\(id:*thread-1*after=reply-2*) sleep 3 ;;
  *node\(id:*thread-1*) printf '%s' '{"data":{"node":{"comments":{"totalCount":2,"pageInfo":{"hasNextPage":true,"endCursor":"reply-2"},"nodes":[{"id":"comment-1","databaseId":101,"body":"root","replyTo":null}]}}}}' ;;
  *) printf '%s' '{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"outer-1"},"nodes":[{"id":"thread-1","path":"src/a.rs","line":3,"isResolved":false,"isOutdated":false,"viewerCanReply":true,"comments":{"totalCount":2}}]}}}}}' ;;
esac"#,
    );
    let result = acquire_discussions_with_gh(
        &binding,
        &state,
        &OperationControl::with_timeout(std::time::Duration::from_millis(100)),
        &gh,
    )
    .unwrap();
    assert_eq!(result["complete"], false);
    assert_eq!(
        result["threads"][0]["comments"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        result["threads"][0]["comments_incomplete_cause"],
        "OPERATION_TIMEOUT"
    );
    assert_eq!(
        load_latest_discussions(&state, &binding.preparation_id).unwrap(),
        result
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn graphql_partial_permission_error_is_reported_without_discarding_pages() {
    let (root, state, binding) = fixture();
    let gh = fake_gh(
        &root,
        r#"case "$*" in
  *node\(id:*thread-1*after=reply-2*) printf '%s' '{"data":{"node":null},"errors":[{"type":"FORBIDDEN","message":"Resource not accessible by integration"}]}' ;;
  *node\(id:*thread-1*) printf '%s' '{"data":{"node":{"comments":{"totalCount":2,"pageInfo":{"hasNextPage":true,"endCursor":"reply-2"},"nodes":[{"id":"comment-1","databaseId":101,"body":"root","replyTo":null}]}}}}' ;;
  *) printf '%s' '{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"outer-1"},"nodes":[{"id":"thread-1","path":"src/a.rs","line":3,"isResolved":false,"isOutdated":false,"viewerCanReply":true,"comments":{"totalCount":2}}]}}}}}' ;;
esac"#,
    );
    let result = run_gh(&binding, &state, &gh);
    assert_eq!(result["complete"], false);
    assert_eq!(result["incomplete_connection"], "comments:thread-1");
    assert_eq!(result["incomplete_cause"], "AUTH_ACCESS");
    assert_eq!(
        result["threads"][0]["comments"].as_array().unwrap().len(),
        1
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn nested_reply_cursor_repetition_is_tracked_per_thread_connection() {
    let (root, state, binding) = fixture();
    let gh = fake_gh(
        &root,
        r#"case "$*" in
  *node\(id:*thread-1*after=comment-2*) printf '%s' '{"data":{"node":{"comments":{"totalCount":2,"pageInfo":{"hasNextPage":true,"endCursor":"comment-2"},"nodes":[{"id":"comment-2","body":"reply"}]}}}}' ;;
  *node\(id:*thread-1*) printf '%s' '{"data":{"node":{"comments":{"totalCount":2,"pageInfo":{"hasNextPage":true,"endCursor":"comment-2"},"nodes":[{"id":"comment-1","body":"root"}]}}}}' ;;
  *) printf '%s' '{"data":{"repository":{"pullRequest":{"reviewThreads":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"thread-1"},"nodes":[{"id":"thread-1","path":"src/a.rs","line":3,"isResolved":false,"isOutdated":false,"viewerCanReply":true,"comments":{"totalCount":2}}]}}}}}' ;;
esac"#,
    );
    let result = run_gh(&binding, &state, &gh);
    assert_eq!(result["complete"], false);
    assert_eq!(result["incomplete_connection"], "comments:thread-1");
    assert_eq!(
        result["threads"][0]["comments_incomplete_cause"],
        "REPEATED_CURSOR"
    );
    assert_eq!(
        result["threads"][0]["comments"].as_array().unwrap().len(),
        2
    );
    assert_eq!(result["page_count"], 3);
    let _ = std::fs::remove_dir_all(root);
}
