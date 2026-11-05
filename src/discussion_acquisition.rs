//! Read-only, independently paginated pull request review-thread observations.
use crate::{ContractError, OperationControl, PreparedBinding};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    io::Read,
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 10_000;
const MAX_ROWS: usize = 100_000;
const MAX_OBSERVATION_BYTES: usize = crate::MAX_INPUT_BYTES;

type ObservationRow = (Option<i64>, i64, bool, bool, Option<String>, Option<String>);

struct RunStatus<'a> {
    expected: Option<u64>,
    fetched: usize,
    threads_complete: bool,
    complete: bool,
    failure_connection: Option<&'a str>,
    failure_cause: Option<&'a str>,
}

pub fn acquire_discussions(
    binding: &PreparedBinding,
    state: &Path,
    control: &OperationControl,
) -> Result<Value, ContractError> {
    acquire_discussions_with_gh(binding, state, control, Path::new("gh"))
}

/// Test seam for controlled `gh api graphql` responses. The transport is read-only.
pub fn acquire_discussions_with_gh(
    binding: &PreparedBinding,
    state: &Path,
    control: &OperationControl,
    gh: &Path,
) -> Result<Value, ContractError> {
    let store = DiscussionStore::open(state)?;
    let run_id = store.start(&binding.preparation_id)?;
    let mut outer_after: Option<String> = None;
    let mut seen_cursors = HashSet::new();
    let mut seen_threads = HashSet::new();
    let mut outer_expected = None;
    let mut outer_total_unknown = false;
    let mut outer_fetched = 0usize;
    let mut outer_count_changed = false;
    let mut observation_bytes = 0usize;
    let mut outer_complete = false;
    let mut failure: Option<(String, String)> = None;
    let mut thread_ordinals = Vec::new();

    for page_no in 0..MAX_PAGES {
        if let Err(error) = control.check() {
            failure = Some(("threads".into(), error.code));
            break;
        }
        let result = gh_graphql(
            gh,
            &outer_query(),
            GraphQlScope {
                host: &binding.context.pr.host,
                owner: &binding.context.pr.owner,
                repository: &binding.context.pr.repository,
                number: binding.context.pr.number,
                thread_id: None,
                after: outer_after.as_deref(),
            },
            control,
        );
        let response = match result {
            Ok(v) => v,
            Err(e) => {
                failure = Some(("threads".into(), e.code));
                break;
            }
        };
        observation_bytes = observation_bytes
            .saturating_add(serde_json::to_vec(&response).map_err(json_error)?.len());
        if observation_bytes > MAX_OBSERVATION_BYTES {
            failure = Some(("threads".into(), "DISCUSSION_LIMIT_EXCEEDED".into()));
            break;
        }
        let graphql_error = graphql_error_cause(&response);
        let Some(connection) = response
            .get("data")
            .and_then(|v| v.get("repository"))
            .and_then(|v| v.get("pullRequest"))
            .and_then(|v| v.get("reviewThreads"))
        else {
            failure = Some((
                "threads".into(),
                graphql_error.unwrap_or_else(|| "GH_RESPONSE_INVALID".into()),
            ));
            break;
        };
        let page = parse_page(connection);
        let Ok(page) = page else {
            failure = Some(("threads".into(), "GH_RESPONSE_INVALID".into()));
            break;
        };
        if page.total_count.is_none() {
            outer_total_unknown = true;
            outer_expected = None;
        } else if !outer_total_unknown {
            if outer_expected.is_some() && outer_expected != page.total_count {
                outer_count_changed = true;
            }
            outer_expected = page.total_count;
        }
        outer_fetched = outer_fetched.saturating_add(page.nodes.len());
        store.save_page(
            &run_id,
            "threads",
            "",
            page_no,
            outer_after.as_deref(),
            &page,
        )?;
        if outer_fetched > MAX_ROWS {
            failure = Some(("threads".into(), "DISCUSSION_LIMIT_EXCEEDED".into()));
        }
        for node in &page.nodes {
            let Some(node_id) = node.get("id").and_then(Value::as_str) else {
                failure.get_or_insert_with(|| ("threads".into(), "NULL_THREAD_NODE".into()));
                continue;
            };
            if node_id.is_empty() || !seen_threads.insert(node_id.to_owned()) {
                failure.get_or_insert_with(|| ("threads".into(), "DUPLICATE_THREAD_NODE".into()));
                continue;
            }
            if seen_threads.len() > MAX_ROWS {
                failure = Some(("threads".into(), "DISCUSSION_LIMIT_EXCEEDED".into()));
                break;
            }
            let ordinal = store.add_thread(&run_id, node_id, node)?;
            thread_ordinals.push((ordinal, node_id.to_owned()));
        }
        if let Some(cause) = graphql_error {
            failure.get_or_insert_with(|| ("threads".into(), cause));
        }
        if failure.is_some() {
            break;
        }
        if !page.has_next_page {
            outer_complete = true;
            break;
        }
        let Some(next) = page.end_cursor.filter(|s| !s.is_empty()) else {
            failure = Some(("threads".into(), "INVALID_CURSOR".into()));
            break;
        };
        if !seen_cursors.insert(next.clone()) {
            failure = Some(("threads".into(), "REPEATED_CURSOR".into()));
            break;
        }
        outer_after = Some(next);
        if page_no + 1 == MAX_PAGES {
            failure = Some(("threads".into(), "PAGE_LIMIT_EXCEEDED".into()));
        }
    }
    if outer_complete && outer_expected.is_some_and(|n| n != outer_fetched as u64) {
        failure.get_or_insert_with(|| ("threads".into(), "COUNT_CHANGED".into()));
        outer_complete = false;
    }
    if outer_count_changed {
        failure.get_or_insert_with(|| ("threads".into(), "COUNT_CHANGED".into()));
        outer_complete = false;
    }

    // Paginate each thread's comments connection independently, even when a later outer page failed.
    let mut completed_threads = HashSet::new();
    for (ordinal, node_id) in thread_ordinals {
        let mut after: Option<String> = None;
        let mut cursors = HashSet::new();
        let mut expected = None;
        let mut expected_unknown = false;
        let mut fetched = 0usize;
        let mut comments_complete = false;
        let mut comments_failure: Option<String> = None;
        let mut comment_ids = HashSet::new();
        for page_no in 0..MAX_PAGES {
            if let Err(error) = control.check() {
                comments_failure = Some(error.code);
                break;
            }
            let response = match gh_graphql(
                gh,
                &comments_query(),
                GraphQlScope {
                    host: &binding.context.pr.host,
                    owner: &binding.context.pr.owner,
                    repository: &binding.context.pr.repository,
                    number: binding.context.pr.number,
                    thread_id: Some(&node_id),
                    after: after.as_deref(),
                },
                control,
            ) {
                Ok(v) => v,
                Err(error) => {
                    comments_failure = Some(error.code);
                    break;
                }
            };
            observation_bytes = observation_bytes
                .saturating_add(serde_json::to_vec(&response).map_err(json_error)?.len());
            if observation_bytes > MAX_OBSERVATION_BYTES {
                comments_failure = Some("DISCUSSION_LIMIT_EXCEEDED".into());
                break;
            }
            let graphql_error = graphql_error_cause(&response);
            let Some(connection) = response
                .get("data")
                .and_then(|v| v.get("node"))
                .and_then(|v| v.get("comments"))
            else {
                comments_failure =
                    Some(graphql_error.unwrap_or_else(|| "GH_RESPONSE_INVALID".into()));
                break;
            };
            let page = match parse_page(connection) {
                Ok(page) => page,
                Err(_) => {
                    comments_failure = Some("GH_RESPONSE_INVALID".into());
                    break;
                }
            };
            if page.total_count.is_none() {
                expected_unknown = true;
                expected = None;
            } else if !expected_unknown {
                if expected.is_some() && expected != page.total_count {
                    comments_failure.get_or_insert_with(|| "COUNT_CHANGED".into());
                }
                expected = page.total_count;
            }
            fetched = fetched.saturating_add(page.nodes.len());
            store.save_page(
                &run_id,
                "comments",
                &ordinal.to_string(),
                page_no,
                after.as_deref(),
                &page,
            )?;
            if fetched > MAX_ROWS {
                comments_failure = Some("DISCUSSION_LIMIT_EXCEEDED".into());
            }
            for node in &page.nodes {
                let Some(id) = node.get("id").and_then(Value::as_str) else {
                    comments_failure.get_or_insert_with(|| "NULL_COMMENT_NODE".into());
                    continue;
                };
                if id.is_empty() || !comment_ids.insert(id.to_owned()) {
                    comments_failure.get_or_insert_with(|| "DUPLICATE_COMMENT_NODE".into());
                }
            }
            if let Some(cause) = graphql_error {
                comments_failure.get_or_insert(cause);
            }
            if comments_failure.is_some() {
                break;
            }
            if !page.has_next_page {
                comments_complete = true;
                break;
            }
            let Some(next) = page.end_cursor.filter(|s| !s.is_empty()) else {
                comments_failure = Some("INVALID_CURSOR".into());
                break;
            };
            if !cursors.insert(next.clone()) {
                comments_failure = Some("REPEATED_CURSOR".into());
                break;
            }
            after = Some(next);
            if page_no + 1 == MAX_PAGES {
                comments_failure = Some("PAGE_LIMIT_EXCEEDED".into());
            }
        }
        if comments_complete && expected.is_some_and(|n| n != fetched as u64) {
            comments_complete = false;
            comments_failure.get_or_insert_with(|| "COUNT_CHANGED".into());
        }
        store.finish_thread(
            &run_id,
            ordinal,
            expected,
            fetched,
            comments_complete,
            comments_failure.as_deref(),
        )?;
        if comments_complete {
            completed_threads.insert(ordinal);
        } else {
            failure.get_or_insert_with(|| {
                (
                    format!("comments:{node_id}"),
                    comments_failure
                        .clone()
                        .unwrap_or_else(|| "INCOMPLETE".into()),
                )
            });
        }
    }
    let every_thread_complete = completed_threads.len() == seen_threads.len();
    let complete = outer_complete && every_thread_complete && failure.is_none();
    store.finish_run(
        &run_id,
        &RunStatus {
            expected: outer_expected,
            fetched: outer_fetched,
            threads_complete: outer_complete,
            complete,
            failure_connection: failure.as_ref().map(|(c, _)| c.as_str()),
            failure_cause: failure.as_ref().map(|(_, cause)| cause.as_str()),
        },
    )?;
    store.load(&run_id)
}

pub fn load_latest_discussions(state: &Path, preparation_id: &str) -> Result<Value, ContractError> {
    DiscussionStore::open(state)?.load_latest(preparation_id)
}

pub fn latest_discussion_summary(
    state: &Path,
    preparation_id: &str,
) -> Result<Value, ContractError> {
    let result = load_latest_discussions(state, preparation_id)?;
    let mut expected_comments = Some(0u64);
    let mut fetched_comments = 0u64;
    for thread in result["threads"].as_array().into_iter().flatten() {
        if let Some(count) = thread["expected_comment_count"].as_u64() {
            if let Some(total) = expected_comments.as_mut() {
                *total = total.saturating_add(count);
            }
        } else {
            expected_comments = None;
        }
        fetched_comments =
            fetched_comments.saturating_add(thread["fetched_comment_count"].as_u64().unwrap_or(0));
    }
    if result["threads_complete"] == false {
        expected_comments = None;
    }
    Ok(json!({
        "available": result["available"],
        "expected_thread_count": result["expected_thread_count"],
        "fetched_thread_count": result["fetched_thread_count"],
        "expected_comment_count": expected_comments,
        "fetched_comment_count": fetched_comments,
        "complete": result["complete"],
        "incomplete_connection": result["incomplete_connection"],
        "incomplete_cause": result["incomplete_cause"],
        "count_changed": result["count_changed"],
        "observation_consistency": "moving_non_atomic",
        "original_side_availability": "not_exposed_by_graphql"
    }))
}

struct ParsedPage {
    total_count: Option<u64>,
    end_cursor: Option<String>,
    has_next_page: bool,
    nodes: Vec<Value>,
}

fn parse_page(value: &Value) -> Result<ParsedPage, ()> {
    let page_info = value.get("pageInfo").ok_or(())?;
    let has_next_page = page_info
        .get("hasNextPage")
        .and_then(Value::as_bool)
        .ok_or(())?;
    let end_cursor = match page_info.get("endCursor") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Null) | None => None,
        _ => return Err(()),
    };
    let total_count = match value.get("totalCount") {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::Null) | None => None,
        _ => return Err(()),
    };
    let nodes = value
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or(())?
        .clone();
    Ok(ParsedPage {
        total_count,
        end_cursor,
        has_next_page,
        nodes,
    })
}

fn graphql_error_cause(response: &Value) -> Option<String> {
    let errors = response.get("errors")?.as_array()?;
    if errors.is_empty() {
        return None;
    }
    let text = errors
        .iter()
        .filter_map(|e| e.get("message").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if text.contains("auth")
        || text.contains("403")
        || text.contains("permission")
        || text.contains("forbidden")
        || text.contains("not accessible")
    {
        Some("AUTH_ACCESS".into())
    } else {
        Some("GRAPHQL_PARTIAL_ERROR".into())
    }
}

fn outer_query() -> String {
    format!(
        "query($owner:String!,$name:String!,$number:Int!,$after:String){{repository(owner:$owner,name:$name){{pullRequest(number:$number){{reviewThreads(first:{PAGE_SIZE},after:$after){{totalCount pageInfo{{hasNextPage endCursor}} nodes{{id path line originalLine diffSide startLine originalStartLine startDiffSide isResolved isOutdated viewerCanReply viewerCanResolve viewerCanUnresolve resolvedBy{{login}} comments(first:1){{totalCount}}}}}}}}}}}}"
    )
}

fn comments_query() -> String {
    format!(
        "query($id:ID!,$after:String){{node(id:$id){{... on PullRequestReviewThread{{comments(first:{PAGE_SIZE},after:$after){{totalCount pageInfo{{hasNextPage endCursor}} nodes{{id databaseId:fullDatabaseId body createdAt updatedAt author{{login}} authorAssociation path line originalLine startLine originalStartLine replyTo{{id databaseId:fullDatabaseId}} viewerCanUpdate viewerCanDelete viewerCanReact viewerDidAuthor}}}}}}}}}}"
    )
}

pub(crate) struct GraphQlScope<'a> {
    pub(crate) host: &'a str,
    pub(crate) owner: &'a str,
    pub(crate) repository: &'a str,
    pub(crate) number: u64,
    pub(crate) thread_id: Option<&'a str>,
    pub(crate) after: Option<&'a str>,
}

pub(crate) fn gh_graphql(
    gh: &Path,
    query: &str,
    scope: GraphQlScope<'_>,
    control: &OperationControl,
) -> Result<Value, ContractError> {
    let mut args = vec![
        "api".to_owned(),
        "graphql".to_owned(),
        "--hostname".to_owned(),
        scope.host.to_owned(),
        "-f".to_owned(),
        format!("query={query}"),
    ];
    if scope.thread_id.is_none() {
        args.extend([
            "-f".into(),
            format!("owner={}", scope.owner),
            "-f".into(),
            format!("name={}", scope.repository),
            "-F".into(),
            format!("number={}", scope.number),
        ]);
    } else if let Some(thread_id) = scope.thread_id {
        args.extend(["-f".into(), format!("id={thread_id}")]);
    }
    if let Some(after) = scope.after {
        args.extend(["-f".into(), format!("after={after}")]);
    }
    let bytes = run_process(gh, &args, control)?;
    serde_json::from_slice(&bytes).map_err(|_| {
        ContractError::new(
            "GH_RESPONSE_INVALID",
            "GitHub returned invalid GraphQL JSON",
            "",
        )
    })
}

pub(crate) fn run_process(
    program: &Path,
    args: &[String],
    control: &OperationControl,
) -> Result<Vec<u8>, ContractError> {
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
    let mut status: Option<std::process::ExitStatus> = None;
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
        if let Some(s) = status
            && out.is_finished()
            && err.is_finished()
        {
            let (_, bytes) = out.join().map_err(|_| gh_failure())?;
            let (_, diagnostic) = err.join().map_err(|_| gh_failure())?;
            if bytes.len() > crate::MAX_INPUT_BYTES || diagnostic.len() > 65_536 {
                return Err(ContractError::new(
                    "GH_RESPONSE_TOO_LARGE",
                    "GitHub response exceeds the supported bound",
                    "",
                ));
            }
            if s.success() {
                return Ok(bytes);
            }
            return Err(map_transport_error(&diagnostic));
        }
        match child.try_wait() {
            Ok(Some(s)) => status = Some(s),
            Ok(None) => {}
            Err(_) => {
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.wait();
                let _ = out.join();
                let _ = err.join();
                return Err(gh_failure());
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn map_transport_error(stderr: &[u8]) -> ContractError {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    let code = if text.contains("auth")
        || text.contains("401")
        || text.contains("403")
        || text.contains("permission")
        || text.contains("404")
        || text.contains("not found")
    {
        "AUTH_ACCESS"
    } else {
        "NETWORK_UNCERTAIN"
    };
    ContractError::new(
        code,
        "GitHub review discussion page could not be fetched",
        "",
    )
}
fn gh_failure() -> ContractError {
    ContractError::new("GH_FAILED", "Cannot read GitHub response", "")
}
fn has_count_change(pages: &[Value]) -> bool {
    let mut observed = std::collections::HashMap::<(String, String), i64>::new();
    for page in pages {
        let key = (
            page["connection"].as_str().unwrap_or_default().to_owned(),
            page["thread_key"].as_str().unwrap_or_default().to_owned(),
        );
        let count = page["total_count"].as_i64();
        if let Some(count) = count {
            if observed
                .get(&key)
                .is_some_and(|previous| *previous != count)
            {
                return true;
            }
            observed.insert(key, count);
        }
    }
    false
}

struct DiscussionStore {
    db: Connection,
}
impl DiscussionStore {
    fn open(path: &Path) -> Result<Self, ContractError> {
        // Establish the prepared-state schema first; this component owns only its additive tables.
        crate::PreparationStore::open(path)?;
        let db = Connection::open(path).map_err(db_error)?;
        crate::sqlite::configure(&db).map_err(db_error)?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS discussion_schema_migrations(version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);")
            .map_err(db_error)?;
        db.execute_batch("BEGIN IMMEDIATE;").map_err(db_error)?;
        let schema_version: i64 = db
            .query_row(
                "SELECT COALESCE(MAX(version),0) FROM discussion_schema_migrations",
                [],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        if schema_version > 1 {
            let _ = db.execute_batch("ROLLBACK;");
            return Err(ContractError::new(
                "STATE_VERSION_UNSUPPORTED",
                "Discussion state schema is newer than this application",
                "",
            ));
        }
        if schema_version == 0 {
            db.execute_batch("CREATE TABLE discussion_observations(run_id TEXT PRIMARY KEY, preparation_id TEXT NOT NULL REFERENCES preparations(preparation_id) ON DELETE CASCADE, expected_threads INTEGER, fetched_threads INTEGER NOT NULL DEFAULT 0, threads_complete INTEGER NOT NULL DEFAULT 0, complete INTEGER NOT NULL DEFAULT 0, incomplete_connection TEXT, incomplete_cause TEXT, created_at INTEGER NOT NULL);
                CREATE TABLE discussion_threads(run_id TEXT NOT NULL REFERENCES discussion_observations(run_id) ON DELETE CASCADE, ordinal INTEGER NOT NULL, node_id TEXT NOT NULL, thread_json TEXT NOT NULL, expected_comments INTEGER, fetched_comments INTEGER NOT NULL DEFAULT 0, comments_complete INTEGER NOT NULL DEFAULT 0, incomplete_cause TEXT, PRIMARY KEY(run_id,ordinal), UNIQUE(run_id,node_id));
                CREATE TABLE discussion_pages(run_id TEXT NOT NULL REFERENCES discussion_observations(run_id) ON DELETE CASCADE, connection_name TEXT NOT NULL, thread_key TEXT NOT NULL, page_no INTEGER NOT NULL, cursor_in TEXT, end_cursor TEXT, has_next_page INTEGER NOT NULL, expected_count INTEGER, nodes_json TEXT NOT NULL, PRIMARY KEY(run_id,connection_name,thread_key,page_no));
                INSERT INTO discussion_schema_migrations(version,applied_at) VALUES(1,unixepoch('subsec')*1000);
                COMMIT;").map_err(db_error)?;
        } else {
            db.execute_batch("COMMIT;").map_err(db_error)?;
        }
        Ok(Self { db })
    }
    fn start(&self, preparation_id: &str) -> Result<String, ContractError> {
        let exists: bool = self
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM preparations WHERE preparation_id=?1)",
                [preparation_id],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        if !exists {
            return Err(ContractError::new(
                "PREPARATION_NOT_FOUND",
                "Preparation ID was not found",
                "",
            ));
        }
        let id = new_run_id();
        let tx = self.db.unchecked_transaction().map_err(db_error)?;
        // A killed process can leave a run unfinished before finish_run gets
        // its pruning turn. Discard stale partial cache rows at the next start,
        // keeping only the latest complete fallback before adding this attempt.
        tx.execute(
            "DELETE FROM discussion_observations
             WHERE preparation_id=?1 AND run_id<>COALESCE((
                 SELECT run_id FROM discussion_observations
                 WHERE preparation_id=?1 AND complete=1
                 ORDER BY created_at DESC,run_id DESC LIMIT 1
             ),'')",
            [preparation_id],
        )
        .map_err(db_error)?;
        tx.execute("INSERT INTO discussion_observations(run_id,preparation_id,created_at) VALUES(?1,?2,?3)",params![id,preparation_id,now_ms()]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(id)
    }
    fn save_page(
        &self,
        run_id: &str,
        connection: &str,
        thread_key: &str,
        page_no: usize,
        cursor_in: Option<&str>,
        page: &ParsedPage,
    ) -> Result<(), ContractError> {
        let tx = self.db.unchecked_transaction().map_err(db_error)?;
        let nodes = serde_json::to_string(&page.nodes).map_err(json_error)?;
        tx.execute("INSERT INTO discussion_pages(run_id,connection_name,thread_key,page_no,cursor_in,end_cursor,has_next_page,expected_count,nodes_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![run_id,connection,thread_key,page_no as i64,cursor_in,page.end_cursor,page.has_next_page as i64,page.total_count.map(|n|n as i64),nodes]).map_err(db_error)?;
        tx.commit().map_err(db_error)
    }
    fn add_thread(
        &self,
        run_id: &str,
        node_id: &str,
        node: &Value,
    ) -> Result<usize, ContractError> {
        let ordinal: i64 = self
            .db
            .query_row(
                "SELECT COALESCE(MAX(ordinal)+1,0) FROM discussion_threads WHERE run_id=?1",
                [run_id],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        self.db.execute("INSERT INTO discussion_threads(run_id,ordinal,node_id,thread_json) VALUES(?1,?2,?3,?4)",params![run_id,ordinal,node_id,serde_json::to_string(node).map_err(json_error)?]).map_err(db_error)?;
        Ok(ordinal as usize)
    }
    fn finish_thread(
        &self,
        run_id: &str,
        ordinal: usize,
        expected: Option<u64>,
        fetched: usize,
        complete: bool,
        cause: Option<&str>,
    ) -> Result<(), ContractError> {
        self.db.execute("UPDATE discussion_threads SET expected_comments=?3,fetched_comments=?4,comments_complete=?5,incomplete_cause=?6 WHERE run_id=?1 AND ordinal=?2",params![run_id,ordinal as i64,expected.map(|n|n as i64),fetched as i64,complete as i64,cause]).map_err(db_error)?;
        Ok(())
    }
    fn finish_run(&self, run_id: &str, status: &RunStatus<'_>) -> Result<(), ContractError> {
        // Keep the latest partial attempt and the most recent complete cache.
        // The new result is fully acquired before this short local transaction;
        // pruning never holds a database write lock across network work.
        let tx = self.db.unchecked_transaction().map_err(db_error)?;
        let preparation_id: String = tx
            .query_row(
                "SELECT preparation_id FROM discussion_observations WHERE run_id=?1",
                [run_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        tx.execute("UPDATE discussion_observations SET expected_threads=?2,fetched_threads=?3,threads_complete=?4,complete=?5,incomplete_connection=?6,incomplete_cause=?7 WHERE run_id=?1",params![run_id,status.expected.map(|n|n as i64),status.fetched as i64,status.threads_complete as i64,status.complete as i64,status.failure_connection,status.failure_cause]).map_err(db_error)?;
        tx.execute(
            "DELETE FROM discussion_observations
             WHERE preparation_id=?1 AND run_id<>?2
               AND (complete=0 OR run_id<>(
                   SELECT run_id FROM discussion_observations
                   WHERE preparation_id=?1 AND complete=1
                   ORDER BY created_at DESC,run_id DESC LIMIT 1
               ))",
            params![preparation_id, run_id],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)
    }
    fn load_latest(&self, preparation_id: &str) -> Result<Value, ContractError> {
        let run:Option<String>=self.db.query_row("SELECT run_id FROM discussion_observations WHERE preparation_id=?1 ORDER BY created_at DESC,run_id DESC LIMIT 1",[preparation_id],|r|r.get(0)).optional().map_err(db_error)?;
        match run {
            Some(id) => self.load(&id),
            None => Ok(
                json!({"available":false,"complete":false,"expected_thread_count":null,"fetched_thread_count":0,"threads_complete":false,"threads":[],"incomplete_connection":"threads","incomplete_cause":"NOT_FETCHED","count_changed":false,"page_count":0,"observation_consistency":"moving_non_atomic","original_side_availability":"not_exposed_by_graphql"}),
            ),
        }
    }
    fn load(&self, run_id: &str) -> Result<Value, ContractError> {
        let row: Option<ObservationRow> = self.db.query_row("SELECT expected_threads,fetched_threads,threads_complete,complete,incomplete_connection,incomplete_cause FROM discussion_observations WHERE run_id=?1",[run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get::<_,i64>(2)?!=0,r.get::<_,i64>(3)?!=0,r.get(4)?,r.get(5)?))).optional().map_err(db_error)?;
        let Some((
            expected,
            fetched,
            threads_complete,
            complete,
            incomplete_connection,
            incomplete_cause,
        )) = row
        else {
            return Err(ContractError::new(
                "STATE_UNAVAILABLE",
                "Discussion observation was not found",
                "",
            ));
        };
        let mut stmt=self.db.prepare("SELECT ordinal,node_id,thread_json,expected_comments,fetched_comments,comments_complete,incomplete_cause FROM discussion_threads WHERE run_id=?1 ORDER BY ordinal").map_err(db_error)?;
        let rows = stmt
            .query_map([run_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)? != 0,
                    r.get::<_, Option<String>>(6)?,
                ))
            })
            .map_err(db_error)?;
        let mut threads = Vec::new();
        for row in rows {
            let (
                ordinal,
                _node_id,
                thread_json,
                expected_comments,
                fetched_comments,
                comments_complete,
                cause,
            ) = row.map_err(db_error)?;
            let mut thread: Value = serde_json::from_str(&thread_json).map_err(json_error)?;
            let mut comments = Vec::new();
            let mut pages=self.db.prepare("SELECT nodes_json FROM discussion_pages WHERE run_id=?1 AND connection_name='comments' AND thread_key=?2 ORDER BY page_no").map_err(db_error)?;
            let page_rows = pages
                .query_map(params![run_id, ordinal.to_string()], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(db_error)?;
            for page in page_rows {
                let page = page.map_err(db_error)?;
                let nodes: Vec<Value> = serde_json::from_str(&page).map_err(json_error)?;
                comments.extend(nodes);
            }
            if let Some(object) = thread.as_object_mut() {
                object.insert("comments".into(), Value::Array(comments));
                object.insert(
                    "expected_comment_count".into(),
                    expected_comments.map(|n| json!(n)).unwrap_or(Value::Null),
                );
                object.insert("fetched_comment_count".into(), json!(fetched_comments));
                object.insert("comments_complete".into(), json!(comments_complete));
                object.insert(
                    "comments_incomplete_cause".into(),
                    cause.map(Value::String).unwrap_or(Value::Null),
                );
                object.insert("originalDiffSide".into(), Value::Null);
                object.insert("originalStartDiffSide".into(), Value::Null);
                object.insert(
                    "original_side_availability".into(),
                    Value::String("not_exposed_by_graphql".into()),
                );
            }
            threads.push(thread);
        }
        let pages = self.pages_value(run_id)?;
        let count_changed = has_count_change(&pages);
        let mut expected_comments = Some(0u64);
        let mut fetched_comments = 0u64;
        for thread in &threads {
            if let Some(count) = thread["expected_comment_count"].as_u64() {
                if let Some(total) = expected_comments.as_mut() {
                    *total = total.saturating_add(count);
                }
            } else {
                expected_comments = None;
            }
            fetched_comments = fetched_comments
                .saturating_add(thread["fetched_comment_count"].as_u64().unwrap_or(0));
        }
        if !threads_complete {
            expected_comments = None;
        }
        Ok(
            json!({"available":true,"complete":complete && !count_changed,"expected_thread_count":expected,"fetched_thread_count":fetched,"expected_comment_count":expected_comments,"fetched_comment_count":fetched_comments,"threads_complete":threads_complete,"threads":threads,"incomplete_connection":incomplete_connection,"incomplete_cause":incomplete_cause,"count_changed":count_changed,"page_count":pages.len(),"observation_consistency":"moving_non_atomic","original_side_availability":"not_exposed_by_graphql"}),
        )
    }
    fn pages_value(&self, run_id: &str) -> Result<Vec<Value>, ContractError> {
        let mut stmt=self.db.prepare("SELECT connection_name,thread_key,page_no,cursor_in,end_cursor,has_next_page,expected_count,nodes_json FROM discussion_pages WHERE run_id=?1 ORDER BY connection_name,thread_key,page_no").map_err(db_error)?;
        let rows = stmt
            .query_map([run_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)? != 0,
                    r.get::<_, Option<i64>>(6)?,
                    r.get::<_, String>(7)?,
                ))
            })
            .map_err(db_error)?;
        let mut out = Vec::new();
        for row in rows {
            let (
                connection,
                thread_key,
                page_no,
                cursor_in,
                end_cursor,
                has_next_page,
                total_count,
                nodes,
            ) = row.map_err(db_error)?;
            out.push(json!({"connection":connection,"thread_key":thread_key,"page":page_no,"cursor_in":cursor_in,"end_cursor":end_cursor,"has_next_page":has_next_page,"total_count":total_count,"nodes":serde_json::from_str::<Value>(&nodes).map_err(json_error)?}));
        }
        Ok(out)
    }
}

static NEXT_RUN: AtomicU64 = AtomicU64::new(0);
fn new_run_id() -> String {
    format!(
        "discussion-{:x}-{:x}-{:x}",
        std::process::id(),
        now_ms(),
        NEXT_RUN.fetch_add(1, Ordering::Relaxed)
    )
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn db_error(_: rusqlite::Error) -> ContractError {
    ContractError::new(
        "STATE_UNAVAILABLE",
        "Cannot read or write local discussion state",
        "",
    )
}
fn json_error(_: serde_json::Error) -> ContractError {
    ContractError::new(
        "STATE_INVALID",
        "Stored discussion observation is invalid",
        "",
    )
}
