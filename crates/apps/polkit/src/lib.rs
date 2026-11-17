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

#[cfg(test)]
reprisebus::test_session!();

mod agent;
mod dialog;
mod first;
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
use first::FirstPrompts;
use helper::{Prompt, Report, Tag, Verdict};

const RUNAS: WinId = WinId::named("runas");
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
    /// The prompts this conversation has reported.
    prompts: u32,
    /// What was submitted ahead of its prompt: the helper gives it to that prompt only if PAM
    /// asks exactly this.
    sent: Option<Tag>,
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
            prompts: 0,
            sent: None,
            notice: None,
            failure_shown: false,
            held: None,
        }
    }

    /// Hang up on any conversation in flight and start one for the user the dialog
    /// shows, the field on the first prompt `first` remembers for them. Without a waker (the
    /// tests) nothing starts.
    fn begin(&mut self, waker: Option<&repriseapp::Waker>, first: &FirstPrompts) {
        self.session = None;
        self.reports = None;
        self.prompted = false;
        self.prompts = 0;
        self.sent = None;
        let user = self.dialog.user();
        self.dialog.open(&first.get(&user));
        self.notice = None;
        self.held = None;
        let Some(waker) = waker else {
            return;
        };
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
    first: FirstPrompts,
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
            Msg::CancelAll => {
                self.finish(Outcome::Withdrawn);
                for r in self.queue.drain(..) {
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
        a.begin(self.waker.as_ref(), &self.first);
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
            Report::Prompt { text, echo } => {
                a.prompts += 1;
                let tag = Tag {
                    n: a.prompts,
                    prompt: Prompt { text, echo },
                };
                if tag.n == 1
                    && let Some((_, user)) = &a.session
                {
                    self.first.learn(user, &tag.prompt);
                }
                // Text sent under another prompt was dropped: ask again.
                let dropped = a.sent.take().is_some_and(|s| s != tag);
                if dropped || *a.dialog.tag() != tag {
                    a.dialog.ask(tag);
                }
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
                    a.begin(self.waker.as_ref(), &self.first);
                }
            }
            Report::Done(Verdict::Unreachable) => {
                // `Report::Error` already put the reason in the status line; nothing was
                // judged, so no retry is spent.
                a.session = None;
                a.reports = None;
                a.prompted = false;
                a.sent = None;
                a.dialog.set_busy(false);
            }
        }
    }

    /// Give the helper what the dialog submitted under `tag`, starting a conversation if none
    /// is running for `user`. A password sent before the helper asks waits for its
    /// prompt, which comes after the earlier PAM steps (the fingerprint reader), and
    /// answers it only if that prompt is `tag`.
    fn submit(&mut self, user: String, tag: Tag, password: zeroize::Zeroizing<String>) {
        let waker = self.waker.clone();
        let Some(a) = self.active.as_mut() else {
            return;
        };
        if a.session.as_ref().is_none_or(|(_, u)| *u != user) {
            a.begin(waker.as_ref(), &self.first);
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
        if tag.n > a.prompts {
            a.sent = Some(tag.clone());
        }
        let sent = match &a.session {
            Some((session, _)) => session.answer(tag, password),
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
            a.begin(waker.as_ref(), &self.first);
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
                if let Event::Pasted(text) = ev {
                    drop(zeroize::Zeroizing::new(text));
                }
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
            dialog::Outcome::Submit {
                user,
                tag,
                password,
            } => {
                self.submit(user, tag, password);
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
        self.first = FirstPrompts::load(first::path());
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

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        c.fill_rect(c.bounds(), forms.colors.button_face);
        if win == RUNAS
            && let Some(a) = &mut self.active
        {
            a.dialog.set_caps_lock(repriseapp::caps_lock());
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

impl repriseapp::Extended for PolkitApp {
    fn hidden(&self) -> bool {
        true
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        if win == RUNAS
            && let Some(a) = &self.active
        {
            let px = a.dialog.form.units().px(ctx.base_px());
            oneshot::access_form(&a.dialog.form, tree, &ctx.text, px);
        }
    }

    fn popup(&mut self, win: WinId, _: &Ctx) -> Option<Box<dyn Hosted + '_>> {
        let a = self.active.as_mut().filter(|_| win == RUNAS)?;
        Some(Box::new(&mut a.dialog.form))
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(_args: Vec<std::ffi::OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    repriseapp::run(Box::new(PolkitApp::default())).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriseapp::Extended;

    fn open(app: &mut PolkitApp) {
        app.open_next(&|m| reprisekit::text::wrap_chars(m, 58), None, None);
    }

    /// Type `text` into the field and press OK, as the user does: nothing goes out while OK
    /// is greyed.
    fn type_ok(app: &mut PolkitApp, text: &str) {
        let a = app.active.as_mut().expect("a box is open");
        a.dialog.form.focus_id("password");
        let ev = a.dialog.form.paste(text.into());
        a.dialog.event(ev);
        let out = a.dialog.event(reprisekit::controls::form::Event::Default);
        assert!(
            matches!(out, dialog::Outcome::Submit { .. }),
            "OK submits {text:?}, got {out:?}"
        );
        app.dialog_outcome(out);
    }

    /// The tag OK carries for `text` typed into the field; the submit goes nowhere.
    fn ok_tag(app: &mut PolkitApp, text: &str) -> Tag {
        let d = &mut app.active.as_mut().expect("a box is open").dialog;
        let ev = d.form.paste(text.into());
        d.event(ev);
        match d.event(reprisekit::controls::form::Event::Default) {
            dialog::Outcome::Submit { tag, .. } => tag,
            other => panic!("OK submits {text:?}, got {other:?}"),
        }
    }

    fn label(app: &PolkitApp) -> String {
        app.active.as_ref().unwrap().dialog.label().to_string()
    }

    /// Answers that reached the conversation: the tag each was typed under, and the text.
    fn sent(answers: &std::sync::mpsc::Receiver<helper::Answer>) -> Vec<(u32, String, String)> {
        answers
            .try_iter()
            .map(|a| (a.tag.n, a.tag.prompt.text, a.text.to_string()))
            .collect()
    }

    /// A detached conversation for `user` in place of the helper's.
    fn attach(app: &mut PolkitApp, user: &str) -> std::sync::mpsc::Receiver<helper::Answer> {
        let (session, answers) = helper::Session::detached();
        app.active.as_mut().unwrap().session = Some((session, user.into()));
        answers
    }

    fn secret(app: &PolkitApp) -> bool {
        let form = &app.active.as_ref().unwrap().dialog.form;
        match form.item("password").map(|i| &i.control) {
            Some(form::Control::Edit { edit, .. }) => edit.is_secret(),
            other => panic!("the field is {other:?}"),
        }
    }

    fn code(echo: bool) -> Prompt {
        Prompt {
            text: "Verification code:".into(),
            echo,
        }
    }

    fn prompt(p: &Prompt) -> Report {
        Report::Prompt {
            text: p.text.clone(),
            echo: p.echo,
        }
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
    fn polkitd_leaving_withdraws_the_open_and_the_queued_requests() {
        let mut app = PolkitApp::default();
        let (a, a_rx) = request("a");
        let (b, b_rx) = request("b");
        app.handle(Msg::Begin(a));
        app.handle(Msg::Begin(b));
        open(&mut app);
        app.handle(Msg::CancelAll);
        assert!(app.active.is_none(), "the open box closes");
        assert!(app.queue.is_empty());
        assert_eq!(a_rx.try_recv(), Ok(Outcome::Withdrawn));
        assert_eq!(b_rx.try_recv(), Ok(Outcome::Withdrawn));
        open(&mut app);
        assert!(app.active.is_none(), "nothing opens after");
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
        app.report(Report::Prompt {
            text: "Password:".into(),
            echo: false,
        });
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
        app.report(Report::Prompt {
            text: "Password:".into(),
            echo: false,
        });
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
        app.report(Report::Prompt {
            text: "Password:".into(),
            echo: false,
        });
        app.report(Report::Done(Verdict::Failed));
        // The next conversation, now on the reader step.
        let (session, passwords) = helper::Session::detached();
        app.active.as_mut().unwrap().session = Some((session, "root".into()));
        app.report(Report::Info(
            "Place your finger on the fingerprint reader".into(),
        ));
        type_ok(&mut app, "hunter2");
        let act = app.active.as_ref().unwrap();
        assert_eq!(
            act.dialog.status(),
            "Place your finger on the fingerprint reader"
        );
        // The password waits in the conversation for its prompt.
        assert_eq!(
            passwords
                .try_recv()
                .ok()
                .map(|a| a.text.to_string())
                .as_deref(),
            Some("hunter2")
        );
        assert_eq!(act.attempts, 1);
    }

    /// A second hidden prompt relabels the field with its text and opens it for a fresh answer,
    /// which goes to the same conversation; the next conversation asks for the password again.
    #[test]
    fn a_second_prompt_asks_with_its_own_text() {
        let mut app = PolkitApp::default();
        let (a, _a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        let (session, answers) = helper::Session::detached();
        app.active.as_mut().unwrap().session = Some((session, "root".into()));
        app.report(Report::Prompt {
            text: "Password:".into(),
            echo: false,
        });
        assert_eq!(label(&app), "&Password:");
        type_ok(&mut app, "hunter2");
        app.report(Report::Prompt {
            text: "Verification code:".into(),
            echo: false,
        });
        assert_eq!(label(&app), "Verification code:");
        assert_eq!(app.active.as_ref().unwrap().dialog.password(), "");
        type_ok(&mut app, "123456");
        let got: Vec<String> = answers.try_iter().map(|a| a.text.to_string()).collect();
        assert_eq!(got, ["hunter2", "123456"]);
        app.report(Report::Done(Verdict::Failed));
        assert_eq!(label(&app), "&Password:", "a retry asks for the password");
    }

    /// A shown prompt (a one-time code) opens the field as plain text; asked first, it is where
    /// the retry opens, in a hidden field.
    #[test]
    fn a_shown_prompt_shows_its_answer_as_typed() {
        let mut app = PolkitApp::default();
        let (a, _a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        attach(&mut app, "root");
        app.report(Report::Prompt {
            text: "Passcode:".into(),
            echo: true,
        });
        assert_eq!(
            app.active.as_ref().unwrap().dialog.label(),
            "Passcode:",
            "a first shown prompt relabels the field"
        );
        assert!(!secret(&app));
        app.report(Report::Done(Verdict::Failed));
        assert!(
            secret(&app),
            "the retry opens hidden on the shown first prompt"
        );
        assert_eq!(app.active.as_ref().unwrap().dialog.label(), "Passcode:");
    }

    /// Each submit names the prompt the box showed when it was typed: its own Password field
    /// as the first, then each prompt PAM asked.
    #[test]
    fn an_answer_names_the_prompt_it_was_typed_under() {
        let mut app = PolkitApp::default();
        let (a, _a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        let (session, answers) = helper::Session::detached();
        app.active.as_mut().unwrap().session = Some((session, "root".into()));
        type_ok(&mut app, "hunter2");
        app.report(Report::Prompt {
            text: "Verification code:".into(),
            echo: true,
        });
        type_ok(&mut app, "424242");
        app.report(Report::Prompt {
            text: "Password:".into(),
            echo: false,
        });
        type_ok(&mut app, "hunter2");
        assert_eq!(
            sent(&answers),
            [
                (1, "Password:".into(), "hunter2".into()),
                (1, "Verification code:".into(), "424242".into()),
                (2, "Password:".into(), "hunter2".into())
            ]
        );
    }

    /// Nothing remembered and a hidden code asked first: the password typed in the own field
    /// is taken back, the box asks for the code with OK usable again, the code goes out under
    /// it, and the code prompt is remembered, so the retry opens on it.
    #[test]
    fn a_hidden_first_prompt_of_another_text_is_asked_and_remembered() {
        let mut app = PolkitApp::default();
        let (a, _a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        let (session, answers) = helper::Session::detached();
        app.active.as_mut().unwrap().session = Some((session, "root".into()));
        assert_eq!(label(&app), "&Password:");
        type_ok(&mut app, "hunter2");
        app.report(prompt(&code(false)));
        assert_eq!(label(&app), "Verification code:");
        let act = app.active.as_ref().unwrap();
        assert_eq!(act.dialog.password(), "");
        assert_eq!(
            act.dialog.form.focus(),
            act.dialog.form.index_of("password")
        );
        type_ok(&mut app, "424242");
        app.report(prompt(&Prompt::password()));
        assert_eq!(label(&app), "Password:");
        type_ok(&mut app, "hunter2");
        assert_eq!(
            sent(&answers),
            [
                (1, "Password:".into(), "hunter2".into()),
                (1, "Verification code:".into(), "424242".into()),
                (2, "Password:".into(), "hunter2".into())
            ]
        );
        assert_eq!(app.first.get("root"), code(false));
        app.report(Report::Done(Verdict::Failed));
        assert_eq!(
            label(&app),
            "Verification code:",
            "the retry opens on the code"
        );
        assert_eq!(
            *app.active.as_ref().unwrap().dialog.tag(),
            Tag {
                n: 1,
                prompt: code(false)
            }
        );
    }

    /// With a shown code prompt remembered, the box opens on it in a hidden field and what is
    /// typed there is sent under the remembered prompt; PAM asking it keeps the box busy on it.
    #[test]
    fn a_remembered_first_prompt_opens_the_box() {
        let mut app = PolkitApp::default();
        app.first.learn("root", &code(true));
        let (a, _a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        let answers = attach(&mut app, "root");
        assert_eq!(label(&app), "Verification code:");
        assert!(secret(&app), "nothing asked yet: the field is hidden");
        type_ok(&mut app, "424242");
        app.report(prompt(&code(true)));
        assert!(
            app.active.as_ref().unwrap().dialog.busy(),
            "the code went to its prompt: still busy"
        );
        let got: Vec<(Tag, String)> = answers
            .try_iter()
            .map(|a| (a.tag, a.text.to_string()))
            .collect();
        assert_eq!(
            got,
            [(
                Tag {
                    n: 1,
                    prompt: code(true)
                },
                "424242".into()
            )]
        );
    }

    /// A state file naming "Password:" as a shown first prompt opens the field hidden; PAM's
    /// hidden "Password: " empties it, keeps it hidden and forgets the entry, on disk too.
    #[test]
    fn a_remembered_shown_password_never_opens_a_plain_field() {
        let dir = std::env::temp_dir().join(format!("reprise-polkit-shown-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("polkit.toml");
        std::fs::write(&path, "[root]\ntext = \"Password:\"\necho = true\n").unwrap();
        let mut app = PolkitApp {
            first: FirstPrompts::load(path.clone()),
            ..PolkitApp::default()
        };
        let (a, _a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        attach(&mut app, "root");
        assert!(secret(&app), "the box opens hidden");
        assert_eq!(label(&app), "Password:");
        let d = &mut app.active.as_mut().unwrap().dialog;
        let ev = d.form.paste("hunter2".into());
        d.event(ev);
        app.report(prompt(&Prompt::password()));
        assert!(secret(&app), "PAM's hidden prompt keeps the field hidden");
        assert_eq!(app.active.as_ref().unwrap().dialog.password(), "");
        assert_eq!(app.first.get("root"), Prompt::password());
        app.first.wait();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Text typed in the opening field under a remembered shown prompt stays hidden whatever PAM
    /// asks first: kept as dots under the same prompt, emptied before the field turns plain for
    /// another.
    #[test]
    fn nothing_typed_hidden_is_shown_plain() {
        let passcode = Prompt {
            text: "Passcode:".into(),
            echo: true,
        };
        for (asked, kept) in [
            (code(true), true),
            (passcode, false),
            (Prompt::password(), false),
        ] {
            let mut app = PolkitApp::default();
            app.first.learn("root", &code(true));
            let (a, _a_rx) = request("a");
            app.handle(Msg::Begin(a));
            open(&mut app);
            attach(&mut app, "root");
            let d = &mut app.active.as_mut().unwrap().dialog;
            let ev = d.form.paste("4242".into());
            d.event(ev);
            assert!(secret(&app), "{asked:?}: typed hidden");
            app.report(prompt(&asked));
            let typed = app.active.as_ref().unwrap().dialog.password().to_string();
            assert_eq!(typed, if kept { "4242" } else { "" }, "{asked:?}");
            assert!(
                secret(&app) || typed.is_empty(),
                "{asked:?}: a plain field holds nothing typed hidden"
            );
            assert_eq!(secret(&app), kept || !asked.echo, "{asked:?}");
        }
    }

    /// A remembered prompt the stack no longer asks: the text is taken back, PAM's prompt is
    /// asked, and the next box opens on the own Password field as before.
    #[test]
    fn a_stale_remembered_prompt_asks_pams_and_is_forgotten() {
        let mut app = PolkitApp::default();
        app.first.learn("root", &code(false));
        let (a, _a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        let (session, _answers) = helper::Session::detached();
        app.active.as_mut().unwrap().session = Some((session, "root".into()));
        type_ok(&mut app, "hunter2");
        app.report(prompt(&Prompt::password()));
        assert_eq!(label(&app), "Password:");
        assert_eq!(app.active.as_ref().unwrap().dialog.password(), "");
        type_ok(&mut app, "hunter2");
        assert_eq!(app.first.get("root"), Prompt::password());
        app.report(Report::Done(Verdict::Failed));
        assert_eq!(label(&app), "&Password:");
    }

    /// Each user's box opens on the first prompt remembered for them, and OK there submits
    /// under it.
    #[test]
    fn two_users_open_on_their_own_first_prompts() {
        let mut app = PolkitApp::default();
        app.first.learn("alice", &code(false));
        let (mut a, _a_rx) = request("a");
        a.users = vec!["root".into(), "alice".into()];
        app.handle(Msg::Begin(a));
        open(&mut app);
        attach(&mut app, "root");
        assert_eq!(label(&app), "&Password:");
        let password = Tag {
            n: 1,
            prompt: Prompt::password(),
        };
        assert_eq!(ok_tag(&mut app, "hunter2"), password);
        let d = &mut app.active.as_mut().unwrap().dialog;
        d.form.set_combo("user", 1);
        let out = d.event(reprisekit::controls::form::Event::Changed("user"));
        app.dialog_outcome(out);
        assert_eq!(label(&app), "Verification code:");
        let alices = Tag {
            n: 1,
            prompt: code(false),
        };
        assert_eq!(ok_tag(&mut app, "424242"), alices);
    }

    /// PAM's "Password: " on a plain stack is the own field: nothing changes on screen and the
    /// password typed there is its answer.
    #[test]
    fn a_plain_stack_answers_the_own_field() {
        let mut app = PolkitApp::default();
        let (a, _a_rx) = request("a");
        app.handle(Msg::Begin(a));
        open(&mut app);
        let (session, answers) = helper::Session::detached();
        app.active.as_mut().unwrap().session = Some((session, "root".into()));
        type_ok(&mut app, "hunter2");
        app.report(prompt(&Prompt::password()));
        assert_eq!(label(&app), "&Password:");
        assert!(app.active.as_ref().unwrap().dialog.busy());
        assert_eq!(sent(&answers), [(1, "Password:".into(), "hunter2".into())]);
        assert!(app.first.get("root") == Prompt::password());
    }

    /// A first prompt that comes before anything is sent (the reader's stack starts at once)
    /// leaves what is being typed under a label that matches it, and relabels and empties a
    /// field that does not.
    #[test]
    fn an_early_first_prompt_keeps_typing_only_under_its_own_label() {
        for (asked, kept) in [(Prompt::password(), true), (code(false), false)] {
            let mut app = PolkitApp::default();
            let (a, _a_rx) = request("a");
            app.handle(Msg::Begin(a));
            open(&mut app);
            let (session, _answers) = helper::Session::detached();
            app.active.as_mut().unwrap().session = Some((session, "root".into()));
            let d = &mut app.active.as_mut().unwrap().dialog;
            let ev = d.form.paste("hun".into());
            d.event(ev);
            app.report(prompt(&asked));
            let d = &app.active.as_ref().unwrap().dialog;
            assert_eq!(d.password(), if kept { "hun" } else { "" }, "{asked:?}");
            assert_eq!(d.tag().prompt, asked);
        }
    }

    fn two_users() -> PolkitApp {
        let mut app = PolkitApp::default();
        let (mut a, _a_rx) = request("a");
        a.users = vec!["root".into(), "alice".into()];
        app.handle(Msg::Begin(a));
        open(&mut app);
        let (session, _answers) = helper::Session::detached();
        app.active.as_mut().unwrap().session = Some((session, "root".into()));
        app
    }

    /// The tags the answers that reached the conversation were typed under.
    fn tags(answers: &std::sync::mpsc::Receiver<helper::Answer>) -> Vec<Tag> {
        answers.try_iter().map(|a| a.tag).collect()
    }

    fn pick(app: &mut PolkitApp, i: usize) {
        let d = &mut app.active.as_mut().unwrap().dialog;
        d.form.set_combo("user", i);
        let out = d.event(reprisekit::controls::form::Event::Changed("user"));
        app.dialog_outcome(out);
    }

    /// Another user picked while the helper works starts their conversation on their first
    /// prompt with OK usable.
    #[test]
    fn a_user_picked_while_busy_can_submit() {
        let mut app = two_users();
        app.first.learn("alice", &code(false));
        type_ok(&mut app, "hunter2");
        assert!(app.active.as_ref().unwrap().dialog.busy());
        pick(&mut app, 1);
        assert_eq!(label(&app), "Verification code:");
        let alices = attach(&mut app, "alice");
        type_ok(&mut app, "424242");
        assert_eq!(
            tags(&alices),
            [Tag {
                n: 1,
                prompt: code(false)
            }]
        );
    }

    /// Text typed under a later prompt does not survive a user change into their conversation.
    #[test]
    fn a_user_change_empties_the_field() {
        let mut app = two_users();
        type_ok(&mut app, "hunter2");
        app.report(prompt(&Prompt::password()));
        app.report(prompt(&code(false)));
        let d = &mut app.active.as_mut().unwrap().dialog;
        let ev = d.form.paste("1234".into());
        d.event(ev);
        pick(&mut app, 1);
        assert_eq!(app.active.as_ref().unwrap().dialog.password(), "");
    }

    /// `key` given to the box as the host gives it, its outcome acted on.
    fn press(app: &mut PolkitApp, key: form::Key) {
        let d = &mut app.active.as_mut().unwrap().dialog;
        let ev = d.form.key(key);
        let out = d.event(ev);
        app.dialog_outcome(out);
    }

    fn click_user(app: &mut PolkitApp, part: reprisekit::controls::combo::Hit) {
        let d = &mut app.active.as_mut().unwrap().dialog;
        let hit = form::Hit {
            item: d.form.index_of("user").unwrap(),
            part: form::Part::Combo(part),
            x: 0,
        };
        let ev = d.form.click(Some(hit));
        let out = d.event(ev);
        app.dialog_outcome(out);
    }

    fn type_keys(app: &mut PolkitApp, text: &str) {
        for ch in text.chars() {
            press(
                app,
                form::Key::Edit(reprisekit::textedit::EditKey::Insert(ch.into())),
            );
        }
    }

    fn focus_on(app: &PolkitApp, id: &str) -> bool {
        let f = &app.active.as_ref().unwrap().dialog.form;
        f.focus().is_some() && f.focus() == f.index_of(id)
    }

    /// A user picked from the open list, by mouse or keyboard, the shown one too, takes the
    /// password typed straight after it, all of it.
    #[test]
    fn a_password_typed_right_after_a_pick_from_the_list_is_all_in_the_field() {
        use reprisekit::controls::combo::Hit;
        type Pick = fn(&mut PolkitApp);
        let picks: [(&str, Pick, &str); 3] = [
            (
                "click",
                |app| {
                    click_user(app, Hit::Field);
                    click_user(app, Hit::Row(1));
                },
                "alice",
            ),
            (
                "keys",
                |app| {
                    press(app, form::Key::Tab { back: true });
                    press(app, form::Key::Drop);
                    press(app, form::Key::Down);
                    press(app, form::Key::Enter);
                },
                "alice",
            ),
            (
                "same user",
                |app| {
                    click_user(app, Hit::Field);
                    click_user(app, Hit::Row(0));
                },
                "root",
            ),
        ];
        for (how, pick, user) in picks {
            let mut app = two_users();
            pick(&mut app);
            let answers = attach(&mut app, user);
            type_keys(&mut app, "in2pass");
            let d = &app.active.as_ref().unwrap().dialog;
            assert_eq!(d.password(), "in2pass", "{how}");
            assert_eq!(d.user(), user, "{how}");
            press(&mut app, form::Key::Enter);
            assert_eq!(
                sent(&answers),
                [(1, "Password:".into(), "in2pass".into())],
                "{how}"
            );
        }
    }

    /// Arrows on the closed list and Escape on the open one keep the keyboard on the list.
    #[test]
    fn stepping_through_users_keeps_the_focus_on_the_list() {
        let mut app = two_users();
        press(&mut app, form::Key::Tab { back: true });
        press(&mut app, form::Key::Down);
        assert_eq!(app.active.as_ref().unwrap().dialog.user(), "alice");
        assert!(focus_on(&app, "user"), "after Down");
        press(&mut app, form::Key::Drop);
        press(&mut app, form::Key::Up);
        press(&mut app, form::Key::Escape);
        assert_eq!(app.active.as_ref().unwrap().dialog.user(), "alice");
        assert!(focus_on(&app, "user"), "after Escape");
        press(&mut app, form::Key::Tab { back: false });
        assert!(focus_on(&app, "password"), "after Tab");
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
