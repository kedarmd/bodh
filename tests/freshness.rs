use bodh::{
    FreshnessStatus, OperationControl, PrIdentity, PreparationStore, ReviewContext,
    record_initial_remote_snapshot, refresh_freshness_with_gh,
};
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

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
