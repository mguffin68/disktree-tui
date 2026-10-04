//! Directory tree data structure.

use std::path::PathBuf;

/// A node in the directory tree.
#[derive(Debug, Clone)]
pub struct DirNode {
    /// Full filesystem path.
    pub path: PathBuf,
    /// Size in bytes (disk usage or apparent, depending on config).
    pub size: u64,
    /// Number of files (including descendants).
    pub file_count: usize,
    /// Depth below the scan root (0 = root itself).
    pub depth: usize,
    /// Basename for display.
    pub name: String,
    /// Last write time (Unix timestamp), if available.
    pub last_write: Option<u64>,
    /// Classified data kind.
    pub class: super::classify::Class,
    /// Whether this directory contains reclaimable space.
    pub reclaimable: bool,
    /// Child directories (non-leaf nodes only).
    pub children: Vec<Box<DirNode>>,
}

impl DirNode {
    /// Create a new directory node.
    pub fn new(path: PathBuf, name: String, depth: usize) -> Self {
        Self {
            path,
            size: 0,
            file_count: 0,
            depth,
            name,
            last_write: None,
            class: super::classify::Class::Unknown,
            reclaimable: false,
            children: Vec::new(),
        }
    }

    /// Sort children by size, descending.
    pub fn sort_children(&mut self) {
        self.children.sort_unstable_by_key(|a| a.size);
        self.children.reverse();
    }
}
