//! The right-click menu of a text box: Undo, Cut, Copy, Paste, Delete and Select All, greyed
//! like an Edit menu. A host opens [`tree`] as a [`PopupMenu`](super::menu::PopupMenu) and maps
//! the chosen path back through [`command`].

use super::menu::{MenuItem, MenuTree};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Undo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
}

/// What the text box can do right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub undo: bool,
    pub selection: bool,
    /// The clipboard holds text.
    pub paste: bool,
}

/// The rows top to bottom, `None` a separator.
const ROWS: [Option<(&str, Command)>; 8] = [
    Some(("&Undo", Command::Undo)),
    None,
    Some(("Cu&t", Command::Cut)),
    Some(("&Copy", Command::Copy)),
    Some(("&Paste", Command::Paste)),
    Some(("&Delete", Command::Delete)),
    None,
    Some(("Select &All", Command::SelectAll)),
];

pub fn tree(live: Live) -> MenuTree {
    MenuTree::flat(
        ROWS.iter()
            .map(|row| match row {
                None => MenuItem::Separator,
                Some((label, cmd)) => MenuItem::item(label).enabled(match cmd {
                    Command::Undo => live.undo,
                    Command::Cut | Command::Copy | Command::Delete => live.selection,
                    Command::Paste => live.paste,
                    Command::SelectAll => true,
                }),
            })
            .collect(),
    )
}

/// The command a chosen path (`MenuAction::Chosen`) names.
pub fn command(path: &[usize]) -> Option<Command> {
    match path {
        [i] => ROWS.get(*i).copied().flatten().map(|(_, cmd)| cmd),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled(t: &MenuTree) -> Vec<Option<bool>> {
        t.iter()
            .map(|item| match item {
                MenuItem::Entry { enabled, .. } => Some(*enabled),
                MenuItem::Separator => None,
            })
            .collect()
    }

    #[test]
    fn rows_grey_with_nothing_to_act_on_but_select_all() {
        let none = tree(Live::default());
        assert_eq!(
            enabled(&none),
            [
                Some(false),
                None,
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                None,
                Some(true)
            ]
        );
        let all = tree(Live {
            undo: true,
            selection: true,
            paste: true,
        });
        assert!(enabled(&all).iter().flatten().all(|on| *on));
        let paste = tree(Live {
            paste: true,
            ..Live::default()
        });
        assert_eq!(
            enabled(&paste)[4],
            Some(true),
            "Paste follows the clipboard"
        );
        assert_eq!(enabled(&paste)[3], Some(false), "Copy needs a selection");
    }

    #[test]
    fn a_chosen_row_names_its_command_and_a_separator_none() {
        let t = tree(Live::default());
        let labels: Vec<_> = t
            .iter()
            .filter_map(|item| match item {
                MenuItem::Entry { label, .. } => Some(label.as_str()),
                MenuItem::Separator => None,
            })
            .collect();
        assert_eq!(
            labels,
            ["Undo", "Cut", "Copy", "Paste", "Delete", "Select All"]
        );
        assert_eq!(command(&[0]), Some(Command::Undo));
        assert_eq!(command(&[1]), None);
        assert_eq!(command(&[4]), Some(Command::Paste));
        assert_eq!(command(&[7]), Some(Command::SelectAll));
        assert_eq!(command(&[8]), None);
        assert_eq!(command(&[4, 0]), None);
    }
}
