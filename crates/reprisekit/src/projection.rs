//! How the connected displays show the desktop: the four choices the display switcher (Super+P)
//! and the Display page's "Multiple displays" combo offer.

/// One way of using the connected displays. `Only(i)` shows the desktop on display `i` (from 0,
/// in the Display page's order) and turns the others off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Projection {
    Extend,
    Duplicate,
    Only(usize),
}

impl Projection {
    /// The choices in the order the switcher's tiles and the combo's rows list them.
    pub const ALL: [Projection; 4] = [
        Projection::Extend,
        Projection::Duplicate,
        Projection::Only(0),
        Projection::Only(1),
    ];

    /// The switcher tile's caption.
    pub fn label(self) -> String {
        match self {
            Projection::Extend => "Extend".into(),
            Projection::Duplicate => "Duplicate".into(),
            Projection::Only(i) => format!("Only {}", i + 1),
        }
    }

    /// The Display page combo's row.
    pub fn row(self) -> String {
        match self {
            Projection::Extend => "Extend these displays".into(),
            Projection::Duplicate => "Duplicate these displays".into(),
            Projection::Only(i) => format!("Show only on {}", i + 1),
        }
    }

    /// Its place in [`Projection::ALL`]; None for an `Only` past the second display.
    pub fn index(self) -> Option<usize> {
        Projection::ALL.iter().position(|p| *p == self)
    }

    /// Whether it can be picked with `connected` displays: Extend and Duplicate need two, `Only(i)`
    /// needs display `i`.
    pub fn available(self, connected: usize) -> bool {
        match self {
            Projection::Extend | Projection::Duplicate => connected >= 2,
            Projection::Only(i) => i < connected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_choices_list_extend_duplicate_then_each_display() {
        let labels: Vec<String> = Projection::ALL.iter().map(|p| p.label()).collect();
        assert_eq!(labels, ["Extend", "Duplicate", "Only 1", "Only 2"]);
        assert_eq!(Projection::Only(1).row(), "Show only on 2");
        assert_eq!(Projection::Duplicate.index(), Some(1));
        assert_eq!(Projection::Only(2).index(), None);
    }

    #[test]
    fn one_display_offers_only_itself() {
        let one: Vec<bool> = Projection::ALL.iter().map(|p| p.available(1)).collect();
        assert_eq!(one, [false, false, true, false]);
        assert!(Projection::ALL.iter().all(|p| p.available(2)));
    }
}
