//! The pointer over the grid: selection, mouse reports, the wheel, primary paste, URL clicks.
//! Report encoding ported from alacritty (Apache-2.0, Copyright The Alacritty Project).

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point, Side as TermSide};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{Term, TermMode};
use repriseapp::{BTN_LEFT, BTN_RIGHT, Ctx, Mods, PointerInput, Reply};
use reprisekit::controls::termgrid::{self, Hit, Side};
use reprisekit::pointer::{Click, DOUBLE_CLICK_SLOP};

use crate::Terminal;
use crate::chrome::{self, BTN_MIDDLE};
use crate::tab::Tab;
use crate::urls;

/// A cell, (col, row) in the viewport.
type CellAt = (usize, usize);

/// The pointer's state over the grid.
#[derive(Default)]
pub struct Mouse {
    /// The pid of the tab the held gesture belongs to; another open tab ends the gesture.
    tab: u32,
    /// A selection whose end follows the pointer.
    selecting: bool,
    /// Presses reported to the program and not yet released: evdev button, report code with
    /// modifiers. Motion reports carry the last.
    reports: Vec<(u32, u8)>,
    clicks: Clicks,
    /// The cell last reported to the program: motion within one cell reports nothing.
    reported: Option<CellAt>,
}

impl Mouse {
    fn held(&self) -> bool {
        self.selecting || !self.reports.is_empty()
    }

    /// Start holding for the tab `pid`, ending a gesture held for another.
    fn hold(&mut self, pid: u32) {
        if self.tab != pid {
            self.end();
            self.tab = pid;
        }
    }

    fn end(&mut self) {
        self.selecting = false;
        self.reports.clear();
    }
}

/// Counts left presses landing close together into single, double and triple clicks.
#[derive(Default)]
struct Clicks {
    last: Option<(Click, u8)>,
}

impl Clicks {
    /// The press at `at` as 1, 2 or 3; a fourth starts over.
    fn press(&mut self, at: Click) -> u8 {
        let n = match self.last {
            Some((first, n))
                if at.time.wrapping_sub(first.time) < at.double_ms.max(0) as u32
                    && (at.x - first.x).abs() <= DOUBLE_CLICK_SLOP
                    && (at.y - first.y).abs() <= DOUBLE_CLICK_SLOP =>
            {
                n % 3 + 1
            }
            _ => 1,
        };
        self.last = Some((at, n));
        n
    }
}

/// The selection a press of `clicks` starts.
fn kind(clicks: u8) -> SelectionType {
    match clicks {
        2 => SelectionType::Semantic,
        3 => SelectionType::Lines,
        _ => SelectionType::Simple,
    }
}

fn point<T>(term: &Term<T>, (col, row): CellAt) -> Point {
    let offset = term.grid().display_offset() as i32;
    Point::new(Line(row as i32 - offset), Column(col))
}

fn side(s: Side) -> TermSide {
    match s {
        Side::Left => TermSide::Left,
        Side::Right => TermSide::Right,
    }
}

/// Start a selection of `ty` at `cell`, replacing any other.
pub fn select<T>(term: &mut Term<T>, ty: SelectionType, cell: CellAt, s: Side) {
    term.selection = Some(Selection::new(ty, point(term, cell), side(s)));
}

/// Move the selection's end to `cell`.
pub fn extend<T>(term: &mut Term<T>, cell: CellAt, s: Side) {
    let p = point(term, cell);
    if let Some(sel) = &mut term.selection {
        sel.update(p, side(s));
    }
}

/// The report code of an evdev button: 0 left, 1 middle, 2 right.
fn button_code(button: u32) -> Option<u8> {
    match button {
        BTN_LEFT => Some(0),
        BTN_MIDDLE => Some(1),
        BTN_RIGHT => Some(2),
        _ => None,
    }
}

fn mods_code(m: Mods) -> u8 {
    4 * u8::from(m.shift) + 8 * u8::from(m.alt) + 16 * u8::from(m.ctrl)
}

/// Motion reports add this to the button code; 3 is "no button".
const MOTION: u8 = 32;
const WHEEL_UP: u8 = 64;
const WHEEL_DOWN: u8 = 65;

/// The bytes reporting button `code` pressed or released over `cell`: SGR when the program asked
/// for it, else the normal encoding, which cannot reach cells past 223 (2015 in UTF-8 mode).
pub fn report(code: u8, pressed: bool, (col, row): CellAt, mode: TermMode) -> Option<Vec<u8>> {
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{code};{};{}{end}", col + 1, row + 1).into_bytes());
    }
    let utf8 = mode.contains(TermMode::UTF8_MOUSE);
    let max = if utf8 { 2015 } else { 223 };
    if col >= max || row >= max {
        return None;
    }
    // A normal release does not say which button: 3, with the modifiers kept.
    let code = if pressed { code } else { 3 + (code & 28) };
    let mut out = vec![0x1b, b'[', b'M', 32 + code];
    for v in [col, row] {
        let v = 33 + v as u32;
        match char::from_u32(v) {
            Some(c) if utf8 && v >= 128 => out.extend(c.encode_utf8(&mut [0; 2]).as_bytes()),
            _ => out.push(v as u8),
        }
    }
    Some(out)
}

/// What a wheel turn does.
#[derive(Debug, PartialEq, Eq)]
pub enum Wheel {
    /// Scroll the view this many lines back (negative: forward).
    Scroll(i32),
    /// Write these bytes to the program.
    Write(Vec<u8>),
}

/// A wheel turn of `rows` (positive is down) over `cell`: reports when the program takes the
/// mouse, arrow keys on the alternate screen, else the scrollback moves.
pub fn wheel(rows: i32, cell: CellAt, mode: TermMode) -> Wheel {
    let n = rows.unsigned_abs() as usize;
    if mode.intersects(TermMode::MOUSE_MODE) {
        let code = if rows < 0 { WHEEL_UP } else { WHEEL_DOWN };
        let one = report(code, true, cell, mode).unwrap_or_default();
        return Wheel::Write(one.repeat(n));
    }
    if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
        let lead = if mode.contains(TermMode::APP_CURSOR) {
            b'O'
        } else {
            b'['
        };
        let arrow = if rows < 0 { b'A' } else { b'B' };
        return Wheel::Write([0x1b, lead, arrow].repeat(n));
    }
    Wheel::Scroll(-rows)
}

impl Terminal {
    /// Route a pointer event: a gesture begun on the grid keeps it until the release.
    pub(crate) fn pointer(&mut self, p: PointerInput, ctx: &mut Ctx) -> Reply {
        if self.chrome.menu.is_some() {
            return self.chrome_pointer(p, ctx);
        }
        if self.tabs.current().map(Tab::pid) != Some(self.mouse.tab) {
            self.mouse.end();
        }
        let grid = chrome::grid_rect(ctx);
        match p {
            PointerInput::Press {
                x,
                y,
                button,
                time,
                mods,
            } if self.chrome.press.is_none() && grid.contains(x, y) => {
                self.grid_press(x, y, button, time, mods, ctx)
            }
            PointerInput::Motion { x, y } if self.mouse.held() => self.grid_drag(x, y, ctx),
            PointerInput::Motion { x, y } => {
                let reply = self.chrome_pointer(p, ctx);
                match grid.contains(x, y) {
                    true => reply.and(self.grid_hover(x, y, ctx)),
                    false => reply,
                }
            }
            PointerInput::Release { x, y, button, .. } if self.mouse.held() => {
                self.grid_release(x, y, button, ctx)
            }
            PointerInput::Scroll { x, y, rows } if grid.contains(x, y) => {
                self.grid_wheel(x, y, rows, ctx)
            }
            _ => self.chrome_pointer(p, ctx),
        }
    }

    /// The cell under (`x`, `y`), the point first pulled inside the grid's cells.
    fn cell_at(&mut self, x: i32, y: i32, ctx: &Ctx) -> Option<Hit> {
        let ((cols, rows), _) = self.fitted?;
        let m = self.metrics(self.scale);
        let s = self.scale.max(1);
        let area = chrome::grid_rect(ctx);
        let x = x.clamp(area.x, area.x + (cols as i32 * m.cell_w - 1) / s);
        let y = y.clamp(area.y, area.y + (rows as i32 * m.cell_h - 1) / s);
        termgrid::hit(&m, area, s, x, y).map(|h| Hit {
            col: h.col.min(cols - 1),
            row: h.row.min(rows - 1),
            ..h
        })
    }

    fn grid_press(
        &mut self,
        x: i32,
        y: i32,
        button: u32,
        time: u32,
        mods: Mods,
        ctx: &Ctx,
    ) -> Reply {
        let Some(hit) = self.cell_at(x, y, ctx) else {
            return Reply::none();
        };
        let cell = (hit.col, hit.row);
        if button == BTN_LEFT && mods.ctrl && self.open_url(cell) {
            return Reply::none();
        }
        let Some(tab) = self.tabs.current() else {
            return Reply::none();
        };
        let mode = tab.mode();
        self.mouse.hold(tab.pid());
        if mode.intersects(TermMode::MOUSE_MODE) && !mods.shift {
            let Some(code) = button_code(button) else {
                return Reply::none();
            };
            let code = code + mods_code(mods);
            self.mouse.reports.retain(|&(b, _)| b != button);
            self.mouse.reports.push((button, code));
            self.mouse.reported = Some(cell);
            if let Some(bytes) = report(code, true, cell, mode) {
                tab.write(bytes.into());
            }
            return Reply::none();
        }
        match button {
            BTN_LEFT => {
                let clicks = self.mouse.clicks.press(ctx.theme.metrics.click(x, y, time));
                tab.with_term(|t| select(t, kind(clicks), cell, hit.side));
                self.mouse.selecting = true;
                Reply::redraw()
            }
            BTN_MIDDLE => Reply {
                paste_primary: true,
                ..Default::default()
            },
            _ => Reply::none(),
        }
    }

    fn grid_drag(&mut self, x: i32, y: i32, ctx: &Ctx) -> Reply {
        let Some(hit) = self.cell_at(x, y, ctx) else {
            return Reply::none();
        };
        let cell = (hit.col, hit.row);
        let Some(tab) = self.tabs.current() else {
            return Reply::none();
        };
        if self.mouse.selecting {
            tab.with_term(|t| extend(t, cell, hit.side));
            return Reply::redraw();
        }
        match self.mouse.reports.last() {
            Some(&(_, code)) => {
                let mode = tab.mode();
                if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
                    && self.mouse.reported != Some(cell)
                {
                    self.mouse.reported = Some(cell);
                    if let Some(bytes) = report(code + MOTION, true, cell, mode) {
                        tab.write(bytes.into());
                    }
                }
                Reply::none()
            }
            None => Reply::none(),
        }
    }

    /// Motion with no button held, reported when the program asked for all motion.
    fn grid_hover(&mut self, x: i32, y: i32, ctx: &Ctx) -> Reply {
        let Some(hit) = self.cell_at(x, y, ctx) else {
            return Reply::none();
        };
        let cell = (hit.col, hit.row);
        let Some(tab) = self.tabs.current() else {
            return Reply::none();
        };
        let mode = tab.mode();
        if mode.contains(TermMode::MOUSE_MOTION) && self.mouse.reported != Some(cell) {
            self.mouse.reported = Some(cell);
            if let Some(bytes) = report(3 + MOTION, true, cell, mode) {
                tab.write(bytes.into());
            }
        }
        Reply::none()
    }

    fn grid_release(&mut self, x: i32, y: i32, button: u32, ctx: &Ctx) -> Reply {
        let cell = self.cell_at(x, y, ctx).map(|h| (h.col, h.row));
        let Some(tab) = self.tabs.current() else {
            self.mouse.end();
            return Reply::none();
        };
        if let Some(i) = self.mouse.reports.iter().position(|&(b, _)| b == button) {
            let (_, code) = self.mouse.reports.remove(i);
            let mode = tab.mode();
            if mode.intersects(TermMode::MOUSE_MODE)
                && let Some(bytes) = cell.and_then(|c| report(code, false, c, mode))
            {
                tab.write(bytes.into());
            }
            return Reply::none();
        }
        if self.mouse.selecting && button == BTN_LEFT {
            self.mouse.selecting = false;
            return Reply {
                copy_primary: tab.selection().filter(|s| !s.is_empty()),
                ..Default::default()
            };
        }
        Reply::none()
    }

    fn grid_wheel(&mut self, x: i32, y: i32, rows: i32, ctx: &Ctx) -> Reply {
        let Some(hit) = self.cell_at(x, y, ctx) else {
            return Reply::none();
        };
        let Some(tab) = self.tabs.current() else {
            return Reply::none();
        };
        match wheel(rows, (hit.col, hit.row), tab.mode()) {
            Wheel::Scroll(n) => {
                tab.scroll(Scroll::Delta(n));
                Reply::redraw()
            }
            Wheel::Write(bytes) => {
                tab.write(bytes.into());
                Reply::none()
            }
        }
    }

    /// Open the URL drawn at `cell` with xdg-open; false when there is none.
    fn open_url(&self, (col, row): CellAt) -> bool {
        if row >= self.snap.rows {
            return false;
        }
        let line: String = self.snap.row(row).iter().map(|c| c.ch).collect();
        let Some(url) = urls::at(&line, col) else {
            return false;
        };
        if let Err(e) = repriseapp::spawn_in(&["xdg-open".into(), url.clone().into()], None, None) {
            log::warn!("cannot open {url}: {e}");
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    fn term(text: &str) -> Term<VoidListener> {
        let mut t = Term::new(Config::default(), &TermSize::new(20, 4), VoidListener);
        Processor::<StdSyncHandler>::new().advance(&mut t, text.as_bytes());
        t
    }

    fn click(x: i32, time: u32) -> Click {
        Click {
            x,
            y: 10,
            time,
            double_ms: 500,
        }
    }

    #[test]
    fn drag_selects_simple() {
        let mut t = term("hello world\r\nsecond line");
        select(&mut t, SelectionType::Simple, (2, 0), Side::Left);
        assert_eq!(
            t.selection_to_string(),
            None,
            "a press alone selects nothing"
        );
        extend(&mut t, (3, 1), Side::Right);
        assert_eq!(t.selection_to_string().as_deref(), Some("llo world\nseco"));
    }

    #[test]
    fn double_click_word() {
        let mut clicks = Clicks::default();
        assert_eq!(clicks.press(click(40, 1000)), 1);
        let n = clicks.press(click(42, 1200));
        assert_eq!(n, 2);
        let mut t = term("one two-three four");
        select(&mut t, kind(n), (6, 0), Side::Left);
        assert_eq!(t.selection_to_string().as_deref(), Some("two-three"));
    }

    #[test]
    fn triple_click_line() {
        let mut clicks = Clicks::default();
        let n: Vec<u8> = [1000, 1200, 1400, 1600]
            .iter()
            .map(|&ms| clicks.press(click(40, ms)))
            .collect();
        assert_eq!(n, [1, 2, 3, 1], "a fourth press starts over");
        assert_eq!(
            clicks.press(click(40, 2200)),
            1,
            "past the double-click time"
        );
        let mut t = term("first\r\nthe whole line\r\nlast");
        select(&mut t, kind(3), (5, 1), Side::Left);
        assert_eq!(t.selection_to_string().as_deref(), Some("the whole line\n"));
    }

    #[test]
    fn copy_wide_and_combining() {
        let mut t = term("x日本e\u{301}y");
        select(&mut t, SelectionType::Simple, (1, 0), Side::Left);
        extend(&mut t, (5, 0), Side::Right);
        assert_eq!(t.selection_to_string().as_deref(), Some("日本e\u{301}"));
    }

    #[test]
    fn mouse_report_sgr() {
        let sgr = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(report(0, true, (4, 2), sgr), Some(b"\x1b[<0;5;3M".to_vec()));
        assert_eq!(
            report(0, false, (4, 2), sgr),
            Some(b"\x1b[<0;5;3m".to_vec())
        );
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        assert_eq!(
            report(2 + mods_code(ctrl), true, (0, 0), sgr),
            Some(b"\x1b[<18;1;1M".to_vec())
        );
        let normal = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(report(0, true, (4, 2), normal), Some(b"\x1b[M %#".to_vec()));
        assert_eq!(
            report(1, false, (4, 2), normal),
            Some(b"\x1b[M#%#".to_vec())
        );
        assert_eq!(
            report(0, true, (223, 0), normal),
            None,
            "past the normal limit"
        );
        let utf8 = normal | TermMode::UTF8_MOUSE;
        assert_eq!(
            report(0, true, (200, 0), utf8),
            Some("\x1b[M \u{e9}!".as_bytes().to_vec())
        );
    }

    #[test]
    fn wheel_scrolls_or_reports() {
        let plain = TermMode::default();
        assert_eq!(wheel(-3, (0, 0), plain), Wheel::Scroll(3));
        assert_eq!(wheel(3, (0, 0), plain), Wheel::Scroll(-3));
        let alt = plain | TermMode::ALT_SCREEN;
        assert_eq!(
            wheel(-2, (0, 0), alt),
            Wheel::Write(b"\x1b[A\x1b[A".to_vec())
        );
        assert_eq!(
            wheel(1, (0, 0), alt | TermMode::APP_CURSOR),
            Wheel::Write(b"\x1bOB".to_vec())
        );
        let sgr = alt | TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(
            wheel(1, (1, 1), sgr),
            Wheel::Write(b"\x1b[<65;2;2M".to_vec())
        );
    }
}
