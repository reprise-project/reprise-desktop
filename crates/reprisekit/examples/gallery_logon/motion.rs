//! The login screen's transitions at a quarter of their way apart, a row each: Glass's pick, back
//! to the tiles, tiles coming forward and busy, Blue's pick and coming forward, then Glass's ring
//! at five steps. Written to `target/reprisekit-gallery-logon-motion@<scale>.png`.

use reprisekit::blue::Variant;
use reprisekit::interface::Interface;
use reprisekit::interfaces::logon::{Drawn, Transition, draw_logon, logon_layout};
use reprisekit::interfaces::{default_wallpaper, theme_with};
use reprisekit::logon::{Logon, LogonKey, Moves, Part};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Color;
use reprisekit::wallpapers;

const OUTPUT: Rect = Rect::new(0, 0, 640, 480);
const GAP: i32 = 10;
const COLS: i32 = 5;

type Row = (Logon, fn(&mut Logon));

fn start(interface: Interface) -> Logon {
    let mut m = Logon::new(interface, super::users(), true, Some("carol"));
    m.moves = Moves::Full;
    m
}

/// Each row's screen before and what changes it.
fn changes() -> Vec<Row> {
    let mut tiles = start(Interface::Glass);
    tiles.click(Part::SwitchUser);
    let mut typed = start(Interface::Glass);
    typed.key(LogonKey::Text("hunter2"));
    let blue = start(Interface::Blue);
    vec![
        (tiles.clone(), |m| {
            m.click(Part::Tile(0));
        }),
        (start(Interface::Glass), |m| {
            m.key(LogonKey::Escape);
        }),
        (tiles, Logon::enter),
        (typed, Logon::sent),
        (blue.clone(), |m| {
            m.click(Part::Tile(1));
        }),
        (blue, Logon::enter),
    ]
}

/// `m` drawn at `scale` as a cell: Glass over its wallpaper, Blue opaque.
fn cell(m: &Logon, scale: i32, draw: impl FnOnce(&mut Canvas)) -> Canvas {
    let (w, h) = (
        (OUTPUT.width * scale) as u32,
        (OUTPUT.height * scale) as u32,
    );
    let mut screen = Canvas::new(w, h);
    draw(&mut screen);
    match default_wallpaper(m.interface) {
        Some(b) if m.interface == Interface::Glass => {
            let mut wall = wallpapers::render(b, w, h, 0);
            wall.blit_over(0, 0, &screen);
            wall
        }
        _ => screen,
    }
}

pub fn render(scale: i32) -> Canvas {
    let mut cells = Vec::new();
    for (mut m, act) in changes() {
        m.take_motion();
        act(&mut m);
        let motion = m.take_motion().expect("a change");
        let theme = theme_with(m.interface, Variant::Blue, &Default::default()).scaled(scale);
        let text = TextRenderer::load(&theme.fonts);
        let l = logon_layout(&m, &theme, &text, OUTPUT);
        let d = Drawn {
            m: &m,
            l: &l,
            theme: &theme,
            text: &text,
            s: scale,
        };
        let t = Transition::begin(&motion, &d).expect("it plays");
        for k in 0..COLS as u32 {
            cells.push(cell(&m, scale, |c| {
                draw_logon(c, &m, &l, &theme, text.for_output(scale), scale);
                t.frame(c, &m, t.ms() * k / (COLS as u32 - 1));
            }));
        }
    }
    let mut busy = start(Interface::Glass);
    busy.set_busy(true);
    let theme = theme_with(Interface::Glass, Variant::Blue, &Default::default()).scaled(scale);
    let text = TextRenderer::load(&theme.fonts);
    let l = logon_layout(&busy, &theme, &text, OUTPUT);
    for k in 0..COLS as u32 {
        busy.spin = k * 12;
        cells.push(cell(&busy, scale, |c| {
            draw_logon(c, &busy, &l, &theme, text.for_output(scale), scale)
        }));
    }
    let rows = (cells.len() as i32 + COLS - 1) / COLS;
    let at = |i: i32, n: i32| (GAP + i * (n + GAP)) * scale;
    let mut c = Canvas::new(
        at(COLS, OUTPUT.width) as u32,
        at(rows, OUTPUT.height) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(0x60, 0x60, 0x60));
    for (i, screen) in cells.iter().enumerate() {
        let i = i as i32;
        c.blit_canvas(
            at(i % COLS, OUTPUT.width),
            at(i / COLS, OUTPUT.height),
            screen,
        );
    }
    c
}
