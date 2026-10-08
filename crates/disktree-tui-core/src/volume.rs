//! Volume enumeration.

use std::path::PathBuf;

/// Information about a mounted volume.
#[derive(Debug, Clone)]
pub struct Volume {
    /// Mount point path.
    pub mount_point: PathBuf,
    /// Device/source name.
    pub device: String,
    /// Filesystem type.
    pub filesystem: String,
    /// Free space in bytes.
    pub free_bytes: u64,
    /// Total space in bytes.
    pub total_bytes: u64,
}

/// List all candidate volumes for scanning.
pub fn list_volumes() -> Vec<Volume> {
    // Placeholder: will be implemented per-platform.
    // Use sysinfo crate for cross-platform volume listing.
    vec![Volume {
        mount_point: PathBuf::from("/"),
        device: "root".to_string(),
        filesystem: "unknown".to_string(),
        free_bytes: 0,
        total_bytes: 0,
    }]
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
    fn disk_space_of_temp_dir() {
        let space = disk_space(&std::env::temp_dir()).unwrap();
        assert!(space.total > 0);
        assert!(space.available <= space.total);
    }
}
