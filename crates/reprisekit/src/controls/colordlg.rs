//! The Color dialog: 48 basic colours and 16 custom wells, and once Define Custom Colors >> widens
//! it, a hue and saturation field, a luminosity bar, Hue/Sat/Lum and Red/Green/Blue spinners and a
//! preview. Hue, saturation and luminosity are whole steps: hue 0..=239, the others 0..=240.
//! [`view`] lays the window out as a form with the wells, field and bar drawn over its blanks.

use crate::controls::form::{self, Form};
use crate::textedit::EditKey;
use crate::theme::Color;

pub mod view;
pub use view::*;

#[cfg(test)]
mod tests;

/// Steps of saturation and luminosity, and of hue round the circle (hue 240 is hue 0).
pub const HLS_MAX: i32 = 240;
const RGB_MAX: i32 = 255;
/// The hue a grey reads with: it has none of its own.
pub const GREY_HUE: i32 = 160;

pub const COLS: usize = 8;
pub const BASIC_ROWS: usize = 6;
pub const CUSTOM_ROWS: usize = 2;
pub const CUSTOM: usize = COLS * CUSTOM_ROWS;

const fn from_hex(v: u32) -> Color {
    Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// The basic colours, row by row: tints, full colours, two darker rows, then greys.
pub const BASIC: [Color; COLS * BASIC_ROWS] = [
    from_hex(0xFF8080),
    from_hex(0xFFFF80),
    from_hex(0x80FF80),
    from_hex(0x00FF80),
    from_hex(0x80FFFF),
    from_hex(0x0080FF),
    from_hex(0xFF80C0),
    from_hex(0xFF80FF),
    from_hex(0xFF0000),
    from_hex(0xFFFF00),
    from_hex(0x80FF00),
    from_hex(0x00FF40),
    from_hex(0x00FFFF),
    from_hex(0x0080C0),
    from_hex(0x8080C0),
    from_hex(0xFF00FF),
    from_hex(0x804040),
    from_hex(0xFF8040),
    from_hex(0x00FF00),
    from_hex(0x008080),
    from_hex(0x004080),
    from_hex(0x8080FF),
    from_hex(0x800040),
    from_hex(0xFF0080),
    from_hex(0x800000),
    from_hex(0xFF8000),
    from_hex(0x008000),
    from_hex(0x008040),
    from_hex(0x0000FF),
    from_hex(0x0000A0),
    from_hex(0x800080),
    from_hex(0x8000FF),
    from_hex(0x400000),
    from_hex(0x804000),
    from_hex(0x004000),
    from_hex(0x004040),
    from_hex(0x000080),
    from_hex(0x000040),
    from_hex(0x400040),
    from_hex(0x400080),
    from_hex(0x000000),
    from_hex(0x808000),
    from_hex(0x808040),
    from_hex(0x808080),
    from_hex(0x408080),
    from_hex(0xC0C0C0),
    from_hex(0x400040),
    from_hex(0xFFFFFF),
];

/// A colour as hue (0..=239), saturation and luminosity (0..=240).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hsl {
    pub h: i32,
    pub s: i32,
    pub l: i32,
}

/// `c` in whole hue, saturation and luminosity steps, each rounded to the nearest; a grey takes
/// [`GREY_HUE`].
pub fn hsl_of(c: Color) -> Hsl {
    let (r, g, b) = (i32::from(c.r), i32::from(c.g), i32::from(c.b));
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let (sum, span) = (max + min, max - min);
    let l = (sum * HLS_MAX + RGB_MAX) / (2 * RGB_MAX);
    if span == 0 {
        return Hsl {
            h: GREY_HUE,
            s: 0,
            l,
        };
    }
    let s = if l <= HLS_MAX / 2 {
        (span * HLS_MAX + sum / 2) / sum
    } else {
        let room = 2 * RGB_MAX - sum;
        (span * HLS_MAX + room / 2) / room
    };
    let delta = |v: i32| ((max - v) * (HLS_MAX / 6) + span / 2) / span;
    let (dr, dg, db) = (delta(r), delta(g), delta(b));
    let h = if r == max {
        db - dg
    } else if g == max {
        HLS_MAX / 3 + dr - db
    } else {
        2 * HLS_MAX / 3 + dg - dr
    };
    Hsl {
        h: h.rem_euclid(HLS_MAX),
        s,
        l,
    }
}

/// The colour of `hsl`, each channel rounded to the nearest.
pub fn rgb_of(hsl: Hsl) -> Color {
    let Hsl { h, s, l } = hsl;
    let (s, l) = (s.clamp(0, HLS_MAX), l.clamp(0, HLS_MAX));
    if s == 0 {
        let v = ((l * RGB_MAX + HLS_MAX / 2) / HLS_MAX) as u8;
        return Color::rgb(v, v, v);
    }
    let hi = if l <= HLS_MAX / 2 {
        (l * (HLS_MAX + s) + HLS_MAX / 2) / HLS_MAX
    } else {
        l + s - (l * s + HLS_MAX / 2) / HLS_MAX
    };
    let lo = 2 * l - hi;
    let ch = |hue: i32| {
        let v = channel(lo, hi, hue.rem_euclid(HLS_MAX));
        ((v * RGB_MAX + HLS_MAX / 2) / HLS_MAX).clamp(0, RGB_MAX) as u8
    };
    Color::rgb(ch(h + HLS_MAX / 3), ch(h), ch(h - HLS_MAX / 3))
}

/// One channel's level, `lo..=hi` in HLS steps, at `hue` (0..240) round from it.
fn channel(lo: i32, hi: i32, hue: i32) -> i32 {
    let sixth = HLS_MAX / 6;
    let ramp = |t: i32| lo + ((hi - lo) * t + HLS_MAX / 12) / sixth;
    if hue < sixth {
        ramp(hue)
    } else if hue < HLS_MAX / 2 {
        hi
    } else if hue < 2 * HLS_MAX / 3 {
        ramp(2 * HLS_MAX / 3 - hue)
    } else {
        lo
    }
}

/// The colour at fractional hue (0..240), saturation and luminosity (0..=240), unrounded: what the
/// field and the bar paint, smooth at every scale.
pub fn rgb_at(h: f64, s: f64, l: f64) -> Color {
    let max = f64::from(HLS_MAX);
    let (s, l) = ((s / max).clamp(0.0, 1.0), (l / max).clamp(0.0, 1.0));
    let hi = if l <= 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let lo = 2.0 * l - hi;
    let ch = |hue: f64| {
        let t = (hue / max).rem_euclid(1.0) * 6.0;
        let v = if t < 1.0 {
            lo + (hi - lo) * t
        } else if t < 3.0 {
            hi
        } else if t < 4.0 {
            lo + (hi - lo) * (4.0 - t)
        } else {
            lo
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    Color::rgb(ch(h + max / 3.0), ch(h), ch(h - max / 3.0))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grid {
    Basic,
    Custom,
}

impl Grid {
    fn rows(self) -> usize {
        match self {
            Grid::Basic => BASIC_ROWS,
            Grid::Custom => CUSTOM_ROWS,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A well: which grid, and its index in it, row by row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Well {
    pub grid: Grid,
    pub i: usize,
}

/// A grid key: an arrow, or Home / End to the grid's first or last well.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Left,
    Right,
    Up,
    Down,
    First,
    Last,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    Field,
    Lum,
}

/// What the dialog asks of its host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    /// OK: close with this colour.
    Ok(Color),
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorDlg {
    color: Color,
    /// Kept beside `color` so a grey keeps the hue it was given and a spinner keeps its step.
    hsl: Hsl,
    pub custom: [Color; CUSTOM],
    picked: Option<Well>,
    expanded: bool,
    /// The grid with the keyboard; `None` while one of the form's controls has it.
    grid: Option<Grid>,
    /// Each grid's keyboard well, where its arrows start.
    cursor: [usize; 2],
    /// The custom well Add to Custom Colors fills while no custom well is picked.
    next_custom: usize,
    drag: Option<Drag>,
}

impl ColorDlg {
    /// The dialog on `color`, its well picked if it has one, with `custom` in the custom wells.
    pub fn new(color: Color, custom: [Color; CUSTOM]) -> ColorDlg {
        let picked = BASIC
            .iter()
            .position(|&c| c == color)
            .map(|i| Well {
                grid: Grid::Basic,
                i,
            })
            .or_else(|| {
                custom.iter().position(|&c| c == color).map(|i| Well {
                    grid: Grid::Custom,
                    i,
                })
            });
        let mut cursor = [0; 2];
        if let Some(w) = picked {
            cursor[w.grid.index()] = w.i;
        }
        ColorDlg {
            color,
            hsl: hsl_of(color),
            custom,
            picked,
            expanded: false,
            grid: Some(picked.map_or(Grid::Basic, |w| w.grid)),
            cursor,
            next_custom: 0,
            drag: None,
        }
    }

    pub fn color(&self) -> Color {
        self.color
    }

    pub fn hsl(&self) -> Hsl {
        self.hsl
    }

    pub fn picked(&self) -> Option<Well> {
        self.picked
    }

    pub fn expanded(&self) -> bool {
        self.expanded
    }

    pub fn grid(&self) -> Option<Grid> {
        self.grid
    }

    pub fn cursor(&self, grid: Grid) -> usize {
        self.cursor[grid.index()]
    }

    pub fn well_color(&self, w: Well) -> Color {
        match w.grid {
            Grid::Basic => BASIC[w.i],
            Grid::Custom => self.custom[w.i],
        }
    }

    /// Well `w` becomes the colour, and its grid takes the keyboard.
    pub fn pick(&mut self, w: Well) {
        self.color = self.well_color(w);
        self.hsl = hsl_of(self.color);
        self.picked = Some(w);
        self.cursor[w.grid.index()] = w.i;
        self.grid = Some(w.grid);
    }

    /// A new colour from the field, the bar or a spinner: a custom pick stays, for Add to Custom
    /// Colors to fill; a basic one no longer holds the colour.
    fn set_hsl(&mut self, hsl: Hsl) -> bool {
        let hsl = Hsl {
            h: hsl.h.clamp(0, HLS_MAX - 1),
            s: hsl.s.clamp(0, HLS_MAX),
            l: hsl.l.clamp(0, HLS_MAX),
        };
        if hsl == self.hsl {
            return false;
        }
        self.hsl = hsl;
        self.color = rgb_of(hsl);
        self.drop_basic_pick();
        true
    }

    fn set_rgb(&mut self, c: Color) -> bool {
        if c == self.color {
            return false;
        }
        self.color = c;
        self.hsl = hsl_of(c);
        self.drop_basic_pick();
        true
    }

    fn drop_basic_pick(&mut self) {
        if self.picked.is_some_and(|w| w.grid == Grid::Basic) {
            self.picked = None;
        }
    }

    /// An arrow or Home / End in the grid with the keyboard: its well moves (stopping at the
    /// grid's ends) and becomes the colour. False without a grid.
    pub fn step(&mut self, step: Step) -> bool {
        let Some(grid) = self.grid else {
            return false;
        };
        let n = COLS * grid.rows();
        let at = self.cursor[grid.index()];
        let to = match step {
            Step::Left => at.saturating_sub(1),
            Step::Right => (at + 1).min(n - 1),
            Step::Up => at.checked_sub(COLS).unwrap_or(at),
            Step::Down => Some(at + COLS).filter(|&i| i < n).unwrap_or(at),
            Step::First => 0,
            Step::Last => n - 1,
        };
        self.pick(Well { grid, i: to });
        true
    }

    /// Add to Custom Colors: the colour goes into the picked custom well, else the next one in
    /// turn, which becomes the pick.
    pub fn add_custom(&mut self) {
        let i = match self.picked {
            Some(Well {
                grid: Grid::Custom,
                i,
            }) => i,
            _ => {
                let i = self.next_custom;
                self.next_custom = (i + 1) % CUSTOM;
                i
            }
        };
        self.custom[i] = self.color;
        self.picked = Some(Well {
            grid: Grid::Custom,
            i,
        });
        self.cursor[Grid::Custom.index()] = i;
    }

    /// Define Custom Colors >>: the field, bar and spinners show from now on.
    pub fn expand(&mut self) -> bool {
        !std::mem::replace(&mut self.expanded, true)
    }

    /// A press on hue `h` and saturation `s` of the field: the colour takes them at its
    /// luminosity, and a drag carries on from here.
    pub fn press_field(&mut self, h: i32, s: i32) -> bool {
        self.drag = Some(Drag::Field);
        self.set_hsl(Hsl { h, s, ..self.hsl })
    }

    pub fn press_lum(&mut self, l: i32) -> bool {
        self.drag = Some(Drag::Lum);
        self.set_hsl(Hsl { l, ..self.hsl })
    }

    /// The pointer at hue `h`, saturation `s` and luminosity `l` (each clamped into range) during
    /// a drag begun on the field or the bar: the one pressed follows it. False with no drag.
    pub fn drag_to(&mut self, h: i32, s: i32, l: i32) -> bool {
        match self.drag {
            Some(Drag::Field) => self.set_hsl(Hsl { h, s, ..self.hsl }),
            Some(Drag::Lum) => self.set_hsl(Hsl { l, ..self.hsl }),
            None => false,
        }
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn release(&mut self) -> bool {
        self.drag.take().is_some()
    }

    /// The grid takes the keyboard from the form.
    pub fn focus_grid(&mut self, f: &mut Form, grid: Grid) {
        self.grid = Some(grid);
        f.set_focus(None);
    }

    /// Tab or Shift+Tab: basic wells, custom wells, then the form's controls, round again.
    pub fn tab(&mut self, f: &mut Form, back: bool) {
        match (self.grid, back) {
            (Some(Grid::Basic), false) => self.grid = Some(Grid::Custom),
            (Some(Grid::Custom), true) => self.grid = Some(Grid::Basic),
            (Some(_), _) => {
                self.grid = None;
                f.focus_edge(back);
            }
            (None, _) => {
                if !f.tab_no_wrap(back) {
                    let to = if back { Grid::Custom } else { Grid::Basic };
                    self.focus_grid(f, to);
                }
            }
        }
    }

    /// A key ahead of the form's: Tab and the grids' keys, and the grids' access keys. `None`
    /// leaves it to the form, whose answer goes through [`ColorDlg::form_event`].
    pub fn key(&mut self, f: &mut Form, key: form::Key) -> Option<Outcome> {
        if f.popup_open() {
            return None;
        }
        let step = match key {
            form::Key::Tab { back } => {
                self.tab(f, back);
                return Some(Outcome::Redraw);
            }
            form::Key::Access(c) => {
                let grid = [(BASIC_LABEL, Grid::Basic), (CUSTOM_LABEL, Grid::Custom)]
                    .into_iter()
                    .find(|(id, _)| access_char(f, id) == Some(c.to_ascii_lowercase()))?
                    .1;
                self.focus_grid(f, grid);
                return Some(Outcome::Redraw);
            }
            _ if self.grid.is_none() => return None,
            form::Key::Left | form::Key::Edit(EditKey::Left { .. }) => Step::Left,
            form::Key::Right | form::Key::Edit(EditKey::Right { .. }) => Step::Right,
            form::Key::Up => Step::Up,
            form::Key::Down => Step::Down,
            form::Key::Home | form::Key::Edit(EditKey::Home { .. }) => Step::First,
            form::Key::End | form::Key::Edit(EditKey::End { .. }) => Step::Last,
            _ => return None,
        };
        self.step(step);
        follow(f, self, None);
        Some(Outcome::Redraw)
    }

    /// What the form's event means: a spinner moved the colour, a button pressed, OK or Cancel.
    pub fn form_event(&mut self, f: &mut Form, ev: form::Event) -> Outcome {
        if f.focus().is_some() {
            self.grid = None;
        }
        match ev {
            form::Event::None => Outcome::None,
            form::Event::Pressed(OK) | form::Event::Default => Outcome::Ok(self.color),
            form::Event::Pressed(CANCEL) | form::Event::Cancel => Outcome::Cancel,
            form::Event::Pressed(DEFINE) => {
                if self.expand() {
                    let mut wide = form(self);
                    wide.carry_pointer(f);
                    *f = wide;
                    f.focus_id(HUE);
                    self.grid = None;
                }
                Outcome::Redraw
            }
            form::Event::Pressed(ADD) => {
                self.add_custom();
                Outcome::Redraw
            }
            form::Event::Changed(id) => {
                let v = |id| spin_value(f, id).unwrap_or(0);
                let changed = match id {
                    HUE | SAT | LUM => self.set_hsl(Hsl {
                        h: v(HUE),
                        s: v(SAT),
                        l: v(LUM),
                    }),
                    RED | GREEN | BLUE => {
                        let ch = |id| v(id).clamp(0, RGB_MAX) as u8;
                        self.set_rgb(Color::rgb(ch(RED), ch(GREEN), ch(BLUE)))
                    }
                    _ => false,
                };
                if changed {
                    follow(f, self, Some(id));
                }
                Outcome::Redraw
            }
            _ => Outcome::Redraw,
        }
    }
}
