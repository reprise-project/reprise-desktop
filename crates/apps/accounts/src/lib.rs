#[cfg(test)]
reprisebus::test_session!();

mod acct;
mod adduser;
mod fingers;
mod passwd;
mod picture;
mod props;
mod rdpassword;
mod remote;
mod users;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use repriseapp::filedlg::{Host, Resolved, Step, type_keep};
use repriseapp::keys::{dialog_key, form_clipboard, form_input, sheet_input};
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{
    App, BTN_LEFT, Child, Ctx, DropForm, Event, FileDrop, PointerInput, Reply, WinId,
};
use reprisekit::access::Tree;
use reprisekit::controls::addfinger::{self, AddFinger};
use reprisekit::controls::filedlg::{
    self, DlgMode, Event as FileEvent, Fields as FileFields, FileDlg,
};
use reprisekit::controls::fingerprints;
use reprisekit::controls::msgbox::Fields;
use reprisekit::controls::remotepassword;
use reprisekit::controls::{form, propsheet};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::keysym;
use reprisekit::theme::{TextUnits, Theme};

use acct::{Op, User};
use rdpassword::RdPasswordDialog;
use repriselogin::account::AccountRefusal;

const PROPS: WinId = WinId::named("props");
const ADD: WinId = WinId::named("add");
const BROWSE: WinId = WinId::named("browse");
const REMOVE: WinId = WinId::named("remove");
const ERROR: WinId = WinId::named("error");
const RD_PASSWORD: WinId = WinId::named("rd_password");
const ADD_FINGER: WinId = WinId::named("add_finger");
const CLEAR_FINGERS: WinId = WinId::named("clear_fingers");

const TITLE: &str = "User Accounts";
const BUSY: &str = "Another change is still being made. Try again once it is done.";

const ROW_PX: u32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum After {
    Read,
    /// Properties' Apply from sheet number `sheet` (see `sheets`), for account `uid`; OK closes
    /// the sheet when it succeeds.
    Props {
        close: bool,
        sheet: u64,
        uid: u32,
    },
    Add,
    Remove,
}

type Answer = (Result<(), acct::Failure>, zbus::Result<Vec<User>>);

struct AccountsApp {
    users: users::Users,
    props: Option<props::Props>,
    /// How many Properties sheets have opened: the open one's number.
    sheets: u64,
    add: Option<adduser::AddUser>,
    browse: Option<FileDlg>,
    browse_host: Host,
    remove: Option<(u32, MsgBox)>,
    error: Option<MsgBox>,
    job: Option<(Receiver<Answer>, After)>,
    waker: Option<repriseapp::Waker>,
    face: Option<Canvas>,
    text: Option<Rc<TextRenderer>>,
    /// The forms theme at the last event: `children` and `initial_size` have no `Ctx`.
    forms: Theme,
    /// The client size last asked for the main window.
    asked: (i32, i32),
    remote: remote::Remote,
    sources: remote::Sources,
    rd_password: Option<RdPasswordDialog>,
    /// Open your Properties on Password once the accounts are read.
    open_yours: bool,
    fingers: fingers::Fingers,
    finger_sources: fingers::Sources,
    add_finger: Option<AddFinger>,
    clear_fingers: Option<MsgBox>,
}

impl AccountsApp {
    fn new() -> AccountsApp {
        AccountsApp {
            users: users::Users::new(),
            props: None,
            sheets: 0,
            add: None,
            browse: None,
            browse_host: Host::default(),
            remove: None,
            error: None,
            job: None,
            waker: None,
            face: None,
            text: None,
            forms: Theme::default(),
            asked: users::CLIENT,
            remote: remote::Remote::default(),
            sources: remote::Sources::default(),
            rd_password: None,
            open_yours: false,
            fingers: fingers::Fingers::default(),
            finger_sources: fingers::Sources::default(),
            add_finger: None,
            clear_fingers: None,
        }
    }

    /// Run `job` off the loop, waking it when done; tests run it at once.
    fn spawn<T: Send + 'static>(&self, job: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
        let (tx, rx) = channel();
        if cfg!(test) {
            let _ = tx.send(job());
            return rx;
        }
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
            if let Some(w) = waker {
                w.wake();
            }
        });
        rx
    }

    /// Read the login manager and your Remote Desktop password after `wait`, unless a read is
    /// under way. Only the first of a run of unanswered reads warns.
    fn read_remote(&mut self, wait: Duration) {
        if self.remote.reading.is_some() {
            return;
        }
        let src = self.sources.clone();
        let failed = match &self.remote.found {
            Some(f) if f.silent() => log::Level::Debug,
            _ => log::Level::Warn,
        };
        self.remote.reading = Some(self.spawn(move || {
            std::thread::sleep(wait);
            remote::read(&src, failed)
        }));
    }

    /// Put the Remote Desktop password section for the open account on its Password page.
    fn show_remote(&mut self) {
        if let Some(p) = &mut self.props {
            let facts = self.remote.facts(p.user.you);
            p.show_remote(facts.as_ref().and_then(remotepassword::section));
        }
    }

    /// A read or a change came back: show it, and in the dialog that sent the change. A login
    /// manager that did not answer is asked again while your own sheet is open. True when
    /// anything changed.
    fn remote_done(&mut self) -> bool {
        let mut changed = false;
        if let Some(found) = self.remote.read_landed() {
            changed = true;
            if found.silent() && self.props.as_ref().is_some_and(|p| p.user.you) {
                self.read_remote(self.sources.retry);
            }
        }
        if let Some((from, ended)) = self.remote.change_landed() {
            changed = true;
            let sender = self.rd_password.take_if(|d| d.generation == from);
            if let remote::Ended::Refused(r) = ended {
                // A failure may be a reply that came too late for a change the daemon made.
                if r == AccountRefusal::Failed {
                    self.remote.reading = None;
                    self.read_remote(Duration::ZERO);
                }
                if let Some(mut d) = sender {
                    d.refused(r);
                    self.rd_password = Some(d);
                }
            }
        }
        if changed {
            self.show_remote();
        }
        changed
    }

    fn open_rd_password(&mut self, action: remotepassword::Action) -> Reply {
        self.remote.opened += 1;
        self.rd_password = Some(RdPasswordDialog::new(action, self.remote.opened));
        Reply::redraw_all()
    }

    /// Read the reader and your fingers on it, unless a read is under way.
    fn read_fingers(&mut self) {
        if self.fingers.reading.is_some() {
            return;
        }
        let read = self.finger_sources.read;
        self.fingers.reading = Some(self.spawn(read));
    }

    /// Put the Fingerprints page for the open account on its tab.
    fn show_fingers(&mut self) {
        if let Some(p) = &mut self.props {
            let facts = self.fingers.facts(p.user.you);
            p.show_fingers(fingerprints::section(&facts));
        }
    }

    /// A read, a delete or an add's step came back: show it, in the dialog that started an add
    /// too. A finished delete or add reads your fingers again. True when anything changed.
    fn fingers_done(&mut self) -> bool {
        let mut changed = self.fingers.read_landed();
        let mut reread = false;
        if let Some(done) = self.fingers.delete_landed() {
            changed = true;
            reread = true;
            if let Err(r) = done {
                self.fail(fingers::refusal(r).text("delete the fingerprint"));
            }
        }
        if let Some((from, steps, ended)) = self.fingers.add_landed() {
            changed = true;
            let dialog = self.add_finger.as_mut().filter(|d| d.generation == from);
            if let Some(d) = dialog {
                for step in steps {
                    match step {
                        fingers::Step::Started => d.started(),
                        fingers::Step::Scanned(s) => d.scanned(s),
                    }
                }
                if let Some(Err(r)) = ended {
                    d.refused(fingers::refusal(r));
                }
            }
            reread |= ended.is_some();
        }
        if reread {
            // A read under way may predate the change.
            self.fingers.reading = None;
            self.read_fingers();
            self.fingers.read_landed();
        }
        if changed {
            self.show_fingers();
        }
        changed
    }

    fn fingers_action(&mut self, a: fingerprints::Action, ctx: &mut Ctx) -> Reply {
        match a {
            fingerprints::Action::Add => {
                let added = self.props.as_ref().map(|p| p.fingers().to_vec());
                let (scan, stages) = self.fingers.scan();
                self.fingers.opened += 1;
                let from = self.fingers.opened;
                self.add_finger = Some(AddFinger::new(
                    added.unwrap_or_default(),
                    scan,
                    stages,
                    from,
                ));
            }
            fingerprints::Action::Delete(f) => self.delete_fingers(Some(f)),
            fingerprints::Action::DeleteAll => {
                self.clear_fingers = Some(MsgBox::new(
                    "Delete All Fingerprints",
                    Fields {
                        text: "Delete all your fingerprints? You unlock with your password until \
                               you add one again."
                            .into(),
                        icon: ctx.icons.get("dialog-question", 32),
                        buttons: vec!["&Delete All".into(), "Cancel".into()],
                        default: 1,
                        min_text_h: None,
                        min_w: 0,
                        max_w: repriseapp::dialog_max_w(),
                    },
                ));
            }
        }
        Reply::redraw_all()
    }

    /// Delete `finger`, or every finger with `None`, off the loop.
    fn delete_fingers(&mut self, finger: Option<fingerprints::Finger>) {
        if self.fingers.deleting.is_some() || self.fingers.add.is_some() {
            return;
        }
        let delete = self.finger_sources.delete;
        self.fingers.deleting = Some(self.spawn(move || delete(finger.map(|f| f.key()))));
        self.show_fingers();
    }

    fn clear_fingers_event(&mut self, ev: Event) -> Reply {
        let text = self.text();
        let Some(b) = &mut self.clear_fingers else {
            return Reply::none();
        };
        match b.event(&ev, &text, &self.forms) {
            oneshot::Outcome::None => Reply::none(),
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::Closed(button) => {
                self.clear_fingers = None;
                if button == Some(0) {
                    self.delete_fingers(None);
                }
                Reply::redraw_all()
            }
        }
    }

    fn add_finger_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.add_finger.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => d.event(form::Event::Cancel),
                ev => match form_input(&mut d.form, &ev, ctx) {
                    Some(e) => d.event(e),
                    None => return Reply::none(),
                },
            }
        };
        match out {
            addfinger::Outcome::None => Reply::none(),
            addfinger::Outcome::Redraw => Reply::redraw(),
            addfinger::Outcome::Start(f) => {
                self.start_add(f);
                Reply::redraw_all()
            }
            addfinger::Outcome::Stop | addfinger::Outcome::Close => {
                self.close_add_finger();
                Reply::redraw_all()
            }
        }
    }

    /// Close the Add a Fingerprint dialog, stopping its add if one runs: the add's end still
    /// lands and reads the fingers again.
    fn close_add_finger(&mut self) {
        self.add_finger = None;
        if let Some(a) = &self.fingers.add {
            a.stop.stop();
        }
    }

    /// Add `finger` on a thread of its own, which tells each step and wakes the loop.
    fn start_add(&mut self, finger: fingerprints::Finger) {
        let Some(from) = self.add_finger.as_ref().map(|d| d.generation) else {
            return;
        };
        if self.fingers.add.is_some() || self.fingers.deleting.is_some() {
            if let Some(d) = &mut self.add_finger {
                d.refused(addfinger::Refusal::InUse);
            }
            return;
        }
        let (tx, rx) = channel();
        let (stop, stopped) = reprisebus::fprint::Stop::new();
        let waker = self.waker.clone();
        let wake = move || {
            if let Some(w) = &waker {
                w.wake();
            }
        };
        std::thread::spawn(move || {
            let step = tx.clone();
            let ended = reprisebus::fprint::add(finger.key(), stopped, |a| {
                let _ = step.send(fingers::Msg::Step(a));
                wake();
            });
            let _ = tx.send(fingers::Msg::Ended(ended));
            wake();
        });
        self.fingers.add = Some(fingers::Add { from, rx, stop });
        self.show_fingers();
    }

    fn rd_password_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.rd_password.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => rdpassword::Outcome::Cancel,
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
            }
        };
        self.rd_password_outcome(out)
    }

    /// Carry out what the dialog asked. A request runs off the loop with the dialog open; a
    /// Cancel drops the dialog and its fields, and a request under way still lands.
    fn rd_password_outcome(&mut self, out: rdpassword::Outcome) -> Reply {
        match out {
            rdpassword::Outcome::None => Reply::none(),
            rdpassword::Outcome::Redraw => Reply::redraw(),
            rdpassword::Outcome::Cancel => {
                self.rd_password = None;
                Reply::redraw_all()
            }
            rdpassword::Outcome::Send(req) => {
                let Some(d) = &self.rd_password else {
                    return Reply::none();
                };
                let (action, from) = (d.action, d.generation);
                let path = self.sources.socket.clone();
                let rx = self.spawn(move || remote::ask(&path, &req, remote::TIMEOUT));
                self.remote.change = Some(remote::Change { action, from, rx });
                self.show_remote();
                Reply::redraw_all()
            }
        }
    }

    fn fonts(&mut self) {
        self.text = Some(repriseapp::start_text());
    }

    fn text(&self) -> Rc<TextRenderer> {
        self.text.clone().expect("fonts are loaded in start")
    }

    /// Run `ops` (none for a plain read) and reread the accounts, off the
    /// event loop. One job runs at a time; a second is dropped with its saved pictures.
    fn run(&mut self, ops: Vec<Op>, after: After) -> bool {
        if self.job.is_some() {
            acct::remove_pictures(&ops);
            return false;
        }
        let (tx, rx) = channel();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let done = if ops.is_empty() {
                Ok(())
            } else {
                acct::apply(&ops)
            };
            let _ = tx.send((done, acct::read()));
            if let Some(w) = waker {
                w.wake();
            }
        });
        self.job = Some((rx, after));
        self.set_busy(true);
        true
    }

    /// Greys what would start a second job while one runs.
    fn set_busy(&mut self, busy: bool) {
        self.users.set_busy(busy);
        if let Some(p) = &mut self.props {
            p.set_busy(busy);
        }
        if let Some(d) = &mut self.add {
            d.set_busy(busy);
        }
    }

    fn fail(&mut self, text: String) {
        self.error = Some(MsgBox::new(
            TITLE,
            Fields {
                text,
                icon: None,
                buttons: vec!["OK".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        ));
    }

    fn user(&self, uid: u32) -> Option<&User> {
        self.users.rows.iter().find(|u| u.uid == uid)
    }

    fn last_admin(&self, uid: u32) -> bool {
        let admins: Vec<u32> = self
            .users
            .rows
            .iter()
            .filter(|u| u.admin)
            .map(|u| u.uid)
            .collect();
        admins == [uid]
    }

    fn wake(&mut self) -> Reply {
        let remote = self.remote_done();
        let fingers = self.fingers_done();
        if self.accounts_done() || remote || fingers {
            Reply::redraw_all()
        } else {
            Reply::none()
        }
    }

    /// The accounts job came back: show the accounts and what it did. True when it had.
    fn accounts_done(&mut self) -> bool {
        let Some((rx, after)) = &self.job else {
            return false;
        };
        let Ok((done, read)) = rx.try_recv() else {
            return false;
        };
        let after = *after;
        self.job = None;
        self.set_busy(false);
        match read {
            Ok(rows) => {
                let pictures = self.row_pictures(&rows);
                self.users.refresh(rows, pictures);
            }
            Err(e) => {
                log::error!("AccountsService does not answer: {e}");
                self.users
                    .fail("The accounts could not be read. Is AccountsService installed?".into());
            }
        }
        let mine = |sheet| matches!(after, After::Props { sheet: s, .. } if s == sheet);
        let ours = mine(self.sheets) && self.props.is_some();
        if let Some(p) = &self.props {
            let uid = p.user.uid;
            let last = self.last_admin(uid);
            match self.user(uid).cloned() {
                None => {
                    self.props = None;
                    self.browse = None;
                    self.rd_password = None;
                    self.close_add_finger();
                    self.clear_fingers = None;
                }
                Some(u) => {
                    let p = self.props.as_mut().expect("checked above");
                    p.set_last_admin(last);
                    if let After::Props { close, .. } = after
                        && ours
                    {
                        if done.is_ok() {
                            p.applied(u);
                            if close {
                                self.props = None;
                            }
                        } else {
                            p.unapplied();
                        }
                    }
                }
            }
        }
        if after == After::Add && done.is_ok() {
            self.add = None;
        }
        if let Some(d) = &mut self.add {
            d.set_taken(self.users.rows.iter().map(|u| u.login.clone()).collect());
        }
        if let Err(acct::Failure::Text(why)) = done {
            // An Apply from a sheet closed since says whose account it was for.
            let whose = match after {
                After::Props { uid, .. } if !ours => self.user(uid).map(User::named),
                _ => None,
            };
            match whose {
                Some(name) => self.fail(format!("The account {name}:\n\n{why}")),
                None => self.fail(why),
            }
        }
        if std::mem::take(&mut self.open_yours)
            && let Some(uid) = self.users.rows.iter().find(|u| u.you).map(|u| u.uid)
        {
            self.open_props(uid);
            if let Some(p) = &mut self.props {
                p.sheet.tabs.open = props::TAB_PASSWORD;
            }
        }
        true
    }

    /// Each account's list picture at [`ROW_PX`] grown with the text.
    fn row_pictures(&self, rows: &[User]) -> Vec<Option<Canvas>> {
        let px = self.forms.text.px(ROW_PX);
        rows.iter()
            .map(|u| picture::of_user(&u.icon, u.you, px))
            .collect()
    }

    /// The text size changed: the list's pictures and the sheet's are made again at their
    /// grown sizes.
    fn regrow(&mut self) {
        self.users.pictures = self.row_pictures(&self.users.rows);
        let face = self.face.take();
        if let Some(p) = &mut self.props {
            p.regrow(self.forms.text, |u, px| sheet_face(u, px, face.as_ref()));
        }
    }

    fn open_props(&mut self, uid: u32) {
        if self.props.as_ref().is_some_and(|p| p.user.uid == uid) {
            return;
        }
        let Some(u) = self.user(uid).cloned() else {
            return;
        };
        let px = self.forms.text.px(props::PREVIEW_PX);
        let face = sheet_face(&u, px, self.face.as_ref());
        let last = self.last_admin(uid);
        self.sheets += 1;
        let mut p = props::Props::new(u, face, last, self.forms.text);
        p.set_busy(self.job.is_some());
        self.props = Some(p);
        self.show_remote();
        self.read_remote(Duration::ZERO);
        self.show_fingers();
        if self.user(uid).is_some_and(|u| u.you) {
            self.read_fingers();
        }
    }

    fn ask_remove(&mut self, uid: u32, ctx: &mut Ctx) {
        let Some(u) = self.user(uid) else {
            return;
        };
        if u.logged_on {
            let why = acct::logged_on_text(u);
            return self.fail(why);
        }
        let b = MsgBox::new(
            "Delete Account",
            Fields {
                text: format!(
                    "Delete the account {}?\nTheir files in {} can stay on this computer or go with the account.",
                    u.named(),
                    u.home
                ),
                icon: ctx.icons.get("dialog-question", 32),
                buttons: vec![
                    "&Delete Files".into(),
                    "&Keep Files".into(),
                    "Cancel".into(),
                ],
                default: 2,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        );
        self.remove = Some((uid, b));
    }

    fn main_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let cm = ctx.theme.controls();
        let list_r = self.users.list_rect();
        let lit = {
            let list = &mut self.users.list;
            let changed = match &ev {
                Event::Pointer(PointerInput::Motion { x, y }) => list.hover(list_r, *x, *y, &cm),
                Event::Pointer(PointerInput::Leave) => list.leave(),
                Event::Pointer(PointerInput::Release {
                    button: BTN_LEFT, ..
                }) => list.release(),
                _ => false,
            };
            changed && ctx.styled()
        };
        let out = match ev {
            Event::CloseRequest => users::Outcome::Close,
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                time,
                ..
            }) if list_r.contains(x, y) => {
                let at = ctx.theme.metrics.click(x, y, time);
                self.users.list_press(at, &cm)
            }
            Event::Pointer(PointerInput::Motion { y, .. })
                if self.users.list.drag(list_r, y, &cm) =>
            {
                users::Outcome::Redraw
            }
            Event::Pointer(PointerInput::Scroll { rows, .. }) => {
                if self.users.list.scroll(rows, list_r, cm.text) {
                    users::Outcome::Redraw
                } else {
                    users::Outcome::None
                }
            }
            Event::Key(ref k)
                if self.users.list_focus && matches!(k.sym, keysym::DELETE | keysym::KP_DELETE) =>
            {
                self.users.delete_key()
            }
            ev => {
                let from_list = match &ev {
                    Event::Key(k) if self.users.list_focus => match dialog_key(k) {
                        Some(form::Key::Tab { .. }) => {
                            self.users.list_focus = false;
                            None
                        }
                        Some(key) => self.users.list_key(&key, &cm),
                        None => None,
                    },
                    Event::Pointer(PointerInput::Press { .. }) => {
                        self.users.list_focus = false;
                        None
                    }
                    _ => None,
                };
                match from_list {
                    Some(out) => out,
                    None => match form_input(&mut self.users.form, &ev, ctx) {
                        Some(e) => self.users.event(e),
                        None => users::Outcome::None,
                    },
                }
            }
        };
        let mut reply = match out {
            users::Outcome::None => Reply::none(),
            users::Outcome::Redraw => Reply::redraw(),
            users::Outcome::Close => Reply {
                quit: true,
                ..Reply::none()
            },
            users::Outcome::Add => {
                if self.add.is_none() {
                    let taken = self.users.rows.iter().map(|u| u.login.clone()).collect();
                    let mut d = adduser::AddUser::new(taken);
                    d.set_busy(self.job.is_some());
                    self.add = Some(d);
                }
                Reply::redraw_all()
            }
            users::Outcome::Remove(uid) => {
                self.ask_remove(uid, ctx);
                Reply::redraw_all()
            }
            users::Outcome::Properties(uid) => {
                self.open_props(uid);
                Reply::redraw_all()
            }
        };
        reply.redraw |= lit;
        reply
    }

    fn props_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(p) = self.props.as_mut() else {
            return Reply::none();
        };
        let save = |src: &picture::Source| picture::save(src);
        let out = match ev {
            Event::CloseRequest => props::Outcome::Close,
            Event::Blink => {
                p.sheet.open_mut().blink(ctx.caret_on);
                return Reply::redraw();
            }
            Event::Pasted(t) => p.paste(t),
            ev => match sheet_input(&mut p.sheet, &ev, ctx) {
                Some(e) => p.event(e, save),
                None => return Reply::none(),
            },
        };
        match out {
            props::Outcome::None => Reply::none(),
            props::Outcome::Redraw => Reply::redraw(),
            props::Outcome::Apply(ops) => {
                let (sheet, uid) = (self.sheets, p.user.uid);
                if !self.run(
                    ops,
                    After::Props {
                        close: false,
                        sheet,
                        uid,
                    },
                ) {
                    p_unapplied(&mut self.props);
                }
                Reply::redraw_all()
            }
            props::Outcome::ApplyAndClose(ops) => {
                let (sheet, uid) = (self.sheets, p.user.uid);
                if !self.run(
                    ops,
                    After::Props {
                        close: true,
                        sheet,
                        uid,
                    },
                ) {
                    p_unapplied(&mut self.props);
                }
                Reply::redraw_all()
            }
            props::Outcome::Refuse(why) => {
                self.fail(why);
                Reply::redraw_all()
            }
            props::Outcome::Browse => {
                self.open_browse(ctx.forms().text);
                Reply::redraw_all()
            }
            props::Outcome::RemotePassword(action) => self.open_rd_password(action),
            props::Outcome::Fingers(a) => self.fingers_action(a, ctx),
            props::Outcome::Close => {
                self.props = None;
                self.browse = None;
                self.rd_password = None;
                self.close_add_finger();
                self.clear_fingers = None;
                Reply::redraw_all()
            }
        }
    }

    fn add_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let Some(d) = self.add.as_mut() else {
            return Reply::none();
        };
        let out = match ev {
            Event::CloseRequest => adduser::Outcome::Cancel,
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
        match out {
            adduser::Outcome::None => Reply::none(),
            adduser::Outcome::Redraw => Reply::redraw(),
            adduser::Outcome::Create(op) => {
                self.run(vec![op], After::Add);
                Reply::redraw_all()
            }
            adduser::Outcome::Cancel => {
                self.add = None;
                Reply::redraw_all()
            }
        }
    }

    fn remove_event(&mut self, ev: Event) -> Reply {
        let text = self.text();
        let Some((uid, b)) = &mut self.remove else {
            return Reply::none();
        };
        match b.event(&ev, &text, &self.forms) {
            oneshot::Outcome::None => Reply::none(),
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::Closed(button) => {
                let uid = *uid;
                self.remove = None;
                if let Some(b @ (0 | 1)) = button {
                    let op = Op::Delete {
                        uid,
                        remove_files: b == 0,
                    };
                    if !self.run(vec![op], After::Remove) {
                        self.fail(BUSY.into());
                    }
                }
                Reply::redraw_all()
            }
        }
    }

    fn error_event(&mut self, ev: Event) -> Reply {
        let text = self.text();
        let Some(b) = &mut self.error else {
            return Reply::none();
        };
        match b.event(&ev, &text, &self.forms) {
            oneshot::Outcome::None => Reply::none(),
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::Closed(_) => {
                self.error = None;
                Reply::redraw_all()
            }
        }
    }

    fn open_browse(&mut self, u: TextUnits) {
        let dir = reprisefs::places::user_dir("PICTURES", "Pictures")
            .filter(|p| p.is_dir())
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/"));
        let fields = FileFields {
            title: "Browse".into(),
            accept_label: "Open".into(),
            mode: DlgMode::Open,
            multiple: false,
            filters: vec![
                "Pictures (*.png;*.jpg;*.jpeg;*.bmp)".into(),
                "All Files (*.*)".into(),
            ],
        };
        self.browse = Some(self.browse_host.open_last(&dir, fields, u));
    }

    fn browse_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(d) = &mut self.browse else {
            return Reply::none();
        };
        let host = &mut self.browse_host;
        let keep = type_keep(d);
        host.ensure_listed(d, &mut ctx.icons, &keep);
        let close = |app: &mut AccountsApp| {
            app.browse = None;
            Reply::redraw_all()
        };
        match host.event(
            d,
            &ev,
            &ctx.text,
            &ctx.forms(),
            ctx.size,
            &ctx.theme.metrics,
        ) {
            Step::Closed | Step::Out(FileEvent::Cancel) => close(self),
            Step::None => Reply::none(),
            Step::Redraw => Reply::redraw(),
            Step::Resize => Reply::resize(),
            Step::Out(FileEvent::NeedListing(id)) => {
                host.list(d, id, &mut ctx.icons, &keep);
                Reply::redraw()
            }
            Step::Out(FileEvent::FilterChanged(_)) => {
                let keep = type_keep(d);
                if let Some(id) = host.folder {
                    host.list(d, id, &mut ctx.icons, &keep);
                }
                Reply::redraw()
            }
            Step::Out(FileEvent::Accept(names)) => {
                let Some(name) = names.into_iter().next() else {
                    return Reply::none();
                };
                match host.resolve(&name) {
                    Ok(Resolved::Folder(id, _)) => {
                        host.list(d, id, &mut ctx.icons, &keep);
                        d.clear_file_name();
                        Reply::redraw()
                    }
                    Ok(Resolved::File(path)) => {
                        let px = self
                            .props
                            .as_ref()
                            .map_or(props::PREVIEW_PX, |p| p.preview_px());
                        match picture::load_file(&path, px) {
                            Some(c) => {
                                if let Some(p) = &mut self.props {
                                    p.browsed(path, c);
                                }
                            }
                            None => {
                                let name = path
                                    .file_name()
                                    .unwrap_or(path.as_os_str())
                                    .to_string_lossy();
                                self.fail(format!("'{name}' is not a picture Reprise can read."));
                            }
                        }
                        close(self)
                    }
                    Err(msg) => {
                        self.fail(msg);
                        close(self)
                    }
                }
            }
            Step::Out(_) => Reply::redraw(),
        }
    }

    fn focused_edit(&self, win: WinId) -> Option<form::FocusedEdit<'_>> {
        match win {
            PROPS => self.props.as_ref()?.sheet.open().focused_edit(),
            ADD => self.add.as_ref()?.form.focused_edit(),
            RD_PASSWORD => self.rd_password.as_ref()?.form.focused_edit(),
            _ => None,
        }
    }
}

/// `u`'s picture for the sheet at `px`, else the stock face `fallback` stands in for a missing one.
fn sheet_face(u: &User, px: u32, fallback: Option<&Canvas>) -> Option<Canvas> {
    picture::of_user(&u.icon, u.you, px).or_else(|| fallback.map(|c| c.resample_box(px, px)))
}

fn p_unapplied(p: &mut Option<props::Props>) {
    if let Some(p) = p {
        p.unapplied();
    }
}

impl App for AccountsApp {
    fn title(&self) -> String {
        TITLE.into()
    }

    fn app_id(&self) -> String {
        "reprise.accounts".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.forms.text.size(users::CLIENT)
    }

    fn children(&self) -> Vec<Child> {
        let mut out = Vec::new();
        if let Some(p) = &self.props {
            out.push(Child {
                id: PROPS,
                title: p.window_title(),
                size: self.forms.text.size(p.sheet.size),
                fixed: true,
                modal: false,
            });
        }
        if self.add.is_some() {
            out.push(Child {
                id: ADD,
                title: "Add Account".into(),
                size: self.forms.text.size(adduser::CLIENT),
                fixed: true,
                modal: false,
            });
        }
        if let Some(d) = &self.rd_password {
            out.push(Child {
                id: RD_PASSWORD,
                title: d.title().into(),
                size: self.forms.text.size(d.client()),
                fixed: true,
                modal: true,
            });
        }
        if let Some(d) = &self.add_finger {
            out.push(Child {
                id: ADD_FINGER,
                title: addfinger::TITLE.into(),
                size: self.forms.text.size(d.client()),
                fixed: true,
                modal: true,
            });
        }
        if self.browse.is_some() {
            out.push(Child {
                id: BROWSE,
                title: "Browse".into(),
                size: self.browse.as_ref().map_or(filedlg::CLIENT, |d| d.client()),
                fixed: false,
                modal: true,
            });
        }
        if let Some(text) = &self.text {
            if let Some((_, b)) = &self.remove {
                out.push(b.child(REMOVE, text, &self.forms));
            }
            if let Some(b) = &self.clear_fingers {
                out.push(b.child(CLEAR_FINGERS, text, &self.forms));
            }
            if let Some(b) = &self.error {
                out.push(b.child(ERROR, text, &self.forms));
            }
        }
        out
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        self.fonts();
        self.waker = Some(waker);
        self.run(Vec::new(), After::Read);
        self.read_remote(Duration::ZERO);
        repriseapp::Start::Continue
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.text = Some(ctx.text.clone());
        let was = std::mem::replace(&mut self.forms, ctx.forms()).text;
        if self.forms.text != was {
            self.regrow();
        }
        let mut reply = self.dispatch(win, ev, ctx);
        let size = self.forms.text.size(users::CLIENT);
        if size != self.asked {
            self.asked = size;
            reply.size = Some(size);
            reply.redraw_all = true;
        }
        reply
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.text = Some(ctx.text.clone());
        let cs = &ctx.theme.colors;
        let forms = ctx.forms();
        let px = forms.fonts.size_px;
        let paint = Paint {
            s: 1,
            ..forms.paint()
        };
        match win {
            PROPS => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(p) = &self.props {
                    propsheet::draw(c, &ctx.text, &paint, &p.sheet, true, px);
                }
            }
            ADD => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(d) = &self.add {
                    form::draw(c, &ctx.text, &paint, &d.form, true, px);
                }
            }
            RD_PASSWORD => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(d) = &self.rd_password {
                    form::draw(c, &ctx.text, &paint, &d.form, true, px);
                }
            }
            ADD_FINGER => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(d) = &self.add_finger {
                    form::draw(c, &ctx.text, &paint, &d.form, true, px);
                }
            }
            CLEAR_FINGERS => {
                if let Some(b) = &self.clear_fingers {
                    b.draw(c, &forms, &ctx.text);
                }
            }
            BROWSE => {
                if let Some(d) = &self.browse {
                    d.draw(
                        c,
                        &ctx.text,
                        &paint,
                        Rect::new(0, 0, d.client().0, d.client().1),
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
            _ => self.users.draw(c, ctx, self.face.as_ref()),
        }
    }
}

impl repriseapp::Extended for AccountsApp {
    fn text_sized(&self) -> bool {
        true
    }

    fn fixed(&self) -> bool {
        true
    }

    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        match &self.browse {
            Some(d) if win == BROWSE => Some(d.min_client()),
            _ => None,
        }
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        let px = forms.fonts.size_px;
        match win {
            PROPS => {
                if let Some(p) = &self.props {
                    oneshot::access_sheet(&p.sheet, tree, &ctx.text, px);
                }
            }
            ADD => {
                if let Some(d) = &self.add {
                    oneshot::access_form(&d.form, tree, &ctx.text, px);
                }
            }
            RD_PASSWORD => {
                if let Some(d) = &self.rd_password {
                    oneshot::access_form(&d.form, tree, &ctx.text, px);
                }
            }
            ADD_FINGER => {
                if let Some(d) = &self.add_finger {
                    oneshot::access_form(&d.form, tree, &ctx.text, px);
                }
            }
            CLEAR_FINGERS => {
                if let Some(b) = &self.clear_fingers {
                    b.access(tree, &ctx.text, &forms);
                }
            }
            BROWSE => {
                if let Some(d) = &self.browse {
                    let r = Rect::new(0, 0, d.client().0, d.client().1);
                    d.access(tree, r, &ctx.text, px);
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
            _ => self.users.access(tree, ctx),
        }
    }

    fn drop_form(&mut self, win: WinId) -> Option<DropForm<'_>> {
        let form = match win {
            PROPS => self.props.as_mut()?.sheet.open_mut(),
            ADD => &mut self.add.as_mut()?.form,
            RD_PASSWORD => &mut self.rd_password.as_mut()?.form,
            ADD_FINGER => &mut self.add_finger.as_mut()?.form,
            _ => return None,
        };
        Some(DropForm { form, px: None })
    }

    fn drop_file_dialog(&mut self, win: WinId) -> Option<FileDrop<'_>> {
        (win == BROWSE).then_some(FileDrop {
            dlg: self.browse.as_mut()?,
            host: &mut self.browse_host,
        })
    }
}

impl AccountsApp {
    fn dispatch(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        if self.face.is_none() {
            self.face = ctx
                .icons
                .get("avatar-default", self.forms.text.len(ROW_PX as i32));
            self.browse_host.icons.prime(&mut ctx.icons);
        }
        if matches!(ev, Event::Wake) {
            return self.wake();
        }
        let clip = form_clipboard(self.focused_edit(win), &ev);
        match win {
            PROPS => self.props_event(ev, ctx),
            ADD => self.add_event(ev, ctx),
            RD_PASSWORD => self.rd_password_event(ev, ctx),
            ADD_FINGER => self.add_finger_event(ev, ctx),
            CLEAR_FINGERS => self.clear_fingers_event(ev),
            BROWSE => self.browse_event(ev, ctx),
            REMOVE => self.remove_event(ev),
            ERROR => self.error_event(ev),
            _ => self.main_event(ev, ctx),
        }
        .with_clipboard(clip)
    }
}

/// Runs the program; `args` are the command line after the program's name.
/// `--remote-password` opens your Properties on Password.
pub fn main(args: Vec<std::ffi::OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let mut app = AccountsApp::new();
    app.open_yours = args.iter().any(|a| a == "--remote-password");
    repriseapp::run(Box::new(app)).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_apply_dropped_behind_a_job_deletes_its_picture() {
        let file = std::env::temp_dir().join(format!(
            "reprise-accounts-dropped-{}.png",
            std::process::id()
        ));
        std::fs::write(&file, "png").expect("write");
        let mut app = AccountsApp::new();
        let (_tx, rx) = channel();
        app.job = Some((rx, After::Read));
        let ops = vec![Op::Picture {
            path: "/org/freedesktop/Accounts/User1000".into(),
            file: file.clone(),
            you: false,
        }];
        assert!(
            !app.run(
                ops,
                After::Props {
                    close: false,
                    sheet: 0,
                    uid: 1000
                }
            ),
            "a second job is dropped"
        );
        assert!(!file.exists(), "the dropped Apply's picture is gone");
    }

    use crate::users::tests::user;

    fn test_ctx() -> Ctx {
        Ctx {
            theme: Theme::default(),
            text: TextRenderer::load(&reprisekit::theme::FontConfig::default()).into(),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: users::CLIENT,
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

    fn error_text(app: &AccountsApp) -> Option<String> {
        app.error.as_ref().map(|b| b.fields.text.clone())
    }

    #[test]
    fn an_apply_from_a_closed_sheet_lands_on_no_other_sheet() {
        let rows = vec![user(1000, "jo", true, true), user(1001, "al", false, false)];
        let mut app = AccountsApp::new();
        app.users.refresh(rows.clone(), vec![None, None]);
        app.open_props(1000);
        let after = After::Props {
            close: true,
            sheet: app.sheets,
            uid: 1000,
        };
        app.props = None;
        app.open_props(1001);
        let p = app.props.as_mut().unwrap();
        let page = &mut p.sheet.pages[props::TAB_GENERAL];
        page.focus_id("name");
        let ev = page.key(form::Key::Char('x'));
        p.event(propsheet::Event::Page(props::TAB_GENERAL, ev), |_| {
            Err(String::new())
        });
        let (tx, rx) = channel();
        let failed = Err(acct::Failure::Text("Not authorized.".into()));
        tx.send((failed, Ok(rows))).unwrap();
        app.job = Some((rx, after));
        assert!(app.accounts_done());
        let p = app.props.as_ref().expect("al's sheet stays open");
        assert_eq!(p.user.uid, 1001);
        assert!(p.dirty(), "al's edit stays");
        assert_eq!(
            error_text(&app).as_deref(),
            Some("The account jo:\n\nNot authorized.")
        );
    }

    #[test]
    fn the_delete_question_names_the_user_name_too() {
        let mut al = user(1001, "mallory", false, false);
        al.real_name = "Jo Doe".into();
        let mut app = AccountsApp::new();
        app.users
            .refresh(vec![user(1000, "jo", true, true), al], vec![None, None]);
        app.ask_remove(1001, &mut test_ctx());
        let text = app.remove.as_ref().map(|(_, b)| b.fields.text.clone());
        assert!(
            text.as_deref()
                .is_some_and(|t| t.starts_with("Delete the account Jo Doe (mallory)?\n")),
            "{text:?}"
        );
    }

    #[test]
    fn a_sheet_whose_account_went_closes_with_its_dialogs() {
        let rows = vec![user(1000, "jo", true, true), user(1001, "al", false, false)];
        let mut app = AccountsApp::new();
        app.users.refresh(rows.clone(), vec![None, None]);
        app.open_props(1001);
        app.open_rd_password(remotepassword::Action::Create);
        let (tx, rx) = channel();
        tx.send((Ok(()), Ok(rows[..1].to_vec()))).unwrap();
        app.job = Some((rx, After::Remove));
        assert!(app.accounts_done());
        assert!(app.props.is_none());
        assert!(
            app.rd_password.is_none(),
            "no dialog is left without its sheet"
        );
    }

    #[test]
    fn remove_greys_until_the_change_under_way_lands() {
        let rows = vec![user(1000, "jo", true, true), user(1001, "al", false, false)];
        let mut app = AccountsApp::new();
        app.users.refresh(rows.clone(), vec![None, None]);
        app.users.list.select_row(Some(1));
        let (tx, rx) = channel();
        app.job = Some((rx, After::Read));
        app.set_busy(true);
        assert_eq!(app.users.delete_key(), users::Outcome::None);
        tx.send((Ok(()), Ok(rows))).unwrap();
        assert!(app.accounts_done());
        assert_eq!(app.users.delete_key(), users::Outcome::Remove(1001));
    }

    #[test]
    fn a_logged_on_account_is_not_offered_for_deletion() {
        let mut al = user(1001, "al", false, false);
        al.real_name = "Al Bo".into();
        al.logged_on = true;
        let mut app = AccountsApp::new();
        app.users
            .refresh(vec![user(1000, "jo", true, true), al], vec![None, None]);
        app.ask_remove(1001, &mut test_ctx());
        assert!(app.remove.is_none(), "no Delete question");
        assert_eq!(
            error_text(&app).as_deref(),
            Some("Al Bo (al) is logged on. Delete the account once they have logged off.")
        );
    }

    fn virtual_reader() -> reprisebus::fprint::Found {
        reprisebus::fprint::Found::Reader(reprisebus::fprint::Reader {
            name: "Virtual reader".into(),
            swipe: false,
            stages: 5,
            fingers: vec!["right-index-finger".into()],
        })
    }

    fn with_reader() -> AccountsApp {
        let mut app = AccountsApp::new();
        app.finger_sources = fingers::Sources {
            read: virtual_reader,
            delete: |_| Err(reprisebus::fprint::Refusal::NotAuthorized),
        };
        app.users.refresh(
            vec![user(1000, "jo", true, true), user(1001, "al", false, false)],
            vec![None, None],
        );
        app
    }

    #[test]
    fn only_your_own_sheet_reads_the_reader() {
        let mut app = with_reader();
        app.open_props(1001);
        assert!(app.fingers.reading.is_none());
        app.props = None;
        app.open_props(1000);
        assert!(app.fingers_done());
        assert_eq!(
            app.props.unwrap().fingers(),
            [fingerprints::Finger::RightIndex]
        );
    }

    #[test]
    fn a_refused_delete_says_why_and_reads_the_fingers_again() {
        let mut app = with_reader();
        app.open_props(1000);
        app.fingers.found = None;
        app.delete_fingers(Some(fingerprints::Finger::RightIndex));
        assert!(app.fingers.facts(true).busy);
        assert!(app.fingers_done());
        assert_eq!(
            error_text(&app).as_deref(),
            Some("You weren't allowed to delete the fingerprint.")
        );
        assert_eq!(app.fingers.found, Some(virtual_reader()));
        assert!(!app.fingers.facts(true).busy);
    }

    #[test]
    fn an_adds_steps_reach_only_the_dialog_that_started_it() {
        use reprisebus::fprint::Adding;
        let mut app = with_reader();
        app.open_props(1000);
        app.fingers_done();
        app.fingers_action(fingerprints::Action::Add, &mut test_ctx());
        let d = app.add_finger.as_mut().unwrap();
        assert_eq!(
            d.picked(),
            Some(fingerprints::Finger::LeftThumb),
            "right index is added"
        );
        d.event(form::Event::Default);
        let send = |app: &mut AccountsApp, from| {
            let (tx, rx) = channel();
            let (stop, _) = reprisebus::fprint::Stop::new();
            tx.send(fingers::Msg::Step(Adding::Started)).unwrap();
            tx.send(fingers::Msg::Ended(Ok(()))).unwrap();
            app.fingers.add = Some(fingers::Add { from, rx, stop });
            app.fingers_done();
        };
        send(&mut app, 0);
        let d = app.add_finger.as_ref().unwrap();
        assert!(
            matches!(d.step, addfinger::Step::Waiting(_)),
            "{:?}",
            d.step
        );
        let from = d.generation;
        send(&mut app, from);
        let d = app.add_finger.as_ref().unwrap();
        assert!(
            matches!(d.step, addfinger::Step::Scanning { .. }),
            "{:?}",
            d.step
        );
    }
}
