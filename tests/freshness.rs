use bodh::{
    FreshnessStatus, OperationControl, PrIdentity, PreparationStore, ReviewContext,
    record_initial_remote_snapshot, refresh_freshness_with_gh,
};
use rusqlite::Connection;
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
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

fn git_output(checkout: &Path, args: &[&str]) -> Vec<u8> {
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
        head: String::from_utf8(git_output(checkout, &["rev-parse", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned(),
        symbolic_ref: String::from_utf8(git_output(checkout, &["symbolic-ref", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned(),
        index: std::fs::read(checkout.join(".git/index")).unwrap(),
        staged: git_output(checkout, &["show", ":tracked.txt"]),
        tracked: std::fs::read(checkout.join("tracked.txt")).unwrap(),
        untracked: std::fs::read(checkout.join("untracked.txt")).unwrap(),
    }
}

fn fixture() -> (
    std::path::PathBuf,
    std::path::PathBuf,
    bodh::PreparedBinding,
) {
    let root = std::env::temp_dir().join(format!(
        "bodh-freshness-{}-{}",
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
        context_id: "ctx-pinned".into(),
        base_sha: "a".repeat(40),
        head_sha: "b".repeat(40),
        merge_base_sha: "c".repeat(40),
    };
    let binding = PreparationStore::open(&state)
        .unwrap()
        .save(
            &context,
            "/checkout",
            "Pinned title",
            Some("Pinned description"),
            &[],
        )
        .unwrap();
    record_initial_remote_snapshot(
        &state,
        &binding.preparation_id,
        "main",
        "feature",
        &context.base_sha,
        &context.head_sha,
        "open",
    )
    .unwrap();
    (root, state, binding)
}

fn fake_gh(root: &Path, response: &str) -> std::path::PathBuf {
    let gh = root.join("gh");
    std::fs::write(
        &gh,
        format!(
            "#!/bin/sh\nprintf '%s' '{}'\n",
            response.replace('\'', "'\\''")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    gh
}

#[test]
fn observing_a_new_head_reports_staleness_without_changing_the_pinned_review() {
    let (root, state, binding) = fixture();
    let current_head = "d".repeat(40);
    let gh = fake_gh(
        &root,
        &format!(
            r#"{{"state":"open","base":{{"ref":"main","sha":"{}"}},"head":{{"ref":"feature","sha":"{}"}}}}"#,
            binding.context.base_sha, current_head
        ),
    );

    let observation =
        refresh_freshness_with_gh(&binding, &state, &OperationControl::default(), &gh).unwrap();

    assert_eq!(observation.status, FreshnessStatus::Changed);
    assert!(observation.head_moved);
    assert!(!observation.base_moved);
    let still_pinned = PreparationStore::open(&state)
        .unwrap()
        .load(&binding.preparation_id)
        .unwrap();
    assert_eq!(still_pinned.context.head_sha, binding.context.head_sha);
    assert_eq!(
        still_pinned.description.as_deref(),
        Some("Pinned description")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn base_movement_and_branch_renames_are_reported_separately() {
    let (root, state, binding) = fixture();
    let gh = fake_gh(
        &root,
        &format!(
            r#"{{"state":"open","base":{{"ref":"trunk","sha":"{}"}},"head":{{"ref":"renamed","sha":"{}"}}}}"#,
            "e".repeat(40),
            binding.context.head_sha
        ),
    );
    let observation =
        refresh_freshness_with_gh(&binding, &state, &OperationControl::default(), &gh).unwrap();
    assert_eq!(observation.status, FreshnessStatus::Changed);
    assert!(observation.base_moved);
    assert_eq!(observation.target_changed, Some(true));
    assert_eq!(observation.head_branch_changed, Some(true));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unavailable_metadata_is_unknown_and_keeps_the_pinned_binding() {
    let (root, state, binding) = fixture();
    let gh = root.join("missing-gh");
    let observation =
        refresh_freshness_with_gh(&binding, &state, &OperationControl::default(), &gh).unwrap();
    assert_eq!(observation.status, FreshnessStatus::Unknown);
    assert!(observation.error_code.is_some());
    let still_pinned = PreparationStore::open(&state)
        .unwrap()
        .load(&binding.preparation_id)
        .unwrap();
    assert_eq!(still_pinned.context, binding.context);
    assert_eq!(
        bodh::latest_freshness_observation(&state, &binding.preparation_id).unwrap(),
        Some(observation)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn review_intent_revision_is_part_of_the_load_latest_protection_fingerprint() {
    let (root, state, binding) = fixture();
    let before = bodh::protected_work_snapshot(&state, &binding).unwrap();
    let mut publication = bodh::ReviewPublicationStore::open(&state).unwrap();
    publication
        .stage(bodh::StageReviewRequest {
            state: state.clone(),
            preparation_id: binding.preparation_id.clone(),
            expected_workspace_revision: before.workspace_revision,
            expected_intent_revision: 0,
            selected: vec![],
            selected_replacements: vec![],
            selected_replies: vec![],
            selected_thread_actions: vec![],
            summary: Some("Explicit review summary".into()),
            event: Some(bodh::ReviewEvent::Comment),
        })
        .unwrap();
    let after = bodh::protected_work_snapshot(&state, &binding).unwrap();
    assert_ne!(before.fingerprint, after.fingerprint);
    assert_eq!(after.unpublished_draft_count, 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn load_latest_approval_expires_on_any_new_review_intent_revision() {
    let (root, state, binding) = fixture();
    let candidate_context = ReviewContext {
        context_id: "ctx-latest".into(),
        head_sha: "d".repeat(40),
        ..binding.context.clone()
    };
    let candidate = PreparationStore::open(&state)
        .unwrap()
        .save(&candidate_context, "/checkout", "Latest title", None, &[])
        .unwrap();
    let protected = bodh::protected_work_snapshot(&state, &binding).unwrap();
    let approval = bodh::authorize_latest_switch(
        &state,
        &binding.preparation_id,
        &candidate.preparation_id,
        &protected.fingerprint,
    )
    .unwrap();

    let mut publication = bodh::ReviewPublicationStore::open(&state).unwrap();
    publication
        .stage(bodh::StageReviewRequest {
            state: state.clone(),
            preparation_id: binding.preparation_id.clone(),
            expected_workspace_revision: protected.workspace_revision,
            expected_intent_revision: 0,
            selected: vec![],
            selected_replacements: vec![],
            selected_replies: vec![],
            selected_thread_actions: vec![],
            summary: Some("Edited after preview".into()),
            event: None,
        })
        .unwrap();

    let error = bodh::apply_latest_switch_approval(&state, &approval.approval_id).unwrap_err();
    assert_eq!(error.code, "DRAFT_REVISION_STALE");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn finding_feedback_is_part_of_load_latest_protection() {
    let (root, state, binding) = fixture();
    drop(bodh::FindingTriageStore::open(&state).unwrap());
    let before = bodh::protected_work_snapshot(&state, &binding).unwrap();
    let candidate_context = ReviewContext {
        context_id: "ctx-latest".into(),
        head_sha: "d".repeat(40),
        ..binding.context.clone()
    };
    let candidate = PreparationStore::open(&state)
        .unwrap()
        .save(&candidate_context, "/checkout", "Latest title", None, &[])
        .unwrap();
    let approval = bodh::authorize_latest_switch(
        &state,
        &binding.preparation_id,
        &candidate.preparation_id,
        &before.fingerprint,
    )
    .unwrap();
    Connection::open(&state).unwrap().execute(
        "INSERT INTO finding_triage_entries(context_id,finding_id,disposition,selected_suggestion_id,manual_feedback,revision,updated_at)
         VALUES(?1,'F1','deferred',NULL,'Recheck the contract',1,1)",
        [&binding.context.context_id],
    ).unwrap();

    let after = bodh::protected_work_snapshot(&state, &binding).unwrap();
    assert_ne!(before.fingerprint, after.fingerprint);
    assert_eq!(
        bodh::apply_latest_switch_approval(&state, &approval.approval_id)
            .unwrap_err()
            .code,
        "DRAFT_REVISION_STALE"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unresolved_discussion_payload_is_part_of_load_latest_protection() {
    let (root, state, binding) = fixture();
    drop(bodh::DiscussionPublicationStore::open(&state).unwrap());
    let db = Connection::open(&state).unwrap();
    db.execute(
        "INSERT INTO discussion_publication_plans(plan_id,preparation_id,context_id,intent_revision,fingerprint,review_operation_id,state,created_at,updated_at)
         VALUES('plan-1',?1,?2,0,'plan-fingerprint',NULL,'failed',1,1)",
        rusqlite::params![binding.preparation_id, binding.context.context_id],
    ).unwrap();
    db.execute(
        "INSERT INTO discussion_publication_steps(step_id,plan_id,ordinal,kind,draft_id,draft_revision,thread_id,root_comment_id,body,action,observed_resolved,state,remote_id,failure,created_at,updated_at)
         VALUES('step-1','plan-1',0,'reply','reply-1',1,'thread-1','42','Pending reply body',NULL,NULL,'failed',NULL,'permission lost',1,1)",
        [],
    ).unwrap();
    drop(db);

    let before = bodh::protected_work_snapshot(&state, &binding).unwrap();
    assert_eq!(before.unresolved_publication_count, 1);
    let db = Connection::open(&state).unwrap();
    db.execute("UPDATE discussion_publication_steps SET body='Changed retained payload' WHERE step_id='step-1'", []).unwrap();
    drop(db);
    let after = bodh::protected_work_snapshot(&state, &binding).unwrap();
    assert_ne!(before.fingerprint, after.fingerprint);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn load_latest_approval_is_one_use_and_returns_the_complete_candidate() {
    let (root, state, current) = fixture();
    let candidate_context = ReviewContext {
        context_id: "ctx-latest".into(),
        head_sha: "d".repeat(40),
        ..current.context.clone()
    };
    let candidate = PreparationStore::open(&state)
        .unwrap()
        .save(
            &candidate_context,
            "/checkout",
            "Latest title",
            Some("Latest body"),
            &[],
        )
        .unwrap();
    let snapshot = bodh::protected_work_snapshot(&state, &current).unwrap();
    let approval = bodh::authorize_latest_switch(
        &state,
        &current.preparation_id,
        &candidate.preparation_id,
        &snapshot.fingerprint,
    )
    .unwrap();

    let selected = bodh::apply_latest_switch_approval(&state, &approval.approval_id).unwrap();
    assert_eq!(selected.preparation_id, candidate.preparation_id);
    assert_eq!(selected.context, candidate.context);
    assert_eq!(selected.description.as_deref(), Some("Latest body"));
    assert_eq!(
        bodh::apply_latest_switch_approval(&state, &approval.approval_id)
            .unwrap_err()
            .code,
        "LOAD_LATEST_APPROVAL_INVALID"
    );
    let still_available = PreparationStore::open(&state)
        .unwrap()
        .load(&current.preparation_id)
        .unwrap();
    assert_eq!(still_available.context, current.context);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn load_latest_switch_keeps_every_draft_attached_to_the_old_preparation() {
    let (root, state, current) = fixture();
    drop(bodh::DraftStore::open(&state).unwrap());
    drop(bodh::FindingTriageStore::open(&state).unwrap());
    drop(bodh::DiscussionPublicationStore::open(&state).unwrap());
    let candidate_context = ReviewContext {
        context_id: "ctx-latest-all-drafts".into(),
        head_sha: "d".repeat(40),
        ..current.context.clone()
    };
    let candidate = PreparationStore::open(&state)
        .unwrap()
        .save(
            &candidate_context,
            "/checkout",
            "Latest title",
            Some("Latest description"),
            &[],
        )
        .unwrap();
    let db = Connection::open(&state).unwrap();
    let pr = &current.context.pr;
    let number = i64::try_from(pr.number).unwrap();
    let source =
        serde_json::json!({"path":"src/lib.rs","revision":"head","start_line":1,"end_line":1})
            .to_string();
    let provenance = serde_json::json!({
        "preparation_id":current.preparation_id,
        "context_id":current.context.context_id,
        "base_sha":current.context.base_sha,
        "head_sha":current.context.head_sha,
        "merge_base_sha":current.context.merge_base_sha,
    })
    .to_string();
    db.execute(
        "INSERT INTO manual_text_drafts(draft_id,context_id,host,owner,repository,number,body,source_json,provenance_json,revision,created_at,updated_at) VALUES('comment-1',?1,?2,?3,?4,?5,'manual comment',?6,?7,1,1,1)",
        rusqlite::params![current.context.context_id,pr.host,pr.owner,pr.repository,number,source,provenance],
    ).unwrap();
    db.execute(
        "INSERT INTO code_replacement_drafts(draft_id,context_id,host,owner,repository,number,source_json,original_text,replacement,provenance_json,revision,created_at,updated_at) VALUES('replacement-1',?1,?2,?3,?4,?5,?6,'old','new',?7,2,1,1)",
        rusqlite::params![current.context.context_id,pr.host,pr.owner,pr.repository,number,source,provenance],
    ).unwrap();
    db.execute(
        "INSERT INTO reply_drafts(draft_id,context_id,host,owner,repository,number,thread_id,root_comment_id,body,revision,created_at,updated_at) VALUES('reply-1',?1,?2,?3,?4,?5,'thread-1','42','reply body',3,1,1)",
        rusqlite::params![current.context.context_id,pr.host,pr.owner,pr.repository,number],
    ).unwrap();
    db.execute(
        "INSERT INTO thread_action_drafts(draft_id,context_id,host,owner,repository,number,thread_id,action,observed_resolved,revision,created_at,updated_at) VALUES('action-1',?1,?2,?3,?4,?5,'thread-1','resolve',0,4,1,1)",
        rusqlite::params![current.context.context_id,pr.host,pr.owner,pr.repository,number],
    ).unwrap();
    db.execute(
        "INSERT INTO finding_triage_entries(context_id,finding_id,disposition,selected_suggestion_id,manual_feedback,revision,updated_at) VALUES(?1,'finding-1','deferred',NULL,'retain feedback',5,1)",
        [&current.context.context_id],
    ).unwrap();
    db.execute(
        "INSERT INTO finding_alternative_edits(context_id,finding_id,suggestion_id,body,source_json,original_text,replacement,draft_id,revision,updated_at) VALUES(?1,'finding-1','suggestion-1','edited alternative',?2,'old','new','alternative-1',6,1)",
        rusqlite::params![current.context.context_id,source],
    ).unwrap();
    drop(db);

    let before = bodh::protected_work_snapshot(&state, &current).unwrap();
    assert_eq!(before.unpublished_draft_count, 5);
    let approval = bodh::authorize_latest_switch(
        &state,
        &current.preparation_id,
        &candidate.preparation_id,
        &before.fingerprint,
    )
    .unwrap();
    let selected = bodh::apply_latest_switch_approval(&state, &approval.approval_id).unwrap();

    assert_eq!(selected.preparation_id, candidate.preparation_id);
    assert_eq!(
        PreparationStore::open(&state)
            .unwrap()
            .load(&current.preparation_id)
            .unwrap()
            .context,
        current.context
    );
    let old_drafts = bodh::DraftStore::open(&state)
        .unwrap()
        .list(&current.preparation_id)
        .unwrap();
    assert_eq!(
        old_drafts
            .drafts
            .iter()
            .map(|d| d.id.as_str())
            .collect::<Vec<_>>(),
        ["comment-1"]
    );
    assert_eq!(
        old_drafts
            .reply_drafts
            .iter()
            .map(|d| d.id.as_str())
            .collect::<Vec<_>>(),
        ["reply-1"]
    );
    assert_eq!(
        old_drafts
            .thread_actions
            .iter()
            .map(|d| d.id.as_str())
            .collect::<Vec<_>>(),
        ["action-1"]
    );
    let old_replacements = bodh::DraftStore::open(&state)
        .unwrap()
        .list_replacements(&current.preparation_id)
        .unwrap();
    assert_eq!(old_replacements.drafts.len(), 1);
    assert_eq!(old_replacements.drafts[0].id, "replacement-1");
    assert_eq!(old_replacements.drafts[0].original_text, "old");
    assert_eq!(old_replacements.drafts[0].replacement, "new");
    let new_drafts = bodh::DraftStore::open(&state)
        .unwrap()
        .list(&selected.preparation_id)
        .unwrap();
    assert!(
        new_drafts.drafts.is_empty()
            && new_drafts.reply_drafts.is_empty()
            && new_drafts.thread_actions.is_empty()
    );
    assert!(
        bodh::DraftStore::open(&state)
            .unwrap()
            .list_replacements(&selected.preparation_id)
            .unwrap()
            .drafts
            .is_empty()
    );
    let triage = bodh::FindingTriageStore::open(&state)
        .unwrap()
        .list(&current.preparation_id)
        .unwrap();
    assert_eq!(
        triage.entries[0].manual_feedback.as_deref(),
        Some("retain feedback")
    );
    assert_eq!(triage.alternative_edits[0].body, "edited alternative");
    let new_triage = bodh::FindingTriageStore::open(&state)
        .unwrap()
        .list(&selected.preparation_id)
        .unwrap();
    assert!(new_triage.entries.is_empty() && new_triage.alternative_edits.is_empty());
    assert_eq!(
        bodh::protected_work_snapshot(&state, &current).unwrap(),
        before
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn concurrent_draft_edit_rejects_load_latest_and_retains_both_preparations() {
    let (root, state, current) = fixture();
    drop(bodh::DraftStore::open(&state).unwrap());
    let candidate_context = ReviewContext {
        context_id: "ctx-latest-concurrent-edit".into(),
        head_sha: "e".repeat(40),
        ..current.context.clone()
    };
    let candidate = PreparationStore::open(&state)
        .unwrap()
        .save(&candidate_context, "/checkout", "Latest title", None, &[])
        .unwrap();
    let before = bodh::protected_work_snapshot(&state, &current).unwrap();
    let approval = bodh::authorize_latest_switch(
        &state,
        &current.preparation_id,
        &candidate.preparation_id,
        &before.fingerprint,
    )
    .unwrap();
    let db = Connection::open(&state).unwrap();
    db.execute(
        "INSERT INTO manual_text_drafts(draft_id,context_id,host,owner,repository,number,body,source_json,provenance_json,revision,created_at,updated_at) VALUES('comment-race',?1,?2,?3,?4,?5,'edited while preview open',?6,?7,1,1,1)",
        rusqlite::params![current.context.context_id,current.context.pr.host,current.context.pr.owner,current.context.pr.repository,i64::try_from(current.context.pr.number).unwrap(),serde_json::json!({"path":"src/lib.rs","revision":"head","start_line":1,"end_line":1}).to_string(),serde_json::json!({"preparation_id":current.preparation_id,"context_id":current.context.context_id,"base_sha":current.context.base_sha,"head_sha":current.context.head_sha,"merge_base_sha":current.context.merge_base_sha}).to_string()],
    ).unwrap();
    drop(db);

    assert_eq!(
        bodh::apply_latest_switch_approval(&state, &approval.approval_id)
            .unwrap_err()
            .code,
        "DRAFT_REVISION_STALE"
    );
    assert_eq!(
        PreparationStore::open(&state)
            .unwrap()
            .load(&current.preparation_id)
            .unwrap()
            .context,
        current.context
    );
    assert_eq!(
        PreparationStore::open(&state)
            .unwrap()
            .load(&candidate.preparation_id)
            .unwrap()
            .context,
        candidate.context
    );
    let drafts = bodh::DraftStore::open(&state)
        .unwrap()
        .list(&current.preparation_id)
        .unwrap();
    assert_eq!(drafts.drafts.len(), 1);
    assert_eq!(drafts.drafts[0].body, "edited while preview open");
    assert!(
        bodh::DraftStore::open(&state)
            .unwrap()
            .list(&candidate.preparation_id)
            .unwrap()
            .drafts
            .is_empty()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn failed_or_cancelled_latest_acquisition_keeps_the_current_binding_selected() {
    let (root, state, current) = fixture();
    drop(bodh::DraftStore::open(&state).unwrap());
    let db = Connection::open(&state).unwrap();
    db.execute(
        "INSERT INTO manual_text_drafts(draft_id,context_id,host,owner,repository,number,body,source_json,provenance_json,revision,created_at,updated_at) VALUES('survive-failed-acquire',?1,?2,?3,?4,?5,'keep me',?6,?7,1,1,1)",
        rusqlite::params![current.context.context_id,current.context.pr.host,current.context.pr.owner,current.context.pr.repository,i64::try_from(current.context.pr.number).unwrap(),serde_json::json!({"path":"src/lib.rs","revision":"head","start_line":1,"end_line":1}).to_string(),serde_json::json!({"preparation_id":current.preparation_id,"context_id":current.context.context_id,"base_sha":current.context.base_sha,"head_sha":current.context.head_sha,"merge_base_sha":current.context.merge_base_sha}).to_string()],
    ).unwrap();
    drop(db);
    let before = bodh::protected_work_snapshot(&state, &current).unwrap();
    assert_eq!(before.unpublished_draft_count, 1);

    // Give every acquisition outcome a real repository with a committed
    // baseline, a staged version, a later unstaged tracked edit, and an
    // untracked file. Capture both the raw index and staged blob contents.
    let checkout = root.join("checkout");
    std::fs::create_dir_all(&checkout).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&checkout)
            .status()
            .unwrap()
            .success()
    );
    for (name, value) in [
        ("user.name", "Bodh Test"),
        ("user.email", "bodh-test@example.invalid"),
    ] {
        assert!(
            Command::new("git")
                .args(["config", name, value])
                .current_dir(&checkout)
                .status()
                .unwrap()
                .success()
        );
    }
    std::fs::write(checkout.join("tracked.txt"), b"committed\n").unwrap();
    assert!(
        Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(&checkout)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["commit", "-m", "fixture"])
            .current_dir(&checkout)
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::write(checkout.join("tracked.txt"), b"staged tracked\n").unwrap();
    assert!(
        Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(&checkout)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(checkout.join("tracked.txt"), b"dirty tracked\n").unwrap();
    std::fs::write(checkout.join("untracked.txt"), b"untracked bytes\n").unwrap();
    let checkout_before = checkout_snapshot(&checkout);
    Connection::open(&state)
        .unwrap()
        .execute(
            "UPDATE preparations SET checkout=?1 WHERE preparation_id=?2",
            rusqlite::params![checkout.to_string_lossy(), current.preparation_id],
        )
        .unwrap();
    let current = PreparationStore::open(&state)
        .unwrap()
        .load(&current.preparation_id)
        .unwrap();

    let missing_gh = root.join("missing-gh");
    let error = bodh::prepare_latest_candidate_with_gh(
        &current.preparation_id,
        &state,
        &OperationControl::default(),
        &missing_gh,
    )
    .unwrap_err();
    assert!(!error.code.is_empty());
    assert_eq!(
        PreparationStore::open(&state)
            .unwrap()
            .load(&current.preparation_id)
            .unwrap()
            .context,
        current.context
    );
    assert_eq!(
        bodh::protected_work_snapshot(&state, &current).unwrap(),
        before
    );
    assert_eq!(checkout_snapshot(&checkout), checkout_before);
    assert_eq!(
        bodh::DraftStore::open(&state)
            .unwrap()
            .list(&current.preparation_id)
            .unwrap()
            .drafts[0]
            .body,
        "keep me"
    );

    // Model a GitHub CLI permission denial after the executable starts. This
    // distinguishes access loss from a missing local dependency while keeping
    // the current preparation selected and its populated draft intact.
    let denied_gh = root.join("denied-gh");
    std::fs::write(
        &denied_gh,
        "#!/bin/sh\necho 'HTTP 403: Resource not accessible' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&denied_gh, std::fs::Permissions::from_mode(0o755)).unwrap();

    // Use a real checkout and matching remote so latest-candidate preparation
    // reaches the controlled GitHub API denial rather than failing locally.
    let remote = Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widget.git",
        ])
        .current_dir(&checkout)
        .output()
        .unwrap();
    assert!(remote.status.success());
    assert_eq!(checkout_snapshot(&checkout), checkout_before);
    let acquisition_error = bodh::prepare_latest_candidate_with_gh(
        &current.preparation_id,
        &state,
        &OperationControl::default(),
        &denied_gh,
    )
    .unwrap_err();
    assert_eq!(acquisition_error.code, "AUTH_ACCESS");
    assert_eq!(
        PreparationStore::open(&state)
            .unwrap()
            .load(&current.preparation_id)
            .unwrap()
            .context,
        current.context
    );
    assert_eq!(
        bodh::protected_work_snapshot(&state, &current).unwrap(),
        before
    );
    assert_eq!(checkout_snapshot(&checkout), checkout_before);
    assert_eq!(
        bodh::DraftStore::open(&state)
            .unwrap()
            .list(&current.preparation_id)
            .unwrap()
            .drafts[0]
            .body,
        "keep me"
    );
    let denied =
        refresh_freshness_with_gh(&current, &state, &OperationControl::default(), &denied_gh)
            .unwrap();
    assert_eq!(denied.status, FreshnessStatus::Unknown);
    assert_eq!(denied.error_code.as_deref(), Some("AUTH_ACCESS"));
    assert_eq!(
        PreparationStore::open(&state)
            .unwrap()
            .load(&current.preparation_id)
            .unwrap()
            .context,
        current.context
    );
    assert_eq!(
        bodh::protected_work_snapshot(&state, &current).unwrap(),
        before
    );
    assert_eq!(checkout_snapshot(&checkout), checkout_before);
    assert_eq!(
        bodh::DraftStore::open(&state)
            .unwrap()
            .list(&current.preparation_id)
            .unwrap()
            .drafts[0]
            .body,
        "keep me"
    );

    let cancelled = OperationControl::default();
    cancelled.cancel();
    let error = bodh::prepare_latest_candidate_with_gh(
        &current.preparation_id,
        &state,
        &cancelled,
        &missing_gh,
    )
    .unwrap_err();
    assert_eq!(error.code, "CANCELLED");
    assert_eq!(
        PreparationStore::open(&state)
            .unwrap()
            .load(&current.preparation_id)
            .unwrap()
            .context,
        current.context
    );
    assert_eq!(
        bodh::protected_work_snapshot(&state, &current).unwrap(),
        before
    );
    let _ = std::fs::remove_dir_all(root);
}
