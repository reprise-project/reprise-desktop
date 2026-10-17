//! Volume Control: the mixer window. A menu bar over one column per line
//! (the output device and each program playing sound, or each recording
//! device) with balance, volume and Mute or Select, and a status bar naming
//! the device. Options opens Properties and turns Advanced Controls on, which
//! puts an Advanced button under the first column.
//!
//! The window takes the size its columns want ([`fit`]). `--record` opens
//! the Recording view. Writes go out at once through `repriseaudio`; a volume
//! or balance write sets each channel, so balance survives a volume change.
//! Hidden columns and Advanced Controls persist ([`state`]).

mod dialogs;
mod fit;
mod state;
mod view;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use repriseapp::keys::{AltArm, form_input};
use repriseapp::oneshot;
use repriseapp::{App, BTN_LEFT, Child, Ctx, Event, KeyInput, PointerInput, Reply, WinId};
use repriseaudio::Snapshot;
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::aboutbox;
use reprisekit::controls::form::{self, Form};
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::menubar::{self, Action, Bar, Key};
use reprisekit::controls::mixer::{self, Mixer};
use reprisekit::controls::statusbar::{self, Panel, StatusBar};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::keysym;
use reprisekit::theme::{TextUnits, Theme};

use state::State;
use view::{Options, Recent, View};

const PROPS_WIN: WinId = "properties";
const ADVANCED_WIN: WinId = "advanced";
const ABOUT_WIN: WinId = "about";
const ICON: &str = "multimedia-volume-control";
/// The speaker at the Balance sliders' ends, the left one flipped.
const SPEAKER: &str = "audio-volume-high";

mod title {
    pub const OPTIONS: usize = 0;
    pub const HELP: usize = 1;
}

mod row {
    pub const PROPERTIES: usize = 0;
    pub const ADVANCED: usize = 1;
    pub const EXIT: usize = 3;
    pub const ABOUT: usize = 2;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slid {
    Volume,
    Balance,
}

#[derive(Clone, Debug, PartialEq)]
enum Level {
    Volume(f64),
    Levels(Vec<f64>),
}

/// The write a moved slider makes on a node with channels `ch`. Only the
/// moved slider is read: a volume keeps the node's balance and vice versa,
/// so neither snaps to the other's steps. A node with no left and right
/// takes no balance.
fn level_write(ch: &repriseaudio::Channels, slid: Slid, vpos: usize, bpos: usize) -> Option<Level> {
    match slid {
        Slid::Volume => Some(Level::Volume(view::volume_of(vpos))),
        Slid::Balance if ch.stereo() => {
            Some(Level::Levels(ch.at(ch.volume(), mixer::balance_of(bpos))))
        }
        Slid::Balance => None,
    }
}

fn menus(advanced: bool) -> Vec<(&'static str, Vec<MenuItem>)> {
    vec![
        (
            "&Options",
            vec![
                MenuItem::item("&Properties"),
                MenuItem::item("&Advanced Controls").checked(advanced),
                MenuItem::Separator,
                MenuItem::item("E&xit"),
            ],
        ),
        (
            "&Help",
            vec![
                MenuItem::item("&Help Topics").enabled(false),
                MenuItem::Separator,
                MenuItem::item("&About Volume Control"),
            ],
        ),
    ]
}

struct Volume {
    opts: Options,
    snapshots: Option<repriseaudio::Watch>,
    snapshot: Snapshot,
    mixer: Mixer,
    recent: Recent,
    bar: Bar,
    alt_arm: AltArm,
    props: Option<dialogs::Props>,
    advanced: Option<Form>,
    about: bool,
    app_icon: Option<Canvas>,
    dialog_text: Rc<TextRenderer>,
    dialog_px: u32,
    /// The forms theme at the last event, which `children` sizes the dialogs with.
    forms: Theme,
    waker: Option<repriseapp::Waker>,
    /// A refused Select or Mute: the boxes go back to the snapshot at once, as no update will come.
    refused: Receiver<()>,
    refused_tx: Sender<()>,
    state_path: Option<PathBuf>,
    room: Option<i32>,
    /// Client size at the last event; a dialog's OK re-lays the columns out in it.
    main_size: (i32, i32),
}

impl Volume {
    fn new(view: View, snapshot: Snapshot, state: State) -> Volume {
        let opts = Options {
            hidden: state.hidden,
            advanced: state.advanced,
            ..Options::new(view)
        };
        let lines = view::lines(&snapshot, &opts, &Recent::default());
        let (refused_tx, refused) = mpsc::channel();
        let mixer = Mixer::new(&lines, opts.advanced);
        let main_size = fit::window(&mixer, &Theme::default(), None);
        Volume {
            mixer,
            bar: Bar::new(menus(opts.advanced)),
            opts,
            snapshots: None,
            snapshot,
            recent: Recent::default(),
            alt_arm: AltArm::default(),
            props: None,
            advanced: None,
            about: false,
            app_icon: None,
            dialog_text: repriseapp::start_text(),
            dialog_px: Theme::default().fonts.size_px,
            forms: Theme::default(),
            waker: None,
            refused,
            refused_tx,
            state_path: None,
            room: None,
            main_size,
        }
    }

    fn kept(&self) -> State {
        State {
            hidden: self.opts.hidden.clone(),
            advanced: self.opts.advanced,
        }
    }

    fn save(&self) {
        if let Some(path) = &self.state_path {
            self.kept().save(path);
        }
    }

    fn refit(&mut self, theme: &Theme) -> Option<(i32, i32)> {
        let want = fit::window(&self.mixer, theme, self.room);
        (want != self.main_size).then(|| {
            self.main_size = want;
            want
        })
    }

    fn body(&self, ctx: &Ctx) -> Rect {
        let top = menubar::bar_h(&ctx.theme.controls());
        let (w, h) = self.main_size;
        Rect::new(0, top, w, h - top - fit::status_h(&ctx.theme))
    }

    fn mixer_layout(&self, ctx: &Ctx) -> mixer::Layout {
        mixer::layout(&self.mixer, &ctx.theme, &ctx.text, self.body(ctx))
    }

    fn bar_layout(&self, ctx: &Ctx) -> menubar::Layout {
        self.bar.layout(
            &ctx.text,
            ctx.theme.fonts.size_px,
            Rect::new(0, 0, ctx.size.0, ctx.size.1),
            &ctx.theme.controls(),
        )
    }

    /// One panel naming the device, text 2 px in, no grip; the bar's last row of
    /// face falls below the window.
    fn status(&self, ctx: &Ctx) -> (StatusBar, Rect) {
        let bar = StatusBar {
            panels: vec![Panel {
                text: view::status(&self.snapshot, &self.opts),
                width: 0,
                pad: statusbar::EXPLORER_TEXT_PAD,
                icon: None,
            }],
            grip: false,
        };
        let h = statusbar::height(&ctx.theme.controls());
        let r = Rect::new(0, ctx.size.1 - fit::status_h(&ctx.theme), ctx.size.0, h);
        (bar, r)
    }

    fn resync(&mut self, ctx: &Ctx) -> Option<(i32, i32)> {
        let lines = view::lines(&self.snapshot, &self.opts, &self.recent);
        let lay = self.mixer_layout(ctx);
        self.mixer.sync(&lay, &lines);
        let size = self.refit(&ctx.theme);
        // A line that came or went moves the columns: sync again on the new layout
        // so the part under the pointer is found on it.
        let lay = self.mixer_layout(ctx);
        self.mixer.sync(&lay, &lines);
        size
    }

    fn set_options(&mut self, opts: Options, ctx: &Ctx) -> Option<(i32, i32)> {
        let view_changed = opts.view != self.opts.view;
        let before = self.kept();
        self.opts = opts;
        self.bar = Bar::new(menus(self.opts.advanced));
        self.mixer.set_advanced(self.opts.advanced);
        let size = if view_changed {
            let lines = view::lines(&self.snapshot, &self.opts, &self.recent);
            let speaker = self.mixer.speaker.take();
            self.mixer = Mixer::new(&lines, self.opts.advanced);
            self.mixer.speaker = speaker;
            self.refit(&ctx.theme)
        } else {
            self.resync(ctx)
        };
        if self.kept() != before {
            self.save();
        }
        size
    }

    fn wake(&mut self, ctx: &Ctx) -> Reply {
        let mut fresh = None;
        if let Some(rx) = &self.snapshots {
            while let Ok(s) = rx.try_recv() {
                fresh = Some(s);
            }
        }
        let refused = self.refused.try_iter().count() > 0;
        let moved = fresh.is_some();
        if let Some(s) = fresh {
            self.snapshot = s;
        }
        Reply {
            size: (moved || refused).then(|| self.resync(ctx)).flatten(),
            ..Reply::redraw_all()
        }
    }

    fn mixer_outcome(&mut self, ev: mixer::Event, ctx: &Ctx) -> Reply {
        use mixer::Event as E;
        match ev {
            E::None => Reply::none(),
            E::Redraw => Reply::redraw(),
            E::Volume(key, _) => {
                self.write_level(&key, Slid::Volume);
                Reply::redraw()
            }
            E::Balance(key, _) => {
                self.write_level(&key, Slid::Balance);
                Reply::redraw()
            }
            E::Check(key, on) => Reply {
                size: match self.write_check(&key, on) {
                    true => None,
                    false => self.resync(ctx),
                },
                ..Reply::redraw()
            },
            E::Advanced => {
                self.advanced = Some(dialogs::advanced(self.app_icon.clone()));
                Reply::redraw_all()
            }
        }
    }

    fn write_level(&mut self, key: &str, slid: Slid) {
        let Some(col) = self.mixer.columns.iter().find(|c| c.key == key) else {
            return;
        };
        let (vpos, bpos) = (col.volume.pos, col.balance.pos);
        self.recent
            .note(key.to_string(), (vpos, bpos), Instant::now());
        for (id, ch) in view::nodes(&self.snapshot, &self.opts, key) {
            match level_write(ch, slid, vpos, bpos) {
                Some(Level::Volume(v)) => repriseaudio::set_volume(&self.snapshot, id, v),
                Some(Level::Levels(l)) => repriseaudio::set_levels(&self.snapshot, id, &l),
                None => continue,
            };
        }
    }

    /// Returns whether every write went out; a refusal comes back through `refused`.
    fn write_check(&mut self, key: &str, on: bool) -> bool {
        let nodes = view::nodes(&self.snapshot, &self.opts, key);
        if key.starts_with("source:") {
            return nodes
                .first()
                .is_some_and(|(id, _)| repriseaudio::set_default_then(*id, self.on_refusal()));
        }
        nodes
            .iter()
            .all(|(id, _)| repriseaudio::set_mute_then(*id, on, self.on_refusal()))
    }

    fn on_refusal(&self) -> impl FnOnce(bool) + Send + 'static {
        let tx = self.refused_tx.clone();
        let waker = self.waker.clone();
        move |took| {
            if !took
                && tx.send(()).is_ok()
                && let Some(w) = waker
            {
                w.wake();
            }
        }
    }

    fn command(&mut self, t: usize, r: usize, ctx: &Ctx) -> Reply {
        match (t, r) {
            (title::OPTIONS, row::PROPERTIES) => {
                self.props = Some(dialogs::Props::new(&self.snapshot, &self.opts));
                Reply::redraw_all()
            }
            (title::OPTIONS, row::ADVANCED) => {
                let mut o = self.opts.clone();
                o.advanced = !o.advanced;
                Reply {
                    size: self.set_options(o, ctx),
                    ..Reply::redraw()
                }
            }
            (title::OPTIONS, row::EXIT) => Reply {
                quit: true,
                ..Reply::none()
            },
            (title::HELP, row::ABOUT) => {
                self.about = true;
                Reply::redraw_all()
            }
            _ => Reply::redraw(),
        }
    }

    fn key(&mut self, k: KeyInput, ctx: &Ctx) -> Reply {
        let lay = self.bar_layout(ctx);
        let m = &ctx.theme;
        let px = ctx.theme.fonts.size_px;
        if k.sym == keysym::ALT_L || k.sym == keysym::ALT_R {
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
        let Some(key) = repriseapp::keys::dialog_key(&k) else {
            return Reply::none();
        };
        let key = match key {
            form::Key::Edit(e) => match form::flat_key(&e) {
                Some(k) => k,
                None => return Reply::none(),
            },
            k => k,
        };
        let ml = self.mixer_layout(ctx);
        let ev = self.mixer.key(&ml, key);
        self.mixer_outcome(ev, ctx)
    }

    fn key_up(&mut self, k: KeyInput, ctx: &Ctx) -> Reply {
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
        if matches!(p, PointerInput::Press { .. }) {
            self.alt_arm.interrupt();
        }
        let lay = self.bar_layout(ctx);
        let m = &ctx.theme;
        let px = ctx.theme.fonts.size_px;
        let ml = self.mixer_layout(ctx);
        let ev = match p {
            PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            } => {
                match self.bar.press(x, y, &lay, &ctx.text, m, px) {
                    Action::Chosen { title, row, .. } => return self.command(title, row, ctx),
                    Action::Redraw => return Reply::redraw(),
                    Action::Ignored => {}
                }
                let (sb, r) = self.status(ctx);
                if statusbar::hit_grip(&sb, r, x, y) {
                    return Reply::resize();
                }
                self.mixer.press(&ml, x, y)
            }
            PointerInput::Motion { x, y } => {
                if self.bar.open.is_some() {
                    self.bar.motion(x, y, &lay, &ctx.text, m, px);
                    return Reply::redraw();
                }
                let title = self.bar.hover(x, y, &lay) && ctx.styled();
                match self.mixer.motion(&ml, x, y) {
                    mixer::Event::None if title => mixer::Event::Redraw,
                    ev => ev,
                }
            }
            PointerInput::Release {
                button: BTN_LEFT, ..
            } => self.mixer.release(),
            PointerInput::Leave => {
                let title = self.bar.leave() && ctx.styled();
                match self.mixer.leave() {
                    mixer::Event::None if title => mixer::Event::Redraw,
                    ev => ev,
                }
            }
            PointerInput::Scroll { x, y, rows } => self.mixer.scroll(&ml, x, y, rows),
            _ => mixer::Event::None,
        };
        self.mixer_outcome(ev, ctx)
    }

    fn props_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        if matches!(ev, Event::CloseRequest) {
            self.props = None;
            return Reply::redraw_all();
        }
        let Some(p) = &mut self.props else {
            return Reply::none();
        };
        let Some(out) = form_input(&mut p.form, &ev, ctx) else {
            return Reply::none();
        };
        match out {
            form::Event::Changed(id) => {
                p.changed(id, &self.snapshot);
                Reply::redraw()
            }
            form::Event::Default | form::Event::Pressed(dialogs::OK) => {
                let o = p.options(&self.opts);
                self.props = None;
                Reply {
                    size: self.set_options(o, ctx),
                    ..Reply::redraw_all()
                }
            }
            form::Event::Cancel | form::Event::Pressed(dialogs::CANCEL) => {
                self.props = None;
                Reply::redraw_all()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    fn advanced_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        if matches!(ev, Event::CloseRequest) {
            self.advanced = None;
            return Reply::redraw_all();
        }
        let Some(f) = &mut self.advanced else {
            return Reply::none();
        };
        match form_input(f, &ev, ctx) {
            Some(
                form::Event::Default | form::Event::Cancel | form::Event::Pressed(dialogs::CLOSE),
            ) => {
                self.advanced = None;
                Reply::redraw_all()
            }
            Some(form::Event::None) | None => Reply::none(),
            Some(_) => Reply::redraw(),
        }
    }

    fn about_fields(&self) -> aboutbox::Fields {
        repriseapp::about::fields("Volume Control", self.app_icon.clone())
    }

    /// The about box's theme: the forms theme with its font at `dialog_px` grown.
    fn about_theme(&self, forms: &Theme) -> Theme {
        let mut t = forms.clone();
        t.fonts.size_px = t.text.px(self.dialog_px);
        t
    }

    /// The about box laid out at 100%, its banner drawn at the grown size.
    fn about_form(&self, u: TextUnits) -> Form {
        let text = &self.dialog_text;
        let mut f = aboutbox::form(&self.about_fields(), text, self.dialog_px);
        if !u.is_one() {
            let w = u.len(aboutbox::CLIENT.0);
            f.set_picture("banner", aboutbox::banner_wide(text, self.dialog_px, w, u));
        }
        f
    }

    fn about_event(&mut self, ev: Event) -> Reply {
        let theme = self.about_theme(&self.forms);
        let step = oneshot::event(&ev, &self.dialog_text, &theme, || {
            self.about_form(theme.text)
        });
        match step {
            oneshot::Step::Closed
            | oneshot::Step::Out(
                form::Event::Cancel | form::Event::Default | form::Event::Pressed(aboutbox::OK),
            ) => {
                self.about = false;
                Reply::redraw_all()
            }
            oneshot::Step::None | oneshot::Step::Out(form::Event::None) => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    fn draw_main(&self, c: &mut Canvas, ctx: &Ctx) {
        let p = Paint {
            s: 1,
            ..ctx.theme.paint()
        };
        c.fill_rect(c.bounds(), p.cs.button_face);
        let ml = self.mixer_layout(ctx);
        mixer::draw(c, &ctx.theme, &ctx.text, &self.mixer, &ml);
        let (sb, r) = self.status(ctx);
        statusbar::draw(c, &ctx.text, &p, &sb, r, ctx.theme.fonts.size_px);
        let lay = self.bar_layout(ctx);
        self.bar.draw(c, &ctx.text, &ctx.theme, &lay);
    }

    fn draw_form(c: &mut Canvas, forms: &Theme, f: &Form, text: &TextRenderer) {
        let p = Paint {
            s: 1,
            ..forms.paint()
        };
        c.fill_rect(c.bounds(), p.cs.button_face);
        form::draw(c, text, &p, f, true, forms.fonts.size_px);
    }
}

impl App for Volume {
    fn title(&self) -> String {
        self.opts.view.title().into()
    }

    fn app_id(&self) -> String {
        "reprise.volume".into()
    }

    /// Wide enough for the view's lines, tall enough for columns, menu bar and
    /// status line, in Classic's metrics until the first event brings the look's.
    fn initial_size(&self) -> (i32, i32) {
        self.main_size
    }

    fn fixed(&self) -> bool {
        true
    }

    fn text_sized(&self) -> bool {
        true
    }

    fn children(&self) -> Vec<Child> {
        let mut kids = Vec::new();
        if self.props.is_some() {
            kids.push(Child {
                id: PROPS_WIN,
                title: "Properties".into(),
                size: self.forms.text.size(dialogs::PROPS),
                fixed: true,
                modal: true,
            });
        }
        if self.advanced.is_some() {
            kids.push(Child {
                id: ADVANCED_WIN,
                title: format!(
                    "Advanced Controls for {}",
                    self.mixer
                        .columns
                        .first()
                        .map_or(view::MASTER_TITLE, |c| c.title.as_str())
                ),
                size: self.forms.text.size(dialogs::ADVANCED),
                fixed: true,
                modal: true,
            });
        }
        if self.about {
            kids.push(Child {
                id: ABOUT_WIN,
                title: "About Volume Control".into(),
                size: self.forms.text.size(aboutbox::CLIENT),
                fixed: true,
                modal: true,
            });
        }
        kids
    }

    fn start(&mut self, waker: repriseapp::Waker) {
        self.waker = Some(waker.clone());
        self.snapshots = Some(repriseaudio::watch(move || waker.wake()));
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.dialog_text = ctx.text.clone();
        if self.app_icon.is_none() {
            self.app_icon = ctx.icons.get(ICON, 32);
        }
        if self.mixer.speaker.is_none() {
            self.mixer.speaker = ctx.icons.get(SPEAKER, 16);
        }
        self.forms = ctx.forms();
        if matches!(ev, Event::Wake) {
            return self.wake(ctx);
        }
        if win == repriseapp::MAIN {
            self.main_size = ctx.size;
        } else {
            self.alt_arm.interrupt();
        }
        match win {
            PROPS_WIN => self.props_event(ev, ctx),
            ADVANCED_WIN => self.advanced_event(ev, ctx),
            ABOUT_WIN => self.about_event(ev),
            _ => match ev {
                Event::Key(k) => self.key(k, ctx),
                Event::KeyUp(k) => self.key_up(k, ctx),
                Event::Pointer(p) => self.pointer(p, ctx),
                Event::Resized => Reply {
                    size: self.refit(&ctx.theme),
                    ..Reply::redraw()
                },
                Event::Output((w, _)) => {
                    self.room = Some(w);
                    Reply {
                        size: self.refit(&ctx.theme),
                        ..Reply::none()
                    }
                }
                Event::Focus(false) => {
                    self.alt_arm.interrupt();
                    Reply::redraw()
                }
                Event::CloseRequest => Reply {
                    quit: true,
                    ..Reply::none()
                },
                _ => Reply::none(),
            },
        }
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.dialog_text = ctx.text.clone();
        let forms = ctx.forms();
        match win {
            PROPS_WIN => {
                if let Some(p) = &self.props {
                    Volume::draw_form(c, &forms, &p.form, &ctx.text);
                }
            }
            ADVANCED_WIN => {
                if let Some(f) = &self.advanced {
                    Volume::draw_form(c, &forms, f, &ctx.text);
                }
            }
            ABOUT_WIN => {
                let theme = self.about_theme(&forms);
                let f = self.about_form(theme.text);
                Volume::draw_form(c, &theme, &f, &self.dialog_text);
            }
            _ => self.draw_main(c, ctx),
        }
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        let (dt, dpx) = (&ctx.text, forms.fonts.size_px);
        match win {
            PROPS_WIN => {
                if let Some(p) = &self.props {
                    oneshot::access_form(&p.form, tree, dt, dpx);
                }
            }
            ADVANCED_WIN => {
                if let Some(f) = &self.advanced {
                    oneshot::access_form(f, tree, dt, dpx);
                }
            }
            ABOUT_WIN => {
                let theme = self.about_theme(&forms);
                let f = self.about_form(theme.text);
                f.drawn_with(&theme.paint());
                oneshot::access_form(&f, tree, &self.dialog_text, theme.fonts.size_px);
            }
            _ => {
                let menus = self.bar.access(&self.bar_layout(ctx), tree, ROOT);
                let focus = self.mixer.access(&self.mixer_layout(ctx), tree, ROOT);
                let (sb, r) = self.status(ctx);
                statusbar::access(&sb, r, ctx.theme.text, tree, ROOT);
                tree.set_focus(menus.unwrap_or(focus));
            }
        }
    }
}

fn main() {
    reprisebus::journal::init_logger("error");
    let view = if std::env::args().skip(1).any(|a| a == "--record") {
        View::Recording
    } else {
        View::Playback
    };
    let snapshot = repriseaudio::snapshot_now();
    let path = State::path();
    let mut app = Volume::new(view, snapshot, State::load(&path));
    app.state_path = Some(path);
    repriseapp::run(Box::new(app));
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::theme::FontConfig;

    fn label_at(t: usize, r: usize) -> String {
        match &menus(false)[t].1[r] {
            MenuItem::Entry { label, .. } => label.clone(),
            MenuItem::Separator => String::new(),
        }
    }

    #[test]
    fn the_menu_rows_the_commands_name() {
        assert_eq!(label_at(title::OPTIONS, row::PROPERTIES), "Properties");
        assert_eq!(label_at(title::OPTIONS, row::ADVANCED), "Advanced Controls");
        assert_eq!(label_at(title::OPTIONS, row::EXIT), "Exit");
        assert_eq!(label_at(title::HELP, row::ABOUT), "About Volume Control");
    }

    fn test_ctx() -> Ctx {
        Ctx {
            theme: Theme::default(),
            text: TextRenderer::load(&FontConfig::default()).into(),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: (300, 300),
            maximized: false,
            origin: (0, 0),
            wake: repriseapp::Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            at_pick: false,
            text_size: 100,
        }
    }

    fn device(id: u32, name: &str) -> repriseaudio::Device {
        repriseaudio::Device {
            id,
            name: name.into(),
            label: name.into(),
            volume: 0.5,
            mute: false,
            card: None,
            channels: Default::default(),
            route: None,
        }
    }

    fn stereo(l: f64, r: f64) -> repriseaudio::Channels {
        repriseaudio::Channels {
            map: vec!["FL".into(), "FR".into()],
            levels: vec![l, r],
            shares: Vec::new(),
        }
    }

    #[test]
    fn a_slider_write_leaves_the_other_axis_as_the_node_has_it() {
        let ch = stereo(0.8, 0.76);
        assert_eq!(
            level_write(&ch, Slid::Volume, 45, mixer::BALANCE_CENTRE),
            Some(Level::Volume(0.75)),
            "a volume write keeps each channel's share"
        );
        let Some(Level::Levels(l)) = level_write(&stereo(0.83, 0.83), Slid::Balance, 0, 15) else {
            panic!("a stereo node takes the balance");
        };
        assert!(
            (l[0] - 0.415).abs() < 1e-9 && (l[1] - 0.83).abs() < 1e-9,
            "{l:?}"
        );
        let mono = repriseaudio::Channels {
            map: vec!["MONO".into()],
            levels: vec![0.5],
            shares: Vec::new(),
        };
        assert_eq!(level_write(&mono, Slid::Balance, 10, 15), None);
    }

    #[test]
    fn a_refused_select_goes_back_at_once() {
        let snap = Snapshot {
            sources: vec![device(50, "mic"), device(51, "line")],
            default_source: Some("line".into()),
            ..Snapshot::default()
        };
        let mut v = Volume::new(View::Recording, snap, State::default());
        let ctx = test_ctx();
        let lay = v.mixer_layout(&ctx);
        v.mixer.focus = mixer::Focus::Check(0);
        assert_eq!(
            v.mixer.key(&lay, form::Key::Space),
            mixer::Event::Check(view::source_key("mic"), true)
        );
        assert!(v.mixer.columns[0].checked, "the box ticks when pressed");
        v.refused_tx.send(()).unwrap();
        v.wake(&ctx);
        let ticked: Vec<bool> = v.mixer.columns.iter().map(|c| c.checked).collect();
        assert_eq!(ticked, [false, true], "the default source keeps Select");
    }

    #[test]
    fn the_kept_columns_and_advanced_controls_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("volume.toml");
        let snap = Snapshot {
            sinks: vec![device(40, "speakers")],
            apps: vec![repriseaudio::App {
                key: "mpv".into(),
                label: "mpv".into(),
                icon_hints: Vec::new(),
                binary: None,
                streams: Vec::new(),
                volume: 0.5,
                mute: false,
            }],
            ..Snapshot::default()
        };
        let mut v = Volume::new(View::Playback, snap.clone(), State::load(&path));
        v.state_path = Some(path.clone());
        let mut o = v.opts.clone();
        o.hidden.insert(view::app_key("mpv"));
        v.set_options(o, &test_ctx());
        v.command(title::OPTIONS, row::ADVANCED, &test_ctx());

        let back = Volume::new(View::Playback, snap, State::load(&path));
        assert!(back.opts.advanced, "Advanced Controls stays on");
        let keys: Vec<&str> = back.mixer.columns.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, [view::MASTER], "mpv's column stays hidden");
        assert!(back.mixer.advanced);
    }

    fn playing(keys: &[&str]) -> Vec<repriseaudio::App> {
        keys.iter()
            .map(|k| repriseaudio::App {
                key: (*k).into(),
                label: (*k).into(),
                icon_hints: Vec::new(),
                binary: None,
                streams: Vec::new(),
                volume: 0.5,
                mute: false,
            })
            .collect()
    }

    #[test]
    fn a_program_that_starts_or_stops_moves_the_width_by_a_column() {
        let snap = Snapshot {
            sinks: vec![device(40, "speakers")],
            apps: playing(&["mpv"]),
            ..Snapshot::default()
        };
        let mut v = Volume::new(View::Playback, snap.clone(), State::default());
        let ctx = test_ctx();
        let (w, h) = v.initial_size();
        assert_eq!((w, h), v.main_size);
        assert_eq!(w, mixer::DEVICE_W + mixer::COL_W);
        v.snapshot.apps = playing(&["mpv", "vlc"]);
        assert_eq!(v.resync(&ctx), Some((w + mixer::COL_W, h)));
        v.snapshot = snap;
        assert_eq!(v.resync(&ctx), Some((w, h)));
        assert_eq!(v.resync(&ctx), None, "nothing moved");
    }

    #[test]
    fn the_status_bar_takes_the_looks_height() {
        let v = Volume::new(View::Playback, Snapshot::default(), State::default());
        let mut ctx = test_ctx();
        ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
        let h = statusbar::height(&ctx.theme.controls());
        assert_ne!(h, statusbar::HEIGHT, "Blue's bar is not Classic's");
        let (_, r) = v.status(&ctx);
        assert_eq!(
            (r.height, r.bottom()),
            (h, ctx.size.1 + 1),
            "its last row of face below the window"
        );
        assert_eq!(v.body(&ctx).bottom(), v.main_size.1 - (h - 1));
    }

    #[test]
    fn a_menu_title_lights_under_the_pointer_and_goes_out_on_leave() {
        let mut v = Volume::new(View::Playback, Snapshot::default(), State::default());
        let mut ctx = test_ctx();
        ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
        let lay = v.bar_layout(&ctx);
        let (x, y) = (lay.frames[0].x + 2, lay.frames[0].y + 2);
        assert!(v.pointer(PointerInput::Motion { x, y }, &ctx).redraw);
        assert_eq!(v.bar.hot, Some(0));
        assert!(v.pointer(PointerInput::Leave, &ctx).redraw);
        assert_eq!(v.bar.hot, None);
    }

    /// Writes target/volume-window.png for a visual check.
    #[test]
    #[ignore]
    fn render_the_window() {
        let snap = Snapshot {
            sinks: vec![device(40, "Intel(r) Integrated Audio")],
            apps: playing(&["Wave", "SW Synth", "CD Player"]),
            ..Snapshot::default()
        };
        let mut v = Volume::new(View::Playback, snap, State::default());
        let mut ctx = test_ctx();
        ctx.size = v.initial_size();
        let mut c = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
        v.draw(repriseapp::MAIN, &mut c, &ctx);
        reprisekit::raster::write_preview(&c, "volume-window.png");
    }

    #[test]
    fn the_window_opens_to_fit_its_columns() {
        let v = Volume::new(View::Playback, Snapshot::default(), State::default());
        assert_eq!(v.initial_size(), (mixer::DEVICE_W, 20 + 225 + 21));
        assert!(v.fixed());
        let rec = Snapshot {
            sources: vec![device(50, "mic")],
            ..Snapshot::default()
        };
        let v = Volume::new(View::Recording, rec, State::default());
        assert_eq!(
            v.initial_size().0,
            mixer::MIN_W,
            "one column keeps the least width"
        );
    }
}
