//! One output's backdrop blurred once for every piece of glass on it.
//!
//! A host that draws its own backdrop per output (the desktop beneath
//! glass at [`blur::backdrop_downscale`] of the output's resolution,
//! upright) hands it to the output's [`Picture`] after each render, with
//! where it changed. The picture redraws only the texels that read a
//! changed one: the coarse blur's 2:1 step over the change, the
//! horizontal pass over it grown by the blur's reach across, the vertical
//! pass and the streaks over it grown both ways. Every other texel keeps
//! its blur.
//!
//! A picture may serve only part of the output ([`Update::needed`]: the
//! glass of one window over another's frame): it then blurs only the
//! changes that reach that part and keeps the others for an update that
//! needs them.
//!
//! Glass on the output draws from it ([`Picture::source`]) as a plain
//! element: where the picture changed under a piece of glass is that
//! glass's damage, so a frame redraws only the glass near a change, and
//! nothing beneath it.

use std::sync::atomic::{AtomicU64, Ordering};

use reprisekit::glass::{Settings, streak};
use smithay::backend::renderer::Texture;
use smithay::backend::renderer::gles::{GlesError, GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::{DamageBag, DamageSnapshot};
use smithay::utils::{Buffer, Physical, Rectangle, Size, Transform};

use crate::blur::{self, Plan, Quality, Streaks};
use crate::element::GlassFx;

/// How many changes not needed yet a picture keeps apart; more are kept
/// as their bounds.
const PENDING_KEPT: usize = 16;

/// How many of the picture's changes a piece of glass can catch up on;
/// glass further behind draws whole.
const DAMAGE_AGE: usize = 8;

/// An output's backdrop, blurred and streaked, at the blur's resolution
/// (half the output's, a quarter for the coarse blur or at scale 2 and
/// up). Make one per output and update it after every render of the
/// backdrop.
#[derive(Debug)]
pub struct Picture {
    id: u64,
    /// The coarse blur's 2:1 step down from the backdrop.
    small: Option<GlesTexture>,
    across: Option<GlesTexture>,
    /// The vertical pass with the streaks over it: what glass shows.
    picture: Option<GlesTexture>,
    /// What the picture was last drawn for; anything else draws it whole.
    drawn: Option<Key>,
    /// The backdrop texture last made to clamp at its edges.
    clamped: Option<GlesTexture>,
    /// Backdrop changes not blurred yet (held by `blur_rate`, or not
    /// needed yet), output physical pixels as seen.
    pending: Vec<Rectangle<i32, Physical>>,
    /// The last update's [`Update::needed`], grown by what its texels
    /// read: the pending changes there are stale; None for all of them.
    reads: Option<Vec<Rectangle<i32, Physical>>>,
    /// [`Picture::refresh`]: the next update blurs, held or not.
    refresh: bool,
    /// Where the picture changed, output physical pixels as seen, grown by
    /// what a lookup into it reads.
    damage: DamageBag<i32, Physical>,
}

/// Everything besides the backdrop that the picture's texels depend on.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Key {
    plan: Plan,
    output: Size<i32, Physical>,
    scale: u64,
    streaks: bool,
}

impl Key {
    fn texels(self) -> Size<i32, Buffer> {
        let (o, f) = (self.output, self.plan.factor);
        ((o.w + f - 1) / f, (o.h + f - 1) / f).into()
    }

    fn extent(self) -> Rectangle<i32, Physical> {
        let t = self.texels();
        let f = self.plan.factor;
        Rectangle::from_size((t.w * f, t.h * f).into())
    }

    /// How far a texel's blur reads, output pixels: `2·pairs` texels
    /// either way along each pass.
    fn reach(self) -> i32 {
        self.plan.factor * 2 * self.plan.pairs as i32
    }
}

/// What one update of a [`Picture`] takes.
#[derive(Clone, Copy, Debug)]
pub struct Update<'a> {
    /// The backdrop: the output as seen at `1 / d` of its resolution
    /// (`(w + d - 1) / d` by `(h + d - 1) / d`), upright, with `d` the
    /// [`blur::backdrop_downscale`] of `scale`.
    pub backdrop: &'a GlesTexture,
    /// Where the backdrop changed since the last update, output physical
    /// pixels as seen.
    pub changed: &'a [Rectangle<i32, Physical>],
    pub settings: &'a Settings,
    /// The coarse blur while a window is dragged ([`blur::quality`]).
    pub quality: Quality,
    /// The output's size as seen, physical pixels, and its scale.
    pub output: Size<i32, Physical>,
    pub scale: f64,
    /// `[glass] blur_rate`'s cap is not up: keep the last blur and the
    /// changes for later ([`Picture::is_stale`]). A new blur, quality or
    /// output size draws anyway.
    pub hold: bool,
    /// The output pixels glass draws from this picture, physical as seen:
    /// only the changes their texels read are blurred, the others wait
    /// for an update that needs them. None: all of the output.
    pub needed: Option<&'a [Rectangle<i32, Physical>]>,
}

impl Default for Picture {
    fn default() -> Picture {
        Picture::new()
    }
}

impl Picture {
    pub fn new() -> Picture {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Picture {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            small: None,
            across: None,
            picture: None,
            drawn: None,
            clamped: None,
            pending: Vec::new(),
            reads: None,
            refresh: false,
            damage: DamageBag::new(DAMAGE_AGE),
        }
    }

    /// Blur what changed in the backdrop (or all of it when the blur, the
    /// streaks or the output changed) where it is needed, unless held.
    pub fn update(
        &mut self,
        renderer: &mut GlesRenderer,
        fx: &GlassFx,
        u: &Update<'_>,
    ) -> Result<(), GlesError> {
        let s = u.settings;
        let key = Key {
            plan: Plan::new(s.blur, s.material, u.quality, u.scale),
            output: u.output,
            scale: u.scale.to_bits(),
            streaks: streak::shown(s),
        };
        let (f, out, extent) = (key.plan.factor, key.texels(), key.extent());
        let whole = self.drawn != Some(key) || self.picture.is_none();
        if whole {
            self.pending = vec![extent];
        } else {
            self.pending
                .extend(u.changed.iter().filter(|r| !r.is_empty()).copied());
            if u.needed.is_some() {
                self.compact();
            }
        }
        // A needed texel reads the changes within the reach of it.
        let g = key.reach();
        self.reads = u.needed.map(|n| {
            n.iter()
                .filter_map(|r| grow(*r, g, g).intersection(extent))
                .collect()
        });
        let due = self.due();
        if !whole && (due.is_empty() || (u.hold && !self.refresh)) {
            return Ok(());
        }
        self.refresh = false;
        self.drawn = None;
        let left = std::mem::take(&mut self.pending);
        if let Some(reads) = &self.reads {
            self.pending = Rectangle::subtract_rects_many_in_place(left, reads.iter().copied());
            self.compact();
        }
        let d = blur::backdrop_downscale(u.scale);
        if f > d {
            blur::fit(&mut self.small, renderer, out)?;
        } else {
            self.small = None;
        }
        let mut across = blur::fit(&mut self.across, renderer, out)?;
        let mut picture = blur::fit(&mut self.picture, renderer, out)?;
        if self.clamped.as_ref().map(GlesTexture::tex_id) != Some(u.backdrop.tex_id()) {
            blur::clamp(renderer, u.backdrop)?;
            self.clamped = Some(u.backdrop.clone());
        }
        let parts = if whole && self.reads.is_none() {
            Parts::whole(extent)
        } else {
            Parts::of(&due, key)
        };
        let texels = |rects: &[Rectangle<i32, Physical>]| -> Vec<Rectangle<i32, Physical>> {
            rects
                .iter()
                .map(|r| {
                    Rectangle::new(
                        (r.loc.x / f, r.loc.y / f).into(),
                        (r.size.w / f, r.size.h / f).into(),
                    )
                })
                .collect()
        };
        let from = match self.small.clone() {
            Some(mut small) => {
                // Each texel the mean of the 2x2 backdrop texels under it.
                let k = f / d;
                let src = Rectangle::from_size((f64::from(out.w * k), f64::from(out.h * k)).into());
                let level = texels(&parts.level);
                blur::pass_from(renderer, u.backdrop, src, &mut small, None, &[], &level)?;
                small
            }
            None => u.backdrop.clone(),
        };
        let src = Rectangle::from_size((f64::from(out.w), f64::from(out.h)).into());
        let program = fx.blur_program();
        let uniforms = blur::gaussian(key.plan, (1.0 / out.w as f32, 0.0));
        let across_parts = texels(&parts.across);
        blur::pass_from(
            renderer,
            &from,
            src,
            &mut across,
            Some(program),
            &uniforms,
            &across_parts,
        )?;
        let uniforms = blur::gaussian(key.plan, (0.0, 1.0 / out.h as f32));
        let picture_parts = texels(&parts.picture);
        blur::pass_from(
            renderer,
            &across,
            src,
            &mut picture,
            Some(program),
            &uniforms,
            &picture_parts,
        )?;
        if let Some(t) = fx.streaks(renderer, s, u.output, u.scale)? {
            let streaks = Streaks {
                texture: &t,
                output: u.output,
                shown: extent,
                transform: Transform::Normal,
            };
            blur::streak_over(renderer, &streaks, &mut picture, &picture_parts)?;
        }
        // A lookup at an output pixel reads the texels either side of it.
        let output = Rectangle::from_size(u.output);
        self.damage.add(
            parts
                .picture
                .iter()
                .filter_map(|r| grow(*r, f, f).intersection(output)),
        );
        self.drawn = Some(key);
        Ok(())
    }

    /// Keep the pending changes few: overlapping ones merged, and past
    /// [`PENDING_KEPT`] the two whose bounds add the least joined, which
    /// blurs a little more once needed.
    fn compact(&mut self) {
        let mut p = merge(std::mem::take(&mut self.pending));
        while p.len() > PENDING_KEPT {
            let waste = |a: Rectangle<i32, Physical>, b: Rectangle<i32, Physical>| {
                area(a.merge(b)) - area(a) - area(b)
            };
            let Some((i, j)) = (0..p.len())
                .flat_map(|i| (i + 1..p.len()).map(move |j| (i, j)))
                .min_by_key(|&(i, j)| waste(p[i], p[j]))
            else {
                break;
            };
            p[i] = p[i].merge(p.swap_remove(j));
        }
        self.pending = p;
    }

    /// The pending changes the last update's needed texels read.
    fn due(&self) -> Vec<Rectangle<i32, Physical>> {
        match &self.reads {
            None => self.pending.clone(),
            Some(reads) => self
                .pending
                .iter()
                .flat_map(|p| reads.iter().filter_map(|r| r.intersection(*p)))
                .collect(),
        }
    }

    /// What glass on the output draws from; None before the first update.
    pub fn source(&self) -> Option<Source> {
        let (picture, key) = (self.picture.clone()?, self.drawn?);
        Some(Source {
            picture,
            factor: key.plan.factor,
            id: self.id,
            damage: self.damage.snapshot(),
        })
    }

    /// Whether a held update left backdrop changes unblurred where they
    /// are needed; [`Picture::refresh`] lets the next update blur them.
    pub fn is_stale(&self) -> bool {
        !self.due().is_empty()
    }

    /// Blur the held changes in the next update, held or not: what the
    /// host calls when `blur_rate`'s cap is up. Schedule a frame after it.
    pub fn refresh(&mut self) {
        self.refresh = true;
    }
}

/// What glass blurs when the host draws it: its output's [`Picture`] as it
/// stands.
#[derive(Clone)]
pub struct Source {
    pub(crate) picture: GlesTexture,
    pub(crate) factor: i32,
    /// Which picture: glass keeps how far it caught up with each.
    pub(crate) id: u64,
    pub(crate) damage: DamageSnapshot<i32, Physical>,
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Source")
            .field("picture", &self.picture.size())
            .field("factor", &self.factor)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Source {
    pub(crate) fn extent(&self) -> Rectangle<i32, Physical> {
        let s = self.picture.size();
        Rectangle::from_size((s.w * self.factor, s.h * self.factor).into())
    }
}

/// The parts of each level one update redraws, output physical pixels on
/// the blur's grid, none overlapping another of its level.
struct Parts {
    /// The backdrop's change on the grid: the coarse blur's 2:1 step.
    level: Vec<Rectangle<i32, Physical>>,
    /// Grown across by the reach: the horizontal pass.
    across: Vec<Rectangle<i32, Physical>>,
    /// Grown both ways: the vertical pass and the streaks.
    picture: Vec<Rectangle<i32, Physical>>,
}

impl Parts {
    fn whole(extent: Rectangle<i32, Physical>) -> Parts {
        Parts {
            level: vec![extent],
            across: vec![extent],
            picture: vec![extent],
        }
    }

    /// For backdrop changes `changed` under `key`.
    fn of(changed: &[Rectangle<i32, Physical>], key: Key) -> Parts {
        let (f, g, extent) = (key.plan.factor, key.reach(), key.extent());
        let level = merge(
            changed
                .iter()
                .filter_map(|r| snap(*r, f).intersection(extent))
                .collect(),
        );
        let across = merge(
            level
                .iter()
                .filter_map(|r| grow(*r, g, 0).intersection(extent))
                .collect(),
        );
        let picture = merge(
            level
                .iter()
                .filter_map(|r| grow(*r, g, g).intersection(extent))
                .collect(),
        );
        Parts {
            level,
            across,
            picture,
        }
    }
}

fn snap(r: Rectangle<i32, Physical>, f: i32) -> Rectangle<i32, Physical> {
    let x0 = r.loc.x.div_euclid(f) * f;
    let y0 = r.loc.y.div_euclid(f) * f;
    let x1 = (r.loc.x + r.size.w + f - 1).div_euclid(f) * f;
    let y1 = (r.loc.y + r.size.h + f - 1).div_euclid(f) * f;
    Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into())
}

fn grow(r: Rectangle<i32, Physical>, x: i32, y: i32) -> Rectangle<i32, Physical> {
    Rectangle::new(
        (r.loc.x - x, r.loc.y - y).into(),
        (r.size.w + 2 * x, r.size.h + 2 * y).into(),
    )
}

/// `rects` made disjoint so each texel is drawn once (the streaks blend): an
/// overlapping pair becomes its bounds only when those add no more than the pair shares.
fn merge(mut rects: Vec<Rectangle<i32, Physical>>) -> Vec<Rectangle<i32, Physical>> {
    let mut out: Vec<Rectangle<i32, Physical>> = Vec::with_capacity(rects.len());
    while let Some(r) = rects.pop() {
        if r.is_empty() {
            continue;
        }
        let Some(i) = out.iter().position(|o| o.overlaps(r)) else {
            out.push(r);
            continue;
        };
        let o = out[i];
        let bounds = o.merge(r);
        if area(bounds) <= area(o) + area(r) {
            out.swap_remove(i);
            rects.push(bounds);
        } else {
            rects.extend(r.subtract_rect(o));
        }
    }
    out
}

fn area(r: Rectangle<i32, Physical>) -> i64 {
    i64::from(r.size.w) * i64::from(r.size.h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::glass::Material;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn key(blur: u8) -> Key {
        Key {
            plan: Plan::new(blur, Material::Frosted, Quality::Full, 1.0),
            output: (400, 300).into(),
            scale: 1f64.to_bits(),
            streaks: false,
        }
    }

    #[test]
    fn a_change_redraws_what_reads_it_on_the_blur_grid() {
        let k = key(2);
        let g = k.reach();
        let p = Parts::of(&[r(101, 51, 10, 10)], k);
        assert_eq!(p.level, vec![r(100, 50, 12, 12)], "out to the 2 px grid");
        assert_eq!(
            p.across,
            vec![r(100 - g, 50, 12 + 2 * g, 12)],
            "across only"
        );
        assert_eq!(p.picture, vec![r(100 - g, 50 - g, 12 + 2 * g, 12 + 2 * g)]);
        let edge = Parts::of(&[r(0, 0, 4, 4)], k);
        assert_eq!(
            edge.picture,
            vec![r(0, 0, 4 + g, 4 + g)],
            "cut to the output"
        );
    }

    #[test]
    fn overlapping_parts_merge_so_each_texel_draws_once() {
        let changed = [r(100, 100, 10, 10), r(112, 100, 10, 10), r(300, 0, 2, 2)];
        let p = Parts::of(&changed, key(1));
        assert_eq!(
            p.picture.len(),
            2,
            "the two near changes are one part: {:?}",
            p.picture
        );
        for (i, a) in p.picture.iter().enumerate() {
            for b in &p.picture[i + 1..] {
                assert!(!a.overlaps(*b), "{a:?} and {b:?}");
            }
        }
    }

    /// The four sides of a 200x150 frame at (100, 100), 8 px wide.
    fn ring() -> Vec<Rectangle<i32, Physical>> {
        vec![
            r(100, 100, 200, 8),
            r(100, 242, 200, 8),
            r(100, 108, 8, 134),
            r(292, 108, 8, 134),
        ]
    }

    #[test]
    fn a_frames_sides_blur_without_the_hole_between_them() {
        let k = key(1);
        let g = k.reach();
        let p = Parts::of(&ring(), k);
        let hole = r(108 + g, 108 + g, 184 - 2 * g, 134 - 2 * g);
        assert!(!hole.is_empty(), "the blur reaches {g} px");
        for part in [&p.level, &p.across, &p.picture] {
            for (i, a) in part.iter().enumerate() {
                assert!(!a.overlaps(hole), "{a:?} covers the hole {hole:?}");
                for b in &part[i + 1..] {
                    assert!(!a.overlaps(*b), "{a:?} and {b:?}");
                }
            }
        }
        let drawn: i64 = p.picture.iter().map(|r| area(*r)).sum();
        let sides = grow(r(100, 100, 200, 150), g, g)
            .intersection(k.extent())
            .map_or(0, area)
            - area(hole);
        assert_eq!(drawn, sides, "every texel the sides reach, once");
    }

    #[test]
    fn many_pending_changes_join_the_nearest_and_leave_far_texels_out() {
        let mut p = Picture::new();
        p.pending = (0..20).map(|i| r(10 + 18 * i, 10, 4, 4)).collect();
        p.pending.push(r(10, 280, 4, 4));
        p.compact();
        assert!(p.pending.len() <= PENDING_KEPT, "{:?}", p.pending);
        let far = r(150, 150, 1, 1);
        assert!(
            p.pending.iter().all(|q| !q.overlaps(far)),
            "nothing between the row and the corner joins: {:?}",
            p.pending
        );
    }
}
