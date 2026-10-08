# Product Requirements Document: disktree-tui

## 1. Vision

A terminal-based disk usage explorer and cleanup tool inspired by [disktree](https://github.com/tobi/disktree). Where disktree uses a GPU-accelerated treemap mosaic in a desktop window, disktree-tui delivers the same core workflow — scan, visualize, mark, remove — entirely within a TUI, making it accessible on servers, SSH sessions, and headless machines.

**Core value proposition:** Find what fills your disk, mark it for removal, and act on it — from anywhere you can run a terminal.

## 2. Problem Statement

- Developers and sysadmins need to understand disk usage on remote servers where no GUI exists.
- Existing CLI tools (`du`, `ncdu`, `dutree`, `dust`) cover parts of this workflow but lack integration:
  - They show sizes but don't help prioritize cleanup.
  - They don't highlight reclaimable/cleanup-targetable space visually.
  - They don't provide a review-and-commit removal flow with safety guards.
  - They don't classify data by kind or show reclaimable-space hatching.
- No existing tool combines scanning, classification, treemap-like visualization, selective marking, and safe removal in a single terminal-native workflow.

## 3. Goals

- Provide a full disk analysis workflow in a TUI that works over SSH and on headless systems.
- Replicate disktree's core workflow: scan → visualize → mark → review → remove.
- Classify directories by data kind (code, caches, docs, media, etc.) with color-coded display.
- Highlight reclaimable space (caches, build output, package stores) visually.
- Provide safe removal with a review step, trash support, and permanent-delete confirmation.
- Support navigation: walk into directories, breadcrumb trail, back/forward history.
- Handle the same edge cases as disktree: hardlinks counted once, symlinks not followed, hidden files included by default, mount points refused for removal.
- Work on both Unix-like systems and Windows (PowerShell/CMD).

## 4. Non-Goals

- A web-based dashboard or API.
- Real-time monitoring or watching (no live updates during browsing).
- GUI (graphical) version — this is TUI only.
- Network-aware features beyond local filesystem scanning (no cloud storage, no remote scanning over NFS unless mounted locally).
- File content inspection or search inside files.
- Graphical charts or graphs beyond the ASCII treemap.

## 5. User Stories

### 5.1 Primary Workflow

| # | As a... | I want to... | So that... |
|---|---------|-------------|------------|
| 1 | sysadmin | Run `disktree-tui` and see an immediate scan of my home directory | I can start understanding what's using space without arguments |
| 2 | developer | Scan a specific directory or the whole disk | I can target my analysis scope |
| 3 | user | See directories rendered as a treemap-like layout colored by data kind | I can visually identify which categories dominate |
| 4 | user | See reclaimable space marked with a hatch pattern | I can spot things worth cleaning without reading sizes |
| 5 | user | Navigate into directories with arrow keys or Enter | I can drill down into what's large |
| 6 | user | Mark tiles with Space/X for removal | I can select what to clean up |
| 7 | user | Review my marked list before acting | I can catch mistakes before anything is deleted |
| 8 | user | Commit removal (trash or permanent delete) from the review screen | I can reclaim space in one confirmed action |
| 9 | user | Rescan after removal to see updated numbers | I can verify how much space I actually recovered |

### 5.2 Secondary Workflow

| # | As a... | I want to... | So that... |
|---|---------|-------------|------------|
| 10 | power user | Filter directories by name with `/` | I can quickly find specific paths |
| 11 | power user | Switch between disk usage, apparent size, file count, and age views | I can analyze different dimensions |
| 12 | power user | Toggle hidden files on/off | I can hide noise or expose config bloat |
| 13 | power user | Limit draw depth with `[/]` | I can keep the view focused or see the full tree |
| 14 | power user | See volume picker (`v`) to scan another mounted volume | I can check disk usage across mounts |
| 15 | power user | Get an agent prompt with `a` listing my marked paths | I can hand the cleanup to an AI agent for safe removal |
| 16 | power user | Save marked paths to a text file (`s`) | I can use the list outside disktree-tui |
| 17 | admin | See scan totals (size, files, unreadable count) | I can assess scan completeness at a glance |
| 18 | user | See breadcrumbs showing where I am in the tree | I never lose my place |
| 19 | user | Go back/forward through navigation history | I can explore without re-traversing |
| 20 | user | See "Worth a Look" suggestions in the side panel | I get tips on the biggest cleanup targets |

## 6. Functional Requirements

### 6.1 Scanning

- Scan a directory tree bottom-up, aggregating disk usage per directory.
- By default, scan `$HOME`; accept `--disk` for whole disk and positional args for arbitrary directories.
- Use `st_blocks * 512` for disk usage (same as `du`), with `--apparent` toggle for apparent size.
- Count hardlinks once (track inodes, deduplicate).
- Do not follow symlinks.
- Include hidden files/dirs by default; toggle with `--no-hidden` or `i` key.
- Skip pseudo-filesystems, network mounts, automount points.
- On the whole disk, refuse to scan system trees (`/usr`, `/etc`, `/nix/store`, `/gnu/store`, Homebrew prefix, Windows `Program Files`, `C:\Windows`, etc.).
- Unreadable directories are counted as "unreadable" in scan totals rather than crashing.
- Cache scan results for widening (e.g., going from `~` to `/` reuses the already-scanned subtree).
- Async scanning with cancellation (`Esc` to stop).

### 6.2 Classification

- Classify directories by name patterns and shapes (same heuristics as disktree):
  - Code (e.g., `src/`, `app/`, `lib/` beside a project file)
  - Caches (e.g., `.cache/`, `cache/`, `__pycache__/`)
  - Build output (e.g., `target/` beside `Cargo.toml`, `build/`, `dist/`)
  - Git (bare repos, `.git/`)
  - Media (e.g., `Videos/`, `Music/`, `images/`)
  - Documents (e.g., `Documents/`, `Downloads/`)
  - Toolchains (e.g., `.rustup/`, `.cargo/`, `node_modules/`, `.local/share/flatpak/`)
  - Synced files (e.g., `Dropbox/`, `Nextcloud/`, `Sync/`)
  - Agent scratch (e.g., `.aider-chat/`, `.claude/`)
  - Package stores (e.g., `pacman/pkg/`, `apt/archives/`, `pip/cache/`)
  - Unknown (unclassified)
- Each class gets a distinct, colorblind-friendly terminal color.
- Depth muffles color saturation (deeper = lighter).
- Reclaimable space gets a diagonal hatch pattern overlay, independent of class color.

### 6.3 Visualization

#### 6.3.1 ASCII Treemap

- Render directories as nested rectangles in the terminal using Unicode box-drawing characters.
- Each rectangle's area is proportional to the directory's size.
- Squarified layout algorithm (same as disktree) for reasonable aspect ratios.
- Color tiles by data kind class.
- Hatch reclaimable tiles with `╱` or `╲` patterns.
- Show directory names inside or below tiles when space permits.
- Top-level directories get a colored strip + name band; deeper levels get a slim label row.

#### 6.3.2 Tree View Alternative

- Provide a tree-view mode (toggleable) as an alternative to the treemap, showing indented directories with sizes and classification.
- This serves as a fallback when the terminal is too small for a meaningful treemap.

#### 6.3.3 Side Panel

- Display alongside the treemap:
  - **Selection info:** size, share of scan, file count, last write time.
  - **Worth a Look:** top cleanup targets that are plausible to remove.
  - **Marked list:** currently marked paths with cumulative size.
  - **Disk info:** current free space, free space after marks, volume name.

#### 6.3.4 Top Bar

- Breadcrumb trail showing current path from filesystem root.
- Mode indicators: Size / Files / Age, Hidden, Apparent, depth level.
- Back / forward buttons.

#### 6.3.5 Legend

- Show color-to-class mapping.
- Show hatch pattern meaning.

### 6.4 Navigation

| Action | Keys |
|--------|------|
| Open selected directory | `Enter` |
| Go up one level | `Esc`, `Backspace` |
| Back / forward | `Alt+Left` / `Alt+Right` |
| Move selection between tiles | `Arrow keys` |
| Next largest sibling | `Tab` |
| Depth control (default 2 levels; steps up to unlimited) | `[` / `]` |
| Name filter | `/` then type, `Enter` to filter, `Esc` to clear |
| Choose scan directory | `Ctrl+O` |
| Rescan | `R` / `F5` |
| Quit | `Q` |

### 6.5 Marking

| Action | Keys |
|--------|------|
| Mark / unmark tile | `Space`, `X` |
| Mark without moving selection | `Ctrl+Click` (mouse) |
| Marked tile style | Danger color (red) applied to tile and all children |
| Directory marking | Absorbs inner marks; inner tiles become unmarkable until directory is unmarked |
| Reversible | Pressing mark again unmarks |
| No double-counting | Same path cannot be counted twice in removal totals |

### 6.6 Removal

#### 6.6.1 Review Screen

Triggered with `C`. Shows all marked paths with sizes.

Actions:

| Action | Key |
|--------|-----|
| Move to trash | `M` |
| Delete permanently | `P` |
| Unmark all | `!` |
| Save paths to file | `S` |
| Copy agent prompt to clipboard | `A` |
| Commit removal | `Enter` |
| Go back | `Esc` |

#### 6.6.2 Safety Guards

- Only paths under the scanned root can be removed.
- Filesystem root, scanned root, and home directory are always refused.
- Mount points and directories containing mount points are refused.
- System trees are refused.
- Symlinks are unlinked, never followed.
- No shell interpretation: file names are passed literally (no `rm -rf` injection via filenames).
- Permanent deletion always shows a confirmation dialog naming what goes and how much is freed.
- Trash deletion commits directly (reversible until trash is emptied).

#### 6.6.3 Trash Implementation

- **Linux:** try `trash-put` (trash-cli), then `gio trash`, then built-in XDG trash compliance.
- **macOS:** system Trash (same as Finder).
- **Windows:** Recycle Bin via shell API.
- If no trash is available, fall back to permanent delete with confirmation.

#### 6.6.4 Post-Removal

- Auto-rescan after removal to show updated disk state.
- Display how much free space was actually gained.

### 6.7 Mode Switching

| Mode | Toggle Key | What changes |
|------|-----------|-------------|
| Size (disk usage) | `T` (cycles) | Tile area = disk usage; color = data kind |
| Apparent size | `D` | Tile area = file size (ls -l style) |
| File count | `T` (cycles) | Tile area = number of files; color = data kind |
| Age | `T` (cycles) | Tile area = size; color = last write time (this week → older) |

### 6.8 Hidden Files

| Toggle | Key | Effect |
|--------|-----|--------|
| Show hidden | Default (on) | `.` files/dirs visible |
| Hide hidden | `I` | `.` files/dirs hidden |

### 6.9 Volume Picker

Press `V` to list all mounted volumes with free space, sorted fullest-first. Selecting one resets the scan root. Pseudo-filesystems and duplicate btrfs/APFS mounts are filtered out.

### 6.10 Agent Integration

- `A` on the review screen: generates a prompt listing marked paths and asking an agent to free space, checking each path first (e.g., git work that exists nowhere else, a tool's own clean command).
- File names containing newlines are escaped in the prompt so they cannot be confused for other paths.
- `S` saves marked paths to a text file (one path per line).

## 7. Non-Functional Requirements

### 7.1 Performance

- Scan a 100K-file home directory in under 5 seconds on a typical SSD.
- Scan a 4M-file whole disk in under 30 seconds.
- Treemap re-render on navigation under 100ms.
- Async scanning: UI remains responsive during scan.

### 7.2 Terminal Compatibility

- Works in any ANSI-compatible terminal (256-color or true color).
- Uses Unicode box-drawing characters (U+2500 series).
- Gracefully degrades in 8-color terminals (fallback to mono + labels).
- Supports terminals of 80+ columns minimum; recommended 140+ for full treemap.
- Resizes dynamically on terminal resize events (SIGWINCH).

### 7.3 Platform Support

| Platform | Requirement |
|----------|-----------|
| Linux | Primary target. XDG trash, system trees, btrfs awareness. |
| macOS | System trash, APFS cloned-file awareness, Full Disk Access prompt guidance. |
| Windows | NTFS allocation awareness, admin-mode MFT scanning for whole disk, Recycle Bin. |

### 7.4 Cross-Platform Architecture

- Core scanning and tree logic in a platform-agnostic crate.
- Platform-specific modules for: trash, volume listing, MFT scanning (Windows), file metadata.
- TUI backend abstracted behind a trait so it could theoretically be swapped.

### 7.5 Dependencies

- Minimal runtime dependencies. Prefer stdlib + well-maintained crates.
- No network access during scanning or operation.

## 8. Proposed Tech Stack

| Layer | Choice | Rationale |
|-------|--------|-----------|
| Language | Rust | Performance, safety, FFI for Windows trash/MFT, same language as upstream disktree |
| TUI Framework | Ratatui (formerly cursive/rustty) | Most mature Rust TUI framework, Ascii rendering support, active community |
| Async Runtime | Tokio | Async file scanning, signal handling |
| Filesystem | `walkdir` + `std::fs` | Cross-platform directory walking, same approach as dust |
| Layout | Squarified treemap algorithm | Same as upstream disktree, proven approach |
| Color | 256-color / true-color detection | Fallback chain for terminal capabilities |
| Trash | `trash` crate | Cross-platform (Linux, macOS, Windows) trash API |
| Volume listing | `sysinfo` crate | Cross-platform volume/mount enumeration |
| CLI parsing | `clap` | Arguably the best Rust CLI parser, help generation, subcommands |
| Detection of terminal | `nu-ansi-term` or `terminal_size` | Detect capabilities, size |
| Configuration | `etc` or `config` | Optional config file (~/.config/disktree-tui/config.toml) |

## 9. Project Structure

```
disktree-tui/
├── Cargo.toml
├── README.md
├── Makefile
├── rust-toolchain.toml
├── rustfmt.toml
│
├── crates/
│   ├── disktree-tui-core/          # Core logic, no TUI
│   │   ├── src/
│   │   │   ├── scan.rs              # Directory scanning, disk usage, dedup
│   │   │   ├── tree.rs              # Directory tree data structure
│   │   │   ├── layout.rs            # Squarified treemap layout
│   │   │   ├── classify.rs          # Directory classification by kind
│   │   │   ├── removal.rs           # Removal logic, safety guards
│   │   │   ├── trash.rs             # Cross-platform trash
│   │   │   ├── volume.rs            # Volume enumeration
│   │   │   └── platform.rs          # Platform-specific code
│   │   └── tests/
│   │       ├── scan_tests.rs
│   │       ├── layout_tests.rs
│   │       └── removal_tests.rs
│   │
│   └── disktree-tui-app/           # TUI application
│       ├── src/
│       │   ├── main.rs              # Entry point, CLI parsing
│       │   ├── app.rs               # App state machine, event loop
│       │   ├── config.rs            # Config loading
│       │   ├── treemap_view.rs      # Treemap rendering with ratatui
│       │   ├── tree_view.rs         # Tree view rendering (fallback)
│       │   ├── panel.rs             # Side panel (selection, marks, disk)
│       │   ├── review.rs            # Review screen
│       │   ├── filter.rs            # Filter input
│       │   ├── keymap.rs            # Keyboard bindings
│       │   └── ui.rs                # Theme, colors, spacing
│       └── tests.rs                 # End-to-end TUI tests
│
├── packaging/
│   └── ...                          # Install scripts, desktop entry (optional)
├── assets/
│   └── ...                          # Icons, screenshots
├── scripts/
│   └── ...                          # Build helpers
└── xtask/                           # Build tasks (release packaging)
```

## 10. Key Bindings Reference

| Key | Action |
|-----|--------|
| `Enter` | Open selected directory |
| `Esc` / `Backspace` | Go up one level |
| `Alt+Left` / `Alt+Right` | Back / forward history |
| `Arrow keys` | Move selection between tiles |
| `Tab` | Next largest sibling |
| `Space` / `X` | Mark / unmark |
| `C` | Open review screen |
| `T` | Cycle: size → apparent → file count → age |
| `D` | Toggle disk usage / apparent size |
| `I` | Toggle hidden files |
| `R` / `F5` | Rescan |
| `/` | Name filter input |
| `[` / `]` | Depth control |
| `V` | Volume picker |
| `Ctrl+O` | Choose scan directory |
| `?` | Show all keybindings |
| `Q` | Quit |

**Review screen:**

| Key | Action |
|-----|--------|
| `M` | Move marked to trash |
| `P` | Delete permanently |
| `!` | Unmark all |
| `S` | Save paths to file |
| `A` | Copy agent prompt |
| `Enter` | Commit |
| `Esc` | Back to scan |

## 11. CLI Interface

```
disktree-tui [OPTIONS] [DIRECTORY]

Arguments:
  [DIRECTORY]    Directory to scan (default: $HOME)

Options:
  -d, --disk                Scan the disk containing the target
  -a, --apparent            Use apparent size instead of disk usage
  -H, --no-hidden           Skip hidden files and directories
  -D, --depth <LEVELS>      Draw depth, or "all" (default: 2; [ and ] change it)
  -m, --mode <MODE>         What to visualize: size|files|age (default: size)
  -v, --volume              Pick a volume to scan (alias for pressing V in TUI)
  -s, --scan-only           Scan and print summary, then exit (no TUI)
      --config <PATH>       Path to config file
      --version             Show version
  -h, --help                Show help
  -V, --verbose             Verbose output (diagnostic info on stderr)
```

## 12. Configuration File

`~/.config/disktree-tui/config.toml`:

```toml
# Default visualization mode
mode = "size"          # "size", "files", "age"

# Default measurement
apparent = false       # false = disk usage, true = apparent size

# Default hidden file behavior
hidden = true

# Default depth limit
depth = 2              # levels drawn; "all" = unlimited

# Color theme
theme = "auto"         # "auto", "dark", "light", "dim"

# Class colors (optional overrides)
[colors]
code = "blue"
caches = "yellow"
build = "magenta"
git = "green"
media = "red"
documents = "cyan"
toolchains = "white"
synced = "blue"
agent = "yellow"
unknown = "dark_gray"
```

## 13. Terminal Rendering Design

### 13.1 Screen Layout

```
┌─────────────────────────────────────────────────────────────────────────┐
│  / > home > user > .cache                    [size] [hidden] [depth:2] │
├─────────────────────────────────────────────────────────────────────────┤
│  Scan: 45.2 GB (312K files)   │  Legend: ╔════════════╗  │ Disk:       │
│                                  │  Code     │ ████████ │  Free: 128 GB │
│  ┌──────────┬─────────────────┐ │  Caches   │ ║╡╡╡╡╡╡╡ │  After: 140GB │
│  │ ~/.cache │ ███████████████ │ │  Build    │ ▓▓▓▓▓▓▓▓ │  │ Marked: 5GB │
│  │          │  ██████████████ │ │  Media    │ ░░░░░░░░ │  │             │
│  │  12.4GB │   █████████████ │ │  ...      │          │  │ Review...[C]│
│  │ caches   │    ████████████ │ └────────────┘         │             │
│  └──────────┴─────────────────┘                          └─────────────┘
│   ┌────────┐  ┌───────────┐
│   │ src/   │  │ Downloads │
│   │ 8.2GB  │  │  6.1GB    │
│   └────────┘  └───────────┘
├─────────────────────────────────────────────────────────────────────────┤
│  ~/.cache  • 12.4 GB • 48 files • Last: 2024-10-01                      │
└─────────────────────────────────────────────────────────────────────────┘
```

- Top bar: breadcrumb + mode indicators
- Main area: ASCII treemap with labeled tiles
- Side panel (right): legend, disk info, marked list, actions
- Bottom bar: selected tile info
- All rendering uses Unicode box-drawing + block characters for treemap tiles

### 13.2 Unicode Tile Characters

- Filled tiles: `█` (U+2588) for the tile body
- Hatched (reclaimable): `╡`, `╢`, `╖`, `╕` overlaid on tile body, or use half-block characters
- Borders between tiles: box-drawing chars (`─`, `│`, `┌`, `┐`, `└`, `┘`)
- Selection highlight: reverse video or bright border (`┏`, `┓`, `┗`, `┛`)
- Marked tiles: red background or red-tinted characters
- Labels: small text below or inside tiles when room permits

### 13.3 Color Palette (True Color / 256-Color)

| Class | Color (256) | Color (RGB) |
|-------|-------------|-------------|
| Code | 4 (Blue) | #0000AF |
| Caches | 11 (Yellow) | #AF8700 |
| Build | 5 (Magenta) | #870087 |
| Git | 2 (Green) | #008700 |
| Media | 9 (Red) | #870000 |
| Documents | 6 (Cyan) | #008787 |
| Toolchains | 7 (White) | #878787 |
| Synced | 12 (Bright Blue) | #005FFF |
| Agent | 3 (Bright Green) | #00AF5F |
| Unknown | 8 (Dark Gray) | #404040 |
| Selection | 3 (Bright Yellow) | #FFAF00 |
| Marked/Danger | 9 (Red) | #870000 |
| Hatch pattern | Darken by 1 level | - |

## 14. Error Handling

- **Permission denied:** Count as unreadable, show count in top bar, do not crash.
- **Broken symlinks:** Skip silently.
- **Corrupted directory:** Log to stderr, skip.
- **Removal failure:** Report which paths failed and why (permission, file in use, etc.), allow retry.
- **Terminal too small:** Fall back to tree view mode.
- **No trash available:** Inform user, offer permanent delete as alternative.

## 15. Testing Strategy

| Test type | Description |
|-----------|-------------|
| Unit tests | Scanner (sizes, hardlinks, hidden), classifier (pattern matching), removal (safety guards), layout (squarified algorithm correctness) |
| Integration tests | End-to-end scan + remove in a temp directory with known file structure |
| TUI tests | Screen rendering validation (terminal frames, key handling) |
| Cross-platform tests | Same tests on Linux, macOS, Windows CI |

## 16. Build & Development

```makefile
run        # Debug build, scan $HOME
release    # Release build
test       # Run all tests
lint       # rustfmt --check + clippy
ci         # lint + test
install    # Build + copy to ~/.local/bin/
uninstall  # Remove from ~/.local/
```

## 17. Deployment

- Single binary, no runtime dependencies.
- Install to `~/.local/bin/disktree-tui` (user-level, no root needed).
- System-wide: `sudo make install PREFIX=/usr/local`.
- Packaging: release tarballs per platform (`disktree-tui-<ver>-<arch>-<os>.tar.gz`).
- AUR package: `disktree-tui` (build from source), `disktree-tui-bin` (release binary).
- Cargo crates.io publish: `disktree-tui`.

## 18. Risk Assessment

| Risk | Mitigation |
|------|-----------|
| Terminal resize during rendering causes garbled output | Redraw on SIGWINCH, buffer rendering before write |
| Very large treemap exceeds terminal width | Adaptive tile sizing, fall back to tree view |
| Removal of wrong files despite safeguards | Review screen mandatory, confirmation dialog for permanent delete, path validation |
| Slow scan on huge directories | Async scanning, progress indicator, allow cancellation |
| Unicode rendering issues across terminals | Graceful fallback to ASCII box-drawing, 8-color mode |
| Windows admin mode requirement for whole disk | Detect, prompt user, offer "Restart as Admin" or fall back to walk |
| ratatui limitations for complex layouts | Use custom rendering, hybrid approach (treemap + tree fallback) |

## 19. Future Enhancements (Out of Scope for v1)

- Comparison mode: scan two directories side by side.
- Saved scans: persist scan results and reopen later.
- Plugins: user-defined classification rules.
- Daemon mode: background scanning with periodic updates.
- Notifications: system tray notifications on completion.
- Git-aware cleanup: detect untracked files, abandoned branches.
- SSH remote scanning: connect to a remote host and scan over SSH.
- Export reports: PDF, JSON, or HTML summary of scan results.
- Undo: track removals, allow undo within a session.

## 20. Success Metrics

- Scan a 100K-file home directory in < 5s.
- Zero crashes on permission errors or unusual filesystems.
- All safety guards pass in removal tests.
- Works in a 80-column terminal (with degraded rendering).
- Works in screen/tmux without issues.
- Lint passes with clippy pedantic at error level.
