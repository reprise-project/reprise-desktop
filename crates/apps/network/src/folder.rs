//! The Network Connections folder: a task pane beside a Details list of the adapters and saved VPNs. The client is the list and pane band alone, and all coordinates are in it.

use repriseapp::{BTN_LEFT, BTN_RIGHT, Ctx, KeyInput};
use reprisekit::access::{ROOT, Tree};
use reprisekit::art::neticon::{self, NetIcon};
use reprisekit::classic::widgets::{Axis, BarHit, ScrollHit};
use reprisekit::controls::listview::{self, Column, ListView};
use reprisekit::controls::menu::{MenuAction, MenuCtx, MenuItem, MenuKey, MenuTree, PopupMenu};
use reprisekit::controls::taskpane::{self, Body, Section, TaskPane};
use reprisekit::look::Paint;
use reprisekit::pointer::Click;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::selection::Mods;
use reprisekit::textedit::keysym;

use crate::nm;
use crate::status;

pub const CLIENT: (i32, i32) = (792, 470);

const PANE_W: i32 = 200;
/// The list is borderless: y 1 puts the header's face on y 2 and the body on y 18.
const LIST_X: i32 = 210;
const LIST_Y: i32 = 1;

/// Device Name takes the rest of the body ([`Folder::fit_columns`]), so the columns never overflow into a horizontal scrollbar.
const NAME_W: i32 = 240;
const TYPE_W: i32 = 144;
const STATUS_W: i32 = 144;
const DEVICE_W: i32 = CLIENT.0 - LIST_X - (NAME_W + TYPE_W + STATUS_W);

const TYPE_LAN: &str = "LAN or High-Speed Internet";
const TYPE_VPN: &str = "Virtual Private Network";

const TASKS: usize = 0;
const DETAILS: usize = 3;

/// One line of the list: a device or a saved VPN, by the object path it came from so a refresh can find it again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Device(String),
    Vpn(String),
}

impl Row {
    fn key(&self) -> &str {
        match self {
            Row::Device(p) | Row::Vpn(p) => p,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    OpenStatus(String),
    Toggle(String),
    Repair(String),
    ViewWireless(String),
    VpnUp(String),
    VpnDown(String),
    VpnProperties(String),
    VpnDelete(String),
    NewWireGuard,
    Import,
    CreateHotspot(String),
    StopHotspot(String),
}

/// An open context menu: the tree it walks, the chain reprisekit lays out, and what each row does (`None` for a separator or greyed row). It draws inside the window's surface, not a popup.
pub struct Popup {
    tree: MenuTree,
    chain: PopupMenu,
    actions: Vec<Option<Outcome>>,
}

/// The list's rows, cells and groups from a snapshot: adapters first, saved VPNs under their own heading; empty without NetworkManager. Icons are left to [`Folder::refresh`], which has the icon theme.
pub fn rows_for(s: &nm::Snapshot) -> (Vec<Row>, Vec<listview::Row>, Vec<listview::Group>) {
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut groups = Vec::new();
    if !s.nm_running {
        return (rows, cells, groups);
    }
    if !s.devices.is_empty() {
        groups.push(listview::Group {
            title: TYPE_LAN.into(),
            first: 0,
        });
    }
    for d in &s.devices {
        rows.push(Row::Device(d.path.clone()));
        cells.push(listview::Row {
            icon: None,
            cells: vec![
                d.shell_name.clone(),
                TYPE_LAN.into(),
                device_status(d),
                d.product.clone(),
            ],
        });
    }
    if !s.vpns.is_empty() {
        groups.push(listview::Group {
            title: TYPE_VPN.into(),
            first: s.devices.len(),
        });
    }
    for v in &s.vpns {
        rows.push(Row::Vpn(v.conn_path.clone()));
        cells.push(listview::Row {
            icon: None,
            cells: vec![
                v.name.clone(),
                TYPE_VPN.into(),
                vpn_status(v).into(),
                nm::vpn_kind_label(&v.kind),
            ],
        });
    }
    (rows, cells, groups)
}

fn device_status(d: &nm::Device) -> String {
    match &d.hotspot {
        Some(ssid) if d.state == nm::State::Connected => format!("Hotspot: {ssid}"),
        Some(_) => "Starting hotspot".into(),
        None => status::status_word(d).into(),
    }
}

fn vpn_status(v: &nm::Vpn) -> &'static str {
    if v.active {
        "Connected"
    } else {
        "Disconnected"
    }
}

/// The row's context menu and what each row does. A device carries Disable / Status / Repair, a radio the wireless chooser under a separator; a disabled device offers Enable and greys the rest.
pub fn context_menu(row: &Row, s: &nm::Snapshot) -> (Vec<MenuItem>, Vec<Option<Outcome>>) {
    match row {
        Row::Device(path) => {
            let Some(d) = s.devices.iter().find(|d| &d.path == path) else {
                return (Vec::new(), Vec::new());
            };
            let live = d.managed;
            let mut items = vec![
                item(if live { "&Disable" } else { "&Enable" }, true),
                item("Stat&us", live),
                item("&Repair", live),
            ];
            let mut actions = vec![
                Some(Outcome::Toggle(path.clone())),
                live.then(|| Outcome::OpenStatus(path.clone())),
                live.then(|| Outcome::Repair(path.clone())),
            ];
            if d.kind == nm::Kind::Wifi {
                items.push(MenuItem::Separator);
                items.push(item("&View Available Wireless Networks", live));
                actions.push(None);
                actions.push(live.then(|| Outcome::ViewWireless(path.clone())));
                if d.hotspot.is_some() {
                    items.push(item("Stop &Hotspot", true));
                    actions.push(Some(Outcome::StopHotspot(path.clone())));
                } else {
                    items.push(item("Create &Hotspot...", live));
                    actions.push(live.then(|| Outcome::CreateHotspot(path.clone())));
                }
            }
            (items, actions)
        }
        Row::Vpn(conn) => {
            let active = s
                .vpns
                .iter()
                .find(|v| &v.conn_path == conn)
                .is_some_and(|v| v.active);
            let (label, out) = if active {
                ("&Disconnect", Outcome::VpnDown(conn.clone()))
            } else {
                ("&Connect", Outcome::VpnUp(conn.clone()))
            };
            (
                vec![
                    item(label, true),
                    MenuItem::Separator,
                    item("&Delete", true),
                    item("P&roperties", true),
                ],
                vec![
                    Some(out),
                    None,
                    Some(Outcome::VpnDelete(conn.clone())),
                    Some(Outcome::VpnProperties(conn.clone())),
                ],
            )
        }
    }
}

const TASK_WIREGUARD: usize = 0;
const TASK_IMPORT: usize = 1;
const TASK_HOTSPOT: usize = 2;

fn hotspot_radio(s: &nm::Snapshot) -> Option<&nm::Device> {
    crate::tray::wifi_device(s).filter(|d| d.managed && d.hotspot.is_none())
}

fn item(label: &str, enabled: bool) -> MenuItem {
    MenuItem::item(label).enabled(enabled)
}

fn task_links(s: &nm::Snapshot) -> Vec<taskpane::Row> {
    let link = |text: &str, enabled| taskpane::Row {
        icon: None,
        text: text.into(),
        enabled,
    };
    vec![
        link("Create a WireGuard connection", s.nm_running),
        link("Import a VPN connection", s.nm_running),
        link("Create a hotspot", hotspot_radio(s).is_some()),
    ]
}

/// The pane's four sections. Links past Network Tasks open nothing this applet has, so they are drawn greyed. Rows carry no icon: this pure builder has no icon theme.
fn pane_sections() -> Vec<Section> {
    let link = |text: &str| taskpane::Row {
        icon: None,
        text: text.into(),
        enabled: false,
    };
    vec![
        Section::links("Network Tasks", task_links(&nm::Snapshot::default())),
        Section::links("See Also", vec![link("Network Troubleshooter")]),
        Section::links(
            "Other Places",
            vec![
                link("Control Panel"),
                link(&reprisefs::places::home_label()),
                link("Documents"),
                link("Computer"),
            ],
        ),
        Section::lines("Details", folder_lines()),
    ]
}

fn details_lines(row: Option<&Row>, s: &nm::Snapshot) -> Vec<(String, bool)> {
    if !s.nm_running {
        return vec![("NetworkManager is not running".into(), true)];
    }
    match row {
        Some(Row::Device(path)) => match s.devices.iter().find(|d| &d.path == path) {
            Some(d) => vec![
                (d.shell_name.clone(), true),
                (TYPE_LAN.into(), false),
                (device_status(d), false),
            ],
            None => folder_lines(),
        },
        Some(Row::Vpn(conn)) => match s.vpns.iter().find(|v| &v.conn_path == conn) {
            Some(v) => vec![
                (v.name.clone(), true),
                (TYPE_VPN.into(), false),
                (vpn_status(v).into(), false),
            ],
            None => folder_lines(),
        },
        None => folder_lines(),
    }
}

fn folder_lines() -> Vec<(String, bool)> {
    vec![
        ("Network Connections".into(), true),
        ("System Folder".into(), false),
    ]
}

/// The window's model: task pane and list. Nothing here reads the bus; the host owns the snapshot and acts on the [`Outcome`]s.
pub struct Folder {
    pub pane: TaskPane,
    pub list: ListView,
    rows: Vec<Row>,
    pub popup: Option<Popup>,
    thumb_drag: Option<i32>,
}

impl Default for Folder {
    fn default() -> Self {
        Folder::new()
    }
}

impl Folder {
    pub fn new() -> Folder {
        Folder {
            pane: TaskPane {
                sections: pane_sections(),
                ..Default::default()
            },
            list: ListView {
                columns: vec![
                    Column::new("Name", NAME_W),
                    Column::new("Type", TYPE_W),
                    Column::new("Status", STATUS_W),
                    Column::new("Device Name", DEVICE_W),
                ],
                frame: false,
                ..Default::default()
            },
            rows: Vec::new(),
            popup: None,
            thumb_drag: None,
        }
    }

    pub fn refresh(&mut self, s: &nm::Snapshot, vpn_icon: Option<Canvas>) {
        let selected = self
            .list
            .selection
            .single()
            .and_then(|i| self.rows.get(i))
            .map(|r| r.key().to_string());
        let (rows, mut cells, groups) = rows_for(s);
        for (row, cell) in rows.iter().zip(cells.iter_mut()) {
            cell.icon = Some(std::rc::Rc::new(match row {
                Row::Device(path) => match s.devices.iter().find(|d| &d.path == path) {
                    Some(d) => neticon::draw(status::device_icon(d), false),
                    None => neticon::draw(NetIcon::Wired, false),
                },
                Row::Vpn(_) => vpn_icon
                    .clone()
                    .unwrap_or_else(|| neticon::draw(NetIcon::Wired, true)),
            }));
        }
        self.rows = rows;
        self.list.rows = cells;
        self.list.groups = groups;
        let keep = selected.and_then(|key| self.rows.iter().position(|r| r.key() == key));
        self.list.selection.set_only(keep);
        self.list.top = self.list.top.min(self.list.rows.len().saturating_sub(1));
        if let Some(tasks) = self.pane.sections.get_mut(TASKS) {
            tasks.body = Body::Links(task_links(s));
        }
        self.set_details(s);
    }

    /// A new snapshot: rows, icons and Details lines. The selection follows its row's path. `vpn_icon` is the theme's "network-vpn" at 16 px; without it a VPN row gets the wired icon's VPN badge.
    pub fn follow(&mut self, ctx: &Ctx) {
        let (_, list_r) = self.layout(ctx);
        let lay = listview::layout(&self.list, list_r, &ctx.theme.look.controls());
        self.list.follow(&lay);
    }

    /// Scroll the kept selection into view; the host calls it after [`Folder::refresh`], which has no [`Ctx`].
    pub fn fit_columns(&mut self, ctx: &Ctx) {
        let (_, list_r) = self.layout(ctx);
        let lay = listview::layout(&self.list, list_r, &ctx.theme.look.controls());
        let fixed: i32 = self.list.columns[..3].iter().map(|c| c.width).sum();
        self.list.resize_column(3, lay.body.width - fixed);
    }

    /// Give Device Name what the body has left, so the last column takes the rest. The host calls it on resize.
    fn set_details(&mut self, s: &nm::Snapshot) {
        let row = self.list.selection.single().and_then(|i| self.rows.get(i));
        let lines = details_lines(row, s);
        if let Some(s) = self.pane.sections.get_mut(DETAILS) {
            s.body = Body::Lines(lines);
        }
    }

    pub fn layout(&self, ctx: &Ctx) -> (Rect, Rect) {
        let (w, h) = ctx.size;
        (
            Rect::new(0, 0, PANE_W, h),
            Rect::new(LIST_X, LIST_Y, (w - LIST_X).max(1), (h - LIST_Y).max(1)),
        )
    }

    #[cfg(test)]
    pub fn row(&self, i: usize) -> Option<&Row> {
        self.rows.get(i)
    }

    pub fn wheel(&mut self, rows: i32, ctx: &Ctx) -> Outcome {
        let (_, list_r) = self.layout(ctx);
        let lay = listview::layout(&self.list, list_r, &ctx.theme.look.controls());
        redraw(self.list.scroll(rows as i64, &lay))
    }

    pub fn press(&mut self, at: Click, button: u32, s: &nm::Snapshot, ctx: &Ctx) -> Outcome {
        let (x, y) = (at.x, at.y);
        // A pointer press. An open menu takes every press; one outside closes it and is eaten. Otherwise the right button selects and opens the row's menu, a double click opens a device's Status window or toggles a VPN, a single click selects.
        let lit = button == BTN_LEFT
            && {
                let over = self.bar_at(x, y, ctx);
                let column = self.column_at(x, y, ctx);
                self.list.bars.press_bar(over) | self.list.header.press(column, false)
            }
            && ctx.styled();
        match self.press_at(at, button, s, ctx) {
            Outcome::None if lit => Outcome::Redraw,
            out => out,
        }
    }

    fn bar_at(&self, x: i32, y: i32, ctx: &Ctx) -> Option<(Axis, ScrollHit)> {
        let (_, list_r) = self.layout(ctx);
        if self.popup.is_some() || !ctx.styled() || !list_r.contains(x, y) {
            return None;
        }
        let lay = listview::layout(&self.list, list_r, &ctx.theme.look.controls());
        let on = |r: Option<Rect>| r.is_some_and(|r| r.contains(x, y));
        if !on(lay.scrollbar) && !on(lay.hscroll) {
            return None;
        }
        self.list.hit(&lay, x, y).bar()
    }

    fn column_at(&self, x: i32, y: i32, ctx: &Ctx) -> Option<usize> {
        let (_, list_r) = self.layout(ctx);
        if self.popup.is_some() || !ctx.styled() || !list_r.contains(x, y) {
            return None;
        }
        let lay = listview::layout(&self.list, list_r, &ctx.theme.look.controls());
        self.list.hit(&lay, x, y).column()
    }

    fn press_at(&mut self, at: Click, button: u32, s: &nm::Snapshot, ctx: &Ctx) -> Outcome {
        let (x, y) = (at.x, at.y);
        if let Some(p) = self.popup.as_mut() {
            let act = p.chain.click(&p.tree, x, y, &popup_ctx(ctx, client(ctx)));
            return self.popup_action(act);
        }
        let (pane_r, list_r) = self.layout(ctx);
        if list_r.contains(x, y) {
            let lay = listview::layout(&self.list, list_r, &ctx.theme.look.controls());
            let hit = self.list.hit(&lay, x, y);
            // Only a left press pairs, so a right click between two left ones does not break the pair.
            let activated = button == BTN_LEFT && self.list.activated(hit, at).is_some();
            return match hit {
                listview::Hit::Row(i) => {
                    self.list.select(Some(i), &lay);
                    self.set_details(s);
                    match button {
                        BTN_RIGHT => {
                            self.open_popup(i, x, y, s, ctx);
                            Outcome::Redraw
                        }
                        _ if activated => self.activate(i, s),
                        _ => Outcome::Redraw,
                    }
                }
                listview::Hit::Background => {
                    self.list.select(None, &lay);
                    self.set_details(s);
                    Outcome::Redraw
                }
                listview::Hit::VBar(BarHit::Back) => redraw(self.list.scroll(-1, &lay)),
                listview::Hit::VBar(BarHit::Forward) => redraw(self.list.scroll(1, &lay)),
                listview::Hit::VBar(BarHit::PageBack) => {
                    redraw(self.list.scroll(-(lay.rows as i64), &lay))
                }
                listview::Hit::VBar(BarHit::PageForward) => {
                    redraw(self.list.scroll(lay.rows as i64, &lay))
                }
                listview::Hit::VBar(BarHit::Thumb(grab)) => {
                    self.thumb_drag = Some(grab);
                    Outcome::None
                }
                listview::Hit::VBar(BarHit::Track) => Outcome::None,
                // The columns are fitted to the body, so the horizontal bar only shows on a window too narrow for them.
                listview::Hit::HBar(BarHit::Back) => redraw(self.list.hscroll(-NAME_W, &lay)),
                listview::Hit::HBar(BarHit::Forward) => redraw(self.list.hscroll(NAME_W, &lay)),
                listview::Hit::HBar(BarHit::PageBack) => {
                    redraw(self.list.hscroll(-lay.body.width, &lay))
                }
                listview::Hit::HBar(BarHit::PageForward) => {
                    redraw(self.list.hscroll(lay.body.width, &lay))
                }
                _ => Outcome::None,
            };
        }
        if pane_r.contains(x, y) {
            let lay = taskpane::layout(
                &self.pane,
                &ctx.text,
                ctx.theme.fonts.size_px,
                pane_r,
                &ctx.theme.look.controls(),
            );
            let hit = taskpane::hit(&self.pane, &lay, x, y);
            // Only the Network Tasks links are live; any other click opens or shuts a section.
            return match self.pane.click(hit) {
                taskpane::Event::None => Outcome::None,
                taskpane::Event::Row(TASKS, TASK_WIREGUARD) => Outcome::NewWireGuard,
                taskpane::Event::Row(TASKS, TASK_IMPORT) => Outcome::Import,
                taskpane::Event::Row(TASKS, TASK_HOTSPOT) => match hotspot_radio(s) {
                    Some(d) => Outcome::CreateHotspot(d.path.clone()),
                    None => Outcome::None,
                },
                _ => Outcome::Redraw,
            };
        }
        Outcome::None
    }

    pub fn motion(&mut self, x: i32, y: i32, ctx: &Ctx) -> Outcome {
        let over = self.bar_at(x, y, ctx);
        let column = self.column_at(x, y, ctx);
        let lit = (self.list.bars.motion(over) | self.list.header.motion(column)) && ctx.styled();
        match self.drag_or_menu(x, y, ctx) {
            Outcome::None if lit => Outcome::Redraw,
            out => out,
        }
    }

    pub fn leave(&mut self, ctx: &Ctx) -> Outcome {
        redraw((self.list.bars.leave() | self.list.header.leave()) && ctx.styled())
    }

    fn drag_or_menu(&mut self, x: i32, y: i32, ctx: &Ctx) -> Outcome {
        if let Some(grab) = self.thumb_drag {
            let (_, list_r) = self.layout(ctx);
            let lay = listview::layout(&self.list, list_r, &ctx.theme.look.controls());
            let top = self.list.top_at(&lay, y, grab);
            let moved = top != self.list.top;
            self.list.top = top;
            return redraw(moved);
        }
        let client = client(ctx);
        let Some(p) = self.popup.as_mut() else {
            return Outcome::None;
        };
        redraw(p.chain.motion(&p.tree, x, y, &popup_ctx(ctx, client)))
    }

    pub fn release(&mut self, ctx: &Ctx) -> Outcome {
        self.thumb_drag = None;
        let column = self.list.header.holding();
        self.list.header.release();
        redraw((self.list.bars.release_bar() || column) && ctx.styled())
    }

    /// A key. An open menu takes the arrows, Enter and Escape; otherwise Up and Down step the list and Enter acts as a double click.
    pub fn key(&mut self, k: &KeyInput, s: &nm::Snapshot, ctx: &Ctx) -> Outcome {
        if let Some(p) = self.popup.as_mut() {
            let Some(mk) = menu_key(k) else {
                return Outcome::None;
            };
            let act = p.chain.key(&p.tree, mk, &popup_ctx(ctx, client(ctx)));
            return self.popup_action(act);
        }
        let (_, list_r) = self.layout(ctx);
        let lay = listview::layout(&self.list, list_r, &ctx.theme.look.controls());
        match k.sym {
            keysym::UP | keysym::KP_UP => self.step(-1, &lay, s),
            keysym::DOWN | keysym::KP_DOWN => self.step(1, &lay, s),
            keysym::HOME | keysym::KP_HOME => self.step(i64::MIN / 2, &lay, s),
            keysym::END | keysym::KP_END => self.step(i64::MAX / 2, &lay, s),
            keysym::RETURN | keysym::KP_ENTER => match self.list.selection.single() {
                Some(i) => self.activate(i, s),
                None => Outcome::None,
            },
            keysym::DELETE | keysym::KP_DELETE => {
                match self.list.selection.single().and_then(|i| self.rows.get(i)) {
                    Some(Row::Vpn(conn)) => Outcome::VpnDelete(conn.clone()),
                    _ => Outcome::None,
                }
            }
            _ => Outcome::None,
        }
    }

    fn step(&mut self, step: i64, lay: &listview::Layout, s: &nm::Snapshot) -> Outcome {
        let moved = self.list.step(step, lay, Mods::default());
        self.set_details(s);
        redraw(moved)
    }

    pub fn draw(&self, c: &mut Canvas, ctx: &Ctx) {
        let (pane_r, list_r) = self.layout(ctx);
        let cs = &ctx.theme.colors;
        let px = ctx.theme.fonts.size_px;
        // The band behind both is `window`: white either side of the
        // pane's sections and up to the list's first column.
        c.fill_rect(c.bounds(), cs.window);
        let pl = taskpane::layout(
            &self.pane,
            &ctx.text,
            px,
            pane_r,
            &ctx.theme.look.controls(),
        );
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
        let ll = listview::layout(&self.list, list_r, &ctx.theme.look.controls());
        listview::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &self.list,
            &ll,
            self.list.bars.lit(),
            px,
        );
        if let Some(p) = &self.popup {
            p.chain.draw(c, &ctx.theme, &ctx.text, true);
        }
    }

    pub fn access(&self, tree: &mut Tree, ctx: &Ctx) {
        let (pane_r, list_r) = self.layout(ctx);
        let px = ctx.theme.fonts.size_px;
        let pl = taskpane::layout(
            &self.pane,
            &ctx.text,
            px,
            pane_r,
            &ctx.theme.look.controls(),
        );
        taskpane::access(&self.pane, &pl, tree, ROOT);
        let ll = listview::layout(&self.list, list_r, &ctx.theme.look.controls());
        let row = self.list.access(&ll, tree, ROOT, "connections");
        let popup = self
            .popup
            .as_ref()
            .map(|p| p.chain.access(tree, ROOT, "popup"));
        tree.set_focus(popup.unwrap_or(row));
    }

    fn open_popup(&mut self, i: usize, x: i32, y: i32, s: &nm::Snapshot, ctx: &Ctx) {
        let Some(row) = self.rows.get(i) else {
            return;
        };
        let (items, actions) = context_menu(row, s);
        if items.is_empty() {
            return;
        }
        let tree = MenuTree::flat(items);
        let chain = PopupMenu::open(&tree, (x, y), &popup_ctx(ctx, client(ctx)), false);
        self.popup = Some(Popup {
            tree,
            chain,
            actions,
        });
    }

    fn popup_action(&mut self, act: MenuAction) -> Outcome {
        match act {
            MenuAction::Chosen(path) => {
                let out = self
                    .popup
                    .as_ref()
                    .and_then(|p| path.first().and_then(|&i| p.actions.get(i).cloned()))
                    .flatten();
                self.popup = None;
                out.unwrap_or(Outcome::Redraw)
            }
            MenuAction::Close => {
                self.popup = None;
                Outcome::Redraw
            }
            MenuAction::Redraw => Outcome::Redraw,
            MenuAction::None => Outcome::None,
        }
    }

    fn activate(&self, i: usize, s: &nm::Snapshot) -> Outcome {
        match self.rows.get(i) {
            Some(Row::Device(path)) => Outcome::OpenStatus(path.clone()),
            Some(Row::Vpn(conn)) => {
                let active = s
                    .vpns
                    .iter()
                    .find(|v| &v.conn_path == conn)
                    .is_some_and(|v| v.active);
                if active {
                    Outcome::VpnDown(conn.clone())
                } else {
                    Outcome::VpnUp(conn.clone())
                }
            }
            None => Outcome::None,
        }
    }
}

fn redraw(changed: bool) -> Outcome {
    if changed {
        Outcome::Redraw
    } else {
        Outcome::None
    }
}

fn client(ctx: &Ctx) -> Rect {
    Rect::new(0, 0, ctx.size.0, ctx.size.1)
}

fn popup_ctx<'a>(ctx: &'a Ctx, client: Rect) -> MenuCtx<'a> {
    MenuCtx {
        text: &ctx.text,
        theme: &ctx.theme,
        px: ctx.theme.fonts.size_px,
        within: client,
    }
}

fn menu_key(k: &KeyInput) -> Option<MenuKey> {
    Some(match k.sym {
        keysym::UP | keysym::KP_UP => MenuKey::Up,
        keysym::DOWN | keysym::KP_DOWN => MenuKey::Down,
        keysym::LEFT | keysym::KP_LEFT => MenuKey::Left,
        keysym::RIGHT | keysym::KP_RIGHT => MenuKey::Right,
        keysym::RETURN | keysym::KP_ENTER => MenuKey::Enter,
        keysym::ESCAPE => MenuKey::Escape,
        _ => MenuKey::Char(k.utf8.as_ref()?.chars().next()?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::controls::metrics::ControlMetrics;

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

    /// In client coordinates: the pane's first section border and header top, the list header's face top, the body's left edge, the column dividers, the group rule and the first row's top.
    const SECTION_X: i32 = 12;
    const SECTION_RIGHT: i32 = 198;
    const PANE_HEADER_Y: i32 = 13;
    const LIST_HEADER_Y: i32 = 2;
    const BODY_X: i32 = 210;
    const DIVIDERS: [i32; 3] = [450, 594, 738];
    const BODY_RIGHT: i32 = 792;
    const GROUP_RULE_Y: i32 = 36;
    const ROW_Y: i32 = 49;

    fn ctx() -> Ctx {
        ctx_sized(CLIENT)
    }

    fn ctx_sized(size: (i32, i32)) -> Ctx {
        crate::tests::test_ctx(size)
    }

    fn wired() -> nm::Device {
        nm::Device {
            path: "/dev/1".into(),
            iface: "enp3s0".into(),
            kind: nm::Kind::Wired,
            shell_name: "Local Area Connection".into(),
            product: "e1000e (enp3s0)".into(),
            state: nm::State::Connected,
            managed: true,
            carrier: true,
            ..Default::default()
        }
    }

    fn wireless() -> nm::Device {
        nm::Device {
            path: "/dev/2".into(),
            iface: "wlan0".into(),
            kind: nm::Kind::Wifi,
            shell_name: "Wireless Network Connection".into(),
            product: "iwlwifi (wlan0)".into(),
            state: nm::State::Disconnected,
            managed: true,
            carrier: true,
            ..Default::default()
        }
    }

    fn vpn() -> nm::Vpn {
        nm::Vpn {
            conn_path: "/settings/7".into(),
            name: "Work VPN".into(),
            kind: "openvpn".into(),
            active: false,
        }
    }

    fn snapshot() -> nm::Snapshot {
        nm::Snapshot {
            devices: vec![wired(), wireless()],
            vpns: vec![vpn()],
            primary: Some("/dev/1".into()),
            nm_running: true,
            ..Default::default()
        }
    }

    fn folder() -> Folder {
        let mut f = Folder::new();
        f.refresh(&snapshot(), None);
        f
    }

    fn row_point(f: &Folder, ctx: &Ctx, i: usize) -> (i32, i32) {
        let (_, list_r) = f.layout(ctx);
        let lay = listview::layout(&f.list, list_r, &ctx.theme.look.controls());
        let mut y = lay.body.y + listview::ROW_H / 2;
        for g in &f.list.groups {
            if g.first <= i {
                y += listview::GROUP_H;
            }
        }
        (lay.body.x + 20, y + i as i32 * listview::ROW_H)
    }

    #[test]
    fn the_lists_scrollbar_lights_under_the_pointer_and_a_leave_clears_it() {
        use reprisekit::pointer::Interaction;
        let mut ctx = crate::tests::test_ctx((420, 110));
        ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
        let mut f = folder();
        let (_, list_r) = f.layout(&ctx);
        let lay = listview::layout(&f.list, list_r, &ctx.theme.look.controls());
        let sb = lay.scrollbar.expect("a short window scrolls the list");
        let (x, y) = (sb.x + 8, sb.y + 2);
        let up = (Axis::Vertical, ScrollHit::Back);
        assert_eq!(f.motion(x, y, &ctx), Outcome::Redraw);
        assert_eq!(f.list.bars.of(up), Interaction::Hot);
        f.press(one(x, y), BTN_LEFT, &snapshot(), &ctx);
        assert_eq!(f.list.bars.of(up), Interaction::Pressed);
        assert_eq!(f.release(&ctx), Outcome::Redraw);
        assert_eq!(f.list.bars.of(up), Interaction::Hot);
        assert_eq!(f.leave(&ctx), Outcome::Redraw);
        assert_eq!(f.list.bars.lit(), None);
    }

    #[test]
    fn rows_for_groups_devices_then_vpns() {
        let (rows, cells, groups) = rows_for(&snapshot());
        assert_eq!(
            rows,
            vec![
                Row::Device("/dev/1".into()),
                Row::Device("/dev/2".into()),
                Row::Vpn("/settings/7".into()),
            ]
        );
        assert_eq!(cells.len(), 3);
        assert_eq!(
            groups,
            vec![
                listview::Group {
                    title: TYPE_LAN.into(),
                    first: 0
                },
                listview::Group {
                    title: TYPE_VPN.into(),
                    first: 2
                },
            ]
        );
        let s = nm::Snapshot {
            vpns: vec![],
            ..snapshot()
        };
        let (rows, _, groups) = rows_for(&s);
        assert_eq!(rows.len(), 2);
        assert_eq!(groups.len(), 1);
        let (rows, cells, groups) = rows_for(&nm::Snapshot::default());
        assert!(rows.is_empty() && cells.is_empty() && groups.is_empty());
        let gone = nm::Snapshot {
            nm_running: false,
            ..snapshot()
        };
        let (rows, cells, groups) = rows_for(&gone);
        assert!(rows.is_empty() && cells.is_empty() && groups.is_empty());
    }

    #[test]
    fn rows_show_the_status_strings() {
        let (_, cells, _) = rows_for(&snapshot());
        assert_eq!(
            cells[0].cells,
            vec![
                "Local Area Connection",
                TYPE_LAN,
                "Connected",
                "e1000e (enp3s0)"
            ]
        );
        assert_eq!(
            cells[1].cells,
            vec![
                "Wireless Network Connection",
                TYPE_LAN,
                "Not connected",
                "iwlwifi (wlan0)"
            ]
        );
        assert_eq!(
            cells[2].cells,
            vec!["Work VPN", TYPE_VPN, "Disconnected", "OpenVPN"]
        );
        let mut s = snapshot();
        s.vpns[0].active = true;
        s.devices[0].managed = false;
        let (_, cells, _) = rows_for(&s);
        assert_eq!(cells[0].cells[2], "Disabled");
        assert_eq!(cells[2].cells[2], "Connected");
    }

    #[test]
    fn context_menu_for_wired_wifi_vpn() {
        let s = snapshot();
        let labels = |items: &[MenuItem]| -> Vec<String> {
            items
                .iter()
                .map(|i| match i {
                    MenuItem::Entry { label, .. } => label.clone(),
                    MenuItem::Separator => "-".into(),
                })
                .collect()
        };
        let (items, actions) = context_menu(&Row::Device("/dev/1".into()), &s);
        assert_eq!(labels(&items), ["Disable", "Status", "Repair"]);
        assert_eq!(
            actions,
            vec![
                Some(Outcome::Toggle("/dev/1".into())),
                Some(Outcome::OpenStatus("/dev/1".into())),
                Some(Outcome::Repair("/dev/1".into())),
            ]
        );
        let (items, actions) = context_menu(&Row::Device("/dev/2".into()), &s);
        assert_eq!(
            labels(&items),
            [
                "Disable",
                "Status",
                "Repair",
                "-",
                "View Available Wireless Networks",
                "Create Hotspot..."
            ]
        );
        assert_eq!(actions[3], None);
        assert_eq!(actions[4], Some(Outcome::ViewWireless("/dev/2".into())));
        assert_eq!(actions[5], Some(Outcome::CreateHotspot("/dev/2".into())));
        let mut on = s.clone();
        on.devices[1].hotspot = Some("Desk".into());
        let (items, actions) = context_menu(&Row::Device("/dev/2".into()), &on);
        assert_eq!(labels(&items)[5], "Stop Hotspot");
        assert_eq!(actions[5], Some(Outcome::StopHotspot("/dev/2".into())));
        let mut off = s.clone();
        off.devices[0].managed = false;
        let (items, actions) = context_menu(&Row::Device("/dev/1".into()), &off);
        assert_eq!(labels(&items), ["Enable", "Status", "Repair"]);
        assert!(matches!(items[1], MenuItem::Entry { enabled: false, .. }));
        assert!(matches!(items[2], MenuItem::Entry { enabled: false, .. }));
        assert_eq!(actions[1..], [None, None]);
        let (items, actions) = context_menu(&Row::Vpn("/settings/7".into()), &s);
        assert_eq!(labels(&items), ["Connect", "-", "Delete", "Properties"]);
        assert_eq!(
            actions,
            vec![
                Some(Outcome::VpnUp("/settings/7".into())),
                None,
                Some(Outcome::VpnDelete("/settings/7".into())),
                Some(Outcome::VpnProperties("/settings/7".into())),
            ]
        );
        let mut up = s.clone();
        up.vpns[0].active = true;
        let (items, actions) = context_menu(&Row::Vpn("/settings/7".into()), &up);
        assert_eq!(labels(&items)[0], "Disconnect");
        assert_eq!(actions[0], Some(Outcome::VpnDown("/settings/7".into())));
        let (items, _) = context_menu(&Row::Device("/dev/gone".into()), &s);
        assert!(items.is_empty());
    }

    #[test]
    fn the_pane_and_the_list_geometry() {
        let ctx = ctx();
        let f = folder();
        let (pane_r, list_r) = f.layout(&ctx);
        assert_eq!(pane_r, Rect::new(0, 0, PANE_W, CLIENT.1));
        assert_eq!(
            list_r,
            Rect::new(LIST_X, LIST_Y, CLIENT.0 - LIST_X, CLIENT.1 - LIST_Y)
        );
        let pl = taskpane::layout(
            &f.pane,
            &ctx.text,
            ctx.theme.fonts.size_px,
            pane_r,
            &ctx.theme.look.controls(),
        );
        assert_eq!(pl.headers[0].x, SECTION_X);
        assert_eq!(pl.headers[0].right(), SECTION_RIGHT);
        assert_eq!(pl.headers[0].y, PANE_HEADER_Y);
        assert_eq!(f.pane.sections.len(), 4);
        // The borderless list puts its header's face, body, group rule and the group's first row on their rows. The header's rect leads with a highlight row, so it starts a px above the face.
        let ll = listview::layout(&f.list, list_r, &ctx.theme.look.controls());
        assert_eq!(ll.header.y + 1, LIST_HEADER_Y);
        assert_eq!(ll.body.x, BODY_X);
        assert_eq!(ll.body.y, LIST_HEADER_Y + 16, "body y 18");
        let rule = ll.body.y + listview::GROUP_RULE_Y;
        assert_eq!(rule, GROUP_RULE_Y);
        let first_row = ll.body.y + listview::GROUP_H;
        assert_eq!(first_row, ROW_Y);
        let titles: Vec<&str> = f.list.columns.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Name", "Type", "Status", "Device Name"]);
        let mut edges = Vec::new();
        let mut x = ll.body.x;
        for col in &f.list.columns {
            x += col.width;
            edges.push(x);
        }
        assert_eq!(edges[..3], DIVIDERS);
        assert_eq!(edges[3], BODY_RIGHT);
        assert_eq!(ll.total_w, ll.body.width, "the columns fill the body");
        assert!(ll.hscroll.is_none(), "nothing to scroll sideways");
        assert!(ll.scrollbar.is_none(), "three rows fit");
        let wide = ctx_sized((CLIENT.0 + 100, CLIENT.1));
        let mut f = f;
        f.fit_columns(&wide);
        assert_eq!(f.list.columns[3].width, DEVICE_W + 100);
        let (_, list_r) = f.layout(&wide);
        let ll = listview::layout(&f.list, list_r, &ctx.theme.look.controls());
        assert_eq!(ll.total_w, ll.body.width);
        assert!(ll.hscroll.is_none());
    }

    #[test]
    fn refresh_keeps_the_selection_and_writes_the_details() {
        let mut f = folder();
        let lines = |f: &Folder| match &f.pane.sections[DETAILS].body {
            Body::Lines(l) => l.clone(),
            other => panic!("Details is {other:?}"),
        };
        assert_eq!(
            lines(&f),
            vec![
                ("Network Connections".to_string(), true),
                ("System Folder".to_string(), false)
            ]
        );
        f.list.selection.set_only(Some(1));
        f.set_details(&snapshot());
        assert_eq!(
            lines(&f),
            vec![
                ("Wireless Network Connection".to_string(), true),
                (TYPE_LAN.to_string(), false),
                ("Not connected".to_string(), false)
            ]
        );
        let mut s = snapshot();
        s.devices.swap(0, 1);
        f.refresh(&s, None);
        assert_eq!(f.list.selection.single(), Some(0));
        assert_eq!(f.row(0), Some(&Row::Device("/dev/2".into())));
        let badge = neticon::draw(NetIcon::Wireless(2), true);
        f.refresh(&s, Some(badge.clone()));
        assert_eq!(f.list.rows[2].icon.as_deref(), Some(&badge));
        assert_eq!(
            f.list.rows[0].icon.as_deref(),
            Some(&neticon::draw(status::device_icon(&wireless()), false))
        );
        f.refresh(&nm::Snapshot::default(), None);
        assert_eq!(f.list.selection.single(), None);
        assert!(f.list.rows.is_empty());
        assert_eq!(
            lines(&f),
            vec![("NetworkManager is not running".to_string(), true)]
        );
    }

    #[test]
    fn press_right_selects_and_opens_popup() {
        let ctx = ctx();
        let s = snapshot();
        let mut f = folder();
        let (x, y) = row_point(&f, &ctx, 1);
        assert_eq!(f.press(one(x, y), BTN_RIGHT, &s, &ctx), Outcome::Redraw);
        assert_eq!(f.list.selection.single(), Some(1));
        assert!(f.popup.is_some(), "the menu is open");
        let key = |sym| KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: None,
        };
        assert_eq!(f.key(&key(keysym::DOWN), &s, &ctx), Outcome::Redraw);
        assert_eq!(
            f.key(&key(keysym::RETURN), &s, &ctx),
            Outcome::Toggle("/dev/2".into())
        );
        assert!(f.popup.is_none(), "picking a row closes the menu");
        f.press(one(x, y), BTN_RIGHT, &s, &ctx);
        assert_eq!(f.key(&key(keysym::ESCAPE), &s, &ctx), Outcome::Redraw);
        assert!(f.popup.is_none());
        f.press(one(x, y), BTN_RIGHT, &s, &ctx);
        assert_eq!(f.press(one(4, 4), BTN_LEFT, &s, &ctx), Outcome::Redraw);
        assert!(f.popup.is_none());
        assert_eq!(f.list.selection.single(), Some(1), "the click did not land");
    }

    #[test]
    fn clicks_and_keys_open_what_the_row_is() {
        let ctx = ctx();
        let mut s = snapshot();
        let mut f = folder();
        let (x, y) = row_point(&f, &ctx, 0);
        assert_eq!(f.press(one(x, y), BTN_LEFT, &s, &ctx), Outcome::Redraw);
        assert_eq!(f.list.selection.single(), Some(0));
        assert_eq!(
            f.press(two(x, y), BTN_LEFT, &s, &ctx),
            Outcome::OpenStatus("/dev/1".into())
        );
        let (x, y) = row_point(&f, &ctx, 2);
        f.press(one(x, y), BTN_LEFT, &s, &ctx);
        assert_eq!(
            f.press(two(x, y), BTN_LEFT, &s, &ctx),
            Outcome::VpnUp("/settings/7".into())
        );
        s.vpns[0].active = true;
        f.refresh(&s, None);
        f.press(one(x, y), BTN_LEFT, &s, &ctx);
        assert_eq!(
            f.press(two(x, y), BTN_LEFT, &s, &ctx),
            Outcome::VpnDown("/settings/7".into())
        );
        let key = |sym| KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: None,
        };
        f.list.selection.set_only(None);
        f.list.selection.focus = None;
        assert_eq!(f.key(&key(keysym::DOWN), &s, &ctx), Outcome::Redraw);
        assert_eq!(f.list.selection.single(), Some(0));
        assert_eq!(
            f.key(&key(keysym::RETURN), &s, &ctx),
            Outcome::OpenStatus("/dev/1".into())
        );
        assert_eq!(f.key(&key(keysym::DELETE), &s, &ctx), Outcome::None);
    }

    #[test]
    fn the_network_tasks_are_live_the_rest_inert_and_headers_fold() {
        let ctx = ctx();
        let s = snapshot();
        let mut f = folder();
        let (pane_r, _) = f.layout(&ctx);
        let lay = taskpane::layout(
            &f.pane,
            &ctx.text,
            ctx.theme.fonts.size_px,
            pane_r,
            &ctx.theme.look.controls(),
        );
        let mut click = |r: Rect| f.press(one(r.x + 2, r.y + 2), BTN_LEFT, &s, &ctx);
        assert_eq!(
            click(lay.rows[TASKS][TASK_WIREGUARD]),
            Outcome::NewWireGuard
        );
        assert_eq!(click(lay.rows[TASKS][TASK_IMPORT]), Outcome::Import);
        assert_eq!(
            click(lay.rows[TASKS][TASK_HOTSPOT]),
            Outcome::CreateHotspot("/dev/2".into()),
            "the radio"
        );
        assert_eq!(click(lay.rows[1][0]), Outcome::None, "See Also is greyed");
        let mut on = s.clone();
        on.devices[1].hotspot = Some("Desk".into());
        on.devices[1].state = nm::State::Connected;
        f.refresh(&on, None);
        assert!(matches!(
            &f.pane.sections[TASKS].body,
            Body::Links(rows) if !rows[TASK_HOTSPOT].enabled
        ));
        assert_eq!(
            f.list.rows[1].cells[2], "Hotspot: Desk",
            "the radio's Status cell"
        );
        let head = lay.headers[0];
        assert_eq!(
            f.press(one(head.x + 2, head.y + 2), BTN_LEFT, &s, &ctx),
            Outcome::Redraw
        );
        assert!(!f.pane.sections[0].open, "the header folded the section");
    }

    #[test]
    fn the_wheel_and_the_scrollbar_move_the_list() {
        let ctx = ctx_sized((CLIENT.0, 120));
        let mut f = Folder::new();
        let mut s = snapshot();
        for i in 0..8 {
            s.devices.push(nm::Device {
                path: format!("/dev/x{i}"),
                ..wired()
            });
        }
        f.refresh(&s, None);
        f.fit_columns(&ctx);
        let (_, list_r) = f.layout(&ctx);
        let lay = listview::layout(&f.list, list_r, &ctx.theme.look.controls());
        let sb = lay.scrollbar.expect("a vertical bar");
        assert_eq!(f.wheel(3, &ctx), Outcome::Redraw);
        assert_eq!(f.list.top, 3);
        assert_eq!(f.wheel(-3, &ctx), Outcome::Redraw);
        assert_eq!(f.list.top, 0);
        assert_eq!(f.wheel(-1, &ctx), Outcome::None, "already at the top");
        assert_eq!(
            f.press(one(sb.x + 2, sb.bottom() - 3), BTN_LEFT, &s, &ctx),
            Outcome::Redraw
        );
        assert_eq!(f.list.top, 1);
        let below = f.list.top;
        assert_eq!(
            f.press(one(sb.x + 2, sb.bottom() - 30), BTN_LEFT, &s, &ctx),
            Outcome::Redraw
        );
        assert!(f.list.top > below + 1, "the trough paged down");
        let paged = f.list.top;
        assert_eq!(
            f.press(
                one(sb.x + 2, sb.y + ControlMetrics::CLASSIC.scroll + 1),
                BTN_LEFT,
                &s,
                &ctx
            ),
            Outcome::Redraw
        );
        assert!(f.list.top < paged, "and back up above the thumb");
        assert!(lay.hscroll.is_none());
    }

    /// Writes target/netconn-folder.png, for eyeballing the geometry
    /// above: the same three connections, nothing selected.
    #[test]
    #[ignore]
    fn render_folder() {
        let ctx = ctx();
        let f = folder();
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        f.draw(&mut c, &ctx);
        reprisekit::raster::write_preview(&c, "netconn-folder.png");
    }
}
