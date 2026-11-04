use bodh::{
    OperationControl, PrIdentity, PreparationStore, ReviewContext, prepare_local_pr_with_gh,
};
use std::{
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn context() -> ReviewContext {
    ReviewContext {
        pr: PrIdentity {
            host: "github.com".into(),
            owner: "acme".into(),
            repository: "widget".into(),
            number: 17,
        },
        context_id: "ctx-1".into(),
        base_sha: "a".repeat(40),
        head_sha: "b".repeat(40),
        merge_base_sha: "c".repeat(40),
    }
}

fn git(repo: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("git")
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
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

#[test]
fn prepares_ordinary_pr_from_pinned_local_commits_without_touching_checkout() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!(
        "bodh-acquire-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("src.txt"), "base\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "base"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["checkout", "-b", "feature"]);
    std::fs::write(repo.join("src.txt"), "head\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "head"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widget.git",
        ],
    );
    std::fs::write(repo.join("src.txt"), "dirty worktree\n").unwrap();
    std::fs::write(repo.join("staged.txt"), "staged\n").unwrap();
    git(&repo, &["add", "staged.txt"]);
    std::fs::write(repo.join("dirty.txt"), "preserve\n").unwrap();
    let fake = root.join("gh");
    let response = format!(
        r#"{{"title":"first","body":"description","base":{{"repo":{{"full_name":"acme/widget"}},"sha":"{base}"}},"head":{{"sha":"{head}"}}}}"#
    );
    std::fs::write(&fake, format!("#!/bin/sh\nprintf '%s' '{response}'\n")).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let state = root.join("state.sqlite");
    let snapshot = || {
        vec![
            git(&repo, &["rev-parse", "HEAD"]).into_bytes(),
            git(&repo, &["symbolic-ref", "HEAD"]).into_bytes(),
            std::fs::read(repo.join(".git/index")).unwrap(),
            std::fs::read(repo.join("src.txt")).unwrap(),
            std::fs::read(repo.join("staged.txt")).unwrap(),
            std::fs::read(repo.join("dirty.txt")).unwrap(),
        ]
    };
    let before = snapshot();
    let first =
        prepare_local_pr_with_gh(&repo, 17, None, &state, &OperationControl::default(), &fake)
            .unwrap();
    assert_eq!(first.title, "first");
    assert_eq!(first.changed_files.len(), 1);
    assert_eq!(first.context.head_sha, head);
    assert!(first.original_description_available);
    assert_eq!(
        std::fs::read(repo.join("dirty.txt")).unwrap(),
        b"preserve\n"
    );
    assert_eq!(snapshot(), before);
    let loaded = PreparationStore::open(&state)
        .unwrap()
        .load(&first.preparation_id)
        .unwrap();
    assert_eq!(
        loaded.checkout,
        repo.canonicalize().unwrap().to_string_lossy()
    );
    let patch = bodh::inspect_diff(&loaded, "src.txt", 0, &OperationControl::default()).unwrap();
    assert!(patch["patch"].as_str().unwrap().contains("+head"));
    assert_eq!(patch["truncated"], false);
    let old_path = std::env::var("PATH").unwrap_or_default();
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::copy(&fake, bin.join("gh")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_bodh"))
        .args([
            "prepare",
            "--repo",
            repo.to_str().unwrap(),
            "--pr",
            "17",
            "--state",
            state.to_str().unwrap(),
        ])
        .env("PATH", format!("{}:{old_path}", bin.display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["protocol_version"], "1.0");
    assert_eq!(envelope["ok"], true);
    assert_ne!(envelope["data"]["preparation_id"], first.preparation_id);
    let cli_id = envelope["data"]["preparation_id"].as_str().unwrap();
    let inspected = Command::new(env!("CARGO_BIN_EXE_bodh"))
        .args([
            "inspect",
            "--prepared",
            cli_id,
            "--kind",
            "source",
            "--state",
            state.to_str().unwrap(),
            "--path",
            "src.txt",
            "--revision",
            "head",
            "--start-line",
            "1",
            "--end-line",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        inspected.status.success(),
        "{}",
        String::from_utf8_lossy(&inspected.stderr)
    );
    let inspected: serde_json::Value = serde_json::from_slice(&inspected.stdout).unwrap();
    assert_eq!(inspected["data"]["text"], "head\n");
    let missing = prepare_local_pr_with_gh(
        &repo,
        17,
        None,
        &state,
        &OperationControl::default(),
        &root.join("absent-gh"),
    )
    .unwrap_err();
    assert_eq!(missing.code, "GH_UNAVAILABLE");
    assert_eq!(snapshot(), before);
    let denied = root.join("denied-gh");
    std::fs::write(
        &denied,
        "#!/bin/sh\necho 'HTTP 401 authentication required' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o755)).unwrap();
    let denied = prepare_local_pr_with_gh(
        &repo,
        17,
        None,
        &state,
        &OperationControl::default(),
        &denied,
    )
    .unwrap_err();
    assert_eq!(denied.code, "AUTH_ACCESS");
    assert_eq!(snapshot(), before);
    let not_found = root.join("not-found-gh");
    std::fs::write(
        &not_found,
        "#!/bin/sh\necho 'HTTP 404 Not Found' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&not_found, std::fs::Permissions::from_mode(0o755)).unwrap();
    let denied = prepare_local_pr_with_gh(
        &repo,
        17,
        None,
        &state,
        &OperationControl::default(),
        &not_found,
    )
    .unwrap_err();
    assert_eq!(denied.code, "AUTH_ACCESS");
    assert_eq!(snapshot(), before);
    let offline = root.join("offline-gh");
    std::fs::write(
        &offline,
        "#!/bin/sh\necho 'Could not resolve host: api.github.com' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&offline, std::fs::Permissions::from_mode(0o755)).unwrap();
    let failure = prepare_local_pr_with_gh(
        &repo,
        17,
        None,
        &state,
        &OperationControl::default(),
        &offline,
    )
    .unwrap_err();
    assert_eq!(failure.code, "NETWORK_UNCERTAIN");
    assert_eq!(snapshot(), before);
    let slow = root.join("slow-gh");
    std::fs::write(&slow, "#!/bin/sh\nsleep 3\n").unwrap();
    std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755)).unwrap();
    let timed = prepare_local_pr_with_gh(
        &repo,
        17,
        None,
        &state,
        &OperationControl::with_timeout(std::time::Duration::from_millis(100)),
        &slow,
    )
    .unwrap_err();
    assert_eq!(timed.code, "OPERATION_TIMEOUT");
    assert_eq!(snapshot(), before);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn repeated_preparations_keep_distinct_ids_and_generation_bindings_after_reopen() {
    let dir = std::env::temp_dir().join(format!(
        "bodh-preparation-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("review.sqlite3");
    let first;
    {
        let store = PreparationStore::open(&path).unwrap();
        first = store
            .save(&context(), "/checkout", "title", None, &[])
            .unwrap();
        let second = store
            .save(
                &context(),
                "/checkout",
                "edited title",
                Some("new body"),
                &[],
            )
            .unwrap();
        assert_ne!(first.preparation_id, second.preparation_id);
        assert_eq!(first.expected_generation, 0);
        assert_eq!(second.expected_generation, 0);
        assert_eq!(second.title, "edited title");
    }
    let reopened = PreparationStore::open(&path).unwrap();
    let loaded = reopened.load(&first.preparation_id).unwrap();
    assert_eq!(loaded.context.context_id, "ctx-1");
    assert_eq!(loaded.expected_generation, 0);
    assert_eq!(loaded.title, "title");
    assert_eq!(loaded.description, None);
    let second_id = {
        let store = PreparationStore::open(&path).unwrap();
        store
            .save(&context(), "/checkout", "third title", None, &[])
            .unwrap()
            .preparation_id
    };
    assert_eq!(reopened.load(&second_id).unwrap().title, "third title");
    let _ = std::fs::remove_dir_all(dir);
}
