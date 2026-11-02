//! The launcher finding open windows and the keyboard shortcut sheet in each look, written to
//! `target/reprisekit-gallery-launcher-windows@<scale>.png` and `...-key-sheet@<scale>.png`.

use reprisekit::controls::keysheet::{self, Binding, Group, KeySheet};
use reprisekit::controls::launcher::{self, Key, Launcher, Row};
use reprisekit::glass::{Mode, Settings};
use reprisekit::interface::Interface;
use reprisekit::interfaces::{theme_for, theme_with};
use reprisekit::look::panel;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::EditKey;
use reprisekit::theme::{Color, Theme};

const LOOKS: [Interface; 3] = [Interface::Classic, Interface::Blue, Interface::Glass];

fn square(px: i32, colour: Color) -> Canvas {
    let mut c = Canvas::new(px as u32, px as u32);
    c.fill_rect(
        Rect::new(px / 16, px / 16, px - px / 8, px - px / 8),
        colour,
    );
    c
}

/// "fire" typed with a browser window open: the window leads, the program under "Programs".
fn windows_first(scale: i32) -> Launcher {
    let mut l = Launcher::new();
    l.key(Key::Char(EditKey::Insert("fire".into())));
    let icon = Some(square(16 * scale, Color::rgb(230, 120, 30)));
    let row = |name: &str| Row {
        name: name.into(),
        icon: icon.clone(),
    };
    l.set_windows(vec![row("Start Page - Firefox")], true);
    l.set_rows(vec![row("Firefox")]);
    l
}

/// Each look's panel side by side over the stripes, at `scale`.
fn side_by_side(scale: i32, draw: impl Fn(&Theme, &TextRenderer) -> Canvas) -> Canvas {
    let panels: Vec<(Canvas, i32, i32)> = LOOKS
        .iter()
        .map(|&i| {
            let theme = super::cued(theme_for(i)).scaled(scale);
            let text = TextRenderer::load(&theme.fonts);
            let m = panel::margin(&theme);
            (draw(&theme, &text), m.left * scale, m.top * scale)
        })
        .collect();
    let gap = 30 * scale;
    let w = panels.iter().map(|p| p.0.width as i32 + gap).sum::<i32>() + gap;
    let h = panels.iter().map(|p| p.0.height as i32).max().unwrap_or(0) + 2 * gap;
    let mut c = super::blurred_stripes(w, h);
    let mut x = gap;
    for (p, left, top) in &panels {
        c.blit_over(x - left, gap - top, p);
        x += p.width as i32 + gap;
    }
    c
}

pub fn launcher_windows(scale: i32) -> Canvas {
    let l = windows_first(scale);
    let icon = square(launcher::ICON * scale, Color::rgb(230, 120, 30));
    side_by_side(scale, |theme, text| {
        launcher::draw(theme, text, &l, Some(&icon), true, None)
    })
}

fn key_sheet_sample() -> KeySheet {
    let group = |title: &str, rows: &[(&str, &str)]| Group {
        title: title.into(),
        rows: rows
            .iter()
            .map(|(keys, what)| Binding {
                keys: (*keys).into(),
                what: (*what).into(),
            })
            .collect(),
    };
    KeySheet {
        groups: vec![
            group(
                "Windows",
                &[
                    ("Super+Q, Alt+F4", "Close the window"),
                    ("Alt+Tab", "Switch windows"),
                    ("Super+Tab", "Switch windows in 3D"),
                    ("Alt+Space", "Window menu"),
                    ("Super+Left", "Snap left"),
                    ("Super+Right", "Snap right"),
                    ("Super+W", "Window overview"),
                ],
            ),
            group(
                "Desktops",
                &[
                    ("Super+Ctrl+Left", "Previous desktop"),
                    ("Super+Ctrl+Right", "Next desktop"),
                ],
            ),
            group(
                "Launch and search",
                &[
                    ("Super, Ctrl+Esc", "Start menu"),
                    ("Super+Space", "Launcher"),
                    ("Super+R", "Run"),
                    ("Super+V", "Clipboard history"),
                ],
            ),
            group(
                "Screenshots",
                &[("Print", "Whole screen"), ("Shift+Print", "Area")],
            ),
            group(
                "System",
                &[
                    ("Super+L", "Lock"),
                    ("Shift+Left Alt", "Next keyboard layout"),
                ],
            ),
        ],
    }
}

/// The sheet laid out for a short output, so it takes columns: each look over the stripes,
/// then Glass light and dark over white.
pub fn key_sheet(scale: i32) -> Canvas {
    let k = key_sheet_sample();
    let sheet = |theme: &Theme, text: &TextRenderer| {
        let lay = keysheet::layout(&k, theme, text, (900, 500));
        keysheet::draw(theme, text, &k, &lay)
    };
    let striped = side_by_side(scale, sheet);
    let gap = 30 * scale;
    let glass: Vec<(Canvas, i32, i32)> = [Mode::Light, Mode::Dark]
        .into_iter()
        .map(|mode| {
            let g = Settings {
                mode,
                ..Default::default()
            };
            let theme = theme_with(Interface::Glass, Default::default(), &g).scaled(scale);
            let text = TextRenderer::load(&theme.fonts);
            let m = panel::margin(&theme);
            (sheet(&theme, &text), m.left * scale, m.top * scale)
        })
        .collect();
    let row_h = glass.iter().map(|p| p.0.height as i32).max().unwrap_or(0) + gap;
    let w =
        (striped.width as i32).max(glass.iter().map(|p| p.0.width as i32 + gap).sum::<i32>() + gap);
    let mut c = Canvas::new(w as u32, striped.height + row_h as u32);
    c.fill_rect(c.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
    c.blit_over(0, 0, &striped);
    let mut x = gap;
    for (p, left, top) in &glass {
        c.blit_over(x - left, striped.height as i32 - top, p);
        x += p.width as i32 + gap;
    }
    c
}
