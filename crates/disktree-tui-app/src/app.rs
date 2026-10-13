//! Application state: what is on screen, the scan feeding it, and how keys
//! change it.

use std::cell::Cell;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Instant;

use disktree_tui_core::filter::Filter;
use disktree_tui_core::layout::{self, DrawDepth, Layout, LayoutOptions, Rect, TileKind, Weight};
use disktree_tui_core::platform;
use disktree_tui_core::removal::{self, RemovalPolicy, RemovalType};
use disktree_tui_core::scan::{ScanConfig, ScanProgress, ScanResult, Scanner};
use disktree_tui_core::tree::DirNode;
use disktree_tui_core::volume::{self, DiskSpace, Volume};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use tokio::sync::watch;

use crate::VisMode;
use crate::nav::{self, Direction};
use crate::tree_view;

/// How the tree is measured and drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewSettings {
    /// What tile area and color represent.
    pub mode: VisMode,
    /// Sizes are apparent (file length) rather than disk usage.
    pub apparent: bool,
    /// Hidden entries are included.
    pub show_hidden: bool,
    /// Nesting levels drawn.
    pub depth: DrawDepth,
}

impl ViewSettings {
    /// What tile area is proportional to.
    pub fn weight(self) -> Weight {
        match (self.mode, self.apparent) {
            (VisMode::Files, _) => Weight::Files,
            (_, true) => Weight::Apparent,
            (_, false) => Weight::Size,
        }
    }

    /// `T`: disk usage → apparent size → file count → age → disk usage.
    fn cycle_mode(&mut self) {
        (self.mode, self.apparent) = match (self.mode, self.apparent) {
            (VisMode::Size, false) => (VisMode::Size, true),
            (VisMode::Size, true) => (VisMode::Files, false),
            (VisMode::Files, _) => (VisMode::Age, false),
            (VisMode::Age, _) => (VisMode::Size, false),
        };
    }
}

/// The screen being shown.
pub enum State {
    /// A scan is running.
    Scanning {
        /// Running totals from the scan thread.
        progress: watch::Receiver<ScanProgress>,
        /// When the scan started.
        started: Instant,
    },
    /// A scan finished (or was cancelled); the treemap is shown.
    Ready(Box<ScanResult>),
    /// The scan could not run.
    Error(String),
}

/// A modal input taking the keyboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// Plain navigation.
    None,
    /// Typing a name filter (`/`).
    Filter(String),
    /// Typing a directory to scan (`Ctrl+O`).
    Path(String),
    /// Choosing a volume to scan (`V`).
    Volumes {
        /// The volumes listed.
        list: Vec<Volume>,
        /// The highlighted row.
        index: usize,
    },
    /// Review screen for marked paths (`C`).
    Review(ReviewState),
}

/// State of the review screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewState {
    /// Which list row is highlighted (0 = top).
    pub selected: usize,
    /// Confirmation prompt state.
    pub confirm: Option<ConfirmState>,
}

/// A confirmation dialog on the review screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmState {
    /// Confirm move to trash.
    Trash,
    /// Confirm permanent delete.
    Permanent,
}

/// What the event loop should do after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Continue,
    Quit,
}

/// Where the view sits on screen, recorded by the renderer so keys act on
/// what the user sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewArea {
    /// The main area (treemap or tree view).
    pub rect: Rect,
    /// Whether it shows the treemap (else the tree-view list).
    pub treemap: bool,
}

/// Everything the UI draws from.
pub struct App {
    /// The directory being scanned.
    pub root: PathBuf,
    /// Measurement and drawing settings.
    pub settings: ViewSettings,
    /// The current screen.
    pub state: State,
    /// Free and total space of the scanned filesystem, if known.
    pub disk: Option<DiskSpace>,
    /// The modal input, if any.
    pub input: Input,
    /// The applied name filter.
    pub filter: Option<Filter>,
    /// Set by the renderer each frame.
    pub view_area: Cell<Option<ViewArea>>,
    view: PathBuf,
    selected: Option<PathBuf>,
    /// Earlier and later views, each with the selection it had.
    back: Vec<(PathBuf, Option<PathBuf>)>,
    forward: Vec<(PathBuf, Option<PathBuf>)>,
    filtered: Option<DirNode>,
    /// View and selection to restore when a rescan finishes.
    restore: Option<(PathBuf, Option<PathBuf>)>,
    scan: Option<RunningScan>,
    /// Paths marked for removal (absorption: marking a dir marks all descendants).
    pub marked: HashSet<PathBuf>,
    /// Report of the last committed removal, shown in the side panel.
    pub last_removal: Option<removal::RemovalReport>,
    /// Whether a post-removal auto-rescan has already been triggered.
    rescan_pending: bool,
}

struct RunningScan {
    handle: JoinHandle<io::Result<ScanResult>>,
    cancel: Arc<AtomicBool>,
}

impl App {
    /// Start scanning `root` on a background thread.
    pub fn start(root: PathBuf, settings: ViewSettings) -> Self {
        let mut app = Self::empty(root.clone(), settings, State::Error(String::new()));
        app.start_scan(root);
        app
    }

    fn empty(root: PathBuf, settings: ViewSettings, state: State) -> Self {
        Self {
            view: root.clone(),
            root,
            settings,
            state,
            disk: None,
            input: Input::None,
            filter: None,
            view_area: Cell::new(None),
            selected: None,
            back: Vec::new(),
            forward: Vec::new(),
            filtered: None,
            restore: None,
            scan: None,
            marked: HashSet::new(),
            last_removal: None,
            rescan_pending: false,
        }
    }

    /// An app showing a finished scan (no background thread).
    #[cfg(test)]
    pub fn ready(result: ScanResult, settings: ViewSettings, disk: Option<DiskSpace>) -> Self {
        let root = result.root.path.clone();
        let mut app = Self::empty(root, settings, State::Ready(Box::new(result)));
        app.disk = disk;
        app
    }

    /// Scan `root` afresh, replacing the current tree.
    fn start_scan(&mut self, root: PathBuf) {
        self.cancel_scan();
        let config = ScanConfig {
            skip_hidden: !self.settings.show_hidden,
            // Keep the whole tree: draw depth changes at runtime, and
            // reclaimable totals need every level.
            max_depth: None,
            whole_disk: false,
        };
        let scanner = Scanner::new(config);
        let cancel = scanner.cancel_flag();
        let (tx, rx) = watch::channel(ScanProgress::default());
        let scan_root = root.clone();
        let handle = std::thread::spawn(move || scanner.scan_with_progress(&scan_root, &tx));
        if root != self.root {
            self.view.clone_from(&root);
            self.back.clear();
            self.forward.clear();
        }
        self.disk = volume::disk_space(&root).ok();
        self.root = root;
        self.clear_filter();
        self.state = State::Scanning {
            progress: rx,
            started: Instant::now(),
        };
        self.scan = Some(RunningScan { handle, cancel });
    }

    /// Rescan the current root, coming back to the same view afterwards.
    fn rescan(&mut self) {
        self.restore = Some((self.view.clone(), self.selected.clone()));
        self.start_scan(self.root.clone());
    }

    /// Move to `Ready` or `Error` once the scan thread has finished.
    pub fn poll_scan(&mut self) {
        if !self.scan.as_ref().is_some_and(|s| s.handle.is_finished()) {
            return;
        }
        let scan = self.scan.take().expect("checked above");
        self.state = match scan.handle.join() {
            Ok(Ok(result)) => State::Ready(Box::new(result)),
            Ok(Err(err)) => State::Error(format!("cannot scan {}: {err}", self.root.display())),
            Err(_) => State::Error("the scan thread panicked".into()),
        };
        let (view, selected) = self
            .restore
            .take()
            .unwrap_or_else(|| (self.root.clone(), None));
        let exists = self.tree().is_some_and(|t| t.find(&view).is_some());
        self.view = if exists { view } else { self.root.clone() };
        self.selected = if exists { selected } else { None };
        // Trigger auto-rescan after a successful removal.
        if self.rescan_pending && matches!(self.state, State::Ready(_)) {
            self.rescan_pending = false;
            self.rescan();
        }
    }

    /// Ask a running scan to stop; it ends shortly with partial results.
    pub fn cancel_scan(&self) {
        if let Some(scan) = &self.scan {
            scan.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The whole scanned tree, once ready.
    pub fn tree(&self) -> Option<&DirNode> {
        match &self.state {
            State::Ready(result) => Some(&result.root),
            _ => None,
        }
    }

    /// The finished scan, once ready.
    pub fn result(&self) -> Option<&ScanResult> {
        match &self.state {
            State::Ready(result) => Some(result),
            _ => None,
        }
    }

    /// The directory being viewed, filtered if a filter is applied.
    pub fn view_node(&self) -> Option<&DirNode> {
        if let Some(filtered) = &self.filtered {
            return Some(filtered);
        }
        let tree = self.tree()?;
        Some(tree.find(&self.view).unwrap_or(tree))
    }

    /// The view directory in the unfiltered tree.
    fn unfiltered_view(&self) -> Option<&DirNode> {
        let tree = self.tree()?;
        Some(tree.find(&self.view).unwrap_or(tree))
    }

    /// The treemap for the current view in `area`.
    pub fn layout_in(&self, area: Rect) -> Option<Layout<'_>> {
        let options = LayoutOptions {
            weight: self.settings.weight(),
            max_depth: self.settings.depth.max_depth(),
            ..LayoutOptions::default()
        };
        Some(layout::layout(self.view_node()?, area, &options))
    }

    /// Directories the selection can rest on, in display order: the view's
    /// children that have their own tile, or all of them, largest first, in
    /// the tree view.
    fn selectable(&self) -> Vec<(&DirNode, Option<Rect>)> {
        let Some(view) = self.view_node() else {
            return Vec::new();
        };
        if let Some(area) = self.view_area.get()
            && area.treemap
            && let Some(l) = self.layout_in(area.rect)
            && !l.too_small
        {
            return l
                .tiles
                .iter()
                .filter_map(|t| match t.kind {
                    TileKind::Dir(node) if t.depth == 1 => Some((node, Some(t.rect))),
                    _ => None,
                })
                .collect();
        }
        let weight = self.settings.weight();
        let mut children: Vec<&DirNode> = view
            .children
            .iter()
            .map(AsRef::as_ref)
            .filter(|c| weight.of(c) > 0)
            .collect();
        children.sort_by_key(|c| std::cmp::Reverse(weight.of(c)));
        children.into_iter().map(|c| (c, None)).collect()
    }

    /// The selected directory: the remembered one if still selectable, else
    /// the largest.
    pub fn selected_node(&self) -> Option<&DirNode> {
        let selectable = self.selectable();
        selectable
            .iter()
            .find(|(n, _)| Some(&n.path) == self.selected.as_ref())
            .or_else(|| selectable.first())
            .map(|&(n, _)| n)
    }

    /// Whether `node` is marked (or an ancestor is, via absorption).
    pub fn is_marked(&self, node: &DirNode) -> bool {
        self.marked.contains(&node.path)
    }

    /// Inner check that only borrows `marked`.
    fn is_marked_inner(&self, path: &Path) -> bool {
        self.marked.contains(path)
    }

    /// Mark or unmark the selected tile. With absorption:
    /// - Marking a directory also marks all descendants.
    /// - Unmarking a directory only unmarks it (descendants stay marked
    ///   only if they themselves were directly marked).
    pub fn toggle_mark(&mut self) {
        let Some(path) = self.selected_node().map(|n| n.path.clone()) else {
            return;
        };
        self.toggle_mark_path(&path);
    }

    /// Mark or unmark `path` without touching the selection.
    fn toggle_mark_path(&mut self, path: &Path) {
        if self.is_marked_inner(path) {
            // Unmark: remove this directory and all its descendants.
            self.marked.retain(|p| !p.starts_with(path));
        } else {
            // Mark: add this directory and all its descendants.
            self.mark_descendants_path(path);
        }
    }

    /// Collect all marked paths that are direct children (or the root) of the
    /// current view, excluding those that are descendants of another marked
    /// path (absorption). Uses the unfiltered tree for correct sizes.
    pub fn marked_paths(&self) -> Vec<removal::MarkedPath> {
        let Some(tree) = self.tree() else {
            return Vec::new();
        };

        // Find the unfiltered node for the current view.
        let view_node = tree.find(&self.view).unwrap_or(tree);
        let mut result = Vec::new();
        self.collect_marked_from(view_node, &mut result);
        result
    }

    /// Recursively collect marked direct paths from the unfiltered tree.
    /// If a node is marked, add it and skip all descendants (absorption).
    fn collect_marked_from(&self, node: &DirNode, result: &mut Vec<removal::MarkedPath>) {
        if self.is_marked(node) {
            let mut mp = removal::MarkedPath {
                path: node.path.clone(),
                size: node.size,
                reclaimable: node.reclaimable,
            };
            if self.settings.weight() == Weight::Files {
                mp.size = node.file_count as u64;
            } else if self.settings.apparent {
                mp.size = node.apparent_size;
            }
            result.push(mp);
            return;
        }
        for child in &node.children {
            self.collect_marked_from(child.as_ref(), result);
        }
    }

    /// Add a directory and all its descendants to the marked set.
    fn mark_descendants_path(&mut self, path: &Path) {
        self.marked.insert(path.to_path_buf());
        // Collect all descendant paths first (avoids borrowing self while mutating).
        if let Some(tree) = self.tree() {
            let mut descendants = Vec::new();
            Self::collect_descendant_paths(path, tree, &mut descendants);
            for p in descendants {
                self.marked.insert(p);
            }
        }
    }

    /// Collect all descendant paths of `path` in the subtree rooted at `node`.
    fn collect_descendant_paths(path: &Path, node: &DirNode, out: &mut Vec<PathBuf>) {
        if node.path == path {
            for child in &node.children {
                out.push(child.path.clone());
                Self::collect_descendant_paths(path, child.as_ref(), out);
            }
        } else {
            for child in &node.children {
                Self::collect_descendant_paths(path, child.as_ref(), out);
            }
        }
    }

    /// Free space that would remain after removing marked paths.
    pub fn free_space_after_marks(&self) -> Option<u64> {
        let disk = self.disk?;
        let marked_bytes: u64 = self.marked_paths().iter().map(|m| m.size).sum();
        Some(disk.available.saturating_add(marked_bytes).min(disk.total))
    }

    /// Marked path count.
    pub fn marked_count(&self) -> usize {
        self.marked_paths().len()
    }

    /// The largest reclaimable directories that aren't yet marked — "worth a look".
    pub fn worth_a_look(&self) -> Vec<(PathBuf, String, u64)> {
        let Some(view) = self.view_node() else {
            return Vec::new();
        };
        let mut candidates: Vec<(PathBuf, String, u64)> = Vec::new();
        self.collect_worth_a_look(view, &mut candidates);
        candidates.sort_by_key(|c| std::cmp::Reverse(c.2));
        // Filter out ancestors of already-marked nodes.
        candidates.retain(|(path, _, _)| {
            self.tree()
                .is_none_or(|t| !t.find(path).is_some_and(|n| self.is_ancestor_of_marked(n)))
        });
        candidates.truncate(5); // Show at most 5.
        candidates
    }

    fn collect_worth_a_look(&self, node: &DirNode, out: &mut Vec<(PathBuf, String, u64)>) {
        for child in &node.children {
            let c = child.as_ref();
            if c.reclaimable && !self.is_marked(c) {
                out.push((c.path.clone(), c.name.clone(), c.size));
            }
            self.collect_worth_a_look(c, out);
        }
    }

    fn is_ancestor_of_marked(&self, node: &DirNode) -> bool {
        self.marked.iter().any(|p| p.starts_with(&node.path))
    }

    /// Build the removal policy from current state.
    fn removal_policy(&self) -> RemovalPolicy {
        // Mounts inside the scan that the scanner skipped count too, so a
        // tree spanning them (e.g. where mountinfo is unavailable) is still
        // protected.
        let mut mount_points = platform::mount_points();
        if let Some(result) = self.result() {
            mount_points.extend(result.skipped_mounts.iter().cloned());
        }
        RemovalPolicy {
            scan_root: self.root.clone(),
            fs_root: platform::filesystem_root(),
            home_dir: platform::home_dir(),
            system_trees: platform::system_trees(),
            mount_points,
        }
    }

    /// Handle a mouse click: a plain click selects what is under the pointer,
    /// `Ctrl+Click` toggles its mark without moving the selection.
    pub fn handle_mouse(&mut self, event: MouseEvent) -> Action {
        if event.kind != MouseEventKind::Down(MouseButton::Left)
            || !matches!(self.input, Input::None)
        {
            return Action::Continue;
        }
        let Some(picked) = self.pick_at(event.column, event.row) else {
            return Action::Continue;
        };
        if event.modifiers.contains(KeyModifiers::CONTROL) {
            self.toggle_mark_path(&picked);
        } else {
            self.selected = Some(picked);
        }
        Action::Continue
    }

    /// What the pointer points to: a child of the view in the treemap, or a
    /// row of the tree view.
    fn pick_at(&self, column: u16, row: u16) -> Option<PathBuf> {
        let area = self.view_area.get()?;
        if !area.rect.contains(column, row) {
            return None;
        }
        if area.treemap {
            let layout = self.layout_in(area.rect)?;
            layout
                .tiles
                .iter()
                .filter(|t| t.depth == 1)
                .find(|t| t.rect.contains(column, row))
                .and_then(|t| match &t.kind {
                    TileKind::Dir(node) => Some(node.path.clone()),
                    _ => None,
                })
        } else {
            let view = self.view_node()?;
            let index = usize::from(row).saturating_sub(usize::from(area.rect.y));
            tree_view::rows(view, self.settings, usize::from(area.rect.height))
                .get(index)
                .map(|&(_, node)| node.path.clone())
        }
    }

    /// Handle one key press.
    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        match self.input {
            Input::Filter(_) => {
                self.filter_key(key);
                return Action::Continue;
            }
            Input::Path(_) => {
                self.path_key(key);
                return Action::Continue;
            }
            Input::Volumes { .. } => {
                self.volume_key(key);
                return Action::Continue;
            }
            Input::Review(_) => {
                let state = std::mem::replace(&mut self.input, Input::None);
                if let Input::Review(mut state) = state {
                    let close = self.review_key(key, &mut state);
                    if close {
                        self.input = Input::None;
                    } else {
                        self.input = Input::Review(state);
                    }
                }
                return Action::Continue;
            }
            Input::None => {}
        }
        if matches!(self.state, State::Scanning { .. }) {
            match key.code {
                KeyCode::Char('q' | 'Q') => return Action::Quit,
                KeyCode::Esc => self.cancel_scan(),
                _ => {}
            }
            return Action::Continue;
        }

        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let tree_view = !self.view_area.get().is_some_and(|a| a.treemap);
        match key.code {
            KeyCode::Char('q' | 'Q') => return Action::Quit,
            KeyCode::Char('o') if ctrl => {
                self.input = Input::Path(self.view.display().to_string());
            }
            KeyCode::Char('d') if ctrl => self.scan_whole_disk(),
            KeyCode::Char('g' | 'G') => self.scan_whole_disk(),
            KeyCode::Char('v' | 'V') => {
                self.input = Input::Volumes {
                    list: volume::list_volumes(),
                    index: 0,
                };
            }
            KeyCode::Char('r' | 'R') | KeyCode::F(5) => self.rescan(),
            KeyCode::Char('i' | 'I') => {
                self.settings.show_hidden = !self.settings.show_hidden;
                self.rescan();
            }
            _ if self.tree().is_none() => {}
            KeyCode::Left if alt => self.history_back(),
            KeyCode::Right if alt => self.history_forward(),
            KeyCode::Esc => {
                if self.filter.is_some() {
                    self.clear_filter();
                } else {
                    self.go_up();
                }
            }
            KeyCode::Backspace => self.go_up(),
            KeyCode::Enter => self.open_selected(),
            KeyCode::Tab => self.cycle_selection(1),
            KeyCode::BackTab => self.cycle_selection(-1),
            KeyCode::Up if tree_view => self.cycle_selection(-1),
            KeyCode::Down if tree_view => self.cycle_selection(1),
            KeyCode::Left if tree_view => self.go_up(),
            KeyCode::Right if tree_view => self.open_selected(),
            KeyCode::Up => self.move_selection(Direction::Up),
            KeyCode::Down => self.move_selection(Direction::Down),
            KeyCode::Left => self.move_selection(Direction::Left),
            KeyCode::Right => self.move_selection(Direction::Right),
            KeyCode::Char('/') => self.input = Input::Filter(String::new()),
            KeyCode::Char('d' | 'D') => self.settings.apparent = !self.settings.apparent,
            KeyCode::Char('t' | 'T') => self.settings.cycle_mode(),
            KeyCode::Char('[') => {
                let height = self.view_node().map_or(0, DirNode::height);
                self.settings.depth = self.settings.depth.shallower(height);
            }
            KeyCode::Char(']') => {
                let height = self.view_node().map_or(0, DirNode::height);
                self.settings.depth = self.settings.depth.deeper(height);
            }
            // Mark the selection without moving it: Ctrl+Space.
            KeyCode::Char(' ') if ctrl => self.toggle_mark(),
            // Mark/unmark: Space or X.
            KeyCode::Char(' ' | 'x' | 'X') => {
                self.toggle_mark();
            }
            // Open review screen: C.
            KeyCode::Char('c' | 'C') => {
                let marked = self.marked_paths();
                if !marked.is_empty() {
                    self.input = Input::Review(ReviewState {
                        selected: 0,
                        confirm: None,
                    });
                }
            }
            _ => {}
        }
        Action::Continue
    }

    /// Handle a key on the review screen. Returns whether to close the screen.
    fn review_key(&mut self, key: KeyEvent, state: &mut ReviewState) -> bool {
        // If in confirmation state.
        if let Some(confirm) = state.confirm.take() {
            match key.code {
                KeyCode::Char('y' | 'Y') => {
                    // Commit the action.
                    self.commit_removal(confirm);
                    return true;
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    // Cancel confirmation, go back to review list.
                }
                _ => {}
            }
            return false;
        }

        let marked = self.marked_paths();
        match key.code {
            KeyCode::Up => state.selected = state.selected.saturating_sub(1),
            KeyCode::Down => {
                state.selected = (state.selected + 1).min(marked.len().saturating_sub(1));
            }
            KeyCode::Char('m' | 'M') => {
                if state.selected < marked.len() {
                    state.confirm = Some(ConfirmState::Trash);
                }
            }
            KeyCode::Char('p') => {
                if state.selected < marked.len() {
                    state.confirm = Some(ConfirmState::Permanent);
                }
            }
            KeyCode::Char('!') => {
                self.marked.clear();
            }
            KeyCode::Char('s' | 'S') => {
                self.save_marked_to_file();
            }
            KeyCode::Char('a' | 'A') => {
                self.copy_agent_prompt();
            }
            KeyCode::Char('u' | 'U') => {
                // Unmark the selected entry.
                let path = marked.get(state.selected).map(|m| m.path.clone());
                if let Some(path) = path {
                    self.marked.remove(&path);
                }
            }
            KeyCode::Esc => return true,
            _ => {}
        }
        false
    }

    /// Commit a removal action (trash or permanent).
    fn commit_removal(&mut self, confirm: ConfirmState) {
        let r#type = match confirm {
            ConfirmState::Trash => RemovalType::Trash,
            ConfirmState::Permanent => RemovalType::Permanent,
        };

        let marked = self.marked_paths();
        let policy = self.removal_policy();

        let report = removal::remove_paths(&marked, r#type, &policy);
        if report.succeeded.is_empty() && report.failed.is_empty() {
            return;
        }
        // Keep the report for display; unmark only what actually went away.
        let gone = report.succeeded.clone();
        self.last_removal = Some(report);
        self.marked
            .retain(|p| !gone.iter().any(|s| p == s || p.starts_with(s)));
        // Schedule auto-rescan after the scan thread is free.
        self.rescan_pending = true;
    }

    /// Save marked paths to a file in ~/.config/disktree-tui/.
    fn save_marked_to_file(&self) {
        let marked = self.marked_paths();
        if marked.is_empty() {
            return;
        }
        let dir =
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/disktree-tui");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("disktree-tui: cannot create config dir: {e}");
            return;
        }
        let path = dir.join("marked.txt");
        let content: String = marked
            .iter()
            .map(|m| m.path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        if let Err(e) = std::fs::write(&path, &content) {
            eprintln!("disktree-tui: cannot write {}: {e}", path.display());
        }
    }

    /// Copy an agent prompt to the clipboard (or print to stdout for piping).
    fn copy_agent_prompt(&self) {
        let marked = self.marked_paths();
        if marked.is_empty() {
            return;
        }
        let prompt: String = marked
            .iter()
            .map(|m| {
                let escaped = m.path.display().to_string().replace('\n', "\\n");
                format!("- {escaped}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let text = format!(
            "Clean up the following directories by freeing up space. Check each first\n\
             (git clean, editor cleanup, etc.) before deleting. Move to trash when\n\
             possible.\n\n{prompt}"
        );
        // Try xclip/xsel; fall back to printing.
        if let Err(_e) = std::process::Command::new("xclip")
            .arg("-selection")
            .arg("clipboard")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(stdin) = child.stdin.as_mut() {
                    use std::io::Write;
                    stdin.write_all(text.as_bytes())?;
                }
                child.wait()
            })
        {
            // Fallback: print to stdout for piping.
            print!("{text}");
        }
    }

    /// Filter for the live preview while typing, if any.
    pub fn preview_filter(&self) -> Option<Filter> {
        match &self.input {
            Input::Filter(query) => Filter::new(query),
            _ => None,
        }
    }

    /// Directories below the view matching what is being typed.
    pub fn preview_count(&self) -> usize {
        match (self.preview_filter(), self.unfiltered_view()) {
            (Some(f), Some(view)) => f.count(view),
            _ => 0,
        }
    }

    fn apply_filter(&mut self, query: &str) {
        self.filter = Filter::new(query);
        self.filtered = match (&self.filter, self.unfiltered_view()) {
            (Some(f), Some(view)) => Some(f.apply(view)),
            _ => None,
        };
        self.selected = None;
    }

    fn clear_filter(&mut self) {
        self.filter = None;
        self.filtered = None;
    }

    fn scan_whole_disk(&mut self) {
        match volume::mount_root(&self.root) {
            Ok(root) => self.start_scan(root),
            Err(err) => {
                self.state = State::Error(format!(
                    "cannot find the disk holding {}: {err}",
                    self.root.display()
                ));
            }
        }
    }

    /// Show `path`, recording the current view in the back history.
    fn navigate(&mut self, path: PathBuf, select: Option<PathBuf>) {
        if path == self.view {
            return;
        }
        // Read the selection before the view changes under it.
        let selection = self.current_selection();
        let here = (std::mem::replace(&mut self.view, path), selection);
        self.back.push(here);
        self.forward.clear();
        self.selected = select;
        self.clear_filter();
    }

    fn open_selected(&mut self) {
        if let Some(node) = self.selected_node() {
            let path = node.path.clone();
            self.navigate(path, None);
        }
    }

    fn go_up(&mut self) {
        if self.view == self.root {
            return;
        }
        if let Some(parent) = self.view.parent().map(Path::to_path_buf) {
            let came_from = self.view.clone();
            self.navigate(parent, Some(came_from));
        }
    }

    fn history_back(&mut self) {
        if let Some((view, selected)) = self.back.pop() {
            // Read the selection before the view changes under it.
            let selection = self.current_selection();
            let here = (std::mem::replace(&mut self.view, view), selection);
            self.forward.push(here);
            self.selected = selected;
            self.clear_filter();
        }
    }

    fn history_forward(&mut self) {
        if let Some((view, selected)) = self.forward.pop() {
            // Read the selection before the view changes under it.
            let selection = self.current_selection();
            let here = (std::mem::replace(&mut self.view, view), selection);
            self.back.push(here);
            self.selected = selected;
            self.clear_filter();
        }
    }

    /// The selection as shown (the largest tile when nothing was chosen).
    fn current_selection(&self) -> Option<PathBuf> {
        self.selected_node().map(|n| n.path.clone())
    }

    fn cycle_selection(&mut self, step: isize) {
        let selectable = self.selectable();
        if selectable.is_empty() {
            return;
        }
        let current = selectable
            .iter()
            .position(|(n, _)| Some(&n.path) == self.selected.as_ref())
            .unwrap_or(0);
        let next = (current as isize + step).rem_euclid(selectable.len() as isize) as usize;
        self.selected = Some(selectable[next].0.path.clone());
    }

    fn move_selection(&mut self, direction: Direction) {
        let selectable = self.selectable();
        let rects: Vec<Rect> = selectable.iter().filter_map(|&(_, r)| r).collect();
        if rects.len() != selectable.len() {
            return;
        }
        let Some(from) = self
            .selected_node()
            .and_then(|s| selectable.iter().find(|(n, _)| std::ptr::eq(*n, s)))
            .and_then(|&(_, r)| r)
        else {
            return;
        };
        if let Some(i) = nav::neighbor(&rects, from, direction) {
            self.selected = Some(selectable[i].0.path.clone());
        }
    }

    /// The directory currently viewed.
    pub fn view_path(&self) -> &Path {
        &self.view
    }

    /// Handle a key while the filter input is active.
    fn filter_key(&mut self, key: KeyEvent) {
        if let Input::Filter(ref mut query) = self.input {
            if edit_text(query, key) {
                return;
            }
            match key.code {
                KeyCode::Enter => {
                    let q = query.clone();
                    self.apply_filter(&q);
                    self.input = Input::None;
                }
                KeyCode::Esc => self.clear_filter(),
                _ => {}
            }
        }
    }

    /// Handle a key while the path input is active.
    fn path_key(&mut self, key: KeyEvent) {
        if let Input::Path(ref mut text) = self.input {
            if edit_text(text, key) {
                return;
            }
            match key.code {
                KeyCode::Enter => {
                    let path = expand_home(text, &self.root);
                    self.start_scan(path);
                }
                KeyCode::Esc => self.input = Input::None,
                _ => {}
            }
        }
    }

    /// Handle a key while the volume picker is active.
    fn volume_key(&mut self, key: KeyEvent) {
        if let Input::Volumes {
            ref mut list,
            ref mut index,
        } = self.input
        {
            match key.code {
                KeyCode::Up => *index = index.saturating_sub(1),
                KeyCode::Down => *index = (*index + 1).min(list.len().saturating_sub(1)),
                KeyCode::Enter if !list.is_empty() => {
                    let vol = &list[*index];
                    match volume::mount_root(&vol.mount_point) {
                        Ok(root) => self.start_scan(root),
                        Err(err) => {
                            self.state = State::Error(format!(
                                "cannot mount {}: {err}",
                                vol.mount_point.display()
                            ));
                        }
                    }
                }
                KeyCode::Esc => self.input = Input::None,
                _ => {}
            }
        }
    }
}

/// Apply a line-editing key to `text`: printable characters, Backspace, and
/// `Ctrl+U` to clear. Returns whether the key was consumed. Other Ctrl/Alt
/// chords are not text.
fn edit_text(text: &mut String, key: KeyEvent) -> bool {
    let chord = key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    match key.code {
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => text.clear(),
        KeyCode::Char(c) if !chord => text.push(c),
        KeyCode::Backspace => {
            text.pop();
        }
        _ => return false,
    }
    true
}

/// `~` and `~/x` relative to `home`; anything else as typed.
fn expand_home(text: &str, home: &Path) -> PathBuf {
    match text.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with('/') => home.join(&rest[1..]),
        _ => PathBuf::from(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use disktree_tui_core::classify::Class;
    use ratatui::crossterm::event::{KeyEventKind, KeyEventState};
    use std::time::Duration;

    fn settings() -> ViewSettings {
        ViewSettings {
            mode: VisMode::Size,
            apparent: false,
            show_hidden: true,
            depth: DrawDepth::DEFAULT,
        }
    }

    fn wait_for_scan(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while matches!(app.state, State::Scanning { .. }) {
            assert!(Instant::now() < deadline, "scan did not finish");
            app.poll_scan();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn with(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            modifiers,
            ..key(code)
        }
    }

    fn press(app: &mut App, codes: &[KeyCode]) {
        for &c in codes {
            assert_eq!(app.handle_key(key(c)), Action::Continue);
        }
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
    }

    fn dir(path: &str, size: u64, children: Vec<DirNode>) -> DirNode {
        let name = path.rsplit('/').next().unwrap();
        let mut n = DirNode::new(path.into(), name.into(), 0);
        n.class = Class::Code;
        for c in children {
            n.absorb_child(c);
        }
        n.size += size;
        n.apparent_size += size / 2;
        n.file_count += 1;
        n
    }

    /// /r: big (with x, y), mid, small.
    fn fixture() -> App {
        let mut root = dir(
            "/r",
            0,
            vec![
                dir(
                    "/r/big",
                    0,
                    vec![dir("/r/big/x", 600, vec![]), dir("/r/big/y", 400, vec![])],
                ),
                dir("/r/mid", 500, vec![]),
                dir("/r/small", 300, vec![]),
            ],
        );
        root.sort_recursive();
        let result = ScanResult {
            root: Box::new(root),
            file_count: 6,
            dir_count: 5,
            unreadable: 0,
            cancelled: false,
            skipped_mounts: Vec::new(),
        };
        let app = App::ready(result, settings(), None);
        app.view_area.set(Some(ViewArea {
            rect: Rect::new(0, 1, 90, 30),
            treemap: true,
        }));
        app
    }

    fn selected(app: &App) -> String {
        app.selected_node()
            .map(|n| n.name.clone())
            .unwrap_or_default()
    }

    fn view(app: &App) -> String {
        app.view_path().display().to_string()
    }

    #[test]
    fn largest_child_is_selected_first() {
        assert_eq!(selected(&fixture()), "big");
    }

    #[test]
    fn tab_cycles_by_size_and_wraps() {
        let mut app = fixture();
        press(&mut app, &[KeyCode::Tab]);
        assert_eq!(selected(&app), "mid");
        press(&mut app, &[KeyCode::Tab, KeyCode::Tab]);
        assert_eq!(selected(&app), "big");
        press(&mut app, &[KeyCode::BackTab]);
        assert_eq!(selected(&app), "small");
    }

    #[test]
    fn arrows_move_spatially() {
        let mut app = fixture();
        // `big` takes the left part; the others sit to its right.
        press(&mut app, &[KeyCode::Right]);
        assert_ne!(selected(&app), "big");
        press(&mut app, &[KeyCode::Left]);
        assert_eq!(selected(&app), "big");
        press(&mut app, &[KeyCode::Left]);
        assert_eq!(selected(&app), "big", "nothing further left");
    }

    #[test]
    fn enter_up_back_forward() {
        let mut app = fixture();
        press(&mut app, &[KeyCode::Enter]);
        assert_eq!(view(&app), "/r/big");
        assert_eq!(selected(&app), "x");

        press(&mut app, &[KeyCode::Esc]);
        assert_eq!(view(&app), "/r");
        assert_eq!(selected(&app), "big", "up selects where we came from");

        app.handle_key(with(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(view(&app), "/r/big");
        app.handle_key(with(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(view(&app), "/r");
        app.handle_key(with(KeyCode::Right, KeyModifiers::ALT));
        assert_eq!(view(&app), "/r/big");

        // Up at the scan root stays put.
        press(&mut app, &[KeyCode::Backspace, KeyCode::Backspace]);
        assert_eq!(view(&app), "/r");
    }

    #[test]
    fn history_restores_each_views_selection() {
        let mut app = fixture();
        // In /r select `mid`, open `big`, select `y` there.
        press(&mut app, &[KeyCode::Enter, KeyCode::Tab]);
        assert_eq!(
            (view(&app).as_str(), selected(&app).as_str()),
            ("/r/big", "y")
        );
        press(&mut app, &[KeyCode::Esc]);
        assert_eq!(selected(&app), "big");
        press(&mut app, &[KeyCode::Tab]);
        assert_eq!(selected(&app), "mid");

        app.handle_key(with(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(
            (view(&app).as_str(), selected(&app).as_str()),
            ("/r/big", "y")
        );
        app.handle_key(with(KeyCode::Right, KeyModifiers::ALT));
        assert_eq!(
            (view(&app).as_str(), selected(&app).as_str()),
            ("/r", "mid")
        );
    }

    #[test]
    fn navigating_clears_forward_history() {
        let mut app = fixture();
        press(&mut app, &[KeyCode::Enter, KeyCode::Esc]);
        app.handle_key(with(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(view(&app), "/r/big");
        press(&mut app, &[KeyCode::Esc, KeyCode::Tab, KeyCode::Enter]);
        assert_eq!(view(&app), "/r/mid");
        app.handle_key(with(KeyCode::Right, KeyModifiers::ALT));
        assert_eq!(view(&app), "/r/mid", "forward history was cleared");
    }

    #[test]
    fn filter_preview_apply_and_clear() {
        let mut app = fixture();
        press(&mut app, &[KeyCode::Char('/')]);
        typed(&mut app, "MI");
        assert_eq!(app.input, Input::Filter("MI".into()));
        assert_eq!(app.preview_count(), 1);
        press(&mut app, &[KeyCode::Enter]);
        assert_eq!(app.input, Input::None);
        let names: Vec<_> = app
            .view_node()
            .unwrap()
            .children
            .iter()
            .map(|c| c.name.clone())
            .collect();
        assert_eq!(names, ["mid"]);
        assert_eq!(selected(&app), "mid");

        press(&mut app, &[KeyCode::Esc]);
        assert!(app.filter.is_none());
        assert_eq!(app.view_node().unwrap().children.len(), 3);
        assert_eq!(
            view(&app),
            "/r",
            "Esc cleared the filter instead of going up"
        );
    }

    #[test]
    fn mode_toggles() {
        let mut app = fixture();
        press(&mut app, &[KeyCode::Char('d')]);
        assert_eq!(app.settings.weight(), Weight::Apparent);
        press(&mut app, &[KeyCode::Char('d')]);
        assert_eq!(app.settings.weight(), Weight::Size);

        let mut seen = Vec::new();
        for _ in 0..4 {
            press(&mut app, &[KeyCode::Char('t')]);
            seen.push((app.settings.mode, app.settings.apparent));
        }
        assert_eq!(
            seen,
            [
                (VisMode::Size, true),
                (VisMode::Files, false),
                (VisMode::Age, false),
                (VisMode::Size, false),
            ]
        );
    }

    #[test]
    fn brackets_step_depth() {
        let mut app = fixture();
        assert_eq!(app.settings.depth, DrawDepth::Levels(2));
        press(&mut app, &[KeyCode::Char('[')]);
        assert_eq!(app.settings.depth, DrawDepth::Levels(1));
        press(&mut app, &[KeyCode::Char('[')]);
        assert_eq!(app.settings.depth, DrawDepth::Levels(1));
        // The fixture is 2 levels deep: `]` from 1 reaches unlimited.
        press(&mut app, &[KeyCode::Char(']')]);
        assert_eq!(app.settings.depth, DrawDepth::Unlimited);
    }

    #[test]
    fn tree_view_uses_up_down_for_selection() {
        let mut app = fixture();
        app.view_area.set(Some(ViewArea {
            rect: Rect::new(0, 1, 60, 20),
            treemap: false,
        }));
        press(&mut app, &[KeyCode::Down]);
        assert_eq!(selected(&app), "mid");
        press(&mut app, &[KeyCode::Right]);
        assert_eq!(view(&app), "/r/mid");
        press(&mut app, &[KeyCode::Left]);
        assert_eq!(view(&app), "/r");
    }

    #[test]
    fn quit_keys() {
        let mut app = fixture();
        assert_eq!(app.handle_key(key(KeyCode::Char('q'))), Action::Quit);
        assert_eq!(
            app.handle_key(with(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
        // Esc no longer quits; at the root it does nothing.
        assert_eq!(app.handle_key(key(KeyCode::Esc)), Action::Continue);
    }

    #[test]
    fn path_input_and_volume_picker_open_and_close() {
        let mut app = fixture();
        app.handle_key(with(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(app.input, Input::Path("/r".into()));
        // Typing goes into the input, not to key bindings.
        typed(&mut app, "q");
        assert_eq!(app.input, Input::Path("/rq".into()));
        press(&mut app, &[KeyCode::Esc]);
        assert_eq!(app.input, Input::None);

        press(&mut app, &[KeyCode::Char('v')]);
        assert!(matches!(app.input, Input::Volumes { .. }));
        press(&mut app, &[KeyCode::Esc]);
        assert_eq!(app.input, Input::None);
    }

    #[test]
    fn text_inputs_ignore_chords_and_clear_on_ctrl_u() {
        let mut app = fixture();
        app.handle_key(with(KeyCode::Char('o'), KeyModifiers::CONTROL));
        app.handle_key(with(KeyCode::Char('x'), KeyModifiers::ALT));
        app.handle_key(with(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(app.input, Input::Path("/r".into()));
        app.handle_key(with(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(app.input, Input::Path(String::new()));
        // Shifted capitals are text.
        app.handle_key(with(KeyCode::Char('A'), KeyModifiers::SHIFT));
        press(&mut app, &[KeyCode::Backspace, KeyCode::Char('b')]);
        assert_eq!(app.input, Input::Path("b".into()));
    }

    #[test]
    fn expand_home_paths() {
        let home = Path::new("/home/u");
        assert_eq!(expand_home("~", home), PathBuf::from("/home/u"));
        assert_eq!(expand_home("~/x", home), PathBuf::from("/home/u/x"));
        assert_eq!(expand_home("/tmp", home), PathBuf::from("/tmp"));
    }

    #[test]
    fn background_scan_reaches_ready() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f"), b"hello").unwrap();
        let mut app = App::start(tmp.path().to_path_buf(), settings());
        wait_for_scan(&mut app);
        assert_eq!(app.tree().unwrap().apparent_size, 5);
        assert!(app.disk.is_some());
    }

    #[test]
    fn rescan_keeps_the_view_and_hidden_toggle_rescans() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        std::fs::write(tmp.path().join("a/b/f"), b"hello").unwrap();
        std::fs::write(tmp.path().join(".hidden"), b"secret!").unwrap();
        let mut app = App::start(tmp.path().to_path_buf(), settings());
        wait_for_scan(&mut app);
        press(&mut app, &[KeyCode::Enter]);
        let inside = app.view_path().to_path_buf();
        assert!(inside.ends_with("a"));

        press(&mut app, &[KeyCode::Char('r')]);
        assert!(matches!(app.state, State::Scanning { .. }));
        wait_for_scan(&mut app);
        assert_eq!(app.view_path(), inside);

        assert_eq!(app.tree().unwrap().apparent_size, 12);
        press(&mut app, &[KeyCode::Char('i')]);
        wait_for_scan(&mut app);
        assert!(!app.settings.show_hidden);
        assert_eq!(app.tree().unwrap().apparent_size, 5);
    }

    #[test]
    fn escape_cancels_a_running_scan() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Create many files so the scan takes time to cancel.
        for i in 0..10_000 {
            let path = tmp.path().join(format!("file_{i:05}.bin"));
            std::fs::write(path, b"hello").unwrap();
        }
        let mut app = App::start(tmp.path().to_path_buf(), settings());
        app.handle_key(key(KeyCode::Esc));
        // Scan is cancelled, should show partial results eventually.
        wait_for_scan(&mut app);
        assert!(matches!(app.state, State::Ready(_)));
        assert!(app.result().unwrap().cancelled);
    }

    #[test]
    fn missing_root_reaches_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut app = App::start(tmp.path().join("missing"), settings());
        wait_for_scan(&mut app);
        let State::Error(message) = &app.state else {
            panic!("expected Error");
        };
        assert!(message.contains("cannot scan"));
    }

    #[test]
    fn weights_follow_mode_and_apparent() {
        let mut s = settings();
        assert_eq!(s.weight(), Weight::Size);
        s.apparent = true;
        assert_eq!(s.weight(), Weight::Apparent);
        s.mode = VisMode::Files;
        assert_eq!(s.weight(), Weight::Files);
    }

    // --- M6: Marking tests ---

    #[test]
    fn mark_tile_with_space() {
        let mut app = fixture();
        assert!(app.marked.is_empty());
        // 'big' is selected.
        press(&mut app, &[KeyCode::Char(' ')]);
        assert!(app.is_marked(&app.tree().unwrap().children[0]));
    }

    #[test]
    fn mark_with_x_key() {
        let mut app = fixture();
        press(&mut app, &[KeyCode::Char('X')]);
        assert!(app.is_marked(&app.tree().unwrap().children[0]));
    }

    #[test]
    fn unmark_removes_selection() {
        let mut app = fixture();
        press(&mut app, &[KeyCode::Char(' ')]);
        assert!(app.is_marked(&app.tree().unwrap().children[0]));
        // Navigate to another tile and mark it.
        press(&mut app, &[KeyCode::Right]);
        press(&mut app, &[KeyCode::Char(' ')]);
        // Now mark the first tile again: both should be unmarked (absorption).
        press(&mut app, &[KeyCode::Left]);
        press(&mut app, &[KeyCode::Char(' ')]);
        assert!(!app.is_marked(&app.tree().unwrap().children[0]));
    }

    #[test]
    fn mark_absorbs_descendants() {
        let mut app = fixture();
        // Mark "big" — should absorb "x" and "y" too.
        press(&mut app, &[KeyCode::Enter]); // go into big
        assert_eq!(view(&app), "/r/big");
        press(&mut app, &[KeyCode::Esc]); // go back to /r
        press(&mut app, &[KeyCode::Char(' ')]); // mark big

        // x and y should be marked too.
        let tree = app.tree().unwrap();
        let big = tree.children.iter().find(|c| c.name == "big").unwrap();
        let x = big.children.iter().find(|c| c.name == "x").unwrap();
        let y = big.children.iter().find(|c| c.name == "y").unwrap();
        assert!(app.is_marked(x), "x should be marked via absorption");
        assert!(app.is_marked(y), "y should be marked via absorption");
    }

    #[test]
    fn marked_paths_returns_only_direct_marked() {
        let mut app = fixture();
        // Mark "big" — should absorb "x" and "y".
        press(&mut app, &[KeyCode::Enter]); // go into big
        press(&mut app, &[KeyCode::Esc]); // go back to /r
        press(&mut app, &[KeyCode::Char(' ')]); // mark big

        let paths = app.marked_paths();
        // Should only return "big", not "x" or "y" (absorption).
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].path, PathBuf::from("/r/big"));
    }

    #[test]
    fn review_screen_opens_with_marks() {
        let mut app = fixture();
        // No marks: C does nothing.
        press(&mut app, &[KeyCode::Char('C')]);
        assert!(matches!(app.input, Input::None));

        // Mark something first.
        press(&mut app, &[KeyCode::Char(' ')]);
        press(&mut app, &[KeyCode::Char('C')]);
        assert!(matches!(app.input, Input::Review(_)));
    }

    #[test]
    fn review_navigate_and_unmark() {
        let mut app = fixture();
        press(&mut app, &[KeyCode::Char(' ')]); // mark big
        press(&mut app, &[KeyCode::Tab]); // navigate to mid
        press(&mut app, &[KeyCode::Char(' ')]); // mark mid
        press(&mut app, &[KeyCode::Char('C')]);

        assert!(matches!(app.input, Input::Review(_)));
        if let Input::Review(state) = &app.input {
            assert_eq!(state.selected, 0);
        }

        // Navigate down.
        press(&mut app, &[KeyCode::Down]);
        if let Input::Review(state) = &app.input {
            assert_eq!(state.selected, 1);
        }

        // Unmark the selected entry.
        press(&mut app, &[KeyCode::Char('u')]);
        // "mid" should no longer be in marked.
        let tree = app.tree().unwrap();
        let mid = tree.children.iter().find(|c| c.name == "mid").unwrap();
        assert!(!app.is_marked(mid));
    }

    #[test]
    fn review_esc_close() {
        let mut app = fixture();
        press(&mut app, &[KeyCode::Char(' ')]);
        press(&mut app, &[KeyCode::Char('C')]);
        press(&mut app, &[KeyCode::Esc]);
        assert!(matches!(app.input, Input::None));
    }

    #[test]
    fn review_unmark_all() {
        let mut app = fixture();
        press(&mut app, &[KeyCode::Char(' ')]); // mark big
        press(&mut app, &[KeyCode::Char(' ')]); // mark mid
        press(&mut app, &[KeyCode::Char('C')]);
        press(&mut app, &[KeyCode::Char('!')]);
        assert!(app.marked.is_empty());
        press(&mut app, &[KeyCode::Esc]);
        assert!(matches!(app.input, Input::None));
    }

    #[test]
    fn ctrl_space_marks_the_selection() {
        let mut app = fixture();
        press(&mut app, &[KeyCode::Tab]); // select mid
        app.handle_key(with(KeyCode::Char(' '), KeyModifiers::CONTROL));
        let tree = app.tree().unwrap();
        let mid = tree.children.iter().find(|c| c.name == "mid").unwrap();
        assert!(app.is_marked(mid));
        assert_eq!(selected(&app), "mid");
    }

    fn mouse_down(column: u16, row: u16, ctrl: bool) -> MouseEvent {
        let mut modifiers = KeyModifiers::NONE;
        if ctrl {
            modifiers |= KeyModifiers::CONTROL;
        }
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers,
        }
    }

    #[test]
    fn click_selects_and_ctrl_click_marks() {
        let mut app = fixture();
        let area = app.view_area.get().unwrap();
        let big = app
            .layout_in(area.rect)
            .unwrap()
            .tiles
            .into_iter()
            .find(|t| t.depth == 1 && matches!(&t.kind, TileKind::Dir(n) if n.name == "big"))
            .unwrap();
        let spot = (
            big.rect.x + big.rect.width / 2,
            big.rect.y + big.rect.height / 2,
        );

        app.handle_mouse(mouse_down(spot.0, spot.1, false));
        assert_eq!(selected(&app), "big", "plain click moves the selection");

        press(&mut app, &[KeyCode::Tab]); // select mid
        app.handle_mouse(mouse_down(spot.0, spot.1, true));
        let tree = app.tree().unwrap();
        let big_node = tree.children.iter().find(|c| c.name == "big").unwrap();
        assert!(app.is_marked(big_node), "Ctrl+Click marks the tile");
        assert_eq!(selected(&app), "mid", "Ctrl+Click keeps the selection");
    }

    #[test]
    fn confirmed_removal_reports_and_keeps_failed_marks() {
        let mut app = fixture();
        press(&mut app, &[KeyCode::Char(' ')]); // mark big
        press(&mut app, &[KeyCode::Char('C')]);
        press(&mut app, &[KeyCode::Char('m')]); // ask to trash
        if let Input::Review(state) = &app.input {
            assert_eq!(state.confirm, Some(ConfirmState::Trash));
        } else {
            panic!("expected the review screen");
        }
        press(&mut app, &[KeyCode::Char('n')]); // cancel
        assert!(!app.marked.is_empty(), "cancel keeps the marks");

        press(&mut app, &[KeyCode::Char('p')]); // ask for permanent delete
        press(&mut app, &[KeyCode::Char('y')]);
        assert!(matches!(app.input, Input::None));
        let report = app.last_removal.expect("removal report");
        assert_eq!(report.succeeded, [PathBuf::from("/r/big")]);
        assert!(report.failed.is_empty());
        assert_eq!(report.freed_bytes, 1000);
        assert!(app.marked.is_empty());
        assert!(app.rescan_pending, "a rescan follows the removal");
    }
}
