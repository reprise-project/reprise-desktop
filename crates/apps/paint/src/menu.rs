//! The menu bar's rows, what each does, and the keys that reach them without the menu.

use super::*;
use reprisekit::controls::menu::MenuItem;
use reprisekit::textedit::keysym;

const PLUS: u32 = 0x2b;
const EQUAL: u32 = 0x3d;
const MINUS: u32 = 0x2d;
const ZERO: u32 = 0x30;
const KP_ADD: u32 = 0xffab;
const KP_SUBTRACT: u32 = 0xffad;
const KP_0: u32 = 0xffb0;

fn mi(label: &str, shortcut: Option<&str>) -> MenuItem {
    MenuItem::item(label).shortcut(shortcut.map(str::to_string))
}

pub(crate) mod title {
    pub const FILE: usize = 0;
    pub const EDIT: usize = 1;
    pub const VIEW: usize = 2;
    pub const IMAGE: usize = 3;
    pub const HELP: usize = 4;
}

pub(crate) mod row {
    pub const NEW: usize = 0;
    pub const OPEN: usize = 1;
    pub const SAVE: usize = 2;
    pub const SAVE_AS: usize = 3;
    pub const BACKGROUND: usize = 5;
    pub const EXIT: usize = 7;

    pub const UNDO: usize = 0;
    pub const REDO: usize = 1;
    pub const CUT: usize = 3;
    pub const COPY: usize = 4;
    pub const PASTE: usize = 5;
    pub const CLEAR_SELECTION: usize = 6;
    pub const SELECT_ALL: usize = 7;

    pub const ZOOM_IN: usize = 0;
    pub const ZOOM_OUT: usize = 1;
    pub const ACTUAL_SIZE: usize = 2;
    pub const GRID: usize = 4;
    pub const TEXT_TOOLBAR: usize = 5;
    pub const STATUS_BAR: usize = 6;

    pub const FLIP: usize = 0;
    pub const STRETCH: usize = 1;
    pub const INVERT: usize = 2;
    pub const ATTRIBUTES: usize = 3;
    pub const CLEAR_IMAGE: usize = 4;
    pub const CROP: usize = 5;

    pub const ABOUT: usize = 0;
}

pub(crate) fn menus() -> Vec<(String, Vec<MenuItem>)> {
    vec![
        (
            tr!("menu-file"),
            vec![
                mi(&tr!("menu-new"), Some("Ctrl+N")),
                mi(&tr!("menu-open"), Some("Ctrl+O")),
                mi(&tr!("menu-save"), Some("Ctrl+S")),
                mi(&tr!("menu-save-as"), None),
                MenuItem::Separator,
                mi(&tr!("menu-background"), None),
                MenuItem::Separator,
                mi(&tr!("menu-exit"), None),
            ],
        ),
        (
            tr!("menu-edit"),
            vec![
                mi(&tr!("menu-undo"), Some("Ctrl+Z")),
                mi(&tr!("menu-redo"), Some("Ctrl+Y")),
                MenuItem::Separator,
                mi(&tr!("menu-cut"), Some("Ctrl+X")),
                mi(&tr!("menu-copy"), Some("Ctrl+C")),
                mi(&tr!("menu-paste"), Some("Ctrl+V")),
                mi(&tr!("menu-clear-selection"), Some("Del")),
                mi(&tr!("menu-select-all"), Some("Ctrl+A")),
            ],
        ),
        (
            tr!("menu-view"),
            vec![
                mi(&tr!("menu-zoom-in"), Some("Ctrl++")),
                mi(&tr!("menu-zoom-out"), Some("Ctrl+-")),
                mi(&tr!("menu-actual-size"), Some("Ctrl+0")),
                MenuItem::Separator,
                mi(&tr!("menu-grid"), Some("Ctrl+G")),
                mi(&tr!("menu-text-toolbar"), None),
                mi(&tr!("menu-status-bar"), None),
            ],
        ),
        (
            tr!("menu-image"),
            vec![
                mi(&tr!("menu-flip-rotate"), Some("Ctrl+R")),
                mi(&tr!("menu-stretch"), Some("Ctrl+W")),
                mi(&tr!("menu-invert"), Some("Ctrl+I")),
                mi(&tr!("menu-attributes"), Some("Ctrl+E")),
                mi(&tr!("menu-clear-image"), Some("Ctrl+Shift+N")),
                mi(&tr!("menu-crop"), None),
            ],
        ),
        (tr!("menu-help"), vec![mi(&tr!("menu-about"), None)]),
    ]
}

impl Paint {
    /// A menu row or its key. A greyed row refuses its key too.
    pub(crate) fn command(&mut self, title: usize, row: usize, ctx: &Ctx) -> Reply {
        if !matches!(
            self.bar.menus[title][row],
            MenuItem::Entry { enabled: true, .. }
        ) {
            return Reply::none();
        }
        let mut reply = Reply::redraw_all();
        match (title, row) {
            (title::FILE, row::NEW) => return self.confirm(Then::New),
            (title::FILE, row::OPEN) => return self.confirm(Then::Open),
            (title::FILE, row::SAVE) => return self.save(),
            (title::FILE, row::SAVE_AS) => self.saveas_dialog(),
            (title::FILE, row::BACKGROUND) => return self.set_background(),
            (title::FILE, row::EXIT) => return self.confirm(Then::Quit),
            (title::EDIT, row::UNDO) => {
                self.commit_text();
                self.doc.undo();
                self.hold_scroll(ctx);
            }
            (title::EDIT, row::REDO) => {
                self.commit_text();
                self.doc.redo();
                self.hold_scroll(ctx);
            }
            (title::EDIT, row::CUT) => {
                reply.copy_image = self.selection_png();
                self.doc.delete_selection();
            }
            (title::EDIT, row::COPY) => reply.copy_image = self.selection_png(),
            (title::EDIT, row::PASTE) => reply.paste_image = true,
            (title::EDIT, row::CLEAR_SELECTION) => self.doc.delete_selection(),
            (title::EDIT, row::SELECT_ALL) => {
                self.commit_text();
                self.pick_tool(Tool::Select);
                self.doc.select_all();
            }
            (title::VIEW, row::ZOOM_IN) => self.zoom_by(1, ctx),
            (title::VIEW, row::ZOOM_OUT) => self.zoom_by(-1, ctx),
            (title::VIEW, row::ACTUAL_SIZE) => self.zoom_to(1, None, ctx),
            (title::VIEW, row::GRID) => self.grid = !self.grid,
            (title::VIEW, row::TEXT_TOOLBAR) => {
                if self.fonts.is_some() {
                    self.fonts = None;
                } else {
                    self.open_fonts();
                }
            }
            (title::VIEW, row::STATUS_BAR) => {
                self.status_bar = !self.status_bar;
                self.hold_scroll(ctx);
            }
            (title::IMAGE, row::FLIP) => self.open_dialog(dialogs::Kind::Flip),
            (title::IMAGE, row::STRETCH) => self.open_dialog(dialogs::Kind::Stretch),
            (title::IMAGE, row::ATTRIBUTES) => self.open_dialog(dialogs::Kind::Attributes),
            (title::IMAGE, row::INVERT) => {
                self.commit_text();
                let mut img = self.doc.img.clone();
                img.invert();
                self.doc.replace(img);
            }
            (title::IMAGE, row::CLEAR_IMAGE) => {
                self.commit_text();
                let img = Bitmap::new(self.doc.img.w, self.doc.img.h, self.doc.secondary);
                self.doc.replace(img);
            }
            (title::IMAGE, row::CROP) => {
                self.doc.crop();
                self.hold_scroll(ctx);
            }
            (title::HELP, row::ABOUT) => self.about = true,
            _ => {}
        }
        reply
    }

    /// Greys the rows that cannot act and ticks the ones that are on.
    pub(crate) fn sync_menus(&mut self) {
        let selected = self.doc.sel.is_some();
        let pending = self.doc.text_box.is_some() || self.doc.pending();
        let live = [
            (
                title::EDIT,
                row::UNDO,
                self.doc.hist.can_undo() || pending || self.floating(),
            ),
            (title::EDIT, row::REDO, self.doc.hist.can_redo()),
            (title::EDIT, row::CUT, selected),
            (title::EDIT, row::COPY, selected),
            (title::EDIT, row::PASTE, repriseapp::clipboard_has_image()),
            (title::EDIT, row::CLEAR_SELECTION, selected),
            (title::IMAGE, row::CROP, selected),
            (
                title::VIEW,
                row::ZOOM_IN,
                self.zoom < *view::ZOOMS.last().unwrap_or(&1),
            ),
            (title::VIEW, row::ZOOM_OUT, self.zoom > 1),
        ];
        for (t, r, on) in live {
            if let MenuItem::Entry { enabled, .. } = &mut self.bar.menus[t][r] {
                *enabled = on;
            }
        }
        let ticks = [
            (title::VIEW, row::GRID, self.grid),
            (title::VIEW, row::TEXT_TOOLBAR, self.fonts.is_some()),
            (title::VIEW, row::STATUS_BAR, self.status_bar),
        ];
        for (t, r, on) in ticks {
            if let MenuItem::Entry { checked, .. } = &mut self.bar.menus[t][r] {
                *checked = on;
            }
        }
    }

    fn floating(&self) -> bool {
        self.doc.sel.as_ref().is_some_and(|s| s.float.is_some())
    }

    /// The keys that reach a row without the menu.
    pub(crate) fn accel(&mut self, k: &KeyInput, ctx: &Ctx) -> Option<Reply> {
        let (t, r) = if k.mods.ctrl {
            match (k.sym | 0x20, k.mods.shift) {
                (0x6e, true) => (title::IMAGE, row::CLEAR_IMAGE),
                (0x6e, false) => (title::FILE, row::NEW),
                (0x6f, _) => (title::FILE, row::OPEN),
                (0x73, _) => (title::FILE, row::SAVE),
                (0x7a, _) => (title::EDIT, row::UNDO),
                (0x79, _) => (title::EDIT, row::REDO),
                (0x78, _) => (title::EDIT, row::CUT),
                (0x63, _) => (title::EDIT, row::COPY),
                (0x76, _) => (title::EDIT, row::PASTE),
                (0x61, _) => (title::EDIT, row::SELECT_ALL),
                (0x67, _) => (title::VIEW, row::GRID),
                (0x72, _) => (title::IMAGE, row::FLIP),
                (0x77, _) => (title::IMAGE, row::STRETCH),
                (0x69, _) => (title::IMAGE, row::INVERT),
                (0x65, _) => (title::IMAGE, row::ATTRIBUTES),
                _ => match k.sym {
                    PLUS | EQUAL | KP_ADD => (title::VIEW, row::ZOOM_IN),
                    MINUS | KP_SUBTRACT => (title::VIEW, row::ZOOM_OUT),
                    ZERO | KP_0 => (title::VIEW, row::ACTUAL_SIZE),
                    _ => return None,
                },
            }
        } else {
            match k.sym {
                keysym::DELETE | keysym::KP_DELETE => (title::EDIT, row::CLEAR_SELECTION),
                _ => return None,
            }
        };
        Some(self.command(t, r, ctx))
    }

    /// The selection as PNG bytes for the clipboard.
    fn selection_png(&self) -> Option<std::sync::Arc<Vec<u8>>> {
        let pic = self.doc.selected()?;
        match file::encode(&pic, file::Kind::Png, file::DEFAULT_QUALITY) {
            Ok(png) => Some(std::sync::Arc::new(png)),
            Err(e) => {
                log::warn!("copy: {e}");
                None
            }
        }
    }

    pub(crate) fn zoom_by(&mut self, step: i32, ctx: &Ctx) {
        let i = view::ZOOMS
            .iter()
            .position(|&z| z >= self.zoom)
            .unwrap_or(0) as i32;
        let next = (i + step).clamp(0, view::ZOOMS.len() as i32 - 1) as usize;
        self.zoom_to(view::ZOOMS[next], None, ctx);
    }

    /// Zooms to `z`, keeping picture point `at` (else the view's middle) where it shows.
    pub(crate) fn zoom_to(&mut self, z: i32, at: Option<(i32, i32)>, ctx: &Ctx) {
        let l = self.lay(ctx);
        let mid = (l.view.x + l.view.width / 2, l.view.y + l.view.height / 2);
        let at = at.unwrap_or_else(|| self.to_picture(&l, mid.0, mid.1));
        self.zoom = z;
        let l = self.lay(ctx);
        self.hscroll.top = view::MARGIN + at.0 * z - l.view.width / 2;
        self.vscroll.top = view::MARGIN + at.1 * z - l.view.height / 2;
        self.hold_scroll(ctx);
    }
}

use repriseapp::KeyInput;
