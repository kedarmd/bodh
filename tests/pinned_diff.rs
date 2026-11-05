use bodh::{
    OperationControl, PrIdentity, ReviewContext, Revision, SourceRef, read_diff, read_source,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
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
    String::from_utf8(output.stdout).unwrap().trim().into()
}

struct Fixture(PathBuf);
impl Fixture {
    fn new(base_text: &str, head_text: &str) -> (Self, ReviewContext) {
        let root = std::env::temp_dir().join(format!(
            "bodh-pinned-diff-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-b", "main"]);
        std::fs::write(root.join("src [main].rs"), base_text).unwrap();
        std::fs::write(root.join("other.rs"), "keep\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "base"]);
        let base = git(&root, &["rev-parse", "HEAD"]);
        std::fs::write(root.join("src [main].rs"), head_text).unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "head"]);
        let head = git(&root, &["rev-parse", "HEAD"]);
        // Dirty tracked and untracked content is intentionally present during inspection.
        std::fs::write(root.join("src [main].rs"), "dirty working copy\n").unwrap();
        std::fs::write(root.join("untracked.txt"), "leave me\n").unwrap();
        let context = ReviewContext {
            pr: PrIdentity {
                host: "github.com".into(),
                owner: "example".into(),
                repository: "demo".into(),
                number: 7,
            },
            context_id: "fixture-context".into(),
            base_sha: base.clone(),
            head_sha: head,
            merge_base_sha: base,
        };
        (Self(root), context)
    }

    fn state(&self) -> Vec<Vec<u8>> {
        vec![
            git(&self.0, &["rev-parse", "HEAD"]).into_bytes(),
            git(&self.0, &["symbolic-ref", "HEAD"]).into_bytes(),
            std::fs::read(self.0.join(".git/index")).unwrap(),
            std::fs::read(self.0.join("src [main].rs")).unwrap(),
            std::fs::read(self.0.join("untracked.txt")).unwrap(),
        ]
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn pinned_diff_is_path_scoped_literal_and_leaves_checkout_unchanged() {
    let (fixture, context) = Fixture::new(
        "fn main() {\n    let value = 1;\n    finish();\n}\n",
        "fn main() {\n    let value = 2;\n    finish();\n}\n",
    );
    let before = fixture.state();
    let page = read_diff(
        &fixture.0,
        &context,
        "src [main].rs",
        &OperationControl::default(),
    )
    .unwrap();
    assert!(
        page.text
            .contains("diff --git a/src [main].rs b/src [main].rs")
    );
    assert!(page.text.contains("@@ -1,4 +1,4 @@"));
    assert!(page.text.contains("-    let value = 1;"));
    assert!(page.text.contains("+    let value = 2;"));
    assert!(
        !page.text.contains("keep"),
        "diff must stay scoped to one file"
    );
    assert!(!page.truncated);
    for revision in [Revision::Base, Revision::Head] {
        let unchanged = read_source(
            &fixture.0,
            &context,
            &SourceRef {
                path: "src [main].rs".into(),
                revision,
                start_line: 1,
                end_line: 1,
            },
            &OperationControl::default(),
        )
        .unwrap();
        assert_eq!(unchanged, "fn main() {\n");
    }
    assert_eq!(fixture.state(), before);
}

#[test]
fn pinned_diff_truncates_at_a_complete_utf8_line_boundary() {
    let old = (0..1800)
        .map(|i| format!("let value_{i} = \"{}\";\n", "x".repeat(90)))
        .collect::<String>();
    let new = (0..1800)
        .map(|i| format!("let value_{i} = \"{}\";\n", "y".repeat(90)))
        .collect::<String>();
    let (fixture, context) = Fixture::new(&old, &new);
    let page = read_diff(
        &fixture.0,
        &context,
        "src [main].rs",
        &OperationControl::default(),
    )
    .unwrap();
    assert!(page.truncated);
    assert!(page.text.len() <= 128 * 1024);
    assert!(page.text.is_char_boundary(page.text.len()));
    assert!(page.text.ends_with('\n'));
}

#[test]
fn pinned_diff_rejects_paths_outside_the_prepared_change() {
    let (fixture, context) = Fixture::new("same\n", "changed\n");
    let error = read_diff(
        &fixture.0,
        &context,
        "other.rs",
        &OperationControl::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, "DIFF_PATH_UNAVAILABLE");
}
