//! The menu bar's rows, what each does, and the edit box's context menu.

use super::*;

fn mi(label: &str, shortcut: Option<&str>, enabled: bool) -> MenuItem {
    MenuItem::item(label)
        .enabled(enabled)
        .shortcut(shortcut.map(str::to_string))
}

pub(crate) mod title {
    pub const FILE: usize = 0;
    pub const EDIT: usize = 1;
    pub const VIEW: usize = 3;
    pub const HELP: usize = 4;
}

pub(crate) mod row {
    pub const NEW: usize = 0;
    pub const OPEN: usize = 1;
    pub const SAVE: usize = 2;
    pub const SAVE_AS: usize = 3;
    pub const EXIT: usize = 8;
    pub const UNDO: usize = 0;
    pub const REDO: usize = 1;
    pub const CUT: usize = 3;
    pub const COPY: usize = 4;
    pub const PASTE: usize = 5;
    pub const DELETE: usize = 6;
    pub const FIND: usize = 8;
    pub const FIND_NEXT: usize = 9;
    pub const REPLACE: usize = 10;
    pub const GOTO: usize = 11;
    pub const SELECT_ALL: usize = 13;
    pub const TIME_DATE: usize = 14;
    pub const STATUS_BAR: usize = 0;
    pub const ABOUT: usize = 2;
}

pub(crate) fn menus() -> Vec<(&'static str, Vec<MenuItem>)> {
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
                mi("&Redo", Some("Ctrl+Y"), true),
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

impl Notepad {
    /// An accelerator reaches here without the menu, so a greyed row refuses it too.
    pub(crate) fn command(&mut self, title: usize, row: usize, ctx: &Ctx) -> Reply {
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
            (title::EDIT, row::REDO) => {
                self.edit_key(MultiKey::Edit(EditKey::Redo), ctx);
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
                    self.scroll_to_caret(ctx);
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

    fn menu_ctx<'a>(&self, ctx: &'a Ctx) -> MenuCtx<'a> {
        MenuCtx {
            text: &ctx.text,
            theme: &ctx.theme,
            px: ctx.theme.fonts.size_px,
            within: ctx.menu_within(),
        }
    }

    /// Opens the text box's context menu with its top-left at `at`, greyed like the Edit menu.
    pub(crate) fn open_context(&mut self, at: (i32, i32), by_keyboard: bool, ctx: &Ctx) {
        let live = editmenu::Live {
            undo: self.edit.can_undo(),
            redo: self.edit.can_redo(),
            selection: self.edit.has_selection(),
            paste: repriseapp::clipboard_has_text(),
            secret: false,
        };
        self.context = Some(editmenu::Menu::open(
            live,
            at,
            &self.menu_ctx(ctx),
            by_keyboard,
        ));
    }

    /// Below the caret's line, where Shift+F10 and the Menu key open the context menu.
    pub(crate) fn caret_point(&self, ctx: &Ctx) -> (i32, i32) {
        let el = self.edit_layout(ctx);
        let caret = self.edit.caret();
        let x = multiedit::x_of(&self.mono, &self.metrics, &self.edit, caret) - self.view.left;
        let row = caret.line.saturating_sub(self.view.top) as i32 + 1;
        let at = (
            el.inner.x + 1 + x,
            el.inner.y + 1 + row * self.metrics.line_h,
        );
        (
            at.0.clamp(el.inner.x, el.inner.right()),
            at.1.clamp(el.inner.y, el.inner.bottom()),
        )
    }

    fn context_step(&mut self, step: editmenu::Step, ctx: &Ctx) -> Reply {
        let cmd = match step {
            editmenu::Step::Run(cmd) => cmd,
            editmenu::Step::Close => {
                self.context = None;
                return Reply::redraw();
            }
            editmenu::Step::Redraw => return Reply::redraw(),
            editmenu::Step::None => return Reply::none(),
        };
        self.context = None;
        let row = match cmd {
            editmenu::Command::Undo => row::UNDO,
            editmenu::Command::Redo => row::REDO,
            editmenu::Command::Cut => row::CUT,
            editmenu::Command::Copy => row::COPY,
            editmenu::Command::Paste => row::PASTE,
            editmenu::Command::Delete => row::DELETE,
            editmenu::Command::SelectAll => row::SELECT_ALL,
        };
        Reply {
            redraw: true,
            ..self.command(title::EDIT, row, ctx)
        }
    }

    /// The pointer while the context menu is open: a press anywhere picks a row or closes it.
    pub(crate) fn context_pointer(&mut self, p: PointerInput, ctx: &Ctx) -> Reply {
        let cx = self.menu_ctx(ctx);
        let Some(menu) = self.context.as_mut() else {
            return Reply::none();
        };
        match p {
            PointerInput::Press { x, y, .. } => {
                let step = menu.press(x, y, &cx);
                self.context_step(step, ctx)
            }
            PointerInput::Motion { x, y } if menu.motion(x, y, &cx) => Reply::redraw(),
            _ => Reply::none(),
        }
    }

    /// Every key goes to the open context menu; the ones it has no use for do nothing.
    pub(crate) fn context_key(&mut self, k: &KeyInput, ctx: &Ctx) -> Reply {
        let cx = self.menu_ctx(ctx);
        let Some(menu) = self.context.as_mut() else {
            return Reply::none();
        };
        let step = menu.key(k.sym, k.utf8.as_deref(), &cx);
        self.context_step(step, ctx)
    }

    /// Grey the rows that cannot act.
    pub(crate) fn sync_menus(&mut self) {
        let has_text = self.edit.lines().iter().any(|l| !l.is_empty());
        let selected = self.edit.has_selection();
        let live = [
            (row::UNDO, self.edit.can_undo()),
            (row::REDO, self.edit.can_redo()),
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

    /// repriseapp routes keys only to the focused window, so a modeless Find hands these back here.
    pub(crate) fn accel(&mut self, k: &KeyInput, ctx: &Ctx) -> Option<Reply> {
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
}
