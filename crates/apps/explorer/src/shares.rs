//! Network shares: Connect to Server, typed addresses and the Network place connect through
//! `netshare::mount`, whose prompts show here as boxes; Computer's Network Locations disconnect.

use reprisekit::tr;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use repriseapp::iconjobs::IconWaits;
use repriseapp::{Ctx, Event, Reply};
use reprisefs::netshare::mount::{self, Answer, Connecting, Credentials, Failed, Step};
use reprisefs::netshare::{self, Address, Snapshot};
use reprisekit::controls::form::{self, Form};
use reprisekit::controls::netconnect::{self as nc, Login};

use crate::Explorer;
use crate::dialogs::Dialog;
use crate::place::{self, Place};

/// Connect to Server, kept while a connection is under way so it can show again between the
/// boxes it raises.
pub struct ConnectBox {
    pub form: Form,
    waits: IconWaits,
}

pub struct LoginBox {
    pub form: Form,
    pub host: String,
    /// The message unwrapped, wrapped again at each text size.
    message: String,
    waits: IconWaits,
}

pub(crate) struct Connection {
    /// `None` in a test.
    conn: Option<Connecting>,
    steps: Receiver<Step>,
    addr: Address,
    /// Connect to Server, while the boxes it raises are up.
    connect: Option<ConnectBox>,
}

/// `text` wrapped to the boxes' message width at `ctx`'s text size, at most three lines.
fn wrapped(text: &str, ctx: &Ctx) -> Vec<String> {
    let forms = ctx.forms();
    let width = forms.text.len(nc::MSG_W);
    let mut out = Vec::new();
    for para in text.split('\n').filter(|l| !l.trim().is_empty()) {
        out.extend(reprisekit::text::wrap(
            &ctx.text,
            para,
            width,
            forms.fonts.size_px,
        ));
    }
    out.truncate(nc::MSG_LINES);
    out
}

fn icon(ctx: &mut Ctx, waits: &mut IconWaits, name: &str) -> Option<reprisekit::raster::Canvas> {
    let px = ctx.forms().text.len(32);
    waits.first(&mut ctx.icons, &[name], px)
}

impl ConnectBox {
    fn open(address: &str, ctx: &mut Ctx) -> ConnectBox {
        let mut waits = IconWaits::default();
        let icon = icon(ctx, &mut waits, nc::CONNECT_ICON);
        let form = nc::connect_form(&wrapped(nc::CONNECT_MESSAGE, ctx), address, icon);
        ConnectBox { form, waits }
    }

    pub fn land_icon(&mut self, ctx: &mut Ctx) -> bool {
        if !self.waits.landed(&mut ctx.icons) {
            return false;
        }
        self.waits = IconWaits::default();
        if let Some(c) = icon(ctx, &mut self.waits, nc::CONNECT_ICON) {
            self.form.set_picture("icon", c);
        }
        true
    }

    fn rewrap(&mut self, ctx: &Ctx) {
        self.form
            .set_lines("message", &wrapped(nc::CONNECT_MESSAGE, ctx));
    }
}

impl LoginBox {
    fn open(l: &mount::Login, host: &str, ctx: &mut Ctx) -> LoginBox {
        let mut waits = IconWaits::default();
        let icon = icon(ctx, &mut waits, nc::LOGIN_ICON);
        let lines = wrapped(&l.message, ctx);
        let mut form = nc::login_form(
            &Login {
                lines: &lines,
                user: &l.user,
                asks_user: l.asks_user,
                domain: l.domain.then_some(""),
                can_remember: l.can_remember,
            },
            icon,
        );
        if l.retry {
            form.set_text(nc::STATUS, nc::WRONG);
        }
        LoginBox {
            form,
            host: host.to_string(),
            message: l.message.clone(),
            waits,
        }
    }

    pub fn land_icon(&mut self, ctx: &mut Ctx) -> bool {
        if !self.waits.landed(&mut ctx.icons) {
            return false;
        }
        self.waits = IconWaits::default();
        if let Some(c) = icon(ctx, &mut self.waits, nc::LOGIN_ICON) {
            self.form.set_picture("icon", c);
        }
        true
    }

    fn credentials(&self) -> Credentials {
        Credentials {
            user: self.form.edit_text(nc::USER).to_string(),
            domain: self.form.edit_text(nc::DOMAIN).to_string(),
            password: self.form.edit_text(nc::PASSWORD).to_string().into(),
            remember: self.form.checked(nc::REMEMBER) == Some(true),
        }
    }
}

impl Drop for LoginBox {
    fn drop(&mut self) {
        nc::wipe(&mut self.form);
    }
}

/// What a question box asks about.
pub enum Asking {
    /// A server's question during a connection.
    Connection,
    /// Whether to drop what is still open on a share and unmount it.
    Disconnect(netshare::Share),
}

impl Asking {
    /// The button Enter presses among `n`: for a server's question the last, which gvfs makes
    /// its Cancel, so Enter never accepts an unknown host key.
    pub fn default_choice(&self, n: usize) -> usize {
        match self {
            Asking::Connection => n.saturating_sub(1),
            Asking::Disconnect(_) => 0,
        }
    }
}

/// The box title for a password asked by `host`.
pub fn login_title(host: &str) -> String {
    tr!("shares-login-title-connect-to-host", host = host)
}

impl Explorer {
    /// Take gvfs's mounted shares as they change, so Computer gains and loses a share mounted
    /// or unmounted anywhere.
    pub(crate) fn follow_shares(&mut self, wake: repriseapp::Waker) {
        let (tx, rx) = std::sync::mpsc::channel();
        netshare::follow(move |snap| {
            if tx.send(snap).is_ok() {
                wake.wake();
            }
        });
        self.share_changes = Some(rx);
    }

    /// The latest shares `follow_shares` posted, if any arrived.
    pub(crate) fn poll_shares(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let snap = self.share_changes.as_ref()?.try_iter().last()?;
        Some(self.shares_changed(snap, ctx))
    }

    /// `p`'s title, a mounted share's folder by the share's name.
    pub(crate) fn title_of(&self, p: &Place) -> String {
        p.title_in(&self.shares.shares)
    }

    /// Takes gvfs's mounts, its phones and cameras named after their devices.
    pub(crate) fn shares_changed(&mut self, mut snap: Snapshot, ctx: &mut Ctx) -> Reply {
        reprisefs::portable::dress(&mut snap.shares, &self.portable);
        if snap == self.shares {
            return Reply::none();
        }
        let here = self.place.path();
        let holds = |s: &netshare::Share| here.as_ref().is_some_and(|h| h.starts_with(&s.path));
        let gone: Vec<&netshare::Share> = (self.shares.shares.iter())
            .filter(|s| snap.share_at(&s.path).is_none())
            .collect();
        let new: Vec<&netshare::Share> = (snap.shares.iter())
            .filter(|s| self.shares.share_at(&s.path).is_none())
            .collect();
        let (left, arrived) = (gone.iter().any(|s| holds(s)), new.iter().any(|s| holds(s)));
        // A share's folder may stand as a plain folder where the walk to it went before it was
        // listed as a share (gvfs's folder, under Local Disk): that node is read again.
        let parents: BTreeSet<PathBuf> = (gone.iter().chain(&new))
            .filter_map(|s| s.path.parent().map(Path::to_path_buf))
            .collect();
        self.shares = snap;
        self.drive_mounts = place::drive_mounts(&self.drives, &self.shares.shares);
        self.refollow |= arrived;
        self.refill_tree_node(&Place::Computer, ctx);
        for p in &parents {
            self.refill_tree_node(&Place::from_path(p), ctx);
        }
        self.build_address(ctx);
        if left {
            // The share's folder is gone; its parent is gvfs's own folder, not a place to land.
            return self.go(Place::Computer, ctx);
        }
        // gvfs's folder is its own FUSE mount, which no folder watch sees change.
        let lists_one = here.as_ref().is_some_and(|h| parents.contains(h));
        if self.place == Place::Computer || lists_one {
            return self.refresh(ctx);
        }
        // A share's root now has its name and Computer above it.
        self.pane_key = None;
        self.refresh_chrome(ctx);
        Reply::redraw_all()
    }

    /// File > Connect to Server...: the box with the last address typed.
    pub(crate) fn open_connect(&mut self, ctx: &mut Ctx) -> Reply {
        if self.connection.is_some() {
            return Reply::none();
        }
        let b = ConnectBox::open(&self.last_server, ctx);
        self.raise(Dialog::Connect(Box::new(b)))
    }

    /// Connects to `text`, an address, showing Connect to Server while it does.
    pub(crate) fn connect_to(&mut self, text: &str, ctx: &mut Ctx) -> Reply {
        let Some(addr) = Address::parse(text) else {
            return Reply::none();
        };
        if self.dialog().is_some() || self.connection.is_some() {
            return Reply::none();
        }
        let mut b = ConnectBox::open(&addr.uri(), ctx);
        self.start_connection(addr, &mut b, ctx);
        self.raise(Dialog::Connect(Box::new(b)));
        // The address bar goes back to the place shown; the address is in the box.
        self.build_address(ctx);
        Reply::redraw_all()
    }

    fn start_connection(&mut self, addr: Address, b: &mut ConnectBox, ctx: &Ctx) {
        nc::set_busy(
            &mut b.form,
            true,
            &tr!("shares-connecting-to-host", host = addr.host.as_str()),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = ctx.wake.clone();
        // A test presses Connect without running gio.
        let conn = (!cfg!(test)).then(|| {
            mount::connect(addr.clone(), move |step| {
                if tx.send(step).is_ok() {
                    wake.wake();
                }
            })
        });
        self.connection = Some(Connection {
            conn,
            steps: rx,
            addr,
            connect: None,
        });
    }

    /// Connect to Server's and the password box's own events.
    pub(crate) fn connect_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let resized = matches!(ev, Event::Resized);
        let (form, clip) = match self.dialogs.up_mut() {
            Some(Dialog::Connect(b)) => {
                if resized {
                    b.rewrap(ctx);
                }
                (&mut b.form, None)
            }
            Some(Dialog::Login(b)) => {
                if resized {
                    let lines = wrapped(&b.message, ctx);
                    b.form.set_lines("message", &lines);
                }
                (&mut b.form, None)
            }
            _ => return Reply::none(),
        };
        if resized {
            return Reply::redraw();
        }
        let clip =
            clip.unwrap_or_else(|| repriseapp::keys::form_clipboard(form.focused_edit(), &ev));
        let out = match ev {
            Event::Pasted(text) => form.paste(text),
            ev => match repriseapp::keys::form_input(form, &ev, ctx) {
                Some(out) => out,
                None => return Reply::none().with_clipboard(clip),
            },
        };
        self.connect_answered(out, ctx).with_clipboard(clip)
    }

    fn connect_answered(&mut self, out: form::Event, ctx: &mut Ctx) -> Reply {
        match (self.dialogs.up_mut(), out) {
            (Some(Dialog::Connect(b)), form::Event::Changed(_)) => {
                nc::follow(&mut b.form);
                Reply::redraw()
            }
            (Some(Dialog::Login(b)), form::Event::Changed(_)) => {
                nc::follow(&mut b.form);
                Reply::redraw()
            }
            (Some(_), form::Event::Pressed(nc::CANCEL) | form::Event::Cancel) => {
                self.dismiss_up(ctx)
            }
            (Some(Dialog::Connect(b)), form::Event::Pressed(nc::OK) | form::Event::Default) => {
                if !nc::enabled(&b.form, nc::OK) {
                    return Reply::none();
                }
                let typed = b.form.edit_text(nc::ADDRESS).trim().to_string();
                let Some(addr) = Address::parse(&typed) else {
                    b.form.set_text(
                        nc::STATUS,
                        &tr!("shares-connect-answered-that-is-not-a-server-address"),
                    );
                    return Reply::redraw();
                };
                self.last_server = match addr.password {
                    Some(_) => addr.uri(),
                    None => typed,
                };
                let Some(Dialog::Connect(mut b)) = self.take_up() else {
                    return Reply::none();
                };
                self.start_connection(addr, &mut b, ctx);
                match self.connect_behind_held(Dialog::Connect(b)) {
                    Some(connect) => {
                        self.put_back(connect);
                        Reply::redraw()
                    }
                    None => Reply::redraw_all(),
                }
            }
            (Some(Dialog::Login(b)), form::Event::Pressed(nc::OK) | form::Event::Default) => {
                if !nc::enabled(&b.form, nc::OK) {
                    return Reply::none();
                }
                let creds = b.credentials();
                nc::set_busy(
                    &mut b.form,
                    true,
                    &tr!("shares-connect-answered-connecting"),
                );
                nc::wipe(&mut b.form);
                if let Some(c) = self.connection.as_ref().and_then(|c| c.conn.as_ref()) {
                    c.answer(Answer::Login(creds));
                }
                self.show_connect_again()
            }
            (_, form::Event::None) => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    /// The question box's answer: the index of the button pressed, `None` for Cancel.
    pub(crate) fn question_answered(&mut self, choice: Option<usize>, ctx: &mut Ctx) -> Reply {
        if matches!(
            self.dialogs.up(),
            Some(Dialog::Question {
                about: Asking::Disconnect(_),
                ..
            })
        ) {
            let Some(Dialog::Question {
                about: Asking::Disconnect(share),
                ..
            }) = self.take_up()
            else {
                return Reply::none();
            };
            let reply = match choice {
                Some(0) => self.unmount_share(share, true, ctx),
                _ => Reply::none(),
            };
            return Reply {
                redraw_all: true,
                ..reply
            };
        }
        match (choice, &self.connection) {
            (Some(i), Some(c)) => {
                if let Some(conn) = &c.conn {
                    conn.answer(Answer::Choice(i));
                }
                self.show_connect_again()
            }
            _ => self.cancel_connection(),
        }
    }

    /// Whether the box up is Connect to Server showing a connection under way, which takes no
    /// input but Cancel.
    pub(crate) fn connecting_up(&self) -> bool {
        self.connection.is_some() && matches!(self.dialogs.up(), Some(Dialog::Connect(_)))
    }

    /// Connect to Server, still busy, back up while the connection goes on, or behind a held box
    /// waiting ([`Explorer::connect_behind_held`]).
    fn show_connect_again(&mut self) -> Reply {
        self.take_up();
        let connect = self.connection.as_mut().and_then(|c| c.connect.take());
        match connect.and_then(|b| self.connect_behind_held(Dialog::Connect(Box::new(b)))) {
            Some(connect) => self.raise(connect),
            None => Reply::redraw_all(),
        }
    }

    pub(crate) fn cancel_connection(&mut self) -> Reply {
        if let Some(conn) = self.connection.take().and_then(|c| c.conn) {
            conn.cancel();
        }
        self.take_connection_boxes();
        Reply::redraw_all()
    }

    /// Takes Connect to Server and the connection's password and question boxes, up or waiting;
    /// the Connect to Server box if there was one.
    fn take_connection_boxes(&mut self) -> Option<ConnectBox> {
        let ours = |d: &Dialog| {
            matches!(
                d,
                Dialog::Connect(_)
                    | Dialog::Login(_)
                    | Dialog::Question {
                        about: Asking::Connection,
                        ..
                    }
            )
        };
        let up = self.take_up_if(ours);
        let mut connect = None;
        for d in up.into_iter().chain(self.dialogs.withdraw(ours)) {
            if let Dialog::Connect(b) = d {
                connect = Some(*b);
            }
        }
        connect
    }

    /// The connection's next prompt or its outcome, if one arrived.
    pub(crate) fn poll_connection(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let step = self.connection.as_ref()?.steps.try_recv().ok()?;
        let reply = self.connection_step(step, ctx);
        self.answer_ends();
        Some(reply)
    }

    fn connection_step(&mut self, step: Step, ctx: &mut Ctx) -> Reply {
        let host = match &self.connection {
            Some(c) => c.addr.host.clone(),
            None => return Reply::none(),
        };
        // The prompt waits behind any other box up; the busy Connect to Server is kept for after.
        let shelve = |ex: &mut Explorer| {
            if let Some(b) = ex.take_connection_boxes()
                && let Some(c) = ex.connection.as_mut()
            {
                c.connect = Some(b);
            }
        };
        match step {
            Step::Login(l) => {
                shelve(self);
                let b = LoginBox::open(&l, &host, ctx);
                self.raise(Dialog::Login(Box::new(b)))
            }
            Step::Question(mut q) => {
                shelve(self);
                // A message box holds three buttons.
                q.choices.truncate(3);
                self.raise(Dialog::Question {
                    title: login_title(&host),
                    text: q.message,
                    choices: q.choices,
                    about: Asking::Connection,
                    focus: None,
                })
            }
            Step::Done(outcome) => {
                let Some(c) = self.connection.take() else {
                    return Reply::none();
                };
                let up = matches!(self.dialogs.up(), Some(Dialog::Connect(_)));
                let shown = self.take_connection_boxes();
                match outcome {
                    Err(Failed::Error(e)) => {
                        let mut b = (shown.or(c.connect))
                            .unwrap_or_else(|| ConnectBox::open(&c.addr.uri(), ctx));
                        nc::set_busy(&mut b.form, false, &e);
                        let b = Dialog::Connect(Box::new(b));
                        // Up, it stays in its window; in place of the password box or a question it
                        // maps afresh.
                        if up {
                            self.put_back(b);
                        } else {
                            self.raise(b);
                        }
                        Reply::redraw_all()
                    }
                    done => {
                        let reply = match done {
                            Ok(Some(path)) => self.go(Place::Folder(path), ctx),
                            Ok(None) => self.go(Place::Network(c.addr.uri()), ctx),
                            Err(_) => Reply::none(),
                        };
                        Reply {
                            redraw_all: true,
                            ..reply
                        }
                    }
                }
            }
        }
    }

    /// Disconnect on a share's row: gvfs unmounts it, and Computer drops the row when gvfs says so.
    pub(crate) fn disconnect_selected(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(it) = self.sv.selection.single().and_then(|i| self.items.get(i)) else {
            return Reply::none();
        };
        let Some(share) = place::row_share(&self.shares.shares, it).cloned() else {
            return Reply::none();
        };
        self.unmount_share(share, false, ctx)
    }

    /// Unmounts `share` on a job; one in use asks before `force` drops what is open on it.
    fn unmount_share(&mut self, share: netshare::Share, force: bool, ctx: &mut Ctx) -> Reply {
        self.spawn_job(ctx, move || {
            let done = netshare::unmount(&share.path, force);
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                let title = tr!("shares-problem-disconnecting", name = share.name.as_str());
                match done {
                    Ok(()) => Reply::none(),
                    Err(netshare::Unmount::Busy) => {
                        let title = tr!("shares-disconnect-title", name = share.name.as_str());
                        let text = tr!("shares-disconnect-busy", name = share.name.as_str());
                        ex.raise(Dialog::Question {
                            title,
                            text,
                            choices: vec![
                                tr!("shares-unmount-share-disconnect"),
                                reprisekit::kit_tr!("cancel"),
                            ],
                            about: Asking::Disconnect(share),
                            focus: None,
                        })
                    }
                    Err(netshare::Unmount::Error(e)) => ex.message_box(&title, e, ctx),
                }
            })
        });
        Reply::redraw()
    }

    /// Opens a server or share listed in the Network place: an address connects first, anything
    /// else gvfs lists (a workgroup) is listed in place.
    pub(crate) fn open_network(&mut self, uri: &str, ctx: &mut Ctx) -> Reply {
        match Address::parse(uri) {
            Some(_) => self.connect_to(uri, ctx),
            None => self.go(Place::Network(uri.to_string()), ctx),
        }
    }

    /// The address of the server or share selected in the Network place.
    pub(crate) fn selected_network(&self) -> Option<String> {
        if !matches!(self.place, Place::Network(_)) {
            return None;
        }
        let it = self.items.get(self.sv.selection.single()?)?;
        Some(it.path.to_string_lossy().into_owned())
    }

    /// The selected row is a mounted share.
    pub(crate) fn share_selected(&self, i: usize) -> bool {
        self.place == Place::Computer
            && (self.items.get(i))
                .is_some_and(|it| place::row_share(&self.shares.shares, it).is_some())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tests::{test_ctx, test_explorer};
    use std::sync::mpsc::Sender;

    /// An Explorer with a connection to `uri` under way whose steps the test sends.
    pub(crate) fn connecting(uri: &str, ctx: &mut Ctx) -> (Explorer, Sender<Step>) {
        let mut ex = test_explorer(Place::Computer, ctx);
        let steps = connect_in(&mut ex, uri, ctx);
        (ex, steps)
    }

    /// Starts a connection to `uri` in `ex`, Connect to Server up and busy; the test sends its
    /// steps.
    pub(crate) fn connect_in(ex: &mut Explorer, uri: &str, ctx: &mut Ctx) -> Sender<Step> {
        let addr = Address::parse(uri).expect("an address");
        let (tx, steps) = std::sync::mpsc::channel();
        let mut b = ConnectBox::open(&addr.uri(), ctx);
        nc::set_busy(&mut b.form, true, "Connecting to nas...");
        ex.raise(Dialog::Connect(Box::new(b)));
        ex.connection = Some(Connection {
            conn: None,
            steps,
            addr,
            connect: None,
        });
        tx
    }

    pub(crate) fn login(retry: bool) -> mount::Login {
        mount::Login {
            message: "Password required for share private on nas".into(),
            user: "alice".into(),
            asks_user: true,
            domain: true,
            retry,
            can_remember: true,
        }
    }

    fn status(form: &Form) -> String {
        match form.item(nc::STATUS).map(|i| &i.control) {
            Some(form::Control::Label { text, .. }) => text.clone(),
            _ => String::new(),
        }
    }

    #[test]
    fn a_password_request_raises_the_password_box_and_ok_brings_connect_back() {
        let mut ctx = test_ctx();
        let (mut ex, tx) = connecting("smb://nas/private", &mut ctx);
        tx.send(Step::Login(login(false))).unwrap();
        ex.poll_connection(&mut ctx);
        let Some(Dialog::Login(b)) = ex.dialogs.up_mut() else {
            panic!("the password box is up");
        };
        assert_eq!(login_title(&b.host), "Connect to nas");
        assert_eq!(b.form.edit_text(nc::USER), "alice");
        assert_eq!(status(&b.form), "");
        b.form.show_edit_text(nc::PASSWORD, "secret");
        ex.connect_answered(form::Event::Default, &mut ctx);
        let Some(Dialog::Connect(b)) = ex.dialog() else {
            panic!("Connect to Server is back while the server checks the password");
        };
        assert!(!nc::enabled(&b.form, nc::OK), "still busy");

        tx.send(Step::Login(login(true))).unwrap();
        ex.poll_connection(&mut ctx);
        let Some(Dialog::Login(b)) = ex.dialog() else {
            panic!("asked again");
        };
        assert_eq!(status(&b.form), nc::WRONG);
        assert_eq!(b.form.edit_text(nc::PASSWORD), "", "the field starts empty");
    }

    /// A box raised at a wake while the password box is up waits behind it, and comes up once the
    /// password box is answered.
    #[test]
    fn a_box_raised_over_the_password_box_waits_for_its_answer() {
        let mut ctx = test_ctx();
        let (mut ex, tx) = connecting("smb://nas/private", &mut ctx);
        tx.send(Step::Login(login(false))).unwrap();
        ex.poll_connection(&mut ctx);
        ex.message_box("Error Compressing", "The disk is full.".into(), &mut ctx);
        assert!(
            matches!(ex.dialog(), Some(Dialog::Login(_))),
            "the password box stays up"
        );

        ex.event(ex.dialog_win(), Event::CloseRequest, &mut ctx);
        assert!(ex.connection.is_none(), "the connection is cancelled");
        assert!(
            matches!(ex.dialog(), Some(Dialog::Message { text, .. }) if text == "The disk is full."),
            "the error box comes up after"
        );
    }

    #[test]
    fn a_failed_connection_says_why_in_connect_to_server_and_takes_another_address() {
        let mut ctx = test_ctx();
        let (mut ex, tx) = connecting("smb://nas/public", &mut ctx);
        tx.send(Step::Done(Err(Failed::Error(
            "Failed to mount share: Connection refused".into(),
        ))))
        .unwrap();
        let win = ex.dialog_win();
        ex.poll_connection(&mut ctx);
        assert!(ex.connection.is_none());
        assert_eq!(ex.dialog_win(), win, "in the window it was in");
        let Some(Dialog::Connect(b)) = ex.dialog() else {
            panic!("Connect to Server stays up");
        };
        assert_eq!(status(&b.form), "Failed to mount share: Connection refused");
        assert!(nc::enabled(&b.form, nc::ADDRESS) && nc::enabled(&b.form, nc::OK));
        assert_eq!(b.form.edit_text(nc::ADDRESS), "smb://nas/public");
    }

    #[test]
    fn a_question_raises_a_box_with_one_button_per_choice() {
        let mut ctx = test_ctx();
        let (mut ex, tx) = connecting("sftp://box/", &mut ctx);
        tx.send(Step::Question(mount::Question {
            message: "The identity of box is unknown.".into(),
            choices: vec!["Log In Anyway".into(), "Cancel Login".into()],
        }))
        .unwrap();
        ex.poll_connection(&mut ctx);
        let Some(Dialog::Question { title, choices, .. }) = ex.dialog() else {
            panic!("the question is up");
        };
        assert_eq!(title, "Connect to box");
        assert_eq!(choices, &["Log In Anyway", "Cancel Login"]);
        assert_eq!(Asking::Connection.default_choice(choices.len()), 1);
        ex.question_answered(Some(0), &mut ctx);
        assert!(matches!(ex.dialog(), Some(Dialog::Connect(_))));
        ex.question_answered(None, &mut ctx);
        assert!(
            ex.dialog().is_none() && ex.connection.is_none(),
            "Cancel ends it"
        );
    }

    #[test]
    fn a_mounted_share_opens_its_folder_and_a_server_lists_its_shares() {
        let mut ctx = test_ctx();
        let dir = tempfile::tempdir().expect("tempdir");
        let (mut ex, tx) = connecting("smb://nas/public", &mut ctx);
        tx.send(Step::Done(Ok(Some(dir.path().to_path_buf()))))
            .unwrap();
        ex.poll_connection(&mut ctx);
        assert!(ex.dialog().is_none());
        assert_eq!(ex.place, Place::Folder(dir.path().to_path_buf()));

        let (mut ex, tx) = connecting("smb://nas", &mut ctx);
        tx.send(Step::Done(Ok(None))).unwrap();
        ex.poll_connection(&mut ctx);
        assert_eq!(ex.place, Place::Network("smb://nas/".into()));
        assert_eq!(ex.place.title(), "nas");
        assert_eq!(ex.place.parent(), Some(Place::network()));
    }

    /// A connection that lands takes Connect to Server and opens no box of its own, so a box a
    /// later step of the same wake raises waits behind the held box already waiting.
    #[test]
    fn a_box_raised_after_a_connection_lands_waits_behind_a_held_box() {
        let mut ctx = test_ctx();
        let dir = tempfile::tempdir().expect("tempdir");
        let (mut ex, tx) = connecting("smb://nas/public", &mut ctx);
        let meter = std::sync::Arc::new(reprisefs::progress::Meter::default());
        let job = reprisefs::progress::Asked::new(meter);
        ex.ask_failed(job, "Error Copying File or Folder", "x".into(), &mut ctx);
        tx.send(Step::Done(Ok(Some(dir.path().to_path_buf()))))
            .unwrap();
        ex.poll_connection(&mut ctx);
        ex.message_box("Explorer", "x".into(), &mut ctx);
        assert!(ex.holds_a_job(), "the failure box is up");
    }

    #[test]
    fn computer_lists_a_mounted_share_and_drops_it_when_unmounted() {
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Computer, &mut ctx);
        let share = netshare::Share {
            name: "public on nas".into(),
            icon: "folder-remote".into(),
            path: "/run/user/1000/gvfs/smb-share:server=nas,share=public".into(),
            backend: "smb-share".into(),
            host: "nas".into(),
        };
        let snap = Snapshot {
            gio: true,
            shares: vec![share.clone()],
        };
        ex.shares_changed(snap, &mut ctx);
        crate::tests::wake_until(&mut ex, &mut ctx, |ex| {
            ex.items.iter().any(|it| it.path == share.path)
        });
        let i = ex
            .items
            .iter()
            .position(|it| it.path == share.path)
            .unwrap();
        assert!(ex.share_selected(i));
        assert!(
            ex.drive_mounts.contains(&share.path),
            "Up from the share is Computer"
        );
        assert_eq!(
            Place::Folder(share.path.clone()).parent_in(&ex.drive_mounts),
            Some(Place::Computer)
        );
        ex.shares_changed(Snapshot::default(), &mut ctx);
        crate::tests::wake_until(&mut ex, &mut ctx, |ex| {
            !ex.items.iter().any(|it| it.path == share.path)
        });
    }
}
