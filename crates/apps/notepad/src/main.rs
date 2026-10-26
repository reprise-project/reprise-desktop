mod dialogs;
mod document;
mod drop;
mod file;
mod filedlg_host;
mod find;
mod input;
mod menu;
mod pending;
mod window;

use dialogs::{FormStep, form_event, goto_form, ok_box};
use document::{
    Stamp, has_crlf, load, open_failed_text, read_text, recovered_in, recovery_dir, write_document,
    write_recovery,
};
use drop::Hover;
use menu::menus;
use pending::{Pending, PendingAction};
use repriseapp::filedlg::{Host, Resolved, Step, taken, type_keep};
use repriseapp::keys::AltArm;
use repriseapp::keys::{dialog_key, form_clipboard};
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{
    App, BTN_LEFT, BTN_RIGHT, Child, Ctx, DropForm, Event, FileDrop, KeyInput, MAIN, PointerInput,
    Reply, WinId,
};
use reprisekit::access::{ROOT, Tree};
use reprisekit::classic::widgets::{self, Axis, ScrollHit};
use reprisekit::controls::aboutbox;
use reprisekit::controls::editmenu;
use reprisekit::controls::filedlg::{CLIENT, DlgMode, Event as FileEvent, Fields, FileDlg};
use reprisekit::controls::finddlg;
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::menu::{MenuCtx, MenuItem};
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
use window::{EDIT_PX, edit_font};

const GOTO: WinId = "goto";
const GOTO_CLIENT: (i32, i32) = (268, 95);

const FIND: WinId = "find";
const NOTFOUND: WinId = "notfound";
const ERRORBOX: WinId = "error";
const REPLACE_WIN: WinId = "replace";
const ABOUT: WinId = "about";
const SAVECHANGES: WinId = "savechanges";
const OVERWRITE: WinId = "overwrite";
const LOSSY: WinId = "lossy";
const RECOVERED: WinId = "recovered";
const CHANGED: WinId = "changed";
const OPEN: WinId = "open";
const SAVEAS: WinId = "saveas";

struct Notepad {
    edit: MultiEdit,
    view: View,
    metrics: Metrics,
    mono: TextRenderer,
    bar: Bar,
    path: Option<std::path::PathBuf>,
    /// `path` as it was opened or last saved; a save asks first when the file no longer matches.
    stamp: Option<Stamp>,
    /// The prompt a save raises when the file changed on disk.
    changed: Option<MsgBox>,
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
    savechanges_shown: oneshot::Shown,
    overwrite: Option<PathBuf>,
    overwrite_focus: usize,
    overwrite_shown: oneshot::Shown,
    lossy: Option<(MsgBox, String)>,
    /// Where `emergency` leaves unsaved text and the next start finds it.
    rescue_dir: Option<PathBuf>,
    /// The prompt for one recovery file, and the files offered after it.
    recovered: Option<(MsgBox, PathBuf)>,
    recover_queue: Vec<PathBuf>,
    /// The recovery file the open document came from, deleted once it is saved or discarded.
    rescued: Option<PathBuf>,
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
    /// The text box's right-click menu while it is open.
    context: Option<editmenu::Menu>,
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
            stamp: path.as_deref().and_then(Stamp::of),
            changed: None,
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
            savechanges_shown: oneshot::Shown::default(),
            overwrite: None,
            overwrite_focus: 1,
            overwrite_shown: oneshot::Shown::default(),
            lossy: None,
            rescue_dir: recovery_dir(),
            recovered: None,
            recover_queue: Vec::new(),
            rescued: None,
            dialog_text: repriseapp::start_text(),
            dialog_px: Theme::default().fonts.size_px,
            forms: Theme::default(),
            open: None,
            saveas: None,
            host: Host::default(),
            hover: None,
            context: None,
        };
        if let Some((p, e)) = failed {
            np.open_failed(&p, &e);
        }
        np.sync_menus();
        np
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

    fn follow(&mut self, ctx: &Ctx) {
        self.forms = ctx.forms();
        self.forms.fonts.size_px = self.forms.text.px(self.dialog_px);
        self.metrics.follow_look(&ctx.theme.look.controls());
        let px = reprisekit::theme::text_px(EDIT_PX, ctx.text_size);
        self.metrics.font_px = px;
        self.metrics.line_h = px as i32;
    }

    /// The Find or Replace form behind `win`.
    fn find_form(&self, win: WinId) -> Option<&Form> {
        match win {
            FIND => self.find.as_ref(),
            REPLACE_WIN => self.replace.as_ref(),
            _ => None,
        }
    }

    /// The Open or Save As dialog behind `win`.
    fn file_dlg(&self, win: WinId) -> Option<&FileDlg> {
        match win {
            OPEN => self.open.as_ref(),
            SAVEAS => self.saveas.as_ref(),
            _ => None,
        }
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
            .chain(self.lossy.iter_mut().map(|(b, _)| b))
            .chain(self.recovered.iter_mut().map(|(b, _)| b))
            .chain(self.changed.iter_mut());
        for b in raised.filter(|b| b.fields.icon.is_none()) {
            b.fields.icon = self.warn_icon.clone();
        }
        self.host.icons.prime(&mut ctx.icons);
    }
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

    fn emergency(&mut self) {
        self.rescue();
    }

    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        self.file_dlg(win).map(FileDlg::min_client)
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
        if let Some((b, _)) = &self.recovered {
            kids.push(b.child(RECOVERED, &self.dialog_text, &self.forms));
        }
        if let Some(b) = &self.changed {
            kids.push(b.child(CHANGED, &self.dialog_text, &self.forms));
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
        let clip = match win {
            GOTO | FIND | REPLACE_WIN => self
                .drop_form(win)
                .map(|d| form_clipboard(d.form.focused_edit(), &ev)),
            _ => None,
        };
        let reply = match win {
            GOTO => self.goto_event(ev, ctx),
            FIND => self.find_event(ev, ctx),
            REPLACE_WIN => self.replace_event(ev, ctx),
            NOTFOUND => self.notfound_event(ev),
            ERRORBOX => self.error_event(ev),
            ABOUT => self.about_event(ev),
            SAVECHANGES => self.savechanges_event(ev),
            OVERWRITE => self.overwrite_event(ev),
            LOSSY => self.lossy_event(ev),
            RECOVERED => self.recovered_event(ev),
            CHANGED => self.changed_event(ev),
            OPEN => self.open_event(ev, ctx),
            SAVEAS => self.saveas_event(ev, ctx),
            _ => self.main_event(ev, ctx),
        };
        let reply = match clip {
            Some(clip) => reply.with_clipboard(clip),
            None => reply,
        };
        self.sync_menus();
        reply
    }

    fn popup(&mut self, win: WinId, ctx: &Ctx) -> Option<Box<dyn Hosted + '_>> {
        if win != MAIN {
            return repriseapp::reader_popup(self, win, ctx);
        }
        if self.bar.open.is_some() {
            return Some(Box::new(&mut self.bar));
        }
        Some(Box::new(&mut self.context))
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
            FIND | REPLACE_WIN => {
                c.fill_rect(c.bounds(), cs.button_face);
                if let Some(f) = self.find_form(win) {
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
            RECOVERED => {
                if let Some((b, _)) = &self.recovered {
                    b.draw(c, &self.forms, &self.dialog_text);
                }
            }
            CHANGED => {
                if let Some(b) = &self.changed {
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
                let f = msgbox::form(
                    &self.savechanges_fields(),
                    &self.dialog_text,
                    self.forms.fonts.size_px,
                    self.forms.text,
                );
                self.savechanges_shown
                    .draw(c, &self.forms, &self.dialog_text, f);
            }
            OVERWRITE => {
                if let Some(path) = &self.overwrite {
                    let f = msgbox::form(
                        &self.overwrite_fields(path),
                        &self.dialog_text,
                        self.forms.fonts.size_px,
                        self.forms.text,
                    );
                    self.overwrite_shown
                        .draw(c, &self.forms, &self.dialog_text, f);
                }
            }
            OPEN | SAVEAS => {
                if let Some(d) = self.file_dlg(win) {
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
            FIND | REPLACE_WIN => {
                if let Some(f) = self.find_form(win) {
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
            RECOVERED => {
                if let Some((b, _)) = &self.recovered {
                    b.access(tree, dt, forms);
                }
            }
            CHANGED => {
                if let Some(b) = &self.changed {
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
            OPEN | SAVEAS => {
                if let Some(d) = self.file_dlg(win) {
                    d.access(tree, Rect::new(0, 0, d.client().0, d.client().1), dt, fpx);
                }
            }
            _ => self.access_main(tree, ctx),
        }
    }
}

fn main() {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let mut notepad = Notepad::new(path);
    notepad.forms = repriseapp::start_forms();
    notepad.offer_recovered();
    repriseapp::run(Box::new(notepad));
}

#[cfg(test)]
mod tests;
