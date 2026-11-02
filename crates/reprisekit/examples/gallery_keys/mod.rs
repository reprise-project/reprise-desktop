//! The launcher finding open windows, in Classic, Blue and Glass, written to
//! `target/reprisekit-gallery-launcher-windows@<scale>.png`.

use reprisekit::controls::launcher::{self, Key, Launcher, Row};
use reprisekit::interface::Interface;
use reprisekit::interfaces::theme_for;
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
