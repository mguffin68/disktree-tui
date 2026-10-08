//! Volume enumeration.

use std::path::{Path, PathBuf};

use crate::platform;

/// A mounted volume that can be scanned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// Mount point path.
    pub mount_point: PathBuf,
    /// Device or source (e.g. `/dev/nvme0n1p2`, `//host/share`).
    pub device: String,
    /// Filesystem type.
    pub filesystem: String,
    /// Free and total space. `None` for network filesystems, which are not
    /// queried: `statvfs` on an unreachable server can block indefinitely.
    pub space: Option<DiskSpace>,
}

/// Filesystem types that hold no user data worth scanning.
const PSEUDO_FILESYSTEMS: &[&str] = &[
    "proc",
    "sysfs",
    "devtmpfs",
    "devpts",
    "tmpfs",
    "securityfs",
    "cgroup",
    "cgroup2",
    "pstore",
    "bpf",
    "autofs",
    "mqueue",
    "hugetlbfs",
    "debugfs",
    "tracefs",
    "configfs",
    "fusectl",
    "binfmt_misc",
    "efivarfs",
    "ramfs",
    "rpc_pipefs",
    "nsfs",
    "overlay",
    "squashfs",
    "selinuxfs",
    "fuse.portal",
    "fuse.gvfsd-fuse",
    "nfsd",
];

/// Filesystem types reached over the network.
fn is_network(filesystem: &str) -> bool {
    matches!(
        filesystem,
        "cifs" | "smb3" | "smbfs" | "nfs" | "nfs4" | "9p" | "afs" | "fuse.sshfs" | "fuse.rclone"
    )
}

/// List mounted volumes worth scanning, root first.
pub fn list_volumes() -> Vec<Volume> {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "linux")] {
            let text = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
            volumes_from_mountinfo(&text, |p| disk_space(p).ok())
        } else if #[cfg(windows)] {
            ('A'..='Z')
                .map(|letter| PathBuf::from(format!("{letter}:\\")))
                .filter(|p| p.exists())
                .map(|p| Volume {
                    space: disk_space(&p).ok(),
                    device: p.display().to_string(),
                    filesystem: String::new(),
                    mount_point: p,
                })
                .collect()
        } else {
            // macOS and other Unixes: the root plus whatever is under /Volumes.
            let mut points = vec![PathBuf::from("/")];
            if let Ok(entries) = std::fs::read_dir("/Volumes") {
                points.extend(entries.flatten().map(|e| e.path()));
            }
            points
                .into_iter()
                .map(|p| Volume {
                    space: disk_space(&p).ok(),
                    device: String::new(),
                    filesystem: String::new(),
                    mount_point: p,
                })
                .collect()
        }
    }
}

/// Parse mountinfo into scannable volumes, querying space with `space`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn volumes_from_mountinfo(text: &str, space: impl Fn(&Path) -> Option<DiskSpace>) -> Vec<Volume> {
    let mut volumes: Vec<Volume> = Vec::new();
    for line in text.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let Some(field) = left.split(' ').nth(4) else {
            continue;
        };
        let mut rest = right.split(' ');
        let filesystem = rest.next().unwrap_or_default().to_string();
        let device = rest.next().unwrap_or_default().to_string();
        let mount_point = PathBuf::from(platform::unescape_mount_path(field));
        let hidden_tree = ["/proc", "/sys", "/dev", "/run", "/snap", "/var/lib/docker"]
            .iter()
            .any(|p| mount_point.starts_with(p) && !mount_point.starts_with("/run/media"));
        if PSEUDO_FILESYSTEMS.contains(&filesystem.as_str()) || hidden_tree {
            continue;
        }
        // Stacked mounts on one path: the last one is what is visible.
        volumes.retain(|v| v.mount_point != mount_point);
        let space = if is_network(&filesystem) {
            None
        } else {
            space(&mount_point)
        };
        volumes.push(Volume {
            mount_point,
            device,
            filesystem,
            space,
        });
    }
    volumes.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    volumes
}

/// The root of the filesystem holding `path`: what a whole-disk scan starts
/// from. Scans never cross filesystems, so this is where one ends too.
pub fn mount_root(path: &Path) -> std::io::Result<PathBuf> {
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            let path = path.canonicalize()?;
            let dev = std::fs::metadata(&path)?.dev();
            let mut root = path.clone();
            for ancestor in path.ancestors().skip(1) {
                match std::fs::metadata(ancestor) {
                    Ok(meta) if meta.dev() == dev => root = ancestor.to_path_buf(),
                    _ => break,
                }
            }
            Ok(root)
        } else {
            let path = path.canonicalize()?;
            Ok(path.ancestors().last().map(Path::to_path_buf).unwrap_or(path))
        }
    }
}

/// Space on the filesystem holding a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskSpace {
    /// Bytes available to unprivileged users.
    pub available: u64,
    /// Total size of the filesystem.
    pub total: u64,
}

/// Free and total space of the filesystem containing `path`.
pub fn disk_space(path: &std::path::Path) -> std::io::Result<DiskSpace> {
    let stats = fs4::statvfs(path)?;
    Ok(DiskSpace {
        available: stats.available_space(),
        total: stats.total_space(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mountinfo_keeps_real_filesystems_and_skips_network_space() {
        let text = "\
22 1 0:21 / / rw - btrfs /dev/nvme0n1p2 rw
23 22 0:5 / /proc rw - proc proc rw
24 22 0:22 / /home rw - btrfs /dev/nvme0n1p2 rw
25 22 0:23 / /tmp rw - tmpfs tmpfs rw
26 22 8:1 / /run/media/u/USB\\040Stick rw - exfat /dev/sda1 rw
27 24 0:60 / /home/u/mac rw - autofs systemd-1 rw
28 27 0:90 / /home/u/mac rw - cifs //host/DATA rw
";
        let fake = DiskSpace {
            available: 1,
            total: 2,
        };
        let volumes = volumes_from_mountinfo(text, |_| Some(fake));
        let points: Vec<_> = volumes
            .iter()
            .map(|v| v.mount_point.to_str().unwrap())
            .collect();
        assert_eq!(
            points,
            ["/", "/home", "/home/u/mac", "/run/media/u/USB Stick"]
        );
        let mac = &volumes[2];
        assert_eq!(mac.filesystem, "cifs");
        assert_eq!(mac.space, None, "network volumes are never queried");
        assert_eq!(volumes[0].space, Some(fake));
    }

    #[test]
    fn mount_root_is_an_ancestor_on_the_same_filesystem() {
        let tmp = std::env::temp_dir().canonicalize().unwrap();
        let root = mount_root(&tmp).unwrap();
        assert!(tmp.starts_with(&root));
    }

    #[test]
    fn disk_space_of_temp_dir() {
        let space = disk_space(&std::env::temp_dir()).unwrap();
        assert!(space.total > 0);
        assert!(space.available <= space.total);
    }
}
