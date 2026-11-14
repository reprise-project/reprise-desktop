//! Welcome: [`reprisekit::controls::welcome`]'s pages, each pick written through Settings' model.
//! `--first-login` (its autostart entry) shows them only while `repriseconf::welcome` says so.

#[cfg(test)]
reprisebus::test_session!();

use reprise_settings::welcome as settings;
use repriseapp::keys::form_input;
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{App, Caption, Child, Ctx, Event, Reply, WinId};
use reprisefs::apps::AppEntry;
use reprisekit::access::Tree;
use reprisekit::controls::msgbox;
use reprisekit::controls::welcome::{self, Link, Outcome, Picks, Welcome};
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;

const TITLE: &str = "Welcome to Reprise";
const ERROR_WIN: WinId = WinId::named("error");

/// The last page's buttons and the desktop entries they start; a button shows only while its
/// entry is installed and shown.
const LINKS: [(Link, &str); 2] = [
    (
        Link {
            id: "settings",
            label: "Open &Settings",
        },
        "reprise-settings.desktop",
    ),
    (
        Link {
            id: "import",
            label: "&Import from Another Desktop",
        },
        "reprise-import.desktop",
    ),
];

struct WelcomeApp {
    config: PathBuf,
    /// The picks the config file has; the pages' own run ahead of them until a write lands.
    picks: Picks,
    entries: Vec<(Link, AppEntry)>,
    /// Built at the first event, with the window's text renderer.
    welcome: Option<Welcome>,
    /// The box telling why a pick was not written.
    error: Option<MsgBox>,
    /// The renderer and forms theme the box is sized with: `children` has no `Ctx`.
    dialog: Option<(Rc<TextRenderer>, Theme)>,
}

fn read(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

impl WelcomeApp {
    fn new(config: PathBuf, entries: Vec<(Link, AppEntry)>) -> Self {
        let picks = settings::read(&read(&config));
        WelcomeApp {
            config,
            picks,
            entries,
            welcome: None,
            error: None,
            dialog: None,
        }
    }

    /// The pages, laid out again first when the text size moved.
    fn welcome(&mut self, ctx: &Ctx) -> &mut Welcome {
        let forms = ctx.forms();
        let w = self.welcome.get_or_insert_with(|| {
            let sample = settings::sample(&read(&self.config), &self.picks);
            let links = self.entries.iter().map(|(l, _)| *l).collect();
            let mut w = Welcome::new(self.picks, sample, links, &ctx.text, &forms);
            w.keys = settings::ready_keys(&read(&self.config));
            w
        });
        w.follow(&ctx.text, &forms);
        w
    }

    fn dispatch(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let forms = ctx.forms();
        let w = self.welcome(ctx);
        let out = match ev {
            Event::CloseRequest => Outcome::Close,
            Event::Blink => {
                w.wizard.form.blink(ctx.caret_on);
                return Reply::redraw();
            }
            Event::Resized | Event::Focus(_) => return Reply::redraw(),
            ev => match form_input(&mut w.wizard.form, &ev, ctx) {
                Some(e) => w.event(e, &ctx.text, &forms),
                None => return Reply::none(),
            },
        };
        match out {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Picked => {
                self.picked(ctx);
                Reply::redraw()
            }
            Outcome::Open(id) => self.open(id),
            Outcome::Close => Reply {
                quit: true,
                ..Reply::none()
            },
        }
    }

    /// Write the pages' picks into the config file and show the sample of what it now says. A
    /// failed write says why, and the next pick writes what it missed.
    fn picked(&mut self, ctx: &Ctx) {
        let picks = self.welcome(ctx).picks;
        match settings::write(&self.config, &self.picks, &picks) {
            Ok(()) => self.picks = picks,
            Err(e) => {
                log::warn!("{}: {e}", self.config.display());
                self.failed(format!(
                    "Your choice could not be written to {}.\n\n{e}",
                    self.config.display()
                ));
            }
        }
        let sample = settings::sample(&read(&self.config), &picks);
        self.welcome(ctx).set_sample(sample);
    }

    fn failed(&mut self, text: String) {
        self.error = Some(MsgBox::new(
            TITLE,
            msgbox::Fields {
                text,
                icon: None,
                buttons: vec!["OK".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        ));
    }

    fn error_event(&mut self, ev: Event) -> Reply {
        let (Some(b), Some((text, forms))) = (&mut self.error, &self.dialog) else {
            return Reply::none();
        };
        match b.event(&ev, text, forms) {
            oneshot::Outcome::Closed(_) => {
                self.error = None;
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    fn open(&self, id: &str) -> Reply {
        let Some((_, e)) = self.entries.iter().find(|(l, _)| l.id == id) else {
            return Reply::none();
        };
        match reprisefs::apps::launch_argv(e, &repriseconf::terminal()) {
            Ok(argv) => Reply {
                spawn: vec![argv],
                spawn_app: Some(e.id.clone()),
                ..Reply::redraw()
            },
            Err(refused) => {
                log::warn!("{refused}");
                Reply::none()
            }
        }
    }
}

impl App for WelcomeApp {
    fn title(&self) -> String {
        TITLE.into()
    }

    fn app_id(&self) -> String {
        "reprise.welcome".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        repriseapp::start_forms().text.size(welcome::CLIENT)
    }

    fn children(&self) -> Vec<Child> {
        match (&self.error, &self.dialog) {
            (Some(b), Some((text, forms))) => vec![b.child(ERROR_WIN, text, forms)],
            _ => Vec::new(),
        }
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.dialog = Some((ctx.text.clone(), ctx.forms()));
        if win == ERROR_WIN {
            return self.error_event(ev);
        }
        let mut reply = self.dispatch(ev, ctx);
        let size = ctx.forms().text.size(welcome::CLIENT);
        if ctx.size != size {
            reply.size = Some(size);
            reply.redraw = true;
        }
        reply
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        if win == ERROR_WIN {
            if let Some(b) = &self.error {
                b.draw(c, &forms, &ctx.text);
            }
            return;
        }
        let p = Paint {
            s: 1,
            ..forms.paint()
        };
        let w = self.welcome(ctx);
        welcome::draw(c, &ctx.text, &p, w, true, forms.fonts.size_px);
    }
}

impl repriseapp::Extended for WelcomeApp {
    fn fixed(&self) -> bool {
        true
    }

    fn caption(&self, _win: WinId) -> Caption {
        Caption::Fixed
    }

    fn reader_form(&mut self, win: WinId) -> Option<repriseapp::ReaderForm<'_>> {
        let w = self.welcome.as_mut().filter(|_| win != ERROR_WIN)?;
        let form = repriseapp::DropForm {
            form: &mut w.wizard.form,
            px: None,
        };
        Some(repriseapp::ReaderForm::Form(form))
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        if win == ERROR_WIN {
            if let Some(b) = &self.error {
                b.access(tree, &ctx.text, &forms);
            }
            return;
        }
        self.welcome(ctx)
            .access(tree, &ctx.text, forms.fonts.size_px);
    }
}

/// Whether a `--first-login` start has anything to show: the flag is set, and is cleared here so
/// Welcome runs once.
fn first_login() -> bool {
    let state = repriseconf::state_path();
    match repriseconf::welcome::take(&state) {
        Ok(true) => true,
        Ok(false) => {
            log::info!("not a first login, nothing to show");
            false
        }
        Err(e) => {
            log::warn!("{}: {e}", state.display());
            false
        }
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(args: Vec<OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    if args.iter().any(|a| a == "--first-login") && !first_login() {
        return;
    }
    let entries = LINKS
        .iter()
        .filter_map(|(l, id)| Some((*l, settings::entry(id)?)))
        .collect();
    let app = WelcomeApp::new(repriseconf::config_path(), entries);
    repriseapp::run(Box::new(app)).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::interface::Interface;

    fn test_ctx(text_size: u32) -> Ctx {
        Ctx {
            theme: reprisekit::theme::Theme::default(),
            text: reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default())
                .into(),
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
            output_size: None,
            at_pick: false,
            text_size,
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("reprise-welcome-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_pick_lands_in_the_config_file_and_keeps_its_comments() {
        let config = scratch("pick").join("config.toml");
        std::fs::write(&config, "# mine\nterminal = \"foot\"\n").unwrap();
        let mut app = WelcomeApp::new(config.clone(), Vec::new());
        let ctx = test_ctx(100);
        app.welcome(&ctx).picks.interface = Interface::Glass;
        app.picked(&ctx);
        let text = read(&config);
        assert!(text.starts_with("# mine\nterminal = \"foot\"\n"), "{text}");
        assert!(text.contains("interface = \"glass\""), "{text}");
        assert_eq!(app.picks.interface, Interface::Glass);
    }

    #[test]
    fn a_failed_write_says_why_and_the_next_pick_writes_what_it_missed() {
        let dir = scratch("failed");
        let blocker = dir.join("reprise");
        std::fs::write(&blocker, "").unwrap();
        let config = blocker.join("config.toml");
        let mut app = WelcomeApp::new(config.clone(), Vec::new());
        let ctx = test_ctx(100);
        app.welcome(&ctx).picks.interface = Interface::Glass;
        app.picked(&ctx);
        assert!(app.error.is_some(), "the box says why");
        assert_eq!(app.picks.interface, Interface::Classic, "the file's picks");
        app.error = None;
        std::fs::remove_file(&blocker).unwrap();
        app.welcome(&ctx).picks.text_size = 150;
        app.picked(&ctx);
        assert!(app.error.is_none());
        let text = read(&config);
        assert!(text.contains("interface = \"glass\""), "{text}");
        assert!(text.contains("text_size = 150"), "{text}");
        assert_eq!(app.picks.interface, Interface::Glass);
    }

    #[test]
    fn two_welcomes_on_one_file_keep_each_others_picks() {
        let config = scratch("two").join("config.toml");
        let (mut a, mut b) = (
            WelcomeApp::new(config.clone(), Vec::new()),
            WelcomeApp::new(config.clone(), Vec::new()),
        );
        let ctx = test_ctx(100);
        a.welcome(&ctx).picks.interface = Interface::Glass;
        a.picked(&ctx);
        b.welcome(&ctx).picks.text_size = 150;
        b.picked(&ctx);
        let text = read(&config);
        assert!(text.contains("interface = \"glass\""), "{text}");
        assert!(text.contains("text_size = 150"), "{text}");
    }

    #[test]
    fn the_window_follows_the_text_size() {
        let mut app = WelcomeApp::new(scratch("size").join("config.toml"), Vec::new());
        let mut ctx = test_ctx(100);
        ctx.size = ctx.forms().text.size(welcome::CLIENT);
        let reply = app.event(repriseapp::MAIN, Event::Resized, &mut ctx);
        assert_eq!(reply.size, None, "at 100% the window is the wizard's size");
        ctx.text_size = 150;
        let reply = app.event(repriseapp::MAIN, Event::Resized, &mut ctx);
        let grown = ctx.forms().text.size(welcome::CLIENT);
        assert_ne!(grown, welcome::CLIENT);
        assert_eq!(reply.size, Some(grown), "150% grows it");
    }
}
