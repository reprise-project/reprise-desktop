//! Client commits that show nothing new.
//!
//! Some clients commit the same pixels again under fresh damage (a terminal redrawing its caret cell
//! twice a second with nothing changed). Each such commit would cost a screen frame and a page flip and
//! wake the GPU of an idle desktop.
//!
//! For every `wl_shm` buffer a surface commits, each small damage rect's pixels are compared with what
//! that rect held at its last commit (`Kept`). A commit whose every damage rect holds the pixels it
//! already had, and that changed nothing else about how its window shows (`Commit::shows_nothing_new`),
//! asks for no frame: the screen already shows those pixels. Its frame callbacks are answered on the
//! screen's pace (`backend::answer_frames`). The surface keeps the damage, so the next frame rendered
//! for another reason uploads it.
//!
//! `Kept` remembers a rect's pixels only while nothing else has touched it: a commit whose damage meets
//! a kept rect in any other shape forgets it, and a buffer of another size or format, surface-coordinate
//! damage or a non-shm buffer forgets all of them.

use crate::compositor::State;
use crate::raster::Rect;
use crate::window::WindowId;
use smithay::backend::renderer::utils::{SurfaceView, with_renderer_surface_state};
use smithay::reexports::wayland_server::protocol::{wl_shm, wl_surface::WlSurface};
use smithay::reexports::wayland_server::{Resource, backend::ObjectId};
use smithay::utils::{Logical, Point, Rectangle, Size};
use smithay::wayland::compositor::{
    BufferAssignment, Damage, SubsurfaceCachedState, SurfaceAttributes, get_children, with_states,
};
use std::cell::RefCell;

const MAX_AREA: i32 = 4096;
const MAX_KEPT: usize = 8;
const MAX_CHECKED: usize = 16;

/// A buffer's pixel layout: size and format. Kept pixels are only compared within one layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub width: i32,
    pub height: i32,
    pub format: u32,
}

/// The pixels a surface last committed under the rects it damaged, as far as they are known.
#[derive(Default)]
pub struct Kept {
    layout: Option<Layout>,
    rects: Vec<(Rect, Vec<u8>)>,
}

#[derive(Debug, PartialEq)]
pub enum Plan {
    Keep,
    Forget,
    Upload {
        layout: Layout,
        damage: Vec<Rect>,
        same: Vec<Rect>,
        fresh: Vec<(Rect, Vec<u8>)>,
    },
}

impl Plan {
    pub fn pixels(&self) -> Pixels {
        match self {
            Plan::Keep => Pixels::Kept,
            Plan::Upload { damage, same, .. }
                if !damage.is_empty() && same.len() == damage.len() =>
            {
                Pixels::Same
            }
            _ => Pixels::Changed,
        }
    }
}

impl Kept {
    /// The plan for a buffer of `layout` committed under `damage`, with `pixels` reading a rect of it.
    pub fn plan(
        &self,
        layout: Layout,
        damage: &[Rect],
        mut pixels: impl FnMut(Rect) -> Vec<u8>,
    ) -> Plan {
        let mut same = Vec::new();
        let mut fresh = Vec::new();
        for r in damage.iter().filter(|r| is_small(**r)).take(MAX_CHECKED) {
            let now = pixels(*r);
            let known = self.layout == Some(layout)
                && self.rects.iter().any(|(k, px)| k == r && *px == now);
            if known {
                same.push(*r);
            } else {
                fresh.push((*r, now));
            }
        }
        Plan::Upload {
            layout,
            damage: damage.to_vec(),
            same,
            fresh,
        }
    }

    pub fn apply(&mut self, plan: Plan) {
        let (layout, damage, same, fresh) = match plan {
            Plan::Keep => return,
            Plan::Forget => {
                *self = Kept::default();
                return;
            }
            Plan::Upload {
                layout,
                damage,
                same,
                fresh,
            } => (layout, damage, same, fresh),
        };
        if self.layout != Some(layout) {
            *self = Kept {
                layout: Some(layout),
                rects: Vec::new(),
            };
        }
        let uploaded: Vec<Rect> = damage.into_iter().filter(|r| !same.contains(r)).collect();
        self.rects
            .retain(|(k, _)| uploaded.iter().all(|r| r.intersect(k).is_empty()));
        for r in &same {
            if let Some(i) = self.rects.iter().position(|(k, _)| k == r) {
                let e = self.rects.remove(i);
                self.rects.push(e);
            }
        }
        self.rects.extend(fresh);
        let over = self.rects.len().saturating_sub(MAX_KEPT);
        self.rects.drain(..over);
    }
}

fn is_small(r: Rect) -> bool {
    !r.is_empty() && r.width * r.height <= MAX_AREA
}

fn bytes_per_pixel(format: wl_shm::Format) -> Option<usize> {
    use wl_shm::Format as F;
    matches!(
        format,
        F::Argb8888
            | F::Xrgb8888
            | F::Abgr8888
            | F::Xbgr8888
            | F::Argb2101010
            | F::Xrgb2101010
            | F::Abgr2101010
            | F::Xbgr2101010
    )
    .then_some(4)
}

/// The bytes of `r` (inside the buffer) in a buffer of `stride` bytes per row and `bpp` bytes per pixel, row after row.
fn read(bytes: &[u8], stride: usize, bpp: usize, r: Rect) -> Vec<u8> {
    let (x, w) = (r.x.max(0) as usize * bpp, r.width.max(0) as usize * bpp);
    let mut out = Vec::with_capacity(w * r.height.max(0) as usize);
    for y in r.y.max(0)..r.bottom() {
        let at = y as usize * stride + x;
        out.extend_from_slice(&bytes[at..at + w]);
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pixels {
    Kept,
    Same,
    Changed,
}

#[derive(Clone, Copy, Debug)]
pub struct Commit {
    pub pixels: Pixels,
    /// The surface's buffer size, scale, transform, viewport and opaque region are unchanged.
    pub view_kept: bool,
    pub tree_kept: bool,
    /// The surface belongs to a mapped window whose place and size are unchanged.
    pub window_kept: bool,
}

impl Commit {
    pub fn shows_nothing_new(&self) -> bool {
        self.pixels != Pixels::Changed && self.view_kept && self.tree_kept && self.window_kept
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    view: Option<SurfaceView>,
    size: Option<Size<i32, Logical>>,
    opaque: Option<Vec<Rectangle<i32, Logical>>>,
}

/// Where a window is and how big, as its commit leaves it: a commit that maps, unmaps, moves or resizes changes this.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMark {
    pub mapped: bool,
    at: (i32, i32),
    client: (i32, i32),
    surface_at: Option<(Point<i32, Logical>, bool)>,
}

impl State {
    pub fn window_mark(&self, w: WindowId) -> Option<WindowMark> {
        let win = self.win(w)?;
        Some(WindowMark {
            mapped: win.mapped,
            at: (win.x, win.y),
            client: (win.client_w, win.client_h),
            surface_at: self.scene.coords(win.surface),
        })
    }
}

pub fn view(surface: &WlSurface) -> Option<View> {
    with_renderer_surface_state(surface, |s| View {
        view: s.view(),
        size: s.buffer_size(),
        opaque: s.opaque_regions().map(<[_]>::to_vec),
    })
}

/// What `check` keeps per surface: its pixels and the place of its subsurfaces as of its last commit.
#[derive(Default)]
struct Watch {
    kept: Kept,
    tree: Option<Tree>,
}

/// Where a surface sits in its parent (if a subsurface) and where its subsurfaces sit in it, in stacking order.
#[derive(Clone, Debug, PartialEq)]
struct Tree {
    at: Point<i32, Logical>,
    children: Vec<(ObjectId, Point<i32, Logical>)>,
}

fn location(surface: &WlSurface) -> Point<i32, Logical> {
    with_states(surface, |s| {
        s.cached_state
            .get::<SubsurfaceCachedState>()
            .current()
            .location
    })
}

/// Read `surface`'s just-applied state before the buffer handler takes its buffer: what its new buffer
/// did to its pixels (kept), and whether its subsurfaces, or it in its parent, stayed put.
pub fn check(surface: &WlSurface) -> (Pixels, bool) {
    let tree = Tree {
        at: location(surface),
        children: get_children(surface)
            .iter()
            .map(|c| (c.id(), location(c)))
            .collect(),
    };
    with_states(surface, |states| {
        let watch = states
            .data_map
            .get_or_insert(|| RefCell::new(Watch::default()));
        let mut watch = watch.borrow_mut();
        let mut attrs = states.cached_state.get::<SurfaceAttributes>();
        let plan = plan(attrs.current(), &watch.kept);
        let pixels = plan.pixels();
        watch.kept.apply(plan);
        let tree_kept = watch.tree.as_ref() == Some(&tree);
        watch.tree = Some(tree);
        (pixels, tree_kept)
    })
}

fn plan(attrs: &SurfaceAttributes, kept: &Kept) -> Plan {
    let buffer = match &attrs.buffer {
        None => return Plan::Keep,
        Some(BufferAssignment::Removed) => return Plan::Forget,
        Some(BufferAssignment::NewBuffer(b)) => b,
    };
    if attrs.buffer_delta.is_some_and(|d| d.x != 0 || d.y != 0) {
        return Plan::Forget;
    }
    let mut damage = Vec::with_capacity(attrs.damage.len());
    for d in &attrs.damage {
        match d {
            Damage::Buffer(r) => damage.push(Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h)),
            Damage::Surface(_) => return Plan::Forget,
        }
    }
    let planned = smithay::wayland::shm::with_buffer_contents(buffer, |ptr, len, data| {
        let layout = Layout {
            width: data.width,
            height: data.height,
            format: u32::from(data.format),
        };
        let bounds = Rect::new(0, 0, layout.width, layout.height);
        let damage: Vec<Rect> = damage
            .iter()
            .map(|r| r.intersect(&bounds))
            .filter(|r| !r.is_empty())
            .collect();
        let Some(bpp) = bytes_per_pixel(data.format) else {
            return Plan::Forget;
        };
        if !damage.iter().any(|r| is_small(*r)) {
            return kept.plan(layout, &damage, |_| Vec::new());
        }
        let (offset, stride) = (data.offset.max(0) as usize, data.stride.max(0) as usize);
        let end = offset + stride * layout.height.max(0) as usize;
        if end > len || stride < layout.width.max(0) as usize * bpp {
            return Plan::Forget;
        }
        // SAFETY: the pool maps `len` bytes at `ptr` for this call and the buffer's rows lie within them (checked above).
        let bytes = unsafe { std::slice::from_raw_parts(ptr.add(offset), end - offset) };
        kept.plan(layout, &damage, |r| read(bytes, stride, bpp, r))
    });
    planned.unwrap_or(Plan::Forget)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUT: Layout = Layout {
        width: 100,
        height: 50,
        format: 0,
    };

    fn buffer(f: impl Fn(i32, i32) -> u8) -> Vec<u8> {
        (0..50)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .map(|(x, y)| f(x, y))
            .collect()
    }

    fn commit(kept: &mut Kept, buf: &[u8], damage: &[Rect]) -> Vec<Rect> {
        let plan = kept.plan(LAYOUT, damage, |r| read(buf, 100, 1, r));
        let same = match &plan {
            Plan::Upload { same, .. } => same.clone(),
            _ => unreachable!(),
        };
        kept.apply(plan);
        same
    }

    const CARET: Rect = Rect::new(10, 20, 6, 16);

    #[test]
    fn a_rect_committed_again_with_the_same_pixels_is_same() {
        let mut kept = Kept::default();
        let buf = buffer(|x, y| (x + y) as u8);
        assert_eq!(commit(&mut kept, &buf, &[CARET]), vec![]);
        assert_eq!(commit(&mut kept, &buf, &[CARET]), vec![CARET]);
        assert_eq!(commit(&mut kept, &buf, &[CARET]), vec![CARET]);
    }

    #[test]
    fn a_rect_whose_pixels_changed_is_not_same() {
        let mut kept = Kept::default();
        commit(&mut kept, &buffer(|_, _| 1), &[CARET]);
        assert_eq!(
            commit(&mut kept, &buffer(|x, _| (x == 12) as u8), &[CARET]),
            vec![]
        );
        assert_eq!(
            commit(&mut kept, &buffer(|x, _| (x == 12) as u8), &[CARET]),
            vec![CARET]
        );
    }

    #[test]
    fn a_blinking_caret_changes_on_every_commit() {
        let mut kept = Kept::default();
        let on = buffer(|x, y| CARET.contains(x, y) as u8);
        let off = buffer(|_, _| 0);
        commit(&mut kept, &on, &[CARET]);
        for _ in 0..3 {
            assert_eq!(commit(&mut kept, &off, &[CARET]), vec![]);
            assert_eq!(commit(&mut kept, &on, &[CARET]), vec![]);
        }
    }

    #[test]
    fn damage_over_part_of_a_kept_rect_forgets_it() {
        let mut kept = Kept::default();
        let old = buffer(|_, _| 1);
        commit(&mut kept, &old, &[CARET]);
        commit(&mut kept, &buffer(|_, _| 2), &[Rect::new(0, 30, 100, 10)]);
        // The caret cell is repainted over a wider damage, then the old pixels are committed there again: the screen holds 2s
        // in the lower rows, so the caret's damage must show.
        assert_eq!(commit(&mut kept, &old, &[CARET]), vec![]);
    }

    #[test]
    fn a_new_layout_forgets_every_rect() {
        let mut kept = Kept::default();
        let buf = buffer(|_, _| 7);
        commit(&mut kept, &buf, &[CARET]);
        let wide = Layout {
            width: 120,
            ..LAYOUT
        };
        let plan = kept.plan(wide, &[CARET], |r| read(&buf, 100, 1, r));
        assert!(matches!(&plan, Plan::Upload { same, .. } if same.is_empty()));
        kept.apply(plan);
        kept.apply(Plan::Upload {
            layout: LAYOUT,
            damage: vec![],
            same: vec![],
            fresh: vec![],
        });
        assert_eq!(commit(&mut kept, &buf, &[CARET]), vec![]);
    }

    #[test]
    fn a_forget_plan_forgets_and_a_keep_plan_keeps() {
        let mut kept = Kept::default();
        let buf = buffer(|_, _| 3);
        commit(&mut kept, &buf, &[CARET]);
        kept.apply(Plan::Keep);
        assert_eq!(commit(&mut kept, &buf, &[CARET]), vec![CARET]);
        kept.apply(Plan::Forget);
        assert_eq!(commit(&mut kept, &buf, &[CARET]), vec![]);
    }

    #[test]
    fn large_rects_are_never_compared() {
        let mut kept = Kept::default();
        let buf = buffer(|_, _| 3);
        let big = Rect::new(0, 0, 100, 41);
        commit(&mut kept, &buf, &[big]);
        assert_eq!(commit(&mut kept, &buf, &[big]), vec![]);
    }

    #[test]
    fn only_the_most_recent_rects_are_kept() {
        let mut kept = Kept::default();
        let buf = buffer(|_, _| 3);
        let cell = |i: i32| Rect::new(i * 10, 0, 4, 4);
        commit(&mut kept, &buf, &[cell(0)]);
        for i in 1..=MAX_KEPT as i32 {
            commit(&mut kept, &buf, &[cell(i)]);
            if i < 4 {
                assert_eq!(commit(&mut kept, &buf, &[cell(0)]), vec![cell(0)]);
            }
        }
        assert_eq!(commit(&mut kept, &buf, &[cell(1)]), vec![]);
        assert_eq!(commit(&mut kept, &buf, &[cell(0)]), vec![cell(0)]);
    }

    #[test]
    fn a_commit_without_small_rects_reads_no_pixels() {
        let kept = Kept::default();
        let plan = kept.plan(LAYOUT, &[Rect::new(0, 0, 100, 50)], |_| unreachable!());
        assert!(
            matches!(plan, Plan::Upload { same, fresh, .. } if same.is_empty() && fresh.is_empty())
        );
    }

    #[test]
    fn reads_a_rect_row_by_row() {
        let buf = buffer(|x, y| (x + 10 * y) as u8);
        assert_eq!(
            read(&buf, 100, 1, Rect::new(2, 3, 2, 2)),
            vec![32, 33, 42, 43]
        );
    }

    #[test]
    fn only_32_bit_formats_are_compared() {
        assert_eq!(bytes_per_pixel(wl_shm::Format::Xrgb8888), Some(4));
        assert_eq!(bytes_per_pixel(wl_shm::Format::Rgb565), None);
    }

    fn upload(damage: &[Rect], same: &[Rect]) -> Plan {
        Plan::Upload {
            layout: LAYOUT,
            damage: damage.to_vec(),
            same: same.to_vec(),
            fresh: vec![],
        }
    }

    #[test]
    fn a_buffer_is_the_same_only_when_every_damage_rect_is() {
        let row = Rect::new(0, 20, 100, 16);
        assert_eq!(upload(&[CARET], &[CARET]).pixels(), Pixels::Same);
        assert_eq!(upload(&[CARET, row], &[CARET]).pixels(), Pixels::Changed);
        assert_eq!(
            upload(&[], &[]).pixels(),
            Pixels::Changed,
            "a new buffer without damage"
        );
        assert_eq!(Plan::Keep.pixels(), Pixels::Kept);
        assert_eq!(Plan::Forget.pixels(), Pixels::Changed);
    }

    #[test]
    fn a_commit_shows_nothing_new_only_when_nothing_about_its_window_moved() {
        let quiet = Commit {
            pixels: Pixels::Same,
            view_kept: true,
            tree_kept: true,
            window_kept: true,
        };
        assert!(quiet.shows_nothing_new());
        assert!(
            Commit {
                pixels: Pixels::Kept,
                ..quiet
            }
            .shows_nothing_new(),
            "a commit for a frame callback alone"
        );
        for loud in [
            Commit {
                pixels: Pixels::Changed,
                ..quiet
            },
            Commit {
                view_kept: false,
                ..quiet
            },
            Commit {
                tree_kept: false,
                ..quiet
            },
            Commit {
                window_kept: false,
                ..quiet
            },
        ] {
            assert!(!loud.shows_nothing_new(), "{loud:?} asks for a frame");
        }
    }
}
