//! Printing for a program: File > Page Setup... and Print... in modal child windows, and the job
//! sent to CUPS as a PDF. A program hands its document over whole ([`Doc`]); listing the
//! printers, cutting the document into pages, drawing them at [`DPI`] and sending them all run
//! on threads, so the event loop never waits on cupsd or on drawing. A program routes the
//! windows [`Printing::owns`] here and calls [`Printing::wake`] on `Event::Wake`.

pub mod cups;
mod doc;
pub mod http;
pub mod ipp;
pub mod pdf;

pub use doc::{Doc, TextFont};

use crate::keys::form_input;
use crate::oneshot::{self, ErrorBox};
use crate::{Child, Ctx, DropForm, Event, Poster, Reply, WinId};
use doc::Laid;
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::form::{self, Form};
use reprisekit::controls::pagesetup::{self, PageSetup};
use reprisekit::controls::printdlg::{self, NoDest, Range};
use reprisekit::kit_tr;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use reprisekit::tr;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const PAGE_SETUP: WinId = WinId::named("pagesetup");
pub const PRINT: WinId = WinId::named("print");

/// Device pixels per inch the pages are drawn at.
pub const DPI: i32 = 300;

/// How long a program that quits waits for jobs still being sent: room to draw a
/// [`printdlg::MAX_PAGES`] job, whose every socket step has its own timeout.
const QUIT_WAIT: Duration = Duration::from_secs(600);

/// The paper the user's locale prints on (LC_ALL, then LC_PAPER, then LANG).
pub fn locale_setup() -> PageSetup {
    let locale = ["LC_ALL", "LC_PAPER", "LANG"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()))
        .unwrap_or_default();
    PageSetup::on(pagesetup::paper_for_locale(&locale))
}

enum Open {
    Setup(Form),
    Print {
        form: Form,
        /// Which listing the dialog waits for, so a late one from an earlier opening is dropped.
        ask: u64,
        listed: Option<Listed>,
    },
}

/// What the listing thread found: the printers and the document cut into pages.
struct Listed {
    queues: Result<cups::Queues, cups::Error>,
    laid: Laid,
}

enum Msg {
    Listed(u64, Box<Listed>),
    Sent(Result<i32, cups::Error>),
}

/// What a print dialog asks the program to do.
pub enum Step {
    Reply(Box<Reply>),
    /// Page Setup closed on OK with [`Printing::setup`] changed; a program that keeps it saves it.
    SetupChanged,
}

impl Step {
    fn reply(r: Reply) -> Step {
        Step::Reply(Box::new(r))
    }
}

pub struct Printing {
    pub setup: PageSetup,
    open: Option<Open>,
    errors: ErrorBox,
    /// The program's name, the title of the error boxes.
    title: String,
    /// The title of the document the open Print dialog prints.
    job_title: String,
    warn_icon: Option<Canvas>,
    msgs: Option<(Poster<Msg>, Receiver<Msg>)>,
    asks: u64,
    /// Jobs being drawn or sent.
    sending: Vec<JoinHandle<()>>,
    /// The forms theme and font of the last event or draw, for [`Printing::children`].
    forms: Theme,
    text: Rc<TextRenderer>,
}

impl Printing {
    /// Printing for the program called `title`, on the paper of the user's locale.
    pub fn new(title: &str) -> Printing {
        Printing {
            setup: locale_setup(),
            open: None,
            errors: ErrorBox::new(WinId::named("printerror")),
            title: title.into(),
            job_title: String::new(),
            warn_icon: None,
            msgs: None,
            asks: 0,
            sending: Vec::new(),
            forms: crate::start_forms(),
            text: crate::start_text(),
        }
    }

    /// Open Page Setup on [`Printing::setup`].
    pub fn page_setup(&mut self, ctx: &Ctx) {
        self.follow_forms(ctx);
        self.open = Some(Open::Setup(pagesetup::form(&self.setup)));
    }

    /// Open Print for `doc`, titled `title` in the print queue. The dialog looks for printers
    /// while a thread lists them and cuts `doc` into pages.
    pub fn print(&mut self, doc: Doc, title: &str, ctx: &Ctx) {
        self.follow_forms(ctx);
        self.asks += 1;
        let ask = self.asks;
        self.job_title = title.into();
        self.open = Some(Open::Print {
            form: printdlg::form(Err(NoDest::Looking), 0, 1),
            ask,
            listed: None,
        });
        let poster = self.poster(ctx);
        let setup = self.setup.clone();
        std::thread::spawn(move || {
            let queues = cups::queues(&http::Server::find(), &login_name());
            let laid = Laid::new(doc, setup);
            poster.post(Msg::Listed(ask, Box::new(Listed { queues, laid })));
        });
    }

    fn poster(&mut self, ctx: &Ctx) -> Poster<Msg> {
        let (poster, _) = self
            .msgs
            .get_or_insert_with(|| Poster::channel(ctx.wake.clone()));
        poster.clone()
    }

    /// Whether `win` is a print dialog's or one of its error boxes'.
    pub fn owns(&self, win: WinId) -> bool {
        self.errors.owns(win) || self.dialog(win).is_some()
    }

    /// The window `win` covers: a box covers the dialog it was raised over, a dialog the main
    /// window.
    pub fn parent(&self, win: WinId) -> Option<WinId> {
        match self.errors.parent(win) {
            Some(p) if self.dialog(p).is_some() => Some(p),
            _ => Some(crate::MAIN),
        }
    }

    fn dialog(&self, win: WinId) -> Option<&Form> {
        match (&self.open, win) {
            (Some(Open::Setup(f)), PAGE_SETUP) => Some(f),
            (Some(Open::Print { form, .. }), PRINT) => Some(form),
            _ => None,
        }
    }

    fn dialog_mut(&mut self, win: WinId) -> Option<&mut Form> {
        match (&mut self.open, win) {
            (Some(Open::Setup(f)), PAGE_SETUP) => Some(f),
            (Some(Open::Print { form, .. }), PRINT) => Some(form),
            _ => None,
        }
    }

    pub fn children(&self) -> Vec<Child> {
        let units = self.forms.text;
        let mut kids: Vec<Child> = match &self.open {
            Some(Open::Setup(_)) => vec![Child {
                id: PAGE_SETUP,
                title: kit_tr!("pagesetup-title"),
                size: units.size(pagesetup::CLIENT),
                fixed: true,
                modal: true,
            }],
            Some(Open::Print { .. }) => vec![Child {
                id: PRINT,
                title: kit_tr!("printdlg-title"),
                size: units.size(printdlg::CLIENT),
                fixed: true,
                modal: true,
            }],
            None => Vec::new(),
        };
        kids.extend(self.errors.child(&self.text, &self.forms));
        kids
    }

    fn follow_forms(&mut self, ctx: &Ctx) {
        self.forms = ctx.forms();
        self.text = ctx.text.clone();
    }

    fn follow(&mut self, ctx: &mut Ctx) {
        self.follow_forms(ctx);
        if self.warn_icon.is_none() {
            self.warn_icon = ctx.icons.get("dialog-warning", 32);
        }
    }

    fn close(&mut self) {
        self.open = None;
        self.errors.withdraw(|w| w == PAGE_SETUP || w == PRINT);
    }

    fn refuse(&mut self, over: WinId, text: String) {
        let icon = self.warn_icon.clone();
        self.errors.show_over(over, &self.title, text, icon);
    }

    pub fn event(&mut self, win: WinId, ev: &Event, ctx: &mut Ctx) -> Step {
        self.follow(ctx);
        if self.errors.owns(win) {
            return Step::reply(self.errors.event(win, ev, &self.text, &self.forms));
        }
        if matches!(ev, Event::CloseRequest) {
            self.close();
            return Step::reply(Reply::redraw_all());
        }
        let Some(f) = self.dialog_mut(win) else {
            return Step::reply(Reply::none());
        };
        let Some(out) = form_input(f, ev, ctx) else {
            return Step::reply(Reply::none());
        };
        match (win, out) {
            (_, form::Event::Cancel | form::Event::Pressed(pagesetup::CANCEL)) => {
                self.close();
                Step::reply(Reply::redraw_all())
            }
            (PAGE_SETUP, form::Event::Default | form::Event::Pressed(pagesetup::OK)) => {
                self.setup_ok()
            }
            (PRINT, form::Event::Default | form::Event::Pressed(printdlg::PRINT)) => {
                self.print_ok(ctx)
            }
            (PRINT, form::Event::Changed(id)) => {
                if let Some(Open::Print {
                    form,
                    listed: Some(l),
                    ..
                }) = &mut self.open
                    && let Ok(q) = &l.queues
                {
                    printdlg::changed(form, &q.list, id);
                }
                Step::reply(Reply::redraw())
            }
            (_, form::Event::None) => Step::reply(Reply::none()),
            _ => Step::reply(Reply::redraw()),
        }
    }

    fn setup_ok(&mut self) -> Step {
        let Some(Open::Setup(f)) = &mut self.open else {
            return Step::reply(Reply::none());
        };
        match pagesetup::read(f) {
            Ok(s) => {
                self.setup = s;
                self.close();
                Step::SetupChanged
            }
            Err(r) => {
                pagesetup::refused(f, r);
                self.refuse(PAGE_SETUP, r.text());
                Step::reply(Reply::redraw_all())
            }
        }
    }

    fn print_ok(&mut self, ctx: &Ctx) -> Step {
        let Some(Open::Print {
            form,
            listed: Some(listed),
            ..
        }) = &mut self.open
        else {
            return Step::reply(Reply::none());
        };
        let Ok(q) = &listed.queues else {
            return Step::reply(Reply::none());
        };
        let choice = match printdlg::read(form, listed.laid.pages()) {
            Ok(c) => c,
            Err(r) => {
                printdlg::refused(form);
                self.refuse(PRINT, r.text());
                return Step::reply(Reply::redraw_all());
            }
        };
        let Some(dest) = q.list.get(choice.dest) else {
            return Step::reply(Reply::none());
        };
        let queue = dest.name.clone();
        let Some(Open::Print {
            listed: Some(listed),
            ..
        }) = self.open.take()
        else {
            return Step::reply(Reply::none());
        };
        self.close();
        let poster = self.poster(ctx);
        let (title, copies) = (self.job_title.clone(), choice.copies);
        self.sending.push(std::thread::spawn(move || {
            let sent = send(&listed.laid, choice.range, &queue, &title, copies);
            match &sent {
                Ok(id) => log::info!("print: job {id} on {queue}"),
                Err(e) => log::warn!("print: {queue} refused the job: {e:?}"),
            }
            poster.post(Msg::Sent(sent));
        }));
        Step::reply(Reply::redraw_all())
    }

    /// On `Event::Wake`: fills an open Print dialog with the printers listed, and shows why a
    /// job sent was refused. Whether anything changed on screen; a program goes on with its
    /// own wake handling either way.
    pub fn wake(&mut self) -> bool {
        self.sending.retain(|h| !h.is_finished());
        let Some((_, rx)) = &self.msgs else {
            return false;
        };
        let msgs: Vec<Msg> = rx.try_iter().collect();
        let mut changed = false;
        for m in msgs {
            changed |= match m {
                Msg::Listed(ask, l) => self.listed(ask, *l),
                Msg::Sent(Ok(_)) => false,
                Msg::Sent(Err(e)) => {
                    let icon = self.warn_icon.clone();
                    let text = tr!("print-failed", why = e.text());
                    self.errors.show(&self.title, text, icon);
                    true
                }
            };
        }
        changed
    }

    fn listed(&mut self, ask: u64, l: Listed) -> bool {
        let Some(Open::Print {
            form,
            ask: waiting,
            listed,
        }) = &mut self.open
        else {
            return false;
        };
        if *waiting != ask {
            return false;
        }
        let pages = l.laid.pages();
        *form = match &l.queues {
            Ok(q) if q.list.is_empty() => printdlg::form(Err(NoDest::NoneInstalled), 0, pages),
            Ok(q) => printdlg::form(Ok(&q.list), q.default_at(), pages),
            Err(cups::Error::NotRunning) => printdlg::form(Err(NoDest::NotRunning), 0, pages),
            Err(_) => printdlg::form(Err(NoDest::Unlisted), 0, pages),
        };
        *listed = Some(l);
        true
    }

    pub fn draw(&self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        if self.errors.owns(win) {
            self.errors.draw(c, &forms, &ctx.text);
            return;
        }
        let Some(f) = self.dialog(win) else {
            return;
        };
        let p = Paint {
            s: 1,
            ..forms.paint()
        };
        c.fill_rect(c.bounds(), p.cs.button_face);
        let px = forms.fonts.size_px;
        match win {
            PAGE_SETUP => pagesetup::draw(c, &ctx.text, &p, f, px),
            _ => form::draw(c, &ctx.text, &p, f, true, px),
        }
    }

    pub fn access(&self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        if self.errors.owns(win) {
            self.errors.access(tree, &ctx.text, &forms);
            return;
        }
        let Some(f) = self.dialog(win) else {
            return;
        };
        let px = forms.fonts.size_px;
        match win {
            PAGE_SETUP => {
                if let Some(n) = pagesetup::access(f, tree, ROOT, &ctx.text, px) {
                    tree.set_focus(n);
                }
            }
            _ => oneshot::access_form(f, tree, &ctx.text, px),
        }
    }

    /// The dialog form behind `win`, for its drop lists and a screen reader.
    pub fn drop_form(&mut self, win: WinId) -> Option<DropForm<'_>> {
        let form = self.dialog_mut(win)?;
        Some(DropForm { form, px: None })
    }
}

/// A program quitting while a job is still being drawn or sent waits for it, up to
/// [`QUIT_WAIT`]: the app is dropped after its windows are gone.
impl Drop for Printing {
    fn drop(&mut self) {
        let until = Instant::now() + QUIT_WAIT;
        while self.sending.iter().any(|h| !h.is_finished()) {
            if Instant::now() >= until {
                log::warn!("print: quit with a job still being sent; it was cut off");
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Draw `laid`'s pages in `range` one at a time into a PDF and send it to `queue`.
fn send(
    laid: &Laid,
    range: Range,
    queue: &str,
    title: &str,
    copies: u32,
) -> Result<i32, cups::Error> {
    let mut pdf = pdf::Writer::new(range.of(laid.pages()).len());
    laid.render(range, |p| pdf.page(&p));
    let job = cups::Job {
        title: title.into(),
        pdf: pdf.finish(),
        copies,
        media: laid.setup().paper().media.to_string(),
    };
    cups::print(&http::Server::find(), &login_name(), queue, job)
}

/// The account requests to cupsd go out under.
pub fn login_name() -> String {
    if let Ok(u) = std::env::var("USER")
        && !u.is_empty()
    {
        return u;
    }
    // SAFETY: getuid cannot fail and touches no memory of ours.
    let uid = unsafe { libc::getuid() };
    reprisefs::sysinfo::account(uid)
        .map(|(login, _)| login)
        .unwrap_or_default()
}
