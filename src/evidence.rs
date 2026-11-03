//! Immutable evidence validation shared by CLI and future workspace consumers.
use crate::{ContractError, ReviewContext, ReviewResult, Revision, SourceRef};
use serde::{Deserialize, Serialize};
use std::os::unix::process::CommandExt;
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// A trusted preparation snapshot. Loading this fixture does not acquire remote context.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedEvidenceContext {
    pub context: ReviewContext,
    #[serde(default, deserialize_with = "crate::bounded_optional_text")]
    pub original_description: Option<String>,
}
#[derive(Debug, Clone)]
pub struct OperationControl {
    started: Instant,
    timeout: Duration,
    cancelled: Arc<AtomicBool>,
}
impl Default for OperationControl {
    fn default() -> Self {
        Self::with_timeout(Duration::from_secs(30))
    }
}
impl OperationControl {
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            started: Instant::now(),
            timeout,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn check(&self) -> Result<(), ContractError> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(ContractError::new(
                "CANCELLED",
                "Operation was cancelled",
                "",
            ));
        }
        if self.started.elapsed() >= self.timeout {
            return Err(ContractError::new(
                "OPERATION_TIMEOUT",
                "Operation exceeded its time limit",
                "",
            ));
        }
        Ok(())
    }
}
pub const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;

pub fn validate_evidence(
    input: &[u8],
    prepared: &PreparedEvidenceContext,
    repository: &Path,
    control: &OperationControl,
) -> Result<ReviewResult, ContractError> {
    let result = crate::validate_structure(input)?;
    if result.context != prepared.context {
        return Err(ContractError::new(
            "CONTEXT_MISMATCH",
            "Submission context differs from the preparation",
            "/context",
        ));
    }
    validate_prepared_context(repository, &prepared.context, control)?;
    validate_content(&result)?;
    for (i, change) in result.overview.key_changes.iter().enumerate() {
        for (j, evidence) in change.evidence.iter().enumerate() {
            read_source(repository, &prepared.context, &evidence.source, control).map_err(
                |mut e| {
                    e.pointer =
                        format!("/overview/key_changes/{i}/evidence/{j}/source{}", e.pointer);
                    e
                },
            )?;
        }
    }
    for (i, assessment) in result.overview.intent_assessments.iter().enumerate() {
        let pointer = format!("/overview/intent_assessments/{i}");
        let description = prepared.original_description.as_ref().ok_or_else(|| {
            ContractError::new(
                "DESCRIPTION_UNAVAILABLE",
                "No original description is available for claim validation",
                &format!("{pointer}/quote"),
            )
        })?;
        if assessment.quote.is_empty()
            || assessment.occurrence == 0
            || description
                .char_indices()
                .filter(|(offset, _)| description[*offset..].starts_with(&assessment.quote))
                .nth(assessment.occurrence.saturating_sub(1) as usize)
                .is_none()
        {
            return Err(ContractError::new(
                "DESCRIPTION_QUOTE_INVALID",
                "Quote must be exact contiguous original text at the one-based occurrence",
                &format!("{pointer}/quote"),
            ));
        }
        for (j, evidence) in assessment.evidence.iter().enumerate() {
            read_source(repository, &prepared.context, &evidence.source, control).map_err(
                |mut e| {
                    e.pointer = format!("{pointer}/evidence/{j}/source{}", e.pointer);
                    e
                },
            )?;
        }
    }
    Ok(result)
}

/// Returns exact UTF-8 commit-blob lines, including original LF/CRLF delimiters.
pub fn read_source(
    repository: &Path,
    context: &ReviewContext,
    source: &SourceRef,
    control: &OperationControl,
) -> Result<String, ContractError> {
    if source.path.is_empty()
        || source.path.starts_with('/')
        || source.path.contains('\\')
        || source.path.chars().any(char::is_control)
        || source
            .path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(ContractError::new(
            "SOURCE_PATH_INVALID",
            "Use an unambiguous repository-relative POSIX path",
            "/path",
        ));
    }
    if source.start_line == 0 || source.end_line < source.start_line {
        return Err(ContractError::new(
            "SOURCE_RANGE_INVALID",
            "Line ranges are positive, inclusive and ordered",
            "/start_line",
        ));
    }
    let revision = match source.revision {
        Revision::Head => &context.head_sha,
        Revision::Base => &context.merge_base_sha,
    };
    if !valid_sha(revision) {
        return Err(ContractError::new(
            "REVISION_INVALID",
            "Pinned revisions must be full lowercase object IDs",
            "",
        ));
    }
    if git(repository, control, &["cat-file", "-t", revision])? != b"commit\n" {
        return Err(ContractError::new(
            "REVISION_INVALID",
            "Pinned revision must identify a commit",
            "",
        ));
    }
    let entry = git(
        repository,
        control,
        &[
            "ls-tree",
            "-rz",
            "--full-tree",
            revision,
            "--",
            &format!(":(literal){}", source.path),
        ],
    )?;
    let Some(tab) = entry.iter().position(|b| *b == b'\t') else {
        return Err(ContractError::new(
            "SOURCE_UNAVAILABLE",
            "Path is absent from the pinned tree",
            "/path",
        ));
    };
    let (metadata, tail) = entry.split_at(tab);
    let path = &tail[1..];
    if path != [source.path.as_bytes(), b"\0"].concat() {
        return Err(ContractError::new(
            "SOURCE_UNAVAILABLE",
            "Path is not a single pinned tree entry",
            "/path",
        ));
    }
    let fields = metadata.split(|b| *b == b' ').collect::<Vec<_>>();
    if fields.len() != 3 || !matches!(fields[0], b"100644" | b"100755") || fields[1] != b"blob" {
        return Err(ContractError::new(
            "SOURCE_CONTENT_UNSUPPORTED",
            "Only regular commit blobs have source lines",
            "/path",
        ));
    }
    let blob = std::str::from_utf8(fields[2])
        .map_err(|_| ContractError::new("GIT_FAILED", "Invalid tree entry", ""))?;
    let bytes = git(repository, control, &["cat-file", "blob", blob])?;
    if bytes.contains(&0)
        || bytes.starts_with(b"version https://git-lfs.github.com/spec/v1\n")
        || bytes.starts_with(b"version https://git-lfs.github.com/spec/v1\r\n")
    {
        return Err(ContractError::new(
            "SOURCE_CONTENT_UNSUPPORTED",
            "Binary and LFS pointer content has no supported source lines",
            "/path",
        ));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        ContractError::new("SOURCE_CONTENT_UNSUPPORTED", "Source is not UTF-8", "/path")
    })?;
    let lines = text.split_inclusive('\n').collect::<Vec<_>>();
    if source.end_line as usize > lines.len() {
        return Err(ContractError::new(
            "SOURCE_RANGE_INVALID",
            "Range exceeds the exact pinned blob",
            "/end_line",
        ));
    }
    Ok(lines[source.start_line as usize - 1..source.end_line as usize].concat())
}
fn valid_sha(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64)
        && sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn git(
    repository: &Path,
    control: &OperationControl,
    args: &[&str],
) -> Result<Vec<u8>, ContractError> {
    control.check()?;
    let mut command = Command::new("git");
    command
        .args(["--no-pager", "--no-replace-objects", "-C"])
        .arg(repository)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    // Ambient Git redirection must not select a different repository or index.
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
    let mut child = command.spawn().map_err(|_| {
        ContractError::new("GIT_UNAVAILABLE", "Installed git could not be started", "")
    })?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out = thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(MAX_SOURCE_BYTES as u64 + 1)
            .read_to_end(&mut bytes);
        (result, bytes)
    });
    let err = thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stderr.take(65_537).read_to_end(&mut bytes);
        (result, bytes)
    });
    let status = loop {
        if let Err(failure) = control.check() {
            // The child is a dedicated process-group leader. Reap all descendants before joining readers.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            return Err(failure);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(_) => {
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.wait();
                let _ = out.join();
                let _ = err.join();
                return Err(ContractError::new(
                    "GIT_FAILED",
                    "Cannot await Git operation",
                    "",
                ));
            }
        }
    };
    let (outcome, bytes) = out
        .join()
        .map_err(|_| ContractError::new("GIT_FAILED", "Cannot read Git output", ""))?;
    let (diagnostics, error_bytes) = err
        .join()
        .map_err(|_| ContractError::new("GIT_FAILED", "Cannot read Git diagnostics", ""))?;
    if bytes.len() > MAX_SOURCE_BYTES || error_bytes.len() > 65_536 {
        return Err(ContractError::new(
            "SOURCE_TOO_LARGE",
            "Git output exceeds the supported bound",
            "",
        ));
    }
    if outcome.is_err() || diagnostics.is_err() {
        return Err(ContractError::new(
            "GIT_FAILED",
            "Cannot read Git output",
            "",
        ));
    }
    if !status.success() {
        return Err(ContractError::new(
            "GIT_OBJECT_UNAVAILABLE",
            "Pinned Git object or tree is unavailable",
            "",
        ));
    }
    Ok(bytes)
}

fn validate_prepared_context(
    repository: &Path,
    context: &ReviewContext,
    control: &OperationControl,
) -> Result<(), ContractError> {
    let pr = &context.pr;
    let segment = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    };
    if pr.number == 0
        || !pr.host.split('.').all(segment)
        || !segment(&pr.owner)
        || pr.repository.is_empty()
        || matches!(pr.repository.as_str(), "." | "..")
        || !pr.repository.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
        })
    {
        return Err(ContractError::new(
            "PR_IDENTITY_INVALID",
            "Preparation requires normalized lowercase GitHub host/owner/repository and a positive PR number",
            "/context/pr",
        ));
    }
    if context.context_id.trim().is_empty() {
        return Err(ContractError::new(
            "PREPARED_CONTEXT_INVALID",
            "Preparation requires a context identity",
            "/context/context_id",
        ));
    }
    for (field, revision) in [
        ("base_sha", &context.base_sha),
        ("head_sha", &context.head_sha),
        ("merge_base_sha", &context.merge_base_sha),
    ] {
        if !valid_sha(revision) {
            return Err(ContractError::new(
                "REVISION_INVALID",
                "Pinned revisions must be full lowercase object IDs",
                &format!("/context/{field}"),
            ));
        }
        if git(repository, control, &["cat-file", "-t", revision])? != b"commit\n" {
            return Err(ContractError::new(
                "REVISION_INVALID",
                "Pinned revision must identify a commit",
                &format!("/context/{field}"),
            ));
        }
    }
    let merge_base = git(
        repository,
        control,
        &["merge-base", "--all", &context.base_sha, &context.head_sha],
    )?;
    if merge_base != format!("{}\n", context.merge_base_sha).as_bytes() {
        return Err(ContractError::new(
            "PREPARED_CONTEXT_INVALID",
            "Preparation must contain the unique base/head merge-base",
            "/context/merge_base_sha",
        ));
    }
    Ok(())
}

fn require_text(text: &str, pointer: &str) -> Result<(), ContractError> {
    if text.trim().is_empty() {
        return Err(ContractError::new(
            "EVIDENCE_CONTENT_INVALID",
            "Expected nonempty explanatory text",
            pointer,
        ));
    }
    Ok(())
}
fn validate_content(result: &ReviewResult) -> Result<(), ContractError> {
    require_text(
        &result.overview.implementation_summary,
        "/overview/implementation_summary",
    )?;
    for (i, change) in result.overview.key_changes.iter().enumerate() {
        let p = format!("/overview/key_changes/{i}");
        require_text(&change.summary, &format!("{p}/summary"))?;
        if change.evidence.is_empty() {
            return Err(ContractError::new(
                "EVIDENCE_CONTENT_INVALID",
                "A key change needs source evidence",
                &format!("{p}/evidence"),
            ));
        }
        for (j, e) in change.evidence.iter().enumerate() {
            require_text(&e.explanation, &format!("{p}/evidence/{j}/explanation"))?;
        }
    }
    for (i, a) in result.overview.intent_assessments.iter().enumerate() {
        let p = format!("/overview/intent_assessments/{i}");
        require_text(&a.explanation, &format!("{p}/explanation"))?;
        for (j, e) in a.evidence.iter().enumerate() {
            require_text(&e.explanation, &format!("{p}/evidence/{j}/explanation"))?;
        }
    }
    for (i, area) in result.coverage.examined_areas.iter().enumerate() {
        require_text(area, &format!("/coverage/examined_areas/{i}"))?;
    }
    for (i, check) in result.coverage.checks.iter().enumerate() {
        require_text(&check.name, &format!("/coverage/checks/{i}/name"))?;
        require_text(&check.details, &format!("/coverage/checks/{i}/details"))?;
    }
    for (i, area) in result.coverage.excluded_areas.iter().enumerate() {
        require_text(&area.area, &format!("/coverage/excluded_areas/{i}/area"))?;
        require_text(
            &area.reason,
            &format!("/coverage/excluded_areas/{i}/reason"),
        )?;
    }
    for (i, limitation) in result.coverage.limitations.iter().enumerate() {
        require_text(limitation, &format!("/coverage/limitations/{i}"))?;
    }
    Ok(())
}
