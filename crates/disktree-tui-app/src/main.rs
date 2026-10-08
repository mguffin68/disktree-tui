//! disktree-tui: CLI entry point and terminal event loop.

mod app;
mod tree_view;
mod treemap_view;
mod ui;

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use std::time::Duration;

use clap::Parser;
use disktree_tui_core::layout::DrawDepth;
use disktree_tui_core::platform;
use disktree_tui_core::scan::{self, ScanConfig, ScanResult};
use disktree_tui_core::tree::{DirNode, format_size};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::app::{App, ViewSettings};
use crate::ui::{ColorDepth, Theme};

/// disktree-tui: Terminal disk usage explorer and cleanup tool.
#[derive(Debug, Parser)]
#[command(name = "disktree-tui", version, about, long_about = None)]
pub struct CliArgs {
    /// Directory to scan (default: $HOME).
    #[arg(value_name = "DIRECTORY")]
    pub directory: Option<PathBuf>,

    /// Scan the whole disk containing the target.
    #[arg(short, long)]
    pub disk: bool,

    /// Use apparent file size instead of disk usage.
    #[arg(short, long)]
    pub apparent: bool,

    /// Skip hidden files and directories.
    #[arg(short = 'H', long)]
    pub no_hidden: bool,

    /// Treemap draw depth: levels below the view, or "all" [default: 2;
    /// change with [ and ]]. With --scan-only: levels printed [default: 1].
    #[arg(short = 'D', long, value_name = "LEVELS")]
    pub depth: Option<DrawDepth>,

    /// What to visualize: size, files, or age.
    #[arg(short, long, default_value = "size")]
    pub mode: VisMode,

    /// Scan and print summary, then exit (no TUI).
    #[arg(long)]
    pub scan_only: bool,
}

/// Visualization mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisMode {
    /// Disk usage / apparent size.
    #[default]
    Size,
    /// File count.
    Files,
    /// Last write age.
    Age,
}

impl std::str::FromStr for VisMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "size" => Ok(Self::Size),
            "files" => Ok(Self::Files),
            "age" => Ok(Self::Age),
            other => Err(format!(
                "unknown visualization mode: {other} (expected size, files, or age)"
            )),
        }
    }
}

impl std::fmt::Display for VisMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Size => write!(f, "size"),
            Self::Files => write!(f, "files"),
            Self::Age => write!(f, "age"),
        }
    }
}

/// Children listed per directory in the `--scan-only` summary.
const SUMMARY_CHILDREN: usize = 20;

fn main() -> ExitCode {
    let args = CliArgs::parse();

    if args.scan_only {
        return scan_only(&args);
    }

    if args.disk {
        eprintln!("disktree-tui: --disk is not supported yet");
        return ExitCode::FAILURE;
    }
    match run_tui(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("disktree-tui: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Show the treemap until the user quits. Display only for now: `q`, `Esc`
/// or `Ctrl+C` quit, and a resize redraws.
fn run_tui(args: &CliArgs) -> io::Result<()> {
    let root = args.directory.clone().unwrap_or_else(platform::home_dir);
    let settings = ViewSettings {
        mode: args.mode,
        apparent: args.apparent,
        show_hidden: !args.no_hidden,
        depth: args.depth.unwrap_or_default(),
    };
    let theme = Theme::new(ColorDepth::detect());
    let mut app = App::start(root, settings);

    let mut terminal = ratatui::try_init()?;
    let result = event_loop(&mut terminal, &mut app, theme);
    ratatui::restore();
    app.cancel_scan();
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    theme: Theme,
) -> io::Result<()> {
    loop {
        app.poll_scan();
        terminal.draw(|frame| ui::render(frame, app, theme))?;
        // Poll briefly so scan progress keeps updating; resizes simply fall
        // through to the next draw.
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            let ctrl_c =
                key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
            if ctrl_c || matches!(key.code, KeyCode::Char('q' | 'Q') | KeyCode::Esc) {
                return Ok(());
            }
        }
    }
}

/// Scan the target, print a text summary, and exit.
fn scan_only(args: &CliArgs) -> ExitCode {
    if args.disk {
        eprintln!("disktree-tui: --disk is not supported yet");
        return ExitCode::FAILURE;
    }

    let root = args.directory.clone().unwrap_or_else(platform::home_dir);
    let config = scan_only_config(args);

    let result = match scan::scan(&root, &config) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("disktree-tui: cannot scan {}: {err}", root.display());
            return ExitCode::FAILURE;
        }
    };

    // Without --depth, summarize the top level only.
    let depth = args
        .depth
        .map_or(1, |d| d.max_depth().unwrap_or(usize::MAX));
    match print_summary(&mut io::stdout().lock(), &result, args.apparent, depth) {
        Ok(()) => ExitCode::SUCCESS,
        // Output piped into `head` and closed early: not an error.
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("disktree-tui: {err}");
            ExitCode::FAILURE
        }
    }
}

fn scan_only_config(args: &CliArgs) -> ScanConfig {
    ScanConfig {
        apparent_size: args.apparent,
        skip_hidden: args.no_hidden,
        // Keep the full tree so the reclaimable total sees nested caches and
        // build output at any depth; --depth only limits what is printed.
        max_depth: None,
        whole_disk: false,
    }
}

fn print_summary(
    out: &mut impl Write,
    result: &ScanResult,
    apparent: bool,
    depth: usize,
) -> io::Result<()> {
    let size_kind = if apparent { "apparent" } else { "disk usage" };
    writeln!(
        out,
        "{}  {} ({size_kind})  {} files  {} dirs  {}",
        result.root.path.display(),
        format_size(result.root.size),
        result.file_count,
        result.dir_count,
        class_tag(&result.root),
    )?;
    writeln!(
        out,
        "reclaimable: {}",
        format_size(result.root.reclaimable_size())
    )?;
    print_children(out, &result.root, depth)?;
    for mount in &result.skipped_mounts {
        writeln!(
            out,
            "skipped mount point (other filesystem): {}",
            mount.display()
        )?;
    }
    if result.unreadable > 0 {
        writeln!(
            out,
            "{} entries could not be read; sizes marked ! are lower bounds",
            result.unreadable
        )?;
    }
    Ok(())
}

/// `[Class]`, or `[Class, reclaimable]`.
fn class_tag(node: &DirNode) -> String {
    if node.reclaimable {
        format!("[{}, reclaimable]", node.class.label())
    } else {
        format!("[{}]", node.class.label())
    }
}

fn print_children(out: &mut impl Write, node: &DirNode, max_depth: usize) -> io::Result<()> {
    if node.depth >= max_depth {
        return Ok(());
    }
    let indent = "  ".repeat(node.depth + 1);
    for child in node.children.iter().take(SUMMARY_CHILDREN) {
        writeln!(
            out,
            "{indent}{:>10}{} {:>9} files  {}/  {}",
            format_size(child.size),
            if child.unreadable { "!" } else { " " },
            child.file_count,
            child.name,
            class_tag(child),
        )?;
        print_children(out, child, max_depth)?;
    }
    let hidden = node.children.len().saturating_sub(SUMMARY_CHILDREN);
    if hidden > 0 {
        let rest: u64 = node.children[SUMMARY_CHILDREN..]
            .iter()
            .map(|c| c.size)
            .sum();
        writeln!(out, "{indent}{:>10}  ({hidden} more)", format_size(rest))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_only_scans_full_depth_regardless_of_depth_flag() {
        let args = CliArgs::parse_from(["disktree-tui", "--scan-only", "-D", "1", "--apparent"]);
        let config = scan_only_config(&args);
        assert_eq!(config.max_depth, None);
        assert!(config.apparent_size);
    }

    #[test]
    fn depth_flag_accepts_levels_and_all() {
        let parse = |v: &str| CliArgs::try_parse_from(["disktree-tui", "-D", v]).map(|a| a.depth);
        assert_eq!(parse("3").unwrap(), Some(DrawDepth::Levels(3)));
        assert_eq!(parse("all").unwrap(), Some(DrawDepth::Unlimited));
        assert!(parse("0").is_err());
        assert_eq!(CliArgs::parse_from(["disktree-tui"]).depth, None);
    }
}
