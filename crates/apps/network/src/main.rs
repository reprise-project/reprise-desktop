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

mod chooser;
mod folder;
mod hotspot;
mod import;
mod keydlg;
mod nm;
mod ovpn;
mod stations;
mod status;
mod tray;
mod vpnsheet;
mod wg;

use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use repriseapp::filedlg::{Host, Resolved, Step, type_keep};
use repriseapp::keys::{form_clipboard, form_input, sheet_input};
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{App, Child, Ctx, DropForm, Event, FileDrop, PointerInput, Reply, WinId};
use reprisekit::access::Tree;
use reprisekit::art::neticon;
use reprisekit::controls::filedlg::{self, DlgMode, Event as FileEvent, Fields, FileDlg};
use reprisekit::controls::form;
use reprisekit::controls::msgbox;
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::{TextUnits, Theme};

/// The windows the folder and the tray open. One Status sheet and one VPN
/// sheet at a time: opening another replaces it.
const CHOOSER: WinId = "chooser";
const KEY: WinId = "key";
const STATUS: WinId = "status";
const VPN: WinId = "vpn";
const IMPORT: WinId = "import";
const HOTSPOT: WinId = "hotspot";
const NOTICE: WinId = "notice";
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
}

struct Notice {
    msg: MsgBox,
    /// The saved VPN Yes deletes.
    delete: Option<String>,
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
    key: Option<keydlg::KeyDialog>,
    status: Option<status::Status>,
    vpn: Option<VpnWin>,
    import: Option<FileDlg>,
    import_host: Host,
    /// The file dialog is the OpenVPN sheet's Browse... for this field, not Import.
    browse_for: Option<&'static str>,
    hotspot: Option<hotspot::HotspotDialog>,
    notice: Option<Notice>,
    /// The message box's font, loaded in `start`: `children` sizes its window without a `Ctx`.
    dialog_text: Option<TextRenderer>,
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

    /// The Status sheet for `path`, replacing whatever was open. The
    /// packet counters are off until a window asks for them.
    fn open_status(&mut self, path: &str) {
        if self.status.as_ref().is_some_and(|s| s.device_path == path) {
            return;
        }
        let Some(next) = self
            .snapshot
            .devices
            .iter()
            .find(|d| d.path == path)
            .map(status::Status::new)
        else {
            return;
        };
        self.close_status();
        self.status = Some(next);
        self.send(nm::Cmd::Stats {
            device: path.into(),
            on: true,
        });
    }

    /// Drop the Status sheet and turn its device's counters back off. A
    /// device that has gone away takes its statistics interface with it,
    /// so there is nothing left to write to.
    fn close_status(&mut self) {
        let Some(s) = self.status.take() else {
            return;
        };
        if self
            .snapshot
            .devices
            .iter()
            .any(|d| d.path == s.device_path)
        {
            self.send(nm::Cmd::Stats {
                device: s.device_path,
                on: false,
            });
        }
    }

    fn open_chooser(&mut self, path: &str) {
        let found = self
            .snapshot
            .devices
            .iter()
            .find(|d| d.path == path && d.kind == nm::Kind::Wifi)
            .map(|d| chooser::Chooser::new(d, self.forms.text));
        if let Some(c) = found {
            self.chooser = Some(c);
        }
    }

    /// Drop the chooser, and the key dialog that belongs to it.
    fn close_chooser(&mut self) {
        self.chooser = None;
        self.key = None;
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
            log::info!("{}: {}", b.summary, b.body);
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
        if self.notice.is_some() {
            return;
        }
        self.notice = Some(Notice {
            msg: MsgBox::new(
                title,
                msgbox::Fields {
                    text: wrap(text),
                    icon: None,
                    buttons: vec!["OK".into()],
                    default: 0,
                    min_text_h: None,
                    min_w: 0,
                },
            ),
            delete: None,
        });
    }

    fn ask_delete(&mut self, conn_path: &str) {
        let name = self
            .snapshot
            .vpns
            .iter()
            .find(|v| v.conn_path == conn_path)
            .map(|v| v.name.clone())
            .unwrap_or_default();
        self.notice = Some(Notice {
            msg: MsgBox::new(
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
                },
            ),
            delete: Some(conn_path.to_string()),
        });
    }

    fn toggle_vpn(&mut self, i: usize) {
        let Some(v) = self.snapshot.vpns.get(i) else {
            return;
        };
        let cmd = if v.active {
            nm::Cmd::VpnDown(v.conn_path.clone())
        } else {
            nm::Cmd::VpnUp(v.conn_path.clone())
        };
        self.send(cmd);
    }

    fn open_hotspot(&mut self, device: &str) {
        self.hotspot = Some(hotspot::HotspotDialog::new(
            device,
            &hotspot::default_ssid(),
        ));
    }

    fn open_import(&mut self, u: TextUnits) {
        self.browse_for = None;
        self.import = Some(self.import_host.open(
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

    /// The OpenVPN sheet's Browse... for `field`: the file dialog opens where its file is.
    fn open_browse(&mut self, field: &'static str, u: TextUnits) {
        let Some(VpnWin::OpenVpn(s)) = &self.vpn else {
            return;
        };
        let start = s
            .path_of(field)
            .and_then(|p| p.parent().map(PathBuf::from))
            .filter(|d| d.is_dir())
            .unwrap_or_else(home_dir);
        self.browse_for = Some(field);
        self.import = Some(self.import_host.open(
            &start,
            Fields {
                title: BROWSE_TITLE.into(),
                accept_label: "Open".into(),
                mode: DlgMode::Open,
                multiple: false,
                filters: vec![
                    "Certificates and Keys (*.crt;*.pem;*.key)".into(),
                    "All Files (*.*)".into(),
                ],
            },
            u,
        ));
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
    fn import_file(&mut self, path: PathBuf) {
        let text = match std::fs::read(&path) {
            Ok(bytes) => String::from_utf8(bytes).ok(),
            Err(e) => {
                self.tell(
                    "Import",
                    &format!("{} could not be read: {e}.", path.display()),
                );
                return;
            }
        };
        match import::plan(&path, text.as_deref(), &import::installed_plugins()) {
            Ok(import::Plan::WireGuard(cfg)) => {
                self.vpn = Some(VpnWin::WireGuard(vpnsheet::WgSheet::new(&cfg, None)));
            }
            Ok(import::Plan::Plugins(kinds)) => self.send(nm::Cmd::Import { path, kinds }),
            Err(why) => self.tell("Import", &why),
        }
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
        let mut latest = None;
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
                nm::NmMsg::Vpn(edit) => {
                    self.vpn = Some(match edit {
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
                    });
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
        while let Some(raw) = self.msgs.as_ref().and_then(|rx| rx.try_recv().ok()) {
            let Some(msg) = tray::translate(raw) else {
                continue;
            };
            match msg {
                tray::TrayMsg::Exit => return self.quit(),
                tray::TrayMsg::Folder => self.hidden = false,
                tray::TrayMsg::Status => {
                    if let Some(p) = self.primary_path() {
                        self.open_status(&p);
                    }
                }
                tray::TrayMsg::Wireless => {
                    if let Some(p) = self.wifi_path() {
                        self.open_chooser(&p);
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
                tray::TrayMsg::Hotspot => {
                    let radio = tray::wifi_device(&self.snapshot)
                        .map(|d| (d.path.clone(), d.hotspot.is_some()));
                    match radio {
                        Some((p, true)) => self.send(nm::Cmd::Disconnect(p)),
                        Some((p, false)) => self.open_hotspot(&p),
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
            activate: self.tray.as_ref().and_then(|t| t.take_handoff()),
            ..Reply::redraw_all()
        }
    }

    /// Stop the program once the thread has turned an open Status sheet's
    /// counters back off.
    fn quit(&mut self) -> Reply {
        self.close_status();
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
            _ => return Reply::none(),
        };
        match out {
            folder::Outcome::None => Reply::none(),
            folder::Outcome::Redraw => Reply::redraw(),
            folder::Outcome::OpenStatus(p) => {
                self.open_status(&p);
                Reply::redraw_all()
            }
            folder::Outcome::ViewWireless(p) => {
                self.open_chooser(&p);
                Reply::redraw_all()
            }
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
                self.vpn = Some(VpnWin::WireGuard(vpnsheet::WgSheet::fresh(
                    &self.snapshot.vpns,
                )));
                Reply::redraw_all()
            }
            folder::Outcome::Import => {
                self.open_import(ctx.forms().text);
                Reply::redraw_all()
            }
            folder::Outcome::CreateHotspot(p) => {
                self.open_hotspot(&p);
                Reply::redraw_all()
            }
            folder::Outcome::StopHotspot(p) => {
                self.send(nm::Cmd::Disconnect(p));
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
                self.tell(&title, &text);
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
                self.open_browse(field, ctx.forms().text);
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
        let host = &mut self.import_host;
        let keep = type_keep(d);
        host.ensure_listed(d, &mut ctx.icons, &keep);
        let picked = match host.event(
            d,
            &ev,
            &ctx.text,
            ctx.forms().fonts.size_px,
            ctx.size,
            &ctx.theme.metrics,
        ) {
            Step::Closed | Step::Out(FileEvent::Cancel) => None,
            Step::None => return Reply::none(),
            Step::Redraw => return Reply::redraw(),
            Step::Resize => return Reply::resize(),
            Step::Out(FileEvent::NeedListing(id)) => {
                host.list(d, id, &mut ctx.icons, &keep);
                return Reply::redraw();
            }
            Step::Out(FileEvent::FilterChanged(_)) => {
                let keep = type_keep(d);
                if let Some(id) = host.folder {
                    host.list(d, id, &mut ctx.icons, &keep);
                }
                return Reply::redraw();
            }
            Step::Out(FileEvent::Accept(names)) => {
                let Some(name) = names.into_iter().next() else {
                    return Reply::none();
                };
                match host.resolve(&name) {
                    Ok(Resolved::Folder(id, _)) => {
                        host.list(d, id, &mut ctx.icons, &keep);
                        d.clear_file_name();
                        return Reply::redraw();
                    }
                    Ok(Resolved::File(path)) => Some(Ok(path)),
                    Err(msg) => Some(Err(msg)),
                }
            }
            Step::Out(_) => return Reply::redraw(),
        };
        self.import = None;
        let title = self.file_title();
        match (picked, self.browse_for.take()) {
            (Some(Ok(path)), Some(field)) => {
                if let Some(VpnWin::OpenVpn(s)) = &mut self.vpn {
                    s.set_path(field, &path.to_string_lossy());
                }
            }
            (Some(Ok(path)), None) => self.import_file(path),
            (Some(Err(msg)), _) => self.tell(title, &msg),
            (None, _) => {}
        }
        Reply::redraw_all()
    }

    fn notice_event(&mut self, ev: Event) -> Reply {
        let Some(text) = &self.dialog_text else {
            return Reply::none();
        };
        let Some(n) = &mut self.notice else {
            return Reply::none();
        };
        match n.msg.event(&ev, text, &self.forms) {
            oneshot::Outcome::Closed(button) => {
                if let (Some(conn), Some(0)) = (n.delete.take(), button) {
                    self.send(nm::Cmd::DeleteVpn(conn));
                }
                self.notice = None;
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
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
                security,
                needs_key: true,
            } => {
                self.key = Some(keydlg::KeyDialog::new(&ssid, &ap, security));
                Reply::redraw_all()
            }
            chooser::Outcome::Connect { ap, ssid, .. } => {
                self.dial(device, ap, ssid, None);
                Reply::redraw()
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
            keydlg::Outcome::Connect(psk) => {
                let Some(k) = self.key.take() else {
                    return Reply::redraw_all();
                };
                let Some(device) = self.chooser.as_ref().map(|c| c.device_path.clone()) else {
                    return Reply::redraw_all();
                };
                self.dial(device, k.ap.clone(), k.ssid.clone(), Some(psk));
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
            status::Outcome::ViewWireless => {
                self.open_chooser(&device);
                Reply::redraw_all()
            }
            status::Outcome::Close => {
                self.close_status();
                Reply::redraw_all()
            }
        }
    }

    /// Dial `ssid` and leave the chooser saying so until the device
    /// settles.
    fn dial(&mut self, device: String, ap: String, ssid: nm::Ssid, psk: Option<nm::Psk>) {
        if let Some(c) = &mut self.chooser {
            c.connecting = Some(ssid.clone());
        }
        self.send(nm::Cmd::Connect {
            device,
            ap,
            ssid,
            psk,
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

    fn text_sized(&self) -> bool {
        true
    }

    fn hidden(&self) -> bool {
        self.hidden
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
        if self.key.is_some() {
            out.push(Child {
                id: KEY,
                title: keydlg::KeyDialog::window_title(),
                size: u.size(keydlg::CLIENT),
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
        if self.hotspot.is_some() {
            out.push(Child {
                id: HOTSPOT,
                title: hotspot::HotspotDialog::window_title(),
                size: u.size(hotspot::CLIENT),
                fixed: true,
                modal: false,
            });
        }
        if let (Some(n), Some(text)) = (&self.notice, &self.dialog_text) {
            out.push(n.msg.child(NOTICE, text, &self.forms));
        }
        out
    }

    /// The Import file dialog resizes, down to its opening size.
    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        match &self.import {
            Some(d) if win == IMPORT => Some(d.min_client()),
            _ => None,
        }
    }

    fn start(&mut self, waker: repriseapp::Waker) {
        // The tray goes first: it takes the `reprise.network` bus name and a
        // second instance exits here once it has raised the first.
        match tray::start(waker.clone()) {
            reprisetray::Started::Running(tray, msgs) => {
                self.tray = Some(tray);
                self.msgs = Some(msgs);
            }
            reprisetray::Started::AlreadyRunning => std::process::exit(0),
            reprisetray::Started::Unavailable if self.tray_only => {
                log::error!(
                    "--tray was given but the tray item could not start (no session bus?); exiting"
                );
                std::process::exit(1);
            }
            reprisetray::Started::Unavailable => {}
        }
        let station_waker = waker.clone();
        stations::watch(move || station_waker.wake());
        let (rx, handle) = nm::watch(waker);
        self.nm_rx = Some(rx);
        self.nm = Some(handle);
        self.dialog_text = Some(TextRenderer::load(&repriseapp::start_fonts()));
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.forms = ctx.forms();
        if matches!(ev, Event::Wake) {
            return self.wake();
        }
        self.read_icons(ctx);
        self.import_host.icons.prime(&mut ctx.icons);
        match win {
            CHOOSER => self.chooser_event(ev, ctx),
            KEY => {
                let field = self.key.as_ref().and_then(|k| k.form.focused_edit());
                let clip = form_clipboard(field, &ev);
                self.key_event(ev, ctx).with_clipboard(clip)
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
            NOTICE => self.notice_event(ev),
            _ => self.folder_event(ev, ctx),
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
            NOTICE => {
                if let Some(n) = &self.notice {
                    n.msg.access(tree, &ctx.text, &forms);
                }
            }
            _ => self.folder.access(tree, ctx),
        }
    }

    fn drop_form(&mut self, win: WinId) -> Option<DropForm<'_>> {
        let form = match win {
            VPN => self.vpn.as_mut()?.sheet_mut().open_mut(),
            HOTSPOT => &mut self.hotspot.as_mut()?.form,
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

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
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
            NOTICE => {
                if let Some(n) = &self.notice {
                    n.msg.draw(c, &forms, &ctx.text);
                }
            }
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

fn main() {
    reprisebus::journal::init_logger("error");
    let tray_only = std::env::args().any(|a| a == "--tray");
    repriseapp::run(Box::new(NetApp {
        hidden: tray_only,
        tray_only,
        forms: repriseapp::start_forms(),
        ..NetApp::default()
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriseapp::KeyInput;

    /// A `Ctx` for a window of `size`: the default theme and font, an
    /// icon theme nothing carries.
    pub(crate) fn test_ctx(size: (i32, i32)) -> Ctx {
        Ctx {
            theme: reprisekit::theme::Theme::default(),
            text: reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default()),
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
            at_pick: false,
            text_size: 100,
        }
    }

    #[test]
    fn the_key_field_pastes_and_copies_nothing() {
        let mut ctx = test_ctx(keydlg::CLIENT);
        let mut app = NetApp {
            key: Some(keydlg::KeyDialog::new(
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
}
