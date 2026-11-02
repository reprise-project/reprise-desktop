//! Mini views on the scene: a live picture of a window (`livepreview`) in `layers.minis`, drawn
//! as `reprisekit::controls::miniview`; docs/features.md has the behaviour.

use reprisekit::controls::miniview::{self, Layout, Look, Part};
use reprisescene::{BufferId, Input, TreeId};
use smithay::utils::Rectangle;

use crate::compositor::{SceneOwner, State};
use crate::glass::{self, Backdrop, Pane, PaneShape};
use crate::livepreview::ThumbId;
use crate::output::upload_scaled;
use crate::raster::Rect;
use crate::window::WindowId;

#[derive(Default)]
pub struct MiniViews {
    minis: Vec<Mini>,
    next: u64,
    /// The last click on a mini view's picture, for a double-click.
    click: reprisekit::pointer::DoubleClick<u64>,
}

struct Mini {
    id: u64,
    window: WindowId,
    /// The frame, in layout coordinates; the glass shadow reaches past it.
    rect: Rect,
    look: Look,
    /// At the frame's top-left, tagged `SceneOwner::MiniView`: the glass, the frame, the
    /// picture, then the close box.
    tree: TreeId,
    glass_tree: TreeId,
    backdrop: Backdrop,
    /// Made by `rebuild`.
    frame: Option<BufferId>,
    controls: Option<BufferId>,
    thumb: Option<ThumbId>,
    scale: i32,
    hot: bool,
}

impl Mini {
    fn layout(&self) -> Layout {
        miniview::layout(self.look, (self.rect.width, self.rect.height))
    }
}

impl MiniViews {
    pub fn of(&self, w: WindowId) -> Option<u64> {
        self.minis.iter().find(|m| m.window == w).map(|m| m.id)
    }

    pub fn window(&self, id: u64) -> Option<WindowId> {
        self.get(id).map(|m| m.window)
    }

    #[cfg(test)]
    fn thumb(&self, id: u64) -> Option<ThumbId> {
        self.get(id).and_then(|m| m.thumb)
    }

    pub fn rect(&self, id: u64) -> Option<Rect> {
        self.get(id).map(|m| m.rect)
    }

    fn get(&self, id: u64) -> Option<&Mini> {
        self.minis.iter().find(|m| m.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Mini> {
        self.minis.iter_mut().find(|m| m.id == id)
    }
}

/// Open a mini view of `w`, or close the one it has.
pub fn toggle(state: &mut State, w: WindowId) {
    match state.mini_views.of(w) {
        Some(id) => close(state, id),
        None => open(state, w),
    }
}

fn open(state: &mut State, w: WindowId) {
    let Some(win) = state.win(w) else {
        return;
    };
    if !state.has_picture(w) {
        log::info!("mini view: {:?} has nothing to show yet", win.app());
        return;
    }
    let aspect = (win.client_w, win.client_h);
    let r = win.frame_rect();
    let out = (state.output_at(f64::from(r.x + r.width / 2), f64::from(r.y + r.height / 2)))
        .or_else(|| state.first_output());
    let work = state.usable_area(out.as_ref());
    let look = miniview::look(&state.config.theme);
    let size = miniview::size_for(look, miniview::DEFAULT_W, aspect);
    let taken: Vec<Rect> = state.mini_views.minis.iter().map(|m| m.rect).collect();
    let rect = miniview::place(work, size, &taken);
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.minis);
    state.mini_views.next += 1;
    let id = state.mini_views.next;
    scene.set_data(tree, Some(SceneOwner::MiniView(id)));
    let glass_tree = scene.tree_create(tree);
    let mini = Mini {
        id,
        window: w,
        rect,
        look,
        tree,
        glass_tree,
        backdrop: Backdrop::new(&state.glass),
        frame: None,
        controls: None,
        thumb: None,
        scale: 0,
        hot: false,
    };
    state.mini_views.minis.push(mini);
    rebuild(state, id);
    log::info!(
        "mini view {id}: {:?} at {rect:?}",
        state.win(w).map(|x| x.app())
    );
}

/// Make `id`'s picture and drawing for its rect and the output under it.
fn rebuild(state: &mut State, id: u64) {
    let Some(m) = state.mini_views.get(id) else {
        return;
    };
    let (w, tree, rect) = (m.window, m.tree, m.rect);
    let out = state.output_at(
        f64::from(rect.x + rect.width / 2),
        f64::from(rect.y + rect.height / 2),
    );
    let scale = state.draw_scale_at(out.as_ref()).max(1);
    let layout = m.layout();
    let old = state.mini_views.get_mut(id).and_then(|m| m.thumb.take());
    if let Some(t) = old {
        state.thumb_drop(t);
    }
    let was = state.scene.outputs_of(tree);
    // Made again so the picture stays between the frame and the close box.
    let old = (state.mini_views.get_mut(id)).map(|m| [m.frame.take(), m.controls.take()]);
    for b in old.into_iter().flatten().flatten() {
        state.scene.destroy(b);
    }
    let scene = &mut state.scene;
    scene.set_position(tree, rect.x, rect.y);
    let frame = scene.buffer_create(tree);
    let (fw, fh) = layout.size;
    scene.set_input(
        frame,
        Input::Rect(Rectangle::new((0, 0).into(), (fw, fh).into())),
    );
    let thumb = state.thumb_create(w, tree, layout.picture, scale);
    let controls = state.scene.buffer_create(tree);
    state.scene.set_input(controls, Input::Passthrough);
    let pane = glass::wanted(&state.config).then(|| Pane {
        shapes: vec![PaneShape {
            rect: Rect::new(0, 0, fw, fh),
            corner: reprisekit::look::panel::glass(&state.config.theme)
                .map_or(&[][..], |(_, p)| p.corner),
            square_bottom: false,
            square_top: false,
        }],
        holes: Vec::new(),
    });
    if let Some(m) = state.mini_views.get_mut(id) {
        m.frame = Some(frame);
        m.controls = Some(controls);
        m.thumb = thumb;
        m.scale = scale;
        let glass_tree = m.glass_tree;
        m.backdrop
            .set(&mut state.scene, &state.glass, glass_tree, pane);
    }
    if thumb.is_none() {
        log::info!("mini view {id}: the window has no picture now, showing none");
    }
    draw(state, id);
    state.redraw_nodes(was, &[tree.into()]);
    crate::access::touch(state);
}

fn draw(state: &mut State, id: u64) {
    let Some(m) = state.mini_views.get(id) else {
        return;
    };
    let theme = state.config.theme.scaled(m.scale);
    let layout = m.layout();
    let (Some(frame), Some(controls)) = (m.frame, m.controls) else {
        return;
    };
    let (hot, scale, margin) = (m.hot, m.scale, m.look.margin);
    let canvas = miniview::draw_frame(&theme, &layout);
    upload_scaled(&mut state.scene, frame, &canvas, scale);
    state.scene.set_position(frame, -margin.left, -margin.top);
    let over = miniview::draw_controls(&theme, &layout);
    upload_scaled(&mut state.scene, controls, &over, scale);
    state.scene.set_enabled(controls, hot);
    if let Some(m) = state.mini_views.get(id) {
        let tree = m.tree;
        let was = state.scene.outputs_of(tree);
        state.redraw_nodes(was, &[tree.into()]);
    }
}

pub fn close(state: &mut State, id: u64) {
    let Some(i) = state.mini_views.minis.iter().position(|m| m.id == id) else {
        return;
    };
    let mut m = state.mini_views.minis.remove(i);
    if let Some(t) = m.thumb.take() {
        state.thumb_drop(t);
    }
    m.backdrop
        .set(&mut state.scene, &state.glass, m.glass_tree, None);
    state.destroy_redrawn(m.tree);
    crate::access::touch(state);
}

/// `w` is gone: its mini view goes too.
pub fn forget_window(state: &mut State, w: WindowId) {
    while let Some(id) = state.mini_views.of(w) {
        close(state, id);
    }
}

/// The theme, glass or an output's scale changed: every mini view draws again.
pub fn restyle(state: &mut State) {
    let ids: Vec<u64> = state.mini_views.minis.iter().map(|m| m.id).collect();
    for id in ids {
        let look = miniview::look(&state.config.theme);
        let aspect = state
            .mini_views
            .window(id)
            .and_then(|w| state.win(w))
            .map(|x| (x.client_w, x.client_h));
        if let (Some(m), Some(aspect)) = (state.mini_views.get_mut(id), aspect) {
            let (w, h) = miniview::size_for(look, m.rect.width, aspect);
            m.look = look;
            m.rect = Rect::new(m.rect.x, m.rect.y, w, h);
        }
        rebuild(state, id);
    }
}

/// The part of mini view `id` at layout point (`x`, `y`).
pub fn hit(state: &State, id: u64, x: f64, y: f64) -> Option<Part> {
    let m = state.mini_views.get(id)?;
    let (lx, ly) = (x.floor() as i32 - m.rect.x, y.floor() as i32 - m.rect.y);
    miniview::hit(&m.layout(), true, lx, ly)
}

/// The pointer is over mini view `id` (None: over none): it shows its close box.
pub fn hover(state: &mut State, id: Option<u64>) {
    let changed: Vec<u64> = (state.mini_views.minis.iter_mut())
        .filter_map(|m| {
            let hot = Some(m.id) == id;
            (std::mem::replace(&mut m.hot, hot) != hot).then_some(m.id)
        })
        .collect();
    for c in changed {
        draw(state, c);
        crate::access::touch(state);
    }
}

/// The cursor over `part`.
pub fn cursor(part: Option<Part>) -> &'static str {
    match part {
        Some(Part::Grip) => "se-resize",
        _ => "default",
    }
}

/// A left press on `part` of mini view `id` at `at`: the close box closes it, a second click on
/// the picture closes it and brings up its window, else a move or size starts.
pub fn press(state: &mut State, id: u64, part: Option<Part>, at: (f64, f64), time: u32) {
    let Some(rect) = state.mini_views.rect(id) else {
        return;
    };
    match part {
        Some(Part::Close) => close(state, id),
        Some(Part::Grip) => {
            state.interaction = crate::input::Interaction::MiniResize {
                id,
                start: rect,
                cx: at.0,
                cy: at.1,
            };
        }
        Some(Part::Body) => {
            let click = (state.config.theme.metrics).click(at.0 as i32, at.1 as i32, time);
            if state.mini_views.click.press(id, click) {
                let w = state.mini_views.window(id);
                close(state, id);
                if let Some(w) = w {
                    state.bring_up(w);
                }
                return;
            }
            state.interaction = crate::input::Interaction::MiniMove {
                id,
                dx: at.0 - f64::from(rect.x),
                dy: at.1 - f64::from(rect.y),
                press: at,
                moved: false,
            };
        }
        None => {}
    }
}

/// A held mini view follows the pointer at (`x`, `y`): moved once past `desktop::DRAG_PX`.
pub fn drag_to(state: &mut State, x: f64, y: f64) {
    let crate::input::Interaction::MiniMove {
        id,
        dx,
        dy,
        press,
        moved,
    } = state.interaction
    else {
        return;
    };
    let far = (x - press.0).abs().max((y - press.1).abs()) >= f64::from(crate::desktop::DRAG_PX);
    if !moved && !far {
        return;
    }
    state.interaction = crate::input::Interaction::MiniMove {
        id,
        dx,
        dy,
        press,
        moved: true,
    };
    let Some(m) = state.mini_views.get_mut(id) else {
        return;
    };
    m.rect.x = (x - dx).round() as i32;
    m.rect.y = (y - dy).round() as i32;
    let (tree, r) = (m.tree, m.rect);
    let was = state.scene.outputs_of(tree);
    state.scene.set_position(tree, r.x, r.y);
    state.redraw_nodes(was, &[tree.into()]);
}

/// A held grip sizes mini view `id` from `start` by the pointer's travel from (`cx`, `cy`).
pub fn size_to(state: &mut State, id: u64, start: Rect, (dx, dy): (f64, f64)) {
    let Some(m) = state.mini_views.get(id) else {
        return;
    };
    let Some(aspect) = state.win(m.window).map(|x| (x.client_w, x.client_h)) else {
        return;
    };
    let out = state.output_at(f64::from(start.x), f64::from(start.y));
    let work = state.usable_area(out.as_ref());
    let r = miniview::resized(m.look, start, aspect, (dx as i32, dy as i32), work);
    if r == m.rect {
        return;
    }
    if let Some(m) = state.mini_views.get_mut(id) {
        m.rect = r;
    }
    rebuild(state, id);
}

/// The release of a move that never moved: a click, which brings up the window.
pub fn released(state: &mut State, was: crate::input::Interaction) {
    match was {
        crate::input::Interaction::MiniMove {
            id, moved: false, ..
        } => {
            if let Some(w) = state.mini_views.window(id) {
                state.bring_up(w);
            }
        }
        crate::input::Interaction::MiniMove { id, .. } => {
            // Moved onto another output, it draws at that output's scale.
            let scale = state.mini_views.rect(id).map(|r| {
                let out =
                    state.output_at(f64::from(r.x + r.width / 2), f64::from(r.y + r.height / 2));
                state.draw_scale_at(out.as_ref()).max(1)
            });
            if scale != state.mini_views.get(id).map(|m| m.scale) {
                rebuild(state, id);
            }
        }
        _ => {}
    }
}

impl MiniViews {
    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.minis.iter().map(|m| m.id)
    }
}

/// Mini view `id`'s screen reader tree, placed where it stands.
pub fn access(state: &State, id: u64) -> Option<reprisekit::access::Tree> {
    use reprisekit::access::{ROOT, accesskit::Role};
    let m = state.mini_views.get(id)?;
    let title = state.win(m.window).map(|x| x.title()).unwrap_or_default();
    let mut tree = crate::access::root(Role::Window, "", m.rect);
    miniview::access(&m.layout(), &title, m.hot, &mut tree, ROOT);
    Some(tree)
}

#[cfg(test)]
mod tests {
    use crate::harness::{Harness, LEFT, TestClient, Toplevel};
    use crate::raster::Rect;

    fn desk() -> (Harness, TestClient, Toplevel) {
        let mut h = Harness::new();
        h.output("DP-1", 1920, 1080);
        let mut c = h.connect();
        let t = h.map_titled(&mut c, "foot", "top", (800, 600));
        (h, c, t)
    }

    fn open(h: &mut Harness, t: Toplevel) -> u64 {
        super::toggle(&mut h.state, t.id);
        h.dispatch();
        h.state.mini_views.of(t.id).expect("a mini view")
    }

    fn centre(r: Rect) -> (f64, f64) {
        (f64::from(r.x + r.width / 2), f64::from(r.y + r.height / 2))
    }

    #[test]
    fn a_mini_view_opens_320_wide_in_the_lower_right_above_every_window() {
        let (mut h, _c, t) = desk();
        let id = open(&mut h, t);
        let r = h.state.mini_views.rect(id).unwrap();
        let work = h.state.usable_area(h.state.scene.outputs().first());
        assert_eq!(r.width, 320);
        assert_eq!(
            (r.right(), r.bottom()),
            (work.right() - 16, work.bottom() - 16)
        );
        assert_eq!(r.height, 6 + 236, "the window's 4:3 picture");
        let m = h.state.mini_views.get(id).unwrap();
        assert_eq!(h.state.scene.parent(m.tree), Some(h.state.layers.minis));
        assert!(m.thumb.is_some(), "a live picture");
        super::toggle(&mut h.state, t.id);
        assert_eq!(
            h.state.mini_views.of(t.id),
            None,
            "Mini View again closes it"
        );
    }

    #[test]
    fn the_picture_follows_the_windows_commits_while_it_is_minimized_too() {
        let (mut h, mut c, t) = desk();
        let id = open(&mut h, t);
        h.state.set_minimized(t.id, true);
        assert_eq!(h.state.mini_views.of(t.id), Some(id), "minimizing keeps it");
        let thumb = h.state.mini_views.thumb(id).unwrap();
        assert!(!h.state.thumb_marked(thumb), "drawn as it opened");
        // Headless nothing draws, so the mark stays: it shows the commit reached the picture.
        h.draw_self(&mut c, t, 0xFF00_FF00);
        assert!(
            h.state.thumb_marked(thumb),
            "a minimized window's commit draws it again"
        );
    }

    #[test]
    fn the_mini_view_closes_with_its_window() {
        let (mut h, mut c, t) = desk();
        open(&mut h, t);
        c.destroy(t);
        h.roundtrip(&mut c);
        assert_eq!(h.state.mini_views.of(t.id), None);
        assert_eq!(h.state.scene.children(h.state.layers.minis).len(), 0);
    }

    #[test]
    fn a_click_brings_up_the_window_and_a_double_click_closes_the_mini_view() {
        let (mut h, mut c, t) = desk();
        let other = h.map_titled(&mut c, "ed", "notes", (400, 300));
        let id = open(&mut h, t);
        h.state.set_minimized(t.id, true);
        h.state.focus_window(other.id);
        let (x, y) = centre(h.state.mini_views.rect(id).unwrap());
        h.click(x, y);
        assert_eq!(h.state.focused, Some(t.id), "the window comes up");
        assert!(!h.state.win(t.id).unwrap().minimized);
        assert_eq!(h.state.mini_views.of(t.id), Some(id), "a click keeps it");
        h.state.focus_window(other.id);
        h.click(x + 1.0, y);
        assert_eq!(
            h.state.mini_views.of(t.id),
            None,
            "the second click closes it"
        );
        assert_eq!(h.state.focused, Some(t.id));
    }

    #[test]
    fn its_close_box_shows_under_the_pointer_and_closes_it() {
        let (mut h, _c, t) = desk();
        let id = open(&mut h, t);
        let r = h.state.mini_views.rect(id).unwrap();
        let close = h.state.mini_views.get(id).unwrap().layout().close;
        let (x, y) = (r.x + close.x + 2, r.y + close.y + 2);
        h.pointer_to(f64::from(x), f64::from(y));
        assert!(h.state.mini_views.get(id).unwrap().hot);
        h.click(f64::from(x), f64::from(y));
        assert_eq!(h.state.mini_views.of(t.id), None);
    }

    #[test]
    fn a_drag_moves_it_and_its_corner_sizes_it_at_the_windows_aspect() {
        let (mut h, _c, t) = desk();
        let id = open(&mut h, t);
        let r = h.state.mini_views.rect(id).unwrap();
        let (x, y) = centre(r);
        h.pointer_to(x, y);
        h.button(LEFT, true);
        h.pointer_to(x - 300.0, y - 200.0);
        h.button(LEFT, false);
        let moved = h.state.mini_views.rect(id).unwrap();
        assert_eq!((moved.x, moved.y), (r.x - 300, r.y - 200));
        let corner = (f64::from(moved.right() - 4), f64::from(moved.bottom() - 4));
        h.pointer_to(corner.0, corner.1);
        h.button(LEFT, true);
        h.pointer_to(corner.0 + 80.0, corner.1);
        h.button(LEFT, false);
        let sized = h.state.mini_views.rect(id).unwrap();
        assert_eq!((sized.x, sized.y, sized.width), (moved.x, moved.y, 400));
        assert_eq!(sized.height, 6 + 296, "still 4:3");
        assert_eq!(h.state.focused, Some(t.id), "neither gesture is a click");
    }
}
