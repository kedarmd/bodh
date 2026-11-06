use bodh::{DiscussionPublicationStore, ReviewPublicationStore};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn state_path() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "bodh-publication-history-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    root.join("state.sqlite")
}

#[test]
fn history_readers_return_empty_for_preparations_without_publications() {
    let state = state_path();
    let reviews = ReviewPublicationStore::open(&state).unwrap();
    let discussions = DiscussionPublicationStore::open(&state).unwrap();

    assert!(
        reviews
            .operation_history("missing-preparation")
            .unwrap()
            .is_empty()
    );
    assert!(
        discussions
            .plan_history("missing-preparation")
            .unwrap()
            .is_empty()
    );
}
