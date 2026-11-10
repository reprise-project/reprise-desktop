//! A change of the login screen played from rasters drawn once: the chrome, the screen before and
//! the screen after, mixed inside the rect where they differ, with Glass's picked frame flying on
//! top.

use crate::anim::Ease;
use crate::glass::logon::{self as glass, Pose};
use crate::interface::Interface;
use crate::logon::{Change, Logon, LogonLayout, Motion, Part, Plan, Show};
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;

use super::logon_layout;

/// The host keeps one while a change plays and asks [`Transition::frame`] for each frame.
pub struct Transition {
    pub change: Change,
    plan: Plan,
    s: i32,
    /// The chrome of the screen after.
    base: Canvas,
    was: Option<Canvas>,
    now: Canvas,
    rect: Rect,
    flight: Option<Flight>,
}

struct Flight {
    part: Part,
    from: Pose,
    to: Pose,
    ms: u32,
}

impl Flight {
    fn at(&self, at: u32) -> Pose {
        let u = Ease::OutCubic.apply(at.min(self.ms) as f32 / self.ms.max(1) as f32);
        Pose::between(&self.from, &self.to, f64::from(u))
    }
}

/// What a transition draws with: the screen after, its layout, and how it is drawn.
pub struct Drawn<'a> {
    pub m: &'a Logon,
    pub l: &'a LogonLayout,
    pub theme: &'a Theme,
    /// The unscaled renderer the layout measures with.
    pub text: &'a TextRenderer,
    pub s: i32,
}

impl Drawn<'_> {
    fn canvas(&self, m: &Logon, l: &LogonLayout, show: Show) -> Canvas {
        let (o, s) = (l.output, self.s);
        let mut c = Canvas::new((o.width * s) as u32, (o.height * s) as u32);
        let text = self.text.for_output(s);
        match m.interface {
            Interface::Classic => crate::classic::logon::draw(&mut c, m, l, self.theme, text, s),
            Interface::Blue => {
                crate::blue::logon::draw_shown(&mut c, m, l, self.theme, text, s, show)
            }
            Interface::Glass => glass::draw_shown(&mut c, m, l, self.theme, text, s, show),
        }
        c
    }

    /// The screen a change began from, its ring at the current step, and its layout.
    fn before(&self, motion: &Motion) -> (Logon, LogonLayout) {
        let mut was = motion.was.model().clone();
        was.spin = self.m.spin;
        let l = logon_layout(&was, self.theme, self.text, self.l.output);
        (was, l)
    }
}

/// How `change` plays in `d.m`'s interface; None when it shows at once.
fn plan(change: Change, d: &Drawn) -> Option<Plan> {
    let plan = match d.m.interface {
        Interface::Classic => None,
        Interface::Blue => crate::blue::logon::plan(change),
        Interface::Glass => Some(glass::plan(change, d.m, d.l, d.s)),
    }?;
    (plan.ms() > 0).then_some(plan)
}

/// `r` of `c` set to `from`'s pixels there; both are the screen's canvas.
fn copy_in(c: &mut Canvas, from: &Canvas, r: Rect) {
    debug_assert_eq!((c.width, c.height, c.scale()), (from.width, from.height, 1));
    let (w, n) = (c.width as usize, r.width as usize);
    for y in r.y..r.bottom() {
        let i = y as usize * w + r.x as usize;
        c.pixels[i..i + n].copy_from_slice(&from.pixels[i..i + n]);
    }
}

/// The smallest rect holding both, either of which may be empty.
fn join(a: Rect, b: Rect) -> Rect {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b,
        (_, true) => a,
        _ => a.union(&b),
    }
}

impl Transition {
    /// `motion` on the screen `d` draws; None when it shows at once or changes nothing.
    pub fn begin(motion: &Motion, d: &Drawn) -> Option<Transition> {
        let plan = plan(motion.change, d)?;
        let (was_m, was_l) = d.before(motion);
        let flight = plan.carried.and_then(|(part, ms)| {
            let from = glass::pose(&was_m, &was_l, d.theme, d.s, part)?;
            let to = glass::pose(d.m, d.l, d.theme, d.s, part)?;
            Some(Flight { part, from, to, ms })
        });
        let show = flight.as_ref().map_or(Show::All, |f| Show::AllBut(f.part));
        let was = plan.out.map(|_| d.canvas(&was_m, &was_l, show));
        Transition::build(motion.change, plan, d, was, flight, Rect::default())
    }

    /// `motion` begun `at` ms into `prev`: the screen before is `prev`'s frame then, and a frame
    /// flying in both flies on from where it is.
    pub fn then(prev: &Transition, at: u32, motion: &Motion, d: &Drawn) -> Option<Transition> {
        let mut plan = plan(motion.change, d)?;
        let carried = plan
            .carried
            .filter(|c| prev.flight.as_ref().is_some_and(|f| f.part == c.0));
        plan.carried = carried;
        let flight = carried.and_then(|(part, ms)| {
            let from = prev.flight.as_ref()?.at(at);
            let to = glass::pose(d.m, d.l, d.theme, d.s, part)?;
            Some(Flight { part, from, to, ms })
        });
        let mut was = prev.base.clone();
        prev.compose(&mut was, d.m, at, flight.is_none());
        let was = plan.out.map(|_| was);
        Transition::build(motion.change, plan, d, was, flight, prev.rect)
    }

    fn build(
        change: Change,
        plan: Plan,
        d: &Drawn,
        was: Option<Canvas>,
        flight: Option<Flight>,
        also: Rect,
    ) -> Option<Transition> {
        let base = d.canvas(d.m, d.l, Show::Chrome);
        let show = flight.as_ref().map_or(Show::All, |f| Show::AllBut(f.part));
        let now = d.canvas(d.m, d.l, show);
        let mut rect = join(also, base.differing_rect(&now));
        if let Some(was) = &was {
            rect = join(rect, base.differing_rect(was));
        }
        if let Some(f) = &flight {
            rect = join(rect, join(f.from.reach(d.s), f.to.reach(d.s)));
        }
        let rise = plan.into.iter().map(|s| s.rise).max().unwrap_or(0);
        rect.height += rise;
        let rect = rect.intersect(&base.bounds());
        if rect.is_empty() {
            return None;
        }
        Some(Transition {
            change,
            plan,
            s: d.s,
            base,
            was,
            now,
            rect,
            flight,
        })
    }

    /// Draw the screen after again, as it changed while this plays (a key typed, a part lit).
    pub fn refresh_now(&mut self, d: &Drawn) {
        let show = (self.flight.as_ref()).map_or(Show::All, |f| Show::AllBut(f.part));
        self.now = d.canvas(d.m, d.l, show);
        if let Some(f) = &mut self.flight
            && let Some(to) = glass::pose(d.m, d.l, d.theme, d.s, f.part)
        {
            f.to = to;
        }
        let changed = self.base.differing_rect(&self.now);
        self.rect = join(self.rect, changed).intersect(&self.base.bounds());
    }

    /// How long it plays, ms.
    pub fn ms(&self) -> u32 {
        self.plan.ms()
    }

    /// The device rect every frame draws into; the rest of the canvas stays as it was.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Where the flying frame is `at` ms in, device px.
    pub fn carried_at(&self, at: u32) -> Option<Rect> {
        self.flight.as_ref().map(|f| f.at(at).rect)
    }

    /// The frame `at` ms in, drawn into [`Transition::rect`] of `c` (the canvas of the whole
    /// screen); `m` is the screen after. Returns the rect drawn.
    pub fn frame(&self, c: &mut Canvas, m: &Logon, at: u32) -> Rect {
        self.compose(c, m, at, true);
        self.rect
    }

    fn compose(&self, c: &mut Canvas, m: &Logon, at: u32, carried: bool) {
        let r = self.rect;
        copy_in(c, &self.base, r);
        let out = (self.was.as_ref().zip(self.plan.out))
            .map(|(w, win)| (w, 1.0 - Ease::InOut.apply(win.at(at))));
        let level = |w: crate::logon::Window| Ease::InOut.apply(w.at(at));
        match self.plan.into.as_slice() {
            [one] if one.columns.is_none() && one.rise == 0 => {
                let all: Vec<(&Canvas, f32)> = out
                    .into_iter()
                    .chain([(&self.now, level(one.window))])
                    .collect();
                c.mix_all(&all, r);
            }
            slices => {
                if let Some((w, l)) = out {
                    c.mix(w, r, l);
                }
                for sl in slices {
                    let (x0, x1) = sl.columns.unwrap_or((r.x, r.right()));
                    let (x0, x1) = (x0.max(r.x), x1.min(r.right()));
                    let src = Rect::new(x0, r.y, x1 - x0, r.height - sl.rise);
                    let up = 1.0 - Ease::OutCubic.apply(sl.window.at(at));
                    let dy = (sl.rise as f32 * up).round() as i32;
                    c.mix_shifted(&self.now, src, (0, dy), level(sl.window));
                }
            }
        }
        if carried && let Some(f) = &self.flight {
            glass::draw_posed(c, m, f.part, &f.at(at), self.s);
        }
    }
}

#[cfg(test)]
mod tests;
