//! Bluetooth Devices: the Reprise Bluetooth icon and its windows.
//!
//! One process: a tray icon following BlueZ, the Devices sheet, the Add
//! wizard, device Properties, and the Pairing Request box. `--tray` (passed by
//! the autostart entry) starts with the sheet hidden. While the tray icon is
//! up, closing the sheet hides it and only Exit in the tray menu quits.
//!
//! Glue only: the bus thread hands over snapshots and agent requests, the
//! window modules own the models, and this file turns outcomes into
//! `bz::Cmd`s and windows.

mod addwiz;
mod bz;
mod devices;
mod icons;
mod pairdlg;
mod passkey;
mod props;
mod tray;

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::mpsc::Receiver;

use repriseapp::keys::{dialog_key, form_clipboard, form_input, sheet_input};
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{App, BTN_LEFT, Child, Ctx, DropForm, Event, PointerInput, Reply, WinId};
use reprisekit::access::Tree;
use reprisekit::controls::msgbox::Fields;
use reprisekit::controls::{form, propsheet, wizard};
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::textedit::keysym;
use reprisekit::theme::Theme;

/// Child windows; one of each at a time.
const WIZARD: WinId = "wizard";
const PROPS: WinId = "props";
const PAIR: WinId = "pair";
const REMOVE: WinId = "remove";
const ERROR: WinId = "error";

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
    remove: Option<(String, MsgBox)>,
    error: Option<MsgBox>,
    icons: icons::Icons,
    /// Devices with an unfinished Connect not yet shown connected; no second Connect goes out.
    connecting: HashSet<String>,
    /// Devices disconnected since their last Connect; a Connect failing for one shows no box.
    disconnected: HashSet<String>,
    /// Dialog renderer, loaded in `start`: `children` and the wake have no `Ctx` to measure with.
    text: Option<Rc<TextRenderer>>,
    /// The last event's [`Ctx::forms`], which dialogs are built and sized with.
    forms: Theme,
    /// The client size last asked for the main window, which grows with the text size.
    asked: Option<(i32, i32)>,
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
            devices: devices::Devices::new(),
            wizard: None,
            props: None,
            pair: None,
            remove: None,
            error: None,
            icons: icons::Icons::new(),
            connecting: HashSet::new(),
            disconnected: HashSet::new(),
            text: None,
            forms: Theme::default(),
            asked: None,
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

    fn snapshot_in(&mut self, s: bz::Snapshot) {
        log::debug!(
            "snapshot: adapter {}, {} devices",
            s.adapter.as_ref().map_or("none", |a| a.path.as_str()),
            s.devices.len()
        );
        self.snapshot = s;
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
                    p.refresh(&d);
                }
            }
            None => self.props = None,
        }
        // Dropping the box answers its request Canceled.
        if self.snapshot.adapter.is_none() {
            self.pair = None;
        }
        if self.snapshot.adapter.is_none()
            || self
                .remove
                .as_ref()
                .is_some_and(|(p, _)| self.device(p).is_none())
        {
            self.remove = None;
        }
        let devs = &self.snapshot.devices;
        self.connecting
            .retain(|p| devs.iter().any(|d| d.path == *p && !d.connected));
        self.disconnected
            .retain(|p| devs.iter().any(|d| d.path == *p));
        self.push_state();
    }

    /// The wizard takes the request for the device it is adding; any other opens the Pairing Request box.
    fn agent(&mut self, req: bz::AgentRequest) {
        let name = match self.device(&req.device) {
            Some(d) => d.alias.clone(),
            None => {
                let last = req.device.rsplit('/').next().unwrap_or_default();
                last.strip_prefix("dev_").unwrap_or(last).replace('_', ":")
            }
        };
        let (text, forms) = self.text_forms();
        let req = match &mut self.wizard {
            Some(w) => match w.agent(req, &text, &forms) {
                Ok(()) => return,
                Err(r) => r,
            },
            None => req,
        };
        // Replacing an open box drops its request, answering it Canceled.
        self.pair = Some(pairdlg::PairDialog::new(req, &name, &text, &forms));
    }

    /// BlueZ cancelled the pending request held by the wizard or the Pairing Request box. A shown code is never waited on.
    fn agent_cancel(&mut self) {
        let (text, forms) = self.text_forms();
        if let Some(w) = &mut self.wizard
            && w.agent_cancel(&text, &forms)
        {
            return;
        }
        if self.pair.as_ref().is_some_and(|d| d.req.done.is_some()) {
            self.pair = None;
        }
    }

    /// A Connect finished; a failure gets a box unless the user has since disconnected the device.
    fn connected(&mut self, device: String, error: Option<String>) {
        self.connecting.remove(&device);
        let Some(body) = error else {
            return;
        };
        let title = bz::failure_title(&bz::Cmd::Connect(device.clone()));
        if self.disconnected.contains(&device) {
            log::info!("{title} {device} after a Disconnect: {body}");
            return;
        }
        self.failed(title.into(), body);
    }

    fn failed(&mut self, title: String, body: String) {
        log::warn!("{title}: {body}");
        self.error = Some(MsgBox::new(
            title,
            Fields {
                text: body,
                icon: None,
                buttons: vec!["OK".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        ));
    }

    /// Drains the bus thread's and tray's channels. Quits only on the tray's Exit.
    fn wake(&mut self) -> Reply {
        let mut latest = None;
        while let Some(msg) = self.bz_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match msg {
                bz::BzMsg::Snapshot(s) => latest = Some(s),
                bz::BzMsg::Failed { title, body } => self.failed(title, body),
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
                    if let Some(s) = latest.take() {
                        self.snapshot_in(s);
                    }
                    self.agent(req)
                }
                bz::BzMsg::AgentCancel => self.agent_cancel(),
            }
        }
        if let Some(s) = latest {
            self.snapshot_in(s);
        }
        let mut shown = None;
        let mut raised = false;
        while let Some(raw) = self.msgs.as_ref().and_then(|rx| rx.try_recv().ok()) {
            let Some(msg) = tray::translate(raw) else {
                continue;
            };
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

    fn apply_change(&self, path: &str, c: props::Change) {
        if let Some(alias) = c.alias {
            self.send(bz::Cmd::SetAlias {
                device: path.into(),
                alias,
            });
        }
        if let Some(on) = c.trusted {
            self.send(bz::Cmd::SetTrusted {
                device: path.into(),
                on,
            });
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
        self.remove = Some((path, b));
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
                self.props = None;
                Reply::redraw_all()
            }
            props::Outcome::Close => {
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

    /// A device paired from its own side is trusted like the wizard's, or the agent refuses every service it connects.
    fn pair_outcome(&mut self, out: pairdlg::Outcome) -> Reply {
        match out {
            pairdlg::Outcome::None => Reply::none(),
            pairdlg::Outcome::Redraw => Reply::redraw(),
            pairdlg::Outcome::Answered(a) => {
                if let Some(d) = self.pair.take()
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
                Reply::redraw_all()
            }
            // Dropping the box answers an unanswered request Canceled.
            pairdlg::Outcome::Done => {
                self.pair = None;
                Reply::redraw_all()
            }
        }
    }

    fn remove_event(&mut self, ev: Event) -> Reply {
        let (text, forms) = self.text_forms();
        let Some((path, b)) = &mut self.remove else {
            return Reply::none();
        };
        match b.event(&ev, &text, &forms) {
            oneshot::Outcome::None => Reply::none(),
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::Closed(button) => {
                let path = path.clone();
                self.remove = None;
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

    fn error_event(&mut self, ev: Event) -> Reply {
        let (text, forms) = self.text_forms();
        let Some(b) = &mut self.error else {
            return Reply::none();
        };
        match b.event(&ev, &text, &forms) {
            oneshot::Outcome::None => Reply::none(),
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::Closed(_) => {
                self.error = None;
                Reply::redraw_all()
            }
        }
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
        let forms = repriseapp::start_theme().with_form_text(repriseconf::text_size());
        forms.text.size(devices::CLIENT)
    }

    fn fixed(&self) -> bool {
        true
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
                title: "Bluetooth Pairing Request".into(),
                size: u.size(pairdlg::CLIENT),
                fixed: true,
                modal: true,
            });
        }
        if let Some(text) = &self.text {
            if let Some((_, b)) = &self.remove {
                out.push(b.child(REMOVE, text, &self.forms));
            }
            if let Some(b) = &self.error {
                out.push(b.child(ERROR, text, &self.forms));
            }
        }
        out
    }

    fn start(&mut self, waker: repriseapp::Waker) {
        self.fonts();
        // The tray takes the `reprise.bluetooth` bus name; a second instance exits here after raising the first.
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
        let (rx, handle) = bz::watch(waker);
        self.bz_rx = Some(rx);
        self.bz = Some(handle);
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.text = Some(ctx.text.clone());
        self.forms = ctx.forms();
        if matches!(ev, Event::Wake) {
            return self.wake();
        }
        let clip = form_clipboard(self.focused_edit(win), &ev);
        let mut reply = match win {
            WIZARD => self.wizard_event(ev, ctx),
            PROPS => self.props_event(ev, ctx),
            PAIR => self.pair_event(ev, ctx),
            REMOVE => self.remove_event(ev),
            ERROR => self.error_event(ev),
            _ => self.main_event(ev, ctx),
        }
        .with_clipboard(clip);
        let size = self.forms.text.size(devices::CLIENT);
        if self.asked != Some(size) {
            self.asked = Some(size);
            reply.size = Some(size);
            reply.redraw = true;
        }
        reply
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
            REMOVE => {
                if let Some((_, b)) = &self.remove {
                    b.access(tree, &ctx.text, &forms);
                }
            }
            ERROR => {
                if let Some(b) = &self.error {
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
            REMOVE => {
                if let Some((_, b)) = &self.remove {
                    b.draw(c, &forms, &ctx.text);
                }
            }
            ERROR => {
                if let Some(b) = &self.error {
                    b.draw(c, &forms, &ctx.text);
                }
            }
            _ => self.devices.draw(c, ctx, &self.icons),
        }
    }
}

fn main() {
    reprisebus::journal::init_logger("error");
    let tray_only = std::env::args().any(|a| a == "--tray");
    repriseapp::run(Box::new(BtApp::new(tray_only)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bz::{Adapter, Ask, Device, Kind};
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
        }
    }

    #[test]
    fn adapter_loss_closes_props_and_the_pairing_box_and_fails_the_wizard() {
        let mut a = app();
        a.snapshot_in(snap(true));
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
        a.snapshot_in(bz::Snapshot::default());
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
        a.snapshot_in(snap(true));
        a.open_props("/a/cans");
        assert!(a.props.is_some());
        let mut s = snap(true);
        s.devices.clear();
        a.snapshot_in(s);
        assert!(a.props.is_none());
    }

    #[test]
    fn the_properties_sheet_takes_drops_on_its_open_page() {
        let mut a = app();
        a.snapshot_in(snap(true));
        assert!(a.drop_form(PROPS).is_none());
        a.open_props("/a/cans");
        let d = a.drop_form(PROPS).expect("the open sheet's page");
        assert!(d.form.item(props::NAME).is_some());
        assert!(a.drop_form(MAIN).is_none());
    }

    #[test]
    fn agent_requests_route_to_the_wizard_or_the_dialog() {
        let mut a = app();
        a.snapshot_in(snap(true));
        let (d1, w1) = async_channel::bounded(1);
        a.agent(bz::AgentRequest {
            device: "/a/cans".into(),
            ask: Ask::Authorize,
            done: Some(d1),
        });
        assert!(a.pair.is_some(), "no wizard: the dialog opens");
        let (d2, _w2) = async_channel::bounded(1);
        a.agent(bz::AgentRequest {
            device: "/a/cans".into(),
            ask: Ask::Confirm(1),
            done: Some(d2),
        });
        assert!(
            w1.try_recv().is_err() && w1.is_closed(),
            "the first request was dropped unanswered"
        );
    }

    #[test]
    fn bluez_cancel_closes_a_waiting_prompt_only() {
        let mut a = app();
        a.snapshot_in(snap(true));
        a.agent(bz::AgentRequest {
            device: "/a/cans".into(),
            ask: Ask::ShowPasskey(123456),
            done: None,
        });
        a.agent_cancel();
        assert!(a.pair.is_some(), "a shown code stays up");
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
        a.snapshot_in(snap(true));
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
            a.error.is_none(),
            "a Connect that went through shows no box"
        );
        a.toggle("/a/cans");
        assert_eq!(connects(&rx), 1, "finished, so the next one is sent");
    }

    #[test]
    fn a_connect_failing_after_a_disconnect_shows_no_box() {
        let mut a = app();
        a.snapshot_in(snap(true));
        a.toggle("/a/cans");
        let mut s = snap(true);
        s.devices = vec![cans(true)];
        a.snapshot_in(s);
        a.toggle("/a/cans");
        a.connected("/a/cans".into(), Some("canceled".into()));
        assert!(a.error.is_none(), "the user asked to disconnect it");

        let mut a = app();
        a.snapshot_in(snap(true));
        a.toggle("/a/cans");
        a.connected("/a/cans".into(), Some("timed out".into()));
        assert_eq!(
            a.error.as_ref().map(|b| b.title.as_str()),
            Some("Could not connect"),
            "a Connect failing on its own shows a box"
        );
    }

    #[test]
    fn a_vanished_device_closes_its_remove_box() {
        let mut a = app();
        a.snapshot_in(snap(true));
        let mut ctx = test_ctx();
        a.ask_remove("/a/cans".into(), &mut ctx);
        assert!(a.remove.is_some());
        let mut s = snap(true);
        s.devices.clear();
        a.snapshot_in(s);
        assert!(a.remove.is_none());
    }

    #[test]
    fn accepting_the_pairing_request_trusts_the_device() {
        let (mut a, rx) = wired();
        let trusted = |rx: &async_channel::Receiver<bz::Cmd>| {
            std::iter::from_fn(|| rx.try_recv().ok())
                .filter(|c| {
                    matches!(c, bz::Cmd::SetTrusted { device, on: true } if device == "/a/cans")
                })
                .count()
        };
        let (done, _wait) = async_channel::bounded(1);
        a.agent(bz::AgentRequest {
            device: "/a/cans".into(),
            ask: Ask::Confirm(7),
            done: Some(done),
        });
        let out = a.pair.as_mut().unwrap().event(form::Event::Default);
        a.pair_outcome(out);
        assert!(a.pair.is_none());
        assert_eq!(trusted(&rx), 1, "accepted: trusted");

        let (done, _wait) = async_channel::bounded(1);
        a.agent(bz::AgentRequest {
            device: "/a/cans".into(),
            ask: Ask::Confirm(7),
            done: Some(done),
        });
        let out = a.pair.as_mut().unwrap().event(form::Event::Cancel);
        a.pair_outcome(out);
        assert!(a.pair.is_none());
        assert_eq!(trusted(&rx), 0, "refused: not trusted");
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
        a.snapshot_in(s);
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
        a.snapshot_in(snap(true));
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
        a.snapshot_in(snap(true));
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
            props::edit_text(p.sheet.open(), "name")
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
        a.snapshot_in(snap(true));
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
        a.snapshot_in(s);
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
        a.snapshot_in(s);
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

    #[test]
    fn add_from_the_tray_hands_its_token_to_the_wizard() {
        let mut a = app();
        a.snapshot_in(snap(true));
        let (handoff, tx) = holding_tray(&mut a);
        tx.send(reprisetray::Msg::Clicked(tray::ID_ADD)).unwrap();
        assert_eq!(a.wake().activate, Some((WIZARD, handoff)));
    }

    #[test]
    fn a_tray_row_that_shows_nothing_drops_its_handoff() {
        let mut a = app();
        a.snapshot_in(snap(true));
        let (_, tx) = holding_tray(&mut a);
        tx.send(reprisetray::Msg::Clicked(tray::ID_POWER)).unwrap();
        assert_eq!(a.wake().activate, None);
        assert_eq!(a.tray.as_ref().unwrap().take_handoff(), None);
    }
}
