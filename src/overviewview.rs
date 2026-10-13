//! The window overview on the scene: `reprisekit::overview` on every
//! output, a grid of live window pictures over the dimmed screen.
//!
//! Its tree in `layers.menu` is tagged `SceneOwner::Overview`: a dim rect
//! per output, then a cells' tree per output (the look's chrome in one
//! buffer, a `livepreview` picture per window above it). A window with
//! nothing to show gets its icon. While open it owns the pointer and the
//! keyboard; typed text closes it and goes on to the launcher's search.
//!
//! Snap assist is the same model in a zone: after a snap that leaves zones
//! free (`apply_layout`, `key_into_zone`), the first free zone shows a panel 13 px in from
//! its edges with a card per other window of the desktop. Picking one snaps
//! it there and moves on to the next free zone. It has no dim; under a
//! glass look the panel is a glass sheet (`glass::Backdrop`).

use std::time::Instant;

use reprisekit::controls::overview::{self, Event, Key, Overview, OverviewLayout};
use reprisekit::interfaces;
use reprisekit::taskpreview::fit;
use reprisekit::textedit::EditKey;
use reprisescene::{BufferId, TreeId};
use smithay::backend::input::InputTime;
use smithay::output::Output;

use crate::compositor::{SceneOwner, State};
use crate::dialogview::DialogKey;
use crate::glass::Backdrop;
use crate::input::BTN_LEFT;
use crate::keys;
use crate::livepreview::ThumbId;
use crate::output::upload_scaled;
use crate::raster::{Canvas, Rect};
use crate::snapview::Snapping;
use crate::window::WindowId;

const DIM: [f32; 4] = [0.0, 0.0, 0.0, 0.6];

/// The overview's state on `State`. `taken` holds the buttons whose press
/// the overview took, so their releases reach no client.
#[derive(Default)]
pub struct Overviews {
    pub view: Option<OverviewView>,
    taken: Vec<u32>,
    pub corner: crate::hotcorner::Corner,
}

pub struct OverviewView {
    pub model: Overview,
    pub windows: Vec<WindowId>,
    /// One grid per output, its area the output's box in layout
    /// coordinates; `grids[i]` draws `layouts[i]`.
    layouts: Vec<OverviewLayout>,
    grids: Vec<Grid>,
    /// Snap assist: the window the picker snapped, and the zones still
    /// empty as (zone, rect), the first being the grid's.
    assist: Option<(WindowId, Vec<(usize, Rect)>)>,
    /// The dim over every output (none under snap assist) and each grid's `cells`.
    tree: TreeId,
    fade: Option<FadeRun>,
}

/// A released vertical swipe's run-on: the fade eases over `span` (opacities) in `ms`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FadeRun {
    span: (f64, f64),
    ms: u32,
    started: Instant,
}

/// The run-on from `opacity` to shown (`open`) or gone, timed as a desktop swipe's run-on of `ms`.
fn fade_run(opacity: f32, open: bool, ms: u32) -> ((f64, f64), u32) {
    let (span, share) = crate::switchanim::drag_finish(f64::from(opacity), open);
    (span, ((f64::from(ms) * share).round() as u32).max(1))
}

/// The opacity `elapsed` ms into a run over `span` lasting `ms`; None once it has ended.
fn fade_at((from, to): (f64, f64), ms: u32, elapsed: f64) -> Option<f32> {
    let t = elapsed / f64::from(ms.max(1));
    (t < 1.0).then(|| (from + (to - from) * crate::windowanim::ease(t)) as f32)
}

struct Grid {
    output: Output,
    /// At the grid area's top-left: snap assist's glass, the chrome, then the pictures.
    cells: TreeId,
    /// Snap assist's glass: the backdrop's tree, at the panel's top-left, and the backdrop.
    glass: Option<(TreeId, Backdrop)>,
    chrome: BufferId,
    /// The live picture per cell; None where the window has nothing to show.
    thumbs: Vec<Option<ThumbId>>,
}

fn key_of(k: &DialogKey) -> Option<Key> {
    let shift = k.mods & keys::MOD_SHIFT != 0;
    k.syms.iter().find_map(|&sym| {
        Some(match sym {
            keys::KEY_Left | keys::KEY_KP_Left => Key::Left,
            keys::KEY_Right | keys::KEY_KP_Right => Key::Right,
            keys::KEY_Up | keys::KEY_KP_Up => Key::Up,
            keys::KEY_Down | keys::KEY_KP_Down => Key::Down,
            keys::KEY_Home | keys::KEY_KP_Home => Key::Home,
            keys::KEY_End | keys::KEY_KP_End => Key::End,
            keys::KEY_ISO_Left_Tab => Key::BackTab,
            keys::KEY_Tab if shift => Key::BackTab,
            keys::KEY_Tab => Key::Tab,
            keys::KEY_Return | keys::KEY_KP_Enter => Key::Enter,
            keys::KEY_Escape => Key::Escape,
            _ => return None,
        })
    })
}

impl State {
    /// Alt+Tab's windows (`switchable_windows`) without the dialogs, most
    /// recently focused first, minimized last.
    fn overview_windows(&self) -> Vec<WindowId> {
        let mut v = self.switchable_windows();
        v.reverse();
        v.sort_by_key(|&w| self.win(w).is_some_and(|x| x.minimized));
        v
    }

    /// The grid of `count` windows over `area`; snap assist shapes the
    /// pictures like the usable area the zone is part of.
    fn overview_grid(&self, count: usize, area: Rect, assist: bool) -> OverviewLayout {
        if !assist {
            return OverviewLayout::compute(count, area);
        }
        let (cx, cy) = (area.x + area.width / 2, area.y + area.height / 2);
        let work = self.usable_area(self.output_at(f64::from(cx), f64::from(cy)).as_ref());
        OverviewLayout::compute_assist(count, area, (work.width, work.height))
    }

    fn overview_outputs(&self) -> Vec<(Output, Rect)> {
        self.scene
            .outputs()
            .iter()
            .map(|o| (o.clone(), self.layout_box(Some(o))))
            .filter(|(_, b)| b.width > 0 && b.height > 0)
            .collect()
    }

    /// `windows` grouped by the output each lies mostly on, in the order of
    /// `boxes`; and each group's size.
    fn overview_grouped(
        &self,
        windows: &[WindowId],
        boxes: &[Rect],
    ) -> (Vec<WindowId>, Vec<usize>) {
        let frames: Vec<Rect> = windows
            .iter()
            .map(|&w| self.win(w).map(|x| x.frame_rect()).unwrap_or_default())
            .collect();
        let to = overview::assign(&frames, boxes);
        let mut out = Vec::with_capacity(windows.len());
        let mut counts = vec![0; boxes.len()];
        for (g, count) in counts.iter_mut().enumerate() {
            for (i, &w) in windows.iter().enumerate() {
                if to[i] == g {
                    out.push(w);
                    *count += 1;
                }
            }
        }
        (out, counts)
    }

    pub fn toggle_overview(&mut self) {
        self.overview_fade_settle();
        if self.overview.view.is_some() {
            self.close_overview();
        } else {
            self.open_overview();
        }
    }

    /// Open the overview, the most recently focused window selected. No-op
    /// while locked, while a switcher is up or already open.
    pub fn open_overview(&mut self) {
        if self.overview.view.is_some() || self.is_locked() || self.switcher.is_open() {
            return;
        }
        let outputs = self.overview_outputs();
        if outputs.is_empty() {
            return;
        }
        let order = self.overview_windows();
        let boxes: Vec<Rect> = outputs.iter().map(|(_, b)| *b).collect();
        let (windows, counts) = self.overview_grouped(&order, &boxes);
        let grids: Vec<(Output, OverviewLayout)> = outputs
            .into_iter()
            .zip(counts)
            .map(|((output, b), n)| (output, OverviewLayout::compute(n, b)))
            .collect();
        let mut model = Overview::new(windows.len());
        if let Some(first) = order.first() {
            model.selected = windows.iter().position(|w| w == first).unwrap_or(0);
        }
        let all = self.layout_box(None);
        self.show_overview(all, model, windows, grids, None);
    }

    pub fn snap_assist_open(&self) -> bool {
        self.overview
            .view
            .as_ref()
            .is_some_and(|v| v.assist.is_some())
    }

    /// Snap assist after `snapped` filled a zone from the snap layout
    /// picker: offers the desktop's other snappable windows outside
    /// `snapped`'s group for the first of `zones`.
    pub fn open_snap_assist(&mut self, snapped: WindowId, zones: Vec<(usize, Rect)>) {
        if self.overview.view.is_some() || self.is_locked() || self.switcher.is_open() {
            return;
        }
        self.snap_assist_at(snapped, zones);
    }

    /// Snap assist for the first of `zones`: a single grid over the zone, on the output under
    /// its centre, offering the desktop's other snappable windows outside `leader`'s group, as
    /// many as its panel holds.
    fn snap_assist_at(&mut self, leader: WindowId, zones: Vec<(usize, Rect)>) {
        let Some(&(_, area)) = zones.first() else {
            return;
        };
        let mut windows: Vec<WindowId> = (self.overview_windows().into_iter())
            .filter(|&w| {
                !self.wm.snap.shares_group(leader, w) && self.win(w).is_some_and(|x| x.snappable())
            })
            .collect();
        if windows.is_empty() || area.width <= 0 || area.height <= 0 {
            return;
        }
        let (cx, cy) = (area.x + area.width / 2, area.y + area.height / 2);
        let Some(output) = self.output_at(f64::from(cx), f64::from(cy)) else {
            return;
        };
        let work = self.usable_area(Some(&output));
        let fits = OverviewLayout::assist_fits(windows.len(), area, (work.width, work.height));
        windows.truncate(fits);
        if windows.is_empty() {
            return;
        }
        let layout = self.overview_grid(windows.len(), area, true);
        let model = Overview::new(windows.len());
        let grids = vec![(output, layout)];
        self.show_overview(area, model, windows, grids, Some((leader, zones)));
    }

    /// Put the overview of `windows` up, a grid per `(output, layout)`
    /// over `dim`; `assist` for snap assist, which dims nothing.
    fn show_overview(
        &mut self,
        dim: Rect,
        model: Overview,
        windows: Vec<WindowId>,
        grids: Vec<(Output, OverviewLayout)>,
        assist: Option<(WindowId, Vec<(usize, Rect)>)>,
    ) {
        self.cancel_interaction();
        self.close_menus_above(0);
        self.close_launcher();
        crate::taskpreview::hide(self);
        crate::snappickerview::hide(self);
        let tree = self.scene.tree_create(self.layers.menu);
        self.scene.set_data(tree, Some(SceneOwner::Overview));
        crate::glass::leave_out(&mut self.scene, tree);
        self.scene.raise_to_top(tree);
        if assist.is_none() {
            let shade = self.scene.rect_create(tree, (dim.width, dim.height), DIM);
            self.scene.set_position(shade, dim.x, dim.y);
        }
        let mut layouts = Vec::with_capacity(grids.len());
        let grids = grids
            .into_iter()
            .map(|(output, layout)| {
                let cells = self.scene.tree_create(tree);
                self.scene.set_position(cells, layout.area.x, layout.area.y);
                let glass = assist.is_some().then(|| {
                    let t = self.scene.tree_create(cells);
                    (t, Backdrop::new(&self.glass))
                });
                let chrome = self.scene.buffer_create(cells);
                layouts.push(layout);
                Grid {
                    output,
                    cells,
                    glass,
                    chrome,
                    thumbs: Vec::new(),
                }
            })
            .collect();
        self.overview.view = Some(OverviewView {
            model,
            windows,
            layouts,
            grids,
            assist,
            tree,
            fade: None,
        });
        self.clear_pointer_focus();
        self.set_cursor("default");
        self.redraw_overview();
        let p = self.pointer_location();
        self.repoint(p, InputTime::now());
    }

    /// Show the open overview at `opacity`, dim and all (the swipe fade).
    pub fn set_overview_fade(&mut self, opacity: f32) {
        if let Some(v) = self.overview.view.as_ref() {
            self.scene.set_opacity(v.tree, opacity);
            self.redraw_all();
        }
    }

    /// Ease the open overview from its opacity to shown (`open`) or gone, closing it at the end.
    pub fn overview_fade_to(&mut self, open: bool) {
        let ms = self.config.workspace_animation.ms;
        let Some(v) = self.overview.view.as_mut() else {
            return;
        };
        let opacity = self.scene.opacity(v.tree).unwrap_or(1.0);
        let (span, ms) = fade_run(opacity, open, ms);
        v.fade = Some(FadeRun {
            span,
            ms,
            started: Instant::now(),
        });
        self.redraw_all();
    }

    /// Called from each output's render: the next step of a fade run-on.
    pub fn overview_fade_frame(&mut self) {
        let Some(run) = self.overview.view.as_ref().and_then(|v| v.fade) else {
            return;
        };
        let elapsed = run.started.elapsed().as_secs_f64() * 1000.0;
        match fade_at(run.span, run.ms, elapsed) {
            Some(opacity) => self.set_overview_fade(opacity),
            None => self.overview_fade_settle(),
        }
    }

    /// End a running fade run-on where it was heading.
    pub fn overview_fade_settle(&mut self) {
        let Some(run) = self.overview.view.as_mut().and_then(|v| v.fade.take()) else {
            return;
        };
        if run.span.1 > 0.0 {
            self.set_overview_fade(1.0);
        } else {
            self.close_overview();
        }
    }

    /// Close the overview without changing the focus.
    pub fn close_overview(&mut self) {
        let Some(v) = self.overview.view.take() else {
            return;
        };
        for id in v.grids.into_iter().flat_map(|g| g.thumbs).flatten() {
            self.thumb_drop(id);
        }
        self.scene.destroy(v.tree);
        self.redraw_all();
        crate::access::touch(self);
        let p = self.pointer_location();
        self.repoint(p, InputTime::now());
    }

    /// Close the overview and bring up window `i`. Under snap assist it
    /// fills the zone instead and the next empty zone offers the rest.
    fn overview_activate(&mut self, i: usize) {
        let Some(v) = self.overview.view.as_ref() else {
            return;
        };
        let w = v.windows.get(i).copied();
        let assist = v.assist.clone();
        self.close_overview();
        let Some(w) = w else {
            return;
        };
        if let Some((leader, zones)) = assist {
            if let Some((&(zone, rect), rest)) = zones.split_first() {
                self.wm.snap.join_group(leader, w, zone);
                self.snap_to_zone(w, rect);
                self.snap_assist_at(leader, rest.to_vec());
            }
            return;
        }
        if self.win(w).is_some_and(|x| x.minimized) {
            self.set_minimized(w, false);
        } else {
            self.focus_window(w);
        }
        let p = self.pointer_location();
        self.repoint(p, InputTime::now());
    }

    fn apply_overview(&mut self, e: Event) {
        match e {
            Event::None => {}
            Event::Redraw => self.redraw_overview(),
            Event::Activate(i) => self.overview_activate(i),
            Event::Close => self.close_overview(),
        }
    }

    /// A press the open overview took. Typed text closes it and starts
    /// the launcher's search with it; under snap assist it only closes.
    pub fn overview_key(&mut self, k: &DialogKey) {
        let Some(v) = self.overview.view.as_mut() else {
            return;
        };
        if let Some(key) = key_of(k) {
            let e = v.model.key(&v.layouts, key);
            self.apply_overview(e);
            return;
        }
        let assist = v.assist.is_some();
        if let Some(EditKey::Insert(_)) = &k.edit
            && k.mods & (keys::MOD_CTRL | keys::MOD_ALT | keys::MOD_LOGO) == 0
        {
            self.close_overview();
            if !assist {
                self.toggle_launcher();
                self.launcher_key(k);
            }
        }
    }

    pub fn overview_hit(&self, lx: f64, ly: f64) -> Option<usize> {
        let v = self.overview.view.as_ref()?;
        overview::hit_grids(&v.layouts, lx as i32, ly as i32)
    }

    pub fn overview_hover(&mut self, cell: Option<usize>) {
        if let Some(v) = self.overview.view.as_mut() {
            let e = v.model.hover(cell);
            self.apply_overview(e);
        }
    }

    /// A button pressed while the overview is up: a left click on a cell
    /// activates its window, any other press closes it.
    pub fn overview_press(&mut self, button: u32) {
        if !self.overview.taken.contains(&button) {
            self.overview.taken.push(button);
        }
        // Snap assist's close button closes the hot card's window. A reader's
        // click reaches any card's button: it has no pointer to make one hot.
        if button == BTN_LEFT
            && let Some(v) = self.overview.view.as_ref()
            && v.assist.is_some()
        {
            let p = self.pointer_location();
            let l = &v.layouts[0];
            let (x, y) = (p.x as i32 - l.area.x, p.y as i32 - l.area.y);
            let card = match self.access.click_at {
                Some(_) => overview::hit(l, x, y),
                None => v.model.hot,
            };
            if let Some(i) = overview::assist_close_hit(l, card, x, y)
                && let Some(&w) = v.windows.get(i)
            {
                self.close(w);
                return;
            }
        }
        let p = self.pointer_location();
        let cell = self.overview_hit(p.x, p.y).filter(|_| button == BTN_LEFT);
        if let Some(v) = self.overview.view.as_mut() {
            let e = v.model.click(cell);
            self.apply_overview(e);
        }
    }

    /// Whether the release of `button` belongs to a press the overview took.
    pub fn overview_release(&mut self, button: u32) -> bool {
        let taken = &mut self.overview.taken;
        let had = taken.contains(&button);
        taken.retain(|&b| b != button);
        had
    }

    /// The window list may have changed: keep the open overview's order,
    /// drop gone windows, append new ones to their output's grid. Snap
    /// assist keeps the windows offered at the pick (`assist_offer`).
    pub fn overview_refresh(&mut self) {
        let Some(v) = self.overview.view.as_ref() else {
            return;
        };
        let now = self.overview_windows();
        let boxes: Vec<Rect> = v.layouts.iter().map(|l| l.area).collect();
        let selected = v.windows.get(v.model.selected).copied();
        let (windows, counts) = match v.assist {
            Some((leader, _)) => {
                let Some(offer) = assist_offer(&self.wm.snap, leader, &v.windows, &now) else {
                    self.close_overview();
                    return;
                };
                let n = offer.len();
                (offer, vec![n])
            }
            None => {
                let mut order: Vec<WindowId> = (v.windows.iter().copied())
                    .filter(|w| now.contains(w))
                    .collect();
                order.extend(now.iter().copied().filter(|w| !v.windows.contains(w)));
                self.overview_grouped(&order, &boxes)
            }
        };
        let Some(v) = self.overview.view.as_mut() else {
            return;
        };
        if windows == v.windows {
            return;
        }
        v.windows = windows;
        v.model.set_len(v.windows.len());
        if let Some(i) = selected.and_then(|w| v.windows.iter().position(|&x| x == w)) {
            v.model.selected = i;
        }
        let assist = v.assist.is_some();
        let layouts = boxes
            .iter()
            .zip(counts)
            .map(|(&b, n)| self.overview_grid(n, b, assist))
            .collect();
        let Some(v) = self.overview.view.as_mut() else {
            return;
        };
        v.layouts = layouts;
        let old: Vec<ThumbId> = v
            .grids
            .iter_mut()
            .flat_map(|g| std::mem::take(&mut g.thumbs))
            .flatten()
            .collect();
        for id in old {
            self.thumb_drop(id);
        }
        self.redraw_overview();
    }

    /// Draw the open overview again.
    fn redraw_overview(&mut self) {
        let Some(v) = self.overview.view.as_ref() else {
            return;
        };
        let mut first = 0;
        for g in 0..v.grids.len() {
            first += self.redraw_overview_grid(g, first);
        }
        self.redraw_all();
        crate::access::touch(self);
    }

    /// Draw grid `g`, whose first window is `windows[first]`; returns how
    /// many windows it shows.
    fn redraw_overview_grid(&mut self, g: usize, first: usize) -> usize {
        let Some(v) = self.overview.view.as_ref() else {
            return 0;
        };
        let output = v.grids[g].output.clone();
        let n = v.layouts[g].cells.len();
        let windows: Vec<WindowId> = v.windows[first..first + n].to_vec();
        let scale = self.draw_scale_at(Some(&output));
        self.place_overview_thumbs(g, &windows, scale);
        self.place_assist_glass(g);
        let theme = self.config.theme.scaled(scale);
        let icons: Vec<Option<Canvas>> = windows
            .iter()
            .map(|&w| self.window_icon(w, overview::ICON_PX * scale))
            .collect();
        let pictured: Vec<bool> = self.overview.view.as_ref().map_or_else(Vec::new, |v| {
            v.grids[g].thumbs.iter().map(Option::is_some).collect()
        });
        let stand_ins: Vec<Option<Canvas>> = windows
            .iter()
            .enumerate()
            .map(|(i, &w)| match pictured.get(i) {
                Some(true) => None,
                _ => self.window_icon(w, overview::STAND_IN_PX * scale),
            })
            .collect();
        let Some(v) = self.overview.view.as_ref() else {
            return n;
        };
        let titles: Vec<String> = windows
            .iter()
            .map(|&w| self.win(w).map(|x| x.title()).unwrap_or_default())
            .collect();
        let grid = &v.grids[g];
        let logical = &v.layouts[g];
        let layout = logical.scaled(scale);
        let cells: Vec<overview::Cell> = (0..n)
            .map(|i| {
                let picture = match (grid.thumbs.get(i), self.win(windows[i]), logical.well(i)) {
                    (Some(Some(_)), Some(x), Some(well)) => {
                        Some(fit(x.client_w, x.client_h, well, 1).scaled(scale))
                    }
                    _ => None,
                };
                overview::Cell {
                    title: &titles[i],
                    icon: icons[i].as_ref(),
                    picture,
                    stand_in: stand_ins[i].as_ref(),
                }
            })
            .collect();
        let local = |i: usize| i.checked_sub(first).filter(|&i| i < n);
        let selected = local(v.model.selected).unwrap_or(usize::MAX);
        let hot = v.model.hot.and_then(local);
        let (w, h) = (
            layout.area.width.max(1) as u32,
            layout.area.height.max(1) as u32,
        );
        let mut canvas = Canvas::new(w, h);
        if v.assist.is_some() {
            // A card lights for the keyboard only while none is hovered.
            let lit = Some(selected).filter(|&i| i < n && hot.is_none());
            interfaces::draw_snap_assist(
                &mut canvas,
                &theme,
                &self.text,
                &layout,
                &cells,
                lit,
                hot,
            );
        } else {
            interfaces::draw_overview(
                &mut canvas,
                &theme,
                &self.text,
                &layout,
                &cells,
                selected,
                hot,
            );
        }
        let chrome = grid.chrome;
        upload_scaled(&mut self.scene, chrome, &canvas, scale);
        n
    }

    /// Show snap assist's glass under grid `g`'s panel (none without a glass look).
    fn place_assist_glass(&mut self, g: usize) {
        let wanted = crate::glass::wanted(&self.config);
        let Some(v) = self.overview.view.as_mut() else {
            return;
        };
        let area = v.layouts[g].area;
        let Some((tree, backdrop)) = v.grids[g].glass.as_mut() else {
            return;
        };
        let panel = overview::assist_panel((area.width, area.height), 1);
        self.scene.set_position(*tree, panel.x, panel.y);
        let pane = wanted.then(|| {
            crate::glass::sheet_pane(
                (panel.width, panel.height),
                reprisekit::snaplayouts::PANEL_CORNER,
            )
        });
        backdrop.set(&mut self.scene, &self.glass, *tree, pane);
    }

    /// Keep a live picture in each cell's well of grid `g`: one already
    /// showing that window at `scale` is kept, the rest made anew or dropped.
    fn place_overview_thumbs(&mut self, g: usize, windows: &[WindowId], scale: i32) {
        let Some(v) = self.overview.view.as_mut() else {
            return;
        };
        let parent = v.grids[g].cells;
        let wells: Vec<Option<Rect>> = (0..windows.len()).map(|i| v.layouts[g].well(i)).collect();
        let mut old = std::mem::take(&mut v.grids[g].thumbs);
        let mut thumbs = Vec::with_capacity(windows.len());
        for (&w, well) in windows.iter().zip(wells) {
            let Some(well) = well else {
                thumbs.push(None);
                continue;
            };
            let kept = old
                .iter()
                .position(|t| t.is_some_and(|id| self.thumb_shows(id, w, well, scale)))
                .and_then(|i| old[i].take());
            thumbs.push(kept.or_else(|| self.thumb_create(w, parent, well, scale)));
        }
        for id in old.into_iter().flatten() {
            self.thumb_drop(id);
        }
        if let Some(v) = self.overview.view.as_mut() {
            v.grids[g].thumbs = thumbs;
        }
    }

    /// The open overview's tree: windows by title, the selected one focused.
    pub fn overview_access(&self) -> Option<reprisekit::access::Tree> {
        use reprisekit::access::{ROOT, accesskit::Role};
        let v = self.overview.view.as_ref()?;
        let titles: Vec<String> = v
            .windows
            .iter()
            .map(|&w| self.win(w).map(|x| x.title()).unwrap_or_default())
            .collect();
        let (name, bounds) = match &v.assist {
            Some(_) => ("Snap assist", v.layouts[0].area),
            None => ("Windows", self.layout_box(None)),
        };
        let mut tree = crate::access::root(Role::Window, name, bounds);
        let focus = overview::access(
            &titles,
            v.model.selected,
            v.assist.is_some(),
            &v.layouts,
            bounds,
            &mut tree,
            ROOT,
        );
        tree.set_focus(focus);
        Some(tree)
    }

    /// The pointer moved to `pos`: the hot corner opens the overview only
    /// for a free pointer (no button, drag, move, modal or fullscreen window).
    pub fn hot_corner_motion(&mut self, x: f64, y: f64) {
        let corner = self.config.hot_corner;
        if corner == crate::config::HotCorner::None {
            return;
        }
        let bounds = self.layout_box(None);
        let fired = self.overview.corner.motion(corner, (x, y), bounds);
        if !fired
            || self.overview.view.is_some()
            || self.buttons_down > 0
            || self.drag_running()
            || self.panels.region.is_some()
            || !matches!(self.interaction, crate::input::Interaction::Idle)
        {
            return;
        }
        let out = self.output_at(x, y);
        let b = self.layout_box(out.as_ref());
        if self.fullscreen_on(b) {
            return;
        }
        self.open_overview();
    }
}

/// Snap assist's cards once the window list is `now`: the `offered`
/// windows still open, or None to close it when none is left or `leader`
/// has left its group, which a pick would join.
fn assist_offer(
    snap: &Snapping,
    leader: WindowId,
    offered: &[WindowId],
    now: &[WindowId],
) -> Option<Vec<WindowId>> {
    snap.group_of(leader)?;
    let left: Vec<WindowId> = (offered.iter().copied())
        .filter(|w| now.contains(w))
        .collect();
    (!left.is_empty()).then_some(left)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(syms: &[u32], mods: u32) -> DialogKey {
        DialogKey {
            syms: syms.to_vec(),
            mods,
            edit: None,
        }
    }

    #[test]
    fn a_released_fade_runs_on_from_where_the_fingers_left_it() {
        let (span, ms) = fade_run(0.6, true, 200);
        assert!((span.0 - 0.6).abs() < 1e-6);
        assert_eq!((span.1, ms), (1.0, 80));
        assert_eq!(fade_at(span, ms, 0.0), Some(0.6));
        let mid = fade_at(span, ms, 40.0).unwrap();
        assert!(mid > 0.8 && mid < 1.0, "eased past halfway: {mid}");
        assert_eq!(fade_at(span, ms, 80.0), None);
        let (span, ms) = fade_run(0.6, false, 200);
        assert_eq!((span.1, ms), (0.0, 120));
        assert!(fade_at(span, ms, 60.0).unwrap() < 0.3);
        assert_eq!(fade_run(1.0, true, 200).1, 40, "the shortest run");
        assert_eq!(
            fade_run(0.5, false, 0).1,
            1,
            "no animation ends on the next frame"
        );
    }

    #[test]
    fn keys_map_to_the_overview_s_own() {
        assert_eq!(key_of(&press(&[keys::KEY_Left], 0)), Some(Key::Left));
        assert_eq!(key_of(&press(&[keys::KEY_KP_Down], 0)), Some(Key::Down));
        assert_eq!(key_of(&press(&[keys::KEY_Tab], 0)), Some(Key::Tab));
        assert_eq!(
            key_of(&press(&[keys::KEY_Tab], keys::MOD_SHIFT)),
            Some(Key::BackTab)
        );
        assert_eq!(
            key_of(&press(&[keys::KEY_ISO_Left_Tab], keys::MOD_SHIFT)),
            Some(Key::BackTab)
        );
        assert_eq!(key_of(&press(&[keys::KEY_Return], 0)), Some(Key::Enter));
        assert_eq!(key_of(&press(&[keys::KEY_Escape], 0)), Some(Key::Escape));
        assert_eq!(key_of(&press(&[keys::KEY_a], 0)), None, "text goes on");
    }

    #[test]
    fn snap_assist_closes_once_its_snapped_window_leaves_the_group() {
        use crate::snap::{Splits, ZoneLayout};
        let (a, b, c, d) = (
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
        );
        let thirds = ZoneLayout::parse("1|1|1").unwrap();
        let splits = Splits::new(&thirds, Rect::new(0, 0, 1920, 1032));
        let mut snap = Snapping::default();
        snap.new_group(a, "DP-1", &thirds, splits, 0);
        snap.join_group(a, b, 1);
        assert_eq!(
            assist_offer(&snap, a, &[c, d], &[a, b, d]),
            Some(vec![d]),
            "a closed card goes"
        );
        assert_eq!(assist_offer(&snap, a, &[c], &[a, b]), None, "no card left");
        snap.forget_window(a);
        assert_eq!(
            assist_offer(&snap, a, &[c, d], &[b, c, d]),
            None,
            "the snapped window closed, though its group has another member"
        );
    }
}
