//! Removal logic with safety guards, trash, and permanent delete.

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
    /// Known mount points: a marked path may not be one or contain one.
    pub mount_points: Vec<PathBuf>,
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

    // Never remove a mount point or a directory containing one.
    for mount in &policy.mount_points {
        if path == mount || mount.starts_with(path) {
            return Err(RemovalError::MountPoint);
        }
    }

    Ok(())
}

/// Perform the removal action.
pub fn remove_paths(
    paths: &[MarkedPath],
    r#type: RemovalType,
    policy: &RemovalPolicy,
) -> RemovalReport {
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();

    for marked in paths {
        // Safety check before attempting any removal.
        if let Err(e) = can_remove(&marked.path, policy) {
            failed.push((marked.path.clone(), format!("{e}")));
            continue;
        }

        match r#type {
            RemovalType::Trash => match try_trash(&marked.path) {
                Ok(()) => succeeded.push(marked.path.clone()),
                Err(err) => failed.push((marked.path.clone(), err)),
            },
            RemovalType::Permanent => match try_permanent_delete(&marked.path) {
                Ok(()) => succeeded.push(marked.path.clone()),
                Err(err) => failed.push((marked.path.clone(), err)),
            },
        }
    }

    let freed_bytes: u64 = succeeded
        .iter()
        .filter_map(|s| paths.iter().find(|p| p.path == *s).map(|p| p.size))
        .sum();

    RemovalReport {
        succeeded,
        failed,
        freed_bytes,
    }
}

/// Try to move a path to trash, returning an error message on failure.
fn try_trash(path: &Path) -> Result<(), String> {
    use std::process::Command;

    // Try trash-put (trash-cli) first.
    if let Ok(output) = Command::new("trash-put").arg(path).output()
        && output.status.success()
    {
        return Ok(());
    }

    // Try gio trash.
    if let Ok(output) = Command::new("gio").arg("trash").arg(path).output()
        && output.status.success()
    {
        return Ok(());
    }

    // Built-in XDG trash compliance.
    if let Err(e) = xdg_trash(path) {
        return Err(format!(
            "no trash available: {e} (tried trash-put, gio, XDG)"
        ));
    }
    Ok(())
}

/// Move to XDG Trash directory (~/.local/share/Trash/files/).
fn xdg_trash(path: &Path) -> Result<(), String> {
    use std::fs;

    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;
    let trash_dir = PathBuf::from(home).join(".local/share/Trash/files");
    fs::create_dir_all(&trash_dir).map_err(|e| format!("cannot create trash dir: {e}"))?;

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| uuid_for(path));

    // Avoid name collisions by appending a counter.
    let mut final_name = name.clone();
    let mut counter = 0;
    loop {
        let candidate = trash_dir.join(&final_name);
        if candidate.exists() {
            counter += 1;
            final_name = format!("{name}.{counter}");
        } else {
            break;
        }
    }

    fs::rename(path, trash_dir.join(&final_name))
        .map_err(|e| format!("cannot move to trash: {e}"))?;

    Ok(())
}

/// Generate a UUID-like name for a path that was moved, to avoid name collisions.
fn uuid_for(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "trash_item".to_string());
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{name}.trash.{ts}")
}

/// Remove a path permanently using `std::fs`, never through shell.
fn try_permanent_delete(path: &Path) -> Result<(), String> {
    use std::fs;

    if !path.exists() {
        return Ok(());
    }

    let metadata =
        fs::metadata(path).map_err(|e| format!("cannot stat {}: {e}", path.display()))?;

    if metadata.is_dir() {
        fs::remove_dir_all(path)
            .map_err(|e| format!("cannot delete directory {}: {e}", path.display()))?;
    } else {
        fs::remove_file(path).map_err(|e| format!("cannot delete file {}: {e}", path.display()))?;
    }

    Ok(())
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
    /// Attempting to remove a mount point, or a directory holding one.
    MountPoint,
}

impl std::fmt::Display for RemovalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideScanRoot => write!(f, "path is outside the scan root"),
            Self::FilesystemRoot => write!(f, "cannot remove the filesystem root"),
            Self::ScanRoot => write!(f, "cannot remove the scan root"),
            Self::HomeDirectory => write!(f, "cannot remove the home directory"),
            Self::SystemTree => write!(f, "cannot remove a system tree"),
            Self::MountPoint => write!(f, "cannot remove a mount point"),
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
            mount_points: vec![],
        }
    }

    #[test]
    fn refuse_filesystem_root() {
        let policy = RemovalPolicy {
            scan_root: PathBuf::from("/"),
            fs_root: PathBuf::from("/"),
            home_dir: PathBuf::from("/home/user"),
            system_trees: vec![],
            mount_points: vec![],
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
            mount_points: vec![],
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
            mount_points: vec![],
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

    #[test]
    fn refuse_subtree_of_system_tree() {
        let policy = make_policy("/");
        assert_eq!(
            can_remove(Path::new("/usr/local"), &policy),
            Err(RemovalError::SystemTree)
        );
    }

    #[test]
    fn refuse_mount_point() {
        let policy = RemovalPolicy {
            scan_root: PathBuf::from("/home/user"),
            fs_root: PathBuf::from("/"),
            home_dir: PathBuf::from("/home/user"),
            system_trees: vec![],
            mount_points: vec![PathBuf::from("/home/user/mnt")],
        };
        assert_eq!(
            can_remove(Path::new("/home/user/mnt"), &policy),
            Err(RemovalError::MountPoint)
        );
        // A sibling of the mount is fine.
        assert!(can_remove(Path::new("/home/user/data"), &policy).is_ok());
    }

    #[test]
    fn refuse_directory_containing_a_mount_point() {
        let policy = RemovalPolicy {
            scan_root: PathBuf::from("/home/user"),
            fs_root: PathBuf::from("/"),
            home_dir: PathBuf::from("/home/user"),
            system_trees: vec![],
            mount_points: vec![PathBuf::from("/home/user/mnt/sub")],
        };
        assert_eq!(
            can_remove(Path::new("/home/user/mnt"), &policy),
            Err(RemovalError::MountPoint)
        );
        // Paths inside the mount itself are relative to that filesystem and
        // stay deletable.
        assert!(can_remove(Path::new("/home/user/mnt/sub/data"), &policy).is_ok());
    }

    #[test]
    fn permanent_delete_removes_special_names() {
        let tmp = tempfile::TempDir::new().unwrap();
        let policy = make_policy(tmp.path().to_str().unwrap());
        let names = ["my dir", "quote\"name", "new\nline", "tab\tsep"];
        let paths: Vec<MarkedPath> = names
            .iter()
            .map(|name| MarkedPath {
                path: tmp.path().join(name),
                size: 0,
                reclaimable: false,
            })
            .collect();
        for (name, marked) in names.iter().zip(&paths) {
            std::fs::create_dir(&marked.path).unwrap();
            std::fs::write(marked.path.join("inner"), name).unwrap();
        }

        let report = remove_paths(&paths, RemovalType::Permanent, &policy);
        assert!(
            report.failed.is_empty(),
            "unexpected failures: {:?}",
            report.failed
        );
        for marked in &paths {
            assert!(
                !marked.path.exists(),
                "{} still there",
                marked.path.display()
            );
        }
    }
}
