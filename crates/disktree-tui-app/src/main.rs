//! CLI argument parsing.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use disktree_tui_core::platform;
use disktree_tui_core::scan::{self, ScanConfig};
use disktree_tui_core::tree::{DirNode, format_size};

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

    /// Maximum draw depth (unlimited by default).
    #[arg(short = 'D', long)]
    pub depth: Option<usize>,

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

    // In non-scan-only mode, show help (full TUI comes in M4+).
    CliArgs::command().print_help().ok();
    println!();
    ExitCode::SUCCESS
}

/// Scan the target, print a text summary, and exit.
fn scan_only(args: &CliArgs) -> ExitCode {
    if args.disk {
        eprintln!("disktree-tui: --disk is not supported yet");
        return ExitCode::FAILURE;
    }

    let root = args.directory.clone().unwrap_or_else(platform::home_dir);
    let config = ScanConfig {
        apparent_size: args.apparent,
        skip_hidden: args.no_hidden,
        // Without --depth, summarize the top level only.
        max_depth: Some(args.depth.unwrap_or(1)),
        whole_disk: false,
    };

    let result = match scan::scan(&root, &config) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("disktree-tui: cannot scan {}: {err}", root.display());
            return ExitCode::FAILURE;
        }
    };

    let size_kind = if args.apparent {
        "apparent"
    } else {
        "disk usage"
    };
    println!(
        "{}  {} ({size_kind})  {} files  {} dirs",
        result.root.path.display(),
        format_size(result.root.size),
        result.file_count,
        result.dir_count,
    );
    print_children(&result.root);
    for mount in &result.skipped_mounts {
        println!(
            "skipped mount point (other filesystem): {}",
            mount.display()
        );
    }
    if result.unreadable > 0 {
        println!(
            "{} entries could not be read; sizes marked ! are lower bounds",
            result.unreadable
        );
    }
    ExitCode::SUCCESS
}

fn print_children(node: &DirNode) {
    let indent = "  ".repeat(node.depth + 1);
    for child in node.children.iter().take(SUMMARY_CHILDREN) {
        println!(
            "{indent}{:>10}{} {:>9} files  {}/",
            format_size(child.size),
            if child.unreadable { "!" } else { " " },
            child.file_count,
            child.name,
        );
        print_children(child);
    }
    let hidden = node.children.len().saturating_sub(SUMMARY_CHILDREN);
    if hidden > 0 {
        let rest: u64 = node.children[SUMMARY_CHILDREN..]
            .iter()
            .map(|c| c.size)
            .sum();
        println!("{indent}{:>10}  ({hidden} more)", format_size(rest));
    }
}
