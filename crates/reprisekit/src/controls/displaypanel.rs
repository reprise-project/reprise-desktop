//! The displays flyout: a brightness slider per display, the night light check box and a Settings... link.

use crate::access::{self, Tree, accesskit};
use crate::art::icons::Icon;
use crate::controls::osd::{self, Level};
use crate::flyout;
use crate::look::widgets::{
    Interaction, draw_checkbox, draw_checkbox_disabled, draw_focus_ring, draw_link, draw_trackbar,
};
use crate::metrics::ControlMetrics;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;
use crate::trackbar::{self, TrackBar};
use accesskit::{NodeId, Role};

pub struct Row {
    pub label: String,
    pub icon: &'static Icon,
    pub level: Level,
}

/// The night light line: `enabled` in the config, `on` (warming, not paused), and the text on its right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Night {
    pub enabled: bool,
    pub on: bool,
    pub state: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Row(usize),
    Night,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Slider(usize, trackbar::Hit),
    Row(usize),
    Night,
    Settings,
    Panel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Tab,
    BackTab,
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Space,
    Enter,
    Escape,
}

/// What the host does after an input: `Set` a display's level (already shown), `Step` one by
/// wheel notches at the host's step, tick or untick night light, open Settings, or close.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Set { row: usize, level: u8 },
    Step { row: usize, steps: i32 },
    Night(bool),
    Settings,
    Close,
}

pub struct DisplayPanel {
    pub rows: Vec<Row>,
    pub night: Night,
    pub focus: Focus,
    /// The part under the pointer, set by the host from [`hit`].
    pub hot: Hit,
    held: Option<usize>,
    /// Whether the keyboard has moved the focus, so focus rectangles show.
    cues: bool,
}

pub const BODY_W: i32 = 314;
pub const SETTINGS: &str = "Settings...";
pub const NIGHT_LIGHT: &str = "Night light";

const PAD: i32 = 6;
const LINE: i32 = 13;
const INSET: i32 = 4;
const ICON: i32 = 16;
const ICON_GAP: i32 = 10;
const LABEL_GAP: i32 = 10;
const PERCENT_W: i32 = 34;
const ABOVE: i32 = 10;
const SLIDER_GAP: i32 = 4;
const BELOW: i32 = 12;
const SEPARATOR_H: i32 = 2;
const BOX: i32 = 13;
const BOX_GAP: i32 = 6;
const FOOT: i32 = 4;
const RING_PAD: i32 = 1;

/// The slider for a display at `level`: 0 to 100, paging by 10, no ticks.
fn track(level: u8) -> TrackBar {
    TrackBar::new(level.into(), 101).untick().paged(10, 10)
}

impl DisplayPanel {
    /// The focus starts on the first adjustable display, else the check box, else Settings.
    pub fn new(rows: Vec<Row>, night: Night) -> DisplayPanel {
        let mut p = DisplayPanel {
            rows,
            night,
            focus: Focus::Settings,
            hot: Hit::Panel,
            held: None,
            cues: false,
        };
        p.focus = p.first_focus();
        p
    }

    fn focusable(&self) -> Vec<Focus> {
        let rows = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r.level, Level::At(_)))
            .map(|(i, _)| Focus::Row(i));
        rows.chain(self.night.enabled.then_some(Focus::Night))
            .chain([Focus::Settings])
            .collect()
    }

    fn first_focus(&self) -> Focus {
        self.focusable()[0]
    }

    fn is_at(&self, i: usize) -> bool {
        self.rows
            .get(i)
            .is_some_and(|r| matches!(r.level, Level::At(_)))
    }

    fn adjustable(&self, label: &str) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| r.label == label && matches!(r.level, Level::At(_)))
    }

    /// New rows from the host. The focus and a held slider follow their display by label; a held
    /// display that went or stopped being adjustable ends the drag.
    pub fn set_rows(&mut self, rows: Vec<Row>) {
        let label = |i: Option<usize>| i.and_then(|i| self.rows.get(i)).map(|r| r.label.clone());
        let focused = label(match self.focus {
            Focus::Row(i) => Some(i),
            _ => None,
        });
        let held = label(self.held);
        self.rows = rows;
        self.held = held.and_then(|l| self.adjustable(&l));
        if let Focus::Row(_) = self.focus {
            self.focus = focused
                .and_then(|l| self.adjustable(&l))
                .map_or_else(|| self.first_focus(), Focus::Row);
        }
    }

    pub fn set_night(&mut self, night: Night) {
        self.night = night;
        if self.focus == Focus::Night && !self.night.enabled {
            self.focus = self.first_focus();
        }
    }

    pub fn held(&self) -> Option<usize> {
        self.held
    }

    pub fn key(&mut self, k: Key) -> Event {
        match (k, self.focus) {
            (Key::Escape, _) => Event::Close,
            (Key::Tab | Key::BackTab, _) => {
                let list = self.focusable();
                let n = list.len();
                let at = list.iter().position(|&f| f == self.focus);
                let next = match (at, k) {
                    (None, _) => list[0],
                    (Some(_), _) if n == 1 => return Event::None,
                    (Some(i), Key::Tab) => list[(i + 1) % n],
                    (Some(i), _) => list[(i + n - 1) % n],
                };
                self.focus = next;
                self.cues = true;
                Event::Redraw
            }
            (_, Focus::Row(i)) => {
                let Some(Level::At(v)) = self.rows.get(i).map(|r| r.level) else {
                    return Event::None;
                };
                let v = i32::from(v);
                let to = match k {
                    Key::Right | Key::Up => v + 1,
                    Key::Left | Key::Down => v - 1,
                    Key::PageUp => v + 10,
                    Key::PageDown => v - 10,
                    Key::Home => 0,
                    Key::End => 100,
                    _ => return Event::None,
                };
                self.set_level(i, to.clamp(0, 100) as u8)
            }
            (Key::Space | Key::Enter, Focus::Night) => self.toggle_night(),
            (Key::Space | Key::Enter, Focus::Settings) => Event::Settings,
            _ => Event::None,
        }
    }

    fn set_level(&mut self, row: usize, level: u8) -> Event {
        let r = &mut self.rows[row];
        if r.level == Level::At(level) {
            return Event::None;
        }
        r.level = Level::At(level);
        Event::Set { row, level }
    }

    fn toggle_night(&mut self) -> Event {
        if !self.night.enabled {
            return Event::None;
        }
        self.night.on = !self.night.on;
        Event::Night(self.night.on)
    }

    /// A press at (`x`, `y`): a channel press jumps the thumb there and holds it, a thumb press
    /// holds it where it is.
    pub fn press(&mut self, lay: &Layout, x: i32, y: i32) -> Event {
        match hit(lay, x, y) {
            Hit::Slider(i, part) if self.is_at(i) => {
                self.focus = Focus::Row(i);
                self.held = Some(i);
                let jump = match (part, &lay.rows[i].slider) {
                    (trackbar::Hit::Thumb, _) | (_, None) => Event::None,
                    (_, Some(s)) => self.set_level(i, trackbar::position_at(s, x) as u8),
                };
                match jump {
                    Event::None => Event::Redraw,
                    e => e,
                }
            }
            Hit::Night if self.night.enabled => {
                self.focus = Focus::Night;
                self.toggle_night()
            }
            Hit::Settings => Event::Settings,
            _ => Event::None,
        }
    }

    /// The pointer at `x` while a slider is held.
    pub fn drag(&mut self, lay: &Layout, x: i32) -> Event {
        let Some(i) = self.held else {
            return Event::None;
        };
        match lay.rows.get(i).and_then(|r| r.slider.as_ref()) {
            Some(s) => self.set_level(i, trackbar::position_at(s, x) as u8),
            None => Event::None,
        }
    }

    pub fn release(&mut self) {
        self.held = None;
    }

    /// `steps` wheel notches at (`x`, `y`), for the adjustable display whose row is there.
    pub fn wheel(&mut self, lay: &Layout, x: i32, y: i32, steps: i32) -> Event {
        match hit(lay, x, y) {
            Hit::Slider(i, _) | Hit::Row(i) if steps != 0 && self.is_at(i) => {
                Event::Step { row: i, steps }
            }
            _ => Event::None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowLayout {
    /// The display's whole band, the wheel's target.
    pub block: Rect,
    pub icon: Rect,
    pub label: Rect,
    /// The percentage, or "Not adjustable" on a display that is not.
    pub percent: Rect,
    pub slider: Option<trackbar::Layout>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub size: (i32, i32),
    pub rows: Vec<RowLayout>,
    pub separator: Rect,
    pub night_box: Rect,
    pub night_label: Rect,
    pub night_state: Rect,
    pub settings: Rect,
}

/// The panel's parts at scale 1, for controls `cm` and text at `px`.
pub fn layout(cm: &ControlMetrics, panel: &DisplayPanel, text: &TextRenderer, px: u32) -> Layout {
    let u = cm.text;
    let e = cm.flyout_border;
    let line = u.len(LINE);
    let w = u.len(BODY_W) + 2 * (e + PAD);
    let left = e + PAD + u.len(INSET);
    let right = w - (e + PAD) - u.len(INSET);
    let mut y = e + PAD;
    let rows = panel
        .rows
        .iter()
        .map(|row| {
            let fixed = row.level == Level::Fixed;
            let h = if fixed {
                ABOVE + line + BELOW
            } else {
                ABOVE + line + SLIDER_GAP + trackbar::HEIGHT + BELOW
            };
            let block = Rect::new(0, y, w, h);
            let ly = y + ABOVE;
            y += h;
            let icon = Rect::new(left, ly + (line - ICON) / 2, ICON, ICON);
            let pw = if fixed {
                text.measure(osd::NOT_ADJUSTABLE, false, px)
            } else {
                u.len(PERCENT_W)
            };
            let percent = Rect::new(right - pw, ly, pw, line);
            let lx = icon.right() + u.len(ICON_GAP);
            let label = Rect::new(lx, ly, (percent.x - u.len(LABEL_GAP) - lx).max(0), line);
            let slider = match row.level {
                Level::At(v) => Some(trackbar::layout(
                    &track(v),
                    lx,
                    label.bottom() + SLIDER_GAP,
                    right - lx,
                )),
                _ => None,
            };
            RowLayout {
                block,
                icon,
                label,
                percent,
                slider,
            }
        })
        .collect();
    y += ABOVE;
    let separator = Rect::new(left, y, right - left, SEPARATOR_H);
    y += SEPARATOR_H + ABOVE;
    let night_box = Rect::new(left, y + (line - BOX) / 2, BOX, BOX);
    let nx = night_box.right() + u.len(BOX_GAP);
    let night_label = Rect::new(nx, y, text.measure(NIGHT_LIGHT, false, px), line);
    let sw = text.measure(&panel.night.state, false, px);
    let night_state = Rect::new(right - sw, y, sw, line);
    y += line + ABOVE;
    let lw = text.measure(SETTINGS, false, px);
    let settings = Rect::new(right - lw, y, lw, line);
    y += line + FOOT + e + PAD;
    Layout {
        size: (w, y),
        rows,
        separator,
        night_box,
        night_label,
        night_state,
        settings,
    }
}

pub fn hit(lay: &Layout, x: i32, y: i32) -> Hit {
    for (i, r) in lay.rows.iter().enumerate() {
        if let Some(h) = r.slider.as_ref().and_then(|s| trackbar::hit(s, x, y)) {
            return Hit::Slider(i, h);
        }
        if r.block.contains(x, y) {
            return Hit::Row(i);
        }
    }
    if lay.night_box.contains(x, y) || lay.night_label.contains(x, y) {
        Hit::Night
    } else if lay.settings.contains(x, y) {
        Hit::Settings
    } else {
        Hit::Panel
    }
}

/// The panel at `theme`'s scale on the flyout's panel, with the look's shadow round it as
/// [`flyout::shadowed`] gives.
pub fn draw(theme: &Theme, text: &TextRenderer, panel: &DisplayPanel) -> Canvas {
    let p = &theme.paint();
    let (cs, s) = (p.cs, p.s);
    let px = theme.fonts.size_px;
    let lay = layout(&theme.controls(), panel, text, px / s as u32);
    let (w, h) = lay.size;
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    let all = c.bounds();
    flyout::draw_panel(
        theme,
        &mut c,
        all,
        cs.button_face,
        flyout::Backdrop::Blurred,
    );
    let ring = |c: &mut Canvas, r: Rect| {
        draw_focus_ring(c, p, r.inset(-RING_PAD).scaled(s), cs.button_text);
    };
    // The layout measures text at scale 1, and a run at scale 2 can be wider than twice that: each
    // run is laid out again at the device size and kept on its rect's left or right edge.
    let fit = |r: Rect, t: &str, right: bool| {
        let (r, w) = (r.scaled(s), text.measure(t, false, px));
        Rect::new(if right { r.right() - w } else { r.x }, r.y, w, r.height)
    };
    let put = |c: &mut Canvas, r: Rect, t: &str, ink, right: bool| {
        text.draw(c, fit(r, t, right), t, false, ink, Align::Left, px);
    };
    for (i, (row, rl)) in panel.rows.iter().zip(&lay.rows).enumerate() {
        row.icon.paint(&mut c, rl.icon.x * s, rl.icon.y * s, s);
        let label_ink = if row.level == Level::Fixed {
            cs.gray_text
        } else {
            cs.button_text
        };
        let lr = rl.label.scaled(s);
        let label = text.ellipsize(&row.label, lr.width, false, px);
        text.draw(&mut c, lr, &label, false, label_ink, Align::Left, px);
        match row.level {
            Level::At(v) => put(&mut c, rl.percent, &format!("{v}%"), cs.button_text, true),
            Level::Fixed => put(&mut c, rl.percent, osd::NOT_ADJUSTABLE, cs.gray_text, true),
            Level::Unread => {}
        }
        if let Some(sl) = &rl.slider {
            let thumb = if panel.held == Some(i) {
                Interaction::Pressed
            } else if panel.hot == Hit::Slider(i, trackbar::Hit::Thumb) {
                Interaction::Hot
            } else {
                Interaction::Idle
            };
            let focused = panel.cues && panel.focus == Focus::Row(i);
            draw_trackbar(&mut c, p, sl, true, focused, thumb);
            if focused {
                ring(&mut c, sl.bounds);
            }
        }
    }
    let sep = lay.separator.scaled(s);
    c.etched_divider_t(sep.x, sep.right() - s, sep.y, cs, s);
    let night = &panel.night;
    let b = lay.night_box.scaled(s);
    let night_ink = if night.enabled {
        let ptr = if panel.hot == Hit::Night {
            Interaction::Hot
        } else {
            Interaction::Idle
        };
        draw_checkbox(&mut c, p, b, night.on, ptr);
        cs.button_text
    } else {
        draw_checkbox_disabled(&mut c, p, b, night.on);
        cs.gray_text
    };
    put(&mut c, lay.night_label, NIGHT_LIGHT, night_ink, false);
    put(&mut c, lay.night_state, &night.state, cs.gray_text, true);
    let link = fit(lay.settings, SETTINGS, true);
    draw_link(&mut c, text, p, link, SETTINGS, true, px);
    if panel.cues {
        match panel.focus {
            Focus::Night => ring(&mut c, lay.night_label),
            Focus::Settings => ring(&mut c, lay.settings),
            Focus::Row(_) => {}
        }
    }
    flyout::shadowed(theme, c)
}

/// The panel's `access` part: a slider per adjustable display named by its label, a label for
/// the rest, the night light check box and the Settings... link. Returns the focused node.
pub fn access(panel: &DisplayPanel, lay: &Layout, tree: &mut Tree, parent: NodeId) -> NodeId {
    let mut focused = parent;
    for (i, (row, rl)) in panel.rows.iter().zip(&lay.rows).enumerate() {
        let line = Rect::new(
            rl.label.x,
            rl.label.y,
            rl.percent.right() - rl.label.x,
            rl.label.height,
        );
        let id = match (row.level, &rl.slider) {
            (Level::At(v), Some(sl)) => {
                let id = trackbar::access(&track(v), sl, tree, parent, row.label.as_str());
                if let Some(n) = tree.get_mut(id) {
                    n.set_label(row.label.as_str());
                }
                id
            }
            (Level::Fixed, _) => {
                let mut n = access::node(Role::Label, line);
                n.set_label(format!("{}: {}", row.label, osd::NOT_ADJUSTABLE));
                n.set_disabled();
                tree.push(parent, row.label.as_str(), n)
            }
            _ => {
                let mut n = access::node(Role::Label, line);
                n.set_label(row.label.as_str());
                tree.push(parent, row.label.as_str(), n)
            }
        };
        if panel.focus == Focus::Row(i) {
            focused = id;
        }
    }
    let night = &panel.night;
    let b = lay.night_box;
    let r = Rect::new(b.x, b.y, lay.night_label.right() - b.x, b.height);
    let toggled = if night.on {
        accesskit::Toggled::True
    } else {
        accesskit::Toggled::False
    };
    let mut n = access::toggle(Role::CheckBox, r, NIGHT_LIGHT, toggled, night.enabled);
    if !night.state.is_empty() {
        n.set_description(night.state.as_str());
    }
    let id = tree.push(parent, "night", n);
    if panel.focus == Focus::Night {
        focused = id;
    }
    let mut n = access::named(Role::Link, lay.settings, SETTINGS);
    access::focusable(&mut n, true);
    let id = tree.push(parent, "settings", n);
    if panel.focus == Focus::Settings {
        focused = id;
    }
    focused
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::ROOT;
    use crate::art::icons;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::raster::{Canvas, Rect};
    use crate::theme::{FontConfig, TextUnits, Theme};

    const LABELS: [&str; 4] = ["Built-in display", "DELL U2720Q", "LG 27GL850", "AOC 24G2"];

    fn rows(levels: &[Level]) -> Vec<Row> {
        levels
            .iter()
            .enumerate()
            .map(|(i, &level)| Row {
                label: LABELS[i].into(),
                icon: if i == 0 {
                    &icons::LAPTOP
                } else {
                    &icons::MONITOR
                },
                level,
            })
            .collect()
    }

    fn night(enabled: bool, on: bool) -> Night {
        Night {
            enabled,
            on,
            state: "until 07:00".into(),
        }
    }

    fn text() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    fn any_px(c: &Canvas, r: Rect, f: impl Fn(u32) -> bool) -> bool {
        (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| f(c.get(x, y))))
    }

    #[test]
    fn tab_walks_the_adjustable_sliders_then_the_check_box_then_settings() {
        let mut p = DisplayPanel::new(
            rows(&[Level::At(60), Level::Fixed, Level::At(40)]),
            night(true, true),
        );
        assert_eq!(p.focus, Focus::Row(0));
        for want in [Focus::Row(2), Focus::Night, Focus::Settings, Focus::Row(0)] {
            assert_eq!(p.key(Key::Tab), Event::Redraw);
            assert_eq!(p.focus, want);
        }
        p.key(Key::BackTab);
        assert_eq!(p.focus, Focus::Settings);
        let mut off = DisplayPanel::new(rows(&[Level::Fixed]), night(false, false));
        assert_eq!(off.focus, Focus::Settings, "nothing else takes the focus");
        assert_eq!(off.key(Key::Tab), Event::None);
    }

    #[test]
    fn arrows_step_one_percent_pages_ten_home_and_end_go_to_the_ends() {
        let mut p = DisplayPanel::new(rows(&[Level::At(60)]), night(true, true));
        assert_eq!(p.key(Key::Right), Event::Set { row: 0, level: 61 });
        assert_eq!(p.key(Key::Down), Event::Set { row: 0, level: 60 });
        assert_eq!(p.key(Key::Up), Event::Set { row: 0, level: 61 });
        assert_eq!(p.key(Key::Left), Event::Set { row: 0, level: 60 });
        assert_eq!(p.key(Key::PageUp), Event::Set { row: 0, level: 70 });
        assert_eq!(p.key(Key::PageDown), Event::Set { row: 0, level: 60 });
        assert_eq!(p.key(Key::Home), Event::Set { row: 0, level: 0 });
        assert_eq!(p.key(Key::End), Event::Set { row: 0, level: 100 });
        assert_eq!(p.key(Key::End), Event::None);
        assert_eq!(p.rows[0].level, Level::At(100));
        assert_eq!(p.key(Key::Space), Event::None, "a slider takes no Space");
        assert_eq!(p.key(Key::Escape), Event::Close);
    }

    #[test]
    fn the_check_box_ticks_and_unticks_night_light_and_settings_opens() {
        let mut p = DisplayPanel::new(rows(&[Level::At(60)]), night(true, true));
        p.focus = Focus::Night;
        assert_eq!(p.key(Key::Space), Event::Night(false));
        assert_eq!(p.key(Key::Space), Event::Night(true));
        p.focus = Focus::Settings;
        assert_eq!(p.key(Key::Enter), Event::Settings);
        let text = text();
        let lay = layout(&ControlMetrics::CLASSIC, &p, &text, 11);
        let b = lay.night_box;
        assert_eq!(p.press(&lay, b.x + 3, b.y + 3), Event::Night(false));
        assert_eq!(p.focus, Focus::Night);
        let mut off = DisplayPanel::new(rows(&[Level::At(60)]), night(false, false));
        let lay = layout(&ControlMetrics::CLASSIC, &off, &text, 11);
        let b = lay.night_box;
        assert_eq!(
            off.press(&lay, b.x + 3, b.y + 3),
            Event::None,
            "greyed while night light is off"
        );
    }

    #[test]
    fn a_press_on_the_channel_jumps_the_thumb_and_a_drag_follows() {
        let text = text();
        let mut p = DisplayPanel::new(rows(&[Level::At(60), Level::Fixed]), night(true, true));
        let lay = layout(&ControlMetrics::CLASSIC, &p, &text, 11);
        let s = lay.rows[0].slider.as_ref().unwrap();
        let (x20, x30) = (s.ticks[20], s.ticks[30]);
        assert_eq!(
            p.press(&lay, x20, s.channel.y + 1),
            Event::Set { row: 0, level: 20 }
        );
        assert_eq!(p.held(), Some(0));
        assert_eq!(p.drag(&lay, x30), Event::Set { row: 0, level: 30 });
        assert_eq!(p.drag(&lay, x30), Event::None);
        p.release();
        assert_eq!(p.held(), None);
        assert_eq!(p.drag(&lay, x20), Event::None);
        let lay = layout(&ControlMetrics::CLASSIC, &p, &text, 11);
        let t = lay.rows[0].slider.as_ref().unwrap().thumb;
        assert_eq!(
            p.press(&lay, t.x + 2, t.y + 2),
            Event::Redraw,
            "the thumb holds without moving"
        );
        assert_eq!((p.held(), p.rows[0].level), (Some(0), Level::At(30)));
        p.release();
        let st = lay.settings;
        assert_eq!(p.press(&lay, st.x + 1, st.y + 1), Event::Settings);
        let fixed = lay.rows[1].label;
        assert_eq!(p.press(&lay, fixed.x + 1, fixed.y + 1), Event::None);
    }

    #[test]
    fn the_wheel_over_a_row_steps_that_row_only() {
        let text = text();
        let mut p = DisplayPanel::new(
            rows(&[Level::At(60), Level::Fixed, Level::At(40)]),
            night(true, true),
        );
        let lay = layout(&ControlMetrics::CLASSIC, &p, &text, 11);
        let inside = |r: Rect| (r.x + 20, r.y + 5);
        let (x, y) = inside(lay.rows[2].block);
        assert_eq!(p.wheel(&lay, x, y, 1), Event::Step { row: 2, steps: 1 });
        let (x, y) = inside(lay.rows[1].block);
        assert_eq!(p.wheel(&lay, x, y, 1), Event::None);
        let sep = lay.separator;
        assert_eq!(p.wheel(&lay, sep.x + 5, sep.y, -1), Event::None);
        assert_eq!(p.rows[2].level, Level::At(40), "the host applies a step");
    }

    #[test]
    fn new_rows_keep_the_focus_in_range_and_end_a_drag_on_a_row_that_went() {
        let text = text();
        let mut p = DisplayPanel::new(rows(&[Level::At(60), Level::At(40)]), night(true, true));
        let lay = layout(&ControlMetrics::CLASSIC, &p, &text, 11);
        let t = lay.rows[1].slider.as_ref().unwrap().thumb;
        p.press(&lay, t.x + 2, t.y + 2);
        assert_eq!((p.focus, p.held()), (Focus::Row(1), Some(1)));
        p.set_rows(rows(&[Level::At(80), Level::At(45)]));
        assert_eq!((p.focus, p.held()), (Focus::Row(1), Some(1)), "same rows");
        let dell = rows(&[Level::At(60), Level::At(45)]).remove(1);
        p.set_rows(vec![dell]);
        assert_eq!(
            (p.focus, p.held()),
            (Focus::Row(0), Some(0)),
            "the held display moved up a row"
        );
        p.set_rows(rows(&[Level::At(60)]));
        assert_eq!((p.focus, p.held()), (Focus::Row(0), None));
        assert_eq!(p.rows[0].level, Level::At(60));
        p.set_rows(rows(&[Level::Fixed]));
        assert_eq!(p.focus, Focus::Night);
        p.set_night(night(false, false));
        assert_eq!(p.focus, Focus::Settings);
    }

    fn three() -> DisplayPanel {
        DisplayPanel::new(
            rows(&[Level::At(60), Level::At(40), Level::At(75)]),
            night(true, true),
        )
    }

    #[test]
    fn the_panel_is_330_wide_with_two_lines_per_display() {
        let text = text();
        let cm = ControlMetrics::CLASSIC;
        let p = three();
        let lay = layout(&cm, &p, &text, 11);
        assert_eq!(lay.size, (330, 273));
        let r = &lay.rows[0];
        assert_eq!(r.block.y, 8);
        assert_eq!(r.icon, Rect::new(12, 17, 16, 16));
        assert_eq!(r.label, Rect::new(38, 18, 236, 13));
        assert_eq!(r.percent, Rect::new(284, 18, 34, 13));
        let s = r.slider.as_ref().unwrap();
        assert_eq!(s.bounds, Rect::new(38, 35, 280, 26));
        assert_eq!(s.channel, Rect::new(38, 42, 280, 4));
        assert_eq!(lay.rows[1].block.y, 73);
        assert_eq!(lay.separator, Rect::new(12, 213, 306, 2));
        assert_eq!(lay.night_box, Rect::new(12, 225, 13, 13));
        assert_eq!((lay.settings.y, lay.settings.right()), (248, 318));
        let fixed = DisplayPanel::new(rows(&[Level::Fixed]), night(true, true));
        let fl = layout(&cm, &fixed, &text, 11);
        assert_eq!(fl.size, (330, 113));
        let f = &fl.rows[0];
        assert_eq!(f.percent.right(), 318);
        assert_eq!(
            f.percent.width,
            text.measure(osd::NOT_ADJUSTABLE, false, 11)
        );
        assert_eq!(f.label.right(), f.percent.x - 10);
        assert!(f.slider.is_none());
        let glass = ControlMetrics {
            flyout_border: 8,
            ..cm
        };
        assert_eq!(layout(&glass, &p, &text, 11).size, (342, 285));
    }

    #[test]
    fn a_grown_panel_grows_its_lines_and_width() {
        let text = text();
        let cm = ControlMetrics::CLASSIC.grown(TextUnits::new(22, 11));
        let lay = layout(&cm, &three(), &text, 22);
        assert_eq!(lay.size, (644, 338));
        assert_eq!(lay.rows[0].label.height, 26);
        assert_eq!(lay.rows[0].percent.width, 68);
        assert_eq!(lay.rows[0].slider.as_ref().unwrap().bounds.height, 26);
    }

    #[test]
    fn a_classic_panel_wears_the_raised_edge_and_draws_each_part() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = text();
        let p = DisplayPanel::new(
            rows(&[Level::At(60), Level::Fixed, Level::At(75)]),
            night(true, true),
        );
        let lay = layout(&theme.controls(), &p, &text, 11);
        let c = draw(&theme, &text, &p);
        let (w, h) = lay.size;
        assert_eq!((c.width as i32, c.height as i32), (w, h));
        assert_eq!(c.get(0, 0), cs.button_face.argb());
        assert_eq!(c.get(1, 1), cs.button_hilight.argb());
        assert_eq!(c.get(w - 1, h - 1), cs.button_dk_shadow.argb());
        assert_eq!(c.get(38, 42), cs.button_shadow.argb());
        let thumb = lay.rows[0].slider.as_ref().unwrap().thumb;
        assert_eq!((thumb.x, thumb.right()), (199, 210));
        let sep = lay.separator;
        assert_eq!(c.get(sep.x, sep.y), cs.button_shadow.argb());
        assert_eq!(c.get(sep.x, sep.y + 1), cs.button_hilight.argb());
        let grey = lay.rows[1].label;
        assert!(any_px(&c, grey, |px| px == cs.gray_text.argb()));
        assert!(!any_px(&c, grey, |px| px == cs.button_text.argb()));
        assert!(any_px(&c, lay.rows[1].percent, |px| px == cs.gray_text.argb()));
        assert!(lay.rows[1].slider.is_none());
        let st = lay.settings;
        let link = cs.hot_light.argb();
        assert!(any_px(&c, st, |px| px == link));
        assert!(
            (st.x..st.right()).all(|x| c.get(x, st.bottom() - 1) == link),
            "Settings... is underlined"
        );
        let b = lay.night_box;
        assert_eq!(c.get(b.x + 3, b.y + 3), cs.window.argb());
        let mut off = p;
        off.set_night(night(false, false));
        let c = draw(&theme, &text, &off);
        assert_eq!(c.get(b.x + 3, b.y + 3), cs.button_face.argb());
    }

    #[test]
    fn a_long_label_ends_before_the_percentage() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = text();
        let long = "A monitor with an extremely long model name".repeat(3);
        let mut rs = rows(&[Level::At(60), Level::Fixed]);
        for r in &mut rs {
            r.label = long.clone();
        }
        let p = DisplayPanel::new(rs, night(true, true));
        let lay = layout(&theme.controls(), &p, &text, 11);
        let c = draw(&theme, &text, &p);
        assert!(
            text.ellipsize(&long, lay.rows[0].label.width, false, 11)
                .ends_with("...")
        );
        for (r, ink) in [(&lay.rows[0], cs.button_text), (&lay.rows[1], cs.gray_text)] {
            let band = Rect::new(r.percent.x - 10, r.label.y, 10, r.label.height);
            assert!(!any_px(&c, band, |px| px == ink.argb()), "{band:?}");
            assert!(any_px(&c, r.percent, |px| px == ink.argb()));
        }
    }

    fn pin_cases() -> Vec<(String, Canvas)> {
        let text = text();
        let p = DisplayPanel::new(
            rows(&[Level::At(60), Level::At(40), Level::Fixed]),
            night(true, true),
        );
        let mut out = Vec::new();
        for (name, theme) in [
            ("classic", Theme::default()),
            ("blue", theme_for(Interface::Blue)),
            ("glass", theme_for(Interface::Glass)),
        ] {
            for s in [1, 2] {
                let t = theme.scaled(s);
                out.push((format!("{name} {s}"), draw(&t, &text, &p)));
            }
        }
        out
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for v in [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
        {
            for b in v.to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }

    const PINS: [(&str, u64); 6] = [
        ("classic 1", 0x960a_0587_cd47_5d2a),
        ("classic 2", 0xab3e_218a_448d_dd52),
        ("blue 1", 0x8f82_dce3_9656_86f6),
        ("blue 2", 0x92f3_b785_db6c_6f12),
        ("glass 1", 0xfd07_12ec_136b_0394),
        ("glass 2", 0xb3fe_7c3a_31d7_bf74),
    ];

    #[test]
    fn the_panel_pins_per_interface_at_scale_1_and_2() {
        if !crate::text::test_tahoma() {
            return;
        }
        let lines: Vec<String> = pin_cases()
            .iter()
            .map(|(name, c)| format!("(\"{name}\", 0x{:016x}),", fnv(c)))
            .collect();
        let want: Vec<String> = PINS
            .iter()
            .map(|(name, h)| format!("(\"{name}\", 0x{h:016x}),"))
            .collect();
        assert!(lines == want, "the panel moved:\n{}", lines.join("\n"));
    }

    #[test]
    fn a_mouse_open_shows_no_focus_until_a_key() {
        let text = text();
        for i in [Interface::Blue, Interface::Glass] {
            let theme = theme_for(i);
            let mut p = DisplayPanel::new(rows(&[Level::At(60), Level::At(40)]), night(true, true));
            let open = draw(&theme, &text, &p);
            p.focus = Focus::Settings;
            assert!(
                draw(&theme, &text, &p).pixels == open.pixels,
                "{i:?}: the first slider looks the same as an unfocused one"
            );
        }
    }

    #[test]
    #[ignore]
    fn render_panel() {
        for (name, c) in pin_cases() {
            let file = format!("displaypanel-{}.png", name.replace(' ', "-"));
            crate::raster::write_preview(&c, &file);
        }
    }

    #[test]
    fn the_panel_reads_as_a_slider_per_display_then_the_check_box() {
        let text = text();
        let p = DisplayPanel::new(
            rows(&[Level::At(60), Level::At(40), Level::Fixed]),
            night(true, true),
        );
        let lay = layout(&ControlMetrics::CLASSIC, &p, &text, 11);
        let mut tree = Tree::window("Displays", lay.size);
        let id = access(&p, &lay, &mut tree, ROOT);
        let slider = tree.find(Role::Slider, "Built-in display").unwrap();
        let n = tree.get(slider).unwrap();
        assert_eq!(n.numeric_value(), Some(60.0));
        assert_eq!(n.min_numeric_value(), Some(0.0));
        assert_eq!(n.max_numeric_value(), Some(100.0));
        assert!(tree.find(Role::Slider, "DELL U2720Q").is_some());
        assert!(
            tree.find(Role::Label, "LG 27GL850: Not adjustable")
                .is_some()
        );
        let check = tree.find(Role::CheckBox, "Night light").unwrap();
        assert_eq!(
            tree.get(check).unwrap().toggled(),
            Some(accesskit::Toggled::True)
        );
        assert!(tree.find(Role::Link, "Settings...").is_some());
        assert_eq!(id, slider);
    }
}
