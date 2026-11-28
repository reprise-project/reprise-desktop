//! A command bar: a pale band of text commands for what is selected, Views, the preview
//! pane's toggle and a help mark at its right end. A command lights a box under the pointer and
//! while held or its menu is open. The commands that do not fit fold, last first, into a » menu
//! at the end of the left run; the right end always shows. The help mark is drawn and inert, from
//! the style's art (none in Classic).

use crate::access::{self, Tree, accesskit};
use crate::art::cmdglyph::{self, CmdGlyph};
use crate::art::viewsicon;
use crate::controls::metrics::ControlMetrics;
use crate::look::Paint;
use crate::look::widgets::toolbar as tb;
use crate::raster::{Canvas, Rect};
use crate::shape::{self, premul};
use crate::style::{Part, State};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;
use accesskit::{NodeId, Role};

/// The bar's height and a command box's at 100% text; both grow with the text.
pub const HEIGHT: i32 = 31;
pub const BOX_H: i32 = 24;
/// From the bar's top to a box's.
const BOX_TOP: i32 = 3;
/// From the bar's left edge to the first box, and from the last box to its right edge.
const LEAD: i32 = 3;
const TRAIL: i32 = 12;
/// From a box's edge to its label; the left boxes sit side by side.
const PAD: i32 = 15;
/// The least room between the left run and the right end, and between Views and the help mark.
const CLEAR: i32 = 8;
/// A drop arrow: 7 x 4, 6 px after the label. A split command's drop part, and Views', is 16 wide.
const ARROW_W: i32 = 7;
const ARROW_H: i32 = 4;
const ARROW_GAP: i32 = 6;
const DROP_W: i32 = 16;
/// The » mark (two chevrons, 8 x 6) and its box's width.
const MORE_MARK: (i32, i32) = (8, 6);
const MORE_W: i32 = 25;
/// The Views icon's box and the help mark's; both icons are 16 px.
const VIEWS_W: i32 = 22;
const HELP_W: i32 = 25;
/// The preview pane toggle's box; its glyph is 16 px too.
const PANE_W: i32 = 25;
const ICON: i32 = 16;

/// What pressing a command does beyond running it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drop {
    /// It runs its command.
    None,
    /// The whole command opens a menu, shown by an arrow after its label.
    Menu,
    /// The label runs the command; a drop part after it opens a menu of alternatives.
    Split,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    /// The host's id for what the command does.
    pub command: u32,
    pub drop: Drop,
    pub enabled: bool,
}

impl Item {
    pub fn new(label: impl Into<String>, command: u32) -> Item {
        Item {
            label: label.into(),
            command,
            drop: Drop::None,
            enabled: true,
        }
    }

    pub fn drop(self, drop: Drop) -> Item {
        Item { drop, ..self }
    }

    pub fn enabled(self, enabled: bool) -> Item {
        Item { enabled, ..self }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RightItem {
    /// The icon cycles the view modes; its arrow opens their list.
    Views,
    /// Shows or hides the preview pane; held down while it shows ([`Commandbar::pane_shown`]).
    PreviewPane,
    Help,
}

/// A part of the bar a press or a key lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Item(usize),
    /// A split command's drop part.
    ItemDrop(usize),
    More,
    ViewsCycle,
    ViewsDrop,
    PreviewPane,
    Help,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Commandbar {
    pub left: Vec<Item>,
    pub right: Vec<RightItem>,
    /// The part under the pointer.
    pub hot: Option<Hit>,
    /// The part held, or whose menu is open.
    pub pressed: Option<Hit>,
    /// The part the keyboard is on while the bar has the focus.
    pub focus: Option<Hit>,
    /// The preview pane shows: its toggle is drawn checked.
    pub pane_shown: bool,
}

/// Keys the bar takes while it has the focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Prev,
    Next,
    First,
    Last,
    /// Enter or Space.
    Activate,
    /// Down, or Alt+Down: a part's menu.
    Drop,
}

/// A shown left command's box, and its drop part's when it is a split command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub main: Rect,
    pub drop: Option<Rect>,
}

/// Where each part of a [`Commandbar`] lies, in the bar's own px.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub bar: Rect,
    /// One per shown left command, from the first: the rest are folded into [`Layout::more`].
    pub items: Vec<Slot>,
    pub more: Option<Rect>,
    /// The Views icon's box and its arrow's.
    pub views: Option<(Rect, Rect)>,
    pub pane: Option<Rect>,
    pub help: Option<Rect>,
}

impl Commandbar {
    /// Shows `left` in place of the commands there were, keeping the keyboard on the part it was on
    /// when that part is still there, else on the first.
    pub fn set_left(&mut self, left: Vec<Item>) {
        let n = left.len();
        self.left = left;
        let gone = |h: Option<Hit>| match h {
            Some(Hit::Item(i) | Hit::ItemDrop(i)) => i >= n,
            _ => false,
        };
        if gone(self.hot) {
            self.hot = None;
        }
        if gone(self.pressed) {
            self.pressed = None;
        }
        if gone(self.focus) {
            self.focus = (n > 0).then_some(Hit::Item(0));
        }
    }

    /// The parts the keyboard steps through, in order: the shown commands that are live, », Views,
    /// the preview pane's toggle.
    /// The help mark is left out: it does nothing.
    pub fn stops(&self, l: &Layout) -> Vec<Hit> {
        let mut out: Vec<Hit> = (self.left.iter().zip(&l.items).enumerate())
            .filter(|(_, (item, _))| item.enabled)
            .map(|(i, _)| Hit::Item(i))
            .collect();
        out.extend(l.more.map(|_| Hit::More));
        out.extend(l.views.map(|_| Hit::ViewsCycle));
        out.extend(l.pane.map(|_| Hit::PreviewPane));
        out
    }

    /// Moves the keyboard by `k`, or answers it with the part to act on: [`Hit::Item`] of a
    /// [`Drop::Menu`] command, [`Hit::ItemDrop`], [`Hit::More`] and [`Hit::ViewsDrop`] ask for a
    /// menu. `None` when the key moved the focus or does nothing here.
    pub fn key(&mut self, k: Key, l: &Layout) -> Option<Hit> {
        let stops = self.stops(l);
        let at = self.focus.and_then(|f| stops.iter().position(|&s| s == f));
        let go = |i: usize| stops.get(i).copied();
        match k {
            Key::Prev => {
                self.focus = at.map_or(stops.last().copied(), |i| go(i.saturating_sub(1)));
                None
            }
            Key::Next => {
                self.focus =
                    at.map_or(stops.first().copied(), |i| go((i + 1).min(stops.len() - 1)));
                None
            }
            Key::First => {
                self.focus = stops.first().copied();
                None
            }
            Key::Last => {
                self.focus = stops.last().copied();
                None
            }
            Key::Activate => at.and(self.focus),
            Key::Drop => match at.and(self.focus)? {
                Hit::Item(i) => match self.left.get(i)?.drop {
                    Drop::None => None,
                    Drop::Menu => Some(Hit::Item(i)),
                    Drop::Split => Some(Hit::ItemDrop(i)),
                },
                Hit::ViewsCycle => Some(Hit::ViewsDrop),
                Hit::More => Some(Hit::More),
                _ => None,
            },
        }
    }

    /// The left commands folded into the » menu, as indices into [`Commandbar::left`].
    pub fn folded(&self, l: &Layout) -> std::ops::Range<usize> {
        l.items.len()..self.left.len()
    }
}

/// The bar's height where text grows by `cm.text`.
pub fn height(cm: &ControlMetrics) -> i32 {
    cm.text.len(HEIGHT)
}

fn item_width(item: &Item, text: &TextRenderer, px: u32, cm: &ControlMetrics) -> (i32, i32) {
    let u = cm.text;
    let label = text.measure(&item.label, false, px);
    let arrow = u.len(ARROW_GAP) + u.len(ARROW_W);
    match item.drop {
        Drop::None => (2 * u.len(PAD) + label, 0),
        Drop::Menu => (2 * u.len(PAD) + label + arrow, 0),
        Drop::Split => (2 * u.len(PAD) + label, u.len(DROP_W)),
    }
}

/// Lays `bar` out in `r`: the right end first, from its right edge, then the left commands from
/// its left edge while they fit; when one does not, it and those after it fold into » after the
/// last that fits.
pub fn layout(
    bar: &Commandbar,
    r: Rect,
    text: &TextRenderer,
    px: u32,
    cm: &ControlMetrics,
) -> Layout {
    let u = cm.text;
    let box_h = u.len(BOX_H).min(r.height);
    let y = r.y + u.len(BOX_TOP).min(r.height - box_h);
    let mut right = r.right() - u.len(TRAIL);
    let (mut views, mut pane, mut help) = (None, None, None);
    for item in bar.right.iter().rev() {
        match item {
            RightItem::PreviewPane => {
                right -= u.len(PANE_W);
                pane = Some(Rect::new(right, y, u.len(PANE_W), box_h));
            }
            RightItem::Help => {
                right -= u.len(HELP_W);
                help = Some(Rect::new(right, y, u.len(HELP_W), box_h));
            }
            RightItem::Views => {
                right -= u.len(DROP_W);
                let drop = Rect::new(right, y, u.len(DROP_W), box_h);
                right -= u.len(VIEWS_W);
                views = Some((Rect::new(right, y, u.len(VIEWS_W), box_h), drop));
            }
        }
        right -= u.len(CLEAR);
    }
    let widths: Vec<(i32, i32)> = (bar.left.iter())
        .map(|item| item_width(item, text, px, cm))
        .collect();
    let lead = r.x + u.len(LEAD);
    let end_of = |n: usize| lead + widths[..n].iter().map(|(m, d)| m + d).sum::<i32>();
    let all = widths.len();
    let shown = if end_of(all) <= right {
        all
    } else {
        (0..all)
            .rev()
            .find(|&n| end_of(n) + u.len(MORE_W) <= right)
            .unwrap_or(0)
    };
    let mut x = lead;
    let items = widths[..shown]
        .iter()
        .map(|&(main, drop)| {
            let slot = Slot {
                main: Rect::new(x, y, main, box_h),
                drop: (drop > 0).then(|| Rect::new(x + main, y, drop, box_h)),
            };
            x += main + drop;
            slot
        })
        .collect();
    let more = (shown < all).then(|| Rect::new(x, y, u.len(MORE_W), box_h));
    Layout {
        bar: r,
        items,
        more,
        views,
        pane,
        help,
    }
}

/// The part under (`x`, `y`): a box's columns over the bar's whole height. A greyed command is no
/// part.
pub fn hit(bar: &Commandbar, l: &Layout, x: i32, y: i32) -> Option<Hit> {
    if !l.bar.contains(x, y) {
        return None;
    }
    let across = |r: Rect| x >= r.x && x < r.right();
    for (i, (slot, item)) in l.items.iter().zip(&bar.left).enumerate() {
        if !item.enabled {
            continue;
        }
        if across(slot.main) {
            return Some(Hit::Item(i));
        }
        if slot.drop.is_some_and(across) {
            return Some(Hit::ItemDrop(i));
        }
    }
    if l.more.is_some_and(across) {
        return Some(Hit::More);
    }
    if let Some((cycle, drop)) = l.views {
        if across(cycle) {
            return Some(Hit::ViewsCycle);
        }
        if across(drop) {
            return Some(Hit::ViewsDrop);
        }
    }
    if l.pane.is_some_and(across) {
        return Some(Hit::PreviewPane);
    }
    l.help.filter(|&r| across(r)).map(|_| Hit::Help)
}

/// The box state `part` draws in: Pressed while held or its menu is open, Hot under the pointer,
/// Focused under the keyboard. A split command lights both its parts while either is hot, as
/// does Views.
fn state_of(bar: &Commandbar, part: Hit, twin: Option<Hit>) -> State {
    let either = |h: Option<Hit>| h.is_some_and(|h| h == part || Some(h) == twin);
    if bar.pressed == Some(part) {
        State::Pressed
    } else if either(bar.hot) || either(bar.pressed) {
        State::Hot
    } else if either(bar.focus) {
        State::Focused
    } else {
        State::Normal
    }
}

/// A look without a focused box of its own lights the keyboard's command as hot.
fn draw_box(c: &mut Canvas, p: &Paint, r: Rect, state: State) {
    if state == State::Normal {
        return;
    }
    match p.look.style() {
        Some(st) if let Some(art) = st.part(Part::CommandButton) => {
            let state = match state {
                State::Focused if !art.states.contains(&State::Focused) => State::Hot,
                state => state,
            };
            st.draw(c, r, Part::CommandButton, state, p.s);
        }
        _ => {
            let state = match state {
                State::Focused => State::Hot,
                state => state,
            };
            tb::draw_button(c, p, r, None, state)
        }
    }
}

fn ink(p: &Paint, enabled: bool) -> Color {
    let state = if enabled {
        State::Normal
    } else {
        State::Disabled
    };
    let styled = p
        .look
        .style()
        .and_then(|st| st.text_color(Part::CommandButton, state));
    styled.unwrap_or(if enabled {
        p.cs.button_text
    } else {
        p.cs.gray_text
    })
}

/// `mark` (`w` x `h` design px, a coverage function of device px at `d`) inked `ink`, centred
/// in `r` (device px), `dx` design px right of centre.
fn draw_mark(
    c: &mut Canvas,
    p: &Paint,
    r: Rect,
    (w, h): (i32, i32),
    ink: Color,
    mark: impl Fn(f64, f64, f64) -> f64,
) {
    let cs = c.scale();
    let d = p.s * cs;
    let k = f64::from(d);
    let art = shape::draw(w, h, d, |x, y| {
        shape::scale(premul(ink, 0xFF), mark(x, y, k))
    });
    let (x, y) = (
        r.x + (r.width - w * p.s) / 2,
        r.y + (r.height - h * p.s) / 2,
    );
    c.blit_over(x, y, &art.at_scale(cs));
}

fn arrow(x: f64, y: f64, k: f64) -> f64 {
    let (w, h) = (f64::from(ARROW_W) * k, f64::from(ARROW_H) * k);
    shape::polygon(x, y, &[(0.0, 0.0), (w, 0.0), (w / 2.0, h)])
}

fn chevrons(x: f64, y: f64, k: f64) -> f64 {
    let one = |dx: f64| {
        let pts = [(dx + 0.5, 0.5), (dx + 3.0, 3.0), (dx + 0.5, 5.5)].map(|(a, b)| (a * k, b * k));
        shape::stroke(x, y, &pts, 1.3 * k)
    };
    (one(0.5) + one(4.5)).min(1.0)
}

/// Draws the bar into `l.bar` (scaled by `p.s`) with labels at `px`.
pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, bar: &Commandbar, l: &Layout, px: u32) {
    let s = p.s;
    let band = l.bar.scaled(s);
    match p.look.style() {
        Some(st) if st.part(Part::CommandBar).is_some() => {
            st.draw(c, band, Part::CommandBar, State::Normal, s);
        }
        _ => tb::draw_background(c, p, band),
    }
    let pad = p.text.len(PAD) * s;
    for (i, (slot, item)) in l.items.iter().zip(&bar.left).enumerate() {
        let main = slot.main.scaled(s);
        let ink = ink(p, item.enabled);
        if item.enabled {
            let twin = slot.drop.map(|_| Hit::ItemDrop(i));
            draw_box(c, p, main, state_of(bar, Hit::Item(i), twin));
            if let Some(drop) = slot.drop {
                let st = state_of(bar, Hit::ItemDrop(i), Some(Hit::Item(i)));
                draw_box(c, p, drop.scaled(s), st);
            }
        }
        let label = Rect::new(
            main.x + pad,
            main.y,
            (main.width - 2 * pad).max(0),
            main.height,
        );
        text.draw(c, label, &item.label, false, ink, Align::Left, px);
        let arrow_at = match (item.drop, slot.drop) {
            (Drop::Split, Some(drop)) => Some(drop.scaled(s)),
            (Drop::Menu, _) => {
                let w = p.text.len(ARROW_W) * s;
                let right = main.right() - pad;
                Some(Rect::new(right - w, main.y, w, main.height))
            }
            _ => None,
        };
        if let Some(r) = arrow_at {
            draw_mark(c, p, r, (ARROW_W, ARROW_H), ink, arrow);
        }
    }
    let live = ink(p, true);
    if let Some(more) = l.more {
        let r = more.scaled(s);
        draw_box(c, p, r, state_of(bar, Hit::More, None));
        draw_mark(c, p, r, MORE_MARK, live, chevrons);
    }
    if let Some((cycle, drop)) = l.views {
        let (cycle, drop) = (cycle.scaled(s), drop.scaled(s));
        draw_box(
            c,
            p,
            cycle,
            state_of(bar, Hit::ViewsCycle, Some(Hit::ViewsDrop)),
        );
        draw_box(
            c,
            p,
            drop,
            state_of(bar, Hit::ViewsDrop, Some(Hit::ViewsCycle)),
        );
        let cs = c.scale();
        let icon = viewsicon::draw16_at(s * cs).at_scale(cs);
        let side = ICON * s;
        c.blit_over(
            cycle.x + (cycle.width - side) / 2,
            cycle.y + (cycle.height - side) / 2,
            &icon,
        );
        draw_mark(c, p, drop, (ARROW_W, ARROW_H), live, arrow);
    }
    if let Some(pane) = l.pane {
        let r = pane.scaled(s);
        let state = match state_of(bar, Hit::PreviewPane, None) {
            State::Normal | State::Hot | State::Focused if bar.pane_shown => State::Pressed,
            state => state,
        };
        draw_box(c, p, r, state);
        let cs = c.scale();
        let ink = p
            .look
            .style()
            .and_then(|st| st.text_color(Part::CommandBar, State::Normal));
        let glyph = cmdglyph::canvas_inked(CmdGlyph::PreviewPane, ICON * s, cs, ink);
        let side = ICON * s;
        c.blit_over(
            r.x + (r.width - side) / 2,
            r.y + (r.height - side) / 2,
            &glyph,
        );
    }
    if let Some(help) = l.help {
        let r = help.scaled(s);
        let side = ICON * s;
        let at = Rect::new(
            r.x + (r.width - side) / 2,
            r.y + (r.height - side) / 2,
            side,
            side,
        );
        if let Some(st) = p.look.style() {
            st.draw(c, at, Part::CommandHelp, State::Normal, s);
        }
    }
}

/// The bar's `access` part: a toolbar of buttons, those that open a menu saying so. » stands in
/// for the folded commands; the help mark is a greyed button.
pub fn access(
    bar: &Commandbar,
    l: &Layout,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let id = tree.push(parent, key, access::node(Role::Toolbar, l.bar));
    let push =
        |tree: &mut Tree, k: (u8, usize), r: Rect, label: &str, enabled: bool, menu: bool| {
            let mut n = access::button(r, label, enabled, false);
            if menu {
                n.set_has_popup(accesskit::HasPopup::Menu);
            }
            tree.push(id, k, n);
        };
    for (i, (slot, item)) in l.items.iter().zip(&bar.left).enumerate() {
        push(
            tree,
            (0, i),
            slot.main,
            &item.label,
            item.enabled,
            item.drop == Drop::Menu,
        );
        if let Some(drop) = slot.drop {
            let name = tr!("commandbar-more-options", command = item.label.as_str());
            push(tree, (1, i), drop, &name, item.enabled, true);
        }
    }
    if let Some(more) = l.more {
        push(tree, (2, 0), more, &tr!("commandbar-more"), true, true);
    }
    if let Some((cycle, drop)) = l.views {
        push(tree, (3, 0), cycle, &tr!("toolbar-views"), true, false);
        let name = tr!("commandbar-more-options", command = tr!("toolbar-views"));
        push(tree, (3, 1), drop, &name, true, true);
    }
    if let Some(r) = l.pane {
        let shown = accesskit::Toggled::from(bar.pane_shown);
        let n = access::toggle(
            Role::Button,
            r,
            &tr!("commandbar-preview-pane"),
            shown,
            true,
        );
        tree.push(id, (5, 0), n);
    }
    if let Some(help) = l.help {
        push(tree, (4, 0), help, &tr!("commandbar-help"), false, false);
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::Mode;
    use crate::interface::Interface;
    use crate::interfaces::{theme_for, theme_with};
    use crate::theme::{FontConfig, TextUnits, Theme};

    fn sample() -> Commandbar {
        Commandbar {
            left: vec![
                Item::new("Organize", 0).drop(Drop::Menu),
                Item::new("Open", 1).drop(Drop::Split),
                Item::new("New folder", 2),
                Item::new("Eject", 3).enabled(false),
            ],
            right: vec![RightItem::Views, RightItem::Help],
            ..Commandbar::default()
        }
    }

    fn laid(bar: &Commandbar, w: i32, cm: &ControlMetrics) -> Layout {
        let text = TextRenderer::load(&FontConfig::default());
        let px = cm.text.len(11) as u32;
        layout(bar, Rect::new(0, 0, w, height(cm)), &text, px, cm)
    }

    #[test]
    fn commands_run_left_to_right_and_views_and_help_hold_the_right_end() {
        use crate::glass::commandbar as art;
        let sizes = (art::BAR_H, art::BOX_H, art::HELP);
        assert_eq!((HEIGHT, BOX_H, ICON), sizes, "Glass's art fits the bar");
        let cm = ControlMetrics::CLASSIC;
        let bar = sample();
        let l = laid(&bar, 700, &cm);
        assert_eq!(l.items.len(), 4, "everything fits");
        assert_eq!(l.more, None);
        assert_eq!(l.items[0].main.x, LEAD);
        assert_eq!(
            (l.items[0].main.y, l.items[0].main.height),
            (BOX_TOP, BOX_H)
        );
        for w in l.items.windows(2) {
            let end = w[0].drop.unwrap_or(w[0].main).right();
            assert_eq!(w[1].main.x, end, "boxes side by side");
        }
        let text = TextRenderer::load(&FontConfig::default());
        let organize = text.measure("Organize", false, 11);
        assert_eq!(
            l.items[0].main.width,
            2 * PAD + organize + ARROW_GAP + ARROW_W,
            "a menu command's arrow sits in its box"
        );
        let open = l.items[1];
        assert_eq!(
            open.drop.map(|d| (d.x, d.width)),
            Some((open.main.right(), DROP_W)),
            "a split command's drop part follows its label"
        );
        let help = l.help.expect("help");
        assert_eq!((help.right(), help.width), (700 - TRAIL, HELP_W));
        let (cycle, drop) = l.views.expect("views");
        assert_eq!(drop.right() + CLEAR, help.x);
        assert_eq!((cycle.right(), cycle.width), (drop.x, VIEWS_W));
    }

    #[test]
    fn commands_past_the_bar_fold_into_the_more_menu() {
        let cm = ControlMetrics::CLASSIC;
        let mut bar = sample();
        let wide = laid(&bar, 700, &cm);
        let right_end = 700 - wide.views.unwrap().0.x + CLEAR;
        let need = |n: usize| {
            wide.items[n - 1]
                .drop
                .unwrap_or(wide.items[n - 1].main)
                .right()
        };
        // Room for Organize and Open but not New folder and Eject.
        let w = need(2) + MORE_W + right_end;
        let l = laid(&bar, w, &cm);
        assert_eq!(l.items.len(), 2);
        assert_eq!(bar.folded(&l), 2..4, "the last commands fold first");
        let more = l.more.expect("»");
        assert_eq!(more.x, need(2), "» follows the last shown command");
        assert_eq!(
            more.right() + CLEAR,
            l.views.unwrap().0.x,
            "» stays clear of Views"
        );
        assert_eq!(
            laid(&bar, w - 1, &cm).items.len(),
            1,
            "a px less folds Open too"
        );
        assert_eq!(
            l.help.unwrap().right(),
            w - TRAIL,
            "the right end always shows"
        );
        assert_eq!(hit(&bar, &l, more.x + 2, 10), Some(Hit::More));
        bar.focus = Some(Hit::More);
        assert_eq!(bar.key(Key::Activate, &l), Some(Hit::More));
        assert_eq!(bar.key(Key::Drop, &l), Some(Hit::More));

        let narrow = laid(&bar, 100, &cm);
        assert!(narrow.items.is_empty(), "nothing fits: everything folds");
        assert_eq!(bar.folded(&narrow), 0..4);
        assert!(narrow.more.is_some());

        let big = ControlMetrics::CLASSIC.grown(TextUnits::new(22, 11));
        let l = laid(&bar, 700, &big);
        assert!(l.items.len() < 4, "text at 200% folds more");
        assert_eq!(l.bar.height, 2 * HEIGHT);
        assert_eq!(l.items[0].main.height, 2 * BOX_H);
    }

    #[test]
    fn a_press_lands_on_the_part_under_it_and_skips_greyed_commands() {
        let cm = ControlMetrics::CLASSIC;
        let bar = sample();
        let l = laid(&bar, 700, &cm);
        let mid = |r: Rect| (r.x + r.width / 2, 1);
        let at = |(x, y)| hit(&bar, &l, x, y);
        assert_eq!(at(mid(l.items[0].main)), Some(Hit::Item(0)));
        assert_eq!(at(mid(l.items[1].main)), Some(Hit::Item(1)));
        assert_eq!(at(mid(l.items[1].drop.unwrap())), Some(Hit::ItemDrop(1)));
        assert_eq!(at(mid(l.items[3].main)), None, "Eject is greyed");
        assert_eq!(at(mid(l.views.unwrap().0)), Some(Hit::ViewsCycle));
        assert_eq!(at(mid(l.views.unwrap().1)), Some(Hit::ViewsDrop));
        assert_eq!(at(mid(l.help.unwrap())), Some(Hit::Help));
        let last = l.items[3].main.right();
        assert_eq!(
            at((last + 2, 5)),
            None,
            "the room between the commands and Views"
        );
        assert_eq!(hit(&bar, &l, 20, HEIGHT), None, "below the bar");
    }

    #[test]
    fn views_click_cycles_and_its_arrow_opens_the_list() {
        let bar = sample();
        let l = laid(&bar, 700, &ControlMetrics::CLASSIC);
        let (cycle, drop) = l.views.unwrap();
        assert_eq!(hit(&bar, &l, cycle.x + 1, 10), Some(Hit::ViewsCycle));
        assert_eq!(hit(&bar, &l, drop.right() - 1, 10), Some(Hit::ViewsDrop));
        let mut bar = bar;
        bar.focus = Some(Hit::ViewsCycle);
        assert_eq!(
            bar.key(Key::Activate, &l),
            Some(Hit::ViewsCycle),
            "Enter cycles"
        );
        assert_eq!(
            bar.key(Key::Drop, &l),
            Some(Hit::ViewsDrop),
            "Down opens the list"
        );
    }

    #[test]
    fn help_is_drawn_and_inert() {
        let mut bar = sample();
        let l = laid(&bar, 700, &ControlMetrics::CLASSIC);
        assert!(
            !bar.stops(&l).contains(&Hit::Help),
            "the keyboard passes it by"
        );
        bar.hot = Some(Hit::Help);
        bar.pressed = Some(Hit::Help);
        let (lit, _) = render(&bar, Mode::Light, 1);
        let (rest, _) = render(&sample(), Mode::Light, 1);
        assert_eq!(lit.pixels, rest.pixels, "no box lights under it");
        let h = l.help.unwrap();
        let mark = (h.y..h.bottom())
            .flat_map(|y| (h.x..h.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let p = rest.get(x, y);
                (p & 0xFF) > (p >> 16 & 0xFF) + 0x40
            })
            .count();
        assert!(mark > 40, "the blue mark is drawn: {mark} px");
        let mut t = Tree::window("Explorer", (700, 31));
        access(&bar, &l, &mut t, access::ROOT, "cmd");
        let help = t
            .find(Role::Button, &tr!("commandbar-help"))
            .expect("help node");
        assert!(
            t.get(help).unwrap().is_disabled(),
            "a reader hears it greyed"
        );
    }

    #[test]
    fn the_keyboard_steps_through_live_commands_then_more_and_views() {
        let mut bar = sample();
        let l = laid(&bar, 700, &ControlMetrics::CLASSIC);
        assert_eq!(
            bar.stops(&l),
            vec![Hit::Item(0), Hit::Item(1), Hit::Item(2), Hit::ViewsCycle],
            "greyed Eject and help are skipped"
        );
        assert_eq!(bar.key(Key::Next, &l), None);
        assert_eq!(bar.focus, Some(Hit::Item(0)));
        bar.key(Key::Next, &l);
        assert_eq!(
            bar.key(Key::Activate, &l),
            Some(Hit::Item(1)),
            "Enter runs Open"
        );
        assert_eq!(
            bar.key(Key::Drop, &l),
            Some(Hit::ItemDrop(1)),
            "Down opens Open with"
        );
        bar.key(Key::Last, &l);
        assert_eq!(bar.focus, Some(Hit::ViewsCycle));
        bar.key(Key::Next, &l);
        assert_eq!(bar.focus, Some(Hit::ViewsCycle), "the end holds");
        bar.key(Key::First, &l);
        assert_eq!(
            bar.key(Key::Drop, &l),
            Some(Hit::Item(0)),
            "Organize's menu"
        );
        bar.key(Key::Prev, &l);
        assert_eq!(bar.focus, Some(Hit::Item(0)));
        bar.focus = Some(Hit::Item(2));
        bar.set_left(vec![Item::new("Open", 1)]);
        assert_eq!(
            bar.focus,
            Some(Hit::Item(0)),
            "a gone command hands the keyboard on"
        );
    }

    #[test]
    fn a_reader_hears_a_toolbar_of_named_buttons() {
        let bar = sample();
        let l = laid(&bar, 700, &ControlMetrics::CLASSIC);
        let mut t = Tree::window("Explorer", (700, 31));
        let id = access(&bar, &l, &mut t, access::ROOT, "cmd");
        assert_eq!(t.get(id).unwrap().role(), Role::Toolbar);
        let org = t.find(Role::Button, "Organize").expect("Organize");
        assert_eq!(
            t.get(org).unwrap().has_popup(),
            Some(accesskit::HasPopup::Menu)
        );
        let open = t.find(Role::Button, "Open").expect("Open");
        assert_eq!(t.get(open).unwrap().has_popup(), None, "Open runs");
        let more = tr!("commandbar-more-options", command = "Open");
        assert!(
            t.find(Role::Button, &more).is_some(),
            "its drop part is named"
        );
        let eject = t.find(Role::Button, "Eject").expect("Eject");
        assert!(t.get(eject).unwrap().is_disabled());
        assert!(t.find(Role::Button, &tr!("toolbar-views")).is_some());
    }

    fn glass_in(style: crate::glass::Style, mode: Mode) -> Theme {
        let g = crate::glass::Settings {
            mode,
            style,
            ..Default::default()
        };
        theme_with(Interface::Glass, crate::blue::Variant::Blue, false, &g)
    }

    fn render(bar: &Commandbar, mode: Mode, s: i32) -> (Canvas, Layout) {
        render_in(crate::glass::Style::Glass, bar, mode, s)
    }

    fn render_in(
        style: crate::glass::Style,
        bar: &Commandbar,
        mode: Mode,
        s: i32,
    ) -> (Canvas, Layout) {
        let base = glass_in(style, mode);
        let one = TextRenderer::load(&base.fonts);
        let l = layout(
            bar,
            Rect::new(0, 0, 700, height(&base.controls())),
            &one,
            base.fonts.size_px,
            &base.controls(),
        );
        let theme = base.scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let mut c = Canvas::new((700 * s) as u32, (l.bar.height * s) as u32);
        draw(&mut c, &text, &theme.paint(), bar, &l, theme.fonts.size_px);
        (c, l)
    }

    fn states() -> Vec<(&'static str, Commandbar)> {
        let with = |f: &dyn Fn(&mut Commandbar)| {
            let mut b = sample();
            f(&mut b);
            b
        };
        vec![
            ("rest", sample()),
            ("organize hot", with(&|b| b.hot = Some(Hit::Item(0)))),
            ("organize open", with(&|b| b.pressed = Some(Hit::Item(0)))),
            ("open drop hot", with(&|b| b.hot = Some(Hit::ItemDrop(1)))),
            ("open held", with(&|b| b.pressed = Some(Hit::Item(1)))),
            ("views hot", with(&|b| b.hot = Some(Hit::ViewsCycle))),
            ("views list", with(&|b| b.pressed = Some(Hit::ViewsDrop))),
            ("focus new folder", with(&|b| b.focus = Some(Hit::Item(2)))),
            ("pane rest", with(&with_pane)),
            (
                "pane hot",
                with(&|b| {
                    with_pane(b);
                    b.hot = Some(Hit::PreviewPane);
                }),
            ),
            (
                "pane held",
                with(&|b| {
                    with_pane(b);
                    b.pressed = Some(Hit::PreviewPane);
                }),
            ),
            (
                "pane shown",
                with(&|b| {
                    with_pane(b);
                    b.pane_shown = true;
                }),
            ),
        ]
    }

    fn with_pane(b: &mut Commandbar) {
        b.right = vec![RightItem::Views, RightItem::PreviewPane, RightItem::Help];
    }

    #[test]
    fn the_pane_toggle_sits_between_views_and_help_takes_the_keyboard_and_reads_as_a_toggle() {
        let mut bar = sample();
        with_pane(&mut bar);
        let l = laid(&bar, 600, &ControlMetrics::CLASSIC);
        let (views, pane, help) = (l.views.unwrap().1, l.pane.unwrap(), l.help.unwrap());
        assert!(
            views.right() < pane.x && pane.right() < help.x,
            "{views:?} {pane:?} {help:?}"
        );
        let mid = pane.x + pane.width / 2;
        assert_eq!(hit(&bar, &l, mid, 10), Some(Hit::PreviewPane));
        assert_eq!(bar.stops(&l).last(), Some(&Hit::PreviewPane));
        bar.focus = Some(Hit::PreviewPane);
        assert_eq!(bar.key(Key::Activate, &l), Some(Hit::PreviewPane));
        bar.pane_shown = true;
        let mut t = Tree::window("", (600, 40));
        access(&bar, &l, &mut t, access::ROOT, "bar");
        let toggled = t.nodes().find_map(|(_, n)| n.toggled());
        assert_eq!(toggled, Some(accesskit::Toggled::True));
    }

    #[test]
    fn command_bar_pins_per_state_in_light_and_dark_at_scale_1_and_2() {
        crate::pins::fonts();
        let mut got = Vec::new();
        for mode in [Mode::Light, Mode::Dark] {
            for s in [1, 2] {
                for (name, bar) in states() {
                    let (c, _) = render(&bar, mode, s);
                    got.push((format!("{name} {mode:?} {s}"), crate::pins::fnv(&c)));
                }
            }
        }
        crate::pins::check("commandbar", &got);
    }

    #[test]
    fn onyx_command_bar_pins_per_state_in_light_and_dark_at_scale_1_and_2() {
        crate::pins::fonts();
        let mut got = Vec::new();
        for mode in [Mode::Light, Mode::Dark] {
            for s in [1, 2] {
                for (name, bar) in states() {
                    let (c, _) = render_in(crate::glass::Style::Onyx, &bar, mode, s);
                    got.push((format!("{name} {mode:?} {s}"), crate::pins::fnv(&c)));
                }
            }
        }
        crate::pins::check("commandbar_onyx", &got);
    }

    #[test]
    fn onyx_marks_the_keyboards_command_apart_from_a_hot_one_and_glass_lights_both_alike() {
        use crate::glass::Style;
        crate::pins::fonts();
        let (mut hot, mut keyed) = (sample(), sample());
        hot.hot = Some(Hit::Item(2));
        keyed.focus = Some(Hit::Item(2));
        let drawn = |style, bar: &Commandbar| render_in(style, bar, Mode::Light, 1).0.pixels;
        assert_eq!(drawn(Style::Glass, &hot), drawn(Style::Glass, &keyed));
        assert_ne!(drawn(Style::Onyx, &hot), drawn(Style::Onyx, &keyed));
        assert_ne!(drawn(Style::Onyx, &keyed), drawn(Style::Onyx, &sample()));
    }

    #[test]
    fn onyx_leaves_the_pane_glyphs_page_open_on_its_band() {
        use crate::glass::Style;
        let mut bar = sample();
        with_pane(&mut bar);
        let white = |style| {
            let (c, l) = render_in(style, &bar, Mode::Light, 1);
            let r = l.pane.unwrap();
            (r.y..r.bottom())
                .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == 0xFFFF_FFFF)
                .count()
        };
        assert!(white(Style::Glass) > 20, "Glass draws a white page");
        assert_eq!(white(Style::Onyx), 0, "Onyx shows its band through it");
    }

    #[test]
    fn a_hot_command_lights_a_box_and_a_greyed_one_none() {
        crate::pins::fonts();
        let (rest, l) = render(&sample(), Mode::Light, 1);
        for (name, bar) in states().into_iter().skip(1) {
            let (c, _) = render(&bar, Mode::Light, 1);
            assert_ne!(c.pixels, rest.pixels, "{name} differs from rest");
        }
        let eject = l.items[3].main;
        let band = rest.get(eject.x + 1, 20);
        let mut hot = sample();
        hot.hot = Some(Hit::Item(3));
        let (c, _) = render(&hot, Mode::Light, 1);
        assert_eq!(
            c.get(eject.x + 1, 20),
            band,
            "a greyed command lights nothing"
        );
        let ink = |c: &Canvas, r: Rect| {
            (r.y..r.bottom())
                .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
                .any(|(x, y)| c.get(x, y) == 0xFF1E_395B)
        };
        assert!(ink(&rest, l.items[0].main), "labels take the navy ink");
        assert!(!ink(&rest, eject), "a greyed label does not");
    }

    #[test]
    fn dark_mode_turns_the_band_dark_and_the_labels_light() {
        crate::pins::fonts();
        let (light, l) = render(&sample(), Mode::Light, 1);
        let (dark, _) = render(&sample(), Mode::Dark, 1);
        let lum = |p: u32| (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF);
        assert!(
            lum(dark.get(300, 20)) < lum(light.get(300, 20)) / 2,
            "a dark band"
        );
        let brightest = |c: &Canvas, r: Rect| {
            (r.y..r.bottom())
                .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
                .map(|(x, y)| lum(c.get(x, y)))
                .max()
                .unwrap()
        };
        let label = l.items[0].main;
        assert!(
            brightest(&dark, label) > lum(dark.get(300, 20)) + 300,
            "light labels on it"
        );
    }

    #[test]
    fn classic_draws_the_bar_from_its_toolbar_art() {
        let theme = theme_for(Interface::Classic);
        let text = TextRenderer::load(&theme.fonts);
        let mut bar = sample();
        bar.hot = Some(Hit::Item(0));
        let l = layout(
            &bar,
            Rect::new(0, 0, 700, HEIGHT),
            &text,
            11,
            &theme.controls(),
        );
        let mut c = Canvas::new(700, HEIGHT as u32);
        draw(&mut c, &text, &theme.paint(), &bar, &l, 11);
        assert_eq!(c.get(300, 20), theme.colors.button_face.argb());
        let o = l.items[0].main;
        assert_eq!(
            c.get(o.x, o.y),
            theme.colors.button_hilight.argb(),
            "a raised box"
        );
    }
}
