//! Immutable evidence validation shared by CLI and future workspace consumers.
use crate::{ContractError, ReviewContext, ReviewResult, Revision, SourceRef};
use serde::{Deserialize, Serialize};
use std::{path::Path, process::Command};

/// A trusted preparation snapshot. Loading this fixture does not acquire remote context.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedEvidenceContext {
    pub context: ReviewContext,
    #[serde(default, deserialize_with = "crate::bounded_optional_text")]
    pub original_description: Option<String>,
}
#[derive(Debug, Default)]
pub struct OperationControl;

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
    Ok(result)
}

/// Returns exact UTF-8 commit-blob lines, including original LF/CRLF delimiters.
pub fn read_source(
    repository: &Path,
    context: &ReviewContext,
    source: &SourceRef,
    _control: &OperationControl,
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
    if git(repository, &["cat-file", "-t", revision])? != b"commit\n" {
        return Err(ContractError::new(
            "REVISION_INVALID",
            "Pinned revision must identify a commit",
            "",
        ));
    }
    let entry = git(
        repository,
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
    let bytes = git(repository, &["cat-file", "blob", blob])?;
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
fn git(repository: &Path, args: &[&str]) -> Result<Vec<u8>, ContractError> {
    let output = Command::new("git")
        .arg("--no-pager")
        .arg("--no-replace-objects")
        .arg("-C")
        .arg(repository)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|_| {
            ContractError::new("GIT_UNAVAILABLE", "Installed git could not be started", "")
        })?;
    if !output.status.success() {
        return Err(ContractError::new(
            "GIT_OBJECT_UNAVAILABLE",
            "Pinned Git object or tree is unavailable",
            "",
        ));
    }
    Ok(output.stdout)
}
