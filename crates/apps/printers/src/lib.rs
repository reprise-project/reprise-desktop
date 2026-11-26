//! reprise-printers: the CUPS Printers folder, queues, Properties, Add Printer Wizard and balloons,
//! with a tray icon while documents wait (`--tray` starts with the folder hidden). cupsd is asked
//! over IPP on its local socket; an account in cupsd's SystemGroup changes printers on its peer
//! credentials, any other through cups-pk-helper and polkit. Everything here is glue between the
//! worker thread and the window modules.

#[cfg(test)]
reprisebus::test_session!();

mod addwiz;
mod avahi;
mod cups;
mod folder;
mod http;
mod ipp;
mod lpwatch;
mod notifier;
mod pkhelper;
mod ppd;
mod props;
mod queue;
mod tray;
mod wins;
mod worker;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::Receiver;

use repriseapp::asks::{Answer, Asks};
use repriseapp::filedlg::{Driven, Host};
use repriseapp::keys::{form_clipboard, form_input, sheet_input};
use repriseapp::oneshot::{self, ErrorBox, MsgBox};
use repriseapp::{
    App, BTN_LEFT, Child, Ctx, DropForm, Event, FileDrop, MAIN, PointerInput, Reply, WinId,
};
use reprisekit::access::Tree;
use reprisekit::controls::filedlg::{DlgMode, Fields as FileFields, FileDlg};
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::msgbox::Fields;
use reprisekit::controls::{form, propsheet, wizard};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

use cups::{JobState, PrinterState, Snapshot};
use wins::{Ids, Opened, Wins};
use worker::{Cmd, Msg};

const WIZARD: WinId = WinId::named("wizard");
const BROWSE: WinId = WinId::named("browse");

const PRINTER_ICON: &str = "printer";

/// What a confirm box asks before it is sent.
enum Ask {
    Delete(String),
    Rename { old: String, new: String },
}

impl Ask {
    fn printer(&self) -> &str {
        match self {
            Ask::Delete(p) | Ask::Rename { old: p, .. } => p,
        }
    }
}

#[derive(Default)]
struct PrintersApp {
    tray: Option<reprisetray::TrayHandle>,
    msgs: Option<Receiver<reprisetray::Msg>>,
    rx: Option<Receiver<Msg>>,
    worker: Option<worker::Handle>,
    snapshot: Snapshot,
    user: String,
    tray_state: Option<reprisetray::Shown>,
    folder: folder::Folder,
    queues: Wins<queue::Queue>,
    props: Wins<props::PropsSheet>,
    /// A sheet opened from a queue window, with that window: it covers it and closes with it.
    /// Any other sheet is the folder's.
    sheet_over: HashMap<WinId, WinId>,
    ids: Ids,
    wizard: Option<addwiz::AddWizard>,
    browse: Option<(Host, FileDlg)>,
    confirm: Asks<Ask>,
    note: ErrorBox,
    icon16: Option<Canvas>,
    icon32: Option<Canvas>,
    doc_icon: Option<Canvas>,
    icons_read: bool,
    refit: bool,
    refit_queues: HashSet<WinId>,
    hidden: bool,
    tray_only: bool,
    watching: bool,
    balloons: Vec<String>,
    announced: HashSet<String>,
    select: Option<String>,
    /// Printers cupsd renamed whose new name a snapshot has yet to show: their windows stay.
    renaming: HashSet<String>,
    /// A renamed printer whose Ports tab opens once cupsd has it, to type its password again.
    ports_after: Option<String>,
    text: Option<Rc<TextRenderer>>,
    /// `Ctx::forms` at the last event, for `children` and `initial_size`, which have no `Ctx`.
    forms: Theme,
}

impl PrintersApp {
    fn new(tray_only: bool) -> PrintersApp {
        PrintersApp {
            hidden: tray_only,
            tray_only,
            user: cups::Client::new().user,
            forms: repriseapp::start_forms(),
            ..PrintersApp::default()
        }
    }

    fn fonts(&mut self) {
        self.text = Some(repriseapp::start_text());
    }

    fn send(&self, cmd: Cmd) {
        if let Some(w) = &self.worker {
            w.send(cmd);
        }
    }

    fn push_tray(&mut self) {
        let next = tray::shown(&self.snapshot, &self.user);
        if self.tray_state.as_ref() == Some(&next) {
            return;
        }
        if let Some(t) = &self.tray {
            t.set_state(next.clone());
        }
        self.tray_state = Some(next);
    }

    fn sync_watching(&mut self) {
        let on = !self.hidden
            || !self.queues.is_empty()
            || !self.props.is_empty()
            || self.wizard.is_some();
        if on != self.watching {
            self.watching = on;
            self.send(Cmd::Watching(on));
        }
    }

    fn balloon(&mut self, summary: &str, body: &str, printer: &str) {
        let Some(t) = &self.tray else {
            return;
        };
        let tag = self.balloons.len() as u32;
        self.balloons.push(printer.to_string());
        t.notify_clickable(summary, body, tag);
    }

    /// A notice about `printer` covers its Properties sheet, else its queue, else the folder.
    fn notify(&mut self, printer: Option<&str>, title: &str, body: String) {
        let over = printer.and_then(|p| self.props.of(p).or_else(|| self.queues.of(p)));
        self.note.show_over(over.unwrap_or(MAIN), title, body, None);
    }

    /// Closes the sheets over a queue window that closed, then takes the notices over a sheet or
    /// queue window that closed.
    fn withdraw_orphans(&mut self) {
        let (queues, props) = (&self.queues, &self.props);
        let mut orphans = Vec::new();
        self.sheet_over
            .retain(|&sheet, queue| match queues.get(*queue) {
                None => {
                    orphans.push(sheet);
                    false
                }
                Some(_) => props.get(sheet).is_some(),
            });
        for &sheet in &orphans {
            self.props.close(sheet);
        }
        if !orphans.is_empty() {
            self.sync_watching();
        }
        let (queues, props) = (&self.queues, &self.props);
        self.note
            .withdraw(|w| w != MAIN && queues.get(w).is_none() && props.get(w).is_none());
    }

    fn snapshot_in(&mut self, s: Snapshot) {
        for p in &s.printers {
            let broken = p.state == PrinterState::Stopped && !p.paused();
            let mine = s.jobs_of(&p.name).any(|j| j.owner == self.user);
            if broken && mine && self.announced.insert(p.name.clone()) {
                let body = if p.message.is_empty() {
                    format!("{} has a problem: {}.", p.name, p.status_word())
                } else {
                    format!("{}: {}", p.name, p.message)
                };
                let name = p.name.clone();
                self.balloon("Printer problem", &body, &name);
            }
            if !broken {
                self.announced.remove(&p.name);
            }
        }
        self.announced
            .retain(|n| s.printers.iter().any(|p| &p.name == n));
        self.snapshot = s;
        let snap = &self.snapshot;
        self.renaming.retain(|n| snap.printer(n).is_none());
        if snap.running {
            self.confirm
                .withdraw(|ask| snap.printer(ask.printer()).is_none());
        }
        self.folder.refresh(&self.snapshot, self.icon16.as_ref());
        self.refit = true;
        let (snap, renaming) = (&self.snapshot, &self.renaming);
        self.queues.retain(|q| {
            let keep = snap.printer(&q.printer).is_some()
                || !snap.running
                || renaming.contains(&q.printer);
            if keep {
                q.refresh(snap);
            }
            keep
        });
        let queues = &self.queues;
        self.refit_queues.retain(|id| queues.get(*id).is_some());
        self.props.retain(|p| match snap.printer(&p.printer) {
            Some(pr) => {
                p.refresh(pr, false);
                true
            }
            None => !snap.running || renaming.contains(&p.printer),
        });
        self.withdraw_orphans();
        self.push_tray();
    }

    fn job_ended(&mut self, j: cups::Job) {
        if j.owner != self.user {
            return;
        }
        let name = if j.name.is_empty() {
            "Untitled".to_string()
        } else {
            j.name.clone()
        };
        match j.state {
            JobState::Completed => self.balloon(
                "Document printed",
                &format!("{name} was printed on {}.", j.printer),
                &j.printer,
            ),
            JobState::Aborted => self.balloon(
                "Document not printed",
                &format!("{name} could not be printed on {}.", j.printer),
                &j.printer,
            ),
            _ => {}
        }
    }

    fn wake(&mut self) -> Reply {
        let mut latest = None;
        let mut ended = Vec::new();
        while let Some(m) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match m {
                Msg::Snapshot(s) => latest = Some(s),
                Msg::JobEnded(j) => ended.push(j),
                Msg::Failed {
                    title,
                    body,
                    printer,
                } => self.notify(printer.as_deref(), &title, body),
                Msg::Renamed { old, new, made } => self.renamed(old, new, made),
                Msg::Options(name, r) => {
                    let printer = self.snapshot.printer(&name).cloned();
                    if let (Some(p), Some(pr)) = (self.props.by_printer_mut(&name), printer) {
                        match r {
                            Ok(o) => p.set_options(o, &pr),
                            Err(e) => p.options_failed(&e.text()),
                        }
                    }
                }
                Msg::Devices(r) => {
                    let out = self.wizard.as_mut().map(|w| w.devices_in(r));
                    self.search(out);
                }
                Msg::Network(r) => {
                    let out = self.wizard.as_mut().map(|w| w.network_in(r));
                    self.search(out);
                }
                Msg::Ppds(r) => {
                    if let Some(w) = &mut self.wizard {
                        w.ppds_in(r, &self.snapshot);
                    }
                }
                Msg::Added(Ok(name)) => {
                    self.close_wizard();
                    self.hidden = false;
                    self.select = Some(name);
                }
                Msg::Added(Err(e)) => {
                    if let Some(w) = &mut self.wizard {
                        w.add_failed(&e.text(), &self.snapshot);
                    }
                }
                Msg::TestPage(p) => {
                    let body = format!("A test page has been sent to {p}.");
                    self.notify(Some(&p), "Print Test Page", body);
                }
            }
        }
        if let Some(s) = latest {
            self.snapshot_in(s);
        }
        for j in ended {
            self.job_ended(j);
        }
        let mut shown = None;
        let mut raised = false;
        let ports = self
            .ports_after
            .take_if(|n| self.snapshot.printer(n).is_some())
            .and_then(|n| self.open_props(&n, props::PORTS))
            .and_then(Opened::raise);
        while let Some(raw) = self.msgs.as_ref().and_then(|rx| rx.try_recv().ok()) {
            let Some(msg) = tray::translate(raw, &self.snapshot, &self.user) else {
                continue;
            };
            raised = true;
            match msg {
                tray::TrayMsg::Folder => {
                    self.hidden = false;
                    shown = Some(repriseapp::MAIN);
                }
                tray::TrayMsg::Open => {
                    match tray::busy_printers(&self.snapshot, &self.user)
                        .first()
                        .cloned()
                    {
                        Some(p) => shown = Some(self.open_queue(&p).id()),
                        None => {
                            self.hidden = false;
                            shown = Some(repriseapp::MAIN);
                        }
                    }
                }
                tray::TrayMsg::Queue(i) => {
                    if let Some(p) = tray::busy_printers(&self.snapshot, &self.user)
                        .get(i)
                        .cloned()
                    {
                        shown = Some(self.open_queue(&p).id());
                    }
                }
                tray::TrayMsg::Balloon(tag) => {
                    if let Some(p) = self.balloons.get(tag as usize).cloned() {
                        shown = Some(self.open_queue(&p).id());
                    }
                }
            }
        }
        self.sync_watching();
        Reply {
            activate: reprisetray::handoff_for(self.tray.as_ref(), raised, shown),
            raise: ports,
            ..Reply::redraw_all()
        }
    }

    fn open_queue(&mut self, printer: &str) -> Opened {
        let (icon, snap) = (&self.doc_icon, &self.snapshot);
        let opened = self.queues.open(printer, &mut self.ids, || {
            let mut q = queue::Queue::new(printer);
            q.icon = icon.clone().map(Rc::new);
            q.refresh(snap);
            q
        });
        if let Opened::New(id) = opened {
            self.refit_queues.insert(id);
            self.send(Cmd::Refresh);
        }
        opened
    }

    fn open_props(&mut self, printer: &str, tab: usize) -> Option<Opened> {
        if let Some(p) = self.props.by_printer_mut(printer) {
            p.open_tab(tab);
            return self.props.of(printer).map(Opened::Already);
        }
        let pr = self.snapshot.printer(printer)?;
        let icon = &self.icon32;
        let opened = self.props.open(printer, &mut self.ids, || {
            let mut p = props::PropsSheet::new(pr, icon.clone());
            p.open_tab(tab);
            p
        });
        if let Opened::New(_) = opened {
            self.send(Cmd::Options(printer.into()));
        }
        Some(opened)
    }

    fn open_wizard(&mut self) {
        if self.wizard.is_none() {
            self.wizard = Some(addwiz::AddWizard::new(&self.snapshot, self.icon32.clone()));
            self.send(Cmd::Network(true));
            self.send(Cmd::Devices { prompt: false });
        }
    }

    fn close_wizard(&mut self) {
        self.wizard = None;
        self.browse = None;
        self.send(Cmd::Network(false));
    }

    fn search(&self, out: Option<addwiz::Outcome>) {
        if let Some(addwiz::Outcome::Search { prompt }) = out {
            self.send(Cmd::Devices { prompt });
        }
    }

    /// Rename `old` to `new`, at `device_uri` when one was typed in the same Apply. Without one,
    /// an address that may hold a password is asked about first, over window `from`: the new
    /// queue cannot copy it.
    fn start_rename(
        &mut self,
        old: String,
        new: String,
        device_uri: Option<String>,
        from: WinId,
        ctx: &mut Ctx,
    ) {
        let hidden = self
            .snapshot
            .printer(&old)
            .is_some_and(|p| cups::may_hide_password(&p.device_uri));
        if device_uri.is_some() || !hidden {
            return self.rename(old, new, device_uri);
        }
        let b = MsgBox::new(
            "Rename Printer",
            Fields {
                text: format!(
                    "Renaming {old} to {new} drops the user name and password from its address. Properties then opens on the Ports tab to type the address again.\nDo you want to rename the printer?"
                ),
                icon: ctx.icons.get("dialog-question", 32),
                buttons: vec!["&Yes".into(), "&No".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        );
        self.confirm.ask_over(from, b, Ask::Rename { old, new });
    }

    fn rename(&self, old: String, new: String, device_uri: Option<String>) {
        self.send(Cmd::Rename {
            old,
            new,
            device_uri: device_uri.map(cups::DeviceUri),
        });
    }

    /// The windows of `old` take the name `new` once cupsd `made` it; else a sheet's name field
    /// goes back to `old`.
    fn renamed(&mut self, old: String, new: String, made: bool) {
        if !made {
            if self.ports_after.as_ref() == Some(&new) {
                self.ports_after = None;
            }
            if let Some(p) = self.props.by_printer_mut(&old) {
                p.rename_failed(&new);
            }
            return;
        }
        self.queues.rename(&old, &new);
        self.props.rename(&old, &new);
        if self.folder.selected_name() == Some(old.as_str()) {
            self.select = Some(new.clone());
        }
        self.renaming.insert(new);
    }

    fn ask_delete(&mut self, printer: String, ctx: &mut Ctx) {
        let b = MsgBox::new(
            "Delete Printer",
            Fields {
                text: format!(
                    "Are you sure you want to delete the printer {printer}?\nDocuments waiting on it are cancelled."
                ),
                icon: ctx.icons.get("dialog-question", 32),
                buttons: vec!["&Yes".into(), "&No".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        );
        self.confirm.ask(b, Ask::Delete(printer));
    }

    fn quit(&mut self) -> Reply {
        Reply {
            quit: true,
            ..Reply::none()
        }
    }

    fn close_main(&mut self) -> Reply {
        if self.tray.is_none() {
            return self.quit();
        }
        self.hidden = true;
        self.sync_watching();
        Reply::redraw()
    }

    fn read_icons(&mut self, ctx: &mut Ctx) {
        if self.icons_read {
            return;
        }
        self.icons_read = true;
        self.icon16 = ctx
            .icons
            .get(PRINTER_ICON, 16)
            .or_else(|| ctx.icons.get("document-print", 16));
        self.icon32 = ctx
            .icons
            .get(PRINTER_ICON, 32)
            .or_else(|| ctx.icons.get("document-print", 32));
        self.doc_icon = ctx
            .icons
            .get("text-x-generic", 16)
            .or_else(|| ctx.icons.get("x-office-document", 16));
        self.folder.refresh(&self.snapshot, self.icon16.as_ref());
        self.refit = true;
    }

    fn folder_outcome(&mut self, out: folder::Outcome, ctx: &mut Ctx) -> Reply {
        use folder::Outcome as O;
        match out {
            O::None => Reply::none(),
            O::Redraw => Reply::redraw(),
            O::Add => {
                self.open_wizard();
                Reply::redraw_all()
            }
            O::Refresh => {
                self.send(Cmd::Refresh);
                Reply::redraw()
            }
            O::Open(p) => {
                let raise = self.open_queue(&p).raise();
                Reply {
                    raise,
                    ..Reply::redraw_all()
                }
            }
            O::SetDefault(p) => {
                self.send(Cmd::SetDefault(p));
                Reply::redraw()
            }
            O::Pause(p) => {
                self.send(Cmd::Pause(p));
                Reply::redraw()
            }
            O::Resume(p) => {
                self.send(Cmd::Resume(p));
                Reply::redraw()
            }
            O::CancelAll(p) => {
                self.send(Cmd::CancelAll(p));
                Reply::redraw()
            }
            O::Sharing(p) => self.props_opened(&p, props::SHARING, MAIN),
            O::Properties(p) => self.props_opened(&p, props::GENERAL, MAIN),
            O::Delete(p) => {
                self.ask_delete(p, ctx);
                Reply::redraw_all()
            }
            O::Rename { old, new } => {
                self.start_rename(old, new, None, MAIN, ctx);
                Reply::redraw_all()
            }
            O::BadName(why) => {
                self.notify(None, "Rename", why);
                Reply::redraw_all()
            }
        }
    }

    /// Opens or raises `printer`'s sheet from window `from`. A sheet opened from a queue window
    /// stays that window's while it is open, wherever it is raised from later.
    fn props_opened(&mut self, printer: &str, tab: usize, from: WinId) -> Reply {
        let opened = self.open_props(printer, tab);
        if let Some(Opened::New(sheet)) = opened
            && from != MAIN
        {
            self.sheet_over.insert(sheet, from);
        }
        let raise = opened.and_then(Opened::raise);
        Reply {
            raise,
            ..Reply::redraw_all()
        }
    }

    fn main_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let s = &self.snapshot;
        let out = match ev {
            Event::CloseRequest => return self.close_main(),
            Event::Resized => {
                self.refit = true;
                return Reply::redraw();
            }
            Event::Blink if self.folder.editing() => return Reply::redraw(),
            Event::Key(k) => self.folder.key(&k, s, ctx),
            Event::Pointer(PointerInput::Press {
                x, y, button, time, ..
            }) => self
                .folder
                .press(ctx.theme.metrics.click(x, y, time), button, s, ctx),
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

    fn queue_event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let s = &self.snapshot;
        let Some(q) = self.queues.get_mut(win) else {
            return Reply::none();
        };
        let out = match ev {
            Event::CloseRequest => queue::Outcome::Close,
            Event::Resized => {
                self.refit_queues.insert(win);
                return Reply::redraw();
            }
            Event::Key(k) => q.key(&k, s, ctx),
            Event::KeyUp(k) => q.key_up(&k, ctx),
            Event::Pointer(PointerInput::Press {
                x, y, button, time, ..
            }) => {
                if button == BTN_LEFT && q.on_grip(x, y, ctx) {
                    return Reply::resize();
                }
                q.press(ctx.theme.metrics.click(x, y, time), button, s, ctx)
            }
            Event::Pointer(PointerInput::Motion { x, y }) => q.motion(x, y, ctx),
            Event::Pointer(PointerInput::Release { .. }) => q.release(ctx),
            Event::Pointer(PointerInput::Leave) => q.leave(ctx),
            Event::Focus(false) => q.blur(),
            Event::Pointer(PointerInput::Scroll { rows, .. }) => q.wheel(rows, ctx),
            Event::Pointer(PointerInput::ScrollAcross { cols, .. }) => q.wheel_across(cols, ctx),
            _ => return Reply::none(),
        };
        self.queue_outcome(win, out, ctx)
    }

    fn queue_outcome(&mut self, win: WinId, out: queue::Outcome, ctx: &mut Ctx) -> Reply {
        let Some(printer) = self.queues.get(win).map(|q| q.printer.clone()) else {
            return Reply::none();
        };
        use queue::Outcome as O;
        match out {
            O::None => Reply::none(),
            O::Redraw => Reply::redraw(),
            O::Close => {
                self.queues.close(win);
                self.refit_queues.remove(&win);
                self.sync_watching();
                Reply::redraw_all()
            }
            O::Job(id, op) => {
                self.send(Cmd::Job { id, op });
                Reply::redraw()
            }
            O::SetDefault => self.folder_outcome(folder::Outcome::SetDefault(printer), ctx),
            O::Pause => self.folder_outcome(folder::Outcome::Pause(printer), ctx),
            O::Resume => self.folder_outcome(folder::Outcome::Resume(printer), ctx),
            O::CancelAll => self.folder_outcome(folder::Outcome::CancelAll(printer), ctx),
            O::Sharing => self.props_opened(&printer, props::SHARING, win),
            O::Properties => self.props_opened(&printer, props::GENERAL, win),
        }
    }

    fn props_event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(p) = self.props.get_mut(win) else {
            return Reply::none();
        };
        let out = match ev {
            Event::CloseRequest => props::Outcome::Close,
            Event::Blink => {
                p.sheet.open_mut().blink(ctx.caret_on);
                return Reply::redraw();
            }
            Event::Pasted(t) => p.paste(t, &self.snapshot),
            ev => match sheet_input(&mut p.sheet, &ev, ctx) {
                Some(e) => p.event(e, &self.snapshot),
                None => return Reply::none(),
            },
        };
        let printer = p.printer.clone();
        match out {
            props::Outcome::None => Reply::none(),
            props::Outcome::Redraw => Reply::redraw(),
            props::Outcome::TestPage => {
                self.send(Cmd::TestPage(printer));
                Reply::redraw()
            }
            props::Outcome::Apply {
                change,
                rename,
                close,
            } => {
                let typed_uri = change.device_uri.clone();
                if !change.is_empty() {
                    self.send(Cmd::Modify {
                        printer: printer.clone(),
                        change,
                    });
                }
                if let Some(new) = rename {
                    self.start_rename(printer, new, typed_uri, win, ctx);
                }
                if close {
                    self.props.close(win);
                    self.sync_watching();
                }
                Reply::redraw_all()
            }
            props::Outcome::BadName(why) => {
                self.notify(Some(&printer), "Rename", why);
                Reply::redraw_all()
            }
            props::Outcome::Close => {
                self.props.close(win);
                self.sync_watching();
                Reply::redraw_all()
            }
        }
    }

    fn wizard_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let s = &self.snapshot;
        let Some(w) = self.wizard.as_mut() else {
            return Reply::none();
        };
        let out = match ev {
            Event::CloseRequest => addwiz::Outcome::Close,
            Event::Blink => {
                w.page.form.blink(ctx.caret_on);
                return Reply::redraw();
            }
            Event::Pasted(t) => {
                let e = w.page.form.paste(t);
                w.event(e, s)
            }
            ev => match form_input(&mut w.page.form, &ev, ctx) {
                Some(e) => w.event(e, s),
                None => return Reply::none(),
            },
        };
        match out {
            addwiz::Outcome::None => Reply::none(),
            addwiz::Outcome::Redraw => Reply::redraw(),
            addwiz::Outcome::Search { prompt } => {
                self.send(Cmd::Devices { prompt });
                Reply::redraw()
            }
            addwiz::Outcome::Drivers => {
                self.send(Cmd::Ppds);
                Reply::redraw()
            }
            addwiz::Outcome::HaveDisk => {
                let mut host = Host::default();
                host.icons.prime(&mut ctx.icons);
                let start = std::env::var_os("HOME")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| "/".into());
                let dlg = host.open_last(
                    Path::new(&start),
                    FileFields {
                        title: "Have Disk".into(),
                        accept_label: "Open".into(),
                        mode: DlgMode::Open,
                        multiple: false,
                        filters: vec!["Printer Drivers (*.ppd)".into(), "All Files (*.*)".into()],
                    },
                    ctx.forms().text,
                );
                self.browse = Some((host, dlg));
                Reply::redraw_all()
            }
            addwiz::Outcome::Add {
                printer,
                default,
                test_page,
            } => {
                self.send(Cmd::Add {
                    printer,
                    default,
                    test_page,
                });
                Reply::redraw()
            }
            addwiz::Outcome::Close => {
                self.close_wizard();
                self.sync_watching();
                Reply::redraw_all()
            }
        }
    }

    fn browse_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some((host, d)) = self.browse.as_mut() else {
            return Reply::none();
        };
        let (text, forms) = (ctx.text.clone(), ctx.forms());
        match host.drive(d, &ev, &text, &forms, ctx) {
            Driven::Reply(r) => *r,
            Driven::Cancelled => {
                self.browse = None;
                Reply::redraw_all()
            }
            Driven::Picked(path) => {
                self.browse = None;
                if let Some(w) = &mut self.wizard {
                    w.file_in(path, &self.snapshot);
                }
                Reply::redraw_all()
            }
        }
    }

    fn confirm_event(&mut self, win: WinId, ev: Event, ctx: &Ctx) -> Reply {
        match self.confirm.event(win, &ev, &ctx.text, &self.forms) {
            Answer::None => Reply::none(),
            Answer::Redraw => Reply::redraw(),
            Answer::Closed(ask, button) => {
                match ask {
                    Ask::Delete(printer) if button == Some(0) => self.send(Cmd::Delete(printer)),
                    Ask::Rename { old, new } if button == Some(0) => {
                        self.ports_after = Some(new.clone());
                        self.rename(old, new, None);
                    }
                    _ => {}
                }
                Reply::redraw_all()
            }
        }
    }

    fn focused_edit(&self, win: WinId) -> Option<form::FocusedEdit<'_>> {
        match win {
            WIZARD => self.wizard.as_ref()?.page.form.focused_edit(),
            _ => self.props.get(win)?.sheet.open().focused_edit(),
        }
    }
}

impl App for PrintersApp {
    fn title(&self) -> String {
        tray::TITLE.into()
    }

    fn app_id(&self) -> String {
        "reprise.printers".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.forms.text.size(folder::CLIENT)
    }

    fn children(&self) -> Vec<Child> {
        let mut out = Vec::new();
        for (id, q) in self.queues.iter() {
            out.push(Child {
                id,
                title: q.title(&self.snapshot),
                size: self.forms.text.size(queue::CLIENT),
                fixed: false,
                modal: false,
            });
        }
        for (id, p) in self.props.iter() {
            out.push(Child {
                id,
                title: p.window_title(),
                size: self.forms.text.size(props::CLIENT),
                fixed: true,
                modal: false,
            });
        }
        if self.wizard.is_some() {
            out.push(Child {
                id: WIZARD,
                title: "Add Printer Wizard".into(),
                size: self.forms.text.size(wizard::CLIENT),
                fixed: true,
                modal: false,
            });
        }
        if let Some((_, d)) = &self.browse {
            out.push(Child {
                id: BROWSE,
                title: "Have Disk".into(),
                size: d.client(),
                fixed: false,
                modal: true,
            });
        }
        if let Some(text) = &self.text {
            out.extend(self.confirm.child(text, &self.forms));
            out.extend(self.note.child(text, &self.forms));
        }
        out
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        self.fonts();
        let first = tray::shown(&self.snapshot, &self.user);
        match tray::start(waker.clone(), first.clone()).or_exit(self.tray_only) {
            Ok(Some((t, msgs))) => {
                self.tray = Some(t);
                self.msgs = Some(msgs);
                self.tray_state = Some(first);
            }
            Ok(None) => {}
            Err(code) => return repriseapp::Start::Exit(code),
        }
        let (rx, handle) = worker::start(waker);
        self.rx = Some(rx);
        self.worker = Some(handle);
        self.sync_watching();
        repriseapp::Start::Continue
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.text = Some(ctx.text.clone());
        self.forms = ctx.forms();
        if matches!(ev, Event::Wake) {
            return self.wake();
        }
        self.read_icons(ctx);
        if let Some(icon) = &self.doc_icon {
            for q in self.queues.values_mut().filter(|q| q.icon.is_none()) {
                q.icon = Some(Rc::new(icon.clone()));
                q.refresh(&self.snapshot);
            }
        }
        let clip = form_clipboard(self.focused_edit(win), &ev);
        let reply = match win {
            _ if self.queues.get(win).is_some() => self.queue_event(win, ev, ctx),
            _ if self.props.get(win).is_some() => self.props_event(win, ev, ctx),
            WIZARD => self.wizard_event(ev, ctx),
            BROWSE => self.browse_event(ev, ctx),
            w if self.confirm.owns(w) => self.confirm_event(w, ev, ctx),
            w if self.note.owns(w) => self.note.event(w, &ev, &ctx.text, &self.forms),
            _ => self.main_event(ev, ctx),
        };
        self.withdraw_orphans();
        reply.with_clipboard(clip)
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.text = Some(ctx.text.clone());
        let forms = ctx.forms();
        let px = forms.fonts.size_px;
        let paint = Paint {
            s: 1,
            ..forms.paint()
        };
        match win {
            _ if let Some(q) = self.queues.get_mut(win) => {
                if self.refit_queues.remove(&win) {
                    q.fit_columns(ctx);
                }
                q.draw(c, ctx);
            }
            _ if let Some(p) = self.props.get(win) => {
                c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
                propsheet::draw(c, &ctx.text, &paint, &p.sheet, true, px);
            }
            WIZARD => {
                if let Some(w) = &self.wizard {
                    w.draw(c, &paint, &ctx.text, px);
                }
            }
            BROWSE => {
                if let Some((_, d)) = &self.browse {
                    let (w, h) = d.client();
                    let paint = Paint {
                        s: 1,
                        ..forms.paint()
                    };
                    d.draw(
                        c,
                        &ctx.text,
                        &paint,
                        Rect::new(0, 0, w, h),
                        forms.fonts.size_px,
                    );
                }
            }
            w if self.confirm.owns(w) => self.confirm.draw(c, &forms, &ctx.text),
            w if self.note.owns(w) => self.note.draw(c, &forms, &ctx.text),
            _ => {
                if std::mem::take(&mut self.refit) {
                    self.folder.fit_columns(ctx);
                }
                if let Some(n) = self.select.take() {
                    if self.snapshot.printer(&n).is_some() {
                        self.folder.select_name(&n, ctx, &self.snapshot);
                    } else {
                        self.select = Some(n);
                    }
                }
                self.folder.draw(c, ctx);
            }
        }
    }
}

impl repriseapp::Extended for PrintersApp {
    fn hidden(&self) -> bool {
        self.hidden
    }

    fn parent(&self, win: WinId) -> Option<WinId> {
        if let Some(over) = self.note.parent(win) {
            return Some(over);
        }
        if let Some(&queue) = self.sheet_over.get(&win) {
            return Some(queue);
        }
        // Rename? covers the sheet it was asked from while that sheet is open, else the folder.
        match self.confirm.parent(win) {
            Some(sheet) if self.props.get(sheet).is_some() => Some(sheet),
            _ => Some(MAIN),
        }
    }

    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        let (_, d) = self.browse.as_ref().filter(|_| win == BROWSE)?;
        Some(d.min_client())
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        let px = forms.fonts.size_px;
        match win {
            _ if let Some(q) = self.queues.get(win) => q.access(tree, ctx),
            _ if let Some(p) = self.props.get(win) => {
                oneshot::access_sheet(&p.sheet, tree, &ctx.text, px);
            }
            WIZARD => {
                if let Some(w) = &self.wizard {
                    w.access(tree, &ctx.text, px);
                }
            }
            BROWSE => {
                if let Some((_, d)) = &self.browse {
                    let (w, h) = d.client();
                    d.access(
                        tree,
                        Rect::new(0, 0, w, h),
                        &ctx.text,
                        ctx.forms().fonts.size_px,
                    );
                }
            }
            w if self.confirm.owns(w) => self.confirm.access(tree, &ctx.text, &forms),
            w if self.note.owns(w) => self.note.access(tree, &ctx.text, &forms),
            _ => self.folder.access(tree, ctx),
        }
    }

    fn reader(&mut self, win: WinId, act: repriseapp::Reader, ctx: &Ctx) -> bool {
        match (win, act) {
            (_, repriseapp::Reader::Reveal { row, .. }) if self.queues.get(win).is_some() => {
                self.queues.get_mut(win).is_some_and(|q| q.reveal(row, ctx))
            }
            (repriseapp::MAIN, repriseapp::Reader::Reveal { row, .. }) => {
                self.folder.reveal(row, ctx)
            }
            _ => repriseapp::reader_forms(self, win, act, ctx),
        }
    }

    fn popup(&mut self, win: WinId, ctx: &Ctx) -> Option<Box<dyn Hosted + '_>> {
        if win == repriseapp::MAIN {
            return Some(self.folder.popup());
        }
        if self.queues.get(win).is_some() {
            return self.queues.get_mut(win).map(queue::Queue::popup);
        }
        repriseapp::reader_popup(self, win, ctx)
    }

    fn drop_form(&mut self, win: WinId) -> Option<DropForm<'_>> {
        let form = match win {
            WIZARD => &mut self.wizard.as_mut()?.page.form,
            _ => self.props.get_mut(win)?.sheet.open_mut(),
        };
        Some(DropForm { form, px: None })
    }

    fn drop_file_dialog(&mut self, win: WinId) -> Option<FileDrop<'_>> {
        let (host, dlg) = self.browse.as_mut().filter(|_| win == BROWSE)?;
        Some(FileDrop { dlg, host })
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(args: Vec<std::ffi::OsString>) {
    repriseapp::applet_main(&args, PrintersApp::new);
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriseapp::Extended;

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
    fn form_windows_open_at_the_text_size() {
        let mut app = PrintersApp::default();
        app.open_wizard();
        app.snapshot.printers.push(cups::Printer {
            name: "Laser".into(),
            ..Default::default()
        });
        let props = app.open_props("Laser", props::GENERAL).unwrap().id();
        let mut ctx = Ctx {
            text_size: 150,
            ..test_ctx(folder::CLIENT)
        };
        app.event(repriseapp::MAIN, Event::Blink, &mut ctx);
        let size = |id| {
            app.children()
                .into_iter()
                .find(|c| c.id == id)
                .unwrap()
                .size
        };
        assert_eq!(size(WIZARD), (768, 556));
        assert_eq!(size(props), (587, 680));
    }

    #[test]
    fn have_disk_shrinks_back_to_the_dialog_minimum() {
        let mut app = PrintersApp::default();
        let mut d = FileDlg::new(
            FileFields {
                title: "Have Disk".into(),
                accept_label: "Open".into(),
                mode: DlgMode::Open,
                multiple: false,
                filters: vec!["Printer Drivers (*.ppd)".into()],
            },
            Vec::new(),
        );
        d.set_size((800, 600));
        let min = d.min_client();
        assert!(min.0 < 800 && min.1 < 600, "{min:?}");
        app.browse = Some((Host::default(), d));
        assert_eq!(app.min_size(BROWSE), Some(min));
        assert_eq!(app.min_size(WIZARD), None);
    }

    #[test]
    fn the_folder_and_the_queue_open_at_the_text_size() {
        let mut app = PrintersApp::default();
        assert_eq!(app.initial_size(), folder::CLIENT);
        let queue = app.open_queue("Laser").id();
        let mut ctx = Ctx {
            text_size: 200,
            ..test_ctx(folder::CLIENT)
        };
        app.event(repriseapp::MAIN, Event::Blink, &mut ctx);
        let twice = |(w, h): (i32, i32)| (2 * w, 2 * h);
        assert_eq!(app.initial_size(), twice(folder::CLIENT));
        let queue = app.children().into_iter().find(|c| c.id == queue);
        assert_eq!(queue.map(|c| c.size), Some(twice(queue::CLIENT)));
    }

    /// The folder's and the queue's column titles fit their header items under every interface
    /// and text size.
    #[test]
    fn every_column_title_fits_its_header() {
        let over = reprisekit::interfaces::overflows(|_, theme, text| {
            let lists = [
                ("folder", folder::Folder::new().list),
                ("queue", queue::Queue::new("Laser").list),
            ];
            let p = reprisekit::look::Paint {
                s: 1,
                ..theme.paint()
            };
            let mut over = Vec::new();
            for (name, list) in &lists {
                let r = reprisekit::raster::Rect::new(0, 0, 900, 400);
                let lay = reprisekit::controls::listview::layout(&list.view, r, &theme.controls());
                for (ci, by) in reprisekit::controls::listview::header_overflows(
                    &list.view,
                    &lay,
                    text,
                    &p,
                    theme.fonts.size_px,
                ) {
                    let title = list.view.columns[ci].title.clone();
                    over.push((format!("{name} {title}"), "header", by));
                }
            }
            over
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    /// A wake that shows no window leaves a second instance's handoff for the wake whose raise does.
    #[test]
    fn the_handoff_waits_for_the_wake_that_shows_the_window() {
        let handoff = reprisebus::instance::Handoff {
            token: Some("t".into()),
            by_key: true,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = PrintersApp {
            tray: Some(reprisetray::TrayHandle::holding(handoff.clone())),
            msgs: Some(rx),
            hidden: true,
            ..PrintersApp::default()
        };
        assert_eq!(app.wake().activate, None);
        tx.send(reprisetray::Msg::Activate(0, 0)).unwrap();
        assert_eq!(app.wake().activate, Some((repriseapp::MAIN, handoff)));
    }

    #[test]
    fn a_raise_that_opens_a_queue_hands_its_token_to_the_queue_window() {
        let handoff = reprisebus::instance::Handoff {
            token: Some("t".into()),
            by_key: false,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = PrintersApp {
            tray: Some(reprisetray::TrayHandle::holding(handoff.clone())),
            msgs: Some(rx),
            hidden: true,
            user: "u".into(),
            snapshot: cups::Snapshot {
                printers: vec![cups::Printer {
                    name: "Laser".into(),
                    ..Default::default()
                }],
                jobs: vec![cups::Job {
                    printer: "Laser".into(),
                    owner: "u".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..PrintersApp::default()
        };
        tx.send(reprisetray::Msg::Activate(0, 0)).unwrap();
        let activate = app.wake().activate;
        let queue = app.queues.of("Laser").expect("the Laser queue opened");
        assert_eq!(activate, Some((queue, handoff)));
    }

    fn two_printers() -> PrintersApp {
        let printer = |name: &str| cups::Printer {
            name: name.into(),
            ..Default::default()
        };
        PrintersApp {
            snapshot: cups::Snapshot {
                printers: vec![printer("Laser"), printer("Inkjet")],
                running: true,
                ..Default::default()
            },
            ..PrintersApp::default()
        }
    }

    #[test]
    fn two_printers_show_two_queues_and_two_property_sheets() {
        let mut app = two_printers();
        let mut ctx = test_ctx(folder::CLIENT);
        for p in ["Laser", "Inkjet"] {
            app.folder_outcome(folder::Outcome::Open(p.into()), &mut ctx);
            app.folder_outcome(folder::Outcome::Properties(p.into()), &mut ctx);
        }
        let ids: HashSet<WinId> = app.children().into_iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), 4, "every window has its own id");
    }

    #[test]
    fn reopening_a_printer_s_window_raises_it_and_keeps_the_others() {
        let mut app = two_printers();
        let mut ctx = test_ctx(folder::CLIENT);
        let open = |app: &mut PrintersApp, out, ctx: &mut Ctx| app.folder_outcome(out, ctx).raise;
        assert_eq!(
            open(&mut app, folder::Outcome::Open("Laser".into()), &mut ctx),
            None
        );
        assert_eq!(
            open(&mut app, folder::Outcome::Open("Inkjet".into()), &mut ctx),
            None
        );
        let laser = app.queues.of("Laser");
        assert_eq!(
            open(&mut app, folder::Outcome::Open("Laser".into()), &mut ctx),
            laser
        );
        open(
            &mut app,
            folder::Outcome::Properties("Laser".into()),
            &mut ctx,
        );
        let sheet = app.props.of("Laser");
        let again = open(&mut app, folder::Outcome::Sharing("Laser".into()), &mut ctx);
        assert_eq!(again, sheet);
        assert_eq!(app.children().len(), 3);
    }

    #[test]
    fn closing_one_queue_leaves_the_other_open() {
        let mut app = two_printers();
        let laser = app.open_queue("Laser").id();
        let inkjet = app.open_queue("Inkjet").id();
        let mut ctx = test_ctx(queue::CLIENT);
        app.event(laser, Event::CloseRequest, &mut ctx);
        assert!(app.queues.get(laser).is_none());
        assert_eq!(app.queues.of("Inkjet"), Some(inkjet));
    }

    /// The question up and its window.
    fn question(app: &PrintersApp) -> Option<(&Ask, WinId)> {
        Some((app.confirm.up()?.1, app.confirm.win()?))
    }

    #[test]
    fn renaming_an_smb_queue_asks_first_then_opens_its_ports_tab() {
        let mut app = two_printers();
        app.snapshot.printers[0].device_uri = "smb://server/q".into();
        let (worker, sent) = worker::Handle::channel();
        app.worker = Some(worker);
        let mut ctx = test_ctx(folder::CLIENT);
        let rename = |old: &str| folder::Outcome::Rename {
            old: old.into(),
            new: format!("{old}_2"),
        };
        app.folder_outcome(rename("Inkjet"), &mut ctx);
        assert!(app.confirm.is_empty(), "an address without a password");
        app.folder_outcome(rename("Laser"), &mut ctx);
        let (ask, win) = question(&app).expect("the question");
        assert!(matches!(ask, Ask::Rename { .. }));
        let w = app.confirm.child(&ctx.text, &ctx.forms()).unwrap().size.0;
        let cap = reprisekit::controls::msgbox::WRAP_W;
        assert!(w <= cap, "the question wraps at {cap}, not {w}");
        let renames = |sent: &Receiver<Cmd>| {
            sent.try_iter()
                .filter_map(|c| match c {
                    Cmd::Rename { new, .. } => Some(new),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(renames(&sent), ["Inkjet_2"], "Laser's is not sent yet");
        let enter = repriseapp::KeyInput {
            sym: reprisekit::textedit::keysym::RETURN,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: None,
        };
        app.confirm_event(win, Event::Key(enter), &ctx);
        assert!(app.confirm.is_empty());
        assert_eq!(renames(&sent), ["Laser_2"]);
        let mut s = app.snapshot.clone();
        s.printers[0].name = "Laser_2".into();
        let renamed = Msg::Renamed {
            old: "Laser".into(),
            new: "Laser_2".into(),
            made: true,
        };
        worker_says(&mut app, [renamed, Msg::Snapshot(s)]);
        let sheet = app
            .props
            .by_printer_mut("Laser_2")
            .expect("Properties opened");
        assert_eq!(sheet.sheet.tabs.open, props::PORTS);
        assert_eq!(app.ports_after, None);
    }

    #[test]
    fn a_rename_with_its_address_typed_does_not_ask() {
        let mut app = two_printers();
        app.snapshot.printers[0].device_uri = "smb://server/q".into();
        let (worker, sent) = worker::Handle::channel();
        app.worker = Some(worker);
        let mut ctx = test_ctx(folder::CLIENT);
        app.start_rename(
            "Laser".into(),
            "Office".into(),
            Some("smb://u:p@server/q".into()),
            MAIN,
            &mut ctx,
        );
        assert!(app.confirm.is_empty());
        assert!(matches!(
            &sent.try_iter().collect::<Vec<_>>()[..],
            [Cmd::Rename { new, device_uri: Some(_), .. }] if new == "Office"
        ));
    }

    #[test]
    fn a_rename_asked_while_delete_is_up_waits_and_each_answer_acts_on_its_own_printer() {
        let mut app = two_printers();
        app.snapshot.printers[1].device_uri = "smb://server/q".into();
        let (worker, sent) = worker::Handle::channel();
        app.worker = Some(worker);
        let mut ctx = test_ctx(folder::CLIENT);
        let sheet = app.open_props("Inkjet", props::GENERAL).unwrap().id();
        app.folder_outcome(folder::Outcome::Delete("Laser".into()), &mut ctx);
        app.start_rename("Inkjet".into(), "Office".into(), None, sheet, &mut ctx);
        let enter = || {
            Event::Key(repriseapp::KeyInput {
                sym: reprisekit::textedit::keysym::RETURN,
                utf8: None,
                mods: repriseapp::Mods::default(),
                edit: None,
            })
        };

        let (ask, win) = question(&app).expect("Delete is up");
        assert!(matches!(ask, Ask::Delete(p) if p == "Laser"));
        assert_eq!(app.parent(win), Some(repriseapp::MAIN));
        let sent = move || sent.try_iter().collect::<Vec<Cmd>>();
        sent();
        app.event(win, enter(), &mut ctx);
        assert!(
            matches!(&sent()[..], [Cmd::Delete(p)] if p == "Laser"),
            "Yes deletes Laser alone"
        );

        let (ask, win) = question(&app).expect("Rename is up after Delete's answer");
        assert!(matches!(ask, Ask::Rename { old, new, .. } if old == "Inkjet" && new == "Office"));
        assert_eq!(
            app.parent(win),
            Some(sheet),
            "Rename covers the sheet it came from"
        );
        app.event(win, enter(), &mut ctx);
        assert!(matches!(
            &sent()[..],
            [Cmd::Rename { old, new, device_uri: None }] if old == "Inkjet" && new == "Office"
        ));
        assert!(question(&app).is_none());
    }

    #[test]
    fn a_question_about_a_printer_that_is_gone_is_withdrawn() {
        let mut app = two_printers();
        let mut ctx = test_ctx(folder::CLIENT);
        app.folder_outcome(folder::Outcome::Delete("Laser".into()), &mut ctx);
        app.folder_outcome(folder::Outcome::Delete("Inkjet".into()), &mut ctx);
        let mut s = app.snapshot.clone();
        s.printers.retain(|p| p.name != "Laser");
        app.snapshot_in(s);
        let (ask, _) = question(&app).expect("Inkjet's question");
        assert!(matches!(ask, Ask::Delete(p) if p == "Inkjet"));
        assert_eq!(app.confirm.len(), 1);
    }

    /// Hands the program the worker's `msgs` in one wake.
    fn worker_says(app: &mut PrintersApp, msgs: impl IntoIterator<Item = Msg>) {
        let (tx, rx) = std::sync::mpsc::channel();
        for m in msgs {
            tx.send(m).unwrap();
        }
        app.rx = Some(rx);
        app.wake();
    }

    /// The window the notice up covers.
    fn notice_over(app: &PrintersApp) -> Option<WinId> {
        app.parent(app.note.win()?)
    }

    #[test]
    fn a_notice_covers_its_printer_s_sheet_else_its_queue_else_the_main_window() {
        let mut app = two_printers();
        let laser_queue = app.open_queue("Laser").id();
        let laser_sheet = app.open_props("Laser", props::GENERAL).unwrap().id();
        let inkjet_queue = app.open_queue("Inkjet").id();
        let sent = |p: &str| Msg::TestPage(p.into());
        worker_says(&mut app, [sent("Laser"), sent("Inkjet"), sent("Office")]);
        assert_eq!(notice_over(&app), Some(laser_sheet), "Laser's sheet");
        app.note.close();
        assert_eq!(notice_over(&app), Some(inkjet_queue), "Inkjet's queue");
        app.note.close();
        assert_eq!(notice_over(&app), Some(MAIN), "no window of Office's");
        app.note.close();
        app.props.close(laser_sheet);
        worker_says(&mut app, [sent("Laser")]);
        assert_eq!(notice_over(&app), Some(laser_queue), "Laser's queue");
    }

    #[test]
    fn a_long_notice_wraps_at_the_message_box_cap() {
        let ctx = test_ctx(props::CLIENT);
        let notice = |body: String| {
            let mut app = two_printers();
            let sheet = app.open_props("Laser", props::GENERAL).unwrap().id();
            let failed = Msg::Failed {
                title: "Rename".into(),
                body,
                printer: Some("Laser".into()),
            };
            worker_says(&mut app, [failed]);
            assert_eq!(notice_over(&app), Some(sheet));
            app.note.child(&ctx.text, &ctx.forms()).unwrap().size
        };
        let one_line = notice("Laser could not be paused.".into());
        let (w, h) = notice(format!(
            "Laser is now called Office, but Office could not be paused. {}",
            cups::Error::NeedsHelper.text()
        ));
        let cap = reprisekit::controls::msgbox::WRAP_W;
        assert!(w <= cap, "the notice wraps at {cap}, not {w}");
        assert!(h > one_line.1, "the notice takes more than one line");
    }

    #[test]
    fn a_notice_goes_with_the_window_it_covers() {
        let mut app = two_printers();
        let mut ctx = test_ctx(props::CLIENT);
        let sheet = app.open_props("Laser", props::GENERAL).unwrap().id();
        worker_says(&mut app, [Msg::TestPage("Laser".into())]);
        app.event(sheet, Event::CloseRequest, &mut ctx);
        assert!(!app.note.is_shown(), "closed with its sheet");

        app.open_props("Laser", props::GENERAL);
        let queue = app.open_queue("Inkjet").id();
        let sent = |p: &str| Msg::TestPage(p.into());
        worker_says(&mut app, [sent("Laser"), sent("Inkjet"), sent("Office")]);
        let mut s = app.snapshot.clone();
        s.printers.retain(|p| p.name != "Laser");
        app.snapshot_in(s);
        assert_eq!(
            app.note.boxes().count(),
            2,
            "Laser's notice left with its sheet"
        );
        assert_eq!(notice_over(&app), Some(queue), "Inkjet's notice is up");
        app.event(queue, Event::CloseRequest, &mut ctx);
        assert_eq!(
            app.note.boxes().count(),
            1,
            "Inkjet's notice left with its queue"
        );
        assert_eq!(notice_over(&app), Some(MAIN), "Office's notice stays");
    }

    #[test]
    fn a_sheet_is_owned_by_the_window_it_was_opened_from_and_leaves_with_it() {
        let mut app = two_printers();
        let mut ctx = test_ctx(queue::CLIENT);
        let laser_queue = app.open_queue("Laser").id();
        let inkjet_queue = app.open_queue("Inkjet").id();
        app.queue_outcome(laser_queue, queue::Outcome::Properties, &mut ctx);
        let laser = app.props.of("Laser").expect("Laser's sheet opened");
        app.folder_outcome(folder::Outcome::Properties("Inkjet".into()), &mut ctx);
        let inkjet = app.props.of("Inkjet").expect("Inkjet's sheet opened");
        assert_eq!(
            app.parent(laser),
            Some(laser_queue),
            "the queue window it opened from"
        );
        assert_eq!(app.parent(inkjet), Some(MAIN), "opened from the folder");
        assert_eq!(
            app.parent(laser_queue),
            Some(MAIN),
            "a queue window is the folder's"
        );

        app.folder_outcome(folder::Outcome::Properties("Laser".into()), &mut ctx);
        app.queue_outcome(inkjet_queue, queue::Outcome::Sharing, &mut ctx);
        assert_eq!(
            app.parent(laser),
            Some(laser_queue),
            "its queue window, raised from the folder"
        );
        assert_eq!(
            app.parent(inkjet),
            Some(MAIN),
            "the folder, raised from its queue"
        );

        worker_says(&mut app, [Msg::TestPage("Laser".into())]);
        assert_eq!(
            notice_over(&app),
            Some(laser),
            "the notice covers the sheet"
        );
        app.event(laser_queue, Event::CloseRequest, &mut ctx);
        assert_eq!(app.props.of("Laser"), None, "left with its queue window");
        assert!(!app.note.is_shown(), "the notice left with the sheet");
        app.event(inkjet_queue, Event::CloseRequest, &mut ctx);
        assert_eq!(app.props.of("Inkjet"), Some(inkjet), "the folder's stays");
    }

    #[test]
    fn a_rename_cupsd_did_not_make_leaves_its_windows_on_the_old_name() {
        let refused = Msg::Failed {
            title: "Could not rename the printer".into(),
            body: cups::Error::NotRunning.text(),
            printer: Some("Laser".into()),
        };
        let not_made = || Msg::Renamed {
            old: "Laser".into(),
            new: "Office".into(),
            made: false,
        };
        let down = Msg::Snapshot(Snapshot::default());
        let cases = [
            ("cupsd down", vec![not_made(), refused, down]),
            ("prompt dismissed", vec![not_made()]),
        ];
        for (case, replies) in cases {
            let mut app = two_printers();
            let mut ctx = test_ctx(props::CLIENT);
            let queue = app.open_queue("Laser").id();
            let sheet = app.open_props("Laser", props::GENERAL).unwrap().id();
            app.start_rename("Laser".into(), "Office".into(), None, sheet, &mut ctx);
            let told = replies.len() > 1;
            worker_says(&mut app, replies);
            if told {
                assert_eq!(notice_over(&app), Some(sheet), "{case}: over the sheet");
            }
            worker_says(&mut app, [Msg::Snapshot(two_printers().snapshot)]);
            assert_eq!(
                app.queues.get(queue).map(|q| q.printer.as_str()),
                Some("Laser"),
                "{case}: the queue window"
            );
            assert_eq!(
                app.props.get(sheet).map(|p| p.window_title()).as_deref(),
                Some("Laser Properties"),
                "{case}: the sheet"
            );
        }
    }

    #[test]
    fn a_deleted_printer_s_windows_close_and_the_other_printer_s_stay() {
        let mut app = two_printers();
        let laser = app.open_queue("Laser").id();
        app.open_props("Laser", props::GENERAL);
        let inkjet = app.open_queue("Inkjet").id();
        let mut s = app.snapshot.clone();
        s.printers.retain(|p| p.name != "Laser");
        app.snapshot_in(s);
        assert_eq!(app.queues.of("Laser"), None);
        assert_eq!(app.props.of("Laser"), None);
        assert_eq!(app.queues.of("Inkjet"), Some(inkjet));
        assert!(
            !app.refit_queues.contains(&laser),
            "the queue's fit goes with it"
        );
        assert!(app.refit_queues.contains(&inkjet));
    }
}
