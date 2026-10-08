//! Application state: what is on screen, the scan feeding it, and how keys
//! change it.

use std::cell::Cell;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Instant;

use disktree_tui_core::filter::Filter;
use disktree_tui_core::layout::{self, DrawDepth, Layout, LayoutOptions, Rect, TileKind, Weight};
use disktree_tui_core::platform;
use disktree_tui_core::scan::{ScanConfig, ScanProgress, ScanResult, Scanner};
use disktree_tui_core::tree::DirNode;
use disktree_tui_core::volume::{self, DiskSpace, Volume};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::watch;

use crate::VisMode;
use crate::nav::{self, Direction};

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
            _ => {}
        }
        Action::Continue
    }

    fn filter_key(&mut self, key: KeyEvent) {
        let Input::Filter(query) = &mut self.input else {
            return;
        };
        if edit_text(query, key) {
            return;
        }
        match key.code {
            KeyCode::Enter => {
                let query = std::mem::take(query);
                self.input = Input::None;
                self.apply_filter(&query);
            }
            KeyCode::Esc => {
                self.input = Input::None;
                self.clear_filter();
            }
            _ => {}
        }
    }

    fn path_key(&mut self, key: KeyEvent) {
        let Input::Path(text) = &mut self.input else {
            return;
        };
        if edit_text(text, key) {
            return;
        }
        match key.code {
            KeyCode::Enter => {
                let path = expand_home(text.trim(), &platform::home_dir());
                self.input = Input::None;
                self.start_scan(path);
            }
            KeyCode::Esc => self.input = Input::None,
            _ => {}
        }
    }

    fn volume_key(&mut self, key: KeyEvent) {
        let Input::Volumes { list, index } = &mut self.input else {
            return;
        };
        match key.code {
            KeyCode::Up => *index = index.saturating_sub(1),
            KeyCode::Down => *index = (*index + 1).min(list.len().saturating_sub(1)),
            KeyCode::Enter => {
                let chosen = list.get(*index).map(|v| v.mount_point.clone());
                self.input = Input::None;
                if let Some(path) = chosen {
                    self.start_scan(path);
                }
            }
            KeyCode::Esc | KeyCode::Char('q' | 'v' | 'V') => self.input = Input::None,
            _ => {}
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
        let mut app = App::start(tmp.path().to_path_buf(), settings());
        press(&mut app, &[KeyCode::Esc]);
        wait_for_scan(&mut app);
        assert!(app.tree().is_some());
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
}
