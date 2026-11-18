//! Bluetooth Devices: the Reprise Bluetooth icon and its windows.
//!
//! One process: a tray icon following BlueZ, the Devices sheet, the Add
//! wizard, device Properties, the Pairing Request box, and the prompt for a
//! file a paired device sends. `--tray` (passed by
//! the autostart entry) starts with the sheet hidden. While the tray icon is
//! up, closing the sheet hides it and only Exit in the tray menu quits.
//!
//! Glue only: the bus thread hands over snapshots and agent requests, the
//! window modules own the models, and this file turns outcomes into
//! `bz::Cmd`s and windows.

#[cfg(test)]
reprisebus::test_session!();

mod addwiz;
mod battery;
mod bz;
mod devices;
mod icons;
mod pairdlg;
mod passkey;
mod props;
mod receive;
mod tray;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Receiver;

use repriseapp::asks::{Answer, Asks};
use repriseapp::keys::{dialog_key, form_clipboard, form_input, sheet_input};
use repriseapp::oneshot::{self, ErrorBox, MsgBox};
use repriseapp::{App, BTN_LEFT, Child, Ctx, DropForm, Event, PointerInput, Reply, WinId};
use reprisebus::airplane::Mode;
use reprisekit::access::Tree;
use reprisekit::controls::msgbox::Fields;
use reprisekit::controls::{form, propsheet, wizard};
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::textedit::keysym;
use reprisekit::theme::Theme;

/// Child windows; one of each at a time.
const WIZARD: WinId = WinId::named("wizard");
const PROPS: WinId = WinId::named("props");
const PAIR: WinId = WinId::named("pair");
const REMOVE: WinId = WinId::named("remove");

const OFFER: WinId = WinId::named("offer");

struct BtApp {
    tray: Option<reprisetray::TrayHandle>,
    msgs: Option<Receiver<reprisetray::Msg>>,
    bz_rx: Option<Receiver<bz::BzMsg>>,
    bz: Option<bz::BzHandle>,
    snapshot: bz::Snapshot,
    tray_state: Option<reprisetray::Shown>,
    devices: devices::Devices,
    wizard: Option<addwiz::AddWizard>,
    props: Option<props::PropsSheet>,
    pair: Option<pairdlg::PairDialog>,
    /// Requests that came while the Pairing Request box was open, shown one after another.
    pair_queue: VecDeque<bz::AgentRequest>,
    /// Confirm Device Removal, with the device's path.
    remove: Asks<String>,
    error: ErrorBox,
    /// The file offer on screen, and the ones that came while it was.
    offer: Option<(bz::Offer, MsgBox)>,
    offer_queue: VecDeque<bz::Offer>,
    /// Received files by their balloon's tag, for its Open and Show in folder.
    received: HashMap<u32, PathBuf>,
    next_tag: u32,
    icons: icons::Icons,
    /// Devices with an unfinished Connect not yet shown connected; no second Connect goes out.
    connecting: HashSet<String>,
    /// Devices disconnected since their last Connect; a Connect failing for one shows no box.
    disconnected: HashSet<String>,
    /// A device per SetAlias or SetTrusted not yet answered; its sheet waits on them.
    setting: Vec<String>,
    battery: battery::Warner,
    /// Dialog renderer, loaded in `start`: `children` and the wake have no `Ctx` to measure with.
    text: Option<Rc<TextRenderer>>,
    /// The last event's [`Ctx::forms`], which dialogs are built and sized with.
    forms: Theme,
    /// The client size last asked for the main window, which grows with the text size.
    asked: (i32, i32),
    hidden: bool,
    tray_only: bool,
}

impl BtApp {
    fn new(tray_only: bool) -> BtApp {
        BtApp {
            tray: None,
            msgs: None,
            bz_rx: None,
            bz: None,
            snapshot: bz::Snapshot::default(),
            tray_state: None,
            devices: devices::Devices::new(
                repriseconf::current().bluetooth.battery_warning,
                repriseconf::current().bluetooth.receive_files,
            ),
            wizard: None,
            props: None,
            pair: None,
            pair_queue: VecDeque::new(),
            remove: Asks::new(REMOVE),
            error: ErrorBox::default(),
            offer: None,
            offer_queue: VecDeque::new(),
            received: HashMap::new(),
            next_tag: 0,
            icons: icons::Icons::new(),
            connecting: HashSet::new(),
            disconnected: HashSet::new(),
            setting: Vec::new(),
            battery: battery::Warner::default(),
            text: None,
            forms: Theme::default(),
            asked: devices::CLIENT,
            hidden: tray_only,
            tray_only,
        }
    }

    fn fonts(&mut self) {
        self.text = Some(repriseapp::start_text());
        self.forms = Theme::default();
    }

    fn text_forms(&self) -> (Rc<TextRenderer>, Theme) {
        let text = self.text.clone().expect("fonts are loaded in start");
        (text, self.forms.clone())
    }

    fn send(&self, cmd: bz::Cmd) {
        if let Some(h) = &self.bz {
            h.send(cmd);
        }
    }

    fn adapter(&self) -> Option<String> {
        self.snapshot.adapter.as_ref().map(|a| a.path.clone())
    }

    fn device(&self, path: &str) -> Option<&bz::Device> {
        self.snapshot.devices.iter().find(|d| d.path == path)
    }

    fn push_state(&mut self) {
        let next = tray::shown(&self.snapshot);
        if self.tray_state.as_ref() == Some(&next) {
            return;
        }
        if let Some(t) = &self.tray {
            t.set_state(next.clone());
        }
        self.tray_state = Some(next);
    }

    /// The key is read only while some device reports a battery, so a change applies to the next
    /// reading.
    fn warn_battery(&mut self) {
        if !self.snapshot.devices.iter().any(|d| d.battery.is_some()) {
            return;
        }
        let level = repriseconf::current().bluetooth.battery_warning;
        for w in self.battery.check(&self.snapshot, level) {
            if let Some(t) = &self.tray {
                t.notify_with_icon(&w.summary, &w.body, w.icon);
            }
        }
    }

    /// Takes snapshot `s`, read once BlueZ had answered `answered` commands.
    fn snapshot_in(&mut self, s: bz::Snapshot, answered: u64) {
        log::debug!(
            "snapshot: adapter {}, {} devices",
            s.adapter.as_ref().map_or("none", |a| a.path.as_str()),
            s.devices.len()
        );
        self.snapshot = s;
        self.warn_battery();
        self.devices.refresh(&self.snapshot);
        let (text, forms) = self.text_forms();
        if let Some(w) = &mut self.wizard {
            w.refresh(&self.snapshot, &text, &forms);
        }
        // Properties closes once its device is gone or unpaired, or its adapter went away.
        let live = self.props.as_ref().and_then(|p| {
            self.snapshot
                .adapter
                .as_ref()
                .and(self.device(&p.path))
                .filter(|d| d.paired)
                .cloned()
        });
        match live {
            Some(d) => {
                if let Some(p) = &mut self.props {
                    p.refresh(&d, answered);
                }
            }
            None => self.props = None,
        }
        // Dropping the box answers its request Canceled.
        if self.snapshot.adapter.is_none() {
            self.pair = None;
            self.pair_queue.clear();
        }
        let devs = &self.snapshot.devices;
        let lost = self.snapshot.adapter.is_none();
        self.remove
            .withdraw(|p| lost || !devs.iter().any(|d| d.path == *p));
        self.connecting
            .retain(|p| devs.iter().any(|d| d.path == *p && !d.connected));
        self.disconnected
            .retain(|p| devs.iter().any(|d| d.path == *p));
        self.push_state();
    }

    /// The wizard takes the request for the device it is adding; any other opens the Pairing
    /// Request box, or waits for the open one to close.
    fn agent(&mut self, req: bz::AgentRequest) {
        let (text, forms) = self.text_forms();
        let req = match &mut self.wizard {
            Some(w) => match w.agent(req, &text, &forms) {
                Ok(()) => return,
                Err(r) => r,
            },
            None => req,
        };
        if self.pair.is_some() {
            self.pair_queue.push_back(req);
        } else {
            self.open_pair(req);
        }
    }

    /// The Pairing Request box for `req`, naming the device by alias and address. A Connect the
    /// user started from Devices counts as a pairing started here.
    fn open_pair(&mut self, req: bz::AgentRequest) {
        let address = {
            let last = req.device.rsplit('/').next().unwrap_or_default();
            last.strip_prefix("dev_").unwrap_or(last).replace('_', ":")
        };
        let name = match self.device(&req.device) {
            Some(d) if !d.address.is_empty() => bz::label(&d.alias, &d.address),
            Some(d) => bz::label(&d.alias, &address),
            None => address,
        };
        let started = self.connecting.contains(&req.device);
        let (text, forms) = self.text_forms();
        self.pair = Some(pairdlg::PairDialog::new(req, &name, started, &text, &forms));
    }

    /// The next queued request once the box has closed; one BlueZ gave up on is skipped. It
    /// shows in the closed one's window, which gets no new focus, so its consent wait starts now.
    fn next_pair(&mut self) {
        while self.pair.is_none() {
            let Some(req) = self.pair_queue.pop_front() else {
                return;
            };
            if req.done.as_ref().is_some_and(|d| d.is_closed()) {
                continue;
            }
            self.open_pair(req);
        }
        if let Some(d) = &mut self.pair {
            d.form.consent_focused(std::time::Instant::now());
        }
    }

    /// BlueZ cancelled the pending request held by the wizard, the Pairing Request box or the
    /// queue behind it. BlueZ waits on one request at a time, so every waiting one goes; a shown
    /// code is never waited on.
    fn agent_cancel(&mut self) {
        let (text, forms) = self.text_forms();
        if let Some(w) = &mut self.wizard
            && w.agent_cancel(&text, &forms)
        {
            return;
        }
        self.pair_queue.retain(|r| r.done.is_none());
        if self.pair.as_ref().is_some_and(|d| d.req.done.is_some()) {
            self.pair = None;
            self.next_pair();
        }
    }

    /// A Connect finished; a failure gets a box unless the user has since disconnected the device.
    fn connected(&mut self, device: String, error: Option<String>) {
        self.connecting.remove(&device);
        let Some(body) = error else {
            return;
        };
        let cmd = bz::Cmd::Connect(device.clone());
        if self.disconnected.contains(&device) {
            let title = bz::failure_title(&cmd);
            log::info!("{title} {device} after a Disconnect: {body}");
            return;
        }
        self.failed(&cmd, body);
    }

    /// A failure covers the window it is about while that one is open: its device's sheet, the
    /// wizard's search, else the folder.
    fn failed(&mut self, cmd: &bz::Cmd, body: String) {
        let title = bz::failure_title(cmd);
        log::warn!("{title}: {body}");
        if let bz::Cmd::SetAlias { device, .. } | bz::Cmd::SetTrusted { device, .. } = cmd
            && let Some(p) = self.props.as_mut().filter(|p| &p.path == device)
        {
            p.leaving = false;
            p.refused(cmd);
        }
        let sheet_of = |d: &String| self.props.as_ref().is_some_and(|p| &p.path == d);
        let over = match cmd {
            bz::Cmd::Connect(d)
            | bz::Cmd::Disconnect(d)
            | bz::Cmd::SetTrusted { device: d, .. }
            | bz::Cmd::SetAlias { device: d, .. }
                if sheet_of(d) =>
            {
                PROPS
            }
            bz::Cmd::StartDiscovery(_) if self.wizard.is_some() => WIZARD,
            _ => repriseapp::MAIN,
        };
        self.error.show_over(over, title, body, None);
    }

    /// Takes the failures over a sheet or wizard that closed.
    fn withdraw_orphans(&mut self) {
        let (props, wizard) = (self.props.is_some(), self.wizard.is_some());
        self.error
            .withdraw(|w| (w == PROPS && !props) || (w == WIZARD && !wizard));
    }

    /// Drains the bus thread's and tray's channels, and redraws when either had something. Quits
    /// only on the tray's Exit.
    fn wake(&mut self, ctx: &mut Ctx) -> Reply {
        let mut latest = None;
        let mut any = false;
        while let Some(msg) = self.bz_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            any = true;
            match msg {
                bz::BzMsg::Snapshot { snapshot, answered } => latest = Some((snapshot, answered)),
                bz::BzMsg::Failed { cmd, body } => self.failed(&cmd, body),
                bz::BzMsg::Ran { cmd, answer } => self.ran(&cmd, answer),
                bz::BzMsg::Connected { device, error } => self.connected(device, error),
                bz::BzMsg::Paired {
                    device,
                    connect_error,
                } => {
                    let (text, forms) = self.text_forms();
                    if let Some(w) = &mut self.wizard {
                        w.paired(&device, connect_error, &text, &forms);
                    }
                }
                bz::BzMsg::PairFailed { device, reason } => {
                    let (text, forms) = self.text_forms();
                    if let Some(w) = &mut self.wizard {
                        w.pair_failed(&device, reason, &text, &forms);
                    }
                }
                // The request's device may be in this same burst; the box names it by alias.
                bz::BzMsg::Agent(req) => {
                    if let Some((s, answered)) = latest.take() {
                        self.snapshot_in(s, answered);
                    }
                    self.agent(req)
                }
                bz::BzMsg::AgentCancel => self.agent_cancel(),
                bz::BzMsg::Obex(present) => {
                    self.devices.set_obex(present);
                    if present && repriseconf::current().bluetooth.receive_files {
                        self.send(bz::Cmd::Receive(true));
                    }
                }
                bz::BzMsg::Offer(o) => self.offer_in(o, ctx),
                bz::BzMsg::OfferGone(transfer) => self.offer_gone(&transfer, ctx),
                bz::BzMsg::Received { path, sender } => self.file_received(path, sender),
                bz::BzMsg::NotReceived { name, sender } => {
                    let (summary, body) = receive::not_received(&name, &sender);
                    if let Some(t) = &self.tray {
                        t.notify(&summary, &body);
                    }
                }
            }
        }
        if let Some((s, answered)) = latest {
            self.snapshot_in(s, answered);
        }
        let mut shown = None;
        let mut raised = false;
        while let Some(raw) = self.msgs.as_ref().and_then(|rx| rx.try_recv().ok()) {
            let Some(msg) = tray::translate(raw, &self.snapshot) else {
                continue;
            };
            any = true;
            raised = true;
            match msg {
                tray::TrayMsg::Exit => return self.quit(),
                tray::TrayMsg::Show => {
                    self.hidden = false;
                    shown = Some(repriseapp::MAIN);
                }
                tray::TrayMsg::Add => {
                    self.open_wizard();
                    shown = Some(WIZARD);
                }
                tray::TrayMsg::Device(i) => {
                    if let Some(path) = tray::connected(&self.snapshot)
                        .get(i)
                        .map(|d| d.path.clone())
                    {
                        self.open_props(&path);
                        shown = Some(PROPS);
                    }
                }
                tray::TrayMsg::Balloon(tag, i) => {
                    if let Some(path) = self.received.get(&tag) {
                        let open = if i == receive::OPEN {
                            let apps = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
                            let globs = reprisefs::apps::mime_globs();
                            let terminal = repriseconf::current().terminal.clone();
                            let found = reprisefs::props::open_in(path, &globs, &apps, &terminal);
                            receive::open_command(path, found)
                        } else {
                            receive::shown_in_folder(path)
                        };
                        self.spawn_with_token(&open);
                    }
                }
                tray::TrayMsg::Airplane => {
                    if let Some(on) = self.snapshot.airplane.and_then(Mode::flipped) {
                        self.send(bz::Cmd::Airplane(on));
                    }
                }
                tray::TrayMsg::AirplaneNote => {}
                tray::TrayMsg::TogglePower => {
                    if let Some(a) = &self.snapshot.adapter {
                        self.send(bz::Cmd::SetPowered {
                            adapter: a.path.clone(),
                            on: !a.powered,
                        });
                    }
                }
            }
        }
        if !any {
            return Reply::none();
        }
        Reply {
            activate: reprisetray::handoff_for(self.tray.as_ref(), raised, shown),
            ..Reply::redraw_all()
        }
    }

    fn quit(&mut self) -> Reply {
        if let Some(h) = &self.bz {
            h.quit();
        }
        Reply {
            quit: true,
            ..Reply::none()
        }
    }

    /// Hides the sheet if the tray icon can bring it back, else quits.
    fn close_main(&mut self) -> Reply {
        if self.tray.is_none() {
            return self.quit();
        }
        self.hidden = true;
        Reply::redraw()
    }

    fn open_wizard(&mut self) {
        if self.wizard.is_none() {
            let (text, forms) = self.text_forms();
            self.wizard = Some(addwiz::AddWizard::new(&text, &forms));
        }
    }

    /// Opens Properties for `path`, replacing another device's; the same device's sheet stays, edits intact.
    fn open_props(&mut self, path: &str) {
        if self.props.as_ref().is_some_and(|p| p.path == path) {
            return;
        }
        let Some(d) = self.device(path) else {
            return;
        };
        let next = props::PropsSheet::new(d, self.icons.get(d.kind).clone());
        self.props = Some(next);
        self.sync_props_busy();
    }

    /// Disconnects a connected device, else connects it unless a Connect is pending.
    fn toggle(&mut self, path: &str) {
        let Some(connected) = self.device(path).map(|d| d.connected) else {
            return;
        };
        if connected {
            self.disconnected.insert(path.into());
            self.send(bz::Cmd::Disconnect(path.into()));
        } else if self.connecting.insert(path.into()) {
            self.disconnected.remove(path);
            self.send(bz::Cmd::Connect(path.into()));
        }
    }

    fn apply_options(&self, o: devices::Options) {
        if o.battery_warning != repriseconf::current().bluetooth.battery_warning
            && let Err(e) = repriseconf::save_bt_battery_warning(o.battery_warning)
        {
            log::warn!("could not save the battery warning: {e}");
        }
        if o.receive != repriseconf::current().bluetooth.receive_files {
            if let Err(e) = repriseconf::save_bt_receive_files(o.receive) {
                log::warn!("could not save Receive files: {e}");
            }
            self.send(bz::Cmd::Receive(o.receive));
        }
        let Some(a) = &self.snapshot.adapter else {
            return;
        };
        if o.powered != a.powered {
            self.send(bz::Cmd::SetPowered {
                adapter: a.path.clone(),
                on: o.powered,
            });
        }
        if o.discoverable != a.discoverable {
            self.send(bz::Cmd::SetDiscoverable {
                adapter: a.path.clone(),
                on: o.discoverable,
            });
        }
    }

    fn apply_change(&mut self, path: &str, c: props::Change) {
        if let Some(alias) = c.alias {
            self.send(bz::Cmd::SetAlias {
                device: path.into(),
                alias,
            });
            self.setting.push(path.into());
        }
        if let Some(on) = c.trusted {
            self.send(bz::Cmd::SetTrusted {
                device: path.into(),
                on,
            });
            self.setting.push(path.into());
        }
        self.sync_props_busy();
    }

    fn ran(&mut self, cmd: &bz::Cmd, answer: u64) {
        if let bz::Cmd::SetAlias { device, .. } | bz::Cmd::SetTrusted { device, .. } = cmd
            && let Some(i) = self.setting.iter().position(|d| d == device)
        {
            self.setting.remove(i);
            if let Some(p) = self.props.as_mut().filter(|p| &p.path == device) {
                p.answered(answer);
            }
            if self
                .props
                .as_ref()
                .is_some_and(|p| p.leaving && !self.setting.contains(&p.path))
            {
                self.props = None;
            }
            self.sync_props_busy();
        }
    }

    fn sync_props_busy(&mut self) {
        if let Some(p) = &mut self.props {
            p.set_busy(self.setting.contains(&p.path));
        }
    }

    fn ask_remove(&mut self, path: String, ctx: &mut Ctx) {
        let alias = self
            .device(&path)
            .map_or_else(|| path.clone(), |d| d.alias.clone());
        let b = MsgBox::new(
            "Confirm Device Removal",
            Fields {
                text: format!(
                    "Are you sure you want to remove {alias}?\nTo use it again, you will have to add it again."
                ),
                icon: ctx.icons.get("dialog-question", 32),
                buttons: vec!["&Yes".into(), "&No".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        );
        self.remove.ask(b, path);
    }

    /// The Devices list's share of a sheet event; `None` leaves it to the sheet.
    fn devices_list_event(&mut self, ev: &Event, ctx: &Ctx) -> Option<devices::Outcome> {
        let (metrics, cm) = (&ctx.theme.metrics, &ctx.theme.controls());
        let on_devices = self.devices.sheet.tabs.open == devices::TAB_DEVICES;
        let list = self.devices.list_rect();
        let on_list = |x: i32, y: i32| {
            let px = ctx.form_units().px(ctx.base_px());
            on_devices
                && list.contains(x, y)
                && self.devices.sheet.reaches_page(x, y, &ctx.text, px)
        };
        match ev {
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                time,
                ..
            }) => {
                if on_list(*x, *y) {
                    let at = metrics.click(*x, *y, *time);
                    return Some(self.devices.list_press(at, cm));
                }
                self.devices.list_focus = false;
                None
            }
            Event::Pointer(PointerInput::Motion { y, .. }) => self
                .devices
                .list
                .drag(list, *y, cm)
                .then_some(devices::Outcome::Redraw),

            Event::Pointer(PointerInput::Scroll { x, y, rows }) if on_list(*x, *y) => {
                let live = self.devices.blank_text().is_none();
                Some(if live && self.devices.list.scroll(*rows, list, cm.text) {
                    devices::Outcome::Redraw
                } else {
                    devices::Outcome::None
                })
            }
            Event::Key(k) if on_devices => {
                if matches!(k.sym, keysym::DELETE | keysym::KP_DELETE) {
                    return Some(self.devices.delete_key());
                }
                if !self.devices.list_focus {
                    return None;
                }
                match dialog_key(k)? {
                    form::Key::Tab { .. } => {
                        self.devices.list_focus = false;
                        None
                    }
                    key => self.devices.list_key(&key, cm),
                }
            }
            _ => None,
        }
    }

    /// Updates the Devices list's scrollbar highlight for a pointer event. True when a styled look must redraw.
    fn devices_list_light(&mut self, ev: &Event, ctx: &Ctx) -> bool {
        let cm = ctx.theme.controls();
        let rect = self.devices.list_rect();
        let list = &mut self.devices.list;
        let px = ctx.form_units().px(ctx.base_px());
        let on_devices = self.devices.sheet.tabs.open == devices::TAB_DEVICES;
        let changed = match ev {
            Event::Pointer(PointerInput::Motion { x, y })
                if on_devices && self.devices.sheet.reaches_page(*x, *y, &ctx.text, px) =>
            {
                list.hover(rect, *x, *y, &cm)
            }
            Event::Pointer(PointerInput::Motion { .. } | PointerInput::Leave) => list.leave(),
            Event::Pointer(PointerInput::Release {
                button: BTN_LEFT, ..
            }) => list.release(),
            _ => false,
        };
        changed && ctx.styled()
    }

    fn main_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let lit = self.devices_list_light(&ev, ctx);
        let mut reply = self.main_event_inner(ev, ctx);
        reply.redraw |= lit;
        reply
    }

    fn main_event_inner(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let out = match ev {
            Event::CloseRequest => return self.close_main(),
            Event::Blink => {
                self.devices.sheet.open_mut().blink(ctx.caret_on);
                return Reply::redraw();
            }
            ev => match self.devices_list_event(&ev, ctx) {
                Some(out) => out,
                None => match sheet_input(&mut self.devices.sheet, &ev, ctx) {
                    Some(e) => self.devices.event(e),
                    None => return Reply::none(),
                },
            },
        };
        match out {
            devices::Outcome::None => Reply::none(),
            devices::Outcome::Redraw => Reply::redraw(),
            devices::Outcome::Add => {
                self.open_wizard();
                Reply::redraw_all()
            }
            devices::Outcome::Remove(p) => {
                self.ask_remove(p, ctx);
                Reply::redraw_all()
            }
            devices::Outcome::Toggle(p) => {
                self.toggle(&p);
                Reply::redraw()
            }
            devices::Outcome::Properties(p) => {
                self.open_props(&p);
                Reply::redraw_all()
            }
            devices::Outcome::Apply(o) => {
                self.apply_options(o);
                Reply::redraw()
            }
            devices::Outcome::ApplyAndClose(o) => {
                self.apply_options(o);
                self.close_main()
            }
            devices::Outcome::Close => self.close_main(),
        }
    }

    fn wizard_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let lit = self.wizard.as_mut().is_some_and(|w| {
            let searching = w.step == addwiz::Step::Search;
            let changed = match &ev {
                Event::Pointer(PointerInput::Motion { x, y }) if searching => {
                    let r = w.list_rect();
                    w.list.hover(r, *x, *y, &ctx.theme.controls())
                }
                Event::Pointer(PointerInput::Motion { .. } | PointerInput::Leave) => w.list.leave(),
                Event::Pointer(PointerInput::Release {
                    button: BTN_LEFT, ..
                }) => w.list.release(),
                _ => false,
            };
            changed && ctx.styled()
        });
        let mut reply = self.wizard_event_inner(ev, ctx);
        reply.redraw |= lit;
        reply
    }

    fn wizard_event_inner(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let (text, forms) = self.text_forms();
        let Some(w) = self.wizard.as_mut() else {
            return Reply::none();
        };
        let searching = w.step == addwiz::Step::Search;
        let list = w.list_rect();
        let out = match ev {
            Event::CloseRequest => w.event(form::Event::Cancel, &text, &forms),
            Event::Blink => {
                w.page.form.blink(ctx.caret_on);
                return Reply::redraw();
            }
            Event::Pasted(t) => {
                let e = w.page.form.paste(t);
                w.event(e, &text, &forms)
            }
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                time,
                ..
            }) if searching && list.contains(x, y) => {
                let at = ctx.theme.metrics.click(x, y, time);
                w.list_press(at, &text, &forms, &ctx.theme.controls())
            }
            Event::Pointer(PointerInput::Motion { y, .. })
                if w.list.drag(list, y, &ctx.theme.controls()) =>
            {
                return Reply::redraw();
            }
            Event::Pointer(PointerInput::Scroll { rows, .. }) => {
                return if searching && w.list.scroll(rows, list, ctx.theme.text) {
                    Reply::redraw()
                } else {
                    Reply::none()
                };
            }
            ev => {
                if let Event::Key(k) = &ev
                    && dialog_key(k).is_some_and(|key| w.list_key(&key, &ctx.theme.controls()))
                {
                    return Reply::redraw();
                }
                match form_input(&mut w.page.form, &ev, ctx) {
                    Some(e) => w.event(e, &text, &forms),
                    None => return Reply::none(),
                }
            }
        };
        let adapter = self.adapter();
        let stop = |app: &BtApp| {
            if let Some(a) = &adapter {
                app.send(bz::Cmd::StopDiscovery(a.clone()));
            }
        };
        let start = |app: &BtApp| {
            if let Some(a) = &adapter {
                app.send(bz::Cmd::StartDiscovery(a.clone()));
            }
        };
        match out {
            addwiz::Outcome::None => Reply::none(),
            addwiz::Outcome::Redraw => Reply::redraw(),
            addwiz::Outcome::StartDiscovery => {
                start(self);
                Reply::redraw()
            }
            addwiz::Outcome::StopDiscovery => {
                stop(self);
                Reply::redraw()
            }
            addwiz::Outcome::SearchAgain => {
                stop(self);
                start(self);
                Reply::redraw()
            }
            addwiz::Outcome::Pair(p) => {
                stop(self);
                self.send(bz::Cmd::Pair(p));
                Reply::redraw()
            }
            addwiz::Outcome::Cancel { discovery, pairing } => {
                if discovery {
                    stop(self);
                }
                if let Some(p) = pairing {
                    self.send(bz::Cmd::CancelPair(p));
                }
                self.wizard = None;
                Reply::redraw_all()
            }
            addwiz::Outcome::Close => {
                self.wizard = None;
                Reply::redraw_all()
            }
        }
    }

    /// The focused edit field in dialog `win`, for [`repriseapp::keys::form_clipboard`].
    fn focused_edit(&self, win: WinId) -> Option<form::FocusedEdit<'_>> {
        match win {
            WIZARD => self.wizard.as_ref()?.page.form.focused_edit(),
            PROPS => self.props.as_ref()?.sheet.open().focused_edit(),
            PAIR => self.pair.as_ref()?.form.focused_edit(),
            _ => None,
        }
    }

    fn props_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let Some(p) = self.props.as_mut() else {
            return Reply::none();
        };
        let out = match ev {
            Event::CloseRequest => props::Outcome::Close,
            Event::Blink => {
                p.sheet.open_mut().blink(ctx.caret_on);
                return Reply::redraw();
            }
            Event::Pasted(t) => p.paste(t),
            ev => match sheet_input(&mut p.sheet, &ev, ctx) {
                Some(e) => p.event(e),
                None => return Reply::none(),
            },
        };
        let path = p.path.clone();
        match out {
            props::Outcome::None => Reply::none(),
            props::Outcome::Redraw => Reply::redraw(),
            props::Outcome::Toggle => {
                self.toggle(&path);
                Reply::redraw()
            }
            props::Outcome::Apply(c) => {
                self.apply_change(&path, c);
                Reply::redraw_all()
            }
            props::Outcome::ApplyAndClose(c) => {
                self.apply_change(&path, c);
                self.close_props()
            }
            props::Outcome::Close => self.close_props(),
        }
    }

    /// Closes the sheet, or while a change it sent waits on BlueZ, leaves it up until BlueZ
    /// answers every change: it closes then, or stays with the failure over it.
    fn close_props(&mut self) -> Reply {
        match &mut self.props {
            Some(p) if self.setting.contains(&p.path) => {
                p.leaving = true;
                Reply::redraw()
            }
            _ => {
                self.props = None;
                Reply::redraw_all()
            }
        }
    }

    fn pair_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let Some(d) = self.pair.as_mut() else {
            return Reply::none();
        };
        let out = match ev {
            Event::CloseRequest => pairdlg::Outcome::Done,
            Event::Blink => {
                d.form.blink(ctx.caret_on);
                return Reply::redraw();
            }
            Event::Pasted(t) => {
                let e = d.form.paste(t);
                d.event(e)
            }
            ev => match form_input(&mut d.form, &ev, ctx) {
                Some(e) => d.event(e),
                None => return Reply::none(),
            },
        };
        self.pair_outcome(out)
    }

    /// An accepted pairing trusts the device when [`pairdlg::PairDialog::trusts`] says so;
    /// untrusted, the agent refuses every service it connects until the user connects it.
    fn pair_outcome(&mut self, out: pairdlg::Outcome) -> Reply {
        match out {
            pairdlg::Outcome::None => Reply::none(),
            pairdlg::Outcome::Redraw => Reply::redraw(),
            pairdlg::Outcome::Answered(a) => {
                if let Some(d) = self.pair.take()
                    && d.trusts()
                    && matches!(
                        a,
                        bz::Answer::Accept | bz::Answer::Pin(_) | bz::Answer::Passkey(_)
                    )
                {
                    self.send(bz::Cmd::SetTrusted {
                        device: d.req.device,
                        on: true,
                    });
                }
                self.next_pair();
                Reply::redraw_all()
            }
            // Dropping the box answers an unanswered request Canceled.
            pairdlg::Outcome::Done => {
                self.pair = None;
                self.next_pair();
                Reply::redraw_all()
            }
        }
    }

    fn remove_event(&mut self, win: WinId, ev: Event) -> Reply {
        let (text, forms) = self.text_forms();
        match self.remove.event(win, &ev, &text, &forms) {
            Answer::None => Reply::none(),
            Answer::Redraw => Reply::redraw(),
            Answer::Closed(path, button) => {
                if button == Some(0)
                    && let Some(adapter) = self.adapter()
                {
                    if self.props.as_ref().is_some_and(|p| p.path == path) {
                        self.props = None;
                    }
                    self.send(bz::Cmd::Remove {
                        adapter,
                        device: path,
                    });
                }
                Reply::redraw_all()
            }
        }
    }

    /// A file offer opens the prompt, or waits for the open one to close.
    fn offer_in(&mut self, o: bz::Offer, ctx: &mut Ctx) {
        if self.offer.is_some() {
            self.offer_queue.push_back(o);
        } else {
            self.open_offer(o, ctx);
        }
    }

    /// The prompt starts on Decline and is a consent box, as a request the user did not start is.
    fn open_offer(&mut self, o: bz::Offer, ctx: &mut Ctx) {
        let b = MsgBox::new(
            receive::TITLE,
            Fields {
                text: receive::prompt(&o),
                icon: ctx.icons.get("dialog-question", 32),
                buttons: receive::BUTTONS.map(String::from).to_vec(),
                default: 1,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        )
        .consent();
        self.offer = Some((o, b));
    }

    /// The next queued offer once the prompt has closed; one obexd gave up on is skipped. It
    /// shows in the closed one's window, which gets no new focus, so its consent wait starts now.
    fn next_offer(&mut self, ctx: &mut Ctx) {
        while self.offer.is_none() {
            let Some(o) = self.offer_queue.pop_front() else {
                return;
            };
            if o.done.as_ref().is_none_or(|d| d.is_closed()) {
                continue;
            }
            self.open_offer(o, ctx);
        }
        if let Some((_, b)) = &mut self.offer {
            b.consent_focused(std::time::Instant::now());
        }
    }

    /// obexd gave up on `transfer`'s offer, or it was answered: its prompt closes, declining.
    fn offer_gone(&mut self, transfer: &str, ctx: &mut Ctx) {
        self.offer_queue.retain(|o| o.transfer != transfer);
        if self
            .offer
            .as_ref()
            .is_some_and(|(o, _)| o.transfer == transfer)
        {
            self.offer = None;
            self.next_offer(ctx);
        }
    }

    fn offer_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let (text, forms) = self.text_forms();
        let Some((o, b)) = &mut self.offer else {
            return Reply::none();
        };
        match b.event(&ev, &text, &forms) {
            oneshot::Outcome::None => Reply::none(),
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::Closed(button) => {
                o.answer(button == Some(receive::ACCEPT));
                self.offer = None;
                self.next_offer(ctx);
                Reply::redraw_all()
            }
        }
    }

    /// The balloon for a file saved at `path`, whose buttons open it or its folder.
    fn file_received(&mut self, path: PathBuf, sender: String) {
        let (summary, body) = receive::received(&path, &sender);
        let tag = self.next_tag;
        self.next_tag = self.next_tag.wrapping_add(1);
        self.received.insert(tag, path);
        if let Some(t) = &self.tray {
            t.notify_buttons(&summary, &body, tag, &receive::BALLOON_BUTTONS);
        }
    }

    /// Starts `open` in its program's app scope and folder with the tray's activation token, so
    /// its window comes up.
    fn spawn_with_token(&self, open: &reprisefs::props::Opening) {
        let argv =
            reprisefs::scope::in_scope(&open.argv, open.app.as_ref().map(|a| a.app_id()).as_ref());
        let handoff = (self.tray.as_ref())
            .and_then(|t| t.take_handoff())
            .unwrap_or_default();
        if let Err(e) = repriseapp::spawn_handed(&argv, open.dir.as_deref(), None, &handoff) {
            log::warn!("could not start {argv:?}: {e}");
        }
    }

    fn error_event(&mut self, win: WinId, ev: Event) -> Reply {
        let (text, forms) = self.text_forms();
        self.error.event(win, &ev, &text, &forms)
    }
}

impl App for BtApp {
    fn title(&self) -> String {
        "Bluetooth Devices".into()
    }

    fn app_id(&self) -> String {
        "reprise.bluetooth".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        let forms = repriseapp::start_theme()
            .with_form_text(repriseconf::current().accessibility.text_size);
        forms.text.size(devices::CLIENT)
    }

    fn children(&self) -> Vec<Child> {
        let u = self.forms.text;
        let mut out = Vec::new();
        if self.wizard.is_some() {
            out.push(Child {
                id: WIZARD,
                title: "Add Bluetooth Device Wizard".into(),
                size: u.size(wizard::CLIENT),
                fixed: true,
                modal: false,
            });
        }
        if let Some(p) = &self.props {
            out.push(Child {
                id: PROPS,
                title: p.window_title(),
                size: u.size(props::CLIENT),
                fixed: true,
                modal: false,
            });
        }
        if self.pair.is_some() {
            out.push(Child {
                id: PAIR,
                // The compositor focuses the box by this title (`mapfocus::DEVICE_PROMPTS`).
                title: "Bluetooth Pairing Request".into(),
                size: u.size(pairdlg::CLIENT),
                fixed: true,
                modal: true,
            });
        }
        if let Some(text) = &self.text {
            out.extend(self.remove.child(text, &self.forms));
            out.extend(self.error.child(text, &self.forms));
            if let Some((_, b)) = &self.offer {
                out.push(b.child(OFFER, text, &self.forms));
            }
        }
        out
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        self.fonts();
        // The tray takes the `reprise.bluetooth` bus name; a second instance exits here after raising the first.
        match tray::start(waker.clone()).or_exit(self.tray_only) {
            Ok(Some((tray, msgs))) => {
                self.tray = Some(tray);
                self.msgs = Some(msgs);
            }
            Ok(None) => {}
            Err(code) => return repriseapp::Start::Exit(code),
        }
        let (rx, handle) = bz::watch(waker);
        self.bz_rx = Some(rx);
        self.bz = Some(handle);
        repriseapp::Start::Continue
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.text = Some(ctx.text.clone());
        self.forms = ctx.forms();
        if matches!(ev, Event::Wake) {
            let reply = self.wake(ctx);
            self.withdraw_orphans();
            return reply;
        }
        let clip = form_clipboard(self.focused_edit(win), &ev);
        let mut reply = match win {
            WIZARD => self.wizard_event(ev, ctx),
            PROPS => self.props_event(ev, ctx),
            PAIR => self.pair_event(ev, ctx),
            w if self.remove.owns(w) => self.remove_event(w, ev),
            w if self.error.owns(w) => self.error_event(w, ev),
            OFFER => self.offer_event(ev, ctx),
            _ => self.main_event(ev, ctx),
        }
        .with_clipboard(clip);
        self.withdraw_orphans();
        let size = self.forms.text.size(devices::CLIENT);
        reply.ask_size(&mut self.asked, size);
        reply
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.text = Some(ctx.text.clone());
        let cs = &ctx.theme.colors;
        let forms = ctx.forms();
        let px = forms.fonts.size_px;
        match win {
            WIZARD => {
                if let Some(w) = &self.wizard {
                    w.draw(c, ctx, &self.icons);
                }
            }
            PROPS => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(p) = &self.props {
                    propsheet::draw(
                        c,
                        &ctx.text,
                        &Paint {
                            s: 1,
                            ..forms.paint()
                        },
                        &p.sheet,
                        true,
                        px,
                    );
                }
            }
            PAIR => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(d) = &self.pair {
                    form::draw(
                        c,
                        &ctx.text,
                        &Paint {
                            s: 1,
                            ..forms.paint()
                        },
                        &d.form,
                        true,
                        px,
                    );
                }
            }
            w if self.remove.owns(w) => self.remove.draw(c, &forms, &ctx.text),
            w if self.error.owns(w) => self.error.draw(c, &forms, &ctx.text),
            OFFER => {
                if let Some((_, b)) = &self.offer {
                    b.draw(c, &forms, &ctx.text);
                }
            }
            _ => self.devices.draw(c, ctx, &self.icons),
        }
    }
}

impl repriseapp::Extended for BtApp {
    fn fixed(&self) -> bool {
        true
    }

    fn hidden(&self) -> bool {
        self.hidden
    }

    /// A failure covers the window it is about ([`BtApp::failed`]), every other child the folder.
    fn parent(&self, win: WinId) -> Option<WinId> {
        Some(self.error.parent(win).unwrap_or(repriseapp::MAIN))
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        let px = forms.fonts.size_px;
        match win {
            WIZARD => {
                if let Some(w) = &self.wizard {
                    w.access(tree, ctx);
                }
            }
            PROPS => {
                if let Some(p) = &self.props {
                    oneshot::access_sheet(&p.sheet, tree, &ctx.text, px);
                }
            }
            PAIR => {
                if let Some(d) = &self.pair {
                    oneshot::access_form(&d.form, tree, &ctx.text, px);
                }
            }
            w if self.remove.owns(w) => self.remove.access(tree, &ctx.text, &forms),
            w if self.error.owns(w) => self.error.access(tree, &ctx.text, &forms),
            OFFER => {
                if let Some((_, b)) = &self.offer {
                    b.access(tree, &ctx.text, &forms);
                }
            }
            _ => self.devices.access(tree, ctx),
        }
    }

    fn drop_form(&mut self, win: WinId) -> Option<DropForm<'_>> {
        let form = match win {
            WIZARD => &mut self.wizard.as_mut()?.page.form,
            PROPS => self.props.as_mut()?.sheet.open_mut(),
            PAIR => &mut self.pair.as_mut()?.form,
            _ => return None,
        };
        Some(DropForm { form, px: None })
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(args: Vec<std::ffi::OsString>) {
    repriseapp::applet_main(&args, BtApp::new);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bz::{Adapter, Ask, Device, Kind};
    use repriseapp::Extended;
    use repriseapp::{KeyInput, MAIN};
    use reprisekit::textedit::EditKey;
    use reprisekit::theme::FontConfig;

    fn app() -> BtApp {
        let mut a = BtApp::new(false);
        a.fonts();
        a
    }

    fn cans(connected: bool) -> Device {
        Device {
            path: "/a/cans".into(),
            alias: "Cans".into(),
            kind: Kind::Headphones,
            paired: true,
            connected,
            ..Device::default()
        }
    }

    fn snap(adapter: bool) -> bz::Snapshot {
        bz::Snapshot {
            adapter: adapter.then(|| Adapter {
                path: "/a".into(),
                powered: true,
                ..Adapter::default()
            }),
            devices: vec![cans(false)],
            airplane: None,
        }
    }

    #[test]
    fn adapter_loss_closes_props_and_the_pairing_box_and_fails_the_wizard() {
        let mut a = app();
        a.snapshot_in(snap(true), 0);
        a.open_props("/a/cans");
        a.open_wizard();
        let (text, forms) = a.text_forms();
        let w = a.wizard.as_mut().unwrap();
        w.page.form.set_check("ready", true);
        w.event(form::Event::Changed("ready"), &text, &forms);
        assert_eq!(
            w.event(form::Event::Default, &text, &forms),
            addwiz::Outcome::StartDiscovery
        );
        let (done, wait) = async_channel::bounded(1);
        a.agent(bz::AgentRequest {
            device: "/a/other".into(),
            ask: Ask::Authorize,
            done: Some(done),
        });
        assert!(a.pair.is_some());
        a.snapshot_in(bz::Snapshot::default(), 0);
        assert!(a.props.is_none());
        assert!(a.pair.is_none());
        assert!(
            wait.try_recv().is_err() && wait.is_closed(),
            "the box's request is answered Canceled"
        );
        assert_eq!(
            a.wizard.as_ref().and_then(|w| w.reason()),
            Some("The Bluetooth adapter was removed.")
        );
    }

    #[test]
    fn a_removed_device_closes_its_properties() {
        let mut a = app();
        a.snapshot_in(snap(true), 0);
        a.open_props("/a/cans");
        assert!(a.props.is_some());
        let mut s = snap(true);
        s.devices.clear();
        a.snapshot_in(s, 0);
        assert!(a.props.is_none());
    }

    #[test]
    fn the_properties_sheet_takes_drops_on_its_open_page() {
        let mut a = app();
        a.snapshot_in(snap(true), 0);
        assert!(a.drop_form(PROPS).is_none());
        a.open_props("/a/cans");
        let d = a.drop_form(PROPS).expect("the open sheet's page");
        assert!(d.form.item(props::NAME).is_some());
        assert!(a.drop_form(MAIN).is_none());
    }

    #[test]
    fn agent_requests_route_to_the_wizard_or_the_dialog() {
        let mut a = app();
        a.snapshot_in(snap(true), 0);
        let (d1, w1) = async_channel::bounded(1);
        a.agent(bz::AgentRequest {
            device: "/a/cans".into(),
            ask: Ask::Authorize,
            done: Some(d1),
        });
        assert!(a.pair.is_some(), "no wizard: the dialog opens");
        let (d2, _w2) = async_channel::bounded(1);
        a.agent(bz::AgentRequest {
            device: "/a/other".into(),
            ask: Ask::Confirm(1),
            done: Some(d2),
        });
        assert!(!w1.is_closed(), "the open box keeps its request");
        let shown = |a: &BtApp| a.pair.as_ref().map(|d| d.req.ask.clone());
        assert_eq!(shown(&a), Some(Ask::Authorize));
        a.pair_outcome(pairdlg::Outcome::Done);
        assert!(w1.is_closed(), "closed unanswered");
        assert_eq!(
            shown(&a),
            Some(Ask::Confirm(1)),
            "the queued one opens next"
        );
        let now = std::time::Instant::now();
        let f = &a.pair.as_ref().unwrap().form;
        assert!(!f.consent_press(now), "in the same window, it waits again");
        assert!(f.consent_press(now + form::CONSENT_ARM));
    }

    #[test]
    fn the_box_names_the_device_by_alias_and_address() {
        let mut a = app();
        let mut s = snap(true);
        s.devices[0].address = "AA:BB:CC:DD:EE:FF".into();
        a.snapshot_in(s, 0);
        let (done, _wait) = async_channel::bounded(1);
        a.agent(bz::AgentRequest {
            device: "/a/cans".into(),
            ask: Ask::Authorize,
            done: Some(done),
        });
        let f = &a.pair.as_ref().unwrap().form;
        let said = match &f.item("sentence").unwrap().control {
            form::Control::Lines(l) => l.join(" "),
            other => panic!("sentence is {other:?}"),
        };
        assert!(
            said.contains("Cans") && said.contains("AA:BB:CC:DD:EE:FF"),
            "{said}"
        );
    }

    #[test]
    fn a_line_break_in_the_device_name_adds_no_line_to_the_box() {
        let mut a = app();
        let mut s = snap(true);
        s.devices[0].address = "AA:BB:CC:DD:EE:FF".into();
        s.devices[0].alias = "Cans\n999999\u{202e}".into();
        a.snapshot_in(s, 0);
        let (done, _wait) = async_channel::bounded(1);
        a.agent(bz::AgentRequest {
            device: "/a/cans".into(),
            ask: Ask::Authorize,
            done: Some(done),
        });
        let f = &a.pair.as_ref().unwrap().form;
        let lines = match &f.item("sentence").unwrap().control {
            form::Control::Lines(l) => l.clone(),
            other => panic!("sentence is {other:?}"),
        };
        assert!(
            lines[0].starts_with("Cans 999999 (AA:BB:CC:DD:EE:FF)"),
            "{lines:?}"
        );
    }

    #[test]
    fn bluez_cancel_closes_a_waiting_prompt_only() {
        let mut a = app();
        a.snapshot_in(snap(true), 0);
        a.agent(bz::AgentRequest {
            device: "/a/cans".into(),
            ask: Ask::ShowPasskey(123456),
            done: None,
        });
        a.agent_cancel();
        assert!(a.pair.is_some(), "a shown code stays up");
        let (done, wait) = async_channel::bounded(1);
        a.agent(bz::AgentRequest {
            device: "/a/cans".into(),
            ask: Ask::Authorize,
            done: Some(done),
        });
        a.agent_cancel();
        assert!(wait.is_closed(), "the queued request is dropped");
        a.pair_outcome(pairdlg::Outcome::Done);
        assert!(a.pair.is_none(), "nothing left to show");
        let (done, _wait) = async_channel::bounded(1);
        a.agent(bz::AgentRequest {
            device: "/a/cans".into(),
            ask: Ask::Authorize,
            done: Some(done),
        });
        a.agent_cancel();
        assert!(a.pair.is_none());
    }

    fn wired() -> (BtApp, async_channel::Receiver<bz::Cmd>) {
        let mut a = app();
        let (h, rx) = bz::BzHandle::detached();
        a.bz = Some(h);
        a.snapshot_in(snap(true), 0);
        (a, rx)
    }

    fn connects(rx: &async_channel::Receiver<bz::Cmd>) -> usize {
        std::iter::from_fn(|| rx.try_recv().ok())
            .filter(|c| matches!(c, bz::Cmd::Connect(_)))
            .count()
    }

    #[test]
    fn a_pending_connect_is_sent_once_until_it_finishes() {
        let (mut a, rx) = wired();
        a.toggle("/a/cans");
        a.toggle("/a/cans");
        assert_eq!(connects(&rx), 1, "the second waits on the first");
        a.connected("/a/cans".into(), None);
        assert!(
            !a.error.is_shown(),
            "a Connect that went through shows no box"
        );
        a.toggle("/a/cans");
        assert_eq!(connects(&rx), 1, "finished, so the next one is sent");
    }

    #[test]
    fn a_connect_failing_after_a_disconnect_shows_no_box() {
        let mut a = app();
        a.snapshot_in(snap(true), 0);
        a.toggle("/a/cans");
        let mut s = snap(true);
        s.devices = vec![cans(true)];
        a.snapshot_in(s, 0);
        a.toggle("/a/cans");
        a.connected("/a/cans".into(), Some("canceled".into()));
        assert!(!a.error.is_shown(), "the user asked to disconnect it");

        let mut a = app();
        a.snapshot_in(snap(true), 0);
        a.toggle("/a/cans");
        a.connected("/a/cans".into(), Some("timed out".into()));
        assert_eq!(
            a.error.shown().map(|b| b.title.as_str()),
            Some("Could not connect"),
            "a Connect failing on its own shows a box"
        );
    }

    fn ran(cmd: bz::Cmd, answer: u64) -> bz::BzMsg {
        bz::BzMsg::Ran { cmd, answer }
    }

    /// Hands the program the bus thread's `msgs` in one wake.
    fn bus_says(a: &mut BtApp, msgs: impl IntoIterator<Item = bz::BzMsg>) {
        let (tx, rx) = std::sync::mpsc::channel();
        for m in msgs {
            tx.send(m).unwrap();
        }
        a.bz_rx = Some(rx);
        a.event(MAIN, Event::Wake, &mut test_ctx());
    }

    #[test]
    fn a_failure_covers_the_sheet_or_wizard_it_is_about_and_leaves_with_it() {
        let mut a = app();
        a.snapshot_in(snap(true), 0);
        a.open_props("/a/cans");
        a.open_wizard();
        let failed = |cmd| bz::BzMsg::Failed {
            cmd,
            body: "Not ready".into(),
        };
        let alias = bz::Cmd::SetAlias {
            device: "/a/cans".into(),
            alias: "Mine".into(),
        };
        let power = bz::Cmd::SetPowered {
            adapter: "/a".into(),
            on: true,
        };
        let search = || bz::Cmd::StartDiscovery("/a".into());
        bus_says(&mut a, [failed(alias), failed(search()), failed(power)]);
        a.connected("/a/cans".into(), Some("timed out".into()));
        let over = |a: &BtApp| a.parent(a.error.win()?);
        assert_eq!(over(&a), Some(PROPS), "a change from the sheet covers it");
        a.error.close();
        assert_eq!(over(&a), Some(WIZARD), "the search covers the wizard");
        a.error.close();
        assert_eq!(over(&a), Some(MAIN), "the adapter's covers the folder");
        a.error.close();
        assert_eq!(over(&a), Some(PROPS), "a connect covers its device's sheet");

        let mut ctx = test_ctx();
        a.event(PROPS, Event::CloseRequest, &mut ctx);
        assert!(!a.error.is_shown(), "the box left with the sheet");
        bus_says(&mut a, [failed(search())]);
        a.event(WIZARD, Event::CloseRequest, &mut ctx);
        assert!(!a.error.is_shown(), "the box left with the wizard");
    }

    /// Types "Mine" over the open sheet's name; the SetAlias that Apply or OK sends for it.
    fn rename_to_mine(a: &mut BtApp, ctx: &mut Ctx) -> bz::Cmd {
        let page = a.props.as_mut().unwrap().sheet.open_mut();
        page.focus_id(props::NAME);
        page.key(form::Key::Edit(EditKey::SelectAll));
        a.event(PROPS, Event::Pasted("Mine".into()), ctx);
        bz::Cmd::SetAlias {
            device: "/a/cans".into(),
            alias: "Mine".into(),
        }
    }

    fn sheet_button_enabled(a: &BtApp, id: &str) -> bool {
        match a
            .props
            .as_ref()
            .and_then(|p| p.sheet.open().item(id))
            .map(|i| &i.control)
        {
            Some(form::Control::Button { enabled, .. }) => *enabled,
            other => panic!("{id} is {other:?}"),
        }
    }

    #[test]
    fn ok_on_a_rename_greys_the_sheet_until_bluez_answers_and_then_closes_it() {
        let (mut a, rx) = wired();
        let mut ctx = test_ctx();
        a.open_props("/a/cans");
        let alias = rename_to_mine(&mut a, &mut ctx);
        a.event(PROPS, key(keysym::RETURN, false), &mut ctx);
        assert_eq!(rx.try_recv().ok(), Some(alias.clone()));
        assert!(a.props.is_some(), "the sheet waits for BlueZ");
        assert!(!sheet_button_enabled(&a, propsheet::OK));
        assert!(!sheet_button_enabled(&a, propsheet::APPLY));

        bus_says(&mut a, [ran(alias, 1)]);
        assert!(a.props.is_none(), "the sheet closes once BlueZ answers");
    }

    #[test]
    fn ok_on_a_rename_bluez_refuses_keeps_the_sheet_and_its_edit_under_the_error() {
        let (mut a, _rx) = wired();
        let mut ctx = test_ctx();
        a.open_props("/a/cans");
        let alias = rename_to_mine(&mut a, &mut ctx);
        a.event(PROPS, key(keysym::RETURN, false), &mut ctx);
        let failed = bz::BzMsg::Failed {
            cmd: alias.clone(),
            body: "I/O error: timed out".into(),
        };
        bus_says(&mut a, [failed, ran(alias, 1)]);

        let p = a.props.as_ref().expect("the sheet stays");
        assert_eq!(p.sheet.open().edit_text(props::NAME), "Mine");
        assert_eq!(p.window_title(), "Cans Properties");
        assert_eq!(a.error.win().and_then(|w| a.parent(w)), Some(PROPS));
        assert!(
            sheet_button_enabled(&a, propsheet::APPLY),
            "the rename can be sent again"
        );
        assert!(sheet_button_enabled(&a, propsheet::OK));
    }

    #[test]
    fn cancel_or_the_close_box_while_a_rename_waits_closes_the_sheet_once_bluez_answers() {
        for close in [key(keysym::ESCAPE, false), Event::CloseRequest] {
            let (mut a, _rx) = wired();
            let mut ctx = test_ctx();
            a.open_props("/a/cans");
            let alias = rename_to_mine(&mut a, &mut ctx);
            a.event(PROPS, key(u32::from(b'a'), true), &mut ctx);
            a.event(PROPS, close, &mut ctx);
            assert!(a.props.is_some(), "the sheet waits for BlueZ");

            bus_says(&mut a, [ran(alias, 1)]);
            assert!(a.props.is_none());

            a.open_props("/a/cans");
            a.event(PROPS, key(keysym::RETURN, false), &mut ctx);
            assert!(a.props.is_none(), "OK with nothing to apply closes at once");
        }
    }

    #[test]
    fn an_applied_rename_greys_apply_and_ok_until_bluez_answers_it() {
        let (mut a, rx) = wired();
        let mut ctx = test_ctx();
        a.open_props("/a/cans");
        let alias = rename_to_mine(&mut a, &mut ctx);
        a.event(PROPS, key(u32::from(b'a'), true), &mut ctx);
        assert_eq!(rx.try_recv().ok(), Some(alias.clone()));
        assert!(
            !sheet_button_enabled(&a, propsheet::OK),
            "OK greys while the rename waits"
        );
        assert!(!sheet_button_enabled(&a, propsheet::APPLY));
        let p = a.props.as_mut().unwrap();
        assert_eq!(p.event(propsheet::Event::Ok), props::Outcome::None);

        bus_says(&mut a, [ran(alias, 1)]);
        assert!(
            sheet_button_enabled(&a, propsheet::OK),
            "OK is back once BlueZ answers"
        );
        assert!(
            !sheet_button_enabled(&a, propsheet::APPLY),
            "nothing left to apply"
        );
    }

    fn props_title(a: &BtApp) -> Option<String> {
        a.children()
            .into_iter()
            .find(|c| c.id == PROPS)
            .map(|c| c.title)
    }

    /// BlueZ's snapshot with Cans named `alias`, trusted as `trusted` and its battery at `battery`,
    /// read after `answered` answers.
    fn cans_as(alias: &str, trusted: bool, battery: u8, answered: u64) -> bz::BzMsg {
        let mut s = snap(true);
        s.devices[0].alias = alias.into();
        s.devices[0].trusted = trusted;
        s.devices[0].battery = Some(battery);
        bz::BzMsg::Snapshot {
            snapshot: s,
            answered,
        }
    }

    #[test]
    fn a_sent_rename_titles_the_sheet_only_once_bluez_reports_the_new_name() {
        let (mut a, _rx) = wired();
        let mut ctx = test_ctx();
        a.open_props("/a/cans");
        let alias = rename_to_mine(&mut a, &mut ctx);
        a.event(PROPS, key(keysym::RETURN, false), &mut ctx);
        assert_eq!(props_title(&a).as_deref(), Some("Cans Properties"));

        let (mut a, _rx) = wired();
        a.open_props("/a/cans");
        rename_to_mine(&mut a, &mut ctx);
        a.event(PROPS, key(u32::from(b'a'), true), &mut ctx);
        assert_eq!(props_title(&a).as_deref(), Some("Cans Properties"));
        bus_says(&mut a, [ran(alias, 1)]);
        assert_eq!(props_title(&a).as_deref(), Some("Cans Properties"));
        bus_says(&mut a, [cans_as("Mine", false, 80, 1)]);
        assert_eq!(props_title(&a).as_deref(), Some("Mine Properties"));

        bus_says(&mut a, [cans_as("Other", false, 80, 1)]);
        assert_eq!(props_title(&a).as_deref(), Some("Other Properties"));
        let p = a.props.as_ref().unwrap();
        assert_eq!(p.sheet.open().edit_text(props::NAME), "Other");
    }

    #[test]
    fn a_snapshot_read_before_bluez_answered_keeps_the_sent_name_and_trust_however_late_it_lands() {
        for in_one_wake in [true, false] {
            let (mut a, rx) = wired();
            let mut ctx = test_ctx();
            a.open_props("/a/cans");
            let alias = rename_to_mine(&mut a, &mut ctx);
            a.event(PROPS, key(u32::from(b'm'), true), &mut ctx);
            a.event(PROPS, key(u32::from(b'a'), true), &mut ctx);
            let trust = bz::Cmd::SetTrusted {
                device: "/a/cans".into(),
                on: true,
            };
            let sent: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
            assert_eq!(sent, [alias.clone(), trust.clone()]);

            let (alias, trust) = (ran(alias, 1), ran(trust, 2));
            let stale = cans_as("Cans", false, 70, 0);
            if in_one_wake {
                bus_says(&mut a, [stale, alias, trust]);
            } else {
                bus_says(&mut a, [alias, trust]);
                bus_says(&mut a, [stale]);
            }
            let page = a.props.as_ref().unwrap().sheet.open();
            assert_eq!(
                page.edit_text(props::NAME),
                "Mine",
                "in one wake: {in_one_wake}"
            );
            assert_eq!(
                page.checked(props::AUTO),
                Some(true),
                "in one wake: {in_one_wake}"
            );
            assert!(!sheet_button_enabled(&a, propsheet::APPLY));

            bus_says(&mut a, [cans_as("Other", false, 70, 2)]);
            let page = a.props.as_ref().unwrap().sheet.open();
            assert_eq!(page.edit_text(props::NAME), "Other", "a later change shows");
            assert_eq!(page.checked(props::AUTO), Some(false));
        }
    }

    #[test]
    fn a_snapshot_while_a_change_waits_keeps_it_for_a_failure() {
        let (mut a, rx) = wired();
        let mut ctx = test_ctx();
        a.open_props("/a/cans");
        let alias = rename_to_mine(&mut a, &mut ctx);
        a.event(PROPS, key(u32::from(b'm'), true), &mut ctx);
        a.event(PROPS, key(u32::from(b'a'), true), &mut ctx);
        let trust = bz::Cmd::SetTrusted {
            device: "/a/cans".into(),
            on: true,
        };
        let sent: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(sent, [alias.clone(), trust.clone()]);

        bus_says(&mut a, [cans_as("Cans", false, 70, 0)]);
        let page = a.props.as_ref().unwrap().sheet.open();
        assert_eq!(page.edit_text(props::NAME), "Mine");
        assert_eq!(page.checked(props::AUTO), Some(true));

        let failed = |cmd: &bz::Cmd| bz::BzMsg::Failed {
            cmd: cmd.clone(),
            body: "I/O error: timed out".into(),
        };
        bus_says(
            &mut a,
            [failed(&alias), ran(alias, 1), failed(&trust), ran(trust, 2)],
        );
        let page = a.props.as_ref().unwrap().sheet.open();
        assert_eq!(page.edit_text(props::NAME), "Mine");
        assert_eq!(page.checked(props::AUTO), Some(true));
        assert!(
            sheet_button_enabled(&a, propsheet::APPLY),
            "the change can be sent again"
        );
    }

    #[test]
    fn a_vanished_device_closes_its_remove_box() {
        let mut a = app();
        a.snapshot_in(snap(true), 0);
        let mut ctx = test_ctx();
        a.ask_remove("/a/cans".into(), &mut ctx);
        assert!(!a.remove.is_empty());
        let mut s = snap(true);
        s.devices.clear();
        a.snapshot_in(s, 0);
        assert!(a.remove.is_empty());
    }

    /// Accepts the open box's Confirm by picking its Yes radio and pressing OK; `trust` ticks
    /// its trust box first when it has one.
    fn accept(a: &mut BtApp, trust: bool) {
        let d = a.pair.as_mut().unwrap();
        for (id, on) in [(passkey::YES, true), (passkey::NO, false)] {
            if let Some(form::Control::Radio { checked, .. }) =
                d.form.item_mut(id).map(|i| &mut i.control)
            {
                *checked = on;
            }
        }
        if trust {
            d.form.set_check("trust", true);
        }
        let out = d.event(form::Event::Default);
        assert_eq!(out, pairdlg::Outcome::Answered(bz::Answer::Accept));
        a.pair_outcome(out);
    }

    #[test]
    fn an_accepted_pairing_trusts_the_device_only_when_started_here_or_ticked() {
        let (mut a, rx) = wired();
        let trusted = |rx: &async_channel::Receiver<bz::Cmd>| {
            std::iter::from_fn(|| rx.try_recv().ok())
                .filter(|c| {
                    matches!(c, bz::Cmd::SetTrusted { device, on: true } if device == "/a/cans")
                })
                .count()
        };
        let confirm = |a: &mut BtApp| {
            let (done, wait) = async_channel::bounded(1);
            a.agent(bz::AgentRequest {
                device: "/a/cans".into(),
                ask: Ask::Confirm(7),
                done: Some(done),
            });
            wait
        };
        let _wait = confirm(&mut a);
        accept(&mut a, false);
        assert!(a.pair.is_none());
        assert_eq!(
            trusted(&rx),
            0,
            "nobody here started it: paired, not trusted"
        );

        let _wait = confirm(&mut a);
        accept(&mut a, true);
        assert_eq!(trusted(&rx), 1, "the trust box ticked");

        let _wait = confirm(&mut a);
        let out = a.pair.as_mut().unwrap().event(form::Event::Cancel);
        a.pair_outcome(out);
        assert_eq!(trusted(&rx), 0, "refused: not trusted");

        a.toggle("/a/cans");
        let _wait = confirm(&mut a);
        accept(&mut a, false);
        assert_eq!(trusted(&rx), 1, "a Connect from Devices started it");
    }

    /// Under Blue the scrollbar arrow lights under the pointer and goes cold after press, release and leave; each change asks for a redraw.
    #[test]
    fn the_devices_lists_arrow_goes_cold_after_press_release_and_leave() {
        use reprisekit::controls::tilelist;
        use reprisekit::pointer::Interaction;
        let mut a = app();
        let mut ctx = test_ctx();
        ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
        let mut s = snap(true);
        s.devices = (0..20)
            .map(|i| Device {
                path: format!("/a/d{i}"),
                alias: format!("Device {i}"),
                ..cans(false)
            })
            .collect();
        a.snapshot_in(s, 0);
        let cm = ctx.theme.controls();
        let sb = tilelist::layout(&a.devices.list, devices::LIST, &cm)
            .scrollbar
            .expect("20 devices overflow the list");
        let (x, y) = (sb.x + 8, sb.y + 2);
        let ptr = |e| Event::Pointer(e);
        assert!(
            a.main_event(ptr(PointerInput::Motion { x, y }), &mut ctx)
                .redraw
        );
        assert!(matches!(
            a.devices.list.bar_lit(),
            Some((_, Interaction::Hot))
        ));
        let press = PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: repriseapp::Mods::default(),
        };
        a.main_event(ptr(press), &mut ctx);
        assert!(matches!(
            a.devices.list.bar_lit(),
            Some((_, Interaction::Pressed))
        ));
        let release = PointerInput::Release {
            x,
            y,
            button: BTN_LEFT,
            mods: repriseapp::Mods::default(),
        };
        assert!(a.main_event(ptr(release), &mut ctx).redraw, "held to hot");
        let (ox, oy) = (devices::LIST.x + 20, devices::LIST.y + 20);
        assert!(
            a.main_event(ptr(PointerInput::Motion { x: ox, y: oy }), &mut ctx)
                .redraw
        );
        assert_eq!(a.devices.list.bar_lit(), None, "moved off the bar");
        a.main_event(ptr(PointerInput::Motion { x, y }), &mut ctx);
        assert!(a.main_event(ptr(PointerInput::Leave), &mut ctx).redraw);
        assert_eq!(a.devices.list.bar_lit(), None, "left the window");
    }

    /// Same for the Add wizard's search list.
    #[test]
    fn the_wizards_list_arrow_goes_cold_after_press_release_and_leave() {
        use reprisekit::controls::tilelist;
        let mut a = app();
        let mut ctx = test_ctx();
        ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
        a.snapshot_in(snap(true), 0);
        a.open_wizard();
        let w = a.wizard.as_mut().unwrap();
        w.step = addwiz::Step::Search;
        w.list = tilelist::TileList::rows(30, 40);
        let cm = ctx.theme.controls();
        let sb = tilelist::layout(&w.list, addwiz::LIST, &cm)
            .scrollbar
            .expect("30 rows overflow the list");
        let (x, y) = (sb.x + 8, sb.y + 2);
        let ptr = |e| Event::Pointer(e);
        let lit = |a: &BtApp| a.wizard.as_ref().unwrap().list.bar_lit();
        assert!(
            a.wizard_event(ptr(PointerInput::Motion { x, y }), &ctx)
                .redraw
        );
        assert!(lit(&a).is_some());
        let press = PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: repriseapp::Mods::default(),
        };
        a.wizard_event(ptr(press), &ctx);
        let release = PointerInput::Release {
            x,
            y,
            button: BTN_LEFT,
            mods: repriseapp::Mods::default(),
        };
        assert!(a.wizard_event(ptr(release), &ctx).redraw);
        assert!(a.wizard_event(ptr(PointerInput::Leave), &ctx).redraw);
        assert_eq!(lit(&a), None, "nothing stays lit");
    }

    #[test]
    fn the_name_field_copies_and_cuts() {
        let mut ctx = test_ctx();
        let mut a = app();
        a.snapshot_in(snap(true), 0);
        a.open_props("/a/cans");
        let page = a.props.as_mut().unwrap().sheet.open_mut();
        page.focus_id("name");
        page.key(form::Key::Edit(EditKey::SelectAll));
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
        let name = |a: &BtApp| {
            let p = a.props.as_ref().unwrap();
            p.sheet.open().edit_text("name").to_string()
        };
        let copy = a.event(PROPS, ctrl(b'c'), &mut ctx).copy;
        assert_eq!(copy.as_deref(), Some("Cans"));
        assert_eq!(name(&a), "Cans", "a copy leaves the field");
        let cut = a.event(PROPS, ctrl(b'x'), &mut ctx).copy;
        assert_eq!(cut.as_deref(), Some("Cans"));
        assert_eq!(name(&a), "", "a cut empties the field");
    }

    #[test]
    fn dialogs_open_at_the_text_size() {
        let mut a = app();
        a.snapshot_in(snap(true), 0);
        let mut ctx = Ctx {
            text_size: 150,
            ..test_ctx()
        };
        a.event(MAIN, Event::Blink, &mut ctx);
        a.open_wizard();
        a.open_props("/a/cans");
        let size = |a: &BtApp, id| a.children().iter().find(|c| c.id == id).unwrap().size;
        assert_eq!(size(&a, WIZARD), (768, 556));
        assert_eq!(size(&a, PROPS), (558, 648));
    }

    /// Bluetooth Devices, a fixed window, asks for its grown size when the text size changes
    /// while it is open, and asks nothing while the size stands.
    #[test]
    fn the_open_window_grows_with_the_text_size() {
        let mut a = app();
        let mut ctx = test_ctx();
        a.event(MAIN, Event::Blink, &mut ctx);
        assert_eq!(a.event(MAIN, Event::Blink, &mut ctx).size, None);
        ctx.text_size = 150;
        let u = ctx.forms().text;
        let grown = a.event(MAIN, Event::Resized, &mut ctx);
        assert_eq!(grown.size, Some(u.size(devices::CLIENT)));
        assert_eq!(a.event(MAIN, Event::Blink, &mut ctx).size, None);
    }

    /// In a window shorter than the sheet the device list scrolls with the page and draws only
    /// inside it; the wheel over the page's scrollbar scrolls the page.
    #[test]
    fn the_devices_list_scrolls_and_clips_with_a_short_page() {
        let mut a = app();
        let mut ctx = test_ctx();
        let mut s = snap(true);
        s.devices = (0..20)
            .map(|i| Device {
                path: format!("/a/d{i}"),
                alias: format!("Device {i}"),
                ..cans(false)
            })
            .collect();
        a.snapshot_in(s, 0);
        ctx.size = (devices::CLIENT.0, 300);
        a.event(MAIN, Event::Pointer(PointerInput::Leave), &mut ctx);
        let px = ctx.theme.fonts.size_px;
        let bar = a
            .devices
            .sheet
            .layout(&ctx.text, px)
            .bar
            .expect("the page scrolls");
        let wheel = PointerInput::Scroll {
            x: bar.x + 2,
            y: bar.y + bar.height / 2,
            rows: 3,
        };
        a.event(MAIN, Event::Pointer(wheel), &mut ctx);
        let (_, dy) = a.devices.sheet.page_offset();
        assert!(dy < 0, "the page scrolled");
        assert_eq!(a.devices.list_rect().y, devices::LIST.y + dy);
        let clip = a.devices.sheet.page_clip(&ctx.text, px).unwrap();
        assert!(
            a.devices.list_rect().y < clip.y,
            "the list runs above the page"
        );

        let draw = |a: &mut BtApp| {
            let mut c = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
            a.draw(MAIN, &mut c, &ctx);
            c
        };
        let first = draw(&mut a);
        let list = a.devices.list_rect();
        a.devices
            .list
            .scroll(2, list, reprisekit::theme::TextUnits::ONE);
        let scrolled = draw(&mut a);
        let outside = |x: i32, y: i32| !clip.contains(x, y);
        let (w, h) = (ctx.size.0, ctx.size.1);
        let pts = (0..h).flat_map(|y| (0..w).map(move |x| (x, y)));
        assert!(
            pts.clone()
                .filter(|&(x, y)| outside(x, y))
                .all(|(x, y)| first.get(x, y) == scrolled.get(x, y)),
            "nothing of the list draws outside the page"
        );
        assert!(
            pts.filter(|&(x, y)| clip.contains(x, y))
                .any(|(x, y)| first.get(x, y) != scrolled.get(x, y)),
            "the list draws inside it"
        );
    }

    /// At 200% the Devices list grows with its page and its tiles grow with it: a press on the
    /// last grown tile picks that device.
    #[test]
    fn at_200_percent_the_device_tiles_grow_with_the_page() {
        use reprisekit::controls::tilelist;
        let mut a = app();
        let mut ctx = test_ctx();
        ctx.text_size = 200;
        ctx.theme = Theme::default().with_text_size(200);
        let u = ctx.theme.text;
        ctx.size = u.size(devices::CLIENT);
        let mut s = snap(true);
        s.devices = (0..2)
            .map(|i| Device {
                path: format!("/a/d{i}"),
                alias: format!("Device {i}"),
                ..cans(false)
            })
            .collect();
        a.snapshot_in(s, 0);
        a.event(MAIN, Event::Pointer(PointerInput::Leave), &mut ctx);
        let mut c = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
        a.draw(MAIN, &mut c, &ctx);
        let list = a.devices.list_rect();
        assert_eq!(list.width, u.rect(devices::LIST).width);
        let cm = ctx.theme.controls();
        let lay = tilelist::layout(&a.devices.list, list, &cm);
        let last = lay.slots.last().expect("a tile");
        assert_eq!(last.rect.height, u.len(tilelist::TILE_H));
        // The tile's icon is the 32 px one doubled, 8 px in and centred down the row.
        let small = a.icons.get(a.devices.rows[1].kind);
        let (x0, y0) = (last.rect.x + 8, last.rect.y + (last.rect.height - 64) / 2);
        for y in 0..small.height as i32 {
            for x in 0..small.width as i32 {
                let px = small.get(x, y);
                if px >> 24 == 0xff {
                    assert_eq!(c.get(x0 + 2 * x + 1, y0 + 2 * y + 1), px, "({x}, {y})");
                }
            }
        }
        let (x, y) = (last.rect.x + 60, last.rect.bottom() - 4);
        let press = PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: repriseapp::Mods::default(),
        };
        a.main_event(Event::Pointer(press), &mut ctx);
        assert_eq!(a.devices.list.selected, Some(last.entry));
    }

    fn test_ctx() -> Ctx {
        Ctx {
            theme: Theme::default(),
            text: TextRenderer::load(&FontConfig::default()).into(),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: devices::CLIENT,
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

    fn holding_tray(
        a: &mut BtApp,
    ) -> (
        reprisebus::instance::Handoff,
        std::sync::mpsc::Sender<reprisetray::Msg>,
    ) {
        let handoff = reprisebus::instance::Handoff {
            token: Some("t".into()),
            by_key: false,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        a.tray = Some(reprisetray::TrayHandle::holding(handoff.clone()));
        a.msgs = Some(rx);
        (handoff, tx)
    }

    fn offer(transfer: &str) -> (bz::Offer, async_channel::Receiver<bool>) {
        let (done, wait) = async_channel::bounded(1);
        let o = bz::Offer {
            transfer: transfer.into(),
            sender: "Pixel (AA:BB:CC:DD:EE:01)".into(),
            name: "holiday.jpg".into(),
            size: Some(1000),
            done: Some(done),
        };
        (o, wait)
    }

    fn key(sym: u32, alt: bool) -> Event {
        Event::Key(KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods {
                alt,
                ..Default::default()
            },
            edit: alt.then(|| EditKey::Insert(char::from_u32(sym).unwrap_or('?').to_string())),
        })
    }

    #[test]
    fn the_offer_prompt_starts_on_decline_and_accept_answers_yes() {
        let mut ctx = test_ctx();
        let mut a = app();
        let (o, wait) = offer("/t/0");
        a.offer_in(o, &mut ctx);
        assert!(a.children().iter().any(|c| c.id == OFFER));
        armed_offer(&mut a);
        a.offer_event(key(keysym::RETURN, false), &mut ctx);
        assert_eq!(wait.try_recv(), Ok(false), "Enter declines");
        assert!(a.offer.is_none());
        let (o, wait) = offer("/t/1");
        a.offer_in(o, &mut ctx);
        armed_offer(&mut a);
        a.offer_event(key(u32::from('a'), true), &mut ctx);
        assert_eq!(wait.try_recv(), Ok(true), "Alt+A accepts");
    }

    #[test]
    fn the_offer_box_gives_a_screen_reader_its_own_buttons() {
        let mut ctx = test_ctx();
        let mut a = app();
        let (o, _wait) = offer("/t/0");
        a.offer_in(o, &mut ctx);
        let mut tree = Tree::window(receive::TITLE, (300, 150));
        a.access(OFFER, &mut tree, &ctx);
        for button in ["Accept", "Decline"] {
            let named = tree.nodes().any(|(_, n)| n.label() == Some(button));
            assert!(named, "{button}");
        }
    }

    /// The offer box as its window had the keyboard for the consent wait.
    fn armed_offer(a: &mut BtApp) {
        let (_, b) = a.offer.as_mut().expect("an offer box");
        b.consent_focused(std::time::Instant::now() - reprisekit::controls::form::CONSENT_ARM);
    }

    fn bare(ch: char) -> Event {
        Event::Key(KeyInput {
            sym: u32::from(ch),
            utf8: None,
            mods: Default::default(),
            edit: Some(EditKey::Insert(ch.to_string())),
        })
    }

    #[test]
    fn a_bare_letter_or_an_early_key_leaves_the_offer_box_open() {
        let mut ctx = test_ctx();
        let mut a = app();
        let (o, wait) = offer("/t/0");
        a.offer_in(o, &mut ctx);
        a.offer_event(Event::Focus(true), &mut ctx);
        a.offer_event(key(u32::from('a'), true), &mut ctx);
        assert!(
            a.offer.is_some(),
            "Alt+A right after the box took the keyboard"
        );
        armed_offer(&mut a);
        a.offer_event(bare('a'), &mut ctx);
        assert!(a.offer.is_some(), "a bare a");
        assert!(wait.try_recv().is_err(), "unanswered");
    }

    #[test]
    fn a_queued_offer_waits_again_in_the_same_window() {
        let mut ctx = test_ctx();
        let mut a = app();
        let (first, w1) = offer("/t/0");
        let (second, w2) = offer("/t/1");
        a.offer_in(first, &mut ctx);
        a.offer_in(second, &mut ctx);
        armed_offer(&mut a);
        a.offer_event(key(u32::from('d'), true), &mut ctx);
        assert_eq!(w1.try_recv(), Ok(false), "Alt+D declines the first");
        a.offer_event(key(u32::from('a'), true), &mut ctx);
        assert!(w2.try_recv().is_err(), "the second ignores Alt+A at once");
        let Event::Key(k) = key(u32::from('a'), true) else {
            unreachable!()
        };
        a.offer_event(Event::KeyUp(k), &mut ctx);
        armed_offer(&mut a);
        a.offer_event(key(u32::from('a'), true), &mut ctx);
        assert_eq!(w2.try_recv(), Ok(true), "and takes it after the wait");
    }

    #[test]
    fn a_second_offer_waits_and_a_given_up_one_closes() {
        let mut ctx = test_ctx();
        let mut a = app();
        let (first, _w1) = offer("/t/0");
        let (second, w2) = offer("/t/1");
        a.offer_in(first, &mut ctx);
        a.offer_in(second, &mut ctx);
        assert_eq!(a.offer_queue.len(), 1);
        a.offer_gone("/t/0", &mut ctx);
        assert_eq!(
            a.offer.as_ref().map(|(o, _)| o.transfer.as_str()),
            Some("/t/1")
        );
        a.offer_gone("/t/1", &mut ctx);
        assert!(a.offer.is_none());
        assert!(w2.try_recv().is_err(), "closed unanswered declines");
    }

    #[test]
    fn a_received_file_keeps_its_path_for_the_balloon_buttons() {
        let mut a = app();
        holding_tray(&mut a);
        a.file_received(PathBuf::from("/nowhere/a.jpg"), "Pixel".into());
        assert_eq!(a.received.get(&0), Some(&PathBuf::from("/nowhere/a.jpg")));
        assert_eq!(
            tray::translate(reprisetray::Msg::BalloonButton(0, 1), &a.snapshot),
            Some(tray::TrayMsg::Balloon(0, 1))
        );
    }

    #[test]
    fn add_from_the_tray_hands_its_token_to_the_wizard() {
        let mut a = app();
        a.snapshot_in(snap(true), 0);
        let (handoff, tx) = holding_tray(&mut a);
        tx.send(reprisetray::Rows::clicked(&tray::TrayMsg::Add))
            .unwrap();
        assert_eq!(a.wake(&mut test_ctx()).activate, Some((WIZARD, handoff)));
    }

    #[test]
    fn a_tray_row_that_shows_nothing_drops_its_handoff() {
        let mut a = app();
        a.snapshot_in(snap(true), 0);
        let (_, tx) = holding_tray(&mut a);
        tx.send(reprisetray::Rows::clicked(&tray::TrayMsg::TogglePower))
            .unwrap();
        assert_eq!(a.wake(&mut test_ctx()).activate, None);
        assert_eq!(a.tray.as_ref().unwrap().take_handoff(), None);
    }
}
