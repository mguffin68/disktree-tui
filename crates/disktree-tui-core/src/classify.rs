//! Classification of directories by data kind.

/// A category of data stored in a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Source code and project files.
    Code,
    /// Cache directories (browser, compiler, package manager, etc.).
    Caches,
    /// Build output (compiler artifacts, dist bundles, etc.).
    Build,
    /// Git metadata and bare repositories.
    Git,
    /// Media files (video, audio, images).
    Media,
    /// Documents and downloads.
    Documents,
    /// Language toolchains and SDKs.
    Toolchains,
    /// Synchronized cloud folders.
    Synced,
    /// AI agent scratch/work directories.
    Agent,
    /// Unclassified.
    Unknown,
}

impl Class {
    /// Human-readable label for display.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Code => "Code",
            Self::Caches => "Caches",
            Self::Build => "Build",
            Self::Git => "Git",
            Self::Media => "Media",
            Self::Documents => "Documents",
            Self::Toolchains => "Toolchains",
            Self::Synced => "Synced",
            Self::Agent => "Agent",
            Self::Unknown => "Unknown",
        }
    }
}

/// Classify a directory by its name and context.
pub fn classify(name: &str, _context: &ClassifyContext) -> Class {
    let lower = name.to_lowercase();

    // Caches
    if lower == ".cache"
        || lower == "cache"
        || lower == "__pycache__"
        || lower == "cachegrind-out"
        || lower == "trash"
    {
        return Class::Caches;
    }

    // Build output
    if lower == "target"
        || lower == "build"
        || lower == "dist"
        || lower == ".dart_tool"
        || lower == ".gradle"
        || lower == "out"
        || lower == "bin"
    {
        return Class::Build;
    }

    // Git
    if lower == ".git" {
        return Class::Git;
    }

    // Media
    if lower == "videos"
        || lower == "movies"
        || lower == "music"
        || lower == "pictures"
        || lower == "images"
        || lower == "photos"
        || lower == "media"
    {
        return Class::Media;
    }

    // Documents
    if lower == "documents" || lower == "downloads" || lower == "desktop" || lower == "public" {
        return Class::Documents;
    }

    // Toolchains
    if lower == ".rustup"
        || lower == ".cargo"
        || lower == "node_modules"
        || lower == ".local"
        || lower == "appdata"
    {
        return Class::Toolchains;
    }

    // Synced
    if lower == "dropbox" || lower == "nextcloud" || lower == "sync" || lower == "onedrive" {
        return Class::Synced;
    }

    // Agent scratch
    if lower.starts_with(".aider")
        || lower == ".claude"
        || lower == ".cursor"
        || lower == ".windsurf"
    {
        return Class::Agent;
    }

    // Code (last resort for common project dirs)
    if lower == "src"
        || lower == "app"
        || lower == "lib"
        || lower == "packages"
        || lower == "source"
    {
        return Class::Code;
    }

    Class::Unknown
}

/// Context information available during classification.
pub struct ClassifyContext {
    /// Parent directory name.
    pub parent_name: String,
    /// Whether a Cargo.toml exists nearby.
    pub has_cargo_toml: bool,
    /// Whether a package.json exists nearby.
    pub has_package_json: bool,
    /// Whether a go.mod exists nearby.
    pub has_go_mod: bool,
}

/// Check if a directory is reclaimable based on its class and name.
pub fn is_reclaimable(class: &Class, name: &str) -> bool {
    // Caches are always reclaimable.
    if *class == Class::Caches {
        return true;
    }

    // Build output is reclaimable.
    if *class == Class::Build {
        return true;
    }

    // Package stores
    let lower = name.to_lowercase();
    if lower.contains("pacman/pkg")
        || lower.contains("apt/archives")
        || lower.contains("pip/cache")
        || lower.contains("nuget")
        || lower.contains("m2/repository")
        || lower.contains("composer/cache")
    {
        return true;
    }

    // Temp directories
    if lower == "tmp" || lower == "temp" || lower == ".tmp" {
        return true;
    }

    false
}
