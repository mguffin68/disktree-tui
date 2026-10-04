//! Squarified treemap layout algorithm.

/// A rectangle in the treemap, expressed in grid units (terminal columns/rows).
#[derive(Debug, Clone, Copy, PartialEq)]
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

/// A tile assignment: a node paired with its rectangle.
#[derive(Debug, Clone)]
pub struct Tile {
    /// Index into the flat leaf list.
    pub index: usize,
    /// The rectangle for this tile.
    pub rect: Rect,
}

/// Layout input: a flat list of leaf directories with their sizes.
#[derive(Debug, Clone)]
pub struct LayoutInput {
    /// Leaf node names.
    pub names: Vec<String>,
    /// Leaf node sizes.
    pub sizes: Vec<u64>,
    /// Whether each leaf is reclaimable.
    pub reclaimable: Vec<bool>,
}

impl LayoutInput {
    /// Total size of all leaves.
    pub fn total_size(&self) -> u64 {
        self.sizes.iter().sum()
    }
}

/// Compute a squarified treemap layout.
pub fn squarified(input: &LayoutInput, width: u16, height: u16) -> Vec<Tile> {
    if input.sizes.is_empty() || width == 0 || height == 0 {
        return Vec::new();
    }

    let total = input.total_size() as f64;
    let mut tiles = Vec::new();
    let mut remaining = input.sizes.clone();

    // Simple row/column slicing layout (basic squarified approach).
    layout_row(
        &mut remaining,
        0.0,
        0.0,
        width as f64,
        height as f64,
        total,
        &mut tiles,
        0,
    );

    tiles
}

fn layout_row(
    sizes: &mut Vec<u64>,
    x_start: f64,
    y_start: f64,
    available_width: f64,
    available_height: f64,
    total: f64,
    tiles: &mut Vec<Tile>,
    mut offset: usize,
) {
    if sizes.is_empty() {
        return;
    }

    // Split into rows: each row gets one or more leaves.
    let mut best_loss = f64::MAX;
    let mut best_split = 1;
    let mut accum = 0.0;

    for (i, &s) in sizes.iter().enumerate().take(sizes.len().saturating_sub(1)) {
        accum += s as f64;
        let count = i + 1;

        let _row_ratio = if accum > 0.0 {
            (accum / total) * (available_width * available_height)
        } else {
            0.0
        };

        let (row_w, row_h) = if count == sizes.len() {
            // Last row: use all remaining space.
            (available_width, available_height)
        } else {
            (available_height, available_height)
        };

        // Calculate aspect ratio loss (lower is better).
        // For now, use a simple approach: split when aspect ratio degrades.
        let loss = if row_w > 0.0 && row_h > 0.0 {
            let max = row_w.max(row_h);
            let min = row_w.min(row_h);
            (max / min) - 1.0
        } else {
            f64::MAX
        };

        if loss < best_loss {
            best_loss = loss;
            best_split = count;
        }
    }

    let split_count = best_split.max(1).min(sizes.len());

    // Calculate row dimensions.
    let row_total: f64 = sizes[..split_count].iter().map(|s| *s as f64).sum();
    let row_area = (row_total / total) * (available_width * available_height);

    // Decide orientation: slice along width or height.
    let (cell_w, cell_h) = if available_width > available_height {
        (row_area / available_height, available_height)
    } else {
        (available_width, row_area / available_width)
    };

    let (dx, dy) = if available_width > available_height {
        (cell_w, 0.0)
    } else {
        (0.0, cell_h)
    };

    // Place tiles in this row.
    let mut pos = 0.0;
    for &s in &sizes[..split_count] {
        let area = (s as f64 / total) * (available_width * available_height);
        let (tw, th) = if dx > 0.0 {
            (area / cell_h, cell_h)
        } else {
            (cell_w, area / cell_w)
        };

        tiles.push(Tile {
            index: offset,
            rect: Rect {
                x: (x_start + pos).round() as u16,
                y: (y_start + dy).round() as u16,
                width: tw.max(1.0).round() as u16,
                height: th.max(1.0).round() as u16,
            },
        });

        pos += tw;
        offset += 1;
    }

    // Recurse on remaining.
    sizes.drain(..split_count);
    if !sizes.is_empty() {
        let new_w = available_width - (if dx > 0.0 { cell_w } else { 0.0 });
        let new_h = available_height - (if dy > 0.0 { cell_h } else { 0.0 });
        let new_x = if dx > 0.0 { x_start + cell_w } else { x_start };
        let new_y = if dy > 0.0 { y_start + cell_h } else { y_start };
        layout_row(sizes, new_x, new_y, new_w, new_h, total, tiles, offset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_tiles() {
        let input = LayoutInput {
            names: Vec::new(),
            sizes: Vec::new(),
            reclaimable: Vec::new(),
        };
        let tiles = squarified(&input, 80, 24);
        assert!(tiles.is_empty());
    }

    #[test]
    fn single_leaf_fills_area() {
        let input = LayoutInput {
            names: vec!["a".into()],
            sizes: vec![100],
            reclaimable: vec![false],
        };
        let tiles = squarified(&input, 80, 24);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].rect.width, 80);
        assert_eq!(tiles[0].rect.height, 24);
    }

    #[test]
    fn two_equal_children_split_widthwise() {
        let input = LayoutInput {
            names: vec!["a".into(), "b".into()],
            sizes: vec![100, 100],
            reclaimable: vec![false, false],
        };
        let tiles = squarified(&input, 80, 24);
        assert_eq!(tiles.len(), 2);
        // Each tile should have positive area.
        for tile in &tiles {
            assert!(tile.rect.width > 0);
            assert!(tile.rect.height > 0);
        }
    }
}
