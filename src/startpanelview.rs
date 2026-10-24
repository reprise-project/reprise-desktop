mod draw;
mod input;
mod lists;
mod menus;
mod picture;
mod search;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::time::Instant;

use reprisekit::anim::UnrollFrom;
use reprisekit::startpanel::{Spot, StartPanel};
use reprisescene::{BufferId, TreeId};
use smithay::output::Output;
use smithay::reexports::calloop::RegistrationToken;

use crate::compositor::{SceneOwner, State};
use crate::config::{MenuAnimation, MenuEntry};
use crate::glass::Backdrop;
use crate::raster::Rect;
use crate::search::{FileSearch, Setting};

use draw::{Crossfade, Drawn, Rasters};
use search::ResultKind;

pub use picture::UserPictures;

const PICTURE: i32 = 48;
const RESULT_ICON: i32 = 16;

pub struct StartPanelView {
    pub model: StartPanel,
    bar: TreeId,
    output: Output,
    /// Indices into `app_index` parallel to `model.pinned` then `model.frequent`.
    programs: Vec<usize>,
    /// Leading `model.frequent` rows from launch history; the rest are starters.
    launched: usize,
    jumps: Vec<Vec<PathBuf>>,
    new_pins: Vec<String>,
    /// Parallel to `model.tree`; None for a folder.
    tree_apps: Vec<Option<usize>>,
    result_apps: Vec<usize>,
    answer: Option<String>,
    result_settings: Vec<Setting>,
    result_kinds: Vec<ResultKind>,
    pub search: FileSearch,
    /// Wheel travel short of a notch, in 120ths.
    wheel: i32,
    /// `startmenu::places()` without its rules, parallel to `model.places`.
    places: Vec<MenuEntry>,
    pressed: Option<Spot>,
    hover: Option<RegistrationToken>,
    opened: Instant,
    /// The opening's timer, fading or unrolling.
    fade: Option<RegistrationToken>,
    /// The side the panel unrolls from while it does.
    unroll: Option<UnrollFrom>,
    /// Where the unrolling panel draws from `at` this frame.
    shift: (i32, i32),
    /// The drawn canvas, panel-local: what unrolls.
    extent: Rect,
    /// Layout coordinates.
    at: (i32, i32),
    tree: TreeId,
    glass_tree: TreeId,
    backdrop: Backdrop,
    buffer: BufferId,
    drawn: Option<Drawn>,
    rasters: Rasters,
    crossfades: Vec<Crossfade>,
    crossfade_timer: Option<RegistrationToken>,
}

impl StartPanelView {
    /// Every file the programs' jump lists show.
    pub(crate) fn jump_files(&self) -> impl Iterator<Item = &PathBuf> {
        self.jumps.iter().flatten()
    }
}

fn outlived(on: &Output, outputs: &[Output]) -> bool {
    !outputs.contains(on)
}

impl State {
    pub(crate) fn toggle_start_panel(&mut self, bar: TreeId) {
        if self.start_panel.is_some() {
            self.close_start_panel();
        } else {
            self.open_start_panel(bar);
        }
    }

    pub(crate) fn toggle_start_panel_from_keys(&mut self) {
        if self.start_panel.is_some() {
            self.close_start_panel();
            return;
        }
        if !self.menus.is_empty() {
            self.close_menu();
            return;
        }
        let p = self.pointer_location();
        let Some(output) = self.output_at(p.x, p.y) else {
            return;
        };
        if let Some(bar) = self.bars.iter().find(|b| b.output == output) {
            let bar = bar.tree;
            self.open_start_panel(bar);
        }
    }

    fn open_start_panel(&mut self, bar: TreeId) {
        self.close_menu();
        self.close_launcher();
        let Some(output) = self.bar(bar).map(|b| b.output.clone()) else {
            return;
        };
        if self.layout_box(Some(&output)).width <= 0 {
            return;
        }
        let px = PICTURE * self.draw_scale_at(Some(&output)).max(1);
        let user = crate::lock::current_user();
        let mut model = StartPanel::new();
        model.text = self.config.theme.text;
        self.refresh_recent();
        let (rows, places) = self.start_places(px, &user);
        model.places = rows;
        model.user_picture = self.start_user_picture(px);
        let opening = self.config.menu_opening();
        let unroll = (opening == MenuAnimation::Slide)
            .then(|| crate::menuview::unroll_from_bar(self.bar_placement(&output).edge));
        let scene = &mut self.scene;
        let tree = scene.tree_create(self.layers.menu);
        scene.set_data(tree, Some(SceneOwner::StartPanel));
        crate::glass::leave_out(scene, tree);
        let glass_tree = scene.tree_create(tree);
        let buffer = scene.buffer_create(tree);
        if opening == MenuAnimation::Fade {
            scene.set_opacity(tree, 0.0);
        }
        self.start_panel = Some(StartPanelView {
            model,
            bar,
            output,
            programs: Vec::new(),
            launched: 0,
            jumps: Vec::new(),
            new_pins: Vec::new(),
            tree_apps: Vec::new(),
            result_apps: Vec::new(),
            answer: None,
            result_settings: Vec::new(),
            result_kinds: Vec::new(),
            search: FileSearch::new(),
            wheel: 0,
            places,
            pressed: None,
            hover: None,
            opened: Instant::now(),
            fade: None,
            unroll,
            shift: (0, 0),
            extent: Rect::new(0, 0, 0, 0),
            at: (0, 0),
            tree,
            glass_tree,
            backdrop: self
                .start_backdrop
                .take()
                .unwrap_or_else(|| Backdrop::new(&self.glass)),
            buffer,
            drawn: None,
            rasters: Rasters::default(),
            crossfades: Vec::new(),
            crossfade_timer: None,
        });
        self.read_user_pictures(vec![px]);
        self.list_start_programs();
        self.restart_caret();
        if let Some(b) = self.bar_mut(bar) {
            b.start_pressed = true;
        }
        if opening != MenuAnimation::None {
            self.arm_start_panel_fade();
        }
        self.redraw_start_panel();
        self.redraw_taskbar(bar);
    }

    /// Close the panel if its output left the scene: it would stay at
    /// stale coordinates taking every key and the pointer.
    pub fn close_start_panel_off_scene(&mut self) {
        if self
            .start_panel
            .as_ref()
            .is_some_and(|v| outlived(&v.output, self.scene.outputs()))
        {
            self.close_start_panel();
        }
    }

    pub(crate) fn close_start_panel(&mut self) {
        let Some(mut v) = self.start_panel.take() else {
            return;
        };
        for t in [v.hover.take(), v.fade.take(), v.crossfade_timer.take()]
            .into_iter()
            .flatten()
        {
            self.handle.remove(t);
        }
        crate::tooltip::hover_start_panel(self, None);
        self.close_menu();
        crate::recentwatch::lists_closed(self);
        v.search.stop(&self.handle);
        if self.editor().is_some() {
            self.restart_caret();
        } else {
            self.stop_caret();
        }
        v.backdrop
            .set(&mut self.scene, &self.glass, v.glass_tree, None);
        self.start_backdrop = Some(v.backdrop);
        self.scene.destroy(v.tree);
        if let Some(b) = self.bar_mut(v.bar) {
            b.start_pressed = false;
        }
        self.redraw_taskbar(v.bar);
    }

    pub(crate) fn start_panel_bar(&self) -> Option<TreeId> {
        self.start_panel.as_ref().map(|v| v.bar)
    }

    pub(crate) fn start_panel_output(&self) -> Option<Output> {
        self.start_panel.as_ref().map(|v| v.output.clone())
    }

    /// The open panel's draw scale, at least 1.
    fn start_panel_scale(&self) -> Option<i32> {
        let v = self.start_panel.as_ref()?;
        Some(self.draw_scale_at(Some(&v.output)).max(1))
    }
}
