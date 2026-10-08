//! Indented list fallback for terminals too narrow for the treemap.

use disktree_tui_core::layout::Weight;
use disktree_tui_core::tree::{DirNode, format_size};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::ViewSettings;
use crate::ui::Theme;

/// List `root`'s subtree, largest first, down to the draw depth, as many
/// rows as fit in `area`.
pub fn render(
    buf: &mut Buffer,
    area: Rect,
    root: &DirNode,
    settings: ViewSettings,
    theme: Theme,
    selected: Option<&DirNode>,
) {
    let max_depth = settings.depth.max_depth().unwrap_or(usize::MAX);
    let mut rows = Vec::new();
    collect(
        root,
        1,
        max_depth,
        settings.weight(),
        usize::from(area.height),
        &mut rows,
    );

    for (y, (depth, node)) in (area.y..area.bottom()).zip(rows) {
        let indent = "  ".repeat(depth - 1);
        let swatch = if node.reclaimable { "╱╱" } else { "██" };
        let weight = settings.weight();
        let size = match weight {
            Weight::Files => format!("{} files", node.file_count),
            _ => format_size(weight.of(node)),
        };
        let row_style = if selected.is_some_and(|s| std::ptr::eq(s, node)) {
            theme.selection()
        } else {
            Style::new()
        };
        let x = buf
            .set_stringn(
                area.x,
                y,
                format!(" {indent}"),
                usize::from(area.width),
                Style::new(),
            )
            .0;
        let x = buf
            .set_stringn(
                x,
                y,
                swatch,
                usize::from(area.right().saturating_sub(x)),
                theme.swatch(node.class),
            )
            .0;
        let rest = format!(" {size:>10}  {}/", node.name);
        buf.set_stringn(
            x,
            y,
            rest,
            usize::from(area.right().saturating_sub(x)),
            row_style,
        );
    }
}

/// Pre-order rows of `(depth, node)` below `node`, children ordered by weight.
fn collect<'a>(
    node: &'a DirNode,
    depth: usize,
    max_depth: usize,
    weight: Weight,
    limit: usize,
    rows: &mut Vec<(usize, &'a DirNode)>,
) {
    if depth > max_depth {
        return;
    }
    let mut children: Vec<&DirNode> = node
        .children
        .iter()
        .map(AsRef::as_ref)
        .filter(|c| weight.of(c) > 0)
        .collect();
    children.sort_by_key(|c| std::cmp::Reverse(weight.of(c)));
    for child in children {
        if rows.len() >= limit {
            return;
        }
        rows.push((depth, child));
        collect(child, depth + 1, max_depth, weight, limit, rows);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VisMode;
    use crate::ui::ColorDepth;
    use disktree_tui_core::layout::DrawDepth;

    fn tree() -> DirNode {
        let mut root = DirNode::new("/r".into(), "r".into(), 0);
        let mut a = DirNode::new("/r/a".into(), "a".into(), 1);
        let mut deep = DirNode::new("/r/a/deep".into(), "deep".into(), 2);
        deep.size = 700;
        let mut deeper = DirNode::new("/r/a/deep/deeper".into(), "deeper".into(), 3);
        deeper.size = 100;
        deep.absorb_child(deeper);
        a.absorb_child(deep);
        let mut b = DirNode::new("/r/b".into(), "b".into(), 1);
        b.size = 300;
        b.reclaimable = true;
        root.absorb_child(a);
        root.absorb_child(b);
        root.sort_recursive();
        root
    }

    fn render_text(depth: DrawDepth, height: u16) -> String {
        let root = tree();
        let area = Rect::new(0, 0, 40, height);
        let mut buf = Buffer::empty(area);
        let settings = ViewSettings {
            mode: VisMode::Size,
            apparent: false,
            show_hidden: true,
            depth,
        };
        render(
            &mut buf,
            area,
            &root,
            settings,
            Theme::new(ColorDepth::Ansi16),
            None,
        );
        (0..height)
            .map(|y| (0..40).map(|x| buf[(x, y)].symbol()).collect::<String>() + "\n")
            .collect()
    }

    #[test]
    fn lists_largest_first_down_to_draw_depth() {
        let text = render_text(DrawDepth::Levels(2), 10);
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(
            lines[0].contains("a/") && lines[0].contains("800 B"),
            "{text}"
        );
        assert!(
            lines[1].contains("  ██") && lines[1].contains("deep/"),
            "{text}"
        );
        assert!(lines[2].contains("╱╱") && lines[2].contains("b/"), "{text}");
        assert!(!text.contains("deeper"));
    }

    #[test]
    fn unlimited_depth_and_row_limit() {
        assert!(render_text(DrawDepth::Unlimited, 10).contains("deeper/"));
        let text = render_text(DrawDepth::Unlimited, 2);
        assert_eq!(text.lines().filter(|l| !l.trim().is_empty()).count(), 2);
    }
}
