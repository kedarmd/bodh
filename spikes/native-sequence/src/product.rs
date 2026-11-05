//! Native product workspace. GitHub acquisition and pinned-source rules stay in `bodh`.
use bodh::{
    DraftSnapshot, DraftStore, OperationControl, PinnedDiff, PreparationManifest, PreparationStore,
    PreparedBinding, Revision, SourceRef, read_diff,
};
use bodh_sequence_spike::gh_prerequisite::{GhCliStatus, probe_gh_cli, resolve_repository_path};
use bodh_sequence_spike::preferences::{
    NavigationPreferences as Preferences, RecentReview, WorkspaceTab,
};
use bodh_sequence_spike::{
    DiffLine, DiffLineKind, MarkdownBlock, MarkdownBlockKind, TextDraftEditor, diff_rows,
    safe_markdown_blocks, split_diff_rows, syntax_spans,
};
use gpui::{UniformListScrollHandle, prelude::*, uniform_list, *};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

const CANVAS_DARK: u32 = 0x16191e;
const PANEL_DARK: u32 = 0x1d2128;
const RAISED_DARK: u32 = 0x252a33;
const BORDER_DARK: u32 = 0x343b46;
const TEXT_DARK: u32 = 0xe8ebef;
const MUTED_DARK: u32 = 0xa7afbc;
const ACCENT_DARK: u32 = 0x8ccecb;
const CANVAS_LIGHT: u32 = 0xf3f4f6;
const PANEL_LIGHT: u32 = 0xffffff;
const RAISED_LIGHT: u32 = 0xf0f2f5;
const BORDER_LIGHT: u32 = 0xd9dee5;
const TEXT_LIGHT: u32 = 0x20262f;
const MUTED_LIGHT: u32 = 0x596579;
const ACCENT_LIGHT: u32 = 0x1e6867;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Home,
    Repository,
    Workspace,
}

#[derive(Clone, Debug, Deserialize)]
struct PullRequest {
    number: u64,
    title: String,
    author: Author,
    state: String,
}
#[derive(Clone, Debug, Deserialize)]
struct Author {
    login: String,
}

#[derive(Clone, Copy)]
struct Palette {
    canvas: u32,
    panel: u32,
    raised: u32,
    border: u32,
    text: u32,
    muted: u32,
    accent: u32,
}
impl Palette {
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                canvas: CANVAS_DARK,
                panel: PANEL_DARK,
                raised: RAISED_DARK,
                border: BORDER_DARK,
                text: TEXT_DARK,
                muted: MUTED_DARK,
                accent: ACCENT_DARK,
            }
        } else {
            Self {
                canvas: CANVAS_LIGHT,
                panel: PANEL_LIGHT,
                raised: RAISED_LIGHT,
                border: BORDER_LIGHT,
                text: TEXT_LIGHT,
                muted: MUTED_LIGHT,
                accent: ACCENT_LIGHT,
            }
        }
    }
}

pub struct Product {
    screen: Screen,
    prefs: Preferences,
    prefs_path: PathBuf,
    preparation_db: PathBuf,
    repository_input: String,
    active_repository: Option<String>,
    pull_requests: Vec<PullRequest>,
    selected_pr: Option<u64>,
    search: String,
    lifecycle: String,
    loading: bool,
    busy_label: String,
    error: Option<String>,
    status: Option<String>,
    gh_cli_status: GhCliStatus,
    gh_check_epoch: Arc<AtomicU64>,
    pending_initial: Option<(String, Option<u64>)>,
    selected_review: Option<PreparedBinding>,
    active_tab: WorkspaceTab,
    selected_file: Option<usize>,
    source_text: Option<String>,
    source_loading: bool,
    source_label: Option<String>,
    operation_epoch: Arc<AtomicU64>,
    active_control: Option<OperationControl>,
    focus: FocusHandle,
    search_focus: FocusHandle,
    file_search_focus: FocusHandle,
    file_scroll: UniformListScrollHandle,
    diff_scroll: UniformListScrollHandle,
    source_scroll: ScrollHandle,
    description_scroll: ScrollHandle,
    file_search: String,
    source_revision: Revision,
    split_diff: bool,
    diff_page: Option<PinnedDiff>,
    diff_rows: Vec<DiffLine>,
    diff_loading: bool,
    diff_error: Option<String>,
    source_ref: Option<SourceRef>,
    description_blocks: Vec<MarkdownBlock>,
    draft_snapshot: Option<DraftSnapshot>,
    draft_editor: Option<TextDraftEditor>,
    draft_saving: bool,
    draft_epoch: Arc<AtomicU64>,
    draft_focus: FocusHandle,
    dragging_pane: bool,
}

impl Product {
    pub fn new(
        prefs_path: PathBuf,
        preparation_db: PathBuf,
        initial_repo: Option<String>,
        initial_pr: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Self {
        let prefs = Preferences::load(&prefs_path);
        let repository_input = initial_repo.clone().unwrap_or_default();
        let pending_initial = initial_repo.clone().map(|path| (path, initial_pr));
        let mut app = Self {
            screen: Screen::Home,
            active_repository: None,
            prefs,
            prefs_path,
            preparation_db,
            repository_input,
            pull_requests: Vec::new(),
            selected_pr: None,
            search: String::new(),
            lifecycle: "All".into(),
            loading: false,
            busy_label: String::new(),
            error: None,
            status: None,
            gh_cli_status: GhCliStatus::Checking,
            gh_check_epoch: Arc::new(AtomicU64::new(0)),
            pending_initial,
            selected_review: None,
            active_tab: WorkspaceTab::Overview,
            selected_file: None,
            source_text: None,
            source_loading: false,
            source_label: None,
            operation_epoch: Arc::new(AtomicU64::new(0)),
            active_control: None,
            focus: cx.focus_handle(),
            search_focus: cx.focus_handle(),
            file_search_focus: cx.focus_handle(),
            file_scroll: UniformListScrollHandle::new(),
            diff_scroll: UniformListScrollHandle::new(),
            source_scroll: ScrollHandle::new(),
            description_scroll: ScrollHandle::new(),
            file_search: String::new(),
            source_revision: Revision::Head,
            split_diff: false,
            diff_page: None,
            diff_rows: Vec::new(),
            diff_loading: false,
            diff_error: None,
            source_ref: None,
            description_blocks: Vec::new(),
            draft_snapshot: None,
            draft_editor: None,
            draft_saving: false,
            draft_epoch: Arc::new(AtomicU64::new(0)),
            draft_focus: cx.focus_handle(),
            dragging_pane: false,
        };
        app.refresh_gh_cli(cx);
        app
    }

    fn persist(&self) {
        let _ = self.prefs.save(&self.prefs_path);
    }

    fn cancel_pending(&mut self) {
        self.operation_epoch.fetch_add(1, Ordering::Relaxed);
        if let Some(control) = self.active_control.take() {
            control.cancel();
        }
        self.loading = false;
    }

    fn open_repository(&mut self, path: String, cx: &mut Context<Self>) {
        if self.draft_saving
            || self
                .draft_editor
                .as_ref()
                .is_some_and(TextDraftEditor::is_dirty)
        {
            self.error = Some(
                "Save or explicitly discard the current draft before switching repositories."
                    .into(),
            );
            cx.notify();
            return;
        }
        match self.gh_cli_status {
            GhCliStatus::Checking => {
                self.status =
                    Some("Checking the GitHub CLI prerequisite. Try again in a moment.".into());
                cx.notify();
                return;
            }
            GhCliStatus::Missing => {
                self.error = Some(gh_install_prompt());
                cx.notify();
                return;
            }
            GhCliStatus::Failed(ref detail) => {
                self.error = Some(format!("Could not verify the GitHub CLI: {detail}"));
                cx.notify();
                return;
            }
            GhCliStatus::Available(_) => {}
        }
        let home = home_directory();
        let submitted_path = path.trim().to_owned();
        let path = match resolve_repository_path(&submitted_path, &home) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(error) => {
                self.error = Some(format!("{}: {}", error.code, error.message));
                cx.notify();
                return;
            }
        };
        if path.is_empty() {
            self.error = Some("Enter a repository checkout path.".into());
            cx.notify();
            return;
        }
        self.cancel_pending();
        self.error = None;
        self.status = None;
        self.screen = Screen::Repository;
        self.active_repository = Some(path.clone());
        self.repository_input = path.clone();
        self.prefs.repositories.retain(|saved| {
            saved != &submitted_path
                && resolve_repository_path(saved, &home)
                    .map(|saved_path| saved_path.to_string_lossy() != path)
                    .unwrap_or(true)
        });
        self.prefs.repositories.insert(0, path.clone());
        self.prefs.repositories.truncate(20);
        self.persist();
        self.load_pull_requests(path, cx);
    }

    fn choose_repository_folder(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose Repository".into()),
        });
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| match prompt.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.first() {
                    let path = path.to_string_lossy().into_owned();
                    let _ = weak.update(cx, |this, cx| {
                        this.repository_input = path;
                        this.error = None;
                        this.status = None;
                        cx.notify();
                    });
                }
            }
            Ok(Ok(None)) | Err(_) => {}
            Ok(Err(error)) => {
                let _ = weak.update(cx, |this, cx| {
                    this.error = Some(format!("Could not open the folder picker: {error}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn refresh_gh_cli(&mut self, cx: &mut Context<Self>) {
        let token = self.gh_check_epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let epoch = self.gh_check_epoch.clone();
        let weak = cx.entity().downgrade();
        self.gh_cli_status = GhCliStatus::Checking;
        cx.spawn(async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async { probe_gh_cli(Duration::from_secs(3)) })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if epoch.load(Ordering::Relaxed) != token {
                    return;
                }
                this.gh_cli_status = result.clone();
                if let GhCliStatus::Available(_) = result
                    && let Some((path, initial_pr)) = this.pending_initial.take()
                {
                    this.open_repository(path.clone(), cx);
                    if let Some(number) = initial_pr {
                        this.prepare(path, number, cx);
                    }
                } else if matches!(result, GhCliStatus::Missing) {
                    this.pending_initial = None;
                    this.error = Some(gh_install_prompt());
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_pull_requests(&mut self, path: String, cx: &mut Context<Self>) {
        self.cancel_pending();
        let token = self.operation_epoch.load(Ordering::Relaxed);
        let control = OperationControl::with_timeout(Duration::from_secs(30));
        self.active_control = Some(control.clone());
        self.loading = true;
        self.error = None;
        self.pull_requests.clear();
        self.busy_label = "Loading pull requests…".into();
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { list_pull_requests(Path::new(&path), &control) })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token {
                    return;
                }
                this.loading = false;
                this.active_control = None;
                match result {
                    Ok(rows) => {
                        this.pull_requests = rows;
                        this.error = None;
                    }
                    Err(message) => {
                        this.error = Some(message);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn prepare(&mut self, path: String, number: u64, cx: &mut Context<Self>) {
        if self.draft_saving
            || self
                .draft_editor
                .as_ref()
                .is_some_and(TextDraftEditor::is_dirty)
        {
            self.error = Some(
                "Save or explicitly discard the current draft before opening another review."
                    .into(),
            );
            cx.notify();
            return;
        }
        self.cancel_pending();
        self.screen = Screen::Repository;
        self.active_repository = Some(path.clone());
        self.loading = true;
        self.busy_label = format!("Preparing PR #{number}…");
        self.error = None;
        self.status = None;
        let token = self.operation_epoch.load(Ordering::Relaxed);
        let control = OperationControl::with_timeout(Duration::from_secs(60));
        self.active_control = Some(control.clone());
        let state = self.preparation_db.clone();
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut manifest =
                        bodh::prepare_local_pr(Path::new(&path), number, None, &state, &control)?;
                    let binding = PreparationStore::open(&state)?.load(&manifest.preparation_id)?;
                    let drafts = DraftStore::open(&state)?.list(&manifest.preparation_id)?;
                    manifest.expected_generation = drafts.workspace_revision;
                    manifest.draft_counts = drafts.counts.clone();
                    let markdown = binding.description.as_deref().unwrap_or_default();
                    let blocks = safe_markdown_blocks(markdown);
                    Ok::<_, bodh::ContractError>((manifest, binding, blocks, drafts))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token {
                    return;
                }
                this.loading = false;
                this.active_control = None;
                match result {
                    Ok((manifest, binding, blocks, drafts)) => {
                        this.remember(&manifest, &binding, cx);
                        this.selected_review = Some(binding);
                        this.description_blocks = blocks;
                        this.draft_snapshot = Some(drafts);
                        this.draft_editor = None;
                        this.draft_saving = false;
                        this.active_tab = this.prefs.active_tab.unwrap_or(WorkspaceTab::Overview);
                        this.screen = Screen::Workspace;
                        this.selected_file = None;
                        this.source_text = None;
                        this.source_loading = false;
                        this.source_label = None;
                        this.source_ref = None;
                        this.diff_page = None;
                        this.diff_rows.clear();
                        this.diff_loading = false;
                        this.diff_error = None;
                        this.error = None;
                    }
                    Err(error) => {
                        this.error = Some(format!("{}: {}", error.code, error.message));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn remember(
        &mut self,
        manifest: &PreparationManifest,
        binding: &PreparedBinding,
        cx: &mut Context<Self>,
    ) {
        let pr = &binding.context.pr;
        let path = binding.checkout.clone();
        self.prefs
            .recent
            .retain(|item| item.preparation_id != binding.preparation_id);
        self.prefs.recent.insert(
            0,
            RecentReview {
                preparation_id: binding.preparation_id.clone(),
                repository_path: path,
                title: manifest.title.clone(),
                owner: pr.owner.clone(),
                repository: pr.repository.clone(),
                number: pr.number,
                pending_work: pending_draft_count(&manifest.draft_counts),
                selected_file: None,
                source_revision: None,
                split_diff: false,
                file_scroll_y: 0.,
                diff_scroll_y: 0.,
                source_scroll_y: 0.,
                description_scroll_y: 0.,
            },
        );
        self.prefs.recent.truncate(20);
        self.persist();
        let _ = cx;
    }

    fn resume(&mut self, id: String, cx: &mut Context<Self>) {
        if self.draft_saving
            || self
                .draft_editor
                .as_ref()
                .is_some_and(TextDraftEditor::is_dirty)
        {
            self.error = Some(
                "Save or explicitly discard the current draft before switching reviews.".into(),
            );
            cx.notify();
            return;
        }
        self.cancel_pending();
        self.loading = true;
        self.busy_label = "Resuming review…".into();
        self.error = None;
        let state = self.preparation_db.clone();
        let token = self.operation_epoch.load(Ordering::Relaxed);
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let binding = PreparationStore::open(&state)?.load(&id)?;
                    let drafts = DraftStore::open(&state)?.list(&id)?;
                    let blocks =
                        safe_markdown_blocks(binding.description.as_deref().unwrap_or_default());
                    Ok::<_, bodh::ContractError>((binding, blocks, drafts))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token {
                    return;
                }
                this.loading = false;
                match result {
                    Ok((binding, blocks, drafts)) => {
                        this.active_repository = Some(binding.checkout.clone());
                        this.screen = Screen::Workspace;
                        this.description_blocks = blocks;
                        this.draft_snapshot = Some(drafts.clone());
                        this.draft_editor = None;
                        this.draft_saving = false;
                        this.selected_review = Some(binding.clone());
                        this.update_draft_counts(drafts.counts);
                        this.active_tab = this.prefs.active_tab.unwrap_or(WorkspaceTab::Overview);
                        if let Some(saved) = this
                            .prefs
                            .recent
                            .iter()
                            .find(|review| review.preparation_id == binding.preparation_id)
                            .cloned()
                        {
                            this.source_revision = saved.source_revision.unwrap_or(Revision::Head);
                            this.split_diff = saved.split_diff;
                            this.description_scroll
                                .set_offset(point(px(0.), px(saved.description_scroll_y)));
                            this.selected_file = saved.selected_file.as_deref().and_then(|path| {
                                binding.changed_files.iter().position(|file| {
                                    file.new_path.as_deref().or(file.old_path.as_deref())
                                        == Some(path)
                                })
                            });
                            if let Some(index) = this.selected_file {
                                this.select_file(index, cx);
                                this.file_scroll
                                    .0
                                    .borrow()
                                    .base_handle
                                    .set_offset(point(px(0.), px(saved.file_scroll_y)));
                                this.diff_scroll
                                    .0
                                    .borrow()
                                    .base_handle
                                    .set_offset(point(px(0.), px(saved.diff_scroll_y)));
                                this.source_scroll
                                    .set_offset(point(px(0.), px(saved.source_scroll_y)));
                            }
                        }
                    }
                    Err(error) => this.error = Some(format!("{}: {}", error.code, error.message)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_file(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        let Some(file) = binding.changed_files.get(index).cloned() else {
            return;
        };
        self.cancel_pending();
        self.selected_file = Some(index);
        self.diff_page = None;
        self.diff_rows.clear();
        self.diff_error = None;
        self.diff_loading = true;
        self.source_text = None;
        self.source_loading = true;
        self.source_ref = None;
        self.source_label = None;
        self.diff_scroll
            .0
            .borrow()
            .base_handle
            .set_offset(point(px(0.), px(0.)));
        self.source_scroll.set_offset(point(px(0.), px(0.)));
        let revision = if file.new_path.is_some() {
            Revision::Head
        } else {
            Revision::Base
        };
        self.source_revision = revision;
        let Some(path) = (match revision {
            Revision::Head => file.new_path.clone(),
            Revision::Base => file.old_path.clone(),
        }) else {
            self.diff_loading = false;
            self.source_loading = false;
            self.diff_error = Some("This tree entry has no readable file path.".into());
            cx.notify();
            return;
        };
        if let Some(reason) = file.unsupported_reason {
            self.diff_loading = false;
            self.source_loading = false;
            self.diff_error = Some(format!("Unsupported content: {reason}"));
            self.source_label = Some(format!("Unsupported content: {reason}"));
            self.persist_navigation();
            cx.notify();
            return;
        }
        let Some(hunk) = file.hunks.first() else {
            self.diff_loading = false;
            self.source_loading = false;
            self.diff_error = Some("This file has no line-based source range.".into());
            self.source_label = Some("This file has no line-based source range.".into());
            self.persist_navigation();
            cx.notify();
            return;
        };
        let (start, count) = match revision {
            Revision::Head => (hunk.new_start, hunk.new_lines),
            Revision::Base => (hunk.old_start, hunk.old_lines),
        };
        if count == 0 {
            self.diff_loading = false;
            self.source_loading = false;
            self.source_label =
                Some("This change has no readable lines on the selected side.".into());
            self.diff_error = Some("No readable lines are available for this file.".into());
            cx.notify();
            return;
        }
        let source = SourceRef {
            path: path.clone(),
            revision,
            start_line: start.max(1),
            end_line: start
                .saturating_add(count.saturating_sub(1))
                .min(start.saturating_add(79)),
        };
        self.source_ref = Some(source.clone());
        self.source_label = Some(source_title(&source));
        let checkout = binding.checkout.clone();
        let context = binding.context.clone();
        let control = OperationControl::with_timeout(Duration::from_secs(30));
        self.active_control = Some(control.clone());
        let token = self.operation_epoch.load(Ordering::Relaxed);
        self.persist_navigation();
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let diff = read_diff(Path::new(&checkout), &context, &path, &control)?;
                    let source_text =
                        bodh::read_source(Path::new(&checkout), &context, &source, &control)?;
                    Ok::<_, bodh::ContractError>((diff, source_text))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token
                    || this.selected_file != Some(index)
                {
                    return;
                }
                this.active_control = None;
                this.diff_loading = false;
                this.source_loading = false;
                match result {
                    Ok((diff, text)) => {
                        this.diff_rows = diff_rows(&diff.text);
                        this.diff_page = Some(diff);
                        this.source_text = Some(text);
                    }
                    Err(error) => {
                        let message = format!("{}: {}", error.code, error.message);
                        this.diff_error = Some(message.clone());
                        this.source_label = Some(message);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn select_source_line(
        &mut self,
        index: usize,
        revision: Revision,
        line: u32,
        cx: &mut Context<Self>,
    ) {
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        let Some(file) = binding.changed_files.get(index) else {
            return;
        };
        let path = match revision {
            Revision::Base => file.old_path.as_ref(),
            Revision::Head => file.new_path.as_ref(),
        };
        let Some(path) = path.cloned() else {
            return;
        };
        let source = SourceRef {
            path,
            revision,
            start_line: line.max(1),
            end_line: line.max(1),
        };
        self.cancel_pending();
        self.selected_file = Some(index);
        self.source_revision = revision;
        self.source_ref = Some(source.clone());
        self.source_label = Some(source_title(&source));
        self.source_text = None;
        self.source_loading = true;
        self.source_scroll.set_offset(point(px(0.), px(0.)));
        self.persist_navigation();
        let repository = binding.checkout.clone();
        let context = binding.context.clone();
        let control = OperationControl::with_timeout(Duration::from_secs(10));
        self.active_control = Some(control.clone());
        let token = self.operation_epoch.load(Ordering::Relaxed);
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    bodh::read_source(Path::new(&repository), &context, &source, &control)
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token
                    || this.selected_file != Some(index)
                {
                    return;
                }
                this.active_control = None;
                this.source_loading = false;
                match result {
                    Ok(text) => this.source_text = Some(text),
                    Err(error) => {
                        this.source_label = Some(format!("{}: {}", error.code, error.message));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn begin_text_draft(&mut self, cx: &mut Context<Self>) {
        if self.draft_saving {
            return;
        }
        let Some(source) = self.source_ref.clone() else {
            self.status = Some("Select a changed diff line before drafting a comment.".into());
            cx.notify();
            return;
        };
        if self
            .draft_editor
            .as_ref()
            .is_some_and(TextDraftEditor::is_dirty)
        {
            self.status = Some(
                "Save or explicitly discard the current draft before starting another.".into(),
            );
            cx.notify();
            return;
        }
        let workspace_revision = self
            .draft_snapshot
            .as_ref()
            .map(|snapshot| snapshot.workspace_revision)
            .unwrap_or_else(|| {
                self.selected_review
                    .as_ref()
                    .map(|binding| binding.expected_generation)
                    .unwrap_or(0)
            });
        self.draft_editor = Some(TextDraftEditor::new(source, workspace_revision));
        self.status =
            Some("New inline comment draft. Saving keeps it local; nothing is published.".into());
        cx.notify();
    }

    fn resume_text_draft(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.draft_saving
            || self
                .draft_editor
                .as_ref()
                .is_some_and(TextDraftEditor::is_dirty)
        {
            self.status = Some(
                "Save or explicitly discard the current draft before resuming another.".into(),
            );
            cx.notify();
            return;
        }
        let Some(snapshot) = self.draft_snapshot.clone() else {
            return;
        };
        let Some(draft) = snapshot.drafts.iter().find(|draft| draft.id == id).cloned() else {
            self.error = Some("That saved draft is no longer in this review context.".into());
            cx.notify();
            return;
        };
        let index = self.selected_review.as_ref().and_then(|binding| {
            binding
                .changed_files
                .iter()
                .position(|file| match draft.source.revision {
                    Revision::Base => file.old_path.as_deref() == Some(draft.source.path.as_str()),
                    Revision::Head => file.new_path.as_deref() == Some(draft.source.path.as_str()),
                })
        });
        self.draft_editor = Some(TextDraftEditor::resume(&draft, snapshot.workspace_revision));
        if let Some(index) = index {
            self.select_source_line(index, draft.source.revision, draft.source.start_line, cx);
        }
        self.status = Some("Saved local comment draft resumed.".into());
        cx.notify();
    }

    fn save_text_draft(&mut self, cx: &mut Context<Self>) {
        if self.draft_saving {
            return;
        }
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        let Some(editor) = self.draft_editor.clone() else {
            return;
        };
        if !editor.can_save() {
            self.status = Some("Add comment text before saving the local draft.".into());
            cx.notify();
            return;
        }
        let submitted_body = editor.body.clone();
        let draft_id = editor.draft_id.clone();
        let source = editor.source.clone();
        let workspace_revision = editor.workspace_revision;
        let draft_revision = editor.draft_revision;
        let preparation_id = binding.preparation_id.clone();
        let preparation_id_for_ui = preparation_id.clone();
        let submitted_body_for_save = submitted_body.clone();
        let state = self.preparation_db.clone();
        let control = OperationControl::with_timeout(Duration::from_secs(20));
        let epoch = self.draft_epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let draft_epoch = self.draft_epoch.clone();
        self.draft_saving = true;
        self.status = Some("Saving local comment draft…".into());
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut store = DraftStore::open(Path::new(&state))?;
                    let saved = store.save(
                        &preparation_id,
                        draft_id.as_deref(),
                        &submitted_body_for_save,
                        &source,
                        workspace_revision,
                        draft_revision,
                        &control,
                    );
                    let snapshot = store.list(&preparation_id).ok();
                    Ok::<_, bodh::ContractError>((saved, snapshot))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if draft_epoch.load(Ordering::Relaxed) != epoch
                    || this
                        .selected_review
                        .as_ref()
                        .is_none_or(|current| current.preparation_id != preparation_id_for_ui)
                {
                    return;
                }
                this.draft_saving = false;
                match result {
                    Ok((Ok(saved), snapshot)) => {
                        if let Some(snapshot) = snapshot {
                            this.update_draft_counts(snapshot.counts.clone());
                            this.draft_snapshot = Some(snapshot);
                        }
                        if let Some(editor) = &mut this.draft_editor {
                            editor.save_succeeded(&saved, &submitted_body);
                        }
                        this.status =
                            Some("Draft saved locally. It has not been published.".into());
                        this.error = None;
                    }
                    Ok((Err(error), snapshot)) => {
                        if let Some(snapshot) = snapshot {
                            this.update_draft_counts(snapshot.counts.clone());
                            this.draft_snapshot = Some(snapshot);
                        }
                        if let Some(editor) = &mut this.draft_editor {
                            editor.save_failed(error.code.clone(), error.message.clone());
                        }
                        this.status =
                            Some("Save failed; your entered text is still in this editor.".into());
                        this.error = Some(format!("{}: {}", error.code, error.message));
                    }
                    Err(error) => {
                        if let Some(editor) = &mut this.draft_editor {
                            editor.save_failed(error.code.clone(), error.message.clone());
                        }
                        this.status =
                            Some("Save failed; your entered text is still in this editor.".into());
                        this.error = Some(format!("{}: {}", error.code, error.message));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn discard_text_draft(&mut self, cx: &mut Context<Self>) {
        if self.draft_saving {
            return;
        }
        let Some(editor) = self.draft_editor.clone() else {
            return;
        };
        let Some(id) = editor.draft_id.clone() else {
            if let Some(editor) = &mut self.draft_editor {
                editor.clear_new_draft();
            }
            self.status = Some("Unsaved local text cleared; no stored draft was changed.".into());
            cx.notify();
            return;
        };
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        let state = self.preparation_db.clone();
        let preparation_id = binding.preparation_id.clone();
        let preparation_id_for_ui = preparation_id.clone();
        let epoch = self.draft_epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let draft_epoch = self.draft_epoch.clone();
        self.draft_saving = true;
        self.status = Some("Discarding local draft…".into());
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut store = DraftStore::open(Path::new(&state))?;
                    let discarded = store.discard(
                        &preparation_id,
                        &id,
                        editor.workspace_revision,
                        editor.draft_revision,
                    );
                    let snapshot = store.list(&preparation_id).ok();
                    Ok::<_, bodh::ContractError>((discarded, snapshot))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if draft_epoch.load(Ordering::Relaxed) != epoch
                    || this
                        .selected_review
                        .as_ref()
                        .is_none_or(|current| current.preparation_id != preparation_id_for_ui)
                {
                    return;
                }
                this.draft_saving = false;
                match result {
                    Ok((Ok(snapshot), current)) => {
                        this.draft_snapshot = Some(current.unwrap_or(snapshot.clone()));
                        this.update_draft_counts(snapshot.counts);
                        if let Some(editor) = &mut this.draft_editor {
                            editor.discard_succeeded(snapshot.workspace_revision);
                        }
                        this.status = Some("Local draft discarded. Nothing was published.".into());
                        this.error = None;
                    }
                    Ok((Err(error), snapshot)) => {
                        if let Some(snapshot) = snapshot {
                            this.update_draft_counts(snapshot.counts.clone());
                            this.draft_snapshot = Some(snapshot);
                        }
                        if let Some(editor) = &mut this.draft_editor {
                            editor.save_failed(error.code.clone(), error.message.clone());
                        }
                        this.status = Some(
                            "Discard failed; the saved draft and your entered text were retained."
                                .into(),
                        );
                        this.error = Some(format!("{}: {}", error.code, error.message));
                    }
                    Err(error) => {
                        if let Some(editor) = &mut this.draft_editor {
                            editor.save_failed(error.code.clone(), error.message.clone());
                        }
                        this.status = Some(
                            "Discard failed; the saved draft and your entered text were retained."
                                .into(),
                        );
                        this.error = Some(format!("{}: {}", error.code, error.message));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn recover_draft_as_new(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.draft_snapshot.as_ref() else {
            return;
        };
        let Some(editor) = &mut self.draft_editor else {
            return;
        };
        editor.recover_as_new(snapshot.workspace_revision);
        self.status =
            Some("Local text is now a new draft; the newer saved version remains intact.".into());
        self.error = None;
        cx.notify();
    }

    fn update_draft_counts(&mut self, counts: bodh::DraftCounts) {
        if let Some(binding) = &self.selected_review
            && let Some(recent) = self
                .prefs
                .recent
                .iter_mut()
                .find(|recent| recent.preparation_id == binding.preparation_id)
        {
            recent.pending_work = pending_draft_count(&counts);
            self.persist();
        }
    }

    fn persist_navigation(&mut self) {
        let Some(binding) = &self.selected_review else {
            return;
        };
        let id = &binding.preparation_id;
        if let Some(saved) = self
            .prefs
            .recent
            .iter_mut()
            .find(|r| &r.preparation_id == id)
        {
            saved.selected_file = self.selected_file.and_then(|index| {
                binding
                    .changed_files
                    .get(index)
                    .and_then(|file| file.new_path.clone().or(file.old_path.clone()))
            });
            saved.source_revision = Some(self.source_revision);
            saved.split_diff = self.split_diff;
            saved.file_scroll_y =
                f32::from(self.file_scroll.0.borrow().base_handle.offset().y).max(0.);
            saved.diff_scroll_y =
                f32::from(self.diff_scroll.0.borrow().base_handle.offset().y).max(0.);
            saved.source_scroll_y = f32::from(self.source_scroll.offset().y).max(0.);
            saved.description_scroll_y = f32::from(self.description_scroll.offset().y).max(0.);
        }
        self.persist();
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        if self.draft_saving
            || self
                .draft_editor
                .as_ref()
                .is_some_and(TextDraftEditor::is_dirty)
        {
            self.error = Some(
                "Save or explicitly discard the current draft before leaving this review.".into(),
            );
            cx.notify();
            return;
        }
        self.cancel_pending();
        self.error = None;
        self.status = None;
        self.screen = match self.screen {
            Screen::Workspace => Screen::Repository,
            Screen::Repository => Screen::Home,
            Screen::Home => Screen::Home,
        };
        if self.screen == Screen::Repository
            && let Some(path) = self.active_repository.clone()
        {
            self.load_pull_requests(path, cx);
        }
        cx.notify();
    }

    fn palette(&self) -> Palette {
        Palette::new(self.prefs.dark)
    }

    fn search_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match key {
            "backspace" => {
                self.search.pop();
            }
            "escape" => self.search.clear(),
            _ if key.chars().count() == 1
                && !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.platform =>
            {
                self.search.push_str(key)
            }
            _ => return,
        }
        cx.notify();
    }

    fn workspace_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        if self.draft_focus.is_focused(window) {
            return;
        }
        if self.screen == Screen::Repository {
            if self.search_focus.is_focused(window) {
                return;
            }
            let rows = filter_pull_requests(&self.pull_requests, &self.search, &self.lifecycle);
            if rows.is_empty() {
                return;
            }
            let current = self
                .selected_pr
                .and_then(|selected| rows.iter().position(|pr| pr.number == selected));
            match event.keystroke.key.as_str() {
                "down" => {
                    self.selected_pr =
                        Some(rows[current.map(|i| (i + 1) % rows.len()).unwrap_or(0)].number)
                }
                "up" => {
                    self.selected_pr = Some(
                        rows[current
                            .map(|i| (i + rows.len() - 1) % rows.len())
                            .unwrap_or(rows.len() - 1)]
                        .number,
                    )
                }
                "enter" => {
                    if let (Some(path), Some(number)) = (
                        self.active_repository.clone(),
                        self.selected_pr
                            .or_else(|| rows.first().map(|pr| pr.number)),
                    ) {
                        self.prepare(path, number, cx);
                    }
                }
                _ => {}
            }
            cx.notify();
            return;
        }
        match event.keystroke.key.as_str() {
            "[" => self.prefs.inspector_width = (self.prefs.inspector_width - 24.).max(340.),
            "]" => {
                self.prefs.inspector_width = (self.prefs.inspector_width + 24.)
                    .min(f32::from(window.viewport_size().width) - 300.)
            }
            _ => return,
        }
        self.persist();
        cx.notify();
    }
}

impl Render for Product {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let mut root = div()
            .id("bodh-workspace")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.canvas))
            .text_color(rgb(p.text))
            .font_family("DejaVu Sans")
            .text_size(px(14.))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.workspace_key(event, window, cx)
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if this.dragging_pane && event.pressed_button == Some(MouseButton::Left) {
                    let width = f32::from(window.viewport_size().width);
                    let next =
                        (width - f32::from(event.position.x)).clamp(280., (width - 300.).max(280.));
                    if (next - this.prefs.inspector_width).abs() >= 1. {
                        this.prefs.inspector_width = next;
                        this.persist();
                        cx.notify();
                    }
                } else {
                    this.dragging_pane = false;
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging_pane = false),
            );
        root = root.child(self.render_topbar(p, cx));
        root = match self.screen {
            Screen::Home => root.child(self.render_home(p, cx)),
            Screen::Repository => root.child(self.render_repository(p, cx)),
            Screen::Workspace => root.child(self.render_workspace(
                p,
                cx,
                f32::from(_window.viewport_size().width),
                f32::from(_window.viewport_size().height),
            )),
        };
        if let Some(error) = self.error.clone() {
            root = root.child(self.notice(p, &error, true));
        }
        if let Some(status) = self.status.clone() {
            root = root.child(self.notice(p, &status, false));
        }
        if self.loading {
            root = root.child(self.notice(p, &self.busy_label.clone(), false));
        }
        root
    }
}

impl Product {
    fn render_topbar(&mut self, p: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self
            .selected_review
            .as_ref()
            .map(|r| {
                format!(
                    "{}/{}  #{}",
                    r.context.pr.owner, r.context.pr.repository, r.context.pr.number
                )
            })
            .unwrap_or_else(|| "Bodh".into());
        let mut bar = div()
            .flex()
            .items_center()
            .gap_3()
            .px_6()
            .py_3()
            .border_b_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.panel));
        bar = bar.child(
            div()
                .font_weight(FontWeight::BOLD)
                .text_size(px(17.))
                .child("Bodh"),
        );
        bar = bar.child(div().text_color(rgb(p.muted)).child(title));
        bar = bar.child(div().flex_1());
        if self.screen != Screen::Home {
            bar = bar.child(button(
                "back",
                "‹  Back",
                p,
                |this, _, _, cx| this.back(cx),
                cx,
            ));
        }
        bar = bar.child(button(
            "home",
            "Home",
            p,
            |this, _, _, cx| {
                if this.draft_saving
                    || this
                        .draft_editor
                        .as_ref()
                        .is_some_and(TextDraftEditor::is_dirty)
                {
                    this.error = Some(
                        "Save or explicitly discard the current draft before leaving this review."
                            .into(),
                    );
                    cx.notify();
                } else {
                    this.cancel_pending();
                    this.screen = Screen::Home;
                    this.error = None;
                    cx.notify();
                }
            },
            cx,
        ));
        bar = bar.child(button(
            "theme",
            if self.prefs.dark { "Light" } else { "Dark" },
            p,
            |this, _, _, cx| {
                this.prefs.dark = !this.prefs.dark;
                this.persist();
                cx.notify();
            },
            cx,
        ));
        if self.selected_review.is_some() {
            let pending = self
                .draft_snapshot
                .as_ref()
                .map(|snapshot| pending_draft_count(&snapshot.counts))
                .unwrap_or(0);
            let finish_label = if pending > 0 {
                format!("Finish Review · {pending} local")
            } else {
                "Finish Review".to_owned()
            };
            bar = bar.child(button(
                "finish-review",
                &finish_label,
                p,
                move |this, _, _, cx| {
                    let pending = this
                        .draft_snapshot
                        .as_ref()
                        .map(|snapshot| pending_draft_count(&snapshot.counts))
                        .unwrap_or(0);
                    this.status = Some(if pending > 0 {
                        format!("{pending} local comment draft(s) are saved. Nothing has been published.")
                    } else {
                        "No review content has been staged yet. Your saved workspace is unchanged.".into()
                    });
                    cx.notify();
                },
                cx,
            ));
        }
        bar
    }

    fn render_home(&mut self, p: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div()
            .id("home-content")
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .p_6()
            .flex()
            .flex_col()
            .gap_5();
        content = content.child(
            div()
                .child(
                    div()
                        .text_size(px(24.))
                        .font_weight(FontWeight::BOLD)
                        .child("Your review workspace"),
                )
                .child(div().mt_2().text_color(rgb(p.muted)).child(
                    "Open a local checkout to browse its pull requests, or resume a saved review.",
                )),
        );
        let (cli_title, cli_message, can_retry) = match &self.gh_cli_status {
            GhCliStatus::Checking => (
                "Checking GitHub CLI",
                "Bodh checks for the GitHub CLI before opening a repository.",
                false,
            ),
            GhCliStatus::Available(version) => ("GitHub CLI ready", version.as_str(), false),
            GhCliStatus::Missing => (
                "GitHub CLI required",
                "Install GitHub CLI from https://cli.github.com/ and restart Bodh.",
                true,
            ),
            GhCliStatus::Failed(detail) => ("Could not verify GitHub CLI", detail.as_str(), true),
        };
        let mut cli_card =
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(empty_card(cli_title, cli_message, p));
        if can_retry {
            cli_card = cli_card.child(button(
                "check-gh-again",
                "Check Again",
                p,
                |this, _, _, cx| this.refresh_gh_cli(cx),
                cx,
            ));
        }
        content = content.child(cli_card);
        let mut repos = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(section_title("Repositories", p));
        if self.prefs.repositories.is_empty() {
            repos = repos.child(empty_card("No registered repositories yet", "Add an existing Git checkout below. Bodh reads it without changing its branch, index, or working files.", p));
        } else {
            for path in self.prefs.repositories.clone() {
                let open_path = path.clone();
                let label = path.clone();
                repos = repos.child(button(
                    &format!("repo-{label}"),
                    &label,
                    p,
                    move |this, _, _, cx| this.open_repository(open_path.clone(), cx),
                    cx,
                ));
            }
        }
        content = content.child(repos);
        content = content.child(
            div()
                .flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .p_3()
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.panel))
                        .focus(|style| style.border_color(rgb(p.accent)))
                        .track_focus(&self.search_focus)
                        .child(if self.repository_input.is_empty() {
                            "Repository path · type a path then press Enter".to_owned()
                        } else {
                            self.repository_input.clone()
                        })
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                            let key = event.keystroke.key.as_str();
                            if key == "backspace" {
                                this.repository_input.pop();
                            } else if key == "enter" {
                                let value = this.repository_input.clone();
                                this.open_repository(value, cx);
                            } else if let Some(text) =
                                bodh_sequence_spike::repository_input::typed_text(
                                    key,
                                    event.keystroke.key_char.as_deref(),
                                    event.keystroke.modifiers.control,
                                    event.keystroke.modifiers.platform,
                                )
                            {
                                this.repository_input.push_str(text);
                            }
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, _| window.focus(&this.search_focus)),
                        ),
                )
                .child(button(
                    "choose-repository-folder",
                    "Choose Folder",
                    p,
                    |this, _, _, cx| this.choose_repository_folder(cx),
                    cx,
                ))
                .child(button(
                    "open-repository",
                    "Open Repository",
                    p,
                    move |this, _, _, cx| this.open_repository(this.repository_input.clone(), cx),
                    cx,
                )),
        );
        let mut recent = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(section_title("Recent reviews", p));
        if self.prefs.recent.is_empty() {
            recent = recent.child(empty_card(
                "No recent reviews",
                "Prepared reviews appear here and can be resumed later.",
                p,
            ));
        } else {
            for review in self.prefs.recent.clone() {
                let id = review.preparation_id.clone();
                let label = format!(
                    "{}/{}  #{}  ·  {}  ·  {} pending",
                    review.owner,
                    review.repository,
                    review.number,
                    review.title,
                    review.pending_work
                );
                recent = recent.child(button(
                    &format!("recent-{}", review.preparation_id),
                    &label,
                    p,
                    move |this, _, _, cx| this.resume(id.clone(), cx),
                    cx,
                ));
            }
        }
        content.child(recent)
    }

    fn render_repository(&mut self, p: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let path = self.active_repository.clone().unwrap_or_default();
        let filtered = filter_pull_requests(&self.pull_requests, &self.search, &self.lifecycle);
        let mut content = div()
            .id("repository-browser")
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .p_6()
            .flex()
            .flex_col()
            .gap_4();
        content = content.child(
            div()
                .child(
                    div()
                        .text_size(px(22.))
                        .font_weight(FontWeight::BOLD)
                        .child("Pull requests"),
                )
                .child(div().mt_1().text_color(rgb(p.muted)).child(path)),
        );
        let mut filter = div().flex().items_center().gap_2();
        filter = filter.child(
            div()
                .id("pr-search")
                .w(px(360.))
                .p_3()
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.panel))
                .focus(|style| style.border_color(rgb(p.accent)))
                .track_focus(&self.search_focus)
                .child(if self.search.is_empty() {
                    "Search title, number or author · type to filter".to_owned()
                } else {
                    self.search.clone()
                })
                .on_key_down(
                    cx.listener(|this, event: &KeyDownEvent, _, cx| this.search_key(event, cx)),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, _| window.focus(&this.search_focus)),
                ),
        );
        filter = filter.child(button(
            "lifecycle",
            &format!("Lifecycle · {}", self.lifecycle),
            p,
            |this, _, _, cx| {
                this.lifecycle = match this.lifecycle.as_str() {
                    "All" => "Open".into(),
                    "Open" => "Closed".into(),
                    _ => "All".into(),
                };
                cx.notify();
            },
            cx,
        ));
        content = content.child(filter);
        let mut list = div().flex().flex_col().gap_2();
        if self.loading {
            list = list.child(empty_card(
                &self.busy_label,
                "This runs outside the UI thread; changing repositories invalidates its response.",
                p,
            ));
        } else if self.error.is_some() {
            list = list.child(empty_card("Pull requests could not be loaded", "Check that `gh` is installed, authenticated, and this checkout has a matching GitHub remote.", p));
        } else if filtered.is_empty() {
            list = list.child(empty_card(
                "No pull requests match",
                if self.pull_requests.is_empty() {
                    "This repository returned no pull requests."
                } else {
                    "Try a different title, number, author, or lifecycle filter."
                },
                p,
            ));
        } else {
            for pr in filtered {
                let path = self.active_repository.clone().unwrap_or_default();
                let number = pr.number;
                let label = format!("#{}  {}", pr.number, pr.title);
                let author = pr.author.login.clone();
                let state = pr.state.to_ascii_lowercase();
                let local_state = if self
                    .prefs
                    .recent
                    .iter()
                    .any(|recent| recent.repository_path == path && recent.number == number)
                {
                    "Saved workspace"
                } else {
                    "No local workspace"
                };
                let selected = self.selected_pr == Some(number);
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .p_3()
                        .border_1()
                        .border_color(rgb(if selected { p.accent } else { p.border }))
                        .bg(rgb(if selected { p.raised } else { p.panel }))
                        .child(
                            div()
                                .flex_1()
                                .child(div().font_weight(FontWeight::MEDIUM).child(label.clone()))
                                .child(
                                    div()
                                        .mt_1()
                                        .text_size(px(12.))
                                        .text_color(rgb(p.muted))
                                        .child(format!("{} · {} · {}", author, state, local_state)),
                                ),
                        )
                        .child(button(
                            &format!("open-pr-{number}"),
                            "Open review",
                            p,
                            move |this, _, _, cx| {
                                this.selected_pr = Some(number);
                                this.prepare(path.clone(), number, cx);
                            },
                            cx,
                        )),
                );
            }
        }
        content.child(list)
    }

    fn render_workspace(
        &mut self,
        p: Palette,
        cx: &mut Context<Self>,
        viewport_width: f32,
        viewport_height: f32,
    ) -> impl IntoElement {
        let binding = self
            .selected_review
            .clone()
            .expect("workspace screen has a prepared review");
        let mut page = div()
            .id("review-workspace")
            .flex_1()
            .min_h_0()
            .p_6()
            .flex()
            .flex_col()
            .gap_4();
        page = if self.active_tab == WorkspaceTab::Changes {
            page.overflow_hidden()
        } else {
            page.overflow_scroll()
        };
        page = page.child(
            div()
                .child(
                    div()
                        .text_size(px(21.))
                        .font_weight(FontWeight::BOLD)
                        .child(binding.title.clone()),
                )
                .child(
                    div()
                        .mt_1()
                        .text_size(px(12.))
                        .font_family("DejaVu Sans Mono")
                        .text_color(rgb(p.muted))
                        .child(format!(
                            "{}/{} #{} · base {} · head {}",
                            binding.context.pr.owner,
                            binding.context.pr.repository,
                            binding.context.pr.number,
                            short_sha(&binding.context.base_sha),
                            short_sha(&binding.context.head_sha)
                        )),
                ),
        );
        let mut tabs = div()
            .flex()
            .gap_2()
            .border_b_1()
            .border_color(rgb(p.border));
        for tab in [
            WorkspaceTab::Overview,
            WorkspaceTab::Changes,
            WorkspaceTab::Findings,
        ] {
            let active = self.active_tab == tab;
            tabs = tabs.child(
                button(
                    &format!("tab-{}", tab.label()),
                    tab.label(),
                    p,
                    move |this, _, _, cx| {
                        this.active_tab = tab;
                        this.prefs.active_tab = Some(tab);
                        this.persist();
                        cx.notify();
                    },
                    cx,
                )
                .border_b_2()
                .border_color(rgb(if active { p.accent } else { p.panel }))
                .text_color(rgb(if active { p.text } else { p.muted })),
            );
        }
        page = page.child(tabs);
        let panel: AnyElement = match self.active_tab {
            WorkspaceTab::Overview => self.render_overview(&binding, p, cx).into_any_element(),
            WorkspaceTab::Changes => self
                .render_changes(&binding, p, cx, viewport_width, viewport_height)
                .into_any_element(),
            WorkspaceTab::Findings => div().id("findings-empty").flex_1().min_h(px(360.)).p_6().border_1().border_color(rgb(p.border)).bg(rgb(p.panel))
                .child(div().text_size(px(18.)).font_weight(FontWeight::MEDIUM).child("No findings to show"))
                .child(div().mt_2().text_color(rgb(p.muted)).child("No generated review result is attached. Findings never start an agent; review the pinned changes manually.")).into_any_element(),
        };
        page.child(panel)
    }

    fn render_overview(
        &self,
        binding: &PreparedBinding,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut body = div()
            .id("overview-content")
            .flex()
            .flex_col()
            .gap_4()
            .overflow_scroll();
        body = body.child(empty_card("No generated overview", "This prepared review contains pinned code context only. No sequence diagram or analysis has been generated.", p));
        let excerpt = markdown_excerpt(&self.description_blocks, 320);
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.panel))
                .child(section_title("Pull request description", p))
                .child(
                    div()
                        .mt_2()
                        .text_color(rgb(p.muted))
                        .child(if excerpt.is_empty() {
                            "No description was supplied.".into()
                        } else {
                            excerpt
                        }),
                ),
        );
        let mut reader = div()
            .id("description-reader")
            .track_scroll(&self.description_scroll)
            .on_scroll_wheel(cx.listener(|this, _, _, _| this.persist_navigation()))
            .h(px(440.))
            .min_h(px(260.))
            .overflow_scroll()
            .p_4()
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.panel))
            .flex()
            .flex_col()
            .gap_3();
        if self.description_blocks.is_empty() {
            reader = reader.child(
                div()
                    .text_color(rgb(p.muted))
                    .child("No description was supplied."),
            );
        } else {
            for (index, block) in self.description_blocks.iter().enumerate() {
                reader = reader.child(render_markdown_block(block, index, p));
            }
        }
        body = body.child(reader);
        body.child(
            div()
                .text_color(rgb(p.muted))
                .text_size(px(12.))
                .child(format!(
                    "{} changed files · context {}",
                    binding.changed_files.len(),
                    short_sha(&binding.context.context_id)
                )),
        )
    }

    fn render_changes(
        &mut self,
        binding: &PreparedBinding,
        p: Palette,
        cx: &mut Context<Self>,
        viewport_width: f32,
        viewport_height: f32,
    ) -> impl IntoElement {
        let selected = self.selected_file;
        let source_width = self
            .prefs
            .inspector_width
            .clamp(300., (viewport_width - 520.).max(300.));
        let file_width = 220.;
        let pane_height = (viewport_height - 190.).max(360.);
        let filtered = binding
            .changed_files
            .iter()
            .enumerate()
            .filter(|(_, file)| {
                file.new_path
                    .as_deref()
                    .or(file.old_path.as_deref())
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains(&self.file_search.to_ascii_lowercase())
                    || file
                        .status
                        .to_ascii_lowercase()
                        .contains(&self.file_search.to_ascii_lowercase())
            })
            .collect::<Vec<_>>();
        let file_rows = filtered
            .iter()
            .map(|(index, file)| {
                let name = file
                    .new_path
                    .as_deref()
                    .or(file.old_path.as_deref())
                    .unwrap_or("(unknown path)");
                let label = if file.old_path.as_deref().is_some_and(|old| old != name) {
                    format!(
                        "{} → {}  {}",
                        file.old_path.as_deref().unwrap(),
                        name,
                        file.status
                    )
                } else {
                    format!("{name}  {}", file.status)
                };
                (*index, label)
            })
            .collect::<Vec<_>>();
        let current_file = selected;
        let owner = cx.entity().downgrade();
        let virtual_files = uniform_list(
            "changed-file-rows",
            file_rows.len(),
            move |range, _window, _cx| {
                range
                    .filter_map(|row| file_rows.get(row))
                    .map(|(file_index, label)| {
                        let file_index = *file_index;
                        let label = label.clone();
                        let owner = owner.clone();
                        div()
                            .id(file_index)
                            .h(px(34.))
                            .w_full()
                            .px_2()
                            .border_1()
                            .border_color(rgb(p.border))
                            .bg(rgb(if current_file == Some(file_index) {
                                p.raised
                            } else {
                                p.panel
                            }))
                            .text_color(rgb(if current_file == Some(file_index) {
                                p.text
                            } else {
                                p.muted
                            }))
                            .text_size(px(12.))
                            .overflow_hidden()
                            .cursor_pointer()
                            .child(label)
                            .on_click(move |_, window, app| {
                                let _ = owner.update(app, |this, cx| {
                                    window.focus(&this.focus);
                                    this.select_file(file_index, cx);
                                });
                            })
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(self.file_scroll.clone())
        .on_scroll_wheel(cx.listener(|this, _, _, _| this.persist_navigation()))
        .flex_1()
        .min_h_0();
        let file_list = div()
            .id("change-files")
            .w(px(file_width))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgb(p.border))
            .p_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(section_title("Changed files", p))
            .child(
                div()
                    .id("file-search")
                    .track_focus(&self.file_search_focus)
                    .p_2()
                    .border_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.raised))
                    .focus(|style| style.border_color(rgb(p.accent)))
                    .text_size(px(12.))
                    .child(if self.file_search.is_empty() {
                        "Filter files".into()
                    } else {
                        self.file_search.clone()
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, _| window.focus(&this.file_search_focus)),
                    )
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        match event.keystroke.key.as_str() {
                            "backspace" => {
                                this.file_search.pop();
                            }
                            "escape" => this.file_search.clear(),
                            key if key.chars().count() == 1
                                && !event.keystroke.modifiers.control
                                && !event.keystroke.modifiers.platform =>
                            {
                                this.file_search.push_str(key)
                            }
                            _ => return,
                        }
                        this.file_scroll
                            .0
                            .borrow()
                            .base_handle
                            .set_offset(point(px(0.), px(0.)));
                        cx.notify();
                    })),
            )
            .child(virtual_files);

        let mut diff = div()
            .id("review-diff")
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgb(p.border))
            .flex()
            .flex_col();
        let selected_path = selected
            .and_then(|index| binding.changed_files.get(index))
            .map(|file| {
                file.new_path
                    .as_deref()
                    .or(file.old_path.as_deref())
                    .unwrap_or("")
            })
            .unwrap_or("");
        let mut diff_toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .p_2()
            .border_b_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.))
                    .child(selected_path.to_owned()),
            );
        for (label, split) in [("Unified", false), ("Split", true)] {
            diff_toolbar = diff_toolbar.child(
                button(
                    &format!("diff-mode-{label}"),
                    label,
                    p,
                    move |this, _, _, cx| {
                        this.split_diff = split;
                        this.persist_navigation();
                        cx.notify();
                    },
                    cx,
                )
                .text_size(px(12.))
                .text_color(rgb(if self.split_diff == split {
                    p.text
                } else {
                    p.muted
                })),
            );
        }
        diff = diff.child(diff_toolbar);
        if self.diff_loading {
            diff = diff.child(empty_card(
                "Loading pinned diff",
                "The selected file patch is read from the prepared base and head commits.",
                p,
            ));
        } else if let Some(message) = self.diff_error.clone() {
            diff = diff.child(empty_card("Diff unavailable", &message, p));
        } else if let Some(page) = &self.diff_page {
            let truncated = page.truncated;
            let mut rows_view = div()
                .id("diff-rows")
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .flex()
                .flex_col();
            if truncated {
                rows_view = rows_view.child(
                    div()
                        .p_2()
                        .text_color(rgb(p.muted))
                        .child("Patch is truncated at the 128 KiB display limit."),
                );
            }
            let extension = path_extension(selected_path).to_owned();
            let owner = cx.entity().downgrade();
            let virtual_rows = if self.split_diff {
                let rows = split_diff_rows(&self.diff_rows);
                uniform_list(
                    "split-diff-virtual-list",
                    rows.len(),
                    move |range, _window, _cx| {
                        range
                            .filter_map(|index| rows.get(index).map(|row| (index, row)))
                            .map(|(index, row)| {
                                render_split_diff_row(
                                    index,
                                    row,
                                    selected.unwrap_or(0),
                                    &extension,
                                    p,
                                    &owner,
                                )
                            })
                            .collect::<Vec<_>>()
                    },
                )
            } else {
                let rows = self.diff_rows.clone();
                let fallback = self.source_revision;
                uniform_list(
                    "unified-diff-virtual-list",
                    rows.len(),
                    move |range, _window, _cx| {
                        range
                            .filter_map(|index| rows.get(index).map(|row| (index, row)))
                            .map(|(index, row)| {
                                render_unified_diff_row(
                                    index,
                                    row,
                                    selected.unwrap_or(0),
                                    fallback,
                                    &extension,
                                    p,
                                    &owner,
                                )
                            })
                            .collect::<Vec<_>>()
                    },
                )
            }
            .track_scroll(self.diff_scroll.clone())
            .on_scroll_wheel(cx.listener(|this, _, _, _| this.persist_navigation()))
            .flex_1()
            .min_h_0();
            rows_view = rows_view.child(virtual_rows);
            diff = diff.child(rows_view);
        } else {
            diff = diff.child(empty_card(
                "Select a changed file",
                "The unified patch and exact base/head line numbers appear here.",
                p,
            ));
        }

        let mut source = div()
            .id("pinned-source")
            .track_scroll(&self.source_scroll)
            .on_scroll_wheel(cx.listener(|this, _, _, _| this.persist_navigation()))
            .overflow_scroll()
            .w(px(source_width))
            .flex_shrink_0()
            .min_w_0()
            .h_full()
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgb(p.border))
            .p_3()
            .flex()
            .flex_col();
        let file = selected.and_then(|index| binding.changed_files.get(index));
        let has_base = file.is_some_and(|f| f.old_path.is_some());
        let has_head = file.is_some_and(|f| f.new_path.is_some());
        let mut source_toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .child(section_title("Pinned source", p));
        for (label, revision, enabled) in [
            ("Base", Revision::Base, has_base),
            ("Head", Revision::Head, has_head),
        ] {
            if enabled {
                source_toolbar = source_toolbar.child(
                    button(
                        &format!("source-side-{label}"),
                        label,
                        p,
                        move |this, _, _, cx| {
                            let index = this.selected_file.unwrap_or(0);
                            let line = this
                                .source_ref
                                .as_ref()
                                .map(|source| source.start_line)
                                .unwrap_or(1);
                            this.select_source_line(index, revision, line, cx);
                        },
                        cx,
                    )
                    .text_size(px(12.))
                    .text_color(rgb(if self.source_revision == revision {
                        p.text
                    } else {
                        p.muted
                    })),
                );
            }
        }
        if self.source_ref.is_some() {
            source_toolbar = source_toolbar.child(button(
                "new-inline-comment-draft",
                "Draft comment",
                p,
                |this, _, _, cx| this.begin_text_draft(cx),
                cx,
            ));
        }
        source = source.child(source_toolbar);
        if let Some(label) = self.source_label.clone() {
            source = source.child(
                div()
                    .mt_2()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(label),
            );
        } else {
            source = source.child(
                div()
                    .mt_2()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child("Select a diff line to inspect its exact pinned blob line."),
            );
        }
        if self.source_loading {
            source = source.child(
                div()
                    .mt_3()
                    .text_color(rgb(p.muted))
                    .child("Loading source from the pinned commit…"),
            );
        } else if let (Some(text), Some(source_ref)) = (&self.source_text, &self.source_ref) {
            let extension = path_extension(&source_ref.path);
            let mut source_rows = div().mt_3().flex().flex_col().gap_0();
            for (offset, line) in text.lines().enumerate() {
                let number = source_ref.start_line.saturating_add(offset as u32);
                source_rows = source_rows.child(render_source_row(number, line, extension, p));
            }
            source = source.child(source_rows);
        }
        source = source.child(self.render_draft_panel(p, cx));
        div()
            .id("changes-panes")
            .flex()
            .gap_2()
            .h(px(pane_height))
            .min_h(px(360.))
            .child(file_list)
            .child(diff)
            .child(
                div()
                    .id("changes-divider")
                    .w(px(8.))
                    .h_full()
                    .flex_shrink_0()
                    .bg(rgb(p.border))
                    .cursor_col_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.dragging_pane = true),
                    ),
            )
            .child(source)
    }

    fn render_draft_panel(&mut self, p: Palette, cx: &mut Context<Self>) -> Stateful<Div> {
        let counts = self
            .draft_snapshot
            .as_ref()
            .map(|snapshot| snapshot.counts.inline_comments)
            .unwrap_or(0);
        let mut panel = div()
            .id("manual-draft-panel")
            .mt_4()
            .p_3()
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.raised))
            .flex()
            .flex_col()
            .gap_2()
            .child(section_title(
                &format!("Local comment drafts · {counts}"),
                p,
            ));

        if let Some(editor) = self.draft_editor.clone() {
            let anchor = format!(
                "{} · {}:{}–{}",
                editor.source.path,
                match editor.source.revision {
                    Revision::Base => "Base",
                    Revision::Head => "Head",
                },
                editor.source.start_line,
                editor.source.end_line
            );
            panel = panel.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(anchor),
            );
            let display = if editor.body.is_empty() {
                "|Type a review comment. Enter inserts a new line; Ctrl+A replaces all text."
                    .to_owned()
            } else {
                editor.display_text_with_cursor()
            };
            panel = panel.child(
                div()
                    .id("manual-comment-editor")
                    .track_focus(&self.draft_focus)
                    .min_h(px(92.))
                    .max_h(px(180.))
                    .overflow_scroll()
                    .p_2()
                    .border_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.panel))
                    .focus(|style| style.border_color(rgb(p.accent)))
                    .text_size(px(12.))
                    .text_color(rgb(p.text))
                    .child(display)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, _| {
                            window.focus(&this.draft_focus);
                            if let Some(editor) = &mut this.draft_editor {
                                editor.move_cursor_to_edge(true);
                            }
                        }),
                    )
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        window.focus(&this.draft_focus);
                        let key = event.keystroke.key.as_str();
                        let command =
                            event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
                        let Some(editor) = &mut this.draft_editor else {
                            return;
                        };
                        match key {
                            "backspace" => editor.backspace(),
                            "delete" => editor.delete_forward(),
                            "left" => editor.move_cursor(-1),
                            "right" => editor.move_cursor(1),
                            "home" => editor.move_cursor_to_edge(false),
                            "end" => editor.move_cursor_to_edge(true),
                            "enter" => editor.insert_text("\n"),
                            "tab" => editor.insert_text("    "),
                            "a" if command => editor.select_all(),
                            text if text.chars().count() == 1 && !command => {
                                editor.insert_text(text)
                            }
                            _ => return,
                        }
                        cx.notify();
                    })),
            );
            if let Some(error) = editor.error.as_deref() {
                panel = panel.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(
                            if editor.error_code.as_deref().is_some_and(|code| {
                                code == "WORKSPACE_REVISION_STALE" || code == "DRAFT_REVISION_STALE"
                            }) {
                                0xffb7b7
                            } else {
                                p.muted
                            },
                        ))
                        .child(error.to_owned()),
                );
            }
            let mut actions = div().flex().items_center().gap_2();
            actions = actions.child(button(
                "save-local-comment-draft",
                if self.draft_saving {
                    "Saving…"
                } else {
                    "Save locally"
                },
                p,
                |this, _, _, cx| this.save_text_draft(cx),
                cx,
            ));
            actions = actions.child(button(
                "discard-local-comment-draft",
                if editor.draft_id.is_some() {
                    "Discard draft"
                } else {
                    "Clear text"
                },
                p,
                |this, _, _, cx| this.discard_text_draft(cx),
                cx,
            ));
            if editor.error_code.as_deref().is_some_and(|code| {
                code == "WORKSPACE_REVISION_STALE" || code == "DRAFT_REVISION_STALE"
            }) {
                actions = actions.child(button(
                    "recover-as-new-draft",
                    "Recover as new draft",
                    p,
                    |this, _, _, cx| this.recover_draft_as_new(cx),
                    cx,
                ));
            }
            panel = panel.child(actions);
        } else if self.source_ref.is_some() {
            panel = panel.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child("Select a diff line, then choose Draft comment to add a local note."),
            );
        }

        if let Some(snapshot) = self.draft_snapshot.clone()
            && !snapshot.drafts.is_empty()
        {
            let mut saved = div().mt_2().flex().flex_col().gap_1();
            for draft in snapshot.drafts {
                let id = draft.id.clone();
                let label = format!(
                    "Resume · {}:{} · {}",
                    draft.source.path,
                    draft.source.start_line,
                    draft.body.lines().next().unwrap_or_default()
                );
                saved = saved.child(button(
                    &format!("resume-draft-{id}"),
                    &label,
                    p,
                    move |this, _, _, cx| this.resume_text_draft(&id, cx),
                    cx,
                ));
            }
            panel = panel.child(saved);
        }
        panel
    }

    fn notice(&self, p: Palette, text: &str, danger: bool) -> impl IntoElement {
        div()
            .px_6()
            .py_3()
            .bg(rgb(if danger {
                if self.prefs.dark { 0x3b2428 } else { 0xffeded }
            } else {
                p.raised
            }))
            .text_color(rgb(if danger {
                if self.prefs.dark { 0xffb7b7 } else { 0x8c2525 }
            } else {
                p.text
            }))
            .child(text.to_owned())
    }
}

fn source_title(source: &SourceRef) -> String {
    let side = match source.revision {
        Revision::Base => "Base",
        Revision::Head => "Head",
    };
    if source.start_line == source.end_line {
        format!("{side} · {}:{}", source.path, source.start_line)
    } else {
        format!(
            "{side} · {}:{}–{}",
            source.path, source.start_line, source.end_line
        )
    }
}

fn pending_draft_count(counts: &bodh::DraftCounts) -> u32 {
    counts
        .inline_comments
        .saturating_add(counts.replies)
        .saturating_add(counts.thread_intents)
        .saturating_add(counts.review_comments)
        .min(u64::from(u32::MAX)) as u32
}

fn path_extension(path: &str) -> &str {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
}

fn markdown_excerpt(blocks: &[MarkdownBlock], limit: usize) -> String {
    let text = blocks
        .iter()
        .filter(|block| block.kind != MarkdownBlockKind::PlainHtml)
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.chars().count() <= limit {
        text
    } else {
        format!("{}…", text.chars().take(limit).collect::<String>())
    }
}

fn render_markdown_block(block: &MarkdownBlock, index: usize, p: Palette) -> Stateful<Div> {
    let base = div().id(("description-block", index));
    match &block.kind {
        MarkdownBlockKind::Heading(level) => base
            .text_size(px((22. - f32::from(*level) * 1.3).max(15.)))
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(p.text))
            .child(block.text.clone()),
        MarkdownBlockKind::Paragraph => base.text_color(rgb(p.text)).child(block.text.clone()),
        MarkdownBlockKind::Quote => base
            .pl_3()
            .border_l_2()
            .border_color(rgb(p.accent))
            .text_color(rgb(p.muted))
            .child(block.text.clone()),
        MarkdownBlockKind::ListItem => base
            .pl_3()
            .text_color(rgb(p.text))
            .child(block.text.clone()),
        MarkdownBlockKind::Code(language) => {
            let mut content = div()
                .p_3()
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.raised))
                .font_family("DejaVu Sans Mono")
                .text_size(px(12.));
            if !language.is_empty() {
                content = content.child(
                    div()
                        .mb_2()
                        .text_size(px(10.))
                        .text_color(rgb(p.muted))
                        .child(language.clone()),
                );
            }
            content = content.child(code_text(&block.text, language, p));
            base.child(content)
        }
        MarkdownBlockKind::Table => base
            .font_family("DejaVu Sans Mono")
            .text_size(px(12.))
            .text_color(rgb(p.text))
            .child(block.text.clone()),
        MarkdownBlockKind::PlainHtml => base
            .p_2()
            .bg(rgb(p.raised))
            .font_family("DejaVu Sans Mono")
            .text_size(px(11.))
            .text_color(rgb(p.muted))
            .child(block.text.clone()),
        MarkdownBlockKind::Rule => base.text_color(rgb(p.border)).child(block.text.clone()),
    }
}

fn code_text(text: &str, language: &str, p: Palette) -> StyledText {
    let highlights = syntax_spans(text, language)
        .into_iter()
        .map(|span| {
            let color = match span.kind {
                bodh_sequence_spike::SyntaxTokenKind::Keyword => 0x8ccecb,
                bodh_sequence_spike::SyntaxTokenKind::String => 0xa6c98c,
                bodh_sequence_spike::SyntaxTokenKind::Comment => 0x8391a5,
                bodh_sequence_spike::SyntaxTokenKind::Number => 0xe5b37a,
            };
            (
                span.start..span.end,
                HighlightStyle::color(rgb(color).into()),
            )
        })
        .collect::<Vec<_>>();
    let _ = p;
    StyledText::new(text.to_owned()).with_highlights(highlights)
}

fn render_diff_gutter(row: &DiffLine) -> String {
    let old = row
        .old_line
        .map(|line| line.to_string())
        .unwrap_or_default();
    let new = row
        .new_line
        .map(|line| line.to_string())
        .unwrap_or_default();
    format!("{old:>5} {new:>5}  ")
}

fn diff_row_color(kind: DiffLineKind, p: Palette) -> (u32, u32) {
    match kind {
        DiffLineKind::Addition => (
            if p.canvas == CANVAS_DARK {
                0x20352c
            } else {
                0xe9f5eb
            },
            if p.canvas == CANVAS_DARK {
                0xb5e3bf
            } else {
                0x245331
            },
        ),
        DiffLineKind::Deletion => (
            if p.canvas == CANVAS_DARK {
                0x3b292d
            } else {
                0xfbeaea
            },
            if p.canvas == CANVAS_DARK {
                0xffc1c3
            } else {
                0x7e292c
            },
        ),
        DiffLineKind::Hunk => (p.raised, p.accent),
        DiffLineKind::Header => (p.panel, p.muted),
        DiffLineKind::Context => (p.panel, p.text),
        DiffLineKind::Meta => (p.panel, p.muted),
    }
}

fn row_source(row: &DiffLine, fallback: Revision) -> Option<(Revision, u32)> {
    match row.kind {
        DiffLineKind::Addition => row.new_line.map(|line| (Revision::Head, line)),
        DiffLineKind::Deletion => row.old_line.map(|line| (Revision::Base, line)),
        DiffLineKind::Context => match fallback {
            Revision::Base => row.old_line.map(|line| (Revision::Base, line)),
            Revision::Head => row.new_line.map(|line| (Revision::Head, line)),
        },
        _ => None,
    }
}

fn render_unified_diff_row(
    index: usize,
    row: &DiffLine,
    file_index: usize,
    fallback: Revision,
    language: &str,
    p: Palette,
    owner: &WeakEntity<Product>,
) -> Stateful<Div> {
    let (background, foreground) = diff_row_color(row.kind, p);
    let content = if matches!(
        row.kind,
        DiffLineKind::Addition | DiffLineKind::Deletion | DiffLineKind::Context
    ) {
        code_text(&row.content, language, p)
    } else {
        StyledText::new(row.content.clone())
    };
    let mut element = div()
        .id(("diff-row", index))
        .h(px(24.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .whitespace_nowrap()
        .bg(rgb(background))
        .text_color(rgb(foreground))
        .font_family("DejaVu Sans Mono")
        .text_size(px(11.))
        .child(
            div()
                .w(px(90.))
                .flex_shrink_0()
                .text_color(rgb(p.muted))
                .child(render_diff_gutter(row)),
        )
        .child(content);
    if let Some((revision, line)) = row_source(row, fallback) {
        let owner = owner.clone();
        element = element.cursor_pointer().on_click(move |_, window, app| {
            let _ = owner.update(app, |this, cx| {
                window.focus(&this.focus);
                this.select_source_line(file_index, revision, line, cx);
            });
        });
    }
    element
}

fn render_split_diff_row(
    index: usize,
    row: &bodh_sequence_spike::SplitDiffRow,
    file_index: usize,
    language: &str,
    p: Palette,
    owner: &WeakEntity<Product>,
) -> Stateful<Div> {
    let left_cell = render_split_cell(
        index * 2,
        row.left.as_ref(),
        file_index,
        Revision::Base,
        language,
        p,
        owner,
    );
    let right_cell = render_split_cell(
        index * 2 + 1,
        row.right.as_ref(),
        file_index,
        Revision::Head,
        language,
        p,
        owner,
    );
    div()
        .id(("split-diff-row", index))
        .h(px(24.))
        .flex_shrink_0()
        .flex()
        .child(left_cell)
        .child(right_cell)
}

fn render_split_cell(
    id: usize,
    row: Option<&DiffLine>,
    file_index: usize,
    revision: Revision,
    language: &str,
    p: Palette,
    owner: &WeakEntity<Product>,
) -> Stateful<Div> {
    let Some(row) = row else {
        return div().id(("split-empty", id)).flex_1().h(px(24.));
    };
    let (background, foreground) = diff_row_color(row.kind, p);
    let line = match revision {
        Revision::Base => row.old_line,
        Revision::Head => row.new_line,
    };
    let number = line.map(|line| line.to_string()).unwrap_or_default();
    let content = if matches!(
        row.kind,
        DiffLineKind::Addition | DiffLineKind::Deletion | DiffLineKind::Context
    ) {
        code_text(&row.content, language, p)
    } else {
        StyledText::new(row.content.clone())
    };
    let mut element = div()
        .id(("split-diff-cell", id))
        .flex_1()
        .min_w_0()
        .h(px(24.))
        .flex()
        .items_center()
        .whitespace_nowrap()
        .overflow_hidden()
        .bg(rgb(background))
        .text_color(rgb(foreground))
        .font_family("DejaVu Sans Mono")
        .text_size(px(11.))
        .child(
            div()
                .w(px(42.))
                .flex_shrink_0()
                .text_color(rgb(p.muted))
                .child(number),
        )
        .child(content);
    if let Some(line) = line
        && matches!(
            row.kind,
            DiffLineKind::Addition | DiffLineKind::Deletion | DiffLineKind::Context
        )
    {
        let owner = owner.clone();
        element = element.cursor_pointer().on_click(move |_, window, app| {
            let _ = owner.update(app, |this, cx| {
                window.focus(&this.focus);
                this.select_source_line(file_index, revision, line, cx);
            });
        });
    }
    element
}

fn render_source_row(line_number: u32, text: &str, language: &str, p: Palette) -> Stateful<Div> {
    div()
        .id(("source-line", line_number))
        .h(px(22.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .whitespace_nowrap()
        .font_family("DejaVu Sans Mono")
        .text_size(px(11.))
        .child(
            div()
                .w(px(42.))
                .flex_shrink_0()
                .text_right()
                .mr_3()
                .text_color(rgb(p.muted))
                .child(line_number.to_string()),
        )
        .child(code_text(text, language, p))
}

fn button(
    id: &str,
    label: &str,
    p: Palette,
    action: impl Fn(&mut Product, &ClickEvent, &mut Window, &mut Context<Product>) + 'static,
    cx: &mut Context<Product>,
) -> Stateful<Div> {
    div()
        .id(ElementId::Name(SharedString::from(id.to_owned())))
        .px_3()
        .py_2()
        .border_1()
        .border_color(rgb(p.border))
        .bg(rgb(p.panel))
        .text_color(rgb(p.text))
        .cursor_pointer()
        .focus(|style| style.border_color(rgb(p.accent)))
        .child(label.to_owned())
        .on_click(cx.listener(action))
}
fn section_title(label: &str, p: Palette) -> Div {
    div()
        .text_size(px(15.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(p.text))
        .child(label.to_owned())
}
fn empty_card(title: &str, description: &str, p: Palette) -> Div {
    div()
        .p_4()
        .border_1()
        .border_color(rgb(p.border))
        .bg(rgb(p.panel))
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .child(title.to_owned()),
        )
        .child(
            div()
                .mt_1()
                .text_color(rgb(p.muted))
                .child(description.to_owned()),
        )
}

fn state_paths() -> (PathBuf, PathBuf) {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir);
    let app = base.join("bodh");
    (
        app.join("native-preferences.json"),
        app.join("reviews.sqlite"),
    )
}
fn home_directory() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}
fn gh_install_prompt() -> String {
    "GitHub CLI is required. Install it from https://cli.github.com/ and restart Bodh.".into()
}
fn list_pull_requests(
    repository: &Path,
    control: &OperationControl,
) -> Result<Vec<PullRequest>, String> {
    list_pull_requests_with_gh(repository, control, Path::new("gh"), &home_directory())
}
fn list_pull_requests_with_gh(
    repository: &Path,
    control: &OperationControl,
    gh: &Path,
    home: &Path,
) -> Result<Vec<PullRequest>, String> {
    let repository = resolve_repository_path(&repository.to_string_lossy(), home)
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let output = run_bounded(
        Command::new(gh)
            .args([
                "pr",
                "list",
                "--state",
                "all",
                "--limit",
                "100",
                "--json",
                "number,title,author,state",
            ])
            .current_dir(&repository),
        control,
    )
    .map_err(|error| format!("{}: {}", error.code, error.message))?;
    serde_json::from_slice(&output)
        .map_err(|_| "GitHub CLI returned an invalid pull-request list.".into())
}
fn run_bounded(
    command: &mut Command,
    control: &OperationControl,
) -> Result<Vec<u8>, bodh::ContractError> {
    use std::io::Read;
    if command
        .get_current_dir()
        .is_some_and(|directory| !directory.is_dir())
    {
        return Err(bodh::ContractError::new(
            "REPOSITORY_PATH_INVALID",
            "The checkout directory cannot be opened.",
            "",
        ));
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                bodh::ContractError::new(
                    "GH_UNAVAILABLE",
                    "GitHub CLI was not found on Bodh's PATH",
                    "",
                )
            } else {
                bodh::ContractError::new(
                    "GH_START_FAILED",
                    "GitHub CLI could not be started for this repository",
                    "",
                )
            }
        })?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes);
        bytes
    });
    let err = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.take(64 * 1024 + 1).read_to_end(&mut bytes);
        bytes
    });
    loop {
        if let Err(error) = control.check() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = out.join().unwrap_or_default();
                let error = err.join().unwrap_or_default();
                if !status.success() {
                    let detail = String::from_utf8_lossy(&error).to_ascii_lowercase();
                    let (code, message) = if detail.contains("auth")
                        || detail.contains("401")
                        || detail.contains("login")
                    {
                        ("GH_AUTH_REQUIRED", "GitHub CLI authentication is required")
                    } else if detail.contains("404")
                        || detail.contains("not found")
                        || detail.contains("denied")
                    {
                        (
                            "GH_ACCESS_DENIED",
                            "GitHub CLI cannot access this repository",
                        )
                    } else {
                        (
                            "GH_REQUEST_FAILED",
                            "GitHub CLI could not list pull requests",
                        )
                    };
                    return Err(bodh::ContractError::new(code, message, ""));
                }
                if output.len() > 8 * 1024 * 1024 {
                    return Err(bodh::ContractError::new(
                        "GH_RESPONSE_TOO_LARGE",
                        "Pull request list exceeds the supported bound",
                        "",
                    ));
                }
                return Ok(output);
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(_) => {
                return Err(bodh::ContractError::new(
                    "GH_REQUEST_FAILED",
                    "Could not inspect GitHub CLI process",
                    "",
                ));
            }
        }
    }
}
fn filter_pull_requests<'a>(
    rows: &'a [PullRequest],
    query: &str,
    lifecycle: &str,
) -> Vec<&'a PullRequest> {
    rows.iter()
        .filter(|pr| {
            bodh_sequence_spike::matches_pull_request(
                pr.number,
                &pr.title,
                &pr.author.login,
                &pr.state,
                query,
                lifecycle,
            )
        })
        .collect()
}
fn short_sha(value: &str) -> &str {
    value.get(..value.len().min(12)).unwrap_or(value)
}

pub fn open_product(cx: &mut App, repo: Option<String>, pr: Option<u64>) {
    let (prefs, database) = state_paths();
    let initial_repo = repo.clone();
    let initial_pr = pr;
    cx.open_window(
        WindowOptions {
            app_id: Some("bodh-review-workspace".into()),
            focus: true,
            show: true,
            titlebar: Some(TitlebarOptions {
                title: Some("Bodh · Review workspace".into()),
                ..Default::default()
            }),
            window_min_size: Some(size(px(760.), px(560.))),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1280.), px(820.)),
                cx,
            ))),
            ..Default::default()
        },
        move |window, cx| {
            cx.new(|cx| {
                let product = Product::new(
                    prefs.clone(),
                    database.clone(),
                    initial_repo.clone(),
                    initial_pr,
                    cx,
                );
                window.focus(&product.focus);
                product
            })
        },
    )
    .expect("open Bodh native review workspace");
    cx.activate(true);
}

#[cfg(test)]
mod gh_prerequisite_tests {
    use super::list_pull_requests_with_gh;
    use bodh::OperationControl;
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "bodh-gh-list-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn tilde_repository_is_expanded_before_the_pr_list_child_is_spawned() {
        let root = temp_dir();
        fs::create_dir_all(root.join("dev/nayati")).unwrap();
        let gh = root.join("gh");
        fs::write(&gh, "#!/bin/sh\nprintf '[]\\n'\n").unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();

        let result = list_pull_requests_with_gh(
            Path::new("~/dev/nayati"),
            &OperationControl::default(),
            &gh,
            &root,
        )
        .unwrap();
        assert!(result.is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
