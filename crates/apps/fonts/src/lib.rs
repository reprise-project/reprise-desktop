//! Font Viewer: the window round `reprisekit::controls::fontview` for one font file, opened from
//! Explorer or the desktop. Install runs on a listing thread ([`install`]).

#[cfg(test)]
reprisebus::test_session!();

mod install;

use install::user_dir;
use repriseapp::listjob::Job;
use repriseapp::{App, Ctx, Event, Reply, WinId};
use reprisekit::access::Tree;
use reprisekit::controls::fontview::{self, Install};
use reprisekit::controls::form::{self, Form};
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::text::fontfile::{self, FaceNames};
use reprisekit::tr;
use std::path::PathBuf;

reprisekit::strings!("fonts");

struct Viewer {
    path: PathBuf,
    faces: Vec<FaceNames>,
    picked: usize,
    install: Install,
    /// The picked face, which the sample is set in.
    sample: Option<TextRenderer>,
    /// Built for the state as it is; dropped when the face or Install changes.
    form: Option<Form>,
    job: Option<Job<Result<PathBuf, String>>>,
    /// The client size last asked for, at the text size of the last event.
    asked: (i32, i32),
}

impl Viewer {
    fn new(path: PathBuf) -> Viewer {
        let faces = fontfile::faces(&path).unwrap_or_default();
        let mut v = Viewer {
            path,
            faces,
            picked: 0,
            install: Install::No,
            sample: None,
            form: None,
            job: None,
            asked: fontview::CLIENT,
        };
        v.pick(0);
        v
    }

    fn pick(&mut self, i: usize) {
        self.picked = i;
        let Some(face) = self.faces.get(i) else {
            return;
        };
        self.sample = TextRenderer::from_file(&self.path, face.index);
        self.install = install::state(face, user_dir().as_deref());
        self.form = None;
    }

    fn form(&mut self, ctx: &Ctx) -> &mut Form {
        let forms = ctx.forms();
        let paint = forms.paint();
        if self.form.as_ref().is_some_and(|f| f.units() != paint.text) {
            self.form = None;
        }
        self.form.get_or_insert_with(|| {
            let f = fontview::form(&self.faces, self.picked, &self.install);
            f.drawn_with(&paint);
            f
        })
    }

    /// Rebuild the form for a changed face or Install, keeping its focus and pointer but not its
    /// controls, which [`Form::carry`] would take back from the old form.
    fn rebuild(&mut self) {
        let mut f = fontview::form(&self.faces, self.picked, &self.install);
        if let Some(old) = &self.form {
            if let Some(id) = old.focus().and_then(|i| old.items.get(i)).map(|i| i.id) {
                f.focus_id(id);
            }
            f.carry_pointer(old);
        }
        self.form = Some(f);
    }

    fn start_install(&mut self, ctx: &Ctx) -> Reply {
        if !matches!(self.install, Install::No | Install::Failed(_)) || self.faces.is_empty() {
            return Reply::none();
        }
        let Some(dir) = user_dir() else {
            self.install = Install::Failed("no home folder".into());
            self.rebuild();
            return Reply::redraw();
        };
        let file = self.path.clone();
        self.job = Some(Job::spawn(Some(&ctx.wake), move || {
            install::install(&file, &dir)
        }));
        self.install = Install::Running;
        self.rebuild();
        Reply::redraw()
    }

    fn landed(&mut self) -> Reply {
        let Some(done) = self.job.as_ref().and_then(Job::take) else {
            return Reply::none();
        };
        self.job = None;
        self.install = match done {
            Ok(at) => {
                log::info!("installed {}", at.display());
                Install::User
            }
            Err(e) => Install::Failed(e),
        };
        self.rebuild();
        Reply::redraw()
    }

    fn answer(&mut self, ev: form::Event, ctx: &Ctx) -> Reply {
        match ev {
            form::Event::Cancel => Reply {
                quit: true,
                ..Default::default()
            },
            form::Event::Default | form::Event::Pressed(fontview::INSTALL) => {
                self.start_install(ctx)
            }
            form::Event::Changed(fontview::FACE) => {
                let i = self
                    .form
                    .as_ref()
                    .and_then(|f| f.combo_selected(fontview::FACE))
                    .unwrap_or(0);
                if i != self.picked {
                    self.pick(i);
                    self.rebuild();
                }
                Reply::redraw()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    fn dispatch(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        match ev {
            Event::CloseRequest => Reply {
                quit: true,
                ..Default::default()
            },
            Event::Wake => self.landed(),
            ev => {
                self.form(ctx);
                let form = self.form.as_mut().expect("built above");
                match repriseapp::keys::form_input(form, &ev, ctx) {
                    Some(out) => self.answer(out, ctx),
                    None => Reply::none(),
                }
            }
        }
    }

    fn title_of(&self) -> String {
        self.faces
            .get(self.picked)
            .map(FaceNames::title)
            .filter(|t| !t.is_empty())
            .or_else(|| {
                self.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| tr!("title"))
    }
}

impl App for Viewer {
    fn title(&self) -> String {
        self.title_of()
    }

    fn app_id(&self) -> String {
        "reprise.fonts".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.asked
    }

    fn event(&mut self, _win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let mut reply = self.dispatch(ev, ctx);
        let size = ctx.forms().text.size(fontview::CLIENT);
        if size != self.asked {
            self.asked = size;
            reply.size = Some(size);
            reply.redraw = true;
        }
        reply
    }

    fn draw(&mut self, _win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        let p = Paint {
            s: 1,
            ..forms.paint()
        };
        let px = forms.fonts.size_px;
        self.form(ctx);
        let form = self.form.as_ref().expect("built above");
        fontview::draw(c, &p, &ctx.text, px, form, self.sample.as_ref());
    }
}

impl repriseapp::Extended for Viewer {
    fn fixed(&self) -> bool {
        true
    }

    fn caption(&self, _win: WinId) -> repriseapp::Caption {
        repriseapp::Caption::Fixed
    }

    fn reader_form(&mut self, _win: WinId) -> Option<repriseapp::ReaderForm<'_>> {
        let form = repriseapp::DropForm {
            form: self.form.as_mut()?,
            px: None,
        };
        Some(repriseapp::ReaderForm::Form(form))
    }

    fn access(&mut self, _win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let px = ctx.forms().fonts.size_px;
        let title = self.title_of();
        let form = self.form(ctx).clone();
        if let Some(f) =
            fontview::access(tree, reprisekit::access::ROOT, &form, &ctx.text, px, &title)
        {
            tree.set_focus(f);
        }
    }
}

/// Runs the program; `args` are the command line after the program's name: the font file.
pub fn main(args: Vec<std::ffi::OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let Some(path) = args.into_iter().next().map(PathBuf::from) else {
        eprintln!("usage: reprise-fonts FILE");
        std::process::exit(2);
    };
    repriseapp::run(Box::new(Viewer::new(path))).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_is_no_font_shows_no_face_and_no_install() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("broken.ttf");
        std::fs::write(&path, b"not a font").unwrap();
        let v = Viewer::new(path);
        assert!(v.faces.is_empty());
        assert!(v.sample.is_none());
        assert_eq!(v.title_of(), "broken.ttf");
    }

    #[test]
    fn a_landed_install_relabels_the_button_and_keeps_the_focus_off_it() {
        let mut v = Viewer::new(reprisekit::pins::fonts().join("tahoma.ttf"));
        v.install = Install::No;
        v.rebuild();
        v.form.as_mut().unwrap().focus_id(fontview::INSTALL);
        let button = |v: &Viewer| match &v
            .form
            .as_ref()
            .unwrap()
            .item(fontview::INSTALL)
            .unwrap()
            .control
        {
            form::Control::Button { label, enabled, .. } => (label.clone(), *enabled),
            _ => unreachable!(),
        };
        assert_eq!(button(&v), ("&Install".into(), true));
        v.install = Install::Running;
        v.rebuild();
        assert_eq!(button(&v), ("Installing...".into(), false));
        v.install = Install::User;
        v.rebuild();
        assert_eq!(button(&v), ("Installed".into(), false));
        assert_eq!(
            v.form.as_ref().unwrap().focus(),
            None,
            "a greyed button takes no focus"
        );
    }
}
