use bodh::{FindingTriageStore, PrIdentity, PreparationStore, ReviewContext};
use rusqlite::{Connection, params};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, PartialEq, Eq)]
struct CheckoutSnapshot {
    head: String,
    symbolic_ref: String,
    index: Vec<u8>,
    staged: Vec<u8>,
    tracked: Vec<u8>,
    untracked: Vec<u8>,
}

fn git(checkout: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .args(args)
        .current_dir(checkout)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {:?}", output.stderr);
    output.stdout
}

fn checkout_snapshot(checkout: &Path) -> CheckoutSnapshot {
    CheckoutSnapshot {
        head: String::from_utf8(git(checkout, &["rev-parse", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned(),
        symbolic_ref: String::from_utf8(git(checkout, &["symbolic-ref", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned(),
        index: std::fs::read(checkout.join(".git/index")).unwrap(),
        staged: git(checkout, &["show", ":tracked.txt"]),
        tracked: std::fs::read(checkout.join("tracked.txt")).unwrap(),
        untracked: std::fs::read(checkout.join("untracked.txt")).unwrap(),
    }
}

fn run_cli(args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_bodh"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "bodh {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("CLI returns one JSON envelope")
}

fn create_checkout(root: &Path) -> PathBuf {
    let checkout = root.join("checkout");
    std::fs::create_dir_all(&checkout).unwrap();
    git(&checkout, &["init", "-b", "main"]);
    for (name, value) in [
        ("user.name", "Bodh CLI fixture"),
        ("user.email", "bodh-cli@example.invalid"),
    ] {
        git(&checkout, &["config", name, value]);
    }
    std::fs::write(checkout.join("tracked.txt"), b"committed\n").unwrap();
    git(&checkout, &["add", "tracked.txt"]);
    git(&checkout, &["commit", "-m", "fixture"]);
    std::fs::write(checkout.join("tracked.txt"), b"staged tracked\n").unwrap();
    git(&checkout, &["add", "tracked.txt"]);
    std::fs::write(checkout.join("tracked.txt"), b"dirty tracked\n").unwrap();
    std::fs::write(checkout.join("untracked.txt"), b"untracked bytes\n").unwrap();
    checkout
}

#[test]
fn cli_views_keep_every_draft_on_old_preparation_after_approved_switch() {
    let root = std::env::temp_dir().join(format!(
        "bodh-freshness-cli-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let checkout = create_checkout(&root);
    let checkout_before = checkout_snapshot(&checkout);
    let state = root.join("state.sqlite");
    let old_context = ReviewContext {
        pr: PrIdentity {
            host: "github.com".into(),
            owner: "acme".into(),
            repository: "widget".into(),
            number: 17,
        },
        context_id: "ctx-cli-old".into(),
        base_sha: "a".repeat(40),
        head_sha: "b".repeat(40),
        merge_base_sha: "c".repeat(40),
    };
    let old = PreparationStore::open(&state)
        .unwrap()
        .save(
            &old_context,
            checkout.to_str().unwrap(),
            "Pinned title",
            Some("Pinned description"),
            &[],
        )
        .unwrap();
    let new_context = ReviewContext {
        context_id: "ctx-cli-new".into(),
        head_sha: "d".repeat(40),
        ..old_context.clone()
    };
    let new = PreparationStore::open(&state)
        .unwrap()
        .save(
            &new_context,
            checkout.to_str().unwrap(),
            "Latest title",
            Some("Latest description"),
            &[],
        )
        .unwrap();

    drop(bodh::DraftStore::open(&state).unwrap());
    drop(FindingTriageStore::open(&state).unwrap());
    drop(bodh::ReviewPublicationStore::open(&state).unwrap());
    drop(bodh::DiscussionPublicationStore::open(&state).unwrap());
    let source = serde_json::json!({
        "path":"src/lib.rs",
        "revision":"head",
        "start_line":1,
        "end_line":1
    })
    .to_string();
    let provenance = serde_json::json!({
        "preparation_id":old.preparation_id,
        "context_id":old.context.context_id,
        "base_sha":old.context.base_sha,
        "head_sha":old.context.head_sha,
        "merge_base_sha":old.context.merge_base_sha
    })
    .to_string();
    let number = i64::try_from(old.context.pr.number).unwrap();
    let db = Connection::open(&state).unwrap();
    db.execute(
        "INSERT INTO manual_text_drafts(draft_id,context_id,host,owner,repository,number,body,source_json,provenance_json,revision,created_at,updated_at) VALUES('comment-1',?1,?2,?3,?4,?5,'manual comment',?6,?7,1,1,1)",
        params![old.context.context_id,old.context.pr.host,old.context.pr.owner,old.context.pr.repository,number,source,provenance],
    ).unwrap();
    db.execute(
        "INSERT INTO code_replacement_drafts(draft_id,context_id,host,owner,repository,number,source_json,original_text,replacement,provenance_json,revision,created_at,updated_at) VALUES('replacement-1',?1,?2,?3,?4,?5,?6,'old','new',?7,2,1,1)",
        params![old.context.context_id,old.context.pr.host,old.context.pr.owner,old.context.pr.repository,number,source,provenance],
    ).unwrap();
    db.execute(
        "INSERT INTO reply_drafts(draft_id,context_id,host,owner,repository,number,thread_id,root_comment_id,body,revision,created_at,updated_at) VALUES('reply-1',?1,?2,?3,?4,?5,'thread-1','42','reply body',3,1,1)",
        params![old.context.context_id,old.context.pr.host,old.context.pr.owner,old.context.pr.repository,number],
    ).unwrap();
    db.execute(
        "INSERT INTO thread_action_drafts(draft_id,context_id,host,owner,repository,number,thread_id,action,observed_resolved,revision,created_at,updated_at) VALUES('action-1',?1,?2,?3,?4,?5,'thread-1','resolve',0,4,1,1)",
        params![old.context.context_id,old.context.pr.host,old.context.pr.owner,old.context.pr.repository,number],
    ).unwrap();
    db.execute(
        "INSERT INTO finding_triage_entries(context_id,finding_id,disposition,selected_suggestion_id,manual_feedback,revision,updated_at) VALUES(?1,'finding-1','deferred',NULL,'retain feedback',5,1)",
        [&old.context.context_id],
    ).unwrap();
    db.execute(
        "INSERT INTO finding_alternative_edits(context_id,finding_id,suggestion_id,body,source_json,original_text,replacement,draft_id,revision,updated_at) VALUES(?1,'finding-1','suggestion-1','edited alternative',?2,'old','new','alternative-1',6,1)",
        params![old.context.context_id,source],
    ).unwrap();
    drop(db);

    let protection = bodh::protected_work_snapshot(&state, &old).unwrap();
    assert_eq!(protection.unpublished_draft_count, 5);
    let approval = bodh::authorize_latest_switch(
        &state,
        &old.preparation_id,
        &new.preparation_id,
        &protection.fingerprint,
    )
    .unwrap();
    let selected = bodh::apply_latest_switch_approval(&state, &approval.approval_id).unwrap();
    assert_eq!(selected.preparation_id, new.preparation_id);

    let old_id = old.preparation_id.as_str();
    let new_id = new.preparation_id.as_str();
    let state_arg = state.to_str().unwrap();
    let old_manifest = run_cli(&[
        "inspect",
        "--prepared",
        old_id,
        "--kind",
        "manifest",
        "--state",
        state_arg,
    ]);
    assert_eq!(old_manifest["ok"], true);
    assert_eq!(old_manifest["data"]["context"]["context_id"], "ctx-cli-old");
    assert_eq!(old_manifest["data"]["draft_counts"]["inline_comments"], 1);
    assert_eq!(old_manifest["data"]["draft_counts"]["code_replacements"], 1);
    assert_eq!(old_manifest["data"]["draft_counts"]["replies"], 1);
    assert_eq!(old_manifest["data"]["draft_counts"]["thread_intents"], 1);
    let new_manifest = run_cli(&[
        "inspect",
        "--prepared",
        new_id,
        "--kind",
        "manifest",
        "--state",
        state_arg,
    ]);
    assert_eq!(new_manifest["ok"], true);
    assert_eq!(new_manifest["data"]["context"]["context_id"], "ctx-cli-new");
    for kind in [
        "inline_comments",
        "code_replacements",
        "replies",
        "thread_intents",
        "review_comments",
    ] {
        assert_eq!(new_manifest["data"]["draft_counts"][kind], 0, "{kind}");
    }

    let old_drafts = run_cli(&["draft", "list", "--prepared", old_id, "--state", state_arg]);
    assert_eq!(old_drafts["ok"], true);
    assert_eq!(old_drafts["data"]["drafts"][0]["id"], "comment-1");
    assert_eq!(old_drafts["data"]["reply_drafts"][0]["id"], "reply-1");
    assert_eq!(old_drafts["data"]["thread_actions"][0]["id"], "action-1");
    assert_eq!(
        old_drafts["data"]["finding_triage"]["context_id"],
        "ctx-cli-old"
    );
    assert_eq!(
        old_drafts["data"]["finding_triage"]["entries"][0]["finding_id"],
        "finding-1"
    );
    assert_eq!(
        old_drafts["data"]["finding_triage"]["entries"][0]["manual_feedback"],
        "retain feedback"
    );
    assert_eq!(
        old_drafts["data"]["finding_triage"]["alternative_edits"][0]["draft_id"],
        "alternative-1"
    );
    assert_eq!(
        old_drafts["data"]["finding_triage"]["alternative_edits"][0]["body"],
        "edited alternative"
    );
    let new_drafts = run_cli(&["draft", "list", "--prepared", new_id, "--state", state_arg]);
    assert_eq!(new_drafts["ok"], true);
    assert_eq!(
        new_drafts["data"]["finding_triage"]["context_id"],
        "ctx-cli-new"
    );
    assert!(new_drafts["data"]["drafts"].as_array().unwrap().is_empty());
    assert!(
        new_drafts["data"]["reply_drafts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        new_drafts["data"]["thread_actions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        new_drafts["data"]["finding_triage"]["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        new_drafts["data"]["finding_triage"]["alternative_edits"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let old_replacements = run_cli(&[
        "draft",
        "replacement-list",
        "--prepared",
        old_id,
        "--state",
        state_arg,
    ]);
    assert_eq!(old_replacements["ok"], true);
    assert_eq!(old_replacements["data"]["drafts"][0]["id"], "replacement-1");
    assert_eq!(
        old_replacements["data"]["drafts"][0]["original_text"],
        "old"
    );
    assert_eq!(old_replacements["data"]["drafts"][0]["replacement"], "new");
    let new_replacements = run_cli(&[
        "draft",
        "replacement-list",
        "--prepared",
        new_id,
        "--state",
        state_arg,
    ]);
    assert_eq!(new_replacements["ok"], true);
    assert!(
        new_replacements["data"]["drafts"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let old_triage = FindingTriageStore::open(&state)
        .unwrap()
        .list(old_id)
        .unwrap();
    assert_eq!(
        old_triage.entries[0].manual_feedback.as_deref(),
        Some("retain feedback")
    );
    assert_eq!(old_triage.alternative_edits[0].body, "edited alternative");
    let new_triage = FindingTriageStore::open(&state)
        .unwrap()
        .list(new_id)
        .unwrap();
    assert!(new_triage.entries.is_empty() && new_triage.alternative_edits.is_empty());
    assert_eq!(checkout_snapshot(&checkout), checkout_before);

    let _ = std::fs::remove_dir_all(root);
}
