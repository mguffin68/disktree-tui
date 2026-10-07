//! Platform-specific utilities.

use std::path::PathBuf;

/// Get the platform's filesystem root path.
pub fn filesystem_root() -> PathBuf {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "windows")] {
            // Windows uses the drive letter of the current user's profile.
            std::env::var("SYSTEMDRIVE")
                .map(PathBuf::from)
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

/// Size an entry contributes to the total.
///
/// Disk usage is allocated blocks (`st_blocks * 512`) on Unix. On Windows both
/// modes use the logical length for now; NTFS allocation-aware sizing is M7.
pub fn entry_size(meta: &std::fs::Metadata, apparent: bool) -> u64 {
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            if apparent { meta.len() } else { meta.blocks() * 512 }
        } else {
            let _ = apparent;
            meta.len()
        }
    }
}

/// Identity of a file with more than one hardlink, as `(device, inode)`.
///
/// Returns `None` for singly-linked files so the dedup set stays small. On
/// Windows hardlink identity needs a file handle, so dedup is not done there yet.
pub fn hardlink_id(meta: &std::fs::Metadata) -> Option<(u64, u64)> {
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            (meta.nlink() > 1).then(|| (meta.dev(), meta.ino()))
        } else {
            let _ = meta;
            None
        }
    }
}

/// Device the entry lives on, used to stay on one filesystem. `None` on
/// Windows, where a scan never leaves its volume through a plain directory.
pub fn device_id(meta: &std::fs::Metadata) -> Option<u64> {
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            Some(meta.dev())
        } else {
            let _ = meta;
            None
        }
    }
}

/// Last modification time as a Unix timestamp, if available.
pub fn modified_secs(meta: &std::fs::Metadata) -> Option<u64> {
    meta.modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Mount points currently mounted, read from `/proc/self/mountinfo`.
///
/// Lets a scan skip mounts by path without ever calling `stat` on them, which
/// can block indefinitely on an unreachable network filesystem. Elsewhere this
/// returns an empty list and scans rely on device-number checks alone.
pub fn mount_points() -> Vec<PathBuf> {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "linux")] {
            std::fs::read_to_string("/proc/self/mountinfo")
                .map(|s| parse_mountinfo(&s))
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    }
}

/// Extract mount points (the fifth field) from mountinfo text.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_mountinfo(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|line| line.split(' ').nth(4))
        .map(|field| PathBuf::from(unescape_mount_path(field)))
        .collect()
}

/// Decode the octal escapes (`\040` for space, etc.) used in mountinfo paths.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn unescape_mount_path(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && let Some(code) = bytes
                .get(i + 1..i + 4)
                .and_then(|oct| std::str::from_utf8(oct).ok())
                .and_then(|oct| u8::from_str_radix(oct, 8).ok())
        {
            out.push(code);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mountinfo_extracts_and_unescapes_mount_points() {
        let text = "\
22 1 0:21 / / rw,relatime shared:1 - btrfs /dev/nvme0n1p2 rw
249 22 0:60 / /home/u/mac rw,relatime shared:213 - autofs systemd-1 rw
1663 249 0:190 / /home/u/my\\040share rw - cifs //host/DATA rw
";
        assert_eq!(
            parse_mountinfo(text),
            [
                PathBuf::from("/"),
                PathBuf::from("/home/u/mac"),
                PathBuf::from("/home/u/my share"),
            ]
        );
    }
}
