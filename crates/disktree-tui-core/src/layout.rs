//! Squarified treemap layout.
//!
//! [`layout`] maps a directory's subtree onto a grid of terminal cells. Each
//! directory tile that is large enough and within the depth limit gets a
//! one-row label band and its children laid out below it. Children too small
//! to draw are merged into one overflow tile, and a directory's own files get
//! a tile of their own so areas stay proportional.

use crate::tree::DirNode;

/// Terminal cells are roughly twice as tall as they are wide. Squarifying in
/// visual units makes tiles look square rather than merely measure square.
const CELL_ASPECT: f64 = 2.0;

/// Narrowest area the treemap is drawn in; below this callers should fall
/// back to a list view.
pub const MIN_COLUMNS: u16 = 40;

/// Fewest rows the treemap is drawn in.
pub const MIN_ROWS: u16 = 4;

/// A rectangle in the treemap, expressed in grid units (terminal columns/rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    /// X offset in columns.
    pub x: u16,
    /// Y offset in rows.
    pub y: u16,
    /// Width in columns.
    pub width: u16,
    /// Height in rows.
    pub height: u16,
}

impl Rect {
    /// Create a rectangle.
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Number of cells covered.
    pub fn area(self) -> u32 {
        u32::from(self.width) * u32::from(self.height)
    }

    /// One past the last column.
    pub fn right(self) -> u16 {
        self.x + self.width
    }

    /// One past the last row.
    pub fn bottom(self) -> u16 {
        self.y + self.height
    }

    /// Whether the cell at (`x`, `y`) lies inside.
    pub fn contains(self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// How many nesting levels the treemap draws below the view root.
///
/// Defaults to [`DrawDepth::DEFAULT`] levels: deeper views of a real home
/// directory turn into a mosaic of unreadable tiles. `[` and `]` step it via
/// [`shallower`](Self::shallower) and [`deeper`](Self::deeper).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawDepth {
    /// Draw this many levels (at least 1).
    Levels(usize),
    /// Draw as deep as space allows.
    Unlimited,
}

impl DrawDepth {
    /// The starting depth.
    pub const DEFAULT: Self = Self::Levels(2);

    /// The depth limit to pass to [`LayoutOptions::max_depth`].
    pub fn max_depth(self) -> Option<usize> {
        match self {
            Self::Levels(n) => Some(n),
            Self::Unlimited => None,
        }
    }

    /// One level deeper. `tree_height` is the view root's
    /// [`DirNode::height`]: stepping past it becomes `Unlimited`, so the user
    /// never presses `]` without anything changing.
    #[must_use]
    pub fn deeper(self, tree_height: usize) -> Self {
        match self {
            Self::Levels(n) if n + 1 < tree_height => Self::Levels(n + 1),
            _ => Self::Unlimited,
        }
    }

    /// One level shallower, never below 1. From `Unlimited` it steps to one
    /// level above the deepest the tree has, so the change is visible.
    #[must_use]
    pub fn shallower(self, tree_height: usize) -> Self {
        match self {
            Self::Levels(n) => Self::Levels(n.saturating_sub(1).max(1)),
            Self::Unlimited => Self::Levels(tree_height.saturating_sub(1).max(1)),
        }
    }
}

impl Default for DrawDepth {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl std::fmt::Display for DrawDepth {
    /// `2`, or `∞` when unlimited (for the `[depth:…]` indicator).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Levels(n) => write!(f, "{n}"),
            Self::Unlimited => f.write_str("∞"),
        }
    }
}

impl std::str::FromStr for DrawDepth {
    type Err = String;

    /// A positive number of levels, or `all` for unlimited.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "all" | "unlimited" | "inf" => Ok(Self::Unlimited),
            n => match n.parse::<usize>() {
                Ok(0) | Err(_) => Err(format!(
                    "invalid depth: {s} (expected a positive number or \"all\")"
                )),
                Ok(levels) => Ok(Self::Levels(levels)),
            },
        }
    }
}

/// What tile area is proportional to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Weight {
    /// Bytes (disk usage or apparent, whichever the scan measured).
    #[default]
    Size,
    /// Number of files.
    Files,
}

impl Weight {
    fn of(self, node: &DirNode) -> u64 {
        match self {
            Self::Size => node.size,
            Self::Files => node.file_count as u64,
        }
    }
}

/// Layout settings.
#[derive(Debug, Clone, Copy)]
pub struct LayoutOptions {
    /// What tile area is proportional to.
    pub weight: Weight,
    /// Nesting levels to draw below the view root (None = as deep as fits).
    /// Defaults to [`DrawDepth::DEFAULT`].
    /// Directories at the last level are drawn whole, their contents
    /// aggregated into the one tile.
    pub max_depth: Option<usize>,
    /// Narrowest tile, in columns.
    pub min_width: u16,
    /// Shortest tile, in rows.
    pub min_height: u16,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            weight: Weight::Size,
            max_depth: DrawDepth::DEFAULT.max_depth(),
            min_width: 2,
            min_height: 1,
        }
    }
}

/// What a tile stands for.
#[derive(Debug, Clone, Copy)]
pub enum TileKind<'a> {
    /// A directory (with everything below it, unless subdivided).
    Dir(&'a DirNode),
    /// Files directly inside `parent`, not in any subdirectory.
    Files {
        /// The directory holding the files.
        parent: &'a DirNode,
        /// Their combined weight.
        weight: u64,
    },
    /// Children of `parent` too small to draw individually.
    Overflow {
        /// The directory they belong to.
        parent: &'a DirNode,
        /// How many subdirectories were merged.
        dirs: usize,
        /// Whether `parent`'s own files were merged in too.
        includes_files: bool,
        /// Their combined weight.
        weight: u64,
    },
}

/// A tile assignment: what it shows and where.
#[derive(Debug, Clone, Copy)]
pub struct Tile<'a> {
    /// What the tile stands for.
    pub kind: TileKind<'a>,
    /// The tile's cells, including its label band if subdivided.
    pub rect: Rect,
    /// Nesting level: 1 for children of the view root.
    pub depth: usize,
    /// When subdivided, the area below the label band holding the children.
    /// The label band is the tile's first row.
    pub inner: Option<Rect>,
}

/// A computed treemap.
#[derive(Debug, Clone, Default)]
pub struct Layout<'a> {
    /// Tiles in pre-order: every subdivided tile precedes its children, so
    /// painting in order draws parents underneath.
    pub tiles: Vec<Tile<'a>>,
    /// The area was below [`MIN_COLUMNS`] x [`MIN_ROWS`]; no tiles were laid out.
    pub too_small: bool,
}

/// Lay out `root`'s contents in `area`.
///
/// The root itself is not a tile (it is the breadcrumb); its children fill the
/// area. The result is a pure function of its inputs, so a resize is just
/// another call with the new area.
pub fn layout<'a>(root: &'a DirNode, area: Rect, options: &LayoutOptions) -> Layout<'a> {
    if area.width < MIN_COLUMNS || area.height < MIN_ROWS {
        return Layout {
            tiles: Vec::new(),
            too_small: true,
        };
    }
    let mut tiles = Vec::new();
    layout_children(root, area, 1, options, &mut tiles);
    Layout {
        tiles,
        too_small: false,
    }
}

/// One candidate tile among a directory's contents.
#[derive(Debug, Clone, Copy)]
enum Item<'a> {
    Dir(&'a DirNode),
    Files,
    Overflow { dirs: usize, includes_files: bool },
}

fn layout_children<'a>(
    parent: &'a DirNode,
    area: Rect,
    depth: usize,
    options: &LayoutOptions,
    tiles: &mut Vec<Tile<'a>>,
) {
    let weight = options.weight;
    let mut items: Vec<(Item<'a>, u64)> = parent
        .children
        .iter()
        .map(|c| (Item::Dir(c.as_ref()), weight.of(c)))
        .filter(|&(_, w)| w > 0)
        .collect();
    let child_total: u64 = items.iter().map(|&(_, w)| w).sum();
    let own_files = weight.of(parent).saturating_sub(child_total);
    if own_files > 0 {
        items.push((Item::Files, own_files));
    }
    if items.is_empty() {
        return;
    }
    // Largest first (ties by name), as squarifying expects.
    items.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| item_name(a.0).cmp(item_name(b.0)))
    });

    let placed = fit(&mut items, area, options);
    for ((item, w), rect) in items.into_iter().zip(placed) {
        let kind = match item {
            Item::Dir(node) => TileKind::Dir(node),
            Item::Files => TileKind::Files { parent, weight: w },
            Item::Overflow {
                dirs,
                includes_files,
            } => TileKind::Overflow {
                parent,
                dirs,
                includes_files,
                weight: w,
            },
        };
        let index = tiles.len();
        tiles.push(Tile {
            kind,
            rect,
            depth,
            inner: None,
        });
        if let Item::Dir(node) = item
            && options.max_depth.is_none_or(|max| depth < max)
            // Subdividing into nothing but its own files would only repeat it.
            && node.children.iter().any(|c| weight.of(c) > 0)
            && rect.height > options.min_height
            && rect.width >= options.min_width * 2
        {
            let inner = Rect::new(rect.x, rect.y + 1, rect.width, rect.height - 1);
            tiles[index].inner = Some(inner);
            layout_children(node, inner, depth + 1, options, tiles);
            // If no subdirectory earned a tile of its own (only the files
            // tile and an overflow), the split adds nothing: draw it whole.
            let any_dir = tiles[index + 1..]
                .iter()
                .any(|t| t.depth == depth + 1 && matches!(t.kind, TileKind::Dir(_)));
            if !any_dir {
                tiles.truncate(index + 1);
                tiles[index].inner = None;
            }
        }
    }
}

fn item_name(item: Item<'_>) -> &str {
    match item {
        Item::Dir(node) => &node.name,
        Item::Files | Item::Overflow { .. } => "",
    }
}

/// Squarify `items` into `area`, merging the smallest into an overflow item
/// until every tile meets the minimum size. Returns one rect per remaining
/// item, in order; `items` is updated to match.
fn fit(items: &mut Vec<(Item<'_>, u64)>, area: Rect, options: &LayoutOptions) -> Vec<Rect> {
    let total: u64 = items.iter().map(|&(_, w)| w).sum();
    let min_area = f64::from(options.min_width) * f64::from(options.min_height);
    let cells = f64::from(area.width) * f64::from(area.height);

    // Up front, merge everything whose share of the area can't hold a
    // minimum tile; the loop below handles shapes that still come out thin.
    let keep = items
        .iter()
        .take_while(|&&(_, w)| w as f64 / total as f64 * cells >= min_area * 2.0)
        .count()
        .max(1);
    merge_tail(items, keep);

    // The overflow tile must stay visible however little it weighs. Its
    // layout weight starts at a couple of minimum tiles and doubles while it
    // alone comes out too thin; its real weight is still reported on the tile.
    let mut overflow_floor = total as f64 * (min_area * 2.0 / cells);
    loop {
        let weights: Vec<f64> = items
            .iter()
            .map(|&(item, w)| match item {
                Item::Overflow { .. } => (w as f64).max(overflow_floor),
                _ => w as f64,
            })
            .collect();
        let rects = squarify(&weights, area);
        let too_small = |r: &Rect| r.width < options.min_width || r.height < options.min_height;
        let failing: Vec<bool> = rects.iter().map(too_small).collect();
        if !failing.contains(&true) || items.len() == 1 {
            return rects;
        }
        let only_overflow_fails = items
            .iter()
            .zip(&failing)
            .all(|((item, _), &bad)| !bad || matches!(item, Item::Overflow { .. }));
        if only_overflow_fails && overflow_floor * 2.0 < total as f64 / 2.0 {
            overflow_floor *= 2.0;
            continue;
        }
        // Fold the smallest individual item into the overflow and retry.
        let individual = items
            .iter()
            .filter(|(item, _)| !matches!(item, Item::Overflow { .. }))
            .count();
        merge_tail(items, individual - 1);
    }
}

/// Keep the first `keep` items that aren't overflow; merge the rest (and any
/// existing overflow) into a single overflow item at the end.
fn merge_tail(items: &mut Vec<(Item<'_>, u64)>, keep: usize) {
    let mut dirs = 0;
    let mut includes_files = false;
    let mut weight = 0;
    let mut kept = Vec::with_capacity(keep + 1);
    let mut individual = 0;
    for (item, w) in items.drain(..) {
        match item {
            Item::Overflow {
                dirs: d,
                includes_files: f,
            } => {
                dirs += d;
                includes_files |= f;
                weight += w;
            }
            _ if individual < keep => {
                individual += 1;
                kept.push((item, w));
            }
            Item::Dir(_) => {
                dirs += 1;
                weight += w;
            }
            Item::Files => {
                includes_files = true;
                weight += w;
            }
        }
    }
    if weight > 0 {
        // Always last: squarify places it in the final strip, where growing
        // its floored layout weight widens it without reordering anything.
        kept.push((
            Item::Overflow {
                dirs,
                includes_files,
            },
            weight,
        ));
    }
    *items = kept;
}

/// Squarified layout of `weights` (positive, roughly descending) in `area`.
///
/// Works on tile edges rather than sizes: adjacent tiles share the same
/// floating-point edge, so rounding each edge once tiles the area exactly,
/// with no gaps or overlaps. Tiles may round to zero width or height.
fn squarify(weights: &[f64], area: Rect) -> Vec<Rect> {
    let total: f64 = weights.iter().sum();
    // Visual units: one cell is CELL_ASPECT units tall.
    let (mut x0, mut y0) = (0.0_f64, 0.0_f64);
    let (x_end, y_end) = (f64::from(area.width), f64::from(area.height) * CELL_ASPECT);
    let scale = (x_end * y_end) / total;
    let areas: Vec<f64> = weights.iter().map(|w| w * scale).collect();

    let mut edges = Vec::with_capacity(weights.len());
    let mut i = 0;
    while i < areas.len() {
        let (w, h) = (x_end - x0, y_end - y0);
        let side = w.min(h);

        // Grow the row while it makes the worst aspect ratio no worse.
        let mut j = i + 1;
        let mut sum = areas[i];
        let mut worst = worst_ratio(areas[i], areas[i], sum, side);
        while j < areas.len() {
            let next = worst_ratio(areas[i], areas[j], sum + areas[j], side);
            if next > worst {
                break;
            }
            worst = next;
            sum += areas[j];
            j += 1;
        }

        let last_row = j == areas.len();
        if w >= h {
            // Column along the left edge.
            let x1 = if last_row { x_end } else { x0 + sum / h };
            let mut ya = y0;
            let row = &areas[i..j];
            for (k, a) in row.iter().enumerate() {
                let yb = if k + 1 == row.len() {
                    y_end
                } else {
                    ya + a / (x1 - x0)
                };
                edges.push((x0, ya, x1, yb));
                ya = yb;
            }
            x0 = x1;
        } else {
            // Row along the top edge.
            let y1 = if last_row { y_end } else { y0 + sum / w };
            let mut xa = x0;
            let row = &areas[i..j];
            for (k, a) in row.iter().enumerate() {
                let xb = if k + 1 == row.len() {
                    x_end
                } else {
                    xa + a / (y1 - y0)
                };
                edges.push((xa, y0, xb, y1));
                xa = xb;
            }
            y0 = y1;
        }
        i = j;
    }

    edges
        .into_iter()
        .map(|(xa, ya, xb, yb)| {
            let (cx0, cx1) = (snap(xa), snap(xb));
            let (cy0, cy1) = (snap(ya / CELL_ASPECT), snap(yb / CELL_ASPECT));
            Rect::new(area.x + cx0, area.y + cy0, cx1 - cx0, cy1 - cy0)
        })
        .collect()
}

/// Worst aspect ratio in a row of total area `sum` laid along `side`, given
/// its largest and smallest member areas.
fn worst_ratio(largest: f64, smallest: f64, sum: f64, side: f64) -> f64 {
    let side2 = side * side;
    let sum2 = sum * sum;
    (side2 * largest / sum2).max(sum2 / (side2 * smallest))
}

fn snap(v: f64) -> u16 {
    v.round() as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 0, 80, 24);

    fn node(name: &str, size: u64, children: Vec<DirNode>) -> DirNode {
        let mut n = DirNode::new(format!("/{name}").into(), name.into(), 0);
        for c in children {
            n.absorb_child(c);
        }
        n.size += size;
        n.file_count += 1;
        n
    }

    fn leaf(name: &str, size: u64) -> DirNode {
        node(name, size, Vec::new())
    }

    fn top_level<'a>(l: &'a Layout<'a>) -> Vec<&'a Tile<'a>> {
        l.tiles.iter().filter(|t| t.depth == 1).collect()
    }

    fn name(tile: &Tile<'_>) -> String {
        match tile.kind {
            TileKind::Dir(n) => n.name.clone(),
            TileKind::Files { .. } => "<files>".into(),
            TileKind::Overflow { dirs, .. } => format!("<{dirs} more>"),
        }
    }

    /// Paint tiles of one level onto a grid: every cell of `area` covered
    /// exactly once.
    fn assert_exact_cover(tiles: &[&Tile<'_>], area: Rect) {
        let mut grid = vec![0u8; area.area() as usize];
        for t in tiles {
            for y in t.rect.y..t.rect.bottom() {
                for x in t.rect.x..t.rect.right() {
                    assert!(area.contains(x, y), "{:?} outside {area:?}", t.rect);
                    let i =
                        usize::from(y - area.y) * usize::from(area.width) + usize::from(x - area.x);
                    grid[i] += 1;
                }
            }
        }
        assert!(grid.iter().all(|&c| c == 1), "gaps or overlaps");
    }

    #[test]
    fn empty_input_yields_no_tiles() {
        let root = leaf("r", 0);
        assert!(
            layout(&root, AREA, &LayoutOptions::default())
                .tiles
                .is_empty()
        );
    }

    #[test]
    fn single_child_fills_area() {
        let root = node("r", 0, vec![leaf("a", 100)]);
        let l = layout(&root, AREA, &LayoutOptions::default());
        assert_eq!(l.tiles.len(), 1);
        assert_eq!(l.tiles[0].rect, AREA);
    }

    #[test]
    fn two_equal_children_split_into_equal_columns() {
        let root = node("r", 0, vec![leaf("a", 100), leaf("b", 100)]);
        let l = layout(&root, AREA, &LayoutOptions::default());
        let tiles = top_level(&l);
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].rect, Rect::new(0, 0, 40, 24));
        assert_eq!(tiles[1].rect, Rect::new(40, 0, 40, 24));
    }

    #[test]
    fn fifty_twenty_five_twenty_five_split() {
        let root = node("r", 0, vec![leaf("a", 50), leaf("b", 25), leaf("c", 25)]);
        let l = layout(&root, AREA, &LayoutOptions::default());
        let tiles = top_level(&l);
        assert_exact_cover(&tiles, AREA);
        let total = f64::from(AREA.area());
        for (t, share) in tiles.iter().zip([0.5, 0.25, 0.25]) {
            let got = f64::from(t.rect.area()) / total;
            assert!((got - share).abs() < 0.03, "{}: {got} vs {share}", name(t));
        }
        assert_eq!(name(tiles[0]), "a");
    }

    #[test]
    fn tiles_exactly_cover_the_area() {
        let sizes = [500, 300, 200, 120, 80, 60, 40, 30, 20, 10, 7, 5, 3];
        let children = sizes
            .iter()
            .enumerate()
            .map(|(i, &s)| leaf(&format!("d{i}"), s))
            .collect();
        let root = node("r", 0, children);
        for area in [AREA, Rect::new(5, 3, 120, 40), Rect::new(0, 0, 40, 4)] {
            let l = layout(&root, area, &LayoutOptions::default());
            assert_exact_cover(&top_level(&l), area);
        }
    }

    #[test]
    fn tiles_are_reasonably_square_and_never_too_thin() {
        let children = (1..=30).map(|i| leaf(&format!("d{i}"), i * i)).collect();
        let root = node("r", 0, children);
        let l = layout(&root, AREA, &LayoutOptions::default());
        for t in top_level(&l) {
            assert!(t.rect.width >= 2, "{} too narrow: {:?}", name(t), t.rect);
            assert!(t.rect.height >= 1, "{} too short: {:?}", name(t), t.rect);
        }
        // The largest tiles should look roughly square (visual aspect < 4:1).
        for t in top_level(&l).iter().take(5) {
            let (w, h) = (
                f64::from(t.rect.width),
                f64::from(t.rect.height) * CELL_ASPECT,
            );
            assert!(
                w.max(h) / w.min(h) < 4.0,
                "{} skewed: {:?}",
                name(t),
                t.rect
            );
        }
    }

    #[test]
    fn tiny_children_aggregate_into_overflow() {
        let mut children = vec![leaf("big", 1_000_000)];
        children.extend((0..50).map(|i| leaf(&format!("tiny{i}"), 1)));
        let root = node("r", 0, children);
        let l = layout(&root, AREA, &LayoutOptions::default());
        let tiles = top_level(&l);
        assert_exact_cover(&tiles, AREA);
        let overflow: Vec<_> = tiles
            .iter()
            .filter_map(|t| match t.kind {
                TileKind::Overflow { dirs, weight, .. } => Some((dirs, weight)),
                _ => None,
            })
            .collect();
        assert_eq!(overflow, [(50, 50)]);
        assert_eq!(name(tiles[0]), "big");
    }

    #[test]
    fn overflow_weight_conserves_total() {
        let children = (0..200).map(|i| leaf(&format!("d{i}"), 1000 + i)).collect();
        let root = node("r", 0, children);
        let l = layout(&root, AREA, &LayoutOptions::default());
        let total: u64 = top_level(&l)
            .iter()
            .map(|t| match t.kind {
                TileKind::Dir(n) => n.size,
                TileKind::Files { weight, .. } | TileKind::Overflow { weight, .. } => weight,
            })
            .sum();
        assert_eq!(total, root.size);
        assert_exact_cover(&top_level(&l), AREA);
    }

    #[test]
    fn own_files_get_their_own_tile() {
        let root = node("r", 60, vec![leaf("a", 40)]);
        let l = layout(&root, AREA, &LayoutOptions::default());
        let names: Vec<_> = top_level(&l).iter().map(|t| name(t)).collect();
        assert_eq!(names, ["<files>", "a"]);
    }

    #[test]
    fn nested_directories_get_a_label_band_and_children() {
        let root = node(
            "r",
            0,
            vec![
                node("a", 0, vec![leaf("x", 30), leaf("y", 30)]),
                leaf("b", 60),
            ],
        );
        let l = layout(&root, AREA, &LayoutOptions::default());
        let a = l.tiles.iter().find(|t| name(t) == "a").unwrap();
        let inner = a.inner.expect("a is subdivided");
        assert_eq!(
            inner,
            Rect::new(a.rect.x, a.rect.y + 1, a.rect.width, a.rect.height - 1)
        );
        let kids: Vec<_> = l.tiles.iter().filter(|t| t.depth == 2).collect();
        assert_eq!(kids.len(), 2);
        assert_exact_cover(&kids, inner);
        // Pre-order: the parent precedes its children.
        let a_index = l.tiles.iter().position(|t| name(t) == "a").unwrap();
        let x_index = l.tiles.iter().position(|t| name(t) == "x").unwrap();
        assert!(a_index < x_index);
    }

    #[test]
    fn directory_without_weighted_subdirs_is_not_subdivided() {
        let root = node(
            "r",
            0,
            vec![node("a", 500, vec![leaf("empty", 0)]), leaf("b", 400)],
        );
        let l = layout(&root, AREA, &LayoutOptions::default());
        let a = l.tiles.iter().find(|t| name(t) == "a").unwrap();
        assert!(a.inner.is_none());
        assert_eq!(l.tiles.len(), 2);
    }

    #[test]
    fn subdivision_without_any_drawable_subdir_is_undone() {
        // `a` is almost all loose files; its subdirs are too small to draw.
        let tiny = (0..20).map(|i| leaf(&format!("t{i}"), 1)).collect();
        let root = node("r", 0, vec![node("a", 1_000_000, tiny), leaf("b", 900_000)]);
        let l = layout(&root, AREA, &LayoutOptions::default());
        let a = l.tiles.iter().find(|t| name(t) == "a").unwrap();
        assert!(a.inner.is_none());
        assert_eq!(l.tiles.len(), 2);
    }

    #[test]
    fn depth_limit_stops_subdivision() {
        let deep = node(
            "a",
            0,
            vec![node("b", 0, vec![leaf("c", 10), leaf("d", 10)])],
        );
        let root = node("r", 0, vec![deep]);
        let options = LayoutOptions {
            max_depth: Some(1),
            ..LayoutOptions::default()
        };
        let l = layout(&root, AREA, &options);
        assert_eq!(l.tiles.len(), 1);
        assert!(l.tiles[0].inner.is_none());
        assert_eq!(name(&l.tiles[0]), "a");

        let options = LayoutOptions {
            max_depth: Some(2),
            ..LayoutOptions::default()
        };
        let l = layout(&root, AREA, &options);
        assert!(l.tiles.iter().all(|t| t.depth <= 2));
        assert_eq!(l.tiles.iter().filter(|t| t.depth == 2).count(), 1);
    }

    #[test]
    fn default_draw_depth_is_two_levels() {
        assert_eq!(DrawDepth::default(), DrawDepth::Levels(2));
        assert_eq!(LayoutOptions::default().max_depth, Some(2));

        let deep = node(
            "a",
            0,
            vec![node("b", 0, vec![leaf("c", 10), leaf("d", 10)])],
        );
        let root = node("r", 0, vec![deep]);
        let l = layout(&root, AREA, &LayoutOptions::default());
        assert_eq!(l.tiles.iter().map(|t| t.depth).max(), Some(2));
    }

    #[test]
    fn draw_depth_steps() {
        use DrawDepth::{Levels, Unlimited};
        // Tree 5 levels deep.
        assert_eq!(Levels(2).deeper(5), Levels(3));
        assert_eq!(Levels(4).deeper(5), Unlimited);
        assert_eq!(Unlimited.deeper(5), Unlimited);
        assert_eq!(Unlimited.shallower(5), Levels(4));
        assert_eq!(Levels(2).shallower(5), Levels(1));
        assert_eq!(Levels(1).shallower(5), Levels(1));
        // Shallow trees: `]` reaches unlimited immediately.
        assert_eq!(Levels(2).deeper(1), Unlimited);
        assert_eq!(Unlimited.shallower(0), Levels(1));
        assert_eq!(Levels(3).max_depth(), Some(3));
        assert_eq!(Unlimited.max_depth(), None);
    }

    #[test]
    fn draw_depth_parses_and_displays() {
        assert_eq!("3".parse::<DrawDepth>(), Ok(DrawDepth::Levels(3)));
        assert_eq!("all".parse::<DrawDepth>(), Ok(DrawDepth::Unlimited));
        assert!("0".parse::<DrawDepth>().is_err());
        assert!("x".parse::<DrawDepth>().is_err());
        assert_eq!(DrawDepth::Levels(2).to_string(), "2");
        assert_eq!(DrawDepth::Unlimited.to_string(), "∞");
    }

    #[test]
    fn file_count_weight() {
        let mut a = leaf("a", 1_000_000);
        a.file_count = 1;
        let mut b = leaf("b", 1);
        b.file_count = 3;
        let root = node("r", 0, vec![a, b]);
        let options = LayoutOptions {
            weight: Weight::Files,
            ..LayoutOptions::default()
        };
        let l = layout(&root, AREA, &options);
        let tiles = top_level(&l);
        assert_eq!(name(tiles[0]), "b");
        assert!(tiles[0].rect.area() > tiles[1].rect.area() * 2);
    }

    #[test]
    fn resize_recomputes_within_new_bounds() {
        let children = (1..=12).map(|i| leaf(&format!("d{i}"), i * 10)).collect();
        let root = node("r", 0, children);
        let small = Rect::new(0, 0, 60, 20);
        let big = Rect::new(0, 0, 200, 60);
        let l1 = layout(&root, small, &LayoutOptions::default());
        let l2 = layout(&root, big, &LayoutOptions::default());
        assert_exact_cover(&top_level(&l1), small);
        assert_exact_cover(&top_level(&l2), big);
        assert!(l2.tiles[0].rect.area() > l1.tiles[0].rect.area());
    }

    #[test]
    fn too_small_area_is_reported() {
        let root = node("r", 0, vec![leaf("a", 1)]);
        let l = layout(&root, Rect::new(0, 0, 39, 24), &LayoutOptions::default());
        assert!(l.too_small);
        assert!(l.tiles.is_empty());
        let l = layout(&root, Rect::new(0, 0, 80, 3), &LayoutOptions::default());
        assert!(l.too_small);
    }

    #[test]
    fn deep_tree_layout_is_consistent_at_every_level() {
        // Every subdivided tile's children exactly cover its inner area.
        let mut children = Vec::new();
        for i in 0..6 {
            let grandkids = (0..8)
                .map(|j| leaf(&format!("g{i}{j}"), (j + 1) * 100))
                .collect();
            children.push(node(&format!("c{i}"), 50, grandkids));
        }
        let root = node("r", 0, children);
        let l = layout(&root, Rect::new(0, 0, 160, 50), &LayoutOptions::default());
        for (i, t) in l.tiles.iter().enumerate() {
            let Some(inner) = t.inner else { continue };
            let kids: Vec<_> = l.tiles[i + 1..]
                .iter()
                .take_while(|k| k.depth > t.depth)
                .filter(|k| k.depth == t.depth + 1)
                .collect();
            assert_exact_cover(&kids, inner);
        }
    }

    /// Small deterministic generator, so the property test needs no crate.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            self.0 >> 33
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn random_tree(rng: &mut Lcg, name: &str, depth: usize) -> DirNode {
        let kids = if depth == 0 { 0 } else { rng.below(12) };
        let children = (0..kids)
            .map(|i| random_tree(rng, &format!("{name}.{i}"), depth - 1))
            .collect();
        // Heavy-tailed sizes, zero included.
        let own = match rng.below(4) {
            0 => 0,
            1 => rng.below(100),
            2 => rng.below(100_000),
            _ => rng.below(100_000_000),
        };
        node(name, own, children)
    }

    fn tile_weight(t: &Tile<'_>, weight: Weight) -> u64 {
        match t.kind {
            TileKind::Dir(n) => weight.of(n),
            TileKind::Files { weight, .. } | TileKind::Overflow { weight, .. } => weight,
        }
    }

    #[test]
    fn random_trees_satisfy_layout_invariants() {
        let mut rng = Lcg(42);
        for case in 0..300 {
            let root = random_tree(&mut rng, "r", 4);
            let area = Rect::new(
                rng.below(5) as u16,
                rng.below(5) as u16,
                40 + rng.below(200) as u16,
                4 + rng.below(80) as u16,
            );
            let options = LayoutOptions {
                weight: if case % 3 == 0 {
                    Weight::Files
                } else {
                    Weight::Size
                },
                max_depth: [None, Some(1), Some(2), Some(3)][case % 4],
                ..LayoutOptions::default()
            };
            let l = layout(&root, area, &options);

            // Each level covers its parent's area exactly, every tile meets
            // the minimum size, and weights are conserved.
            let mut levels: Vec<(Rect, usize, usize, u64)> =
                vec![(area, 0, 0, options.weight.of(&root))];
            for (i, t) in l.tiles.iter().enumerate() {
                if let Some(inner) = t.inner {
                    levels.push((inner, t.depth, i + 1, tile_weight(t, options.weight)));
                }
                assert!(
                    t.rect.width >= 2 && t.rect.height >= 1,
                    "case {case}: {t:?}"
                );
                assert!(
                    options.max_depth.is_none_or(|m| t.depth <= m),
                    "case {case}"
                );
            }
            for (inner, depth, start, weight) in levels {
                let kids: Vec<_> = l.tiles[start..]
                    .iter()
                    .take_while(|k| k.depth > depth)
                    .filter(|k| k.depth == depth + 1)
                    .collect();
                if weight == 0 {
                    assert!(kids.is_empty());
                    continue;
                }
                assert_exact_cover(&kids, inner);
                let sum: u64 = kids.iter().map(|k| tile_weight(k, options.weight)).sum();
                assert_eq!(sum, weight, "case {case}: weight not conserved");
            }
        }
    }

    #[test]
    fn many_children_lay_out_quickly() {
        let children = (0..20_000)
            .map(|i| leaf(&format!("d{i}"), 1 + i % 997))
            .collect();
        let root = node("r", 0, children);
        let start = std::time::Instant::now();
        let l = layout(&root, Rect::new(0, 0, 300, 90), &LayoutOptions::default());
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        assert_exact_cover(&top_level(&l), Rect::new(0, 0, 300, 90));
    }
}
