//! The VPN Properties sheets: a WireGuard connection's two pages (interface,
//! and peers in a list with the selected one's fields under it), an OpenVPN
//! connection's two (gateway and sign-in, then certificates and keys), and any
//! other plugin VPN's one page (name and `key = value` settings). Each hands
//! back what to save.

use std::cell::Cell;

use reprisekit::classic::widgets::{BUTTON_H, LINE_H};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::listbox::ListBox;
use reprisekit::controls::propsheet::{self, Buttons, PropSheet};
use reprisekit::raster::Rect;
use reprisekit::textedit::TextEdit;

use crate::nm::{self, Psk};
use crate::ovpn;
use crate::wg::{self, Field};

pub const CLIENT: (i32, i32) = (420, 384);
pub const TABS: [&str; 2] = ["WireGuard", "Peers"];
const TAB_IFACE: usize = 0;
const TAB_PEERS: usize = 1;

const LABEL_X: i32 = 18;
const LABEL_W: i32 = 104;
const FIELD_X: i32 = 124;
const FIELD_RIGHT: i32 = CLIENT.0 - 18;
const FIELD_W: i32 = FIELD_RIGHT - FIELD_X;
const FIELD_H: i32 = 20;
const ROW_Y: i32 = 46;
const PITCH: i32 = 30;
const LABEL_DY: i32 = 4;
/// The public key's row: a line taller than the others, its value set on two.
const PUB_ROW: i32 = 3;
const GEN_W: i32 = 75;

const LIST: Rect = Rect::new(LABEL_X, ROW_Y, 290, 82);
const SIDE_X: i32 = LABEL_X + 290 + 8;
const SIDE_W: i32 = FIELD_RIGHT - SIDE_X;
const GROUP: Rect = Rect::new(12, ROW_Y + 94, CLIENT.0 - 24, 176);
const PEER_LABEL_X: i32 = 26;
const PEER_FIELD_X: i32 = FIELD_X;
const PEER_FIELD_W: i32 = CLIENT.0 - 26 - PEER_FIELD_X;
const PEER_ROW_Y: i32 = GROUP.y + 22;
pub const KEEP_CUE: &str = "Leave empty to keep the saved key";

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    None,
    Redraw,
    Save(Box<wg::Config>),
    SavePlugin {
        name: String,
        data: Vec<(String, String)>,
    },
    SaveOpenVpn(Box<ovpn::Config>),
    /// A Browse... button: pick a file for edit field `id`.
    Browse(&'static str),
    /// OK found something wrong; the sheet stays open on that field and
    /// the host says why.
    Problem(String),
    Close,
}

fn edit(id: &'static str, x: i32, y: i32, w: i32, text: &str, secret: bool) -> Item {
    let mut e = if secret {
        TextEdit::secret()
    } else {
        TextEdit::new(text, false)
    };
    if secret {
        e.paste(text, &[]);
    }
    Item::new(
        id,
        Rect::new(x, y, w, FIELD_H),
        Control::Edit {
            edit: e,
            first: Cell::new(0),
            enabled: true,
        },
    )
}

pub fn text_of<'a>(f: &'a Form, id: &str) -> &'a str {
    match f.item(id).map(|i| &i.control) {
        Some(Control::Edit { edit, .. }) => &edit.text,
        _ => "",
    }
}

/// Put `s` in edit field `id`, the caret at its end; a secret field is
/// wiped first.
fn set_field(f: &mut Form, id: &str, s: &str) {
    if let Some(Control::Edit { edit, first, .. }) = f.item_mut(id).map(|i| &mut i.control) {
        if edit.is_secret() {
            edit.wipe();
            edit.paste(s, &[]);
        } else {
            *edit = TextEdit::new(s, false);
        }
        first.set(0);
    }
}

fn set_cue(f: &mut Form, id: &str, cue: &str) {
    if let Some(Control::Edit { edit, .. }) = f.item_mut(id).map(|i| &mut i.control) {
        edit.set_cue(cue);
    }
}

fn row_y(i: i32) -> i32 {
    ROW_Y + i * PITCH + if i > PUB_ROW { LINE_H } else { 0 }
}

/// A public key as its two lines: a 44-character key does not fit one line of the field in
/// every face.
fn key_lines(key: &str) -> Vec<String> {
    let (a, b) = key.split_at(key.len().div_ceil(2));
    vec![a.to_string(), b.to_string()]
}

fn label(id: &'static str, i: i32, text: &str) -> Item {
    Item::label_at(id, LABEL_X, row_y(i) + LABEL_DY, LABEL_W, text)
}

fn iface_page(cfg: &wg::Config) -> Form {
    let port = cfg.listen_port.map(|p| p.to_string()).unwrap_or_default();
    let note = [
        "Separate several addresses or servers with commas.",
        "Search domains may go among the DNS servers.",
    ];
    let mut f = Form::new(vec![
        label("name_l", 0, "Connection &name:"),
        edit("name", FIELD_X, row_y(0), FIELD_W, &cfg.name, false),
        label("iface_l", 1, "&Interface:"),
        edit("iface", FIELD_X, row_y(1), FIELD_W, &cfg.iface, false),
        label("key_l", 2, "Private &key:"),
        edit(
            "key",
            FIELD_X,
            row_y(2),
            FIELD_W - GEN_W - 6,
            &cfg.private_key,
            true,
        ),
        Item::button_at(
            "generate",
            FIELD_RIGHT - GEN_W,
            row_y(2) - (BUTTON_H - FIELD_H) / 2,
            GEN_W,
            "&Generate",
            true,
        ),
        label("pub_l", PUB_ROW, "Public key:"),
        Item::new(
            "pub",
            Rect::new(FIELD_X, row_y(PUB_ROW) + LABEL_DY, FIELD_W, 2 * LINE_H),
            Control::Lines(key_lines(
                &wg::public_key(&cfg.private_key).unwrap_or_default(),
            )),
        ),
        label("addr_l", 4, "&Addresses:"),
        edit(
            "addr",
            FIELD_X,
            row_y(4),
            FIELD_W,
            &wg::join(&cfg.addresses),
            false,
        ),
        label("dns_l", 5, "&DNS servers:"),
        edit(
            "dns",
            FIELD_X,
            row_y(5),
            FIELD_W,
            &wg::join(&cfg.dns),
            false,
        ),
        label("port_l", 6, "&Listen port:"),
        edit("port", FIELD_X, row_y(6), 70, &port, false),
        Item::label_at(
            "port_hint",
            FIELD_X + 80,
            row_y(6) + LABEL_DY,
            FIELD_W - 80,
            "Blank picks a free port.",
        ),
        Item::lines_at("note", LABEL_X, row_y(7) + 8, FIELD_RIGHT - LABEL_X, &note),
    ]);
    if cfg.stored_key {
        set_cue(&mut f, "key", KEEP_CUE);
    }
    f
}

pub fn peer_line(i: usize, p: &wg::Peer) -> String {
    let what = if !p.endpoint.is_empty() {
        p.endpoint.clone()
    } else if !p.public_key.is_empty() {
        p.public_key.chars().take(16).collect::<String>() + "..."
    } else {
        "(new peer)".into()
    };
    format!("Peer {}: {what}", i + 1)
}

fn peer_label(id: &'static str, i: i32, text: &str) -> Item {
    Item::label_at(
        id,
        PEER_LABEL_X,
        PEER_ROW_Y + i * PITCH + LABEL_DY,
        96,
        text,
    )
}

fn peers_page(peers: &[wg::Peer]) -> Form {
    let lines: Vec<String> = peers
        .iter()
        .enumerate()
        .map(|(i, p)| peer_line(i, p))
        .collect();
    let selected = (!peers.is_empty()).then_some(0);
    Form::new(vec![
        Item::new(
            "peers",
            LIST,
            Control::ListBox {
                list: ListBox::new(lines, selected),
                icons: vec![],
            },
        ),
        Item::button_at("add", SIDE_X, LIST.y, SIDE_W, "A&dd", true),
        Item::button_at(
            "remove",
            SIDE_X,
            LIST.y + BUTTON_H + 6,
            SIDE_W,
            "&Remove",
            selected.is_some(),
        ),
        Item::group("peer_g", GROUP, "Peer"),
        peer_label("pk_l", 0, "P&ublic key:"),
        edit("pk", PEER_FIELD_X, PEER_ROW_Y, PEER_FIELD_W, "", false),
        peer_label("psk_l", 1, "Pre&shared key:"),
        edit(
            "psk",
            PEER_FIELD_X,
            PEER_ROW_Y + PITCH,
            PEER_FIELD_W,
            "",
            true,
        ),
        peer_label("endpoint_l", 2, "&Endpoint:"),
        edit(
            "endpoint",
            PEER_FIELD_X,
            PEER_ROW_Y + 2 * PITCH,
            PEER_FIELD_W,
            "",
            false,
        ),
        peer_label("allowed_l", 3, "Allowed &IPs:"),
        edit(
            "allowed",
            PEER_FIELD_X,
            PEER_ROW_Y + 3 * PITCH,
            PEER_FIELD_W,
            "",
            false,
        ),
        peer_label("keepalive_l", 4, "Keep&alive:"),
        edit(
            "keepalive",
            PEER_FIELD_X,
            PEER_ROW_Y + 4 * PITCH,
            60,
            "",
            false,
        ),
        Item::label_at(
            "keepalive_hint",
            PEER_FIELD_X + 70,
            PEER_ROW_Y + 4 * PITCH + LABEL_DY,
            PEER_FIELD_W - 70,
            "seconds; blank for none",
        ),
    ])
}

const PEER_FIELDS: [&str; 5] = ["pk", "psk", "endpoint", "allowed", "keepalive"];

/// A WireGuard connection's sheet: the pages, the peers as they stand
/// (the fields write through on every change), and which one the fields
/// show.
pub struct WgSheet {
    pub sheet: PropSheet,
    pub conn_path: Option<String>,
    peers: Vec<wg::Peer>,
    shown: Option<usize>,
    /// The sheet shows no MTU; a loaded or imported one rides along.
    mtu: Option<u32>,
    stored_key: bool,
}

impl WgSheet {
    pub fn new(cfg: &wg::Config, conn_path: Option<String>) -> WgSheet {
        let pages = vec![iface_page(cfg), peers_page(&cfg.peers)];
        let mut s = WgSheet {
            sheet: PropSheet::with_buttons(&TABS, pages, CLIENT, Buttons::OkCancel),
            conn_path,
            peers: cfg.peers.clone(),
            shown: None,
            mtu: cfg.mtu,
            stored_key: cfg.stored_key,
        };
        s.show_peer((!cfg.peers.is_empty()).then_some(0));
        s.sheet.pages[TAB_IFACE].focus_id("name");
        s
    }

    pub fn fresh(existing: &[nm::Vpn]) -> WgSheet {
        let n = (1..)
            .find(|n| {
                let name = format!("WireGuard {n}");
                !existing.iter().any(|v| v.name == name)
            })
            .unwrap_or(1);
        let cfg = wg::Config {
            name: format!("WireGuard {n}"),
            iface: format!("wg{}", n - 1),
            private_key: Psk::new(&wg::generate_key().unwrap_or_default()),
            peers: vec![wg::Peer::default()],
            ..wg::Config::default()
        };
        WgSheet::new(&cfg, None)
    }

    pub fn window_title(&self) -> String {
        if self.conn_path.is_none() {
            "New WireGuard Connection".into()
        } else {
            format!(
                "{} Properties",
                text_of(&self.sheet.pages[TAB_IFACE], "name")
            )
        }
    }

    fn peers_form(&mut self) -> &mut Form {
        &mut self.sheet.pages[TAB_PEERS]
    }

    fn show_peer(&mut self, i: Option<usize>) {
        self.shown = i.filter(|&i| i < self.peers.len());
        let peer = self
            .shown
            .map(|i| self.peers[i].clone())
            .unwrap_or_default();
        let live = self.shown.is_some();
        let f = self.peers_form();
        set_field(f, "pk", &peer.public_key);
        set_field(f, "psk", &peer.preshared_key);
        set_cue(f, "psk", if peer.stored_psk { KEEP_CUE } else { "" });
        set_field(f, "endpoint", &peer.endpoint);
        set_field(f, "allowed", &wg::join(&peer.allowed_ips));
        set_field(
            f,
            "keepalive",
            &peer.keepalive.map(|k| k.to_string()).unwrap_or_default(),
        );
        for id in PEER_FIELDS {
            f.set_enabled(id, live);
        }
        f.set_enabled("remove", live);
    }

    fn sync_list(&mut self) {
        let lines: Vec<String> = self
            .peers
            .iter()
            .enumerate()
            .map(|(i, p)| peer_line(i, p))
            .collect();
        let shown = self.shown;
        if let Some(Control::ListBox { list, .. }) =
            self.peers_form().item_mut("peers").map(|i| &mut i.control)
        {
            list.items = lines;
            list.selected = shown;
            list.top = list.top.min(list.items.len().saturating_sub(1));
        }
    }

    fn take_peer_fields(&mut self) {
        let Some(i) = self.shown else {
            return;
        };
        let f = &self.sheet.pages[TAB_PEERS];
        let keepalive = text_of(f, "keepalive").trim();
        let public_key = text_of(f, "pk").trim().to_string();
        // A stored preshared key belongs to the public key it was saved with.
        let stored_psk = self.peers[i].stored_psk && self.peers[i].public_key == public_key;
        let peer = wg::Peer {
            public_key,
            stored_psk,
            preshared_key: Psk::new(text_of(f, "psk").trim()),
            endpoint: text_of(f, "endpoint").trim().to_string(),
            allowed_ips: wg::split(text_of(f, "allowed")),
            // Anything but a number reads as none; the field is narrow
            // enough that a typo shows.
            keepalive: keepalive.parse::<u16>().ok().filter(|&k| k != 0),
        };
        self.peers[i] = peer;
        if !stored_psk {
            set_cue(self.peers_form(), "psk", "");
        }
    }

    pub fn config(&self) -> wg::Config {
        let f = &self.sheet.pages[TAB_IFACE];
        let port = text_of(f, "port").trim();
        wg::Config {
            name: text_of(f, "name").trim().to_string(),
            iface: text_of(f, "iface").trim().to_string(),
            private_key: Psk::new(text_of(f, "key").trim()),
            addresses: wg::split(text_of(f, "addr")),
            dns: wg::split(text_of(f, "dns")),
            listen_port: port.parse::<u16>().ok().filter(|&p| p != 0),
            mtu: self.mtu,
            peers: self.peers.clone(),
            stored_key: self.stored_key,
        }
    }

    fn focus_field(&mut self, field: Field) {
        let (tab, id, peer) = match field {
            Field::Name => (TAB_IFACE, "name", None),
            Field::Iface => (TAB_IFACE, "iface", None),
            Field::PrivateKey => (TAB_IFACE, "key", None),
            Field::Addresses => (TAB_IFACE, "addr", None),
            Field::Dns => (TAB_IFACE, "dns", None),
            Field::PublicKey(i) => (TAB_PEERS, "pk", Some(i)),
            Field::PresharedKey(i) => (TAB_PEERS, "psk", Some(i)),
            Field::Endpoint(i) => (TAB_PEERS, "endpoint", Some(i)),
            Field::AllowedIps(i) => (TAB_PEERS, "allowed", Some(i)),
        };
        if let Some(i) = peer {
            self.show_peer(Some(i));
            self.sync_list();
        }
        self.sheet.open_tab(tab);
        self.sheet.pages[tab].focus_id(id);
    }

    pub fn event(&mut self, ev: propsheet::Event) -> Outcome {
        match ev {
            propsheet::Event::Cancel => Outcome::Close,
            propsheet::Event::Ok => {
                self.take_peer_fields();
                let cfg = self.config();
                match wg::check(&cfg) {
                    Some(p) => {
                        self.focus_field(p.field);
                        Outcome::Problem(p.text)
                    }
                    None => Outcome::Save(Box::new(cfg)),
                }
            }
            propsheet::Event::Page(TAB_IFACE, e) => self.iface_event(e),
            propsheet::Event::Page(TAB_PEERS, e) => self.peers_event(e),
            propsheet::Event::Redraw | propsheet::Event::Page(..) => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    fn iface_event(&mut self, e: form::Event) -> Outcome {
        let f = &mut self.sheet.pages[TAB_IFACE];
        match e {
            form::Event::Pressed("generate") => {
                if let Some(k) = wg::generate_key() {
                    set_field(f, "key", &k);
                }
                let public = wg::public_key(text_of(f, "key")).unwrap_or_default();
                f.set_lines("pub", &key_lines(&public));
                Outcome::Redraw
            }
            form::Event::Changed("key") => {
                let public = wg::public_key(text_of(f, "key")).unwrap_or_default();
                f.set_lines("pub", &key_lines(&public));
                Outcome::Redraw
            }
            form::Event::Default => self.event(propsheet::Event::Ok),
            form::Event::Cancel => Outcome::Close,
            form::Event::None => Outcome::None,
            _ => Outcome::Redraw,
        }
    }

    fn peers_event(&mut self, e: form::Event) -> Outcome {
        match e {
            form::Event::Changed("peers") => {
                self.take_peer_fields();
                let picked = match self.peers_form().item("peers").map(|i| &i.control) {
                    Some(Control::ListBox { list, .. }) => list.selected,
                    _ => None,
                };
                self.show_peer(picked);
                self.sync_list();
                Outcome::Redraw
            }
            form::Event::Changed(id) if PEER_FIELDS.contains(&id) => {
                self.take_peer_fields();
                self.sync_list();
                Outcome::Redraw
            }
            form::Event::Pressed("add") => {
                self.take_peer_fields();
                self.peers.push(wg::Peer::default());
                self.show_peer(Some(self.peers.len() - 1));
                self.sync_list();
                self.peers_form().focus_id("pk");
                Outcome::Redraw
            }
            form::Event::Pressed("remove") => {
                if let Some(i) = self.shown {
                    self.peers.remove(i);
                    let next = (!self.peers.is_empty()).then(|| i.min(self.peers.len() - 1));
                    self.show_peer(next);
                    self.sync_list();
                    self.peers_form()
                        .focus_id(if next.is_some() { "remove" } else { "add" });
                }
                Outcome::Redraw
            }
            form::Event::Default => self.event(propsheet::Event::Ok),
            form::Event::Cancel => Outcome::Close,
            form::Event::None => Outcome::None,
            _ => Outcome::Redraw,
        }
    }
}

pub const PLUGIN_CLIENT: (i32, i32) = (380, 360);
const MEMO: Rect = Rect::new(LABEL_X, ROW_Y + 2 * PITCH + 18, PLUGIN_CLIENT.0 - 36, 176);

/// A plugin VPN's settings as the memo shows them: `key = value`, one a
/// line.
pub fn data_text(data: &[(String, String)]) -> String {
    data.iter()
        .map(|(k, v)| format!("{k} = {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The memo's lines back as pairs; a line with no `=` is a key with an
/// empty value, a blank line nothing.
pub fn parse_data(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| match l.split_once('=') {
            Some((k, v)) => (k.trim().to_string(), v.trim().to_string()),
            None => (l.to_string(), String::new()),
        })
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

/// A plugin VPN's sheet: its name, kind and settings.
pub struct PluginSheet {
    pub sheet: PropSheet,
    pub conn_path: String,
}

impl PluginSheet {
    pub fn new(conn_path: &str, name: &str, kind: &str, data: &[(String, String)]) -> PluginSheet {
        let w = PLUGIN_CLIENT.0 - 18 - FIELD_X;
        let page = Form::new(vec![
            label("name_l", 0, "Connection &name:"),
            edit("name", FIELD_X, row_y(0), w, name, false),
            label("kind_l", 1, "Type:"),
            Item::label_at(
                "kind",
                FIELD_X,
                row_y(1) + LABEL_DY,
                w,
                &nm::vpn_kind_label(kind),
            ),
            Item::label_at(
                "data_l",
                LABEL_X,
                ROW_Y + 2 * PITCH,
                PLUGIN_CLIENT.0 - 36,
                "&Settings, one key = value a line:",
            ),
            Item::memo("data", MEMO, &data_text(data), true),
            Item::lines_at(
                "note",
                LABEL_X,
                MEMO.bottom() + 6,
                PLUGIN_CLIENT.0 - 36,
                &["Passwords and keys are kept as they are."],
            ),
        ]);
        let mut sheet =
            PropSheet::with_buttons(&["General"], vec![page], PLUGIN_CLIENT, Buttons::OkCancel);
        sheet.pages[0].focus_id("name");
        PluginSheet {
            sheet,
            conn_path: conn_path.to_string(),
        }
    }

    pub fn window_title(&self) -> String {
        format!("{} Properties", text_of(&self.sheet.pages[0], "name"))
    }

    pub fn event(&mut self, ev: propsheet::Event) -> Outcome {
        let f = &self.sheet.pages[0];
        let save = || {
            let name = text_of(f, "name").trim().to_string();
            if name.is_empty() {
                return Outcome::Problem("Type a name for the connection.".into());
            }
            let data = match f.item("data").map(|i| &i.control) {
                Some(Control::Memo(m)) => parse_data(&m.edit.text()),
                _ => vec![],
            };
            Outcome::SavePlugin { name, data }
        };
        match ev {
            propsheet::Event::Cancel | propsheet::Event::Page(_, form::Event::Cancel) => {
                Outcome::Close
            }
            propsheet::Event::Ok | propsheet::Event::Page(_, form::Event::Default) => save(),
            propsheet::Event::None | propsheet::Event::Page(_, form::Event::None) => Outcome::None,
            _ => Outcome::Redraw,
        }
    }
}

pub const OVPN_TABS: [&str; 2] = ["General", "Authentication"];
const TAB_GENERAL: usize = 0;
const TAB_AUTH: usize = 1;
const BROWSE_W: i32 = 75;
pub const KEEP_PASSWORD_CUE: &str = "Leave empty to keep the saved password";
const STATIC_GROUP: Rect = Rect::new(12, ROW_Y + 4 * PITCH + 2, CLIENT.0 - 24, 4 * PITCH + 34);
const STATIC_ROW_Y: i32 = STATIC_GROUP.y + 22;

fn orow(i: i32) -> i32 {
    ROW_Y + i * PITCH
}

fn olabel(id: &'static str, i: i32, text: &str) -> Item {
    Item::label_at(id, LABEL_X, orow(i) + LABEL_DY, LABEL_W, text)
}

/// A file field: its label, the path and a Browse... button at the right.
fn file_row(id: &'static str, browse: &'static str, y: i32, text: &str) -> [Item; 2] {
    [
        edit(id, FIELD_X, y, FIELD_W - BROWSE_W - 6, text, false),
        Item::button_at(
            browse,
            FIELD_RIGHT - BROWSE_W,
            y - (BUTTON_H - FIELD_H) / 2,
            BROWSE_W,
            "Browse...",
            true,
        ),
    ]
}

fn static_label(id: &'static str, i: i32, text: &str) -> Item {
    Item::label_at(
        id,
        PEER_LABEL_X,
        STATIC_ROW_Y + i * PITCH + LABEL_DY,
        FIELD_X - PEER_LABEL_X - 2,
        text,
    )
}

fn general_page(cfg: &ovpn::Config) -> Form {
    let types: Vec<&str> = ovpn::Auth::ALL.iter().map(|a| a.label()).collect();
    let mut f = Form::new(vec![
        olabel("name_l", 0, "Connection &name:"),
        edit("name", FIELD_X, orow(0), FIELD_W, &cfg.name, false),
        olabel("gateway_l", 1, "&Gateway:"),
        edit("gateway", FIELD_X, orow(1), FIELD_W, &cfg.gateway, false),
        olabel("type_l", 2, "&Type:"),
        Item::combo("type", FIELD_X, orow(2), FIELD_W, &types, types.len()),
        olabel("user_l", 3, "&User name:"),
        edit("user", FIELD_X, orow(3), FIELD_W, &cfg.user, false),
        olabel("password_l", 4, "&Password:"),
        edit("password", FIELD_X, orow(4), FIELD_W, "", true),
        Item::lines_at(
            "note",
            LABEL_X,
            orow(5) + 8,
            FIELD_RIGHT - LABEL_X,
            &[
                "Separate several gateways with commas.",
                "Settings not shown here are kept as they are.",
            ],
        ),
    ]);
    f.set_combo("type", cfg.auth.index());
    set_cue(&mut f, "password", KEEP_PASSWORD_CUE);
    f
}

fn auth_page(cfg: &ovpn::Config) -> Form {
    let mut items = vec![olabel("ca_l", 0, "&CA certificate:")];
    items.extend(file_row("ca", "ca_browse", orow(0), &cfg.ca));
    items.push(olabel("cert_l", 1, "&User certificate:"));
    items.extend(file_row("cert", "cert_browse", orow(1), &cfg.cert));
    items.push(olabel("key_l", 2, "Private &key:"));
    items.extend(file_row("key", "key_browse", orow(2), &cfg.key));
    items.push(olabel("keypass_l", 3, "Key &password:"));
    items.push(edit("keypass", FIELD_X, orow(3), FIELD_W, "", true));
    items.push(Item::group("static_g", STATIC_GROUP, "Static key"));
    items.push(static_label("static_l", 0, "Key &file:"));
    items.extend(file_row(
        "static",
        "static_browse",
        STATIC_ROW_Y,
        &cfg.static_key,
    ));
    items.push(static_label("direction_l", 1, "&Direction:"));
    items.push(Item::combo(
        "direction",
        FIELD_X,
        STATIC_ROW_Y + PITCH,
        70,
        &ovpn::DIRECTIONS,
        ovpn::DIRECTIONS.len(),
    ));
    items.push(static_label("local_l", 2, "&Local IP:"));
    items.push(edit(
        "local",
        FIELD_X,
        STATIC_ROW_Y + 2 * PITCH,
        FIELD_W - 12,
        &cfg.local_ip,
        false,
    ));
    items.push(static_label("remote_l", 3, "&Remote IP:"));
    items.push(edit(
        "remote",
        FIELD_X,
        STATIC_ROW_Y + 3 * PITCH,
        FIELD_W - 12,
        &cfg.remote_ip,
        false,
    ));
    let mut f = Form::new(items);
    f.set_combo("direction", cfg.direction);
    set_cue(&mut f, "keypass", KEEP_PASSWORD_CUE);
    f
}

/// Which page and Browse... button each file field has.
const FILE_FIELDS: [(&str, &str); 4] = [
    ("ca_browse", "ca"),
    ("cert_browse", "cert"),
    ("key_browse", "key"),
    ("static_browse", "static"),
];

/// An OpenVPN connection's sheet. The type greys the fields it does not use; their values stay
/// and are saved as they are.
pub struct OvpnSheet {
    pub sheet: PropSheet,
    pub conn_path: String,
    rest: Vec<(String, String)>,
}

impl OvpnSheet {
    pub fn new(conn_path: &str, cfg: &ovpn::Config) -> OvpnSheet {
        let pages = vec![general_page(cfg), auth_page(cfg)];
        let mut s = OvpnSheet {
            sheet: PropSheet::with_buttons(&OVPN_TABS, pages, CLIENT, Buttons::OkCancel),
            conn_path: conn_path.to_string(),
            rest: cfg.rest.clone(),
        };
        s.sync_enabled();
        s.sheet.pages[TAB_GENERAL].focus_id("name");
        s
    }

    pub fn window_title(&self) -> String {
        format!(
            "{} Properties",
            text_of(&self.sheet.pages[TAB_GENERAL], "name")
        )
    }

    fn auth(&self) -> ovpn::Auth {
        let i = self.sheet.pages[TAB_GENERAL]
            .combo_selected("type")
            .unwrap_or(0);
        ovpn::Auth::ALL.get(i).copied().unwrap_or_default()
    }

    fn sync_enabled(&mut self) {
        let a = self.auth();
        let g = &mut self.sheet.pages[TAB_GENERAL];
        for id in ["user_l", "user", "password_l", "password"] {
            g.set_enabled(id, a.user());
        }
        let p = &mut self.sheet.pages[TAB_AUTH];
        for id in ["ca_l", "ca", "ca_browse"] {
            p.set_enabled(id, a.ca());
        }
        for id in [
            "cert_l",
            "cert",
            "cert_browse",
            "key_l",
            "key",
            "key_browse",
            "keypass_l",
            "keypass",
        ] {
            p.set_enabled(id, a.certs());
        }
        for id in [
            "static_l",
            "static",
            "static_browse",
            "direction_l",
            "direction",
            "local_l",
            "local",
            "remote_l",
            "remote",
        ] {
            p.set_enabled(id, a == ovpn::Auth::StaticKey);
        }
    }

    pub fn config(&self) -> ovpn::Config {
        let g = &self.sheet.pages[TAB_GENERAL];
        let p = &self.sheet.pages[TAB_AUTH];
        let t = |f: &Form, id| text_of(f, id).trim().to_string();
        ovpn::Config {
            name: t(g, "name"),
            gateway: t(g, "gateway"),
            auth: self.auth(),
            user: t(g, "user"),
            password: Psk::new(text_of(g, "password")),
            ca: t(p, "ca"),
            cert: t(p, "cert"),
            key: t(p, "key"),
            cert_pass: Psk::new(text_of(p, "keypass")),
            static_key: t(p, "static"),
            direction: p.combo_selected("direction").unwrap_or(0),
            local_ip: t(p, "local"),
            remote_ip: t(p, "remote"),
            rest: self.rest.clone(),
        }
    }

    /// The file field `id` names, if it names one.
    pub fn path_of(&self, id: &str) -> Option<std::path::PathBuf> {
        let t = text_of(&self.sheet.pages[TAB_AUTH], id).trim();
        (!t.is_empty()).then(|| t.into())
    }

    /// The file picked for field `id` by its Browse... button.
    pub fn set_path(&mut self, id: &str, path: &str) {
        let p = &mut self.sheet.pages[TAB_AUTH];
        set_field(p, id, path);
        p.focus_id(id);
    }

    pub fn event(&mut self, ev: propsheet::Event) -> Outcome {
        match ev {
            propsheet::Event::Cancel | propsheet::Event::Page(_, form::Event::Cancel) => {
                Outcome::Close
            }
            propsheet::Event::Ok | propsheet::Event::Page(_, form::Event::Default) => {
                let cfg = self.config();
                match ovpn::check(&cfg) {
                    Some((id, text)) => {
                        let tab = if ["name", "gateway", "user"].contains(&id) {
                            TAB_GENERAL
                        } else {
                            TAB_AUTH
                        };
                        self.sheet.open_tab(tab);
                        self.sheet.pages[tab].focus_id(id);
                        Outcome::Problem(text.into())
                    }
                    None => Outcome::SaveOpenVpn(Box::new(cfg)),
                }
            }
            propsheet::Event::Page(TAB_GENERAL, form::Event::Changed("type")) => {
                self.sync_enabled();
                Outcome::Redraw
            }
            propsheet::Event::Page(TAB_AUTH, form::Event::Pressed(b)) => FILE_FIELDS
                .iter()
                .find(|(button, _)| *button == b)
                .map_or(Outcome::Redraw, |(_, id)| Outcome::Browse(id)),
            propsheet::Event::None | propsheet::Event::Page(_, form::Event::None) => Outcome::None,
            _ => Outcome::Redraw,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::raster::Canvas;

    const KEY_A: &str = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=";
    const KEY_B: &str = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=";

    fn cfg() -> wg::Config {
        wg::Config {
            name: "Away".into(),
            iface: "wg0".into(),
            private_key: Psk::new(KEY_A),
            addresses: vec!["10.0.0.2/24".into()],
            dns: vec!["10.0.0.1".into()],
            listen_port: Some(51820),
            mtu: Some(1380),
            peers: vec![wg::Peer {
                public_key: KEY_B.into(),
                endpoint: "vpn.example.org:51820".into(),
                allowed_ips: vec!["0.0.0.0/0".into()],
                keepalive: Some(25),
                ..wg::Peer::default()
            }],
            stored_key: false,
        }
    }

    /// A profile whose keys NetworkManager would not give out.
    fn kept() -> wg::Config {
        let mut c = cfg();
        c.private_key = Psk::new("");
        c.stored_key = true;
        c.peers[0].stored_psk = true;
        c
    }

    fn cue_of<'a>(f: &'a Form, id: &str) -> &'a str {
        match f.item(id).map(|i| &i.control) {
            Some(Control::Edit { edit, .. }) => edit.cue(),
            _ => "",
        }
    }

    /// Both sheets fit their text under every interface and text size.
    #[test]
    fn the_sheets_fit_their_text() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            let wg = WgSheet::new(&cfg(), None).sheet.pages;
            let plugin = PluginSheet::new("/Settings/3", "Work", "openvpn", &[])
                .sheet
                .pages;
            let openvpn = OvpnSheet::new("/Settings/5", &ovpn_cfg()).sheet.pages;
            TABS.iter()
                .map(|t| t.to_string())
                .zip(wg)
                .chain(plugin.into_iter().map(|p| ("plugin".into(), p)))
                .chain(OVPN_TABS.iter().map(|t| t.to_string()).zip(openvpn))
                .collect()
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    #[test]
    fn empty_stored_key_fields_say_they_keep_the_saved_key() {
        let s = WgSheet::new(&kept(), Some("/Settings/4".into()));
        assert_eq!(cue_of(&s.sheet.pages[TAB_IFACE], "key"), KEEP_CUE);
        assert_eq!(cue_of(&s.sheet.pages[TAB_PEERS], "psk"), KEEP_CUE);
        let s = WgSheet::new(&cfg(), Some("/Settings/4".into()));
        assert_eq!(cue_of(&s.sheet.pages[TAB_IFACE], "key"), "");
        assert_eq!(cue_of(&s.sheet.pages[TAB_PEERS], "psk"), "");
    }

    #[test]
    fn ok_on_untouched_stored_keys_saves_them_as_kept() {
        let mut s = WgSheet::new(&kept(), Some("/Settings/4".into()));
        assert_eq!(
            s.event(propsheet::Event::Ok),
            Outcome::Save(Box::new(kept()))
        );
    }

    #[test]
    fn a_new_preshared_key_beside_a_kept_private_key_asks_for_both() {
        let mut s = WgSheet::new(&kept(), Some("/Settings/4".into()));
        set_field(&mut s.sheet.pages[TAB_PEERS], "psk", KEY_A);
        s.take_peer_fields();
        assert!(matches!(s.event(propsheet::Event::Ok), Outcome::Problem(_)));
        assert_eq!(
            s.sheet.tabs.open, TAB_IFACE,
            "the sheet turns to the private key"
        );
        let f = &s.sheet.pages[TAB_IFACE];
        assert_eq!(f.focus(), f.index_of("key"));
    }

    #[test]
    fn a_new_public_key_drops_the_stored_preshared_key() {
        let mut s = WgSheet::new(&kept(), Some("/Settings/4".into()));
        set_field(&mut s.sheet.pages[TAB_PEERS], "pk", KEY_A);
        s.event(propsheet::Event::Page(
            TAB_PEERS,
            form::Event::Changed("pk"),
        ));
        assert!(!s.config().peers[0].stored_psk);
        assert_eq!(cue_of(&s.sheet.pages[TAB_PEERS], "psk"), "");
    }

    fn type_into(f: &mut Form, id: &str, s: &str) -> form::Event {
        f.focus_id(id);
        let mut last = form::Event::None;
        for ch in s.chars() {
            last = f.key(form::Key::Char(ch));
        }
        last
    }

    fn shown_key(s: &WgSheet) -> Vec<String> {
        match &s.sheet.pages[TAB_IFACE].item("pub").unwrap().control {
            Control::Lines(lines) => lines.clone(),
            other => panic!("pub is {other:?}"),
        }
    }

    /// A public key of the widest characters fits its field in DejaVu Sans, at 100% and 200%
    /// text, and the rows under it clear its second line.
    #[test]
    fn the_public_key_fits_its_field() {
        let text = reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig {
            family: vec!["DejaVu Sans".into()],
            ..Default::default()
        });
        if !text.has_face("DejaVu Sans") {
            return;
        }
        let mut s = WgSheet::new(&cfg(), None);
        let page = &mut s.sheet.pages[TAB_IFACE];
        let lines = key_lines(&format!("{}=", "W".repeat(43)));
        page.set_lines("pub", &lines);
        for pct in [100, 200] {
            let theme = reprisekit::theme::Theme::default().with_form_text(pct);
            page.drawn_with(&theme.paint());
            let field = page.rect("pub").unwrap().width;
            for l in &lines {
                let w = text.measure(l, false, theme.fonts.size_px);
                assert!(w <= field, "{pct}%: {w} px in {field}");
            }
        }
        let (key, addr) = (
            page.item("pub").unwrap().rect,
            page.item("addr").unwrap().rect,
        );
        assert!(key.bottom() <= addr.y, "{key:?} runs into {addr:?}");
    }

    #[test]
    fn ok_hands_back_the_connection_as_it_was_loaded() {
        let mut s = WgSheet::new(&cfg(), Some("/Settings/4".into()));
        assert_eq!(
            s.event(propsheet::Event::Ok),
            Outcome::Save(Box::new(cfg()))
        );
        assert_eq!(s.window_title(), "Away Properties");
        assert_eq!(
            shown_key(&s),
            ["HIgo9xNzJMWLKASShiTqIy", "bxZ0U3wGLiUeJ1PKf8ykw="],
            "the public key is text on two lines, not a field"
        );
    }

    #[test]
    fn generate_puts_a_new_key_in_and_shows_its_public_half() {
        let mut s = WgSheet::new(&cfg(), None);
        s.event(propsheet::Event::Page(
            TAB_IFACE,
            form::Event::Pressed("generate"),
        ));
        let key = text_of(&s.sheet.pages[TAB_IFACE], "key").to_string();
        assert_ne!(key, KEY_A);
        assert_eq!(Some(shown_key(&s).concat()), wg::public_key(&key));
    }

    #[test]
    fn peers_are_added_edited_and_removed_through_the_list() {
        let mut s = WgSheet::new(&cfg(), None);
        let page = |s: &mut WgSheet, e| s.event(propsheet::Event::Page(TAB_PEERS, e));
        page(&mut s, form::Event::Pressed("add"));
        assert_eq!(s.shown, Some(1));
        let ev = type_into(&mut s.sheet.pages[TAB_PEERS], "pk", KEY_A);
        page(&mut s, ev);
        let ev = type_into(&mut s.sheet.pages[TAB_PEERS], "allowed", "10.9.0.0/16");
        page(&mut s, ev);
        let lines = match &s.sheet.pages[TAB_PEERS].item("peers").unwrap().control {
            Control::ListBox { list, .. } => list.items.clone(),
            _ => vec![],
        };
        assert_eq!(
            lines,
            [
                "Peer 1: vpn.example.org:51820",
                "Peer 2: yAnz5TF+lXXJte14..."
            ]
        );
        let saved = match s.event(propsheet::Event::Ok) {
            Outcome::Save(c) => c,
            other => panic!("{other:?}"),
        };
        assert_eq!(saved.peers.len(), 2);
        assert_eq!(saved.peers[1].allowed_ips, ["10.9.0.0/16"]);
        // Removing the first leaves the second in the fields.
        s.show_peer(Some(0));
        page(&mut s, form::Event::Pressed("remove"));
        assert_eq!(s.peers.len(), 1);
        assert_eq!(text_of(&s.sheet.pages[TAB_PEERS], "pk"), KEY_A);
        page(&mut s, form::Event::Pressed("remove"));
        assert!(s.peers.is_empty());
        assert!(matches!(
            s.sheet.pages[TAB_PEERS].item("pk").unwrap().control,
            Control::Edit { enabled: false, .. }
        ));
    }

    #[test]
    fn a_bad_field_keeps_the_sheet_open_on_it() {
        let mut s = WgSheet::new(&cfg(), None);
        set_field(&mut s.sheet.pages[TAB_PEERS], "endpoint", "nowhere");
        s.take_peer_fields();
        let out = s.event(propsheet::Event::Ok);
        assert_eq!(
            out,
            Outcome::Problem(
                "Peer 1's endpoint must be host:port, such as vpn.example.org:51820.".into()
            )
        );
        assert_eq!(s.sheet.tabs.open, TAB_PEERS);
        let f = &s.sheet.pages[TAB_PEERS];
        assert_eq!(f.focus(), f.index_of("endpoint"));
    }

    #[test]
    fn a_fresh_sheet_has_a_key_a_free_name_and_one_peer() {
        let taken = vec![nm::Vpn {
            name: "WireGuard 1".into(),
            ..nm::Vpn::default()
        }];
        let s = WgSheet::fresh(&taken);
        let c = s.config();
        assert_eq!(c.name, "WireGuard 2");
        assert_eq!(c.iface, "wg1");
        assert!(wg::decode_key(&c.private_key).is_some());
        assert_eq!(c.peers.len(), 1);
    }

    #[test]
    fn plugin_settings_round_trip_through_the_memo() {
        let data = vec![
            ("dev".to_string(), "tun".to_string()),
            ("remote".to_string(), "vpn.example.org".to_string()),
        ];
        assert_eq!(data_text(&data), "dev = tun\nremote = vpn.example.org");
        assert_eq!(parse_data(&data_text(&data)), data);
        assert_eq!(
            parse_data("a=1\n\n  flag  \n= x\n"),
            [
                ("a".to_string(), "1".to_string()),
                ("flag".into(), "".into())
            ]
        );
        let mut s = PluginSheet::new("/Settings/3", "Work", "openvpn", &data);
        assert_eq!(s.window_title(), "Work Properties");
        assert_eq!(
            s.event(propsheet::Event::Ok),
            Outcome::SavePlugin {
                name: "Work".into(),
                data
            }
        );
    }

    /// Every control sits inside the page, clear of the button row.
    #[test]
    fn the_pages_fit_the_sheet() {
        let s = WgSheet::new(&cfg(), None);
        let o = OvpnSheet::new("/Settings/5", &ovpn_cfg());
        let page_bottom = CLIENT.1 - 36;
        for page in s.sheet.pages.iter().chain(&o.sheet.pages) {
            for item in &page.items {
                if matches!(item.id, propsheet::OK | propsheet::CANCEL) {
                    continue;
                }
                let r = item.rect;
                assert!(
                    r.x >= 6 && r.right() <= CLIENT.0 - 6 && r.bottom() <= page_bottom,
                    "{} at {r:?} is outside the page",
                    item.id
                );
            }
        }
    }

    /// The WireGuard page's pixels: the key field masked, the public key
    /// set in the label column's font, Generate beside the key.
    #[test]
    fn the_wireguard_page_draws_its_fields() {
        let theme = reprisekit::theme::Theme::default();
        let text = reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let s = WgSheet::new(&cfg(), None);
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        propsheet::draw(
            &mut c,
            &text,
            &theme.paint(),
            &s.sheet,
            true,
            theme.fonts.size_px,
        );
        let f = &s.sheet.pages[TAB_IFACE];
        let key = f.item("key").unwrap().rect;
        let field = |r: Rect| (r.x + 4..r.right() - 4).map(move |x| (x, r.y + r.height / 2));
        // The field is the window colour inside its sunken edge, and the
        // bullets put dark ink on its middle row.
        assert_eq!(
            c.get(key.x + 3, key.y + 3) & 0xffffff,
            theme.colors.window.argb() & 0xffffff,
            "the key field's face is the window colour"
        );
        assert!(
            field(key).any(|(x, y)| c.get(x, y) & 0xffffff == 0),
            "the key's bullets are drawn"
        );
        let generate = f.item("generate").unwrap().rect;
        assert_eq!(
            c.get(generate.x, generate.y) & 0xffffff,
            theme.colors.button_hilight.argb() & 0xffffff,
            "Generate's raised top-left edge"
        );
        let public = f.item("pub").unwrap().rect;
        assert!(
            (public.x..public.right())
                .any(|x| c.get(x, public.y + 6) & 0xffffff
                    == theme.colors.button_text.argb() & 0xffffff),
            "the public key's ink"
        );
    }

    fn ovpn_cfg() -> ovpn::Config {
        ovpn::from_data(
            "Office",
            &[
                ("connection-type".into(), "password-tls".into()),
                ("remote".into(), "vpn.example.org:1194".into()),
                ("username".into(), "u".into()),
                ("ca".into(), "/srv/vpn/ca.crt".into()),
                ("cert".into(), "/srv/vpn/u.crt".into()),
                ("key".into(), "/srv/vpn/u.key".into()),
                ("dev".into(), "tun".into()),
            ],
        )
    }

    fn live(f: &Form, id: &str) -> bool {
        match &f.item(id).unwrap().control {
            Control::Edit { enabled, .. } | Control::Button { enabled, .. } => *enabled,
            Control::Combo(c) => c.enabled,
            other => panic!("{id} is {other:?}"),
        }
    }

    #[test]
    fn ok_on_an_untouched_openvpn_sheet_saves_it_as_loaded() {
        let mut s = OvpnSheet::new("/Settings/5", &ovpn_cfg());
        assert_eq!(s.window_title(), "Office Properties");
        assert_eq!(
            s.event(propsheet::Event::Ok),
            Outcome::SaveOpenVpn(Box::new(ovpn_cfg()))
        );
        assert_eq!(
            cue_of(&s.sheet.pages[TAB_GENERAL], "password"),
            KEEP_PASSWORD_CUE
        );
    }

    #[test]
    fn the_type_greys_the_fields_it_does_not_use() {
        let mut s = OvpnSheet::new("/Settings/5", &ovpn_cfg());
        let (g, a) = (TAB_GENERAL, TAB_AUTH);
        assert!(live(&s.sheet.pages[g], "user") && live(&s.sheet.pages[a], "cert"));
        assert!(!live(&s.sheet.pages[a], "static_browse"));
        for (auth, user, ca, certs, stat) in [
            (ovpn::Auth::Tls, false, true, true, false),
            (ovpn::Auth::Password, true, true, false, false),
            (ovpn::Auth::StaticKey, false, false, false, true),
        ] {
            s.sheet.pages[g].set_combo("type", auth.index());
            s.event(propsheet::Event::Page(g, form::Event::Changed("type")));
            let (gp, ap) = (&s.sheet.pages[g], &s.sheet.pages[a]);
            assert_eq!(live(gp, "password"), user, "{auth:?} password");
            assert_eq!(live(ap, "ca_browse"), ca, "{auth:?} CA");
            assert_eq!(live(ap, "keypass"), certs, "{auth:?} key password");
            assert_eq!(live(ap, "direction"), stat, "{auth:?} direction");
        }
        assert_eq!(
            s.config().cert,
            "/srv/vpn/u.crt",
            "a greyed field keeps its value"
        );
    }

    #[test]
    fn browse_names_its_field_and_the_pick_fills_it() {
        let mut s = OvpnSheet::new("/Settings/5", &ovpn_cfg());
        assert_eq!(
            s.event(propsheet::Event::Page(
                TAB_AUTH,
                form::Event::Pressed("key_browse")
            )),
            Outcome::Browse("key")
        );
        s.set_path("key", "/srv/vpn/new.key");
        assert_eq!(s.config().key, "/srv/vpn/new.key");
        assert_eq!(
            s.path_of("ca"),
            Some(std::path::PathBuf::from("/srv/vpn/ca.crt"))
        );
    }

    #[test]
    fn a_typed_password_saves_and_a_missing_file_keeps_the_sheet_open_on_it() {
        let mut s = OvpnSheet::new("/Settings/5", &ovpn_cfg());
        set_field(&mut s.sheet.pages[TAB_GENERAL], "password", "hunter2");
        match s.event(propsheet::Event::Ok) {
            Outcome::SaveOpenVpn(c) => assert_eq!(&*c.password, "hunter2"),
            other => panic!("{other:?}"),
        }
        set_field(&mut s.sheet.pages[TAB_AUTH], "ca", " ");
        assert_eq!(
            s.event(propsheet::Event::Ok),
            Outcome::Problem("Choose the CA certificate file.".into())
        );
        assert_eq!(s.sheet.tabs.open, TAB_AUTH);
        let f = &s.sheet.pages[TAB_AUTH];
        assert_eq!(f.focus(), f.index_of("ca"));
    }

    /// The Authentication page's pixels: live file fields in the window colour beside raised
    /// Browse... buttons, the static key's greyed fields in the button face.
    #[test]
    fn the_openvpn_authentication_page_draws_live_and_greyed_fields() {
        let theme = reprisekit::theme::Theme::default();
        let text = reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let mut s = OvpnSheet::new("/Settings/5", &ovpn_cfg());
        s.sheet.open_tab(TAB_AUTH);
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        propsheet::draw(
            &mut c,
            &text,
            &theme.paint(),
            &s.sheet,
            true,
            theme.fonts.size_px,
        );
        let f = &s.sheet.pages[TAB_AUTH];
        let rgb = |x, y| c.get(x, y) & 0xffffff;
        let face = |id: &str| {
            let r = f.item(id).unwrap().rect;
            rgb(r.right() - 4, r.y + 3)
        };
        let window = theme.colors.window.argb() & 0xffffff;
        let button = theme.colors.button_face.argb() & 0xffffff;
        assert_eq!(face("ca"), window, "a live file field");
        assert_eq!(face("static"), button, "a greyed file field");
        assert_eq!(face("local"), button, "a greyed address");
        let b = f.item("ca_browse").unwrap().rect;
        assert_eq!(
            rgb(b.x, b.y),
            theme.colors.button_hilight.argb() & 0xffffff,
            "Browse...'s raised top-left edge"
        );
        let ca = f.item("ca").unwrap().rect;
        assert!(
            (ca.x + 4..ca.right() - 4).any(|x| rgb(x, ca.y + ca.height / 2) == 0),
            "the CA path's ink"
        );
    }

    /// Writes target/vpn-sheet-*.png, for eyeballing the layout.
    #[test]
    #[ignore]
    fn render_vpn_sheets() {
        let theme = reprisekit::theme::Theme::default();
        let text = reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let mut s = WgSheet::new(&cfg(), None);
        for (i, name) in ["vpn-sheet-wireguard.png", "vpn-sheet-peers.png"]
            .iter()
            .enumerate()
        {
            s.sheet.open_tab(i);
            let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            propsheet::draw(
                &mut c,
                &text,
                &theme.paint(),
                &s.sheet,
                true,
                theme.fonts.size_px,
            );
            reprisekit::raster::write_preview(&c, name);
        }
        let p = PluginSheet::new(
            "/Settings/3",
            "Work",
            "openvpn",
            &[("remote".into(), "vpn.example.org".into())],
        );
        let mut c = Canvas::new(PLUGIN_CLIENT.0 as u32, PLUGIN_CLIENT.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        propsheet::draw(
            &mut c,
            &text,
            &theme.paint(),
            &p.sheet,
            true,
            theme.fonts.size_px,
        );
        reprisekit::raster::write_preview(&c, "vpn-sheet-plugin.png");
        let mut o = OvpnSheet::new("/Settings/5", &ovpn_cfg());
        for (i, name) in ["vpn-sheet-openvpn.png", "vpn-sheet-openvpn-auth.png"]
            .iter()
            .enumerate()
        {
            o.sheet.open_tab(i);
            let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            propsheet::draw(
                &mut c,
                &text,
                &theme.paint(),
                &o.sheet,
                true,
                theme.fonts.size_px,
            );
            reprisekit::raster::write_preview(&c, name);
        }
    }
}
