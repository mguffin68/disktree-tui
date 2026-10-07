//! Directory scanning: walk, measure, deduplicate, aggregate.
//!
//! The scan is synchronous; callers that need it off the UI thread run it via
//! `tokio::task::spawn_blocking` and cancel it through [`Scanner::cancel_flag`].

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::watch;

use crate::platform;
use crate::tree::DirNode;

/// Send a progress update every this many entries.
const PROGRESS_INTERVAL: usize = 1024;

/// Result of scanning a directory tree.
pub struct ScanResult {
    /// Root node of the tree.
    pub root: Box<DirNode>,
    /// Total file count across the scan (hardlinks counted once).
    pub file_count: usize,
    /// Number of directories below the root, including any beyond `max_depth`.
    pub dir_count: usize,
    /// Number of entries that could not be read.
    pub unreadable: usize,
    /// The scan was cancelled; totals cover only what was walked.
    pub cancelled: bool,
    /// Mount points under the root that were not entered (other filesystems).
    pub skipped_mounts: Vec<PathBuf>,
}

/// Configuration for a scan.
#[derive(Debug, Clone, Default)]
pub struct ScanConfig {
    /// Use apparent file size instead of disk usage.
    pub apparent_size: bool,
    /// Skip hidden entries (names starting with `.`). The scan root itself is
    /// never skipped.
    pub skip_hidden: bool,
    /// Deepest level that gets its own node (None = unlimited). Anything deeper
    /// is still measured and counted toward its nearest kept ancestor.
    pub max_depth: Option<usize>,
    /// Use the whole disk for scanning.
    pub whole_disk: bool,
}

/// Running totals published while a scan is in progress.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanProgress {
    /// Files counted so far.
    pub files: usize,
    /// Directories entered so far.
    pub dirs: usize,
    /// Bytes counted so far.
    pub bytes: u64,
}

/// A configured, cancellable directory scanner.
pub struct Scanner {
    config: ScanConfig,
    cancel: Arc<AtomicBool>,
}

impl Scanner {
    /// Create a scanner with the given configuration.
    pub fn new(config: ScanConfig) -> Self {
        Self {
            config,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Flag that stops a running scan when set to `true`.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Scan `root` and return the aggregated tree.
    pub fn scan(&self, root: &Path) -> io::Result<ScanResult> {
        self.run(root, None)
    }

    /// Scan `root`, publishing running totals to `progress`.
    pub fn scan_with_progress(
        &self,
        root: &Path,
        progress: &watch::Sender<ScanProgress>,
    ) -> io::Result<ScanResult> {
        self.run(root, Some(progress))
    }

    fn run(
        &self,
        root: &Path,
        progress: Option<&watch::Sender<ScanProgress>>,
    ) -> io::Result<ScanResult> {
        self.walk(root, progress, &platform::mount_points())
    }

    fn walk(
        &self,
        root: &Path,
        progress: Option<&watch::Sender<ScanProgress>>,
        mount_points: &[PathBuf],
    ) -> io::Result<ScanResult> {
        let root_meta = fs::metadata(root)?;
        if !root_meta.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("{} is not a directory", root.display()),
            ));
        }

        let apparent = self.config.apparent_size;
        let max_depth = self.config.max_depth.unwrap_or(usize::MAX);
        // Stay on the root's filesystem. Known mounts are skipped by path before
        // anything stats them, because even `lstat` on a dead network mount can
        // hang; the device check below catches the rest.
        let mounts: HashSet<PathBuf> = nested_mounts(root, mount_points).into_iter().collect();
        let root_dev = platform::device_id(&root_meta);

        let mut skipped_mounts = Vec::new();
        let mut seen_links: HashSet<(u64, u64)> = HashSet::new();
        let mut totals = ScanProgress::default();
        let mut unreadable = 0;
        let mut cancelled = false;
        let mut visited: usize = 0;

        let mut root_node = DirNode::new(root.to_path_buf(), node_name(root), 0);
        root_node.last_write = platform::modified_secs(&root_meta);
        if !apparent {
            // Directories' own blocks count toward disk usage, as with `du`.
            root_node.size = platform::entry_size(&root_meta, false);
            totals.bytes = root_node.size;
        }
        let root_entries = read_sorted(root).unwrap_or_else(|_| {
            unreadable += 1;
            root_node.unreadable = true;
            Vec::new()
        });
        let mut stack = vec![Frame {
            node: Some(root_node),
            entries: root_entries.into_iter(),
        }];
        let mut finished_root = None;

        while let Some(frame) = stack.last_mut() {
            let Some(entry) = frame.entries.next() else {
                finished_root = pop_frame(&mut stack, max_depth);
                continue;
            };

            if self.cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            if let Some(tx) = progress
                && visited.is_multiple_of(PROGRESS_INTERVAL)
            {
                tx.send_replace(totals);
            }
            visited += 1;

            let name = entry.file_name();
            if self.config.skip_hidden && name.to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                unreadable += 1;
                kept_node(&mut stack, max_depth).unreadable = true;
                continue;
            };
            if file_type.is_symlink() {
                // Never followed; only the link's own blocks count, as with `du`.
                if !apparent && let Ok(meta) = entry.metadata() {
                    let size = platform::entry_size(&meta, false);
                    totals.bytes += size;
                    kept_node(&mut stack, max_depth).size += size;
                }
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() && mounts.contains(&path) {
                skipped_mounts.push(path);
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                unreadable += 1;
                kept_node(&mut stack, max_depth).unreadable = true;
                continue;
            };
            let mtime = platform::modified_secs(&meta);

            if file_type.is_dir() {
                if root_dev.is_some() && platform::device_id(&meta) != root_dev {
                    skipped_mounts.push(path);
                    continue;
                }
                let own = if apparent {
                    0
                } else {
                    platform::entry_size(&meta, false)
                };
                totals.dirs += 1;
                totals.bytes += own;
                let (entries, failed) = if let Ok(entries) = read_sorted(&path) {
                    (entries, false)
                } else {
                    unreadable += 1;
                    (Vec::new(), true)
                };
                let depth = stack.len();
                let node = if depth <= max_depth {
                    let mut node = DirNode::new(path, name.to_string_lossy().into_owned(), depth);
                    node.size = own;
                    node.last_write = mtime;
                    node.unreadable = failed;
                    Some(node)
                } else {
                    let kept = kept_node(&mut stack, max_depth);
                    kept.size += own;
                    kept.last_write = kept.last_write.max(mtime);
                    kept.unreadable |= failed;
                    None
                };
                stack.push(Frame {
                    node,
                    entries: entries.into_iter(),
                });
            } else {
                if let Some(id) = platform::hardlink_id(&meta)
                    && !seen_links.insert(id)
                {
                    continue;
                }
                let size = platform::entry_size(&meta, apparent);
                totals.files += 1;
                totals.bytes += size;
                let kept = kept_node(&mut stack, max_depth);
                kept.size += size;
                kept.file_count += 1;
                kept.last_write = kept.last_write.max(mtime);
            }
        }

        // After a cancel, fold whatever is still open into the root.
        while !stack.is_empty() {
            finished_root = pop_frame(&mut stack, max_depth);
        }
        let mut root_node = finished_root.expect("root frame is popped last");
        root_node.sort_recursive();
        skipped_mounts.sort();
        skipped_mounts.dedup();

        if let Some(tx) = progress {
            tx.send_replace(totals);
        }

        Ok(ScanResult {
            root: Box::new(root_node),
            file_count: totals.files,
            dir_count: totals.dirs,
            unreadable,
            cancelled,
            skipped_mounts,
        })
    }
}

/// A directory being walked: its node (if within `max_depth`) and the entries
/// still to visit.
struct Frame {
    node: Option<DirNode>,
    entries: std::vec::IntoIter<fs::DirEntry>,
}

/// Scan a directory tree and return the aggregated result.
pub fn scan(root: &Path, config: &ScanConfig) -> io::Result<ScanResult> {
    Scanner::new(config.clone()).scan(root)
}

/// Scan with progress reporting.
pub fn scan_with_progress(
    root: &Path,
    config: &ScanConfig,
    progress: &watch::Sender<ScanProgress>,
) -> io::Result<ScanResult> {
    Scanner::new(config.clone()).scan_with_progress(root, progress)
}

/// Mount points strictly below `root`, expressed under `root` as given so they
/// compare equal to walked entry paths.
fn nested_mounts(root: &Path, mount_points: &[PathBuf]) -> Vec<PathBuf> {
    let Ok(canonical) = root.canonicalize() else {
        return Vec::new();
    };
    let mut nested: Vec<PathBuf> = mount_points
        .iter()
        .filter_map(|m| m.strip_prefix(&canonical).ok())
        .filter(|rel| !rel.as_os_str().is_empty())
        .map(|rel| root.join(rel))
        .collect();
    // Stacked mounts (e.g. autofs under cifs) share a path.
    nested.sort();
    nested.dedup();
    nested
}

/// Pop the top frame, folding its node into the nearest kept ancestor.
/// Returns the root node once the last frame is popped.
fn pop_frame(stack: &mut Vec<Frame>, max_depth: usize) -> Option<DirNode> {
    let node = stack.pop()?.node?;
    if stack.is_empty() {
        return Some(node);
    }
    kept_node(stack, max_depth).absorb_child(node);
    None
}

/// Deepest node on the stack. Frames at depths `0..=max_depth` carry nodes,
/// so it is the top frame or the one at `max_depth`.
fn kept_node(stack: &mut [Frame], max_depth: usize) -> &mut DirNode {
    let index = (stack.len() - 1).min(max_depth);
    stack[index]
        .node
        .as_mut()
        .expect("frames within max_depth carry a node")
}

/// Directory entries sorted by name, so scans (and which hardlink keeps the
/// bytes) are deterministic.
fn read_sorted(dir: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(dir)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn node_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    fn apparent() -> ScanConfig {
        ScanConfig {
            apparent_size: true,
            ..ScanConfig::default()
        }
    }

    fn write(path: &Path, len: usize) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, vec![b'x'; len]).unwrap();
    }

    fn child<'a>(node: &'a DirNode, name: &str) -> &'a DirNode {
        node.children
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no child {name} under {}", node.name))
    }

    #[test]
    fn scan_empty_directory() {
        let tmp = TempDir::new().unwrap();
        let result = scan(tmp.path(), &ScanConfig::default()).unwrap();
        assert_eq!(result.file_count, 0);
        assert!(result.root.children.is_empty());
        assert!(!result.cancelled);
    }

    #[test]
    fn scan_directory_with_files() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("test.txt"), 11);

        let result = scan(tmp.path(), &apparent()).unwrap();
        assert_eq!(result.file_count, 1);
        assert_eq!(result.root.size, 11);
    }

    #[test]
    fn simple_tree_totals() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("a.bin"), 100);
        write(&tmp.path().join("sub/b.bin"), 200);
        write(&tmp.path().join("sub/deep/c.bin"), 300);
        write(&tmp.path().join("other/d.bin"), 50);

        let result = scan(tmp.path(), &apparent()).unwrap();
        let root = &result.root;
        assert_eq!(root.size, 650);
        assert_eq!(root.file_count, 4);
        assert_eq!(result.file_count, 4);
        assert_eq!(result.dir_count, 3);

        let sub = child(root, "sub");
        assert_eq!(sub.size, 500);
        assert_eq!(sub.file_count, 2);
        assert_eq!(sub.depth, 1);
        assert_eq!(child(sub, "deep").size, 300);
        assert_eq!(child(sub, "deep").depth, 2);
        assert_eq!(child(root, "other").size, 50);

        // Largest child first.
        assert_eq!(root.children[0].name, "sub");
        assert!(root.last_write.is_some());
    }

    #[test]
    fn disk_usage_mode_counts_blocks() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("f.bin"), 10_000);
        let result = scan(tmp.path(), &ScanConfig::default()).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let file_blocks = fs::metadata(tmp.path().join("f.bin")).unwrap().blocks() * 512;
            let dir_blocks = fs::metadata(tmp.path()).unwrap().blocks() * 512;
            assert_eq!(result.root.size, file_blocks + dir_blocks);
        }
        #[cfg(not(unix))]
        assert_eq!(result.root.size, 10_000);
    }

    #[cfg(unix)]
    #[test]
    fn hardlinks_counted_once() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("a/original.bin"), 1000);
        fs::create_dir(tmp.path().join("b")).unwrap();
        fs::hard_link(
            tmp.path().join("a/original.bin"),
            tmp.path().join("b/link.bin"),
        )
        .unwrap();

        let result = scan(tmp.path(), &apparent()).unwrap();
        assert_eq!(result.root.size, 1000);
        assert_eq!(result.file_count, 1);
        // Sorted walk: the first path by name ("a/...") keeps the bytes.
        assert_eq!(child(&result.root, "a").size, 1000);
        assert_eq!(child(&result.root, "b").size, 0);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_not_followed() {
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        write(&outside.path().join("big.bin"), 5000);
        write(&tmp.path().join("real.bin"), 10);
        symlink(outside.path(), tmp.path().join("dir-link")).unwrap();
        symlink(outside.path().join("big.bin"), tmp.path().join("file-link")).unwrap();
        symlink(tmp.path().join("missing"), tmp.path().join("broken")).unwrap();

        let result = scan(tmp.path(), &apparent()).unwrap();
        assert_eq!(result.root.size, 10);
        assert_eq!(result.file_count, 1);
        assert!(result.root.children.is_empty());
        assert_eq!(result.unreadable, 0);
    }

    #[test]
    fn hidden_included_by_default_and_skippable() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("visible.bin"), 10);
        write(&tmp.path().join(".hidden.bin"), 20);
        write(&tmp.path().join(".config/settings"), 40);

        let all = scan(tmp.path(), &apparent()).unwrap();
        assert_eq!(all.root.size, 70);
        assert_eq!(all.file_count, 3);

        let config = ScanConfig {
            skip_hidden: true,
            ..apparent()
        };
        let visible = scan(tmp.path(), &config).unwrap();
        assert_eq!(visible.root.size, 10);
        assert_eq!(visible.file_count, 1);
        assert!(visible.root.children.is_empty());
    }

    #[test]
    fn hidden_root_is_still_scanned() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(".cache");
        write(&root.join("blob"), 30);

        let config = ScanConfig {
            skip_hidden: true,
            ..apparent()
        };
        let result = scan(&root, &config).unwrap();
        assert_eq!(result.root.size, 30);
        assert_eq!(result.root.name, ".cache");
    }

    #[test]
    fn depth_limit_prunes_nodes_but_keeps_totals() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("l1/f.bin"), 1);
        write(&tmp.path().join("l1/l2/f.bin"), 10);
        write(&tmp.path().join("l1/l2/l3/f.bin"), 100);
        write(&tmp.path().join("l1/l2/l3/l4/f.bin"), 1000);

        let config = ScanConfig {
            max_depth: Some(2),
            ..apparent()
        };
        let result = scan(tmp.path(), &config).unwrap();
        assert_eq!(result.root.size, 1111);
        assert_eq!(result.file_count, 4);
        assert_eq!(result.dir_count, 4);

        let l1 = child(&result.root, "l1");
        assert_eq!(l1.size, 1111);
        let l2 = child(l1, "l2");
        assert_eq!(l2.size, 1110);
        assert_eq!(l2.file_count, 3);
        assert!(l2.children.is_empty(), "nodes past depth 2 are folded in");

        let zero = ScanConfig {
            max_depth: Some(0),
            ..apparent()
        };
        let result = scan(tmp.path(), &zero).unwrap();
        assert_eq!(result.root.size, 1111);
        assert!(result.root.children.is_empty());
    }

    #[test]
    fn siblings_after_deep_branch_attach_to_correct_parent() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("a/b/c/d/f"), 1);
        write(&tmp.path().join("z/f"), 2);

        let result = scan(tmp.path(), &apparent()).unwrap();
        let names: Vec<_> = result
            .root
            .children
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["z", "a"]);
        assert_eq!(child(&result.root, "z").size, 2);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_directory_counted_not_fatal() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("ok/f.bin"), 10);
        let locked = tmp.path().join("locked");
        write(&locked.join("secret.bin"), 1000);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        // Running as root ignores permissions; nothing to test then.
        let readable_anyway = fs::read_dir(&locked).is_ok();
        let result = scan(tmp.path(), &apparent());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        if readable_anyway {
            return;
        }

        let result = result.unwrap();
        assert_eq!(result.unreadable, 1);
        assert_eq!(result.root.size, 10);
        assert!(child(&result.root, "locked").unreadable);
        assert!(!child(&result.root, "ok").unreadable);
        assert!(result.root.unreadable, "unreadable propagates upward");
    }

    #[test]
    fn nested_mounts_are_relative_to_given_root() {
        let tmp = TempDir::new().unwrap();
        let canonical = tmp.path().canonicalize().unwrap();
        let mounts = [
            PathBuf::from("/"),
            canonical.clone(),
            canonical.join("net/share"),
            canonical.join("net/share"),
            PathBuf::from("/somewhere/else"),
        ];
        assert_eq!(
            nested_mounts(tmp.path(), &mounts),
            [tmp.path().join("net/share")]
        );
    }

    #[test]
    fn skipped_mount_does_not_hide_later_siblings() {
        // Regression: skipping a mount once aborted the rest of its parent.
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("a/f"), 1);
        fs::create_dir(tmp.path().join("m")).unwrap();
        write(&tmp.path().join("m/f"), 1000);
        write(&tmp.path().join("z/f"), 2);

        let mounts = [tmp.path().canonicalize().unwrap().join("m")];
        let result = Scanner::new(apparent())
            .walk(tmp.path(), None, &mounts)
            .unwrap();
        let names: Vec<_> = result
            .root
            .children
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["z", "a"]);
        assert_eq!(result.root.size, 3);
        assert_eq!(result.skipped_mounts, [tmp.path().join("m")]);
    }

    #[test]
    fn missing_root_is_an_error() {
        let tmp = TempDir::new().unwrap();
        assert!(scan(&tmp.path().join("nope"), &ScanConfig::default()).is_err());
    }

    #[test]
    fn file_root_is_an_error() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("f"), 1);
        let err = scan(&tmp.path().join("f"), &ScanConfig::default())
            .err()
            .unwrap();
        assert_eq!(err.kind(), io::ErrorKind::NotADirectory);
    }

    #[test]
    fn cancelled_scan_stops_early() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("f"), 1);
        let scanner = Scanner::new(ScanConfig::default());
        scanner.cancel_flag().store(true, Ordering::Relaxed);
        let result = scanner.scan(tmp.path()).unwrap();
        assert!(result.cancelled);
        assert_eq!(result.file_count, 0);
    }

    #[test]
    fn progress_reports_final_totals() {
        let tmp = TempDir::new().unwrap();
        write(&tmp.path().join("a"), 5);
        write(&tmp.path().join("d/b"), 7);
        let (tx, rx) = watch::channel(ScanProgress::default());
        let result = scan_with_progress(tmp.path(), &apparent(), &tx).unwrap();
        let last = *rx.borrow();
        assert_eq!(last.files, 2);
        assert_eq!(last.dirs, 1);
        assert_eq!(last.bytes, 12);
        assert_eq!(result.file_count, 2);
    }

    #[test]
    fn large_directory_within_time_budget() {
        let tmp = TempDir::new().unwrap();
        for d in 0..100 {
            let dir = tmp.path().join(format!("d{d}"));
            fs::create_dir(&dir).unwrap();
            for f in 0..100 {
                fs::write(dir.join(format!("f{f}")), b"x").unwrap();
            }
        }

        let start = Instant::now();
        let result = scan(tmp.path(), &apparent()).unwrap();
        assert!(start.elapsed() < Duration::from_secs(10));
        assert_eq!(result.file_count, 10_000);
        assert_eq!(result.root.size, 10_000);
        assert_eq!(result.root.children.len(), 100);
    }
}
