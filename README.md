# disktree-tui

A terminal-based disk usage explorer and cleanup tool. Find what fills your disk, mark what should go, and remove it — from anywhere you can run a terminal.

Inspired by [disktree](https://github.com/tobi/disktree), a treemap-based disk analyzer for the desktop. disktree-tui brings the same workflow to the terminal, making it accessible over SSH and on headless machines.

## Install

### From source

```sh
git clone https://github.com/mguffin68/disktree-tui
cd disktree-tui
make install     # ~/.local/bin/disktree-tui
make uninstall   # remove installed files
```

### From crates.io

```sh
cargo install disktree-tui
```

### On Arch (AUR)

```sh
yay -S disktree-tui-bin     # release binary
# or
yay -S disktree-tui         # build from source
```

## Use

```sh
disktree-tui             # scan $HOME
disktree-tui --disk      # the whole disk
disktree-tui ~/projects  # or any directory
disktree-tui --help      # all options
```

### `--scan-only`

For scripting and automation:

```sh
disktree-tui --scan-only ~/.cache   # prints summary, exits
disktree-tui --scan-only --apparent --no-hidden -D 3 /
```

## The interface

### The treemap

Directories are rendered as colored rectangles sized by their disk usage. Each rectangle's color indicates the data kind:

- **Code** (blue), **Caches** (yellow), **Build** (magenta), **Git** (green)
- **Media** (red), **Documents** (cyan), **Toolchains** (white), **Synced** (bright blue)
- **Agent** (bright green), **Unknown** (dim gray)

A diagonal hatch pattern marks reclaimable space — caches, build output, package stores, temporary files — independent of the color.

### Navigation

```
┌──────────────────────────────────────────────────────────┐
│  / > home > user > .cache              [size] [hidden]   │
├──────────────┬───────────────────────────────────────────┤
│  Legend      │  ┌───────────┬─────────────────────────┐  │
│  Code     █  │  │ ~/.cache  │ ████████████████████████│  │
│  Caches ║╡   │  │  12.4GB   │ ████████████████████████│  │
│  Build  ▓▓   │  │ caches    │ ████████████████████████│  │
│  ...         │  └───────────┴─────────────────────────┘  │
│              │  ┌────────┐  ┌───────────┐                │
│  Disk:       │  │ src/   │  │ Downloads │                │
│  Free: 128GB │  │ 8.2GB  │  │  6.1GB    │                │
│  After: 140G │  └────────┘  └───────────┘                │
│  Marks: 5GB  │                                           │
│  Review [C]  │                                           │
├──────────────────────────────────────────────────────────┤
│  ~/.cache  • 12.4 GB • 48 files • Last: 2024-10-01      │
└──────────────────────────────────────────────────────────┘
```

### Marking and removal

1. Point at a tile and press **Space** or **X** to mark it for removal.
2. Marked tiles turn red, including everything inside them.
3. Press **C** to open the review list.
4. Choose:
   - **M** — Move to trash (reversible)
   - **P** — Delete permanently (with confirmation)
   - **!** — Unmark all
   - **S** — Save paths to a text file
   - **A** — Copy a prompt for an AI agent to free the space

### Mode switching

Press **T** to cycle: **Size** → **Apparent** → **File count** → **Age**. In Age mode, color shows last write time instead of data kind.

## Keys

| Key | Action |
|-----|--------|
| `Enter` | Open selected directory |
| `Esc` / `Backspace` | Go up one level |
| `Arrow keys` | Move selection between tiles |
| `Tab` | Next largest sibling |
| `Space` / `X` | Mark / unmark |
| `C` | Review marked list |
| `T` | Cycle: size → apparent → files → age |
| `D` | Toggle disk usage / apparent size |
| `I` | Toggle hidden files |
| `R` / `F5` | Rescan |
| `/` | Name filter |
| `[` / `]` | Draw depth: shallower / deeper (default 2 levels, up to unlimited) |
| `V` | Volume picker |
| `Ctrl+O` | Choose scan directory |
| `G` / `--disk` | Scan whole disk |
| `?` | Show all keybindings |
| `Q` | Quit |

**Review screen:** `M` trash · `P` permanent · `!` unmark all · `S` save · `A` agent prompt · `Enter` commit · `Esc` back

## Configuration

Create `~/.config/disktree-tui/config.toml`:

```toml
mode = "size"           # "size", "files", "age"
apparent = false
hidden = true
depth = 2               # levels drawn; "all" = unlimited

[colors]
code = "blue"
caches = "yellow"
build = "magenta"
# ...
```

## What it measures

- **Disk usage** by default (`st_blocks × 512`, the same as `du`). Apparent size (`ls -l`) is one toggle away.
- **Hardlinks** counted once.
- **Hidden entries** included by default — `~/.cache` is often the biggest thing.
- **Symlinks** are not followed.

## Safety

disktree-tui refuses to remove:

- the filesystem root, the scanned root, or your home directory
- mount points or directories containing mount points
- system trees (`/usr`, `/etc`, `/nix/store`, `Program Files`, etc.)

Permanent deletion always shows a confirmation dialog. Trash removal is reversible until the trash is emptied. No file name is ever passed through a shell.

## Build from source

```sh
make run      # release build, scanning $HOME
make lint     # rustfmt --check, then clippy (every warning is an error)
make test     # unit + integration tests
make ci       # lint, then test
```

You need Rust 1.98 or newer. The toolchain is pinned in `rust-toolchain.toml`, so `rustup` will fetch the right version automatically.

## License

MIT
