use std::collections::HashMap;

use repriseapp::{BTN_LEFT, Ctx, KeyInput};
use reprisekit::access::{self, ROOT, Tree, accesskit};
use reprisekit::art::signalbars;
use reprisekit::classic::widgets::{BUTTON_H, LINE_H};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::taskpane::{self, Section, TaskPane};
use reprisekit::controls::tilelist::{self, Press, TileList};
use reprisekit::look::Paint;
use reprisekit::pointer::Click;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::Align;
use reprisekit::textedit::keysym::{self, SPACE};
use reprisekit::theme::{Color, TextUnits};

use crate::nm;

pub const CLIENT: (i32, i32) = (600, 440);
pub const ROW_H: i32 = 52;

const PANE_W: i32 = 180;
const BODY_X: i32 = PANE_W + 12;
const BODY_W: i32 = CLIENT.0 - BODY_X - 12;
const HEAD_Y: i32 = 12;
const INTRO_Y: i32 = 34;
const LIST_Y: i32 = 70;
const LIST_R: Rect = Rect::new(BODY_X, LIST_Y, BODY_W, LIST_H);
const BTN_W: i32 = 88;
const BTN_Y: i32 = CLIENT.1 - 12 - BUTTON_H;
const BTN_X: i32 = CLIENT.0 - 12 - BTN_W;
const LIST_H: i32 = BTN_Y - 8 - LIST_Y;

const TEXT_X: i32 = 9;
const NAME_Y: i32 = 9;
const CAPTION_Y: i32 = 28;
const LOCK_W: i32 = 11;
const BARS_RIGHT: i32 = 8;
const STAR_W: i32 = 10;
const TAG_GAP: i32 = 8;

const INTRO: [&str; 2] = [
    "Click an item in the list below to connect to a wireless network in range",
    "or to get more information.",
];

const PADLOCK: [&str; 9] = [
    "..###..", ".#...#.", ".#...#.", "#######", "#######", "##...##", "##...##", "#######",
    "#######",
];
const STAR: [&str; 7] = [
    "...#...", "...#...", "#######", ".#####.", "..###..", ".##.##.", "##...##",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Connect {
        ap: String,
        ssid: nm::Ssid,
        security: nm::Security,
        needs_key: bool,
    },
    Disconnect,
    Rescan,
    Close,
}

/// One per SSID; `ap` is the strongest access point carrying it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetRow {
    pub ap: String,
    pub ssid: nm::Ssid,
    pub strength: u8,
    pub security: nm::Security,
    pub known: bool,
    /// Saved, but without a key NetworkManager can dial with.
    pub keyless: bool,
    pub active: bool,
    pub tag: String,
}

pub fn rows_for(w: &nm::Wifi, state: nm::State, connecting: &Option<nm::Ssid>) -> Vec<NetRow> {
    let mut rows: Vec<NetRow> = Vec::new();
    let mut by_ssid: HashMap<&nm::Ssid, usize> = HashMap::new();
    for ap in w.aps.iter().filter(|ap| !ap.ssid.is_empty()) {
        match by_ssid.get(&ap.ssid) {
            Some(&i) => {
                let row = &mut rows[i];
                row.known |= ap.known;
                row.keyless |= ap.keyless;
                row.active |= ap.active;
                if ap.strength > row.strength {
                    row.ap = ap.path.clone();
                    row.strength = ap.strength;
                    row.security = ap.security;
                }
            }
            None => {
                by_ssid.insert(&ap.ssid, rows.len());
                rows.push(NetRow {
                    ap: ap.path.clone(),
                    ssid: ap.ssid.clone(),
                    strength: ap.strength,
                    security: ap.security,
                    known: ap.known,
                    keyless: ap.keyless,
                    active: ap.active,
                    tag: String::new(),
                });
            }
        }
    }
    for row in &mut rows {
        row.tag = tag(row, state, connecting).into();
    }
    rows.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(b.known.cmp(&a.known))
            .then(b.strength.cmp(&a.strength))
            .then(a.ssid.cmp(&b.ssid))
    });
    rows
}

/// The network being dialled wins over the one NetworkManager still calls
/// active: the device is on its way off that one.
fn tag(row: &NetRow, state: nm::State, connecting: &Option<nm::Ssid>) -> &'static str {
    if state == nm::State::Connecting && connecting.as_ref() == Some(&row.ssid) {
        return "Connecting";
    }
    if row.active {
        return "Connected";
    }
    if row.known {
        return "Automatic";
    }
    ""
}

/// The second line of a row: what it says about the network's
/// security. Enterprise and WEP need a profile this applet cannot build,
/// so they send the user to NetworkManager's own editor.
pub fn caption(sec: nm::Security) -> &'static str {
    match sec {
        nm::Security::Open => "Unsecured wireless network",
        nm::Security::Psk => "Security-enabled wireless network (WPA2)",
        nm::Security::Sae => "Security-enabled wireless network (WPA3)",
        nm::Security::Enterprise => "This network requires configuration",
    }
}

pub struct Chooser {
    pub device_path: String,
    pub pane: TaskPane,
    pub rows: Vec<NetRow>,
    pub list: TileList,
    pub connecting: Option<nm::Ssid>,
    pub button: Form,
    /// The text units the window is laid out at: its 100% layout grows by them.
    pub units: TextUnits,
}

impl Chooser {
    pub fn new(d: &nm::Device, units: TextUnits) -> Chooser {
        let mut c = Chooser {
            device_path: d.path.clone(),
            pane: TaskPane {
                sections: pane_sections(),
                ..Default::default()
            },
            rows: Vec::new(),
            list: TileList::new(ROW_H),
            connecting: None,
            button: Form::new(vec![Item::new(
                "connect",
                Rect::new(BTN_X, BTN_Y, BTN_W, BUTTON_H),
                Control::Button {
                    label: "&Connect".into(),
                    enabled: false,
                    default: true,
                },
            )]),
            units,
        };
        c.refresh(d);
        c
    }

    pub fn refresh(&mut self, d: &nm::Device) {
        if matches!(d.state, nm::State::Connected | nm::State::Failed) {
            self.connecting = None;
        }
        let ssid = self.selected().map(|r| r.ssid.clone());
        self.rows = match &d.wifi {
            Some(w) => rows_for(w, d.state, &self.connecting),
            None => Vec::new(),
        };
        self.list.entries = (0..self.rows.len()).map(tilelist::Entry::Row).collect();
        self.list
            .select_row(ssid.and_then(|s| self.rows.iter().position(|r| r.ssid == s)));
        self.list.clamp(self.list_r(), self.units);
        self.sync_button();
    }

    pub fn selected(&self) -> Option<&NetRow> {
        self.list.selected_row().and_then(|i| self.rows.get(i))
    }

    pub fn layout(&self, ctx: &Ctx) -> (Rect, Rect) {
        let pane = Rect::new(0, 0, self.units.len(PANE_W), ctx.size.1);
        (pane, self.list_r())
    }

    fn list_r(&self) -> Rect {
        self.units.rect(LIST_R)
    }

    /// The rows the list shows at once, which Page Up and Page Down move by.
    fn page(&self) -> i32 {
        ((self.list_r().height - 4) / self.units.len(ROW_H)).max(1)
    }

    pub fn press(&mut self, at: Click, button: u32, ctx: &Ctx) -> Outcome {
        let (x, y) = (at.x, at.y);
        if button != BTN_LEFT {
            return Outcome::None;
        }
        let (pane_r, list_r) = self.layout(ctx);
        if let Some(hit) = self.button.hit(x, y, &ctx.text, ctx.theme.fonts.size_px) {
            // A form acts on the release, which this window has no handler for.
            return match self.button.items[hit.item].control {
                Control::Button { enabled: true, .. } => self.activate(),
                _ => Outcome::None,
            };
        }
        if list_r.contains(x, y) {
            return self.press_list(at, list_r, &ctx.theme.controls());
        }
        if pane_r.contains(x, y) {
            let lay = taskpane::layout(
                &self.pane,
                &ctx.text,
                ctx.theme.fonts.size_px,
                pane_r,
                &ctx.theme.controls(),
            );
            let hit = taskpane::hit(&self.pane, &lay, x, y);
            return match self.pane.click(hit) {
                taskpane::Event::Row(0, 0) => Outcome::Rescan,
                taskpane::Event::None => Outcome::None,
                _ => Outcome::Redraw,
            };
        }
        Outcome::None
    }

    fn press_list(&mut self, at: Click, list_r: Rect, cm: &ControlMetrics) -> Outcome {
        match self.list.press(list_r, at, cm) {
            Press::None => Outcome::None,
            Press::Redraw => Outcome::Redraw,
            Press::Row {
                changed, activate, ..
            } => {
                self.sync_button();
                if activate {
                    self.activate()
                } else if changed {
                    Outcome::Redraw
                } else {
                    Outcome::None
                }
            }
        }
    }

    pub fn key(&mut self, k: &KeyInput, cm: &ControlMetrics) -> Outcome {
        match k.sym {
            keysym::UP | keysym::KP_UP => self.step(-1, cm),
            keysym::DOWN | keysym::KP_DOWN => self.step(1, cm),
            keysym::HOME | keysym::KP_HOME => self.step(i32::MIN / 2, cm),
            keysym::END | keysym::KP_END => self.step(i32::MAX / 2, cm),
            keysym::PAGE_UP => self.step(-self.page(), cm),
            keysym::PAGE_DOWN => self.step(self.page(), cm),
            keysym::ESCAPE => Outcome::Close,
            keysym::RETURN | keysym::KP_ENTER => self.button_key(form::Key::Enter),
            SPACE => self.button_key(form::Key::Space),
            _ => Outcome::None,
        }
    }

    fn button_key(&mut self, key: form::Key) -> Outcome {
        match self.button.key(key) {
            form::Event::Pressed(_) | form::Event::Default => self.activate(),
            form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    fn step(&mut self, step: i32, cm: &ControlMetrics) -> Outcome {
        if self.list.step(step, self.list_r(), cm) {
            self.sync_button();
            Outcome::Redraw
        } else {
            Outcome::None
        }
    }

    pub fn motion(&mut self, x: i32, y: i32, ctx: &Ctx) -> Outcome {
        let (cm, list_r) = (ctx.theme.controls(), self.list_r());
        let lit = self.list.hover(list_r, x, y, &cm) && ctx.styled();
        if self.list.drag(list_r, y, &cm) || lit {
            Outcome::Redraw
        } else {
            Outcome::None
        }
    }

    pub fn release(&mut self, ctx: &Ctx) -> Outcome {
        if self.list.release() && ctx.styled() {
            Outcome::Redraw
        } else {
            Outcome::None
        }
    }

    pub fn leave(&mut self, ctx: &Ctx) -> Outcome {
        if self.list.leave() && ctx.styled() {
            Outcome::Redraw
        } else {
            Outcome::None
        }
    }

    pub fn scroll(&mut self, rows: i32) -> Outcome {
        if self.list.scroll(rows, self.list_r(), self.units) {
            Outcome::Redraw
        } else {
            Outcome::None
        }
    }

    fn activate(&mut self) -> Outcome {
        let Some(row) = self.selected() else {
            return Outcome::None;
        };
        if row.active {
            return Outcome::Disconnect;
        }
        if row.security == nm::Security::Enterprise {
            return Outcome::None;
        }
        Outcome::Connect {
            ap: row.ap.clone(),
            ssid: row.ssid.clone(),
            security: row.security,
            needs_key: matches!(row.security, nm::Security::Psk | nm::Security::Sae)
                && (!row.known || row.keyless),
        }
    }

    /// The button's word and whether it is live: nothing picked, or a
    /// network needing a profile this applet cannot build, greys it.
    fn sync_button(&mut self) {
        let row = self.selected();
        let live = row.is_some_and(|r| r.security != nm::Security::Enterprise);
        let word = if row.is_some_and(|r| r.active) {
            "&Disconnect"
        } else {
            "&Connect"
        };
        if let Some(Control::Button { label, enabled, .. }) =
            self.button.item_mut("connect").map(|i| &mut i.control)
        {
            *label = word.into();
            *enabled = live;
        }
        self.button.set_focus(live.then_some(0));
    }

    pub fn draw(&self, c: &mut Canvas, ctx: &Ctx) {
        let cs = &ctx.theme.colors;
        let px = ctx.theme.fonts.size_px;
        let (pane_r, list_r) = self.layout(ctx);
        c.fill_rect(c.bounds(), cs.window);
        let pl = taskpane::layout(&self.pane, &ctx.text, px, pane_r, &ctx.theme.controls());
        let paint = Paint {
            s: 1,
            ..ctx.theme.paint()
        };
        taskpane::draw(c, &ctx.text, &paint, &self.pane, &pl, px);
        taskpane::draw_gap(
            c,
            &paint,
            &pl,
            Rect::new(
                pane_r.right(),
                pane_r.y,
                list_r.x - pane_r.right(),
                pane_r.height,
            ),
        );
        let u = self.units;
        ctx.text.draw(
            c,
            u.rect(Rect::new(BODY_X, HEAD_Y, BODY_W, LINE_H)),
            "Choose a wireless network",
            true,
            cs.window_text,
            Align::Left,
            px,
        );
        for (i, line) in INTRO.iter().enumerate() {
            ctx.text.draw(
                c,
                u.rect(Rect::new(
                    BODY_X,
                    INTRO_Y + i as i32 * LINE_H,
                    BODY_W,
                    LINE_H,
                )),
                line,
                false,
                cs.window_text,
                Align::Left,
                px,
            );
        }
        let lay = tilelist::layout(&self.list, list_r, &ctx.theme.controls());
        tilelist::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &self.list,
            &lay,
            px,
            |c, r, i, sel| draw_row(c, ctx, r, &self.rows[i], sel),
        );
        form::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &self.button,
            false,
            px,
        );
    }
}

impl Chooser {
    pub fn access(&self, tree: &mut Tree, ctx: &Ctx) {
        let px = ctx.theme.fonts.size_px;
        let (pane_r, list_r) = self.layout(ctx);
        let pl = taskpane::layout(&self.pane, &ctx.text, px, pane_r, &ctx.theme.controls());
        taskpane::access(&self.pane, &pl, tree, ROOT);
        let u = self.units;
        let mut head = access::label(
            u.rect(Rect::new(BODY_X, HEAD_Y, BODY_W, LINE_H)),
            "Choose a wireless network",
            true,
        );
        head.set_role(accesskit::Role::Heading);
        tree.push(ROOT, "heading", head);
        let intro = u.rect(Rect::new(
            BODY_X,
            INTRO_Y,
            BODY_W,
            LINE_H * INTRO.len() as i32,
        ));
        tree.push(ROOT, "intro", access::label(intro, &INTRO.join(" "), true));
        let lay = tilelist::layout(&self.list, list_r, &ctx.theme.controls());
        let row = tilelist::access(
            &self.list,
            &lay,
            |i| {
                let r = &self.rows[i];
                let mut about = caption(r.security).to_string();
                if !r.tag.is_empty() {
                    about = format!("{about}, {}", r.tag);
                }
                (r.ssid.to_string(), about)
            },
            tree,
            ROOT,
            "networks",
        );
        self.button.access(tree, ROOT, &ctx.text, px);
        tree.set_focus(row);
    }
}

/// A network's row in `r`: its two lines sit where a 100% row puts them, grown by the theme's
/// text; the bars, the padlock and the star keep their size.
pub fn draw_row(c: &mut Canvas, ctx: &Ctx, r: Rect, row: &NetRow, selected: bool) {
    let cs = &ctx.theme.colors;
    let px = ctx.theme.fonts.size_px;
    let u = ctx.theme.text;
    let (name_y, caption_y, line_h) = (r.y + u.len(NAME_Y), r.y + u.len(CAPTION_Y), u.len(LINE_H));
    let lift = (line_h - LINE_H) / 2;
    let greyed = row.security == nm::Security::Enterprise;
    let ink = match (selected, greyed) {
        (true, _) => cs.hilight_text,
        (false, true) => cs.gray_text,
        (false, false) => cs.window_text,
    };
    c.fill_rect(r, if selected { cs.hilight } else { cs.window });
    let bars_x = r.right() - BARS_RIGHT - signalbars::WIDTH;
    signalbars::draw(
        c,
        bars_x,
        r.y + (r.height - signalbars::HEIGHT) / 2,
        signalbars::five(row.strength),
        !greyed,
    );
    let name_r = Rect::new(r.x + TEXT_X, name_y, r.width, line_h);
    ctx.text
        .draw(c, name_r, &row.ssid.to_string(), true, ink, Align::Left, px);
    if !row.tag.is_empty() {
        let w = ctx.text.measure(&row.tag, false, px);
        let tag_x = bars_x - TAG_GAP - w;
        ctx.text.draw(
            c,
            Rect::new(tag_x, name_y, w, line_h),
            &row.tag,
            false,
            ink,
            Align::Left,
            px,
        );
        sprite(c, tag_x - STAR_W, name_y + lift + 3, &STAR, ink);
    }
    let lock = row.security != nm::Security::Open;
    if lock {
        padlock(c, r.x + TEXT_X, caption_y + lift + 2, ink);
    }
    let cap_x = r.x + TEXT_X + if lock { LOCK_W } else { 0 };
    ctx.text.draw(
        c,
        Rect::new(cap_x, caption_y, r.width, line_h),
        caption(row.security),
        false,
        ink,
        Align::Left,
        px,
    );
}

pub fn padlock(c: &mut Canvas, x: i32, y: i32, ink: Color) {
    sprite(c, x, y, &PADLOCK, ink);
}

fn sprite(c: &mut Canvas, x: i32, y: i32, map: &[&str], ink: Color) {
    for (dy, row) in map.iter().enumerate() {
        for (dx, ch) in row.chars().enumerate() {
            if ch == '#' {
                c.put(x + dx as i32, y + dy as i32, ink);
            }
        }
    }
}

fn pane_sections() -> Vec<Section> {
    let link = |text: &str, enabled: bool| taskpane::Row {
        icon: None,
        text: text.into(),
        enabled,
    };
    vec![
        Section::links(
            "Network Tasks",
            vec![
                link("Refresh network list", true),
                link(
                    "Set up a wireless network for a home or small office",
                    false,
                ),
            ],
        ),
        Section::links(
            "Related Tasks",
            vec![
                link("Learn about wireless networking", false),
                link("Change the order of preferred networks", false),
                link("Change advanced settings", false),
            ],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A press that never completes a double click.
    fn one(x: i32, y: i32) -> Click {
        Click {
            x,
            y,
            time: 0,
            double_ms: 0,
        }
    }

    /// A press that completes a double click after a press on the same row.
    fn two(x: i32, y: i32) -> Click {
        Click {
            x,
            y,
            time: 1,
            double_ms: 500,
        }
    }

    fn ctx() -> Ctx {
        crate::tests::test_ctx(CLIENT)
    }

    fn ap(path: &str, ssid: &str, strength: u8, security: nm::Security) -> nm::AccessPoint {
        nm::AccessPoint {
            path: path.into(),
            ssid: ssid.into(),
            strength,
            security,
            known: false,
            keyless: false,
            active: false,
        }
    }

    fn wifi() -> nm::Wifi {
        nm::Wifi {
            active_ssid: Some("Home".into()),
            strength: 70,
            aps: vec![
                ap("/ap/1", "Cafe", 90, nm::Security::Open),
                nm::AccessPoint {
                    known: true,
                    active: true,
                    ..ap("/ap/2", "Home", 40, nm::Security::Psk)
                },
                nm::AccessPoint {
                    known: true,
                    ..ap("/ap/3", "Home", 70, nm::Security::Psk)
                },
                nm::AccessPoint {
                    known: true,
                    ..ap("/ap/4", "Work", 10, nm::Security::Enterprise)
                },
                ap("/ap/5", "Neighbour", 50, nm::Security::Psk),
            ],
        }
    }

    fn device() -> nm::Device {
        nm::Device {
            path: "/dev/2".into(),
            iface: "wlan0".into(),
            kind: nm::Kind::Wifi,
            shell_name: "Wireless Network Connection".into(),
            state: nm::State::Connected,
            managed: true,
            carrier: true,
            wifi: Some(wifi()),
            ..Default::default()
        }
    }

    fn chooser() -> Chooser {
        Chooser::new(&device(), TextUnits::ONE)
    }

    fn ssids(c: &Chooser) -> Vec<String> {
        c.rows.iter().map(|r| r.ssid.to_string()).collect()
    }

    fn slot_point(ctx: &Ctx, c: &Chooser, n: usize) -> (i32, i32) {
        let (_, list_r) = c.layout(ctx);
        let inner = list_r.inset(2);
        let row_h = c.units.len(ROW_H);
        (inner.x + 40, inner.y + n as i32 * row_h + row_h / 2)
    }

    fn key(sym: u32) -> KeyInput {
        KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: None,
        }
    }

    #[test]
    fn rows_dedup_and_sort() {
        let rows = rows_for(&wifi(), nm::State::Connected, &None);
        assert_eq!(
            rows.iter().map(|r| r.ssid.to_string()).collect::<Vec<_>>(),
            ["Home", "Work", "Cafe", "Neighbour"],
            "the live network, then the saved one, then by strength"
        );
        let home = &rows[0];
        assert_eq!(home.ap, "/ap/3");
        assert_eq!(home.strength, 70);
        assert!(home.active && home.known);
        assert_eq!(
            rows.iter().map(|r| r.tag.as_str()).collect::<Vec<_>>(),
            ["Connected", "Automatic", "", ""]
        );
        let mut w = wifi();
        w.aps.push(ap("/ap/6", "", 99, nm::Security::Psk));
        assert_eq!(rows_for(&w, nm::State::Connected, &None).len(), 4);
    }

    #[test]
    fn two_non_utf8_ssids_keep_their_own_rows() {
        let latin1 = |last: u8| nm::AccessPoint {
            ssid: nm::Ssid(vec![b'C', b'a', b'f', last]),
            ..ap("/ap/x", "", 50, nm::Security::Psk)
        };
        let w = nm::Wifi {
            aps: vec![latin1(0xE9), latin1(0xE8)],
            ..nm::Wifi::default()
        };
        let rows = rows_for(&w, nm::State::Disconnected, &None);
        assert_eq!(
            rows.iter().map(|r| r.ssid.0.clone()).collect::<Vec<_>>(),
            [b"Caf\xe8".to_vec(), b"Caf\xe9".to_vec()],
            "two rows, each with its own bytes"
        );
    }

    #[test]
    fn caption_per_security() {
        assert_eq!(caption(nm::Security::Open), "Unsecured wireless network");
        assert_eq!(
            caption(nm::Security::Psk),
            "Security-enabled wireless network (WPA2)"
        );
        assert_eq!(
            caption(nm::Security::Sae),
            "Security-enabled wireless network (WPA3)"
        );
        assert_eq!(
            caption(nm::Security::Enterprise),
            "This network requires configuration"
        );
    }

    #[test]
    fn connecting_tag_follows_state() {
        let dialling = Some("Neighbour".into());
        let rows = rows_for(&wifi(), nm::State::Connecting, &dialling);
        let row = |s: &str| {
            rows.iter()
                .find(|r| r.ssid == s.into())
                .unwrap_or_else(|| panic!("no {s}"))
                .tag
                .clone()
        };
        assert_eq!(row("Neighbour"), "Connecting");
        assert_eq!(row("Home"), "Connected");
        let rows = rows_for(&wifi(), nm::State::Connected, &dialling);
        assert_eq!(
            rows.iter()
                .find(|r| r.ssid == "Neighbour".into())
                .unwrap()
                .tag,
            ""
        );
        let mut c = chooser();
        c.connecting = Some("Neighbour".into());
        c.refresh(&nm::Device {
            state: nm::State::Connecting,
            ..device()
        });
        assert_eq!(c.connecting, Some("Neighbour".into()));
        c.refresh(&device());
        assert_eq!(c.connecting, None);
        let failed = nm::Device {
            state: nm::State::Failed,
            ..device()
        };
        c.connecting = Some("Neighbour".into());
        c.refresh(&failed);
        assert_eq!(c.connecting, None);
    }

    #[test]
    fn the_pane_the_list_and_the_button_fit_the_client() {
        let ctx = ctx();
        let c = chooser();
        let (pane_r, list_r) = c.layout(&ctx);
        assert_eq!(pane_r, Rect::new(0, 0, PANE_W, CLIENT.1));
        assert_eq!(list_r, Rect::new(192, LIST_Y, 396, LIST_H));
        let btn = c.button.item("connect").unwrap().rect;
        assert_eq!(btn, Rect::new(BTN_X, BTN_Y, BTN_W, BUTTON_H));
        assert!(btn.right() <= CLIENT.0 && btn.bottom() <= CLIENT.1);
        assert!(btn.y > list_r.bottom(), "the button clears the list");
        assert_eq!(list_r.right(), btn.right(), "both end on one column");
        assert_eq!(c.page(), 6);
        assert_eq!(c.pane.sections.len(), 2);
        let taskpane::Body::Links(rows) = &c.pane.sections[0].body else {
            panic!("Network Tasks has links")
        };
        assert!(rows[0].enabled && !rows[1].enabled);
    }

    /// A profile another desktop's applet made keeps its key in that
    /// desktop's wallet; NetworkManager cannot dial it alone, so the row
    /// asks for the key as an unsaved network would.
    #[test]
    fn a_saved_network_without_its_key_asks_for_one() {
        let ctx = ctx();
        let mut d = device();
        d.wifi = Some(nm::Wifi {
            active_ssid: None,
            strength: 0,
            aps: vec![nm::AccessPoint {
                known: true,
                keyless: true,
                ..ap("/ap/6", "Office", 60, nm::Security::Psk)
            }],
        });
        let mut c = Chooser::new(&d, TextUnits::ONE);
        let (x, y) = slot_point(&ctx, &c, 0);
        c.press(one(x, y), BTN_LEFT, &ctx);
        assert_eq!(
            c.press(two(x, y), BTN_LEFT, &ctx),
            Outcome::Connect {
                ap: "/ap/6".into(),
                ssid: "Office".into(),
                security: nm::Security::Psk,
                needs_key: true,
            }
        );
    }

    /// At 200% the window, its pane, list, rows and button grow with the text: the third grown
    /// row takes a press, Page Down moves by the rows the grown list shows, and the button stays
    /// inside the grown client.
    #[test]
    fn at_200_percent_the_window_and_its_rows_grow() {
        let u = TextUnits::new(22, 11);
        let mut ctx = crate::tests::test_ctx(u.size(CLIENT));
        ctx.text_size = 200;
        ctx.theme = reprisekit::theme::Theme::default().with_text_size(200);
        assert_eq!(ctx.theme.text, u);
        let mut c = Chooser::new(&device(), u);
        let (pane_r, list_r) = c.layout(&ctx);
        assert_eq!(pane_r.width, 2 * PANE_W);
        assert_eq!(list_r, u.rect(LIST_R));
        assert_eq!(c.page(), (list_r.height - 4) / (2 * ROW_H));
        let mut canvas = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
        c.draw(&mut canvas, &ctx);
        let btn = c
            .button
            .units()
            .rect(c.button.item("connect").unwrap().rect);
        assert!(btn.right() <= ctx.size.0 && btn.bottom() <= ctx.size.1);
        assert!(btn.y > list_r.bottom(), "the button clears the grown list");
        let (x, y) = slot_point(&ctx, &c, 2);
        c.press(one(x, y), BTN_LEFT, &ctx);
        assert_eq!(c.list.selected_row(), Some(2));
    }

    #[test]
    fn an_unsaved_wpa3_network_asks_for_its_key() {
        let ctx = ctx();
        let mut d = device();
        d.wifi = Some(nm::Wifi {
            active_ssid: None,
            strength: 0,
            aps: vec![ap("/ap/7", "Lab", 60, nm::Security::Sae)],
        });
        let mut c = Chooser::new(&d, TextUnits::ONE);
        let (x, y) = slot_point(&ctx, &c, 0);
        c.press(one(x, y), BTN_LEFT, &ctx);
        assert_eq!(
            c.press(two(x, y), BTN_LEFT, &ctx),
            Outcome::Connect {
                ap: "/ap/7".into(),
                ssid: "Lab".into(),
                security: nm::Security::Sae,
                needs_key: true,
            }
        );
    }

    #[test]
    fn a_row_picks_itself_and_the_button_connects() {
        let ctx = ctx();
        let mut c = chooser();
        let label = |c: &Chooser| match &c.button.item("connect").unwrap().control {
            Control::Button { label, enabled, .. } => (label.clone(), *enabled),
            other => panic!("{other:?}"),
        };
        assert_eq!(label(&c), ("&Connect".into(), false), "nothing picked yet");
        let (x, y) = slot_point(&ctx, &c, 0);
        assert_eq!(c.press(one(x, y), BTN_LEFT, &ctx), Outcome::Redraw);
        assert_eq!(c.list.selected_row(), Some(0));
        assert_eq!(label(&c), ("&Disconnect".into(), true));
        let btn = c.button.item("connect").unwrap().rect;
        assert_eq!(
            c.press(one(btn.x + 4, btn.y + 4), BTN_LEFT, &ctx),
            Outcome::Disconnect
        );
        let (x, y) = slot_point(&ctx, &c, 3);
        c.press(one(x, y), BTN_LEFT, &ctx);
        assert_eq!(c.press(two(x, y), BTN_LEFT, &ctx), {
            Outcome::Connect {
                ap: "/ap/5".into(),
                ssid: "Neighbour".into(),
                security: nm::Security::Psk,
                needs_key: true,
            }
        });
        let (x, y) = slot_point(&ctx, &c, 2);
        c.press(one(x, y), BTN_LEFT, &ctx);
        assert_eq!(label(&c), ("&Connect".into(), true));
        assert_eq!(
            c.press(one(btn.x + 4, btn.y + 4), BTN_LEFT, &ctx),
            Outcome::Connect {
                ap: "/ap/1".into(),
                ssid: "Cafe".into(),
                security: nm::Security::Open,
                needs_key: false,
            },
            "an open network needs no key"
        );
        let (x, y) = slot_point(&ctx, &c, 1);
        assert_eq!(c.press(one(x, y), BTN_LEFT, &ctx), Outcome::Redraw);
        assert_eq!(label(&c), ("&Connect".into(), false));
        assert_eq!(
            c.press(one(btn.x + 4, btn.y + 4), BTN_LEFT, &ctx),
            Outcome::None
        );
        c.press(one(x, y), BTN_LEFT, &ctx);
        assert_eq!(c.press(two(x, y), BTN_LEFT, &ctx), Outcome::None);
        let (x, y) = slot_point(&ctx, &c, 5);
        assert_eq!(c.press(one(x, y), BTN_LEFT, &ctx), Outcome::None);
        assert_eq!(c.list.selected_row(), Some(1));
        let (pane_r, _) = c.layout(&ctx);
        let lay = taskpane::layout(
            &c.pane,
            &ctx.text,
            ctx.theme.fonts.size_px,
            pane_r,
            &ctx.theme.controls(),
        );
        let row = lay.rows[0][0];
        assert_eq!(
            c.press(one(row.x + 2, row.y + 2), BTN_LEFT, &ctx),
            Outcome::Rescan
        );
        let row = lay.rows[0][1];
        assert_eq!(
            c.press(one(row.x + 2, row.y + 2), BTN_LEFT, &ctx),
            Outcome::None
        );
    }

    #[test]
    fn keys_walk_the_list_and_work_the_button() {
        let mut c = chooser();
        assert_eq!(
            c.key(&key(keysym::DOWN), &ControlMetrics::CLASSIC),
            Outcome::Redraw
        );
        assert_eq!(
            c.list.selected_row(),
            Some(0),
            "the first key picks the first row"
        );
        assert_eq!(
            c.key(&key(keysym::DOWN), &ControlMetrics::CLASSIC),
            Outcome::Redraw
        );
        assert_eq!(c.list.selected_row(), Some(1));
        assert_eq!(
            c.key(&key(keysym::END), &ControlMetrics::CLASSIC),
            Outcome::Redraw
        );
        assert_eq!(c.list.selected_row(), Some(3));
        assert_eq!(
            c.key(&key(keysym::DOWN), &ControlMetrics::CLASSIC),
            Outcome::None,
            "already last"
        );
        assert_eq!(
            c.key(&key(keysym::HOME), &ControlMetrics::CLASSIC),
            Outcome::Redraw
        );
        assert_eq!(c.list.selected_row(), Some(0));
        assert_eq!(
            c.key(&key(keysym::RETURN), &ControlMetrics::CLASSIC),
            Outcome::Disconnect
        );
        assert_eq!(
            c.key(&key(SPACE), &ControlMetrics::CLASSIC),
            Outcome::Disconnect
        );
        assert_eq!(
            c.key(&key(keysym::ESCAPE), &ControlMetrics::CLASSIC),
            Outcome::Close
        );
        assert_eq!(
            c.key(&key(keysym::DELETE), &ControlMetrics::CLASSIC),
            Outcome::None
        );
    }

    #[test]
    fn the_scrollbar_follows_the_rows() {
        use reprisekit::classic::widgets::scrollbar_parts;

        let mut c = chooser();
        let ctx = ctx();
        let (_, list_r) = c.layout(&ctx);
        assert!(
            tilelist::layout(&c.list, list_r, &ctx.theme.look.controls())
                .scrollbar
                .is_none(),
            "four rows fit"
        );
        assert_eq!(c.scroll(1), Outcome::None);
        let mut w = wifi();
        for i in 0..6 {
            w.aps.push(ap(
                &format!("/ap/x{i}"),
                &format!("Net {i}"),
                30,
                nm::Security::Open,
            ));
        }
        c.refresh(&nm::Device {
            wifi: Some(w),
            ..device()
        });
        assert_eq!(c.rows.len(), 10);
        let sb = tilelist::layout(&c.list, list_r, &ctx.theme.look.controls())
            .scrollbar
            .expect("a bar");
        assert_eq!(c.scroll(3), Outcome::Redraw);
        assert_eq!(c.list.top, 3);
        assert_eq!(c.scroll(9), Outcome::Redraw);
        assert_eq!(c.list.top, 4, "the last row stays in the box");
        assert_eq!(c.scroll(1), Outcome::None);
        assert_eq!(c.scroll(-99), Outcome::Redraw);
        assert_eq!(c.list.top, 0);
        assert_eq!(
            c.press(one(sb.x + 2, sb.bottom() - 3), BTN_LEFT, &ctx),
            Outcome::Redraw
        );
        assert_eq!(c.list.top, 1);
        assert_eq!(
            c.press(one(sb.x + 2, sb.y + 2), BTN_LEFT, &ctx),
            Outcome::Redraw
        );
        assert_eq!(c.list.top, 0);
        let parts = scrollbar_parts(
            sb,
            1,
            c.list.top,
            c.page() as usize,
            c.rows.len(),
            false,
            ctx.theme.look.controls().thumb_min,
        );
        let thumb = parts.thumb.expect("a thumb");
        assert_eq!(
            c.press(one(thumb.x + 2, thumb.y + 3), BTN_LEFT, &ctx),
            Outcome::None
        );
        assert_eq!(c.list.top, 0, "a press on the thumb does not page");
        assert_eq!(
            c.motion(thumb.x + 2, parts.track.bottom(), &ctx),
            Outcome::Redraw
        );
        assert_eq!(c.list.top, 4);
        assert_eq!(
            c.motion(thumb.x + 2, parts.track.y + 3, &ctx),
            Outcome::Redraw
        );
        assert_eq!(c.list.top, 0);
        c.release(&ctx);
        assert_eq!(
            c.motion(thumb.x + 2, parts.track.bottom(), &ctx),
            Outcome::None
        );
        assert_eq!(c.list.top, 0, "released: motion no longer scrolls");
        for _ in 0..7 {
            c.key(&key(keysym::DOWN), &ControlMetrics::CLASSIC);
        }
        assert_eq!(c.list.selected_row(), Some(6));
        assert_eq!(c.list.top, 1);
    }

    #[test]
    fn refresh_keeps_the_selection_by_ssid() {
        let mut c = chooser();
        c.list.select_row(Some(3));
        c.sync_button();
        assert_eq!(c.selected().unwrap().ssid, "Neighbour".into());
        let mut w = wifi();
        w.aps[4].strength = 99;
        c.refresh(&nm::Device {
            wifi: Some(w),
            ..device()
        });
        assert_eq!(ssids(&c), ["Home", "Work", "Neighbour", "Cafe"]);
        assert_eq!(c.list.selected_row(), Some(2));
        let mut w = wifi();
        w.aps.remove(4);
        c.refresh(&nm::Device {
            wifi: Some(w),
            ..device()
        });
        assert_eq!(c.list.selected_row(), None);
        assert!(matches!(
            c.button.item("connect").unwrap().control,
            Control::Button { enabled: false, .. }
        ));
        c.refresh(&nm::Device {
            wifi: None,
            ..device()
        });
        assert!(c.rows.is_empty());
        assert_eq!(c.device_path, "/dev/2");
    }

    #[test]
    #[ignore]
    fn render_chooser() {
        let mut ctx = ctx();
        ctx.theme.access_keys = true;
        let mut c = chooser();
        c.list.select_row(Some(2));
        c.sync_button();
        let mut canvas = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        c.draw(&mut canvas, &ctx);
        reprisekit::raster::write_preview(&canvas, "wireless-chooser.png");
    }
}
