//! Name filtering: prune a tree to the directories whose names match.

use crate::tree::DirNode;

/// A case-insensitive substring filter on directory names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    query: String,
}

impl Filter {
    /// A filter for `query`; `None` when it is blank.
    pub fn new(query: &str) -> Option<Self> {
        let query = query.trim().to_lowercase();
        (!query.is_empty()).then_some(Self { query })
    }

    /// The (lowercased) text being matched.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Whether `node` or anything below it matches.
    pub fn reaches(&self, node: &DirNode) -> bool {
        self.matches(node) || node.children.iter().any(|c| self.reaches(c))
    }

    /// Whether `node`'s name matches.
    pub fn matches(&self, node: &DirNode) -> bool {
        node.name.to_lowercase().contains(&self.query)
    }

    /// Matching directories strictly below `root`.
    pub fn count(&self, root: &DirNode) -> usize {
        root.children
            .iter()
            .map(|c| usize::from(self.matches(c)) + self.count(c))
            .sum()
    }

    /// A copy of `root` holding only matching directories (each with its
    /// whole subtree) and the directories leading to them. A non-matching
    /// ancestor's totals become the sum of what it keeps, so tile areas show
    /// the matches alone. The root itself is always returned.
    pub fn apply(&self, root: &DirNode) -> DirNode {
        let mut kept = shell(root);
        for child in &root.children {
            if let Some(c) = self.prune(child) {
                kept.absorb_child(c);
            }
        }
        kept
    }

    fn prune(&self, node: &DirNode) -> Option<DirNode> {
        if self.matches(node) {
            return Some(node.clone());
        }
        let mut kept = shell(node);
        for child in &node.children {
            if let Some(c) = self.prune(child) {
                kept.absorb_child(c);
            }
        }
        (!kept.children.is_empty()).then_some(kept)
    }
}

/// `node` without children or totals, keeping identity and classification.
fn shell(node: &DirNode) -> DirNode {
    let mut s = DirNode::new(node.path.clone(), node.name.clone(), node.depth);
    s.class = node.class;
    s.reclaimable = node.reclaimable;
    s.markers = node.markers;
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(path: &str, size: u64, children: Vec<DirNode>) -> DirNode {
        let name = path.rsplit('/').next().unwrap();
        let mut n = DirNode::new(path.into(), name.into(), 0);
        for c in children {
            n.absorb_child(c);
        }
        n.size += size;
        n.apparent_size += size;
        n.file_count += 1;
        n
    }

    fn tree() -> DirNode {
        dir(
            "/r",
            5,
            vec![
                dir(
                    "/r/proj",
                    10,
                    vec![
                        dir("/r/proj/node_modules", 300, vec![]),
                        dir("/r/proj/src", 20, vec![]),
                    ],
                ),
                dir("/r/web", 0, vec![dir("/r/web/node_modules", 200, vec![])]),
                dir("/r/docs", 1000, vec![]),
            ],
        )
    }

    #[test]
    fn reaches_matches_below() {
        let f = Filter::new("src").unwrap();
        let t = tree();
        assert!(f.reaches(&t.children[0]));
        assert!(!f.reaches(&t.children[1]));
    }

    #[test]
    fn blank_query_is_no_filter() {
        assert!(Filter::new("  ").is_none());
    }

    #[test]
    fn counts_matches_case_insensitively() {
        let f = Filter::new("NODE").unwrap();
        assert_eq!(f.count(&tree()), 2);
    }

    #[test]
    fn apply_keeps_matches_and_their_ancestors_only() {
        let f = Filter::new("node_mod").unwrap();
        let t = f.apply(&tree());
        let names: Vec<_> = t.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["proj", "web"]);
        // Ancestors are re-totaled from what they keep.
        assert_eq!(t.children[0].size, 300);
        assert_eq!(t.children[0].children.len(), 1);
        assert_eq!(t.size, 500);
        assert_eq!(t.path, std::path::Path::new("/r"));
    }

    #[test]
    fn matching_directory_keeps_its_whole_subtree() {
        let f = Filter::new("proj").unwrap();
        let t = f.apply(&tree());
        assert_eq!(t.children.len(), 1);
        assert_eq!(t.children[0].children.len(), 2);
        assert_eq!(t.children[0].size, 330);
    }

    #[test]
    fn no_matches_leaves_an_empty_root() {
        let t = Filter::new("zzz").unwrap().apply(&tree());
        assert!(t.children.is_empty());
        assert_eq!(t.size, 0);
    }
}
