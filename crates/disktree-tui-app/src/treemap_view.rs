//! Treemap rendering: paints laid-out tiles into a ratatui buffer.

use disktree_tui_core::classify::Class;
use disktree_tui_core::layout::{Rect as LayoutRect, Tile, TileKind};
use disktree_tui_core::tree::{DirNode, format_size};
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

use crate::ui::Theme;

/// Fill glyph for reclaimable tiles.
const HATCH: &str = "╱";

/// Thin line marking each tile's left edge.
const SEPARATOR: &str = "▏";

/// How a tile is painted.
struct Look {
    fill: Color,
    band: Color,
    hatch: Option<Color>,
    label: String,
    size: u64,
}

/// Paint `tiles` (in layout pre-order, so parents first) into `buf`.
pub fn render(buf: &mut Buffer, tiles: &[Tile<'_>], selected: Option<&DirNode>, theme: Theme) {
    for tile in tiles {
        let look = look(tile, theme);
        let r = tile.rect;
        fill(buf, r, &look, theme);

        let is_selected = matches!(
            (tile.kind, selected),
            (TileKind::Dir(node), Some(sel)) if std::ptr::eq(node, sel)
        );
        let text_style = Style::new().fg(theme.tile_text());
        let first_row = if is_selected {
            theme.selection()
        } else if tile.inner.is_some() {
            text_style.bg(look.band).add_modifier(Modifier::BOLD)
        } else {
            text_style.bg(look.fill)
        };
        if tile.inner.is_some() || is_selected {
            for x in r.x..r.right() {
                let cell = &mut buf[(x, r.y)];
                cell.set_symbol(" ");
                cell.set_style(first_row);
            }
        }
        label(
            buf,
            r,
            &look,
            first_row,
            text_style.bg(look.fill),
            tile.inner.is_some(),
        );
    }
}

fn look(tile: &Tile<'_>, theme: Theme) -> Look {
    let d = tile.depth;
    let (class, depth, reclaimable, label, size) = match tile.kind {
        TileKind::Dir(node) => (
            node.class,
            d,
            node.reclaimable,
            node.name.clone(),
            node.size,
        ),
        TileKind::Files { parent, weight } => (
            parent.class,
            d + 1,
            parent.reclaimable,
            "(files)".into(),
            weight,
        ),
        TileKind::Overflow {
            parent,
            dirs,
            includes_files,
            weight,
        } => {
            let label = match (dirs, includes_files) {
                (0, _) => "(files)".to_string(),
                (n, false) => format!("+{n} more"),
                (n, true) => format!("+{n} more, files"),
            };
            (Class::Unknown, d, parent.reclaimable, label, weight)
        }
    };
    Look {
        fill: theme.tile(class, depth),
        band: theme.band(class, depth),
        hatch: reclaimable.then(|| theme.hatch(class, depth)),
        label,
        size,
    }
}

fn fill(buf: &mut Buffer, r: LayoutRect, look: &Look, theme: Theme) {
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            let cell = &mut buf[(x, y)];
            cell.reset();
            cell.set_bg(look.fill);
            if x == r.x {
                cell.set_symbol(SEPARATOR);
                cell.set_fg(theme.separator());
            } else if let Some(hatch) = look.hatch {
                cell.set_symbol(HATCH);
                cell.set_fg(hatch);
            } else {
                cell.set_symbol(" ");
            }
        }
    }
}

/// Name on the first row; size on the second when there is room, or after
/// the name on a one-row tile.
fn label(
    buf: &mut Buffer,
    r: LayoutRect,
    look: &Look,
    first_row: Style,
    body: Style,
    subdivided: bool,
) {
    if r.width < 3 {
        return;
    }
    let room = usize::from(r.width - 1);
    let size = format_size(look.size);
    let one_line = format!("{}  {size}", look.label);
    let first = if (subdivided || r.height == 1) && one_line.chars().count() < room {
        one_line
    } else {
        look.label.clone()
    };
    buf.set_stringn(r.x + 1, r.y, &first, room, first_row);
    // A cut-off size ("46" of "46.1 MiB") misleads; show it whole or not at all.
    if !subdivided && r.height >= 2 && first == look.label && size.chars().count() <= room {
        buf.set_stringn(r.x + 1, r.y + 1, &size, room, body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::ColorDepth;
    use disktree_tui_core::layout::{self, LayoutOptions};
    use ratatui::layout::Rect;

    fn node(name: &str, size: u64, class: Class, children: Vec<DirNode>) -> DirNode {
        let mut n = DirNode::new(format!("/r/{name}").into(), name.into(), 1);
        n.class = class;
        for c in children {
            n.absorb_child(c);
        }
        n.size += size;
        n
    }

    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    fn text(buf: &Buffer) -> String {
        (0..buf.area.height).map(|y| row(buf, y) + "\n").collect()
    }

    fn paint(root: &DirNode, theme: Theme) -> (Buffer, Vec<Tile<'_>>) {
        let area = Rect::new(0, 0, 60, 16);
        let tiles = layout::layout(
            root,
            layout::Rect::new(0, 0, 60, 16),
            &LayoutOptions::default(),
        )
        .tiles;
        let mut buf = Buffer::empty(area);
        render(&mut buf, &tiles, None, theme);
        (buf, tiles)
    }

    #[test]
    fn tiles_show_names_and_sizes() {
        let root = node(
            "r",
            0,
            Class::Unknown,
            vec![
                node("docs", 6000, Class::Documents, vec![]),
                node("media", 4000, Class::Media, vec![]),
            ],
        );
        let (buf, _) = paint(&root, Theme::new(ColorDepth::TrueColor));
        let screen = text(&buf);
        assert!(screen.contains("docs"), "{screen}");
        assert!(screen.contains("media"), "{screen}");
        assert!(screen.contains("5.9 KiB"), "{screen}");
    }

    #[test]
    fn reclaimable_tiles_are_hatched_and_colored_by_class() {
        let mut cache = node("cache", 5000, Class::Caches, vec![]);
        cache.reclaimable = true;
        let root = node(
            "r",
            0,
            Class::Unknown,
            vec![cache, node("src", 5000, Class::Code, vec![])],
        );
        let theme = Theme::new(ColorDepth::TrueColor);
        let (buf, tiles) = paint(&root, theme);
        for t in &tiles {
            let TileKind::Dir(n) = t.kind else { continue };
            // A body cell away from the label and the separator.
            let cell = &buf[(t.rect.x + 2, t.rect.bottom() - 1)];
            assert_eq!(cell.bg, theme.tile(n.class, 1), "{}", n.name);
            let hatched = cell.symbol() == HATCH;
            assert_eq!(hatched, n.reclaimable, "{}", n.name);
        }
    }

    #[test]
    fn subdivided_tiles_get_a_label_band() {
        let inner = vec![
            node("a", 3000, Class::Code, vec![]),
            node("b", 3000, Class::Code, vec![]),
        ];
        let root = node(
            "r",
            0,
            Class::Unknown,
            vec![node("proj", 0, Class::Code, inner)],
        );
        let theme = Theme::new(ColorDepth::TrueColor);
        let (buf, tiles) = paint(&root, theme);
        let proj = tiles.iter().find(|t| t.inner.is_some()).unwrap();
        assert!(row(&buf, proj.rect.y).contains("proj  5.9 KiB"));
        assert_eq!(
            buf[(proj.rect.x + 1, proj.rect.y)].bg,
            theme.band(Class::Code, 1)
        );
        assert!(
            buf[(proj.rect.x + 1, proj.rect.y)]
                .modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn selected_tile_is_highlighted() {
        let root = node(
            "r",
            0,
            Class::Unknown,
            vec![
                node("a", 6000, Class::Code, vec![]),
                node("b", 4000, Class::Code, vec![]),
            ],
        );
        let theme = Theme::new(ColorDepth::TrueColor);
        let tiles = layout::layout(
            &root,
            layout::Rect::new(0, 0, 60, 16),
            &LayoutOptions::default(),
        )
        .tiles;
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 16));
        let selected = root
            .children
            .iter()
            .find(|c| c.name == "b")
            .map(AsRef::as_ref);
        render(&mut buf, &tiles, selected, theme);
        let b = tiles
            .iter()
            .find(|t| matches!(t.kind, TileKind::Dir(n) if n.name == "b"))
            .unwrap();
        assert_eq!(
            buf[(b.rect.right() - 1, b.rect.y)].bg,
            theme.selection().bg.unwrap()
        );
        let a = tiles
            .iter()
            .find(|t| matches!(t.kind, TileKind::Dir(n) if n.name == "a"))
            .unwrap();
        assert_ne!(
            buf[(a.rect.right() - 1, a.rect.y)].bg,
            theme.selection().bg.unwrap()
        );
    }

    #[test]
    fn sizes_are_never_truncated() {
        // Many siblings: some tiles are too narrow for "x.y KiB".
        let children = (0..30)
            .map(|i| node(&format!("d{i}"), 2000 + i * 300, Class::Code, vec![]))
            .collect();
        let root = node("r", 0, Class::Unknown, children);
        let (buf, tiles) = paint(&root, Theme::new(ColorDepth::TrueColor));
        for t in &tiles {
            if t.rect.height < 2 {
                continue;
            }
            let line: String = (t.rect.x + 1..t.rect.right())
                .map(|x| buf[(x, t.rect.y + 1)].symbol())
                .collect();
            let line = line.trim();
            assert!(
                line.is_empty() || line.ends_with("KiB"),
                "truncated size {line:?}"
            );
        }
    }

    #[test]
    fn mono_theme_still_hatches_and_labels() {
        let mut cache = node("cache", 5000, Class::Caches, vec![]);
        cache.reclaimable = true;
        let root = node("r", 0, Class::Unknown, vec![cache]);
        let (buf, _) = paint(&root, Theme::new(ColorDepth::Mono));
        let screen = text(&buf);
        assert!(screen.contains(HATCH));
        assert!(screen.contains("cache"));
    }
}
