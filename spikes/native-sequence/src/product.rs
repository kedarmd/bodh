//! Native product workspace. GitHub acquisition and pinned-source rules stay in `bodh`.
use super::diagram::SequenceDiagram;
use super::diagram_adapter::{AdaptedDiagram, ChangedLines, adapt_diagram};
use bodh::{
    AnchorRepairPreview, AnchorRepairStatus, CodeReplacementSnapshot, DiscussionPlan,
    DiscussionPublicationStore, DiscussionSelection, DraftSnapshot, DraftStore,
    GeneratedReviewStore, GhDiscussionPublisher, GhReviewPublisher, OperationControl, PinnedDiff,
    PreparationManifest, PreparationStore, PreparedBinding, ReviewEvent, ReviewIntent,
    ReviewOperation, ReviewPublicationStore, Revision, SelectedDraft, SourceRef,
    StageReviewRequest, inspect_remote_review_with_gh, read_diff,
};
use bodh_sequence_spike::gh_prerequisite::{GhCliStatus, probe_gh_cli, resolve_repository_path};
use bodh_sequence_spike::overview::{
    CommittedOverviewStatus, CommittedOverviewView, FollowUpProminence,
    apply_committed_overview_load, committed_matches_context, evidence_file_index,
    follow_up_concerns, is_significant_assessment,
};
use bodh_sequence_spike::preferences::{
    NavigationPreferences as Preferences, RecentReview, WorkspaceTab,
};
use bodh_sequence_spike::{
    CodeReplacementEditor, DiffLine, DiffLineKind, MarkdownBlock, MarkdownBlockKind,
    TextDraftEditor, diff_rows, safe_markdown_blocks, split_diff_rows, syntax_spans,
};
use gpui::{UniformListScrollHandle, prelude::*, uniform_list, *};
use serde::Deserialize;
use serde_json::Value;
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
    FinishReview,
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

#[derive(Clone, Debug)]
struct PriorSourcePreview {
    key: String,
    loading: bool,
    text: Option<String>,
    error: Option<String>,
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
    selection: u32,
    danger: u32,
    caution: u32,
    success: u32,
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
                selection: 0x303a4a,
                danger: 0xf0a5a5,
                caution: 0xedc680,
                success: 0x9fceb4,
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
                selection: 0xe7edf7,
                danger: 0xa33c49,
                caution: 0x77530f,
                success: 0x266244,
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
    source_display_ref: Option<SourceRef>,
    description_blocks: Vec<MarkdownBlock>,
    draft_snapshot: Option<DraftSnapshot>,
    draft_editor: Option<TextDraftEditor>,
    replacement_snapshot: Option<CodeReplacementSnapshot>,
    replacement_editor: Option<CodeReplacementEditor>,
    draft_saving: bool,
    draft_epoch: Arc<AtomicU64>,
    draft_focus: FocusHandle,
    discussion_state: serde_json::Value,
    discussion_loading: bool,
    discussion_expanded: bool,
    discussion_loaded_for: Option<String>,
    reply_thread: Option<String>,
    reply_root: Option<String>,
    reply_draft_id: Option<String>,
    reply_draft_revision: u64,
    reply_body: String,
    reply_focus: FocusHandle,
    reply_saving: bool,
    dragging_pane: bool,
    review_intent: Option<ReviewIntent>,
    review_summary: String,
    review_plan: Option<ReviewOperation>,
    discussion_plan: Option<DiscussionPlan>,
    review_loading: bool,
    review_summary_focus: FocusHandle,
    freshness_observation: Option<bodh::FreshnessObservation>,
    latest_switch_preview: Option<bodh::LatestSwitchPreview>,
    freshness_loading: bool,
    last_freshness_check: Option<std::time::Instant>,
    anchor_repair_source_id: Option<String>,
    anchor_repair_previews: Vec<AnchorRepairPreview>,
    anchor_repair_loading: bool,
    committed_overview: CommittedOverviewView,
    overview_loading: bool,
    overview_epoch: Arc<AtomicU64>,
    finding_triage_snapshot: Option<bodh::FindingTriageSnapshot>,
    finding_triage_loading: bool,
    finding_action_saving: bool,
    finding_feedback_target: Option<String>,
    finding_feedback_body: String,
    finding_feedback_focus: FocusHandle,
    focused_discussion_thread: Option<String>,
    pending_overview_source: Option<(usize, SourceRef)>,
    diagram_views: Vec<AdaptedDiagram>,
    diagram_changed_lines: ChangedLines,
    diagram_diff_context: Option<String>,
    diagram_diff_loading: bool,
    diagram_selected: std::collections::HashMap<String, usize>,
    diagram_scrolls: std::collections::HashMap<String, ScrollHandle>,
    active_diagram: Option<String>,
    diagram_source: Option<SourceRef>,
    diagram_source_display: Option<SourceRef>,
    diagram_source_text: Option<String>,
    diagram_source_loading: bool,
    diagram_source_error: Option<String>,
    diagram_source_scroll: ScrollHandle,
    diagram_epoch: Arc<AtomicU64>,
    diagram_source_epoch: Arc<AtomicU64>,
    diagram_diff_control: Option<OperationControl>,
    diagram_source_control: Option<OperationControl>,
    selected_finding_id: Option<String>,
    finding_source: Option<SourceRef>,
    finding_source_display: Option<SourceRef>,
    finding_source_text: Option<String>,
    finding_source_loading: bool,
    finding_source_error: Option<String>,
    finding_source_scroll: ScrollHandle,
    finding_source_epoch: Arc<AtomicU64>,
    finding_source_control: Option<OperationControl>,
    prior_source_preview: Option<PriorSourcePreview>,
    finding_prior_source: Option<bodh::PriorConcernEvidence>,
    prior_source_epoch: Arc<AtomicU64>,
    expanded_follow_up: std::collections::HashSet<String>,
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
            source_display_ref: None,
            description_blocks: Vec::new(),
            draft_snapshot: None,
            draft_editor: None,
            replacement_snapshot: None,
            replacement_editor: None,
            draft_saving: false,
            draft_epoch: Arc::new(AtomicU64::new(0)),
            draft_focus: cx.focus_handle(),
            discussion_state: serde_json::json!({"available":false,"threads":[]}),
            discussion_loading: false,
            discussion_expanded: false,
            discussion_loaded_for: None,
            reply_thread: None,
            reply_root: None,
            reply_draft_id: None,
            reply_draft_revision: 0,
            reply_body: String::new(),
            reply_focus: cx.focus_handle(),
            reply_saving: false,
            dragging_pane: false,
            review_intent: None,
            review_summary: String::new(),
            review_plan: None,
            discussion_plan: None,
            review_loading: false,
            review_summary_focus: cx.focus_handle(),
            freshness_observation: None,
            latest_switch_preview: None,
            freshness_loading: false,
            last_freshness_check: None,
            anchor_repair_source_id: None,
            anchor_repair_previews: Vec::new(),
            anchor_repair_loading: false,
            committed_overview: CommittedOverviewView::default(),
            overview_loading: false,
            overview_epoch: Arc::new(AtomicU64::new(0)),
            finding_triage_snapshot: None,
            finding_triage_loading: false,
            finding_action_saving: false,
            finding_feedback_target: None,
            finding_feedback_body: String::new(),
            finding_feedback_focus: cx.focus_handle(),
            focused_discussion_thread: None,
            pending_overview_source: None,
            diagram_views: Vec::new(),
            diagram_changed_lines: ChangedLines::new(),
            diagram_diff_context: None,
            diagram_diff_loading: false,
            diagram_selected: std::collections::HashMap::new(),
            diagram_scrolls: std::collections::HashMap::new(),
            active_diagram: None,
            diagram_source: None,
            diagram_source_display: None,
            diagram_source_text: None,
            diagram_source_loading: false,
            diagram_source_error: None,
            diagram_source_scroll: ScrollHandle::new(),
            diagram_epoch: Arc::new(AtomicU64::new(0)),
            diagram_source_epoch: Arc::new(AtomicU64::new(0)),
            diagram_diff_control: None,
            diagram_source_control: None,
            selected_finding_id: None,
            finding_source: None,
            finding_source_display: None,
            finding_source_text: None,
            finding_source_loading: false,
            finding_source_error: None,
            finding_source_scroll: ScrollHandle::new(),
            finding_source_epoch: Arc::new(AtomicU64::new(0)),
            finding_source_control: None,
            prior_source_preview: None,
            finding_prior_source: None,
            prior_source_epoch: Arc::new(AtomicU64::new(0)),
            expanded_follow_up: std::collections::HashSet::new(),
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
            || self
                .replacement_editor
                .as_ref()
                .is_some_and(CodeReplacementEditor::is_dirty)
            || (!self.reply_body.is_empty() && !self.reply_saving)
            || (!self.reply_body.is_empty() && !self.reply_saving)
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
            GhCliStatus::Available { .. } => {}
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
                if let GhCliStatus::Available { .. } = result
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
            || self
                .replacement_editor
                .as_ref()
                .is_some_and(CodeReplacementEditor::is_dirty)
            || (!self.reply_body.is_empty() && !self.reply_saving)
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
                    let replacements =
                        DraftStore::open(&state)?.list_replacements(&manifest.preparation_id)?;
                    manifest.expected_generation = drafts.workspace_revision;
                    manifest.draft_counts = drafts.counts.clone();
                    let markdown = binding.description.as_deref().unwrap_or_default();
                    let blocks = safe_markdown_blocks(markdown);
                    Ok::<_, bodh::ContractError>((manifest, binding, blocks, drafts, replacements))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token {
                    return;
                }
                this.loading = false;
                this.active_control = None;
                match result {
                    Ok((manifest, binding, blocks, drafts, replacements)) => {
                        this.remember(&manifest, &binding, cx);
                        this.selected_review = Some(binding);
                        this.refresh_committed_overview(cx);
                        this.refresh_anchor_repair_previews(cx);
                        this.description_blocks = blocks;
                        this.draft_snapshot = Some(drafts);
                        this.replacement_snapshot = Some(replacements);
                        this.draft_editor = None;
                        this.replacement_editor = None;
                        this.draft_saving = false;
                        this.discussion_state = serde_json::json!({"available":false,"threads":[]});
                        this.discussion_loaded_for = None;
                        this.discussion_expanded = false;
                        this.reply_thread = None;
                        this.reply_root = None;
                        this.reply_draft_id = None;
                        this.reply_draft_revision = 0;
                        this.reply_body.clear();
                        this.reply_saving = false;
                        this.active_tab = this.prefs.active_tab.unwrap_or(WorkspaceTab::Overview);
                        this.screen = Screen::Workspace;
                        this.selected_file = None;
                        this.source_text = None;
                        this.source_loading = false;
                        this.source_label = None;
                        this.source_ref = None;
                        this.source_display_ref = None;
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

    fn refresh_committed_overview(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = self.selected_review.clone() else {
            self.committed_overview = CommittedOverviewView::default();
            self.overview_loading = false;
            self.finding_triage_snapshot = None;
            self.finding_triage_loading = false;
            self.diagram_views.clear();
            self.diagram_changed_lines.clear();
            self.diagram_diff_context = None;
            self.active_diagram = None;
            self.diagram_source = None;
            self.diagram_source_display = None;
            self.diagram_source_text = None;
            self.diagram_source_error = None;
            return;
        };
        self.refresh_finding_triage(cx);
        if self.diagram_diff_context.as_deref() != Some(&binding.context.context_id) {
            if let Some(control) = self.diagram_diff_control.take() {
                control.cancel();
            }
            if let Some(control) = self.diagram_source_control.take() {
                control.cancel();
            }
            self.diagram_views.clear();
            self.diagram_changed_lines.clear();
            self.diagram_diff_context = None;
            self.diagram_diff_loading = false;
            self.diagram_selected.clear();
            self.diagram_scrolls.clear();
            self.active_diagram = None;
            self.diagram_source = None;
            self.diagram_source_display = None;
            self.diagram_source_text = None;
            self.diagram_source_error = None;
            self.selected_finding_id = None;
            self.diagram_source_epoch.fetch_add(1, Ordering::Relaxed);
            self.diagram_epoch.fetch_add(1, Ordering::Relaxed);
        }
        if self
            .committed_overview
            .committed
            .as_ref()
            .is_some_and(|review| !committed_matches_context(review, &binding.context))
        {
            self.committed_overview = CommittedOverviewView::default();
        }
        let token = self.overview_epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let epoch = self.overview_epoch.clone();
        let state = self.preparation_db.clone();
        let preparation_id = binding.preparation_id.clone();
        let expected_context = binding.context.clone();
        self.overview_loading = true;
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    GeneratedReviewStore::open(Path::new(&state))?.current(&preparation_id)
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if epoch.load(Ordering::Relaxed) != token
                    || this.selected_review.as_ref().is_none_or(|current| {
                        current.preparation_id != binding.preparation_id
                            || current.context != expected_context
                    })
                {
                    return;
                }
                this.overview_loading = false;
                let previous_generation = this
                    .committed_overview
                    .committed
                    .as_ref()
                    .map(|review| review.generation);
                this.committed_overview = apply_committed_overview_load(
                    &expected_context,
                    &this.committed_overview,
                    result,
                );
                let current_generation = this
                    .committed_overview
                    .committed
                    .as_ref()
                    .map(|review| review.generation);
                if previous_generation != current_generation {
                    if let Some(control) = this.diagram_source_control.take() {
                        control.cancel();
                    }
                    this.diagram_source_epoch.fetch_add(1, Ordering::Relaxed);
                    this.diagram_selected.clear();
                    this.diagram_scrolls.clear();
                    this.active_diagram = None;
                    this.diagram_source = None;
                    this.diagram_source_display = None;
                    this.diagram_source_text = None;
                    this.diagram_source_error = None;
                    this.selected_finding_id = None;
                }
                let initial_finding = if this.active_tab == WorkspaceTab::Findings
                    && this.selected_finding_id.is_none()
                {
                    this.committed_overview
                        .committed
                        .as_ref()
                        .and_then(|review| {
                            bodh::prioritized_finding_indices(&review.result.findings)
                                .first()
                                .and_then(|index| review.result.findings.get(*index))
                                .map(|finding| finding.id.clone())
                        })
                } else {
                    None
                };
                if let Some(id) = initial_finding {
                    this.select_finding(id, cx);
                }
                this.rebuild_diagram_views();
                this.ensure_initial_diagram_selection(cx);
                this.refresh_diagram_change_lines(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn refresh_finding_triage(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = self.selected_review.clone() else {
            self.finding_triage_snapshot = None;
            self.finding_triage_loading = false;
            return;
        };
        if self.finding_triage_loading
            || self
                .finding_triage_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.context_id == binding.context.context_id)
        {
            return;
        }
        let state = self.preparation_db.clone();
        let preparation_id = binding.preparation_id.clone();
        let context_id = binding.context.context_id.clone();
        let epoch = self.operation_epoch.load(Ordering::Relaxed);
        self.finding_triage_loading = true;
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    bodh::FindingTriageStore::open(Path::new(&state))?.list(&preparation_id)
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != epoch
                    || this
                        .selected_review
                        .as_ref()
                        .is_none_or(|current| current.context.context_id != context_id)
                {
                    return;
                }
                this.finding_triage_loading = false;
                match result {
                    Ok(snapshot) => this.finding_triage_snapshot = Some(snapshot),
                    Err(error) => this.error = Some(format!("{}: {}", error.code, error.message)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn rebuild_diagram_views(&mut self) {
        let Some(committed) = self.committed_overview.committed.as_ref() else {
            self.diagram_views.clear();
            self.active_diagram = None;
            return;
        };
        self.diagram_views = committed
            .result
            .diagrams
            .iter()
            .map(|diagram| {
                adapt_diagram(
                    diagram,
                    &committed.result.findings,
                    &self.diagram_changed_lines,
                )
            })
            .collect();
        let ids = self
            .diagram_views
            .iter()
            .map(|diagram| diagram.id.clone())
            .collect::<std::collections::HashSet<_>>();
        self.diagram_selected.retain(|id, _| ids.contains(id));
        self.diagram_scrolls.retain(|id, _| ids.contains(id));
        if self
            .active_diagram
            .as_ref()
            .is_none_or(|active| !ids.contains(active))
        {
            self.active_diagram = self.diagram_views.first().map(|diagram| diagram.id.clone());
        }
        for diagram in &self.diagram_views {
            let selected = self.diagram_selected.entry(diagram.id.clone()).or_insert(0);
            if *selected >= diagram.links.len() {
                *selected = 0;
            }
        }
    }

    fn ensure_initial_diagram_selection(&mut self, cx: &mut Context<Self>) {
        if self.diagram_source.is_some() || self.diagram_source_loading {
            return;
        }
        let Some(id) = self.active_diagram.clone() else {
            return;
        };
        let selected = self.diagram_selected.get(&id).copied().unwrap_or(0);
        self.select_diagram_interaction(selected, cx);
    }

    fn refresh_diagram_change_lines(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        if self.diagram_diff_context.as_deref() == Some(&binding.context.context_id)
            || self.diagram_diff_loading
        {
            return;
        }
        let Some(committed) = self.committed_overview.committed.as_ref() else {
            return;
        };
        let findings = committed.result.findings.clone();
        let empty = ChangedLines::new();
        let mut paths = std::collections::BTreeSet::new();
        for diagram in &committed.result.diagrams {
            let adapted = adapt_diagram(diagram, &findings, &empty);
            for link in adapted.links {
                if let Some(source) = link.source {
                    paths.insert(source.path);
                }
            }
        }
        let files = binding
            .changed_files
            .iter()
            .filter_map(|file| {
                let old_path = file.old_path.clone();
                let new_path = file.new_path.clone();
                if !old_path.as_ref().is_some_and(|path| paths.contains(path))
                    && !new_path.as_ref().is_some_and(|path| paths.contains(path))
                {
                    return None;
                }
                let read_path = new_path.clone().or_else(|| old_path.clone())?;
                Some((old_path, new_path, read_path))
            })
            .collect::<Vec<_>>();
        let checkout = binding.checkout.clone();
        let context = binding.context.clone();
        let context_id = context.context_id.clone();
        let token = self.diagram_epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let epoch = self.diagram_epoch.clone();
        self.diagram_diff_loading = true;
        self.diagram_diff_context = Some(context_id.clone());
        let control = OperationControl::with_timeout(Duration::from_secs(30));
        self.diagram_diff_control = Some(control.clone());
        cx.spawn(async move |weak, cx| {
            let changed = cx
                .background_executor()
                .spawn(async move {
                    let mut result = ChangedLines::new();
                    for (old_path, new_path, read_path) in files {
                        if control.check().is_err() {
                            break;
                        }
                        let Ok(diff) =
                            read_diff(Path::new(&checkout), &context, &read_path, &control)
                        else {
                            continue;
                        };
                        for row in diff_rows(&diff.text) {
                            match row.kind {
                                DiffLineKind::Addition => {
                                    if let (Some(path), Some(line)) =
                                        (new_path.as_ref(), row.new_line)
                                    {
                                        result
                                            .entry((path.clone(), true))
                                            .or_default()
                                            .insert(line);
                                    }
                                }
                                DiffLineKind::Deletion => {
                                    if let (Some(path), Some(line)) =
                                        (old_path.as_ref(), row.old_line)
                                    {
                                        result
                                            .entry((path.clone(), false))
                                            .or_default()
                                            .insert(line);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    result
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if epoch.load(Ordering::Relaxed) != token
                    || this
                        .selected_review
                        .as_ref()
                        .is_none_or(|current| current.context.context_id != context_id)
                {
                    return;
                }
                this.diagram_diff_loading = false;
                this.diagram_diff_control = None;
                this.diagram_diff_context = Some(context_id.clone());
                this.diagram_changed_lines = changed;
                this.rebuild_diagram_views();
                cx.notify();
            });
        })
        .detach();
    }

    fn select_diagram_flow(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.diagram_views.iter().any(|diagram| diagram.id == id) {
            return;
        }
        self.active_diagram = Some(id.clone());
        let selected = self.diagram_selected.get(&id).copied().unwrap_or(0);
        self.select_diagram_interaction(selected, cx);
    }

    fn select_diagram_interaction(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(id) = self.active_diagram.clone() else {
            return;
        };
        let Some(diagram) = self.diagram_views.iter().find(|diagram| diagram.id == id) else {
            return;
        };
        if index >= diagram.links.len() {
            return;
        }
        if let Some(control) = self.diagram_source_control.take() {
            control.cancel();
        }
        self.diagram_selected.insert(id, index);
        self.diagram_source_scroll.set_offset(point(px(0.), px(0.)));
        self.diagram_source_error = None;
        let Some(source) = diagram.links[index].source.clone() else {
            self.diagram_source = None;
            self.diagram_source_display = None;
            self.diagram_source_text = None;
            self.diagram_source_loading = false;
            self.diagram_source_epoch.fetch_add(1, Ordering::Relaxed);
            cx.notify();
            return;
        };
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        let display = source_context_range(&source, 4);
        self.diagram_source = Some(source.clone());
        self.diagram_source_display = Some(display.clone());
        self.diagram_source_text = None;
        self.diagram_source_loading = true;
        let token = self.diagram_source_epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let epoch = self.diagram_source_epoch.clone();
        let checkout = binding.checkout.clone();
        let context = binding.context.clone();
        let control = OperationControl::with_timeout(Duration::from_secs(10));
        self.diagram_source_control = Some(control.clone());
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    read_source_context(
                        Path::new(&checkout),
                        &context,
                        &display,
                        source.start_line,
                        &control,
                    )
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if epoch.load(Ordering::Relaxed) != token {
                    return;
                }
                this.diagram_source_loading = false;
                this.diagram_source_control = None;
                match result {
                    Ok((display, text)) => {
                        this.diagram_source_display = Some(display);
                        this.diagram_source_text = Some(text);
                    }
                    Err(error) => {
                        this.diagram_source_error =
                            Some(format!("{}: {}", error.code, error.message));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn select_diagram_finding(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_finding_id = Some(id);
        self.active_tab = WorkspaceTab::Findings;
        self.prefs.active_tab = Some(WorkspaceTab::Findings);
        self.persist();
        self.refresh_committed_overview(cx);
        cx.notify();
    }

    fn select_finding(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_finding_id = Some(id.clone());
        self.prior_source_preview = None;
        self.finding_prior_source = None;
        let source = self
            .committed_overview
            .committed
            .as_ref()
            .and_then(|review| {
                review
                    .result
                    .findings
                    .iter()
                    .find(|finding| finding.id == id)
            })
            .and_then(|finding| {
                finding
                    .evidence
                    .first()
                    .map(|evidence| evidence.source.clone())
                    .or_else(|| {
                        finding
                            .locations
                            .first()
                            .map(|location| location.source.clone())
                    })
            });
        if let Some(source) = source {
            self.select_finding_source(source, cx);
        } else {
            cx.notify();
        }
    }

    fn select_workspace_tab(&mut self, tab: WorkspaceTab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        self.prefs.active_tab = Some(tab);
        self.persist();
        if matches!(tab, WorkspaceTab::Overview | WorkspaceTab::Findings) {
            self.refresh_committed_overview(cx);
        }
        cx.notify();
    }

    fn open_overview_evidence(&mut self, source: SourceRef, cx: &mut Context<Self>) {
        let Some(binding) = self.selected_review.as_ref() else {
            return;
        };
        let Some(index) = evidence_file_index(&binding.changed_files, &source) else {
            self.error = Some("This evidence source is not part of the pinned change.".into());
            cx.notify();
            return;
        };
        if let Some(control) = self.diagram_source_control.take() {
            control.cancel();
        }
        self.diagram_source_epoch.fetch_add(1, Ordering::Relaxed);
        self.active_tab = WorkspaceTab::Changes;
        self.prefs.active_tab = Some(WorkspaceTab::Changes);
        self.pending_overview_source = Some((index, source));
        self.select_file(index, cx);
    }

    fn open_related_discussion(&mut self, thread_id: String, cx: &mut Context<Self>) {
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        let state = self.preparation_db.clone();
        let preparation_id = binding.preparation_id.clone();
        let context = binding.context.clone();
        let token = self.operation_epoch.load(Ordering::Relaxed);
        let lookup_thread_id = thread_id.clone();
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let observation =
                        bodh::load_latest_discussions(Path::new(&state), &preparation_id)?;
                    let thread = observation["threads"]
                        .as_array()
                        .and_then(|threads| {
                            threads
                                .iter()
                                .find(|thread| thread["id"] == lookup_thread_id)
                        })
                        .cloned()
                        .ok_or_else(|| {
                            bodh::ContractError::new(
                                "THREAD_NOT_FOUND",
                                "Related discussion is not in the prepared observation",
                                "/thread_id",
                            )
                        })?;
                    let path = thread["path"]
                        .as_str()
                        .ok_or_else(|| {
                            bodh::ContractError::new(
                                "THREAD_LOCATION_UNAVAILABLE",
                                "Related discussion has no source path",
                                "/thread_id",
                            )
                        })?
                        .to_owned();
                    let outdated = thread["isOutdated"] == true;
                    let line = if outdated {
                        thread["originalLine"].as_u64()
                    } else {
                        thread["line"]
                            .as_u64()
                            .or_else(|| thread["originalLine"].as_u64())
                    }
                    .ok_or_else(|| {
                        bodh::ContractError::new(
                            "THREAD_LOCATION_UNAVAILABLE",
                            "Related discussion has no readable line",
                            "/thread_id",
                        )
                    })?;
                    let revision = if thread["diffSide"].as_str() == Some("LEFT") || outdated {
                        Revision::Base
                    } else {
                        Revision::Head
                    };
                    Ok::<_, bodh::ContractError>(SourceRef {
                        path,
                        revision,
                        start_line: line as u32,
                        end_line: line as u32,
                    })
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token
                    || this
                        .selected_review
                        .as_ref()
                        .is_none_or(|current| current.context != context)
                {
                    return;
                }
                match result {
                    Ok(source) => {
                        this.discussion_expanded = true;
                        this.discussion_loaded_for = None;
                        this.discussion_state = serde_json::json!({"available":false,"threads":[]});
                        this.focused_discussion_thread = Some(thread_id.clone());
                        this.open_overview_evidence(source, cx);
                    }
                    Err(error) => this.error = Some(format!("{}: {}", error.code, error.message)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn resume(&mut self, id: String, cx: &mut Context<Self>) {
        if self.draft_saving
            || self
                .draft_editor
                .as_ref()
                .is_some_and(TextDraftEditor::is_dirty)
            || self
                .replacement_editor
                .as_ref()
                .is_some_and(CodeReplacementEditor::is_dirty)
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
                    let replacements = DraftStore::open(&state)?.list_replacements(&id)?;
                    let blocks =
                        safe_markdown_blocks(binding.description.as_deref().unwrap_or_default());
                    Ok::<_, bodh::ContractError>((binding, blocks, drafts, replacements))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token {
                    return;
                }
                this.loading = false;
                match result {
                    Ok((binding, blocks, drafts, replacements)) => {
                        this.active_repository = Some(binding.checkout.clone());
                        this.screen = Screen::Workspace;
                        this.description_blocks = blocks;
                        this.draft_snapshot = Some(drafts.clone());
                        this.replacement_snapshot = Some(replacements);
                        this.draft_editor = None;
                        this.replacement_editor = None;
                        this.draft_saving = false;
                        this.discussion_state = serde_json::json!({"available":false,"threads":[]});
                        this.discussion_loaded_for = None;
                        this.discussion_expanded = false;
                        this.reply_thread = None;
                        this.reply_root = None;
                        this.reply_draft_id = None;
                        this.reply_draft_revision = 0;
                        this.reply_body.clear();
                        this.reply_saving = false;
                        this.selected_review = Some(binding.clone());
                        this.refresh_committed_overview(cx);
                        this.refresh_anchor_repair_previews(cx);
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
        let pending_evidence = self
            .pending_overview_source
            .take()
            .filter(|(pending_index, _)| *pending_index == index)
            .map(|(_, source)| source);
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
        self.source_display_ref = None;
        self.source_label = None;
        self.discussion_state = serde_json::json!({"available":false,"threads":[]});
        self.discussion_loading = false;
        self.discussion_loaded_for = None;
        if self.discussion_expanded {
            self.load_selected_file_discussions(cx);
        }
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
        self.source_display_ref = Some(source.clone());
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
                if let Some(source) = pending_evidence {
                    this.select_source_line(index, source.revision, source.start_line, cx);
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
        let previous_discussion_path = self.selected_discussion_path();
        let reuse_display = self.source_text.is_some()
            && self.source_display_ref.as_ref().is_some_and(|display| {
                display.path == source.path
                    && display.revision == source.revision
                    && display.start_line <= line
                    && line <= display.end_line
            });
        self.cancel_pending();
        self.selected_file = Some(index);
        self.source_revision = revision;
        self.source_ref = Some(source.clone());
        self.source_label = Some(source_title(&source));
        if previous_discussion_path.as_deref() != Some(source.path.as_str()) {
            self.discussion_state = serde_json::json!({"available":false,"threads":[]});
            self.discussion_loading = false;
            self.discussion_loaded_for = None;
        }
        if self.discussion_expanded {
            self.load_selected_file_discussions(cx);
        }
        self.source_scroll.set_offset(point(px(0.), px(0.)));
        self.persist_navigation();
        if reuse_display {
            self.source_loading = false;
            cx.notify();
            return;
        }
        let display_source = source_context_range(&source, 4);
        self.source_display_ref = Some(display_source.clone());
        self.source_text = None;
        self.source_loading = true;
        let repository = binding.checkout.clone();
        let context = binding.context.clone();
        let control = OperationControl::with_timeout(Duration::from_secs(10));
        self.active_control = Some(control.clone());
        let token = self.operation_epoch.load(Ordering::Relaxed);
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    read_source_context(
                        Path::new(&repository),
                        &context,
                        &display_source,
                        line,
                        &control,
                    )
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
                    Ok((display, text)) => {
                        this.source_display_ref = Some(display);
                        this.source_text = Some(text);
                    }
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
            || self
                .replacement_editor
                .as_ref()
                .is_some_and(CodeReplacementEditor::is_dirty)
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

    fn begin_code_replacement(&mut self, cx: &mut Context<Self>) {
        if self.draft_saving {
            return;
        }
        let Some(source) = self.source_ref.clone() else {
            self.status =
                Some("Select a changed head-side diff line before drafting a replacement.".into());
            cx.notify();
            return;
        };
        if source.revision != Revision::Head {
            self.status = Some("Code replacements can target only changed head-side lines.".into());
            cx.notify();
            return;
        }
        if self
            .replacement_editor
            .as_ref()
            .is_some_and(CodeReplacementEditor::is_dirty)
        {
            self.status =
                Some("Save or clear the current replacement before starting another.".into());
            cx.notify();
            return;
        }
        let Some(original) = self.source_text.clone() else {
            self.status = Some("Load the pinned head source before drafting a replacement.".into());
            cx.notify();
            return;
        };
        let workspace_revision = self
            .replacement_snapshot
            .as_ref()
            .map(|s| s.workspace_revision)
            .or_else(|| self.draft_snapshot.as_ref().map(|s| s.workspace_revision))
            .unwrap_or_else(|| {
                self.selected_review
                    .as_ref()
                    .map(|b| b.expected_generation)
                    .unwrap_or(0)
            });
        self.replacement_editor = Some(CodeReplacementEditor::new(
            source,
            original,
            workspace_revision,
        ));
        self.status = Some(
            "New code replacement draft. Preview and save locally; the checkout is unchanged."
                .into(),
        );
        cx.notify();
    }

    fn resume_code_replacement(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.draft_saving
            || self
                .replacement_editor
                .as_ref()
                .is_some_and(CodeReplacementEditor::is_dirty)
        {
            self.status =
                Some("Save or clear the current replacement before resuming another.".into());
            cx.notify();
            return;
        }
        let Some(snapshot) = self.replacement_snapshot.clone() else {
            return;
        };
        let Some(draft) = snapshot.drafts.iter().find(|draft| draft.id == id).cloned() else {
            self.error = Some("That saved replacement is no longer in this review context.".into());
            cx.notify();
            return;
        };
        self.replacement_editor = Some(CodeReplacementEditor::resume(
            &draft,
            snapshot.workspace_revision,
        ));
        self.status =
            Some("Saved local replacement resumed; edit the preview before saving.".into());
        cx.notify();
    }

    fn save_code_replacement(&mut self, cx: &mut Context<Self>) {
        if self.draft_saving {
            return;
        }
        let (Some(binding), Some(editor)) = (
            self.selected_review.clone(),
            self.replacement_editor.clone(),
        ) else {
            return;
        };
        if !editor.can_save() {
            self.status = Some("Edit the replacement before saving it.".into());
            cx.notify();
            return;
        }
        let state = self.preparation_db.clone();
        let preparation_id = binding.preparation_id.clone();
        let ui_id = preparation_id.clone();
        let draft_id = editor.draft_id.clone();
        let source = editor.source.clone();
        let original = editor.original_text.clone();
        let replacement = editor.replacement.clone();
        let submitted = replacement.clone();
        let workspace_revision = editor.workspace_revision;
        let draft_revision = editor.draft_revision;
        let control = OperationControl::with_timeout(Duration::from_secs(20));
        let epoch = self.draft_epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let draft_epoch = self.draft_epoch.clone();
        self.draft_saving = true;
        self.status = Some("Saving local code replacement…".into());
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut store = DraftStore::open(Path::new(&state))?;
                    let saved = store.save_replacement(
                        &preparation_id,
                        draft_id.as_deref(),
                        &source,
                        &original,
                        &replacement,
                        workspace_revision,
                        draft_revision,
                        &control,
                    );
                    let snapshot = store.list_replacements(&preparation_id).ok();
                    Ok::<_, bodh::ContractError>((saved, snapshot))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if draft_epoch.load(Ordering::Relaxed) != epoch
                    || this
                        .selected_review
                        .as_ref()
                        .is_none_or(|b| b.preparation_id != ui_id)
                {
                    return;
                }
                this.draft_saving = false;
                match result {
                    Ok((Ok(saved), snapshot)) => {
                        if let Some(snapshot) = snapshot {
                            if let Some(text_snapshot) = &mut this.draft_snapshot {
                                text_snapshot.workspace_revision = snapshot.workspace_revision;
                            }
                            this.replacement_snapshot = Some(snapshot);
                        }
                        if let Some(editor) = &mut this.replacement_editor {
                            editor.save_succeeded(&saved, &submitted);
                        }
                        this.status = Some(
                            "Replacement saved locally. It has not been applied or published."
                                .into(),
                        );
                        this.error = None;
                    }
                    Ok((Err(error), snapshot)) => {
                        if let Some(snapshot) = snapshot {
                            if let Some(text_snapshot) = &mut this.draft_snapshot {
                                text_snapshot.workspace_revision = snapshot.workspace_revision;
                            }
                            this.replacement_snapshot = Some(snapshot);
                        }
                        if let Some(editor) = &mut this.replacement_editor {
                            editor.save_failed(error.code.clone(), error.message.clone());
                        }
                        this.status = Some(
                            "Save failed; your replacement text is still in the editor.".into(),
                        );
                        this.error = Some(format!("{}: {}", error.code, error.message));
                    }
                    Err(error) => {
                        if let Some(editor) = &mut this.replacement_editor {
                            editor.save_failed(error.code.clone(), error.message.clone());
                        }
                        this.status = Some(
                            "Save failed; your replacement text is still in the editor.".into(),
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

    fn discard_code_replacement(&mut self, cx: &mut Context<Self>) {
        if self.draft_saving {
            return;
        }
        let Some(editor) = self.replacement_editor.clone() else {
            return;
        };
        let Some(id) = editor.draft_id.clone() else {
            self.replacement_editor = None;
            self.status =
                Some("Unsaved replacement cleared; no stored proposal was changed.".into());
            cx.notify();
            return;
        };
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        let state = self.preparation_db.clone();
        let preparation_id = binding.preparation_id.clone();
        let ui_id = preparation_id.clone();
        let epoch = self.draft_epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let draft_epoch = self.draft_epoch.clone();
        self.draft_saving = true;
        self.status = Some("Discarding local replacement…".into());
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut store = DraftStore::open(Path::new(&state))?;
                    let discarded = store.discard_replacement(
                        &preparation_id,
                        &id,
                        editor.workspace_revision,
                        editor.draft_revision,
                    );
                    let snapshot = store.list_replacements(&preparation_id).ok();
                    Ok::<_, bodh::ContractError>((discarded, snapshot))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if draft_epoch.load(Ordering::Relaxed) != epoch
                    || this
                        .selected_review
                        .as_ref()
                        .is_none_or(|b| b.preparation_id != ui_id)
                {
                    return;
                }
                this.draft_saving = false;
                match result {
                    Ok((Ok(snapshot), current)) => {
                        let current = current.unwrap_or(snapshot.clone());
                        if let Some(text_snapshot) = &mut this.draft_snapshot {
                            text_snapshot.workspace_revision = current.workspace_revision;
                        }
                        this.replacement_snapshot = Some(current);
                        if let Some(editor) = &mut this.replacement_editor {
                            editor.discard_succeeded(snapshot.workspace_revision);
                        }
                        this.status =
                            Some("Replacement draft discarded. Nothing was applied.".into());
                        this.error = None;
                    }
                    Ok((Err(error), snapshot)) => {
                        if let Some(snapshot) = snapshot {
                            if let Some(text_snapshot) = &mut this.draft_snapshot {
                                text_snapshot.workspace_revision = snapshot.workspace_revision;
                            }
                            this.replacement_snapshot = Some(snapshot);
                        }
                        if let Some(editor) = &mut this.replacement_editor {
                            editor.save_failed(error.code.clone(), error.message.clone());
                        }
                        this.status =
                            Some("Discard failed; your replacement text was retained.".into());
                        this.error = Some(format!("{}: {}", error.code, error.message));
                    }
                    Err(error) => {
                        if let Some(editor) = &mut this.replacement_editor {
                            editor.save_failed(error.code.clone(), error.message.clone());
                        }
                        this.status =
                            Some("Discard failed; your replacement text was retained.".into());
                        this.error = Some(format!("{}: {}", error.code, error.message));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn resume_text_draft(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.draft_saving
            || self
                .draft_editor
                .as_ref()
                .is_some_and(TextDraftEditor::is_dirty)
            || self
                .replacement_editor
                .as_ref()
                .is_some_and(CodeReplacementEditor::is_dirty)
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
                            if let Some(replacements) = &mut this.replacement_snapshot {
                                replacements.workspace_revision = snapshot.workspace_revision;
                            }
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
                            if let Some(replacements) = &mut this.replacement_snapshot {
                                replacements.workspace_revision = snapshot.workspace_revision;
                            }
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
                        if let Some(replacements) = &mut this.replacement_snapshot {
                            replacements.workspace_revision = snapshot.workspace_revision;
                        }
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
                            if let Some(replacements) = &mut this.replacement_snapshot {
                                replacements.workspace_revision = snapshot.workspace_revision;
                            }
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
            || self
                .replacement_editor
                .as_ref()
                .is_some_and(CodeReplacementEditor::is_dirty)
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
            Screen::FinishReview => Screen::Workspace,
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

    fn open_finish_review(&mut self, cx: &mut Context<Self>) {
        if self.draft_saving
            || self
                .draft_editor
                .as_ref()
                .is_some_and(TextDraftEditor::is_dirty)
        {
            self.error = Some(
                "Save or explicitly discard the open inline draft before finishing the review."
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(binding) = self.selected_review.as_ref() else {
            return;
        };
        match ReviewPublicationStore::open(&self.preparation_db).and_then(|mut store| {
            store.recover_interrupted(&binding.preparation_id)?;
            let mut discussions = DiscussionPublicationStore::open(&self.preparation_db)?;
            discussions.recover_interrupted(&binding.preparation_id)?;
            let intent = store.intent(&binding.preparation_id)?;
            let pending = store.pending_operation(&binding.preparation_id)?;
            let discussion = discussions.latest_plan(&binding.preparation_id)?;
            Ok::<_, bodh::ContractError>((intent, pending, discussion))
        }) {
            Ok((mut intent, pending, discussion)) => {
                self.review_plan = pending.clone();
                self.discussion_plan = discussion.clone();
                self.review_summary = intent.summary.clone().unwrap_or_default();
                let mut stale_selection = false;
                let can_refresh = pending.is_none()
                    && discussion.as_ref().is_none_or(|p| {
                        !matches!(p.state.as_str(), "planned" | "sending" | "unknown")
                    });
                if can_refresh && let Some(snapshot) = self.draft_snapshot.as_ref() {
                    intent.selected.retain_mut(|selected| {
                        if let Some(draft) =
                            snapshot.drafts.iter().find(|draft| draft.id == selected.id)
                        {
                            if selected.revision != draft.revision {
                                selected.revision = draft.revision;
                                stale_selection = true;
                            }
                            true
                        } else {
                            stale_selection = true;
                            false
                        }
                    });
                }
                if can_refresh && let Some(snapshot) = self.draft_snapshot.as_ref() {
                    intent.selected_replies.retain_mut(|selected| {
                        if let Some(draft) = snapshot
                            .reply_drafts
                            .iter()
                            .find(|draft| draft.id == selected.id)
                        {
                            if selected.revision != draft.revision {
                                selected.revision = draft.revision;
                                stale_selection = true;
                            }
                            true
                        } else {
                            stale_selection = true;
                            false
                        }
                    });
                    intent.selected_thread_actions.retain_mut(|selected| {
                        if let Some(draft) = snapshot
                            .thread_actions
                            .iter()
                            .find(|draft| draft.id == selected.id)
                        {
                            if selected.revision != draft.revision {
                                selected.revision = draft.revision;
                                stale_selection = true;
                            }
                            true
                        } else {
                            stale_selection = true;
                            false
                        }
                    });
                }
                if can_refresh && let Some(snapshot) = self.replacement_snapshot.as_ref() {
                    intent.selected_replacements.retain_mut(|selected| {
                        if let Some(draft) =
                            snapshot.drafts.iter().find(|draft| draft.id == selected.id)
                        {
                            if selected.revision != draft.revision {
                                selected.revision = draft.revision;
                                stale_selection = true;
                            }
                            true
                        } else {
                            stale_selection = true;
                            false
                        }
                    });
                }
                self.review_intent = Some(intent);
                self.screen = Screen::FinishReview;
                self.error = None;
                if stale_selection {
                    self.save_review_intent();
                }
            }
            Err(error) => self.error = Some(format!("{}: {}", error.code, error.message)),
        }
        cx.notify();
    }

    fn save_review_intent(&mut self) {
        let (Some(binding), Some(intent), Some(snapshot)) = (
            self.selected_review.as_ref(),
            self.review_intent.as_ref(),
            self.draft_snapshot.as_ref(),
        ) else {
            return;
        };
        let mut store = match ReviewPublicationStore::open(&self.preparation_db) {
            Ok(store) => store,
            Err(error) => {
                self.error = Some(format!("{}: {}", error.code, error.message));
                return;
            }
        };
        let saved = store.stage(StageReviewRequest {
            state: self.preparation_db.clone(),
            preparation_id: binding.preparation_id.clone(),
            expected_workspace_revision: snapshot.workspace_revision,
            expected_intent_revision: intent.revision,
            selected: intent.selected.clone(),
            selected_replacements: intent.selected_replacements.clone(),
            selected_replies: intent.selected_replies.clone(),
            selected_thread_actions: intent.selected_thread_actions.clone(),
            summary: if self.review_summary.trim().is_empty() {
                None
            } else {
                Some(self.review_summary.clone())
            },
            event: intent.event,
        });
        match saved {
            Ok(intent) => {
                self.review_intent = Some(intent);
                self.review_plan = None;
                self.error = None;
            }
            Err(error) => self.error = Some(format!("{}: {}", error.code, error.message)),
        }
    }

    fn toggle_review_draft(&mut self, id: &str, cx: &mut Context<Self>) {
        if self
            .review_plan
            .as_ref()
            .is_some_and(|plan| matches!(plan.state.as_str(), "planned" | "sending" | "unknown"))
            || self.discussion_plan.as_ref().is_some_and(|plan| {
                matches!(plan.state.as_str(), "planned" | "sending" | "unknown")
            })
        {
            self.status=Some("This operation is already planned or has an uncertain outcome. Reconcile it before changing its drafts.".into());
            cx.notify();
            return;
        }
        let Some(intent) = self.review_intent.as_mut() else {
            return;
        };
        if let Some(index) = intent.selected.iter().position(|s| s.id == id) {
            intent.selected.remove(index);
        } else if let Some(draft) = self
            .draft_snapshot
            .as_ref()
            .and_then(|s| s.drafts.iter().find(|d| d.id == id))
        {
            intent.selected.push(SelectedDraft {
                id: draft.id.clone(),
                revision: draft.revision,
            });
        }
        self.save_review_intent();
        cx.notify();
    }

    fn toggle_review_replacement(&mut self, id: &str, cx: &mut Context<Self>) {
        if self
            .review_plan
            .as_ref()
            .is_some_and(|plan| matches!(plan.state.as_str(), "planned" | "sending" | "unknown"))
            || self.discussion_plan.as_ref().is_some_and(|plan| {
                matches!(plan.state.as_str(), "planned" | "sending" | "unknown")
            })
        {
            self.status = Some("This operation is already planned or has an uncertain outcome. Reconcile it before changing its drafts.".into());
            cx.notify();
            return;
        }
        let Some(intent) = self.review_intent.as_mut() else {
            return;
        };
        if let Some(index) = intent.selected_replacements.iter().position(|s| s.id == id) {
            intent.selected_replacements.remove(index);
        } else if let Some(draft) = self
            .replacement_snapshot
            .as_ref()
            .and_then(|s| s.drafts.iter().find(|d| d.id == id))
        {
            intent.selected_replacements.push(SelectedDraft {
                id: draft.id.clone(),
                revision: draft.revision,
            });
        }
        self.save_review_intent();
        cx.notify();
    }

    fn toggle_review_reply(&mut self, id: &str, cx: &mut Context<Self>) {
        let locked = self
            .review_plan
            .as_ref()
            .is_some_and(|p| matches!(p.state.as_str(), "planned" | "sending" | "unknown"))
            || self
                .discussion_plan
                .as_ref()
                .is_some_and(|p| matches!(p.state.as_str(), "planned" | "sending" | "unknown"));
        if locked {
            self.status=Some("This publication is already planned or uncertain. Reconcile it before changing selected drafts.".into());
            cx.notify();
            return;
        }
        let Some(intent) = self.review_intent.as_mut() else {
            return;
        };
        if let Some(i) = intent.selected_replies.iter().position(|s| s.id == id) {
            intent.selected_replies.remove(i);
        } else if let Some(draft) = self
            .draft_snapshot
            .as_ref()
            .and_then(|s| s.reply_drafts.iter().find(|d| d.id == id))
        {
            intent.selected_replies.push(SelectedDraft {
                id: draft.id.clone(),
                revision: draft.revision,
            });
        }
        self.save_review_intent();
        cx.notify();
    }

    fn toggle_review_thread_action(&mut self, id: &str, cx: &mut Context<Self>) {
        let locked = self
            .review_plan
            .as_ref()
            .is_some_and(|p| matches!(p.state.as_str(), "planned" | "sending" | "unknown"))
            || self
                .discussion_plan
                .as_ref()
                .is_some_and(|p| matches!(p.state.as_str(), "planned" | "sending" | "unknown"));
        if locked {
            self.status=Some("This publication is already planned or uncertain. Reconcile it before changing selected drafts.".into());
            cx.notify();
            return;
        }
        let Some(intent) = self.review_intent.as_mut() else {
            return;
        };
        if let Some(i) = intent
            .selected_thread_actions
            .iter()
            .position(|s| s.id == id)
        {
            intent.selected_thread_actions.remove(i);
        } else if let Some(draft) = self
            .draft_snapshot
            .as_ref()
            .and_then(|s| s.thread_actions.iter().find(|d| d.id == id))
        {
            intent.selected_thread_actions.push(SelectedDraft {
                id: draft.id.clone(),
                revision: draft.revision,
            });
        }
        self.save_review_intent();
        cx.notify();
    }

    fn choose_review_event(&mut self, event: ReviewEvent, cx: &mut Context<Self>) {
        if self
            .review_plan
            .as_ref()
            .is_some_and(|plan| matches!(plan.state.as_str(), "planned" | "sending" | "unknown"))
            || self.discussion_plan.as_ref().is_some_and(|plan| {
                matches!(plan.state.as_str(), "planned" | "sending" | "unknown")
            })
        {
            self.status=Some("This operation is already planned or has an uncertain outcome. Reconcile it before changing its event.".into());
            cx.notify();
            return;
        }
        if let Some(intent) = self.review_intent.as_mut() {
            intent.event = Some(event);
        }
        self.save_review_intent();
        cx.notify();
    }

    fn review_summary_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if self
            .review_plan
            .as_ref()
            .is_some_and(|plan| matches!(plan.state.as_str(), "planned" | "sending" | "unknown"))
            || self.discussion_plan.as_ref().is_some_and(|plan| {
                matches!(plan.state.as_str(), "planned" | "sending" | "unknown")
            })
        {
            return;
        }
        match event.keystroke.key.as_str() {
            "backspace" => {
                self.review_summary.pop();
            }
            "enter" => self.review_summary.push('\n'),
            "escape" => {
                self.review_summary.clear();
            }
            key if key.chars().count() == 1
                && !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.platform =>
            {
                self.review_summary.push_str(key)
            }
            _ => return,
        }
        self.save_review_intent();
        cx.notify();
    }

    fn confirm_review_plan(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        let Some(intent) = self.review_intent.clone() else {
            return;
        };
        self.review_loading = true;
        self.error = None;
        self.status = Some("Preparing the exact review and discussion operations…".into());
        let token = self.operation_epoch.load(Ordering::Relaxed);
        let state = self.preparation_db.clone();
        let control = OperationControl::with_timeout(Duration::from_secs(30));
        self.active_control = Some(control.clone());
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let has_review = !intent.selected.is_empty()
                        || !intent.selected_replacements.is_empty()
                        || intent
                            .summary
                            .as_deref()
                            .is_some_and(|s| !s.trim().is_empty());
                    if has_review && intent.event.is_none() {
                        return Err(bodh::ContractError::new(
                            "REVIEW_EVENT_REQUIRED",
                            "Choose an event for selected inline comments, replacements or summary",
                            "/event",
                        ));
                    }
                    let review = if has_review {
                        let remote =
                            inspect_remote_review_with_gh(&binding, Path::new("gh"), &control)?;
                        Some(ReviewPublicationStore::open(&state)?.confirm(
                            &binding.preparation_id,
                            &state,
                            intent.revision,
                            &remote,
                            &control,
                        )?)
                    } else {
                        None
                    };
                    if intent.selected_replies.is_empty()
                        && intent.selected_thread_actions.is_empty()
                        && review.is_none()
                    {
                        return Err(bodh::ContractError::new(
                            "PUBLICATION_SELECTION_EMPTY",
                            "Select at least one review operation, reply or thread action",
                            "",
                        ));
                    }
                    let discussion = if intent.selected_replies.is_empty()
                        && intent.selected_thread_actions.is_empty()
                    {
                        None
                    } else {
                        Some(
                            DiscussionPublicationStore::open(&state)?.confirm(
                                &binding.preparation_id,
                                intent.revision,
                                review.as_ref().map(|p| p.id.as_str()),
                                &intent
                                    .selected_replies
                                    .iter()
                                    .map(|s| DiscussionSelection {
                                        id: s.id.clone(),
                                        revision: s.revision,
                                    })
                                    .collect::<Vec<_>>(),
                                &intent
                                    .selected_thread_actions
                                    .iter()
                                    .map(|s| DiscussionSelection {
                                        id: s.id.clone(),
                                        revision: s.revision,
                                    })
                                    .collect::<Vec<_>>(),
                            )?,
                        )
                    };
                    Ok::<_, bodh::ContractError>((review, discussion))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token {
                    return;
                }
                this.review_loading = false;
                this.active_control = None;
                match result {
                    Ok((review, discussion)) => {
                        if let Some(plan) = review {
                            this.review_plan = Some(plan);
                        }
                        if let Some(plan) = discussion {
                            this.discussion_plan = Some(plan);
                        }
                        this.status = Some(
                            "Exact operation plan saved. Inspect each operation before publishing."
                                .into(),
                        );
                        this.error = None;
                    }
                    Err(error) => {
                        this.error = Some(format!("{}: {}", error.code, error.message));
                        this.status = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn publish_discussion_plan(&mut self, cx: &mut Context<Self>) {
        let (Some(binding), Some(plan)) =
            (self.selected_review.clone(), self.discussion_plan.clone())
        else {
            return;
        };
        self.review_loading = true;
        self.error = None;
        self.status = Some("Rechecking thread permissions and state before publishing…".into());
        let token = self.operation_epoch.load(Ordering::Relaxed);
        let state = self.preparation_db.clone();
        let control = OperationControl::with_timeout(Duration::from_secs(60));
        self.active_control = Some(control.clone());
        cx.spawn(async move|weak,cx|{
            let result=cx.background_executor().spawn(async move{
                let publisher=GhDiscussionPublisher{executable:PathBuf::from("gh"),state:state.clone()};
                let mut store=DiscussionPublicationStore::open(&state)?;
                let plan=store.publish(&plan.id,&publisher,&control)?;
                let snapshot=DraftStore::open(&state)?.list(&binding.preparation_id)?;
                Ok::<_,bodh::ContractError>((plan,snapshot))
            }).await;
            let _=weak.update(cx,|this,cx|{if this.operation_epoch.load(Ordering::Relaxed)!=token{return;}this.review_loading=false;this.active_control=None;match result{Ok((plan,snapshot))=>{this.discussion_plan=Some(plan.clone());this.update_draft_counts(snapshot.counts.clone());this.draft_snapshot=Some(snapshot);this.status=Some(format!("Discussion publication finished with state {}. Confirmed steps are removed from the unpublished drafts.",plan.state));this.error=None;},Err(error)=>{this.error=Some(format!("{}: {}",error.code,error.message));this.status=None;}}cx.notify();});
        }).detach();
    }

    fn publish_review_plan(&mut self, cx: &mut Context<Self>) {
        let (Some(binding), Some(plan)) = (self.selected_review.clone(), self.review_plan.clone())
        else {
            return;
        };
        self.review_loading = true;
        self.error = None;
        self.status = Some("Rechecking PR permissions and head before publication…".into());
        let token = self.operation_epoch.load(Ordering::Relaxed);
        let state = self.preparation_db.clone();
        let control = OperationControl::with_timeout(Duration::from_secs(45));
        self.active_control = Some(control.clone());
        cx.spawn(async move |weak,cx| {
            let result=cx.background_executor().spawn(async move {
                let remote=inspect_remote_review_with_gh(&binding,Path::new("gh"),&control)?;
                let mut store=ReviewPublicationStore::open(&state)?;
                let publisher=GhReviewPublisher{executable:PathBuf::from("gh"),checkout:PathBuf::from(&binding.checkout),timeout:Duration::from_secs(35)};
                let operation=store.publish_confirmed(&plan.id,&remote,&publisher,&control)?;
                let snapshot=if operation.state=="confirmed" { Some(DraftStore::open(&state)?.list(&binding.preparation_id)?) } else { None };
                let replacements=if operation.state=="confirmed" { Some(DraftStore::open(&state)?.list_replacements(&binding.preparation_id)?) } else { None };
                Ok::<_,bodh::ContractError>((operation,snapshot,replacements))
            }).await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token {
                    return;
                }
                this.review_loading = false;
                this.active_control = None;
                match result {
                    Ok((operation, snapshot, replacements)) => {
                        if let Some(snapshot) = snapshot {
                            this.update_draft_counts(snapshot.counts.clone());
                            this.draft_snapshot = Some(snapshot);
                        }
                        if let Some(replacements) = replacements {
                            this.replacement_snapshot = Some(replacements);
                        }
                        this.status = Some(match operation.state.as_str() {
                            "confirmed" => format!(
                                "Review published · {}",
                                operation.remote_url.as_deref().unwrap_or(
                                    operation.remote_review_id.as_deref().unwrap_or("confirmed")
                                )
                            ),
                            "failed"
                                if operation.failure.as_deref().is_some_and(|f| {
                                    f.starts_with("No GitHub request was sent.")
                                }) =>
                            {
                                format!(
                                    "Nothing was sent to GitHub: {}",
                                    operation.failure.as_deref().unwrap_or("validation failed")
                                )
                            }
                            "failed" => format!(
                                "GitHub rejected this review: {}",
                                operation.failure.as_deref().unwrap_or("unknown error")
                            ),
                            _ => format!(
                                "Publication outcome is uncertain. Do not resend. Reconcile operation {} before continuing.",
                                operation.id
                            ),
                        });
                        this.review_plan = Some(operation);
                        this.error = None;
                    }
                    Err(error) => {
                        this.error = Some(format!("{}: {}", error.code, error.message));
                        this.status = None;
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    fn refresh_anchor_repair_previews(&mut self, cx: &mut Context<Self>) {
        let Some(candidate) = self.selected_review.clone() else {
            self.anchor_repair_source_id = None;
            self.anchor_repair_previews.clear();
            self.anchor_repair_loading = false;
            return;
        };
        let sources = self
            .prefs
            .recent
            .iter()
            .filter(|review| {
                review.preparation_id != candidate.preparation_id
                    && review.owner == candidate.context.pr.owner
                    && review.repository == candidate.context.pr.repository
                    && review.number == candidate.context.pr.number
            })
            .map(|review| review.preparation_id.clone())
            .collect::<Vec<_>>();
        let state = self.preparation_db.clone();
        let candidate_id = candidate.preparation_id.clone();
        let candidate_id_for_work = candidate_id.clone();
        let control = OperationControl::with_timeout(Duration::from_secs(45));
        let token = self.operation_epoch.load(Ordering::Relaxed);
        self.anchor_repair_loading = true;
        self.anchor_repair_source_id = None;
        self.anchor_repair_previews.clear();
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let store = DraftStore::open(Path::new(&state))?;
                    for source_id in sources {
                        let Ok(text) = store.list(&source_id) else {
                            continue;
                        };
                        let Ok(replacements) = store.list_replacements(&source_id) else {
                            continue;
                        };
                        if text.drafts.is_empty() && replacements.drafts.is_empty() {
                            continue;
                        }
                        let previews = store.preview_anchor_repairs(
                            &source_id,
                            &candidate_id_for_work,
                            &control,
                        )?;
                        if !previews.is_empty() {
                            return Ok::<_, bodh::ContractError>(Some((source_id, previews)));
                        }
                    }
                    Ok(None)
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token
                    || this
                        .selected_review
                        .as_ref()
                        .is_none_or(|binding| binding.preparation_id != candidate_id)
                {
                    return;
                }
                this.anchor_repair_loading = false;
                match result {
                    Ok(Some((source_id, previews))) => {
                        this.anchor_repair_source_id = Some(source_id);
                        this.anchor_repair_previews = previews;
                    }
                    Ok(None) => {
                        this.anchor_repair_source_id = None;
                        this.anchor_repair_previews.clear();
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

    fn reattach_anchor_draft(
        &mut self,
        draft_id: String,
        new_source: SourceRef,
        cx: &mut Context<Self>,
    ) {
        if self.anchor_repair_loading || self.draft_saving {
            return;
        }
        let (Some(candidate), Some(source_id)) = (
            self.selected_review.clone(),
            self.anchor_repair_source_id.clone(),
        ) else {
            return;
        };
        let Some(preview) = self
            .anchor_repair_previews
            .iter()
            .find(|preview| preview.draft_id == draft_id)
            .cloned()
        else {
            return;
        };
        let expected_workspace_revision = self
            .draft_snapshot
            .as_ref()
            .map(|snapshot| snapshot.workspace_revision)
            .or_else(|| {
                self.replacement_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.workspace_revision)
            })
            .unwrap_or(0);
        let state = self.preparation_db.clone();
        let candidate_id = candidate.preparation_id.clone();
        let candidate_id_for_work = candidate_id.clone();
        let token = self.operation_epoch.load(Ordering::Relaxed);
        let control = OperationControl::with_timeout(Duration::from_secs(30));
        self.anchor_repair_loading = true;
        self.status = Some("Rechecking the pinned source before attaching this draft…".into());
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut store = DraftStore::open(Path::new(&state))?;
                    let message = match preview.kind.as_str() {
                        "text_comment" => {
                            store.reattach_text_comment(
                                &source_id,
                                &candidate_id_for_work,
                                &draft_id,
                                preview.draft_revision,
                                expected_workspace_revision,
                                &new_source,
                                &control,
                            )?;
                            "Comment copied to the newer review. The original draft remains on the earlier review.".to_owned()
                        }
                        "code_replacement" => {
                            store.reattach_code_replacement(
                                &source_id,
                                &candidate_id_for_work,
                                &draft_id,
                                preview.draft_revision,
                                expected_workspace_revision,
                                &new_source,
                                &control,
                            )?;
                            "Replacement copied to the newer review. Review its new source text and proposal before publishing.".to_owned()
                        }
                        _ => return Err(bodh::ContractError::new("ANCHOR_REPAIR_INVALID", "Unknown draft kind", "")),
                    };
                    let text = store.list(&candidate_id_for_work)?;
                    let replacements = store.list_replacements(&candidate_id_for_work)?;
                    let previews = store.preview_anchor_repairs(
                        &source_id,
                        &candidate_id_for_work,
                        &control,
                    )?;
                    Ok::<_, bodh::ContractError>((message, text, replacements, previews))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token
                    || this
                        .selected_review
                        .as_ref()
                        .is_none_or(|binding| binding.preparation_id != candidate_id)
                {
                    return;
                }
                this.anchor_repair_loading = false;
                match result {
                    Ok((message, text, replacements, previews)) => {
                        this.update_draft_counts(text.counts.clone());
                        this.draft_snapshot = Some(text);
                        this.replacement_snapshot = Some(replacements);
                        this.anchor_repair_previews = previews;
                        this.status = Some(message);
                        this.error = None;
                    }
                    Err(error) => {
                        this.status = None;
                        this.error = Some(format!("{}: {}", error.code, error.message));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn check_freshness(&mut self, cx: &mut Context<Self>) {
        if self.freshness_loading {
            return;
        }
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        let state = self.preparation_db.clone();
        let gh = match &self.gh_cli_status {
            GhCliStatus::Available { executable, .. } => executable.clone(),
            _ => PathBuf::from("gh"),
        };
        let control = OperationControl::with_timeout(Duration::from_secs(12));
        let token = self.operation_epoch.load(Ordering::Relaxed);
        let id = binding.preparation_id.clone();
        self.freshness_loading = true;
        self.last_freshness_check = Some(std::time::Instant::now());
        self.error = None;
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { bodh::refresh_freshness_with_gh(&binding, &state, &control, &gh) },
                )
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token
                    || this
                        .selected_review
                        .as_ref()
                        .is_none_or(|item| item.preparation_id != id)
                {
                    return;
                }
                this.freshness_loading = false;
                match result {
                    Ok(observation) => {
                        this.freshness_observation = Some(observation);
                        this.latest_switch_preview = None;
                    }
                    Err(error) => this.error = Some(format!("{}: {}", error.code, error.message)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn stage_latest(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        let state = self.preparation_db.clone();
        let gh = match &self.gh_cli_status {
            GhCliStatus::Available { executable, .. } => executable.clone(),
            _ => PathBuf::from("gh"),
        };
        let control = OperationControl::with_timeout(Duration::from_secs(90));
        let token = self.operation_epoch.load(Ordering::Relaxed);
        let id = binding.preparation_id.clone();
        let prepare_id = id.clone();
        self.freshness_loading = true;
        self.error = None;
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    bodh::prepare_latest_candidate_with_gh(&prepare_id, &state, &control, &gh)
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token
                    || this
                        .selected_review
                        .as_ref()
                        .is_none_or(|item| item.preparation_id != id)
                {
                    return;
                }
                this.freshness_loading = false;
                match result {
                    Ok(preview) => {
                        this.latest_switch_preview = Some(preview);
                        this.error = None;
                    }
                    Err(error) => this.error = Some(format!("{}: {}", error.code, error.message)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_latest(&mut self, cx: &mut Context<Self>) {
        if self.draft_saving
            || self
                .draft_editor
                .as_ref()
                .is_some_and(TextDraftEditor::is_dirty)
            || self
                .replacement_editor
                .as_ref()
                .is_some_and(CodeReplacementEditor::is_dirty)
            || (!self.reply_body.is_empty() && !self.reply_saving)
            || self.review_intent.as_ref().is_some_and(|intent| {
                intent.summary.as_deref() != Some(self.review_summary.as_str())
            })
        {
            self.error =
                Some("Save the current editor contents before loading another preparation.".into());
            cx.notify();
            return;
        }
        let (Some(binding), Some(preview)) = (
            self.selected_review.clone(),
            self.latest_switch_preview.clone(),
        ) else {
            return;
        };
        let state = self.preparation_db.clone();
        let gh = match &self.gh_cli_status {
            GhCliStatus::Available { executable, .. } => executable.clone(),
            _ => PathBuf::from("gh"),
        };
        let current_id = binding.preparation_id.clone();
        let candidate_id = preview.candidate.preparation_id.clone();
        let candidate = preview.candidate.clone();
        let expected = preview.protected_work.fingerprint.clone();
        let control = OperationControl::with_timeout(Duration::from_secs(90));
        let token = self.operation_epoch.load(Ordering::Relaxed);
        self.freshness_loading = true;
        cx.spawn(async move |weak, cx| {
            let result = cx.background_executor().spawn(async move {
                bodh::verify_latest_candidate_with_gh(&candidate, &state, &control, &gh)?;
                let approval = bodh::authorize_latest_switch(&state, &current_id, &candidate_id, &expected)?;
                let switched = bodh::apply_latest_switch_approval(&state, &approval.approval_id)?;
                let drafts = DraftStore::open(&state)?.list(&switched.preparation_id)?;
                let replacements = DraftStore::open(&state)?.list_replacements(&switched.preparation_id)?;
                let freshness = bodh::refresh_freshness_with_gh(&switched, &state, &control, &gh)?;
                Ok::<_, bodh::ContractError>((switched, drafts, replacements, freshness))
            }).await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token { return; }
                this.freshness_loading = false;
                match result {
                    Ok((binding, drafts, replacements, freshness)) => {
                        let id = binding.preparation_id.clone();
                        this.active_repository = Some(binding.checkout.clone());
                        this.description_blocks = safe_markdown_blocks(binding.description.as_deref().unwrap_or_default());
                        this.draft_snapshot = Some(drafts.clone()); this.update_draft_counts(drafts.counts);
                        this.replacement_snapshot = Some(replacements);
                        this.discussion_state = serde_json::json!({"available":false,"threads":[]});
                        this.discussion_loaded_for = None;
                        this.discussion_expanded = false;
                        this.selected_review = Some(binding.clone()); this.remember(&PreparationManifest::from(binding.clone()), &binding, cx);
                        this.refresh_committed_overview(cx);
                        this.refresh_anchor_repair_previews(cx);
                        this.freshness_observation = Some(freshness); this.latest_switch_preview = None;
                        this.draft_editor = None; this.replacement_editor = None; this.reply_body.clear();
                        this.status = Some(format!("Loaded latest PR preparation {}. Previous review work remains on the earlier preparation.", id));
                        this.error = None;
                    }
                    Err(error) => this.error = Some(format!("{}: {}", error.code, error.message)),
                }
                cx.notify();
            });
        }).detach();
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
        if self.review_summary_focus.is_focused(window) {
            self.review_summary_key(event, cx);
            return;
        }
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
        if self.screen == Screen::Workspace && self.active_tab == WorkspaceTab::Overview {
            let key = event.keystroke.key.as_str();
            if matches!(key, "down" | "right" | "up" | "left") {
                let selection_state = self.active_diagram.as_ref().and_then(|id| {
                    self.diagram_views
                        .iter()
                        .find(|item| item.id == *id)
                        .map(|diagram| (id.clone(), diagram.links.len()))
                });
                if let Some((id, count)) = selection_state.filter(|(_, count)| *count > 0) {
                    let current = self.diagram_selected.get(&id).copied().unwrap_or(0);
                    let next = match key {
                        "down" | "right" => (current + 1) % count,
                        _ => (current + count - 1) % count,
                    };
                    self.select_diagram_interaction(next, cx);
                    let row_rect = self
                        .diagram_views
                        .iter()
                        .find(|diagram| diagram.id == id)
                        .and_then(|diagram| diagram.scene.rows.get(next))
                        .map(|row| row.rect);
                    if let (Some(row), Some(scroll)) = (row_rect, self.diagram_scrolls.get(&id)) {
                        scroll.set_offset(point(
                            px(-(row.x - 40.).max(0.)),
                            px(-(row.y - 90.).max(0.)),
                        ));
                    }
                    cx.notify();
                }
                return;
            }
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
        if self.screen == Screen::Workspace
            && _window.is_window_active()
            && !self.freshness_loading
            && self
                .last_freshness_check
                .is_none_or(|last| last.elapsed() >= Duration::from_secs(60))
        {
            self.check_freshness(cx);
        }
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
            Screen::FinishReview => root.child(self.render_finish_review(p, cx)),
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
                    || this
                        .replacement_editor
                        .as_ref()
                        .is_some_and(CodeReplacementEditor::is_dirty)
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
                |this, _, _, cx| this.open_finish_review(cx),
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
            GhCliStatus::Available { version, .. } => ("GitHub CLI ready", version.as_str(), false),
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

    fn render_finish_review(&mut self, p: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let binding = self
            .selected_review
            .clone()
            .expect("Finish Review screen requires a prepared review");
        let intent = self.review_intent.clone();
        let selected = intent
            .as_ref()
            .map(|i| {
                i.selected
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect::<std::collections::HashSet<_>>()
            })
            .unwrap_or_default();
        let mut page = div()
            .id("finish-review")
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .p_6()
            .flex()
            .flex_col()
            .gap_4();
        page = page.child(
            div()
                .text_size(px(22.))
                .font_weight(FontWeight::BOLD)
                .child("Finish Review"),
        );
        page = page.child(div().text_color(rgb(p.muted)).child(format!(
            "{}/{} #{} · nothing is sent until you confirm the exact plan below",
            binding.context.pr.owner, binding.context.pr.repository, binding.context.pr.number
        )));
        let drafts = self
            .draft_snapshot
            .as_ref()
            .map(|s| s.drafts.clone())
            .unwrap_or_default();
        if drafts.is_empty() {
            page = page.child(empty_card(
                "No inline comments saved",
                "You can still write a review summary and choose an event.",
                p,
            ));
        } else {
            page = page.child(section_title(
                "Inline comments · select or unstage each draft",
                p,
            ));
            for draft in drafts {
                let id = draft.id.clone();
                let select_id = id.clone();
                let edit_id = id.clone();
                let label = if selected.contains(id.as_str()) {
                    "✓ Selected"
                } else {
                    "＋ Add to review"
                };
                let entry = div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .p_3()
                    .border_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.panel))
                    .child(button(
                        &format!("review-select-{id}"),
                        label,
                        p,
                        move |this, _, _, cx| this.toggle_review_draft(&select_id, cx),
                        cx,
                    ))
                    .child(button(
                        &format!("review-edit-{id}"),
                        "Edit in Changes",
                        p,
                        move |this, _, _, cx| {
                            this.screen = Screen::Workspace;
                            this.active_tab = WorkspaceTab::Changes;
                            this.resume_text_draft(&edit_id, cx);
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .font_family("DejaVu Sans Mono")
                                    .text_size(px(11.))
                                    .text_color(rgb(p.muted))
                                    .child(source_title(&draft.source)),
                            )
                            .child(div().child(draft.body)),
                    );
                page = page.child(entry);
            }
        }
        let selected_replacements = intent
            .as_ref()
            .map(|i| {
                i.selected_replacements
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect::<std::collections::HashSet<_>>()
            })
            .unwrap_or_default();
        let replacements = self
            .replacement_snapshot
            .as_ref()
            .map(|s| s.drafts.clone())
            .unwrap_or_default();
        page = page.child(section_title(
            "Code replacement suggestions · select or unstage",
            p,
        ));
        if replacements.is_empty() {
            page = page.child(empty_card("No code replacements saved", "Replacement suggestions are authored in Changes and remain local until you publish a confirmed review.", p));
        } else {
            for draft in replacements {
                let id = draft.id.clone();
                let select_id = id.clone();
                let edit_id = id.clone();
                let label = if selected_replacements.contains(id.as_str()) {
                    "✓ Selected"
                } else {
                    "＋ Add suggestion"
                };
                page = page.child(
                    div()
                        .flex()
                        .items_start()
                        .gap_3()
                        .p_3()
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.panel))
                        .child(button(
                            &format!("review-replacement-select-{id}"),
                            label,
                            p,
                            move |this, _, _, cx| this.toggle_review_replacement(&select_id, cx),
                            cx,
                        ))
                        .child(button(
                            &format!("review-replacement-edit-{id}"),
                            "Edit in Changes",
                            p,
                            move |this, _, _, cx| {
                                this.screen = Screen::Workspace;
                                this.active_tab = WorkspaceTab::Changes;
                                this.resume_code_replacement(&edit_id, cx);
                                cx.notify();
                            },
                            cx,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .font_family("DejaVu Sans Mono")
                                        .text_size(px(11.))
                                        .text_color(rgb(p.muted))
                                        .child(source_title(&draft.source)),
                                )
                                .child(div().text_size(px(11.)).text_color(rgb(p.muted)).child(
                                    format!(
                                        "Replace {} character(s)",
                                        draft.original_text.chars().count()
                                    ),
                                ))
                                .child(
                                    div()
                                        .font_family("DejaVu Sans Mono")
                                        .child(draft.replacement),
                                ),
                        ),
                );
            }
        }
        let selected_replies = intent
            .as_ref()
            .map(|i| {
                i.selected_replies
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect::<std::collections::HashSet<_>>()
            })
            .unwrap_or_default();
        let selected_actions = intent
            .as_ref()
            .map(|i| {
                i.selected_thread_actions
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect::<std::collections::HashSet<_>>()
            })
            .unwrap_or_default();
        let replies = self
            .draft_snapshot
            .as_ref()
            .map(|s| s.reply_drafts.clone())
            .unwrap_or_default();
        page = page.child(section_title(
            "Replies · publish after the grouped review",
            p,
        ));
        if replies.is_empty() {
            page=page.child(empty_card("No reply drafts saved","Replies are attached to the exact top-level review comment selected with its inline thread.",p));
        }
        for draft in replies {
            let id = draft.id.clone();
            let toggle_id = id.clone();
            let label = if selected_replies.contains(id.as_str()) {
                "✓ Selected"
            } else {
                "＋ Add reply"
            };
            page = page.child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .p_3()
                    .border_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.panel))
                    .child(button(
                        &format!("review-reply-select-{id}"),
                        label,
                        p,
                        move |this, _, _, cx| this.toggle_review_reply(&toggle_id, cx),
                        cx,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .font_family("DejaVu Sans Mono")
                                    .text_size(px(11.))
                                    .text_color(rgb(p.muted))
                                    .child(format!(
                                        "Thread {} · top-level comment {}",
                                        draft.thread_id, draft.root_comment_id
                                    )),
                            )
                            .child(div().child(draft.body)),
                    ),
            );
        }
        let actions = self
            .draft_snapshot
            .as_ref()
            .map(|s| s.thread_actions.clone())
            .unwrap_or_default();
        page = page.child(section_title("Thread actions · resolve or reopen", p));
        if actions.is_empty() {
            page=page.child(empty_card("No thread actions staged","Resolve and reopen requests remain local until the current permission and thread state are checked.",p));
        }
        for draft in actions {
            let id = draft.id.clone();
            let toggle_id = id.clone();
            let verb = match draft.action {
                bodh::ThreadAction::Resolve => "Resolve",
                bodh::ThreadAction::Reopen => "Reopen",
            };
            let label = if selected_actions.contains(id.as_str()) {
                format!("✓ {verb} selected")
            } else {
                format!("＋ {verb} thread")
            };
            page = page.child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .p_3()
                    .border_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.panel))
                    .child(button(
                        &format!("review-thread-action-{id}"),
                        &label,
                        p,
                        move |this, _, _, cx| this.toggle_review_thread_action(&toggle_id, cx),
                        cx,
                    ))
                    .child(div().flex_1().child(format!(
                        "Thread {} · observed {}",
                        draft.thread_id,
                        if draft.observed_resolved {
                            "resolved"
                        } else {
                            "open"
                        }
                    ))),
            );
        }
        page = page.child(section_title("Review event · choose explicitly", p));
        let event = intent.as_ref().and_then(|i| i.event);
        let events = [
            ("COMMENT", ReviewEvent::Comment),
            ("APPROVE", ReviewEvent::Approve),
            ("REQUEST CHANGES", ReviewEvent::RequestChanges),
        ];
        let mut event_row = div().flex().gap_2();
        for (label, value) in events {
            let chosen = event == Some(value);
            let button_label = if chosen {
                match label {
                    "COMMENT" => "✓ COMMENT",
                    "APPROVE" => "✓ APPROVE",
                    _ => "✓ REQUEST CHANGES",
                }
            } else {
                label
            };
            event_row = event_row.child(button(
                &format!("review-event-{label}"),
                button_label,
                p,
                move |this, _, _, cx| this.choose_review_event(value, cx),
                cx,
            ));
        }
        page = page.child(event_row);
        page = page.child(section_title("Review summary", p));
        page = page.child(
            div()
                .id("finish-review-summary")
                .min_h(px(88.))
                .p_3()
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.panel))
                .focus(|style| style.border_color(rgb(p.accent)))
                .track_focus(&self.review_summary_focus)
                .child(if self.review_summary.is_empty() {
                    "Click to write a review summary. Request changes requires a summary."
                        .to_owned()
                } else {
                    self.review_summary.clone()
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, _| window.focus(&this.review_summary_focus)),
                ),
        );
        page=page.child(div().text_size(px(11.)).text_color(rgb(p.muted)).child("Summary text input: type to append · Backspace removes · Enter starts a new line · Escape clears."));
        if let Some(plan) = self.discussion_plan.clone() {
            page = page.child(section_title("Exact replies and thread actions", p));
            page = page.child(empty_card(
                &format!("{} operation(s) · {}", plan.steps.len(), plan.state),
                &format!(
                    "Plan {} · grouped review {}",
                    plan.id,
                    plan.review_operation_id
                        .as_deref()
                        .unwrap_or("not selected")
                ),
                p,
            ));
            for step in &plan.steps {
                let description = if step.kind == "reply" {
                    format!(
                        "Reply to top-level comment {}",
                        step.root_comment_id.as_deref().unwrap_or("(missing)")
                    )
                } else {
                    format!(
                        "{} thread",
                        step.action
                            .map(|a| format!("{a:?}"))
                            .unwrap_or_else(|| "Unknown".into())
                    )
                };
                page = page.child(
                    div()
                        .p_3()
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.raised))
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .child(format!("{} · {}", description, step.state)),
                        )
                        .child(
                            div()
                                .font_family("DejaVu Sans Mono")
                                .text_size(px(10.))
                                .text_color(rgb(p.muted))
                                .child(format!(
                                    "Thread {} · draft {} revision {}",
                                    step.thread_id, step.draft_id, step.draft_revision
                                )),
                        )
                        .child(
                            step.body
                                .clone()
                                .or_else(|| step.failure.clone())
                                .unwrap_or_default(),
                        ),
                );
            }
            if plan.state == "planned"
                && self
                    .review_plan
                    .as_ref()
                    .is_none_or(|review| review.state == "confirmed")
                && !self.review_loading
            {
                page = page.child(button(
                    "publish-confirmed-discussions",
                    "Publish replies and thread actions",
                    p,
                    |this, _, _, cx| this.publish_discussion_plan(cx),
                    cx,
                ));
            } else if plan.state == "unknown" {
                page = page.child(empty_card(
                    "Discussion outcome uncertain",
                    "Do not resend. Inspect the remote thread before continuing.",
                    p,
                ));
            }
        }
        if let Some(plan) = self.review_plan.clone() {
            page = page.child(section_title("Exact confirmed plan", p));
            page = page.child(empty_card(
                &format!(
                    "{} · {} inline comment(s) · {} code suggestion(s)",
                    format!("{:?}", plan.payload.event).to_ascii_uppercase(),
                    plan.payload.comments.len(),
                    plan.payload.suggestions.len()
                ),
                &format!(
                    "Summary: {}",
                    plan.payload.summary.as_deref().unwrap_or("(none)")
                ),
                p,
            ));
            for comment in &plan.payload.comments {
                page = page.child(
                    div()
                        .p_3()
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.raised))
                        .child(
                            div()
                                .font_family("DejaVu Sans Mono")
                                .text_size(px(11.))
                                .text_color(rgb(p.muted))
                                .child(source_title(&comment.source)),
                        )
                        .child(comment.body.clone()),
                );
            }
            for suggestion in &plan.payload.suggestions {
                page = page.child(
                    div()
                        .p_3()
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.raised))
                        .child(
                            div()
                                .font_family("DejaVu Sans Mono")
                                .text_size(px(11.))
                                .text_color(rgb(p.muted))
                                .child(source_title(&suggestion.source)),
                        )
                        .child(
                            div()
                                .font_family("DejaVu Sans Mono")
                                .text_size(px(11.))
                                .child(suggestion.body.clone()),
                        ),
                );
            }
            page = page.child(
                div()
                    .font_family("DejaVu Sans Mono")
                    .text_size(px(10.))
                    .text_color(rgb(p.muted))
                    .child(format!(
                        "Fingerprint {} · operation {}",
                        plan.fingerprint, plan.id
                    )),
            );
            if plan.state == "planned" {
                page = page.child(button(
                    "publish-confirmed-review",
                    "Publish this exact review to GitHub",
                    p,
                    |this, _, _, cx| this.publish_review_plan(cx),
                    cx,
                ));
            } else if plan.state == "unknown" {
                page=page.child(empty_card("Outcome uncertain","Do not resend this operation. Inspect the PR for the exact review before taking further action.",p));
            } else {
                page = page.child(
                    div()
                        .text_color(rgb(p.muted))
                        .child(format!("Operation state: {}", plan.state)),
                );
            }
        } else {
            let can_confirm = intent.as_ref().is_some_and(|i| {
                let has_review = !i.selected.is_empty()
                    || !i.selected_replacements.is_empty()
                    || !self.review_summary.trim().is_empty();
                let has_discussion =
                    !i.selected_replies.is_empty() || !i.selected_thread_actions.is_empty();
                (has_review || has_discussion) && (!has_review || i.event.is_some())
            }) && !self.review_loading;
            if can_confirm {
                page = page.child(button(
                    "confirm-review-plan",
                    "Check PR state and confirm exact plan",
                    p,
                    |this, _, _, cx| this.confirm_review_plan(cx),
                    cx,
                ));
            } else {
                page = page.child(empty_card(
                    "Select operations to publish",
                    "A grouped review needs an explicit event. Replies and thread actions can be confirmed on their own.",
                    p,
                ));
            }
        }
        if self.review_loading {
            page = page.child(div().text_color(rgb(p.muted)).child("Working…"));
        }
        page
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
        page = if matches!(
            self.active_tab,
            WorkspaceTab::Changes | WorkspaceTab::Findings
        ) {
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
        let freshness_label = if self.freshness_loading {
            "Checking GitHub metadata…".to_owned()
        } else if let Some(observation) = &self.freshness_observation {
            match observation.status {
                bodh::FreshnessStatus::Current => {
                    format!("Up to date · checked {}", observation.checked_at_ms)
                }
                bodh::FreshnessStatus::Changed => format!(
                    "PR changed on GitHub · head moved: {}, base moved: {}",
                    observation.head_moved, observation.base_moved
                ),
                bodh::FreshnessStatus::Closed => "PR is no longer open on GitHub".into(),
                bodh::FreshnessStatus::Unknown => format!(
                    "Freshness unknown · {}",
                    observation
                        .error_code
                        .as_deref()
                        .unwrap_or("metadata unavailable")
                ),
            }
        } else {
            "Freshness has not been checked".into()
        };
        let mut freshness_bar = div()
            .flex()
            .items_center()
            .gap_3()
            .p_3()
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.panel))
            .child(
                div()
                    .flex_1()
                    .text_color(rgb(p.muted))
                    .child(freshness_label),
            );
        freshness_bar = freshness_bar.child(button(
            "check-pr-freshness",
            "Check for Updates",
            p,
            |this, _, _, cx| this.check_freshness(cx),
            cx,
        ));
        if self
            .freshness_observation
            .as_ref()
            .is_some_and(|item| item.status == bodh::FreshnessStatus::Changed)
        {
            freshness_bar = freshness_bar.child(button(
                "stage-latest-pr",
                "Load Latest…",
                p,
                |this, _, _, cx| this.stage_latest(cx),
                cx,
            ));
        }
        page = page.child(freshness_bar);
        if let Some(preview) = self.latest_switch_preview.clone() {
            page = page.child(div().flex().items_center().gap_3().p_3().border_1()
                .border_color(rgb(p.accent)).bg(rgb(p.raised))
                .child(div().flex_1().child(format!(
                    "Ready to load latest · base {} · head {} · {} pending work item(s) stay on the current review",
                    short_sha(&preview.candidate.context.base_sha), short_sha(&preview.candidate.context.head_sha),
                    preview.protected_work.unpublished_draft_count + preview.protected_work.unresolved_publication_count
                )))
                .child(button("apply-latest-pr", "Load this preparation", p,
                    |this, _, _, cx| this.apply_latest(cx), cx)));
        }
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
                        this.select_workspace_tab(tab, cx);
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
            WorkspaceTab::Overview => self
                .render_overview(&binding, p, cx, viewport_width, viewport_height)
                .into_any_element(),
            WorkspaceTab::Changes => self
                .render_changes(&binding, p, cx, viewport_width)
                .into_any_element(),
            WorkspaceTab::Findings => self.render_findings(&binding, p, cx).into_any_element(),
        };
        page.child(panel)
    }

    fn render_overview(
        &mut self,
        binding: &PreparedBinding,
        p: Palette,
        cx: &mut Context<Self>,
        viewport_width: f32,
        viewport_height: f32,
    ) -> impl IntoElement {
        let mut body = div()
            .id("overview-content")
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .min_h_0()
            .overflow_scroll();
        let current = self
            .committed_overview
            .committed
            .clone()
            .filter(|review| committed_matches_context(review, &binding.context));
        if self.overview_loading && current.is_none() {
            body = body.child(empty_card(
                "Loading committed review",
                "Reading the last complete result for this pinned preparation.",
                p,
            ));
        }
        if let Some(error_code) = &self.committed_overview.error_code {
            body = body.child(empty_card(
                if current.is_some() {
                    "Could not refresh generated review"
                } else {
                    "Generated review unavailable"
                },
                &format!(
                    "The last successful display is retained when available. State read error: {error_code}."
                ),
                p,
            ));
        }
        if let Some(review) = current {
            let generated = &review.result;
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_3()
                    .border_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.panel))
                    .child(div().flex_1().child(format!(
                        "Committed review · generation {} · context {}",
                        review.generation,
                        short_sha(&generated.context.context_id)
                    )))
                    .child(button(
                        "refresh-committed-review",
                        if self.overview_loading {
                            "Refreshing…"
                        } else {
                            "Refresh result"
                        },
                        p,
                        |this, _, _, cx| this.refresh_committed_overview(cx),
                        cx,
                    )),
            );

            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_4()
                    .border_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.panel))
                    .child(section_title("Implementation summary", p))
                    .child(generated.overview.implementation_summary.clone()),
            );

            body = body.child(self.render_sequence_workspace(
                generated,
                binding,
                p,
                cx,
                viewport_width,
                viewport_height,
            ));

            if generated.overview.key_changes.is_empty() {
                body = body.child(empty_card(
                    "No key changes recorded",
                    "The committed result contains no key-change claims.",
                    p,
                ));
            } else {
                body = body.child(section_title("Key changes", p));
                for (change_index, change) in generated.overview.key_changes.iter().enumerate() {
                    let mut card = div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_4()
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.panel))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(change.summary.clone()),
                        );
                    for (evidence_index, evidence) in change.evidence.iter().enumerate() {
                        let source = evidence.source.clone();
                        card = card.child(
                            div()
                                .text_color(rgb(p.muted))
                                .child(evidence.explanation.clone()),
                        );
                        card = card.child(button(
                            &format!("key-change-evidence-{change_index}-{evidence_index}"),
                            &format!("Open {}", source_title(&source)),
                            p,
                            move |this, _, _, cx| this.open_overview_evidence(source.clone(), cx),
                            cx,
                        ));
                    }
                    body = body.child(card);
                }
            }

            body = body.child(section_title("Intent assessment", p));
            if generated.overview.intent_assessments.is_empty() {
                body = body.child(empty_card(
                    "No intent assessments recorded",
                    "The committed result does not assess quoted PR intent.",
                    p,
                ));
            } else {
                for (assessment_index, assessment) in
                    generated.overview.intent_assessments.iter().enumerate()
                {
                    let concern = is_significant_assessment(&assessment.status);
                    let mut card = div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_4()
                        .border_1()
                        .border_color(rgb(if concern { p.accent } else { p.border }))
                        .bg(rgb(if concern { p.raised } else { p.panel }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .font_weight(FontWeight::BOLD)
                                        .child(format!("{:?}", assessment.status)),
                                )
                                .child(if concern {
                                    "Needs reviewer attention"
                                } else {
                                    "Aligned"
                                }),
                        )
                        .child(format!(
                            "Exact PR quote · occurrence {}: “{}”",
                            assessment.occurrence, assessment.quote
                        ))
                        .child(assessment.explanation.clone());
                    for (evidence_index, evidence) in assessment.evidence.iter().enumerate() {
                        let source = evidence.source.clone();
                        card = card.child(
                            div()
                                .text_color(rgb(p.muted))
                                .child(evidence.explanation.clone()),
                        );
                        card = card.child(button(
                            &format!("intent-evidence-{assessment_index}-{evidence_index}"),
                            &format!("Open {}", source_title(&source)),
                            p,
                            move |this, _, _, cx| this.open_overview_evidence(source.clone(), cx),
                            cx,
                        ));
                    }
                    if !assessment.finding_ids.is_empty() {
                        card = card.child(format!(
                            "Related findings: {}",
                            assessment.finding_ids.join(", ")
                        ));
                    }
                    body = body.child(card);
                }
            }

            body = body.child(section_title("Coverage and checks", p));
            let mut coverage = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.panel));
            if generated.coverage.examined_areas.is_empty() {
                coverage = coverage.child("No examined areas were recorded.");
            } else {
                coverage = coverage.child(section_title("Examined areas", p));
                for area in &generated.coverage.examined_areas {
                    coverage = coverage.child(format!("• {area}"));
                }
            }
            if generated.coverage.checks.is_empty() {
                coverage = coverage.child("No checks were recorded.");
            } else {
                coverage = coverage.child(section_title("Checks", p));
                for check in &generated.coverage.checks {
                    coverage = coverage.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .border_l_2()
                            .border_color(rgb(
                                if matches!(&check.status, bodh::CheckStatus::Failed) {
                                    p.accent
                                } else {
                                    p.border
                                },
                            ))
                            .pl_3()
                            .child(format!("{} · {:?}", check.name, check.status))
                            .child(check.details.clone()),
                    );
                }
            }
            if !generated.coverage.excluded_areas.is_empty() {
                coverage = coverage.child(section_title("Excluded areas", p));
                for excluded in &generated.coverage.excluded_areas {
                    coverage = coverage.child(format!("{} · {}", excluded.area, excluded.reason));
                }
            }
            if !generated.coverage.limitations.is_empty() {
                coverage = coverage.child(section_title("Limitations", p));
                for limitation in &generated.coverage.limitations {
                    coverage = coverage.child(format!("• {limitation}"));
                }
            }
            body = body.child(coverage);
        } else {
            let (title, message) = match self.committed_overview.status {
                CommittedOverviewStatus::StaleContext => (
                    "Earlier result hidden",
                    "A committed result exists for another pinned context and is not shown here.",
                ),
                CommittedOverviewStatus::LoadFailed => (
                    "Generated review unavailable",
                    "The committed result could not be read. Refresh after the state store is available.",
                ),
                CommittedOverviewStatus::Missing | CommittedOverviewStatus::Ready => (
                    "No generated overview",
                    "This prepared review contains pinned code context only. No committed analysis is attached.",
                ),
            };
            body = body.child(empty_card(title, message, p));
            if self.committed_overview.status != CommittedOverviewStatus::LoadFailed {
                body = body.child(button(
                    "refresh-missing-committed-review",
                    if self.overview_loading {
                        "Loading…"
                    } else {
                        "Check for committed result"
                    },
                    p,
                    |this, _, _, cx| this.refresh_committed_overview(cx),
                    cx,
                ));
            }
        }

        body = body.child(section_title("Original pull request description", p));
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
                .child(section_title("Author-provided text", p))
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

    fn render_sequence_workspace(
        &mut self,
        generated: &bodh::ReviewResult,
        _binding: &PreparedBinding,
        p: Palette,
        cx: &mut Context<Self>,
        viewport_width: f32,
        viewport_height: f32,
    ) -> impl IntoElement {
        let mut workspace = div()
            .id("sequence-review-workspace")
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2();
        if self.diagram_views.is_empty() {
            return workspace.child(empty_card(
                "No sequence diagrams",
                "The committed result contains no sequence flows. No diagram is synthesized.",
                p,
            ));
        }

        let active_id = self
            .active_diagram
            .clone()
            .unwrap_or_else(|| self.diagram_views[0].id.clone());
        let Some(diagram) = self
            .diagram_views
            .iter()
            .find(|diagram| diagram.id == active_id)
            .cloned()
        else {
            return workspace.child(empty_card(
                "Sequence flow unavailable",
                "The selected flow is no longer part of the committed result.",
                p,
            ));
        };
        let selected = self
            .diagram_selected
            .get(&diagram.id)
            .copied()
            .unwrap_or(0)
            .min(diagram.links.len().saturating_sub(1));
        let mut toolbar = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(section_title(&diagram.flow.title, p));
        if self.diagram_views.len() > 1 {
            for flow in &self.diagram_views {
                let id = flow.id.clone();
                let active = id == diagram.id;
                toolbar = toolbar.child(
                    button(
                        &format!("select-diagram-{}", id),
                        &flow.flow.title,
                        p,
                        move |this, _, window, cx| {
                            window.focus(&this.focus);
                            this.select_diagram_flow(id.clone(), cx);
                        },
                        cx,
                    )
                    .text_size(px(11.))
                    .text_color(rgb(if active { p.text } else { p.muted })),
                );
            }
        }
        workspace = workspace.child(toolbar);

        let pair_height = (viewport_height - 340.).max(360.);
        let source_width = (viewport_width * 0.42).clamp(300., 560.);
        let mut diagram_panel = div()
            .id("sequence-diagram-panel")
            .flex_1()
            .min_w_0()
            .h(px(pair_height))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.panel))
            .flex()
            .flex_col()
            .child(
                div()
                    .p_2()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .child(diagram.description.clone()),
            );
        let scroll = self.diagram_scrolls.entry(diagram.id.clone()).or_default();
        let diagram_element = SequenceDiagram {
            flow: std::rc::Rc::new(diagram.flow.clone()),
            scene: std::rc::Rc::new(diagram.scene.clone()),
            selected,
            palette: super::Palette {
                bg: p.canvas,
                panel: p.panel,
                text: p.text,
                muted: p.muted,
                accent: p.accent,
            },
            owner: cx.entity().downgrade(),
            select: select_product_diagram_interaction,
            interaction_ids: std::rc::Rc::new(
                diagram
                    .links
                    .iter()
                    .map(|link| {
                        format!(
                            "diagram:{}:{}:step:{}:{}",
                            diagram.id.len(),
                            diagram.id,
                            link.id.len(),
                            link.id
                        )
                    })
                    .collect(),
            ),
            changed_interactions: Some(std::rc::Rc::new(diagram.changed_interactions.clone())),
        };
        diagram_panel = diagram_panel.child(
            div()
                .id(ElementId::Name(SharedString::from(format!(
                    "sequence-diagram-scroll-{}",
                    diagram.id
                ))))
                .track_scroll(scroll)
                .on_scroll_wheel(|event, _, cx| stop_vertical_wheel_bubbling(event, cx))
                .overflow_scroll()
                .flex_1()
                .min_h_0()
                .child(diagram_element),
        );

        let mut inspector = div()
            .id("sequence-source-inspector")
            .w(px(source_width))
            .flex_shrink_0()
            .min_w_0()
            .h(px(pair_height))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.panel))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(section_title("Pinned source", p));
        if let Some(link) = diagram.links.get(selected) {
            inspector = inspector.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(link.explanation.clone()),
            );
            if let Some(source) = link.source.clone() {
                inspector = inspector.child(button(
                    "open-diagram-source-in-changes",
                    "Open source in Changes",
                    p,
                    move |this, _, _, cx| {
                        this.open_overview_evidence(source.clone(), cx);
                    },
                    cx,
                ));
            }
            for finding_id in &link.findings {
                if let Some(finding) = generated
                    .findings
                    .iter()
                    .find(|finding| finding.id == *finding_id)
                {
                    let id = finding.id.clone();
                    inspector = inspector.child(button(
                        &format!("diagram-linked-finding-{id}"),
                        &format!("Finding · {}", finding.title),
                        p,
                        move |this, _, window, cx| {
                            window.focus(&this.focus);
                            this.select_diagram_finding(id.clone(), cx);
                        },
                        cx,
                    ));
                }
            }
        }
        if self.diagram_source_loading {
            inspector = inspector.child(
                div()
                    .text_color(rgb(p.muted))
                    .child("Loading source from the pinned commit…"),
            );
        }
        if let Some(error) = self.diagram_source_error.clone() {
            inspector = inspector.child(empty_card("Pinned source unavailable", &error, p));
        }
        if let (Some(text), Some(display), Some(source)) = (
            &self.diagram_source_text,
            &self.diagram_source_display,
            &self.diagram_source,
        ) {
            let extension = path_extension(&display.path);
            let mut rows = div().flex().flex_col();
            for (offset, line) in text.lines().enumerate() {
                let number = display.start_line.saturating_add(offset as u32);
                let selected = (source.start_line..=source.end_line).contains(&number);
                rows = rows.child(render_source_row(number, line, extension, p, selected));
            }
            inspector = inspector.child(
                div()
                    .id("diagram-source-code")
                    .track_scroll(&self.diagram_source_scroll)
                    .overflow_scroll()
                    .flex_1()
                    .min_h_0()
                    .child(rows),
            );
        } else if diagram
            .links
            .get(selected)
            .is_some_and(|link| link.source.is_some())
            && !self.diagram_source_loading
        {
            inspector = inspector.child(empty_card(
                "Source not loaded",
                "Select this interaction again to read its pinned source range.",
                p,
            ));
        } else if diagram
            .links
            .get(selected)
            .is_none_or(|link| link.source.is_none())
        {
            inspector = inspector.child(empty_card(
                "No pinned source range",
                "This interaction has no source link in the committed result.",
                p,
            ));
        }
        workspace.child(
            div()
                .id("sequence-source-pair")
                .w_full()
                .min_w_0()
                .flex()
                .gap_2()
                .h(px(pair_height))
                .child(diagram_panel)
                .child(inspector),
        )
    }

    fn select_finding_source(&mut self, source: SourceRef, cx: &mut Context<Self>) {
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        if let Some(control) = self.finding_source_control.take() {
            control.cancel();
        }
        let token = self.finding_source_epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let epoch = self.finding_source_epoch.clone();
        let checkout = binding.checkout.clone();
        let context = binding.context.clone();
        let display = source_context_range(&source, 4);
        let control = OperationControl::with_timeout(Duration::from_secs(10));
        self.finding_source_control = Some(control.clone());
        self.finding_source = Some(source.clone());
        self.finding_source_display = Some(display.clone());
        self.finding_source_text = None;
        self.finding_source_error = None;
        self.finding_source_loading = true;
        self.finding_prior_source = None;
        self.prior_source_epoch.fetch_add(1, Ordering::Relaxed);
        self.prior_source_preview = None;
        self.finding_source_scroll.set_offset(point(px(0.), px(0.)));
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    read_source_context(
                        Path::new(&checkout),
                        &context,
                        &display,
                        source.start_line,
                        &control,
                    )
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if epoch.load(Ordering::Relaxed) != token {
                    return;
                }
                this.finding_source_loading = false;
                this.finding_source_control = None;
                match result {
                    Ok((display, text)) => {
                        this.finding_source_display = Some(display);
                        this.finding_source_text = Some(text);
                    }
                    Err(error) => {
                        this.finding_source_error =
                            Some(format!("{}: {}", error.code, error.message));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn load_prior_source(&mut self, source: bodh::PriorConcernEvidence, cx: &mut Context<Self>) {
        let key = format!(
            "{}:{}:{}-{}",
            source.commit_sha, source.source.path, source.source.start_line, source.source.end_line
        );
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        if self
            .prior_source_preview
            .as_ref()
            .is_some_and(|preview| preview.key == key && preview.text.is_some())
        {
            self.prior_source_preview = None;
            self.finding_prior_source = None;
            cx.notify();
            return;
        }
        let token = self.prior_source_epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let epoch = self.prior_source_epoch.clone();
        let repository = binding.checkout.clone();
        let preparation_id = binding.preparation_id.clone();
        let expected_context = binding.context.clone();
        self.prior_source_preview = Some(PriorSourcePreview {
            key: key.clone(),
            loading: true,
            text: None,
            error: None,
        });
        self.finding_prior_source = Some(source.clone());
        if let Some(control) = self.finding_source_control.take() {
            control.cancel();
        }
        self.finding_source_epoch.fetch_add(1, Ordering::Relaxed);
        self.finding_source_loading = false;
        self.finding_source_text = None;
        self.finding_source = None;
        self.finding_source = None;
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    bodh::read_source_at_commit(
                        Path::new(&repository),
                        &source.commit_sha,
                        &source.source,
                        &bodh::OperationControl::default(),
                    )
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if epoch.load(Ordering::Relaxed) != token
                    || this.selected_review.as_ref().is_none_or(|current| {
                        current.preparation_id != preparation_id
                            || current.context != expected_context
                    })
                {
                    return;
                }
                this.prior_source_preview = Some(match result {
                    Ok(text) => PriorSourcePreview {
                        key,
                        loading: false,
                        text: Some(text),
                        error: None,
                    },
                    Err(error) => PriorSourcePreview {
                        key,
                        loading: false,
                        text: None,
                        error: Some(format!("{}: {}", error.code, error.message)),
                    },
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_findings(
        &self,
        binding: &PreparedBinding,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current = self
            .committed_overview
            .committed
            .as_ref()
            .filter(|review| committed_matches_context(review, &binding.context));
        let Some(review) = current else {
            let (title, message) = match self.committed_overview.status {
                CommittedOverviewStatus::StaleContext => (
                    "Earlier result hidden",
                    "Findings from another pinned context are not shown here.",
                ),
                CommittedOverviewStatus::LoadFailed => (
                    "Generated findings unavailable",
                    "The committed result could not be read. A prior result remains visible after same-context refresh failures when available.",
                ),
                CommittedOverviewStatus::Missing | CommittedOverviewStatus::Ready => (
                    if self.overview_loading {
                        "Loading committed findings"
                    } else {
                        "No committed findings"
                    },
                    "No complete generated result is attached to this pinned preparation.",
                ),
            };
            return div()
                .id("generated-findings")
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .flex_1()
                .min_h_0()
                .child(empty_card(title, message, p))
                .child(button(
                    "refresh-committed-findings",
                    if self.overview_loading {
                        "Refreshing…"
                    } else {
                        "Refresh result"
                    },
                    p,
                    |this, _, _, cx| this.refresh_committed_overview(cx),
                    cx,
                ));
        };

        let prioritized = bodh::prioritized_finding_indices(&review.result.findings);
        let selected_index = prioritized
            .iter()
            .copied()
            .find(|index| {
                self.selected_finding_id.as_deref()
                    == Some(review.result.findings[*index].id.as_str())
            })
            .or_else(|| prioritized.first().copied());
        let selected_id = selected_index.map(|index| review.result.findings[index].id.clone());
        let mut list = div()
            .id("finding-list")
            .w(px(310.))
            .flex_shrink_0()
            .min_w_0()
            .h_full()
            .overflow_scroll()
            .px_3()
            .pt(px(18.))
            .pb_3()
            .flex()
            .flex_col()
            .gap_2()
            .border_r_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.canvas))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(p.muted))
                    .child(format!("ACTIONABLE · {}", prioritized.len())),
            );
        if prioritized.is_empty() {
            list = list.child(
                div()
                    .text_color(rgb(p.muted))
                    .child("No findings recorded."),
            );
        }
        for (position, index) in prioritized.iter().enumerate() {
            let finding = &review.result.findings[*index];
            let selected = selected_id.as_deref() == Some(finding.id.as_str());
            let is_new = review
                .context
                .prior_concerns
                .as_ref()
                .is_some_and(|prior| !prior.iter().any(|concern| concern.id == finding.id));
            let metadata = format!("{} · {:?} confidence", finding.id, finding.confidence);
            let id = finding.id.clone();
            let mut row = div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(if selected { p.selection } else { p.panel }))
                .child(status_badge(
                    &format!("{:?}", finding.severity),
                    severity_color(&finding.severity, p),
                ));
            if is_new {
                row = row.child(status_badge("New", p.accent));
            }
            row = row
                .child(
                    button(
                        &format!("finding-select-{position}"),
                        &finding.title,
                        p,
                        move |this, _, _, cx| this.select_finding(id.clone(), cx),
                        cx,
                    )
                    .w_full()
                    .text_left()
                    .border_0()
                    .bg(transparent_black())
                    .text_size(px(13.)),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.muted))
                        .child(metadata),
                );
            list = list.child(row);
        }

        let concerns = follow_up_concerns(review).unwrap_or_default();
        let unresolved = concerns
            .iter()
            .filter(|row| row.prominence.is_prominent())
            .collect::<Vec<_>>();
        let resolved = concerns
            .iter()
            .filter(|row| !row.prominence.is_prominent())
            .collect::<Vec<_>>();
        if !unresolved.is_empty() || !resolved.is_empty() {
            list = list.child(
                div()
                    .mt_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .child(section_title("Follow-up", p)),
            );
            for (index, row) in unresolved.iter().enumerate() {
                let status = row.prominence.label();
                if let Some(finding) = row.current_finding {
                    let id = finding.id.clone();
                    list = list.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .p_2()
                            .border_1()
                            .border_color(rgb(p.border))
                            .bg(rgb(p.panel))
                            .child(status_badge(status, follow_up_color(row.prominence, p)))
                            .child(
                                button(
                                    &format!("follow-up-select-{index}"),
                                    &row.concern.title,
                                    p,
                                    move |this, _, _, cx| this.select_finding(id.clone(), cx),
                                    cx,
                                )
                                .w_full()
                                .text_left(),
                            ),
                    );
                } else {
                    list = list.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .p_2()
                            .border_1()
                            .border_color(rgb(p.border))
                            .bg(rgb(p.panel))
                            .child(status_badge(status, follow_up_color(row.prominence, p)))
                            .child(row.concern.title.clone()),
                    );
                }
            }
            if !resolved.is_empty() {
                let resolved_expanded = resolved
                    .iter()
                    .any(|row| self.expanded_follow_up.contains(&row.concern.id));
                let toggle_ids = resolved
                    .iter()
                    .map(|row| row.concern.id.clone())
                    .collect::<Vec<_>>();
                list = list.child(
                    button(
                        "toggle-resolved-prior-concerns",
                        &format!(
                            "Resolved prior concerns · {} {}",
                            resolved.len(),
                            if resolved_expanded { "▾" } else { "▸" }
                        ),
                        p,
                        move |this, _, _, cx| {
                            for id in &toggle_ids {
                                if resolved_expanded {
                                    this.expanded_follow_up.remove(id);
                                } else {
                                    this.expanded_follow_up.insert(id.clone());
                                }
                            }
                            cx.notify();
                        },
                        cx,
                    )
                    .w_full()
                    .text_left()
                    .mt_2(),
                );
                if resolved_expanded {
                    let mut resolved_list = div().flex().flex_col().gap_2();
                    for (index, row) in resolved.iter().enumerate() {
                        resolved_list = resolved_list.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .p_2()
                                .border_1()
                                .border_color(rgb(p.border))
                                .bg(rgb(p.panel))
                                .child(status_badge(
                                    row.prominence.label(),
                                    follow_up_color(row.prominence, p),
                                ))
                                .child(
                                    div()
                                        .text_color(rgb(p.muted))
                                        .child(row.concern.title.clone()),
                                ),
                        );
                        if let Some(source) = row.concern.evidence.first().cloned() {
                            resolved_list = resolved_list.child(button(
                                &format!("resolved-prior-source-{index}"),
                                "View prior source",
                                p,
                                move |this, _, _, cx| this.load_prior_source(source.clone(), cx),
                                cx,
                            ));
                        }
                    }
                    list = list.child(
                        div()
                            .child(resolved_list)
                            .id("resolved-prior-concerns")
                            .p_2()
                            .bg(rgb(p.raised)),
                    );
                }
            }
        }

        let mut detail = div()
            .id("finding-detail")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .overflow_scroll()
            .p_6()
            .flex()
            .flex_col()
            .gap_3();
        if let Some(index) = selected_index {
            let finding = &review.result.findings[index];
            let is_new = review
                .context
                .prior_concerns
                .as_ref()
                .is_some_and(|prior| !prior.iter().any(|concern| concern.id == finding.id));
            let mut detail_status = div()
                .flex()
                .items_center()
                .gap_2()
                .child(status_badge(
                    &format!("{:?}", finding.severity),
                    severity_color(&finding.severity, p),
                ))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(p.muted))
                        .child(format!("{:?} confidence", finding.confidence)),
                );
            if is_new {
                detail_status = detail_status.child(status_badge("New concern", p.accent));
            }
            detail = detail
                .child(detail_status)
                .child(
                    div()
                        .text_size(px(20.))
                        .font_weight(FontWeight::BOLD)
                        .child(finding.title.clone()),
                )
                .child(finding.summary.clone())
                .child(finding.reason.clone())
                .child(finding.impact.clone());
            let triage_entry = self.finding_triage_snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .entries
                    .iter()
                    .find(|entry| entry.finding_id == finding.id)
            });
            let selected_suggestion =
                triage_entry.and_then(|entry| entry.selected_suggestion_id.as_deref());
            let disposition = triage_entry
                .map(|entry| format!("{:?}", entry.disposition))
                .unwrap_or_else(|| "Unreviewed".into());
            detail = detail.child(
                div()
                    .text_color(rgb(p.muted))
                    .child(match selected_suggestion {
                        Some(suggestion_id) => {
                            format!("{disposition} · alternative {suggestion_id}")
                        }
                        None if triage_entry
                            .and_then(|entry| entry.manual_feedback.as_ref())
                            .is_some() =>
                        {
                            format!("{disposition} · independent feedback saved locally")
                        }
                        None => disposition,
                    }),
            );
            if !finding.evidence.is_empty() {
                detail = detail.child(section_title("Source evidence", p));
            }
            for (evidence_index, evidence) in finding.evidence.iter().enumerate() {
                let source = evidence.source.clone();
                detail = detail.child(
                    div()
                        .p_3()
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.panel))
                        .child(
                            div()
                                .text_color(rgb(p.muted))
                                .child(evidence.explanation.clone()),
                        )
                        .child(link_button(
                            &format!("finding-evidence-{evidence_index}"),
                            &format!(
                                "{} · {}:{}–{}",
                                source_title(&source),
                                source.path,
                                source.start_line,
                                source.end_line
                            ),
                            p,
                            move |this, _, _, cx| this.select_finding_source(source.clone(), cx),
                            cx,
                        )),
                );
            }

            let mut source_panel = div()
                .id("finding-source-context")
                .mt_1()
                .p_3()
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.canvas))
                .flex()
                .flex_col()
                .gap_2();
            let source_header = div().flex().items_center().justify_between().gap_3().child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(p.muted))
                            .child("SOURCE CODE"),
                    )
                    .child(
                        div().text_size(px(12.)).text_color(rgb(p.muted)).child(
                            self.finding_prior_source
                                .as_ref()
                                .map(|prior| {
                                    format!(
                                        "Prior revision · {}:{}–{} · {}",
                                        prior.source.path,
                                        prior.source.start_line,
                                        prior.source.end_line,
                                        short_sha(&prior.commit_sha)
                                    )
                                })
                                .or_else(|| {
                                    self.finding_source.as_ref().map(|source| {
                                        format!(
                                            "{} · {}:{}–{}",
                                            match source.revision {
                                                Revision::Base => "Base",
                                                Revision::Head => "Head",
                                            },
                                            source.path,
                                            source.start_line,
                                            source.end_line
                                        )
                                    })
                                })
                                .unwrap_or_else(|| "Select evidence to read its source".into()),
                        ),
                    ),
            );
            let source_header = if let Some(source) = self.finding_source.clone() {
                source_header.child(link_button(
                    "open-finding-source-in-changes",
                    "Open in Changes →",
                    p,
                    move |this, _, _, cx| this.open_overview_evidence(source.clone(), cx),
                    cx,
                ))
            } else {
                source_header
            };
            source_panel = source_panel.child(source_header);

            if let Some(preview) = self.prior_source_preview.as_ref() {
                if preview.loading {
                    source_panel = source_panel.child(
                        div()
                            .text_color(rgb(p.muted))
                            .child("Reading the exact prior revision…"),
                    );
                }
                if let Some(error) = &preview.error {
                    source_panel =
                        source_panel.child(empty_card("Prior source unavailable", error, p));
                }
                if let (Some(text), Some(prior)) =
                    (preview.text.as_ref(), self.finding_prior_source.as_ref())
                {
                    let mut rows = div().flex().flex_col();
                    for (offset, line) in text.lines().enumerate() {
                        let number = prior.source.start_line.saturating_add(offset as u32);
                        let selected =
                            (prior.source.start_line..=prior.source.end_line).contains(&number);
                        rows = rows.child(render_source_row(
                            number,
                            line,
                            path_extension(&prior.source.path),
                            p,
                            selected,
                        ));
                    }
                    source_panel = source_panel.child(
                        div()
                            .id("finding-prior-source-code")
                            .max_h(px(420.))
                            .overflow_scroll()
                            .child(rows),
                    );
                }
            } else if self.finding_source_loading {
                source_panel = source_panel.child(
                    div()
                        .text_color(rgb(p.muted))
                        .child("Reading source from the selected revision…"),
                );
            } else if let Some(error) = self.finding_source_error.as_ref() {
                source_panel = source_panel.child(empty_card("Source unavailable", error, p));
            } else if let (Some(text), Some(display), Some(source)) = (
                self.finding_source_text.as_ref(),
                self.finding_source_display.as_ref(),
                self.finding_source.as_ref(),
            ) {
                let extension = path_extension(&display.path);
                let mut rows = div().flex().flex_col();
                for (offset, line) in text.lines().enumerate() {
                    let number = display.start_line.saturating_add(offset as u32);
                    let selected = (source.start_line..=source.end_line).contains(&number);
                    rows = rows.child(render_source_row(number, line, extension, p, selected));
                }
                source_panel = source_panel.child(
                    div()
                        .id("finding-source-code")
                        .track_scroll(&self.finding_source_scroll)
                        .max_h(px(420.))
                        .overflow_scroll()
                        .child(rows),
                );
            } else {
                source_panel = source_panel.child(empty_card(
                    "Select source evidence",
                    "Choose an evidence or source link above to read the exact source for this finding.",
                    p,
                ));
            }
            detail = detail.child(source_panel);

            let follow_up = follow_up_concerns(review).and_then(|rows| {
                rows.into_iter().find(|row| {
                    row.current_finding
                        .is_some_and(|current| current.id == finding.id)
                })
            });
            if let Some(row) = follow_up {
                if let Some(assessment) = row.assessment {
                    detail = detail.child(
                        div()
                            .p_3()
                            .border_1()
                            .border_color(rgb(p.border))
                            .bg(rgb(p.panel))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(section_title("Follow-up", p))
                                    .child(status_badge(
                                        row.prominence.label(),
                                        follow_up_color(row.prominence, p),
                                    )),
                            )
                            .child(row.concern.title.clone())
                            .child(assessment.explanation.clone()),
                    );
                }
            }

            if !finding.related_thread_ids.is_empty() {
                detail = detail.child(section_title("Related discussions", p));
                for (thread_index, thread_id) in finding.related_thread_ids.iter().enumerate() {
                    let related_id = thread_id.clone();
                    detail = detail.child(link_button(
                        &format!("finding-related-thread-{thread_index}"),
                        &format!("Open thread {thread_id} in Changes"),
                        p,
                        move |this, _, _, cx| this.open_related_discussion(related_id.clone(), cx),
                        cx,
                    ));
                }
            }
            if !finding.suggestions.is_empty() {
                detail = detail.child(section_title(
                    "Suggested feedback · choose an alternative",
                    p,
                ));
            }
            for (suggestion_index, suggestion) in finding.suggestions.iter().enumerate() {
                let (suggestion_id, generated_body, source) = match suggestion {
                    bodh::Suggestion::TextComment { id, body, source } => {
                        (id.clone(), body.clone(), source.clone())
                    }
                    bodh::Suggestion::CodeReplacement {
                        id,
                        body,
                        source,
                        replacement,
                        ..
                    } => (
                        id.clone(),
                        format!("{body}\n\nProposed replacement:\n{replacement}"),
                        source.clone(),
                    ),
                };
                let saved_edit = self.finding_triage_snapshot.as_ref().and_then(|snapshot| {
                    snapshot.alternative_edits.iter().find(|edit| {
                        edit.finding_id == finding.id && edit.suggestion_id == suggestion_id
                    })
                });
                let label = match (suggestion, saved_edit) {
                    (bodh::Suggestion::TextComment { .. }, Some(edit)) => edit.body.clone(),
                    (bodh::Suggestion::CodeReplacement { .. }, Some(edit)) => format!(
                        "{}\n\nProposed replacement:\n{}",
                        edit.body,
                        edit.replacement.as_deref().unwrap_or_default()
                    ),
                    (_, None) => generated_body,
                };
                detail = detail.child(
                    div()
                        .p_3()
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.raised))
                        .child(div().text_color(rgb(p.muted)).child(format!(
                            "Alternative {}{} · {}",
                            suggestion_index + 1,
                            if saved_edit.is_some() {
                                " · edited locally"
                            } else {
                                ""
                            },
                            source_title(&source)
                        )))
                        .child(label),
                );
                let source_to_open = source.clone();
                detail = detail.child(link_button(
                    &format!("finding-alternative-source-{suggestion_index}"),
                    "View source",
                    p,
                    move |this, _, _, cx| this.select_finding_source(source_to_open.clone(), cx),
                    cx,
                ));
                let is_selected = selected_suggestion == Some(suggestion_id.as_str());
                let action_finding = finding.id.clone();
                let action_suggestion = suggestion_id.clone();
                detail = detail.child(button(
                    &format!("finding-alternative-stage-{suggestion_index}"),
                    if is_selected {
                        "Unstage alternative"
                    } else {
                        "Stage alternative"
                    },
                    p,
                    move |this, _, _, cx| {
                        if is_selected {
                            this.set_finding_disposition(
                                action_finding.clone(),
                                bodh::FindingDisposition::Unreviewed,
                                cx,
                            );
                        } else {
                            this.stage_finding_alternative(
                                action_finding.clone(),
                                action_suggestion.clone(),
                                cx,
                            );
                        }
                    },
                    cx,
                ));
            }
            for (location_index, location) in finding.locations.iter().enumerate() {
                let source = location.source.clone();
                detail = detail.child(link_button(
                    &format!("finding-location-{location_index}"),
                    &format!(
                        "{} source · {}",
                        if location.primary {
                            "Primary"
                        } else {
                            "Related"
                        },
                        source_title(&source)
                    ),
                    p,
                    move |this, _, _, cx| this.select_finding_source(source.clone(), cx),
                    cx,
                ));
            }
            for (action, label) in [
                (bodh::FindingDisposition::Dismissed, "Dismiss finding"),
                (bodh::FindingDisposition::Deferred, "Defer finding"),
            ] {
                let id = finding.id.clone();
                detail = detail.child(button(
                    &format!("finding-disposition-{label}"),
                    label,
                    p,
                    move |this, _, _, cx| this.set_finding_disposition(id.clone(), action, cx),
                    cx,
                ));
            }
        } else {
            detail = detail.child(empty_card(
                "No findings to show",
                "This result has no actionable findings. Follow-up concerns, when present, appear in the list.",
                p,
            ));
        }

        if let Some(index) = selected_index {
            let finding = &review.result.findings[index];
            let finding_id = finding.id.clone();
            let current_feedback = self
                .finding_triage_snapshot
                .as_ref()
                .and_then(|snapshot| {
                    snapshot
                        .entries
                        .iter()
                        .find(|entry| entry.finding_id == finding.id)
                })
                .and_then(|entry| entry.manual_feedback.as_deref())
                .unwrap_or("")
                .to_owned();
            detail = detail
                .child(div().mt_2().pt_3().border_t_1().border_color(rgb(p.border)))
                .child(section_title("Add a comment", p))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.muted))
                        .child("Saved locally with this finding. Nothing is sent to GitHub."),
                );
            if self.finding_feedback_target.as_deref() == Some(finding.id.as_str()) {
                let feedback_display = if self.finding_feedback_body.is_empty() {
                    "|Type a review comment. Enter inserts a new line.".to_owned()
                } else {
                    format!("{}|", self.finding_feedback_body)
                };
                detail = detail.child(
                    div()
                        .id("finding-independent-feedback-editor")
                        .track_focus(&self.finding_feedback_focus)
                        .min_h(px(72.))
                        .p_2()
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.raised))
                        .child(feedback_display)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, _| {
                                window.focus(&this.finding_feedback_focus)
                            }),
                        )
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            window.focus(&this.finding_feedback_focus);
                            let key = event.keystroke.key.as_str();
                            let command = event.keystroke.modifiers.control
                                || event.keystroke.modifiers.platform;
                            match key {
                                "backspace" => {
                                    this.finding_feedback_body.pop();
                                }
                                "enter" => this.finding_feedback_body.push('\n'),
                                "a" if command => this.finding_feedback_body.clear(),
                                text if text.chars().count() == 1 && !command => {
                                    this.finding_feedback_body.push_str(text)
                                }
                                _ => return,
                            }
                            cx.notify();
                        })),
                );
                let save_finding = finding_id.clone();
                detail = detail.child(button(
                    "save-finding-feedback",
                    if self.finding_action_saving {
                        "Saving locally…"
                    } else {
                        "Save comment locally"
                    },
                    p,
                    move |this, _, _, cx| this.save_finding_feedback(save_finding.clone(), cx),
                    cx,
                ));
            } else {
                let open_finding = finding_id;
                detail = detail.child(button(
                    "edit-finding-feedback",
                    if current_feedback.is_empty() {
                        "Add comment"
                    } else {
                        "Edit local comment"
                    },
                    p,
                    move |this, _, window, cx| {
                        this.finding_feedback_target = Some(open_finding.clone());
                        this.finding_feedback_body = this
                            .finding_triage_snapshot
                            .as_ref()
                            .and_then(|snapshot| {
                                snapshot
                                    .entries
                                    .iter()
                                    .find(|entry| entry.finding_id == open_finding)
                            })
                            .and_then(|entry| entry.manual_feedback.clone())
                            .unwrap_or_default();
                        window.focus(&this.finding_feedback_focus);
                        cx.notify();
                    },
                    cx,
                ));
            }
        }

        div()
            .id("generated-findings")
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .min_w_0()
            .overflow_hidden()
            .child(list)
            .child(detail)
    }

    fn stage_finding_alternative(
        &mut self,
        finding_id: String,
        suggestion_id: String,
        cx: &mut Context<Self>,
    ) {
        let locked = self.finding_action_saving
            || self.review_plan.as_ref().is_some_and(|plan| {
                matches!(plan.state.as_str(), "planned" | "sending" | "unknown")
            });
        if locked {
            self.status = Some(
                "Resolve the current or uncertain publication before changing finding drafts."
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        let expected_triage_revision = self
            .finding_triage_snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .entries
                    .iter()
                    .find(|entry| entry.finding_id == finding_id)
            })
            .map(|entry| entry.revision)
            .unwrap_or(0);
        let state = self.preparation_db.clone();
        let preparation_id = binding.preparation_id.clone();
        let token = self.operation_epoch.load(Ordering::Relaxed);
        self.finding_action_saving = true;
        cx.spawn(async move |weak, cx| {
            let result = cx.background_executor().spawn(async move {
                let mut triage = bodh::FindingTriageStore::open(Path::new(&state))?;
                let (intent, entry) = triage.stage_alternative(
                    &preparation_id, &finding_id, &suggestion_id, expected_triage_revision,
                )?;
                let triage_snapshot = triage.list(&preparation_id)?;
                let drafts = DraftStore::open(Path::new(&state))?.list(&preparation_id)?;
                let replacements = DraftStore::open(Path::new(&state))?.list_replacements(&preparation_id)?;
                Ok::<_, bodh::ContractError>((intent, entry, triage_snapshot, drafts, replacements))
            }).await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token { return; }
                this.finding_action_saving = false;
                match result {
                    Ok((intent, entry, triage, drafts, replacements)) => {
                        this.review_intent = Some(intent);
                        this.finding_triage_snapshot = Some(triage);
                        this.draft_snapshot = Some(drafts.clone());
                        this.replacement_snapshot = Some(replacements);
                        this.update_draft_counts(drafts.counts);
                        this.status = Some(format!("Alternative staged for finding {} at revision {}. It remains local until you confirm Finish Review.", entry.finding_id, entry.revision));
                    }
                    Err(error) => this.error = Some(format!("{}: {}", error.code, error.message)),
                }
                cx.notify();
            });
        }).detach();
    }

    fn set_finding_disposition(
        &mut self,
        finding_id: String,
        disposition: bodh::FindingDisposition,
        cx: &mut Context<Self>,
    ) {
        if self.finding_action_saving
            || self.review_plan.as_ref().is_some_and(|plan| {
                matches!(plan.state.as_str(), "planned" | "sending" | "unknown")
            })
        {
            self.status = Some(
                "Resolve the current or uncertain publication before changing finding drafts."
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        let expected_revision = self
            .finding_triage_snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .entries
                    .iter()
                    .find(|entry| entry.finding_id == finding_id)
            })
            .map(|entry| entry.revision)
            .unwrap_or(0);
        let state = self.preparation_db.clone();
        let preparation_id = binding.preparation_id.clone();
        let token = self.operation_epoch.load(Ordering::Relaxed);
        self.finding_action_saving = true;
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut triage = bodh::FindingTriageStore::open(Path::new(&state))?;
                    let entry = triage.set_disposition(
                        &preparation_id,
                        &finding_id,
                        disposition,
                        None,
                        None,
                        expected_revision,
                    )?;
                    let snapshot = triage.list(&preparation_id)?;
                    let intent =
                        ReviewPublicationStore::open(Path::new(&state))?.intent(&preparation_id)?;
                    Ok::<_, bodh::ContractError>((entry, snapshot, intent))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token {
                    return;
                }
                this.finding_action_saving = false;
                match result {
                    Ok((entry, snapshot, intent)) => {
                        this.finding_triage_snapshot = Some(snapshot);
                        this.review_intent = Some(intent);
                        this.status = Some(format!(
                            "Finding {} marked {:?}; saved alternative edits were retained.",
                            entry.finding_id, entry.disposition
                        ));
                    }
                    Err(error) => this.error = Some(format!("{}: {}", error.code, error.message)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_finding_feedback(&mut self, finding_id: String, cx: &mut Context<Self>) {
        if self.finding_action_saving
            || self.review_plan.as_ref().is_some_and(|plan| {
                matches!(plan.state.as_str(), "planned" | "sending" | "unknown")
            })
        {
            self.status = Some(
                "Resolve the current or uncertain publication before changing finding feedback."
                    .into(),
            );
            cx.notify();
            return;
        }
        let body = self.finding_feedback_body.clone();
        if body.trim().is_empty() {
            self.error = Some("Write independent feedback before saving it.".into());
            cx.notify();
            return;
        }
        let Some(binding) = self.selected_review.clone() else {
            return;
        };
        let expected_revision = self
            .finding_triage_snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .entries
                    .iter()
                    .find(|entry| entry.finding_id == finding_id)
            })
            .map(|entry| entry.revision)
            .unwrap_or(0);
        let state = self.preparation_db.clone();
        let preparation_id = binding.preparation_id.clone();
        let token = self.operation_epoch.load(Ordering::Relaxed);
        self.finding_action_saving = true;
        cx.spawn(async move |weak, cx| {
            let result = cx.background_executor().spawn(async move {
                let mut store = bodh::FindingTriageStore::open(Path::new(&state))?;
                let entry = store.set_disposition(&preparation_id, &finding_id, bodh::FindingDisposition::Staged, None, Some(&body), expected_revision)?;
                let snapshot = store.list(&preparation_id)?;
                Ok::<_, bodh::ContractError>((entry,snapshot))
            }).await;
            let _ = weak.update(cx, |this,cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token { return; }
                this.finding_action_saving = false;
                match result {
                    Ok((entry,snapshot)) => {
                        this.finding_triage_snapshot = Some(snapshot);
                        this.finding_feedback_target = None;
                        this.finding_feedback_body.clear();
                        this.status = Some(format!("Independent feedback saved locally for finding {}. It is not published.",entry.finding_id));
                    }
                    Err(error) => this.error = Some(format!("{}: {}",error.code,error.message)),
                }
                cx.notify();
            });
        }).detach();
    }

    fn selected_discussion_path(&self) -> Option<String> {
        if let Some(source) = &self.source_ref {
            return Some(source.path.clone());
        }
        let binding = self.selected_review.as_ref()?;
        let file = binding.changed_files.get(self.selected_file?)?;
        file.new_path
            .as_deref()
            .or(file.old_path.as_deref())
            .map(str::to_owned)
    }

    fn load_selected_file_discussions(&mut self, cx: &mut Context<Self>) {
        if self.discussion_loading {
            return;
        }
        let (Some(binding), Some(path)) = (
            self.selected_review.clone(),
            self.selected_discussion_path(),
        ) else {
            return;
        };
        if self.discussion_loaded_for.as_deref() == Some(path.as_str()) {
            return;
        }
        let state = self.preparation_db.clone();
        let preparation_id = binding.preparation_id.clone();
        let path_for_ui = path.clone();
        let token = self.operation_epoch.load(Ordering::Relaxed);
        self.discussion_loading = true;
        self.discussion_state = serde_json::json!({"available":false,"threads":[]});
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    bodh::load_latest_discussions(&state, &preparation_id)
                        .map(|observation| discussion_observation_for_path(observation, &path))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token {
                    return;
                }
                this.discussion_loading = false;
                if !this.discussion_expanded
                    || this.selected_discussion_path().as_deref() != Some(path_for_ui.as_str())
                {
                    this.discussion_state = serde_json::json!({"available":false,"threads":[]});
                    this.discussion_loaded_for = None;
                    if this.discussion_expanded {
                        this.load_selected_file_discussions(cx);
                    }
                    cx.notify();
                    return;
                }
                match result {
                    Ok(observation) => {
                        this.discussion_state = observation;
                        this.discussion_loaded_for = Some(path_for_ui);
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

    fn render_inline_discussions(
        &mut self,
        selected_path: &str,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut view = div()
            .id("inline-discussions")
            .flex()
            .flex_col()
            .gap_2()
            .mt_4()
            .pt_3()
            .border_t_1()
            .border_color(rgb(p.border));
        let mut heading = div()
            .flex()
            .items_center()
            .justify_between()
            .child(section_title(
                &format!("Review discussion · {selected_path}"),
                p,
            ));
        let expanded = self.discussion_expanded;
        heading = heading.child(button(
            "toggle-inline-discussions",
            if expanded { "Collapse" } else { "Show" },
            p,
            |this, _, _, cx| {
                this.discussion_expanded = !this.discussion_expanded;
                this.discussion_state = serde_json::json!({"available":false,"threads":[]});
                this.discussion_loaded_for = None;
                if this.discussion_expanded {
                    this.load_selected_file_discussions(cx);
                }
                cx.notify();
            },
            cx,
        ));
        view = view.child(heading);
        if !self.discussion_expanded {
            return view;
        }
        let summary = format!(
            "Read-only observation · {}",
            if self.discussion_state["complete"] == true {
                "complete"
            } else {
                "partial or not refreshed"
            }
        );
        view = view.child(
            div()
                .text_size(px(12.))
                .text_color(rgb(p.muted))
                .child(summary),
        );
        view = view.child(button(
            "refresh-discussions",
            if self.discussion_loading {
                "Loading discussions…"
            } else {
                if self.discussion_state["available"] == true {
                    "Refresh discussion"
                } else {
                    "Load discussion for this file"
                }
            },
            p,
            |this, _, _, cx| this.refresh_discussions(cx),
            cx,
        ));
        if self.discussion_state["available"] != true {
            view=view.child(empty_card("Discussion not loaded","Load the read-only GitHub discussion observation to show comments beside this source.",p));
            return view;
        }
        let threads = self.discussion_state["threads"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        view = view.child(
            div()
                .text_size(px(11.))
                .text_color(rgb(p.muted))
                .child(format!("{} thread(s) on this file", threads.len())),
        );
        if threads.is_empty() {
            return view.child(empty_card(
                "No discussion on this source",
                "Review threads for the selected source appear here beside its pinned code.",
                p,
            ));
        }
        for (thread_index, thread) in threads.into_iter().enumerate() {
            let id = thread["id"].as_str().unwrap_or_default().to_owned();
            let path = thread["path"].as_str().unwrap_or("location unavailable");
            let current = if thread["isOutdated"] == true {
                "unavailable (outdated)".into()
            } else {
                thread["line"]
                    .as_u64()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unavailable".into())
            };
            let original = thread["originalLine"]
                .as_u64()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "unavailable".into());
            let status = if thread["isOutdated"] == true {
                "Outdated"
            } else if thread["isResolved"] == true {
                "Resolved"
            } else {
                "Unresolved"
            };
            let mut card = div()
                .id(("discussion-thread", thread_index))
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .border_1()
                .border_color(rgb(
                    if self.focused_discussion_thread.as_deref() == Some(id.as_str()) {
                        p.accent
                    } else {
                        p.border
                    },
                ))
                .bg(rgb(p.panel));
            card=card.child(div().flex().justify_between().child(section_title(path,p)).child(div().text_size(px(11.)).text_color(rgb(p.muted)).child(format!("{status} · current line {current} · original line {original} · side {}",thread["diffSide"].as_str().unwrap_or("unknown")))));
            if let Some(comments) = thread["comments"].as_array() {
                for comment in comments {
                    let author = comment["author"]["login"]
                        .as_str()
                        .or(comment["author"]["name"].as_str())
                        .unwrap_or("Unknown author");
                    let body = comment["body"].as_str().unwrap_or("(empty comment)");
                    card = card.child(
                        div()
                            .p_2()
                            .bg(rgb(p.raised))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.muted))
                                    .child(author.to_owned()),
                            )
                            .child(div().mt_1().text_size(px(12.)).child(body.to_owned())),
                    );
                }
            }
            let mut actions = div().flex().items_center().gap_2();
            if thread["viewerCanReply"] == true
                && let Some(root) = thread["comments"]
                    .as_array()
                    .and_then(|items| items.iter().find(|comment| comment["replyTo"].is_null()))
                    .and_then(|comment| comment["id"].as_str())
                    .map(str::to_owned)
            {
                let reply_thread_id = id.clone();
                actions = actions.child(button(
                    &format!("draft-reply-{id}"),
                    "Draft reply",
                    p,
                    move |this, _, window, cx| {
                        this.reply_thread = Some(reply_thread_id.clone());
                        this.reply_root = Some(root.clone());
                        this.reply_draft_id = None;
                        this.reply_draft_revision = 0;
                        this.reply_body.clear();
                        window.focus(&this.reply_focus);
                        cx.notify();
                    },
                    cx,
                ));
            }
            if thread["isResolved"] == false && thread["viewerCanResolve"] == true {
                let action_id = id.clone();
                actions = actions.child(button(
                    &format!("stage-resolve-{id}"),
                    if self.has_thread_action(&id, bodh::ThreadAction::Resolve) {
                        "Resolve pending"
                    } else {
                        "Stage resolve"
                    },
                    p,
                    move |this, _, _, cx| {
                        this.stage_thread_action(action_id.clone(), bodh::ThreadAction::Resolve, cx)
                    },
                    cx,
                ));
            } else if thread["isResolved"] == true && thread["viewerCanUnresolve"] == true {
                let action_id = id.clone();
                actions = actions.child(button(
                    &format!("stage-reopen-{id}"),
                    if self.has_thread_action(&id, bodh::ThreadAction::Reopen) {
                        "Reopen pending"
                    } else {
                        "Stage reopen"
                    },
                    p,
                    move |this, _, _, cx| {
                        this.stage_thread_action(action_id.clone(), bodh::ThreadAction::Reopen, cx)
                    },
                    cx,
                ));
            }
            card = card.child(actions);
            if self.reply_thread.as_deref() == Some(id.as_str()) {
                let body = if self.reply_body.is_empty() {
                    "|Type a reply draft. Enter inserts a line.".to_owned()
                } else {
                    format!("{}|", self.reply_body)
                };
                card = card.child(
                    div()
                        .id("reply-draft-editor")
                        .track_focus(&self.reply_focus)
                        .min_h(px(72.))
                        .p_2()
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.raised))
                        .text_size(px(12.))
                        .child(body)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, _| window.focus(&this.reply_focus)),
                        )
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            window.focus(&this.reply_focus);
                            let key = event.keystroke.key.as_str();
                            let command = event.keystroke.modifiers.control
                                || event.keystroke.modifiers.platform;
                            match key {
                                "backspace" => {
                                    this.reply_body.pop();
                                }
                                "enter" => this.reply_body.push('\n'),
                                "tab" => this.reply_body.push_str("    "),
                                "a" if command => {}
                                text if text.chars().count() == 1 && !command => {
                                    this.reply_body.push_str(text)
                                }
                                _ => return,
                            }
                            cx.notify();
                        })),
                );
                card = card.child(button(
                    &format!("save-reply-{id}"),
                    if self.reply_saving {
                        "Saving…"
                    } else {
                        "Save reply draft locally"
                    },
                    p,
                    |this, _, _, cx| this.save_reply_draft(cx),
                    cx,
                ));
            }
            if let Some(snapshot) = &self.draft_snapshot {
                for draft in snapshot
                    .reply_drafts
                    .iter()
                    .filter(|draft| draft.thread_id == id)
                {
                    let resume_saved = draft.clone();
                    let discard_saved = draft.clone();
                    card = card.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(button(
                                &format!("resume-reply-{}", draft.id),
                                &format!(
                                    "Resume local reply draft · revision {} · not sent",
                                    draft.revision
                                ),
                                p,
                                move |this, _, window, cx| {
                                    this.reply_thread = Some(resume_saved.thread_id.clone());
                                    this.reply_root = Some(resume_saved.root_comment_id.clone());
                                    this.reply_draft_id = Some(resume_saved.id.clone());
                                    this.reply_draft_revision = resume_saved.revision;
                                    this.reply_body = resume_saved.body.clone();
                                    window.focus(&this.reply_focus);
                                    cx.notify();
                                },
                                cx,
                            ))
                            .child(button(
                                &format!("discard-reply-{}", draft.id),
                                "Discard reply draft",
                                p,
                                move |this, _, _, cx| {
                                    this.discard_reply_draft(
                                        discard_saved.id.clone(),
                                        discard_saved.revision,
                                        cx,
                                    )
                                },
                                cx,
                            )),
                    );
                }
                for action in snapshot
                    .thread_actions
                    .iter()
                    .filter(|action| action.thread_id == id)
                {
                    let action_id = action.id.clone();
                    let action_rev = action.revision;
                    card=card.child(div().flex().items_center().gap_2().child(div().text_size(px(11.)).text_color(rgb(p.accent)).child(format!("Pending local intent: {:?} · observed remote state {} · not applied",action.action,if action.observed_resolved{"resolved"}else{"unresolved"}))).child(button(&format!("unstage-thread-{}",action.id),"Unstage",p,move|this,_,_,cx|this.discard_thread_action(action_id.clone(),action_rev,cx),cx)));
                }
            }
            view = view.child(card);
        }
        view
    }

    fn has_thread_action(&self, thread_id: &str, action: bodh::ThreadAction) -> bool {
        self.draft_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .thread_actions
                .iter()
                .any(|pending| pending.thread_id == thread_id && pending.action == action)
        })
    }

    fn refresh_discussions(&mut self, cx: &mut Context<Self>) {
        if self.discussion_loading || !self.discussion_expanded {
            return;
        }
        let (Some(binding), Some(path)) = (
            self.selected_review.clone(),
            self.selected_discussion_path(),
        ) else {
            return;
        };
        let state = self.preparation_db.clone();
        let control = OperationControl::with_timeout(Duration::from_secs(90));
        let epoch = self.operation_epoch.load(Ordering::Relaxed);
        let path_for_ui = path.clone();
        self.discussion_loading = true;
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    bodh::acquire_discussions(&binding, &state, &control)?;
                    bodh::load_latest_discussions(&state, &binding.preparation_id)
                        .map(|observation| discussion_observation_for_path(observation, &path))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != epoch {
                    return;
                }
                this.discussion_loading = false;
                if !this.discussion_expanded
                    || this.selected_discussion_path().as_deref() != Some(path_for_ui.as_str())
                {
                    this.discussion_state = serde_json::json!({"available":false,"threads":[]});
                    this.discussion_loaded_for = None;
                    if this.discussion_expanded {
                        this.load_selected_file_discussions(cx);
                    }
                    cx.notify();
                    return;
                }
                match result {
                    Ok(discussions) => {
                        this.discussion_state = discussions;
                        this.discussion_loaded_for = Some(path_for_ui);
                        this.status = Some(
                            "Read-only discussion observation refreshed. No remote changes were made."
                                .into(),
                        );
                    }
                    Err(error) => this.error = Some(format!("{}: {}", error.code, error.message)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_reply_draft(&mut self, cx: &mut Context<Self>) {
        if self.reply_saving || self.reply_body.trim().is_empty() {
            return;
        }
        let (Some(binding), Some(thread), Some(root), Some(snapshot)) = (
            self.selected_review.clone(),
            self.reply_thread.clone(),
            self.reply_root.clone(),
            self.draft_snapshot.clone(),
        ) else {
            return;
        };
        self.reply_saving = true;
        let state = self.preparation_db.clone();
        let body = self.reply_body.clone();
        let epoch = self.operation_epoch.load(Ordering::Relaxed);
        let draft_id = self.reply_draft_id.clone();
        let draft_revision = self.reply_draft_revision;
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut store = DraftStore::open(Path::new(&state))?;
                    let result = store.save_reply(
                        &binding.preparation_id,
                        draft_id.as_deref(),
                        &thread,
                        &root,
                        &body,
                        snapshot.workspace_revision,
                        draft_revision,
                    )?;
                    let updated = store.list(&binding.preparation_id)?;
                    Ok::<_, bodh::ContractError>((result, updated))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != epoch {
                    return;
                }
                this.reply_saving = false;
                match result {
                    Ok((saved, snapshot)) => {
                        this.draft_snapshot = Some(snapshot.clone());
                        this.update_draft_counts(snapshot.counts);
                        this.reply_thread = None;
                        this.reply_root = None;
                        this.reply_draft_id = None;
                        this.reply_draft_revision = 0;
                        this.reply_body.clear();
                        this.status = Some(format!(
                            "Reply draft saved at workspace revision {}. It has not been sent.",
                            saved.workspace_revision
                        ));
                    }
                    Err(error) => this.error = Some(format!("{}: {}", error.code, error.message)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn discard_reply_draft(&mut self, id: String, revision: u64, cx: &mut Context<Self>) {
        self.discard_discussion_work(id, revision, true, cx)
    }
    fn discard_thread_action(&mut self, id: String, revision: u64, cx: &mut Context<Self>) {
        self.discard_discussion_work(id, revision, false, cx)
    }
    fn discard_discussion_work(
        &mut self,
        id: String,
        revision: u64,
        is_reply: bool,
        cx: &mut Context<Self>,
    ) {
        let (Some(binding), Some(snapshot)) =
            (self.selected_review.clone(), self.draft_snapshot.clone())
        else {
            return;
        };
        let state = self.preparation_db.clone();
        let epoch = self.operation_epoch.load(Ordering::Relaxed);
        let ui_id = id.clone();
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut store = DraftStore::open(Path::new(&state))?;
                    let updated = if is_reply {
                        store.discard_reply(
                            &binding.preparation_id,
                            &id,
                            snapshot.workspace_revision,
                            revision,
                        )?
                    } else {
                        store.discard_thread_action(
                            &binding.preparation_id,
                            &id,
                            snapshot.workspace_revision,
                            revision,
                        )?
                    };
                    Ok::<_, bodh::ContractError>(updated)
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != epoch {
                    return;
                }
                match result {
                    Ok(snapshot) => {
                        this.draft_snapshot = Some(snapshot.clone());
                        this.update_draft_counts(snapshot.counts);
                        if this.reply_draft_id.as_deref() == Some(ui_id.as_str()) {
                            this.reply_thread = None;
                            this.reply_root = None;
                            this.reply_draft_id = None;
                            this.reply_draft_revision = 0;
                            this.reply_body.clear();
                        }
                        this.status = Some(
                            "Local discussion draft discarded. No remote change was made.".into(),
                        );
                    }
                    Err(error) => this.error = Some(format!("{}: {}", error.code, error.message)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn stage_thread_action(
        &mut self,
        thread_id: String,
        action: bodh::ThreadAction,
        cx: &mut Context<Self>,
    ) {
        let (Some(binding), Some(snapshot)) =
            (self.selected_review.clone(), self.draft_snapshot.clone())
        else {
            return;
        };
        let existing = snapshot
            .thread_actions
            .iter()
            .find(|item| item.thread_id == thread_id)
            .cloned();
        let state = self.preparation_db.clone();
        let epoch = self.operation_epoch.load(Ordering::Relaxed);
        cx.spawn(async move|weak,cx|{let result=cx.background_executor().spawn(async move{let mut store=DraftStore::open(Path::new(&state))?;let saved=store.stage_thread_action(&binding.preparation_id,existing.as_ref().map(|item|item.id.as_str()),&thread_id,action,snapshot.workspace_revision,existing.as_ref().map(|item|item.revision).unwrap_or(0))?;let updated=store.list(&binding.preparation_id)?;Ok::<_,bodh::ContractError>((saved,updated))}).await;let _=weak.update(cx,|this,cx|{if this.operation_epoch.load(Ordering::Relaxed)!=epoch{return}match result{Ok((saved,snapshot))=>{this.draft_snapshot=Some(snapshot.clone());this.update_draft_counts(snapshot.counts);this.status=Some(format!("{:?} intent staged at workspace revision {}. Remote thread state is unchanged.",saved.action.action,saved.workspace_revision));},Err(error)=>this.error=Some(format!("{}: {}",error.code,error.message))}cx.notify();});}).detach();
    }

    fn render_changes(
        &mut self,
        binding: &PreparedBinding,
        p: Palette,
        cx: &mut Context<Self>,
        viewport_width: f32,
    ) -> impl IntoElement {
        let selected = self.selected_file;
        let source_width = self
            .prefs
            .inspector_width
            .clamp(300., (viewport_width - 520.).max(300.));
        let narrow_workspace = viewport_width < 800.;
        let file_width = 220.;
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
                (
                    *index,
                    changed_file_presentation(
                        file.old_path.as_deref(),
                        file.new_path.as_deref(),
                        &file.status,
                    ),
                )
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
                            .h(px(48.))
                            .w_full()
                            .px_2()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .gap_0()
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
                            .cursor_pointer()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_size(px(12.))
                                            .truncate()
                                            .child(label.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_size(px(10.))
                                            .text_color(rgb(p.muted))
                                            .child(label.status.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.muted))
                                    .truncate()
                                    .child(label.directory.clone()),
                            )
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
        .on_scroll_wheel(cx.listener(|this, event, _, cx| {
            this.persist_navigation();
            stop_vertical_wheel_bubbling(event, cx);
        }))
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
            .min_w(px(240.))
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
            .w_full()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .p_2()
            .border_b_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_size(px(12.))
                    .truncate()
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
                .border_color(rgb(if self.split_diff == split {
                    p.accent
                } else {
                    p.border
                }))
                .bg(rgb(if self.split_diff == split {
                    p.raised
                } else {
                    p.panel
                }))
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
                .w_full()
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
            .on_scroll_wheel(cx.listener(|this, event, _, cx| {
                this.persist_navigation();
                stop_vertical_wheel_bubbling(event, cx);
            }))
            .flex_1()
            .min_h_0();
            let virtual_rows = virtual_rows.w_full();
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
            .w_full()
            .min_w_0()
            .flex()
            .flex_wrap()
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
                    .border_color(rgb(if self.source_revision == revision {
                        p.accent
                    } else {
                        p.border
                    }))
                    .bg(rgb(if self.source_revision == revision {
                        p.raised
                    } else {
                        p.panel
                    }))
                    .text_color(rgb(if self.source_revision == revision {
                        p.text
                    } else {
                        p.muted
                    })),
                );
            }
        }
        if self.source_ref.is_some() {
            let draft_editor_open = self.draft_editor.is_some();
            source_toolbar = source_toolbar.child(
                button(
                    "new-inline-comment-draft",
                    "Draft comment",
                    p,
                    |this, _, _, cx| this.begin_text_draft(cx),
                    cx,
                )
                .border_color(rgb(if draft_editor_open {
                    p.accent
                } else {
                    p.border
                }))
                .bg(rgb(if draft_editor_open { p.raised } else { p.panel })),
            );
            if self
                .source_ref
                .as_ref()
                .is_some_and(|source| source.revision == Revision::Head)
            {
                let replacement_editor_open = self.replacement_editor.is_some();
                source_toolbar = source_toolbar.child(
                    button(
                        "new-code-replacement-draft",
                        "Draft replacement",
                        p,
                        |this, _, _, cx| this.begin_code_replacement(cx),
                        cx,
                    )
                    .border_color(rgb(if replacement_editor_open {
                        p.accent
                    } else {
                        p.border
                    }))
                    .bg(rgb(if replacement_editor_open {
                        p.raised
                    } else {
                        p.panel
                    })),
                );
            }
        }
        source = source.child(source_toolbar);
        let mut source_content = div()
            .id("pinned-source-content")
            .track_scroll(&self.source_scroll)
            .on_scroll_wheel(cx.listener(|this, event, _, cx| {
                this.persist_navigation();
                stop_vertical_wheel_bubbling(event, cx);
            }))
            .overflow_scroll()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col();
        if let Some(label) = self.source_label.clone() {
            source_content = source_content.child(
                div()
                    .mt_2()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(label),
            );
        } else {
            source_content = source_content.child(
                div()
                    .mt_2()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child("Select a diff line to inspect its exact pinned blob line."),
            );
        }
        if self.source_loading {
            source_content = source_content.child(
                div()
                    .mt_3()
                    .text_color(rgb(p.muted))
                    .child("Loading source from the pinned commit…"),
            );
        } else if let (Some(text), Some(source_ref)) = (&self.source_text, &self.source_ref) {
            let display_ref = self.source_display_ref.as_ref().unwrap_or(source_ref);
            let extension = path_extension(&display_ref.path);
            let mut source_rows = div().mt_3().flex().flex_col().gap_0();
            for (offset, line) in text.lines().enumerate() {
                let number = display_ref.start_line.saturating_add(offset as u32);
                let selected = (source_ref.start_line..=source_ref.end_line).contains(&number);
                source_rows =
                    source_rows.child(render_source_row(number, line, extension, p, selected));
            }
            source_content = source_content.child(source_rows);
        }
        source_content = source_content.child(self.render_anchor_repair_panel(p, cx));
        let discussion_path = self
            .source_ref
            .as_ref()
            .map(|source| source.path.clone())
            .unwrap_or_else(|| selected_path.to_owned());
        source_content =
            source_content.child(self.render_inline_discussions(&discussion_path, p, cx));
        source_content = source_content.child(self.render_draft_panel(p, cx));
        source = source.child(source_content);
        let panes = div()
            .id("changes-panes")
            .flex()
            .gap_2()
            .flex_1()
            .min_h_0()
            .overflow_x_scroll()
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
            .child(source);
        let mut workspace = div()
            .w_full()
            .min_w_0()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_1();
        if narrow_workspace {
            workspace = workspace.child(
                div()
                    .px_2()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child("Narrow window: scroll horizontally to view each review pane."),
            );
        }
        workspace.child(panes)
    }

    fn render_anchor_repair_panel(&mut self, p: Palette, cx: &mut Context<Self>) -> Stateful<Div> {
        if self.anchor_repair_previews.is_empty() && !self.anchor_repair_loading {
            return div().id("anchor-repair-panel-empty");
        }
        let mut panel = div()
            .id("anchor-repair-panel")
            .mt_3()
            .p_3()
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.raised))
            .flex()
            .flex_col()
            .gap_2()
            .child(section_title("Drafts from an earlier PR revision", p));
        if self.anchor_repair_loading && self.anchor_repair_previews.is_empty() {
            return panel.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child("Checking older local drafts against this pinned diff…"),
            );
        }
        panel = panel.child(
            div()
                .text_size(px(11.))
                .text_color(rgb(p.muted))
                .child("A relocation is only a proposal. Confirm it after checking the exact text and both locations; the original draft stays on its earlier review."),
        );
        let selected_source = self.source_ref.clone();
        for preview in self.anchor_repair_previews.clone() {
            let old_location = anchor_label(&preview.source);
            let status = match preview.status {
                AnchorRepairStatus::Current => "Current anchor remains valid",
                AnchorRepairStatus::RelocationProposed => "One exact relocation proposed",
                AnchorRepairStatus::Reattached => "Already copied to this review",
                AnchorRepairStatus::Changed => "Source changed or is no longer eligible",
                AnchorRepairStatus::Removed => "Source was removed",
                AnchorRepairStatus::Unsupported => "Source content is unsupported",
                AnchorRepairStatus::Ambiguous => "Several exact locations match",
            };
            let mut card = div()
                .p_2()
                .border_1()
                .border_color(rgb(p.border))
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.text))
                        .child(format!("{} · {}", preview.kind, status)),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(p.muted))
                        .child(format!("Earlier · {old_location}")),
                );
            if let Some(proposed) = preview.proposed_source.as_ref() {
                card = card.child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(p.accent))
                        .child(format!("Proposed · {}", anchor_label(proposed))),
                );
            }
            if let Some(text) = preview.exact_text.as_deref() {
                let snippet = text.chars().take(240).collect::<String>();
                card = card.child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(p.muted))
                        .font_family("DejaVu Sans Mono")
                        .child(if text.chars().count() > 240 {
                            format!("Exact text · {snippet}…")
                        } else {
                            format!("Exact text · {snippet}")
                        }),
                );
            }
            card = card.child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.muted))
                    .child(preview.explanation.clone()),
            );
            if preview.status == AnchorRepairStatus::Reattached {
                if let Some(id) = preview.reattached_draft_id.as_deref() {
                    card = card.child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.accent))
                            .child(format!("Attached draft · {id}")),
                    );
                }
            } else if preview.status == AnchorRepairStatus::RelocationProposed {
                if let Some(proposed) = preview.proposed_source.clone() {
                    let id = preview.draft_id.clone();
                    card = card.child(button(
                        &format!("confirm-anchor-repair-{id}"),
                        "Confirm proposed relocation",
                        p,
                        move |this, _, _, cx| {
                            this.reattach_anchor_draft(id.clone(), proposed.clone(), cx)
                        },
                        cx,
                    ));
                }
            } else if matches!(
                preview.status,
                AnchorRepairStatus::Changed
                    | AnchorRepairStatus::Removed
                    | AnchorRepairStatus::Unsupported
                    | AnchorRepairStatus::Ambiguous
            ) {
                if let Some(source) = selected_source.clone()
                    && (preview.kind != "code_replacement" || source.revision == Revision::Head)
                {
                    let id = preview.draft_id.clone();
                    card = card.child(button(
                        &format!("reattach-anchor-here-{id}"),
                        "Attach to selected diff line",
                        p,
                        move |this, _, _, cx| {
                            this.reattach_anchor_draft(id.clone(), source.clone(), cx)
                        },
                        cx,
                    ));
                } else {
                    card = card.child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.muted))
                            .child("Select a new eligible diff line to attach this draft."),
                    );
                }
            }
            panel = panel.child(card);
        }
        panel
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

        if let Some(editor) = self.replacement_editor.clone() {
            let anchor = format!(
                "{} · Head:{}–{}",
                editor.source.path, editor.source.start_line, editor.source.end_line
            );
            panel = panel.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(anchor),
            );
            panel = panel.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.muted))
                    .child(format!("Original · {}", editor.original_text.trim_end())),
            );
            panel = panel.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.accent))
                    .child(format!(
                        "Proposed preview · {}",
                        editor.replacement.trim_end()
                    )),
            );
            let display = if editor.replacement.is_empty() {
                "|Type replacement code; Enter inserts a line. The checkout remains unchanged."
                    .to_owned()
            } else {
                editor.display_text_with_cursor()
            };
            panel = panel.child(
                div()
                    .id("code-replacement-editor")
                    .track_focus(&self.draft_focus)
                    .min_h(px(92.))
                    .max_h(px(180.))
                    .overflow_scroll()
                    .p_2()
                    .border_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.panel))
                    .focus(|style| style.border_color(rgb(p.accent)))
                    .font_family("DejaVu Sans Mono")
                    .text_size(px(12.))
                    .text_color(rgb(p.text))
                    .child(display)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, _| window.focus(&this.draft_focus)),
                    )
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        window.focus(&this.draft_focus);
                        let key = event.keystroke.key.as_str();
                        let command =
                            event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
                        let Some(editor) = &mut this.replacement_editor else {
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
                        .text_color(rgb(0xffb7b7))
                        .child(error.to_owned()),
                );
            }
            let mut actions = div().flex().items_center().gap_2();
            actions = actions.child(button(
                "save-code-replacement",
                if self.draft_saving {
                    "Saving…"
                } else {
                    "Save replacement"
                },
                p,
                |this, _, _, cx| this.save_code_replacement(cx),
                cx,
            ));
            actions = actions.child(button(
                "discard-code-replacement",
                if editor.draft_id.is_some() {
                    "Discard proposal"
                } else {
                    "Clear replacement"
                },
                p,
                |this, _, _, cx| this.discard_code_replacement(cx),
                cx,
            ));
            panel = panel.child(actions);
        }
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
        if let Some(snapshot) = self.replacement_snapshot.clone()
            && !snapshot.drafts.is_empty()
        {
            let mut saved = div()
                .mt_2()
                .flex()
                .flex_col()
                .gap_1()
                .child(section_title("Saved code replacements", p));
            for draft in snapshot.drafts {
                let id = draft.id.clone();
                let label = format!(
                    "Resume · {}:{} · {}",
                    draft.source.path,
                    draft.source.start_line,
                    draft.replacement.lines().next().unwrap_or_default()
                );
                saved = saved.child(button(
                    &format!("resume-code-replacement-{id}"),
                    &label,
                    p,
                    move |this, _, _, cx| this.resume_code_replacement(&id, cx),
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

fn select_product_diagram_interaction(
    product: &mut Product,
    index: usize,
    window: &mut Window,
    cx: &mut Context<Product>,
) {
    window.focus(&product.focus);
    product.select_diagram_interaction(index, cx);
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ChangedFilePresentation {
    name: String,
    directory: String,
    status: String,
}

fn path_parts(path: Option<&str>) -> (&str, &str) {
    let Some(path) = path else {
        return ("(root)", "(unknown path)");
    };
    path.rsplit_once('/')
        .map_or(("(root)", path), |(directory, name)| (directory, name))
}

fn changed_file_presentation(
    old_path: Option<&str>,
    new_path: Option<&str>,
    status: &str,
) -> ChangedFilePresentation {
    let display_path = new_path.or(old_path);
    let (directory, name) = path_parts(display_path);
    let directory = match (old_path, new_path) {
        (Some(old), Some(new)) if old != new => {
            let (old_directory, _) = path_parts(Some(old));
            let (new_directory, _) = path_parts(Some(new));
            if old_directory == new_directory {
                new_directory.to_owned()
            } else {
                format!("{old_directory} → {new_directory}")
            }
        }
        _ => directory.to_owned(),
    };
    ChangedFilePresentation {
        name: name.to_owned(),
        directory,
        status: status.to_owned(),
    }
}

fn stop_vertical_wheel_bubbling(event: &ScrollWheelEvent, cx: &mut App) {
    if !event.delta.pixel_delta(px(16.)).y.is_zero() {
        cx.stop_propagation();
    }
}

#[cfg(test)]
mod changed_file_presentation_tests {
    use super::changed_file_presentation;

    #[test]
    fn deep_paths_keep_the_filename_and_directory_separate_for_two_line_rows() {
        let row = changed_file_presentation(
            Some("config/quickshell/components/BarModule.qml"),
            Some("config/quickshell/components/BarModule.qml"),
            "M",
        );
        assert_eq!(row.name, "BarModule.qml");
        assert_eq!(row.directory, "config/quickshell/components");
        assert_eq!(row.status, "M");
    }

    #[test]
    fn renames_show_the_new_filename_and_both_directories() {
        let row = changed_file_presentation(
            Some("old/widgets/Launcher.qml"),
            Some("config/quickshell/LauncherBase.qml"),
            "R100",
        );
        assert_eq!(row.name, "LauncherBase.qml");
        assert_eq!(row.directory, "old/widgets → config/quickshell");
        assert_eq!(row.status, "R100");
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

fn status_badge(label: &str, color: u32) -> Div {
    div()
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(color))
        .child(label.to_uppercase())
}

fn link_button(
    id: &str,
    label: &str,
    p: Palette,
    action: impl Fn(&mut Product, &ClickEvent, &mut Window, &mut Context<Product>) + 'static,
    cx: &mut Context<Product>,
) -> Stateful<Div> {
    div()
        .id(ElementId::Name(SharedString::from(id.to_owned())))
        .px_0()
        .py_0()
        .border_0()
        .bg(transparent_black())
        .text_color(rgb(p.accent))
        .cursor_pointer()
        .focus(|style| style.text_color(rgb(p.accent)))
        .child(label.to_owned())
        .on_click(cx.listener(action))
}

fn severity_color(severity: &bodh::Severity, p: Palette) -> u32 {
    match severity {
        bodh::Severity::Critical | bodh::Severity::High => p.danger,
        bodh::Severity::Medium => p.caution,
        bodh::Severity::Low => p.success,
    }
}

fn follow_up_color(status: FollowUpProminence, p: Palette) -> u32 {
    match status {
        FollowUpProminence::Regressed => p.danger,
        FollowUpProminence::Unresolved | FollowUpProminence::Uncertain => p.caution,
        FollowUpProminence::Resolved | FollowUpProminence::Unassessed => p.success,
    }
}

fn anchor_label(source: &SourceRef) -> String {
    source_title(source)
}

fn pending_draft_count(counts: &bodh::DraftCounts) -> u32 {
    counts
        .inline_comments
        .saturating_add(counts.replies)
        .saturating_add(counts.thread_intents)
        .saturating_add(counts.code_replacements)
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
        .w_full()
        .h(px(24.))
        .flex_shrink_0()
        .flex()
        .child(left_cell)
        .child(
            div()
                .id(("split-diff-divider", index))
                .w(px(1.))
                .h_full()
                .flex_shrink_0()
                .bg(rgb(p.border)),
        )
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

fn discussion_observation_for_path(mut observation: Value, path: &str) -> Value {
    if let Some(threads) = observation.get_mut("threads").and_then(Value::as_array_mut) {
        threads.retain(|thread| thread["path"].as_str() == Some(path));
    }
    observation
}

fn source_context_range(anchor: &SourceRef, context_lines: u32) -> SourceRef {
    SourceRef {
        path: anchor.path.clone(),
        revision: anchor.revision,
        start_line: anchor.start_line.saturating_sub(context_lines).max(1),
        end_line: anchor.end_line.saturating_add(context_lines),
    }
}

fn read_source_context(
    checkout: &Path,
    context: &bodh::ReviewContext,
    display: &SourceRef,
    selected_line: u32,
    control: &OperationControl,
) -> Result<(SourceRef, String), bodh::ContractError> {
    let mut range = display.clone();
    loop {
        match bodh::read_source(checkout, context, &range, control) {
            Ok(text) => return Ok((range, text)),
            Err(error)
                if error.code == "SOURCE_RANGE_INVALID" && range.end_line > selected_line =>
            {
                range.end_line -= 1;
            }
            Err(error) => return Err(error),
        }
    }
}

fn render_source_row(
    line_number: u32,
    text: &str,
    language: &str,
    p: Palette,
    selected: bool,
) -> Stateful<Div> {
    div()
        .id(("source-line", line_number))
        .w_full()
        .min_w_0()
        .min_h(px(22.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .bg(rgb(if selected { p.raised } else { p.panel }))
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
        .child(div().flex_1().min_w_0().child(code_text(text, language, p)))
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

#[cfg(test)]
mod nested_scroll_tests {
    use super::stop_vertical_wheel_bubbling;
    use gpui::{
        AppContext, Context, InteractiveElement, IntoElement, Modifiers, ParentElement, Pixels,
        Render, ScrollDelta, ScrollHandle, ScrollWheelEvent, StatefulInteractiveElement, Styled,
        TestAppContext, Window, div, point, px,
    };

    struct NestedScrollView {
        page: ScrollHandle,
        diagram: ScrollHandle,
        wheel_seen: std::rc::Rc<std::cell::Cell<bool>>,
    }

    impl Render for NestedScrollView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let wheel_seen = self.wheel_seen.clone();
            div()
                .id("page-scroll-probe")
                .w(px(300.))
                .h(px(300.))
                .track_scroll(&self.page)
                .overflow_scroll()
                .flex()
                .flex_col()
                .child(
                    div()
                        .id("diagram-scroll-probe")
                        .w(px(200.))
                        .h(px(100.))
                        .track_scroll(&self.diagram)
                        .overflow_scroll()
                        .on_scroll_wheel(move |event, _, cx| {
                            wheel_seen.set(true);
                            stop_vertical_wheel_bubbling(event, cx);
                        })
                        .child(div().w(px(400.)).h(px(500.))),
                )
                .child(div().h(px(800.)))
        }
    }

    #[gpui::test]
    fn scrolling_inside_diagram_does_not_also_scroll_overview_page(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        let page = ScrollHandle::new();
        let diagram = ScrollHandle::new();
        let wheel_seen = std::rc::Rc::new(std::cell::Cell::new(false));
        cx.draw(
            point(px(0.), px(0.)),
            gpui::size(px(300.), px(300.)),
            |_, cx| {
                cx.new(|_| NestedScrollView {
                    page: page.clone(),
                    diagram: diagram.clone(),
                    wheel_seen: wheel_seen.clone(),
                })
            },
        );
        let diagram_bounds = diagram.bounds();
        let pointer = point(
            diagram_bounds.origin.x + px(20.),
            diagram_bounds.origin.y + px(20.),
        );
        cx.simulate_mouse_move(pointer, None, Modifiers::default());
        cx.simulate_event(ScrollWheelEvent {
            position: pointer,
            delta: ScrollDelta::Pixels(point(Pixels::ZERO, px(40.))),
            ..Default::default()
        });
        assert!(
            wheel_seen.get(),
            "diagram scroll hitbox should receive wheel"
        );
        assert_ne!(diagram.offset().y, Pixels::ZERO, "diagram should scroll");
        assert_eq!(page.offset().y, Pixels::ZERO, "overview page must not move");
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(250.), px(250.)),
            delta: ScrollDelta::Pixels(point(Pixels::ZERO, px(40.))),
            ..Default::default()
        });
        assert_ne!(
            page.offset().y,
            Pixels::ZERO,
            "overview scrolls outside diagram"
        );
    }
}
