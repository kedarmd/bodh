//! Native product workspace. GitHub acquisition and pinned-source rules stay in `bodh`.
use bodh::{
    OperationControl, PreparationManifest, PreparationStore, PreparedBinding, Revision, SourceRef,
};
use bodh_sequence_spike::preferences::{
    NavigationPreferences as Preferences, RecentReview, WorkspaceTab,
};
use gpui::{prelude::*, *};
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
    file_scroll: ScrollHandle,
    source_scroll: ScrollHandle,
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
        let mut app = Self {
            screen: if initial_repo.is_some() {
                Screen::Repository
            } else {
                Screen::Home
            },
            active_repository: initial_repo,
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
            file_scroll: ScrollHandle::new(),
            source_scroll: ScrollHandle::new(),
            dragging_pane: false,
        };
        if let Some(path) = app.active_repository.clone()
            && !app.prefs.repositories.contains(&path)
        {
            app.prefs.repositories.insert(0, path);
            app.persist();
        }
        if let (Some(path), Some(number)) = (app.active_repository.clone(), initial_pr) {
            app.prepare(path, number, cx);
        } else if let Some(path) = app.active_repository.clone() {
            app.load_pull_requests(path, cx);
        }
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
        let path = path.trim().to_owned();
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
        if !self.prefs.repositories.contains(&path) {
            self.prefs.repositories.insert(0, path.clone());
        }
        self.prefs.repositories.truncate(20);
        self.persist();
        self.load_pull_requests(path, cx);
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
                    let manifest =
                        bodh::prepare_local_pr(Path::new(&path), number, None, &state, &control)?;
                    let binding = PreparationStore::open(&state)?.load(&manifest.preparation_id)?;
                    Ok::<_, bodh::ContractError>((manifest, binding))
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token {
                    return;
                }
                this.loading = false;
                this.active_control = None;
                match result {
                    Ok((manifest, binding)) => {
                        this.remember(&manifest, &binding, cx);
                        this.selected_review = Some(binding);
                        this.active_tab = this.prefs.active_tab.unwrap_or(WorkspaceTab::Overview);
                        this.screen = Screen::Workspace;
                        this.selected_file = None;
                        this.source_text = None;
                        this.source_loading = false;
                        this.source_label = None;
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
                pending_work: 0,
            },
        );
        self.prefs.recent.truncate(20);
        self.persist();
        let _ = cx;
    }

    fn resume(&mut self, id: String, cx: &mut Context<Self>) {
        self.cancel_pending();
        self.loading = true;
        self.busy_label = "Resuming review…".into();
        self.error = None;
        let state = self.preparation_db.clone();
        let token = self.operation_epoch.load(Ordering::Relaxed);
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { PreparationStore::open(&state)?.load(&id) })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(binding) => {
                        this.active_repository = Some(binding.checkout.clone());
                        this.screen = Screen::Workspace;
                        this.selected_review = Some(binding);
                        this.active_tab = this.prefs.active_tab.unwrap_or(WorkspaceTab::Overview);
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
        self.selected_file = Some(index);
        self.source_text = None;
        self.source_loading = false;
        self.source_scroll.set_offset(point(px(0.), px(0.)));
        let Some(path) = file.new_path.clone().or(file.old_path.clone()) else {
            self.source_label = Some("This tree entry has no readable file path.".into());
            cx.notify();
            return;
        };
        if let Some(reason) = file.unsupported_reason {
            self.source_label = Some(reason);
            cx.notify();
            return;
        }
        let Some(hunk) = file.hunks.first() else {
            self.source_label = Some("This file has no line-based source range.".into());
            cx.notify();
            return;
        };
        let (revision, start, count) = if file.new_path.is_some() {
            (Revision::Head, hunk.new_start, hunk.new_lines)
        } else {
            (Revision::Base, hunk.old_start, hunk.old_lines)
        };
        if count == 0 {
            self.source_label =
                Some("This change has no readable lines on the selected side.".into());
            cx.notify();
            return;
        }
        let revision_label = if revision == Revision::Head {
            "Head"
        } else {
            "Base"
        };
        let source = SourceRef {
            path: path.clone(),
            revision,
            start_line: start.max(1),
            end_line: start
                .saturating_add(count.saturating_sub(1))
                .min(start.saturating_add(79)),
        };
        self.source_label = Some(format!(
            "{} · {}:{}–{}",
            revision_label, path, source.start_line, source.end_line
        ));
        self.source_loading = true;
        let checkout = binding.checkout.clone();
        let context = binding.context.clone();
        let control = OperationControl::with_timeout(Duration::from_secs(10));
        let token = self.operation_epoch.load(Ordering::Relaxed);
        cx.spawn(async move |weak, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    bodh::read_source(Path::new(&checkout), &context, &source, &control)
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.operation_epoch.load(Ordering::Relaxed) != token
                    || this.selected_file != Some(index)
                {
                    return;
                }
                this.source_loading = false;
                match result {
                    Ok(text) => this.source_text = Some(text),
                    Err(error) => {
                        this.source_label = Some(format!("{}: {}", error.code, error.message))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn back(&mut self, cx: &mut Context<Self>) {
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
            Screen::Workspace => {
                root.child(self.render_workspace(p, cx, f32::from(_window.viewport_size().width)))
            }
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
                this.cancel_pending();
                this.screen = Screen::Home;
                this.error = None;
                cx.notify();
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
            bar = bar.child(button(
                "finish-review",
                "Finish Review",
                p,
                move |this, _, _, cx| {
                    this.status = Some(
                        "No review content has been staged yet. Your saved workspace is unchanged."
                            .into(),
                    );
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
                            } else if key.chars().count() == 1
                                && !event.keystroke.modifiers.control
                                && !event.keystroke.modifiers.platform
                            {
                                this.repository_input.push_str(key);
                            }
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, _| window.focus(&this.search_focus)),
                        ),
                )
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
    ) -> impl IntoElement {
        let binding = self
            .selected_review
            .clone()
            .expect("workspace screen has a prepared review");
        let mut page = div()
            .id("review-workspace")
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .p_6()
            .flex()
            .flex_col()
            .gap_4();
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
            WorkspaceTab::Overview => self.render_overview(&binding, p).into_any_element(),
            WorkspaceTab::Changes => self.render_changes(&binding, p, cx, viewport_width).into_any_element(),
            WorkspaceTab::Findings => div().id("findings-empty").flex_1().min_h(px(360.)).p_6().border_1().border_color(rgb(p.border)).bg(rgb(p.panel))
                .child(div().text_size(px(18.)).font_weight(FontWeight::MEDIUM).child("No findings to show"))
                .child(div().mt_2().text_color(rgb(p.muted)).child("No generated review result is attached. Findings never start an agent; review the pinned changes manually.")).into_any_element(),
        };
        page.child(panel)
    }

    fn render_overview(&self, binding: &PreparedBinding, p: Palette) -> impl IntoElement {
        let mut body = div().id("overview-content").flex().flex_col().gap_4();
        body = body.child(empty_card("No generated overview", "This prepared review contains pinned code context only. No sequence diagram or analysis has been generated.", p));
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
                    div().text_color(rgb(p.text)).child(
                        binding
                            .description
                            .clone()
                            .unwrap_or_else(|| "No description was supplied.".into()),
                    ),
                ),
        );
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
    ) -> impl IntoElement {
        let selected = self.selected_file;
        let source_width = self
            .prefs
            .inspector_width
            .clamp(280., (viewport_width - 300.).max(280.));
        let file_width = (viewport_width - 48. - source_width - 8.).max(240.);
        let mut file_list = div()
            .id("change-files")
            .track_scroll(&self.file_scroll)
            .overflow_scroll()
            .w(px(file_width))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgb(p.border))
            .p_2()
            .flex()
            .flex_col()
            .gap_1();
        file_list = file_list.child(section_title("Changed files", p));
        for (index, file) in binding.changed_files.iter().enumerate() {
            let name = file
                .new_path
                .as_deref()
                .or(file.old_path.as_deref())
                .unwrap_or("(unknown path)");
            let status = &file.status;
            let is_selected = selected == Some(index);
            file_list = file_list.child(
                button(
                    &format!("file-{index}"),
                    &format!("{name}  {status}"),
                    p,
                    move |this, _, _, cx| this.select_file(index, cx),
                    cx,
                )
                .text_color(rgb(if is_selected { p.text } else { p.muted }))
                .bg(rgb(if is_selected { p.raised } else { p.panel })),
            );
        }
        let mut source = div()
            .id("pinned-source")
            .track_scroll(&self.source_scroll)
            .overflow_scroll()
            .w(px(source_width))
            .flex_shrink_0()
            .min_w_0()
            .h_full()
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgb(p.border))
            .p_4();
        source = source.child(
            div().font_weight(FontWeight::MEDIUM).child(
                self.source_label
                    .clone()
                    .unwrap_or_else(|| "Pinned source · select a changed file".into()),
            ),
        );
        if let Some(text) = self.source_text.clone() {
            source = source.child(
                div()
                    .mt_3()
                    .font_family("DejaVu Sans Mono")
                    .text_size(px(12.))
                    .whitespace_nowrap()
                    .child(text),
            );
        } else if self.source_label.is_none() {
            source = source.child(div().mt_3().text_color(rgb(p.muted)).child("Source excerpts are read from the prepared base/head commit blobs, never the working tree."));
        } else if self.source_loading {
            source = source.child(
                div()
                    .mt_3()
                    .text_color(rgb(p.muted))
                    .child("Loading the pinned source excerpt…"),
            );
        } else if self.source_text.is_none() {
            source = source.child(
                div()
                    .mt_3()
                    .text_color(rgb(p.muted))
                    .child("The selected entry has no readable anchored range yet."),
            );
        }
        div()
            .id("changes-panes")
            .flex()
            .gap_0()
            .h(px(500.))
            .min_h(px(360.))
            .child(file_list)
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
fn list_pull_requests(
    repository: &Path,
    control: &OperationControl,
) -> Result<Vec<PullRequest>, String> {
    let output = run_bounded(
        Command::new("gh")
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
            .current_dir(repository),
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
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| {
            bodh::ContractError::new("GH_UNAVAILABLE", "Installed gh could not be started", "")
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
