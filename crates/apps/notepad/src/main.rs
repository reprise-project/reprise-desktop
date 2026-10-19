mod document;
mod drop;
mod filedlg_host;
mod pending;

use document::{has_crlf, load, open_failed_text, read_text, write_document};
use drop::Hover;
use pending::{Pending, PendingAction};
use repriseapp::filedlg::{Host, Resolved, Step, taken, type_keep};
use repriseapp::keys::AltArm;
use repriseapp::keys::dialog_key;
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{
    App, BTN_LEFT, Child, Ctx, DropForm, Event, FileDrop, KeyInput, MAIN, PointerInput, Reply,
    WinId,
};
use reprisekit::access::{ROOT, Tree};
use reprisekit::classic::widgets::{self, Axis, ScrollHit};
use reprisekit::controls::aboutbox;
use reprisekit::controls::filedlg::{CLIENT, DlgMode, Event as FileEvent, Fields, FileDlg};
use reprisekit::controls::finddlg;
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::menubar::{self, Action, Bar, Key};
use reprisekit::controls::msgbox;
use reprisekit::controls::multiedit::{self, Metrics, MultiEdit, MultiKey, View};
use reprisekit::controls::scrollbar::ThumbDrag;
use reprisekit::controls::statusbar::{self, Panel, StatusBar};
use reprisekit::icontheme::IconCache;
use reprisekit::look::Paint;
use reprisekit::look::widgets::draw_scroll_corner;
use reprisekit::pointer::DoubleClick;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::{EditKey, TextEdit, keysym};
use reprisekit::theme::{FontConfig, Theme};
use std::path::{Path, PathBuf};
use std::rc::Rc;

const GOTO: WinId = "goto";
const GOTO_CLIENT: (i32, i32) = (268, 95);
const GOTO_LINE: &str = "line";
const GOTO_OK: &str = "ok";
const GOTO_CANCEL: &str = "cancel";

const FIND: WinId = "find";
const NOTFOUND: WinId = "notfound";
const ERRORBOX: WinId = "error";
const REPLACE_WIN: WinId = "replace";
const ABOUT: WinId = "about";
const SAVECHANGES: WinId = "savechanges";
const OVERWRITE: WinId = "overwrite";
const LOSSY: WinId = "lossy";
const OPEN: WinId = "open";
const SAVEAS: WinId = "saveas";

fn goto_form(value: &str) -> Form {
    Form::new(vec![
        Item::new(
            "linelabel",
            Rect::new(8, 12, 72, 20),
            Control::Label {
                text: "&Line Number:".into(),
                enabled: true,
                align: reprisekit::text::Align::Left,
            },
        ),
        Item::new(
            GOTO_LINE,
            Rect::new(80, 12, 86, 20),
            Control::Edit {
                edit: TextEdit::new(value, true),
                first: std::cell::Cell::new(0),
                enabled: true,
            },
        ),
        Item::new(
            GOTO_OK,
            Rect::new(8, 56, widgets::BUTTON_W, widgets::BUTTON_H),
            Control::Button {
                label: "OK".into(),
                enabled: true,
                default: true,
            },
        ),
        Item::new(
            GOTO_CANCEL,
            Rect::new(114, 56, widgets::BUTTON_W, widgets::BUTTON_H),
            Control::Button {
                label: "Cancel".into(),
                enabled: true,
                default: false,
            },
        ),
    ])
}

type FormStep = oneshot::Step;

fn form_event(
    form: &mut Option<Form>,
    ev: &Event,
    text: &TextRenderer,
    px: u32,
    caret_on: bool,
) -> FormStep {
    match ev {
        Event::CloseRequest => FormStep::Closed,
        Event::Blink => {
            if let Some(f) = form {
                f.caret = caret_on;
            }
            FormStep::Redraw
        }
        Event::Key(k) => {
            let Some(key) = dialog_key(k) else {
                return FormStep::None;
            };
            let Some(f) = form else {
                return FormStep::None;
            };
            FormStep::Out(f.key(key))
        }
        Event::Pointer(PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            ..
        }) => {
            let Some(f) = form else {
                return FormStep::None;
            };
            let hit = f.hit(*x, *y, text, px);
            FormStep::Out(f.click(hit))
        }
        Event::Pointer(PointerInput::Release {
            button: BTN_LEFT, ..
        }) => {
            let Some(f) = form else {
                return FormStep::None;
            };
            FormStep::Out(f.release())
        }
        Event::Pointer(PointerInput::Motion { x, y }) => {
            let Some(f) = form else {
                return FormStep::None;
            };
            let hit = f.hit(*x, *y, text, px);
            FormStep::Out(f.motion(*x, *y, hit))
        }
        Event::Pointer(PointerInput::Leave) => match form {
            Some(f) => FormStep::Out(f.leave()),
            None => FormStep::None,
        },
        Event::FieldDropped(e) => FormStep::Out(*e),
        _ => FormStep::None,
    }
}

fn set_actions_live(f: &mut Form, live: bool) {
    for id in [finddlg::FIND_NEXT, finddlg::REPLACE, finddlg::REPLACE_ALL] {
        if let Some(i) = f.index_of(id)
            && let Control::Button { enabled, .. } = &mut f.items[i].control
        {
            *enabled = live;
        }
    }
}

struct Notepad {
    edit: MultiEdit,
    view: View,
    metrics: Metrics,
    mono: TextRenderer,
    bar: Bar,
    path: Option<std::path::PathBuf>,
    /// The file had a CRLF break, so every break saves as CRLF, a mixed file's LF ones too.
    crlf: bool,
    /// Where the held selection drag's pointer last was; ticks scroll from there.
    dragging: Option<(i32, i32)>,
    thumb: ThumbDrag,
    /// Left presses in the text, paired into the double click that selects a word.
    text_clicks: DoubleClick<()>,
    /// Alt went down and nothing else since; Alt's release activates the menu bar unless anything
    /// took the keyboard in between.
    alt_arm: AltArm,
    status_bar: bool,
    goto: Option<Form>,
    find: Option<Form>,
    notfound: Option<MsgBox>,
    error: Option<MsgBox>,
    replace: Option<Form>,
    last_query: Option<(String, multiedit::FindOpts)>,
    info_icon: Option<Canvas>,
    /// The About dialog is open; its fields never change.
    about: bool,
    app_icon: Option<Canvas>,
    warn_icon: Option<Canvas>,
    /// Any edit sets it; undoing back to the saved text does not clear it.
    dirty: bool,
    pending: Pending,
    savechanges: bool,
    savechanges_focus: usize,
    overwrite: Option<PathBuf>,
    overwrite_focus: usize,
    lossy: Option<(MsgBox, String)>,
    /// Kept here because `children(&self)` has no `Ctx`.
    dialog_text: Rc<TextRenderer>,
    /// The dialog font's px at 100%; file dialogs stay at it.
    dialog_px: u32,
    /// The forms theme with the dialog font at the text size: Find, Replace, Go To, About and the
    /// message boxes draw and size their windows with it; the main window opens at its text size.
    forms: Theme,
    open: Option<FileDlg>,
    saveas: Option<FileDlg>,
    host: Host,
    hover: Option<Hover>,
}

fn mi(label: &str, shortcut: Option<&str>, enabled: bool) -> MenuItem {
    MenuItem::item(label)
        .enabled(enabled)
        .shortcut(shortcut.map(str::to_string))
}

mod title {
    pub const FILE: usize = 0;
    pub const EDIT: usize = 1;
    pub const VIEW: usize = 3;
    pub const HELP: usize = 4;
}

mod row {
    pub const NEW: usize = 0;
    pub const OPEN: usize = 1;
    pub const SAVE: usize = 2;
    pub const SAVE_AS: usize = 3;
    pub const EXIT: usize = 8;
    pub const UNDO: usize = 0;
    pub const CUT: usize = 2;
    pub const COPY: usize = 3;
    pub const PASTE: usize = 4;
    pub const DELETE: usize = 5;
    pub const FIND: usize = 7;
    pub const FIND_NEXT: usize = 8;
    pub const REPLACE: usize = 9;
    pub const GOTO: usize = 10;
    pub const SELECT_ALL: usize = 12;
    pub const TIME_DATE: usize = 13;
    pub const STATUS_BAR: usize = 0;
    pub const ABOUT: usize = 2;
}

fn menus() -> Vec<(&'static str, Vec<MenuItem>)> {
    vec![
        (
            "&File",
            vec![
                mi("&New", Some("Ctrl+N"), true),
                mi("&Open...", Some("Ctrl+O"), true),
                mi("&Save", Some("Ctrl+S"), true),
                mi("Save &As...", None, true),
                MenuItem::Separator,
                mi("Page Set&up...", None, false),
                mi("&Print...", Some("Ctrl+P"), false),
                MenuItem::Separator,
                mi("E&xit", None, true),
            ],
        ),
        (
            "&Edit",
            vec![
                mi("&Undo", Some("Ctrl+Z"), true),
                MenuItem::Separator,
                mi("Cu&t", Some("Ctrl+X"), true),
                mi("&Copy", Some("Ctrl+C"), true),
                mi("&Paste", Some("Ctrl+V"), true),
                mi("De&lete", Some("Del"), true),
                MenuItem::Separator,
                mi("&Find...", Some("Ctrl+F"), true),
                mi("Find &Next", Some("F3"), true),
                mi("&Replace...", Some("Ctrl+H"), true),
                mi("&Go To...", Some("Ctrl+G"), true),
                MenuItem::Separator,
                mi("Select &All", Some("Ctrl+A"), true),
                mi("Time/&Date", Some("F5"), true),
            ],
        ),
        (
            "F&ormat",
            vec![mi("&Word Wrap", None, false), mi("&Font...", None, false)],
        ),
        ("&View", vec![mi("&Status Bar", None, true)]),
        (
            "&Help",
            vec![
                mi("&Help Topics", None, false),
                MenuItem::Separator,
                mi("&About Notepad", None, true),
            ],
        ),
    ]
}

const EDIT_PX: u32 = 13;

fn edit_font() -> TextRenderer {
    let cfg = |antialias| FontConfig {
        family: vec![
            "Lucida Console".into(),
            "Liberation Mono".into(),
            "DejaVu Sans Mono".into(),
        ],
        size_px: EDIT_PX,
        antialias,
        italic: false,
        full_hinting: false,
        shell_dlg: None,
    };
    // The first face is drawn hinted monochrome; a fallback mono blotches that way at 13 px, so it
    // is antialiased.
    let mono = TextRenderer::load(&cfg(false));
    if mono.has_face("Lucida Console") {
        mono
    } else {
        TextRenderer::load(&cfg(true))
    }
}

impl Notepad {
    fn new(path: Option<PathBuf>) -> Notepad {
        let mut failed = None;
        let (text, path) = match path {
            None => (String::new(), None),
            Some(p) => match load(&p) {
                Ok(text) => (text, Some(p)),
                Err(e) => {
                    failed = Some((p, e));
                    (String::new(), None)
                }
            },
        };
        let mono = edit_font();
        let bar = Bar::new(menus());
        let mut np = Notepad {
            edit: MultiEdit::new(&text),
            crlf: has_crlf(&text),
            view: View::default(),
            metrics: Metrics::default(),
            mono,
            bar,
            path,
            dragging: None,
            thumb: ThumbDrag::default(),
            text_clicks: DoubleClick::default(),
            alt_arm: AltArm::default(),
            status_bar: false,
            goto: None,
            find: None,
            notfound: None,
            error: None,
            replace: None,
            last_query: None,
            info_icon: None,
            about: false,
            app_icon: None,
            warn_icon: None,
            dirty: false,
            pending: Pending::default(),
            savechanges: false,
            savechanges_focus: 0,
            overwrite: None,
            overwrite_focus: 1,
            lossy: None,
            dialog_text: repriseapp::start_text(),
            dialog_px: Theme::default().fonts.size_px,
            forms: Theme::default(),
            open: None,
            saveas: None,
            host: Host::default(),
            hover: None,
        };
        if let Some((p, e)) = failed {
            np.open_failed(&p, &e);
        }
        np.sync_menus();
        np
    }

    fn client(&self, ctx: &Ctx) -> Rect {
        Rect::new(0, 0, ctx.size.0, ctx.size.1)
    }

    fn bar_layout(&self, ctx: &Ctx) -> menubar::Layout {
        self.bar.layout(
            &ctx.text,
            ctx.theme.fonts.size_px,
            self.client(ctx),
            &ctx.theme.controls(),
        )
    }

    /// The status bar reserves no space of its own, so the edit gives up its height here, under
    /// a `menu_h` menu bar.
    fn edit_height(&self, h: i32, menu_h: i32, bar_h: i32) -> i32 {
        let bar = if self.status_bar { bar_h } else { 0 };
        h - menu_h - bar
    }

    fn edit_rect(&self, ctx: &Ctx) -> Rect {
        let c = self.client(ctx);
        let cm = ctx.theme.controls();
        let menu_h = menubar::bar_h(&cm);
        Rect::new(
            c.x,
            c.y + menu_h,
            c.width,
            self.edit_height(c.height, menu_h, statusbar::height(&cm)),
        )
    }

    fn bar_text(&self) -> String {
        let line = self
            .edit
            .lines()
            .get(self.edit.caret().line)
            .map(String::as_str)
            .unwrap_or("");
        // Col counts characters, not bytes and not tab stops.
        format!(
            "Ln {}, Col {}",
            self.edit.caret().line + 1,
            multiedit::char_col(line, self.edit.caret().col)
        )
    }

    fn edit_layout(&self, ctx: &Ctx) -> multiedit::Layout {
        multiedit::layout(
            &self.mono,
            &self.metrics,
            &self.edit,
            self.edit_rect(ctx),
            1,
        )
    }

    fn edit_key(&mut self, key: MultiKey, ctx: &Ctx) -> bool {
        let changed = self.edit.key(key, &self.mono, &self.metrics);
        let lay = self.edit_layout(ctx);
        self.view
            .follow(&self.mono, &self.metrics, &self.edit, &lay, 1);
        if changed {
            self.dirty = true;
        }
        changed
    }

    fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        format!("{name} - Notepad")
    }

    /// An accelerator reaches here without the menu, so a greyed row refuses it too.
    fn command(&mut self, title: usize, row: usize, ctx: &Ctx) -> Reply {
        if !matches!(
            self.bar.menus[title][row],
            MenuItem::Entry { enabled: true, .. }
        ) {
            return Reply::none();
        }
        let mut reply = Reply::redraw();
        match (title, row) {
            (title::FILE, row::NEW) => return self.confirm_then(PendingAction::New),
            (title::FILE, row::OPEN) => return self.confirm_then(PendingAction::Open),
            // A never-saved document opens Save As. Checking the path rather
            // than `save`'s bool keeps a failed write, which also returns
            // false, from raising Save As over the error box.
            (title::FILE, row::SAVE) => {
                if self.path.is_none() {
                    self.saveas_dialog();
                } else {
                    self.save();
                }
            }
            (title::FILE, row::SAVE_AS) => self.saveas_dialog(),
            (title::FILE, row::EXIT) => return self.confirm_then(PendingAction::Quit),
            (title::EDIT, row::UNDO) => {
                self.edit_key(MultiKey::Edit(EditKey::Undo), ctx);
            }
            (title::EDIT, row::CUT) => {
                reply.copy = Some(self.edit.selected_text());
                self.edit_key(MultiKey::Edit(EditKey::Cut), ctx);
            }
            (title::EDIT, row::COPY) => reply.copy = Some(self.edit.selected_text()),
            (title::EDIT, row::PASTE) => reply.paste = true,
            (title::EDIT, row::DELETE) => {
                self.edit_key(MultiKey::Edit(EditKey::Delete { word: false }), ctx);
            }
            (title::EDIT, row::FIND) => self.open_find(),
            (title::EDIT, row::REPLACE) => self.open_replace(),
            (title::EDIT, row::FIND_NEXT) => {
                if self.last_query.is_none() {
                    self.open_find();
                } else if self.find_next() {
                    let lay = self.edit_layout(ctx);
                    self.view
                        .follow(&self.mono, &self.metrics, &self.edit, &lay, 1);
                }
            }
            (title::EDIT, row::SELECT_ALL) => self.edit.select_all(),
            (title::EDIT, row::TIME_DATE) => self.insert_time_date(ctx),
            (title::EDIT, row::GOTO) => {
                let at = (self.edit.caret().line + 1).to_string();
                self.goto = Some(goto_form(&at));
            }
            (title::VIEW, row::STATUS_BAR) => {
                self.status_bar = !self.status_bar;
                if let MenuItem::Entry { checked, .. } =
                    &mut self.bar.menus[title::VIEW][row::STATUS_BAR]
                {
                    *checked = self.status_bar;
                }
            }
            (title::HELP, row::ABOUT) => self.about = true,
            _ => {}
        }
        reply
    }

    /// The edit keeps LF, so a CRLF file gets its breaks back here.
    fn write_to(&self, path: &Path) -> std::io::Result<()> {
        let sep = if self.crlf { "\r\n" } else { "\n" };
        write_document(path, self.edit.lines(), sep)
    }

    /// False when there is nowhere to write yet: the caller opens Save As.
    fn save(&mut self) -> bool {
        let Some(p) = self.path.clone() else {
            return false;
        };
        match self.write_to(&p) {
            Ok(()) => {
                self.dirty = false;
                true
            }
            Err(e) => {
                self.save_failed(&p, &e);
                false
            }
        }
    }

    fn save_failed(&mut self, path: &Path, e: &std::io::Error) {
        log::warn!("save {}: {e}", path.display());
        self.error = Some(self.error_box(&format!(
            "Cannot create the {} file.\nMake sure that the path and file name are correct.",
            path.display()
        )));
    }

    fn open_failed(&mut self, path: &Path, e: &std::io::Error) {
        log::warn!("open {}: {e}", path.display());
        if let Some(bad) = e
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<std::string::FromUtf8Error>())
        {
            let text = String::from_utf8_lossy(bad.as_bytes()).into_owned();
            let prompt = MsgBox::new(
                "Notepad",
                msgbox::Fields {
                    text: format!(
                        "{} is not a UTF-8 text file.\nDo you want to open a copy with the unreadable characters replaced?",
                        path.display()
                    ),
                    icon: self.warn_icon.clone(),
                    buttons: vec!["Yes".into(), "No".into()],
                    default: 0,
                    min_text_h: None,
                    min_w: 0,
                },
            );
            self.lossy = Some((prompt, text));
            return;
        }
        self.error = Some(self.error_box(&open_failed_text(path, e)));
    }

    /// Yes shows the copy as Untitled, so a save cannot write the replaced characters over the
    /// file.
    fn lossy_event(&mut self, ev: Event, _ctx: &mut Ctx) -> Reply {
        let Some((b, _)) = &mut self.lossy else {
            return Reply::none();
        };
        match b.event(&ev, &self.dialog_text, &self.forms) {
            oneshot::Outcome::Closed(answer) => {
                let (_, text) = self.lossy.take().expect("checked above");
                if answer == Some(0) {
                    self.show_document(&text, None);
                }
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    fn insert_time_date(&mut self, ctx: &Ctx) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let stamp = local_stamp(now);
        self.edit.paste(&stamp);
        self.dirty = true;
        let lay = self.edit_layout(ctx);
        self.view
            .follow(&self.mono, &self.metrics, &self.edit, &lay, 1);
    }

    fn key(&mut self, k: KeyInput, ctx: &Ctx) -> Reply {
        let lay = self.bar_layout(ctx);
        let m = &ctx.theme;
        let px = ctx.theme.fonts.size_px;
        let is_alt = k.sym == keysym::ALT_L || k.sym == keysym::ALT_R;
        if is_alt {
            self.alt_arm.press_alt();
            return Reply::none();
        }
        self.alt_arm.interrupt();
        let bar_key = match k.sym {
            keysym::F10 => Some(Key::Activate),
            keysym::ESCAPE => Some(Key::Escape),
            keysym::LEFT | keysym::KP_LEFT => Some(Key::Left),
            keysym::RIGHT | keysym::KP_RIGHT => Some(Key::Right),
            keysym::UP | keysym::KP_UP => Some(Key::Up),
            keysym::DOWN | keysym::KP_DOWN => Some(Key::Down),
            keysym::RETURN | keysym::KP_ENTER => Some(Key::Enter),
            _ => k
                .utf8
                .as_deref()
                .and_then(|s| s.chars().next())
                .filter(|c| c.is_alphanumeric())
                .map(|c| Key::Char(c, k.mods.alt)),
        };
        if let Some(bk) = bar_key {
            match self.bar.key(bk, &lay, &ctx.text, m, px) {
                Action::Chosen { title, row, .. } => return self.command(title, row, ctx),
                Action::Redraw => return Reply::redraw(),
                Action::Ignored => {}
            }
        }
        if let Some(r) = self.accel(&k, ctx) {
            return r;
        }
        let key = match k.sym {
            keysym::UP | keysym::KP_UP => Some(MultiKey::Up {
                select: k.mods.shift,
            }),
            keysym::DOWN | keysym::KP_DOWN => Some(MultiKey::Down {
                select: k.mods.shift,
            }),
            keysym::PAGE_UP | keysym::KP_PAGE_UP => Some(MultiKey::PageUp {
                select: k.mods.shift,
                rows: self.edit_layout(ctx).rows,
            }),
            keysym::PAGE_DOWN | keysym::KP_PAGE_DOWN => Some(MultiKey::PageDown {
                select: k.mods.shift,
                rows: self.edit_layout(ctx).rows,
            }),
            keysym::HOME | keysym::KP_HOME if k.mods.ctrl => Some(MultiKey::DocHome {
                select: k.mods.shift,
            }),
            keysym::END | keysym::KP_END if k.mods.ctrl => Some(MultiKey::DocEnd {
                select: k.mods.shift,
            }),
            keysym::RETURN | keysym::KP_ENTER => Some(MultiKey::Enter),
            keysym::TAB => Some(MultiKey::Tab),
            _ => k.edit.clone().map(MultiKey::Edit),
        };
        let Some(key) = key else {
            return Reply::none();
        };
        let mut reply = Reply::redraw();
        match &key {
            MultiKey::Edit(EditKey::Copy) | MultiKey::Edit(EditKey::Cut) => {
                reply.copy = Some(self.edit.selected_text()).filter(|s| !s.is_empty());
            }
            MultiKey::Edit(EditKey::Paste) => reply.paste = true,
            _ => {}
        }
        self.edit_key(key, ctx);
        reply
    }

    /// Alt released with nothing typed since it went down: activate the menu bar and underline the
    /// access keys.
    fn key_up(&mut self, k: KeyInput, ctx: &Ctx) -> Reply {
        // A release for a key whose press never arrived is a key the
        // compositor grabbed (Alt+space opens its window menu): it took
        // the keyboard, so the Alt that follows means nothing here.
        if k.sym != keysym::ALT_L && k.sym != keysym::ALT_R {
            self.alt_arm.interrupt();
            return Reply::none();
        }
        if !self.alt_arm.release_alt() {
            return Reply::none();
        }
        let lay = self.bar_layout(ctx);
        self.bar.key(
            Key::Activate,
            &lay,
            &ctx.text,
            &ctx.theme,
            ctx.theme.fonts.size_px,
        );
        Reply::redraw()
    }

    fn pointer(&mut self, p: PointerInput, ctx: &Ctx) -> Reply {
        // A button takes the keyboard away from a half-finished Alt:
        // its release must not activate the menu bar afterwards.
        if matches!(p, PointerInput::Press { .. }) {
            self.alt_arm.interrupt();
        }
        let lay = self.bar_layout(ctx);
        let m = &ctx.theme;
        let px = ctx.theme.fonts.size_px;
        match p {
            PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                time,
                ..
            } => {
                match self.bar.press(x, y, &lay, &ctx.text, m, px) {
                    Action::Chosen { title, row, .. } => return self.command(title, row, ctx),
                    Action::Redraw => return Reply::redraw(),
                    Action::Ignored => {}
                }
                if let Some((bar, r)) = self.status(ctx)
                    && statusbar::hit_grip(&bar, r, x, y)
                {
                    return Reply::resize();
                }
                let el = self.edit_layout(ctx);
                let over = self.bar_at(&el, x, y, ctx);
                let lit = self.view.bars.press_bar(over) && ctx.styled();
                if el.inner.contains(x, y) {
                    let pos = multiedit::pos_at(
                        &self.mono,
                        &self.metrics,
                        &self.edit,
                        &el,
                        self.view,
                        x,
                        y,
                        1,
                    );
                    if self.text_clicks.press((), m.metrics.click(x, y, time)) {
                        self.edit.select_word_at(pos);
                    } else {
                        self.edit.place(pos, false);
                        self.dragging = Some((x, y));
                    }
                    return Reply::redraw();
                }
                if let Some((bar, hit)) =
                    multiedit::bar_hit(&self.mono, &self.metrics, &self.edit, &el, self.view, x, y)
                {
                    self.thumb.press(bar.axis(), hit);
                    self.view =
                        self.view
                            .press_bar(&self.mono, &self.metrics, &self.edit, &el, bar, hit);
                    return Reply::redraw();
                }
                if lit { Reply::redraw() } else { Reply::none() }
            }
            PointerInput::Release { .. } => {
                self.dragging = None;
                self.thumb.release();
                if self.view.bars.release_bar() && ctx.styled() {
                    Reply::redraw()
                } else {
                    Reply::none()
                }
            }
            PointerInput::Leave => {
                let title = self.bar.leave();
                if (self.view.bars.leave() || title) && ctx.styled() {
                    Reply::redraw()
                } else {
                    Reply::none()
                }
            }
            PointerInput::Motion { x, y } => {
                if self.bar.open.is_some() {
                    self.view.bars.leave();
                    self.bar.motion(x, y, &lay, &ctx.text, m, px);
                    return Reply::redraw();
                }
                let title = self.bar.hover(x, y, &lay);
                let el = self.edit_layout(ctx);
                let over = self.bar_at(&el, x, y, ctx);
                let lit = (self.view.bars.motion(over) || title) && ctx.styled();
                match self.drag_motion(x, y, ctx) {
                    r if lit => Reply { redraw: true, ..r },
                    r => r,
                }
            }
            PointerInput::Scroll { rows, .. } => {
                if self.scroll(rows as i64, ctx) {
                    Reply::redraw()
                } else {
                    Reply::none()
                }
            }
            _ => Reply::none(),
        }
    }

    fn bar_at(
        &self,
        el: &multiedit::Layout,
        x: i32,
        y: i32,
        ctx: &Ctx,
    ) -> Option<(Axis, ScrollHit)> {
        if !ctx.styled() {
            return None;
        }
        multiedit::bar_hit(&self.mono, &self.metrics, &self.edit, el, self.view, x, y)
            .and_then(|(bar, hit)| Some((bar.axis(), hit.part()?)))
    }

    fn drag_motion(&mut self, x: i32, y: i32, ctx: &Ctx) -> Reply {
        if self.thumb.holding() {
            let el = self.edit_layout(ctx);
            let to = multiedit::drag_bar(
                &self.mono,
                &self.metrics,
                &self.edit,
                &el,
                self.view,
                &self.thumb,
                (x, y),
            );
            if let Some(v) = to {
                self.view = v;
            }
            return Reply::redraw();
        }
        if self.dragging.is_some() {
            self.dragging = Some((x, y));
            return self.select_drag(false, ctx);
        }
        Reply::none()
    }

    fn select_drag(&mut self, tick: bool, ctx: &Ctx) -> Reply {
        let Some((x, y)) = self.dragging else {
            return Reply::none();
        };
        let el = self.edit_layout(ctx);
        let step = multiedit::select_drag(
            &self.mono,
            &self.metrics,
            &mut self.edit,
            &el,
            &mut self.view,
            x,
            y,
            1,
            tick,
        );
        Reply {
            redraw: step.changed,
            tick: step.scrolling,
            ..Reply::none()
        }
    }

    fn scroll(&mut self, by: i64, ctx: &Ctx) -> bool {
        let lay = self.edit_layout(ctx);
        self.view.scroll_lines(by, &self.edit, lay.rows)
    }

    fn go_to_line(&mut self, value: &str) -> bool {
        let Ok(n) = value.trim().parse::<usize>() else {
            return false;
        };
        if n == 0 || n > self.edit.lines().len() {
            return false;
        }
        self.edit.place(
            multiedit::Pos {
                line: n - 1,
                col: 0,
            },
            false,
        );
        true
    }

    fn field(form: &Form, id: &str) -> String {
        match &form.item(id).expect("the field exists").control {
            Control::Edit { edit, .. } => edit.text.clone(),
            other => panic!("{id} is {other:?}, not an edit"),
        }
    }

    fn goto_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let px = ctx.forms().fonts.size_px;
        match form_event(&mut self.goto, &ev, &ctx.text, px, ctx.caret_on) {
            FormStep::Closed => {
                self.goto = None;
                Reply::redraw_all()
            }
            FormStep::Redraw => Reply::redraw(),
            FormStep::None => Reply::none(),
            FormStep::Out(out) => self.goto_outcome(out, ctx),
        }
    }

    fn goto_outcome(&mut self, out: form::Event, ctx: &Ctx) -> Reply {
        match out {
            form::Event::Cancel | form::Event::Pressed(GOTO_CANCEL) => {
                self.goto = None;
                Reply::redraw_all()
            }
            form::Event::Default | form::Event::Pressed(GOTO_OK) => {
                let value = self
                    .goto
                    .as_ref()
                    .map(|f| Notepad::field(f, GOTO_LINE))
                    .unwrap_or_default();
                if self.go_to_line(&value) {
                    let lay = self.edit_layout(ctx);
                    self.view
                        .follow(&self.mono, &self.metrics, &self.edit, &lay, 1);
                    self.goto = None;
                }
                Reply::redraw_all()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    /// Grey the rows that cannot act.
    fn sync_menus(&mut self) {
        let has_text = self.edit.lines().iter().any(|l| !l.is_empty());
        let selected = self.edit.has_selection();
        let live = [
            (row::UNDO, self.edit.can_undo()),
            (row::CUT, selected),
            (row::COPY, selected),
            (row::PASTE, repriseapp::clipboard_has_text()),
            (row::DELETE, selected),
            (row::FIND, has_text),
            (row::FIND_NEXT, has_text),
        ];
        for (r, on) in live {
            if let MenuItem::Entry { enabled, .. } = &mut self.bar.menus[title::EDIT][r] {
                *enabled = on;
            }
        }
    }

    fn find_next(&mut self) -> bool {
        let Some((query, opts)) = self.last_query.clone() else {
            return false;
        };
        // Search onward from the selection's far end, so a repeat does
        // not keep finding the match already selected.
        let (start, end) = self.edit.selection();
        let from = if opts.down { end } else { start };
        match multiedit::find(self.edit.lines(), &query, from, &opts) {
            Some((s, e)) => {
                self.edit.place(s, false);
                self.edit.place(e, true);
                true
            }
            None => {
                self.notfound = Some(self.notfound_box(&query));
                false
            }
        }
    }

    fn replace_once(&mut self, query: &str, with: &str, match_case: bool) -> bool {
        let selected = self.edit.selected_text();
        let matches = if match_case {
            selected == query
        } else {
            selected.to_lowercase() == query.to_lowercase()
        };
        if matches {
            self.edit.paste(with);
            self.dirty = true;
        }
        self.find_next();
        matches
    }

    fn replace_every(&mut self, query: &str, with: &str, o: &multiedit::FindOpts) -> usize {
        let n = self.edit.replace_all(query, with, o);
        if n > 0 {
            self.dirty = true;
        }
        n
    }

    fn open_replace(&mut self) {
        self.find = None;
        let query = self
            .last_query
            .as_ref()
            .map(|(q, _)| q.clone())
            .unwrap_or_default();
        let match_case = self.last_query.as_ref().is_some_and(|(_, o)| o.match_case);
        self.replace = Some(finddlg::replace_form(&finddlg::ReplaceFields {
            query,
            with: String::new(),
            match_case,
        }));
    }

    fn open_find(&mut self) {
        self.replace = None;
        let query = self
            .last_query
            .as_ref()
            .map(|(q, _)| q.clone())
            .unwrap_or_default();
        let down = self
            .last_query
            .as_ref()
            .map(|(_, o)| o.down)
            .unwrap_or(true);
        let match_case = self.last_query.as_ref().is_some_and(|(_, o)| o.match_case);
        self.find = Some(finddlg::find_form(&finddlg::FindFields {
            query,
            match_case,
            down,
        }));
    }

    fn follow(&mut self, ctx: &Ctx) {
        self.forms = ctx.forms();
        self.forms.fonts.size_px = self.forms.text.px(self.dialog_px);
        self.metrics.follow_look(&ctx.theme.look.controls());
        let px = reprisekit::theme::text_px(EDIT_PX, ctx.text_size);
        self.metrics.font_px = px;
        self.metrics.line_h = px as i32;
    }

    /// `IconCache::get` needs `&mut` and `App::draw` gets `&Ctx`, so icons are fetched here, during
    /// an event, and kept.
    fn prime(&mut self, ctx: &mut Ctx) {
        self.follow(ctx);
        if self.info_icon.is_none() {
            self.info_icon = ctx.icons.get("dialog-information", 32);
        }
        if self.app_icon.is_none() {
            self.app_icon = ctx.icons.get("accessories-text-editor", 32);
        }
        if self.warn_icon.is_none() {
            self.warn_icon = ctx.icons.get("dialog-warning", 32);
        }
        let raised = self
            .error
            .iter_mut()
            .chain(self.lossy.iter_mut().map(|(b, _)| b));
        for b in raised.filter(|b| b.fields.icon.is_none()) {
            b.fields.icon = self.warn_icon.clone();
        }
        self.host.icons.prime(&mut ctx.icons);
    }

    /// repriseapp routes keys only to the focused window, so a modeless Find hands these back here.
    fn accel(&mut self, k: &KeyInput, ctx: &Ctx) -> Option<Reply> {
        if k.mods.ctrl {
            match k.sym | 0x20 {
                0x6e => return Some(self.command(title::FILE, row::NEW, ctx)),
                0x6f => return Some(self.command(title::FILE, row::OPEN, ctx)),
                0x73 => return Some(self.command(title::FILE, row::SAVE, ctx)),
                0x66 => return Some(self.command(title::EDIT, row::FIND, ctx)),
                0x68 => return Some(self.command(title::EDIT, row::REPLACE, ctx)),
                0x67 => return Some(self.command(title::EDIT, row::GOTO, ctx)),
                _ => {}
            }
        }
        if k.sym == keysym::F3 {
            return Some(self.command(title::EDIT, row::FIND_NEXT, ctx));
        }
        if k.sym == keysym::F5 {
            self.insert_time_date(ctx);
            return Some(Reply::redraw());
        }
        None
    }

    fn checked(form: &Form, id: &str) -> bool {
        match &form.item(id).expect("the field exists").control {
            Control::Check { checked, .. } => *checked,
            Control::Radio { checked, .. } => *checked,
            other => panic!("{id} is {other:?}, not a check or radio"),
        }
    }

    fn error_box(&self, text: &str) -> MsgBox {
        MsgBox::new(
            "Notepad",
            msgbox::Fields {
                text: text.into(),
                icon: self.warn_icon.clone(),
                buttons: vec!["OK".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
            },
        )
    }

    fn notfound_box(&self, query: &str) -> MsgBox {
        MsgBox::new(
            "Notepad",
            msgbox::Fields {
                text: format!("Cannot find \"{query}\""),
                icon: self.info_icon.clone(),
                buttons: vec!["OK".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
            },
        )
    }

    fn about_fields(&self) -> aboutbox::Fields {
        repriseapp::about::fields("Notepad", self.app_icon.clone())
    }

    /// Laid out for the 100% font; the banner is drawn at the grown size.
    fn about_form(&self) -> Form {
        let (text, px) = (&self.dialog_text, self.dialog_px);
        let mut f = aboutbox::form(&self.about_fields(), text, px);
        let u = self.forms.text;
        let w = u.len(aboutbox::CLIENT.0);
        f.set_picture("banner", aboutbox::banner_wide(text, px, w, u));
        f
    }

    fn msgbox_size(&self, fields: &msgbox::Fields) -> (i32, i32) {
        let u = self.forms.text;
        u.size(msgbox::client(
            fields,
            &self.dialog_text,
            self.forms.fonts.size_px,
            u,
        ))
    }

    fn find_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if let Event::Key(k) = &ev
            && let Some(r) = self.accel(k, ctx)
        {
            return r;
        }
        match form_event(
            &mut self.find,
            &ev,
            &ctx.text,
            self.forms.fonts.size_px,
            ctx.caret_on,
        ) {
            FormStep::Closed => {
                self.find = None;
                Reply::redraw_all()
            }
            FormStep::Redraw => Reply::redraw(),
            FormStep::None => Reply::none(),
            FormStep::Out(out) => self.find_outcome(out, ctx),
        }
    }

    fn find_outcome(&mut self, out: form::Event, ctx: &Ctx) -> Reply {
        match out {
            form::Event::Cancel | form::Event::Pressed(finddlg::CANCEL) => {
                self.find = None;
                Reply::redraw_all()
            }
            form::Event::Default | form::Event::Pressed(finddlg::FIND_NEXT) => {
                if let Some(f) = &self.find {
                    let query = Notepad::field(f, finddlg::QUERY);
                    let match_case = Notepad::checked(f, finddlg::MATCH_CASE);
                    let down = Notepad::checked(f, finddlg::DOWN);
                    self.last_query = Some((query, multiedit::FindOpts { match_case, down }));
                }
                if self.find_next() {
                    let lay = self.edit_layout(ctx);
                    self.view
                        .follow(&self.mono, &self.metrics, &self.edit, &lay, 1);
                }
                Reply::redraw_all()
            }
            form::Event::Changed(finddlg::QUERY) => {
                if let Some(f) = &mut self.find {
                    let live = !Notepad::field(f, finddlg::QUERY).is_empty();
                    set_actions_live(f, live);
                }
                Reply::redraw_all()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    fn replace_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if let Event::Key(k) = &ev
            && let Some(r) = self.accel(k, ctx)
        {
            return r;
        }
        match form_event(
            &mut self.replace,
            &ev,
            &ctx.text,
            self.forms.fonts.size_px,
            ctx.caret_on,
        ) {
            FormStep::Closed => {
                self.replace = None;
                Reply::redraw_all()
            }
            FormStep::Redraw => Reply::redraw(),
            FormStep::None => Reply::none(),
            FormStep::Out(out) => self.replace_outcome(out, ctx),
        }
    }

    fn replace_outcome(&mut self, out: form::Event, ctx: &Ctx) -> Reply {
        match out {
            form::Event::Cancel | form::Event::Pressed(finddlg::CANCEL) => {
                self.replace = None;
                Reply::redraw_all()
            }
            form::Event::Default | form::Event::Pressed(finddlg::FIND_NEXT) => {
                if let Some(f) = &self.replace {
                    let query = Notepad::field(f, finddlg::QUERY);
                    let match_case = Notepad::checked(f, finddlg::MATCH_CASE);
                    self.last_query = Some((
                        query,
                        multiedit::FindOpts {
                            match_case,
                            down: true,
                        },
                    ));
                }
                if self.find_next() {
                    let lay = self.edit_layout(ctx);
                    self.view
                        .follow(&self.mono, &self.metrics, &self.edit, &lay, 1);
                }
                Reply::redraw_all()
            }
            form::Event::Pressed(finddlg::REPLACE) => {
                let mut hit = false;
                if let Some(f) = &self.replace {
                    let query = Notepad::field(f, finddlg::QUERY);
                    let with = Notepad::field(f, finddlg::WITH);
                    let match_case = Notepad::checked(f, finddlg::MATCH_CASE);
                    self.last_query = Some((
                        query.clone(),
                        multiedit::FindOpts {
                            match_case,
                            down: true,
                        },
                    ));
                    hit = self.replace_once(&query, &with, match_case);
                }
                if hit {
                    let lay = self.edit_layout(ctx);
                    self.view
                        .follow(&self.mono, &self.metrics, &self.edit, &lay, 1);
                }
                Reply::redraw_all()
            }
            form::Event::Pressed(finddlg::REPLACE_ALL) => {
                let mut hit = false;
                if let Some(f) = &self.replace {
                    let query = Notepad::field(f, finddlg::QUERY);
                    let with = Notepad::field(f, finddlg::WITH);
                    let match_case = Notepad::checked(f, finddlg::MATCH_CASE);
                    let opts = multiedit::FindOpts {
                        match_case,
                        down: true,
                    };
                    hit = self.replace_every(&query, &with, &opts) > 0;
                    self.last_query = Some((query, opts));
                }
                if hit {
                    let lay = self.edit_layout(ctx);
                    self.view
                        .follow(&self.mono, &self.metrics, &self.edit, &lay, 1);
                }
                Reply::redraw_all()
            }
            form::Event::Changed(finddlg::QUERY) => {
                if let Some(f) = &mut self.replace {
                    let live = !Notepad::field(f, finddlg::QUERY).is_empty();
                    set_actions_live(f, live);
                }
                Reply::redraw_all()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    fn notfound_event(&mut self, ev: Event, _ctx: &mut Ctx) -> Reply {
        let Some(b) = &mut self.notfound else {
            return Reply::none();
        };
        match b.event(&ev, &self.dialog_text, &self.forms) {
            oneshot::Outcome::Closed(_) => {
                self.notfound = None;
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    fn error_event(&mut self, ev: Event, _ctx: &mut Ctx) -> Reply {
        let Some(b) = &mut self.error else {
            return Reply::none();
        };
        match b.event(&ev, &self.dialog_text, &self.forms) {
            oneshot::Outcome::Closed(_) => {
                self.error = None;
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    /// About's own loop.
    fn about_event(&mut self, ev: Event, _ctx: &mut Ctx) -> Reply {
        let step = oneshot::event(&ev, &self.dialog_text, &self.forms, || self.about_form());
        match step {
            FormStep::Closed => {
                self.about = false;
                Reply::redraw_all()
            }
            FormStep::Redraw => Reply::redraw(),
            FormStep::None => Reply::none(),
            FormStep::Out(out) => self.about_outcome(out),
        }
    }

    fn about_outcome(&mut self, out: form::Event) -> Reply {
        match out {
            form::Event::Cancel | form::Event::Default | form::Event::Pressed(aboutbox::OK) => {
                self.about = false;
                Reply::redraw_all()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    fn savechanges_fields(&self) -> msgbox::Fields {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        msgbox::Fields {
            text: format!(
                "The text in the {name} file has changed.\n\
                 Do you want to save the changes?"
            ),
            icon: self.warn_icon.clone(),
            buttons: vec!["&Yes".into(), "&No".into(), "Cancel".into()],
            default: self.savechanges_focus,
            min_text_h: None,
            min_w: 0,
        }
    }

    fn confirm_then(&mut self, action: PendingAction) -> Reply {
        match self.pending.ask(action, self.dirty) {
            Some(action) => self.run_pending(action),
            None => {
                self.savechanges = true;
                self.savechanges_focus = 0;
                Reply::redraw_all()
            }
        }
    }

    fn run_pending(&mut self, action: PendingAction) -> Reply {
        match action {
            PendingAction::Quit => Reply {
                quit: true,
                ..Default::default()
            },
            PendingAction::New => {
                self.edit = MultiEdit::new("");
                self.crlf = false;
                self.view = View::default();
                self.path = None;
                self.dirty = false;
                Reply::redraw_all()
            }
            PendingAction::Open => {
                self.open_dialog();
                Reply::redraw_all()
            }
            PendingAction::OpenFile(path) => {
                match load(&path) {
                    Ok(text) => self.show_document(&text, Some(path)),
                    Err(e) => self.open_failed(&path, &e),
                }
                Reply::redraw_all()
            }
        }
    }

    /// With nowhere to write yet, Save As opens and the action keeps waiting on it.
    fn savechanges_yes(&mut self) -> Reply {
        self.savechanges = false;
        if self.path.is_none() {
            self.saveas_dialog();
            return Reply::redraw_all();
        }
        let ok = self.save();
        match self.pending.saved(ok) {
            Some(action) => self.run_pending(action),
            None => Reply::redraw_all(),
        }
    }

    fn savechanges_event(&mut self, ev: Event, _ctx: &mut Ctx) -> Reply {
        let mut fields = self.savechanges_fields();
        let step = oneshot::msgbox_event(&ev, &self.dialog_text, &self.forms, &mut fields);
        self.savechanges_focus = fields.default;
        match step {
            FormStep::Closed => {
                self.savechanges = false;
                self.pending.cancelled();
                Reply::redraw_all()
            }
            FormStep::Redraw => Reply::redraw(),
            FormStep::None => Reply::none(),
            FormStep::Out(out) => self.savechanges_outcome(out),
        }
    }

    fn savechanges_outcome(&mut self, out: form::Event) -> Reply {
        match out {
            form::Event::Default => self.savechanges_yes(),
            form::Event::Pressed(id) if id == msgbox::button_id(0) => self.savechanges_yes(),
            form::Event::Pressed(id) if id == msgbox::button_id(1) => {
                self.savechanges = false;
                self.dirty = false;
                match self.pending.discarded() {
                    Some(action) => self.run_pending(action),
                    None => Reply::redraw_all(),
                }
            }
            form::Event::Cancel | form::Event::Pressed(_) => {
                self.savechanges = false;
                self.pending.cancelled();
                Reply::redraw_all()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }
}

fn local_stamp(secs: i64) -> String {
    let tm = reprisefs::locale::localtime(secs);
    let (h12, ampm) = match tm.tm_hour {
        0 => (12, "AM"),
        1..=11 => (tm.tm_hour, "AM"),
        12 => (12, "PM"),
        h => (h - 12, "PM"),
    };
    format!(
        "{}:{:02} {} {}/{}/{}",
        h12,
        tm.tm_min,
        ampm,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_year + 1900
    )
}

impl App for Notepad {
    fn title(&self) -> String {
        Notepad::title(self)
    }
    fn app_id(&self) -> String {
        "reprise.notepad".into()
    }
    fn initial_size(&self) -> (i32, i32) {
        self.forms.text.size((568, 328))
    }

    fn text_sized(&self) -> bool {
        true
    }

    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        match win {
            OPEN => self.open.as_ref().map(FileDlg::min_client),
            SAVEAS => self.saveas.as_ref().map(FileDlg::min_client),
            _ => None,
        }
    }

    fn children(&self) -> Vec<Child> {
        let mut kids = Vec::new();
        if self.goto.is_some() {
            kids.push(Child {
                id: GOTO,
                title: "Goto line".into(),
                size: self.forms.text.size(GOTO_CLIENT),
                fixed: true,
                modal: true,
            });
        }
        if self.find.is_some() {
            kids.push(Child {
                id: FIND,
                title: "Find".into(),
                size: self.forms.text.size(finddlg::FIND_CLIENT),
                fixed: true,
                modal: false,
            });
        }
        if self.replace.is_some() {
            kids.push(Child {
                id: REPLACE_WIN,
                title: "Replace".into(),
                size: self.forms.text.size(finddlg::REPLACE_CLIENT),
                fixed: true,
                modal: false,
            });
        }
        if let Some(b) = &self.notfound {
            kids.push(b.child(NOTFOUND, &self.dialog_text, &self.forms));
        }
        if let Some(b) = &self.error {
            kids.push(b.child(ERRORBOX, &self.dialog_text, &self.forms));
        }
        if self.about {
            kids.push(Child {
                id: ABOUT,
                title: "About Notepad".into(),
                size: self.forms.text.size(aboutbox::CLIENT),
                fixed: true,
                modal: true,
            });
        }
        if self.savechanges {
            let fields = self.savechanges_fields();
            kids.push(Child {
                id: SAVECHANGES,
                title: "Notepad".into(),
                size: self.msgbox_size(&fields),
                fixed: true,
                modal: true,
            });
        }
        if self.open.is_some() {
            kids.push(Child {
                id: OPEN,
                title: "Open".into(),
                size: self.open.as_ref().map_or(CLIENT, FileDlg::client),
                fixed: false,
                modal: true,
            });
        }
        if self.saveas.is_some() {
            kids.push(Child {
                id: SAVEAS,
                title: "Save As".into(),
                size: self.saveas.as_ref().map_or(CLIENT, FileDlg::client),
                fixed: false,
                modal: true,
            });
        }
        if let Some((b, _)) = &self.lossy {
            kids.push(b.child(LOSSY, &self.dialog_text, &self.forms));
        }
        if let Some(path) = &self.overwrite {
            let fields = self.overwrite_fields(path);
            kids.push(Child {
                id: OVERWRITE,
                title: "Save As".into(),
                size: self.msgbox_size(&fields),
                fixed: true,
                modal: true,
            });
        }
        kids
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.dialog_text = ctx.text.clone();
        self.prime(ctx);
        // The keyboard is in a dialog, so a held Alt will be released there.
        if win != MAIN {
            self.alt_arm.interrupt();
        }
        // Selection and text change in many places (the pointer, Find, Replace), so the rows
        // are greyed after every event, before the next draw or menu choice.
        let reply = match win {
            GOTO => self.goto_event(ev, ctx),
            FIND => self.find_event(ev, ctx),
            REPLACE_WIN => self.replace_event(ev, ctx),
            NOTFOUND => self.notfound_event(ev, ctx),
            ERRORBOX => self.error_event(ev, ctx),
            ABOUT => self.about_event(ev, ctx),
            SAVECHANGES => self.savechanges_event(ev, ctx),
            OVERWRITE => self.overwrite_event(ev, ctx),
            LOSSY => self.lossy_event(ev, ctx),
            OPEN => self.open_event(ev, ctx),
            SAVEAS => self.saveas_event(ev, ctx),
            _ => self.main_event(ev, ctx),
        };
        self.sync_menus();
        reply
    }

    fn drop_form(&mut self, win: WinId) -> Option<DropForm<'_>> {
        let (form, px) = match win {
            GOTO => (self.goto.as_mut()?, None),
            FIND => (self.find.as_mut()?, Some(self.forms.fonts.size_px)),
            REPLACE_WIN => (self.replace.as_mut()?, Some(self.forms.fonts.size_px)),
            _ => return None,
        };
        Some(DropForm { form, px })
    }

    fn drop_file_dialog(&mut self, win: WinId) -> Option<FileDrop<'_>> {
        let dlg = match win {
            OPEN => self.open.as_mut()?,
            SAVEAS => self.saveas.as_mut()?,
            _ => return None,
        };
        Some(FileDrop {
            dlg,
            host: &mut self.host,
        })
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.dialog_text = ctx.text.clone();
        self.follow(ctx);
        let cs = &ctx.theme.colors;
        let forms = ctx.forms();
        match win {
            GOTO => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(f) = &self.goto {
                    form::draw(
                        c,
                        &ctx.text,
                        &Paint {
                            s: 1,
                            ..forms.paint()
                        },
                        f,
                        true,
                        forms.fonts.size_px,
                    );
                }
            }
            FIND => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(f) = &self.find {
                    form::draw(
                        c,
                        &ctx.text,
                        &Paint {
                            s: 1,
                            ..self.forms.paint()
                        },
                        f,
                        true,
                        self.forms.fonts.size_px,
                    );
                }
            }
            REPLACE_WIN => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(f) = &self.replace {
                    form::draw(
                        c,
                        &ctx.text,
                        &Paint {
                            s: 1,
                            ..self.forms.paint()
                        },
                        f,
                        true,
                        self.forms.fonts.size_px,
                    );
                }
            }
            NOTFOUND => {
                if let Some(b) = &self.notfound {
                    b.draw(c, &self.forms, &self.dialog_text);
                }
            }
            ERRORBOX => {
                if let Some(b) = &self.error {
                    b.draw(c, &self.forms, &self.dialog_text);
                }
            }
            LOSSY => {
                if let Some((b, _)) = &self.lossy {
                    b.draw(c, &self.forms, &self.dialog_text);
                }
            }
            ABOUT => {
                c.fill_rect(c.bounds(), cs.button_face);
                let f = self.about_form();
                form::draw(
                    c,
                    &self.dialog_text,
                    &Paint {
                        s: 1,
                        ..self.forms.paint()
                    },
                    &f,
                    true,
                    self.forms.fonts.size_px,
                );
            }
            SAVECHANGES => {
                c.fill_rect(c.bounds(), cs.button_face);
                let f = msgbox::form(
                    &self.savechanges_fields(),
                    &self.dialog_text,
                    self.forms.fonts.size_px,
                    self.forms.text,
                );
                form::draw(
                    c,
                    &self.dialog_text,
                    &Paint {
                        s: 1,
                        ..self.forms.paint()
                    },
                    &f,
                    true,
                    self.forms.fonts.size_px,
                );
            }
            OVERWRITE => {
                if let Some(path) = &self.overwrite {
                    c.fill_rect(c.bounds(), cs.button_face);
                    let f = msgbox::form(
                        &self.overwrite_fields(path),
                        &self.dialog_text,
                        self.forms.fonts.size_px,
                        self.forms.text,
                    );
                    form::draw(
                        c,
                        &self.dialog_text,
                        &Paint {
                            s: 1,
                            ..self.forms.paint()
                        },
                        &f,
                        true,
                        self.forms.fonts.size_px,
                    );
                }
            }
            OPEN => {
                if let Some(d) = &self.open {
                    d.draw(
                        c,
                        &self.dialog_text,
                        &Paint {
                            s: 1,
                            ..self.forms.paint()
                        },
                        Rect::new(0, 0, d.client().0, d.client().1),
                        self.forms.fonts.size_px,
                    );
                }
            }
            SAVEAS => {
                if let Some(d) = &self.saveas {
                    d.draw(
                        c,
                        &self.dialog_text,
                        &Paint {
                            s: 1,
                            ..self.forms.paint()
                        },
                        Rect::new(0, 0, d.client().0, d.client().1),
                        self.forms.fonts.size_px,
                    );
                }
            }
            _ => self.draw_main(c, ctx),
        }
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        self.follow(ctx);
        let dt = &self.dialog_text;
        let (forms, fpx) = (&self.forms, self.forms.fonts.size_px);
        let built = |f: &Form, tree: &mut Tree| {
            f.drawn_with(&forms.paint());
            oneshot::access_form(f, tree, dt, fpx);
        };
        match win {
            GOTO => {
                if let Some(f) = &self.goto {
                    oneshot::access_form(f, tree, &ctx.text, ctx.forms().fonts.size_px);
                }
            }
            FIND => {
                if let Some(f) = &self.find {
                    oneshot::access_form(f, tree, &ctx.text, fpx);
                }
            }
            REPLACE_WIN => {
                if let Some(f) = &self.replace {
                    oneshot::access_form(f, tree, &ctx.text, fpx);
                }
            }
            NOTFOUND => {
                if let Some(b) = &self.notfound {
                    b.access(tree, dt, forms);
                }
            }
            ERRORBOX => {
                if let Some(b) = &self.error {
                    b.access(tree, dt, forms);
                }
            }
            LOSSY => {
                if let Some((b, _)) = &self.lossy {
                    b.access(tree, dt, forms);
                }
            }
            ABOUT => {
                built(&self.about_form(), tree);
            }
            SAVECHANGES => {
                built(
                    &msgbox::form(&self.savechanges_fields(), dt, fpx, forms.text),
                    tree,
                );
            }
            OVERWRITE => {
                if let Some(path) = &self.overwrite {
                    built(
                        &msgbox::form(&self.overwrite_fields(path), dt, fpx, forms.text),
                        tree,
                    );
                }
            }
            OPEN => {
                if let Some(d) = &self.open {
                    d.access(tree, Rect::new(0, 0, d.client().0, d.client().1), dt, fpx);
                }
            }
            SAVEAS => {
                if let Some(d) = &self.saveas {
                    d.access(tree, Rect::new(0, 0, d.client().0, d.client().1), dt, fpx);
                }
            }
            _ => self.access_main(tree, ctx),
        }
    }
}

impl Notepad {
    fn main_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        match ev {
            Event::Key(k) => self.key(k, ctx),
            Event::KeyUp(k) => self.key_up(k, ctx),
            Event::Pointer(p) => self.pointer(p, ctx),
            Event::Pasted(text) => {
                if self.edit.paste(&text) {
                    self.dirty = true;
                    let lay = self.edit_layout(ctx);
                    self.view
                        .follow(&self.mono, &self.metrics, &self.edit, &lay, 1);
                }
                Reply::redraw()
            }
            Event::CloseRequest => self.confirm_then(PendingAction::Quit),
            Event::Focus(false) => {
                self.alt_arm.interrupt();
                Reply::redraw()
            }
            Event::Resized | Event::Blink | Event::Focus(_) => Reply::redraw(),
            Event::DragEnter { .. }
            | Event::DragMotion { .. }
            | Event::DragFiles { .. }
            | Event::DragLeave
            | Event::DragDrop { .. } => self.drop_event(ev, ctx),
            Event::Tick if self.dragging.is_some() => self.select_drag(true, ctx),
            Event::Tick => self.drop_event(ev, ctx),
            Event::PastedFiles(_)
            | Event::PastedPrimary(_)
            | Event::FieldDropped(_)
            | Event::Wake
            | Event::DragEnd { .. }
            | Event::DropCancelled
            | Event::Output(_)
            | Event::ShortcutsInactive
            | Event::Heads(_)
            | Event::HeadsApplied { .. } => Reply::none(),
        }
    }

    fn draw_main(&self, c: &mut Canvas, ctx: &Ctx) {
        let cs = &ctx.theme.colors;
        c.fill_rect(self.client(ctx), cs.button_face);
        let el = self.edit_layout(ctx);
        multiedit::draw(
            c,
            &self.mono,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &self.metrics,
            &self.edit,
            self.view,
            &el,
            reprisekit::textedit::Caret::new(true, ctx.caret_on),
        );
        if let Some(at) = self.drop_point(ctx) {
            multiedit::draw_drop_point(
                c,
                &self.mono,
                &Paint {
                    s: 1,
                    ..ctx.theme.paint()
                },
                &self.metrics,
                &self.edit,
                self.view,
                &el,
                at,
            );
        }
        if let Some(corner) = el.corner().filter(|_| !ctx.maximized && !self.status_bar) {
            draw_scroll_corner(
                c,
                &Paint {
                    s: 1,
                    ..ctx.theme.paint()
                },
                corner,
                true,
            );
        }
        if let Some((bar, r)) = self.status(ctx) {
            statusbar::draw(
                c,
                &ctx.text,
                &Paint {
                    s: 1,
                    ..ctx.theme.paint()
                },
                &bar,
                r,
                ctx.theme.fonts.size_px,
            );
        }
        let lay = self.bar_layout(ctx);
        self.bar.draw(c, &ctx.text, &ctx.theme, &lay);
    }

    fn status(&self, ctx: &Ctx) -> Option<(StatusBar, Rect)> {
        if !self.status_bar {
            return None;
        }
        let bar = StatusBar {
            panels: vec![
                Panel {
                    text: String::new(),
                    width: 0,
                    pad: statusbar::TEXT_PAD,
                    icon: None,
                },
                Panel {
                    text: self.bar_text(),
                    width: 356,
                    pad: statusbar::TEXT_PAD,
                    icon: None,
                },
            ],
            grip: !ctx.maximized,
        };
        let h = statusbar::height(&ctx.theme.controls());
        let r = Rect::new(0, ctx.size.1 - h, ctx.size.0, h);
        Some((bar, r))
    }

    fn access_main(&self, tree: &mut Tree, ctx: &Ctx) {
        let menus = self.bar.access(&self.bar_layout(ctx), tree, ROOT);
        let el = self.edit_layout(ctx);
        let edit = multiedit::access(
            &self.edit,
            self.view,
            &el,
            &self.metrics,
            &self.mono,
            tree,
            ROOT,
            "edit",
        );
        if let Some((bar, r)) = self.status(ctx) {
            statusbar::access(&bar, r, ctx.theme.text, tree, ROOT);
        }
        tree.set_focus(menus.unwrap_or(edit));
    }
}

fn main() {
    reprisebus::journal::init_logger("error");
    let path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let mut notepad = Notepad::new(path);
    notepad.forms = repriseapp::start_forms();
    repriseapp::run(Box::new(notepad));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_to_fits_its_text_under_every_interface() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            vec![("Go To Line".into(), goto_form("12345"))]
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }
    use repriseapp::filedlg::places;

    #[test]
    fn a_fallback_edit_font_is_antialiased() {
        let mono = edit_font();
        if mono.has_face("Lucida Console") {
            return;
        }
        let mut c = Canvas::new(120, 20);
        c.fill_rect(
            Rect::new(0, 0, 120, 20),
            reprisekit::theme::Color::rgb(255, 255, 255),
        );
        let r = Rect::new(0, 0, 120, 20);
        mono.draw(
            &mut c,
            r,
            "Things to do",
            false,
            reprisekit::theme::Color::rgb(0, 0, 0),
            reprisekit::text::Align::Left,
            EDIT_PX,
        );
        let grey = (0..20)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .map(|(x, y)| c.get(x, y) & 0xff)
            .filter(|&v| v != 0 && v != 0xff)
            .count();
        assert!(
            grey > 0,
            "a fallback mono font draws grey edge pixels, not monochrome"
        );
    }

    fn label_of(item: &MenuItem) -> String {
        match item {
            MenuItem::Entry { label, .. } => label.clone(),
            MenuItem::Separator => panic!("that row is a separator"),
        }
    }

    fn label_at(t: usize, r: usize) -> String {
        label_of(&menus()[t].1[r])
    }

    #[test]
    fn menu_constants_point_at_their_rows() {
        assert_eq!(label_at(title::FILE, row::NEW), "New");
        assert_eq!(label_at(title::FILE, row::SAVE), "Save");
        assert_eq!(label_at(title::FILE, row::EXIT), "Exit");
        assert_eq!(label_at(title::EDIT, row::UNDO), "Undo");
        assert_eq!(label_at(title::EDIT, row::FIND), "Find...");
        assert_eq!(label_at(title::EDIT, row::FIND_NEXT), "Find Next");
        assert_eq!(label_at(title::EDIT, row::REPLACE), "Replace...");
        assert_eq!(label_at(title::EDIT, row::GOTO), "Go To...");
        assert_eq!(label_at(title::EDIT, row::SELECT_ALL), "Select All");
        assert_eq!(label_at(title::EDIT, row::TIME_DATE), "Time/Date");
        assert_eq!(label_at(title::VIEW, row::STATUS_BAR), "Status Bar");
        assert_eq!(label_at(title::HELP, row::ABOUT), "About Notepad");
    }

    #[test]
    fn the_bar_reads_the_caret_as_line_and_column() {
        let mut np = Notepad::new(None);
        assert_eq!(np.bar_text(), "Ln 1, Col 1");
        np.edit = MultiEdit::new("ab\tc");
        np.edit.place(multiedit::Pos { line: 0, col: 4 }, false);
        assert_eq!(np.bar_text(), "Ln 1, Col 5");
        np.edit = MultiEdit::new("one\ntwo");
        np.edit.place(multiedit::Pos { line: 1, col: 3 }, false);
        assert_eq!(np.bar_text(), "Ln 2, Col 4");
    }

    /// Showing the bar takes its height off the edit, so the two never overlap.
    #[test]
    fn showing_the_bar_shortens_the_edit() {
        let mut np = Notepad::new(None);
        assert!(!np.status_bar, "starts with the bar hidden");
        let tall = np.edit_height(400, menubar::BAR_H, 23);
        np.status_bar = true;
        let short = np.edit_height(400, menubar::BAR_H, 23);
        assert_eq!(tall - short, 23);
    }

    #[test]
    fn the_status_bars_grip_resizes_the_window() {
        let mut ctx = test_ctx();
        let mut np = Notepad::new(None);
        np.status_bar = true;
        let (w, h) = ctx.size;
        let press = |x, y| PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: repriseapp::Mods::default(),
        };
        assert!(np.pointer(press(w - 4, h - 4), &ctx).resize, "on the grip");
        assert!(
            !np.pointer(press(w - 40, h - 4), &ctx).resize,
            "on the panel beside it"
        );
        ctx.maximized = true;
        assert!(
            !np.pointer(press(w - 4, h - 4), &ctx).resize,
            "no grip while maximized"
        );
    }

    #[test]
    fn a_double_click_in_the_text_selects_a_word_within_the_configured_time() {
        let mut ctx = test_ctx();
        ctx.theme.metrics.double_click_ms = 900;
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new("alpha beta");
        let el = np.edit_layout(&ctx);
        let (x, y) = (el.inner.x + 4, el.inner.y + 4);
        let press = |x, y, time| PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time,
            mods: repriseapp::Mods::default(),
        };
        let release = PointerInput::Release {
            x,
            y,
            button: BTN_LEFT,
            mods: repriseapp::Mods::default(),
        };
        np.pointer(press(x, y, 1000), &ctx);
        np.pointer(release, &ctx);
        np.pointer(press(x + 4, y, 1800), &ctx);
        assert_eq!(np.edit.selected_text(), "alpha ", "800 ms and 4 px apart");
        np.pointer(release, &ctx);
        np.pointer(press(x, y, 3000), &ctx);
        np.pointer(release, &ctx);
        np.pointer(press(x + 5, y, 3100), &ctx);
        assert!(!np.edit.has_selection(), "5 px apart is two clicks");
    }

    /// Opening Go To declares the child window; closing it withdraws it.
    #[test]
    fn go_to_opens_and_closes_as_a_child() {
        let mut np = Notepad::new(None);
        assert!(np.children().is_empty(), "nothing is open to start with");
        np.goto = Some(goto_form("1"));
        let kids = np.children();
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].id, GOTO);
        assert!(kids[0].modal, "Go To is modal");
        assert!(kids[0].fixed, "and cannot be resized");
        np.goto = None;
        assert!(np.children().is_empty());
    }

    /// A line number outside the document does not move the caret.
    #[test]
    fn go_to_only_accepts_a_line_that_exists() {
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new("one\ntwo\nthree");
        assert!(np.go_to_line("2"), "line 2 exists");
        assert_eq!(np.edit.caret(), multiedit::Pos { line: 1, col: 0 });
        assert!(!np.go_to_line("9"), "line 9 does not");
        assert_eq!(
            np.edit.caret(),
            multiedit::Pos { line: 1, col: 0 },
            "and the caret stayed"
        );
        assert!(!np.go_to_line("x"), "nor does a non-number");
    }

    fn tr() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    #[test]
    fn find_next_selects_a_match_and_reports_a_miss() {
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new("hello world hello");
        np.last_query = Some((
            "hello".into(),
            multiedit::FindOpts {
                match_case: true,
                down: true,
            },
        ));
        np.edit.place(multiedit::Pos { line: 0, col: 1 }, false);
        assert!(np.find_next());
        assert_eq!(
            np.edit.selection(),
            (
                multiedit::Pos { line: 0, col: 12 },
                multiedit::Pos { line: 0, col: 17 },
            )
        );
        assert!(np.notfound.is_none(), "a hit raises no box");

        np.last_query = Some((
            "zzz".into(),
            multiedit::FindOpts {
                match_case: true,
                down: true,
            },
        ));
        assert!(!np.find_next());
        assert_eq!(
            np.notfound.as_ref().map(|b| b.fields.text.as_str()),
            Some("Cannot find \"zzz\""),
            "a miss names the query"
        );
        assert_eq!(
            np.edit.selection(),
            (
                multiedit::Pos { line: 0, col: 12 },
                multiedit::Pos { line: 0, col: 17 },
            ),
            "a miss does not move the caret"
        );
    }

    #[test]
    fn find_rows_grey_on_an_empty_buffer() {
        let mut np = Notepad::new(None);
        let on = |np: &Notepad, r: usize| match &np.bar.menus[title::EDIT][r] {
            MenuItem::Entry { enabled, .. } => *enabled,
            MenuItem::Separator => panic!("that row is a separator"),
        };
        np.sync_menus();
        assert!(!on(&np, row::FIND), "Find is greyed with no text");
        assert!(!on(&np, row::FIND_NEXT));
        assert!(on(&np, row::REPLACE), "Replace stays live");
        assert!(on(&np, row::GOTO), "so does Go To");
        np.edit = MultiEdit::new("text");
        np.sync_menus();
        assert!(on(&np, row::FIND), "text enables Find");
        assert!(on(&np, row::FIND_NEXT));
    }

    #[test]
    fn edit_rows_grey_until_there_is_a_selection_or_an_edit_to_undo() {
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new("text");
        let mut ctx = test_ctx();
        let on = |np: &Notepad, r: usize| match &np.bar.menus[title::EDIT][r] {
            MenuItem::Entry { enabled, .. } => *enabled,
            MenuItem::Separator => panic!("that row is a separator"),
        };
        let key = |edit| {
            Event::Key(KeyInput {
                sym: 0,
                utf8: None,
                mods: repriseapp::Mods::default(),
                edit: Some(edit),
            })
        };
        np.event(MAIN, Event::Focus(true), &mut ctx);
        for r in [row::UNDO, row::CUT, row::COPY, row::DELETE] {
            assert!(
                !on(&np, r),
                "row {r} is greyed with no selection and no edit"
            );
        }
        assert!(
            !on(&np, row::PASTE),
            "Paste is greyed with no text to paste"
        );
        assert!(
            !np.command(title::EDIT, row::PASTE, &ctx).paste,
            "a greyed Paste asks for nothing"
        );
        repriseapp::fake_clipboard_text(true);
        np.event(MAIN, Event::Focus(true), &mut ctx);
        assert!(
            on(&np, row::PASTE),
            "text on the clipboard makes Paste live"
        );
        assert!(np.command(title::EDIT, row::PASTE, &ctx).paste);
        repriseapp::fake_clipboard_text(false);
        assert_eq!(
            np.command(title::EDIT, row::COPY, &ctx).copy,
            None,
            "a greyed Copy copies nothing"
        );

        np.event(MAIN, key(EditKey::SelectAll), &mut ctx);
        for r in [row::CUT, row::COPY, row::DELETE] {
            assert!(on(&np, r), "row {r} is live over a selection");
        }
        assert!(!on(&np, row::UNDO), "selecting is not an edit");

        np.event(MAIN, key(EditKey::Insert("x".into())), &mut ctx);
        assert!(on(&np, row::UNDO), "typing makes Undo live");
        assert!(!on(&np, row::CUT), "the typed text replaced the selection");
    }

    /// Find's Cancel lights under the pointer and goes cold when the pointer leaves; held and
    /// dragged off, it draws at rest and does not act on the release.
    #[test]
    fn a_find_button_lights_and_a_held_one_dragged_off_does_not_act() {
        use reprisekit::controls::form::Light;
        use reprisekit::pointer::Interaction;
        let mut find = Some(finddlg::find_form(&finddlg::FindFields {
            query: "zzz".into(),
            match_case: false,
            down: true,
        }));
        let (text, px) = (tr(), 11);
        let blue = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
        let f = find.as_ref().unwrap();
        let mut c = Canvas::new(400, 200);
        form::draw(&mut c, &text, &blue.paint(), f, false, px);
        let cancel = f.index_of(finddlg::CANCEL).unwrap();
        let r = f.items[cancel].rect;
        let (x, y) = (r.x + 5, r.y + 5);
        let lit = |f: &Option<Form>| f.as_ref().unwrap().lit(cancel, Light::Body);
        let step = |f: &mut Option<Form>, ev: Event| form_event(f, &ev, &text, px, true);
        let motion = |x, y| Event::Pointer(PointerInput::Motion { x, y });
        assert!(matches!(
            step(&mut find, motion(x, y)),
            FormStep::Out(form::Event::Redraw)
        ));
        assert_eq!(lit(&find), Interaction::Hot);
        let leave = Event::Pointer(PointerInput::Leave);
        assert!(matches!(
            step(&mut find, leave),
            FormStep::Out(form::Event::Redraw)
        ));
        assert_eq!(lit(&find), Interaction::Idle);
        let press = Event::Pointer(PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: repriseapp::Mods::default(),
        });
        step(&mut find, press);
        assert_eq!(lit(&find), Interaction::Pressed);
        assert!(matches!(
            step(&mut find, motion(1, 1)),
            FormStep::Out(form::Event::Redraw)
        ));
        assert_eq!(lit(&find), Interaction::Idle, "dragged off, it is at rest");
        let release = Event::Pointer(PointerInput::Release {
            x: 1,
            y: 1,
            button: BTN_LEFT,
            mods: repriseapp::Mods::default(),
        });
        assert!(matches!(
            step(&mut find, release),
            FormStep::Out(form::Event::Redraw)
        ));
    }

    #[test]
    fn a_selection_drag_held_below_the_edit_scrolls_on_ticks_until_released() {
        let mut ctx = test_ctx();
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new(&"line\n".repeat(200));
        let lay = np.edit_layout(&ctx);
        let x = lay.inner.x + 20;
        np.pointer(
            PointerInput::Press {
                x,
                y: lay.inner.y + 4,
                button: BTN_LEFT,
                time: 0,
                mods: repriseapp::Mods::default(),
            },
            &ctx,
        );
        let below = lay.inner.bottom() + 30;
        let r = np.pointer(PointerInput::Motion { x, y: below }, &ctx);
        assert!(r.tick && r.redraw);
        assert_eq!(np.view.top, 0);
        for top in 1..=3 {
            let r = np.event(MAIN, Event::Tick, &mut ctx);
            assert!(r.tick && r.redraw);
            assert_eq!(np.view.top, top);
        }
        assert_eq!(np.edit.caret().line, 3 + lay.rows - 1);
        np.pointer(
            PointerInput::Release {
                x,
                y: below,
                button: BTN_LEFT,
                mods: repriseapp::Mods::default(),
            },
            &ctx,
        );
        let r = np.event(MAIN, Event::Tick, &mut ctx);
        assert!(!r.tick, "released, the ticks stop");
        assert_eq!(np.view.top, 3);
    }

    #[test]
    fn under_glass_the_menu_bar_is_clear_over_the_frames_glass() {
        use reprisekit::interface::Interface;
        let mut ctx = test_ctx();
        ctx.size = (400, 300);
        let np = Notepad::new(None);
        for (look, rows) in [
            (Interface::Classic, 0),
            (Interface::Blue, 0),
            (Interface::Glass, 1),
        ] {
            ctx.theme = reprisekit::interfaces::theme_for(look);
            let mut c = Canvas::new(400, 300);
            np.draw_main(&mut c, &ctx);
            let bar = np.bar_layout(&ctx).bar;
            let alpha = c.get(390, bar.y + bar.height / 2) >> 24;
            assert_eq!(
                c.take_glass_top(),
                rows * bar.bottom(),
                "{look:?} glass rows"
            );
            assert_eq!(
                alpha,
                0xFF * (1 - rows as u32),
                "{look:?} bar past its titles"
            );
            assert_eq!(
                c.get(200, bar.bottom() + 20) >> 24,
                0xFF,
                "{look:?} the text box"
            );
        }
    }

    #[test]
    fn the_text_boxs_scrollbar_lights_under_the_pointer_and_a_leave_clears_it() {
        use reprisekit::pointer::Interaction;
        let mut ctx = test_ctx();
        ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
        let mut np = Notepad::new(None);
        np.metrics.follow_look(&ctx.theme.look.controls());
        np.edit = MultiEdit::new(&"line\n".repeat(200));
        let sb = np
            .edit_layout(&ctx)
            .vscroll
            .expect("200 lines overflow the box");
        let (x, y) = (sb.x + 8, sb.y + 2);
        let up = (Axis::Vertical, ScrollHit::Back);
        assert!(np.pointer(PointerInput::Motion { x, y }, &ctx).redraw);
        assert_eq!(np.view.bars.of(up), Interaction::Hot);
        let press = PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: repriseapp::Mods::default(),
        };
        np.pointer(press, &ctx);
        assert_eq!(np.view.bars.of(up), Interaction::Pressed);
        let release = PointerInput::Release {
            x,
            y,
            button: BTN_LEFT,
            mods: repriseapp::Mods::default(),
        };
        assert!(np.pointer(release, &ctx).redraw);
        assert_eq!(np.view.bars.of(up), Interaction::Hot);
        assert!(np.pointer(PointerInput::Leave, &ctx).redraw);
        assert_eq!(np.view.bars.lit(), None);
    }

    #[test]
    fn the_text_boxs_thumb_drags_and_its_track_pages() {
        let ctx = test_ctx();
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new(&"line\n".repeat(200));
        let el = np.edit_layout(&ctx);
        let sb = el.vscroll.expect("200 lines overflow the box");
        let last = np.edit.lines().len() - el.rows;
        let press = |x, y| PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: repriseapp::Mods::default(),
        };
        let release = PointerInput::Release {
            x: 0,
            y: 0,
            button: BTN_LEFT,
            mods: repriseapp::Mods::default(),
        };
        np.pointer(press(sb.x + 8, sb.bottom() - 20), &ctx);
        assert_eq!(np.view.top, el.rows, "the track below the thumb: a page");
        np.pointer(release, &ctx);
        let thumb_y = |np: &Notepad| {
            let el = np.edit_layout(&ctx);
            (sb.y..sb.bottom())
                .find(|&y| {
                    matches!(
                        multiedit::bar_hit(
                            &np.mono,
                            &np.metrics,
                            &np.edit,
                            &el,
                            np.view,
                            sb.x + 8,
                            y
                        ),
                        Some((_, widgets::BarHit::Thumb(_)))
                    )
                })
                .expect("a thumb")
        };
        let y = thumb_y(&np);
        np.pointer(press(sb.x + 8, y), &ctx);
        assert_eq!(np.view.top, el.rows, "the press on the thumb moves nothing");
        np.pointer(PointerInput::Motion { x: 0, y: 5000 }, &ctx);
        assert_eq!(np.view.top, last, "dragged past the end, off the bar");
        np.pointer(release, &ctx);
        np.pointer(PointerInput::Motion { x: sb.x + 8, y }, &ctx);
        assert_eq!(np.view.top, last, "let go: the motion moves nothing");
    }

    #[test]
    fn the_not_found_box_is_its_own_child() {
        let mut np = Notepad::new(None);
        np.find = Some(finddlg::find_form(&finddlg::FindFields {
            query: "zzz".into(),
            match_case: false,
            down: true,
        }));
        np.notfound = Some(np.notfound_box("zzz"));
        let ids: Vec<_> = np.children().iter().map(|c| c.id).collect();
        assert!(ids.contains(&FIND));
        assert!(ids.contains(&NOTFOUND));
        let box_child = np
            .children()
            .into_iter()
            .find(|c| c.id == NOTFOUND)
            .expect("the box");
        assert!(box_child.modal, "the box is modal");
        let find_child = np
            .children()
            .into_iter()
            .find(|c| c.id == FIND)
            .expect("find");
        assert!(!find_child.modal, "Find is modeless");
    }

    #[test]
    fn replace_swaps_the_match_then_advances() {
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new("a a a");
        np.last_query = Some((
            "a".into(),
            multiedit::FindOpts {
                match_case: true,
                down: true,
            },
        ));
        assert!(np.find_next(), "select the first match");
        np.replace_once("a", "b", true);
        assert_eq!(np.edit.text(), "b a a");

        // Select "a" directly and ask to replace "z": the selection does
        // not match the query, so the guard must refuse to swap it.
        np.edit.place(multiedit::Pos { line: 0, col: 2 }, false);
        np.edit.place(multiedit::Pos { line: 0, col: 3 }, true);
        assert_eq!(np.edit.selected_text(), "a");
        np.replace_once("z", "x", true);
        assert_eq!(
            np.edit.text(),
            "b a a",
            "a query that misses the selection replaces nothing"
        );
    }

    /// With Match Case off, the search can select text differing in case from the query; Replace
    /// must still recognise it as the match rather than silently skipping past it.
    #[test]
    fn replace_once_ignores_case_when_match_case_is_off() {
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new("Cat cat");
        np.last_query = Some((
            "cat".into(),
            multiedit::FindOpts {
                match_case: false,
                down: true,
            },
        ));
        assert!(np.find_next(), "case-insensitively selects \"Cat\"");
        assert_eq!(np.edit.selected_text(), "Cat");
        np.replace_once("cat", "dog", false);
        assert_eq!(np.edit.text(), "dog cat");
    }

    /// Replace All rewrites the document and counts, and never opens the not-found box for a query
    /// it did replace.
    #[test]
    fn replace_all_rewrites_the_document() {
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new("a a\na");
        let n = np.replace_every(
            "a",
            "bb",
            &multiedit::FindOpts {
                match_case: true,
                down: true,
            },
        );
        assert_eq!(n, 3);
        assert_eq!(np.edit.text(), "bb bb\nbb");
        assert!(np.notfound.is_none());
    }

    #[test]
    fn replace_and_find_do_not_share_the_screen() {
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new("text");
        np.open_find();
        assert!(np.find.is_some());
        np.open_replace();
        assert!(np.replace.is_some());
        assert!(np.find.is_none(), "Find gave up the slot");
        np.open_find();
        assert!(np.replace.is_none(), "and takes it back");
    }

    #[test]
    fn about_shows_notepads_product_row_and_icon() {
        let mut np = Notepad::new(None);
        np.app_icon = Some(Canvas::new(32, 32));
        let f = np.about_fields();
        assert_eq!(f.product, "Notepad");
        assert!(f.icon.is_some(), "Notepad fills the icon slot");
        assert!(
            !f.memory_label.is_empty(),
            "the system rows are filled, not blank"
        );
        let form = aboutbox::form(&f, &tr(), 11);
        assert!(form.item("icon").is_some());
    }

    #[test]
    fn about_opens_as_a_fixed_modal_child() {
        let mut np = Notepad::new(None);
        np.about = true;
        let kid = np
            .children()
            .into_iter()
            .find(|c| c.id == ABOUT)
            .expect("the about child");
        assert_eq!(kid.size, aboutbox::CLIENT);
        assert!(kid.fixed && kid.modal);
    }

    #[test]
    fn dialogs_grow_with_the_text_size() {
        let mut np = Notepad::new(None);
        let mut ctx = test_ctx();
        ctx.text_size = 150;
        np.follow(&ctx);
        assert_eq!(np.forms.fonts.size_px, 17);
        assert_eq!(np.forms.text, reprisekit::theme::TextUnits::new(17, 11));
        np.open_find();
        np.goto = Some(goto_form("1"));
        let size = |id| np.children().into_iter().find(|c| c.id == id).unwrap().size;
        assert_eq!(size(FIND), (547, 158));
        assert_eq!(size(GOTO), (414, 147));
    }

    #[test]
    fn the_window_opens_at_the_text_size() {
        let mut np = Notepad::new(None);
        assert_eq!(np.initial_size(), (568, 328));
        let mut ctx = test_ctx();
        ctx.text_size = 200;
        np.follow(&ctx);
        assert_eq!(np.initial_size(), (1136, 656));
    }

    /// At 200% text the window's theme is at the text size: the menu bar and the status bar
    /// grow, the text box takes what is left, and Open opens at its grown size.
    #[test]
    fn the_bars_and_the_file_dialog_grow_with_the_text_size() {
        let mut np = Notepad::new(None);
        let mut ctx = test_ctx();
        ctx.text_size = 200;
        ctx.theme = Theme::default().with_text_size(200);
        np.follow(&ctx);
        np.status_bar = true;
        let r = np.edit_rect(&ctx);
        assert_eq!((r.y, r.bottom()), (40, 328 - 44));
        let (bar, sr) = np.status(&ctx).expect("the status bar is on");
        assert_eq!(sr, Rect::new(0, 328 - 44, 568, 44));
        let panels = statusbar::layout(&bar, sr, ctx.theme.text);
        assert_eq!(panels[1].width, 712);
        np.open_dialog();
        let kid = np.children().into_iter().find(|c| c.id == OPEN).unwrap();
        assert_eq!(kid.size, (2 * CLIENT.0, 2 * CLIENT.1));
    }

    #[test]
    fn typing_marks_the_document_dirty() {
        let mut np = Notepad::new(None);
        assert!(!np.dirty, "a fresh document is clean");
        np.edit = MultiEdit::new("x");
        np.dirty = true;
        let tmp = tempfile::tempdir().expect("tempdir");
        np.path = Some(tmp.path().join("dirty.txt"));
        np.save();
        assert!(!np.dirty, "a successful save cleans it");
    }

    #[test]
    fn new_on_a_dirty_document_prompts() {
        let mut np = Notepad::new(None);
        np.dirty = true;
        let kids_before = np.children().len();
        np.pending = Pending::waiting(PendingAction::New);
        np.savechanges = true;
        assert_eq!(np.children().len(), kids_before + 1);
        assert_eq!(np.children().last().expect("the prompt").id, SAVECHANGES);
    }

    #[test]
    fn no_discards_and_runs_the_pending_action() {
        let mut np = Notepad::new(None);
        np.dirty = true;
        np.pending = Pending::waiting(PendingAction::Quit);
        np.savechanges = true;
        let reply = np.savechanges_outcome(form::Event::Pressed(msgbox::button_id(1)));
        assert!(reply.quit);
        assert!(!np.pending.is_waiting());
    }

    #[test]
    fn arrows_move_the_save_prompt_s_focus_and_enter_presses_it() {
        let mut np = Notepad::new(None);
        np.dirty = true;
        np.confirm_then(PendingAction::Quit);
        let key = |sym, edit| KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit,
        };
        let right = || {
            Some(EditKey::Right {
                select: false,
                word: false,
            })
        };
        let mut ctx = test_ctx();
        np.event(
            SAVECHANGES,
            Event::Key(key(keysym::RIGHT, right())),
            &mut ctx,
        );
        assert_eq!(np.savechanges_focus, 1, "Right moves from Yes to No");
        np.event(
            SAVECHANGES,
            Event::Key(key(keysym::RIGHT, right())),
            &mut ctx,
        );
        np.event(
            SAVECHANGES,
            Event::Key(key(keysym::RIGHT, right())),
            &mut ctx,
        );
        assert_eq!(np.savechanges_focus, 2, "Right stops at Cancel");
        np.event(SAVECHANGES, Event::Key(key(keysym::RETURN, None)), &mut ctx);
        assert!(!np.savechanges, "Enter on Cancel closed the prompt");
        assert!(np.dirty, "Cancel kept the changes");
        assert!(!np.pending.is_waiting(), "Cancel abandoned the quit");
    }

    #[test]
    fn cancel_on_the_prompt_abandons_the_action() {
        let mut np = Notepad::new(None);
        np.dirty = true;
        np.pending = Pending::waiting(PendingAction::Quit);
        np.savechanges = true;
        let reply = np.savechanges_outcome(form::Event::Pressed(msgbox::button_id(2)));
        assert!(!reply.quit);
        assert!(np.dirty, "still dirty");
        assert!(!np.pending.is_waiting());
    }

    #[test]
    fn a_clean_document_exits_without_a_prompt() {
        let mut np = Notepad::new(None);
        assert!(!np.dirty);
        let reply = np.confirm_then(PendingAction::Quit);
        assert!(reply.quit, "a clean document quits immediately");
        assert!(np.children().is_empty(), "no prompt opens");
    }

    pub(crate) fn test_ctx() -> Ctx {
        Ctx {
            theme: Theme::default(),
            text: tr().into(),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: (568, 328),
            maximized: false,
            origin: (0, 0),
            wake: repriseapp::Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            at_pick: false,
            text_size: 100,
        }
    }

    #[test]
    fn a_typed_path_into_a_missing_folder_shows_a_box_and_keeps_the_dialog() {
        let dir = std::env::temp_dir();
        for save in [false, true] {
            let mut np = Notepad::new(None);
            np.host.folder = Some(np.host.folders.id(&dir));
            if save {
                np.saveas = Some(sample_filedlg());
                np.saveas_accept("reprise-no-such-dir/x.txt", &mut test_icons());
                assert!(np.saveas.is_some(), "Save As stays open");
            } else {
                np.open = Some(sample_filedlg());
                np.open_accept("reprise-no-such-dir/x.txt", &mut test_icons());
                assert!(np.open.is_some(), "Open stays open");
            }
            assert!(np.error.is_some(), "a box says the folder is missing");
        }
    }

    fn test_icons() -> IconCache {
        IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]))
    }

    /// A `FileDlg` in the state `open_outcome`/`saveas_outcome` expect to find one in mid-flow:
    /// unlisted (no test here needs a listing) but otherwise exactly what
    /// `open_dialog`/`saveas_dialog` build.
    fn sample_filedlg() -> FileDlg {
        FileDlg::new(
            Fields {
                title: "Open".into(),
                accept_label: "Open".into(),
                mode: DlgMode::Open,
                multiple: false,
                filters: vec!["Text Documents (*.txt)".into(), "All Files (*.*)".into()],
            },
            places(),
        )
    }

    #[test]
    fn a_crlf_file_is_saved_with_its_breaks() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("crlf.txt");
        std::fs::write(&path, "one\r\ntwo\r\n").expect("write");
        let mut np = Notepad::new(Some(path.clone()));
        assert_eq!(np.edit.text(), "one\ntwo\n", "the edit itself holds LF");
        assert!(np.save());
        assert_eq!(std::fs::read(&path).expect("read"), b"one\r\ntwo\r\n");

        let lf = tmp.path().join("lf.txt");
        std::fs::write(&lf, "one\ntwo\n").expect("write");
        let mut np = Notepad::new(Some(lf.clone()));
        assert!(np.save());
        assert_eq!(std::fs::read(&lf).expect("read"), b"one\ntwo\n");

        let mixed = tmp.path().join("mixed.txt");
        std::fs::write(&mixed, "one\r\ntwo\nthree").expect("write");
        let mut np = Notepad::new(Some(mixed.clone()));
        assert!(np.save());
        assert_eq!(
            std::fs::read(&mixed).expect("read"),
            b"one\r\ntwo\r\nthree",
            "a mixed file saves CRLF throughout"
        );
    }

    #[test]
    fn write_document_keeps_mode_and_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let real = dir.join("real.txt");
        std::fs::write(&real, "old").expect("write");
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        write_document(&real, &["new"], "\n").expect("save");
        assert_eq!(std::fs::read_to_string(&real).expect("read"), "new");
        assert_eq!(
            std::fs::metadata(&real).expect("stat").permissions().mode() & 0o777,
            0o600
        );

        let link = dir.join("link.txt");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        write_document(&link, &["via link"], "\n").expect("save");
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("lstat")
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&real).expect("read"), "via link");
        assert!(std::fs::read_dir(dir).expect("dir").flatten().all(|e| {
            !e.file_name()
                .to_string_lossy()
                .starts_with(".reprise-notepad-")
        }));
    }

    /// A save through a symlink renames a new file over the link's target, so the link stays and a
    /// failed write never truncates the target.
    #[test]
    fn write_document_replaces_a_symlink_target_and_writes_hard_links_in_place() {
        use std::os::unix::fs::MetadataExt;
        let dir =
            std::env::temp_dir().join(format!("reprise-notepad-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("store")).expect("mkdir");
        let real = dir.join("store/doc.txt");
        let link = dir.join("doc.txt");
        std::fs::write(&real, "old").expect("write");
        std::os::unix::fs::symlink("store/doc.txt", &link).expect("symlink");
        let before = std::fs::metadata(&real).expect("stat").ino();

        write_document(&link, &["new"], "\n").expect("save");

        let lstat = std::fs::symlink_metadata(&link).expect("lstat");
        assert!(lstat.file_type().is_symlink(), "the link stays a symlink");
        assert_eq!(std::fs::read_to_string(&real).expect("read"), "new");
        assert_ne!(
            std::fs::metadata(&real).expect("stat").ino(),
            before,
            "the target is a new inode renamed over the old one"
        );

        let other = dir.join("store/other.txt");
        std::fs::hard_link(&real, &other).expect("hard link");
        let before = std::fs::metadata(&real).expect("stat").ino();
        write_document(&other, &["both"], "\n").expect("save");
        assert_eq!(std::fs::metadata(&real).expect("stat").ino(), before);
        assert_eq!(std::fs::read_to_string(&real).expect("read"), "both");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_document_keeps_xattrs() {
        let dir =
            std::env::temp_dir().join(format!("reprise-notepad-xattr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let doc = dir.join("doc.txt");
        std::fs::write(&doc, "old").expect("write");
        // A file system without `user.` xattrs has nothing to keep.
        if reprisefs::apps::set_trusted(&doc).is_err() {
            std::fs::remove_dir_all(&dir).ok();
            return;
        }

        write_document(&doc, &["new"], "\n").expect("save");

        assert_eq!(std::fs::read_to_string(&doc).expect("read"), "new");
        assert!(reprisefs::apps::is_trusted(&doc), "user.* xattr kept");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A symlink planted at a temp-file name beside the document is never followed: the file it
    /// points at keeps its bytes.
    #[test]
    fn write_document_leaves_a_planted_temp_symlink_alone() {
        let dir =
            std::env::temp_dir().join(format!("reprise-notepad-plant-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let doc = dir.join("doc.txt");
        let victim = dir.join("victim.txt");
        std::fs::write(&doc, "old").expect("write");
        std::fs::write(&victim, "keep").expect("write");
        let planted = dir.join(format!(".doc.txt.{}.tmp", std::process::id()));
        std::os::unix::fs::symlink(&victim, &planted).expect("symlink");

        write_document(&doc, &["new"], "\n").expect("save");

        assert_eq!(std::fs::read_to_string(&doc).expect("read"), "new");
        assert_eq!(std::fs::read_to_string(&victim).expect("read"), "keep");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_document_refuses_a_read_only_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("reprise-notepad-ro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let doc = dir.join("doc.txt");
        std::fs::write(&doc, "old").expect("write");
        std::fs::set_permissions(&doc, std::fs::Permissions::from_mode(0o444)).expect("chmod");
        // Root writes through any mode, so there is nothing to check.
        if std::fs::OpenOptions::new().write(true).open(&doc).is_ok() {
            std::fs::remove_dir_all(&dir).ok();
            return;
        }

        let err = write_document(&doc, &["new"], "\n").expect_err("save must fail");

        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read_to_string(&doc).expect("read"), "old");
        assert_eq!(std::fs::read_dir(&dir).expect("dir").count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Yes on the save prompt with a path that cannot be written raises the error box and forgets
    /// the action, so Exit does not sit armed.
    #[test]
    fn failed_save_on_the_prompt_raises_the_box_and_drops_the_action() {
        let mut np = Notepad::new(None);
        np.path = Some(std::env::temp_dir().join("reprise-no-such-dir/x.txt"));
        np.dirty = true;
        np.pending = Pending::waiting(PendingAction::Quit);
        np.savechanges = true;
        let reply = np.savechanges_yes();
        assert!(!reply.quit);
        assert!(!np.pending.is_waiting());
        assert!(np.dirty);
        assert!(
            np.error
                .as_ref()
                .is_some_and(|b| b.fields.text.starts_with("Cannot create the "))
        );
        assert_eq!(np.children().last().expect("the box").id, ERRORBOX);
    }

    #[test]
    fn a_file_that_is_not_utf8_opens_as_an_untitled_copy() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let latin1 = dir.join("latin1.txt");
        std::fs::write(&latin1, b"caf\xe9").expect("write");
        let mut np = Notepad::new(Some(latin1.clone()));
        assert_eq!(np.path, None);
        assert_eq!(np.edit.text(), "");
        assert_eq!(np.children().last().expect("the prompt").id, LOSSY);
        let enter = KeyInput {
            sym: keysym::RETURN,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: None,
        };
        let mut ctx = test_ctx();
        np.event(LOSSY, Event::Key(enter), &mut ctx);
        assert!(np.lossy.is_none(), "Yes closed the prompt");
        assert_eq!(np.edit.text(), "caf\u{fffd}");
        assert_eq!(np.path, None);
        assert_eq!(std::fs::read(&latin1).expect("read"), b"caf\xe9");

        let missing = dir.join("missing.txt");
        let np = Notepad::new(Some(missing.clone()));
        assert_eq!(np.path, Some(missing));
    }

    #[test]
    fn a_device_or_folder_on_the_command_line_raises_the_box() {
        for p in ["/dev/zero", "/"] {
            let np = Notepad::new(Some(PathBuf::from(p)));
            assert_eq!(np.path, None);
            assert_eq!(np.edit.text(), "");
            let text = &np.error.as_ref().expect("the box").fields.text;
            assert_eq!(text, &format!("Cannot open {p}.\nIt is not a file."));
        }
    }

    /// A name the Open dialog cannot read raises the box over the dialog, which stays up with the
    /// document untouched.
    #[test]
    fn a_failed_open_from_the_dialog_raises_the_box_over_it() {
        let mut np = Notepad::new(None);
        np.open_dialog();
        let dir = std::env::temp_dir();
        np.host.folder = Some(np.host.folders.id(&dir));
        let missing = dir.join("reprise-notepad-open-missing.txt");
        std::fs::remove_file(&missing).ok();
        np.open_accept("reprise-notepad-open-missing.txt", &mut test_icons());
        assert!(np.open.is_some(), "the dialog stays up");
        assert_eq!(np.path, None);
        assert_eq!(
            np.error.as_ref().expect("the box").fields.text,
            format!(
                "{}\nFile not found.\nPlease verify the correct file name was given.",
                missing.display()
            )
        );
    }

    #[test]
    fn a_listed_subdirectory_carries_a_resolvable_id() {
        let mut np = Notepad::new(None);
        let dir = std::env::temp_dir();
        let real = np.host.folders.id(&dir);
        assert_eq!(np.host.folders.path(real), Some(dir.as_path()));
        let child = np.host.folders.id(&dir.join("child"));
        assert_ne!(child, real, "a second path gets a handle of its own");
        assert_eq!(
            np.host.folders.path(child),
            Some(dir.join("child").as_path())
        );
    }

    /// File > Save on a never-saved document opens Save As.
    #[test]
    fn save_on_an_untitled_document_opens_save_as() {
        let ctx = test_ctx();
        let mut np = Notepad::new(None);
        assert!(np.path.is_none());
        np.command(title::FILE, row::SAVE, &ctx);
        assert!(np.saveas.is_some(), "Save As opened");
    }

    #[test]
    fn the_handle_table_resolves_folder_ids() {
        let mut np = Notepad::new(None);
        let home = std::env::temp_dir();
        let id = np.host.folders.id(&home);
        assert_eq!(np.host.folders.path(id), Some(home.as_path()));
        assert_eq!(
            np.host.folders.id(&home),
            id,
            "the same path gets the same handle"
        );
    }

    /// Opening the dialog declares the child window.
    #[test]
    fn open_declares_its_child_window() {
        let mut np = Notepad::new(None);
        assert!(np.children().is_empty());
        np.open = Some(sample_filedlg());
        let kids = np.children();
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].id, OPEN);
        assert_eq!(kids[0].size, reprisekit::controls::filedlg::CLIENT);
    }

    #[test]
    fn save_as_writes_adopts_the_path_and_runs_the_pending_action() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new("hello");
        np.dirty = true;
        np.pending = Pending::waiting(PendingAction::Quit);
        np.saveas = Some(sample_filedlg());
        let id = np.host.folders.id(dir);
        np.host.folder = Some(id);
        let reply = np.saveas_outcome(
            reprisekit::controls::filedlg::Event::Accept(vec!["out.txt".into()]),
            &mut test_icons(),
        );
        let written = dir.join("out.txt");
        assert_eq!(std::fs::read_to_string(&written).expect("written"), "hello");
        assert_eq!(np.path.as_deref(), Some(written.as_path()));
        assert!(!np.dirty);
        assert!(reply.quit, "the pending quit runs once the save succeeds");
    }

    #[test]
    fn save_as_onto_an_existing_file_asks_before_replacing() {
        let dir = std::env::temp_dir();
        let taken = dir.join(format!("reprise-replace-{}.txt", std::process::id()));
        std::fs::write(&taken, "old").expect("write");
        let name = taken.file_name().unwrap().to_string_lossy().into_owned();
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new("new");
        np.dirty = true;
        np.saveas = Some(sample_filedlg());
        let id = np.host.folders.id(&dir);
        np.host.folder = Some(id);
        np.saveas_outcome(
            reprisekit::controls::filedlg::Event::Accept(vec![name.clone()]),
            &mut test_icons(),
        );
        assert_eq!(np.overwrite.as_deref(), Some(taken.as_path()));
        assert_eq!(
            std::fs::read_to_string(&taken).unwrap(),
            "old",
            "nothing written yet"
        );
        assert!(np.saveas.is_some(), "Save As stays open behind the prompt");
        assert_eq!(np.children().last().expect("the prompt").id, OVERWRITE);
        let f = np.overwrite_fields(&taken);
        assert_eq!(f.buttons, ["Yes", "No"]);
        assert_eq!(f.default, 1, "defaults to No");

        np.overwrite_outcome(form::Event::Default);
        assert!(np.overwrite.is_none());
        assert_eq!(std::fs::read_to_string(&taken).unwrap(), "old");
        assert!(np.saveas.is_some());

        np.saveas_outcome(
            reprisekit::controls::filedlg::Event::Accept(vec![name]),
            &mut test_icons(),
        );
        np.overwrite_outcome(form::Event::Pressed(msgbox::button_id(0)));
        assert_eq!(std::fs::read_to_string(&taken).unwrap(), "new");
        assert_eq!(np.path.as_deref(), Some(taken.as_path()));
        assert!(np.overwrite.is_none());
        assert!(np.saveas.is_none());
        let _ = std::fs::remove_file(&taken);
    }

    #[test]
    fn an_arrow_moves_the_replace_prompt_s_focus_and_enter_presses_it() {
        let dir = std::env::temp_dir();
        let taken = dir.join(format!("reprise-replace-keys-{}.txt", std::process::id()));
        std::fs::write(&taken, "old").expect("write");
        let name = taken.file_name().unwrap().to_string_lossy().into_owned();
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new("new");
        np.saveas = Some(sample_filedlg());
        let id = np.host.folders.id(&dir);
        np.host.folder = Some(id);
        np.saveas_outcome(
            reprisekit::controls::filedlg::Event::Accept(vec![name]),
            &mut test_icons(),
        );
        let key = |sym, edit| {
            Event::Key(KeyInput {
                sym,
                utf8: None,
                mods: repriseapp::Mods::default(),
                edit,
            })
        };
        let left = || {
            Some(EditKey::Left {
                select: false,
                word: false,
            })
        };
        let mut ctx = test_ctx();
        np.event(OVERWRITE, key(keysym::LEFT, left()), &mut ctx);
        assert_eq!(np.overwrite_focus, 0, "Left moves from No to Yes");
        np.event(OVERWRITE, key(keysym::LEFT, left()), &mut ctx);
        assert_eq!(np.overwrite_focus, 0, "Left stops at Yes");
        np.event(OVERWRITE, key(keysym::RETURN, None), &mut ctx);
        assert!(np.overwrite.is_none(), "Enter closed the prompt");
        assert_eq!(
            std::fs::read_to_string(&taken).unwrap(),
            "new",
            "Enter on Yes wrote over the file"
        );
        let _ = std::fs::remove_file(&taken);
    }

    #[test]
    fn save_as_onto_a_dangling_symlink_asks_before_replacing() {
        let dir = std::env::temp_dir();
        let link = dir.join(format!("reprise-dangling-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(dir.join("reprise-no-such-target"), &link).expect("symlink");
        let name = link.file_name().unwrap().to_string_lossy().into_owned();
        let mut np = Notepad::new(None);
        np.saveas = Some(sample_filedlg());
        np.host.folder = Some(np.host.folders.id(&dir));
        np.saveas_outcome(
            reprisekit::controls::filedlg::Event::Accept(vec![name]),
            &mut test_icons(),
        );
        assert_eq!(np.overwrite.as_deref(), Some(link.as_path()));
        let _ = std::fs::remove_file(&link);
    }

    #[test]
    fn open_replaces_the_buffer() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let p = dir.join("reprise-open-test.txt");
        std::fs::write(&p, "loaded").expect("write");
        let mut np = Notepad::new(None);
        np.open = Some(sample_filedlg());
        let id = np.host.folders.id(dir);
        np.host.folder = Some(id);
        np.open_outcome(
            reprisekit::controls::filedlg::Event::Accept(vec!["reprise-open-test.txt".into()]),
            &mut test_icons(),
        );
        assert_eq!(np.edit.text(), "loaded");
        assert_eq!(np.path.as_deref(), Some(p.as_path()));
        assert!(!np.dirty);
    }

    #[test]
    fn cancelling_save_as_abandons_the_pending_quit() {
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new("hello");
        np.dirty = true;
        np.confirm_then(PendingAction::Quit);
        assert!(np.savechanges, "the save prompt opened first");
        let reply = np.savechanges_outcome(form::Event::Default);
        assert!(np.saveas.is_some(), "an untitled document opens Save As");
        assert!(np.pending.is_waiting(), "still pending, waiting on Save As");
        assert!(!reply.quit);
        let reply = np.saveas_outcome(
            reprisekit::controls::filedlg::Event::Cancel,
            &mut test_icons(),
        );
        assert!(np.saveas.is_none());
        assert!(
            !np.pending.is_waiting(),
            "cancelling abandons the pending quit"
        );
        assert!(!reply.quit);
    }

    #[test]
    fn entering_a_listed_subfolder_lists_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let sub = dir.join("Sub");
        std::fs::create_dir_all(&sub).expect("mkdir");
        std::fs::write(sub.join("inside.txt"), "from the subfolder").expect("write");

        let mut np = Notepad::new(None);
        np.path = Some(dir.join("placeholder.txt"));
        np.open_dialog();
        let mut ctx = test_ctx();
        np.open_event(Event::Blink, &mut ctx);
        let parent_id = np.host.folder.expect("start_dir was listed");
        let sub_id = np.host.folders.id(&sub);
        assert_ne!(sub_id, parent_id);

        let enter = KeyInput {
            sym: keysym::RETURN,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: None,
        };
        np.open_event(Event::Key(enter.clone()), &mut ctx);
        assert_eq!(
            np.open.as_ref().expect("still open").folder(),
            Some(sub_id),
            "the dialog now believes it is listing the subfolder"
        );

        np.open_event(Event::Key(enter), &mut ctx);
        assert_eq!(np.edit.text(), "from the subfolder");
        assert_eq!(np.path.as_deref(), Some(sub.join("inside.txt").as_path()));
    }

    fn key(sym: u32) -> Event {
        Event::Key(KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: None,
        })
    }

    #[test]
    fn the_file_type_filters_open_and_save_as() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        std::fs::create_dir(dir.join("Sub")).expect("mkdir");
        for f in ["a.txt", "bash", "b.conf"] {
            std::fs::write(dir.join(f), "").expect("write");
        }
        for save in [false, true] {
            let mut np = Notepad::new(None);
            np.path = Some(dir.join("a.txt"));
            let mut ctx = test_ctx();
            let mut send = |np: &mut Notepad, ev: Event| {
                if save {
                    np.saveas_event(ev, &mut ctx);
                } else {
                    np.open_event(ev, &mut ctx);
                }
            };
            if save {
                np.saveas_dialog();
            } else {
                np.open_dialog();
            }
            let names = |np: &Notepad| {
                let dlg = if save { &np.saveas } else { &np.open };
                let mut n: Vec<String> = dlg
                    .as_ref()
                    .expect("open")
                    .entries()
                    .iter()
                    .map(|e| e.name.clone())
                    .collect();
                n.sort();
                n
            };
            send(&mut np, Event::Blink);
            assert_eq!(
                names(&np),
                ["Sub", "a.txt"],
                "Text Documents (save: {save})"
            );
            send(&mut np, key(keysym::TAB));
            send(&mut np, key(keysym::DOWN));
            assert_eq!(
                names(&np),
                ["Sub", "a.txt", "b.conf", "bash"],
                "All Files (save: {save})"
            );
        }
    }

    #[test]
    fn a_typed_wildcard_filters_the_open_dialog() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        for f in ["a.txt", "bash"] {
            std::fs::write(dir.join(f), "").expect("write");
        }
        let mut np = Notepad::new(None);
        np.path = Some(dir.join("a.txt"));
        np.open_dialog();
        let mut ctx = test_ctx();
        np.open_event(Event::Blink, &mut ctx);
        np.open_event(
            Event::Key(KeyInput {
                sym: '*' as u32,
                utf8: Some("*".into()),
                mods: repriseapp::Mods::default(),
                edit: Some(EditKey::Insert("*".into())),
            }),
            &mut ctx,
        );
        np.open_event(key(keysym::RETURN), &mut ctx);
        assert!(np.error.is_none(), "no box");
        let dlg = np.open.as_ref().expect("the dialog stays open");
        assert_eq!(dlg.file_name(), "*");
        let mut names: Vec<&str> = dlg.entries().iter().map(|e| e.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["a.txt", "bash"]);
    }

    #[test]
    fn the_query_field_keeps_its_caret_between_keystrokes() {
        let mut np = Notepad::new(None);
        let ctx = test_ctx();
        np.open_find();
        let insert = |ch: char| form::Key::Edit(EditKey::Insert(ch.to_string()));
        let left = form::Key::Edit(EditKey::Left {
            select: false,
            word: false,
        });

        for ch in "helo".chars() {
            let out = np.find.as_mut().expect("find is open").key(insert(ch));
            np.find_outcome(out, &ctx);
        }
        assert_eq!(
            Notepad::field(np.find.as_ref().unwrap(), finddlg::QUERY),
            "helo"
        );

        let out = np.find.as_mut().unwrap().key(left);
        np.find_outcome(out, &ctx);

        let out = np.find.as_mut().unwrap().key(insert('l'));
        np.find_outcome(out, &ctx);
        assert_eq!(
            Notepad::field(np.find.as_ref().unwrap(), finddlg::QUERY),
            "hello",
            "the \"l\" lands where the caret sits"
        );
    }

    #[test]
    fn ctrl_f_is_refused_on_an_empty_buffer_but_opens_find_on_text() {
        let mut np = Notepad::new(None);
        let ctx = test_ctx();
        let ctrl_f = KeyInput {
            sym: 0x66,
            utf8: Some("f".into()),
            mods: repriseapp::Mods {
                ctrl: true,
                shift: false,
                alt: false,
            },
            edit: None,
        };
        np.accel(&ctrl_f, &ctx);
        assert!(
            np.find.is_none(),
            "Find is greyed on an empty buffer; Ctrl+F must not open it"
        );

        np.edit = MultiEdit::new("text");
        np.sync_menus();
        np.accel(&ctrl_f, &ctx);
        assert!(np.find.is_some(), "text enables Find; Ctrl+F opens it");
    }

    #[test]
    fn replace_that_matches_nothing_does_not_scroll() {
        let ctx = test_ctx();
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new(&"line\n".repeat(200));
        np.view.top = 50;
        np.replace = Some(finddlg::replace_form(&finddlg::ReplaceFields {
            query: "zzz".into(),
            with: "x".into(),
            match_case: true,
        }));
        np.replace_outcome(form::Event::Pressed(finddlg::REPLACE), &ctx);
        assert_eq!(np.view.top, 50, "nothing matched, nothing scrolled");
    }
}
