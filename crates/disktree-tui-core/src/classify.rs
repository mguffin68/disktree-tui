//! Classification of directories by data kind, and reclaimable-space detection.
//!
//! The scanner records which marker files each directory directly contains
//! ([`Markers`]); [`classify_tree`] then walks the finished tree top-down and
//! decides each node from its name, its own markers, its parent's markers, and
//! what it inherits from its ancestors.

use std::ffi::OsStr;
use std::ops::{BitOr, BitOrAssign};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::platform;
use crate::tree::DirNode;

/// A category of data stored in a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
    /// Every class, in legend order.
    pub const ALL: [Class; 10] = [
        Self::Code,
        Self::Caches,
        Self::Build,
        Self::Git,
        Self::Media,
        Self::Documents,
        Self::Toolchains,
        Self::Synced,
        Self::Agent,
        Self::Unknown,
    ];

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

    /// Whether everything below a directory of this class belongs to it too.
    ///
    /// A `dist/` inside `node_modules/pkg/` is part of a toolchain, not build
    /// output; a `src/` inside `Documents/` is still code.
    pub fn is_sticky(self) -> bool {
        matches!(
            self,
            Self::Caches | Self::Build | Self::Git | Self::Toolchains | Self::Agent
        )
    }
}

/// Marker files and directories a directory directly contains.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Markers(u16);

impl Markers {
    /// No markers.
    pub const NONE: Self = Self(0);
    /// `Cargo.toml`.
    pub const CARGO: Self = Self(1);
    /// `package.json`.
    pub const PACKAGE_JSON: Self = Self(1 << 1);
    /// `go.mod`.
    pub const GO_MOD: Self = Self(1 << 2);
    /// `pyproject.toml`, `setup.py`, or `requirements.txt`.
    pub const PYTHON: Self = Self(1 << 3);
    /// `CMakeLists.txt`, `Makefile`, or `meson.build`.
    pub const NATIVE_BUILD: Self = Self(1 << 4);
    /// `build.gradle(.kts)`, `settings.gradle(.kts)`, or `pom.xml`.
    pub const JVM: Self = Self(1 << 5);
    /// `*.csproj`, `*.fsproj`, `*.vbproj`, or `*.sln`.
    pub const DOTNET: Self = Self(1 << 6);
    /// `pubspec.yaml`.
    pub const PUBSPEC: Self = Self(1 << 7);
    /// `.git` (directory or worktree file).
    pub const GIT: Self = Self(1 << 8);
    /// `CACHEDIR.TAG` (the cross-tool "this is a cache" convention).
    pub const CACHEDIR_TAG: Self = Self(1 << 9);
    /// `HEAD` file (bare git repository shape).
    pub const GIT_HEAD: Self = Self(1 << 10);
    /// `objects` directory (bare git repository shape).
    pub const GIT_OBJECTS: Self = Self(1 << 11);
    /// `refs` directory (bare git repository shape).
    pub const GIT_REFS: Self = Self(1 << 12);

    /// Any project manifest that makes a directory a project root.
    const PROJECT: Self = Self(
        Self::CARGO.0
            | Self::PACKAGE_JSON.0
            | Self::GO_MOD.0
            | Self::PYTHON.0
            | Self::NATIVE_BUILD.0
            | Self::JVM.0
            | Self::DOTNET.0
            | Self::PUBSPEC.0,
    );
    const BARE_GIT: Self = Self(Self::GIT_HEAD.0 | Self::GIT_OBJECTS.0 | Self::GIT_REFS.0);

    /// The marker a directory entry with this name represents, if any.
    pub fn from_entry_name(name: &OsStr) -> Self {
        let Some(name) = name.to_str() else {
            return Self::NONE;
        };
        match name {
            "Cargo.toml" => Self::CARGO,
            "package.json" => Self::PACKAGE_JSON,
            "go.mod" => Self::GO_MOD,
            "pyproject.toml" | "setup.py" | "requirements.txt" => Self::PYTHON,
            "CMakeLists.txt" | "Makefile" | "meson.build" => Self::NATIVE_BUILD,
            "build.gradle"
            | "build.gradle.kts"
            | "settings.gradle"
            | "settings.gradle.kts"
            | "pom.xml" => Self::JVM,
            "pubspec.yaml" => Self::PUBSPEC,
            ".git" => Self::GIT,
            "CACHEDIR.TAG" => Self::CACHEDIR_TAG,
            "HEAD" => Self::GIT_HEAD,
            "objects" => Self::GIT_OBJECTS,
            "refs" => Self::GIT_REFS,
            n if [".csproj", ".fsproj", ".vbproj", ".sln"]
                .iter()
                .any(|ext| n.ends_with(ext)) =>
            {
                Self::DOTNET
            }
            _ => Self::NONE,
        }
    }

    /// Whether all markers in `other` are present.
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    fn is_project(self) -> bool {
        self.intersects(Self::PROJECT)
    }
}

impl BitOr for Markers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Markers {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Multi-component path suffixes that identify package stores and other
/// well-known locations. These win even inside a sticky ancestor, so that
/// `~/.cargo/registry` is a reclaimable cache inside the `.cargo` toolchain.
const PATH_RULES: &[(&[&str], Class, bool)] = &[
    (&["pacman", "pkg"], Class::Caches, true),
    (&["apt", "archives"], Class::Caches, true),
    (&[".npm", "_cacache"], Class::Caches, true),
    (&[".m2", "repository"], Class::Caches, true),
    (&[".nuget", "packages"], Class::Caches, true),
    (&[".cargo", "registry"], Class::Caches, true),
    (&[".cargo", "git"], Class::Caches, true),
    (&["go", "pkg", "mod"], Class::Caches, true),
    (&[".gradle", "caches"], Class::Caches, true),
    (&["library", "caches"], Class::Caches, true),
    (&["appdata", "local", "temp"], Class::Caches, true),
    (&[".local", "share", "trash"], Class::Caches, true),
    (&[".local", "share", "flatpak"], Class::Toolchains, false),
];

/// What a node passes down to its children.
#[derive(Debug, Clone, Copy)]
struct Inherited {
    class: Class,
    reclaimable: bool,
    /// Inside a protected tree: inherit verbatim, no rules apply.
    protected: bool,
    /// The parent directory's own markers ("beside" checks for children).
    markers: Markers,
}

impl Inherited {
    const UNKNOWN: Self = Self {
        class: Class::Unknown,
        reclaimable: false,
        protected: false,
        markers: Markers::NONE,
    };

    /// What a directory with verdict `v` and markers `markers` passes down.
    /// Home's markers are withheld: a stray `~/Makefile` must not turn
    /// `~/build` into reclaimable build output.
    fn below(v: Verdict, markers: Markers, is_home: bool) -> Self {
        Self {
            class: v.class,
            reclaimable: v.reclaimable,
            protected: v.protected,
            markers: if is_home { Markers::NONE } else { markers },
        }
    }
}

/// A classification decision for one directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Verdict {
    class: Class,
    reclaimable: bool,
    protected: bool,
}

// Returns `Option` so rule arms read `=> verdict(..)` alongside `=> None`.
#[allow(clippy::unnecessary_wraps)]
const fn verdict(class: Class, reclaimable: bool) -> Option<Verdict> {
    Some(Verdict {
        class,
        reclaimable,
        protected: false,
    })
}

/// Classify every node of a scanned tree in place.
///
/// The root's ancestors are taken into account, so scanning `~/.cache/pip`
/// directly still yields Caches, and scanning a `target/` beside a
/// `Cargo.toml` still yields Build.
pub fn classify_tree(root: &mut DirNode) {
    let home = home_under(&root.path);
    let context = root_context(&root.path);
    classify_node(root, context, home.as_deref());
}

/// Classify a tree as if its root had no meaningful ancestors. Lets tests use
/// fixtures under `/tmp` (itself classified as reclaimable temp space).
#[cfg(test)]
pub(crate) fn classify_detached(root: &mut DirNode) {
    let context = Inherited::UNKNOWN;
    classify_node(root, context, None);
}

fn classify_node(node: &mut DirNode, parent: Inherited, home: Option<&Path>) {
    let lower = lowercase_components(&node.path);
    let is_home = home == Some(node.path.as_path());
    let v = decide(&lower, node.markers, parent, is_home);
    node.class = v.class;
    node.reclaimable = v.reclaimable;

    let down = Inherited::below(v, node.markers, is_home);
    for child in &mut node.children {
        classify_node(child, down, home);
    }
}

/// Decide one directory. `lower` is its path, lowercased, one entry per
/// component.
fn decide(lower: &[String], own: Markers, parent: Inherited, is_home: bool) -> Verdict {
    let inherited = Verdict {
        class: parent.class,
        reclaimable: parent.reclaimable,
        protected: parent.protected,
    };
    if parent.protected {
        return inherited;
    }
    if is_protected(lower) {
        return Verdict {
            class: Class::Unknown,
            reclaimable: false,
            protected: true,
        };
    }
    if let Some(mut v) = path_rule(lower) {
        v.reclaimable |= parent.reclaimable;
        return v;
    }
    if parent.class.is_sticky() {
        return inherited;
    }
    let name = lower.last().map_or("", String::as_str);
    let in_code = parent.class == Class::Code;
    let Some(mut v) = name_rule(name, own, parent.markers, in_code, is_home) else {
        return inherited;
    };
    // Anything inside something reclaimable is reclaimable too.
    v.reclaimable |= parent.reclaimable;
    v
}

/// Container storage and filesystem snapshots hold whole filesystems: their
/// `tmp/`, `var/cache/` and `__pycache__/` look reclaimable but deleting them
/// corrupts the image or snapshot. Nothing inside is classified or hatched.
const PROTECTED_PATHS: &[&[&str]] = &[
    &[".local", "share", "containers"],
    &["var", "lib", "docker"],
    &["var", "lib", "containers"],
    &["var", "lib", "machines"],
    &["var", "lib", "lxc"],
    &["var", "lib", "libvirt"],
    &[".snapshots"],
    &["timeshift"],
    &["timeshift-btrfs"],
];

fn is_protected(lower: &[String]) -> bool {
    PROTECTED_PATHS
        .iter()
        .any(|suffix| lower.ends_with_strs(suffix))
}

fn path_rule(lower: &[String]) -> Option<Verdict> {
    PATH_RULES
        .iter()
        .find(|(suffix, _, _)| lower.ends_with_strs(suffix))
        .and_then(|&(_, class, reclaimable)| verdict(class, reclaimable))
}

/// Rules based on a directory's own name and shape, and on what sits beside it.
///
/// Generic names like `cache` or `tmp` only count outside Code: a `src/cache/`
/// module is source, and calling it reclaimable would invite deleting it.
fn name_rule(
    name: &str,
    own: Markers,
    beside: Markers,
    in_code: bool,
    is_home: bool,
) -> Option<Verdict> {
    use Class::{Agent, Build, Caches, Code, Documents, Git, Media, Synced, Toolchains};

    // Shape: bare repositories.
    if own.contains(Markers::BARE_GIT) && !own.is_project() {
        return verdict(Git, false);
    }

    match name {
        ".git" => return verdict(Git, false),

        // Build output, recognized by what sits beside it.
        "target" if beside.contains(Markers::CARGO) => return verdict(Build, true),
        "build" | "dist" | "out" if beside.is_project() => return verdict(Build, true),
        "bin" | "obj" if beside.contains(Markers::DOTNET) => return verdict(Build, true),
        ".dart_tool" if beside.contains(Markers::PUBSPEC) => return verdict(Build, true),
        ".gradle" if beside.contains(Markers::JVM) => return verdict(Build, true),
        ".next" | ".nuxt" | ".svelte-kit" | ".angular" | ".expo" | "zig-out" | "zig-cache"
        | ".zig-cache" => return verdict(Build, true),
        n if n.starts_with("cmake-build-") => return verdict(Build, true),

        // Dependencies: regenerable from the manifest beside them.
        "node_modules" => {
            return verdict(Toolchains, beside.contains(Markers::PACKAGE_JSON));
        }

        // Project content beside a manifest.
        "src" | "app" | "lib" | "packages" | "source" | "public" | "assets" | "tests"
            if beside.is_project() =>
        {
            return verdict(Code, false);
        }

        _ => {}
    }

    // Any directory carrying the cache convention tag.
    if own.contains(Markers::CACHEDIR_TAG) {
        return verdict(Caches, true);
    }

    match name {
        // Tool-specific caches, trash, and sync version history: unambiguous
        // anywhere, including inside projects.
        ".cache" | "__pycache__" | ".pytest_cache" | ".mypy_cache" | ".ruff_cache" | ".tox"
        | ".parcel-cache" | ".turbo" | ".stversions" | ".dropbox.cache" => verdict(Caches, true),
        n if n.starts_with(".trash-") => verdict(Caches, true),

        // Generic cache and temp names (browser/Electron `Cache`, `GPUCache`,
        // `component_crx_cache`, ...), outside projects only.
        "cache" | "caches" | "cachestorage" | "cacheddata" | "trash" | ".trash" | "tmp"
        | "temp" | ".tmp"
            if !in_code =>
        {
            verdict(Caches, true)
        }
        n if n.ends_with("cache") && !in_code => verdict(Caches, true),

        // Toolchains and SDKs.
        ".rustup" | ".cargo" | ".nvm" | ".pyenv" | ".rbenv" | ".sdkman" | ".volta" | ".bun"
        | ".deno" | ".dotnet" | ".ghcup" | ".stack" | ".opam" | ".julia" | ".venv" | "venv" => {
            verdict(Toolchains, false)
        }

        // AI agent state.
        ".claude" | ".cursor" | ".windsurf" | ".codex" | ".gemini" | ".continue" | ".agents" => {
            verdict(Agent, false)
        }
        n if n.starts_with(".aider") => verdict(Agent, false),

        "videos" | "movies" | "music" | "pictures" | "images" | "photos" | "media"
        | "wallpapers" | "screenshots" => verdict(Media, false),

        "documents" | "downloads" | "desktop" | "public" => verdict(Documents, false),

        "dropbox" | "nextcloud" | "sync" | "onedrive" | "google drive" | "icloud drive"
        | "mobile documents" => verdict(Synced, false),

        // Conventional homes for checkouts.
        "projects" | "code" | "repos" | "src" | "sources" | "workspace" | "git" => {
            verdict(Code, false)
        }

        // A project root. A dotfiles repo doesn't make all of $HOME code.
        _ if (own.is_project() || own.contains(Markers::GIT)) && !is_home => verdict(Code, false),

        _ => None,
    }
}

/// Context for the scan root, derived from its ancestors as they are on disk.
fn root_context(root: &Path) -> Inherited {
    let Ok(canonical) = root.canonicalize() else {
        return context_from_chain(&[]);
    };
    // Compare canonical to canonical: `HOME` may be a symlink, and Windows
    // canonical paths carry a `\\?\` prefix.
    let home = platform::home_dir().canonicalize().ok();
    let chain: Vec<Ancestor> = canonical
        .ancestors()
        .skip(1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|dir| Ancestor {
            lower: lowercase_components(dir),
            markers: read_markers(dir),
            is_home: home.as_deref() == Some(dir),
        })
        .collect();
    context_from_chain(&chain)
}

/// One ancestor of the scan root, outermost first.
struct Ancestor {
    lower: Vec<String>,
    markers: Markers,
    is_home: bool,
}

/// Run the same decisions the tree walk makes down the ancestor chain; the
/// result is what the scan root inherits.
fn context_from_chain(chain: &[Ancestor]) -> Inherited {
    let mut context = Inherited::UNKNOWN;
    for ancestor in chain {
        let v = decide(&ancestor.lower, ancestor.markers, context, ancestor.is_home);
        context = Inherited::below(v, ancestor.markers, ancestor.is_home);
    }
    context
}

fn lowercase_components(path: &Path) -> Vec<String> {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

/// Markers directly inside `dir`, read from disk.
fn read_markers(dir: &Path) -> Markers {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().fold(Markers::NONE, |m, e| {
                m | Markers::from_entry_name(&e.file_name())
            })
        })
        .unwrap_or_default()
}

/// The home directory expressed under `root` as given, if it lies within it.
fn home_under(root: &Path) -> Option<PathBuf> {
    let home = platform::home_dir().canonicalize().ok()?;
    let rel = home.strip_prefix(root.canonicalize().ok()?).ok()?;
    Some(root.join(rel))
}

trait EndsWithStrs {
    fn ends_with_strs(&self, suffix: &[&str]) -> bool;
}

impl EndsWithStrs for [String] {
    fn ends_with_strs(&self, suffix: &[&str]) -> bool {
        self.len() >= suffix.len()
            && self[self.len() - suffix.len()..]
                .iter()
                .zip(suffix)
                .all(|(a, b)| a == b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNKNOWN: Inherited = Inherited::UNKNOWN;

    fn path(p: &str) -> Vec<String> {
        p.split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase)
            .collect()
    }

    fn check(p: &str, own: Markers, parent: Inherited) -> (Class, bool) {
        let v = decide(&path(p), own, parent, false);
        (v.class, v.reclaimable)
    }

    fn beside(markers: Markers) -> Inherited {
        Inherited { markers, ..UNKNOWN }
    }

    fn inside(class: Class, reclaimable: bool) -> Inherited {
        Inherited {
            class,
            reclaimable,
            ..UNKNOWN
        }
    }

    #[test]
    fn dot_cache_is_reclaimable_cache() {
        assert_eq!(
            check("/home/u/.cache", Markers::NONE, UNKNOWN),
            (Class::Caches, true)
        );
    }

    #[test]
    fn target_beside_cargo_toml_is_build() {
        assert_eq!(
            check("/p/target", Markers::NONE, beside(Markers::CARGO)),
            (Class::Build, true)
        );
        // Without a manifest beside it, `target` is just a name.
        assert_eq!(
            check("/p/target", Markers::NONE, UNKNOWN),
            (Class::Unknown, false)
        );
    }

    #[test]
    fn cachedir_tag_marks_a_cache() {
        assert_eq!(
            check("/x/whatever", Markers::CACHEDIR_TAG, UNKNOWN),
            (Class::Caches, true)
        );
    }

    #[test]
    fn src_is_code_not_reclaimable() {
        assert_eq!(
            check("/p/src", Markers::NONE, beside(Markers::CARGO)),
            (Class::Code, false)
        );
        assert_eq!(
            check("/home/u/src", Markers::NONE, UNKNOWN),
            (Class::Code, false)
        );
    }

    #[test]
    fn downloads_is_documents_not_reclaimable() {
        assert_eq!(
            check("/home/u/Downloads", Markers::NONE, UNKNOWN),
            (Class::Documents, false)
        );
    }

    #[test]
    fn unknown_directory_is_unknown() {
        assert_eq!(
            check("/home/u/stuff", Markers::NONE, UNKNOWN),
            (Class::Unknown, false)
        );
    }

    #[test]
    fn project_root_is_code_except_home() {
        assert_eq!(
            check("/home/u/proj", Markers::PACKAGE_JSON, UNKNOWN),
            (Class::Code, false)
        );
        let v = decide(&path("/home/u"), Markers::GIT, UNKNOWN, true);
        assert_eq!(v.class, Class::Unknown);
    }

    #[test]
    fn build_dirs_need_a_project_beside_them() {
        let project = beside(Markers::PACKAGE_JSON);
        assert_eq!(
            check("/p/dist", Markers::NONE, project),
            (Class::Build, true)
        );
        assert_eq!(
            check("/home/u/build", Markers::NONE, UNKNOWN).0,
            Class::Unknown
        );
        // `~/bin` is user scripts, not build output.
        assert_eq!(
            check("/home/u/bin", Markers::NONE, UNKNOWN).0,
            Class::Unknown
        );
        assert_eq!(
            check("/p/bin", Markers::NONE, beside(Markers::DOTNET)),
            (Class::Build, true)
        );
    }

    #[test]
    fn node_modules_reclaimable_only_beside_package_json() {
        assert_eq!(
            check(
                "/p/node_modules",
                Markers::NONE,
                beside(Markers::PACKAGE_JSON)
            ),
            (Class::Toolchains, true)
        );
        assert_eq!(
            check("/x/node_modules", Markers::NONE, UNKNOWN),
            (Class::Toolchains, false)
        );
    }

    #[test]
    fn sticky_classes_absorb_descendants() {
        // `dist` beside a package.json inside node_modules stays a toolchain.
        let parent = Inherited {
            class: Class::Toolchains,
            reclaimable: true,
            markers: Markers::PACKAGE_JSON,
            ..UNKNOWN
        };
        assert_eq!(
            check("/p/node_modules/pkg/dist", Markers::NONE, parent),
            (Class::Toolchains, true)
        );
        assert_eq!(
            check(
                "/home/u/.cache/mozilla/src",
                Markers::NONE,
                inside(Class::Caches, true)
            ),
            (Class::Caches, true)
        );
    }

    #[test]
    fn non_sticky_classes_can_be_overridden() {
        assert_eq!(
            check(
                "/home/u/Documents/proj",
                Markers::CARGO,
                inside(Class::Documents, false)
            ),
            (Class::Code, false)
        );
        // Unmatched children inherit.
        assert_eq!(
            check(
                "/home/u/Documents/taxes",
                Markers::NONE,
                inside(Class::Documents, false)
            ),
            (Class::Documents, false)
        );
    }

    #[test]
    fn path_rules_win_inside_sticky_parents() {
        assert_eq!(
            check(
                "/home/u/.cargo/registry",
                Markers::NONE,
                inside(Class::Toolchains, false)
            ),
            (Class::Caches, true)
        );
        assert_eq!(
            check(
                "/home/u/.cargo/bin",
                Markers::NONE,
                inside(Class::Toolchains, false)
            ),
            (Class::Toolchains, false)
        );
    }

    #[test]
    fn package_stores_and_temp_are_reclaimable() {
        for p in [
            "/var/cache/pacman/pkg",
            "/var/cache/apt/archives",
            "/home/u/.npm/_cacache",
            "/home/u/.m2/repository",
            "/home/u/go/pkg/mod",
            "/Users/u/Library/Caches",
            "/home/u/.local/share/Trash",
            "/tmp",
            "/var/tmp",
            "/home/u/.stversions",
        ] {
            assert!(
                check(p, Markers::NONE, UNKNOWN).1,
                "{p} should be reclaimable"
            );
        }
    }

    #[test]
    fn reclaimable_is_independent_of_class() {
        // A cache-tagged directory nested in a project is reclaimable even
        // though its parent (Code) is not.
        assert_eq!(
            check(
                "/p/src/.pytest_cache",
                Markers::NONE,
                inside(Class::Code, false)
            ),
            (Class::Caches, true)
        );
        // Children of something reclaimable stay reclaimable.
        assert!(
            check(
                "/p/node_modules/x",
                Markers::NONE,
                inside(Class::Toolchains, true)
            )
            .1
        );
    }

    #[test]
    fn generic_cache_names_outside_projects() {
        for name in [
            "Cache",
            "GPUCache",
            "Code Cache",
            "component_crx_cache",
            "GPUPersistentCache",
        ] {
            let p = format!("/home/u/.config/google-chrome/Default/{name}");
            assert_eq!(
                check(&p, Markers::NONE, UNKNOWN),
                (Class::Caches, true),
                "{name}"
            );
        }
    }

    #[test]
    fn generic_cache_names_inside_projects_are_source() {
        let code = inside(Class::Code, false);
        assert_eq!(
            check("/p/src/cache", Markers::NONE, code),
            (Class::Code, false)
        );
        assert_eq!(
            check("/p/src/tmp", Markers::NONE, code),
            (Class::Code, false)
        );
        assert_eq!(
            check("/p/src/querycache", Markers::NONE, code),
            (Class::Code, false)
        );
        // Tool caches are still caches inside a project.
        assert_eq!(
            check("/p/__pycache__", Markers::NONE, code),
            (Class::Caches, true)
        );
    }

    #[test]
    fn bare_git_repository_shape() {
        let bare = Markers::GIT_HEAD | Markers::GIT_OBJECTS | Markers::GIT_REFS;
        assert_eq!(check("/srv/repo.git", bare, UNKNOWN), (Class::Git, false));
        assert_eq!(check("/p/.git", bare, UNKNOWN), (Class::Git, false));
    }

    #[test]
    fn agent_and_media_names() {
        assert_eq!(
            check("/home/u/.claude", Markers::NONE, UNKNOWN).0,
            Class::Agent
        );
        assert_eq!(
            check("/home/u/.aider.tags", Markers::NONE, UNKNOWN).0,
            Class::Agent
        );
        assert_eq!(
            check("/home/u/Music", Markers::NONE, UNKNOWN).0,
            Class::Media
        );
        assert_eq!(
            check("/home/u/Dropbox", Markers::NONE, UNKNOWN).0,
            Class::Synced
        );
    }

    fn ancestor(p: &str, markers: Markers) -> Ancestor {
        Ancestor {
            lower: path(p),
            markers,
            is_home: false,
        }
    }

    #[test]
    fn ancestor_chain_supplies_root_context() {
        // Scanning ~/proj/target/debug directly: `target` sits beside Cargo.toml.
        let chain = [
            ancestor("/", Markers::NONE),
            ancestor("/home", Markers::NONE),
            ancestor("/home/u", Markers::NONE),
            ancestor("/home/u/proj", Markers::CARGO),
            ancestor("/home/u/proj/target", Markers::CACHEDIR_TAG),
        ];
        let context = context_from_chain(&chain);
        assert_eq!((context.class, context.reclaimable), (Class::Build, true));

        let chain = [
            ancestor("/", Markers::NONE),
            ancestor("/home/u/.cache", Markers::NONE),
        ];
        assert_eq!(context_from_chain(&chain).class, Class::Caches);
        assert_eq!(context_from_chain(&[]).class, Class::Unknown);
    }

    #[test]
    fn home_markers_do_not_leak_to_children() {
        // A dotfiles Makefile at ~ must not make ~/build reclaimable.
        let mut home = ancestor("/home/u", Markers::NATIVE_BUILD | Markers::GIT);
        home.is_home = true;
        let chain = [
            ancestor("/", Markers::NONE),
            ancestor("/home", Markers::NONE),
            home,
        ];
        let context = context_from_chain(&chain);
        assert_eq!(context.class, Class::Unknown);
        assert_eq!(
            check("/home/u/build", Markers::NONE, context),
            (Class::Unknown, false)
        );

        // Same through the tree walk.
        let mut root = DirNode::new("/home/u".into(), "u".into(), 0);
        root.markers = Markers::NATIVE_BUILD;
        root.absorb_child(DirNode::new("/home/u/build".into(), "build".into(), 1));
        classify_node(&mut root, UNKNOWN, Some(Path::new("/home/u")));
        assert!(!root.children[0].reclaimable);
    }

    #[test]
    fn protected_trees_are_never_reclaimable() {
        let protected = check("/home/u/.local/share/containers", Markers::NONE, UNKNOWN);
        assert_eq!(protected, (Class::Unknown, false));
        let v = decide(
            &path("/home/u/.local/share/containers"),
            Markers::NONE,
            UNKNOWN,
            false,
        );
        let inside = Inherited::below(v, Markers::NONE, false);
        for p in [
            "/home/u/.local/share/containers/storage/overlay/x/diff/tmp",
            "/home/u/.local/share/containers/storage/overlay/x/diff/var/cache/apt/archives",
            "/home/u/.local/share/containers/storage/overlay/x/diff/usr/lib/__pycache__",
        ] {
            assert_eq!(
                check(p, Markers::NONE, inside),
                (Class::Unknown, false),
                "{p}"
            );
        }
        assert!(!check("/.snapshots", Markers::NONE, UNKNOWN).1);
    }

    #[test]
    fn path_rules_keep_inherited_reclaimable() {
        assert!(
            check(
                "/x/.local/share/flatpak",
                Markers::NONE,
                inside(Class::Caches, true)
            )
            .1
        );
    }

    #[test]
    fn root_parent_markers_are_read_from_disk() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "").unwrap();
        std::fs::create_dir(tmp.path().join("target")).unwrap();
        let context = root_context(&tmp.path().join("target"));
        assert!(context.markers.contains(Markers::CARGO));
    }

    #[test]
    fn markers_from_entry_names() {
        assert_eq!(
            Markers::from_entry_name(OsStr::new("Cargo.toml")),
            Markers::CARGO
        );
        assert_eq!(
            Markers::from_entry_name(OsStr::new("App.csproj")),
            Markers::DOTNET
        );
        assert_eq!(
            Markers::from_entry_name(OsStr::new("README.md")),
            Markers::NONE
        );
        let m = Markers::CARGO | Markers::GIT;
        assert!(m.contains(Markers::GIT));
        assert!(!m.contains(Markers::PACKAGE_JSON));
    }

    #[test]
    fn class_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&Class::Toolchains).unwrap(),
            "\"toolchains\""
        );
        assert_eq!(
            serde_json::from_str::<Class>("\"caches\"").unwrap(),
            Class::Caches
        );
    }
}
