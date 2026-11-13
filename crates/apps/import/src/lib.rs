//! Import from Another Desktop: [`reprisekit::controls::importsheet`] over what
//! [`reprise_settings::import`] finds, the checked rows written through Settings' model.

#[cfg(test)]
reprisebus::test_session!();

use reprise_settings::import::{self, Plan};
use repriseapp::keys::{dialog_key, form_input};
use repriseapp::{App, BTN_LEFT, Caption, Ctx, Event, PointerInput, Reply, WinId};
use reprisekit::access::Tree;
use reprisekit::controls::importsheet::{self, ListInput, Outcome, Sheet};
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use std::ffi::OsString;
use std::path::PathBuf;

struct ImportApp {
    config: PathBuf,
    quick: Option<PathBuf>,
    plans: Vec<Plan>,
    /// Built at the first event, with the window's text renderer.
    sheet: Option<Sheet>,
}

impl ImportApp {
    fn new(config: PathBuf, quick: Option<PathBuf>, plans: Vec<Plan>) -> Self {
        ImportApp {
            config,
            quick,
            plans,
            sheet: None,
        }
    }

    /// The sheet, laid out again first when the text size moved.
    fn sheet(&mut self, ctx: &Ctx) -> &mut Sheet {
        let forms = ctx.forms();
        let plans = &self.plans;
        let s = self.sheet.get_or_insert_with(|| {
            Sheet::new(plans.iter().map(Plan::source).collect(), &ctx.text, &forms)
        });
        s.follow(&ctx.text, &forms);
        s
    }

    fn list_input(&mut self, ev: &Event, ctx: &Ctx) -> Option<Outcome> {
        let forms = ctx.forms();
        let s = self.sheet(ctx);
        let input = match *ev {
            Event::Key(ref k) => return s.list_key(&dialog_key(k)?, &ctx.text, &forms),
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => ListInput::Press(x, y),
            Event::Pointer(PointerInput::Motion { x, y }) => ListInput::Motion(x, y),
            Event::Pointer(PointerInput::Release {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => ListInput::Release(x, y),
            Event::Pointer(PointerInput::Scroll { x, y, rows }) => ListInput::Scroll(x, y, rows),
            _ => return None,
        };
        s.list_pointer(input, &ctx.text, &forms)
    }

    fn dispatch(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let forms = ctx.forms();
        let out = match ev {
            Event::CloseRequest => Outcome::Close,
            Event::Blink => {
                self.sheet(ctx).form.blink(ctx.caret_on);
                return Reply::redraw();
            }
            Event::Resized | Event::Focus(_) => return Reply::redraw(),
            ev => match self.list_input(&ev, ctx) {
                Some(out) => out,
                None => {
                    let s = self.sheet(ctx);
                    match form_input(&mut s.form, &ev, ctx) {
                        Some(e) => s.event(e, &ctx.text, &forms),
                        None => return Reply::none(),
                    }
                }
            },
        };
        match out {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Import => {
                self.import(ctx);
                Reply::redraw()
            }
            Outcome::Close => Reply {
                quit: true,
                ..Reply::none()
            },
        }
    }

    /// Write the checked rows and say what changed.
    fn import(&mut self, ctx: &Ctx) {
        let s = self.sheet(ctx);
        let (source, checked) = (s.source, s.checked());
        let Some(plan) = self.plans.get(source) else {
            return;
        };
        let picked: Vec<_> = checked.iter().filter_map(|&i| plan.writes.get(i)).collect();
        let (line, wrote) = match import::write(&self.config, self.quick.as_deref(), &picked) {
            Ok(()) => {
                log::info!("imported {} {} settings", picked.len(), plan.name);
                (done_line(picked.len(), plan.name), true)
            }
            Err(e) => {
                log::warn!("import from {}: {e}", plan.name);
                (format!("Reprise couldn't import the settings: {e}"), false)
            }
        };
        let forms = ctx.forms();
        self.sheet(ctx).done(line, wrote, &ctx.text, &forms);
    }
}

fn done_line(n: usize, name: &str) -> String {
    match n {
        1 => format!("Imported 1 {name} setting."),
        n => format!("Imported {n} {name} settings."),
    }
}

impl App for ImportApp {
    fn title(&self) -> String {
        "Import from Another Desktop".into()
    }

    fn app_id(&self) -> String {
        "reprise.import".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        Sheet::size(repriseapp::start_forms().text)
    }

    fn event(&mut self, _win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let mut reply = self.dispatch(ev, ctx);
        let size = Sheet::size(ctx.forms().text);
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
        let s = self.sheet(ctx);
        importsheet::draw(c, &ctx.text, &p, s, forms.fonts.size_px);
    }
}

impl repriseapp::Extended for ImportApp {
    fn fixed(&self) -> bool {
        true
    }

    fn caption(&self, _win: WinId) -> Caption {
        Caption::Fixed
    }

    fn access(&mut self, _win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        if let Some(f) = self.sheet(ctx).access(tree, &ctx.text, &forms) {
            tree.set_focus(f);
        }
    }

    fn reader(&mut self, _win: WinId, act: repriseapp::Reader, ctx: &Ctx) -> bool {
        let forms = ctx.forms();
        let px = forms.fonts.size_px;
        let s = self.sheet(ctx);
        match act {
            repriseapp::Reader::Reveal { row, .. } => s.reveal(row, &forms),
            repriseapp::Reader::Focus((x, y)) => s.focus_at(x, y, &ctx.text, px),
        }
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(_args: Vec<OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let config = repriseconf::config_path();
    let text = std::fs::read_to_string(&config).unwrap_or_default();
    let sys = import::Live::new();
    let plans = import::find()
        .into_iter()
        .map(|(name, found)| import::plan(name, &found, &text, &sys))
        .collect();
    let app = ImportApp::new(config, reprisefs::quicklaunch::dir(), plans);
    repriseapp::run(Box::new(app)).unwrap_or_else(|e| e.exit());
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprise_settings::import::{Setting, Write};
    use reprisekit::controls::importsheet::Line;

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
        let d = std::env::temp_dir().join(format!("reprise-import-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn line(what: &str, now: &str, new: &str) -> Line {
        Line {
            what: what.into(),
            now: now.into(),
            new: new.into(),
            greyed: false,
        }
    }

    fn plan() -> Plan {
        Plan {
            name: "GNOME",
            lines: vec![
                line("Text size", "100%", "125%"),
                line("Tap to click", "On", "Off"),
            ],
            writes: vec![
                Write::Config(Setting::TextSize(125)),
                Write::Config(Setting::TapToClick(false)),
            ],
        }
    }

    #[test]
    fn import_writes_only_the_checked_rows_and_says_so() {
        let config = scratch("checked").join("config.toml");
        std::fs::write(&config, "# mine\nterminal = \"foot\"\n").unwrap();
        let mut app = ImportApp::new(config.clone(), None, vec![plan()]);
        let ctx = test_ctx(100);
        let s = app.sheet(&ctx);
        s.list.checks = Some(vec![false, true]);
        app.import(&ctx);
        let text = std::fs::read_to_string(&config).unwrap();
        assert!(text.starts_with("# mine\nterminal = \"foot\"\n"), "{text}");
        assert!(text.contains("tap_to_click = false"), "{text}");
        assert!(
            !text.contains("text_size"),
            "an unchecked row writes nothing: {text}"
        );
        let s = app.sheet(&ctx);
        assert_eq!(
            s.stage,
            importsheet::Stage::Done("Imported 1 GNOME setting.".into())
        );
    }

    #[test]
    fn closing_writes_nothing() {
        let config = scratch("close").join("config.toml");
        let mut app = ImportApp::new(config.clone(), None, vec![plan()]);
        let mut ctx = test_ctx(100);
        let reply = app.event(repriseapp::MAIN, Event::CloseRequest, &mut ctx);
        assert!(reply.quit);
        assert!(!config.exists());
    }

    #[test]
    fn the_window_follows_the_text_size() {
        let mut app = ImportApp::new(scratch("size").join("config.toml"), None, vec![plan()]);
        let mut ctx = test_ctx(100);
        ctx.size = Sheet::size(ctx.forms().text);
        let reply = app.event(repriseapp::MAIN, Event::Resized, &mut ctx);
        assert_eq!(reply.size, None);
        ctx.text_size = 200;
        let reply = app.event(repriseapp::MAIN, Event::Resized, &mut ctx);
        assert_eq!(reply.size, Some(Sheet::size(ctx.forms().text)));
        assert_ne!(reply.size, Some(importsheet::CLIENT));
    }
}
