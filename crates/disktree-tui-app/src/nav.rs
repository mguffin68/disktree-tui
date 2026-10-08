//! Spatial selection: which tile an arrow key moves to.

use disktree_tui_core::layout::Rect;

/// An arrow-key direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Index of the tile in `rects` to move to from `from`, if any.
///
/// Candidates must lie entirely on the requested side. Among them, tiles
/// that overlap `from` across the axis of travel win over ones that don't;
/// then the nearest along the axis; then the smallest sideways offset.
pub fn neighbor(rects: &[Rect], from: Rect, direction: Direction) -> Option<usize> {
    rects
        .iter()
        .enumerate()
        .filter(|&(_, r)| *r != from)
        .filter_map(|(i, &r)| {
            let (gap, cross_gap, cross_offset) = match direction {
                Direction::Right => (
                    i32::from(r.x) - i32::from(from.right()),
                    interval_gap(from.y, from.bottom(), r.y, r.bottom()),
                    center_offset(from.y, from.bottom(), r.y, r.bottom()),
                ),
                Direction::Left => (
                    i32::from(from.x) - i32::from(r.right()),
                    interval_gap(from.y, from.bottom(), r.y, r.bottom()),
                    center_offset(from.y, from.bottom(), r.y, r.bottom()),
                ),
                Direction::Down => (
                    i32::from(r.y) - i32::from(from.bottom()),
                    interval_gap(from.x, from.right(), r.x, r.right()),
                    center_offset(from.x, from.right(), r.x, r.right()),
                ),
                Direction::Up => (
                    i32::from(from.y) - i32::from(r.bottom()),
                    interval_gap(from.x, from.right(), r.x, r.right()),
                    center_offset(from.x, from.right(), r.x, r.right()),
                ),
            };
            (gap >= 0).then_some((i, (cross_gap > 0, gap, cross_gap, cross_offset)))
        })
        .min_by_key(|&(_, key)| key)
        .map(|(i, _)| i)
}

/// Distance between `[a0, a1)` and `[b0, b1)`; 0 when they overlap.
fn interval_gap(a0: u16, a1: u16, b0: u16, b1: u16) -> i32 {
    (i32::from(b0) - i32::from(a1))
        .max(i32::from(a0) - i32::from(b1))
        .max(0)
}

/// Distance between the centers of `[a0, a1)` and `[b0, b1)`, doubled to
/// stay in integers.
fn center_offset(a0: u16, a1: u16, b0: u16, b1: u16) -> i32 {
    ((i32::from(a0) + i32::from(a1)) - (i32::from(b0) + i32::from(b1))).abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    //  +------+---+
    //  |  A   | B |
    //  |      +---+
    //  |      | C |
    //  +--+---+---+
    //  |D |   E   |
    //  +--+-------+
    const A: Rect = Rect::new(0, 0, 6, 4);
    const B: Rect = Rect::new(6, 0, 4, 2);
    const C: Rect = Rect::new(6, 2, 4, 2);
    const D: Rect = Rect::new(0, 4, 2, 2);
    const E: Rect = Rect::new(2, 4, 8, 2);
    const ALL: [Rect; 5] = [A, B, C, D, E];

    fn go(from: Rect, d: Direction) -> Option<Rect> {
        neighbor(&ALL, from, d).map(|i| ALL[i])
    }

    #[test]
    fn moves_to_adjacent_tiles() {
        assert_eq!(go(A, Direction::Right), Some(B));
        assert_eq!(go(B, Direction::Down), Some(C));
        assert_eq!(go(C, Direction::Up), Some(B));
        assert_eq!(go(C, Direction::Down), Some(E));
        assert_eq!(go(B, Direction::Left), Some(A));
        assert_eq!(go(E, Direction::Left), Some(D));
    }

    #[test]
    fn prefers_overlap_then_nearest_center() {
        // Below A, both D and E overlap A's columns; D is centered closer.
        assert_eq!(go(A, Direction::Down), Some(D));
        // Above E, A and C both overlap; C's center is nearer E's.
        assert_eq!(go(E, Direction::Up), Some(C));
    }

    #[test]
    fn edges_have_no_neighbor() {
        assert_eq!(go(A, Direction::Up), None);
        assert_eq!(go(A, Direction::Left), None);
        assert_eq!(go(E, Direction::Down), None);
        assert_eq!(go(B, Direction::Right), None);
    }
}
