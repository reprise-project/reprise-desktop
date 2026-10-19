//! The size and place every file dialog opens at: programs' toplevels tagged `file-dialog` and
//! the compositor's own Browse dialogs share one [`Geometry`] in `repriseconf::filedialog`'s file.

use std::path::PathBuf;

use repriseconf::filedialog::Geometry;

use crate::compositor::State;
use crate::raster::Rect;
use reprisekit::classic::frame::{Chrome, FrameGeometry};
use smithay::output::Output;

use crate::window::WindowId;

/// The geometry last stored, read once at start: the compositor is its only writer.
pub struct Kept {
    pub path: PathBuf,
    pub geometry: Option<Geometry>,
}

impl Kept {
    pub fn load() -> Kept {
        let path = repriseconf::filedialog::path();
        Kept {
            geometry: Geometry::parse(&repriseconf::filedialog::load_from(&path)),
            path,
        }
    }

    /// Keeps `g` and writes it on the state writer thread, once it differs from the last.
    fn store(&mut self, g: Geometry) {
        if self.geometry == Some(g) {
            return;
        }
        self.geometry = Some(g);
        let path = self.path.clone();
        crate::state::run_in_order(move || {
            if let Err(e) = repriseconf::filedialog::save_to(&path, &g.text()) {
                log::warn!(
                    "file dialog: could not store its geometry in {}: {e}",
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
/// area of the output its title bar lands on (else `screens.fallback`), at the kept place when
/// the title bar lands on any output.
pub fn opening(
    g: &Geometry,
    text_size: u32,
    min: (i32, i32),
    pad: (i32, i32),
    caption: i32,
    screens: &Screens,
) -> Opening {
    let (w, h) = g.size_at(text_size);
    let bar = Rect::new(g.x, g.y, w + pad.0, caption);
    let on = screens
        .outputs
        .iter()
        .find(|(b, _)| b.width > 0 && bar.intersects(*b));
    let area = on.map_or(screens.fallback, |(_, a)| *a);
    let most = (area.width - pad.0, area.height - pad.1);
    Opening {
        client: (w.min(most.0).max(min.0), h.min(most.1).max(min.1)),
        at: on.map(|_| (g.x, g.y)),
    }
}

impl State {
    /// [`opening`] on the current outputs; `fallback` is the output the dialog opens on
    /// without a kept place.
    fn file_dialog_opening(
        &self,
        min: (i32, i32),
        pad: (i32, i32),
        fallback: Option<&Output>,
    ) -> Option<Opening> {
        let g = self.wm.file_dialog.geometry?;
        let m = &self.config.theme.metrics;
        let screens = Screens {
            outputs: (self.scene.outputs().iter())
                .map(|o| (self.layout_box(Some(o)), self.usable_area(Some(o))))
                .collect(),
            fallback: self.usable_area(fallback),
        };
        Some(opening(
            &g,
            self.config.accessibility.text_size,
            min,
            pad,
            m.caption_height + m.frame_width,
            &screens,
        ))
    }

    /// The frame around a toplevel's client under `chrome`.
    fn frame_pad(&self, chrome: Chrome) -> (i32, i32) {
        let total = FrameGeometry::layout(&self.config.theme.metrics, 0, 0, false, chrome).total;
        (total.0, total.1)
    }

    /// The client size a `file-dialog` toplevel `w` asks first.
    pub fn file_dialog_size(&self, w: WindowId) -> Option<(i32, i32)> {
        let win = self.win(w).filter(|x| x.file_dialog && x.is_resizable())?;
        let pad = self.frame_pad(win.chrome());
        let out = self.placement_output(None);
        let o = self.file_dialog_opening(win.size_limits().min, pad, out.as_ref())?;
        Some(o.client)
    }

    /// Where a `file-dialog` toplevel `w` maps, when the kept place is on an output.
    pub fn file_dialog_position(&self, w: WindowId) -> Option<(i32, i32)> {
        let win = self.win(w).filter(|x| x.file_dialog)?;
        let pad = self.frame_pad(win.chrome());
        self.file_dialog_opening(win.size_limits().min, pad, None)?
            .at
    }

    /// Keeps where `file-dialog` toplevel `w` is and its normal client size for the next one.
    pub fn remember_file_dialog(&mut self, w: WindowId) {
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
        let g = Geometry::kept(
            (frame.x, frame.y),
            client,
            self.config.accessibility.text_size,
        );
        self.wm.file_dialog.store(g);
    }

    /// Sizes Browse dialog `p`, `pad` smaller than its frame, from the kept geometry; the place
    /// it goes, if any.
    pub fn restore_browse(
        &self,
        p: &mut crate::filepick::FilePicker,
        pad: (i32, i32),
    ) -> Option<(i32, i32)> {
        p.dlg.set_text(self.config.theme.text);
        let pos = self.pointer_location();
        let out = self.output_at(pos.x, pos.y);
        let o = self.file_dialog_opening(p.dlg.min_client(), pad, out.as_ref())?;
        p.dlg.set_size(o.client);
        o.at
    }

    /// Keeps where Browse dialog `index` is and its client size for the next file dialog.
    pub fn remember_browse(&mut self, index: usize) {
        let Some(v) = self.dialogs.views.get(index) else {
            return;
        };
        let Some(p) = v.dialog.picker() else {
            return;
        };
        let g = Geometry::kept(
            (v.x, v.y),
            p.dlg.client(),
            self.config.accessibility.text_size,
        );
        self.wm.file_dialog.store(g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: Geometry = Geometry {
        x: 1500,
        y: 300,
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
        Geometry { x, y, ..G }
    }

    #[test]
    fn a_kept_place_opens_there_while_its_title_bar_is_on_an_output() {
        let s = screens();
        let open = |g: Geometry| opening(&g, 100, MIN, PAD, 30, &s).at;
        assert_eq!(open(at(100, 100)), Some((100, 100)));
        assert_eq!(
            open(at(2000, 900)),
            Some((2000, 900)),
            "on the second output"
        );
        assert_eq!(
            open(at(-500, 10)),
            Some((-500, 10)),
            "partly off the left edge"
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
