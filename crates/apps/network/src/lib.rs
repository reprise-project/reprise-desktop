//! Network Connections: the Reprise network icon and its windows.
//!
//! One process: a tray icon following NetworkManager, the Network Connections
//! folder behind it and the windows they open. `--tray`, which the autostart
//! entry passes, starts with the folder hidden. While the tray icon is up,
//! closing the folder hides it and only Exit in the tray menu quits.
//!
//! Everything here is glue: the store hands over snapshots, the window modules
//! own the models and hand back [`folder::Outcome`] and its siblings, and this
//! file turns those into `nm::Cmd`s and windows.

#[cfg(test)]
reprisebus::test_session!();

mod chooser;
mod folder;
mod hotspot;
mod import;
mod keydlg;
mod nm;
mod ovpn;
mod speed;
mod stations;
mod status;
mod tray;
mod vpnsheet;
mod wg;

use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Receiver;

use repriseapp::asks::{Answer, Asks};
use repriseapp::filedlg::{Driven, Host};
use repriseapp::keys::{form_clipboard, form_input, sheet_input};
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{App, Child, Ctx, DropForm, Event, FileDrop, PointerInput, Reply, WinId};
use reprisebus::airplane::Mode;
use reprisekit::access::Tree;
use reprisekit::art::neticon;
use reprisekit::controls::filedlg::{self, DlgMode, Fields, FileDlg};
use reprisekit::controls::form;
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::msgbox;
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::{TextUnits, Theme};

/// The windows the folder and the tray open. One Status sheet and one VPN
/// sheet at a time: opening another replaces it, except a VPN sheet with
/// unsaved edits, which comes forward instead.
const CHOOSER: WinId = WinId::named("chooser");
const KEY: WinId = WinId::named("key");
const EAP: WinId = WinId::named("eap");
const STATUS: WinId = WinId::named("status");
const VPN: WinId = WinId::named("vpn");
const IMPORT: WinId = WinId::named("import");
const HOTSPOT: WinId = WinId::named("hotspot");
const NOTICE: WinId = WinId::named("notice");
const BROWSE_TITLE: &str = "Choose File";

/// Where the sign-in balloon sends the browser when NetworkManager has no
/// check address of its own.
const FALLBACK_CHECK_URI: &str = "http://nmcheck.gnome.org/check_network_status.txt";

enum VpnWin {
    WireGuard(vpnsheet::WgSheet),
    OpenVpn(vpnsheet::OvpnSheet),
    Plugin(vpnsheet::PluginSheet),
}

impl VpnWin {
    fn sheet(&self) -> &PropSheet {
        match self {
            VpnWin::WireGuard(s) => &s.sheet,
            VpnWin::OpenVpn(s) => &s.sheet,
            VpnWin::Plugin(s) => &s.sheet,
        }
    }

    fn sheet_mut(&mut self) -> &mut PropSheet {
        match self {
            VpnWin::WireGuard(s) => &mut s.sheet,
            VpnWin::OpenVpn(s) => &mut s.sheet,
            VpnWin::Plugin(s) => &mut s.sheet,
        }
    }

    fn title(&self) -> String {
        match self {
            VpnWin::WireGuard(s) => s.window_title(),
            VpnWin::OpenVpn(s) => s.window_title(),
            VpnWin::Plugin(s) => s.window_title(),
        }
    }

    fn values(&self) -> VpnValues {
        match self {
            VpnWin::WireGuard(s) => VpnValues::WireGuard(Box::new(s.config())),
            VpnWin::OpenVpn(s) => VpnValues::OpenVpn(Box::new(s.config())),
            VpnWin::Plugin(s) => VpnValues::Plugin(s.values()),
        }
    }

    /// A connection NetworkManager does not have yet: a new or imported WireGuard one.
    fn is_new(&self) -> bool {
        matches!(self, VpnWin::WireGuard(s) if s.conn_path.is_none())
    }
}

/// What a VPN sheet's OK would save.
#[derive(PartialEq)]
enum VpnValues {
    WireGuard(Box<wg::Config>),
    OpenVpn(Box<ovpn::Config>),
    Plugin((String, Vec<(String, String)>)),
}

/// The field a Browse... file dialog fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Browse {
    /// A field of VPN sheet number `sheet` ([`NetApp::vpn_opened`]).
    Ovpn {
        field: &'static str,
        sheet: u64,
    },
    Eap(&'static str),
}

/// The window an OK box was raised from: it covers that window and leaves with it.
enum Over {
    Main,
    /// The chooser while it shows this radio.
    Chooser(String),
    /// VPN sheet number `n` ([`NetApp::vpn_opened`]) while it is open.
    Vpn(u64),
}

/// What a notice box answers for.
enum Notice {
    /// An OK box.
    Tell(Over),
    /// Confirm Connection Delete: Yes deletes this saved connection.
    Delete(String),
}

/// The notice boxes, in turn: a type of its own for `NetApp`'s `Default`.
struct Notices(Asks<Notice>);

impl Default for Notices {
    fn default() -> Notices {
        Notices(Asks::new(NOTICE))
    }
}

impl std::ops::Deref for Notices {
    type Target = Asks<Notice>;
    fn deref(&self) -> &Asks<Notice> {
        &self.0
    }
}

impl std::ops::DerefMut for Notices {
    fn deref_mut(&mut self) -> &mut Asks<Notice> {
        &mut self.0
    }
}

#[derive(Default)]
struct NetApp {
    tray: Option<reprisetray::TrayHandle>,
    msgs: Option<Receiver<reprisetray::Msg>>,
    nm_rx: Option<Receiver<nm::NmMsg>>,
    nm: Option<nm::NmHandle>,
    snapshot: nm::Snapshot,
    edges: tray::Edges,
    /// The last pixmap, tooltip, status and menu shape the tray item was
    /// given.
    tray_state: Option<(Vec<u8>, String, bool, tray::MenuShape)>,
    folder: folder::Folder,
    chooser: Option<chooser::Chooser>,
    /// Saved networks whose last join failed on the key, which the chooser asks for again.
    refused: HashSet<nm::Ssid>,
    key: Option<keydlg::KeyDialog>,
    eap: Option<keydlg::eap::EapDialog>,
    status: Option<status::Status>,
    /// Reads the Status sheet's interface counters while the sheet is open.
    speed: Option<speed::Ticker>,
    waker: Option<repriseapp::Waker>,
    vpn: Option<VpnWin>,
    /// The VPN sheets opened so far: the number of the one in `vpn`.
    vpn_opened: u64,
    /// What the sheet in `vpn` held when it opened.
    vpn_opened_with: Option<VpnValues>,
    import: Option<FileDlg>,
    import_host: Host,
    /// The file dialog is a Browse... for this field, not Import.
    browse_for: Option<Browse>,
    hotspot: Option<hotspot::HotspotDialog>,
    notices: Notices,
    /// The message box's font, loaded in `start`: `children` sizes its window without a `Ctx`.
    dialog_text: Option<Rc<TextRenderer>>,
    /// `Ctx::forms` as of the last event, which `children` and `initial_size` size windows by.
    forms: Theme,
    /// The icon theme's "network-vpn" at 16 px; see `read_icons`.
    vpn_icon: Option<Canvas>,
    icons_read: bool,
    /// The rows changed: the next draw of the folder fits its last
    /// column to the body again and scrolls the selection back into view.
    refit: bool,
    hidden: bool,
    tray_only: bool,
}

impl NetApp {
    fn send(&self, cmd: nm::Cmd) {
        if let Some(h) = &self.nm {
            h.send(cmd);
        }
    }

    /// Disable a managed device, enable an unmanaged one. Both tray and
    /// window read the first row's wording off the same flag.
    fn toggle(&mut self, path: &str) {
        match self.snapshot.devices.iter().find(|d| d.path == path) {
            Some(d) if d.managed => self.send(nm::Cmd::Disable(path.into())),
            Some(_) => self.send(nm::Cmd::Enable(path.into())),
            None => {}
        }
    }

    /// The Status sheet for `path`, replacing whatever was open, with its interface counters read
    /// while it stays open. A sheet already showing `path` stays, and comes back as the window to
    /// raise.
    fn open_status(&mut self, path: &str) -> Option<WinId> {
        if self.status.as_ref().is_some_and(|s| s.device_path == path) {
            return Some(STATUS);
        }
        let d = self.snapshot.devices.iter().find(|d| d.path == path)?;
        let (next, iface) = (status::Status::new(d), d.iface.clone());
        self.close_status();
        self.status = Some(next);
        if let Some(w) = self.waker.clone() {
            self.speed = Some(speed::start(iface, move || w.wake()));
        }
        None
    }

    fn close_status(&mut self) {
        self.speed = None;
        self.status = None;
    }

    /// The chooser for radio `path`, in place of the one open, whose key prompt or login dialog
    /// closes unanswered. A chooser already showing `path` stays with its pick and prompts, and
    /// comes back as the window to raise.
    fn open_chooser(&mut self, path: &str) -> Option<WinId> {
        if self.chooser.as_ref().is_some_and(|c| c.device_path == path) {
            return Some(CHOOSER);
        }
        let found = self
            .snapshot
            .devices
            .iter()
            .find(|d| d.path == path && d.kind == nm::Kind::Wifi)
            .map(|d| chooser::Chooser::new(d, self.forms.text));
        if let Some(c) = found {
            self.close_chooser();
            self.chooser = Some(c);
            self.sync_refused();
        }
        None
    }

    /// Hand the chooser the networks whose key it asks for again.
    fn sync_refused(&mut self) {
        if let Some(c) = &mut self.chooser {
            c.refused = self.refused.clone();
        }
    }

    /// Drop the chooser, and the key dialogs that belong to it.
    fn close_chooser(&mut self) {
        self.chooser = None;
        self.key = None;
        self.eap = None;
    }

    fn primary_path(&self) -> Option<String> {
        tray::primary(&self.snapshot).map(|d| d.path.clone())
    }

    fn wifi_path(&self) -> Option<String> {
        tray::wifi_device(&self.snapshot).map(|d| d.path.clone())
    }

    /// Push the icon, tooltip, status and menu shape the snapshot asks
    /// for, unless the tray already has them.
    fn push_state(&mut self) {
        let (icon, vpn) = tray::icon_for(&self.snapshot);
        let mut drawn = neticon::draw(icon, vpn);
        if tray::limited(&self.snapshot) {
            neticon::badge_limited(&mut drawn);
        }
        let next = (
            drawn.to_argb_be(),
            tray::tooltip(&self.snapshot),
            self.snapshot.nm_running && !self.snapshot.devices.is_empty(),
            tray::menu_shape(&self.snapshot),
        );
        if self.tray_state.as_ref() == Some(&next) {
            return;
        }
        if let Some(t) = &self.tray {
            t.set_state(tray::shown(next.0.clone(), next.1.clone(), next.2, &next.3));
        }
        self.tray_state = Some(next);
    }

    /// The balloons a snapshot's change is worth. Every snapshot goes
    /// through here, so a burst drained in one wake still shows each edge.
    fn announce(&mut self, s: &nm::Snapshot) {
        for b in self.edges.feed(s) {
            log::info!("{}", b.summary);
            if let Some(t) = &self.tray {
                match b.tag {
                    Some(tag) => t.notify_clickable(&b.summary, &b.body, tag),
                    None => t.notify(&b.summary, &b.body),
                }
            }
        }
    }

    /// Open the network's sign-in page: the browser goes to the address
    /// NetworkManager checks with, and the network's portal redirects it.
    /// `token` is the balloon click's activation token, so the browser
    /// window may take the focus.
    fn sign_in(&self, token: Option<String>) {
        let uri = if self.snapshot.check_uri.is_empty() {
            FALLBACK_CHECK_URI
        } else {
            &self.snapshot.check_uri
        };
        log::info!("opening the sign-in page through {uri}");
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(uri);
        if let Some(t) = token {
            cmd.env("XDG_ACTIVATION_TOKEN", t);
        }
        match cmd.spawn() {
            // Reaped on a thread so it leaves no zombie behind.
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(e) => log::warn!("xdg-open {uri}: {e}"),
        }
    }

    fn tell(&mut self, title: &str, text: &str) {
        self.tell_over(Over::Main, title, text);
    }

    /// [`NetApp::tell`] for the VPN sheet open.
    fn tell_from_sheet(&mut self, title: &str, text: &str) {
        self.tell_over(Over::Vpn(self.vpn_opened), title, text);
    }

    fn tell_over(&mut self, over: Over, title: &str, text: &str) {
        self.notices.ask(ok_box(title, text), Notice::Tell(over));
    }

    /// The window `over` names, `None` once it closed or shows something else.
    fn window_of(&self, over: &Over) -> Option<WinId> {
        match over {
            Over::Main => Some(repriseapp::MAIN),
            Over::Chooser(radio) => (self.chooser.as_ref())
                .filter(|c| &c.device_path == radio)
                .map(|_| CHOOSER),
            Over::Vpn(n) => (self.vpn.is_some() && *n == self.vpn_opened).then_some(VPN),
        }
    }

    /// The window the file dialog was raised from: a Browse... covers the sheet or dialog that
    /// asked, Import the folder.
    fn browse_over(&self) -> Option<WinId> {
        match self.browse_for {
            Some(Browse::Ovpn { sheet, .. }) => self.window_of(&Over::Vpn(sheet)),
            Some(Browse::Eap(_)) => self.eap.as_ref().map(|_| EAP),
            None => Some(repriseapp::MAIN),
        }
    }

    /// Takes the boxes whose window closed: the notices raised from it, and a Browse... whose
    /// sheet or dialog went.
    fn withdraw_orphans(&mut self) {
        let mut notices = std::mem::take(&mut self.notices);
        notices.withdraw(|n| matches!(n, Notice::Tell(over) if self.window_of(over).is_none()));
        self.notices = notices;
        if self.import.is_some() && self.browse_over().is_none() {
            self.picked(None);
        }
    }

    /// Opens `w` in the VPN sheet's window in place of the sheet there, unless that sheet holds
    /// what NetworkManager does not: then it stays, and comes back as the window to raise.
    fn show_vpn(&mut self, w: VpnWin) -> Option<WinId> {
        let unsaved = (self.vpn.as_ref())
            .is_some_and(|v| v.is_new() || Some(v.values()) != self.vpn_opened_with);
        if unsaved {
            return Some(VPN);
        }
        self.vpn_opened += 1;
        self.vpn_opened_with = Some(w.values());
        self.vpn = Some(w);
        None
    }

    fn ask_delete(&mut self, conn_path: &str) {
        let name = self
            .snapshot
            .vpns
            .iter()
            .find(|v| v.conn_path == conn_path)
            .map(|v| v.name.clone())
            .unwrap_or_default();
        let b = MsgBox::new(
            "Confirm Connection Delete",
            msgbox::Fields {
                text: wrap(&format!(
                    "Are you sure you want to delete the connection '{name}'?"
                )),
                icon: None,
                buttons: vec!["&Yes".into(), "&No".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        );
        self.notices.ask(b, Notice::Delete(conn_path.to_string()));
    }

    fn toggle_vpn(&mut self, key: u64) {
        let vpns = &self.snapshot.vpns;
        let Some(v) = vpns.iter().find(|v| tray::vpn_key(&v.conn_path) == key) else {
            return;
        };
        let cmd = if v.active {
            nm::Cmd::VpnDown(v.conn_path.clone())
        } else {
            nm::Cmd::VpnUp(v.conn_path.clone())
        };
        self.send(cmd);
    }

    /// The Create Hotspot dialog for radio `device`, in place of the one open, unless that one
    /// holds typed input: then it stays, and comes back as the window to raise.
    fn open_hotspot(&mut self, device: &str) -> Option<WinId> {
        if self
            .hotspot
            .as_ref()
            .is_some_and(hotspot::HotspotDialog::edited)
        {
            return Some(HOTSPOT);
        }
        let joined = (self.snapshot.devices.iter())
            .find(|d| d.path == device)
            .and_then(|d| d.wifi.as_ref()?.active_ssid.clone());
        self.hotspot = Some(hotspot::HotspotDialog::new(
            device,
            &hotspot::default_ssid(),
            joined.as_deref(),
        ));
        None
    }

    fn open_import(&mut self, u: TextUnits) {
        self.browse_for = None;
        self.import = Some(self.import_host.open_last(
            &home_dir(),
            Fields {
                title: "Import".into(),
                accept_label: "Open".into(),
                mode: DlgMode::Open,
                multiple: false,
                filters: vec![
                    "VPN Files (*.conf;*.ovpn;*.pcf)".into(),
                    "All Files (*.*)".into(),
                ],
            },
            u,
        ));
    }

    /// A Browse... for `to`: the file dialog opens where its file is.
    fn open_browse(&mut self, to: Browse, u: TextUnits) {
        let current = match to {
            Browse::Ovpn { field, .. } => match &self.vpn {
                Some(VpnWin::OpenVpn(s)) => s.path_of(field),
                _ => return,
            },
            Browse::Eap(field) => match &self.eap {
                Some(d) => d.path_of(field),
                None => return,
            },
        };
        let start = current
            .and_then(|p| p.parent().map(PathBuf::from))
            .filter(|d| d.is_dir());
        self.browse_for = Some(to);
        let fields = Fields {
            title: BROWSE_TITLE.into(),
            accept_label: "Open".into(),
            mode: DlgMode::Open,
            multiple: false,
            filters: vec![
                "Certificates and Keys (*.crt;*.pem;*.key)".into(),
                "All Files (*.*)".into(),
            ],
        };
        self.import = Some(match start {
            Some(dir) => self.import_host.open(&dir, fields, u),
            None => self.import_host.open_last(&home_dir(), fields, u),
        });
    }

    fn file_title(&self) -> &'static str {
        if self.browse_for.is_some() {
            BROWSE_TITLE
        } else {
            "Import"
        }
    }

    /// A file picked in Import: a WireGuard file opens in its sheet, any
    /// other goes to NetworkManager, and one nothing can read says why.
    /// The window to raise comes back, as from [`NetApp::show_vpn`].
    fn import_file(&mut self, path: PathBuf) -> Option<WinId> {
        let text = match std::fs::read(&path) {
            Ok(bytes) => String::from_utf8(bytes).ok(),
            Err(e) => {
                self.tell(
                    "Import",
                    &format!("{} could not be read: {e}.", path.display()),
                );
                return None;
            }
        };
        match import::plan(&path, text.as_deref(), &import::installed_plugins()) {
            Ok(import::Plan::WireGuard(cfg)) => {
                return self.show_vpn(VpnWin::WireGuard(vpnsheet::WgSheet::new(&cfg, None)));
            }
            Ok(import::Plan::Plugins(kinds)) => self.send(nm::Cmd::Import { path, kinds }),
            Err(why) => self.tell("Import", &why),
        }
        None
    }

    /// The latest snapshot: the folder's rows, the two windows watching a
    /// device and the tray item.
    fn snapshot_in(&mut self, s: nm::Snapshot) {
        log::debug!(
            "snapshot: {} devices, {} vpns, NetworkManager {}",
            s.devices.len(),
            s.vpns.len(),
            if s.nm_running { "up" } else { "down" }
        );
        self.snapshot = s;
        let devices = &self.snapshot.devices;
        let joined = |ssid: &nm::Ssid| {
            (devices.iter().filter_map(|d| d.wifi.as_ref()))
                .any(|w| w.aps.iter().any(|a| a.active && &a.ssid == ssid))
        };
        let refused = self.refused.len();
        self.refused.retain(|s| !joined(s));
        if self.refused.len() != refused {
            self.sync_refused();
        }
        self.folder.refresh(&self.snapshot, self.vpn_icon.clone());
        self.refit = true;
        match self.chooser.as_ref().and_then(|c| {
            self.snapshot
                .devices
                .iter()
                .find(|d| d.path == c.device_path)
        }) {
            Some(d) => {
                let d = d.clone();
                if let Some(c) = &mut self.chooser {
                    c.refresh(&d);
                }
            }
            None => self.close_chooser(),
        }
        match self.status.as_ref().and_then(|s| {
            self.snapshot
                .devices
                .iter()
                .find(|d| d.path == s.device_path)
        }) {
            Some(d) => {
                let d = d.clone();
                if let Some(s) = &mut self.status {
                    s.refresh(&d);
                }
            }
            None => self.close_status(),
        }
        self.push_state();
    }

    /// Drain the store's and the tray's channels. Quits only on the
    /// tray's Exit.
    fn wake(&mut self) -> Reply {
        while let Some((at, c)) = self.speed.as_ref().and_then(|t| t.rx.try_recv().ok()) {
            if let Some(s) = &mut self.status {
                s.counters(at, c);
            }
        }
        let (mut latest, mut raise) = (None, None);
        while let Some(msg) = self.nm_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match msg {
                nm::NmMsg::Snapshot(s) => {
                    self.announce(&s);
                    latest = Some(s);
                }
                nm::NmMsg::Failed { title, body } => {
                    log::warn!("{title}: {body}");
                    if let Some(t) = &self.tray {
                        t.notify(&title, &body);
                    }
                }
                nm::NmMsg::KeyRefused(ssid) => {
                    if let Some(c) = self
                        .chooser
                        .as_mut()
                        .filter(|c| c.connecting == Some(ssid.clone()))
                    {
                        c.connecting = None;
                    }
                    self.refused.insert(ssid);
                    self.sync_refused();
                }
                nm::NmMsg::Vpn(edit) => {
                    raise = raise.or(self.show_vpn(match edit {
                        nm::VpnEdit::WireGuard { conn_path, cfg } => {
                            VpnWin::WireGuard(vpnsheet::WgSheet::new(&cfg, Some(conn_path)))
                        }
                        nm::VpnEdit::Plugin {
                            conn_path,
                            name,
                            kind,
                            data,
                        } if kind == "openvpn" => VpnWin::OpenVpn(vpnsheet::OvpnSheet::new(
                            &conn_path,
                            &ovpn::from_data(&name, &data),
                        )),
                        nm::VpnEdit::Plugin {
                            conn_path,
                            name,
                            kind,
                            data,
                        } => VpnWin::Plugin(vpnsheet::PluginSheet::new(
                            &conn_path, &name, &kind, &data,
                        )),
                    }));
                    self.hidden = false;
                }
            }
        }
        match latest {
            Some(mut s) => {
                for d in &mut s.devices {
                    d.hotspot_clients = hotspot_clients(d);
                }
                self.snapshot_in(s);
            }
            None if self
                .snapshot
                .devices
                .iter()
                .any(|d| hotspot_clients(d) != d.hotspot_clients) =>
            {
                let mut s = self.snapshot.clone();
                for d in &mut s.devices {
                    d.hotspot_clients = hotspot_clients(d);
                }
                self.snapshot_in(s);
            }
            None => {}
        }
        let mut shown = None;
        let mut raised = false;
        while let Some(raw) = self.msgs.as_ref().and_then(|rx| rx.try_recv().ok()) {
            let Some(msg) = tray::translate(raw, &tray::menu_shape(&self.snapshot)) else {
                continue;
            };
            raised = true;
            match msg {
                tray::TrayMsg::Exit => return self.quit(),
                tray::TrayMsg::Folder => {
                    self.hidden = false;
                    shown = Some(repriseapp::MAIN);
                }
                tray::TrayMsg::Status => {
                    if let Some(p) = self.primary_path() {
                        self.open_status(&p);
                        shown = Some(STATUS);
                    }
                }
                tray::TrayMsg::Wireless => {
                    if let Some(p) = self.wifi_path() {
                        self.open_chooser(&p);
                        shown = Some(CHOOSER);
                    }
                }
                tray::TrayMsg::Toggle => {
                    if let Some(p) = self.primary_path() {
                        self.toggle(&p);
                    }
                }
                tray::TrayMsg::Repair => {
                    if let Some(p) = self.primary_path() {
                        self.send(nm::Cmd::Repair(p));
                    }
                }
                tray::TrayMsg::Vpn(i) => self.toggle_vpn(i),
                tray::TrayMsg::Airplane => {
                    if let Some(on) = self.snapshot.airplane.and_then(Mode::flipped) {
                        self.send(nm::Cmd::Airplane(on));
                    }
                }
                tray::TrayMsg::AirplaneNote => {}
                tray::TrayMsg::Hotspot => {
                    let radio = tray::wifi_device(&self.snapshot)
                        .map(|d| (d.path.clone(), d.hotspot.is_some()));
                    match radio {
                        Some((p, true)) => self.send(nm::Cmd::StopHotspot(p)),
                        Some((p, false)) => {
                            self.open_hotspot(&p);
                            shown = Some(HOTSPOT);
                        }
                        None => {}
                    }
                }
                tray::TrayMsg::SignIn => {
                    let token = self.tray.as_ref().and_then(|t| t.take_activation_token());
                    self.sign_in(token);
                }
            }
        }
        Reply {
            activate: reprisetray::handoff_for(self.tray.as_ref(), raised, shown),
            raise,
            ..Reply::redraw_all()
        }
    }

    fn quit(&mut self) -> Reply {
        if let Some(h) = &self.nm {
            h.quit();
        }
        Reply {
            quit: true,
            ..Reply::none()
        }
    }
    fn folder_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = match ev {
            Event::CloseRequest => {
                // Hiding is only a way back if the tray icon is there to
                // bring the window up again; without it, close means quit.
                if self.tray.is_none() {
                    return self.quit();
                }
                self.hidden = true;
                return Reply::redraw();
            }
            Event::Resized => {
                self.refit = true;
                return Reply::redraw();
            }
            Event::Key(k) => self.folder.key(&k, &self.snapshot, ctx),
            Event::Pointer(PointerInput::Press {
                x, y, button, time, ..
            }) => {
                let at = ctx.theme.metrics.click(x, y, time);
                self.folder.press(at, button, &self.snapshot, ctx)
            }
            Event::Pointer(PointerInput::Motion { x, y }) => self.folder.motion(x, y, ctx),
            Event::Pointer(PointerInput::Release { .. }) => self.folder.release(ctx),
            Event::Pointer(PointerInput::Leave) => self.folder.leave(ctx),
            Event::Pointer(PointerInput::Scroll { rows, .. }) => self.folder.wheel(rows, ctx),
            Event::Pointer(PointerInput::ScrollAcross { cols, .. }) => {
                self.folder.wheel_across(cols, ctx)
            }
            _ => return Reply::none(),
        };
        self.folder_outcome(out, ctx)
    }

    fn folder_outcome(&mut self, out: folder::Outcome, ctx: &Ctx) -> Reply {
        match out {
            folder::Outcome::None => Reply::none(),
            folder::Outcome::Redraw => Reply::redraw(),
            folder::Outcome::OpenStatus(p) => Reply {
                raise: self.open_status(&p),
                ..Reply::redraw_all()
            },
            folder::Outcome::ViewWireless(p) => Reply {
                raise: self.open_chooser(&p),
                ..Reply::redraw_all()
            },
            folder::Outcome::Toggle(p) => {
                self.toggle(&p);
                Reply::redraw()
            }
            folder::Outcome::Repair(p) => {
                self.send(nm::Cmd::Repair(p));
                Reply::redraw()
            }
            folder::Outcome::VpnUp(p) => {
                self.send(nm::Cmd::VpnUp(p));
                Reply::redraw()
            }
            folder::Outcome::VpnDown(p) => {
                self.send(nm::Cmd::VpnDown(p));
                Reply::redraw()
            }
            folder::Outcome::VpnProperties(p) => {
                self.send(nm::Cmd::LoadVpn(p));
                Reply::redraw()
            }
            folder::Outcome::VpnDelete(p) => {
                self.ask_delete(&p);
                Reply::redraw_all()
            }
            folder::Outcome::NewWireGuard => {
                let fresh = vpnsheet::WgSheet::fresh(&self.snapshot.vpns);
                Reply {
                    raise: self.show_vpn(VpnWin::WireGuard(fresh)),
                    ..Reply::redraw_all()
                }
            }
            folder::Outcome::Import => {
                self.open_import(ctx.forms().text);
                Reply::redraw_all()
            }
            folder::Outcome::CreateHotspot(p) => Reply {
                raise: self.open_hotspot(&p),
                ..Reply::redraw_all()
            },
            folder::Outcome::StopHotspot(p) => {
                self.send(nm::Cmd::StopHotspot(p));
                Reply::redraw()
            }
        }
    }

    fn vpn_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(w) = self.vpn.as_mut() else {
                return Reply::none();
            };
            let ev = match ev {
                Event::CloseRequest => propsheet::Event::Cancel,
                Event::Pasted(text) => w.sheet_mut().paste(text),
                ev => match sheet_input(w.sheet_mut(), &ev, ctx) {
                    Some(ev) => ev,
                    None => return Reply::none(),
                },
            };
            match w {
                VpnWin::WireGuard(s) => s.event(ev),
                VpnWin::OpenVpn(s) => s.event(ev),
                VpnWin::Plugin(s) => s.event(ev),
            }
        };
        match out {
            vpnsheet::Outcome::None => Reply::none(),
            vpnsheet::Outcome::Redraw => Reply::redraw(),
            vpnsheet::Outcome::Close => {
                self.vpn = None;
                Reply::redraw_all()
            }
            vpnsheet::Outcome::Problem(text) => {
                let title = self.vpn.as_ref().map(VpnWin::title).unwrap_or_default();
                self.tell_from_sheet(&title, &text);
                Reply::redraw_all()
            }
            vpnsheet::Outcome::Save(cfg) => {
                let conn_path = match self.vpn.take() {
                    Some(VpnWin::WireGuard(s)) => s.conn_path,
                    _ => None,
                };
                self.send(nm::Cmd::SaveWireGuard { conn_path, cfg });
                Reply::redraw_all()
            }
            vpnsheet::Outcome::SavePlugin { name, data } => {
                if let Some(VpnWin::Plugin(s)) = self.vpn.take() {
                    self.send(nm::Cmd::SavePluginVpn {
                        conn_path: s.conn_path,
                        name,
                        data,
                        secrets: vec![],
                    });
                }
                Reply::redraw_all()
            }
            vpnsheet::Outcome::SaveOpenVpn(cfg) => {
                if let Some(VpnWin::OpenVpn(s)) = self.vpn.take() {
                    self.send(nm::Cmd::SavePluginVpn {
                        conn_path: s.conn_path,
                        name: cfg.name.clone(),
                        data: ovpn::to_data(&cfg),
                        secrets: ovpn::secrets(&cfg),
                    });
                }
                Reply::redraw_all()
            }
            vpnsheet::Outcome::Browse(field) => {
                let sheet = self.vpn_opened;
                self.open_browse(Browse::Ovpn { field, sheet }, ctx.forms().text);
                Reply::redraw_all()
            }
        }
    }

    fn hotspot_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.hotspot.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => hotspot::Outcome::Cancel,
                Event::Pasted(text) => {
                    let ev = d.form.paste(text);
                    d.event(ev)
                }
                ev => match form_input(&mut d.form, &ev, ctx) {
                    Some(ev) => d.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        match out {
            hotspot::Outcome::None => Reply::none(),
            hotspot::Outcome::Redraw => Reply::redraw(),
            hotspot::Outcome::Cancel => {
                self.hotspot = None;
                Reply::redraw_all()
            }
            hotspot::Outcome::Start {
                ssid,
                psk,
                wpa3,
                band,
            } => {
                if let Some(d) = self.hotspot.take() {
                    self.send(nm::Cmd::Hotspot {
                        device: d.device,
                        ssid,
                        psk,
                        wpa3,
                        band,
                    });
                }
                Reply::redraw_all()
            }
        }
    }

    fn import_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(d) = &mut self.import else {
            return Reply::none();
        };
        let (text, forms) = (ctx.text.clone(), ctx.forms());
        let picked = match self.import_host.drive(d, &ev, &text, &forms, ctx) {
            Driven::Reply(r) => return *r,
            Driven::Cancelled => None,
            Driven::Picked(path) => Some(path),
        };
        Reply {
            raise: self.picked(picked),
            ..Reply::redraw_all()
        }
    }

    /// The file dialog closed on `picked`: a Browse... fills its field in the sheet or dialog
    /// that asked, Import imports. The window to raise comes back, as from [`NetApp::import_file`].
    fn picked(&mut self, picked: Option<PathBuf>) -> Option<WinId> {
        self.import = None;
        match (picked, self.browse_for.take()) {
            (Some(path), Some(Browse::Ovpn { field, sheet })) => {
                if let Some(VpnWin::OpenVpn(s)) = &mut self.vpn
                    && sheet == self.vpn_opened
                {
                    s.set_path(field, &path.to_string_lossy());
                }
            }
            (Some(path), Some(Browse::Eap(field))) => {
                if let Some(d) = &mut self.eap {
                    d.set_path(field, &path);
                }
            }
            (Some(path), None) => return self.import_file(path),
            (None, _) => {}
        }
        None
    }

    fn notice_event(&mut self, win: WinId, ev: Event) -> Reply {
        let Some(text) = &self.dialog_text else {
            return Reply::none();
        };
        match self.notices.event(win, &ev, text, &self.forms) {
            Answer::Closed(Notice::Delete(conn), Some(0)) => {
                self.send(nm::Cmd::DeleteVpn(conn));
                Reply::redraw_all()
            }
            Answer::Closed(..) => Reply::redraw_all(),
            Answer::Redraw => Reply::redraw(),
            Answer::None => Reply::none(),
        }
    }

    fn chooser_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(c) = self.chooser.as_mut() else {
                return Reply::none();
            };
            c.units = ctx.theme.text;
            match ev {
                Event::CloseRequest => chooser::Outcome::Close,
                Event::Key(k) => c.key(&k, &ctx.theme.controls()),
                Event::Pointer(PointerInput::Press {
                    x, y, button, time, ..
                }) => c.press(ctx.theme.metrics.click(x, y, time), button, ctx),
                Event::Pointer(PointerInput::Motion { x, y }) => c.motion(x, y, ctx),
                Event::Pointer(PointerInput::Release { .. }) => c.release(ctx),
                Event::Pointer(PointerInput::Leave) => c.leave(ctx),
                Event::Pointer(PointerInput::Scroll { rows, .. }) => c.scroll(rows),
                _ => return Reply::none(),
            }
        };
        self.chooser_outcome(out)
    }

    fn chooser_outcome(&mut self, out: chooser::Outcome) -> Reply {
        let device = match &self.chooser {
            Some(c) => c.device_path.clone(),
            None => return Reply::none(),
        };
        match out {
            chooser::Outcome::None => Reply::none(),
            chooser::Outcome::Redraw => Reply::redraw(),
            chooser::Outcome::Close => {
                self.close_chooser();
                Reply::redraw_all()
            }
            chooser::Outcome::Rescan => {
                self.send(nm::Cmd::Rescan(device));
                Reply::redraw()
            }
            chooser::Outcome::Disconnect => {
                self.send(nm::Cmd::Disconnect(device));
                Reply::redraw()
            }
            chooser::Outcome::Connect {
                ap,
                ssid,
                security: nm::Security::Enterprise,
                needs_key: true,
            } => {
                self.eap = Some(keydlg::eap::EapDialog::new(&device, &ssid, &ap));
                Reply::redraw_all()
            }
            chooser::Outcome::Connect {
                ap,
                ssid,
                security,
                needs_key: true,
            } => {
                self.key = Some(keydlg::KeyDialog::new(&device, &ssid, &ap, security));
                Reply::redraw_all()
            }
            chooser::Outcome::Connect { ap, ssid, .. } => {
                self.dial(device, ap, ssid, None, false);
                Reply::redraw()
            }
            chooser::Outcome::NeedsAgent => {
                let title = self
                    .snapshot
                    .devices
                    .iter()
                    .find(|d| d.path == device)
                    .map_or_else(|| "Network Connections".into(), |d| d.shell_name.clone());
                self.tell_over(Over::Chooser(device), &title, nm::WIFI_NEEDS_AGENT);
                Reply::redraw_all()
            }
            chooser::Outcome::Hidden => {
                self.key = Some(keydlg::KeyDialog::hidden(&device));
                Reply::redraw_all()
            }
        }
    }

    fn key_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(k) = self.key.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => keydlg::Outcome::Cancel,
                // `KeyDialog::event` syncs the Connect button on the
                // `Changed` that comes back.
                Event::Pasted(text) => {
                    let ev = k.form.paste(text);
                    k.event(ev)
                }
                ev => match form_input(&mut k.form, &ev, ctx) {
                    Some(ev) => k.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        match out {
            keydlg::Outcome::None => Reply::none(),
            keydlg::Outcome::Redraw => Reply::redraw(),
            keydlg::Outcome::Cancel => {
                self.key = None;
                Reply::redraw_all()
            }
            keydlg::Outcome::Connect(key) => {
                let Some(k) = self.key.take() else {
                    return Reply::redraw_all();
                };
                self.dial(k.device.clone(), k.ap.clone(), k.ssid(), key, k.hidden);
                Reply::redraw_all()
            }
            keydlg::Outcome::Enterprise => {
                if let Some(k) = self.key.take() {
                    let mut d = keydlg::eap::EapDialog::new(&k.device, &k.ssid(), &k.ap);
                    d.hidden = k.hidden;
                    self.eap = Some(d);
                }
                Reply::redraw_all()
            }
        }
    }

    fn eap_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.eap.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => keydlg::eap::Outcome::Cancel,
                Event::Pasted(text) => {
                    let ev = d.form.paste(text);
                    d.event(ev)
                }
                ev => match form_input(&mut d.form, &ev, ctx) {
                    Some(ev) => d.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        match out {
            keydlg::eap::Outcome::None => Reply::none(),
            keydlg::eap::Outcome::Redraw => Reply::redraw(),
            keydlg::eap::Outcome::Browse(field) => {
                self.open_browse(Browse::Eap(field), ctx.forms().text);
                Reply::redraw_all()
            }
            keydlg::eap::Outcome::Cancel => {
                self.eap = None;
                Reply::redraw_all()
            }
            keydlg::eap::Outcome::Connect(login) => {
                let Some(d) = self.eap.take() else {
                    return Reply::redraw_all();
                };
                let key = nm::Key::Enterprise(login);
                self.dial(
                    d.device.clone(),
                    d.ap.clone(),
                    d.ssid.clone(),
                    Some(key),
                    d.hidden,
                );
                Reply::redraw_all()
            }
        }
    }

    fn status_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(s) = self.status.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => status::Outcome::Close,
                ev => match sheet_input(&mut s.sheet, &ev, ctx) {
                    Some(ev) => s.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        let Some(device) = self.status.as_ref().map(|s| s.device_path.clone()) else {
            return Reply::none();
        };
        match out {
            status::Outcome::None => Reply::none(),
            status::Outcome::Redraw => Reply::redraw(),
            status::Outcome::Disable => {
                self.toggle(&device);
                Reply::redraw()
            }
            status::Outcome::Repair => {
                self.send(nm::Cmd::Repair(device));
                Reply::redraw()
            }
            status::Outcome::ViewWireless => Reply {
                raise: self.open_chooser(&device),
                ..Reply::redraw_all()
            },
            status::Outcome::Close => {
                self.close_status();
                Reply::redraw_all()
            }
        }
    }

    /// Dial `ssid` and leave the chooser saying so until the device
    /// settles.
    fn dial(
        &mut self,
        device: String,
        ap: String,
        ssid: nm::Ssid,
        key: Option<nm::Key>,
        hidden: bool,
    ) {
        if let Some(c) = &mut self.chooser {
            c.connecting = Some(ssid.clone());
        }
        if key.is_some() && self.refused.remove(&ssid) {
            self.sync_refused();
        }
        self.send(nm::Cmd::Connect {
            device,
            ap,
            ssid,
            key,
            hidden,
        });
    }

    /// Hand the folder its VPN icon on the first event with a [`Ctx`],
    /// which `Event::Wake` does not carry.
    fn read_icons(&mut self, ctx: &mut Ctx) {
        if self.icons_read {
            return;
        }
        self.icons_read = true;
        self.vpn_icon = ctx.icons.get("network-vpn", 16);
        self.folder.refresh(&self.snapshot, self.vpn_icon.clone());
        self.refit = true;
    }
}

impl App for NetApp {
    fn title(&self) -> String {
        "Network Connections".into()
    }

    fn app_id(&self) -> String {
        "reprise.network".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.forms.text.size(folder::CLIENT)
    }

    fn children(&self) -> Vec<Child> {
        let u = self.forms.text;
        let mut out = Vec::new();
        if self.chooser.is_some() {
            out.push(Child {
                id: CHOOSER,
                title: "Wireless Network Connection".into(),
                size: u.size(chooser::CLIENT),
                fixed: true,
                modal: false,
            });
        }
        if let Some(k) = &self.key {
            out.push(Child {
                id: KEY,
                title: keydlg::KeyDialog::window_title(),
                size: u.size(k.client()),
                fixed: true,
                modal: true,
            });
        }
        if self.eap.is_some() {
            out.push(Child {
                id: EAP,
                title: keydlg::eap::EapDialog::window_title(),
                size: u.size(keydlg::eap::CLIENT),
                fixed: true,
                modal: true,
            });
        }
        if let Some(s) = &self.status {
            let title = self
                .snapshot
                .devices
                .iter()
                .find(|d| d.path == s.device_path)
                .map(status::Status::window_title)
                .unwrap_or_else(|| "Status".into());
            out.push(Child {
                id: STATUS,
                title,
                size: u.size(status::CLIENT),
                fixed: true,
                modal: false,
            });
        }
        if let Some(w) = &self.vpn {
            out.push(Child {
                id: VPN,
                title: w.title(),
                size: u.size(w.sheet().size),
                fixed: true,
                modal: false,
            });
        }
        if self.import.is_some() {
            out.push(Child {
                id: IMPORT,
                title: self.file_title().into(),
                size: self.import.as_ref().map_or(filedlg::CLIENT, |d| d.client()),
                fixed: false,
                modal: true,
            });
        }
        if let Some(h) = &self.hotspot {
            out.push(Child {
                id: HOTSPOT,
                title: hotspot::HotspotDialog::window_title(),
                size: u.size(h.client),
                fixed: true,
                modal: false,
            });
        }
        if let Some(text) = &self.dialog_text {
            out.extend(self.notices.child(text, &self.forms));
        }
        out
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        // The tray goes first: it takes the `reprise.network` bus name and a
        // second instance exits here once it has raised the first.
        match tray::start(waker.clone()).or_exit(self.tray_only) {
            Ok(Some((tray, msgs))) => {
                self.tray = Some(tray);
                self.msgs = Some(msgs);
            }
            Ok(None) => {}
            Err(code) => return repriseapp::Start::Exit(code),
        }
        self.waker = Some(waker.clone());
        let station_waker = waker.clone();
        stations::watch(move || station_waker.wake());
        let (rx, handle) = nm::watch(waker);
        self.nm_rx = Some(rx);
        self.nm = Some(handle);
        self.dialog_text = Some(repriseapp::start_text());
        repriseapp::Start::Continue
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.dialog_text = Some(ctx.text.clone());
        self.forms = ctx.forms();
        if matches!(ev, Event::Wake) {
            let reply = self.wake();
            self.withdraw_orphans();
            return reply;
        }
        self.read_icons(ctx);
        self.import_host.icons.prime(&mut ctx.icons);
        let reply = match win {
            CHOOSER => self.chooser_event(ev, ctx),
            KEY => {
                let field = self.key.as_ref().and_then(|k| k.form.focused_edit());
                let clip = form_clipboard(field, &ev);
                self.key_event(ev, ctx).with_clipboard(clip)
            }
            EAP => {
                let field = self.eap.as_ref().and_then(|d| d.form.focused_edit());
                let clip = form_clipboard(field, &ev);
                self.eap_event(ev, ctx).with_clipboard(clip)
            }
            STATUS => self.status_event(ev, ctx),
            VPN => {
                let field = self
                    .vpn
                    .as_ref()
                    .and_then(|w| w.sheet().open().focused_edit());
                let clip = form_clipboard(field, &ev);
                self.vpn_event(ev, ctx).with_clipboard(clip)
            }
            HOTSPOT => {
                let field = self.hotspot.as_ref().and_then(|d| d.form.focused_edit());
                let clip = form_clipboard(field, &ev);
                self.hotspot_event(ev, ctx).with_clipboard(clip)
            }
            IMPORT => self.import_event(ev, ctx),
            w if self.notices.owns(w) => self.notice_event(w, ev),
            _ => self.folder_event(ev, ctx),
        };
        self.withdraw_orphans();
        reply
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.dialog_text = Some(ctx.text.clone());
        let cs = &ctx.theme.colors;
        let forms = ctx.forms();
        let form_px = forms.fonts.size_px;
        match win {
            CHOOSER => {
                if let Some(ch) = &self.chooser {
                    ch.draw(c, ctx);
                }
            }
            KEY => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(k) = &self.key {
                    form::draw(
                        c,
                        &ctx.text,
                        &Paint {
                            s: 1,
                            ..forms.paint()
                        },
                        &k.form,
                        true,
                        form_px,
                    );
                }
            }
            EAP => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(d) = &self.eap {
                    form::draw(
                        c,
                        &ctx.text,
                        &Paint {
                            s: 1,
                            ..forms.paint()
                        },
                        &d.form,
                        true,
                        form_px,
                    );
                }
            }
            STATUS => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(s) = &self.status {
                    propsheet::draw(
                        c,
                        &ctx.text,
                        &Paint {
                            s: 1,
                            ..forms.paint()
                        },
                        &s.sheet,
                        true,
                        form_px,
                    );
                }
            }
            VPN => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(w) = &self.vpn {
                    propsheet::draw(
                        c,
                        &ctx.text,
                        &Paint {
                            s: 1,
                            ..forms.paint()
                        },
                        w.sheet(),
                        true,
                        form_px,
                    );
                }
            }
            HOTSPOT => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(d) = &self.hotspot {
                    form::draw(
                        c,
                        &ctx.text,
                        &Paint {
                            s: 1,
                            ..forms.paint()
                        },
                        &d.form,
                        true,
                        form_px,
                    );
                }
            }
            IMPORT => {
                if let Some(d) = &self.import {
                    d.draw(
                        c,
                        &ctx.text,
                        &Paint {
                            s: 1,
                            ..forms.paint()
                        },
                        Rect::new(0, 0, d.client().0, d.client().1),
                        form_px,
                    );
                }
            }
            w if self.notices.owns(w) => self.notices.draw(c, &forms, &ctx.text),
            // The folder's own size decides its last column's width and
            // whether the selection is still in view, and only its draw
            // has a `Ctx` that means the folder's window.
            _ => {
                if std::mem::take(&mut self.refit) {
                    self.folder.fit_columns(ctx);
                    self.folder.follow(ctx);
                }
                self.folder.draw(c, ctx);
            }
        }
    }
}

impl repriseapp::Extended for NetApp {
    fn hidden(&self) -> bool {
        self.hidden
    }

    /// A box covers the window it was raised from while that one is open: the key and login
    /// dialogs the chooser, a notice its [`Over`], a file dialog [`NetApp::browse_over`].
    fn parent(&self, win: WinId) -> Option<WinId> {
        let over = match win {
            KEY | EAP => self.chooser.as_ref().map(|_| CHOOSER),
            IMPORT => self.browse_over(),
            w if self.notices.owns(w) => match self.notices.up() {
                Some((_, Notice::Tell(over))) => self.window_of(over),
                _ => None,
            },
            _ => None,
        };
        Some(over.unwrap_or(repriseapp::MAIN))
    }

    /// The Import file dialog resizes, down to its opening size.
    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        match &self.import {
            Some(d) if win == IMPORT => Some(d.min_client()),
            _ => None,
        }
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        let form_px = forms.fonts.size_px;
        match win {
            CHOOSER => {
                if let Some(ch) = &self.chooser {
                    ch.access(tree, ctx);
                }
            }
            KEY => {
                if let Some(k) = &self.key {
                    oneshot::access_form(&k.form, tree, &ctx.text, form_px);
                }
            }
            EAP => {
                if let Some(d) = &self.eap {
                    oneshot::access_form(&d.form, tree, &ctx.text, form_px);
                }
            }
            STATUS => {
                if let Some(s) = &self.status {
                    oneshot::access_sheet(&s.sheet, tree, &ctx.text, form_px);
                }
            }
            VPN => {
                if let Some(w) = &self.vpn {
                    oneshot::access_sheet(w.sheet(), tree, &ctx.text, form_px);
                }
            }
            HOTSPOT => {
                if let Some(d) = &self.hotspot {
                    oneshot::access_form(&d.form, tree, &ctx.text, form_px);
                }
            }
            IMPORT => {
                if let Some(d) = &self.import {
                    let r = Rect::new(0, 0, d.client().0, d.client().1);
                    d.access(tree, r, &ctx.text, forms.fonts.size_px);
                }
            }
            w if self.notices.owns(w) => self.notices.access(tree, &ctx.text, &forms),
            _ => self.folder.access(tree, ctx),
        }
    }

    fn popup(&mut self, win: WinId, ctx: &Ctx) -> Option<Box<dyn Hosted + '_>> {
        if win == repriseapp::MAIN {
            return Some(Box::new(&mut self.folder.popup));
        }
        repriseapp::reader_popup(self, win, ctx)
    }

    fn drop_form(&mut self, win: WinId) -> Option<DropForm<'_>> {
        let form = match win {
            VPN => self.vpn.as_mut()?.sheet_mut().open_mut(),
            HOTSPOT => &mut self.hotspot.as_mut()?.form,
            KEY => &mut self.key.as_mut()?.form,
            EAP => &mut self.eap.as_mut()?.form,
            _ => return None,
        };
        Some(DropForm { form, px: None })
    }

    fn drop_file_dialog(&mut self, win: WinId) -> Option<FileDrop<'_>> {
        (win == IMPORT).then_some(FileDrop {
            dlg: self.import.as_mut()?,
            host: &mut self.import_host,
        })
    }
}

/// A notice with one OK button.
fn ok_box(title: &str, text: &str) -> MsgBox {
    MsgBox::new(
        title,
        msgbox::Fields {
            text: wrap(text),
            icon: None,
            buttons: vec!["OK".into()],
            default: 0,
            min_text_h: None,
            min_w: 0,
            max_w: repriseapp::dialog_max_w(),
        },
    )
}

/// A message box's text wrapped to lines the box keeps narrow.
fn wrap(text: &str) -> String {
    reprisekit::text::wrap_chars(text, 60).join("\n")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn hotspot_clients(d: &nm::Device) -> Option<usize> {
    d.hotspot.as_ref().and_then(|_| stations::count(&d.iface))
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(args: Vec<std::ffi::OsString>) {
    repriseapp::applet_main(&args, |tray_only| NetApp {
        hidden: tray_only,
        tray_only,
        forms: repriseapp::start_forms(),
        ..NetApp::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriseapp::{Extended, KeyInput};

    /// A `Ctx` for a window of `size`: the default theme and font, an
    /// icon theme nothing carries.
    pub(crate) fn test_ctx(size: (i32, i32)) -> Ctx {
        Ctx {
            theme: reprisekit::theme::Theme::default(),
            text: reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default())
                .into(),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size,
            maximized: false,
            origin: (0, 0),
            wake: repriseapp::Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            output_size: None,
            at_pick: false,
            text_size: 100,
        }
    }

    #[test]
    fn the_key_field_pastes_and_copies_nothing() {
        let mut ctx = test_ctx(keydlg::CLIENT);
        let mut app = NetApp {
            key: Some(keydlg::KeyDialog::new(
                "/dev/1",
                &"Lab".into(),
                "/ap/7",
                nm::Security::Psk,
            )),
            ..NetApp::default()
        };
        app.key.as_mut().unwrap().form.focus_id("key");
        let ctrl = |ch: u8| {
            Event::Key(KeyInput {
                sym: u32::from(ch),
                utf8: None,
                mods: repriseapp::Mods {
                    ctrl: true,
                    ..Default::default()
                },
                edit: reprisekit::textedit::key_for(u32::from(ch), true, false, None),
            })
        };
        assert!(app.event(KEY, ctrl(b'v'), &mut ctx).paste);
        app.event(KEY, Event::Pasted("hunter22".into()), &mut ctx);
        app.event(KEY, ctrl(b'a'), &mut ctx);
        assert_eq!(app.event(KEY, ctrl(b'c'), &mut ctx).copy, None);
        assert_eq!(app.event(KEY, ctrl(b'x'), &mut ctx).copy, None);
    }

    /// The text of the notice up and its window.
    fn shown(app: &NetApp) -> Option<(String, WinId)> {
        let (b, _) = app.notices.up()?;
        Some((b.fields.text.clone(), app.notices.win()?))
    }

    fn enter() -> Event {
        Event::Key(KeyInput {
            sym: reprisekit::textedit::keysym::RETURN,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: None,
        })
    }

    /// An app with saved VPNs at `paths`, whose commands land on the receiver.
    fn with_vpns(paths: &[&str]) -> (NetApp, async_channel::Receiver<nm::Cmd>) {
        let (nm, sent) = nm::NmHandle::channel();
        let mut app = NetApp {
            nm: Some(nm),
            ..NetApp::default()
        };
        for (i, p) in paths.iter().enumerate() {
            app.snapshot.vpns.push(nm::Vpn {
                conn_path: p.to_string(),
                name: format!("VPN {i}"),
                ..nm::Vpn::default()
            });
        }
        (app, sent)
    }

    #[test]
    fn a_notice_told_while_one_is_open_opens_in_a_fresh_window_when_it_closes() {
        let mut ctx = test_ctx(folder::CLIENT);
        let mut app = NetApp::default();
        app.event(repriseapp::MAIN, Event::Wake, &mut ctx);
        app.tell("Import", "first");
        app.tell("Import", "second");
        let (text, first) = shown(&app).expect("a notice");
        assert_eq!(text, "first");
        app.event(first, Event::CloseRequest, &mut ctx);
        let (text, second) = shown(&app).expect("the queued notice");
        assert_eq!(text, "second");
        assert_ne!(second, first, "a fresh window");
        app.event(second, Event::CloseRequest, &mut ctx);
        assert!(app.notices.is_empty());
    }

    #[test]
    fn a_notice_raised_under_confirm_delete_waits_and_comes_up_after_the_answer() {
        let mut ctx = test_ctx(folder::CLIENT);
        let (mut app, sent) = with_vpns(&["/conn/a"]);
        app.event(repriseapp::MAIN, Event::Wake, &mut ctx);
        app.ask_delete("/conn/a");
        app.tell("Import", "unreadable");
        assert!(matches!(app.notices.up(), Some((_, Notice::Delete(p)))if p == "/conn/a"));
        let (_, win) = shown(&app).unwrap();
        app.event(win, Event::CloseRequest, &mut ctx);
        assert!(
            sent.try_recv().is_err(),
            "closing the question deletes nothing"
        );
        assert_eq!(shown(&app).map(|(t, _)| t).as_deref(), Some("unreadable"));
    }

    #[test]
    fn yes_on_confirm_delete_deletes_the_vpn_it_was_asked_about_with_a_notice_queued_behind() {
        let mut ctx = test_ctx(folder::CLIENT);
        let (mut app, sent) = with_vpns(&["/conn/a", "/conn/b"]);
        app.event(repriseapp::MAIN, Event::Wake, &mut ctx);
        app.ask_delete("/conn/a");
        app.tell("Import", "unreadable");
        let (_, win) = shown(&app).unwrap();
        app.event(win, enter(), &mut ctx);
        assert_eq!(sent.try_recv(), Ok(nm::Cmd::DeleteVpn("/conn/a".into())));
        let (_, win) = shown(&app).expect("the notice is up after the answer");
        app.event(win, enter(), &mut ctx);
        assert!(sent.try_recv().is_err(), "OK on the notice deletes nothing");
    }

    fn ovpn_sheet(conn: &str) -> VpnWin {
        VpnWin::OpenVpn(vpnsheet::OvpnSheet::new(conn, &ovpn::Config::default()))
    }

    fn ca_of(app: &NetApp) -> Option<PathBuf> {
        match &app.vpn {
            Some(VpnWin::OpenVpn(s)) => s.path_of("ca"),
            _ => None,
        }
    }

    #[test]
    fn a_browse_result_lands_in_the_sheet_that_asked() {
        let u = Theme::default().text;
        let mut app = NetApp::default();
        app.show_vpn(ovpn_sheet("/conn/a"));
        let sheet = app.vpn_opened;
        app.open_browse(Browse::Ovpn { field: "ca", sheet }, u);
        app.show_vpn(ovpn_sheet("/conn/b"));
        assert_eq!(app.parent(IMPORT), Some(repriseapp::MAIN));
        app.picked(Some("/certs/other.crt".into()));
        assert_eq!(
            ca_of(&app),
            None,
            "the sheet that replaced it keeps its field"
        );

        let sheet = app.vpn_opened;
        app.open_browse(Browse::Ovpn { field: "ca", sheet }, u);
        assert_eq!(app.parent(IMPORT), Some(VPN), "Browse covers its sheet");
        app.picked(Some("/certs/b.crt".into()));
        assert_eq!(ca_of(&app), Some("/certs/b.crt".into()));
    }

    /// NetworkManager's answer to Properties on OpenVPN connection `conn`, read by a wake.
    fn loaded(app: &mut NetApp, tx: &std::sync::mpsc::Sender<nm::NmMsg>, conn: &str) -> Reply {
        let edit = nm::VpnEdit::Plugin {
            conn_path: conn.into(),
            name: conn.trim_start_matches("/conn/").into(),
            kind: "openvpn".into(),
            data: vec![],
        };
        tx.send(nm::NmMsg::Vpn(edit)).unwrap();
        app.event(repriseapp::MAIN, Event::Wake, &mut test_ctx(folder::CLIENT))
    }

    #[test]
    fn properties_on_another_vpn_replace_an_untouched_sheet_and_bring_an_edited_one_forward() {
        let mut ctx = test_ctx(folder::CLIENT);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = NetApp {
            nm_rx: Some(rx),
            ..NetApp::default()
        };
        let title = |app: &NetApp| app.vpn.as_ref().map(VpnWin::title);
        loaded(&mut app, &tx, "/conn/office");
        assert_eq!(loaded(&mut app, &tx, "/conn/lab").raise, None);
        assert_eq!(title(&app).as_deref(), Some("lab Properties"));

        app.event(VPN, Event::Pasted("2".into()), &mut ctx);
        let edited = title(&app);
        assert_ne!(
            edited.as_deref(),
            Some("lab Properties"),
            "the name took the paste"
        );
        let r = loaded(&mut app, &tx, "/conn/office");
        assert_eq!(r.raise, Some(VPN), "the edited sheet comes forward");
        assert_eq!(title(&app), edited, "with its edit");
    }

    #[test]
    fn a_problem_on_a_vpn_sheet_covers_that_sheet_while_it_is_open() {
        let mut ctx = test_ctx(folder::CLIENT);
        let mut app = NetApp::default();
        app.event(repriseapp::MAIN, Event::Wake, &mut ctx);
        app.show_vpn(ovpn_sheet("/conn/a"));
        app.tell_from_sheet("VPN", "The gateway is missing.");
        let (_, win) = shown(&app).unwrap();
        assert_eq!(app.parent(win), Some(VPN));
        app.show_vpn(ovpn_sheet("/conn/b"));
        assert_eq!(app.parent(win), Some(repriseapp::MAIN));
    }

    #[test]
    fn a_box_raised_from_the_chooser_covers_it_and_leaves_with_it() {
        let mut ctx = test_ctx(folder::CLIENT);
        let mut app = NetApp {
            snapshot: radios(&["/dev/1"]),
            ..NetApp::default()
        };
        app.folder_outcome(folder::Outcome::ViewWireless("/dev/1".into()), &ctx);
        app.chooser_outcome(chooser::Outcome::NeedsAgent);
        let (_, notice) = shown(&app).expect("the agent notice is up");
        assert_eq!(
            app.parent(notice),
            Some(CHOOSER),
            "the notice covers the chooser"
        );
        let connect = |security| chooser::Outcome::Connect {
            ap: "/ap/1".into(),
            ssid: "Lab".into(),
            security,
            needs_key: true,
        };
        app.chooser_outcome(connect(nm::Security::Psk));
        assert_eq!(
            app.parent(KEY),
            Some(CHOOSER),
            "the key prompt covers the chooser"
        );
        app.event(KEY, Event::CloseRequest, &mut ctx);
        app.chooser_outcome(connect(nm::Security::Enterprise));
        assert_eq!(
            app.parent(EAP),
            Some(CHOOSER),
            "the login dialog covers the chooser"
        );
        app.open_browse(Browse::Eap("ca"), ctx.theme.text);
        assert_eq!(
            app.parent(IMPORT),
            Some(EAP),
            "Browse covers the login dialog"
        );

        app.event(CHOOSER, Event::CloseRequest, &mut ctx);
        assert!(app.notices.is_empty(), "the notice left with the chooser");
        assert!(app.eap.is_none(), "the login dialog left with the chooser");
        assert!(app.import.is_none(), "Browse left with the login dialog");
    }

    /// An app with Wi-Fi radios `paths`, whose commands land on the receiver.
    fn with_radios(paths: &[&str]) -> (NetApp, async_channel::Receiver<nm::Cmd>) {
        let (nm, sent) = nm::NmHandle::channel();
        let app = NetApp {
            nm: Some(nm),
            snapshot: radios(paths),
            ..NetApp::default()
        };
        (app, sent)
    }

    fn needs_key(ap: &str, security: nm::Security) -> chooser::Outcome {
        chooser::Outcome::Connect {
            ap: ap.into(),
            ssid: "Lab".into(),
            security,
            needs_key: true,
        }
    }

    fn type_key(app: &mut NetApp, ctx: &mut Ctx, key: &str) {
        for field in ["key", "confirm"] {
            app.key.as_mut().unwrap().form.focus_id(field);
            app.event(KEY, Event::Pasted(key.into()), ctx);
        }
    }

    fn psk_dial(device: &str, ap: &str, key: &str) -> nm::Cmd {
        nm::Cmd::Connect {
            device: device.into(),
            ap: ap.into(),
            ssid: "Lab".into(),
            key: Some(nm::Key::Psk(nm::Psk::new(key))),
            hidden: false,
        }
    }

    #[test]
    fn a_key_prompt_or_login_dialog_closes_unanswered_when_the_chooser_moves_to_another_radio() {
        let mut ctx = test_ctx(folder::CLIENT);
        let (mut app, sent) = with_radios(&["/dev/1", "/dev/2"]);
        let view = |app: &mut NetApp, radio: &str| {
            let out = folder::Outcome::ViewWireless(radio.into());
            app.folder_outcome(out, &test_ctx(folder::CLIENT));
            app.withdraw_orphans();
        };
        view(&mut app, "/dev/1");
        app.chooser_outcome(chooser::Outcome::NeedsAgent);
        app.chooser_outcome(needs_key("/ap/1", nm::Security::Psk));
        type_key(&mut app, &mut ctx, "hunter22");
        view(&mut app, "/dev/2");
        assert!(app.key.is_none(), "the key prompt left with radio 1");
        assert!(app.notices.is_empty(), "so did the agent notice");
        assert!(sent.try_recv().is_err(), "nothing dialled");

        view(&mut app, "/dev/1");
        app.chooser_outcome(needs_key("/ap/1", nm::Security::Enterprise));
        app.open_browse(Browse::Eap("ca"), ctx.theme.text);
        view(&mut app, "/dev/2");
        assert!(app.eap.is_none(), "the login dialog left with radio 1");
        assert!(app.import.is_none(), "Browse left with the login dialog");
        assert!(sent.try_recv().is_err(), "nothing dialled");

        app.chooser_outcome(needs_key("/ap/2", nm::Security::Psk));
        type_key(&mut app, &mut ctx, "radio2key");
        app.event(KEY, enter(), &mut ctx);
        assert_eq!(
            sent.try_recv(),
            Ok(psk_dial("/dev/2", "/ap/2", "radio2key"))
        );
    }

    #[test]
    fn a_key_prompt_answered_on_its_radio_dials_what_it_was_raised_for() {
        let mut ctx = test_ctx(folder::CLIENT);
        let (mut app, sent) = with_radios(&["/dev/1", "/dev/2"]);
        let view = |app: &mut NetApp, radio: &str| {
            let out = folder::Outcome::ViewWireless(radio.into());
            app.folder_outcome(out, &test_ctx(folder::CLIENT)).raise
        };
        view(&mut app, "/dev/1");
        app.chooser_outcome(needs_key("/ap/1", nm::Security::Psk));
        type_key(&mut app, &mut ctx, "hunter22");
        assert_eq!(view(&mut app, "/dev/1"), Some(CHOOSER));
        let typed = app
            .key
            .as_ref()
            .map(|k| k.form.edit_text("confirm").to_string());
        assert_eq!(
            typed.as_deref(),
            Some("hunter22"),
            "the prompt stays with its key"
        );
        app.chooser.as_mut().unwrap().list.select_row(Some(0));
        app.event(KEY, enter(), &mut ctx);
        assert_eq!(sent.try_recv(), Ok(psk_dial("/dev/1", "/ap/1", "hunter22")));
    }

    #[test]
    fn a_notice_or_browse_from_a_vpn_sheet_leaves_when_another_sheet_takes_its_place() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = NetApp {
            nm_rx: Some(rx),
            ..NetApp::default()
        };
        loaded(&mut app, &tx, "/conn/a");
        app.tell_from_sheet("a Properties", "The gateway is missing.");
        let sheet = app.vpn_opened;
        app.open_browse(Browse::Ovpn { field: "ca", sheet }, Theme::default().text);
        loaded(&mut app, &tx, "/conn/b");
        assert!(app.notices.is_empty(), "the notice left with a's sheet");
        assert!(app.import.is_none(), "Browse left with a's sheet");
    }

    #[test]
    fn a_dialog_window_opens_at_the_text_size_of_the_last_event() {
        let mut ctx = test_ctx(folder::CLIENT);
        ctx.text_size = 150;
        let mut app = NetApp::default();
        app.open_hotspot("/dev/1");
        app.event(repriseapp::MAIN, Event::Wake, &mut ctx);
        let child = app
            .children()
            .into_iter()
            .find(|c| c.id == HOTSPOT)
            .unwrap();
        let u = reprisekit::theme::TextUnits::new(17, 11);
        assert_eq!(child.size, u.size(hotspot::CLIENT));
        assert_eq!(child.size, (556, 331));
    }

    #[test]
    fn create_hotspot_on_a_joined_radio_warns_in_a_taller_window() {
        let size = |app: &NetApp| {
            let child = app.children().into_iter().find(|c| c.id == HOTSPOT);
            child.unwrap().size
        };
        let mut bare = NetApp::default();
        bare.open_hotspot("/dev/1");
        let mut app = NetApp::default();
        app.snapshot.devices.push(nm::Device {
            path: "/dev/1".into(),
            kind: nm::Kind::Wifi,
            wifi: Some(nm::Wifi {
                active_ssid: Some("Home".into()),
                ..nm::Wifi::default()
            }),
            ..nm::Device::default()
        });
        app.open_hotspot("/dev/1");
        let warned = app.hotspot.as_ref().unwrap().form.item("warning");
        assert!(warned.is_some(), "the dialog says it leaves Home");
        assert!(
            bare.hotspot
                .as_ref()
                .unwrap()
                .form
                .item("warning")
                .is_none()
        );
        assert!(size(&app).1 > size(&bare).1);
    }

    /// A snapshot with Wi-Fi radios `paths`.
    fn radios(paths: &[&str]) -> nm::Snapshot {
        let radio = |p: &&str| nm::Device {
            path: p.to_string(),
            kind: nm::Kind::Wifi,
            wifi: Some(nm::Wifi::default()),
            managed: true,
            ..nm::Device::default()
        };
        nm::Snapshot {
            devices: paths.iter().map(radio).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn create_a_hotspot_picked_again_replaces_an_untouched_dialog_and_brings_a_typed_one_forward() {
        let mut ctx = test_ctx(folder::CLIENT);
        let handoff = reprisebus::instance::Handoff {
            token: Some("t".into()),
            by_key: false,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = NetApp {
            tray: Some(reprisetray::TrayHandle::holding(handoff.clone())),
            msgs: Some(rx),
            snapshot: radios(&["/dev/1", "/dev/2"]),
            ..NetApp::default()
        };
        let create = |app: &mut NetApp, ctx: &Ctx, radio: &str| {
            let out = folder::Outcome::CreateHotspot(radio.into());
            app.folder_outcome(out, ctx).raise
        };
        let dialog = |app: &NetApp| {
            let d = app.hotspot.as_ref().expect("the dialog");
            (d.device.clone(), d.form.edit_text("ssid").to_string())
        };
        assert_eq!(create(&mut app, &ctx, "/dev/1"), None);
        assert_eq!(create(&mut app, &ctx, "/dev/2"), None);
        assert_eq!(dialog(&app).0, "/dev/2", "an untouched dialog is replaced");

        app.hotspot.as_mut().unwrap().form.focus_id("ssid");
        app.event(HOTSPOT, Event::Pasted("Lab".into()), &mut ctx);
        let typed = dialog(&app);
        assert_ne!(typed.1, hotspot::default_ssid(), "the name took the paste");
        assert_eq!(create(&mut app, &ctx, "/dev/1"), Some(HOTSPOT));
        assert_eq!(dialog(&app), typed, "the typed dialog stays with its name");
        tx.send(reprisetray::Rows::clicked(&tray::TrayMsg::Hotspot))
            .unwrap();
        assert_eq!(app.wake().activate, Some((HOTSPOT, handoff)));
        assert_eq!(dialog(&app), typed, "and so from the tray");
    }

    #[test]
    fn status_and_the_chooser_picked_again_for_their_radio_come_forward_as_they_are() {
        let ctx = test_ctx(folder::CLIENT);
        let mut app = NetApp {
            snapshot: radios(&["/dev/1", "/dev/2"]),
            ..NetApp::default()
        };
        let mut pick = |out| app.folder_outcome(out, &ctx).raise;
        assert_eq!(pick(folder::Outcome::OpenStatus("/dev/1".into())), None);
        assert_eq!(
            pick(folder::Outcome::OpenStatus("/dev/1".into())),
            Some(STATUS)
        );
        assert_eq!(pick(folder::Outcome::ViewWireless("/dev/1".into())), None);
        app.chooser.as_mut().unwrap().connecting = Some("Neighbour".into());
        let mut pick = |out| app.folder_outcome(out, &ctx).raise;
        assert_eq!(
            pick(folder::Outcome::ViewWireless("/dev/1".into())),
            Some(CHOOSER)
        );
        let connecting = app.chooser.as_ref().and_then(|c| c.connecting.clone());
        assert_eq!(connecting, Some("Neighbour".into()), "the chooser stays");
        let mut pick = |out| app.folder_outcome(out, &ctx).raise;
        assert_eq!(pick(folder::Outcome::ViewWireless("/dev/2".into())), None);
        let radio = app.chooser.as_ref().map(|c| c.device_path.clone());
        assert_eq!(
            radio.as_deref(),
            Some("/dev/2"),
            "another radio replaces it"
        );
    }

    #[test]
    fn the_window_opens_at_the_text_size_of_the_last_event() {
        let mut app = NetApp::default();
        assert_eq!(app.initial_size(), folder::CLIENT);
        let mut ctx = test_ctx(folder::CLIENT);
        ctx.text_size = 200;
        app.event(repriseapp::MAIN, Event::Wake, &mut ctx);
        assert_eq!(
            app.initial_size(),
            (2 * folder::CLIENT.0, 2 * folder::CLIENT.1)
        );
    }

    /// A wake that shows no window leaves a second instance's handoff for the wake whose raise does.
    #[test]
    fn the_handoff_waits_for_the_wake_that_shows_the_window() {
        let handoff = reprisebus::instance::Handoff {
            token: Some("t".into()),
            by_key: true,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = NetApp {
            tray: Some(reprisetray::TrayHandle::holding(handoff.clone())),
            msgs: Some(rx),
            hidden: true,
            ..NetApp::default()
        };
        assert_eq!(app.wake().activate, None);
        tx.send(reprisetray::Rows::clicked(&tray::TrayMsg::Folder))
            .unwrap();
        assert_eq!(app.wake().activate, Some((repriseapp::MAIN, handoff)));
    }

    #[test]
    fn a_raise_that_opens_status_hands_its_token_to_the_status_window() {
        let handoff = reprisebus::instance::Handoff {
            token: Some("t".into()),
            by_key: false,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = NetApp {
            tray: Some(reprisetray::TrayHandle::holding(handoff.clone())),
            msgs: Some(rx),
            hidden: true,
            snapshot: nm::Snapshot {
                devices: vec![nm::Device {
                    path: "/dev/1".into(),
                    kind: nm::Kind::Wired,
                    state: nm::State::Connected,
                    managed: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..NetApp::default()
        };
        tx.send(reprisetray::Rows::clicked(&tray::TrayMsg::Status))
            .unwrap();
        assert_eq!(app.wake().activate, Some((STATUS, handoff)));
    }

    #[test]
    fn a_raise_that_shows_nothing_drops_its_handoff() {
        let tray = reprisetray::TrayHandle::holding(reprisebus::instance::Handoff {
            token: Some("t".into()),
            by_key: false,
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = NetApp {
            tray: Some(tray),
            msgs: Some(rx),
            hidden: true,
            snapshot: nm::Snapshot {
                devices: vec![nm::Device {
                    path: "/dev/1".into(),
                    kind: nm::Kind::Wired,
                    state: nm::State::Connected,
                    managed: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..NetApp::default()
        };
        tx.send(reprisetray::Rows::clicked(&tray::TrayMsg::Repair))
            .unwrap();
        assert_eq!(app.wake().activate, None);
        assert_eq!(app.tray.as_ref().unwrap().take_handoff(), None);
    }
}
