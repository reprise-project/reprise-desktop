//! The Display page: the outputs the compositor lists (`repriseapp::heads`), where each is placed,
//! the one the page shows, and what is picked for it against what it runs.

use repriseapp::heads::{AdaptiveSync, Head, HeadConfig, HeadMode, HeadSet, Transform};
use reprisekit::controls::arrange::{self, Arrange, Tile};
use reprisekit::controls::combo::{COMBO_H, Combo};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::propsheet::PropSheet;
use reprisekit::raster::Rect;
use reprisekit::text::Align;
use reprisekit::trackbar::{self, TrackBar};

use crate::pages::Area;
use crate::registry::Page;

use crate::outputwrite::hz_string;

/// How many rows the Scale combo has: 100% to 400% in quarter steps, the scales `[[outputs]] scale`
/// takes.
pub const SCALE_ROWS: usize = 13;

/// A mirrored transform in force shows as a fifth row, so the combo tells the truth; picking any
/// row leaves it.
const ORIENTATIONS: [(&str, Transform); 4] = [
    ("Landscape", Transform::Normal),
    ("Portrait", Transform::Rotate90),
    ("Landscape (flipped)", Transform::Rotate180),
    ("Portrait (flipped)", Transform::Rotate270),
];

/// What the page has picked for its output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    pub mode: Option<HeadMode>,
    pub transform: Transform,
    pub scale: f64,
    pub vrr: bool,
}

impl Pick {
    fn of(h: &Head) -> Pick {
        Pick {
            mode: h.current_mode(),
            transform: h.transform,
            scale: h.scale,
            vrr: h.adaptive_sync == AdaptiveSync::On,
        }
    }
}

/// What OK or Apply asks of the outputs.
#[derive(Clone, Debug, PartialEq)]
pub struct Changes {
    /// Each output whose VRR check moved, with its new state.
    pub vrr: Vec<(String, bool)>,
    /// One configuration: each output's mode, transform and scale that moved and, when the
    /// arrangement moved, every output's place.
    pub set: HeadSet,
    /// What the outputs in `set` run now, which a No or a timeout puts back.
    pub back: HeadSet,
    /// The output picked as the primary one, when that moved.
    pub primary: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Display {
    heads: Vec<Head>,
    monitor: usize,
    /// One per head: what the page has picked for it, kept while another output is shown.
    picks: Vec<Pick>,
    places: Vec<(i32, i32)>,
    primary_saved: Option<String>,
    primary: Option<String>,
}

/// A mode's size as the layout sees it under `transform` and `scale`: turned a quarter swaps the
/// sides, and the scale divides them.
pub fn logical_size(mode: Option<HeadMode>, transform: Transform, scale: f64) -> (i32, i32) {
    let Some(m) = mode else {
        return (0, 0);
    };
    let (w, h) = match transform {
        Transform::Rotate90
        | Transform::Rotate270
        | Transform::Flipped90
        | Transform::Flipped270 => (m.height, m.width),
        _ => (m.width, m.height),
    };
    let s = if scale > 0.0 { scale } else { 1.0 };
    (
        (f64::from(w) / s).round() as i32,
        (f64::from(h) / s).round() as i32,
    )
}

impl Display {
    pub fn with_heads(heads: Vec<Head>) -> Display {
        let mut d = Display::default();
        d.set_heads(heads);
        d
    }

    /// The compositor's new list. A picked arrangement stays while the outputs keep their places and
    /// sizes; an output's unapplied picks stay while it runs what it ran.
    pub fn set_heads(&mut self, heads: Vec<Head>) {
        let was = self.head().cloned();
        let old_layout = self.layout_of_heads();
        let places = std::mem::take(&mut self.places);
        let old: Vec<(Head, Pick)> = std::mem::take(&mut self.heads)
            .into_iter()
            .zip(std::mem::take(&mut self.picks))
            .collect();
        self.heads = heads.into_iter().filter(|h| h.enabled).collect();
        self.places = if self.layout_of_heads() == old_layout && places.len() == self.heads.len() {
            places
        } else {
            self.heads.iter().map(|h| h.position).collect()
        };
        self.monitor = was
            .as_ref()
            .and_then(|w| self.heads.iter().position(|h| h.name == w.name))
            .unwrap_or(0);
        self.picks = (self.heads.iter())
            .map(|h| {
                old.iter()
                    .find(|(o, _)| o.name == h.name && Pick::of(o) == Pick::of(h))
                    .map_or_else(|| Pick::of(h), |(_, p)| *p)
            })
            .collect();
    }

    fn layout_of_heads(&self) -> Vec<Tile> {
        self.heads
            .iter()
            .enumerate()
            .map(|(i, h)| Tile {
                number: i as u32 + 1,
                name: h.name.clone(),
                pos: h.position,
                size: logical_size(h.current_mode(), h.transform, h.scale),
            })
            .collect()
    }

    /// The `[[outputs]] primary` the file has.
    pub fn set_primary_saved(&mut self, name: Option<String>) {
        self.primary = name.clone();
        self.primary_saved = name;
    }

    /// The picked primary was written.
    pub fn primary_saved(&mut self) {
        self.primary_saved = self.primary.clone();
    }

    /// The index of the primary output: the one picked while it is listed, else the first, as the
    /// compositor falls back.
    pub fn primary_index(&self) -> usize {
        self.primary
            .as_deref()
            .and_then(|p| self.heads.iter().position(|h| h.name == p))
            .unwrap_or(0)
    }

    /// Make the shown output the primary one.
    pub fn make_primary(&mut self) {
        if let Some(h) = self.head() {
            self.primary = Some(h.name.clone());
        }
    }

    /// The tiles of the arrangement well: each output at its picked place, the shown one at the
    /// size its picks give it.
    pub fn tiles(&self) -> Vec<Tile> {
        self.heads
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let size = match self.picks.get(i) {
                    Some(p) => logical_size(p.mode, p.transform, p.scale),
                    None => logical_size(h.current_mode(), h.transform, h.scale),
                };
                Tile {
                    number: i as u32 + 1,
                    name: h.name.clone(),
                    pos: self.places.get(i).copied().unwrap_or(h.position),
                    size,
                }
            })
            .collect()
    }

    /// The well moved: take its places and its picked tile.
    pub fn set_arrangement(&mut self, a: &Arrange) {
        if a.tiles.len() == self.heads.len() {
            self.places = a.tiles.iter().map(|t| t.pos).collect();
        }
        self.pick_monitor(a.selected);
    }

    /// Outputs that already overlap as they run (mirrored by hand) are left as they are.
    fn resettle(&mut self) {
        let mut tiles = self.tiles();
        if arrange::whole(&self.layout_of_heads()) && !arrange::whole(&tiles) {
            arrange::settle(&mut tiles, self.monitor);
            self.places = tiles.iter().map(|t| t.pos).collect();
        }
    }

    fn moved(&self) -> bool {
        self.heads
            .iter()
            .zip(&self.places)
            .any(|(h, p)| h.position != *p)
    }

    pub fn names(&self) -> Vec<String> {
        self.heads.iter().map(|h| h.name.clone()).collect()
    }

    pub fn monitor(&self) -> usize {
        self.monitor
    }

    pub fn head(&self) -> Option<&Head> {
        self.heads.get(self.monitor)
    }

    pub fn head_named(&self, name: &str) -> Option<&Head> {
        self.heads.iter().find(|h| h.name == name)
    }

    pub fn pick(&self) -> Option<Pick> {
        self.picks.get(self.monitor).copied()
    }

    fn pick_mut(&mut self) -> Option<&mut Pick> {
        self.picks.get_mut(self.monitor)
    }

    /// Whether the picks differ from what the outputs run and the file says.
    pub fn dirty(&self) -> bool {
        self.heads
            .iter()
            .zip(&self.picks)
            .any(|(h, p)| *p != Pick::of(h))
            || self.moved()
            || self.primary != self.primary_saved
    }

    /// Whether the output runs a mode outside its list. No mode change is offered then: a No or a
    /// timeout could not put that mode back.
    pub fn mode_locked(&self) -> bool {
        self.head().is_some_and(|h| h.current_mode().is_none())
    }

    /// The picks back to what the outputs run and the file says.
    pub fn reset(&mut self) {
        self.picks = self.heads.iter().map(Pick::of).collect();
        self.places = self.heads.iter().map(|h| h.position).collect();
        self.primary = self.primary_saved.clone();
    }

    /// Output `name`'s VRR check back to what it runs.
    pub fn reset_vrr(&mut self, name: &str) {
        if let Some((h, p)) = (self.heads.iter().zip(&mut self.picks)).find(|(h, _)| h.name == name)
        {
            p.vrr = Pick::of(h).vrr;
        }
    }

    /// What Apply asks for: every output's picks that differ from what it runs. A mode is never
    /// asked of an output that runs one outside its list ([`Display::mode_locked`]).
    pub fn changes(&self) -> Option<Changes> {
        let mut vrr = Vec::new();
        let mut set: HeadSet = Vec::new();
        for (h, p) in self.heads.iter().zip(&self.picks) {
            let now = Pick::of(h);
            if h.adaptive_sync != AdaptiveSync::Unsupported && p.vrr != now.vrr {
                vrr.push((h.name.clone(), p.vrr));
            }
            let cfg = HeadConfig {
                mode: p.mode.filter(|m| now.mode.is_some_and(|n| !n.same(m))),
                transform: (p.transform != now.transform).then_some(p.transform),
                scale: (p.scale != now.scale).then_some(p.scale),
                ..HeadConfig::default()
            };
            if cfg != HeadConfig::default() {
                set.push((h.name.clone(), cfg));
            }
        }
        // A moved arrangement places every output, so the file and the layout agree on all of them.
        if self.moved() {
            for (o, at) in self.heads.iter().zip(&self.places) {
                match set.iter_mut().find(|(n, _)| *n == o.name) {
                    Some((_, c)) => c.position = Some(*at),
                    None => set.push((
                        o.name.clone(),
                        HeadConfig {
                            position: Some(*at),
                            ..HeadConfig::default()
                        },
                    )),
                }
            }
        }
        let back = set
            .iter()
            .filter_map(|(n, _)| self.head_named(n).map(|o| (n.clone(), o.state())))
            .collect();
        let primary = self
            .primary
            .clone()
            .filter(|_| self.primary != self.primary_saved);
        (!vrr.is_empty() || !set.is_empty() || primary.is_some()).then_some(Changes {
            vrr,
            set,
            back,
            primary,
        })
    }

    /// Show output `i` with its own picks; every output's picks, the arrangement and the primary
    /// pick stay until Apply or Cancel.
    pub fn pick_monitor(&mut self, i: usize) {
        if i < self.heads.len() {
            self.monitor = i;
        }
    }

    fn modes(&self) -> Vec<HeadMode> {
        let mut m = self.head().map(|h| h.modes.clone()).unwrap_or_default();
        sort_modes(&mut m);
        m
    }

    /// The distinct sizes, in the trackbar's order.
    pub fn resolutions(&self) -> Vec<(i32, i32)> {
        let mut out: Vec<(i32, i32)> = Vec::new();
        for m in self.modes() {
            if !out.contains(&(m.width, m.height)) {
                out.push((m.width, m.height));
            }
        }
        out
    }

    /// The trackbar tick of the picked size.
    pub fn resolution_index(&self) -> usize {
        let Some(m) = self.pick().and_then(|p| p.mode) else {
            return 0;
        };
        self.resolutions()
            .iter()
            .position(|s| *s == (m.width, m.height))
            .unwrap_or(0)
    }

    /// Tick `r`: the mode of that size whose refresh is nearest the picked one's.
    pub fn pick_resolution(&mut self, r: usize) {
        if self.mode_locked() {
            return;
        }
        let Some(&size) = self.resolutions().get(r) else {
            return;
        };
        let want = self
            .pick()
            .and_then(|p| p.mode)
            .map_or(0, |m| m.refresh_mhz);
        let best = self
            .modes()
            .into_iter()
            .filter(|m| (m.width, m.height) == size)
            .min_by_key(|m| (m.refresh_mhz - want).abs());
        if let (Some(p), Some(m)) = (self.pick_mut(), best) {
            p.mode = Some(m);
        }
        self.resettle();
    }

    /// The modes at the picked size, the Refresh combo's rows, and the picked one's row.
    pub fn refresh_rows(&self) -> (Vec<HeadMode>, usize) {
        let Some(cur) = self.pick().and_then(|p| p.mode) else {
            return (Vec::new(), 0);
        };
        let rows: Vec<HeadMode> = self
            .modes()
            .into_iter()
            .filter(|m| (m.width, m.height) == (cur.width, cur.height))
            .collect();
        let sel = rows.iter().position(|m| m.same(&cur)).unwrap_or(0);
        (rows, sel)
    }

    pub fn pick_refresh(&mut self, row: usize) {
        if !self.mode_locked()
            && let Some(m) = self.refresh_rows().0.get(row).copied()
            && let Some(p) = self.pick_mut()
        {
            p.mode = Some(m);
        }
        self.resettle();
    }

    /// The Orientation combo's rows and the picked one's.
    pub fn orientation_rows(&self) -> (Vec<(String, Transform)>, usize) {
        let t = self.pick().map_or(Transform::Normal, |p| p.transform);
        let mut rows: Vec<(String, Transform)> = ORIENTATIONS
            .iter()
            .map(|(s, r)| (s.to_string(), *r))
            .collect();
        if !rows.iter().any(|(_, r)| *r == t) {
            rows.push((t.name().to_string(), t));
        }
        let sel = rows.iter().position(|(_, r)| *r == t).unwrap_or(0);
        (rows, sel)
    }

    pub fn pick_orientation(&mut self, row: usize) {
        if let Some((_, t)) = self.orientation_rows().0.get(row).cloned()
            && let Some(p) = self.pick_mut()
        {
            p.transform = t;
        }
        self.resettle();
    }

    pub fn pick_scale(&mut self, row: usize) {
        if let Some(p) = self.pick_mut() {
            p.scale = row_scale(row);
        }
        self.resettle();
    }

    /// Tick or untick VRR; an output without it keeps it off.
    pub fn set_vrr(&mut self, on: bool) {
        let can = self
            .head()
            .is_some_and(|h| h.adaptive_sync != AdaptiveSync::Unsupported);
        if let Some(p) = self.pick_mut() {
            p.vrr = on && can;
        }
    }

    /// "1920 by 1080 pixels" under the trackbar.
    pub fn mode_label(&self) -> String {
        match self.pick().and_then(|p| p.mode) {
            Some(m) => format!("{} by {} pixels", m.width, m.height),
            None => String::new(),
        }
    }
}

/// Modes the way the trackbar walks them: by area, then width, then refresh, each once.
pub fn sort_modes(modes: &mut Vec<HeadMode>) {
    modes.sort_by_key(|m| {
        (
            i64::from(m.width) * i64::from(m.height),
            m.width,
            m.refresh_mhz,
        )
    });
    modes.dedup_by(|a, b| a.same(b));
}

/// The Scale combo's row for an output scale.
pub fn scale_row(scale: f64) -> usize {
    ((scale.clamp(1.0, 4.0) - 1.0) * 4.0).round() as usize
}

/// The output scale the Scale combo's row `i` stands for.
pub fn row_scale(i: usize) -> f64 {
    1.0 + i.min(SCALE_ROWS - 1) as f64 * 0.25
}

/// "1. Make Model on DP-1": the output's number in the well, then the make and model and the
/// connector, or the connector alone.
pub fn head_label(number: usize, h: &Head) -> String {
    let m = format!("{} {}", h.make, h.model).trim().to_string();
    if m.is_empty() || h.make == "Unknown" {
        format!("{number}. {}", h.name)
    } else {
        format!("{number}. {m} on {}", h.name)
    }
}

/// A Refresh row: "60 Hertz", "59.94 Hertz"; "Default" when the mode has no rate.
pub fn refresh_label(m: &HeadMode) -> String {
    if m.refresh_mhz > 0 {
        format!("{} Hertz", hz_string(m.refresh_mhz))
    } else {
        "Default".into()
    }
}

pub const ARRANGE: &str = "arrangement";
pub const IDENTIFY: &str = "identify";
pub const PRIMARY: &str = "primary";
pub const HEAD: &str = "head";
pub const MODE: &str = "resolution";
pub const REFRESH: &str = "refresh_rate";
pub const ORIENTATION: &str = "orientation";
pub const SCALE: &str = "scale";
pub const VRR: &str = "vrr";

const X: i32 = crate::pages::X;
const RX: i32 = 203;
/// The arrangement well across the top of the column: it takes the page's extra width and height.
pub fn well(a: Area) -> Rect {
    Rect::new(X, 44, a.w, 108 + a.dh)
}
/// How far the mode controls sit below where they would without the well at [`Area::MIN`]: the
/// well, then the Identify row.
const DY: i32 = 110;

/// A drop-list combo of `rows`, `selected` picked, greyed unless `enabled`.
fn combo(
    id: &'static str,
    (x, y, w): (i32, i32, i32),
    rows: &[String],
    selected: usize,
    enabled: bool,
) -> Item {
    let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
    let mut c = Combo::new(&refs, selected, refs.len());
    c.enabled = enabled;
    Item::new(id, Rect::new(x, y, w, COMBO_H), Control::Combo(c))
}

/// The page for `d` in `a`, in the client's coordinates: the arrangement well, Identify and the
/// primary check box, the shown output's name, then its mode controls.
pub fn page(d: &Display, a: Area) -> Form {
    let (Some(head), Some(pick)) = (d.head(), d.pick()) else {
        return Form::new(vec![Item::label_at(
            "no_monitors",
            X,
            44,
            a.w,
            "The compositor has listed no monitors.",
        )]);
    };
    let dy = DY + a.dh;
    let rw = a.right() - RX;
    let combo_w = a.combo(rw - a.dw());
    let mut items = Vec::new();
    let well = Arrange::new(d.tiles(), d.monitor());
    items.push(Item::new(
        ARRANGE,
        self::well(a),
        Control::Arrange(Box::new(well)),
    ));
    let primary = d.primary_index() == d.monitor();
    items.push(
        Item::check(PRIMARY, X, 163 + a.dh, "Use as the &primary display")
            .checked(primary)
            .enabled(!primary),
    );
    items.push(Item::button(
        IDENTIFY,
        Rect::new(a.right() - 75, 158 + a.dh, 75, 23),
        "&Identify",
        true,
        false,
    ));
    items.push(Item::label_at(
        HEAD,
        X,
        190 + a.dh,
        a.w,
        &head_label(d.monitor() + 1, head),
    ));
    items.push(Item::group(
        "res_group",
        Rect::new(X, 100 + dy, 173, 64),
        "&Screen resolution",
    ));
    items.push(Item::label_at("less", 30, 111 + dy, 30, "Less"));
    items.push(Item::label_at("more", 156, 111 + dy, 30, "More"));
    let sizes = d.resolutions().len().max(1);
    let mut bar = TrackBar::new(d.resolution_index(), sizes);
    bar.enabled = !d.mode_locked();
    items.push(Item::new(
        MODE,
        Rect::new(65, 113 + dy, 71, trackbar::HEIGHT),
        Control::TrackBar(bar),
    ));
    items.push(Item::aligned(
        "mode_label",
        Rect::new(X, 143 + dy, 172, 13),
        &d.mode_label(),
        Align::Center,
    ));
    let (rates, rate) = d.refresh_rows();
    let rates: Vec<String> = rates.iter().map(refresh_label).collect();
    items.push(Item::label_at(
        "refresh_label",
        RX,
        104 + dy,
        rw,
        "Screen &refresh rate:",
    ));
    let refresh_on = rates.len() > 1 && !d.mode_locked();
    let at = (RX, 120 + dy, combo_w);
    items.push(combo(REFRESH, at, &rates, rate, refresh_on));
    let (turns, turn) = d.orientation_rows();
    let turns: Vec<String> = turns.into_iter().map(|(s, _)| s).collect();
    items.push(Item::label_at(
        "orientation_label",
        X,
        174 + dy,
        173,
        "&Orientation:",
    ));
    items.push(combo(ORIENTATION, (X, 189 + dy, 173), &turns, turn, true));
    let scales: Vec<String> = (0..SCALE_ROWS)
        .map(|i| format!("{}%", (row_scale(i) * 100.0).round()))
        .collect();
    items.push(Item::label_at("scale_label", RX, 174 + dy, rw, "S&cale:"));
    items.push(combo(
        SCALE,
        (RX, 189 + dy, combo_w),
        &scales,
        scale_row(pick.scale),
        true,
    ));
    items.push(
        Item::check(VRR, X, 222 + dy, "Enable &variable refresh rate")
            .checked(pick.vrr)
            .enabled(head.adaptive_sync != AdaptiveSync::Unsupported),
    );
    Form::new(items)
}

/// Read control `id` of the Display page `form` into `d`; false when `id` is not one of the page's.
pub fn read(d: &mut Display, form: &Form, id: &str) -> bool {
    match id {
        ARRANGE => match form.item(ARRANGE).map(|it| &it.control) {
            Some(Control::Arrange(a)) => {
                d.set_arrangement(a);
                Some(())
            }
            _ => None,
        },
        PRIMARY => form
            .checked(PRIMARY)
            .filter(|on| *on)
            .map(|_| d.make_primary()),
        MODE => form.track_pos(MODE).map(|r| d.pick_resolution(r)),
        REFRESH => form.combo_selected(REFRESH).map(|r| d.pick_refresh(r)),
        ORIENTATION => form
            .combo_selected(ORIENTATION)
            .map(|r| d.pick_orientation(r)),
        SCALE => form.combo_selected(SCALE).map(|r| d.pick_scale(r)),
        VRR => form.checked(VRR).map(|on| d.set_vrr(on)),
        _ => return false,
    };
    true
}

fn rebuild(sheet: &mut PropSheet, i: usize, d: &Display) {
    let old = sheet.pages[i].clone();
    let focus = old.focus().and_then(|f| old.items.get(f)).map(|it| it.id);
    sheet.set_page(i, page(d, Area::of(sheet)));

    let now = &mut sheet.pages[i];
    now.carry_pointer(&old);
    if let Some(id) = focus {
        now.focus_id(id);
    }
}

/// The sheet's Display page rebuilt from `d`.
pub fn refresh(sheet: &mut PropSheet, d: &Display) {
    rebuild(sheet, Page::Display.index(), d);
}

#[cfg(test)]
pub(crate) fn test_head(name: &str) -> Head {
    let m = |width, height, refresh_mhz, preferred| HeadMode {
        width,
        height,
        refresh_mhz,
        preferred,
    };
    Head {
        name: name.into(),
        description: String::new(),
        make: "Dell Inc.".into(),
        model: "DELL U2412M".into(),
        // The compositor's order, not the trackbar's.
        modes: vec![
            m(1920, 1080, 60_000, true),
            m(1920, 1080, 75_000, false),
            m(1280, 720, 60_000, false),
            m(1024, 768, 60_000, false),
        ],
        current: Some(0),
        enabled: true,
        position: (0, 0),
        transform: Transform::Normal,
        scale: 1.0,
        adaptive_sync: AdaptiveSync::Off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(width: i32, height: i32, refresh_mhz: i32) -> HeadMode {
        HeadMode {
            width,
            height,
            refresh_mhz,
            preferred: false,
        }
    }

    #[test]
    fn modes_sort_by_area_then_refresh_and_repeat_once() {
        let mut v = vec![
            mode(1920, 1080, 75_000),
            mode(1024, 768, 60_000),
            mode(1920, 1080, 60_000),
            mode(1280, 720, 60_000),
            mode(1024, 768, 60_000),
        ];
        sort_modes(&mut v);
        assert_eq!(
            v,
            vec![
                mode(1024, 768, 60_000),
                mode(1280, 720, 60_000),
                mode(1920, 1080, 60_000),
                mode(1920, 1080, 75_000),
            ]
        );
    }

    #[test]
    fn the_trackbar_walks_sizes_and_keeps_the_nearest_refresh() {
        let mut h = test_head("DP-1");
        h.current = Some(1);
        let mut d = Display::with_heads(vec![h]);
        assert_eq!(
            d.resolutions(),
            vec![(1024, 768), (1280, 720), (1920, 1080)]
        );
        assert_eq!(d.resolution_index(), 2);
        let (rows, sel) = d.refresh_rows();
        assert_eq!((rows.len(), sel), (2, 1), "60 and 75 Hertz, 75 picked");
        assert_eq!(refresh_label(&rows[1]), "75 Hertz");
        d.pick_resolution(1);
        assert_eq!(d.pick().unwrap().mode, Some(mode(1280, 720, 60_000)));
        assert_eq!(d.refresh_rows().0.len(), 1);
        assert_eq!(d.mode_label(), "1280 by 720 pixels");
        d.pick_resolution(2);
        assert_eq!(
            d.pick().unwrap().mode.map(|m| m.refresh_mhz),
            Some(60_000),
            "back at the size, the refresh nearest 60"
        );
        d.pick_refresh(1);
        assert_eq!(d.pick().unwrap().mode.map(|m| m.refresh_mhz), Some(75_000));
    }

    #[test]
    fn a_mirrored_transform_shows_as_a_fifth_orientation_row() {
        let mut h = test_head("DP-1");
        h.transform = Transform::Flipped90;
        let mut d = Display::with_heads(vec![h]);
        let (rows, sel) = d.orientation_rows();
        assert_eq!(rows.len(), 5);
        assert_eq!(
            (rows[4].0.as_str(), rows[4].1, sel),
            ("flipped-90", Transform::Flipped90, 4)
        );
        d.pick_orientation(1);
        let (rows, sel) = d.orientation_rows();
        assert_eq!((rows.len(), rows[1].0.as_str(), sel), (4, "Portrait", 1));
    }

    #[test]
    fn the_scale_rows_are_quarter_steps_from_100_to_400_percent() {
        assert_eq!(SCALE_ROWS, 13);
        assert_eq!(
            (scale_row(1.0), scale_row(1.25), scale_row(4.0)),
            (0, 1, 12)
        );
        assert_eq!((scale_row(9.0), scale_row(0.5)), (12, 0), "clamped");
        assert_eq!((row_scale(2), row_scale(99)), (1.5, 4.0));
    }

    #[test]
    fn changes_name_only_what_differs() {
        let mut d = Display::with_heads(vec![test_head("DP-1")]);
        assert_eq!(d.changes(), None);
        assert!(!d.dirty());
        d.pick_resolution(1);
        d.set_vrr(true);
        assert!(d.dirty());
        let h = test_head("DP-1");
        assert_eq!(
            d.changes(),
            Some(Changes {
                vrr: vec![("DP-1".into(), true)],
                set: vec![(
                    "DP-1".into(),
                    HeadConfig {
                        mode: Some(mode(1280, 720, 60_000)),
                        ..HeadConfig::default()
                    }
                )],
                back: vec![("DP-1".into(), h.state())],
                primary: None,
            })
        );
        d.reset();
        assert!(!d.dirty());
        let mut none = test_head("DP-1");
        none.adaptive_sync = AdaptiveSync::Unsupported;
        let mut d = Display::with_heads(vec![none]);
        d.set_vrr(true);
        assert_eq!(d.changes(), None, "a head without VRR takes no VRR change");
    }

    #[test]
    fn new_heads_keep_a_pending_pick_unless_the_head_changed() {
        let mut d = Display::with_heads(vec![test_head("DP-1"), test_head("HDMI-A-1")]);
        d.pick_monitor(1);
        d.pick_resolution(0);
        d.set_heads(vec![test_head("DP-1"), test_head("HDMI-A-1")]);
        assert_eq!(d.head().unwrap().name, "HDMI-A-1", "the monitor stays");
        assert!(d.dirty(), "the pick stays");
        let mut moved = test_head("HDMI-A-1");
        moved.current = Some(2);
        d.set_heads(vec![test_head("DP-1"), moved]);
        assert!(!d.dirty(), "the head changed under it: the pick follows");
        d.set_heads(vec![test_head("DP-1")]);
        assert_eq!(
            d.head().unwrap().name,
            "DP-1",
            "a gone monitor falls back to the first"
        );
        let mut off = test_head("DP-2");
        off.enabled = false;
        d.set_heads(vec![off, test_head("DP-1")]);
        assert_eq!(
            d.names(),
            vec!["DP-1".to_string()],
            "a disabled head is not listed"
        );
    }

    #[test]
    fn a_mode_outside_the_list_greys_the_mode_controls_and_is_never_asked_for() {
        let mut h = test_head("DP-1");
        h.current = None;
        let mut d = Display::with_heads(vec![h]);
        assert!(d.mode_locked());
        let f = page(&d, Area::MIN);
        assert!(matches!(
            &f.item(MODE).unwrap().control,
            Control::TrackBar(t) if !t.enabled
        ));
        assert!(matches!(
            &f.item(REFRESH).unwrap().control,
            Control::Combo(c) if !c.enabled
        ));
        d.pick_resolution(1);
        d.pick_refresh(0);
        d.pick_scale(4);
        let ch = d.changes().expect("the scale still moves");
        assert_eq!(ch.set[0].1.mode, None, "no mode is asked for");
        assert_eq!(ch.set[0].1.scale, Some(2.0));
        d.set_heads(vec![test_head("DP-1")]);
        assert!(!d.mode_locked());
    }

    #[test]
    fn a_head_is_labelled_by_number_make_model_and_connector() {
        let mut h = test_head("DP-1");
        assert_eq!(head_label(1, &h), "1. Dell Inc. DELL U2412M on DP-1");
        h.make = "Unknown".into();
        assert_eq!(head_label(2, &h), "2. DP-1");
        (h.make, h.model) = (String::new(), String::new());
        assert_eq!(head_label(1, &h), "1. DP-1");
    }

    fn pair() -> Display {
        let mut b = test_head("HDMI-A-1");
        b.position = (1920, 0);
        Display::with_heads(vec![test_head("DP-1"), b])
    }

    #[test]
    fn a_logical_size_turns_with_the_output_and_divides_by_its_scale() {
        let m = Some(mode(1920, 1080, 60_000));
        assert_eq!(logical_size(m, Transform::Normal, 1.0), (1920, 1080));
        assert_eq!(logical_size(m, Transform::Rotate90, 1.0), (1080, 1920));
        assert_eq!(logical_size(m, Transform::Flipped270, 2.0), (540, 960));
        assert_eq!(logical_size(m, Transform::Normal, 1.5), (1280, 720));
        assert_eq!(logical_size(None, Transform::Normal, 1.0), (0, 0));
    }

    #[test]
    fn a_moved_arrangement_asks_for_every_place() {
        let mut d = pair();
        let tiles = d.tiles();
        assert_eq!(
            tiles
                .iter()
                .map(|t| (t.number, t.pos, t.size))
                .collect::<Vec<_>>(),
            vec![(1, (0, 0), (1920, 1080)), (2, (1920, 0), (1920, 1080))]
        );
        let mut a = Arrange::new(tiles, 0);
        a.tiles[1].pos = (0, 1080);
        a.selected = 1;
        d.set_arrangement(&a);
        assert_eq!(d.monitor(), 1, "the dropped tile is shown");
        assert!(d.dirty());
        let ch = d.changes().expect("the places moved");
        let at: Vec<_> = ch
            .set
            .iter()
            .map(|(n, c)| (n.as_str(), c.position, c.mode))
            .collect();
        assert_eq!(
            at,
            vec![
                ("DP-1", Some((0, 0)), None),
                ("HDMI-A-1", Some((0, 1080)), None)
            ]
        );
        assert_eq!(ch.back[1].1.position, Some((1920, 0)));
        d.reset();
        assert!(!d.dirty(), "Cancel puts the places back");
    }

    #[test]
    fn a_pick_that_shrinks_an_output_closes_the_gap() {
        let mut d = pair();
        d.pick_scale(4);
        let t = d.tiles();
        assert_eq!((t[0].size, t[1].pos), ((960, 540), (960, 0)));
        let ch = d.changes().unwrap();
        assert_eq!(
            ch.set[0].1,
            HeadConfig {
                scale: Some(2.0),
                position: Some((0, 0)),
                ..HeadConfig::default()
            }
        );
        assert_eq!(ch.set[1].1.position, Some((960, 0)));
        d.pick_monitor(1);
        let t = d.tiles();
        assert_eq!(
            (t[0].size, t[1].pos),
            ((960, 540), (960, 0)),
            "showing the second output keeps the first one's picks and the places with them"
        );
        assert_eq!(d.changes(), Some(ch));
        d.reset();
        assert_eq!(d.tiles()[1].pos, (1920, 0), "Cancel puts both back");
        assert!(!d.dirty());
    }

    #[test]
    fn each_output_keeps_its_picks_while_another_is_shown_and_apply_asks_for_all() {
        let mut d = pair();
        d.pick_orientation(1);
        d.set_vrr(true);
        d.pick_monitor(1);
        assert_eq!(
            d.pick(),
            Some(Pick::of(d.head().unwrap())),
            "the second output shows as it runs"
        );
        d.pick_scale(4);
        d.pick_monitor(0);
        assert_eq!(
            d.pick().unwrap().transform,
            Transform::Rotate90,
            "the first output's pick is back"
        );
        assert!(d.pick().unwrap().vrr);
        let ch = d.changes().unwrap();
        assert_eq!(ch.vrr, vec![("DP-1".to_string(), true)]);
        let looks: Vec<_> = (ch.set.iter())
            .map(|(n, c)| (n.as_str(), c.transform, c.scale))
            .collect();
        assert_eq!(
            looks,
            vec![
                ("DP-1", Some(Transform::Rotate90), None),
                ("HDMI-A-1", None, Some(2.0)),
            ]
        );
        assert_eq!(ch.back.len(), 2, "No puts both back");
        let mut b = test_head("HDMI-A-1");
        b.position = (1920, 0);
        d.set_heads(vec![test_head("DP-1"), b]);
        assert_eq!(
            d.changes().map(|c| c.set.len()),
            Some(2),
            "a relist keeps them"
        );
    }

    #[test]
    fn the_primary_check_picks_the_shown_output() {
        let mut d = pair();
        d.set_primary_saved(None);
        let f = page(&d, Area::MIN);
        assert!(matches!(
            f.item(PRIMARY).unwrap().control,
            Control::Check {
                checked: true,
                enabled: false,
                ..
            }
        ));
        d.pick_monitor(1);
        let mut f = page(&d, Area::MIN);
        assert_eq!(f.checked(PRIMARY), Some(false));
        f.set_check(PRIMARY, true);
        assert!(read(&mut d, &f, PRIMARY));
        assert_eq!(d.primary_index(), 1);
        assert_eq!(d.changes().unwrap().primary.as_deref(), Some("HDMI-A-1"));
        assert!(
            d.changes().unwrap().set.is_empty(),
            "nothing for the outputs"
        );
        d.primary_saved();
        assert!(!d.dirty());
        d.set_primary_saved(Some("HDMI-A-9".into()));
        assert_eq!(d.primary_index(), 0, "an output not listed: the first");
    }

    #[test]
    fn new_heads_keep_the_arrangement_unless_they_moved() {
        let mut d = pair();
        let mut a = Arrange::new(d.tiles(), 0);
        a.tiles[1].pos = (0, 1080);
        d.set_arrangement(&a);
        let mut b = test_head("HDMI-A-1");
        b.position = (1920, 0);
        d.set_heads(vec![test_head("DP-1"), b.clone()]);
        assert_eq!(d.tiles()[1].pos, (0, 1080));
        b.position = (1920, 200);
        d.set_heads(vec![test_head("DP-1"), b]);
        assert_eq!(d.tiles()[1].pos, (1920, 200));
    }

    use reprisekit::controls::form::{self, Control, Form};
    use reprisekit::controls::metrics::ControlMetrics;
    use reprisekit::controls::propsheet::{self, PropSheet};
    use reprisekit::look::widgets::draw_page;
    use reprisekit::raster::{Canvas, Rect};
    use reprisekit::text::TextRenderer;
    use reprisekit::trackbar::TrackBar;

    fn at(f: &Form, id: &str) -> Rect {
        f.item(id)
            .unwrap_or_else(|| panic!("{id} is on the page"))
            .placed(&ControlMetrics::CLASSIC)
    }

    #[test]
    fn the_page_geometry() {
        let two = page(&pair(), Area::MIN);
        assert_eq!(at(&two, ARRANGE), Rect::new(20, 44, 358, 108));
        assert_eq!(at(&two, PRIMARY), Rect::new(20, 163, 13, 13));
        assert_eq!(at(&two, IDENTIFY), Rect::new(303, 158, 75, 23));
        assert_eq!(at(&two, HEAD), Rect::new(20, 190, 358, 13));
        assert_eq!(at(&two, "res_group"), Rect::new(20, 210, 173, 64));
        assert_eq!(at(&two, MODE), Rect::new(65, 223, 71, 26));
        assert_eq!(at(&two, "mode_label"), Rect::new(20, 253, 172, 13));
        assert_eq!(at(&two, REFRESH), Rect::new(203, 230, 175, 21));
        assert_eq!(at(&two, ORIENTATION), Rect::new(20, 299, 173, 21));
        assert_eq!(at(&two, SCALE), Rect::new(203, 299, 175, 21));
        assert_eq!(at(&two, VRR), Rect::new(20, 332, 13, 13));
        for it in &two.items {
            let r = it.placed(&ControlMetrics::CLASSIC);
            assert!(
                r.x >= 20 && r.right() <= 378 && r.y >= 44 && r.bottom() <= 380,
                "{} at {r:?} leaves the content column",
                it.id
            );
        }
        let one = page(&Display::with_heads(vec![test_head("DP-1")]), Area::MIN);
        assert_eq!(at(&one, HEAD), Rect::new(20, 190, 358, 13));
        assert_eq!(
            page(&Display::default(), Area::MIN).items.len(),
            1,
            "the note alone"
        );
    }

    #[test]
    fn the_pages_access_keys_are_its_own() {
        let f = page(&pair(), Area::MIN);
        let mut keys: Vec<char> = f
            .items
            .iter()
            .filter_map(|it| match &it.control {
                Control::Label { text, .. } => reprisekit::text::access_key(text),
                Control::Check { label, .. } => reprisekit::text::access_key(label),
                Control::Button { label, .. } => reprisekit::text::access_key(label),
                _ => None,
            })
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            vec!['c', 'i', 'o', 'p', 'r', 'v'],
            "Screen resolution's S is its group's"
        );
    }

    #[test]
    fn ctrl_arrows_move_the_picked_tile_on_the_page() {
        let mut d = pair();
        let mut f = page(&d, Area::MIN);
        f.focus_id(PRIMARY);
        assert_eq!(f.shove(arrange::Dir::Right), None, "the well has no focus");
        f.focus_id(ARRANGE);
        assert_eq!(
            f.shove(arrange::Dir::Right),
            Some(form::Event::Changed(ARRANGE))
        );
        assert!(read(&mut d, &f, ARRANGE));
        let at: Vec<_> = d.tiles().iter().map(|t| t.pos).collect();
        assert_eq!(at, vec![(1920, 0), (0, 0)]);
        assert_eq!(
            f.shove(arrange::Dir::Right),
            Some(form::Event::None),
            "at the edge"
        );
    }

    #[test]
    fn the_page_draws_the_arrangement_well() {
        let theme = reprisekit::theme::Theme::default();
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        let f = page(&pair(), Area::MIN);
        let mut c = Canvas::new(398, 400);
        form::draw(&mut c, &text, &p, &f, false, theme.fonts.size_px);
        let Control::Arrange(a) = &f.item(ARRANGE).unwrap().control else {
            panic!("the well is an arrangement")
        };
        let lay = reprisekit::controls::arrange::layout(well(Area::MIN), a);
        let r = a.tile_rect(&lay, 0);
        assert_eq!(c.get(r.x, r.y), theme.colors.hilight.argb());
        assert_eq!(
            c.get(well(Area::MIN).x + 3, well(Area::MIN).y + 3),
            theme.colors.button_shadow.argb()
        );
    }

    #[test]
    fn the_page_shows_the_picks() {
        let mut h = test_head("DP-1");
        h.adaptive_sync = AdaptiveSync::Unsupported;
        h.scale = 1.5;
        let f = page(&Display::with_heads(vec![h]), Area::MIN);
        assert_eq!(
            f.item(MODE).unwrap().control,
            Control::TrackBar(TrackBar::new(2, 3))
        );
        assert_eq!(f.combo_selected(SCALE), Some(2));
        assert_eq!(f.combo_selected(ORIENTATION), Some(0));
        assert_eq!(f.combo_selected(REFRESH), Some(0));
        assert!(matches!(
            f.item(VRR).unwrap().control,
            Control::Check {
                checked: false,
                enabled: false,
                ..
            }
        ));
    }

    #[test]
    fn the_page_draws_its_trackbar_and_greyed_vrr_box_in_every_look() {
        use reprisekit::interface::Interface;
        let mut h = test_head("DP-1");
        h.adaptive_sync = AdaptiveSync::Unsupported;
        let d = Display::with_heads(vec![h]);
        let groups: &[(&str, &[&str])] = &[("Look", &["Display"])];
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = reprisekit::interfaces::theme_for(i);
            let text = TextRenderer::load(&theme.fonts);
            let (p, px, cm) = (theme.paint(), theme.fonts.size_px, theme.look.controls());
            let sheet =
                PropSheet::with_list(groups, vec![page(&d, Area::MIN)], crate::pages::CLIENT);
            let (w, h) = (sheet.size.0 as u32, sheet.size.1 as u32);
            let mut c = Canvas::new(w, h);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            propsheet::draw(&mut c, &text, &p, &sheet, false, px);
            let lay = sheet.layout(&text, px);
            for id in [MODE, VRR] {
                let it = sheet.pages[0].item(id).unwrap();
                let mut want = Canvas::new(w, h);
                want.fill_rect(want.bounds(), theme.colors.button_face);
                draw_page(&mut want, &p, lay.page);
                let mut alone = Form::new(vec![it.clone()]);
                alone.set_focus(None);
                form::draw(&mut want, &text, &p.on_page(lay.page), &alone, false, px);
                let r = it.placed(&cm);
                for y in r.y..r.bottom() {
                    for x in r.x..r.right() {
                        assert_eq!(c.get(x, y), want.get(x, y), "{i:?}: {id} at ({x}, {y})");
                    }
                }
            }
        }
    }

    #[test]
    fn a_read_moves_the_pick_and_the_rebuild_keeps_the_focus() {
        let mut d = Display::with_heads(vec![test_head("DP-1")]);
        let groups: &[(&str, &[&str])] = &[("Look", &["Display"])];
        let mut sheet =
            PropSheet::with_list(groups, vec![page(&d, Area::MIN)], crate::pages::CLIENT);
        sheet.pages[0].set_track(MODE, 0);
        sheet.pages[0].focus_id(MODE);
        assert!(read(&mut d, &sheet.pages[0], MODE));
        assert_eq!(d.mode_label(), "1024 by 768 pixels");
        assert!(
            !read(&mut d, &sheet.pages[0], "snap_picker"),
            "not the page's"
        );
        rebuild(&mut sheet, 0, &d);
        let f = &sheet.pages[0];
        assert_eq!(
            f.focus().and_then(|i| f.items.get(i)).map(|it| it.id),
            Some(MODE)
        );
        assert_eq!(f.track_pos(MODE), Some(0));
    }

    #[test]
    fn re_picking_the_shown_monitor_keeps_the_pending_picks() {
        let mut d = Display::with_heads(vec![test_head("DP-1"), test_head("HDMI-A-1")]);
        d.pick_resolution(0);
        d.pick_monitor(0);
        assert!(d.dirty(), "the same monitor keeps its picks");
        d.pick_monitor(1);
        assert_eq!(
            d.pick(),
            Some(Pick::of(d.head().unwrap())),
            "another monitor shows as it runs"
        );
        assert!(d.dirty(), "the first one's picks wait for Apply or Cancel");
    }

    #[test]
    fn a_scale_between_rows_selects_the_nearer_and_an_untouched_page_has_no_changes() {
        let mut h = test_head("DP-1");
        h.scale = 1.33;
        let d = Display::with_heads(vec![h]);
        assert_eq!(
            scale_row(d.pick().unwrap().scale),
            1,
            "1.33 is the 125% row"
        );
        assert_eq!(d.changes(), None);
        assert!(!d.dirty());
    }
}
