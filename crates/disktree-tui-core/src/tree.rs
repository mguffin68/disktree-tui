//! Directory tree data structure.

use std::cmp::Reverse;
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
    /// Newest modification time of this directory or anything under it
    /// (Unix timestamp), if available.
    pub last_write: Option<u64>,
    /// Classified data kind.
    pub class: super::classify::Class,
    /// Whether this whole directory is reclaimable (cache, build output,
    /// package store, temp), independent of its class.
    pub reclaimable: bool,
    /// Marker files this directory directly contains, used for classification.
    pub markers: super::classify::Markers,
    /// The directory (or something under it) could not be read, so `size`
    /// and `file_count` are lower bounds.
    pub unreadable: bool,
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
            markers: super::classify::Markers::NONE,
            unreadable: false,
            children: Vec::new(),
        }
    }

    /// Sort children by size, descending (ties broken by name).
    pub fn sort_children(&mut self) {
        self.children
            .sort_by(|a, b| (Reverse(a.size), &a.name).cmp(&(Reverse(b.size), &b.name)));
    }

    /// Sort children at every level of the subtree.
    pub fn sort_recursive(&mut self) {
        self.sort_children();
        for child in &mut self.children {
            child.sort_recursive();
        }
    }

    /// Bytes in reclaimable directories at or below this node, each counted
    /// once at its outermost reclaimable directory.
    pub fn reclaimable_size(&self) -> u64 {
        if self.reclaimable {
            self.size
        } else {
            self.children.iter().map(|c| c.reclaimable_size()).sum()
        }
    }

    /// Levels of directories below this node (0 when it has no children).
    pub fn height(&self) -> usize {
        self.children
            .iter()
            .map(|c| c.height() + 1)
            .max()
            .unwrap_or(0)
    }

    /// Fold a finished child's totals into this node and attach it.
    pub fn absorb_child(&mut self, child: DirNode) {
        self.absorb_totals(&child);
        self.children.push(Box::new(child));
    }

    /// Fold another node's totals into this node without attaching it.
    pub fn absorb_totals(&mut self, other: &DirNode) {
        self.size += other.size;
        self.file_count += other.file_count;
        self.last_write = self.last_write.max(other.last_write);
        self.unreadable |= other.unreadable;
    }
}

/// Format a byte count with binary units, e.g. `1.5 GiB`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(1536), "1.5 KiB");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0 GiB");
    }

    #[test]
    fn sort_children_largest_first() {
        let mut root = DirNode::new("/r".into(), "r".into(), 0);
        for (name, size) in [("a", 10), ("b", 30), ("c", 20)] {
            let mut child = DirNode::new(format!("/r/{name}").into(), name.into(), 1);
            child.size = size;
            root.absorb_child(child);
        }
        root.sort_children();
        let names: Vec<_> = root.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["b", "c", "a"]);
        assert_eq!(root.size, 60);
    }

    #[test]
    fn height_counts_levels_below() {
        let mut root = DirNode::new("/r".into(), "r".into(), 0);
        assert_eq!(root.height(), 0);
        let mut a = DirNode::new("/r/a".into(), "a".into(), 1);
        a.absorb_child(DirNode::new("/r/a/b".into(), "b".into(), 2));
        root.absorb_child(a);
        root.absorb_child(DirNode::new("/r/c".into(), "c".into(), 1));
        assert_eq!(root.height(), 2);
    }

    #[test]
    fn reclaimable_size_counts_outermost_only() {
        let mut root = DirNode::new("/r".into(), "r".into(), 0);
        let mut cache = DirNode::new("/r/cache".into(), "cache".into(), 1);
        cache.reclaimable = true;
        let mut inner = DirNode::new("/r/cache/x".into(), "x".into(), 2);
        inner.reclaimable = true;
        inner.size = 40;
        cache.absorb_child(inner);
        cache.size += 10;
        let mut code = DirNode::new("/r/code".into(), "code".into(), 1);
        code.size = 500;
        root.absorb_child(cache);
        root.absorb_child(code);
        assert_eq!(root.reclaimable_size(), 50);
    }
}
