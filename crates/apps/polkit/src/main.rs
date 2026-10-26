//! reprise-polkit: the polkit authentication agent. When an action needs a
//! password, polkitd calls us, we show a Run As dialog, and the password goes to
//! polkit's own helper, which runs PAM. Nothing here checks a password.
//!
//! For a user with a finger enrolled, the helper's conversation starts as the
//! dialog opens, so a PAM stack with `pam_fprintd` takes a finger in place of
//! the password; see `helper`.
//!
//! No main window: the process lives for the session, and each request opens a
//! modal dialog, one at a time; the rest queue.

mod agent;
mod dialog;
mod helper;

use std::collections::VecDeque;
use std::sync::mpsc::Receiver;

use repriseapp::keys::{form_clipboard, form_input};
use repriseapp::oneshot;
use repriseapp::{App, Child, Ctx, Event, Reply, WinId};
use reprisekit::access::Tree;
use reprisekit::controls::form;
use reprisekit::controls::hosted::Hosted;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::theme::TextUnits;

use agent::{Msg, Outcome, Request};
use dialog::RunAs;
use helper::{Report, Verdict};

const RUNAS: WinId = "runas";
const KEY_ICON: &str = "polkit-gnome-authorization";
const USER_ICON: &str = "system-users";
const ATTEMPTS: u8 = 3;

fn request_icons(ctx: &mut Ctx) -> (Option<Canvas>, Option<Canvas>) {
    (ctx.icons.get(KEY_ICON, 32), ctx.icons.get(USER_ICON, 16))
}

struct Active {
    request: Request,
    dialog: RunAs,
    attempts: u8,
    /// The helper conversation in flight, and whom it is for. Dropping it hangs up.
    session: Option<(helper::Session, String)>,
    reports: Option<Receiver<Report>>,
    prompted: bool,
    /// The first PAM error or info text since the password prompt (or the
    /// conversation's start), shown on failure instead of the generic text. The
    /// first explains a rejection (pam_faillock's "account locked" precedes its
    /// minutes-left line); text before the prompt is the fingerprint reader's.
    notice: Option<String>,
    /// The status line shows why the last attempt failed; the next conversation's
    /// info text does not hide it, an error does.
    failure_shown: bool,
    /// The last info text held back by `failure_shown`, shown once the user submits
    /// while the helper still waits on that step.
    held: Option<String>,
}

impl Active {
    fn new(request: Request, dialog: RunAs) -> Active {
        Active {
            request,
            dialog,
            attempts: 0,
            session: None,
            reports: None,
            prompted: false,
            notice: None,
            failure_shown: false,
            held: None,
        }
    }

    /// Hang up on any conversation in flight and start one for the user the dialog
    /// shows. Without a waker (the tests) nothing starts.
    fn begin(&mut self, waker: Option<&repriseapp::Waker>) {
        self.session = None;
        self.reports = None;
        self.prompted = false;
        self.notice = None;
        self.held = None;
        let Some(waker) = waker else {
            return;
        };
        let user = self.dialog.user();
        let (tx, rx) = std::sync::mpsc::channel();
        match helper::authenticate(user.clone(), self.request.cookie.clone(), tx, waker.clone()) {
            Some(session) => {
                self.session = Some((session, user));
                self.reports = Some(rx);
            }
            None => {
                self.dialog
                    .set_status("The polkit helper could not be started.");
            }
        }
    }
}

#[derive(Default)]
struct PolkitApp {
    msgs: Option<Receiver<Msg>>,
    queue: VecDeque<Request>,
    active: Option<Active>,
    waker: Option<repriseapp::Waker>,
    /// The text size the active dialog opened at.
    units: TextUnits,
}

impl PolkitApp {
    fn handle(&mut self, m: Msg) {
        match m {
            Msg::Begin(r) => self.queue.push_back(r),
            Msg::Cancel(cookie) => {
                if self
                    .active
                    .as_ref()
                    .is_some_and(|a| a.request.cookie == cookie)
                {
                    self.finish(Outcome::Withdrawn);
                } else if let Some(i) = self.queue.iter().position(|r| r.cookie == cookie)
                    && let Some(r) = self.queue.remove(i)
                {
                    let _ = r.done.send_blocking(Outcome::Withdrawn);
                }
            }
        }
    }

    fn open_next(
        &mut self,
        wrap: &dyn Fn(&str) -> Vec<String>,
        key: Option<Canvas>,
        user_icon: Option<Canvas>,
    ) {
        if self.active.is_some() {
            return;
        }
        let Some(request) = self.queue.pop_front() else {
            return;
        };
        let lines = wrap(&request.message);
        let dialog = RunAs::new(&lines, &request.users, request.selected, key, user_icon);
        let mut a = Active::new(request, dialog);
        a.begin(self.waker.as_ref());
        self.active = Some(a);
    }

    /// Close the active request with `out` and wipe its field. A Cancel drained in
    /// the same wake as a Done(Success) wins; polkitd ignores the late SUCCESS.
    fn finish(&mut self, out: Outcome) {
        if let Some(mut a) = self.active.take() {
            a.dialog.wipe();
            let _ = a.request.done.send_blocking(out);
        }
    }

    /// `open_next` with what a `Ctx` provides: the icons, and the message wrapped to
    /// the dialog's column.
    fn open_next_in(&mut self, ctx: &mut Ctx) {
        let (key, user) = request_icons(ctx);
        let forms = ctx.forms();
        let (px, width) = (forms.fonts.size_px, forms.text.len(dialog::MSG_W));
        let text = &ctx.text;
        let wrap = |m: &str| reprisekit::text::wrap(text, m, width, px);
        self.open_next(&wrap, key, user);
        self.units = forms.text;
    }

    fn report(&mut self, r: Report) {
        let Some(a) = self.active.as_mut() else {
            return;
        };
        match r {
            Report::Info(t) => {
                if a.failure_shown {
                    a.held = Some(t.clone());
                } else {
                    a.dialog.set_status(&t);
                }
                a.notice.get_or_insert(t);
            }
            Report::Error(t) => {
                a.failure_shown = false;
                a.dialog.set_status(&t);
                a.notice.get_or_insert(t);
            }
            Report::Prompt => {
                a.prompted = true;
                a.notice = None;
            }
            Report::Done(Verdict::Success) => {
                a.reports = None;
                self.finish(Outcome::Success);
            }
            Report::Done(Verdict::Failed) => {
                a.attempts += 1;
                let notice = a.notice.take();
                if a.attempts >= ATTEMPTS {
                    self.finish(Outcome::Failed);
                } else {
                    a.dialog.fail(notice.as_deref());
                    a.failure_shown = true;
                    // Straight into the next try, so the reader is listening before anything is typed.
                    a.begin(self.waker.as_ref());
                }
            }
            Report::Done(Verdict::Unreachable) => {
                // `Report::Error` already put the reason in the status line; nothing was
                // judged, so no retry is spent.
                a.session = None;
                a.reports = None;
                a.prompted = false;
                a.dialog.set_busy(false);
            }
        }
    }

    /// Give the helper what the dialog submitted, starting a conversation if none
    /// is running for `user`. A password sent before the helper asks waits for its
    /// prompt, which comes after the earlier PAM steps (the fingerprint reader).
    fn submit(&mut self, user: String, password: zeroize::Zeroizing<String>) {
        let waker = self.waker.clone();
        let Some(a) = self.active.as_mut() else {
            return;
        };
        if a.session.as_ref().is_none_or(|(_, u)| *u != user) {
            a.begin(waker.as_ref());
        }
        a.dialog.set_busy(true);
        // Asked for: the password goes straight in. Not yet: the helper is
        // still on an earlier step, whose text says what it waits for.
        if a.prompted {
            a.dialog.set_status("");
        } else if a.failure_shown {
            a.dialog.set_status(a.held.as_deref().unwrap_or(""));
        }
        a.failure_shown = false;
        a.held = None;
        a.prompted = false;
        let sent = match &a.session {
            Some((session, _)) => session.answer(password),
            None => false,
        };
        if !sent && waker.is_some() {
            // Its thread has already ended; the report of how is on its way.
            a.dialog.set_busy(false);
        }
    }

    fn user_changed(&mut self, user: &str) {
        let waker = self.waker.clone();
        if let Some(a) = self.active.as_mut()
            && a.session.as_ref().is_some_and(|(_, u)| u != user)
        {
            a.begin(waker.as_ref());
        }
    }

    fn wake(&mut self, ctx: &mut Ctx) -> Reply {
        while let Some(m) = self.msgs.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.handle(m);
        }
        while let Some(r) = self
            .active
            .as_ref()
            .and_then(|a| a.reports.as_ref())
            .and_then(|rx| rx.try_recv().ok())
        {
            self.report(r);
        }
        self.open_next_in(ctx);
        Reply::redraw_all()
    }

    fn dialog_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(a) = self.active.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => dialog::Outcome::Cancel,
                Event::Pasted(text) => {
                    let ev = a.dialog.form.paste(text);
                    a.dialog.event(ev)
                }
                ev => match form_input(&mut a.dialog.form, &ev, ctx) {
                    Some(ev) => a.dialog.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        self.dialog_outcome(out)
    }

    fn dialog_outcome(&mut self, out: dialog::Outcome) -> Reply {
        match out {
            dialog::Outcome::None => Reply::none(),
            dialog::Outcome::Redraw => Reply::redraw(),
            dialog::Outcome::Cancel => {
                self.finish(Outcome::Cancelled);
                Reply::redraw_all()
            }
            dialog::Outcome::Submit { user, password } => {
                self.submit(user, password);
                Reply::redraw()
            }
            dialog::Outcome::UserChanged(user) => {
                self.user_changed(&user);
                Reply::redraw()
            }
        }
    }
}

impl App for PolkitApp {
    fn title(&self) -> String {
        "Run As".into()
    }

    fn app_id(&self) -> String {
        "reprise.polkit".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.active
            .as_ref()
            .map_or(dialog::CLIENT, |a| self.units.size(a.dialog.size))
    }

    fn hidden(&self) -> bool {
        true
    }

    fn children(&self) -> Vec<Child> {
        self.active
            .iter()
            .map(|a| Child {
                id: RUNAS,
                title: "Run As".into(),
                size: self.units.size(a.dialog.size),
                fixed: true,
                modal: true,
            })
            .collect()
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        self.waker = Some(waker.clone());
        match agent::start(waker) {
            Some(rx) => {
                self.msgs = Some(rx);
                repriseapp::Start::Continue
            }
            None => {
                log::info!("no polkit agent to be; exiting");
                repriseapp::Start::Exit(0)
            }
        }
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let reply = match (win, ev) {
            (_, Event::Wake) => self.wake(ctx),
            (RUNAS, ev) => {
                let field = self
                    .active
                    .as_ref()
                    .and_then(|a| a.dialog.form.focused_edit());
                let clip = form_clipboard(field, &ev);
                self.dialog_event(ev, ctx).with_clipboard(clip)
            }
            (_, Event::CloseRequest) => Reply::redraw(),
            _ => Reply::none(),
        };
        // A user-driven close (Esc, Cancel) can empty `active` outside
        // `wake`, and nothing else wakes the loop afterwards: open
        // whatever is next in the queue right here.
        if self.active.is_none() && !self.queue.is_empty() {
            self.open_next_in(ctx);
            return Reply::redraw_all();
        }
        reply
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        if win == RUNAS
            && let Some(a) = &self.active
        {
            let px = a.dialog.form.units().px(ctx.theme.fonts.size_px);
            oneshot::access_form(&a.dialog.form, tree, &ctx.text, px);
        }
    }

    fn popup(&mut self, win: WinId, _: &Ctx) -> Option<Box<dyn Hosted + '_>> {
        let a = self.active.as_mut().filter(|_| win == RUNAS)?;
        Some(Box::new(&mut a.dialog.form))
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        c.fill_rect(c.bounds(), forms.colors.button_face);
        if win == RUNAS
            && let Some(a) = &self.active
        {
            form::draw(
                c,
                &ctx.text,
                &Paint {
                    s: 1,
                    ..forms.paint()
                },
                &a.dialog.form,
                true,
                forms.fonts.size_px,
            );
        }
    }
}

fn main() {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    repriseapp::run(Box::new(PolkitApp::default()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(app: &mut PolkitApp) {
        app.open_next(&|m| reprisekit::text::wrap_chars(m, 58), None, None);
    }

    fn request(cookie: &str) -> (Request, async_channel::Receiver<Outcome>) {
        let (done, rx) = async_channel::bounded(1);
        (
            Request {
                cookie: cookie.into(),
                message: "Authentication is required.".into(),
                users: vec!["root".into()],
                selected: 0,
                done,
            },
            rx,
        )
    }

    #[test]
    fn polkitd_withdraws_requests_by_cookie() {
        let mut app = PolkitApp::default();
        let (a, a_rx) = request("a");
        let (b, b_rx) = request("b");
        app.handle(Msg::Begin(a));
        app.handle(Msg::Begin(b));
        open(&mut app);
        assert_eq!(app.active.as_ref().unwrap().request.cookie, "a");
        assert_eq!(app.queue.len(), 1);
        // A queued request cancels without ever opening.
        app.handle(Msg::Cancel("b".into()));
        assert!(app.queue.is_empty());
        assert_eq!(b_rx.try_recv(), Ok(Outcome::Withdrawn));
        // Cancelling the open one closes it.
        app.handle(Msg::Cancel("a".into()));
        assert!(app.active.is_none());
        assert_eq!(a_rx.try_recv(), Ok(Outcome::Withdrawn));
    }

    #[test]
    fn only_the_prompts_own_cancel_is_a_dismissal() {
        let mut app = PolkitApp::default();
        let (a, a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        app.dialog_outcome(dialog::Outcome::Cancel);
        assert!(app.active.is_none());
        assert_eq!(a_rx.try_recv(), Ok(Outcome::Cancelled));
    }

    #[test]
    fn three_failures_fail_the_request() {
        let mut app = PolkitApp::default();
        let (a, a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        for i in 1..ATTEMPTS {
            app.report(Report::Done(Verdict::Failed));
            let act = app.active.as_ref().expect("still open");
            assert_eq!(act.attempts, i);
            assert_eq!(act.dialog.status(), dialog::FAILED);
        }
        app.report(Report::Done(Verdict::Failed));
        assert!(app.active.is_none());
        assert_eq!(a_rx.try_recv(), Ok(Outcome::Failed));
    }

    #[test]
    fn an_unreachable_helper_spends_no_attempt() {
        let mut app = PolkitApp::default();
        let (a, _a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        app.report(Report::Error("Cannot reach the polkit helper: x".into()));
        app.report(Report::Done(Verdict::Unreachable));
        let act = app.active.as_ref().expect("still open");
        assert_eq!(act.attempts, 0);
        assert_eq!(act.dialog.status(), "Cannot reach the polkit helper: x");
    }

    #[test]
    fn a_pam_error_replaces_the_generic_failure_text() {
        let mut app = PolkitApp::default();
        let (a, _a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        app.report(Report::Error("Account locked.".into()));
        app.report(Report::Done(Verdict::Failed));
        let act = app.active.as_ref().expect("still open");
        assert_eq!(act.dialog.status(), "Account locked.");
    }

    #[test]
    fn a_pam_info_replaces_the_generic_failure_text() {
        let mut app = PolkitApp::default();
        let (a, _a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        app.report(Report::Info(
            "The account is locked due to 3 failed logins.".into(),
        ));
        app.report(Report::Info("(10 minutes left to unlock)".into()));
        app.report(Report::Done(Verdict::Failed));
        let act = app.active.as_ref().expect("still open");
        assert_eq!(
            act.dialog.status(),
            "The account is locked due to 3 failed logins."
        );
    }

    /// The reader's text before the password prompt is not why the
    /// password failed.
    #[test]
    fn text_before_the_prompt_is_not_the_failure_notice() {
        let mut app = PolkitApp::default();
        let (a, _a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        app.report(Report::Info(
            "Place your finger on the fingerprint reader".into(),
        ));
        assert_eq!(
            app.active.as_ref().unwrap().dialog.status(),
            "Place your finger on the fingerprint reader"
        );
        app.report(Report::Prompt);
        app.report(Report::Done(Verdict::Failed));
        let act = app.active.as_ref().expect("still open");
        assert_eq!(act.dialog.status(), dialog::FAILED);
    }

    /// After a failure the next conversation starts at once; its reader
    /// prompt does not wipe out why the last try failed, but an error does.
    #[test]
    fn a_shown_failure_outlasts_the_next_info_but_not_an_error() {
        let mut app = PolkitApp::default();
        let (a, _a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        app.report(Report::Prompt);
        app.report(Report::Done(Verdict::Failed));
        app.report(Report::Info(
            "Place your finger on the fingerprint reader".into(),
        ));
        assert_eq!(app.active.as_ref().unwrap().dialog.status(), dialog::FAILED);
        app.report(Report::Error("Failed to match fingerprint".into()));
        assert_eq!(
            app.active.as_ref().unwrap().dialog.status(),
            "Failed to match fingerprint"
        );
    }

    /// A password submitted while the reader still waits shows the
    /// reader's text, not the last failure.
    #[test]
    fn a_submit_during_the_reader_step_shows_what_it_waits_for() {
        let mut app = PolkitApp::default();
        let (a, _a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        app.report(Report::Prompt);
        app.report(Report::Done(Verdict::Failed));
        // The next conversation, now on the reader step.
        let (session, passwords) = helper::Session::detached();
        app.active.as_mut().unwrap().session = Some((session, "root".into()));
        app.report(Report::Info(
            "Place your finger on the fingerprint reader".into(),
        ));
        app.submit("root".into(), zeroize::Zeroizing::new("hunter2".into()));
        let act = app.active.as_ref().unwrap();
        assert_eq!(
            act.dialog.status(),
            "Place your finger on the fingerprint reader"
        );
        // The password waits in the conversation for its prompt.
        assert_eq!(
            passwords.try_recv().ok().as_deref().map(|p| p.as_str()),
            Some("hunter2")
        );
        assert_eq!(act.attempts, 1);
    }

    /// A finger that matches ends the request with no password typed.
    #[test]
    fn a_finger_alone_succeeds() {
        let mut app = PolkitApp::default();
        let (a, a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        app.report(Report::Info(
            "Place your finger on the fingerprint reader".into(),
        ));
        app.report(Report::Done(Verdict::Success));
        assert!(app.active.is_none());
        assert_eq!(a_rx.try_recv(), Ok(Outcome::Success));
    }

    #[test]
    fn success_closes_and_the_next_opens() {
        let mut app = PolkitApp::default();
        let (a, a_rx) = request("a");
        let (b, _b_rx) = request("b");
        app.handle(Msg::Begin(a));
        app.handle(Msg::Begin(b));
        open(&mut app);
        app.report(Report::Info("hello".into()));
        assert_eq!(app.active.as_ref().unwrap().dialog.status(), "hello");
        app.report(Report::Done(Verdict::Success));
        assert_eq!(a_rx.try_recv(), Ok(Outcome::Success));
        open(&mut app);
        assert_eq!(app.active.as_ref().unwrap().request.cookie, "b");
    }

    #[test]
    fn the_dialog_is_the_only_window() {
        let mut app = PolkitApp::default();
        assert!(app.hidden());
        assert!(app.children().is_empty());
        let (a, _rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        let kids = app.children();
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].id, RUNAS);
        assert!(kids[0].modal && kids[0].fixed);
        assert_eq!(kids[0].size, dialog::CLIENT);
        assert_eq!(kids[0].title, "Run As");
    }
}
