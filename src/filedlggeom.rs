//! The size file dialogs open at and the place the user last moved one to, per app: toplevels
//! tagged `file-dialog` each keep a [`Geometry`] under their app's table in
//! `repriseconf::filedialog`'s file.

use std::collections::BTreeMap;
use std::path::PathBuf;

use repriseconf::filedialog::Geometry;

use crate::compositor::State;
use crate::raster::Rect;
use reprisekit::classic::frame::{Chrome, FrameGeometry};
use smithay::output::Output;

use crate::window::{Window, WindowId};

/// The app id Run's Browse... dialog keeps its folder, view and geometry under.
pub const OWN_APP: &str = "reprise";

/// Each app's geometry, read at start and after Settings
/// imported the file (`State::reload_remembered`): the compositor is their only other writer.
pub struct Kept {
    pub path: PathBuf,
    pub geometry: BTreeMap<String, Geometry>,
}

impl Kept {
    pub fn load() -> Kept {
        let path = repriseconf::filedialog::path();
        Kept::parse(path.clone(), &repriseconf::filedialog::load_from(&path))
    }

    /// What `text`, the file at `path`, keeps.
    pub fn parse(path: PathBuf, text: &str) -> Kept {
        let sections = repriseconf::filedialog::sections(text);
        Kept {
            geometry: (sections.iter())
                .filter_map(|(app, s)| Some((app.clone(), Geometry::parse(s)?)))
                .collect(),
            path,
        }
    }

    /// Keeps `g` for `app` and writes it on the state writer thread, once it differs from the
    /// last.
    fn store(&mut self, app: String, g: Geometry) {
        if self.geometry.get(&app) == Some(&g) {
            return;
        }
        self.geometry.insert(app.clone(), g);
        self.write(app, g.text());
    }

    fn write(&self, app: String, lines: String) {
        let path = self.path.clone();
        crate::state::run_in_order(move || {
            if let Err(e) = repriseconf::filedialog::save_to(&path, &app, &lines) {
                log::warn!(
                    "file dialog: could not store {app:?}'s in {}: {e}",
                    path.display()
                );
            }
        });
    }
}

/// Where the screens are for [`opening`]: each output's layout box and work area, and the work
/// area of the output a dialog opens on when the kept place is off all of them.
pub struct Screens {
    pub outputs: Vec<(Rect, Rect)>,
    pub fallback: Rect,
}

/// A file dialog opening from the kept geometry: its client size and, when the kept place
/// keeps its title bar on an output, its frame's top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opening {
    pub client: (i32, i32),
    pub at: Option<(i32, i32)>,
}

/// How a dialog with client minimum `min`, a frame `pad` larger than its client and a title bar
/// `caption` high opens from `g` at `text_size`: the kept size held between `min` and the work
/// area of the output its title bar lands on (else `screens.fallback`), at the kept place moved
/// inside that work area when there is one and its title bar lands on one of `screens`.
pub fn opening(
    g: &Geometry,
    text_size: u32,
    min: (i32, i32),
    pad: (i32, i32),
    caption: i32,
    screens: &Screens,
) -> Opening {
    let (w, h) = g.size_at(text_size);
    let on = g.at.and_then(|(x, y)| {
        let bar = Rect::new(x, y, w + pad.0, caption);
        crate::window::landing(bar, &screens.outputs).map(|(_, a)| ((x, y), *a))
    });
    let area = on.map_or(screens.fallback, |(_, a)| a);
    let most = (area.width - pad.0, area.height - pad.1);
    let client = (w.min(most.0).max(min.0), h.min(most.1).max(min.1));
    let frame = (client.0 + pad.0, client.1 + pad.1);
    Opening {
        client,
        at: on.map(|(at, _)| crate::window::keep_inside(at, frame, area)),
    }
}

impl State {
    /// What `file-dialog` toplevel `w` opens over, and that one's output: its owner, else the
    /// Reprise dialog it browses for.
    fn file_dialog_over(&self, w: WindowId) -> Option<(Rect, Output)> {
        if let Some(p) = self.owner(w) {
            let frame = self.win(p)?.frame_rect();
            return Some((frame, self.fit_output(p)?));
        }
        let over = self.browse_over(self.client_pid(w))?;
        let v = &self.dialogs.views[self.dialog_index(over)?];
        Some((v.rect(), v.output.clone()))
    }

    /// [`opening`] of `file-dialog` toplevel `w`'s kept geometry. A dialog over a window or a
    /// Reprise dialog keeps its place only on that one's output and fits that output; any
    /// other fits the pointer's when its place is on no output.
    fn file_dialog_opening(&self, w: WindowId, win: &Window) -> Option<Opening> {
        let g = *self.wm.file_dialog.geometry.get(&self.file_dialog_app(w))?;
        let over = self.file_dialog_over(w).map(|(_, o)| o);
        let outputs = (over.clone()).map_or_else(|| self.scene.outputs().to_vec(), |o| vec![o]);
        let fallback = over.or_else(|| self.placement_output(None));
        let screens = Screens {
            outputs: (outputs.iter())
                .map(|o| (self.layout_box(Some(o)), self.usable_area(Some(o))))
                .collect(),
            fallback: self.usable_area(fallback.as_ref()),
        };
        let m = &self.config.theme.metrics;
        Some(opening(
            &g,
            self.config.accessibility.text_size,
            win.size_limits().min,
            self.frame_pad(win.chrome()),
            m.caption_height + m.frame_width,
            &screens,
        ))
    }

    /// The frame around a toplevel's client under `chrome`.
    fn frame_pad(&self, chrome: Chrome) -> (i32, i32) {
        let total = FrameGeometry::layout(&self.config.theme.metrics, 0, 0, false, chrome).total;
        (total.0, total.1)
    }

    /// The app a `file-dialog` toplevel `w` keeps its geometry under: its owner's, so a portal's
    /// dialog counts as the app's that asked for it, else its own.
    fn file_dialog_app(&self, w: WindowId) -> String {
        let owner = self.parent_window(w).and_then(|p| self.win(p));
        owner.or(self.win(w)).map(Window::app).unwrap_or_default()
    }

    /// The client size a `file-dialog` toplevel `w` asks first.
    pub fn file_dialog_size(&self, w: WindowId) -> Option<(i32, i32)> {
        let win = self.win(w).filter(|x| x.file_dialog && x.is_resizable())?;
        Some(self.file_dialog_opening(w, win)?.client)
    }

    /// Where a `file-dialog` toplevel `w` with frame size `total` maps: the kept place while
    /// [`State::file_dialog_opening`] keeps it, else centred over what it opens over inside
    /// that one's output, or in `out`'s work area.
    pub fn file_dialog_position(
        &self,
        w: WindowId,
        total: (i32, i32),
        out: Option<&Output>,
    ) -> Option<(i32, i32)> {
        let win = self.win(w).filter(|x| x.file_dialog)?;
        if let Some(at) = self.file_dialog_opening(w, win).and_then(|o| o.at) {
            return Some(at);
        }
        let (over, area) = match self.file_dialog_over(w) {
            Some((r, o)) => (r, self.usable_area(Some(&o))),
            None => (self.usable_area(out), self.usable_area(out)),
        };
        Some(crate::window::centre_on(over, total, area))
    }

    /// Keeps `file-dialog` toplevel `w`'s normal client size for the next one, and where it is
    /// once the user `moved` it there.
    pub fn remember_file_dialog(&mut self, w: WindowId, moved: bool) {
        let Some(win) = self.win(w).filter(|x| x.file_dialog && x.mapped) else {
            return;
        };
        let frame = match (win.fullscreen, win.maximized) {
            (true, false) => win.saved_fullscreen,
            (_, true) => win.saved,
            _ => None,
        }
        .unwrap_or(win.frame_rect());
        let m = &self.config.theme.metrics;
        let client =
            FrameGeometry::client_size_for_total(m, frame.width, frame.height, false, win.chrome());
        let app = self.file_dialog_app(w);
        let at = match moved {
            true => Some((frame.x, frame.y)),
            false => self.wm.file_dialog.geometry.get(&app).and_then(|g| g.at),
        };
        let g = Geometry::kept(at, client, self.config.accessibility.text_size);
        self.wm.file_dialog.store(app, g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: Geometry = Geometry {
        at: Some((1500, 300)),
        width: 700,
        height: 600,
    };

    fn screens() -> Screens {
        Screens {
            outputs: vec![
                (Rect::new(0, 0, 1920, 1080), Rect::new(0, 0, 1920, 1050)),
                (
                    Rect::new(1920, 0, 1280, 1024),
                    Rect::new(1920, 0, 1280, 1024),
                ),
            ],
            fallback: Rect::new(0, 0, 1920, 1050),
        }
    }

    const MIN: (i32, i32) = (563, 419);
    const PAD: (i32, i32) = (8, 34);

    fn at(x: i32, y: i32) -> Geometry {
        Geometry {
            at: Some((x, y)),
            ..G
        }
    }

    use crate::harness::Harness;
    use crate::window::{FILE_DIALOG_TAG, WindowId};

    /// Drags `w` by its caption so its frame's top-left lands at `to`.
    fn drag(h: &mut Harness, w: WindowId, to: (i32, i32)) {
        let f = h.state.win(w).unwrap().frame_rect();
        let grab = (f.width / 2, 10);
        h.pointer_to(f64::from(f.x + grab.0), f64::from(f.y + grab.1));
        h.button(crate::harness::LEFT, true);
        h.pointer_to(f64::from(to.0 + grab.0), f64::from(to.1 + grab.1));
        h.button(crate::harness::LEFT, false);
        let f = h.state.win(w).unwrap().frame_rect();
        assert_eq!((f.x, f.y), to, "dragged there");
    }

    fn mid(h: &Harness, w: WindowId) -> (i32, i32) {
        let f = h.state.win(w).unwrap().frame_rect();
        (f.x + f.width / 2, f.y + f.height / 2)
    }

    fn at_of(h: &Harness, w: WindowId) -> (i32, i32) {
        let f = h.state.win(w).unwrap().frame_rect();
        (f.x, f.y)
    }

    /// Where `w` opens centred over `owner` on a one-output harness.
    fn over(h: &Harness, w: WindowId, owner: WindowId) -> (i32, i32) {
        let f = h.state.win(w).unwrap().frame_rect();
        let o = h.state.win(owner).unwrap().frame_rect();
        crate::window::centre_on(o, (f.width, f.height), h.state.usable_area(None))
    }

    #[test]
    fn a_dialog_with_nothing_kept_opens_centred_over_its_owner_on_its_owners_output() {
        let mut h = Harness::new();
        let one = h.output("TEST-1", 1024, 768);
        let two = h.output("TEST-2", 1280, 1024);
        let mut c = h.connect();
        let main = h.map_toplevel(&mut c, "test.app", 300, 200);
        let b = h.state.layout_box(Some(&two));
        h.state.move_to(main.id, b.x + 500, b.y + 400);
        let a = h.state.layout_box(Some(&one));
        h.pointer_to(f64::from(a.x + 10), f64::from(a.y + 10));
        let d = h.map_dialog(&mut c, "test.app", Some(main), FILE_DIALOG_TAG, (600, 400));
        assert_eq!(mid(&h, d.id), mid(&h, main.id));
    }

    #[test]
    fn a_dialog_closed_where_it_opened_opens_over_its_owner_again() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let main = h.map_toplevel(&mut c, "test.app", 300, 200);
        let d = h.map_dialog(&mut c, "test.app", Some(main), FILE_DIALOG_TAG, (600, 400));
        c.destroy(d);
        h.roundtrip(&mut c);
        h.state.move_to(main.id, 500, 300);
        let again = h.map_dialog(&mut c, "test.app", Some(main), FILE_DIALOG_TAG, (600, 400));
        assert_eq!(at_of(&h, again.id), over(&h, again.id, main.id));
    }

    #[test]
    fn a_dialog_dragged_away_and_closed_opens_where_it_was_dragged() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let main = h.map_toplevel(&mut c, "test.app", 300, 200);
        let d = h.map_dialog(&mut c, "test.app", Some(main), FILE_DIALOG_TAG, (600, 400));
        drag(&mut h, d.id, (40, 60));
        c.destroy(d);
        h.roundtrip(&mut c);
        assert!(h.state.win(d.id).is_none(), "the dialog went");
        h.state.move_to(main.id, 500, 300);
        let again = h.map_dialog(&mut c, "test.app", Some(main), FILE_DIALOG_TAG, (600, 400));
        let f = h.state.win(again.id).unwrap().frame_rect();
        assert_eq!((f.x, f.y), (40, 60), "the next one opens there");
    }

    #[test]
    fn a_place_kept_on_another_output_than_the_owners_opens_over_the_owner() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let two = h.output("TEST-2", 1280, 1024);
        let mut c = h.connect();
        let main = h.map_toplevel(&mut c, "test.app", 300, 200);
        let d = h.map_dialog(&mut c, "test.app", Some(main), FILE_DIALOG_TAG, (600, 400));
        drag(&mut h, d.id, (40, 60));
        c.destroy(d);
        h.roundtrip(&mut c);
        let b = h.state.layout_box(Some(&two));
        h.state.move_to(main.id, b.x + 500, b.y + 400);
        let again = h.map_dialog(&mut c, "test.app", Some(main), FILE_DIALOG_TAG, (600, 400));
        assert_eq!(mid(&h, again.id), mid(&h, main.id));
    }

    #[test]
    fn each_app_keeps_its_own_dialog_place_and_a_dialog_counts_as_its_owners_app() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let a = h.map_toplevel(&mut c, "org.example.A", 300, 200);
        let b = h.map_toplevel(&mut c, "org.example.B", 300, 200);
        let dialog = |h: &mut Harness, c: &mut crate::harness::TestClient, app, owner| {
            let d = h.map_dialog(c, app, Some(owner), FILE_DIALOG_TAG, (600, 400));
            let f = h.state.win(d.id).unwrap().frame_rect();
            (d, (f.x, f.y))
        };
        let (d, _) = dialog(&mut h, &mut c, "org.example.A", a);
        drag(&mut h, d.id, (40, 60));
        c.destroy(d);
        h.roundtrip(&mut c);
        let (d, _) = dialog(&mut h, &mut c, "org.example.B", b);
        assert_eq!(at_of(&h, d.id), over(&h, d.id, b.id), "B has nothing kept");
        drag(&mut h, d.id, (300, 100));
        c.destroy(d);
        h.roundtrip(&mut c);
        let (d, at) = dialog(&mut h, &mut c, "reprise.portal", a);
        assert_eq!(
            at,
            (40, 60),
            "a dialog opened for A opens where A's was left"
        );
        c.destroy(d);
        h.roundtrip(&mut c);
        assert_eq!(dialog(&mut h, &mut c, "org.example.B", b).1, (300, 100));
        let apps: Vec<&String> = h.state.wm.file_dialog.geometry.keys().collect();
        assert_eq!(apps, ["org.example.A", "org.example.B"]);
    }

    #[test]
    fn an_entry_with_a_place_under_the_old_keys_opens_over_its_owner_at_its_size() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let main = h.map_toplevel(&mut c, "test.app", 300, 200);
        h.state.move_to(main.id, 500, 300);
        let text = "[\"test.app\"]\nused = 1\nx = 20\ny = 30\nwidth = 640\nheight = 450\n";
        h.state.wm.file_dialog.geometry = Kept::parse(PathBuf::from("/nowhere"), text).geometry;
        let d = h.map_dialog(&mut c, "test.app", Some(main), FILE_DIALOG_TAG, (640, 450));
        assert_eq!(at_of(&h, d.id), over(&h, d.id, main.id));
        assert_eq!(h.state.file_dialog_size(d.id), Some((640, 450)));
    }

    #[test]
    fn the_kept_file_gives_each_apps_geometry() {
        let text = "[\"org.example.A\"]\nleft = 5\ntop = 6\nwidth = 600\nheight = 400\n\
                    [\"org.example.B\"]\nview = \"icons\"\n\
                    [reprise]\nfolder = \"/home/u/bin\"\nleft = 1\ntop = 2\nwidth = 3\nheight = 4\n";
        let k = Kept::parse(PathBuf::from("/nowhere"), text);
        assert_eq!(
            k.geometry.get("org.example.A"),
            Some(&Geometry {
                at: Some((5, 6)),
                width: 600,
                height: 400
            })
        );
        assert_eq!(k.geometry.get("org.example.B"), None, "no geometry kept");
        assert!(k.geometry.contains_key(OWN_APP));
    }

    #[test]
    fn a_kept_place_opens_there_while_its_title_bar_is_on_an_output() {
        let s = screens();
        let open = |g: Geometry| opening(&g, 100, MIN, PAD, 30, &s).at;
        assert_eq!(open(at(100, 100)), Some((100, 100)));
        assert_eq!(
            open(at(2000, 300)),
            Some((2000, 300)),
            "on the second output"
        );
    }

    #[test]
    fn a_kept_place_moves_inside_the_work_area_its_title_bar_lands_on() {
        let s = screens();
        let open = |g: Geometry, ts| opening(&g, ts, MIN, PAD, 30, &s).at;
        assert_eq!(open(at(-500, 10), 100), Some((0, 10)), "off the left edge");
        assert_eq!(
            open(at(2000, 900), 100),
            Some((2000, 1024 - 634)),
            "past the second output's bottom"
        );
        let one = Screens {
            outputs: vec![s.outputs[0]],
            fallback: s.fallback,
        };
        assert_eq!(
            opening(&at(1800, 300), 100, MIN, PAD, 30, &one).at,
            Some((1920 - 708, 300)),
            "left straddling an output since unplugged"
        );
        assert_eq!(
            open(at(100, 500), 150),
            Some((100, 1050 - 934)),
            "grown with the text past the bottom"
        );
    }

    #[test]
    fn a_title_bar_off_every_output_opens_at_the_default_place() {
        let s = screens();
        let open = |g: Geometry| opening(&g, 100, MIN, PAD, 30, &s).at;
        assert_eq!(open(at(2000, 1030)), None, "below the shorter output");
        assert_eq!(open(at(100, -30)), None, "above the top");
        assert_eq!(open(at(3200, 100)), None, "right of the last output");
        let none = Screens {
            outputs: vec![],
            fallback: Rect::new(0, 0, 1920, 1050),
        };
        assert_eq!(opening(&G, 100, MIN, PAD, 30, &none).at, None);
    }

    #[test]
    fn a_kept_size_is_held_between_the_minimum_and_the_work_area() {
        let s = screens();
        let size = |g: Geometry, ts| opening(&g, ts, MIN, PAD, 30, &s).client;
        assert_eq!(size(at(100, 100), 100), (700, 600));
        assert_eq!(size(at(100, 100), 150), (1050, 900), "grown with the text");
        assert_eq!(
            size(at(100, 100), 200),
            (1400, 1050 - 34),
            "no taller than the work area"
        );
        assert_eq!(
            size(at(2000, 100), 200),
            (1280 - 8, 1024 - 34),
            "inside the second output"
        );
        assert_eq!(
            size(at(5000, 100), 200),
            (1400, 1050 - 34),
            "off every output: the work area it opens on"
        );
        let small = Geometry {
            width: 300,
            height: 200,
            ..G
        };
        assert_eq!(size(small, 100), MIN);
    }
}
