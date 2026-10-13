//! The keybinding reference shown by the `?` help overlay.
//!
//! [`BINDINGS`] is the single source of truth for the overlay body. Keep the
//! README `## Keys` table consistent with it.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// One entry in the keybinding reference.
pub struct Binding {
    /// Section heading this entry starts, or `""` to continue the current one.
    pub group: &'static str,
    /// The keys that trigger the action.
    pub keys: &'static str,
    /// What the action does.
    pub desc: &'static str,
}

/// Every binding the overlay lists, in display order.
pub const BINDINGS: &[Binding] = &[
    Binding {
        group: "Navigate",
        keys: "Enter",
        desc: "Open selected directory",
    },
    Binding {
        group: "",
        keys: "Esc / Backspace",
        desc: "Go up one level",
    },
    Binding {
        group: "",
        keys: "Alt+\u{2190} / Alt+\u{2192}",
        desc: "Back / forward in history",
    },
    Binding {
        group: "",
        keys: "Arrows",
        desc: "Move selection between tiles",
    },
    Binding {
        group: "",
        keys: "Tab / Shift+Tab",
        desc: "Next / previous sibling by size",
    },
    Binding {
        group: "View",
        keys: "T",
        desc: "Cycle size \u{2192} apparent \u{2192} files \u{2192} age",
    },
    Binding {
        group: "",
        keys: "D",
        desc: "Toggle disk usage / apparent size",
    },
    Binding {
        group: "",
        keys: "I",
        desc: "Toggle hidden files (rescans)",
    },
    Binding {
        group: "",
        keys: "[ / ]",
        desc: "Draw depth shallower / deeper",
    },
    Binding {
        group: "",
        keys: "/",
        desc: "Name filter",
    },
    Binding {
        group: "Scan",
        keys: "R / F5",
        desc: "Rescan",
    },
    Binding {
        group: "",
        keys: "V",
        desc: "Volume picker",
    },
    Binding {
        group: "",
        keys: "Ctrl+O",
        desc: "Choose scan directory",
    },
    Binding {
        group: "",
        keys: "G / Ctrl+D",
        desc: "Scan whole disk",
    },
    Binding {
        group: "Mark",
        keys: "Space / X",
        desc: "Mark / unmark (absorbing)",
    },
    Binding {
        group: "",
        keys: "Ctrl+Space",
        desc: "Mark without moving selection",
    },
    Binding {
        group: "",
        keys: "C",
        desc: "Review marked list",
    },
    Binding {
        group: "Review",
        keys: "M / P",
        desc: "Trash / permanent delete (then y/n)",
    },
    Binding {
        group: "",
        keys: "! / S / A",
        desc: "Unmark all / save paths / agent prompt",
    },
    Binding {
        group: "",
        keys: "U / Esc",
        desc: "Unmark row / leave review",
    },
    Binding {
        group: "Other",
        keys: "?",
        desc: "This help",
    },
    Binding {
        group: "",
        keys: "Q / Ctrl+C",
        desc: "Quit",
    },
];

/// Column at which descriptions line up.
const KEY_COL: usize = 16;

/// The overlay body: one line per section heading or binding.
pub fn lines() -> Vec<Line<'static>> {
    let mut out = Vec::with_capacity(row_count());
    for b in BINDINGS {
        if !b.group.is_empty() {
            out.push(Line::from(Span::styled(
                b.group,
                Style::new().add_modifier(Modifier::BOLD),
            )));
        }
        let keys = b.keys;
        let desc = b.desc;
        let width = KEY_COL;
        out.push(Line::from(format!("  {keys:<width$} {desc}")));
    }
    out
}

/// Number of body lines [`lines`] produces.
pub fn row_count() -> usize {
    BINDINGS.len() + BINDINGS.iter().filter(|b| !b.group.is_empty()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_count_matches_the_rendered_lines() {
        assert_eq!(row_count(), lines().len());
    }

    #[test]
    fn every_group_shows_its_first_binding() {
        // The entry that opens a section carries both the heading and a
        // binding; both must render (regression: the binding used to vanish).
        let body: Vec<String> = lines().iter().map(ToString::to_string).collect();
        assert!(body.iter().any(|l| l.contains("Navigate")));
        assert!(body.iter().any(|l| l.contains("Enter")));
        assert!(body.iter().any(|l| l.contains("Open selected directory")));
    }
}
