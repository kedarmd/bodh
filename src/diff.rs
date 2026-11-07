//! Deterministic merge-base-to-head anchors, separate from arbitrary readable source.
use crate::{ContractError, OperationControl, ReviewContext, Revision, SourceRef, evidence::git};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const MAX_DIFF_FILES: usize = 4096;
pub const MAX_DIFF_HUNKS: usize = 65_536;
/// Maximum unified patch text returned to an interactive workspace for one file.
pub const MAX_DIFF_VIEW_BYTES: usize = 128 * 1024;

/// A bounded unified diff for one path in a pinned merge-base-to-head comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedDiff {
    pub text: String,
    pub truncated: bool,
}

/// Reads displayable patch text for exactly one prepared changed path.
///
/// Git receives only pinned object IDs and a literal pathspec. External diff and
/// textconv are disabled, and the returned UTF-8 page is capped at
/// [`MAX_DIFF_VIEW_BYTES`].
pub fn read_diff(
    repository: &Path,
    context: &ReviewContext,
    path: &str,
    control: &OperationControl,
) -> Result<PinnedDiff, ContractError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(ContractError::new(
            "DIFF_PATH_INVALID",
            "Use an unambiguous repository-relative POSIX path",
            "/path",
        ));
    }
    let index = diff_index(repository, context, control)?;
    let file = index
        .files
        .iter()
        .find(|file| {
            file.old_path.as_deref() == Some(path) || file.new_path.as_deref() == Some(path)
        })
        .ok_or_else(|| {
            ContractError::new(
                "DIFF_PATH_UNAVAILABLE",
                "Path is not part of the prepared change",
                "/path",
            )
        })?;
    if let Some(reason) = &file.unsupported_reason {
        return Err(ContractError::new(
            "DIFF_CONTENT_UNSUPPORTED",
            "This changed entry does not have readable text diff content",
            reason,
        ));
    }
    let literal_path = format!(":(literal){path}");
    let patch = git(
        repository,
        control,
        &[
            "-c",
            "diff.suppressBlankEmpty=false",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--text",
            "--unified=3",
            "--inter-hunk-context=0",
            "--diff-algorithm=myers",
            "--no-indent-heuristic",
            "--no-color",
            "-O/dev/null",
            &context.merge_base_sha,
            &context.head_sha,
            "--",
            &literal_path,
        ],
    )
    .map_err(|error| {
        if error.code == "SOURCE_TOO_LARGE" {
            ContractError::new(
                "DIFF_TOO_LARGE",
                "Pinned patch exceeds the maximum Git output bound",
                "/path",
            )
        } else {
            error
        }
    })?;
    if patch.is_empty() {
        return Err(ContractError::new(
            "DIFF_PATH_UNAVAILABLE",
            "Pinned path has no displayable patch",
            "/path",
        ));
    }
    let text = std::str::from_utf8(&patch).map_err(|_| {
        ContractError::new(
            "DIFF_CONTENT_UNSUPPORTED",
            "Pinned patch output is not UTF-8 text",
            "/path",
        )
    })?;
    if text.len() <= MAX_DIFF_VIEW_BYTES {
        return Ok(PinnedDiff {
            text: text.to_owned(),
            truncated: false,
        });
    }
    let mut end = MAX_DIFF_VIEW_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end = text[..end].rfind('\n').map_or(0, |newline| newline + 1);
    if end == 0 {
        return Err(ContractError::new(
            "DIFF_TOO_LARGE",
            "Pinned patch has no complete line within the display bound",
            "/path",
        ));
    }
    Ok(PinnedDiff {
        text: text[..end].to_owned(),
        truncated: true,
    })
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangedFile {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub old_mode: String,
    pub new_mode: String,
    pub old_oid: String,
    pub new_oid: String,
    /// Git raw status, including the similarity score for a rename.
    pub status: String,
    pub unsupported_reason: Option<String>,
    pub hunks: Vec<DiffHunk>,
}
#[derive(Debug, Clone, Serialize)]
pub struct DiffIndex {
    pub files: Vec<ChangedFile>,
    context: ReviewContext,
}
impl DiffIndex {
    pub fn validate_anchor(&self, source: &SourceRef) -> Result<(), ContractError> {
        if source.start_line == 0 || source.end_line < source.start_line {
            return Err(anchor_error());
        }
        let file=self.files.iter().find(|file|match source.revision{Revision::Base=>file.old_path.as_deref(),Revision::Head=>file.new_path.as_deref()}==Some(source.path.as_str())).ok_or_else(anchor_error)?;
        if file.unsupported_reason.is_some() {
            return Err(ContractError::new(
                "INLINE_CONTENT_UNSUPPORTED",
                "This changed entry has no supported line anchors",
                "/path",
            ));
        }
        if file.hunks.iter().any(|h| {
            let (start, count) = match source.revision {
                Revision::Base => (h.old_start, h.old_lines),
                Revision::Head => (h.new_start, h.new_lines),
            };
            count > 0
                && source.start_line >= start
                && u64::from(source.end_line) < u64::from(start) + u64::from(count)
        }) {
            Ok(())
        } else {
            Err(anchor_error())
        }
    }
    pub fn context(&self) -> &ReviewContext {
        &self.context
    }
}
fn anchor_error() -> ContractError {
    ContractError::new(
        "INLINE_ANCHOR_INVALID",
        "Range must lie on one eligible side within a single pinned diff hunk",
        "/start_line",
    )
}
fn invalid_diff() -> ContractError {
    ContractError::new(
        "GIT_DIFF_INVALID",
        "Git comparison output is incomplete or unsupported",
        "",
    )
}

pub fn diff_index(
    repository: &Path,
    context: &ReviewContext,
    control: &OperationControl,
) -> Result<DiffIndex, ContractError> {
    crate::evidence::validate_prepared_context(repository, context, control)?;
    let raw = git(
        repository,
        control,
        &[
            "diff",
            "--raw",
            "-z",
            "--no-abbrev",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames=50%",
            "-l0",
            "--ignore-submodules=none",
            "--no-relative",
            "-O/dev/null",
            &context.merge_base_sha,
            &context.head_sha,
            "--",
        ],
    )?;
    let mut pieces = raw.split(|b| *b == 0);
    let mut files = Vec::new();
    let mut total_hunks = 0usize;
    while let Some(metadata) = pieces.next() {
        if metadata.is_empty() {
            if pieces.any(|p| !p.is_empty()) {
                return Err(invalid_diff());
            }
            break;
        }
        if files.len() >= MAX_DIFF_FILES {
            return Err(ContractError::new(
                "DIFF_TOO_LARGE",
                "Changed-file count exceeds the supported bound",
                "",
            ));
        }
        let fields = std::str::from_utf8(metadata)
            .map_err(|_| invalid_diff())?
            .strip_prefix(':')
            .ok_or_else(invalid_diff)?
            .split(' ')
            .collect::<Vec<_>>();
        if fields.len() != 5 {
            return Err(invalid_diff());
        }
        let status = fields[4];
        let kind = status
            .as_bytes()
            .first()
            .copied()
            .ok_or_else(invalid_diff)?;
        if !matches!(kind, b'A' | b'D' | b'M' | b'R' | b'T') {
            return Err(invalid_diff());
        }
        let first = path(pieces.next().ok_or_else(invalid_diff)?)?;
        let (old_path, new_path) = match kind {
            b'A' => (None, Some(first)),
            b'D' => (Some(first), None),
            b'R' => (
                Some(first),
                Some(path(pieces.next().ok_or_else(invalid_diff)?)?),
            ),
            _ => (Some(first.clone()), Some(first)),
        };
        let mut file = ChangedFile {
            old_path,
            new_path,
            old_mode: fields[0].into(),
            new_mode: fields[1].into(),
            old_oid: fields[2].into(),
            new_oid: fields[3].into(),
            status: status.into(),
            unsupported_reason: None,
            hunks: Vec::new(),
        };
        if [&file.old_mode, &file.new_mode]
            .into_iter()
            .any(|mode| !matches!(mode.as_str(), "000000" | "100644" | "100755"))
        {
            file.unsupported_reason = Some("non_regular_entry".into());
        } else {
            let old = if file.old_path.is_some() {
                git(repository, control, &["cat-file", "blob", &file.old_oid])?
            } else {
                Vec::new()
            };
            let new = if file.new_path.is_some() {
                git(repository, control, &["cat-file", "blob", &file.new_oid])?
            } else {
                Vec::new()
            };
            file.unsupported_reason = unsupported(&old)
                .or_else(|| unsupported(&new))
                .map(String::from);
            if file.unsupported_reason.is_none() {
                file.hunks = if file.old_path.is_none() {
                    whole_file(&new, false)?
                } else if file.new_path.is_none() {
                    whole_file(&old, true)?
                } else if file.old_oid == file.new_oid {
                    Vec::new()
                } else {
                    let patch = git(
                        repository,
                        control,
                        &[
                            "-c",
                            "diff.suppressBlankEmpty=false",
                            "diff",
                            "--no-ext-diff",
                            "--no-textconv",
                            "--text",
                            "--unified=3",
                            "--inter-hunk-context=0",
                            "--diff-algorithm=myers",
                            "--no-indent-heuristic",
                            "--no-color",
                            "--output-indicator-new=+",
                            "--output-indicator-old=-",
                            "--output-indicator-context= ",
                            "-O/dev/null",
                            &file.old_oid,
                            &file.new_oid,
                            "--",
                        ],
                    )?;
                    parse_hunks(&patch)?
                };
            }
        }
        total_hunks = total_hunks
            .checked_add(file.hunks.len())
            .ok_or_else(invalid_diff)?;
        if total_hunks > MAX_DIFF_HUNKS {
            return Err(ContractError::new(
                "DIFF_TOO_LARGE",
                "Hunk count exceeds the supported bound",
                "",
            ));
        }
        files.push(file);
        control.check()?;
    }
    Ok(DiffIndex {
        files,
        context: context.clone(),
    })
}
fn path(bytes: &[u8]) -> Result<String, ContractError> {
    let path = std::str::from_utf8(bytes).map_err(|_| {
        ContractError::new(
            "DIFF_PATH_UNSUPPORTED",
            "Diff paths must be UTF-8 POSIX paths",
            "",
        )
    })?;
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(ContractError::new(
            "DIFF_PATH_UNSUPPORTED",
            "Diff path is ambiguous or unsupported",
            "",
        ));
    }
    Ok(path.into())
}
fn unsupported(bytes: &[u8]) -> Option<&'static str> {
    if bytes.contains(&0) {
        Some("binary")
    } else if std::str::from_utf8(bytes).is_err() {
        Some("non_utf8")
    } else if bytes.starts_with(b"version https://git-lfs.github.com/spec/v1\n")
        || bytes.starts_with(b"version https://git-lfs.github.com/spec/v1\r\n")
    {
        Some("lfs_pointer")
    } else {
        None
    }
}
fn whole_file(bytes: &[u8], deleted: bool) -> Result<Vec<DiffHunk>, ContractError> {
    let count = u32::try_from(bytes.split_inclusive(|b| *b == b'\n').count())
        .map_err(|_| invalid_diff())?;
    if count == 0 {
        return Ok(Vec::new());
    }
    Ok(vec![if deleted {
        DiffHunk {
            old_start: 1,
            old_lines: count,
            new_start: 0,
            new_lines: 0,
        }
    } else {
        DiffHunk {
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: count,
        }
    }])
}
fn parse_hunks(patch: &[u8]) -> Result<Vec<DiffHunk>, ContractError> {
    let mut hunks = Vec::new();
    let mut remaining = None;
    for line in patch.split(|b| *b == b'\n') {
        if line.starts_with(b"@@ ") {
            if remaining.is_some_and(|counts| counts != (0, 0)) {
                return Err(invalid_diff());
            }
            let header = std::str::from_utf8(line)
                .map_err(|_| invalid_diff())?
                .split(' ')
                .collect::<Vec<_>>();
            if header.len() < 4 || header[3] != "@@" {
                return Err(invalid_diff());
            }
            let (old_start, old_lines) = span(header[1], '-')?;
            let (new_start, new_lines) = span(header[2], '+')?;
            hunks.push(DiffHunk {
                old_start,
                old_lines,
                new_start,
                new_lines,
            });
            remaining = Some((old_lines, new_lines));
            if hunks.len() > MAX_DIFF_HUNKS {
                return Err(ContractError::new(
                    "DIFF_TOO_LARGE",
                    "Hunk count exceeds the supported bound",
                    "",
                ));
            }
        } else if let Some((old, new)) = remaining.as_mut() {
            if line.starts_with(b"\\ ") || line.is_empty() && *old == 0 && *new == 0 {
                continue;
            }
            let (take_old, take_new) = match line.first() {
                Some(b' ') => (1, 1),
                Some(b'-') => (1, 0),
                Some(b'+') => (0, 1),
                _ => return Err(invalid_diff()),
            };
            *old = old.checked_sub(take_old).ok_or_else(invalid_diff)?;
            *new = new.checked_sub(take_new).ok_or_else(invalid_diff)?;
        }
    }
    if remaining.is_some_and(|counts| counts != (0, 0)) {
        return Err(invalid_diff());
    }
    Ok(hunks)
}
fn span(text: &str, prefix: char) -> Result<(u32, u32), ContractError> {
    let mut parts = text
        .strip_prefix(prefix)
        .ok_or_else(invalid_diff)?
        .split(',');
    let start = parts
        .next()
        .ok_or_else(invalid_diff)?
        .parse()
        .map_err(|_| invalid_diff())?;
    let count = parts
        .next()
        .map(str::parse)
        .transpose()
        .map_err(|_| invalid_diff())?
        .unwrap_or(1);
    if parts.next().is_some() || count > 0 && start == 0 {
        return Err(invalid_diff());
    }
    Ok((start, count))
}
