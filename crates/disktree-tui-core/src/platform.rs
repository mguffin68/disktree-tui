//! Platform-specific utilities.

use std::path::PathBuf;

/// Get the platform's filesystem root path.
pub fn filesystem_root() -> PathBuf {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "windows")] {
            // Windows uses the drive letter of the current user's profile.
            std::env::var("SYSTEMDRIVE")
                .map(|d| PathBuf::from(d))
                .unwrap_or_else(|_| PathBuf::from("C:\\"))
        } else {
            PathBuf::from("/")
        }
    }
}

/// Get the platform's home directory.
pub fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|_| {
            cfg_if::cfg_if! {
                if #[cfg(target_os = "windows")] {
                    PathBuf::from("C:/Users/Current")
                } else {
                    PathBuf::from("/tmp")
                }
            }
        })
}

/// Get platform-specific system trees that should not be removed.
pub fn system_trees() -> Vec<PathBuf> {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "linux")] {
            vec![
                PathBuf::from("/usr"),
                PathBuf::from("/etc"),
                PathBuf::from("/boot"),
                PathBuf::from("/var/lib"),
                PathBuf::from("/nix/store"),
                PathBuf::from("/gnu/store"),
            ]
        } else if #[cfg(target_os = "macos")] {
            vec![
                PathBuf::from("/System"),
                PathBuf::from("/usr"),
                PathBuf::from("/Applications"),
            ]
        } else if #[cfg(target_os = "windows")] {
            vec![
                PathBuf::from("C:/Windows"),
                PathBuf::from("C:/Program Files"),
                PathBuf::from("C:/Program Files (x86)"),
            ]
        } else {
            Vec::new()
        }
    }
}
