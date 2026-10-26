mod acct;
mod adduser;
mod passwd;
mod picture;
mod props;
mod users;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};

use repriseapp::filedlg::{Host, Resolved, Step, type_keep};
use repriseapp::keys::{dialog_key, form_clipboard, form_input, sheet_input};
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{
    App, BTN_LEFT, Child, Ctx, DropForm, Event, FileDrop, PointerInput, Reply, WinId,
};
use reprisekit::access::Tree;
use reprisekit::controls::filedlg::{
    self, DlgMode, Event as FileEvent, Fields as FileFields, FileDlg,
};
use reprisekit::controls::msgbox::Fields;
use reprisekit::controls::{form, propsheet};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::keysym;
use reprisekit::theme::{TextUnits, Theme};

use acct::{Op, User};

const PROPS: WinId = WinId::named("props");
const ADD: WinId = WinId::named("add");
const BROWSE: WinId = WinId::named("browse");
const REMOVE: WinId = WinId::named("remove");
const ERROR: WinId = WinId::named("error");

const TITLE: &str = "User Accounts";

const ROW_PX: u32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum After {
    Read,
    /// Properties' Apply; OK closes the sheet when it succeeds.
    Props {
        close: bool,
    },
    Add,
    Remove,
}

type Answer = (Result<(), acct::Failure>, zbus::Result<Vec<User>>);

struct AccountsApp {
    users: users::Users,
    props: Option<props::Props>,
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
}

impl AccountsApp {
    fn new() -> AccountsApp {
        AccountsApp {
            users: users::Users::new(),
            props: None,
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
        true
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
        let Some((rx, after)) = &self.job else {
            return Reply::none();
        };
        let Ok((done, read)) = rx.try_recv() else {
            return Reply::none();
        };
        let after = *after;
        self.job = None;
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
        if let Some(p) = &self.props {
            let uid = p.user.uid;
            let last = self.last_admin(uid);
            match self.user(uid).cloned() {
                None => self.props = None,
                Some(u) => {
                    let p = self.props.as_mut().expect("checked above");
                    p.set_last_admin(last);
                    if let After::Props { close } = after {
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
        if let Err(acct::Failure::Text(why)) = done {
            self.fail(why);
        }
        Reply::redraw_all()
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
        self.props = Some(props::Props::new(u, face, last, self.forms.text));
    }

    fn ask_remove(&mut self, uid: u32, ctx: &mut Ctx) {
        let Some(u) = self.user(uid) else {
            return;
        };
        let b = MsgBox::new(
            "Delete Account",
            Fields {
                text: format!(
                    "Delete the account {}?\nTheir files in {} can stay on this computer or go with the account.",
                    u.shown_name(),
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
                    self.add = Some(adduser::AddUser::new(taken));
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

    fn props_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
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
                if !self.run(ops, After::Props { close: false }) {
                    p_unapplied(&mut self.props);
                }
                Reply::redraw_all()
            }
            props::Outcome::ApplyAndClose(ops) => {
                if !self.run(ops, After::Props { close: true }) {
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
            props::Outcome::Close => {
                self.props = None;
                self.browse = None;
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
                    self.run(vec![op], After::Remove);
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
        self.browse = Some(self.browse_host.open(&dir, fields, u));
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
            BROWSE => self.browse_event(ev, ctx),
            REMOVE => self.remove_event(ev),
            ERROR => self.error_event(ev),
            _ => self.main_event(ev, ctx),
        }
        .with_clipboard(clip)
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(_args: Vec<std::ffi::OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    repriseapp::run(Box::new(AccountsApp::new())).unwrap_or_else(|e| e.exit());
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
            !app.run(ops, After::Props { close: false }),
            "a second job is dropped"
        );
        assert!(!file.exists(), "the dropped Apply's picture is gone");
    }
}
