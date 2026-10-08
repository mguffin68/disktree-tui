//! Application state: what is on screen and the scan feeding it.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Instant;

use disktree_tui_core::layout::{DrawDepth, Weight};
use disktree_tui_core::scan::{ScanConfig, ScanProgress, ScanResult, Scanner};
use disktree_tui_core::volume::{self, DiskSpace};
use tokio::sync::watch;

use crate::VisMode;

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
        match self.mode {
            VisMode::Files => Weight::Files,
            VisMode::Size | VisMode::Age => Weight::Size,
        }
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
    /// A scan finished; the treemap is shown.
    Ready(Box<ScanResult>),
    /// The scan could not run.
    Error(String),
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
    scan: Option<RunningScan>,
}

struct RunningScan {
    handle: JoinHandle<io::Result<ScanResult>>,
    cancel: Arc<AtomicBool>,
}

impl App {
    /// Start scanning `root` on a background thread.
    pub fn start(root: PathBuf, settings: ViewSettings) -> Self {
        let config = ScanConfig {
            apparent_size: settings.apparent,
            skip_hidden: !settings.show_hidden,
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
        Self {
            disk: volume::disk_space(&root).ok(),
            root,
            settings,
            state: State::Scanning {
                progress: rx,
                started: Instant::now(),
            },
            scan: Some(RunningScan { handle, cancel }),
        }
    }

    /// An app showing a finished scan (no background thread).
    #[cfg(test)]
    pub fn ready(result: ScanResult, settings: ViewSettings, disk: Option<DiskSpace>) -> Self {
        Self {
            root: result.root.path.clone(),
            settings,
            state: State::Ready(Box::new(result)),
            disk,
            scan: None,
        }
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
    }

    /// Ask a running scan to stop; it finishes on its own shortly after.
    pub fn cancel_scan(&self) {
        if let Some(scan) = &self.scan {
            scan.cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn settings() -> ViewSettings {
        ViewSettings {
            mode: VisMode::Size,
            apparent: true,
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

    #[test]
    fn background_scan_reaches_ready() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("f"), b"hello").unwrap();
        let mut app = App::start(tmp.path().to_path_buf(), settings());
        wait_for_scan(&mut app);
        let State::Ready(result) = &app.state else {
            panic!("expected Ready");
        };
        assert_eq!(result.root.size, 5);
        assert!(app.disk.is_some());
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
    fn files_mode_weights_by_file_count() {
        let mut s = settings();
        assert_eq!(s.weight(), Weight::Size);
        s.mode = VisMode::Files;
        assert_eq!(s.weight(), Weight::Files);
    }
}
