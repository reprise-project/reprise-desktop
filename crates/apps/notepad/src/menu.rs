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
    pub const FORMAT: usize = 2;
    pub const VIEW: usize = 3;
    pub const HELP: usize = 4;
}

pub(crate) mod row {
    pub const NEW: usize = 0;
    pub const OPEN: usize = 1;
    pub const SAVE: usize = 2;
    pub const SAVE_AS: usize = 3;
    pub const PAGE_SETUP: usize = 5;
    pub const PRINT: usize = 6;
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
    pub const WORD_WRAP: usize = 0;
    pub const FONT: usize = 1;
    pub const ZOOM: usize = 0;
    pub const STATUS_BAR: usize = 1;
    pub const ABOUT: usize = 2;
}

/// The rows of View > Zoom.
pub(crate) mod zoom {
    pub const IN: usize = 0;
    pub const OUT: usize = 1;
    pub const RESET: usize = 2;
}

/// The zoom's percent steps and bounds.
pub(crate) const ZOOM_STEP: u32 = 10;
pub(crate) const ZOOM_MIN: u32 = 10;
pub(crate) const ZOOM_MAX: u32 = 500;

pub(crate) fn menus() -> Vec<(String, MenuTree)> {
    let mut view = MenuTree::new();
    view.push_sub(
        mi(&tr!("menu-zoom"), None, true),
        MenuTree::flat(vec![
            mi(&tr!("menu-zoom-in"), Some("Ctrl+Plus"), true),
            mi(&tr!("menu-zoom-out"), Some("Ctrl+Minus"), true),
            mi(&tr!("menu-zoom-reset"), Some("Ctrl+0"), true),
        ]),
    );
    view.push(mi(&tr!("menu-status-bar"), None, true));
    let flat = |title: String, items: Vec<MenuItem>| (title, MenuTree::flat(items));
    vec![
        flat(
            tr!("menu-file"),
            vec![
                mi(&tr!("menu-new"), Some("Ctrl+N"), true),
                mi(&tr!("menu-open"), Some("Ctrl+O"), true),
                mi(&tr!("menu-save"), Some("Ctrl+S"), true),
                mi(&tr!("menu-save-as"), None, true),
                MenuItem::Separator,
                mi(&tr!("menu-page-setup"), None, true),
                mi(&tr!("menu-print"), Some("Ctrl+P"), true),
                MenuItem::Separator,
                mi(&tr!("menu-exit"), None, true),
            ],
        ),
        flat(
            tr!("menu-edit"),
            vec![
                mi(&tr!("menu-undo"), Some("Ctrl+Z"), true),
                mi(&tr!("menu-redo"), Some("Ctrl+Y"), true),
                MenuItem::Separator,
                mi(&tr!("menu-cut"), Some("Ctrl+X"), true),
                mi(&tr!("menu-copy"), Some("Ctrl+C"), true),
                mi(&tr!("menu-paste"), Some("Ctrl+V"), true),
                mi(&tr!("menu-delete"), Some("Del"), true),
                MenuItem::Separator,
                mi(&tr!("menu-find"), Some("Ctrl+F"), true),
                mi(&tr!("menu-find-next"), Some("F3"), true),
                mi(&tr!("menu-replace"), Some("Ctrl+H"), true),
                mi(&tr!("menu-go-to"), Some("Ctrl+G"), true),
                MenuItem::Separator,
                mi(&tr!("menu-select-all"), Some("Ctrl+A"), true),
                mi(&tr!("menu-time-date"), Some("F5"), true),
            ],
        ),
        flat(
            tr!("menu-format"),
            vec![
                mi(&tr!("menu-word-wrap"), None, true),
                mi(&tr!("menu-font"), None, true),
            ],
        ),
        (tr!("menu-view"), view),
        flat(
            tr!("menu-help"),
            vec![
                mi(&tr!("menu-help-topics"), None, false),
                MenuItem::Separator,
                mi(&tr!("menu-about"), None, true),
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
            (title::FILE, row::PAGE_SETUP) => self.printing.page_setup(ctx),
            (title::FILE, row::PRINT) => self.open_print(ctx),
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
            (title::EDIT, row::FIND) => reply.raise = self.open_find(),
            (title::EDIT, row::REPLACE) => reply.raise = self.open_replace(),
            (title::EDIT, row::FIND_NEXT) => {
                if self.last_query.is_none() {
                    reply.raise = self.open_find();
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
            (title::FORMAT, row::WORD_WRAP) => {
                self.settings.word_wrap = !self.settings.word_wrap;
                self.sync_toggles();
                self.save_settings();
                self.scroll_to_caret(ctx);
            }
            (title::FORMAT, row::FONT) => self.open_font(),
            (title::VIEW, row::ZOOM) => {}
            (title::VIEW, row::STATUS_BAR) => {
                self.settings.status_bar = !self.settings.status_bar;
                self.sync_toggles();
                self.save_settings();
            }
            (title::HELP, row::ABOUT) => self.about = true,
            _ => {}
        }
        reply
    }

    /// Notepad's kept settings take effect: the font, Word Wrap, the status bar and the page.
    pub(crate) fn take_settings(&mut self, s: Settings) {
        if let Some(page) = &s.page {
            self.printing.setup = page.clone();
        }
        self.settings = s;
        self.mono = edit_font(&self.settings);
        self.sync_toggles();
    }

    /// Word Wrap and Status Bar as the settings have them, their rows checked to match.
    fn sync_toggles(&mut self) {
        self.metrics.wrap = self.settings.word_wrap;
        self.status_bar = self.settings.status_bar;
        let rows = [
            (title::FORMAT, row::WORD_WRAP, self.metrics.wrap),
            (title::VIEW, row::STATUS_BAR, self.status_bar),
        ];
        for (t, r, on) in rows {
            if let MenuItem::Entry { checked, .. } = &mut self.bar.menus[t][r] {
                *checked = on;
            }
        }
    }

    /// A row picked by its `path` from title `title`'s menu: View > Zoom's rows, else
    /// [`Notepad::command`].
    pub(crate) fn chosen(&mut self, title: usize, path: &[usize], ctx: &Ctx) -> Reply {
        match path {
            [row::ZOOM, z] if title == title::VIEW => self.zoom_by(*z, ctx),
            _ => self.command(title, path[0], ctx),
        }
    }

    /// Zoom In and Out step the text's size by [`ZOOM_STEP`] percent within its bounds;
    /// Restore Default Zoom goes back to 100%.
    pub(crate) fn zoom_by(&mut self, z: usize, ctx: &Ctx) -> Reply {
        self.zoom = match z {
            zoom::IN => (self.zoom + ZOOM_STEP).min(ZOOM_MAX),
            zoom::OUT => self.zoom.saturating_sub(ZOOM_STEP).max(ZOOM_MIN),
            _ => 100,
        };
        self.follow(ctx);
        self.scroll_to_caret(ctx);
        Reply::redraw()
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
            read_only: false,
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
        multiedit::caret_point(self.mono.get(), &self.metrics, &self.edit, &el, self.view)
    }

    fn context_step(&mut self, step: editmenu::Step, ctx: &Ctx) -> Reply {
        let cmd = match step {
            editmenu::Step::Run(cmd) => cmd,
            editmenu::Step::Close | editmenu::Step::Extra(_) => {
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
            // Plus is Shift+= on most layouts, so = zooms in too.
            let z = match k.sym {
                0x2b | 0x3d | keysym::KP_ADD => Some(zoom::IN),
                0x2d | keysym::KP_SUBTRACT => Some(zoom::OUT),
                0x30 | keysym::KP_0 => Some(zoom::RESET),
                _ => None,
            };
            if let Some(z) = z {
                return Some(self.zoom_by(z, ctx));
            }
            match k.sym | 0x20 {
                0x6e => return Some(self.command(title::FILE, row::NEW, ctx)),
                0x6f => return Some(self.command(title::FILE, row::OPEN, ctx)),
                0x73 => return Some(self.command(title::FILE, row::SAVE, ctx)),
                0x70 => return Some(self.command(title::FILE, row::PRINT, ctx)),
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
