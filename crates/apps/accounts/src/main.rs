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
use reprisekit::theme::{FontConfig, Theme};

use acct::{Op, User};

const PROPS: WinId = "props";
const ADD: WinId = "add";
const BROWSE: WinId = "browse";
const REMOVE: WinId = "remove";
const ERROR: WinId = "error";

const TITLE: &str = "User Accounts";

const DOUBLE_MS: u32 = 400;
const DOUBLE_SLOP: i32 = 4;

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

type Answer = (Result<(), String>, zbus::Result<Vec<User>>);

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
    px: u32,
    last_press: Option<(u32, WinId, i32, i32)>,
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
            px: 0,
            last_press: None,
        }
    }

    fn fonts(&mut self) {
        self.text = Some(Rc::new(TextRenderer::load(&FontConfig::default())));
        self.px = Theme::default().fonts.size_px;
    }

    fn text_px(&self) -> (Rc<TextRenderer>, u32) {
        let text = self.text.clone().expect("fonts are loaded in start");
        (text, self.px)
    }

    /// Run `ops` (none for a plain read) and reread the accounts, off the
    /// event loop. One job runs at a time; a second is dropped.
    fn run(&mut self, ops: Vec<Op>, after: After) -> bool {
        if self.job.is_some() {
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
                let pictures = rows
                    .iter()
                    .map(|u| picture::of_user(&u.icon, u.you, ROW_PX))
                    .collect();
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
        if let Err(why) = done {
            self.fail(why);
        }
        Reply::redraw_all()
    }

    fn open_props(&mut self, uid: u32) {
        if self.props.as_ref().is_some_and(|p| p.user.uid == uid) {
            return;
        }
        let Some(u) = self.user(uid).cloned() else {
            return;
        };
        let face = picture::of_user(&u.icon, u.you, props::PREVIEW_PX).or_else(|| {
            self.face
                .as_ref()
                .map(|c| c.resample_box(props::PREVIEW_PX, props::PREVIEW_PX))
        });
        let last = self.last_admin(uid);
        self.props = Some(props::Props::new(u, face, last));
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
            },
        );
        self.remove = Some((uid, b));
    }

    fn main_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let cm = ctx.theme.look.controls();
        let lit = {
            let list = &mut self.users.list;
            let changed = match &ev {
                Event::Pointer(PointerInput::Motion { x, y }) => {
                    list.hover(users::LIST, *x, *y, &cm)
                }
                Event::Pointer(PointerInput::Leave) => list.leave(),
                Event::Pointer(PointerInput::Release {
                    button: BTN_LEFT, ..
                }) => list.release(),
                _ => false,
            };
            changed && ctx.styled()
        };
        let double = self.double_of(repriseapp::MAIN, &ev);
        let out = match ev {
            Event::CloseRequest => users::Outcome::Close,
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) if users::LIST.contains(x, y) => self.users.list_press(x, y, double, &cm),
            Event::Pointer(PointerInput::Motion { y, .. })
                if self.users.list.drag(users::LIST, y, &cm) =>
            {
                users::Outcome::Redraw
            }
            Event::Pointer(PointerInput::Scroll { rows, .. }) => {
                if self.users.list.scroll(rows, users::LIST) {
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
                p.sheet.open_mut().caret = ctx.caret_on;
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
                self.open_browse();
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
                d.form.caret = ctx.caret_on;
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
        let (text, px) = self.text_px();
        let Some((uid, b)) = &mut self.remove else {
            return Reply::none();
        };
        match b.event(&ev, &text, px) {
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
        let (text, px) = self.text_px();
        let Some(b) = &mut self.error else {
            return Reply::none();
        };
        match b.event(&ev, &text, px) {
            oneshot::Outcome::None => Reply::none(),
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::Closed(_) => {
                self.error = None;
                Reply::redraw_all()
            }
        }
    }

    fn open_browse(&mut self) {
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
        self.browse = Some(self.browse_host.open(&dir, fields));
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
        match host.event(d, &ev, &ctx.text, ctx.theme.fonts.size_px, ctx.size) {
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
                        match picture::load_file(&path, props::PREVIEW_PX) {
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

    fn double_of(&mut self, win: WinId, ev: &Event) -> bool {
        let Event::Pointer(PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time,
            ..
        }) = ev
        else {
            return false;
        };
        let (x, y, time) = (*x, *y, *time);
        let double = self.last_press.is_some_and(|(t, w, px, py)| {
            w == win
                && time.wrapping_sub(t) < DOUBLE_MS
                && (x - px).abs() <= DOUBLE_SLOP
                && (y - py).abs() <= DOUBLE_SLOP
        });
        self.last_press = (!double).then_some((time, win, x, y));
        double
    }
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
        users::CLIENT
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

    fn children(&self) -> Vec<Child> {
        let mut out = Vec::new();
        if let Some(p) = &self.props {
            out.push(Child {
                id: PROPS,
                title: p.window_title(),
                size: props::CLIENT,
                fixed: true,
                modal: false,
            });
        }
        if self.add.is_some() {
            out.push(Child {
                id: ADD,
                title: "Add Account".into(),
                size: adduser::CLIENT,
                fixed: true,
                modal: false,
            });
        }
        if self.browse.is_some() {
            out.push(Child {
                id: BROWSE,
                title: "Browse".into(),
                size: filedlg::CLIENT,
                fixed: false,
                modal: true,
            });
        }
        if let Some(text) = &self.text {
            if let Some((_, b)) = &self.remove {
                out.push(b.child(REMOVE, text, self.px));
            }
            if let Some(b) = &self.error {
                out.push(b.child(ERROR, text, self.px));
            }
        }
        out
    }

    fn start(&mut self, waker: repriseapp::Waker) {
        self.fonts();
        self.waker = Some(waker);
        self.run(Vec::new(), After::Read);
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        if self.face.is_none() {
            self.face = ctx.icons.get("avatar-default", ROW_PX as i32);
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

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let px = ctx.theme.fonts.size_px;
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
                    b.access(tree, &ctx.text, px);
                }
            }
            ERROR => {
                if let Some(b) = &self.error {
                    b.access(tree, &ctx.text, px);
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
            host: &self.browse_host,
        })
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let cs = &ctx.theme.colors;
        let px = ctx.theme.fonts.size_px;
        let paint = Paint {
            s: 1,
            ..ctx.theme.paint()
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
                    b.draw(c, &ctx.theme, &ctx.text, px);
                }
            }
            ERROR => {
                if let Some(b) = &self.error {
                    b.draw(c, &ctx.theme, &ctx.text, px);
                }
            }
            _ => self.users.draw(c, ctx, self.face.as_ref()),
        }
    }
}

fn main() {
    env_logger::init();
    repriseapp::run(Box::new(AccountsApp::new()));
}
