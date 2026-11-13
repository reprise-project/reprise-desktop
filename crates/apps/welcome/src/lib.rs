//! Welcome: [`reprisekit::controls::welcome`]'s pages, each pick written through Settings' model.
//! `--first-login` (its autostart entry) shows them only while `repriseconf::welcome` says so.

#[cfg(test)]
reprisebus::test_session!();

use reprise_settings::welcome as settings;
use repriseapp::keys::form_input;
use repriseapp::{App, Caption, Ctx, Event, Reply, WinId};
use reprisefs::apps::AppEntry;
use reprisekit::access::Tree;
use reprisekit::controls::welcome::{self, Link, Outcome, Picks, Welcome};
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use std::ffi::OsString;
use std::path::PathBuf;

/// The last page's buttons and the desktop entries they start; a button shows only while its
/// entry is installed and shown.
const LINKS: [(Link, &str); 1] = [(
    Link {
        id: "settings",
        label: "Open &Settings",
    },
    "reprise-settings.desktop",
)];

struct WelcomeApp {
    config: PathBuf,
    picks: Picks,
    entries: Vec<(Link, AppEntry)>,
    /// Built at the first event, with the window's text renderer.
    welcome: Option<Welcome>,
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
        }
    }

    /// The pages, laid out again first when the text size moved.
    fn welcome(&mut self, ctx: &Ctx) -> &mut Welcome {
        let forms = ctx.forms();
        let w = self.welcome.get_or_insert_with(|| {
            let sample = settings::sample(&read(&self.config), &self.picks);
            let links = self.entries.iter().map(|(l, _)| *l).collect();
            Welcome::new(self.picks, sample, links, &ctx.text, &forms)
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

    /// Write the pages' picks into the config file and show the sample of what it now says.
    fn picked(&mut self, ctx: &Ctx) {
        let picks = self.welcome(ctx).picks;
        self.picks = picks;
        if let Err(e) = settings::write(&self.config, &picks) {
            log::warn!("{}: {e}", self.config.display());
        }
        let sample = settings::sample(&read(&self.config), &picks);
        self.welcome(ctx).set_sample(sample);
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
        "Welcome to Reprise".into()
    }

    fn app_id(&self) -> String {
        "reprise.welcome".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        repriseapp::start_forms().text.size(welcome::CLIENT)
    }

    fn event(&mut self, _win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let mut reply = self.dispatch(ev, ctx);
        let size = ctx.forms().text.size(welcome::CLIENT);
        if ctx.size != size {
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

    fn reader_form(&mut self, _win: WinId) -> Option<repriseapp::ReaderForm<'_>> {
        let form = repriseapp::DropForm {
            form: &mut self.welcome.as_mut()?.wizard.form,
            px: None,
        };
        Some(repriseapp::ReaderForm::Form(form))
    }

    fn access(&mut self, _win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let px = ctx.forms().fonts.size_px;
        self.welcome(ctx).access(tree, &ctx.text, px);
    }
}

/// Whether a `--first-login` start has anything to show: the flag is set, and is cleared here so
/// Welcome runs once.
fn first_login() -> bool {
    let state = repriseconf::state_path();
    if !repriseconf::welcome::pending(&state) {
        log::info!("not a first login, nothing to show");
        return false;
    }
    if let Err(e) = repriseconf::welcome::set(&state, false) {
        log::warn!("{}: {e}", state.display());
    }
    true
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
