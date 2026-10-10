mod caustic;
mod contour;
mod flow;
mod fold;
mod land;
pub mod noise;
mod panes;
mod reeded;
mod ridges;
mod tarn;
mod terraces;
pub mod tone;

use crate::raster::Canvas;
use std::cell::Cell;
use std::fmt;
use std::sync::Mutex;

pub const SEED: u32 = 7;

/// Bump whenever any design draws different pixels for the same inputs, so
/// pictures cached under the old number are drawn again.
pub const VERSION: u32 = 1;

pub const SCHEME: &str = "reprise:";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Design {
    Caustic,
    Reeded,
    Panes,
    Ridges,
    Tarn,
    Terraces,
    Fold,
    Contour,
    Flow,
}

impl Design {
    pub const ALL: [Design; 9] = [
        Design::Tarn,
        Design::Ridges,
        Design::Terraces,
        Design::Caustic,
        Design::Reeded,
        Design::Panes,
        Design::Flow,
        Design::Fold,
        Design::Contour,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Design::Caustic => "caustic",
            Design::Reeded => "reeded",
            Design::Panes => "panes",
            Design::Ridges => "ridges",
            Design::Tarn => "tarn",
            Design::Terraces => "terraces",
            Design::Fold => "fold",
            Design::Contour => "contour",
            Design::Flow => "flow",
        }
    }

    pub fn from_name(name: &str) -> Option<Design> {
        Design::ALL.into_iter().find(|d| d.name() == name)
    }

    /// The first is the design's default.
    pub fn variants(self) -> &'static [&'static str] {
        match self {
            Design::Caustic | Design::Reeded | Design::Panes => {
                &["sky", "ruby", "green", "violet", "dark"]
            }
            Design::Ridges | Design::Tarn => &["dawn", "day", "dusk", "night", "blue"],
            Design::Terraces => &["spring", "harvest", "dusk", "frost", "blue"],
            Design::Fold => &["teal", "steel", "sand", "night"],
            Design::Contour => &["teal", "night", "paper", "dusk"],
            Design::Flow => &["night", "ink", "teal", "ember"],
        }
    }

    pub fn hue_driven(self) -> bool {
        matches!(
            self,
            Design::Caustic | Design::Reeded | Design::Panes | Design::Flow
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Variant {
    /// One of [`Design::variants`].
    Named(&'static str),
    /// OKLCH hue in degrees (0..360); hue-driven designs only.
    Hue { hue: u16, dark: bool },
    /// The `[glass]` colour's hue; the host resolves it ([`Builtin::resolve`])
    /// before drawing, and drawn unresolved it is the first variant.
    Glass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Builtin {
    pub design: Design,
    pub variant: Variant,
}

impl Builtin {
    pub fn named(design: Design, variant: &str) -> Option<Builtin> {
        let v = design.variants().iter().find(|v| **v == variant)?;
        Some(Builtin {
            design,
            variant: Variant::Named(v),
        })
    }

    pub fn parse(s: &str) -> Result<Builtin, String> {
        let (d, v) = s.split_once('/').unwrap_or((s, ""));
        let design = Design::from_name(d).ok_or_else(|| {
            let all: Vec<&str> = Design::ALL.iter().map(|d| d.name()).collect();
            format!("no built-in wallpaper {d:?} (one of {})", all.join(", "))
        })?;
        if v.is_empty() {
            return Ok(Builtin {
                design,
                variant: Variant::Named(design.variants()[0]),
            });
        }
        if v == "glass" && design.hue_driven() {
            return Ok(Builtin {
                design,
                variant: Variant::Glass,
            });
        }
        if let Some(rest) = v.strip_prefix("hue-")
            && design.hue_driven()
        {
            let (deg, dark) = match rest.strip_suffix("-dark") {
                Some(deg) => (deg, true),
                None => (rest, false),
            };
            if let Ok(hue) = deg.parse::<u16>()
                && hue < 360
            {
                return Ok(Builtin {
                    design,
                    variant: Variant::Hue { hue, dark },
                });
            }
        }
        Builtin::named(design, v).ok_or_else(|| {
            let hue = if design.hue_driven() {
                ", glass, hue-<0..359> or hue-<0..359>-dark"
            } else {
                ""
            };
            format!(
                "{} has no variant {v:?} (one of {}{hue})",
                design.name(),
                design.variants().join(", ")
            )
        })
    }

    pub fn from_uri(s: &str) -> Option<Result<Builtin, String>> {
        s.strip_prefix(SCHEME).map(Builtin::parse)
    }

    pub fn resolve(self, hue: u16, dark: bool) -> Builtin {
        let variant = match (self.variant, self.design, dark) {
            (Variant::Glass, Design::Flow, true) => Variant::Named("night"),
            (Variant::Glass, _, _) => Variant::Hue {
                hue: hue % 360,
                dark,
            },
            (v, _, _) => v,
        };
        Builtin { variant, ..self }
    }

    pub fn uri(&self) -> String {
        format!("{SCHEME}{self}")
    }
}

impl fmt::Display for Builtin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.variant {
            Variant::Named(v) => write!(f, "{}/{v}", self.design.name()),
            Variant::Glass => write!(f, "{}/glass", self.design.name()),
            Variant::Hue { hue, dark } => {
                let dark = if dark { "-dark" } else { "" };
                write!(f, "{}/hue-{hue}{dark}", self.design.name())
            }
        }
    }
}

pub fn render(b: Builtin, w: u32, h: u32, seed: u32) -> Canvas {
    let (wu, hu) = (w.max(1) as usize, h.max(1) as usize);
    let px = match b.design {
        Design::Caustic => caustic::render(wu, hu, b.variant, seed),
        Design::Reeded => reeded::render(wu, hu, b.variant, seed),
        Design::Panes => panes::render(wu, hu, b.variant, seed),
        Design::Ridges => ridges::render(wu, hu, b.variant, seed),
        Design::Tarn => tarn::render(wu, hu, b.variant, seed),
        Design::Terraces => terraces::render(wu, hu, b.variant, seed),
        Design::Fold => fold::render(wu, hu, b.variant, seed),
        Design::Contour => contour::render(wu, hu, b.variant, seed),
        Design::Flow => flow::render(wu, hu, b.variant, seed),
    };
    Canvas::from_pixels(wu as u32, hu as u32, px)
}

pub fn cache_name(b: Builtin, w: u32, h: u32, seed: u32) -> String {
    let v = match b.variant {
        Variant::Named(v) => v.to_string(),
        Variant::Hue { hue, dark } => format!("hue{hue}{}", if dark { "d" } else { "" }),
        Variant::Glass => "glass".into(),
    };
    format!("{}-{v}-{w}x{h}-s{seed}-v{VERSION}", b.design.name())
}

fn hue_of(v: Variant, named: &[(&str, u16, bool)]) -> (f32, bool) {
    match v {
        Variant::Hue { hue, dark } => (hue as f32, dark),
        Variant::Glass => (named[0].1 as f32, named[0].2),
        Variant::Named(n) => named
            .iter()
            .find(|e| e.0 == n)
            .map(|e| (e.1 as f32, e.2))
            .unwrap_or((named[0].1 as f32, named[0].2)),
    }
}

const HUES: [(&str, u16, bool); 5] = [
    ("sky", 245, false),
    ("ruby", 12, false),
    ("green", 160, false),
    ("violet", 300, false),
    ("dark", 245, true),
];

thread_local! {
    /// 0 picks one per core.
    static THREADS: Cell<usize> = const { Cell::new(0) };
}

pub fn with_threads<T>(n: usize, f: impl FnOnce() -> T) -> T {
    let was = THREADS.with(|t| t.replace(n));
    let out = f();
    THREADS.with(|t| t.set(was));
    out
}

/// Fills `out`, rows of `row_len`, by calling `f(row, row_slice)` for each
/// row, bands of rows spread over the cores. `f` must depend only on the
/// row it is given for the result to be the same at any thread count.
/// Every pixel must depend only on its position (or an earlier pass's
/// finished picture), so the result is the same at any thread count.
pub fn par_rows<T: Send>(out: &mut [T], row_len: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    par_bands(out, row_len, |first, band| {
        for (k, row) in band.chunks_mut(row_len).enumerate() {
            f(first + k, row);
        }
    });
}

/// Like [`par_rows`], a band of whole rows at a time: `f(first_row,
/// band)`. What `f` writes to a row must not depend on how the rows were
/// banded.
pub fn par_bands<T: Send>(out: &mut [T], row_len: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    if row_len == 0 || out.is_empty() {
        return;
    }
    let rows = out.len() / row_len;
    let threads = match THREADS.with(|t| t.get()) {
        0 => std::thread::available_parallelism().map_or(1, |n| n.get()),
        n => n,
    }
    .min(rows)
    .max(1);
    if threads == 1 {
        f(0, out);
        return;
    }
    // Small bands handed out on demand: a landscape's sky rows cost far less
    // than its land.
    let band = rows.div_ceil(threads * 8).max(1);
    let queue = Mutex::new(out.chunks_mut(band * row_len).enumerate());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let next = queue.lock().unwrap().next();
                    let Some((b, chunk)) = next else { break };
                    f(b * band, chunk);
                }
            });
        }
    });
}

#[inline]
/// In picture heights from the centre.
fn centred(x: usize, y: usize, w: usize, h: usize) -> (f32, f32) {
    let s = h as f32;
    (
        (x as f32 + 0.5 - w as f32 * 0.5) / s,
        (y as f32 + 0.5 - h as f32 * 0.5) / s,
    )
}

fn each(w: usize, h: usize, f: impl Fn(usize, usize, f32, f32) -> tone::C + Sync) -> Vec<tone::C> {
    let inv = 1.0 / h as f32;
    let mut px = vec![[0.0; 3]; w * h];
    par_rows(&mut px, w, |j, row| {
        let y = (j as f32 + 0.5 - h as f32 * 0.5) * inv;
        for (i, o) in row.iter_mut().enumerate() {
            let x = (i as f32 + 0.5 - w as f32 * 0.5) * inv;
            *o = f(i, j, x, y);
        }
    });
    px
}

#[cfg(test)]
mod tests;
