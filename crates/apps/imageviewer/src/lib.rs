//! Image Viewer: the pictures of a folder one at a time, fitted or zoomed, turned, full screen or
//! as a slideshow, sent to the Recycle Bin or set as the desktop background.

#[cfg(test)]
reprisebus::test_session!();

mod dialogs;
mod folder;
mod input;
mod load;
mod menu;
mod window;

use folder::{Folder, Step};
use load::{Job, Msg, Picture, RenderKey, Timer};
use menu::menus;
use repriseapp::filedlg::Host;
use repriseapp::keys::AltArm;
use repriseapp::oneshot::MsgBox;
use repriseapp::{App, Child, Ctx, Event, Extended, FileDrop, MAIN, Poster, Reply, WinId};
use reprisekit::access::Tree;
use reprisekit::controls::filedlg::{CLIENT, FileDlg};
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::imageview::ImageView;
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::menubar::Bar;
use reprisekit::look::Paint;
use reprisekit::pointer::DoubleClick;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

const NAME: &str = "Image Viewer";
const DELETE: WinId = WinId::named("delete");
const ERRORBOX: WinId = WinId::named("error");
const OPEN: WinId = WinId::named("open");

/// How long each picture of a slideshow shows.
const SLIDE: Duration = Duration::from_secs(3);

/// What the box shows while no picture is decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Blank {
    Nothing,
    Loading,
    Failed(String),
}

struct Viewer {
    folder: Folder,
    picture: Option<Picture>,
    blank: Blank,
    /// Counts opens, so a decode that lands after the next open is dropped.
    generation: u64,
    view: ImageView,
    frame: usize,
    frame_due: Option<Instant>,
    slide_due: Option<Instant>,
    /// The scaled copy the worker made, and the one asked for.
    rendered: Option<(RenderKey, Canvas)>,
    asked: Option<RenderKey>,
    /// The worker and its answers; without them (in tests) jobs run on the spot.
    jobs: Option<Sender<Job>>,
    msgs: Option<Receiver<Msg>>,
    timer: Option<Timer>,
    bar: Bar,
    status_bar: bool,
    /// Full screen as asked of the compositor: no menu bar or status bar, a black ground.
    full: bool,
    /// Device px per logical px of the main window's last draw.
    scale: i32,
    /// Ctrl is held: the wheel zooms.
    ctrl: bool,
    /// Wheel rows not yet a whole notch.
    wheel_rows: i32,
    alt_arm: AltArm,
    clicks: DoubleClick<()>,
    /// The delete prompt, the file it asks about and whether Yes erases it.
    delete: Option<(MsgBox, PathBuf, bool)>,
    error: Option<MsgBox>,
    open: Option<FileDlg>,
    host: Host,
    dialog_text: Rc<TextRenderer>,
    forms: Theme,
    /// The config file Set as Desktop Background writes.
    config: PathBuf,
    trash_icon: Option<Canvas>,
    warn_icon: Option<Canvas>,
}

impl Viewer {
    fn new(path: Option<PathBuf>) -> Viewer {
        let path = path.map(|p| std::path::absolute(&p).unwrap_or(p));
        let mut v = Viewer {
            folder: path.map(Folder::of).unwrap_or_default(),
            picture: None,
            blank: Blank::Nothing,
            generation: 0,
            view: ImageView::new((0, 0)),
            frame: 0,
            frame_due: None,
            slide_due: None,
            rendered: None,
            asked: None,
            jobs: None,
            msgs: None,
            timer: None,
            bar: Bar::new(menus()),
            status_bar: true,
            full: false,
            scale: 1,
            ctrl: false,
            wheel_rows: 0,
            alt_arm: AltArm::default(),
            clicks: DoubleClick::default(),
            delete: None,
            error: None,
            open: None,
            host: Host::default(),
            dialog_text: repriseapp::start_text(),
            forms: Theme::default(),
            config: repriseconf::config_path(),
            trash_icon: None,
            warn_icon: None,
        };
        v.sync_menus();
        v
    }

    fn title(&self) -> String {
        match self.folder.current().and_then(Path::file_name) {
            Some(n) => format!("{} - {NAME}", n.to_string_lossy()),
            None => NAME.into(),
        }
    }

    /// Hand `job` to the worker, or run it here without one.
    fn send(&mut self, job: Job) {
        match &self.jobs {
            Some(tx) => {
                let _ = tx.send(job);
            }
            None => {
                let msg = load::run(job);
                self.handle(msg);
            }
        }
    }

    /// Decode the current picture, and list its folder when that is another one.
    fn load_current(&mut self) {
        self.generation += 1;
        self.picture = None;
        self.rendered = None;
        self.asked = None;
        self.frame_due = None;
        let Some(path) = self.folder.current().map(Path::to_path_buf) else {
            self.blank = Blank::Nothing;
            self.arm();
            return;
        };
        self.blank = Blank::Loading;
        let generation = self.generation;
        self.send(Job::Open { generation, path });
    }

    fn list_folder(&mut self) {
        if let Some(dir) = self.folder.current().and_then(Path::parent) {
            let dir = dir.to_path_buf();
            self.send(Job::List(dir));
        }
    }

    /// Show `path` and the pictures beside it.
    fn show(&mut self, path: PathBuf) {
        self.folder = Folder::of(path);
        self.load_current();
        self.list_folder();
    }

    fn handle(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Opened {
                generation,
                picture,
            } => {
                if generation != self.generation {
                    return false;
                }
                match picture {
                    Ok(p) => {
                        self.view.reset(p.size());
                        self.frame = 0;
                        self.frame_due =
                            p.animated().then(|| Instant::now() + delay(p.frames[0].1));
                        self.picture = Some(p);
                    }
                    Err(e) => self.blank = Blank::Failed(e),
                }
                self.arm();
            }
            Msg::Listed { dir, pictures } => {
                if self.folder.current().and_then(Path::parent) != Some(dir.as_path()) {
                    return false;
                }
                self.folder.listed(pictures);
            }
            Msg::Rendered { key, canvas } => {
                if self.asked != Some(key) {
                    return false;
                }
                self.rendered = Some((key, canvas));
            }
            Msg::Fate { path, mode } => return self.fated(path, mode),
            Msg::Deleted { path, result } => self.deleted(&path, result),
            Msg::Due => self.due(Instant::now()),
        }
        true
    }

    /// The timer's deadline passed: the slideshow steps and a moving picture shows its next frame.
    fn due(&mut self, now: Instant) {
        if self.slide_due.is_some_and(|at| at <= now) {
            self.slide_due = Some(now + SLIDE);
            if self.folder.step(Step::Next) {
                self.load_current();
            }
        }
        if let (Some(at), Some(p)) = (self.frame_due, &self.picture)
            && at <= now
        {
            self.frame = (self.frame + 1) % p.frames.len();
            self.frame_due = Some(now + delay(p.frames[self.frame].1));
        }
        self.arm();
    }

    /// Point the timer at the next of the slide's and the frame's deadlines.
    fn arm(&self) {
        if let Some(t) = &self.timer {
            let next = match (self.slide_due, self.frame_due) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            t.set(next);
        }
    }

    /// Step along the folder and open what lands there.
    fn go(&mut self, step: Step) -> Reply {
        if !self.folder.step(step) {
            return Reply::none();
        }
        if self.slide_due.is_some() {
            self.slide_due = Some(Instant::now() + SLIDE);
        }
        self.load_current();
        Reply::redraw()
    }

    fn full_screen(&mut self, on: bool) -> Reply {
        self.full = on;
        if !on {
            self.slide_due = None;
            self.arm();
        }
        self.bar.close();
        Reply {
            fullscreen: Some(on),
            ..Reply::redraw()
        }
    }

    /// Start a slideshow, full screen, or end it.
    fn slideshow(&mut self, on: bool) -> Reply {
        self.slide_due = on.then(|| Instant::now() + SLIDE);
        self.arm();
        if on {
            self.full_screen(true)
        } else {
            Reply::redraw()
        }
    }

    fn copy(&self) -> Reply {
        let Some(path) = self.folder.current() else {
            return Reply::none();
        };
        Reply {
            copy_files: Some(reprisefs::fileclip::FileClip {
                paths: vec![path.to_path_buf()],
                cut: false,
            }),
            ..Reply::none()
        }
    }

    /// Point `[wallpaper] image` at the current picture, ending a wallpaper slideshow.
    fn set_background(&mut self) -> Reply {
        let Some(path) = self.folder.current().map(Path::to_path_buf) else {
            return Reply::none();
        };
        if let Err(e) = set_wallpaper(&self.config, &path) {
            log::warn!("set {} as the background: {e}", path.display());
            self.error = Some(self.error_box(&format!(
                "The picture could not be set as the desktop background.\n\n{e}"
            )));
            return Reply::redraw_all();
        }
        Reply::none()
    }

    /// Fetch the icons the dialogs show while an event has the cache.
    fn prime(&mut self, ctx: &mut Ctx) {
        self.forms = ctx.forms();
        self.dialog_text = ctx.text.clone();
        if self.trash_icon.is_none() {
            self.trash_icon = ctx.icons.get("user-trash-full", 32);
        }
        if self.warn_icon.is_none() {
            self.warn_icon = ctx.icons.get("dialog-warning", 32);
        }
        self.host.icons.prime(&mut ctx.icons);
    }

    fn drain(&mut self) -> bool {
        let Some(rx) = self.msgs.take() else {
            return false;
        };
        let mut changed = false;
        while let Ok(m) = rx.try_recv() {
            changed |= self.handle(m);
        }
        self.msgs = Some(rx);
        changed
    }
}

/// How long a frame shows; a still picture's 0 never comes due.
fn delay(ms: u32) -> Duration {
    Duration::from_millis(u64::from(ms.max(1)))
}

const IMAGE: repriseconf::Key = repriseconf::key("wallpaper", "image");
const SLIDESHOW: repriseconf::Key = repriseconf::key("wallpaper", "slideshow");

/// Set `path` as the desktop's picture in the config at `config`, ending a slideshow that would
/// show in its place; the compositor picks the change up from the file.
fn set_wallpaper(config: &Path, path: &Path) -> Result<(), String> {
    let image = path.to_str().ok_or("The picture's path is not UTF-8.")?;
    repriseconf::edit(config, |text| {
        let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
        repriseconf::set(&mut doc, &IMAGE, toml_edit::Value::from(image))
            .map_err(|e| e.to_string())?;
        repriseconf::remove(&mut doc, &SLIDESHOW);
        Ok::<_, String>(doc.to_string())
    })
    .map_err(|e| e.to_string())?
    .map(drop)
}

impl App for Viewer {
    fn title(&self) -> String {
        Viewer::title(self)
    }
    fn app_id(&self) -> String {
        "reprise.imageviewer".into()
    }
    fn initial_size(&self) -> (i32, i32) {
        (720, 540)
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        let (post, rx) = Poster::channel(waker);
        self.jobs = Some(load::spawn(post.clone()));
        self.timer = Some(Timer::spawn(post));
        self.msgs = Some(rx);
        self.load_current();
        self.list_folder();
        repriseapp::Start::Continue
    }

    fn children(&self) -> Vec<Child> {
        let mut kids = Vec::new();
        if let Some((b, _, _)) = &self.delete {
            kids.push(b.child(DELETE, &self.dialog_text, &self.forms));
        }
        if let Some(b) = &self.error {
            kids.push(b.child(ERRORBOX, &self.dialog_text, &self.forms));
        }
        if let Some(d) = &self.open {
            kids.push(Child {
                id: OPEN,
                title: "Open".into(),
                size: d.client(),
                fixed: false,
                modal: true,
            });
        }
        kids
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.prime(ctx);
        if win != MAIN {
            self.alt_arm.interrupt();
        }
        let reply = match win {
            DELETE => self.delete_event(ev),
            ERRORBOX => self.error_event(ev),
            OPEN => self.open_event(ev, ctx),
            _ => self.main_event(ev, ctx),
        };
        self.sync_menus();
        reply
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        match win {
            DELETE => {
                if let Some((b, _, _)) = &self.delete {
                    b.draw(c, &self.forms, &self.dialog_text);
                }
            }
            ERRORBOX => {
                if let Some(b) = &self.error {
                    b.draw(c, &self.forms, &self.dialog_text);
                }
            }
            OPEN => {
                if let Some(d) = &self.open {
                    let (w, h) = d.client();
                    let paint = Paint {
                        s: 1,
                        ..self.forms.paint()
                    };
                    d.draw(
                        c,
                        &self.dialog_text,
                        &paint,
                        Rect::new(0, 0, w, h),
                        self.forms.fonts.size_px,
                    );
                }
            }
            _ => self.draw_main(c, ctx),
        }
    }
}

impl Extended for Viewer {
    fn follow_scale(&mut self, win: WinId, scale: i32, _ctx: &mut Ctx) {
        if win == MAIN {
            self.scale = scale.max(1);
        }
    }

    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        (win == OPEN).then(|| self.open.as_ref().map_or(CLIENT, FileDlg::min_client))
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

    fn drop_file_dialog(&mut self, win: WinId) -> Option<FileDrop<'_>> {
        if win != OPEN {
            return None;
        }
        Some(FileDrop {
            dlg: self.open.as_mut()?,
            host: &mut self.host,
        })
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let (dt, forms) = (&self.dialog_text, &self.forms);
        match win {
            DELETE => {
                if let Some((b, _, _)) = &self.delete {
                    b.access(tree, dt, forms);
                }
            }
            ERRORBOX => {
                if let Some(b) = &self.error {
                    b.access(tree, dt, forms);
                }
            }
            OPEN => {
                if let Some(d) = &self.open {
                    let (w, h) = d.client();
                    d.access(tree, Rect::new(0, 0, w, h), dt, forms.fonts.size_px);
                }
            }
            _ => self.access_main(tree, ctx),
        }
    }
}

/// Runs the program; `args` are the command line after the program's name: the picture to show.
pub fn main(args: Vec<std::ffi::OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let path = args.into_iter().next().map(PathBuf::from);
    let mut viewer = Viewer::new(path);
    viewer.forms = repriseapp::start_forms();
    repriseapp::run(Box::new(viewer)).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests;
