use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GhCliStatus {
    Checking,
    Available {
        executable: PathBuf,
        version: String,
    },
    Missing,
    Failed(String),
}

pub fn probe_gh_cli(timeout: Duration) -> GhCliStatus {
    probe_gh_cli_with(Path::new("gh"), timeout)
}

pub fn probe_gh_cli_with(executable: &Path, timeout: Duration) -> GhCliStatus {
    let mut child = match Command::new(executable)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return GhCliStatus::Missing,
        Err(_) => return GhCliStatus::Failed("Could not start the GitHub CLI check.".into()),
    };
    let Some(stdout) = child.stdout.take() else {
        return GhCliStatus::Failed("Could not read the GitHub CLI version.".into());
    };
    probe_running_gh_child(child, stdout, executable, timeout)
}

trait GhProbeChild {
    fn try_wait(&mut self) -> std::io::Result<Option<bool>>;
    fn kill(&mut self) -> std::io::Result<()>;
    fn wait(&mut self) -> std::io::Result<()>;
}

impl GhProbeChild for Child {
    fn try_wait(&mut self) -> std::io::Result<Option<bool>> {
        Child::try_wait(self).map(|status| status.map(|status| status.success()))
    }

    fn kill(&mut self) -> std::io::Result<()> {
        Child::kill(self)
    }

    fn wait(&mut self) -> std::io::Result<()> {
        Child::wait(self).map(|_| ())
    }
}

fn probe_running_gh_child(
    mut child: impl GhProbeChild,
    stdout: impl Read + Send + 'static,
    executable: &Path,
    timeout: Duration,
) -> GhCliStatus {
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.take(4097).read_to_end(&mut output);
        output
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(success)) => {
                let output = reader.join().unwrap_or_default();
                if !success || output.len() > 4096 {
                    return GhCliStatus::Failed("GitHub CLI version check failed.".into());
                }
                let version = String::from_utf8_lossy(&output)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if version.starts_with("gh version ") {
                    return GhCliStatus::Available {
                        executable: executable.to_path_buf(),
                        version,
                    };
                }
                return GhCliStatus::Failed("GitHub CLI returned an unrecognized version.".into());
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return GhCliStatus::Failed("Timed out checking the GitHub CLI.".into());
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return GhCliStatus::Failed("Could not inspect the GitHub CLI process.".into());
            }
        }
    }
}

/// Expands the home shorthand used in repository path fields before passing the
/// path to subprocess APIs, which do not perform shell tilde expansion.
pub fn expand_repository_path(raw: &str, home: &Path) -> PathBuf {
    let raw = raw.trim();
    if raw == "~" {
        home.to_path_buf()
    } else if let Some(relative) = raw.strip_prefix("~/") {
        home.join(relative)
    } else {
        PathBuf::from(raw)
    }
}

pub fn resolve_repository_path(raw: &str, home: &Path) -> Result<PathBuf, bodh::ContractError> {
    let expanded = expand_repository_path(raw, home);
    if expanded.as_os_str().is_empty() {
        return Err(bodh::ContractError::new(
            "REPOSITORY_PATH_INVALID",
            "Enter a repository checkout path.",
            "",
        ));
    }
    let canonical = expanded.canonicalize().map_err(|_| {
        bodh::ContractError::new(
            "REPOSITORY_PATH_INVALID",
            "The repository path does not exist or cannot be opened.",
            "",
        )
    })?;
    if !canonical.is_dir() {
        return Err(bodh::ContractError::new(
            "REPOSITORY_PATH_INVALID",
            "Choose a directory containing a Git checkout.",
            "",
        ));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::{
        GhCliStatus, GhProbeChild, expand_repository_path, probe_gh_cli_with,
        probe_running_gh_child, resolve_repository_path,
    };
    use std::{
        fs, io,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tempfile::TempDir;

    fn temp_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn expands_tilde_checkout_paths_before_they_become_process_working_directories() {
        assert_eq!(
            expand_repository_path("~/dev/nayati", Path::new("/home/kedarmd")),
            PathBuf::from("/home/kedarmd/dev/nayati")
        );
    }

    #[test]
    fn preflight_reports_a_missing_executable_separately() {
        let missing = temp_dir().path().join("gh");
        assert_eq!(
            probe_gh_cli_with(&missing, Duration::from_secs(1)),
            GhCliStatus::Missing
        );
    }

    #[test]
    fn preflight_reports_the_installed_version_and_keeps_executable_path_for_follow_up_calls() {
        let root = temp_dir();
        let gh = root.path().join("gh");
        fs::write(&gh, "#!/bin/sh\nprintf 'gh version 2.9.1 (test)\\n'\n").unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();

        let status = probe_gh_cli_with(&gh, Duration::from_secs(3));
        let GhCliStatus::Available {
            executable,
            version,
        } = status
        else {
            panic!("controlled gh executable should be available, got {status:?}");
        };

        assert_eq!(executable, gh);
        assert_eq!(version, "gh version 2.9.1 (test)");
    }

    #[test]
    fn preflight_terminates_a_stalled_cli_at_its_deadline() {
        #[derive(Default)]
        struct ChildState {
            killed: bool,
            waited: bool,
        }

        struct StalledChild(Arc<Mutex<ChildState>>);

        impl GhProbeChild for StalledChild {
            fn try_wait(&mut self) -> io::Result<Option<bool>> {
                Ok(None)
            }

            fn kill(&mut self) -> io::Result<()> {
                self.0.lock().unwrap().killed = true;
                Ok(())
            }

            fn wait(&mut self) -> io::Result<()> {
                self.0.lock().unwrap().waited = true;
                Ok(())
            }
        }

        let state = Arc::new(Mutex::new(ChildState::default()));
        let started = std::time::Instant::now();
        let status = probe_running_gh_child(
            StalledChild(state.clone()),
            io::empty(),
            Path::new("gh"),
            Duration::from_millis(100),
        );

        assert_eq!(
            status,
            GhCliStatus::Failed("Timed out checking the GitHub CLI.".into())
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "preflight did not stop the stalled CLI promptly: {:?}",
            started.elapsed()
        );
        let state = state.lock().unwrap();
        assert!(state.killed, "stalled child must be killed at the deadline");
        assert!(state.waited, "stalled child must be reaped after killing");
    }

    #[test]
    fn resolver_expands_a_tilde_path_and_returns_an_absolute_checkout() {
        let root = temp_dir();
        let checkout = root.path().join("dev/nayati");
        fs::create_dir_all(&checkout).unwrap();
        assert_eq!(
            resolve_repository_path("~/dev/nayati", root.path()).unwrap(),
            checkout.canonicalize().unwrap()
        );
    }

    #[test]
    fn invalid_checkout_path_has_its_own_error_code() {
        let error =
            resolve_repository_path("~/missing", Path::new("/tmp/bodh-test-home")).unwrap_err();
        assert_eq!(error.code, "REPOSITORY_PATH_INVALID");
    }
}
