//! Safe, pinned Git object acquisition for prepared pull requests.
use crate::{ContractError, OperationControl, evidence::git};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

/// Ensure a commit is available and retained under a Bodh-owned ref without touching HEAD,
/// the index, or the working tree. The source is always an explicit advertised ref or SHA.
pub(crate) fn acquire_commit(
    repository: &Path,
    control: &OperationControl,
    remote_url: &str,
    source_ref: &str,
    expected_sha: &str,
    context_key: &str,
) -> Result<(), ContractError> {
    if !valid_sha(expected_sha) {
        return Err(ContractError::new(
            "GH_RESPONSE_INVALID",
            "Pinned commit SHA from GitHub is invalid",
            "",
        ));
    }
    control.check()?;
    let retention_ref = retention_ref(context_key, expected_sha);
    if let Some(existing) = ref_target(repository, control, &retention_ref)? {
        return if existing == expected_sha {
            Ok(())
        } else {
            Err(ContractError::new(
                "GIT_REF_CONFLICT",
                "Stored Bodh object reference conflicts with the pinned commit",
                "",
            ))
        };
    }
    if commit_exists(repository, control, expected_sha) {
        git(
            repository,
            control,
            &["update-ref", &retention_ref, expected_sha],
        )?;
        return Ok(());
    }

    let fetched = fetch_ref(
        repository,
        control,
        remote_url,
        source_ref,
        &retention_ref,
        expected_sha,
    );
    if fetched.is_ok() {
        return Ok(());
    }
    // Some Git servers reject advertised refs but permit direct fetch of a retained SHA.
    let direct = format!("+{expected_sha}:{retention_ref}");
    match fetch_refspec(
        repository,
        control,
        remote_url,
        &direct,
        &retention_ref,
        expected_sha,
    ) {
        Ok(()) => Ok(()),
        Err(error) if interrupted(&error) => Err(error),
        Err(_) => fetched,
    }
}

fn commit_exists(repository: &Path, control: &OperationControl, sha: &str) -> bool {
    git(
        repository,
        control,
        &["cat-file", "-e", &format!("{sha}^{{commit}}")],
    )
    .is_ok()
}

fn valid_sha(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64)
        && sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn retention_ref(context_key: &str, sha: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"bodh-retained-object-v1\0");
    hash.update(context_key.as_bytes());
    hash.update(b"\0");
    hash.update(sha.as_bytes());
    format!("refs/bodh/prepared/{:x}", hash.finalize())
}

fn ref_target(
    repository: &Path,
    control: &OperationControl,
    reference: &str,
) -> Result<Option<String>, ContractError> {
    match git(repository, control, &["rev-parse", "--verify", reference]) {
        Ok(bytes) => Ok(Some(
            std::str::from_utf8(&bytes)
                .map_err(|_| git_object_error())?
                .trim()
                .to_owned(),
        )),
        Err(error) if error.code == "GIT_OBJECT_UNAVAILABLE" => Ok(None),
        Err(error) => Err(error),
    }
}

fn fetch_ref(
    repository: &Path,
    control: &OperationControl,
    remote_url: &str,
    source_ref: &str,
    destination: &str,
    expected_sha: &str,
) -> Result<(), ContractError> {
    if !source_ref.starts_with("refs/") {
        return Err(git_object_error());
    }
    git(repository, control, &["check-ref-format", source_ref])?;
    let refspec = format!("+{source_ref}:{destination}");
    fetch_refspec(
        repository,
        control,
        remote_url,
        &refspec,
        destination,
        expected_sha,
    )
}

fn fetch_refspec(
    repository: &Path,
    control: &OperationControl,
    remote_url: &str,
    refspec: &str,
    destination: &str,
    expected_sha: &str,
) -> Result<(), ContractError> {
    run_fetch(repository, control, remote_url, refspec)?;
    let actual = ref_target(repository, control, destination)?.ok_or_else(git_object_error)?;
    if actual != expected_sha {
        let _ = git(repository, control, &["update-ref", "-d", destination]);
        return Err(ContractError::new(
            "PR_REF_MOVED",
            "The advertised PR ref moved after its immutable SHA was observed",
            "",
        ));
    }
    if !commit_exists(repository, control, expected_sha) {
        let _ = git(repository, control, &["update-ref", "-d", destination]);
        return Err(git_object_error());
    }
    Ok(())
}

fn run_fetch(
    repository: &Path,
    control: &OperationControl,
    remote_url: &str,
    refspec: &str,
) -> Result<(), ContractError> {
    run_fetch_program(Path::new("git"), repository, control, remote_url, refspec)
}

fn run_fetch_program(
    program: &Path,
    repository: &Path,
    control: &OperationControl,
    remote_url: &str,
    refspec: &str,
) -> Result<(), ContractError> {
    let mut command = Command::new(program);
    command
        .args(["--no-pager", "--no-replace-objects", "-C"])
        .arg(repository)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "fetch.writeCommitGraph=false",
            "-c",
            "credential.helper=",
            "-c",
            "credential.helper=!gh auth git-credential",
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
            "fetch",
            "--no-tags",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            "--no-auto-maintenance",
            "--no-write-commit-graph",
            "--",
        ])
        .arg(remote_url)
        .arg(refspec)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    for (name, _) in std::env::vars_os() {
        if name.as_encoded_bytes().starts_with(b"GIT_") {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_GRAFT_FILE", "/dev/null");
    let mut child = command.spawn().map_err(|_| git_object_error())?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out = thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(65_537).read_to_end(&mut bytes);
        (result, bytes)
    });
    let err = thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stderr.take(65_537).read_to_end(&mut bytes);
        (result, bytes)
    });
    let mut completed: Option<std::process::ExitStatus> = None;
    loop {
        if let Err(failure) = control.check() {
            unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            return Err(failure);
        }
        if let Some(status) = completed
            && out.is_finished()
            && err.is_finished()
        {
            let (outcome, stdout) = out.join().map_err(|_| git_object_error())?;
            let (diagnostics, stderr) = err.join().map_err(|_| git_object_error())?;
            if outcome.is_err()
                || diagnostics.is_err()
                || stdout.len() > 65_536
                || stderr.len() > 65_536
            {
                return Err(git_object_error());
            }
            return if status.success() {
                Ok(())
            } else {
                Err(git_object_error())
            };
        }
        match child.try_wait() {
            Ok(Some(status)) => completed = Some(status),
            Ok(None) => {}
            Err(_) => {
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
                let _ = child.wait();
                let _ = out.join();
                let _ = err.join();
                return Err(git_object_error());
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn git_object_error() -> ContractError {
    ContractError::new(
        "GIT_OBJECT_UNAVAILABLE",
        "The exact pinned Git commit could not be acquired from the selected repository refs",
        "",
    )
}

fn interrupted(error: &ContractError) -> bool {
    matches!(error.code.as_str(), "CANCELLED" | "OPERATION_TIMEOUT")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, process::Command};

    fn git(repository: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository)
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
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn cancelled_fetch_preserves_checkout_state() {
        let root = std::env::temp_dir().join(format!("bodh-fetch-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("tracked.txt"), "initial\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "initial"]);
        std::fs::write(repo.join("tracked.txt"), "dirty\n").unwrap();
        std::fs::write(repo.join("staged.txt"), "staged\n").unwrap();
        git(&repo, &["add", "staged.txt"]);
        std::fs::write(repo.join("untracked.txt"), "untracked\n").unwrap();
        let before = (
            git(&repo, &["rev-parse", "HEAD"]),
            git(&repo, &["symbolic-ref", "HEAD"]),
            std::fs::read(repo.join(".git/index")).unwrap(),
            std::fs::read(repo.join("tracked.txt")).unwrap(),
            std::fs::read(repo.join("staged.txt")).unwrap(),
            std::fs::read(repo.join("untracked.txt")).unwrap(),
        );
        let fake = root.join("git-slow-fetch");
        std::fs::write(
            &fake,
            "#!/bin/sh\ncase \" $* \" in *\" fetch \"*) exec /bin/sleep 5;; esac\nexec /usr/bin/git \"$@\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let failure = run_fetch_program(
            &fake,
            &repo,
            &OperationControl::with_timeout(Duration::from_millis(100)),
            "https://example.invalid/repository.git",
            "+refs/heads/main:refs/bodh/test",
        )
        .unwrap_err();
        assert_eq!(failure.code, "OPERATION_TIMEOUT");
        assert_eq!(before.0, git(&repo, &["rev-parse", "HEAD"]));
        assert_eq!(before.1, git(&repo, &["symbolic-ref", "HEAD"]));
        assert_eq!(before.2, std::fs::read(repo.join(".git/index")).unwrap());
        assert_eq!(before.3, std::fs::read(repo.join("tracked.txt")).unwrap());
        assert_eq!(before.4, std::fs::read(repo.join("staged.txt")).unwrap());
        assert_eq!(before.5, std::fs::read(repo.join("untracked.txt")).unwrap());
        let _ = std::fs::remove_dir_all(root);
    }
}
