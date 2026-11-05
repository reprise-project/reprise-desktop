#[cfg(test)]
reprisebus::test_session!();

mod art;
mod bitmap;
mod dialogs;
mod doc;
mod file;
mod files;
mod history;
mod input;
mod menu;
mod shapes;
mod textbox;
mod view;

reprisekit::strings!("paint");

use bitmap::{Bitmap, WHITE};
use doc::{Doc, Tool};
use repriseapp::filedlg::Host;
use repriseapp::keys::AltArm;
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{App, Child, Ctx, DropForm, Event, Extended, FileDrop, MAIN, Reply, WinId};
use reprisekit::access::Tree;
use reprisekit::controls::aboutbox;
use reprisekit::controls::filedlg::FileDlg;
use reprisekit::controls::form::{self, Form};
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::menubar::Bar;
use reprisekit::controls::scrollbar::PageScroll;
use reprisekit::look::Paint as Look;
use reprisekit::pointer::DoubleClick;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use reprisekit::tr;
use std::path::PathBuf;
use std::rc::Rc;

const PROMPT: WinId = WinId::named("prompt");
const DIALOG: WinId = WinId::named("dialog");
const FONTS: WinId = WinId::named("fonts");
const ABOUT: WinId = WinId::named("about");
const OPEN: WinId = WinId::named("open");
const SAVEAS: WinId = WinId::named("saveas");

/// A fresh picture's size.
const NEW_SIZE: (i32, i32) = (480, 320);

/// What waits on a prompt's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Prompt {
    Error,
    /// Save changes before `Then`; Yes saves first, No goes ahead, Cancel stays.
    SaveChanges(Then),
    Overwrite(PathBuf),
}

/// What runs once the picture is saved or its changes are let go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Then {
    New,
    Open,
    Quit,
    Background,
}

struct Paint {
    doc: Doc,
    tool: Tool,
    /// The tool the color picker goes back to.
    before_pick: Tool,
    path: Option<PathBuf>,
    /// JPEG quality, from Save As.
    quality: u8,
    zoom: i32,
    /// The zoom the magnifier picks.
    magnify: i32,
    grid: bool,
    status_bar: bool,
    hscroll: PageScroll,
    vscroll: PageScroll,
    bar: Bar,
    /// The button held down on the picture.
    held: Option<u32>,
    /// The pointer over the picture, in picture pixels.
    hover: Option<(i32, i32)>,
    clicks: DoubleClick<()>,
    alt_arm: AltArm,
    /// Shift is held, as the last key events had it.
    shift: bool,
    tool_hot: Option<usize>,
    tool_pressed: Option<usize>,
    text: textbox::TextBox,
    fonts: Option<Form>,
    families: Vec<String>,
    dialog: Option<(dialogs::Kind, Form)>,
    prompt: Option<(MsgBox, Prompt)>,
    /// What runs after the Save As that a Yes on the save prompt raised.
    after_save: Option<Then>,
    about: bool,
    open: Option<FileDlg>,
    saveas: Option<FileDlg>,
    host: Host,
    dialog_text: Rc<TextRenderer>,
    dialog_px: u32,
    forms: Theme,
    app_icon: Option<Canvas>,
    warn_icon: Option<Canvas>,
    glyphs: view::Glyphs,
}

impl Paint {
    fn new(path: Option<PathBuf>) -> Paint {
        let mut failed = None;
        let (img, path) = match path {
            None => (Bitmap::new(NEW_SIZE.0, NEW_SIZE.1, WHITE), None),
            Some(p) => match file::load(&p) {
                Ok(img) => (img, Some(p)),
                Err(e) => {
                    failed = Some((p, e));
                    (Bitmap::new(NEW_SIZE.0, NEW_SIZE.1, WHITE), None)
                }
            },
        };
        let mut paint = Paint {
            doc: Doc::new(img),
            tool: Tool::Pencil,
            before_pick: Tool::Pencil,
            path,
            quality: file::DEFAULT_QUALITY,
            zoom: 1,
            magnify: 4,
            grid: true,
            status_bar: true,
            hscroll: PageScroll::across(),
            vscroll: PageScroll::default(),
            bar: Bar::new(menu::menus()),
            held: None,
            hover: None,
            clicks: DoubleClick::default(),
            alt_arm: AltArm::default(),
            shift: false,
            tool_hot: None,
            tool_pressed: None,
            text: textbox::TextBox::default(),
            fonts: None,
            families: Vec::new(),
            dialog: None,
            prompt: None,
            after_save: None,
            about: false,
            open: None,
            saveas: None,
            host: Host::default(),
            dialog_text: repriseapp::start_text(),
            dialog_px: Theme::default().fonts.size_px,
            forms: Theme::default(),
            app_icon: None,
            warn_icon: None,
            glyphs: view::Glyphs::default(),
        };
        if let Some((p, e)) = failed {
            paint.error(tr!(
                "open-failed",
                path = p.display().to_string(),
                error = e
            ));
        }
        paint.sync_menus();
        paint
    }

    fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| tr!("untitled"));
        tr!("window-title", name = name)
    }

    fn follow(&mut self, ctx: &Ctx) {
        self.forms = ctx.forms();
        self.forms.fonts.size_px = self.forms.text.px(self.dialog_px);
    }

    fn prime(&mut self, ctx: &mut Ctx) {
        self.follow(ctx);
        if self.app_icon.is_none() {
            self.app_icon = ctx.icons.get("applications-graphics", 32);
        }
        if self.warn_icon.is_none() {
            self.warn_icon = ctx.icons.get("dialog-warning", 32);
        }
        if let Some((b, _)) = &mut self.prompt
            && b.fields.icon.is_none()
        {
            b.fields.icon = self.warn_icon.clone();
        }
        self.host.icons.prime(&mut ctx.icons);
    }

    fn file_dlg(&self, win: WinId) -> Option<&FileDlg> {
        match win {
            OPEN => self.open.as_ref(),
            SAVEAS => self.saveas.as_ref(),
            _ => None,
        }
    }

    fn about_form(&self) -> Form {
        let (text, px) = (&self.dialog_text, self.dialog_px);
        let fields = repriseapp::about::fields(&tr!("name"), self.app_icon.clone());
        let mut f = aboutbox::form(&fields, text, px);
        let u = self.forms.text;
        let w = u.len(aboutbox::CLIENT.0);
        f.set_picture("banner", aboutbox::banner_wide(text, px, w, u));
        f
    }

    fn form_look(&self) -> Look<'_> {
        Look {
            s: 1,
            ..self.forms.paint()
        }
    }
}

impl App for Paint {
    fn title(&self) -> String {
        Paint::title(self)
    }

    fn app_id(&self) -> String {
        "reprise.paint".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        view::INITIAL_SIZE
    }

    fn children(&self) -> Vec<Child> {
        let mut kids = Vec::new();
        if self.fonts.is_some() {
            kids.push(Child {
                id: FONTS,
                title: tr!("title-fonts"),
                size: self.forms.text.size(dialogs::FONTS_CLIENT),
                fixed: true,
                modal: false,
            });
        }
        if let Some((kind, _)) = &self.dialog {
            kids.push(Child {
                id: DIALOG,
                title: kind.title(),
                size: self.forms.text.size(kind.client()),
                fixed: true,
                modal: true,
            });
        }
        if self.about {
            kids.push(Child {
                id: ABOUT,
                title: tr!("menu-about").replace('&', ""),
                size: self.forms.text.size(aboutbox::CLIENT),
                fixed: true,
                modal: true,
            });
        }
        for (id, dlg, title) in [
            (OPEN, &self.open, tr!("title-open")),
            (SAVEAS, &self.saveas, tr!("title-save-as")),
        ] {
            if let Some(d) = dlg {
                kids.push(Child {
                    id,
                    title,
                    size: d.client(),
                    fixed: false,
                    modal: true,
                });
            }
        }
        if let Some((b, _)) = &self.prompt {
            kids.push(b.child(PROMPT, &self.dialog_text, &self.forms));
        }
        kids
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.dialog_text = ctx.text.clone();
        self.prime(ctx);
        if win != MAIN {
            self.alt_arm.interrupt();
        }
        let reply = match win {
            PROMPT => self.prompt_event(ev),
            DIALOG => self.dialog_event(ev, ctx),
            FONTS => self.fonts_event(ev, ctx),
            ABOUT => self.about_event(ev),
            OPEN => self.open_event(ev, ctx),
            SAVEAS => self.saveas_event(ev, ctx),
            _ => self.main_event(ev, ctx),
        };
        self.sync_menus();
        reply
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.dialog_text = ctx.text.clone();
        self.follow(ctx);
        let cs = &ctx.theme.colors;
        let px = self.forms.fonts.size_px;
        match win {
            PROMPT => {
                if let Some((b, _)) = &self.prompt {
                    b.draw(c, &self.forms, &self.dialog_text);
                }
            }
            DIALOG | FONTS => {
                c.fill_rect(c.bounds(), cs.button_face);
                let f = match win {
                    DIALOG => self.dialog.as_ref().map(|(_, f)| f),
                    _ => self.fonts.as_ref(),
                };
                if let Some(f) = f {
                    form::draw(c, &self.dialog_text, &self.form_look(), f, true, px);
                }
            }
            ABOUT => {
                c.fill_rect(c.bounds(), cs.button_face);
                let f = self.about_form();
                form::draw(c, &self.dialog_text, &self.form_look(), &f, true, px);
            }
            OPEN | SAVEAS => {
                if let Some(d) = self.file_dlg(win) {
                    let r = Rect::new(0, 0, d.client().0, d.client().1);
                    d.draw(c, &self.dialog_text, &self.form_look(), r, px);
                }
            }
            _ => self.draw_main(c, ctx),
        }
    }
}

impl Extended for Paint {
    fn draw_failed(&mut self, error: &str) -> Reply {
        self.error(tr!("draw-failed", error = error));
        Reply::redraw_all()
    }

    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        if win == MAIN {
            return Some(view::MIN_SIZE);
        }
        self.file_dlg(win).map(FileDlg::min_client)
    }

    fn popup(&mut self, win: WinId, ctx: &Ctx) -> Option<Box<dyn Hosted + '_>> {
        if win != MAIN {
            return repriseapp::reader_popup(self, win, ctx);
        }
        if self.bar.open.is_some() {
            return Some(Box::new(&mut self.bar));
        }
        None
    }

    fn drop_form(&mut self, win: WinId) -> Option<DropForm<'_>> {
        let px = Some(self.forms.fonts.size_px);
        let form = match win {
            DIALOG => self.dialog.as_mut().map(|(_, f)| f)?,
            FONTS => self.fonts.as_mut()?,
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

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        self.follow(ctx);
        let (dt, fpx) = (&self.dialog_text, self.forms.fonts.size_px);
        match win {
            PROMPT => {
                if let Some((b, _)) = &self.prompt {
                    b.access(tree, dt, &self.forms);
                }
            }
            DIALOG => {
                if let Some((_, f)) = &self.dialog {
                    oneshot::access_form(f, tree, dt, fpx);
                }
            }
            FONTS => {
                if let Some(f) = &self.fonts {
                    oneshot::access_form(f, tree, dt, fpx);
                }
            }
            ABOUT => {
                let f = self.about_form();
                f.drawn_with(&self.forms.paint());
                oneshot::access_form(&f, tree, dt, fpx);
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

/// Runs the program; `args` are the command line after the program's name.
pub fn main(args: Vec<std::ffi::OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let path = args.into_iter().next().map(PathBuf::from);
    let mut paint = Paint::new(path);
    paint.forms = repriseapp::start_forms();
    repriseapp::run(Box::new(paint)).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests;
