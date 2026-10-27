//! A Droplet opening: layers painted ahead, here or on a worker, and blended per frame.

use crate::backdrop::corner_radius;
use crate::controls::tooltip::{NotifyHit, NotifyParts};
use crate::raster::{Canvas, Rect};
use crate::shape::{mix, over, pack, rounded, unpack};
use crate::style::NotifyLook;
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, Preedit, TextEdit};
use crate::theme::{Color, Theme};

use super::draw::{SheetIcons, draw_parts, relight};
use super::glass::{GlassKey, SheetGlass};
use super::layout::open_layout;
use super::{SheetLayout, notify_look};

/// A sheet `t` (0..1) of the way from `collapsed` to `expanded`, drawn once; see
/// [`SheetOpening::draw`].
#[expect(clippy::too_many_arguments)]
pub fn draw_sheet_opening(
    theme: &Theme,
    text: &TextRenderer,
    collapsed: &SheetLayout,
    expanded: &SheetLayout,
    t: f64,
    icons: SheetIcons,
    tint: Option<Color>,
    pushed: Option<NotifyHit>,
    hot: Option<NotifyHit>,
    reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
) -> Canvas {
    SheetOpening::default().draw(
        &mut SheetGlass::default(),
        theme,
        text,
        collapsed,
        expanded,
        t,
        icons,
        tint,
        pushed,
        hot,
        reply,
    )
}

/// A Droplet's parts for opening, built when its content changes; each end's glass comes from
/// its [`SheetGlass`]. A host keeps one per Droplet and warms it before the pointer arrives.
#[derive(Default)]
pub struct SheetOpening {
    pub(super) layers: Option<Layers>,
    /// Painted here, not by [`build_warm`].
    pub(super) builds: u32,
    /// The key and rim of the [`WarmJob`] out for these layers.
    asked: Option<(OpeningKey, bool)>,
    /// Content whose job panicked on its worker; only [`SheetOpening::warm`] paints it.
    failed: Option<OpeningKey>,
}

/// What [`build_warm`] needs to warm a [`SheetOpening`], owned, so it can go to a worker thread.
pub struct WarmJob {
    theme: Theme,
    key: OpeningKey,
    lit: bool,
}

/// What [`build_warm`] painted, for [`SheetOpening::install`].
pub struct WarmLayers {
    layers: Layers,
    glass: Vec<(GlassKey, Canvas)>,
    lit: bool,
}

/// Paints `job`'s layers with `text`, a renderer loaded from `job`'s fonts ([`WarmJob::fonts`]).
pub fn build_warm(job: WarmJob, text: &TextRenderer) -> WarmLayers {
    let WarmJob { theme, key, lit } = job;
    let look = notify_look(&theme).expect("a job is only made under a look with sheets");
    let (app, picture, actions) = (key.app.clone(), key.picture.clone(), key.actions.clone());
    let icons = SheetIcons {
        app: app.as_ref(),
        picture: picture.as_ref(),
        actions: &actions,
    };
    let layers = Layers::parts(
        key,
        &theme,
        text.for_output(theme.scale),
        &look,
        icons,
        None,
    );
    let glass = layers
        .key
        .ends(lit)
        .map(|k| (k, (look.sheet)(&look, &k.spec, k.scale)))
        .into();
    WarmLayers { layers, glass, lit }
}

impl WarmJob {
    pub fn fonts(&self) -> &crate::theme::FontConfig {
        &self.theme.fonts
    }
}

/// Leaves out the app line, which ages: a frame draws it on top.
#[derive(Clone, PartialEq)]
pub(super) struct OpeningKey {
    collapsed: SheetLayout,
    expanded: SheetLayout,
    scale: i32,
    dark: bool,
    app: Option<Canvas>,
    picture: Option<Canvas>,
    actions: Vec<Option<Canvas>>,
    tint: Option<Color>,
    reply: Option<(TextEdit, Caret, Option<Preedit>)>,
}

impl OpeningKey {
    fn new(
        theme: &Theme,
        look: &NotifyLook,
        collapsed: &SheetLayout,
        expanded: &SheetLayout,
        icons: SheetIcons,
        tint: Option<Color>,
        reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
    ) -> OpeningKey {
        let bare = |lay: &SheetLayout| SheetLayout {
            app_line: String::new(),
            ..lay.clone()
        };
        OpeningKey {
            collapsed: bare(collapsed),
            expanded: bare(expanded),
            scale: theme.scale.max(1),
            dark: look.dark,
            app: icons.app.cloned(),
            picture: icons.picture.cloned(),
            actions: icons.actions.to_vec(),
            tint,
            reply: reply.map(|(e, c, p)| (e.clone(), c, p.cloned())),
        }
    }

    /// The collapsed and expanded glass under the rim `lit`.
    fn ends(&self, lit: bool) -> [GlassKey; 2] {
        [&self.collapsed, &self.expanded]
            .map(|lay| GlassKey::new(lay, self.tint, lit, self.scale, self.dark))
    }
}

/// Which of the expanded pill row fits the sheet part way open.
type Kept = (usize, bool, bool, bool);

pub(super) fn kept(p: &NotifyParts) -> Kept {
    (
        p.buttons.len(),
        p.more.is_some(),
        p.send.is_some(),
        p.field.is_some(),
    )
}

pub(super) struct Layers {
    pub(super) key: OpeningKey,
    /// The collapsed parts on clear, and the expanded ones, unlit, for each `Kept` an opening
    /// passes through.
    shut_parts: Canvas,
    pub(super) open_parts: Vec<(Kept, Canvas)>,
}

impl SheetOpening {
    /// Builds what a frame needs under the rim `lit` (the pointer on the sheet) unless it is
    /// built already; true when it painted.
    #[expect(clippy::too_many_arguments)]
    pub fn warm(
        &mut self,
        glass: &mut SheetGlass,
        theme: &Theme,
        text: &TextRenderer,
        collapsed: &SheetLayout,
        expanded: &SheetLayout,
        icons: SheetIcons,
        tint: Option<Color>,
        reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
        lit: bool,
    ) -> bool {
        let Some(look) = notify_look(theme) else {
            return false;
        };
        let key = OpeningKey::new(theme, &look, collapsed, expanded, icons, tint, reply);
        let glass_builds = glass.builds;
        let mut built = false;
        if self.layers.as_ref().is_none_or(|l| l.key != key) {
            if self.asked.as_ref().is_some_and(|(k, _)| *k != key) {
                self.asked = None;
            }
            self.layers = Some(Layers::parts(key, theme, text, &look, icons, reply));
            built = true;
        }
        let Some(layers) = self.layers.as_ref() else {
            unreachable!("built above");
        };
        for k in layers.key.ends(lit) {
            glass.ensure(&look, k);
        }
        built |= glass.builds > glass_builds;
        self.builds += u32::from(built);
        built
    }

    /// What [`SheetOpening::warm`] would paint, as a job for [`build_warm`] elsewhere; `None`
    /// when it is painted already or out as a job.
    #[expect(clippy::too_many_arguments)]
    pub fn warm_job(
        &mut self,
        glass: &SheetGlass,
        theme: &Theme,
        collapsed: &SheetLayout,
        expanded: &SheetLayout,
        icons: SheetIcons,
        tint: Option<Color>,
        lit: bool,
    ) -> Option<WarmJob> {
        let look = notify_look(theme)?;
        let key = OpeningKey::new(theme, &look, collapsed, expanded, icons, tint, None);
        let ready = self.layers.as_ref().is_some_and(|l| l.key == key)
            && key.ends(lit).iter().all(|k| glass.find(k).is_some());
        if ready
            || self.failed.as_ref() == Some(&key)
            || self
                .asked
                .as_ref()
                .is_some_and(|(k, r)| *k == key && *r == lit)
        {
            return None;
        }
        self.asked = Some((key.clone(), lit));
        Some(WarmJob {
            theme: theme.clone(),
            key,
            lit,
        })
    }

    /// Takes what [`build_warm`] painted, unless the layers were since painted for other content;
    /// the other rim's glass goes, but for the glass at rest.
    pub fn install(&mut self, glass: &mut SheetGlass, built: WarmLayers) {
        let WarmLayers {
            layers,
            glass: ends,
            lit,
        } = built;
        let asked = self.asked.as_ref().is_some_and(|(k, _)| *k == layers.key);
        if asked {
            self.asked = None;
        }
        let same = self.layers.as_ref().is_some_and(|l| l.key == layers.key);
        if !same && !asked {
            return;
        }
        let keys = layers.key.ends(lit);
        if !same {
            self.layers = Some(layers);
        }
        for (k, c) in ends {
            if glass.find(&k).is_none() {
                glass.kept.push((k, c));
            }
        }
        glass.keep_ends(keys, lit);
    }

    /// The job out was lost with its worker; the next [`SheetOpening::warm_job`] asks again.
    pub fn forget_job(&mut self) {
        self.asked = None;
    }

    /// The job out panicked; [`SheetOpening::warm_job`] asks nothing more for its content.
    pub fn job_failed(&mut self) {
        self.failed = self.asked.take().map(|(k, _)| k);
    }

    /// How many times these layers were painted on the caller's thread.
    pub fn builds(&self) -> u32 {
        self.builds
    }

    /// The sheet `t` (0..1) of the way from `collapsed` to `expanded` on `glass`: either end as
    /// drawn at rest, and between them the blend of [`blend_opening`] with the app line on top.
    #[expect(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        glass: &mut SheetGlass,
        theme: &Theme,
        text: &TextRenderer,
        collapsed: &SheetLayout,
        expanded: &SheetLayout,
        t: f64,
        icons: SheetIcons,
        tint: Option<Color>,
        pushed: Option<NotifyHit>,
        hot: Option<NotifyHit>,
        reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
    ) -> Canvas {
        let open = open_layout(collapsed, expanded, t);
        let Some(look) = notify_look(theme) else {
            return Canvas::new(open.width.max(1) as u32, open.height.max(1) as u32);
        };
        let s = theme.scale.max(1);
        let t = t.clamp(0.0, 1.0);
        let lit = hot.is_some();
        let ends = [collapsed, expanded].map(|lay| GlassKey::new(lay, tint, lit, s, look.dark));
        if t <= 0.0 || t >= 1.0 {
            // At rest the next move needs the lit rim while collapsed, the plain one while open.
            let shut = t <= 0.0;
            let (lay, rest) = if shut {
                (collapsed, ends[0])
            } else {
                (expanded, ends[1])
            };
            let mut c = glass.get(&look, rest);
            glass.rest = Some(rest);
            glass.keep_ends(ends, shut);
            let (hot, reply) = if shut { (None, None) } else { (hot, reply) };
            draw_parts(&mut c, theme, text, &look, lay, icons, pushed, hot, reply);
            return c;
        }
        self.warm(
            glass, theme, text, collapsed, expanded, icons, tint, reply, lit,
        );
        glass.rest = None;
        glass.keep_ends(ends, lit);
        let Some(layers) = self.layers.as_mut() else {
            unreachable!("warmed above");
        };
        let under = glass_at(glass, &layers.key, lit, open.sheet, s);
        let want = kept(&open.parts);
        let i = match layers.open_parts.iter().position(|(k, _)| *k == want) {
            Some(i) => i,
            None => {
                let mut c =
                    Canvas::new(expanded.width.max(1) as u32, expanded.height.max(1) as u32);
                let bare = SheetLayout {
                    app_line: String::new(),
                    ..open.clone()
                };
                draw_parts(&mut c, theme, text, &look, &bare, icons, None, None, reply);
                layers.open_parts.push((want, c));
                layers.open_parts.len() - 1
            }
        };
        let mut open_parts = layers.open_parts[i].1.clone();
        let edit_empty = reply.is_none_or(|(e, ..)| e.text.trim().is_empty());
        relight(
            &mut open_parts,
            text,
            &look,
            &open,
            icons.actions,
            edit_empty,
            pushed,
            hot,
            s,
        );
        let r = f64::from(corner_radius((look.corner)(open.shape))) * f64::from(s);
        let mut c = blend_opening(
            under,
            &layers.shut_parts,
            &open_parts,
            open.sheet,
            r,
            opening_fades(t),
        );
        text.draw(
            &mut c,
            open.app_box,
            &open.app_line,
            false,
            look.inks.app,
            Align::Left,
            open.app_px,
        );
        c
    }

    #[cfg(test)]
    pub(super) fn bytes(&self) -> usize {
        self.layers.as_ref().map_or(0, |l| {
            let parts = std::iter::once(&l.shut_parts).chain(l.open_parts.iter().map(|(_, c)| c));
            parts.map(|c| c.pixels.len() * 4).sum()
        })
    }
}

impl Layers {
    fn parts(
        key: OpeningKey,
        theme: &Theme,
        text: &TextRenderer,
        look: &NotifyLook,
        icons: SheetIcons,
        reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
    ) -> Layers {
        let (c, e) = (&key.collapsed, &key.expanded);
        let clear =
            |lay: &SheetLayout| Canvas::new(lay.width.max(1) as u32, lay.height.max(1) as u32);
        let mut shut_parts = clear(c);
        draw_parts(
            &mut shut_parts,
            theme,
            text,
            look,
            c,
            icons,
            None,
            None,
            None,
        );
        let mut open_parts: Vec<(Kept, Canvas)> = Vec::new();
        for i in 1..=100 {
            let open = open_layout(c, e, f64::from(i) / 100.0);
            let want = kept(&open.parts);
            if open_parts.iter().all(|(k, _)| *k != want) {
                let mut layer = clear(e);
                draw_parts(
                    &mut layer, theme, text, look, &open, icons, None, None, reply,
                );
                open_parts.push((want, layer));
            }
        }
        Layers {
            open_parts,
            shut_parts,
            key,
        }
    }
}

/// The glass under the rim `lit` for a sheet `sheet` tall: each end's caps kept and its middle
/// row stretched or its middle cut out, then mixed by how far the height has come.
pub(super) fn glass_at(
    glass: &SheetGlass,
    key: &OpeningKey,
    lit: bool,
    sheet: Rect,
    s: i32,
) -> Canvas {
    let [kc, ke] = key.ends(lit);
    let (Some(c), Some(e)) = (glass.find(&kc), glass.find(&ke)) else {
        unreachable!("warmed before a frame");
    };
    let body = |lay: &SheetLayout| lay.sheet.height / s * s;
    let (hc, he, h) = (
        body(&key.collapsed),
        body(&key.expanded),
        sheet.height / s * s,
    );
    let height = c.height as i32 + (h - hc);
    let u = if he > hc {
        ((h - hc) * 256 / (he - hc)).clamp(0, 256) as u32
    } else {
        256
    };
    let (cut_c, cut_e) = (sheet.y + hc / 2, sheet.y + h / 2);
    let w = e.width as usize;
    let mut out = Canvas::new(e.width, height.max(1) as u32);
    fn row(c: &Canvas, y: i32) -> &[u32] {
        let w = c.width as usize;
        &c.pixels[y as usize * w..][..w]
    }
    for (y, to) in (0..height).zip(out.pixels.chunks_exact_mut(w)) {
        let yc = if y < cut_c {
            y
        } else if y < cut_c + (h - hc) {
            cut_c
        } else {
            y - (h - hc)
        };
        let ye = if y < cut_e { y } else { y + (he - h) };
        for ((to, &a), &b) in to.iter_mut().zip(row(c, yc)).zip(row(e, ye)) {
            *to = lerp_px(a, b, u);
        }
    }
    out
}

/// `a` with `u` 256ths of `b` in its place, per premultiplied channel.
fn lerp_px(a: u32, b: u32, u: u32) -> u32 {
    if a == b {
        return a;
    }
    (0..4).fold(0, |px, i| {
        let (ca, cb) = ((a >> (8 * i)) & 0xFF, (b >> (8 * i)) & 0xFF);
        px | (((ca * (256 - u) + cb * u + 128) >> 8) << (8 * i))
    })
}

/// `glass` with the collapsed parts `gone` and the expanded parts `come` of the way in, inside
/// the sheet `sh` of corner radius `r`; parts both draw alike stay solid.
fn blend_opening(
    mut out: Canvas,
    shut: &Canvas,
    open: &Canvas,
    sh: Rect,
    r: f64,
    (gone, come): (f64, f64),
) -> Canvas {
    let (w, h) = (f64::from(sh.width), f64::from(sh.height));
    let area = sh.intersect(&Rect::new(0, 0, out.width as i32, out.height as i32));
    let layer = |c: &Canvas, x: i32, y: i32| {
        if x < c.width as i32 && y < c.height as i32 {
            c.get(x, y)
        } else {
            0
        }
    };
    let k = 1.0 - gone - come;
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let (pa, pb) = (layer(shut, x, y), layer(open, x, y));
            if pa == 0 && pb == 0 {
                continue;
            }
            let g = out.get(x, y);
            let on = |p: u32| {
                if p == 0 {
                    g
                } else {
                    pack(over(unpack(p), unpack(g)))
                }
            };
            let (a, b) = (on(pa), on(pb));
            let inside = rounded(f64::from(x - sh.x), f64::from(y - sh.y), w, h, r);
            if a == b && inside >= 1.0 {
                out.put_raw(x, y, a);
                continue;
            }
            let gp = unpack(g);
            let parts = if a == b {
                unpack(a)
            } else {
                let (a, b) = (unpack(a), unpack(b));
                let ch = |g: f64, a: f64, b: f64| g * k + a * gone + b * come;
                (
                    ch(gp.0, a.0, b.0),
                    std::array::from_fn(|i| ch(gp.1[i], a.1[i], b.1[i])),
                )
            };
            out.put_raw(x, y, pack(mix(gp, parts, inside)));
        }
    }
    out
}

/// The collapsed ink is gone by `SHUT_BY`, before the expanded ink, in from `OPEN_FROM`, is half
/// in; the two never add past one. Parts both layers draw alike stay solid throughout.
const SHUT_BY: f64 = 0.4;
const OPEN_FROM: f64 = 0.2;

/// Whether a sheet `t` (0..1) of the way open draws its close button.
pub fn opening_shows_close(t: f64) -> bool {
    opening_fades(t.clamp(0.0, 1.0)).1 > 0.0
}

/// How much of the collapsed and of the expanded parts show `t` (0..1) of the way open.
pub(super) fn opening_fades(t: f64) -> (f64, f64) {
    (
        (1.0 - t / SHUT_BY).max(0.0),
        ((t - OPEN_FROM) / (1.0 - OPEN_FROM)).clamp(0.0, 1.0),
    )
}
