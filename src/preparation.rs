//! Durable immutable preparation bindings shared by headless adapters.
use crate::{ContractError, ReviewContext};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedBinding {
    pub preparation_id: String,
    pub expected_generation: u64,
    pub context: ReviewContext,
    pub title: String,
    pub description: Option<String>,
    pub changed_files: Vec<crate::ChangedFile>,
    pub checkout: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangedFileSummary {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub status: String,
    pub unsupported_reason: Option<String>,
    pub hunk_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreparationManifest {
    pub preparation_id: String,
    pub expected_generation: u64,
    pub context: ReviewContext,
    pub title: String,
    pub original_description_available: bool,
    pub changed_files: Vec<ChangedFileSummary>,
    pub discussions_complete: bool,
    pub previous_result_available: bool,
    pub draft_counts: DraftCounts,
    pub supported_contract_versions: Vec<&'static str>,
    pub inspect: Vec<&'static str>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DraftCounts {
    pub inline_comments: u64,
    pub replies: u64,
    pub thread_intents: u64,
    pub code_replacements: u64,
    pub review_comments: u64,
}
impl From<PreparedBinding> for PreparationManifest {
    fn from(p: PreparedBinding) -> Self {
        Self {
            preparation_id: p.preparation_id,
            expected_generation: p.expected_generation,
            context: p.context,
            title: p.title,
            original_description_available: p.description.is_some(),
            changed_files: p
                .changed_files
                .into_iter()
                .map(|f| ChangedFileSummary {
                    old_path: f.old_path,
                    new_path: f.new_path,
                    status: f.status,
                    unsupported_reason: f.unsupported_reason,
                    hunk_count: f.hunks.len(),
                })
                .collect(),
            discussions_complete: false,
            previous_result_available: false,
            draft_counts: DraftCounts {
                inline_comments: 0,
                replies: 0,
                thread_intents: 0,
                code_replacements: 0,
                review_comments: 0,
            },
            supported_contract_versions: vec!["1.0"],
            inspect: vec![
                "manifest",
                "description",
                "files",
                "diff",
                "source",
                "discussions",
                "previous",
            ],
        }
    }
}

pub struct PreparationStore {
    db: Connection,
}
impl PreparationStore {
    pub fn open(path: &Path) -> Result<Self, ContractError> {
        let db = Connection::open(path).map_err(db_error)?;
        crate::sqlite::configure(&db).map_err(db_error)?;
        // Serialize the version read with the migration. Reading user_version
        // before taking the write lock lets two first-open processes both
        // decide that the schema is empty and race to create the same tables.
        db.execute_batch("BEGIN IMMEDIATE;").map_err(db_error)?;
        let version: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(db_error)?;
        if version > 1 {
            let _ = db.execute_batch("ROLLBACK;");
            return Err(ContractError::new(
                "STATE_VERSION_UNSUPPORTED",
                "State database version is newer than this application",
                "",
            ));
        }
        if version == 0 {
            db.execute_batch("CREATE TABLE workspaces (host TEXT NOT NULL, owner TEXT NOT NULL, repository TEXT NOT NULL, number INTEGER NOT NULL, generation INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(host,owner,repository,number)); CREATE TABLE contexts (context_id TEXT PRIMARY KEY, context_json TEXT NOT NULL, changed_files_json TEXT NOT NULL); CREATE TABLE preparations (preparation_id TEXT PRIMARY KEY, context_id TEXT NOT NULL REFERENCES contexts(context_id), host TEXT NOT NULL, owner TEXT NOT NULL, repository TEXT NOT NULL, number INTEGER NOT NULL, expected_generation INTEGER NOT NULL, created_at INTEGER NOT NULL, title TEXT NOT NULL, description TEXT, checkout TEXT NOT NULL, FOREIGN KEY(host,owner,repository,number) REFERENCES workspaces(host,owner,repository,number)); PRAGMA user_version=1;").map_err(db_error)?;
        }
        db.execute_batch("COMMIT;").map_err(db_error)?;
        Ok(Self { db })
    }

    /// Persists a completed acquisition in one short transaction. The caller performs Git/gh first.
    pub fn save(
        &self,
        context: &ReviewContext,
        checkout: &str,
        title: &str,
        description: Option<&str>,
        files: &[crate::ChangedFile],
    ) -> Result<PreparedBinding, ContractError> {
        let tx = self.db.unchecked_transaction().map_err(db_error)?;
        let pr = &context.pr;
        let number = i64::try_from(pr.number).map_err(|_| state_error())?;
        let context_json = serde_json::to_string(context).map_err(json_error)?;
        let files_json = serde_json::to_string(files).map_err(json_error)?;
        tx.execute(
            "INSERT OR IGNORE INTO workspaces(host,owner,repository,number) VALUES(?1,?2,?3,?4)",
            params![pr.host, pr.owner, pr.repository, number],
        )
        .map_err(db_error)?;
        let old: Option<(String, String)> = tx
            .query_row(
                "SELECT context_json,changed_files_json FROM contexts WHERE context_id=?1",
                [&context.context_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        if old
            .as_ref()
            .is_some_and(|(c, f)| c != &context_json || f != &files_json)
        {
            return Err(state_error());
        }
        tx.execute("INSERT OR IGNORE INTO contexts(context_id,context_json,changed_files_json) VALUES(?1,?2,?3)", params![context.context_id, context_json, files_json]).map_err(db_error)?;
        let generation: i64 = tx.query_row("SELECT generation FROM workspaces WHERE host=?1 AND owner=?2 AND repository=?3 AND number=?4", params![pr.host,pr.owner,pr.repository,number], |r| r.get(0)).map_err(db_error)?;
        let id = new_id();
        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(i64::MAX as u128) as i64;
        tx.execute("INSERT INTO preparations(preparation_id,context_id,host,owner,repository,number,expected_generation,created_at,title,description,checkout) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)", params![id,context.context_id,pr.host,pr.owner,pr.repository,number,generation,created,title,description,checkout]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(PreparedBinding {
            preparation_id: id,
            expected_generation: generation as u64,
            context: context.clone(),
            title: title.into(),
            description: description.map(str::to_owned),
            changed_files: files.to_vec(),
            checkout: checkout.to_owned(),
        })
    }

    pub fn load(&self, id: &str) -> Result<PreparedBinding, ContractError> {
        let row: Option<(String,i64,String,Option<String>,String)> = self.db.query_row("SELECT context_id,expected_generation,title,description,checkout FROM preparations WHERE preparation_id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(db_error)?;
        let Some((context_id, generation, title, description, checkout)) = row else {
            return Err(ContractError::new(
                "PREPARATION_NOT_FOUND",
                "Preparation ID was not found",
                "",
            ));
        };
        let (ctx, files): (String, String) = self
            .db
            .query_row(
                "SELECT context_json,changed_files_json FROM contexts WHERE context_id=?1",
                [context_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(db_error)?;
        Ok(PreparedBinding {
            preparation_id: id.into(),
            expected_generation: generation as u64,
            context: serde_json::from_str(&ctx).map_err(json_error)?,
            title,
            description,
            changed_files: serde_json::from_str(&files).map_err(json_error)?,
            checkout,
        })
    }
}

/// Read-only preparation for an ordinary PR whose base and head commits already exist locally.
/// DNS hosts are routed through `gh --hostname`; live Enterprise compatibility remains unqualified.
/// Missing objects fail explicitly, and this path never substitutes refs.
pub fn prepare_local_pr(
    checkout: &Path,
    number: u64,
    selected_remote: Option<&str>,
    state: &Path,
    control: &crate::OperationControl,
) -> Result<PreparationManifest, ContractError> {
    prepare_local_pr_with_gh(
        checkout,
        number,
        selected_remote,
        state,
        control,
        Path::new("gh"),
    )
}

/// Same acquisition boundary with an explicit executable path for controlled transport tests.
pub fn prepare_local_pr_with_gh(
    checkout: &Path,
    number: u64,
    selected_remote: Option<&str>,
    state: &Path,
    control: &crate::OperationControl,
    gh: &Path,
) -> Result<PreparationManifest, ContractError> {
    use crate::{PrIdentity, diff_index};
    if number == 0 {
        return Err(ContractError::new(
            "INVALID_ARGUMENTS",
            "PR number must be positive",
            "",
        ));
    }
    let top = crate::evidence::git(checkout, control, &["rev-parse", "--show-toplevel"])?;
    let root = Path::new(std::str::from_utf8(&top).map_err(|_| state_error())?.trim());
    let shallow = crate::evidence::git(root, control, &["rev-parse", "--is-shallow-repository"])?;
    if shallow.trim_ascii() == b"true" {
        return Err(ContractError::new(
            "SHALLOW_REPOSITORY_UNSUPPORTED",
            "Shallow checkouts cannot establish a complete merge base",
            "",
        ));
    }
    let names = crate::evidence::git(root, control, &["remote"])?;
    let remotes = std::str::from_utf8(&names)
        .map_err(|_| state_error())?
        .lines()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    let mut matching = Vec::new();
    for name in remotes {
        let key = format!("remote.{name}.url");
        let urls = crate::evidence::git(root, control, &["config", "--get-all", &key])?;
        for url in std::str::from_utf8(&urls)
            .map_err(|_| state_error())?
            .lines()
        {
            if let Some((host, owner, repo)) = parse_remote(url)
                && supported_host(&host)
            {
                matching.push((name.to_owned(), url.to_owned(), host, owner, repo));
            }
        }
    }
    let selected = if let Some(name) = selected_remote {
        let candidates = matching
            .iter()
            .filter(|(n, ..)| n == name)
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return Err(ContractError::new(
                "REMOTE_AMBIGUOUS",
                "Selected remote must have exactly one supported URL",
                "",
            ));
        }
        (*candidates[0]).clone()
    } else {
        if matching.len() != 1 {
            return Err(ContractError::new(
                "REMOTE_AMBIGUOUS",
                "Select exactly one supported GitHub remote with --remote",
                "",
            ));
        }
        matching.remove(0)
    };
    let (remote, url, host, remote_owner, remote_repo) = selected;
    let output = run_gh(
        gh,
        &[
            "api",
            "--hostname",
            &host,
            &format!("repos/{remote_owner}/{remote_repo}/pulls/{number}"),
        ],
        control,
    )?;
    let api: serde_json::Value = serde_json::from_slice(&output).map_err(|_| {
        ContractError::new(
            "GH_RESPONSE_INVALID",
            "GitHub returned an invalid pull request response",
            "",
        )
    })?;
    let str_at = |path: &[&str]| -> Result<String, ContractError> {
        let mut v = &api;
        for p in path {
            v = v.get(*p).ok_or_else(|| {
                ContractError::new(
                    "GH_RESPONSE_INVALID",
                    "Required pull request metadata is missing",
                    "",
                )
            })?;
        }
        v.as_str().map(str::to_owned).ok_or_else(|| {
            ContractError::new(
                "GH_RESPONSE_INVALID",
                "Required pull request metadata is invalid",
                "",
            )
        })
    };
    let base_repo = str_at(&["base", "repo", "full_name"])?;
    let mut repo_parts = base_repo.split('/');
    let owner = repo_parts
        .next()
        .ok_or_else(state_error)?
        .to_ascii_lowercase();
    let repository = repo_parts
        .next()
        .ok_or_else(state_error)?
        .to_ascii_lowercase();
    if repo_parts.next().is_some() || owner != remote_owner || repository != remote_repo {
        return Err(ContractError::new(
            "REMOTE_NOT_MATCHING",
            "Selected checkout remote does not match the pull request base repository",
            "",
        ));
    }
    let base = str_at(&["base", "sha"])?;
    let head = str_at(&["head", "sha"])?;
    let base_ref = api
        .get("base")
        .and_then(|v| v.get("ref"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("__unavailable");
    let head_ref = api
        .get("head")
        .and_then(|v| v.get("ref"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("__unavailable");
    let acquisition_key = format!("{host}/{owner}/{repository}/{number}");
    crate::acquisition::acquire_commit(
        root,
        control,
        &url,
        &format!("refs/heads/{base_ref}"),
        &base,
        &format!("{acquisition_key}/base"),
    )?;
    let head_acquired = crate::acquisition::acquire_commit(
        root,
        control,
        &url,
        &format!("refs/pull/{number}/head"),
        &head,
        &format!("{acquisition_key}/head"),
    );
    if let Err(primary_error) = head_acquired {
        let fork_url = api
            .get("head")
            .and_then(|v| v.get("repo"))
            .and_then(|v| v.get("clone_url"))
            .and_then(serde_json::Value::as_str);
        let head_full_name = api
            .get("head")
            .and_then(|v| v.get("repo"))
            .and_then(|v| v.get("full_name"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_ascii_lowercase);
        let fallback = match (fork_url.and_then(parse_remote), head_full_name) {
            (Some((fork_host, fork_owner, fork_repo)), Some(full_name))
                if fork_host == host && full_name == format!("{fork_owner}/{fork_repo}") =>
            {
                Some(crate::acquisition::acquire_commit(
                    root,
                    control,
                    fork_url.expect("parsed clone URL exists"),
                    &format!("refs/heads/{head_ref}"),
                    &head,
                    &format!("{acquisition_key}/head"),
                ))
            }
            _ => None,
        };
        match fallback {
            Some(Ok(())) => {}
            Some(Err(error))
                if matches!(error.code.as_str(), "CANCELLED" | "OPERATION_TIMEOUT") =>
            {
                return Err(error);
            }
            _ => return Err(primary_error),
        }
    }
    let bases = crate::evidence::git(root, control, &["merge-base", "--all", &base, &head])?;
    let bases = std::str::from_utf8(&bases)
        .map_err(|_| state_error())?
        .lines()
        .collect::<Vec<_>>();
    if bases.len() != 1 {
        return Err(ContractError::new(
            "MERGE_BASE_UNSUPPORTED",
            "PR has no unique merge base in the local checkout",
            "",
        ));
    }
    let title = str_at(&["title"])?;
    let description = Some(
        api.get("body")
            .ok_or_else(|| {
                ContractError::new(
                    "GH_RESPONSE_INVALID",
                    "Original PR description is missing from metadata",
                    "",
                )
            })?
            .as_str()
            .unwrap_or("")
            .to_owned(),
    );
    if title.chars().count() > crate::MAX_STRING_CHARS
        || description
            .as_ref()
            .is_some_and(|s| s.chars().count() > crate::MAX_STRING_CHARS)
    {
        return Err(ContractError::new(
            "PR_METADATA_TOO_LARGE",
            "PR title or description exceeds the supported text bound",
            "",
        ));
    }
    let mut context = ReviewContext {
        pr: PrIdentity {
            host: host.clone(),
            owner,
            repository,
            number,
        },
        context_id: String::new(),
        base_sha: base,
        head_sha: head,
        merge_base_sha: bases[0].into(),
    };
    context.context_id = context_hash(&context);
    let diff = diff_index(root, &context, control)?;
    let binding = PreparationStore::open(state)?.save(
        &context,
        &root.to_string_lossy(),
        &title,
        description.as_deref(),
        &diff.files,
    )?;
    let pull_request_state = api
        .get("state")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    crate::record_initial_remote_snapshot(
        state,
        &binding.preparation_id,
        base_ref,
        head_ref,
        &context.base_sha,
        &context.head_sha,
        pull_request_state,
    )?;
    let _ = crate::enforce_retention(
        state,
        crate::RetentionPolicy::default(),
        std::slice::from_ref(&binding.preparation_id),
    );
    let _ = url;
    let _ = remote;
    Ok(binding.into())
}

pub(crate) fn run_gh(
    program: &Path,
    args: &[&str],
    control: &crate::OperationControl,
) -> Result<Vec<u8>, ContractError> {
    use std::{
        io::Read,
        os::unix::process::CommandExt,
        process::{Command, Stdio},
        thread,
        time::Duration,
    };
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|_| {
            ContractError::new("GH_UNAVAILABLE", "Installed gh could not be started", "")
        })?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out = thread::spawn(move || {
        let mut b = Vec::new();
        let r = stdout
            .take((crate::MAX_INPUT_BYTES + 1) as u64)
            .read_to_end(&mut b);
        (r, b)
    });
    let err = thread::spawn(move || {
        let mut b = Vec::new();
        let r = stderr.take(65_537).read_to_end(&mut b);
        (r, b)
    });
    let mut completed: Option<std::process::ExitStatus> = None;
    loop {
        if let Err(e) = control.check() {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            return Err(e);
        }
        if let Some(status) = completed
            && out.is_finished()
            && err.is_finished()
        {
            let (_, bytes) = out
                .join()
                .map_err(|_| ContractError::new("GH_FAILED", "Cannot read GitHub response", ""))?;
            let (_, diagnostic) = err
                .join()
                .map_err(|_| ContractError::new("GH_FAILED", "Cannot read GitHub response", ""))?;
            if bytes.len() > crate::MAX_INPUT_BYTES || diagnostic.len() > 65_536 {
                return Err(ContractError::new(
                    "GH_RESPONSE_TOO_LARGE",
                    "GitHub response exceeds the supported bound",
                    "",
                ));
            }
            if status.success() {
                return Ok(bytes);
            }
            return Err(map_gh_error(&diagnostic));
        }
        match child.try_wait() {
            Ok(Some(s)) => completed = Some(s),
            Ok(None) => {}
            Err(_) => {
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.wait();
                let _ = out.join();
                let _ = err.join();
                return Err(ContractError::new(
                    "GH_FAILED",
                    "Cannot await GitHub request",
                    "",
                ));
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn parse_remote(value: &str) -> Option<(String, String, String)> {
    let (host, path) = if let Some(rest) = value.strip_prefix("https://") {
        let (host, path) = rest.split_once('/')?;
        if host.contains('@') || host.contains(':') || path.contains(['?', '#', '%']) {
            return None;
        }
        (host.to_ascii_lowercase(), path)
    } else if let Some(rest) = value.strip_prefix("ssh://") {
        let (authority, path) = rest.split_once('/')?;
        let host = if let Some((user, host)) = authority.rsplit_once('@') {
            if !matches!(user, "git" | "ssh") {
                return None;
            }
            host
        } else {
            authority
        };
        if host.contains(['@', ':']) || path.contains(['?', '#', '%']) {
            return None;
        }
        (host.to_ascii_lowercase(), path)
    } else if let Some((user, rest)) = value.split_once('@') {
        if !matches!(user, "git" | "ssh") {
            return None;
        }
        let (host, path) = rest.split_once(':')?;
        if host.contains('/') {
            return None;
        }
        (host.to_ascii_lowercase(), path)
    } else {
        return None;
    };
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next().is_some()
        || [owner, repo].iter().any(|p| {
            p.is_empty()
                || matches!(*p, "." | "..")
                || p.chars()
                    .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
        })
    {
        return None;
    }
    Some((host, owner.to_ascii_lowercase(), repo.to_ascii_lowercase()))
}
fn supported_host(host: &str) -> bool {
    !host.is_empty()
        && host.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && part
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && part
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        })
}
fn context_hash(context: &ReviewContext) -> String {
    let text = format!(
        "bodh-context-v1\0{}\0{}/{}/{}\0{}\0{}\0{}",
        context.pr.host,
        context.pr.owner,
        context.pr.repository,
        context.pr.number,
        context.base_sha,
        context.head_sha,
        context.merge_base_sha
    );
    use sha2::{Digest, Sha256};
    format!("v1-{:x}", Sha256::digest(text.as_bytes()))
}

pub fn inspect_diff(
    p: &PreparedBinding,
    path: &str,
    cursor: usize,
    control: &crate::OperationControl,
) -> Result<serde_json::Value, ContractError> {
    let indexed = p
        .changed_files
        .iter()
        .any(|f| f.old_path.as_deref() == Some(path) || f.new_path.as_deref() == Some(path));
    if !indexed {
        return Err(ContractError::new(
            "PATH_NOT_IN_PREPARATION",
            "Path is not in this prepared comparison",
            "/path",
        ));
    }
    let bytes = crate::evidence::git(
        Path::new(&p.checkout),
        control,
        &[
            "--literal-pathspecs",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--unified=3",
            "--no-color",
            "-O/dev/null",
            &p.context.merge_base_sha,
            &p.context.head_sha,
            "--",
            path,
        ],
    )?;
    let all = std::str::from_utf8(&bytes)
        .map_err(|_| ContractError::new("DIFF_PATH_UNSUPPORTED", "Diff output is not UTF-8", ""))?;
    let mut start = cursor.min(all.len());
    while !all.is_char_boundary(start) {
        start += 1;
    }
    let mut end = (start + 64 * 1024).min(all.len());
    while !all.is_char_boundary(end) {
        end -= 1;
    }
    let next = (end < all.len()).then_some(end);
    Ok(
        serde_json::json!({"path":path,"patch":&all[start..end],"next_cursor":next,"truncated":next.is_some()}),
    )
}

pub fn inspect_files(
    p: &PreparedBinding,
    cursor: usize,
    limit: usize,
) -> Result<serde_json::Value, ContractError> {
    if limit == 0 || limit > 100 {
        return Err(ContractError::new(
            "INVALID_ARGUMENTS",
            "File page limit must be 1..100",
            "",
        ));
    }
    let summaries = p
        .changed_files
        .iter()
        .map(|f| ChangedFileSummary {
            old_path: f.old_path.clone(),
            new_path: f.new_path.clone(),
            status: f.status.clone(),
            unsupported_reason: f.unsupported_reason.clone(),
            hunk_count: f.hunks.len(),
        })
        .collect::<Vec<_>>();
    let start = cursor.min(summaries.len());
    let end = start.saturating_add(limit).min(summaries.len());
    Ok(
        serde_json::json!({"files":&summaries[start..end],"next_cursor":(end<summaries.len()).then_some(end),"truncated":end<summaries.len()}),
    )
}
fn map_gh_error(stderr: &[u8]) -> ContractError {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    let (code, msg) = if text.contains("auth")
        || text.contains("401")
        || text.contains("403")
        || text.contains("permission")
    {
        ("AUTH_ACCESS", "GitHub authentication or access was denied")
    } else if text.contains("404") || text.contains("not found") {
        (
            "AUTH_ACCESS",
            "Pull request or repository is not accessible",
        )
    } else {
        (
            "NETWORK_UNCERTAIN",
            "GitHub request failed; outcome is uncertain",
        )
    };
    ContractError::new(code, msg, "")
}
static NEXT_ID: AtomicU64 = AtomicU64::new(0);
fn new_id() -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "prep-{:x}-{:x}-{:x}",
        std::process::id(),
        time,
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}
fn db_error(_: rusqlite::Error) -> ContractError {
    ContractError::new(
        "STATE_UNAVAILABLE",
        "Cannot read or write local preparation state",
        "",
    )
}
fn json_error(_: serde_json::Error) -> ContractError {
    ContractError::new("STATE_INVALID", "Stored preparation state is invalid", "")
}
fn state_error() -> ContractError {
    ContractError::new(
        "STATE_CONFLICT",
        "Immutable preparation identity conflicts with stored state",
        "",
    )
}
