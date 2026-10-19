//! The Open With chooser's input as every host reads it: keys, clicks, motion
//! and the wheel; keys bring out the keyboard cues.

use std::path::Path;

use reprisekit::controls::filedlg::{DlgMode, Fields};
use reprisekit::controls::form;
use reprisekit::controls::openwith::{self, Hit, Layout, OpenWith, Program};
use reprisekit::cues::Cues;
use reprisekit::pointer::DoubleClick;
use reprisekit::textedit::keysym;
use reprisekit::theme::Metrics;

use crate::keys::dialog_key;
use crate::{BTN_LEFT, Event, PointerInput};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChooserInput {
    clicks: DoubleClick<Option<Hit>>,
    cues: Cues,
}

impl ChooserInput {
    pub fn opened(by_key: bool) -> ChooserInput {
        ChooserInput {
            cues: Cues::opened(by_key),
            ..Default::default()
        }
    }

    pub fn cues(&self) -> Cues {
        self.cues
    }

    /// A reader's Focus at `at`: the control there takes the keyboard and the focus rectangles
    /// come out, as a key brings them. At least `Event::Redraw` when they came out.
    pub fn reader_focus(
        &mut self,
        ow: &mut OpenWith,
        lay: &Layout,
        at: (i32, i32),
    ) -> openwith::Event {
        let cued = self.cues.key(false);
        match ow.focus_on(ow.hit(lay, at.0, at.1)) {
            openwith::Event::None if cued => openwith::Event::Redraw,
            e => e,
        }
    }

    /// Feed `ev` to `ow`, laid out as `lay`. A key that brings out a cue is at
    /// least `Event::Redraw`.
    pub fn event(
        &mut self,
        ow: &mut OpenWith,
        lay: &Layout,
        ev: &Event,
        metrics: &Metrics,
    ) -> openwith::Event {
        if let Event::Key(k) = ev {
            let alt = k.mods.alt || matches!(k.sym, keysym::ALT_L | keysym::ALT_R);
            if self.cues.key(alt) {
                return match self.route(ow, lay, ev, metrics) {
                    openwith::Event::None => openwith::Event::Redraw,
                    e => e,
                };
            }
        }
        self.route(ow, lay, ev, metrics)
    }

    fn route(
        &mut self,
        ow: &mut OpenWith,
        lay: &Layout,
        ev: &Event,
        metrics: &Metrics,
    ) -> openwith::Event {
        match ev {
            Event::Key(k) => match dialog_key(k) {
                // The chooser has no edit field: only a flattened key counts, not a paste or IME commit.
                Some(form::Key::Edit(e)) => match form::flat_key(&e) {
                    Some(key) => ow.key(key),
                    None => openwith::Event::None,
                },
                Some(key) => ow.key(key),
                None => openwith::Event::None,
            },
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                time,
                ..
            }) => {
                let hit = ow.hit(lay, *x, *y);
                if self.clicks.press(hit, metrics.click(*x, *y, *time)) {
                    ow.double_click(hit)
                } else {
                    ow.click(hit)
                }
            }
            Event::Pointer(PointerInput::Release {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => ow.release(ow.hit(lay, *x, *y)),
            Event::Pointer(PointerInput::Motion { x, y }) => match ow.drag(lay, *y) {
                Some(ev) => ev,
                None => ow.motion(ow.hit(lay, *x, *y)),
            },
            Event::Pointer(PointerInput::Scroll { x, y, rows }) => {
                ow.wheel(ow.hit(lay, *x, *y), *rows)
            }
            _ => openwith::Event::None,
        }
    }
}

/// Where the chooser's Browse... opens its file dialog.
pub const PROGRAMS_DIR: &str = "/usr/bin";

/// The file dialog the chooser's Browse... opens at [`PROGRAMS_DIR`].
pub fn browse_fields() -> Fields {
    Fields {
        title: "Browse".into(),
        accept_label: "Open".into(),
        filters: vec!["Programs".into(), "All Files".into()],
        mode: DlgMode::Open,
        multiple: false,
    }
}

/// The Browse... dialog's listing filter: its first filter keeps executables
/// only. Folders stay regardless, through `Host::list`'s own `is_dir` check.
pub fn programs_keep(
    filter: usize,
) -> impl Fn(&reprisefs::listing::Item) -> bool + Clone + Send + 'static {
    move |i| filter != 0 || i.exec
}

/// A picked program file as a chooser row: its path is its id, its file
/// name its label. `None` for a path that is not UTF-8, which cannot be
/// run as a command line.
pub fn program_pick(path: &Path) -> Option<Program> {
    let id = path.to_str()?.to_string();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| id.clone());
    Some(Program {
        id,
        name,
        icon: Some(openwith::PROGRAM_ICON.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KeyInput, Mods};
    use reprisekit::controls::openwith::Focus;
    use reprisekit::raster::Rect;
    use reprisekit::text::TextRenderer;
    use reprisekit::theme::FontConfig;

    const PX: u32 = 11;

    fn prog(id: &str, name: &str) -> Program {
        Program {
            id: id.into(),
            name: name.into(),
            icon: None,
        }
    }

    fn chooser() -> (OpenWith, Layout) {
        let ow = OpenWith::new(
            "a.txt",
            vec![prog("alpha", "Alpha"), prog("beta", "Beta")],
            vec![prog("gamma", "Gamma")],
        );
        let lay = ow.layout(
            &TextRenderer::load(&FontConfig::default()),
            PX,
            (0, 0),
            &reprisekit::controls::metrics::ControlMetrics::CLASSIC,
        );
        (ow, lay)
    }

    fn centre(r: Rect) -> (i32, i32) {
        (r.x + r.width / 2, r.y + r.height / 2)
    }

    fn row(lay: &Layout, i: i32) -> (i32, i32) {
        (lay.list.inner.x + 40, lay.list.inner.y + i * 18 + 5)
    }

    fn press((x, y): (i32, i32), time: u32) -> Event {
        Event::Pointer(PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time,
            mods: Mods::default(),
        })
    }

    fn release((x, y): (i32, i32)) -> Event {
        Event::Pointer(PointerInput::Release {
            x,
            y,
            button: BTN_LEFT,
            mods: Mods::default(),
        })
    }

    fn key(sym: u32, utf8: &str) -> Event {
        Event::Key(KeyInput {
            sym,
            utf8: Some(utf8.into()),
            mods: Mods::default(),
            edit: reprisekit::textedit::key_for(sym, false, false, Some(utf8)),
        })
    }

    fn choose(id: &str) -> openwith::Event {
        openwith::Event::Choose {
            id: id.into(),
            always: false,
        }
    }

    #[test]
    fn a_press_and_release_on_ok_chooses_the_pick() {
        let (mut ow, lay) = chooser();
        let mut input = ChooserInput::default();
        assert_eq!(
            input.event(
                &mut ow,
                &lay,
                &press(centre(lay.ok), 1000),
                &Metrics::default()
            ),
            openwith::Event::Redraw,
            "the press only arms OK"
        );
        assert_eq!(
            input.event(&mut ow, &lay, &release(centre(lay.ok)), &Metrics::default()),
            choose("alpha")
        );
    }

    #[test]
    fn a_held_button_follows_the_pointer_off_and_back() {
        let (mut ow, lay) = chooser();
        let mut input = ChooserInput::default();
        let motion = |(x, y)| Event::Pointer(PointerInput::Motion { x, y });
        input.event(
            &mut ow,
            &lay,
            &press(centre(lay.ok), 1000),
            &Metrics::default(),
        );
        let mut drag = |at| input.event(&mut ow, &lay, &motion(at), &Metrics::default());
        assert_eq!(drag((5, 5)), openwith::Event::Redraw, "off OK it pops up");
        assert_eq!(drag((6, 5)), openwith::Event::None, "still off it");
        assert_eq!(drag(centre(lay.ok)), openwith::Event::Redraw, "back on");
        assert_eq!(
            input.event(&mut ow, &lay, &release(centre(lay.ok)), &Metrics::default()),
            choose("alpha")
        );
    }

    #[test]
    fn two_quick_presses_on_one_row_choose_it() {
        let (mut ow, lay) = chooser();
        let mut input = ChooserInput::default();
        input.event(
            &mut ow,
            &lay,
            &press(row(&lay, 2), 1000),
            &Metrics::default(),
        );
        assert_eq!(
            input.event(
                &mut ow,
                &lay,
                &press(row(&lay, 2), 1300),
                &Metrics::default()
            ),
            choose("beta")
        );
    }

    #[test]
    fn presses_pair_within_the_configured_double_click_time() {
        let (mut ow, lay) = chooser();
        let mut input = ChooserInput::default();
        let slow = Metrics {
            double_click_ms: 900,
            ..Metrics::default()
        };
        input.event(&mut ow, &lay, &press(row(&lay, 2), 1000), &slow);
        assert_eq!(
            input.event(&mut ow, &lay, &press(row(&lay, 2), 1800), &slow),
            choose("beta"),
            "800 ms apart is a double click at 900 ms"
        );
    }

    #[test]
    fn two_quick_presses_on_different_rows_only_pick_the_second() {
        let (mut ow, lay) = chooser();
        let mut input = ChooserInput::default();
        input.event(
            &mut ow,
            &lay,
            &press(row(&lay, 1), 1000),
            &Metrics::default(),
        );
        assert_eq!(
            input.event(
                &mut ow,
                &lay,
                &press(row(&lay, 4), 1300),
                &Metrics::default()
            ),
            openwith::Event::Redraw
        );
        assert_eq!(ow.chosen().map(|p| p.id.as_str()), Some("gamma"));
    }

    #[test]
    fn the_wheel_scrolls_the_list_under_the_pointer() {
        let (mut ow, lay) = chooser();
        ow.list
            .rows
            .extend(ow.list.rows.clone().into_iter().cycle().take(15));
        let mut input = ChooserInput::default();
        let wheel = |(x, y), rows| Event::Pointer(PointerInput::Scroll { x, y, rows });
        assert_eq!(
            input.event(&mut ow, &lay, &wheel(row(&lay, 1), 3), &Metrics::default()),
            openwith::Event::Redraw
        );
        assert_eq!(ow.list.top, 3);
        assert_eq!(
            input.event(
                &mut ow,
                &lay,
                &wheel(centre(lay.ok), 3),
                &Metrics::default()
            ),
            openwith::Event::None,
            "off the list"
        );
    }

    #[test]
    fn space_ticks_the_focused_box() {
        let (mut ow, lay) = chooser();
        ow.focus = Focus::Always;
        let mut input = ChooserInput::default();
        input.event(&mut ow, &lay, &key(0x20, " "), &Metrics::default());
        assert!(ow.always.on, "the box is ticked");
    }

    #[test]
    fn a_letter_walks_the_list() {
        let (mut ow, lay) = chooser();
        let mut input = ChooserInput::default();
        input.event(&mut ow, &lay, &key(0x67, "g"), &Metrics::default());
        assert_eq!(ow.chosen().map(|p| p.id.as_str()), Some("gamma"));
    }

    #[test]
    fn the_cues_wait_for_a_key() {
        let (mut ow, lay) = chooser();
        let mut input = ChooserInput::default();
        input.event(
            &mut ow,
            &lay,
            &press(centre(lay.cancel), 1000),
            &Metrics::default(),
        );
        input.event(&mut ow, &lay, &release((0, 0)), &Metrics::default());
        assert_eq!(input.cues(), Cues::NONE, "the pointer alone shows no cues");
        input.event(&mut ow, &lay, &key(0xff09, "\t"), &Metrics::default());
        assert_eq!(
            input.cues(),
            Cues {
                focus: true,
                keys: false
            },
            "a key shows the rectangles only"
        );
    }

    #[test]
    fn a_readers_focus_shows_the_rectangles_only() {
        let (mut ow, lay) = chooser();
        let mut input = ChooserInput::default();
        assert_eq!(
            input.reader_focus(&mut ow, &lay, centre(lay.cancel)),
            openwith::Event::Redraw
        );
        assert_eq!(
            input.cues(),
            Cues {
                focus: true,
                keys: false
            }
        );
        assert_eq!(
            input.reader_focus(&mut ow, &lay, centre(lay.cancel)),
            openwith::Event::None,
            "nothing changes the second time"
        );
    }

    #[test]
    fn alt_shows_the_underlines() {
        let (mut ow, lay) = chooser();
        let mut input = ChooserInput::default();
        assert_eq!(
            input.event(&mut ow, &lay, &key(0xffe9, ""), &Metrics::default()),
            openwith::Event::Redraw
        );
        assert_eq!(input.cues(), Cues::ALL);
        assert_eq!(
            input.event(&mut ow, &lay, &key(0xffe9, ""), &Metrics::default()),
            openwith::Event::None,
            "a second Alt changes nothing"
        );
        assert_eq!(ChooserInput::opened(true).cues(), Cues::ALL);
    }

    #[test]
    fn a_multi_character_insert_does_nothing() {
        let (mut ow, lay) = chooser();
        let mut input = ChooserInput::opened(true);
        assert_eq!(
            input.event(&mut ow, &lay, &key(0, "gamma"), &Metrics::default()),
            openwith::Event::None,
            "a paste or an IME commit does not type-ahead (the cues are out already)"
        );
        assert_eq!(ow.chosen().map(|p| p.id.as_str()), Some("alpha"));
    }

    #[test]
    fn a_program_path_becomes_a_program_named_by_its_file() {
        let p = program_pick(Path::new("/usr/bin/nano")).expect("a UTF-8 path");
        assert_eq!(p.id, "/usr/bin/nano");
        assert_eq!(p.name, "nano");
        assert_eq!(p.icon.as_deref(), Some(openwith::PROGRAM_ICON));
        use std::os::unix::ffi::OsStrExt;
        let odd = Path::new(std::ffi::OsStr::from_bytes(b"/usr/bin/a\xffb"));
        assert!(
            program_pick(odd).is_none(),
            "a path that is not UTF-8 is refused"
        );
    }
}
