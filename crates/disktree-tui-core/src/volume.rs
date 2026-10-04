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
