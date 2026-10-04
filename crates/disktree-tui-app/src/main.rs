//! CLI argument parsing.

use std::path::PathBuf;

use clap::{CommandFactory, Parser};

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

fn main() {
    let args = CliArgs::parse();

    if args.scan_only {
        let root = args.directory.clone().unwrap_or_else(|| {
            std::env::var("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
        });
        println!(
            "scan-only: would scan {} (mode={}, apparent={}, hidden={}, depth={:?})",
            root.display(),
            args.mode,
            args.apparent,
            !args.no_hidden,
            args.depth
        );
        return;
    }

    // In non-scan-only mode, show help (full TUI comes in M4+).
    CliArgs::command().print_help().ok();
    println!();
}
