//! Back and Forward as one pill of two round buttons, then the drop that lists the recent places.
//!
//! Back and Forward fire when the button comes up over the one it went down on; the drop opens
//! on the press. A look without the round buttons' art draws raised arrow buttons in their place.
//! The control grows by the text's whole multiple ([`TextUnits::small`]), drawn afresh at it.
use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::Arrow;
use crate::look::Paint;
use crate::look::widgets::draw_arrow_button;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State};
use crate::theme::TextUnits;
use accesskit::{NodeId, Role};

/// The pill's size at 100% text; Back's and Forward's round buttons sit inside it,
/// [`ORB_PITCH`] apart.
pub const PILL_W: i32 = 57;
pub const PILL_H: i32 = 29;
pub const ORB: i32 = 25;
pub const ORB_PITCH: i32 = 28;
/// From the pill's left and top edges to Back's.
pub const ORB_INSET: i32 = 2;
pub const RECENT_W: i32 = 15;
pub const RECENT_H: i32 = 21;

/// The whole control's width at text `u`, pill and drop.
pub fn width(u: TextUnits) -> i32 {
    u.small() * (PILL_W + RECENT_W)
}

/// The pill's height at text `u`.
pub fn height(u: TextUnits) -> i32 {
    u.small() * PILL_H
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Back,
    Forward,
    Recent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Back,
    Forward,
    /// The drop went down: the host opens the recent places under [`Layout::recent`].
    OpenRecent,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavButtons {
    pub can_back: bool,
    pub can_forward: bool,
    pub hot: Option<Hit>,
    pub pressed: Option<Hit>,
    /// The recent places are open, drawn pressed until [`NavButtons::menu_closed`].
    pub open: bool,
}

impl NavButtons {
    fn enabled(&self, h: Hit) -> bool {
        match h {
            Hit::Back => self.can_back,
            Hit::Forward => self.can_forward,
            Hit::Recent => self.can_back || self.can_forward,
        }
    }

    pub fn press(&mut self, hit: Option<Hit>) -> Event {
        let Some(h) = hit.filter(|&h| self.enabled(h)) else {
            return Event::None;
        };
        if h == Hit::Recent {
            self.open = true;
            return Event::OpenRecent;
        }
        self.pressed = Some(h);
        Event::Redraw
    }

    pub fn release(&mut self, hit: Option<Hit>) -> Event {
        match self.pressed.take() {
            Some(h) if Some(h) == hit && self.enabled(h) => match h {
                Hit::Back => Event::Back,
                Hit::Forward => Event::Forward,
                Hit::Recent => Event::None,
            },
            Some(_) => Event::Redraw,
            None => Event::None,
        }
    }

    /// Returns whether the lit part changed.
    pub fn hover(&mut self, hit: Option<Hit>) -> bool {
        std::mem::replace(&mut self.hot, hit) != hit
    }

    pub fn leave(&mut self) -> bool {
        self.hover(None)
    }

    pub fn menu_closed(&mut self) {
        self.open = false;
    }

    fn state(&self, h: Hit) -> State {
        let held = self.pressed == Some(h) || (h == Hit::Recent && self.open);
        if !self.enabled(h) {
            State::Disabled
        } else if held && self.hot.is_none_or(|hot| hot == h) {
            State::Pressed
        } else if self.hot == Some(h) || held {
            State::Hot
        } else {
            State::Normal
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub pill: Rect,
    pub back: Rect,
    pub forward: Rect,
    pub recent: Rect,
}

impl Layout {
    /// The control's whole width, pill and drop.
    pub fn frame(&self) -> Rect {
        Rect::new(
            self.pill.x,
            self.pill.y,
            self.recent.right() - self.pill.x,
            self.pill.height,
        )
    }
}

/// The control at the left end of `row`, centred down it, at text `u`.
pub fn layout(row: Rect, u: TextUnits) -> Layout {
    let k = u.small();
    let mid = row.y + row.height / 2;
    let pill = Rect::new(row.x, mid - k * (PILL_H / 2), k * PILL_W, k * PILL_H);
    let orb = |i: i32| {
        Rect::new(
            pill.x + k * (ORB_INSET + i * ORB_PITCH),
            pill.y + k * ORB_INSET,
            k * ORB,
            k * ORB,
        )
    };
    Layout {
        pill,
        back: orb(0),
        forward: orb(1),
        recent: Rect::new(
            pill.right(),
            mid - k * (RECENT_H / 2),
            k * RECENT_W,
            k * RECENT_H,
        ),
    }
}

/// The round buttons answer inside their circles; the drop over its whole box.
pub fn hit(lay: &Layout, x: i32, y: i32) -> Option<Hit> {
    let round = |r: Rect| {
        let (dx, dy) = (2 * (x - r.x) + 1 - r.width, 2 * (y - r.y) + 1 - r.height);
        dx * dx + dy * dy <= r.width * r.width
    };
    if round(lay.back) {
        Some(Hit::Back)
    } else if round(lay.forward) {
        Some(Hit::Forward)
    } else {
        lay.recent.contains(x, y).then_some(Hit::Recent)
    }
}

pub fn draw(c: &mut Canvas, p: &Paint, nb: &NavButtons, lay: &Layout) {
    let s = p.s;
    let look = p.small_look(c);
    let grown = p.small(&look);
    let parts = [
        Part::NavPill,
        Part::NavBack,
        Part::NavForward,
        Part::NavRecent,
    ];
    match grown.look.style().filter(|st| st.has_parts(&parts)) {
        Some(st) => {
            let d = grown.s;
            st.draw(c, lay.pill.scaled(s), Part::NavPill, State::Normal, d);
            for (h, part, r) in [
                (Hit::Back, Part::NavBack, lay.back),
                (Hit::Forward, Part::NavForward, lay.forward),
                (Hit::Recent, Part::NavRecent, lay.recent),
            ] {
                st.draw(c, r.scaled(s), part, nb.state(h), d);
            }
        }
        None => {
            for (h, arrow, r) in [
                (Hit::Back, Arrow::Left, lay.back),
                (Hit::Forward, Arrow::Right, lay.forward),
                (Hit::Recent, Arrow::Down, lay.recent),
            ] {
                draw_arrow_button(c, p, r.scaled(s), arrow, nb.enabled(h));
            }
        }
    }
}

/// Back, Forward and Recent Pages as buttons under one toolbar node, which it returns.
pub fn access(nb: &NavButtons, lay: &Layout, tree: &mut Tree, parent: NodeId) -> NodeId {
    let bar = tree.push(
        parent,
        "navigation",
        access::named(Role::Toolbar, lay.frame(), &tr!("navbuttons-bar")),
    );
    for (h, key, label) in [
        (Hit::Back, "back", tr!("toolbar-back")),
        (Hit::Forward, "forward", tr!("toolbar-forward")),
        (Hit::Recent, "recent", tr!("navbuttons-recent")),
    ] {
        let r = match h {
            Hit::Back => lay.back,
            Hit::Forward => lay.forward,
            Hit::Recent => lay.recent,
        };
        tree.push(bar, key, access::button(r, &label, nb.enabled(h), false));
    }
    bar
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::pins::fnv;

    const ROW: Rect = Rect::new(2, 0, 400, 34);

    fn nav(back: bool, forward: bool) -> NavButtons {
        NavButtons {
            can_back: back,
            can_forward: forward,
            ..NavButtons::default()
        }
    }

    #[test]
    fn back_and_forward_sit_in_the_pill_with_the_drop_after_it() {
        let lay = layout(ROW, TextUnits::ONE);
        assert_eq!(lay.pill, Rect::new(2, 3, 57, 29));
        assert_eq!(lay.back, Rect::new(4, 5, 25, 25));
        assert_eq!(lay.forward, Rect::new(32, 5, 25, 25));
        assert_eq!(lay.recent, Rect::new(59, 7, 15, 21));
        assert_eq!(lay.frame().width, width(TextUnits::ONE));
    }

    #[test]
    fn at_200_percent_text_the_pill_orbs_and_drop_are_twice_as_large() {
        let u = TextUnits::new(200, 100);
        let lay = layout(Rect::new(2, 0, 400, 68), u);
        assert_eq!(lay.pill, Rect::new(2, 6, 114, 58));
        assert_eq!(lay.back, Rect::new(6, 10, 50, 50));
        assert_eq!(lay.forward, Rect::new(62, 10, 50, 50));
        assert_eq!(lay.recent, Rect::new(116, 14, 30, 42));
        assert_eq!(lay.frame().width, width(u));
        assert_eq!(height(u), 58);
        assert_eq!(
            layout(ROW, TextUnits::new(150, 100)),
            layout(ROW, TextUnits::ONE)
        );
    }

    #[test]
    fn the_round_buttons_answer_inside_their_circles() {
        let lay = layout(ROW, TextUnits::ONE);
        assert_eq!(hit(&lay, 16, 17), Some(Hit::Back));
        assert_eq!(hit(&lay, 44, 17), Some(Hit::Forward));
        assert_eq!(hit(&lay, 66, 17), Some(Hit::Recent));
        assert_eq!(hit(&lay, 4, 5), None, "Back's box corner is off its circle");
        assert_eq!(hit(&lay, 100, 17), None);
    }

    #[test]
    fn back_fires_on_a_release_over_itself_and_a_disabled_forward_never() {
        let mut nb = nav(true, false);
        assert_eq!(nb.press(Some(Hit::Back)), Event::Redraw);
        assert_eq!(nb.state(Hit::Back), State::Pressed);
        assert_eq!(nb.release(Some(Hit::Back)), Event::Back);
        assert_eq!(nb.press(Some(Hit::Back)), Event::Redraw);
        assert_eq!(nb.release(None), Event::Redraw, "released off it");
        assert_eq!(nb.press(Some(Hit::Forward)), Event::None);
        assert_eq!(nb.release(Some(Hit::Forward)), Event::None);
        assert_eq!(nb.state(Hit::Forward), State::Disabled);
    }

    #[test]
    fn the_drop_opens_on_the_press_and_stays_held_until_its_menu_closes() {
        let mut nb = nav(true, false);
        assert_eq!(nb.press(Some(Hit::Recent)), Event::OpenRecent);
        assert_eq!(nb.state(Hit::Recent), State::Pressed);
        nb.menu_closed();
        assert_eq!(nb.state(Hit::Recent), State::Normal);
        let mut fresh = nav(false, false);
        assert_eq!(fresh.press(Some(Hit::Recent)), Event::None, "no places yet");
    }

    #[test]
    fn hover_lights_one_part_and_reports_changes() {
        let mut nb = nav(true, true);
        assert!(nb.hover(Some(Hit::Forward)));
        assert!(!nb.hover(Some(Hit::Forward)));
        assert_eq!(nb.state(Hit::Forward), State::Hot);
        assert!(nb.leave());
        assert_eq!(nb.state(Hit::Forward), State::Normal);
    }

    #[test]
    fn access_names_back_forward_and_recent_pages() {
        let lay = layout(ROW, TextUnits::ONE);
        let mut t = Tree::window("Explorer", (400, 40));
        let bar = access(&nav(true, false), &lay, &mut t, access::ROOT);
        assert_eq!(t.get(bar).map(|n| n.role()), Some(Role::Toolbar));
        let back = t.find(Role::Button, "Back").expect("Back");
        let fwd = t.find(Role::Button, "Forward").expect("Forward");
        t.find(Role::Button, "Recent Pages").expect("Recent Pages");
        assert!(!t.get(back).unwrap().is_disabled());
        assert!(t.get(fwd).unwrap().is_disabled(), "nothing ahead");
    }

    /// The glass's own blue, which the row lies over.
    const GLASS: crate::theme::Color = crate::theme::Color::rgb(0x23, 0x60, 0x9C);

    fn render(interface: Interface, dark: bool, nb: &NavButtons, d: i32) -> Canvas {
        render_at(interface, dark, nb, d, 100)
    }

    fn render_at(interface: Interface, dark: bool, nb: &NavButtons, d: i32, pct: u32) -> Canvas {
        let g = crate::glass::Settings {
            mode: match dark {
                true => crate::glass::Mode::Dark,
                false => crate::glass::Mode::Light,
            },
            ..crate::glass::Settings::default()
        };
        let theme = crate::interfaces::theme_with(interface, Default::default(), false, &g);
        let theme = theme.with_text_size(pct).scaled(d);
        let k = theme.text.small();
        let (w, h) = (80 * k, 34 * k);
        let mut c = Canvas::new((w * d) as u32, (h * d) as u32);
        c.fill_rect(c.bounds(), GLASS);
        let lay = layout(Rect::new(2, 0, w - 2, h), theme.text);
        draw(&mut c, &theme.paint(), nb, &lay);
        c
    }

    #[test]
    fn the_buttons_pin_per_state_light_and_dark_at_scale_1_and_2() {
        crate::pins::fonts();
        let mut hot = nav(true, true);
        hot.hot = Some(Hit::Back);
        let mut held = nav(true, false);
        held.pressed = Some(Hit::Back);
        let mut open = nav(true, false);
        open.open = true;
        let states = [
            ("rest", nav(true, false)),
            ("none", nav(false, false)),
            ("hot", hot),
            ("held", held),
            ("open", open),
        ];
        let mut got = Vec::new();
        for (look, dark) in [("glass", false), ("glass-dark", true)] {
            for (name, nb) in &states {
                for d in [1, 2] {
                    let c = render(Interface::Glass, dark, nb, d);
                    got.push((format!("{look} {name} @{d}"), fnv(&c)));
                }
            }
        }
        got.push((
            "classic rest @1".to_string(),
            fnv(&render(Interface::Classic, false, &nav(true, false), 1)),
        ));
        for d in [1, 2] {
            let c = render_at(Interface::Glass, false, &nav(true, false), d, 200);
            got.push((format!("glass rest 200% @{d}"), fnv(&c)));
        }
        crate::pins::check("navbuttons", &got);
    }
}
