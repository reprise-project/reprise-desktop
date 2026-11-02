//! A path box completing `~/Do` in each interface, the second match lit, written to
//! `target/reprisekit-gallery-path-complete@<scale>.png`.

use reprisekit::controls::pathcomplete::{self, Entry, Key, PathComplete, Scope};
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::look::widgets::{Interaction, draw_edit_field, draw_edit_text};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::{Caret, EditKey, TextEdit};
use reprisekit::theme::FontConfig;

const W: i32 = 240;
const H: i32 = 130;

pub fn render(scale: i32) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let looks = [Interface::Classic, Interface::Blue, Interface::Glass];
    let mut c = Canvas::new((W * looks.len() as i32 * scale) as u32, (H * scale) as u32);
    for (i, look) in looks.into_iter().enumerate() {
        let theme = interfaces::theme_for(look).scaled(scale);
        let mut cell = Canvas::new((W * scale) as u32, (H * scale) as u32);
        cell.fill_rect(cell.bounds(), theme.colors.button_face);
        let (mut pc, edit) = completing("~/Do");
        let p = theme.paint();
        let px = theme.fonts.size_px;
        let field = Rect::new(10, 10, 220, 21);
        let inner = draw_edit_field(
            &mut cell,
            &p,
            field.scaled(scale),
            theme.colors.window,
            true,
            Interaction::Idle,
        );
        draw_edit_text(&mut cell, &text, &edit, inner, px, &p, Caret::Shown, 0);
        pc.nav(&mut edit.clone(), Key::Down);
        pc.nav(&mut edit.clone(), Key::Down);
        let lay = pathcomplete::layout(&pc, field, None, &theme.controls());
        pathcomplete::draw(&pc, &mut cell, &text, &p, &lay, px);
        c.blit_canvas(i as i32 * W * scale, 0, &cell);
    }
    c
}

fn completing(typed: &str) -> (PathComplete, TextEdit) {
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", true);
    for ch in typed.chars() {
        pc.edit(&mut edit, EditKey::Insert(ch.to_string()));
    }
    let names = [
        ("Documents", true),
        ("Downloads", true),
        ("Docs.txt", false),
        ("Dockerfile", false),
    ];
    if let Some(want) = pc.take_want() {
        let entries = names
            .iter()
            .map(|&(name, dir)| Entry {
                name: name.into(),
                dir,
            })
            .collect();
        pc.set_listing(&mut edit, &want, entries);
    }
    (pc, edit)
}
