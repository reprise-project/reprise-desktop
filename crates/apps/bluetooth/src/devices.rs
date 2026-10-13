//! Bluetooth Devices, the program's main window: a sheet with two tabs.
//! Devices lists paired devices by group; Options turns the radio on and off
//! and makes this computer discoverable. Nothing here reads the bus: the host
//! feeds it snapshots and acts on the [`Outcome`]s it hands back.

use repriseapp::Ctx;
use repriseapp::oneshot;
use reprisekit::access::{self, Tree};
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::controls::tilelist::{self, Entry, Press, TILE_H, Tile, TileList};
use reprisekit::look::Paint;
use reprisekit::pointer::Click;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::Align;

use crate::bz::{self, Device, Snapshot};
use crate::icons::Icons;

pub const CLIENT: (i32, i32) = (398, 424);
pub const TABS: [&str; 2] = ["Devices", "Options"];
pub const TAB_DEVICES: usize = 0;
pub const TAB_OPTIONS: usize = 1;

pub const ADD: &str = "add";
pub const REMOVE: &str = "remove";
pub const CONNECT: &str = "connect";
pub const PROPS: &str = "props";
const POWER: &str = "power";
const DISCOVERABLE: &str = "discoverable";

/// The page runs (6, 28) to (392, 388); buttons sit 8 px above its bottom.
const BTN_Y: i32 = 388 - 8 - BUTTON_H;
pub const LIST: Rect = Rect::new(14, 36, 370, BTN_Y - 8 - 36);
const CONNECT_W: i32 = 88;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub powered: bool,
    pub discoverable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Close,
    Add,
    Remove(String),
    Toggle(String),
    Properties(String),
    Apply(Options),
    ApplyAndClose(Options),
}

pub struct Devices {
    pub sheet: PropSheet,
    pub list: TileList,
    pub rows: Vec<Device>,
    /// A click in the list gave it the keyboard.
    pub list_focus: bool,
    applied: Options,
    pending: Options,
    adapter: bool,
}

pub fn entries(rows: &[Device]) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut last = None;
    for (i, d) in rows.iter().enumerate() {
        let g = d.kind.group();
        if last != Some(g) {
            out.push(Entry::Header(g.title().into()));
            last = Some(g);
        }
        out.push(Entry::Row(i));
    }
    out
}

fn devices_page() -> Form {
    Form::new(vec![
        Item::button_at(ADD, 14, BTN_Y, BUTTON_W, "&Add...", false),
        Item::button_at(REMOVE, 14 + BUTTON_W + 6, BTN_Y, BUTTON_W, "&Remove", false),
        Item::button_at(
            CONNECT,
            14 + 2 * (BUTTON_W + 6),
            BTN_Y,
            CONNECT_W,
            "&Connect",
            false,
        ),
        Item::button_at(
            PROPS,
            LIST.right() - BUTTON_W,
            BTN_Y,
            BUTTON_W,
            "&Properties",
            false,
        ),
    ])
}

fn options_page() -> Form {
    Form::new(vec![
        Item::group("radio_group", Rect::new(14, 36, 370, 50), "Bluetooth radio"),
        Item::check(POWER, 26, 58, "&Turn on Bluetooth"),
        Item::group("discovery_group", Rect::new(14, 96, 370, 70), "Discovery"),
        Item::check(
            DISCOVERABLE,
            26,
            118,
            "Allow Bluetooth &devices to find this computer",
        ),
        Item::lines_at(
            "discovery_note",
            44,
            138,
            330,
            &[
                "While this is on, other devices can see this computer",
                "and ask to pair with it.",
            ],
        ),
    ])
}

impl Devices {
    pub fn new() -> Devices {
        let mut d = Devices {
            sheet: PropSheet::new(&TABS, vec![devices_page(), options_page()], CLIENT),
            list: TileList::new(TILE_H),
            rows: Vec::new(),
            list_focus: false,
            applied: Options::default(),
            pending: Options::default(),
            adapter: false,
        };
        d.write();
        d
    }

    /// The device list on the Devices page as last drawn: 100% tiles in a frame that grows and
    /// scrolls with the page.
    pub fn list_rect(&self) -> Rect {
        self.sheet.on_page(LIST)
    }

    pub fn selected(&self) -> Option<&Device> {
        self.list.selected_row().and_then(|i| self.rows.get(i))
    }

    fn selected_path(&self) -> Option<String> {
        self.selected().map(|d| d.path.clone())
    }

    pub fn blank_text(&self) -> Option<&'static str> {
        if !self.adapter {
            Some("No Bluetooth adapter was found.")
        } else if !self.applied.powered {
            Some("Bluetooth is turned off. Turn it on from the Options tab.")
        } else {
            None
        }
    }

    /// The selection follows its device; Options takes the adapter's state unless
    /// it holds an unapplied change.
    pub fn refresh(&mut self, s: &Snapshot) {
        let keep = self.selected_path();
        self.rows = bz::by_group(s.devices.iter().filter(|d| d.paired));
        self.list.entries = entries(&self.rows);
        self.list
            .select_row(keep.and_then(|p| self.rows.iter().position(|d| d.path == p)));
        self.list
            .clamp(self.list_rect(), reprisekit::theme::TextUnits::ONE);
        self.adapter = s.adapter.is_some();
        let now = s.adapter.as_ref().map_or(Options::default(), |a| Options {
            powered: a.powered,
            discoverable: a.discoverable,
        });
        let dirty = self.pending != self.applied;
        self.applied = now;
        if !dirty {
            self.pending = now;
        }
        self.write();
    }

    fn write(&mut self) {
        let live = self.blank_text().is_none();
        let picked = self.selected().map(|d| d.connected);
        let dirty = self.pending != self.applied;
        {
            let page = &mut self.sheet.pages[TAB_DEVICES];
            page.set_enabled(ADD, live);
            page.set_enabled(REMOVE, live && picked.is_some());
            page.set_enabled(PROPS, live && picked.is_some());
            page.set_enabled(CONNECT, live && picked.is_some());
            if let Some(Item {
                control: Control::Button { label, .. },
                ..
            }) = page.item_mut(CONNECT)
            {
                *label = if picked == Some(true) {
                    "&Disconnect"
                } else {
                    "&Connect"
                }
                .into();
            }
        }
        let page = &mut self.sheet.pages[TAB_OPTIONS];
        page.set_check(POWER, self.pending.powered);
        page.set_enabled(POWER, self.adapter);
        page.set_check(DISCOVERABLE, self.pending.discoverable);
        page.set_enabled(DISCOVERABLE, self.adapter && self.pending.powered);
        self.sheet.set_apply_enabled(dirty);
    }

    pub fn event(&mut self, ev: propsheet::Event) -> Outcome {
        match ev {
            propsheet::Event::None => Outcome::None,
            propsheet::Event::Redraw => Outcome::Redraw,
            propsheet::Event::Page(_, form::Event::Changed(id @ (POWER | DISCOVERABLE))) => {
                let on = self.sheet.pages[TAB_OPTIONS].checked(id).unwrap_or(false);
                if id == POWER {
                    self.pending.powered = on;
                } else {
                    self.pending.discoverable = on;
                }
                self.write();
                Outcome::Redraw
            }
            propsheet::Event::Page(_, form::Event::Pressed(id)) => {
                self.list_focus = false;
                match (id, self.selected_path()) {
                    (ADD, _) => Outcome::Add,
                    (REMOVE, Some(p)) => Outcome::Remove(p),
                    (CONNECT, Some(p)) => Outcome::Toggle(p),
                    (PROPS, Some(p)) => Outcome::Properties(p),
                    _ => Outcome::Redraw,
                }
            }
            propsheet::Event::Page(..) => Outcome::Redraw,
            propsheet::Event::Apply => {
                if self.pending == self.applied {
                    return Outcome::Redraw;
                }
                self.applied = self.pending;
                self.write();
                Outcome::Apply(self.pending)
            }
            propsheet::Event::Ok => {
                if self.pending == self.applied {
                    return Outcome::Close;
                }
                self.applied = self.pending;
                self.write();
                Outcome::ApplyAndClose(self.pending)
            }
            propsheet::Event::Cancel => {
                self.pending = self.applied;
                self.write();
                Outcome::Close
            }
        }
    }

    pub fn list_press(&mut self, at: Click, cm: &ControlMetrics) -> Outcome {
        if self.blank_text().is_some() {
            return Outcome::None;
        }
        self.list_focus = true;
        self.sheet.open_mut().set_focus(None);
        match self.list.press(self.list_rect(), at, cm) {
            Press::None => Outcome::None,
            Press::Redraw => Outcome::Redraw,
            Press::Row {
                changed, activate, ..
            } => {
                self.write();
                match (activate, self.selected_path()) {
                    (true, Some(p)) => Outcome::Toggle(p),
                    _ if changed => Outcome::Redraw,
                    _ => Outcome::None,
                }
            }
        }
    }

    /// A key while the list holds the keyboard; `None` leaves it to the sheet
    /// (always, while the page is blanked).
    pub fn list_key(&mut self, key: &form::Key, cm: &ControlMetrics) -> Option<Outcome> {
        if self.blank_text().is_some() {
            return None;
        }
        if *key == form::Key::Enter {
            return self.selected_path().map(Outcome::Properties);
        }
        if self.list.key(key, self.list_rect(), cm) {
            self.write();
            return Some(Outcome::Redraw);
        }
        None
    }

    /// Delete on the Devices tab; None while the page is blanked.
    pub fn delete_key(&self) -> Outcome {
        match self.selected_path() {
            Some(p) if self.blank_text().is_none() => Outcome::Remove(p),
            _ => Outcome::None,
        }
    }

    pub fn access(&self, tree: &mut Tree, ctx: &Ctx) {
        let px = self.sheet.open().units().px(ctx.theme.fonts.size_px);
        let panel = oneshot::access_sheet(&self.sheet, tree, &ctx.text, px);
        let list = self.list_rect();
        if self.sheet.tabs.open != TAB_DEVICES {
            return;
        }
        if let Some(line) = self.blank_text() {
            tree.push(panel, "empty", access::label(list, line, true));
            return;
        }
        let lay = tilelist::layout(&self.list, list, &ctx.theme.look.controls());
        let row = tilelist::access(
            &self.list,
            &lay,
            |i| (self.rows[i].alias.clone(), bz::status_line(&self.rows[i])),
            tree,
            panel,
            "devices",
        );
        if self.list_focus {
            tree.set_focus(row);
        }
    }

    pub fn draw(&self, c: &mut Canvas, ctx: &Ctx, icons: &Icons) {
        let cs = &ctx.theme.colors;
        let forms = ctx.forms();
        c.fill_rect(c.bounds(), cs.button_face);
        propsheet::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..forms.paint()
            },
            &self.sheet,
            true,
            forms.fonts.size_px,
        );
        if self.sheet.tabs.open != TAB_DEVICES {
            return;
        }
        let clip = self.sheet.page_clip(&ctx.text, forms.fonts.size_px);
        c.with_clip(clip.unwrap_or(c.bounds()), |c| {
            self.draw_list(c, ctx, icons)
        });
    }

    fn draw_list(&self, c: &mut Canvas, ctx: &Ctx, icons: &Icons) {
        let cs = &ctx.theme.colors;
        let px = ctx.theme.fonts.size_px;
        let list = self.list_rect();
        if let Some(line) = self.blank_text() {
            let empty = TileList::new(TILE_H);
            let lay = tilelist::layout(&empty, list, &ctx.theme.look.controls());
            tilelist::draw(
                c,
                &ctx.text,
                &Paint {
                    s: 1,
                    ..ctx.theme.paint()
                },
                &empty,
                &lay,
                px,
                |_, _, _, _| {},
            );
            let r = Rect::new(list.x, list.y + 24, list.width, LINE_H);
            ctx.text
                .draw(c, r, line, false, cs.gray_text, Align::Center, px);
            return;
        }
        let lay = tilelist::layout(&self.list, list, &ctx.theme.look.controls());
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
            |c, r, i, sel| {
                let d = &self.rows[i];
                let caption = bz::status_line(d);
                let tile = Tile {
                    icon: Some(icons.get(d.kind)),
                    title: &d.alias,
                    tag: "",
                    caption: &caption,
                    greyed: false,
                };
                tilelist::draw_tile(c, &ctx.text, cs, r, &tile, sel, px, lay.text);
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bz::{Adapter, Kind};
    use reprisekit::controls::form::Control;

    fn at(x: i32, y: i32, time: u32) -> Click {
        reprisekit::theme::Metrics::default().click(x, y, time)
    }

    fn dev(path: &str, alias: &str, kind: Kind, connected: bool) -> Device {
        Device {
            path: path.into(),
            alias: alias.into(),
            kind,
            paired: true,
            connected,
            ..Device::default()
        }
    }

    fn snap(powered: bool, devices: Vec<Device>) -> Snapshot {
        Snapshot {
            adapter: Some(Adapter {
                path: "/org/bluez/hci0".into(),
                powered,
                ..Adapter::default()
            }),
            devices,
        }
    }

    fn four() -> Snapshot {
        snap(
            true,
            vec![
                dev("/d/1", "Cans", Kind::Headphones, true),
                dev("/d/2", "Mouse", Kind::Mouse, false),
                dev("/d/3", "Pad", Kind::Gamepad, false),
                dev("/d/4", "Phone", Kind::Phone, false),
                Device {
                    paired: false,
                    ..dev("/d/5", "Stranger", Kind::Other, false)
                },
            ],
        )
    }

    fn enabled(d: &Devices, id: &str) -> bool {
        matches!(
            d.sheet.pages[TAB_DEVICES].item(id).map(|i| &i.control),
            Some(Control::Button { enabled: true, .. })
        )
    }

    fn label(d: &Devices, id: &str) -> String {
        match d.sheet.pages[TAB_DEVICES].item(id).map(|i| &i.control) {
            Some(Control::Button { label, .. }) => label.clone(),
            _ => String::new(),
        }
    }

    fn entry_point(d: &Devices, e: usize) -> (i32, i32) {
        let lay = tilelist::layout(&d.list, LIST, &ControlMetrics::CLASSIC);
        let s = lay.slots.iter().find(|s| s.entry == e).expect("on screen");
        (s.rect.x + 40, s.rect.y + s.rect.height / 2)
    }

    #[test]
    fn paired_devices_are_listed_under_group_headers() {
        let mut d = Devices::new();
        d.refresh(&four());
        assert_eq!(
            d.list.entries,
            vec![
                Entry::Header("Audio".into()),
                Entry::Row(0),
                Entry::Header("Input devices".into()),
                Entry::Row(1),
                Entry::Row(2),
                Entry::Header("Phones".into()),
                Entry::Row(3),
            ]
        );
        let names: Vec<&str> = d.rows.iter().map(|r| r.alias.as_str()).collect();
        assert_eq!(
            names,
            ["Cans", "Mouse", "Pad", "Phone"],
            "the unpaired one is left out"
        );
    }

    #[test]
    fn buttons_follow_the_selection() {
        let mut d = Devices::new();
        d.refresh(&four());
        assert!(enabled(&d, ADD));
        assert!(!enabled(&d, REMOVE) && !enabled(&d, CONNECT) && !enabled(&d, PROPS));
        let (x, y) = entry_point(&d, 1);
        d.list_press(at(x, y, 0), &ControlMetrics::CLASSIC);
        assert!(enabled(&d, REMOVE) && enabled(&d, CONNECT) && enabled(&d, PROPS));
        assert_eq!(label(&d, CONNECT), "&Disconnect", "Cans is connected");
        d.list_key(&form::Key::Down, &ControlMetrics::CLASSIC);
        assert_eq!(label(&d, CONNECT), "&Connect", "Mouse is not");
    }

    #[test]
    fn alt_p_is_properties_and_alt_r_is_remove() {
        let mut d = Devices::new();
        d.refresh(&four());
        let (x, y) = entry_point(&d, 1);
        d.list_press(at(x, y, 0), &ControlMetrics::CLASSIC);
        let page = &mut d.sheet.pages[TAB_DEVICES];
        assert_eq!(
            page.key(form::Key::Access('p')),
            form::Event::Pressed(PROPS)
        );
        assert_eq!(
            page.key(form::Key::Access('r')),
            form::Event::Pressed(REMOVE)
        );
    }

    #[test]
    fn a_vanished_device_drops_its_selection() {
        let mut d = Devices::new();
        d.refresh(&four());
        let (x, y) = entry_point(&d, 3);
        d.list_press(at(x, y, 0), &ControlMetrics::CLASSIC);
        assert_eq!(d.selected().map(|r| r.alias.as_str()), Some("Mouse"));
        let mut s = four();
        s.devices.retain(|r| r.alias != "Mouse");
        d.refresh(&s);
        assert_eq!(d.selected(), None);
        assert!(!enabled(&d, REMOVE) && !enabled(&d, CONNECT) && !enabled(&d, PROPS));
    }

    #[test]
    fn the_selection_follows_its_device_across_a_refresh() {
        let mut d = Devices::new();
        d.refresh(&four());
        let (x, y) = entry_point(&d, 6);
        d.list_press(at(x, y, 0), &ControlMetrics::CLASSIC);
        let mut s = four();
        s.devices.push(dev("/d/6", "Buds", Kind::Headphones, false));
        d.refresh(&s);
        assert_eq!(d.selected().map(|r| r.alias.as_str()), Some("Phone"));
    }

    #[test]
    fn radio_off_blanks_the_page() {
        let mut d = Devices::new();
        d.refresh(&snap(false, four().devices));
        assert!(!enabled(&d, ADD));
        let (x, y) = (LIST.x + 40, LIST.y + 40);
        assert_eq!(
            d.list_press(at(x, y, 0), &ControlMetrics::CLASSIC),
            Outcome::None
        );
        d.refresh(&Snapshot::default());
        assert!(!enabled(&d, ADD));
        assert_eq!(d.blank_text(), Some("No Bluetooth adapter was found."));
    }

    #[test]
    fn double_click_toggles_and_enter_opens_properties() {
        let mut d = Devices::new();
        d.refresh(&four());
        let (x, y) = entry_point(&d, 1);
        d.list_press(at(x, y, 0), &ControlMetrics::CLASSIC);
        assert_eq!(
            d.list_press(at(x, y, 100), &ControlMetrics::CLASSIC),
            Outcome::Toggle("/d/1".into())
        );
        assert_eq!(
            d.list_key(&form::Key::Enter, &ControlMetrics::CLASSIC),
            Some(Outcome::Properties("/d/1".into()))
        );
        assert_eq!(d.delete_key(), Outcome::Remove("/d/1".into()));
    }

    #[test]
    fn a_hidden_selection_answers_nothing_once_the_radio_goes_off() {
        let mut d = Devices::new();
        d.refresh(&four());
        let (x, y) = entry_point(&d, 1);
        d.list_press(at(x, y, 0), &ControlMetrics::CLASSIC);
        d.sheet.tabs.open = TAB_OPTIONS;
        d.sheet.pages[TAB_OPTIONS].set_check("power", false);
        d.event(propsheet::Event::Page(
            TAB_OPTIONS,
            form::Event::Changed("power"),
        ));
        assert_eq!(
            d.event(propsheet::Event::Apply),
            Outcome::Apply(Options {
                powered: false,
                discoverable: false
            })
        );
        d.sheet.tabs.open = TAB_DEVICES;
        assert!(!enabled(&d, REMOVE) && !enabled(&d, CONNECT) && !enabled(&d, PROPS));
        assert_eq!(d.delete_key(), Outcome::None);
        assert_eq!(
            d.list_key(&form::Key::Enter, &ControlMetrics::CLASSIC),
            None
        );
    }

    #[test]
    fn options_apply_carries_the_pending_radio_state() {
        let mut d = Devices::new();
        d.refresh(&four());
        d.sheet.tabs.open = TAB_OPTIONS;
        d.sheet.pages[TAB_OPTIONS].set_check("discoverable", true);
        assert_eq!(
            d.event(propsheet::Event::Page(
                TAB_OPTIONS,
                form::Event::Changed("discoverable")
            )),
            Outcome::Redraw
        );
        assert_eq!(
            d.event(propsheet::Event::Apply),
            Outcome::Apply(Options {
                powered: true,
                discoverable: true
            })
        );
        assert_eq!(
            d.event(propsheet::Event::Ok),
            Outcome::Close,
            "nothing left to apply"
        );
    }

    #[test]
    fn a_snapshot_does_not_overwrite_an_unapplied_change() {
        let mut d = Devices::new();
        d.refresh(&four());
        d.sheet.pages[TAB_OPTIONS].set_check("power", false);
        d.event(propsheet::Event::Page(
            TAB_OPTIONS,
            form::Event::Changed("power"),
        ));
        d.refresh(&four());
        assert_eq!(d.sheet.pages[TAB_OPTIONS].checked("power"), Some(false));
        assert_eq!(d.event(propsheet::Event::Cancel), Outcome::Close);
        assert_eq!(
            d.sheet.pages[TAB_OPTIONS].checked("power"),
            Some(true),
            "Cancel reverts"
        );
    }
}
