use std::collections::HashMap;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use egui::{Align, Button, CentralPanel, Context, Frame, Id, Layout, Modal, Panel, RichText, TextStyle, Ui, ViewportCommand};

use super::code_line::WordHit;
use super::commit_box::{self, CommitAction};
use super::diff_view::{self, DiffViewState};
use super::file_list::{self, FileListAction};
use super::keymap::{self, Command};
use super::peek_view::{self, LoadedSource, PeekAction, PeekState};
use super::rendered::{RenderedDiff, ViewRow, view_rows};
use super::tab_bar::{self, TabAction, TabInfo};
use super::theme::Theme;
use crate::application::repository::Repository;
use crate::application::review_service::{RepoSnapshot, ReviewService};
use crate::domain::change::{ChangeKind, ChangedFile, StageState};
use crate::domain::importance::FileGroup;
use crate::infra::config::AppConfig;
use crate::infra::editor;
use crate::infra::git_cli::GitCli;
use crate::infra::highlight::Highlighter;
use crate::infra::index_worker::spawn_index_build;
use crate::infra::single_instance::{self, OpenRequest};
use crate::infra::status_worker::{StatusWorker, WorkerEvent};
use crate::infra::updates::{self, CURRENT_VERSION, ReleaseInfo};

pub struct DiffViewerApp {
    theme: Theme,
    highlighter: Highlighter,
    config: AppConfig,
    tabs: Vec<Workspace>,
    active: usize,
    remote_opens: Receiver<OpenRequest>,
    update: UpdateState,
    hide_tests: bool,
    collapse_unchanged: bool,
    focus_commit: bool,
    status: Option<StatusLine>,
    pending_discard: Option<ChangedFile>,
}

struct Workspace {
    service: Arc<ReviewService>,
    worker: StatusWorker,
    snapshot: RepoSnapshot,
    selected: Option<PathBuf>,
    diffs: HashMap<PathBuf, RenderedDiff>,
    view: DiffViewState,
    rows: Vec<ViewRow>,
    rows_key: Option<RowsKey>,
    scroll_list_to_selected: bool,
    scroll_to_first_hunk: bool,
    peek: Option<PeekState>,
    index_events: Receiver<Result<usize, String>>,
    indexed_files: Option<usize>,
    commit_message: String,
}

struct UpdateState {
    check: Receiver<Result<Option<ReleaseInfo>, String>>,
    available: Option<ReleaseInfo>,
    install: Option<Receiver<Result<(), String>>>,
    installed: Option<String>,
    homebrew: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct RowsKey {
    path: PathBuf,
    collapsed: bool,
    expanded_gaps: usize,
}

struct StatusLine {
    text: String,
    is_error: bool,
    shown_at: Instant,
}

enum HeaderAction {
    ToggleStage,
    Discard,
    NextHunk,
    PrevHunk,
}

enum TopBarAction {
    OpenRepo,
    Refresh,
    StageAll,
    UnstageAll,
    TestsToggled,
    CollapseToggled,
}

enum DiscardDecision {
    Pending,
    Confirm,
    Cancel,
}

impl DiffViewerApp {
    pub fn new(cc: &eframe::CreationContext<'_>, config: AppConfig, request: OpenRequest, listener: Option<UnixListener>) -> Self {
        let theme = Theme::dark();
        theme.apply(&cc.egui_ctx);
        let remote_opens = match listener {
            Some(listener) => {
                let ctx = cc.egui_ctx.clone();
                single_instance::spawn_listener(
                    listener,
                    Box::new(move || {
                        ctx.send_viewport_cmd(ViewportCommand::Focus);
                        ctx.request_repaint();
                    }),
                )
            }
            None => channel().1,
        };
        let check_ctx = cc.egui_ctx.clone();
        let update = UpdateState {
            check: updates::spawn_check(Box::new(move || check_ctx.request_repaint())),
            available: None,
            install: None,
            installed: None,
            homebrew: updates::managed_by_homebrew(),
        };
        let mut app = Self {
            theme,
            highlighter: Highlighter::new(),
            hide_tests: config.hide_tests,
            collapse_unchanged: false,
            config,
            tabs: Vec::new(),
            active: 0,
            remote_opens,
            update,
            focus_commit: false,
            status: None,
            pending_discard: None,
        };
        if let Some(error) = app.config.load_error.clone() {
            app.report_error(error);
        }
        app.open_repo(&cc.egui_ctx, &request);
        app
    }

    fn open_repo(&mut self, ctx: &Context, request: &OpenRequest) {
        let opened = match request.range.clone() {
            Some(spec) => GitCli::discover_range(&request.dir, spec),
            None => GitCli::discover(&request.dir),
        };
        let git = match opened {
            Ok(git) => git,
            Err(error) => {
                self.report_error(format!("{error:#}"));
                return;
            }
        };
        // The same repo opened at a different range is a different review, so the
        // mode is part of the tab identity.
        let existing = self
            .tabs
            .iter()
            .position(|tab| tab.service.root() == git.root() && tab.service.mode() == git.mode());
        if let Some(index) = existing {
            self.activate_tab(ctx, index);
            return;
        }
        let service = Arc::new(ReviewService::new(Box::new(git), self.config.policy.clone()));
        let wake_ctx = ctx.clone();
        let worker = StatusWorker::spawn(
            Arc::clone(&service),
            self.config.poll_interval,
            Box::new(move || wake_ctx.request_repaint()),
        );
        let index_ctx = ctx.clone();
        let index_events = spawn_index_build(Arc::clone(&service), Box::new(move || index_ctx.request_repaint()));
        let snapshot = match service.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.report_error(error.to_string());
                RepoSnapshot::default()
            }
        };
        self.tabs.push(Workspace {
            service,
            worker,
            snapshot: RepoSnapshot::default(),
            selected: None,
            diffs: HashMap::new(),
            view: DiffViewState::new(self.collapse_unchanged),
            rows: Vec::new(),
            rows_key: None,
            scroll_list_to_selected: false,
            scroll_to_first_hunk: false,
            peek: None,
            index_events,
            indexed_files: None,
            commit_message: String::new(),
        });
        let index = self.tabs.len() - 1;
        self.apply_snapshot(ctx, index, snapshot);
        self.activate_tab(ctx, index);
    }

    fn activate_tab(&mut self, ctx: &Context, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
        self.pending_discard = None;
        self.sync_title(ctx);
    }

    fn close_tab(&mut self, ctx: &Context, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if index < self.active {
            self.active -= 1;
        }
        self.pending_discard = None;
        self.sync_title(ctx);
    }

    fn cycle_tab(&mut self, ctx: &Context, delta: isize) {
        if self.tabs.is_empty() {
            return;
        }
        let count = self.tabs.len() as isize;
        let next = (self.active as isize + delta).rem_euclid(count) as usize;
        self.activate_tab(ctx, next);
    }

    fn sync_title(&self, ctx: &Context) {
        let title = match self.tabs.get(self.active) {
            Some(tab) => format!("{} — diffviewer", Self::tab_label(tab)),
            None => "diffviewer".to_string(),
        };
        ctx.send_viewport_cmd(ViewportCommand::Title(title));
    }

    fn tab(&self) -> Option<&Workspace> {
        self.tabs.get(self.active)
    }

    /// True when the active tab reviews a committed range rather than the tree.
    fn is_range(&self) -> bool {
        self.tab().is_some_and(|workspace| workspace.service.is_range())
    }

    /// A repo can be open both as a working tree and as one or more ranges, so
    /// tabs and the title carry the range to stay tellable apart.
    fn tab_label(workspace: &Workspace) -> String {
        let name = workspace.service.repo_name();
        match workspace.service.mode().range() {
            Some(spec) => format!("{name} · {}", spec.label()),
            None => name,
        }
    }

    fn tab_mut(&mut self) -> Option<&mut Workspace> {
        self.tabs.get_mut(self.active)
    }

    fn apply_snapshot(&mut self, ctx: &Context, index: usize, snapshot: RepoSnapshot) {
        let hide_tests = self.hide_tests;
        let Some(workspace) = self.tabs.get_mut(index) else {
            return;
        };
        // A range's content only changes when its head moves — amend, fixup, new
        // commit. Rebuild the symbol index off-thread when that happens, since
        // the cheap per-file refresh below is skipped in range mode.
        let head_moved = workspace.service.is_range()
            && workspace.snapshot.head_id.is_some()
            && workspace.snapshot.head_id != snapshot.head_id;
        workspace.diffs.retain(|path, rendered| {
            snapshot
                .find(path)
                .is_some_and(|file| file.same_content_identity(&rendered.file))
        });
        let touched: Vec<PathBuf> = workspace
            .snapshot
            .files()
            .chain(snapshot.files())
            .map(|file| file.path.clone())
            .collect();
        workspace.service.refresh_symbols(touched.iter().map(PathBuf::as_path));
        workspace.snapshot = snapshot;
        workspace.revalidate_selection(hide_tests);
        if head_moved {
            let wake = ctx.clone();
            workspace.indexed_files = None;
            workspace.index_events =
                spawn_index_build(Arc::clone(&workspace.service), Box::new(move || wake.request_repaint()));
        }
    }

    fn drain_background_events(&mut self, ctx: &Context) {
        let remote: Vec<OpenRequest> = self.remote_opens.try_iter().collect();
        for request in remote {
            self.open_repo(ctx, &request);
        }
        for index in 0..self.tabs.len() {
            let events = self.tabs[index].worker.drain_events();
            for event in events {
                match event {
                    WorkerEvent::Snapshot(snapshot) => self.apply_snapshot(ctx, index, snapshot),
                    WorkerEvent::Failed(error) => self.report_error(error),
                }
            }
            let index_results: Vec<Result<usize, String>> = self.tabs[index].index_events.try_iter().collect();
            for result in index_results {
                match result {
                    Ok(count) => self.tabs[index].indexed_files = Some(count),
                    Err(error) => self.report_error(format!("symbol index: {error}")),
                }
            }
        }
        self.drain_update_events();
    }

    fn drain_update_events(&mut self) {
        for result in self.update.check.try_iter().collect::<Vec<_>>() {
            if let Ok(release) = result {
                self.update.available = release;
            }
        }
        let finished = self
            .update
            .install
            .as_ref()
            .and_then(|install| install.try_iter().next());
        if let Some(result) = finished {
            self.update.install = None;
            match result {
                Ok(()) => {
                    let version = self.update.available.as_ref().map(|release| release.version.clone());
                    self.update.installed = version;
                }
                Err(error) => self.report_error(error),
            }
        }
    }

    fn dispatch(&mut self, ctx: &Context, command: Command) {
        match command {
            Command::NextFile => self.move_selection(1),
            Command::PrevFile => self.move_selection(-1),
            Command::ToggleStage => {
                if let Some(path) = self.selected_path() {
                    self.toggle_stage(&path);
                }
            }
            Command::NextHunk => self.jump_hunk(1),
            Command::PrevHunk => self.jump_hunk(-1),
            Command::ToggleTests => {
                self.hide_tests = !self.hide_tests;
                self.revalidate_selection();
            }
            Command::ToggleCollapse => {
                self.collapse_unchanged = !self.collapse_unchanged;
                self.sync_collapse();
            }
            Command::Refresh => self.request_refresh(),
            Command::StageAll => self.stage_all(),
            Command::OpenRepo => self.pick_repo(ctx),
            Command::Commit if self.is_range() => {}
            Command::Commit => self.commit(),
            Command::FocusCommit if self.is_range() => {}
            Command::FocusCommit => self.focus_commit = true,
            Command::Discard if self.is_range() => {
                self.report_error("Range reviews are read-only".to_string())
            }
            Command::Discard => self.pending_discard = self.selected_file(),
            Command::ClosePeek => self.close_peek(),
            Command::PeekBack => self.peek_back(),
            Command::SwitchTab(index) => self.activate_tab(ctx, index),
            Command::NextTab => self.cycle_tab(ctx, 1),
            Command::PrevTab => self.cycle_tab(ctx, -1),
            Command::CloseTab => self.close_tab(ctx, self.active),
            Command::Unfocus => ctx.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            }),
        }
    }

    fn selected_path(&self) -> Option<PathBuf> {
        self.tab().and_then(|workspace| workspace.selected.clone())
    }

    fn selected_file(&self) -> Option<ChangedFile> {
        let workspace = self.tab()?;
        let path = workspace.selected.as_ref()?;
        workspace.snapshot.find(path).cloned()
    }

    fn revalidate_selection(&mut self) {
        let hide_tests = self.hide_tests;
        for workspace in &mut self.tabs {
            workspace.revalidate_selection(hide_tests);
        }
    }

    fn sync_collapse(&mut self) {
        let collapsed = self.collapse_unchanged;
        for workspace in &mut self.tabs {
            workspace.view.collapsed = collapsed;
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let hide_tests = self.hide_tests;
        let Some(workspace) = self.tab_mut() else {
            return;
        };
        let paths: Vec<PathBuf> = visible_files(&workspace.snapshot, hide_tests)
            .map(|file| file.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        let current = workspace
            .selected
            .as_ref()
            .and_then(|selected| paths.iter().position(|path| path == selected));
        let next = match current {
            Some(index) => (index as isize + delta).clamp(0, paths.len() as isize - 1) as usize,
            None => 0,
        };
        workspace.select(Some(paths[next].clone()));
    }

    fn advance_to_next_unstaged(&mut self, current: &Path) {
        let hide_tests = self.hide_tests;
        let Some(workspace) = self.tab_mut() else {
            return;
        };
        let files: Vec<(PathBuf, StageState)> = visible_files(&workspace.snapshot, hide_tests)
            .map(|file| (file.path.clone(), file.stage))
            .collect();
        let Some(index) = files.iter().position(|(path, _)| path == current) else {
            return;
        };
        let next = files
            .iter()
            .cycle()
            .skip(index + 1)
            .take(files.len().saturating_sub(1))
            .find(|(_, stage)| *stage != StageState::Staged)
            .map(|(path, _)| path.clone());
        if let Some(path) = next {
            workspace.select(Some(path));
        }
    }

    fn toggle_stage(&mut self, path: &Path) {
        let Some(workspace) = self.tab_mut() else {
            return;
        };
        let Some(file) = workspace.snapshot.find(path).cloned() else {
            return;
        };
        let result = workspace.service.toggle_stage(&file);
        match result {
            Ok(new_state) => {
                if let Some(entry) = workspace.snapshot.find_mut(path) {
                    entry.stage = new_state;
                }
                workspace.worker.request_refresh();
                if new_state == StageState::Staged {
                    self.advance_to_next_unstaged(path);
                }
            }
            Err(error) => self.report_error(error.to_string()),
        }
    }

    fn stage_all(&mut self) {
        let result = match self.tab_mut() {
            Some(workspace) => {
                let result = workspace.service.stage_all();
                if result.is_ok() {
                    workspace.set_all_stages(StageState::Staged);
                    workspace.worker.request_refresh();
                }
                result
            }
            None => return,
        };
        if let Err(error) = result {
            self.report_error(error.to_string());
        }
    }

    fn unstage_all(&mut self) {
        let result = match self.tab_mut() {
            Some(workspace) => {
                let result = workspace.service.unstage_all();
                if result.is_ok() {
                    workspace.set_all_stages(StageState::Unstaged);
                    workspace.worker.request_refresh();
                }
                result
            }
            None => return,
        };
        if let Err(error) = result {
            self.report_error(error.to_string());
        }
    }

    fn discard(&mut self, file: &ChangedFile) {
        let hide_tests = self.hide_tests;
        let result = match self.tab_mut() {
            Some(workspace) => {
                let result = workspace.service.discard(file);
                if result.is_ok() {
                    workspace.diffs.remove(&file.path);
                    workspace.remove_file(&file.path);
                    workspace.revalidate_selection(hide_tests);
                    workspace.worker.request_refresh();
                }
                result
            }
            None => return,
        };
        match result {
            Ok(()) => self.report_info(format!("Discarded {}", file.path.display())),
            Err(error) => self.report_error(error.to_string()),
        }
    }

    fn commit(&mut self) {
        let Some(workspace) = self.tab_mut() else {
            return;
        };
        let message = workspace.commit_message.trim().to_string();
        if message.is_empty() {
            self.focus_commit = true;
            self.report_error("Enter a commit message first".to_string());
            return;
        }
        if workspace.snapshot.staged_count() == 0 {
            self.report_error("Nothing is staged".to_string());
            return;
        }
        let result = workspace.service.commit(&message);
        match result {
            Ok(summary) => {
                workspace.commit_message.clear();
                workspace.worker.request_refresh();
                self.report_info(format!("Committed {summary}"));
            }
            Err(error) => self.report_error(error.to_string()),
        }
    }

    fn request_refresh(&mut self) {
        if let Some(workspace) = self.tab_mut() {
            workspace.diffs.clear();
            workspace.rows_key = None;
            workspace.worker.request_refresh();
        }
    }

    fn pick_repo(&mut self, ctx: &Context) {
        let start = self
            .tab()
            .map(|workspace| workspace.service.root().to_path_buf())
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        if let Some(path) = rfd::FileDialog::new().set_directory(start).pick_folder() {
            self.open_repo(ctx, &OpenRequest { dir: path, range: None });
        }
    }

    fn start_update(&mut self, ctx: &Context) {
        if self.update.install.is_some() {
            return;
        }
        let wake = ctx.clone();
        self.update.install = Some(updates::spawn_install(Box::new(move || wake.request_repaint())));
    }

    fn jump_hunk(&mut self, delta: isize) {
        let Some(workspace) = self.tab_mut() else {
            return;
        };
        let Some(path) = &workspace.selected else {
            return;
        };
        let Some(diff) = workspace.diffs.get(path).and_then(RenderedDiff::diff) else {
            return;
        };
        if diff.hunks.is_empty() {
            return;
        }
        let last = diff.hunks.len() - 1;
        let next = match workspace.view.current_hunk {
            None if delta > 0 => 0,
            None => last,
            Some(index) => (index as isize + delta).clamp(0, last as isize) as usize,
        };
        let target_row = diff.hunks[next].start;
        workspace.view.current_hunk = Some(next);
        workspace.view.scroll_to_view_row = workspace
            .rows
            .iter()
            .position(|row| matches!(row, ViewRow::Line(index) if *index == target_row));
    }

    fn ensure_diff_loaded(&mut self) {
        let active = self.active;
        let Some(workspace) = self.tabs.get_mut(active) else {
            return;
        };
        let Some(path) = workspace.selected.clone() else {
            return;
        };
        let Some(file) = workspace.snapshot.find(&path).cloned() else {
            return;
        };
        let stale = workspace
            .diffs
            .get(&path)
            .is_some_and(|rendered| workspace.service.is_stale(&path, rendered.stamp));
        if stale {
            workspace.diffs.remove(&path);
            workspace.rows_key = None;
        }
        if workspace.diffs.contains_key(&path) {
            return;
        }
        match workspace.service.load_diff(&file) {
            Ok(file_diff) => {
                let rendered = RenderedDiff::build(file_diff, &self.highlighter);
                workspace.diffs.insert(path, rendered);
                workspace.rows_key = None;
            }
            Err(error) => self.report_error(error.to_string()),
        }
    }

    fn ensure_rows(&mut self) {
        let Some(workspace) = self.tab_mut() else {
            return;
        };
        let Some(path) = workspace.selected.clone() else {
            workspace.rows.clear();
            workspace.rows_key = None;
            return;
        };
        let Some(rendered) = workspace.diffs.get(&path) else {
            return;
        };
        let key = RowsKey {
            path,
            collapsed: workspace.view.collapsed,
            expanded_gaps: workspace.view.expanded_gaps.len(),
        };
        if workspace.rows_key.as_ref() != Some(&key) {
            workspace.rows = rendered
                .diff()
                .map(|diff| view_rows(diff, workspace.view.collapsed, &workspace.view.expanded_gaps))
                .unwrap_or_default();
            workspace.rows_key = Some(key);
        }
        if workspace.scroll_to_first_hunk {
            workspace.scroll_to_first_hunk = false;
            let first_hunk_row = rendered.diff().and_then(|diff| diff.hunks.first()).map(|hunk| hunk.start);
            workspace.view.scroll_to_view_row = first_hunk_row.and_then(|target| {
                workspace
                    .rows
                    .iter()
                    .position(|row| matches!(row, ViewRow::Line(index) if *index == target))
            });
        }
    }

    fn open_definition(&mut self, file: PathBuf, hit: WordHit, chain: bool) {
        let Some(workspace) = self.tab_mut() else {
            return;
        };
        let symbols = workspace.service.definitions(&file, &hit.word, hit.attribute_access);
        if symbols.is_empty() {
            let message = match workspace.indexed_files {
                Some(_) => format!("No definition found for {}", hit.word),
                None => "Symbol index is still building, try again in a moment".to_string(),
            };
            self.report_info(message);
            return;
        }
        match (workspace.peek.as_mut(), chain) {
            (Some(peek), true) => peek.navigate(symbols),
            _ => workspace.peek = Some(PeekState::new(symbols)),
        }
        self.ensure_peek_loaded();
    }

    fn ensure_peek_loaded(&mut self) {
        let active = self.active;
        let Some(workspace) = self.tabs.get_mut(active) else {
            return;
        };
        let Some(symbol) = workspace
            .peek
            .as_ref()
            .filter(|peek| peek.source.is_none())
            .and_then(|peek| peek.symbol().cloned())
        else {
            return;
        };
        match workspace.service.peek(&symbol) {
            Ok(source) => {
                if let Some(peek) = workspace.peek.as_mut() {
                    peek.source = Some(LoadedSource::build(source, &self.highlighter));
                }
            }
            Err(error) => {
                workspace.peek = None;
                self.report_error(error.to_string());
            }
        }
    }

    fn close_peek(&mut self) {
        if let Some(workspace) = self.tab_mut() {
            workspace.peek = None;
        }
    }

    fn peek_back(&mut self) {
        let Some(workspace) = self.tab_mut() else {
            return;
        };
        let went_back = workspace.peek.as_mut().is_some_and(PeekState::back);
        if !went_back {
            workspace.peek = None;
        }
    }

    fn open_peek_in_editor(&mut self) {
        let Some(workspace) = self.tab() else {
            return;
        };
        let Some(symbol) = workspace.peek.as_ref().and_then(|peek| peek.symbol().cloned()) else {
            return;
        };
        let absolute = workspace.service.root().join(&symbol.path);
        if let Err(error) = editor::open_in_editor(&self.config.editor_command, &absolute, symbol.line) {
            self.report_error(error.to_string());
        }
    }

    fn report_error(&mut self, text: String) {
        self.status = Some(StatusLine { text, is_error: true, shown_at: Instant::now() });
    }

    fn report_info(&mut self, text: String) {
        self.status = Some(StatusLine { text, is_error: false, shown_at: Instant::now() });
    }

    fn expire_status(&mut self, ctx: &Context) {
        let Some(status) = &self.status else {
            return;
        };
        if !status.is_error && status.shown_at.elapsed() > STATUS_LIFETIME {
            self.status = None;
        } else {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn show_top_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut actions = Vec::new();
        Panel::top(Id::new("top_bar"))
            .exact_size(TOP_BAR_HEIGHT)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(6.0);
                    match self.tabs.get(self.active) {
                        Some(workspace) => {
                            ui.label(RichText::new(workspace.service.repo_name()).strong());
                            let prefix = if workspace.service.is_range() { "reviewing" } else { "on" };
                            ui.label(RichText::new(format!("{prefix} {}", workspace.snapshot.branch)).color(self.theme.text_dim));
                        }
                        None => {
                            ui.label(RichText::new("No repository").color(self.theme.text_dim));
                        }
                    }
                    ui.separator();
                    if ui.button("Open…").on_hover_text(keymap::key_label("T")).clicked() {
                        actions.push(TopBarAction::OpenRepo);
                    }
                    if ui.button("Refresh").on_hover_text("r").clicked() {
                        actions.push(TopBarAction::Refresh);
                    }
                    ui.separator();
                    if ui.toggle_value(&mut self.hide_tests, "Hide tests").on_hover_text("t").changed() {
                        actions.push(TopBarAction::TestsToggled);
                    }
                    if ui.toggle_value(&mut self.collapse_unchanged, "Collapse unchanged").on_hover_text("e").changed() {
                        actions.push(TopBarAction::CollapseToggled);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(6.0);
                        let range = self.tabs.get(self.active).is_some_and(|tab| tab.service.is_range());
                        let (clear, mark) = if range {
                            ("Clear marks", "Mark all read")
                        } else {
                            ("Unstage all", "Stage all")
                        };
                        if ui.button(clear).clicked() {
                            actions.push(TopBarAction::UnstageAll);
                        }
                        if ui.button(mark).on_hover_text(keymap::key_label("⇧A")).clicked() {
                            actions.push(TopBarAction::StageAll);
                        }
                        if let Some(workspace) = self.tabs.get(self.active) {
                            let noun = if range { "reviewed" } else { "staged" };
                            let summary = format!(
                                "{} files · {} {noun}",
                                workspace.snapshot.total(),
                                workspace.snapshot.staged_count()
                            );
                            ui.label(RichText::new(summary).color(self.theme.text_dim));
                        }
                    });
                });
            });
        for action in actions {
            match action {
                TopBarAction::OpenRepo => self.pick_repo(&ctx),
                TopBarAction::Refresh => self.request_refresh(),
                TopBarAction::StageAll => self.stage_all(),
                TopBarAction::UnstageAll => self.unstage_all(),
                TopBarAction::TestsToggled => self.revalidate_selection(),
                TopBarAction::CollapseToggled => self.sync_collapse(),
            }
        }
    }

    fn show_tab_bar(&mut self, ui: &mut Ui) {
        if self.tabs.is_empty() {
            return;
        }
        let ctx = ui.ctx().clone();
        let infos: Vec<TabInfo> = self
            .tabs
            .iter()
            .map(|workspace| TabInfo {
                name: Self::tab_label(workspace),
                root: workspace.service.root().to_path_buf(),
                changes: workspace.snapshot.total(),
            })
            .collect();
        let mut action = None;
        Panel::top(Id::new("tab_bar"))
            .exact_size(TAB_BAR_HEIGHT)
            .show_separator_line(false)
            .frame(Frame::NONE.fill(self.theme.panel))
            .show(ui, |ui| {
                action = tab_bar::show(ui, &infos, self.active, &self.theme);
            });
        match action {
            Some(TabAction::Select(index)) => self.activate_tab(&ctx, index),
            Some(TabAction::Close(index)) => self.close_tab(&ctx, index),
            Some(TabAction::Open) => self.pick_repo(&ctx),
            None => {}
        }
    }

    fn show_bottom_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut start_update = false;
        Panel::bottom(Id::new("status_bar"))
            .exact_size(STATUS_BAR_HEIGHT)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(6.0);
                    match &self.status {
                        Some(status) => {
                            let color = if status.is_error { self.theme.removed_fg } else { self.theme.added_fg };
                            ui.label(RichText::new(&status.text).color(color).small());
                        }
                        None => {
                            ui.label(RichText::new(keymap::hints(self.is_range())).color(self.theme.text_faint).small());
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!("v{CURRENT_VERSION} · config: {}", self.config.path.display()))
                                .color(self.theme.text_faint)
                                .small(),
                        );
                        if let Some(workspace) = self.tabs.get(self.active) {
                            let index_label = match workspace.indexed_files {
                                Some(count) => format!("{count} py files indexed"),
                                None => "indexing symbols…".to_string(),
                            };
                            ui.label(RichText::new(index_label).color(self.theme.text_faint).small());
                            ui.separator();
                        }
                        start_update = show_update_notice(ui, &self.update, &self.theme);
                    });
                });
            });
        if start_update {
            self.start_update(&ctx);
        }
    }

    fn show_left_panel(&mut self, ui: &mut Ui) {
        let mut list_action = None;
        let mut commit_action = None;
        let hide_tests = self.hide_tests;
        let active = self.active;
        let focus_commit = std::mem::take(&mut self.focus_commit);
        Panel::left(Id::new("file_panel"))
            .resizable(true)
            .default_size(FILE_PANEL_DEFAULT_WIDTH)
            .size_range(FILE_PANEL_MIN_WIDTH..=FILE_PANEL_MAX_WIDTH)
            .show(ui, |ui| {
                let Some(workspace) = self.tabs.get_mut(active) else {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(format!("Open a git repository with {}", keymap::key_label("T"))).color(self.theme.text_dim));
                    });
                    return;
                };
                let staged_count = workspace.snapshot.staged_count();
                if !workspace.service.is_range() {
                    Panel::bottom(Id::new("commit_panel"))
                        .resizable(false)
                        .show_separator_line(true)
                        .show(ui, |ui| {
                            commit_action = commit_box::show(ui, &mut workspace.commit_message, staged_count, focus_commit, &self.theme);
                        });
                }
                CentralPanel::default()
                    .frame(Frame::NONE)
                    .show(ui, |ui| {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(6.0);
                            let heading = if workspace.service.is_range() {
                                format!("Changes ({}/{} read)", staged_count, workspace.snapshot.total())
                            } else {
                                format!("Changes ({})", workspace.snapshot.total())
                            };
                            ui.label(RichText::new(heading).strong());
                        });
                        ui.add_space(4.0);
                        let groups: Vec<&FileGroup> = visible_groups(&workspace.snapshot, hide_tests).collect();
                        let scroll = std::mem::take(&mut workspace.scroll_list_to_selected);
                        list_action = file_list::show(ui, &groups, workspace.selected.as_deref(), scroll, &self.theme);
                    });
            });
        match list_action {
            Some(FileListAction::Select(path)) => {
                if let Some(workspace) = self.tab_mut() {
                    workspace.select(Some(path));
                }
            }
            Some(FileListAction::ToggleStage(path)) => self.toggle_stage(&path),
            None => {}
        }
        if let Some(CommitAction::Commit) = commit_action {
            self.commit();
        }
    }

    fn show_center(&mut self, ui: &mut Ui) {
        let mut header_action = None;
        let mut peek_action = None;
        let mut definition_request: Option<(PathBuf, WordHit)> = None;
        let editor_name = editor::editor_display_name(&self.config.editor_command);
        let active = self.active;
        CentralPanel::default()
            .frame(Frame::NONE.fill(self.theme.bg))
            .show(ui, |ui| {
                let Some(workspace) = self.tabs.get_mut(active) else {
                    diff_view::centered_message(ui, &format!("Open a git repository ({})", keymap::key_label("T")), &self.theme);
                    return;
                };
                let Some(path) = workspace.selected.clone() else {
                    let text = if workspace.snapshot.total() == 0 { "Working tree clean" } else { "Select a file" };
                    diff_view::centered_message(ui, text, &self.theme);
                    return;
                };
                let Some(file) = workspace.snapshot.find(&path).cloned() else {
                    return;
                };
                let hunk_count = workspace.diffs.get(&path).and_then(RenderedDiff::diff).map(|diff| diff.hunks.len());
                let current_hunk = workspace.view.current_hunk;
                let range = workspace.service.is_range();
                Panel::top(Id::new("diff_header"))
                    .exact_size(DIFF_HEADER_HEIGHT)
                    .show(ui, |ui| {
                        header_action = show_diff_header(ui, &file, hunk_count, current_hunk, range, &self.theme);
                    });
                if workspace.peek.is_some() {
                    Panel::bottom(Id::new("peek_panel"))
                        .resizable(true)
                        .default_size(PEEK_DEFAULT_HEIGHT)
                        .size_range(PEEK_MIN_HEIGHT..=PEEK_MAX_HEIGHT)
                        .show(ui, |ui| {
                            if let Some(peek) = workspace.peek.as_mut() {
                                peek_action = peek_view::show(ui, peek, &editor_name, &self.theme);
                            }
                        });
                }
                CentralPanel::default()
                    .frame(Frame::NONE)
                    .show(ui, |ui| match workspace.diffs.get(&path) {
                        Some(rendered) => {
                            if let Some(hit) = diff_view::show(ui, rendered, &workspace.rows, &mut workspace.view, &self.theme) {
                                definition_request = Some((rendered.file.path.clone(), hit));
                            }
                        }
                        None => diff_view::centered_message(ui, "Loading…", &self.theme),
                    });
            });
        if let Some((path, hit)) = definition_request {
            self.open_definition(path, hit, false);
        }
        match peek_action {
            Some(PeekAction::Close) => self.close_peek(),
            Some(PeekAction::Back) => self.peek_back(),
            Some(PeekAction::Choose(index)) => {
                if let Some(peek) = self.tab_mut().and_then(|workspace| workspace.peek.as_mut()) {
                    peek.choose(index);
                }
            }
            Some(PeekAction::Navigate(hit)) => {
                let origin = self
                    .tab()
                    .and_then(|workspace| workspace.peek.as_ref())
                    .and_then(|peek| peek.source.as_ref())
                    .map(|source| source.path.clone());
                if let Some(origin) = origin {
                    self.open_definition(origin, hit, true);
                }
            }
            Some(PeekAction::OpenInEditor) => self.open_peek_in_editor(),
            None => {}
        }
        match header_action {
            Some(HeaderAction::ToggleStage) => {
                if let Some(path) = self.selected_path() {
                    self.toggle_stage(&path);
                }
            }
            Some(HeaderAction::Discard) => self.pending_discard = self.selected_file(),
            Some(HeaderAction::NextHunk) => self.jump_hunk(1),
            Some(HeaderAction::PrevHunk) => self.jump_hunk(-1),
            None => {}
        }
    }

    fn show_discard_modal(&mut self, ctx: &Context) {
        let Some(file) = self.pending_discard.clone() else {
            return;
        };
        let theme = self.theme.clone();
        let modal = Modal::new(Id::new("discard_modal")).show(ctx, |ui| {
            ui.set_width(DISCARD_MODAL_WIDTH);
            ui.heading("Discard changes?");
            ui.add_space(6.0);
            ui.monospace(file.path.display().to_string());
            let consequence = match file.kind {
                ChangeKind::Untracked => "The file will be deleted from disk.",
                _ => "Working tree and index will be restored from HEAD. This cannot be undone.",
            };
            ui.label(RichText::new(consequence).color(theme.text_dim));
            ui.add_space(12.0);
            let mut decision = DiscardDecision::Pending;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let discard = Button::new(RichText::new("Discard").color(theme.bg)).fill(theme.removed_fg);
                if ui.add(discard).clicked() {
                    decision = DiscardDecision::Confirm;
                }
                if ui.button("Cancel").clicked() {
                    decision = DiscardDecision::Cancel;
                }
            });
            decision
        });
        let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        match modal.inner {
            DiscardDecision::Confirm => {
                self.pending_discard = None;
                self.discard(&file);
            }
            DiscardDecision::Cancel => self.pending_discard = None,
            DiscardDecision::Pending if modal.backdrop_response.clicked() || escape => self.pending_discard = None,
            DiscardDecision::Pending => {}
        }
    }
}

impl eframe::App for DiffViewerApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drain_background_events(&ctx);
        if self.pending_discard.is_none() {
            for command in keymap::poll(&ctx) {
                self.dispatch(&ctx, command);
            }
        }
        self.expire_status(&ctx);
        self.show_tab_bar(ui);
        self.show_top_bar(ui);
        self.show_bottom_bar(ui);
        self.show_left_panel(ui);
        self.ensure_diff_loaded();
        self.ensure_rows();
        self.ensure_peek_loaded();
        self.show_center(ui);
        self.show_discard_modal(&ctx);
    }
}

impl Workspace {
    fn select(&mut self, path: Option<PathBuf>) {
        if self.selected == path {
            return;
        }
        self.selected = path;
        self.view.reset_for_new_file();
        self.peek = None;
        self.rows_key = None;
        self.scroll_list_to_selected = true;
        self.scroll_to_first_hunk = true;
    }

    fn revalidate_selection(&mut self, hide_tests: bool) {
        let still_visible = self
            .selected
            .as_ref()
            .is_some_and(|selected| visible_files(&self.snapshot, hide_tests).any(|file| &file.path == selected));
        if !still_visible {
            let fallback = first_unstaged_or_first(&self.snapshot, hide_tests);
            self.select(fallback);
        }
    }

    fn set_all_stages(&mut self, stage: StageState) {
        for group in &mut self.snapshot.groups {
            for file in &mut group.files {
                file.stage = stage;
            }
        }
    }

    fn remove_file(&mut self, path: &Path) {
        for group in &mut self.snapshot.groups {
            group.files.retain(|file| file.path != path);
        }
        self.snapshot.groups.retain(|group| !group.files.is_empty());
    }
}

fn show_update_notice(ui: &mut Ui, update: &UpdateState, theme: &Theme) -> bool {
    if let Some(version) = &update.installed {
        ui.label(RichText::new(format!("Updated to v{version} — restart diffviewer")).color(theme.added_fg).small());
        ui.separator();
        return false;
    }
    let Some(release) = &update.available else {
        return false;
    };
    let mut start = false;
    if update.install.is_some() {
        ui.label(RichText::new("updating…").color(theme.text_dim).small());
        ui.ctx().request_repaint_after(Duration::from_millis(250));
    } else if update.homebrew {
        ui.label(RichText::new(format!("v{} available · brew upgrade diffviewer", release.version)).color(theme.modified_fg).small());
    } else {
        if ui.small_button("Update").on_hover_text(&release.url).clicked() {
            start = true;
        }
        ui.label(RichText::new(format!("v{} available", release.version)).color(theme.modified_fg).small());
    }
    ui.separator();
    start
}

fn show_diff_header(ui: &mut egui::Ui, file: &ChangedFile, hunk_count: Option<usize>, current_hunk: Option<usize>, range: bool, theme: &Theme) -> Option<HeaderAction> {
    let mut action = None;
    ui.horizontal_centered(|ui| {
        ui.add_space(8.0);
        ui.label(RichText::new(file.path.display().to_string()).text_style(TextStyle::Monospace).color(theme.text));
        let badge_color = match file.kind {
            ChangeKind::Added | ChangeKind::Untracked => theme.added_fg,
            ChangeKind::Deleted => theme.removed_fg,
            ChangeKind::Renamed { .. } => theme.modified_fg,
            ChangeKind::Modified => theme.text_faint,
        };
        ui.label(RichText::new(file.kind.label()).small().color(badge_color));
        if let ChangeKind::Renamed { from } = &file.kind {
            ui.label(RichText::new(format!("from {}", from.display())).small().color(theme.text_faint));
        }
        ui.label(RichText::new(format!("+{}", file.stats.added)).small().color(theme.added_fg));
        ui.label(RichText::new(format!("−{}", file.stats.removed)).small().color(theme.removed_fg));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(8.0);
            let (label, fill, text_color) = match (range, file.is_fully_staged()) {
                (true, true) => ("Mark unread  s", theme.surface, theme.text),
                (true, false) => ("Mark read  s", theme.accent, theme.bg),
                (false, true) => ("Unstage  s", theme.surface, theme.text),
                (false, false) => ("Stage  s", theme.accent, theme.bg),
            };
            if ui.add(Button::new(RichText::new(label).color(text_color)).fill(fill)).clicked() {
                action = Some(HeaderAction::ToggleStage);
            }
            if !range && ui.add(Button::new(RichText::new("Discard").color(theme.removed_fg))).on_hover_text("d").clicked() {
                action = Some(HeaderAction::Discard);
            }
            ui.separator();
            if ui.button("Next").on_hover_text("n").clicked() {
                action = Some(HeaderAction::NextHunk);
            }
            if ui.button("Prev").on_hover_text("p").clicked() {
                action = Some(HeaderAction::PrevHunk);
            }
            let hunk_label = match (hunk_count, current_hunk) {
                (Some(count), Some(index)) => format!("hunk {}/{}", index + 1, count),
                (Some(count), None) => format!("{count} hunks"),
                (None, _) => String::new(),
            };
            ui.label(RichText::new(hunk_label).small().color(theme.text_dim));
        });
    });
    action
}

fn visible_groups(snapshot: &RepoSnapshot, hide_tests: bool) -> impl Iterator<Item = &FileGroup> {
    snapshot.groups.iter().filter(move |group| !(hide_tests && group.is_tests))
}

fn visible_files(snapshot: &RepoSnapshot, hide_tests: bool) -> impl Iterator<Item = &ChangedFile> {
    visible_groups(snapshot, hide_tests).flat_map(|group| group.files.iter())
}

fn first_unstaged_or_first(snapshot: &RepoSnapshot, hide_tests: bool) -> Option<PathBuf> {
    visible_files(snapshot, hide_tests)
        .find(|file| file.stage != StageState::Staged)
        .or_else(|| visible_files(snapshot, hide_tests).next())
        .map(|file| file.path.clone())
}

const TAB_BAR_HEIGHT: f32 = 32.0;
const TOP_BAR_HEIGHT: f32 = 38.0;
const STATUS_BAR_HEIGHT: f32 = 24.0;
const DIFF_HEADER_HEIGHT: f32 = 38.0;
const FILE_PANEL_DEFAULT_WIDTH: f32 = 400.0;
const FILE_PANEL_MIN_WIDTH: f32 = 280.0;
const FILE_PANEL_MAX_WIDTH: f32 = 800.0;
const DISCARD_MODAL_WIDTH: f32 = 440.0;
const PEEK_DEFAULT_HEIGHT: f32 = 340.0;
const PEEK_MIN_HEIGHT: f32 = 120.0;
const PEEK_MAX_HEIGHT: f32 = 900.0;
const STATUS_LIFETIME: Duration = Duration::from_secs(8);
