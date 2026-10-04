//! Removal logic with safety guards.

use std::path::{Path, PathBuf};

/// The type of removal action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalType {
    /// Move to trash (recoverable).
    Trash,
    /// Delete permanently (irreversible).
    Permanent,
}

/// A path marked for removal.
#[derive(Debug, Clone)]
pub struct MarkedPath {
    /// Full filesystem path.
    pub path: PathBuf,
    /// Size in bytes.
    pub size: u64,
    /// Whether it is reclaimable.
    pub reclaimable: bool,
}

/// Safety guard check before removal.
#[derive(Debug, Clone)]
pub struct RemovalPolicy {
    /// The scan root (only paths under this can be removed).
    pub scan_root: PathBuf,
    /// Filesystem root (never remove).
    pub fs_root: PathBuf,
    /// Home directory (never remove).
    pub home_dir: PathBuf,
    /// System trees that should not be removed.
    pub system_trees: Vec<PathBuf>,
}

/// Check if a path is safe to remove according to the policy.
pub fn can_remove(path: &Path, policy: &RemovalPolicy) -> Result<(), RemovalError> {
    // Must be under scan root.
    if !path.starts_with(&policy.scan_root) {
        return Err(RemovalError::OutsideScanRoot);
    }

    // Never remove filesystem root.
    if path == policy.fs_root {
        return Err(RemovalError::FilesystemRoot);
    }

    // Never remove scanned root itself.
    if path == policy.scan_root {
        return Err(RemovalError::ScanRoot);
    }

    // Never remove home directory.
    if path == policy.home_dir {
        return Err(RemovalError::HomeDirectory);
    }

    // Never remove system trees.
    for tree in &policy.system_trees {
        if path == tree || path.starts_with(tree) {
            return Err(RemovalError::SystemTree);
        }
    }

    Ok(())
}

/// Perform the removal action.
pub fn remove_paths(paths: &[MarkedPath], r#type: RemovalType) -> RemovalReport {
    // Placeholder: will be implemented in M6.
    let mut succeeded = Vec::new();
    let failed = Vec::new();

    for marked in paths {
        match r#type {
            RemovalType::Trash => {
                // TODO: implement trash via trash crate / trash-put / gio
                succeeded.push(marked.path.clone());
            }
            RemovalType::Permanent => {
                // TODO: implement rm -rf semantics
                succeeded.push(marked.path.clone());
            }
        }
    }

    RemovalReport {
        succeeded,
        failed,
        freed_bytes: paths.iter().map(|p| p.size).sum(),
    }
}

/// Report after a removal operation.
pub struct RemovalReport {
    /// Paths that were successfully removed.
    pub succeeded: Vec<PathBuf>,
    /// Paths that failed to remove.
    pub failed: Vec<(PathBuf, String)>,
    /// Total bytes freed.
    pub freed_bytes: u64,
}

/// Errors that can occur during removal policy checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemovalError {
    /// Path is outside the scan root.
    OutsideScanRoot,
    /// Attempting to remove the filesystem root.
    FilesystemRoot,
    /// Attempting to remove the scanned root.
    ScanRoot,
    /// Attempting to remove the home directory.
    HomeDirectory,
    /// Attempting to remove a system tree.
    SystemTree,
}

impl std::fmt::Display for RemovalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideScanRoot => write!(f, "path is outside the scan root"),
            Self::FilesystemRoot => write!(f, "cannot remove the filesystem root"),
            Self::ScanRoot => write!(f, "cannot remove the scan root"),
            Self::HomeDirectory => write!(f, "cannot remove the home directory"),
            Self::SystemTree => write!(f, "cannot remove a system tree"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_policy(root: &str) -> RemovalPolicy {
        RemovalPolicy {
            scan_root: PathBuf::from(root),
            fs_root: PathBuf::from("/"),
            home_dir: PathBuf::from("/home/user"),
            system_trees: vec![
                PathBuf::from("/usr"),
                PathBuf::from("/etc"),
                PathBuf::from("/nix/store"),
                PathBuf::from("/gnu/store"),
            ],
        }
    }

    #[test]
    fn refuse_filesystem_root() {
        let policy = RemovalPolicy {
            scan_root: PathBuf::from("/"),
            fs_root: PathBuf::from("/"),
            home_dir: PathBuf::from("/home/user"),
            system_trees: vec![],
        };
        assert_eq!(
            can_remove(Path::new("/"), &policy),
            Err(RemovalError::FilesystemRoot)
        );
    }

    #[test]
    fn refuse_scan_root() {
        let policy = make_policy("/tmp/test");
        assert_eq!(
            can_remove(Path::new("/tmp/test"), &policy),
            Err(RemovalError::ScanRoot)
        );
    }

    #[test]
    fn refuse_home_directory() {
        let policy = RemovalPolicy {
            scan_root: PathBuf::from("/"),
            fs_root: PathBuf::from("/"),
            home_dir: PathBuf::from("/home/user"),
            system_trees: vec![],
        };
        assert_eq!(
            can_remove(Path::new("/home/user"), &policy),
            Err(RemovalError::HomeDirectory)
        );
    }

    #[test]
    fn refuse_system_trees() {
        let policy = RemovalPolicy {
            scan_root: PathBuf::from("/"),
            fs_root: PathBuf::from("/"),
            home_dir: PathBuf::from("/home/user"),
            system_trees: vec![
                PathBuf::from("/usr"),
                PathBuf::from("/etc"),
                PathBuf::from("/nix/store"),
                PathBuf::from("/gnu/store"),
            ],
        };
        assert_eq!(
            can_remove(Path::new("/usr"), &policy),
            Err(RemovalError::SystemTree)
        );
        assert_eq!(
            can_remove(Path::new("/usr/local/bin"), &policy),
            Err(RemovalError::SystemTree)
        );
    }

    #[test]
    fn allow_paths_under_scan_root() {
        let policy = make_policy("/tmp/test");
        assert!(can_remove(Path::new("/tmp/test/some/dir"), &policy).is_ok());
    }

    #[test]
    fn refuse_outside_scan_root() {
        let policy = make_policy("/tmp/test");
        assert_eq!(
            can_remove(Path::new("/tmp/other"), &policy),
            Err(RemovalError::OutsideScanRoot)
        );
    }
}
