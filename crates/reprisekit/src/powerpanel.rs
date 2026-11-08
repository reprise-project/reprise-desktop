//! The Log Off and Turn off computer panels: a title band with the logo, big icon buttons with
//! labels under them on a lit band, and Cancel in the bottom band. Model, layout, hit and access
//! here; `blue::powerpanel` paints it and `interfaces::draw_power_panel` puts the two together.

use crate::access::accesskit::NodeId;
use crate::access::{self as ax, Tree};
use crate::raster::Rect;
use crate::text::split_mnemonic;
use crate::theme::TextUnits;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Choice {
    SwitchUser,
    LogOff,
    StandBy,
    Hibernate,
    TurnOff,
    Restart,
    Disconnect,
}

impl Choice {
    /// The label under the button, with its access key marked by `&`.
    pub fn label(self) -> String {
        match self {
            Choice::SwitchUser => tr!("powerpanel-switch-user"),
            Choice::LogOff => tr!("powerpanel-log-off"),
            Choice::StandBy => tr!("powerpanel-stand-by"),
            Choice::Hibernate => tr!("powerpanel-hibernate"),
            Choice::TurnOff => tr!("powerpanel-turn-off"),
            Choice::Restart => tr!("powerpanel-restart"),
            Choice::Disconnect => tr!("powerpanel-disconnect"),
        }
    }

    /// The balloon tip shown while the pointer rests on the button.
    pub fn tip(self) -> String {
        match self {
            Choice::SwitchUser => tr!("powerpanel-switch-user-tip"),
            Choice::LogOff => tr!("powerpanel-log-off-tip"),
            Choice::StandBy => tr!("powerpanel-stand-by-tip"),
            Choice::Hibernate => tr!("powerpanel-hibernate-tip"),
            Choice::TurnOff => tr!("powerpanel-turn-off-tip"),
            Choice::Restart => tr!("powerpanel-restart-tip"),
            Choice::Disconnect => tr!("powerpanel-disconnect-tip"),
        }
    }
}

/// A big button: `choice` and whether it is live, and the choice it turns into while Shift is
/// held (Stand By into Hibernate), which is always live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Big {
    pub choice: Choice,
    pub enabled: bool,
    pub shifted: Option<Choice>,
}

impl Big {
    fn new(choice: Choice, enabled: bool) -> Big {
        Big {
            choice,
            enabled,
            shifted: None,
        }
    }

    /// What the button offers with Shift held or not, and whether it is live.
    pub fn shown(&self, shift: bool) -> (Choice, bool) {
        match self.shifted {
            Some(c) if shift => (c, true),
            _ => (self.choice, self.enabled),
        }
    }
}

/// The Log Off panel's first button: Switch User, live or greyed, or Disconnect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leave {
    SwitchUser { live: bool },
    Disconnect,
}

/// The Turn Off panel's first button: Stand By, live or greyed and with Hibernate under Shift or
/// not, or Disconnect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum First {
    StandBy { live: bool, hibernate: bool },
    Disconnect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Big(usize),
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panel {
    pub title: String,
    pub bigs: Vec<Big>,
    pub focus: Part,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Tab { back: bool },
    Enter,
    Space,
    Escape,
    Char(char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Press(Part),
}

impl Panel {
    /// `leave`'s button and Log Off, which has the focus.
    pub fn log_off(leave: Leave) -> Panel {
        let first = match leave {
            Leave::SwitchUser { live } => Big::new(Choice::SwitchUser, live),
            Leave::Disconnect => Big::new(Choice::Disconnect, true),
        };
        Panel {
            title: tr!("powerpanel-title-log-off"),
            bigs: vec![first, Big::new(Choice::LogOff, true)],
            focus: Part::Big(1),
            shift: false,
        }
    }

    /// `first`, Turn Off, which has the focus unless `first` is Disconnect, and Restart.
    pub fn turn_off(first: First) -> Panel {
        let (first, focus) = match first {
            First::StandBy { live, hibernate } => {
                let shifted = hibernate.then_some(Choice::Hibernate);
                let stand_by = Big::new(Choice::StandBy, live);
                (
                    Big {
                        shifted,
                        ..stand_by
                    },
                    1,
                )
            }
            First::Disconnect => (Big::new(Choice::Disconnect, true), 0),
        };
        Panel {
            title: tr!("powerpanel-title-turn-off"),
            bigs: vec![
                first,
                Big::new(Choice::TurnOff, true),
                Big::new(Choice::Restart, true),
            ],
            focus: Part::Big(focus),
            shift: false,
        }
    }

    pub fn shown(&self, i: usize) -> Option<(Choice, bool)> {
        self.bigs.get(i).map(|b| b.shown(self.shift))
    }

    pub fn live(&self, part: Part) -> bool {
        match part {
            Part::Big(i) => self.shown(i).is_some_and(|(_, on)| on),
            Part::Cancel => true,
        }
    }

    /// Arrows walk the live big buttons and stop at the ends; Tab and Shift+Tab cycle them and
    /// Cancel. Enter and Space press the focused one, Escape presses Cancel and a letter the
    /// live big button whose access key it is.
    pub fn key(&mut self, key: Key) -> Event {
        match key {
            Key::Left | Key::Up => self.step(false),
            Key::Right | Key::Down => self.step(true),
            Key::Tab { back } => {
                let mut stops: Vec<Part> = (0..self.bigs.len())
                    .map(Part::Big)
                    .filter(|&p| self.live(p))
                    .collect();
                stops.push(Part::Cancel);
                let n = stops.len();
                let at = stops.iter().position(|&p| p == self.focus).unwrap_or(0);
                self.focus_on(stops[if back { (at + n - 1) % n } else { (at + 1) % n }])
            }
            Key::Enter | Key::Space => Event::Press(self.focus),
            Key::Escape => Event::Press(Part::Cancel),
            Key::Char(c) => {
                let c = c.to_lowercase().next();
                (0..self.bigs.len())
                    .find(|&i| {
                        let (choice, on) = self.bigs[i].shown(self.shift);
                        on && access_key(&choice.label()) == c
                    })
                    .map_or(Event::None, |i| Event::Press(Part::Big(i)))
            }
        }
    }

    fn step(&mut self, forward: bool) -> Event {
        let Part::Big(i) = self.focus else {
            return Event::None;
        };
        let next = if forward {
            (i + 1..self.bigs.len()).find(|&j| self.live(Part::Big(j)))
        } else {
            (0..i).rev().find(|&j| self.live(Part::Big(j)))
        };
        next.map_or(Event::None, |j| self.focus_on(Part::Big(j)))
    }

    /// Give `part` the focus, if it is live and does not have it.
    pub fn focus_on(&mut self, part: Part) -> Event {
        if part == self.focus || !self.live(part) {
            return Event::None;
        }
        self.focus = part;
        Event::Redraw
    }

    /// Shift went down or up. A button that turns into another redraws; a focused one that goes
    /// greyed hands the focus to the middle button.
    pub fn set_shift(&mut self, on: bool) -> Event {
        if self.shift == on {
            return Event::None;
        }
        self.shift = on;
        if !self.live(self.focus) {
            self.focus = Part::Big(1.min(self.bigs.len().saturating_sub(1)));
        }
        if self.bigs.iter().any(|b| b.shifted.is_some()) {
            Event::Redraw
        } else {
            Event::None
        }
    }
}

fn access_key(label: &str) -> Option<char> {
    let (plain, at) = split_mnemonic(label);
    at.and_then(|i| plain.chars().nth(i))
        .and_then(|c| c.to_lowercase().next())
}

/// Where everything sits, in logical px from the panel's top-left, grown with the text size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub size: (i32, i32),
    pub title: Rect,
    /// The lit band the big buttons stand on; the separator fills the rows between it and the
    /// title band.
    pub band: Rect,
    pub cells: Vec<Rect>,
    pub cancel: Rect,
    pub units: TextUnits,
}

const W: i32 = 314;
const H: i32 = 200;
pub const BORDER: i32 = 1;
const TITLE_H: i32 = 43;
pub const SEPARATOR: i32 = 2;
const BAND_BOTTOM: i32 = 156;
const CELL_Y: i32 = 72;
const CELL_H: i32 = 64;
pub const ICON: i32 = 32;
const ICON_Y: i32 = 8;
const LABEL_Y: i32 = 44;
const CANCEL: Rect = Rect::new(W - BORDER - 10 - 60, BORDER + 167, 60, 20);
/// The big buttons' cells by count: their width and left edges.
const TWO: (i32, [i32; 2]) = (115, [43, 158]);
const THREE: (i32, [i32; 3]) = (85, [29, 115, 200]);

pub fn layout(p: &Panel, u: TextUnits) -> Layout {
    let title = u.rect(Rect::new(BORDER, BORDER, W - 2 * BORDER, TITLE_H));
    let band_y = title.bottom() + SEPARATOR;
    let band = Rect::new(title.x, band_y, title.width, u.len(BAND_BOTTOM) - band_y);
    let n = p.bigs.len();
    let (w, xs): (i32, Vec<i32>) = match n {
        2 => (TWO.0, TWO.1.to_vec()),
        3 => (THREE.0, THREE.1.to_vec()),
        _ => {
            let left = (W - n as i32 * THREE.0) / 2;
            (THREE.0, (0..n as i32).map(|i| left + i * THREE.0).collect())
        }
    };
    let cells = xs
        .into_iter()
        .map(|x| u.rect(Rect::new(x, BORDER + CELL_Y, w, CELL_H)))
        .collect();
    Layout {
        size: u.size((W, H)),
        title,
        band,
        cells,
        cancel: u.rect(CANCEL),
        units: u,
    }
}

impl Layout {
    /// A cell's icon and label: the label keeps its gap to the cell's bottom as the text grows,
    /// the icon keeps its size and moves down half as far as the label did.
    pub fn cell_parts(&self, cell: Rect) -> (Rect, Rect) {
        let u = self.units;
        let label_y = (cell.height - u.len(CELL_H - LABEL_Y)).max(LABEL_Y);
        let icon_y = ICON_Y + (label_y - LABEL_Y) / 2;
        let icon = Rect::new(
            cell.x + (cell.width - ICON) / 2,
            cell.y + icon_y,
            ICON,
            ICON,
        );
        let label = Rect::new(
            cell.x,
            cell.y + label_y,
            cell.width,
            u.len(CELL_H - LABEL_Y),
        );
        (icon, label)
    }

    pub fn rect(&self, part: Part) -> Option<Rect> {
        match part {
            Part::Big(i) => self.cells.get(i).copied(),
            Part::Cancel => Some(self.cancel),
        }
    }
}

pub fn hit(lay: &Layout, x: i32, y: i32) -> Option<Part> {
    if lay.cancel.contains(x, y) {
        return Some(Part::Cancel);
    }
    lay.cells
        .iter()
        .position(|r| r.contains(x, y))
        .map(Part::Big)
}

/// The buttons under `parent`, placed at `origin`, each described by its tip. Returns the focused
/// one.
pub fn access(
    p: &Panel,
    lay: &Layout,
    origin: (i32, i32),
    tree: &mut Tree,
    parent: NodeId,
) -> NodeId {
    let at = |r: Rect| Rect::new(r.x + origin.0, r.y + origin.1, r.width, r.height);
    let mut focus = parent;
    for (i, cell) in lay.cells.iter().enumerate() {
        let Some((choice, on)) = p.shown(i) else {
            continue;
        };
        let mut n = ax::button(at(*cell), &choice.label(), on, p.focus == Part::Big(i));
        n.set_description(choice.tip());
        let id = tree.push(parent, ("button", i), n);
        if p.focus == Part::Big(i) {
            focus = id;
        }
    }
    let id = tree.push(
        parent,
        ("button", lay.cells.len()),
        ax::button(
            at(lay.cancel),
            &tr!("cancel"),
            true,
            p.focus == Part::Cancel,
        ),
    );
    if p.focus == Part::Cancel {
        focus = id;
    }
    focus
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_off_focuses_turn_off_and_the_arrows_skip_a_greyed_stand_by() {
        let mut p = Panel::turn_off(First::StandBy {
            live: false,
            hibernate: false,
        });
        assert_eq!(p.focus, Part::Big(1));
        assert_eq!(p.key(Key::Left), Event::None, "Stand By is greyed");
        assert_eq!(p.key(Key::Right), Event::Redraw);
        assert_eq!(p.focus, Part::Big(2));
        assert_eq!(p.key(Key::Right), Event::None, "the arrows stop at Restart");
        assert_eq!(p.key(Key::Enter), Event::Press(Part::Big(2)));
        assert_eq!(p.key(Key::Escape), Event::Press(Part::Cancel));
    }

    #[test]
    fn tab_cycles_the_live_buttons_and_cancel() {
        let mut p = Panel::log_off(Leave::SwitchUser { live: false });
        assert_eq!(p.key(Key::Tab { back: false }), Event::Redraw);
        assert_eq!(p.focus, Part::Cancel);
        assert_eq!(p.key(Key::Tab { back: false }), Event::Redraw);
        assert_eq!(
            p.focus,
            Part::Big(1),
            "Tab wraps past the greyed Switch User"
        );
        assert_eq!(p.key(Key::Tab { back: true }), Event::Redraw);
        assert_eq!(p.focus, Part::Cancel);
        assert_eq!(
            p.key(Key::Left),
            Event::None,
            "the arrows leave Cancel alone"
        );
        assert_eq!(p.key(Key::Space), Event::Press(Part::Cancel));
    }

    #[test]
    fn a_letter_presses_the_live_button_it_marks() {
        let mut p = Panel::turn_off(First::StandBy {
            live: true,
            hibernate: false,
        });
        assert_eq!(p.key(Key::Char('S')), Event::Press(Part::Big(0)));
        assert_eq!(p.key(Key::Char('u')), Event::Press(Part::Big(1)));
        assert_eq!(p.key(Key::Char('r')), Event::Press(Part::Big(2)));
        assert_eq!(p.key(Key::Char('z')), Event::None);
        let mut l = Panel::log_off(Leave::SwitchUser { live: false });
        assert_eq!(l.key(Key::Char('s')), Event::None, "Switch User is greyed");
        assert_eq!(l.key(Key::Char('l')), Event::Press(Part::Big(1)));
    }

    #[test]
    fn shift_turns_stand_by_into_hibernate_when_there_is_one() {
        let mut p = Panel::turn_off(First::StandBy {
            live: false,
            hibernate: true,
        });
        assert_eq!(p.shown(0), Some((Choice::StandBy, false)));
        assert_eq!(p.set_shift(true), Event::Redraw);
        assert_eq!(p.shown(0), Some((Choice::Hibernate, true)));
        assert_eq!(p.key(Key::Char('h')), Event::Press(Part::Big(0)));
        assert_eq!(p.key(Key::Left), Event::Redraw);
        assert_eq!(p.set_shift(false), Event::Redraw);
        assert_eq!(
            p.focus,
            Part::Big(1),
            "the greyed Stand By gives up the focus"
        );
        let mut none = Panel::turn_off(First::StandBy {
            live: true,
            hibernate: false,
        });
        assert_eq!(none.set_shift(true), Event::None);
        assert_eq!(none.shown(0), Some((Choice::StandBy, true)));
    }

    #[test]
    fn the_panel_lays_out_its_cells_and_cancel() {
        let t = layout(
            &Panel::turn_off(First::StandBy {
                live: true,
                hibernate: false,
            }),
            TextUnits::ONE,
        );
        assert_eq!(t.size, (314, 200));
        assert_eq!(t.title, Rect::new(1, 1, 312, 43));
        assert_eq!(t.band, Rect::new(1, 46, 312, 110));
        let centres: Vec<i32> = t.cells.iter().map(|r| r.x + r.width / 2).collect();
        assert_eq!(centres, vec![71, 157, 242]);
        assert_eq!(t.cells[0], Rect::new(29, 73, 85, 64));
        assert_eq!(t.cancel, Rect::new(243, 168, 60, 20));
        assert_eq!(
            t.cell_parts(t.cells[1]),
            (Rect::new(141, 81, 32, 32), Rect::new(115, 117, 85, 20))
        );
        assert_eq!(hit(&t, 156, 100), Some(Part::Big(1)));
        assert_eq!(hit(&t, 250, 175), Some(Part::Cancel));
        assert_eq!(hit(&t, 5, 100), None);
        let l = layout(
            &Panel::log_off(Leave::SwitchUser { live: false }),
            TextUnits::ONE,
        );
        let centres: Vec<i32> = l.cells.iter().map(|r| r.x + r.width / 2).collect();
        assert_eq!(centres, vec![100, 215]);
        assert_eq!(l.cancel, t.cancel);
    }

    #[test]
    fn the_panel_grows_with_the_text_but_its_icons_do_not() {
        let p = Panel::turn_off(First::StandBy {
            live: true,
            hibernate: false,
        });
        let u = TextUnits::new(16, 11);
        let l = layout(&p, u);
        assert_eq!(l.size, (u.len(314), u.len(200)));
        let (icon, label) = l.cell_parts(l.cells[0]);
        assert_eq!((icon.width, icon.height), (32, 32));
        assert_eq!(
            label.bottom(),
            l.cells[0].bottom(),
            "the label keeps its gap"
        );
        assert!(icon.bottom() <= label.y, "the icon stays above its label");
    }

    #[test]
    fn access_names_the_buttons_greys_the_dead_and_focuses_the_default() {
        use crate::access::{ROOT, accesskit::Role};
        let p = Panel::log_off(Leave::SwitchUser { live: false });
        let lay = layout(&p, TextUnits::ONE);
        let mut tree = Tree::new(ax::node(Role::Dialog, Rect::new(0, 0, 314, 200)));
        let focus = access(&p, &lay, (100, 50), &mut tree, ROOT);
        let buttons: Vec<_> = tree
            .nodes()
            .filter(|(id, _)| *id != ROOT)
            .map(|(id, n)| (id, n.label().map(str::to_owned), n.is_disabled()))
            .collect();
        assert_eq!(buttons.len(), 3);
        assert_eq!(buttons[0].1.as_deref(), Some("Switch User"));
        assert!(buttons[0].2, "Switch User is greyed");
        assert_eq!(buttons[2].1.as_deref(), Some("Cancel"));
        assert_eq!(focus, buttons[1].0, "Log Off has the focus");
        let b = tree.get(buttons[1].0).and_then(|n| n.bounds()).unwrap();
        assert_eq!((b.x0, b.y0), (258.0, 123.0), "placed at the origin");
    }

    #[test]
    fn the_log_off_panel_offers_disconnect_in_switch_users_place() {
        use crate::access::{ROOT, accesskit::Role};
        let mut p = Panel::log_off(Leave::Disconnect);
        assert_eq!(p.bigs.len(), 2);
        assert_eq!(p.shown(0), Some((Choice::Disconnect, true)));
        assert_eq!(p.shown(1), Some((Choice::LogOff, true)));
        assert_eq!(p.focus, Part::Big(1), "Log Off has the focus");
        assert_eq!(p.key(Key::Char('d')), Event::Press(Part::Big(0)));
        let lay = layout(&p, TextUnits::ONE);
        let mut tree = Tree::new(ax::node(Role::Dialog, Rect::new(0, 0, 314, 200)));
        access(&p, &lay, (0, 0), &mut tree, ROOT);
        let first = tree.nodes().find(|(id, _)| *id != ROOT).map(|(_, n)| n);
        assert_eq!(first.and_then(|n| n.label()), Some("Disconnect"));
    }

    #[test]
    fn the_turn_off_panel_offers_disconnect_in_stand_bys_place_with_the_focus() {
        let mut p = Panel::turn_off(First::Disconnect);
        let shown: Vec<_> = (0..p.bigs.len()).filter_map(|i| p.shown(i)).collect();
        assert_eq!(
            shown,
            [
                (Choice::Disconnect, true),
                (Choice::TurnOff, true),
                (Choice::Restart, true)
            ]
        );
        assert_eq!(p.focus, Part::Big(0), "Disconnect has the focus");
        assert_eq!(
            p.set_shift(true),
            Event::None,
            "nothing turns into Hibernate"
        );
        assert_eq!(p.shown(0), Some((Choice::Disconnect, true)));
        assert_eq!(p.key(Key::Char('d')), Event::Press(Part::Big(0)));
    }
}
