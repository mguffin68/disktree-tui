# Development Plan: disktree-tui

## Milestone 0 — Foundation

**Goal:** Project scaffolding, CI, build system, and core data structures.

### Tasks

- [x] Initialize Cargo workspace with two crates: `disktree-tui-core`, `disktree-tui-app`
- [x] Set up `Cargo.toml`, `rust-toolchain.toml`, `rustfmt.toml`
- [x] Create `Makefile` with `run`, `test`, `lint`, `ci`, `release` targets
- [x] Configure clippy pedantic with documented allow exceptions
- [x] Set up GitHub Actions for CI (lint + test on Linux, macOS, Windows)
- [x] Define module structure for both crates (empty modules, compile to verify)
- [x] Implement CLI argument parsing with `clap` (`--disk`, `--apparent`, `--no-hidden`, `--depth`, `--mode`, `--scan-only`)
- [x] Write integration test that runs binary with various argument combinations

### Acceptance Criteria

- `make run` compiles and prints help text (no scanning yet).
- `make lint` and `make test` pass in CI.
- CLI arguments are parsed correctly and exit cleanly with `--scan-only`.

---

## Milestone 1 — Core Scanner

**Goal:** Scan a directory tree, compute sizes, handle hardlinks, produce an in-memory tree.

### Tasks

- [x] Implement `scan::Scanner` in `disktree-tui-core`:
  - [x] Recursive directory walking (own explicit-stack walker; `walkdir` stats mounts before filtering and its `filter_entry` + `same_file_system` combination dropped siblings)
  - [x] Disk usage aggregation (`st_blocks * 512` for regular files)
  - [x] Apparent size mode (file metadata length)
  - [x] Hardlink deduplication (track inodes, count once)
  - [x] Symlink detection (do not follow, skip broken; a link's own blocks count in disk-usage mode, matching `du`)
  - [x] Stay on the root's filesystem: skip nested mounts by path (Linux mountinfo) before any `stat`, then by device id
  - [x] Hidden file filtering (include by default, respect `--no-hidden`)
  - [x] Depth limiting (stop at `--depth` N)
  - [ ] Async scanning with `tokio` scope — scanner is synchronous; the app runs it via `spawn_blocking` (M4)
  - [ ] Cancellation via `Esc` (signal or atomic flag) — `Scanner::cancel_flag()`; `Esc` binding lands in M5
- [x] Implement `tree::DirNode` data structure:
  - [x] Stores size, file count, depth, name, children list
  - [x] Bottom-up aggregation (size computed when last child completes)
  - [x] Unreadable directory tracking (count as unknown, don't crash)
- [ ] Implement scan caching for widening (memoize results keyed by path) — deferred to M5 alongside the widening UI; key must include `apparent` + `skip_hidden`
- [x] Write unit tests:
  - [x] Simple directory → correct total size
  - [x] Hardlinked files → counted once
  - [x] Symlinks → not followed
  - [x] Hidden files → included by default, excluded with flag
  - [x] Depth limit → correct pruning (deeper nodes fold into their nearest kept ancestor; totals unchanged)
  - [x] Unreadable directory → counted, not crashed
  - [x] Large directory → completes within time budget

### Acceptance Criteria

- Scanner produces a correct `DirNode` tree for any directory.
- Hardlinks deduplicated. Symlinks skipped. Hidden files controlled by flag.
- `--scan-only` prints a text summary of the tree (sizes, file counts).
- All unit tests pass.

---

## Milestone 2 — Classification & Reclaimable Space

**Goal:** Classify directories by data kind and identify reclaimable space.

### Tasks

- [x] Implement classification in `disktree-tui-core` (`classify::classify_tree`, run by the scanner):
  - [x] Pattern matching on directory names (case-insensitive)
  - [x] Class enums: Code, Caches, Build, Git, Media, Documents, Toolchains, Synced, Agent, Unknown
  - [x] Shape-based classification (bare git repo, `target` beside `Cargo.toml`, `node_modules`, etc.) — scanner records marker files per directory (before hidden filtering); `CACHEDIR.TAG` honored; root context derived from its ancestors on disk
  - [ ] Configurable class names/colors via config file — `Class` is serde-(de)serializable (lowercase) so `[colors]` keys parse; colors belong to `ui::Theme` (M4) and the config file (M7)
- [x] Implement reclaimable detection (part of `classify_tree`; protected trees — container storage, snapshots — are never reclaimable):
  - [x] Detect caches (`~/.cache`, `__pycache__`, browser caches)
  - [x] Detect build output (`target/`, `build/`, `dist/`, `.dart_tool/`)
  - [x] Detect package stores (`pacman/pkg/`, `apt/archives/`, `pip/cache/`, `nuget/`)
  - [x] Detect sync history/old versions
  - [x] Detect temporary files (`/tmp`, `/var/tmp`, `Temp/`)
  - [x] Reclaimable is independent of class — a `target/` that is classified as Build is also reclaimable (`node_modules` beside `package.json` is Toolchains + reclaimable; generic `cache`/`tmp` names are not reclaimable inside Code)
- [x] Wire classification into `DirNode` (each node carries class + reclaimable flag)
- [x] Write unit tests:
  - [x] `~/.cache` → Caches + reclaimable
  - [x] `target/` beside `Cargo.toml` → Build + reclaimable
  - [x] `src/` → Code (not reclaimable)
  - [x] `Downloads/` → Documents (not reclaimable)
  - [x] Unknown directory → Unknown (not reclaimable)

### Acceptance Criteria

- Every scanned directory has a class and reclaimable flag.
- Classification matches expected patterns for common directories.
- `--scan-only` output includes class labels.
- All classification tests pass.

---

## Milestone 3 — Layout Algorithm

**Goal:** Implement the squarified treemap layout that maps the tree to rectangles.

### Tasks

- [x] Implement squarified layout in `disktree-tui-core` (`layout::layout`; squarifies in visual units since cells are ~2:1, snaps shared edges so tiles cover the area exactly):
  - [x] Input: a `DirNode` subtree, area, and options (weight = size or file count, depth limit, minimum tile size)
  - [x] Output: tiles in pre-order — directories, a directory's own files, and overflow — each with a rect; subdivided tiles carry a one-row label band and the inner area
  - [x] Recursive subdivision by rows or columns for best aspect ratio
  - [x] Handle terminal resize (recompute layout)
  - [x] Handle case where terminal is too small (< 40 columns or < 4 rows: `too_small`, no tiles; M4 falls back to the tree view)
- [x] Implement depth rendering: only render nodes up to `--depth` N
  - [x] Nodes beyond depth N aggregate into their depth-N directory tile, drawn whole; siblings too small to draw merge into one "N more" overflow tile that always stays visible
- [x] Write unit tests:
  - [x] Two equal children → two equal-width columns
  - [x] Three children with 50/25/25 split → correct proportions
  - [x] Single child → fills entire area
  - [x] Overflow handling → unrendered nodes aggregate correctly
  - [x] Aspect ratio reasonableness (no tile narrower than 2 cells)
  - [x] Resize: layout recomputes with new dimensions

### Acceptance Criteria

- Layout algorithm produces correct, reasonably-square rectangles.
- Layout handles edge cases (single node, equal children, very uneven sizes).
- Depth rendering works correctly.
- All layout tests pass.

---

## Milestone 4 — TUI Rendering (Display Only)

**Goal:** Render the treemap in the terminal using ratatui — no interaction yet.

### Tasks

- [ ] Implement `ui::App` state machine in `disktree-tui-app`:
  - [ ] States: Scanning, Ready, FilterInput, VolumePicker, Review, Error
  - [ ] Current state tracking, state transitions
- [ ] Implement `treemap_view::render` in `disktree-tui-app`:
  - [ ] Top bar: breadcrumb trail, mode indicators (size/files/age, hidden, apparent, depth)
  - [ ] Main area: ASCII treemap with ratatui Widgets
    - [ ] Unicode box-drawing borders between tiles
    - [ ] Filled tiles using `█` (U+2588)
    - [ ] Hatched reclaimable tiles using `╡` / `╢` overlays
    - [ ] Color tiles by class (detect 256-color vs true-color terminal)
    - [ ] Labels below or inside tiles when space permits
    - [ ] Selection highlight using reverse video or bright borders
  - [ ] Side panel (right): legend, disk info, marked list, actions
  - [ ] Bottom bar: selected tile info (path, size, files, last write)
  - [ ] Legend: class → color mapping + hatch meaning
- [ ] Implement tree view fallback (indented list) when terminal < 80 cols or treemap not feasible
- [ ] Implement `ui::Theme`:
  - [ ] Color palette per class (as specified in PRD)
  - [ ] 8-color fallback (mono + labels)
  - [ ] Theme config option
- [ ] Handle terminal resize (SIGWINCH → redraw)
- [ ] Write render validation tests (compare rendered frames to expected output)

### Acceptance Criteria

- Running `disktree-tui` shows a rendered treemap with correct colors and labels.
- Breadcrumb, top bar, side panel, and bottom bar all render.
- Terminal resize updates the layout.
- Falls back to tree view in narrow terminals.
- No user interaction yet — pure display.

---

## Milestone 5 — Navigation & Selection

**Goal:** Full keyboard navigation: walk into directories, back/forward, selection movement.

### Tasks

- [ ] Implement navigation in `app.rs`:
  - [ ] Enter directory (deep at any depth)
  - [ ] Go up one level (Esc/Backspace)
  - [ ] Back/forward history (`Alt+Left` / `Alt+Right`; `[` / `]` are depth control)
  - [ ] Move selection between tiles at current level (arrow keys)
  - [ ] Tab → next largest sibling
  - [ ] Selection tracking (which directory is "selected")
- [ ] Implement `filter::Filter`:
  - [ ] `/` enters filter mode
  - [ ] Type to match directory names
  - [ ] Only matched directories keep their class color
  - [ ] `Enter` applies filter (hides non-matches)
  - [ ] `Esc` clears filter
  - [ ] Filter preview showing match count
- [ ] Implement scan control:
  - [ ] `R` / `F5` — rescan current directory
  - [ ] `Esc` during scan — cancel scan
  - [ ] `Ctrl+O` — choose directory to scan (file picker, minimal)
  - [ ] `V` — volume picker (list mounted volumes, select one)
  - [ ] `Ctrl+D` / `G` — scan whole disk
  - [ ] `D` — toggle disk usage / apparent size
  - [ ] `I` — toggle hidden files
  - [ ] `[` / `]` — depth control: wire to `layout::DrawDepth::shallower` / `deeper` (default 2 levels, `--depth` sets the start)
  - [ ] `T` — cycle mode (size → apparent → file count → age)
- [ ] Implement `classify::age_color`:
  - [ ] Last write time → color gradient (this week = bright → older = muted)
  - [ ] Used in Age mode instead of class color
- [ ] Wire `--scan-only` mode: scan, print summary, exit (useful for scripting)
- [ ] Write integration tests for navigation sequences

### Acceptance Criteria

- Full keyboard navigation works: enter directories, go back, back/forward history.
- Selection moves correctly with arrow keys and Tab.
- Filter works: type pattern, see matches highlighted, apply/clear.
- All mode toggles work: size/apparent, hidden toggle, depth, mode cycling.
- Volume picker lists volumes, selecting one starts new scan.
- Rescan and cancel work.
- `--scan-only` outputs a text summary and exits.

---

## Milestone 6 — Marking & Review Screen

**Goal:** Mark directories for removal, review marked list, commit removal.

### Tasks

- [ ] Implement marking in `app.rs`:
  - [ ] `Space` / `X` — mark/unmark selected tile
  - [ ] Marked tiles display in danger color (red)
  - [ ] Marked tile + all descendants marked (absorption)
  - [ ] Cannot mark individual tiles inside a marked directory
  - [ ] Marking is reversible (press again to unmark)
  - [ ] `Ctrl+Click` or `Ctrl+Space` — mark without moving selection
- [ ] Implement side panel updates:
  - [ ] Marked list: paths with cumulative size
  - [ ] Free space now vs. free space after marks
  - [ ] "Worth a Look" section (largest reclaimable directories)
- [ ] Implement `review::ReviewScreen`:
  - [ ] `C` opens review screen
  - [ ] Lists all marked paths with sizes
  - [ ] Per-path: unmark option
  - [ ] Actions: `M` (trash), `P` (permanent delete), `!` (unmark all), `S` (save to file), `A` (copy agent prompt)
  - [ ] `Enter` commits, `Esc` goes back
- [ ] Implement removal logic in `disktree-tui-core::removal`:
  - [ ] Safety guards: refuse root, scanned root, home dir, mount points, system trees
  - [ ] Trash: cross-platform trash (trash-cli → gio → XDG → system trash → Recycle Bin)
  - [ ] Permanent delete: `rm -rf` equivalent, path verified (no shell injection)
  - [ ] Post-removal auto-rescan
  - [ ] Report free space gained
- [ ] Write removal safety guard tests:
  - [ ] Refuse to delete filesystem root
  - [ ] Refuse to delete scanned root
  - [ ] Refuse to delete home directory
  - [ ] Refuse to delete mount points
  - [ ] Refuse to delete system trees
  - [ ] Accept deleting files under scanned root (non-system)
  - [ ] Handle file names with special characters (spaces, quotes, newlines)

### Acceptance Criteria

- Mark/unmark works correctly with absorption logic.
- Review screen lists marked paths, allows unmarking individual items.
- Trash removal works (when trash available).
- Permanent delete shows confirmation dialog, never proceeds without confirmation.
- Post-removal rescan shows updated numbers.
- All safety guards prevent dangerous deletions.
- All removal tests pass.

---

## Milestone 7 — Polish & Release

**Goal:** Final touches, documentation, packaging, and first release.

### Tasks

- [ ] Implement configuration file (`~/.config/disktree-tui/config.toml`):
  - [ ] Default mode, apparent size, hidden toggle, depth, theme
  - [ ] Optional class color overrides
- [ ] Add `?` key binding: show all keybindings in a help overlay
- [ ] Implement Windows-specific features:
  - [ ] NTFS allocation-aware size reporting
  - [ ] MFT scanning for whole disk (admin mode)
  - [ ] Recycle Bin integration
  - [ ] Refuse system directories (C:\Windows, Program Files, etc.)
- [ ] Implement macOS-specific features:
  - [ ] APFS cloned-file awareness (count cloned files in full)
  - [ ] Time Machine local snapshots exclusion note
  - [ ] Full Disk Access guidance (error message if missing)
- [ ] Cross-platform trash tests pass
- [ ] Implement install script:
  - [ ] `make install` → copies binary to `~/.local/bin/disktree-tui`
  - [ ] Optional: desktop entry (for "Open with disktree-tui" in file managers)
  - [ ] `make uninstall` → removes installed files
- [ ] Write README.md:
  - [ ] Usage examples
  - [ ] Screenshot (ASCII art or terminal screenshot)
  - [ ] Installation instructions
  - [ ] Key bindings reference
  - [ ] Platform-specific notes
- [ ] Generate release tarballs:
  - [ ] `disktree-tui-<version>-x86_64-linux.tar.gz`
  - [ ] `disktree-tui-<version>-aarch64-linux.tar.gz`
  - [ ] `disktree-tui-<version>-aarch64-macos.tar.gz`
  - [ ] `disktree-tui-<version>-x86_64-macos.tar.gz`
  - [ ] `disktree-tui-<version>-x86_64-windows.tar.gz`
- [ ] Publish to crates.io
- [ ] Create AUR package (archified):
  - [ ] `disktree-tui` (build from source)
  - [ ] `disktree-tui-bin` (release binary)
- [ ] Tag first release, create GitHub release with changelog

### Acceptance Criteria

- All features from M0–M6 working on Linux, macOS, and Windows.
- Configuration file loads and applies defaults.
- `?` key shows help overlay.
- `make install` places binary correctly.
- README is complete and accurate.
- Release tarballs build and install correctly on each platform.
- CI passes on all platforms.
- Lint passes with clippy pedantic.

---

## Timeline Estimate

| Milestone | Estimated Effort | Dependencies |
|-----------|-----------------|--------------|
| M0 — Foundation | 1 week | None |
| M1 — Core Scanner | 2 weeks | M0 |
| M2 — Classification | 1 week | M1 |
| M3 — Layout | 1.5 weeks | M1 |
| M4 — TUI Rendering | 2 weeks | M1, M2, M3 |
| M5 — Navigation | 2 weeks | M4 |
| M6 — Marking & Review | 2.5 weeks | M4, M5 |
| M7 — Polish & Release | 2 weeks | M6 |

**Total: ~14 weeks (approximately 3.5 months)** for a single developer.

Parallelizable areas:
- M2 (classification) and M3 (layout) can run in parallel after M1.
- M6 (marking/removal) logic can be partially developed in parallel with M5 (navigation).
- Windows/macOS specifics in M7 can be developed on each platform as they become available.
