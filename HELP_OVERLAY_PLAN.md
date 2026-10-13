# Plan & Checklist: `?` Help Overlay

Status: **implemented** · Owner: — · Last updated: 2026-10-13

> Shipped in `crates/disktree-tui-app/src/help.rs` (data + body) and
> `render_help` in `ui.rs`. Divergence from the original plan: the overlay is
> **not** any-key-to-close — it closes on `?` / `Esc` / `q` / `Q` / `F1` /
> `Enter`, and uses `↑`/`↓`/`j`/`k`/`PageUp`/`PageDown`/`Home`/`End` to scroll,
> so small terminals can page through the list.

## Goal

Pressing `?` shows a centered, bordered overlay listing every keybinding.
Any key (or `?` / `Esc`) dismisses it. It is a modal overlay like the review
screen: it captures the keyboard while open and never leaks keys to navigation,
marking, or scanning.

This closes the last remaining item under **M7 — Polish & Release** that the
README already advertises ("`?` | Show all keybindings").

## Design (grounded in current code)

The existing modal pattern is the template to follow:

- `Input` enum in `crates/disktree-tui-app/src/app.rs` (around L80) holds modal
  states: `None`, `Filter`, `Path`, `Volumes`, `Review`. Add a `Help` variant.
- `App::handle_key` (`app.rs` L566) dispatches on `self.input` first (L571–597),
  then handles global keys in `Input::None`. Add a `Help` arm and a `?` opener.
- `ui::render` (`crates/disktree-tui-app/src/ui.rs` L302) draws the body, then
  conditionally the volume picker (L399) and review popup (L402). Add a help
  popup at the same point.
- `render_review` (L754) and `render_confirm` (L802) show the popup recipe:
  build `Vec<Line>`, measure max width via `Line::width`, clamp with
  `.min(area.width)` / `.min(area.height)`, center, `frame.render_widget(Clear, …)`,
  then `Paragraph::new(lines).block(Block::bordered().title(...))`.

Decisions:

- **Modal type:** new `Input::Help` (captures keys, mirrors `Review`).
- **Open key:** `?` (crossterm reports `KeyCode::Char('?')` with SHIFT). Add `F1`
  as an optional alias.
- **Close:** any key press closes it. Rationale: matches the "press any key"
  convention and makes `?`, `Esc`, `q` all work without special cases. (Alternative:
  only `?`/`Esc`/`q` — decide during implementation; default is any-key.)
- **Scope:** available in `State::Ready`. Opening during `Scanning` is a stretch
  goal (see checklist 7).
- **Overflow:** if the terminal is too short for all groups, scroll with
  `Up`/`Down`/`PageUp`/`PageDown` and show a "↑/↓ more" hint. If we keep it
  simple, clamp/truncate — but scrolling is preferred for small terminals.
- **Source of truth:** one `const` list of `(group, key, description)` used by the
  renderer, so the overlay and any future docs stay consistent. The README table
  is updated manually to match.

## Keybindings to list (from README + `handle_key`)

| Group | Keys | Action |
|-------|------|--------|
| Navigate | `Enter` | Open selected directory |
| | `Esc` / `Backspace` | Go up one level |
| | `Alt+←` / `Alt+→` | Back / forward in history |
| | Arrows | Move selection between tiles |
| | `Tab` / `Shift+Tab` | Next / previous sibling by size |
| View | `T` | Cycle size → apparent → files → age |
| | `D` | Toggle disk usage / apparent size |
| | `I` | Toggle hidden files (rescans) |
| | `[` / `]` | Draw depth shallower / deeper |
| | `/` | Name filter |
| Scan | `R` / `F5` | Rescan |
| | `V` | Volume picker |
| | `Ctrl+O` | Choose scan directory |
| | `G` / `Ctrl+D` | Scan whole disk |
| Mark | `Space` / `X` | Mark / unmark (absorbing) |
| | `Ctrl+Space` | Mark without moving selection |
| | `C` | Review marked list |
| Review | `M` / `P` | Trash / permanent delete (then `y`/`n`) |
| | `!` / `S` / `A` | Unmark all / save paths / agent prompt |
| | `U` / `Esc` | Unmark row / leave review |
| Other | `?` | This help |
| | `Q` / `Ctrl+C` | Quit |

## Checklist

### 1. State & input plumbing
- [x] Add `Input::Help` variant to `Input` in `app.rs` (+ doc comment)
- [x] Add `Input::Help` arm to `handle_key` dispatch (`app.rs` ~L571)
- [x] In the arm, close on `?`/`Esc`/`q`/`Q`/`F1`/`Enter` and `return Action::Continue`
- [x] Add `KeyCode::Char('?')` opener in the `Input::None` block, before the
      `_ if self.tree().is_none()` guard (so it works whenever a tree exists)
- [x] `KeyCode::F(1)` alias for the same opener

### 2. Renderer
- [x] Add `render_help(frame, area, app)` in `ui.rs`, modeled on `render_review`
- [x] Build grouped `Vec<Line>` from the shared keybinding list
- [x] Title the block `" Keys "`; `Clear` beneath; center in `area`
- [x] Clamp width/height to the area (`.min(...)`)
- [x] Call `render_help(frame, body, app)` in the `State::Ready` branch of `render`
- [x] Scrolling when content exceeds height (`help_offset` + `help_viewport`)
- [x] Confirm `Input::Help` and `Input::Review` cannot both be active

### 3. Single source of truth
- [x] Define the `(group, key, description)` table once (`help::BINDINGS`)
- [x] Keep the README `## Keys` table consistent with the list

### 4. Tests
- [x] `app.rs`: `?` sets `self.input` to `Input::Help`
- [x] `app.rs`: opening help then pressing `x` keeps help open and does **not**
      mark (no key leakage)
- [x] `app.rs`: `Esc` while help is open closes help, does not go up
- [x] `ui.rs`: `draw(&app, …)` shows the overlay title and group headings, and
      the previous screen text is cleared behind the popup
- [x] `ui.rs`: tiny terminal (30×8) renders without panic and stays clamped
- [x] Scroll bounds test (`app.rs`, and a renderer clamp test in `ui.rs`)
- [x] `help.rs`: `row_count()` matches `lines().len()`, and every section shows
      its first binding (regression guard for the header/binding collapse bug)

### 5. Docs & tracking
- [x] Split the PLAN.md M7 item into this checklist and check it off
- [x] Verify README wording ("`?` | Show all keybindings") is accurate
- [x] Remove the README/code mismatch note once the overlay ships

### 6. Verification gate
- [x] `cargo test --workspace` passes (69 app + 17 cli + 92 core)
- [x] `cargo clippy --workspace -- -D warnings` is clean (pedantic)
- [x] `cargo fmt --check` passes
- [x] `make ci` green (same commands as above)
- [x] Manual check in a real terminal: `?` opens, keys scroll/close, no visual
      artifacts (verified via tmux, 100×40 and small sizes)

### 7. Stretch / optional
- [ ] Allow `?` during `State::Scanning`
- [ ] Show the current mode/state in the overlay header
- [ ] `README` ASCII screenshot of the overlay
- [x] Page the overlay (`PageUp`/`PageDown`/`Home`/`End`)

## Out of scope

- The config file (`~/.config/disktree-tui/config.toml`) — separate M7 workstream.
- Windows/macOS platform features and release packaging — separate M7 workstreams.

## Definition of done

`?` opens a centered overlay from the `Ready` screen listing all keybindings,
any key closes it cleanly with no key leakage, it renders safely on small
terminals, tests cover open/close/leak/render, and `make ci` is green.
