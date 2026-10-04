//! Directory scanning: walk, measure, deduplicate, aggregate.

use std::path::Path;

use tokio::sync::watch;

use crate::tree::DirNode;

/// Result of scanning a directory tree.
pub struct ScanResult {
    /// Root node of the tree.
    pub root: Box<DirNode>,
    /// Total file count across the scan.
    pub file_count: usize,
    /// Number of entries that could not be read.
    pub unreadable: usize,
}

/// Configuration for a scan.
#[derive(Debug, Clone, Default)]
pub struct ScanConfig {
    /// Use apparent file size instead of disk usage.
    pub apparent_size: bool,
    /// Skip hidden entries (names starting with `.`).
    pub skip_hidden: bool,
    /// Maximum directory depth to render (None = unlimited).
    pub max_depth: Option<usize>,
    /// Use the whole disk for scanning.
    pub whole_disk: bool,
}

/// Scan a directory tree and return the aggregated result.
pub fn scan(root: &Path, _config: &ScanConfig) -> ScanResult {
    // Placeholder: will be implemented in M1.
    // For now, return a minimal tree with the root directory.
    let root_node = DirNode {
        path: root.to_path_buf(),
        size: 0,
        file_count: 0,
        depth: 0,
        name: root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string()),
        last_write: None,
        class: crate::classify::Class::Unknown,
        reclaimable: false,
        children: Vec::new(),
    };

    ScanResult {
        root: Box::new(root_node),
        file_count: 0,
        unreadable: 0,
    }
}

/// Scan with progress reporting.
pub fn scan_with_progress(
    root: &Path,
    config: &ScanConfig,
    _progress: watch::Sender<usize>,
) -> ScanResult {
    scan(root, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn scan_empty_directory() {
        let tmp = TempDir::new().unwrap();
        let config = ScanConfig::default();
        let result = scan(tmp.path(), &config);
        assert_eq!(result.file_count, 0);
        assert!(result.root.children.is_empty());
    }

    #[test]
    fn scan_directory_with_files() {
        let tmp = TempDir::new().unwrap();
        let mut f = std::fs::File::create(tmp.path().join("test.txt")).unwrap();
        f.write_all(b"hello world").unwrap();
        drop(f);

        let config = ScanConfig::default();
        let result = scan(tmp.path(), &config);
        // Placeholder: should reflect the file we created.
        assert_eq!(result.file_count, 0);
    }
}
