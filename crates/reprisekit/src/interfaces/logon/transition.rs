//! A login screen change played from rasters drawn once (the chrome, the screen before and after)
//! mixed inside the rect where they differ, Glass's picked frame flying on top.

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
    /// The device rect of the screen's canvas that `under`, `was` and `now` hold; outside it the
    /// screens before and after and the chrome are alike.
    area: Rect,
    /// What each frame starts from: the chrome of the screen after, or for a crossfade the screen
    /// before.
    under: Canvas,
    /// The screen before, faded out over `under`; None for a crossfade or when nothing fades out.
    was: Option<Canvas>,
    now: Canvas,
    rect: Rect,
    /// How far below its place a slice starts, device px; the rect keeps this much under the
    /// screen after.
    rise: i32,
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
    fn size(&self) -> (u32, u32) {
        let (o, s) = (self.l.output, self.s);
        ((o.width * s) as u32, (o.height * s) as u32)
    }

    fn bounds(&self) -> Rect {
        let (w, h) = self.size();
        Rect::new(0, 0, w as i32, h as i32)
    }

    fn draw(&self, c: &mut Canvas, m: &Logon, l: &LogonLayout, show: Show) {
        let (theme, s) = (self.theme, self.s);
        let text = self.text.for_output(s);
        match m.interface {
            Interface::Classic => crate::classic::logon::draw(c, m, l, theme, text, s),
            Interface::Blue => crate::blue::logon::draw_shown(c, m, l, theme, text, s, show),
            Interface::Glass => glass::draw_shown(c, m, l, theme, text, s, show),
        }
    }

    fn canvas(&self, m: &Logon, l: &LogonLayout, show: Show) -> Canvas {
        let (w, h) = self.size();
        let mut c = Canvas::new(w, h);
        self.draw(&mut c, m, l, show);
        c
    }

    /// `m` on `l` drawn into `c`, the screen's canvas, inside device rects `within` alone.
    fn draw_within(
        &self,
        c: &mut Canvas,
        (m, l): (&Logon, &LogonLayout),
        show: Show,
        within: &[Rect],
    ) {
        for &r in within {
            c.with_clip(r, |c| {
                c.clear_rect(r);
                self.draw(c, m, l, show);
            });
        }
    }

    /// The screen after, as [`Drawn::draw_within`] takes it.
    fn after(&self) -> (&Logon, &LogonLayout) {
        (self.m, self.l)
    }

    /// Design rects within the output as device rects on the screen's canvas, empty ones left out.
    fn device(&self, rects: &[Rect]) -> Vec<Rect> {
        let o = self.l.output;
        (rects.iter())
            .map(|r| Rect::new(r.x - o.x, r.y - o.y, r.width, r.height).scaled(self.s))
            .map(|r| r.intersect(&self.bounds()))
            .filter(|r| !r.is_empty())
            .collect()
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

/// Whether `plan` fades the screen before into the screen after over one window, nothing flying:
/// each frame then mixes the two screens alone, the chrome under both left out.
fn crossfade(plan: &Plan) -> bool {
    match (plan.out, plan.into.as_slice()) {
        (Some(out), [one]) => {
            one.window == out && one.columns.is_none() && one.rise == 0 && plan.carried.is_none()
        }
        _ => false,
    }
}

/// The frame `plan` carries, flying from where it is on `was_m`'s screen to its place on `d`'s.
fn flight(plan: &Plan, d: &Drawn, (was_m, was_l): (&Logon, &LogonLayout)) -> Option<Flight> {
    let (part, ms) = plan.carried?;
    let from = glass::pose(was_m, was_l, d.theme, d.s, part)?;
    let to = glass::pose(d.m, d.l, d.theme, d.s, part)?;
    Some(Flight { part, from, to, ms })
}

/// What the screen shows while `flight` flies: all of it but the frame flying.
fn shown_under(flight: Option<&Flight>) -> Show {
    flight.map_or(Show::All, |f| Show::AllBut(f.part))
}

/// `r` of `c` (device px) set to `from`'s pixels there, `from`'s top-left lying at `at` in `c`.
fn paste(c: &mut Canvas, from: &Canvas, (ax, ay): (i32, i32), r: Rect) {
    debug_assert_eq!((c.scale(), from.scale()), (1, 1));
    let (w, fw, n) = (
        c.width as usize,
        from.width as usize,
        r.width.max(0) as usize,
    );
    for y in r.y..r.bottom() {
        let i = y as usize * w + r.x as usize;
        let j = (y - ay) as usize * fw + (r.x - ax) as usize;
        c.pixels[i..i + n].copy_from_slice(&from.pixels[j..j + n]);
    }
}

/// The smallest rect holding every pixel where `a` and `b`, the same size, differ inside `r`.
fn differing(a: &Canvas, b: &Canvas, r: Rect) -> Rect {
    let r = r.intersect(&a.bounds());
    let (w, n) = (a.width as usize, r.width.max(0) as usize);
    let (mut x0, mut x1, mut y0, mut y1) = (r.right(), r.x - 1, r.bottom(), r.y - 1);
    for y in r.y..r.bottom() {
        let i = y as usize * w + r.x as usize;
        let (p, q) = (&a.pixels[i..i + n], &b.pixels[i..i + n]);
        if p == q {
            continue;
        }
        let pairs = || p.iter().zip(q);
        // Only columns outside those found so far can widen the rect.
        let first = pairs().take((x0 - r.x) as usize).position(|(p, q)| p != q);
        let skip = (x1 - r.x + 1) as usize;
        let last = pairs().skip(skip).rposition(|(p, q)| p != q);
        x0 = first.map_or(x0, |k| r.x + k as i32);
        x1 = last.map_or(x1, |k| r.x + (skip + k) as i32);
        (y0, y1) = (y0.min(y), y);
    }
    if y1 < y0 {
        return Rect::default();
    }
    Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1)
}

/// The smallest rect holding all of `rects`.
fn hull(rects: &[Rect]) -> Rect {
    rects.iter().fold(Rect::default(), |a, b| a.join(b))
}

/// `r` reaching `rise` further down, cut to `bounds`.
fn risen(r: Rect, rise: i32, bounds: Rect) -> Rect {
    Rect {
        height: r.height + rise,
        ..r
    }
    .intersect(&bounds)
}

/// What a transition is built from: the canvases of `area`, the rect a frame must at least cover,
/// and the flight.
struct Parts {
    area: Rect,
    base: Option<Canvas>,
    was: Option<Canvas>,
    now: Canvas,
    also: Rect,
    flight: Option<Flight>,
}

impl Transition {
    /// `motion` on the screen `d` draws; None when it shows at once or changes nothing.
    pub fn begin(motion: &Motion, d: &Drawn) -> Option<Transition> {
        Transition::begin_over(motion, d, None)
    }

    /// [`Transition::begin`] with `shown`, the canvas the screen before showed, standing in for
    /// that screen where no frame flies off it.
    pub fn begin_over(motion: &Motion, d: &Drawn, shown: Option<Canvas>) -> Option<Transition> {
        let plan = plan(motion.change, d)?;
        let (was_m, was_l) = d.before(motion);
        let flight = flight(&plan, d, (&was_m, &was_l));
        let shown = shown.filter(|c| flight.is_none() && (c.width, c.height) == d.size());
        let show = shown_under(flight.as_ref());
        let was = plan
            .out
            .map(|_| shown.unwrap_or_else(|| d.canvas(&was_m, &was_l, show)));
        let base = (!crossfade(&plan)).then(|| d.canvas(d.m, d.l, Show::Chrome));
        let now = d.canvas(d.m, d.l, show);
        let parts = Parts {
            area: d.bounds(),
            base,
            was,
            now,
            also: Rect::default(),
            flight,
        };
        Transition::build(motion.change, plan, d.s, parts)
    }

    /// [`Transition::begin_over`] on `c`, the canvas the screen before shows, when the screens
    /// before and after differ from it only inside `rects` ([`super::logon_region`], design px
    /// within the output): each is drawn into `c` there alone, and `c` is left showing the
    /// first frame, or the screen after when nothing plays. Returns the device rects of `c` that
    /// changed.
    pub fn begin_within(
        motion: &Motion,
        d: &Drawn,
        rects: &[Rect],
        c: &mut Canvas,
    ) -> (Option<Transition>, Vec<Rect>) {
        let dev = d.device(rects);
        let Some(plan) = plan(motion.change, d) else {
            d.draw_within(c, d.after(), Show::All, &dev);
            return (None, dev);
        };
        debug_assert_eq!((c.width, c.height), d.size());
        let flight = plan.carried.and_then(|_| {
            let was = d.before(motion);
            let flight = flight(&plan, d, (&was.0, &was.1))?;
            // The screen before, its frame lifted off to fly, is drawn from its model.
            d.draw_within(c, (&was.0, &was.1), shown_under(Some(&flight)), &dev);
            Some(flight)
        });
        let t = Transition::build_within(motion.change, plan, d, &dev, c, Rect::default(), flight);
        Transition::first(t, c, d.m)
    }

    /// `motion` begun `at` ms into `prev`: the screen before is `prev`'s frame then, composed over
    /// `shown`, the canvas `prev`'s frames draw into, and a frame flying in both flies on from
    /// where it is.
    pub fn then(
        prev: &Transition,
        at: u32,
        motion: &Motion,
        d: &Drawn,
        shown: Canvas,
    ) -> Option<Transition> {
        let (plan, flight) = Transition::carried(prev, at, motion, d)?;
        let mut was = shown;
        prev.compose(&mut was, d.m, at, flight.is_none());
        let was = plan.out.map(|_| was);
        let show = shown_under(flight.as_ref());
        let base = (!crossfade(&plan)).then(|| d.canvas(d.m, d.l, Show::Chrome));
        let now = d.canvas(d.m, d.l, show);
        let parts = Parts {
            area: d.bounds(),
            base,
            was,
            now,
            also: prev.rect,
            flight,
        };
        Transition::build(motion.change, plan, d.s, parts)
    }

    /// [`Transition::then`] on `c`, the canvas `prev`'s frames draw into, when the screen after
    /// differs from `prev`'s only inside `rects` ([`super::logon_region`]): both are drawn into
    /// `c` only there and where `prev` holds the screens. `c` is left showing the first frame, or
    /// the screen after when nothing plays; returns the device rects of `c` that changed.
    pub fn then_within(
        prev: &Transition,
        at: u32,
        motion: &Motion,
        d: &Drawn,
        rects: &[Rect],
        c: &mut Canvas,
    ) -> (Option<Transition>, Vec<Rect>) {
        let mut dev = d.device(rects);
        dev.push(prev.area);
        let Some((plan, flight)) = Transition::carried(prev, at, motion, d) else {
            d.draw_within(c, d.after(), Show::All, &dev);
            return (None, dev);
        };
        debug_assert_eq!((c.width, c.height), d.size());
        // A frame flying on is lifted off the screen before.
        prev.compose(c, d.m, at, flight.is_none());
        match Transition::build_within(motion.change, plan, d, &dev, c, prev.rect, flight) {
            Some(t) => Transition::first(Some(t), c, d.m),
            None => (None, dev),
        }
    }

    /// A transition over `c`, which shows the screen before (without `flight`'s frame) and differs
    /// from the screen after and the chrome only inside device rects `dev`: `c` is held there, then
    /// the chrome and the screen after are drawn into it there and held. `c` is left showing the
    /// screen after, the frame flying left out; the canvases kept cover only the rect the frames
    /// draw into.
    fn build_within(
        change: Change,
        plan: Plan,
        d: &Drawn,
        dev: &[Rect],
        c: &mut Canvas,
        also: Rect,
        flight: Option<Flight>,
    ) -> Option<Transition> {
        let held = |c: &Canvas| -> Vec<Canvas> { dev.iter().map(|&r| c.copy_rect(r)).collect() };
        let was = plan.out.map(|_| held(c));
        let base = (!crossfade(&plan)).then(|| {
            d.draw_within(c, d.after(), Show::Chrome, dev);
            held(c)
        });
        d.draw_within(c, d.after(), shown_under(flight.as_ref()), dev);
        let now = held(c);
        let under = base.as_ref().or(was.as_ref())?;
        let mut rect = (flight.iter()).fold(also, |r, f| {
            r.join(&f.from.reach(d.s)).join(&f.to.reach(d.s))
        });
        for (i, r) in dev.iter().enumerate() {
            let on = |x: Rect| match x.is_empty() {
                true => x,
                false => Rect::new(x.x + r.x, x.y + r.y, x.width, x.height),
            };
            let u = &under[i];
            rect = rect.join(&on(differing(u, &now[i], u.bounds())));
            if let Some(w) = was.as_ref().filter(|_| base.is_some()) {
                rect = rect.join(&on(differing(u, &w[i], u.bounds())));
            }
        }
        let rise = plan.into.iter().map(|s| s.rise).max().unwrap_or(0);
        let area = risen(rect, rise, c.bounds());
        // Outside `dev` the screens and the chrome are all what `c` shows.
        let gather = |held: Vec<Canvas>| {
            let mut g = c.copy_rect(area);
            for (r, h) in dev.iter().zip(&held) {
                let o = r.intersect(&area);
                if !o.is_empty() {
                    let within = Rect::new(o.x - area.x, o.y - area.y, o.width, o.height);
                    paste(&mut g, h, (r.x - area.x, r.y - area.y), within);
                }
            }
            g
        };
        let parts = Parts {
            area,
            base: base.map(gather),
            was: was.map(gather),
            now: c.copy_rect(area),
            also,
            flight,
        };
        Transition::build(change, plan, d.s, parts)
    }

    /// `t`'s first frame drawn into `c`, and the rect drawn.
    fn first(t: Option<Transition>, c: &mut Canvas, m: &Logon) -> (Option<Transition>, Vec<Rect>) {
        let r = t.as_ref().map(|t| t.frame(c, m, 0));
        (t, r.into_iter().collect())
    }

    /// How `motion` plays begun `at` ms into `prev`, with the frame flying in both carried on.
    fn carried(
        prev: &Transition,
        at: u32,
        motion: &Motion,
        d: &Drawn,
    ) -> Option<(Plan, Option<Flight>)> {
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
        Some((plan, flight))
    }

    fn build(change: Change, plan: Plan, s: i32, p: Parts) -> Option<Transition> {
        let (under, was) = match p.base {
            Some(base) => (base, p.was),
            None => (p.was?, None),
        };
        let a = p.area;
        let differ = |b: &Canvas| {
            let r = differing(&under, b, under.bounds());
            if r.is_empty() {
                r
            } else {
                Rect::new(r.x + a.x, r.y + a.y, r.width, r.height)
            }
        };
        let mut rect = p.also.join(&differ(&p.now));
        if let Some(was) = &was {
            rect = rect.join(&differ(was));
        }
        if let Some(f) = &p.flight {
            rect = rect.join(&f.from.reach(s)).join(&f.to.reach(s));
        }
        let rise = plan.into.iter().map(|s| s.rise).max().unwrap_or(0);
        let rect = risen(rect, rise, a);
        if rect.is_empty() {
            return None;
        }
        Some(Transition {
            change,
            plan,
            s,
            area: a,
            under,
            was,
            now: p.now,
            rect,
            rise,
            flight: p.flight,
        })
    }

    /// The canvases cover all of `c`, whose pixels outside [`Transition::rect`] are those of the
    /// screen after.
    fn widen(&mut self, c: &Canvas, to: Rect) {
        let to = self.area.join(&to).intersect(&c.bounds());
        if to == self.area {
            return;
        }
        let a = self.area;
        let at = (a.x - to.x, a.y - to.y);
        let grow = |held: &Canvas| {
            let mut g = c.copy_rect(to);
            paste(&mut g, held, at, Rect::new(at.0, at.1, a.width, a.height));
            g
        };
        self.under = grow(&self.under);
        self.was = self.was.as_ref().map(grow);
        self.now = grow(&self.now);
        self.area = to;
    }

    /// Draw the screen after again, as it changed while this plays (a key typed, a part lit);
    /// `shown` is the canvas the frames draw into.
    pub fn refresh_now(&mut self, d: &Drawn, shown: &Canvas) {
        self.widen(shown, shown.bounds());
        self.now = d.canvas(d.m, d.l, shown_under(self.flight.as_ref()));
        if let Some(f) = &mut self.flight
            && let Some(to) = glass::pose(d.m, d.l, d.theme, d.s, f.part)
        {
            f.to = to;
        }
        let changed = differing(&self.under, &self.now, self.under.bounds());
        self.grow_rect(changed);
    }

    /// [`Transition::refresh_now`] when the screen after changed only inside `rects` (design px
    /// within the output): it is drawn there alone into `c`, the canvas the frames draw into.
    /// Returns the device rect of `c` drawn; the frame for now still has to be drawn over it.
    pub fn refresh_within(&mut self, c: &mut Canvas, d: &Drawn, rects: &[Rect]) -> Rect {
        let dev = d.device(rects);
        let within = hull(&dev);
        if within.is_empty() {
            return within;
        }
        self.widen(c, risen(within, self.rise, c.bounds()));
        d.draw_within(c, d.after(), shown_under(self.flight.as_ref()), &dev);
        let a = self.area;
        for &r in &dev {
            let held = Rect::new(r.x - a.x, r.y - a.y, r.width, r.height);
            paste(&mut self.now, c, (-a.x, -a.y), held);
        }
        if let Some(f) = &mut self.flight
            && let Some(to) = glass::pose(d.m, d.l, d.theme, d.s, f.part)
        {
            f.to = to;
        }
        let held = Rect::new(within.x - a.x, within.y - a.y, within.width, within.height);
        let changed = differing(&self.under, &self.now, held);
        self.grow_rect(changed);
        within
    }

    /// The rect takes in `changed`, a rect of the held canvases, and the rise under it.
    fn grow_rect(&mut self, changed: Rect) {
        if changed.is_empty() {
            return;
        }
        let a = self.area;
        let changed = Rect::new(
            changed.x + a.x,
            changed.y + a.y,
            changed.width,
            changed.height,
        );
        self.rect = self.rect.join(&risen(changed, self.rise, a));
    }

    /// The device rect of the screen's canvas the transition holds its screens over.
    #[cfg(test)]
    pub(super) fn held(&self) -> Rect {
        self.area
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

    /// The screen after at rest, the canvas a host shows under the frames; only for a transition
    /// holding the whole screen (built whole, or after [`Transition::refresh_now`]).
    pub fn rest(&self, m: &Logon) -> Canvas {
        debug_assert_eq!(self.area, self.now.bounds(), "built over the whole screen");
        let mut c = self.now.clone();
        if let Some(f) = &self.flight {
            glass::draw_posed(&mut c, m, f.part, &f.to, self.s);
        }
        c
    }

    /// The screen after at rest, drawn into [`Transition::rect`] of `c` (the canvas the frames
    /// drew into), its changes since `refresh_now` included. Returns the rect drawn.
    pub fn settle(&self, c: &mut Canvas, m: &Logon) -> Rect {
        paste(c, &self.now, (self.area.x, self.area.y), self.rect);
        if let Some(f) = &self.flight {
            glass::draw_posed(c, m, f.part, &f.to, self.s);
        }
        self.rect
    }

    fn compose(&self, c: &mut Canvas, m: &Logon, at: u32, carried: bool) {
        let (r, a) = (self.rect, (self.area.x, self.area.y));
        paste(c, &self.under, a, r);
        let level = |w: crate::logon::Window| Ease::InOut.apply(w.at(at));
        let out = (self.was.as_ref().zip(self.plan.out)).map(|(w, win)| (w, 1.0 - level(win)));
        match self.plan.into.as_slice() {
            [one] if one.columns.is_none() && one.rise == 0 => {
                let all: Vec<(&Canvas, f32)> = out
                    .into_iter()
                    .chain([(&self.now, level(one.window))])
                    .collect();
                c.mix_all_at(&all, r, a);
            }
            slices => {
                if let Some((w, l)) = out {
                    c.mix_all_at(&[(w, l)], r, a);
                }
                for sl in slices {
                    let (x0, x1) = sl.columns.unwrap_or((r.x, r.right()));
                    let (x0, x1) = (x0.max(r.x), x1.min(r.right()));
                    let src = Rect::new(x0, r.y, x1 - x0, r.height - sl.rise);
                    let up = 1.0 - Ease::OutCubic.apply(sl.window.at(at));
                    let dy = (sl.rise as f32 * up).round() as i32;
                    c.mix_shifted_at(&self.now, src, (0, dy), level(sl.window), a);
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
