//! Theme and screen composition: top bar, main area, side panel, bottom bar.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use disktree_tui_core::classify::Class;
use disktree_tui_core::layout::{self, LayoutOptions, Rect as LayoutRect, TileKind};
use disktree_tui_core::platform;
use disktree_tui_core::tree::{DirNode, format_size};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{App, State};
use crate::{tree_view, treemap_view};

/// Below this width the side panel is dropped and the tree view replaces the
/// treemap.
pub const MIN_TREEMAP_COLUMNS: u16 = 80;

/// Width of the side panel.
const SIDE_PANEL_WIDTH: u16 = 30;

/// What the terminal can display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    /// 24-bit color.
    TrueColor,
    /// The xterm 256-color palette.
    Ansi256,
    /// The 16 basic ANSI colors.
    Ansi16,
    /// No color (`NO_COLOR`): labels and hatching carry the meaning.
    Mono,
}

impl ColorDepth {
    /// Detect from the environment.
    pub fn detect() -> Self {
        let var = |name| std::env::var(name).unwrap_or_default();
        Self::from_env(
            std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
            &var("COLORTERM"),
            &var("TERM"),
        )
    }

    fn from_env(no_color: bool, colorterm: &str, term: &str) -> Self {
        if no_color {
            Self::Mono
        } else if matches!(colorterm, "truecolor" | "24bit") {
            Self::TrueColor
        } else if term.contains("256color") {
            Self::Ansi256
        } else {
            Self::Ansi16
        }
    }
}

/// An RGB triple.
type Rgb = (u8, u8, u8);

/// Class colors and the rest of the palette (PRD 13.3).
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    depth: ColorDepth,
}

impl Theme {
    /// A theme for the given terminal capability.
    pub fn new(depth: ColorDepth) -> Self {
        Self { depth }
    }

    fn class_rgb(class: Class) -> Rgb {
        match class {
            Class::Code => (0x00, 0x00, 0xAF),
            Class::Caches => (0xAF, 0x87, 0x00),
            Class::Build => (0x87, 0x00, 0x87),
            Class::Git => (0x00, 0x87, 0x00),
            Class::Media => (0x87, 0x00, 0x00),
            Class::Documents => (0x00, 0x87, 0x87),
            Class::Toolchains => (0x87, 0x87, 0x87),
            Class::Synced => (0x00, 0x5F, 0xFF),
            Class::Agent => (0x00, 0xAF, 0x5F),
            Class::Unknown => (0x40, 0x40, 0x40),
        }
    }

    fn class_ansi16(class: Class) -> Color {
        match class {
            Class::Code => Color::Blue,
            Class::Caches => Color::Yellow,
            Class::Build => Color::Magenta,
            Class::Git => Color::Green,
            Class::Media => Color::Red,
            Class::Documents => Color::Cyan,
            Class::Toolchains => Color::Gray,
            Class::Synced => Color::LightBlue,
            Class::Agent => Color::LightGreen,
            Class::Unknown => Color::DarkGray,
        }
    }

    fn rgb(self, rgb: Rgb) -> Color {
        match self.depth {
            ColorDepth::TrueColor => Color::Rgb(rgb.0, rgb.1, rgb.2),
            ColorDepth::Ansi256 => Color::Indexed(ansi256(rgb)),
            ColorDepth::Ansi16 | ColorDepth::Mono => Color::Reset,
        }
    }

    /// Tile background for a class at a nesting depth; deeper is lighter.
    pub fn tile(self, class: Class, depth: usize) -> Color {
        match self.depth {
            ColorDepth::Ansi16 => Self::class_ansi16(class),
            ColorDepth::Mono => Color::Reset,
            _ => {
                let lift = 0.14 * depth.saturating_sub(1).min(4) as f64;
                self.rgb(mix(Self::class_rgb(class), (0xFF, 0xFF, 0xFF), lift))
            }
        }
    }

    /// Label band of a subdivided tile: a darker shade of its color.
    pub fn band(self, class: Class, depth: usize) -> Color {
        match self.depth {
            ColorDepth::Ansi16 => Self::class_ansi16(class),
            ColorDepth::Mono => Color::Reset,
            _ => {
                let lift = 0.14 * depth.saturating_sub(1).min(4) as f64;
                let base = mix(Self::class_rgb(class), (0xFF, 0xFF, 0xFF), lift);
                self.rgb(mix(base, (0, 0, 0), 0.35))
            }
        }
    }

    /// Hatch strokes over a reclaimable tile: one level darker than its fill.
    pub fn hatch(self, class: Class, depth: usize) -> Color {
        match self.depth {
            ColorDepth::Ansi16 => Color::Black,
            ColorDepth::Mono => Color::Reset,
            _ => {
                let lift = 0.14 * depth.saturating_sub(1).min(4) as f64;
                let base = mix(Self::class_rgb(class), (0xFF, 0xFF, 0xFF), lift);
                self.rgb(mix(base, (0, 0, 0), 0.45))
            }
        }
    }

    /// Text on tiles.
    pub fn tile_text(self) -> Color {
        match self.depth {
            ColorDepth::Mono => Color::Reset,
            _ => Color::White,
        }
    }

    /// Thin separator drawn on each tile's left edge.
    pub fn separator(self) -> Color {
        match self.depth {
            ColorDepth::TrueColor | ColorDepth::Ansi256 => self.rgb((0x12, 0x12, 0x12)),
            ColorDepth::Ansi16 => Color::Black,
            ColorDepth::Mono => Color::Reset,
        }
    }

    /// Highlight for the selected tile.
    pub fn selection(self) -> Style {
        match self.depth {
            ColorDepth::Mono => Style::new().add_modifier(Modifier::REVERSED),
            ColorDepth::Ansi16 => Style::new().bg(Color::LightYellow).fg(Color::Black),
            _ => Style::new()
                .bg(self.rgb((0xFF, 0xAF, 0x00)))
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        }
    }

    /// Top and bottom bars.
    pub fn bar(self) -> Style {
        match self.depth {
            ColorDepth::Mono => Style::new().add_modifier(Modifier::REVERSED),
            ColorDepth::Ansi16 => Style::new().bg(Color::DarkGray).fg(Color::White),
            _ => Style::new()
                .bg(self.rgb((0x26, 0x26, 0x26)))
                .fg(Color::White),
        }
    }

    /// Swatch for a class in the legend.
    pub fn swatch(self, class: Class) -> Style {
        match self.depth {
            ColorDepth::Mono => Style::new(),
            _ => Style::new().fg(self.tile(class, 1)),
        }
    }
}

/// Linear blend of `a` toward `b` by `t` in 0..=1.
fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let ch = |x: u8, y: u8| (f64::from(x) + (f64::from(y) - f64::from(x)) * t).round() as u8;
    (ch(a.0, b.0), ch(a.1, b.1), ch(a.2, b.2))
}

/// Nearest xterm-256 palette index: the 6x6x6 cube or the gray ramp.
fn ansi256((r, g, b): Rgb) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let nearest_level = |v: u8| {
        (0..6)
            .min_by_key(|&i| (i32::from(LEVELS[i]) - i32::from(v)).abs())
            .expect("non-empty")
    };
    let (ri, gi, bi) = (nearest_level(r), nearest_level(g), nearest_level(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
    let cube_index = 16 + 36 * ri + 6 * gi + bi;

    let avg = (u32::from(r) + u32::from(g) + u32::from(b)) / 3;
    let gray_step = ((avg.saturating_sub(8) + 5) / 10).min(23);
    let gray = (8 + 10 * gray_step) as u8;
    let dist = |c: Rgb| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(c.0, r) + d(c.1, g) + d(c.2, b)
    };
    if dist((gray, gray, gray)) < dist(cube) {
        (232 + gray_step) as u8
    } else {
        cube_index as u8
    }
}

/// Draw the whole screen.
pub fn render(frame: &mut Frame, app: &App, theme: Theme) {
    let [top, body, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_top_bar(frame, top, app, theme);

    match &app.state {
        State::Scanning { progress, started } => {
            let p = *progress.borrow();
            let text = vec![
                Line::from(""),
                Line::from(format!("Scanning {} …", app.root.display())),
                Line::from(""),
                Line::from(format!(
                    "{} files   {} dirs   {}   {:.1}s",
                    p.files,
                    p.dirs,
                    format_size(p.bytes),
                    started.elapsed().as_secs_f64()
                )),
            ];
            frame.render_widget(Paragraph::new(text).centered(), body);
            render_bottom_text(frame, bottom, theme, "q quit");
        }
        State::Error(message) => {
            let text = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Error",
                    Style::new().add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(message.as_str()),
            ];
            frame.render_widget(Paragraph::new(text).centered(), body);
            render_bottom_text(frame, bottom, theme, "q quit");
        }
        State::Ready(result) => {
            let root = &result.root;
            let wide = body.width >= MIN_TREEMAP_COLUMNS;
            let (main, side) = if wide {
                let [main, side] =
                    Layout::horizontal([Constraint::Min(0), Constraint::Length(SIDE_PANEL_WIDTH)])
                        .areas(body);
                (main, Some(side))
            } else {
                (body, None)
            };

            let options = LayoutOptions {
                weight: app.settings.weight(),
                max_depth: app.settings.depth.max_depth(),
                ..LayoutOptions::default()
            };
            let tiles = layout::layout(root, to_layout_rect(main), &options);
            let selected = selected_node(&tiles.tiles);

            if wide && !tiles.too_small {
                treemap_view::render(frame.buffer_mut(), &tiles.tiles, selected, theme);
            } else {
                tree_view::render(frame.buffer_mut(), main, root, app.settings, theme);
            }
            if let Some(side) = side {
                render_side_panel(frame, side, app, result, theme);
            }
            render_selection_bar(frame, bottom, selected.unwrap_or(root), theme);
        }
    }
}

/// The selected tile. Until navigation lands (M5), the largest top-level
/// directory.
fn selected_node<'a>(tiles: &[layout::Tile<'a>]) -> Option<&'a DirNode> {
    tiles.iter().find_map(|t| match t.kind {
        TileKind::Dir(node) if t.depth == 1 => Some(node),
        _ => None,
    })
}

fn to_layout_rect(r: Rect) -> LayoutRect {
    LayoutRect::new(r.x, r.y, r.width, r.height)
}

fn render_top_bar(frame: &mut Frame, area: Rect, app: &App, theme: Theme) {
    let s = app.settings;
    let indicators = format!(
        "[{}] [{}] [{}] [depth:{}] ",
        s.mode,
        if s.apparent { "apparent" } else { "disk" },
        if s.show_hidden { "hidden" } else { "no hidden" },
        s.depth,
    );
    let crumbs = breadcrumb(&app.root, &platform::home_dir());
    let room = usize::from(area.width).saturating_sub(indicators.chars().count() + 1);
    let crumbs = truncate_left(&crumbs, room);
    let gap = usize::from(area.width)
        .saturating_sub(crumbs.chars().count() + indicators.chars().count() + 1);
    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled(crumbs, Style::new().add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(gap)),
        Span::raw(indicators),
    ]);
    frame.render_widget(Paragraph::new(line).style(theme.bar()), area);
}

/// `~ › Projects › app`, or `/ › var › log` outside home.
fn breadcrumb(path: &Path, home: &Path) -> String {
    let (start, rest) = match path.strip_prefix(home) {
        Ok(rest) => ("~".to_string(), rest),
        Err(_) => match path.strip_prefix("/") {
            Ok(rest) => ("/".to_string(), rest),
            Err(_) => (String::new(), path),
        },
    };
    let mut parts = Vec::new();
    if !start.is_empty() {
        parts.push(start);
    }
    parts.extend(rest.iter().map(|c| c.to_string_lossy().into_owned()));
    parts.join(" › ")
}

/// Keep the end of `s` within `max` characters, marking the cut with `…`.
fn truncate_left(s: &str, max: usize) -> String {
    let len = s.chars().count();
    if len <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let tail: String = s.chars().skip(len - (max - 1)).collect();
    format!("…{tail}")
}

fn render_bottom_text(frame: &mut Frame, area: Rect, theme: Theme, text: &str) {
    frame.render_widget(Paragraph::new(format!(" {text}")).style(theme.bar()), area);
}

fn render_selection_bar(frame: &mut Frame, area: Rect, node: &DirNode, theme: Theme) {
    let mut parts = vec![
        node.path.display().to_string(),
        format_size(node.size),
        format!("{} files", node.file_count),
    ];
    if let Some(ts) = node.last_write {
        parts.push(format!("modified {}", ago(ts, now_secs())));
    }
    let reclaimable = if node.reclaimable { " reclaimable" } else { "" };
    parts.push(format!("[{}]{reclaimable}", node.class.label()));
    render_bottom_text(frame, area, theme, &parts.join("  "));
}

fn render_side_panel(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    result: &disktree_tui_core::scan::ScanResult,
    theme: Theme,
) {
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let root = &result.root;
    let mut lines = vec![
        Line::from(Span::styled("Scan", bold)),
        Line::from(format!(" {}", format_size(root.size))),
        Line::from(format!(
            " {} files, {} dirs",
            result.file_count, result.dir_count
        )),
        Line::from(format!(
            " reclaimable {}",
            format_size(root.reclaimable_size())
        )),
    ];
    if result.unreadable > 0 {
        lines.push(Line::from(format!(" {} unreadable", result.unreadable)));
    }
    if let Some(disk) = app.disk {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Disk", bold)));
        lines.push(Line::from(format!(
            " free  {}",
            format_size(disk.available)
        )));
        lines.push(Line::from(format!(" total {}", format_size(disk.total))));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Legend", bold)));
    for class in Class::ALL {
        lines.push(Line::from(vec![
            Span::styled(" ██ ", theme.swatch(class)),
            Span::raw(class.label()),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled(" ╱╱ ", Style::new()),
        Span::raw("reclaimable"),
    ]));
    let block = Block::new().borders(Borders::LEFT).title(" disktree ");
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Rough relative time: `just now`, `5 min ago`, `3 days ago`, ...
fn ago(then: u64, now: u64) -> String {
    let secs = now.saturating_sub(then);
    let (n, unit) = match secs {
        0..60 => return "just now".into(),
        60..3_600 => (secs / 60, "min"),
        3_600..86_400 => (secs / 3_600, "hour"),
        86_400..1_209_600 => (secs / 86_400, "day"),
        1_209_600..5_184_000 => (secs / 604_800, "week"),
        5_184_000..31_536_000 => (secs / 2_592_000, "month"),
        _ => (secs / 31_536_000, "year"),
    };
    let plural = if n == 1 || unit == "min" { "" } else { "s" };
    format!("{n} {unit}{plural} ago")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VisMode;
    use crate::app::ViewSettings;
    use disktree_tui_core::layout::DrawDepth;
    use disktree_tui_core::scan::ScanResult;
    use disktree_tui_core::volume::DiskSpace;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn settings() -> ViewSettings {
        ViewSettings {
            mode: VisMode::Size,
            apparent: false,
            show_hidden: true,
            depth: DrawDepth::DEFAULT,
        }
    }

    fn dir(path: &str, size: u64, class: Class, children: Vec<DirNode>) -> DirNode {
        let name = path.rsplit('/').next().unwrap();
        let mut n = DirNode::new(path.into(), name.into(), path.matches('/').count() - 2);
        n.class = class;
        for c in children {
            n.absorb_child(c);
        }
        n.size += size;
        n.file_count += 3;
        n
    }

    fn fixture() -> App {
        let mut cache = dir("/home/u/.cache", 4 << 30, Class::Caches, vec![]);
        cache.reclaimable = true;
        let projects = dir(
            "/home/u/Projects",
            0,
            Class::Code,
            vec![
                dir("/home/u/Projects/app", 2 << 30, Class::Code, vec![]),
                dir("/home/u/Projects/site", 1 << 30, Class::Code, vec![]),
            ],
        );
        let downloads = dir("/home/u/Downloads", 6 << 30, Class::Documents, vec![]);
        let mut root = dir(
            "/home/u",
            0,
            Class::Unknown,
            vec![downloads, cache, projects],
        );
        root.depth = 0;
        root.sort_recursive();
        let result = ScanResult {
            root: Box::new(root),
            file_count: 15,
            dir_count: 6,
            unreadable: 0,
            cancelled: false,
            skipped_mounts: Vec::new(),
        };
        let disk = DiskSpace {
            available: 100 << 30,
            total: 500 << 30,
        };
        App::ready(result, settings(), Some(disk))
    }

    fn draw(app: &App, width: u16, height: u16) -> (String, ratatui::buffer::Buffer) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let theme = Theme::new(ColorDepth::TrueColor);
        terminal.draw(|f| render(f, app, theme)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let text = (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>() + "\n")
            .collect();
        (text, buf)
    }

    #[test]
    fn ready_screen_has_all_regions() {
        let (screen, buf) = draw(&fixture(), 120, 32);
        let lines: Vec<&str> = screen.lines().collect();
        // Top bar: breadcrumb and indicators.
        assert!(
            lines[0].contains("[size] [disk] [hidden] [depth:2]"),
            "{}",
            lines[0]
        );
        // Treemap tiles.
        for name in ["Downloads", ".cache", "Projects", "app", "site"] {
            assert!(screen.contains(name), "missing {name}:\n{screen}");
        }
        // Side panel: scan totals, disk, legend.
        for text in [
            "reclaimable 4.0 GiB",
            "free  100.0 GiB",
            "Legend",
            "Toolchains",
            "╱╱ reclaimable",
        ] {
            assert!(screen.contains(text), "missing {text}:\n{screen}");
        }
        // Bottom bar describes the selected (largest) tile.
        let bottom = lines.last().unwrap();
        assert!(
            bottom.contains("/home/u/Downloads") && bottom.contains("[Documents]"),
            "{bottom}"
        );
        // The selected tile is highlighted.
        let selection = Theme::new(ColorDepth::TrueColor).selection().bg.unwrap();
        assert!((0..120).any(|x| buf[(x, 1)].bg == selection));
    }

    #[test]
    fn narrow_terminal_falls_back_to_tree_view() {
        let (screen, _) = draw(&fixture(), 60, 20);
        assert!(!screen.contains("Legend"), "{screen}");
        assert!(screen.contains("6.0 GiB  Downloads/"), "{screen}");
        assert!(screen.contains("  ██    2.0 GiB  app/"), "{screen}");
    }

    #[test]
    fn resize_redraws_for_new_size() {
        let app = fixture();
        let (wide, _) = draw(&app, 140, 40);
        let (narrow, _) = draw(&app, 70, 20);
        assert!(wide.contains("Legend"));
        assert!(narrow.contains("Downloads/"));
    }

    #[test]
    fn scanning_and_error_screens() {
        let (tx, rx) = tokio::sync::watch::channel(disktree_tui_core::scan::ScanProgress {
            files: 1234,
            dirs: 56,
            bytes: 7 << 20,
        });
        let mut app = fixture();
        app.state = State::Scanning {
            progress: rx,
            started: std::time::Instant::now(),
        };
        let (screen, _) = draw(&app, 100, 20);
        assert!(
            screen.contains("Scanning")
                && screen.contains("1234 files")
                && screen.contains("7.0 MiB"),
            "{screen}"
        );
        drop(tx);

        app.state = State::Error("cannot scan /nope: No such file".into());
        let (screen, _) = draw(&app, 100, 20);
        assert!(screen.contains("cannot scan /nope"), "{screen}");
    }

    #[test]
    fn color_depth_detection() {
        assert_eq!(
            ColorDepth::from_env(true, "truecolor", "xterm-256color"),
            ColorDepth::Mono
        );
        assert_eq!(
            ColorDepth::from_env(false, "truecolor", "xterm"),
            ColorDepth::TrueColor
        );
        assert_eq!(
            ColorDepth::from_env(false, "", "xterm-256color"),
            ColorDepth::Ansi256
        );
        assert_eq!(ColorDepth::from_env(false, "", "linux"), ColorDepth::Ansi16);
    }

    #[test]
    fn ansi256_maps_cube_and_grays() {
        assert_eq!(ansi256((0, 0, 0xAF)), 19);
        assert_eq!(ansi256((0xAF, 0x87, 0x00)), 136);
        assert_eq!(ansi256((0x40, 0x40, 0x40)), 238);
        assert_eq!(ansi256((0xFF, 0xFF, 0xFF)), 231);
    }

    #[test]
    fn deeper_tiles_are_lighter() {
        let theme = Theme::new(ColorDepth::TrueColor);
        let (Color::Rgb(r1, g1, b1), Color::Rgb(r3, g3, b3)) =
            (theme.tile(Class::Code, 1), theme.tile(Class::Code, 3))
        else {
            panic!("expected RGB");
        };
        assert!(
            u32::from(r3) + u32::from(g3) + u32::from(b3)
                > u32::from(r1) + u32::from(g1) + u32::from(b1)
        );
    }

    #[test]
    fn breadcrumb_abbreviates_home() {
        let home = Path::new("/home/u");
        assert_eq!(
            breadcrumb(Path::new("/home/u/Projects/app"), home),
            "~ › Projects › app"
        );
        assert_eq!(breadcrumb(Path::new("/home/u"), home), "~");
        assert_eq!(breadcrumb(Path::new("/var/log"), home), "/ › var › log");
    }

    #[test]
    fn truncate_left_keeps_the_end() {
        assert_eq!(truncate_left("abcdef", 10), "abcdef");
        assert_eq!(truncate_left("abcdef", 4), "…def");
        assert_eq!(truncate_left("abcdef", 0), "");
    }

    #[test]
    fn relative_times() {
        let now = 1_000_000_000;
        assert_eq!(ago(now - 10, now), "just now");
        assert_eq!(ago(now - 300, now), "5 min ago");
        assert_eq!(ago(now - 3_600, now), "1 hour ago");
        assert_eq!(ago(now - 3 * 86_400, now), "3 days ago");
        assert_eq!(ago(now - 21 * 86_400, now), "3 weeks ago");
        assert_eq!(ago(now - 400 * 86_400, now), "1 year ago");
    }
}
