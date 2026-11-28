//! The breadcrumb address in each state on the frame's glass, Glass light and dark, written to
//! `target/reprisekit-gallery-breadcrumb@<scale>.png`.

use crate::interfaces;
use reprisekit::controls::breadcrumb::{self, Breadcrumb, Crumb, Hit, Part};
use reprisekit::interface::Interface;
use reprisekit::look;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::textedit::Caret;
use reprisekit::theme::{Color, FontConfig, Theme};

const FIELD_W: i32 = 420;
const ROW_H: i32 = 30;
const LABEL_W: i32 = 110;

fn place() -> Breadcrumb {
    let mut b = Breadcrumb::new();
    let crumbs = ["Computer", "Local Disk (/)", "home", "Music"]
        .iter()
        .scan(String::new(), |path, label| {
            path.push('/');
            path.push_str(label);
            Some(Crumb {
                label: label.to_string(),
                target: path.clone(),
            })
        })
        .collect();
    let mut icon = Canvas::new(16, 16);
    icon.fill_rect(Rect::new(1, 3, 14, 11), Color::rgb(0xE8, 0xC4, 0x5A));
    b.set_place(crumbs, Some(icon), "/home/u/Music");
    b
}

fn states() -> Vec<(&'static str, Breadcrumb, i32)> {
    let with = |f: &dyn Fn(&mut Breadcrumb)| {
        let mut b = place();
        f(&mut b);
        b
    };
    vec![
        ("rest", place(), FIELD_W),
        ("field hot", with(&|b| b.field_hot = true), FIELD_W),
        (
            "segment hot",
            with(&|b| {
                b.hover(Hit::Segment(2));
            }),
            FIELD_W,
        ),
        (
            "segment pressed",
            with(&|b| b.pressed = Some(Part::Segment(2))),
            FIELD_W,
        ),
        (
            "chevron hot",
            with(&|b| {
                b.hover(Hit::Chevron(2));
            }),
            FIELD_W,
        ),
        (
            "chevron open",
            with(&|b| b.open_menu = Some(Part::Chevron(2))),
            FIELD_W,
        ),
        ("overflow", place(), 190),
        (
            "overflow hot",
            with(&|b| {
                b.hover(Hit::Overflow);
            }),
            190,
        ),
        (
            "history hot",
            with(&|b| {
                b.hover(Hit::History);
            }),
            FIELD_W,
        ),
        (
            "refresh hot",
            with(&|b| {
                b.hover(Hit::Refresh);
            }),
            FIELD_W,
        ),
        (
            "refresh pressed",
            with(&|b| b.pressed = Some(Part::Refresh)),
            FIELD_W,
        ),
        ("editing", with(&Breadcrumb::start_edit), FIELD_W),
    ]
}

fn looks() -> Vec<(String, Theme)> {
    [
        reprisekit::glass::Mode::Light,
        reprisekit::glass::Mode::Dark,
    ]
    .into_iter()
    .map(|mode| {
        let g = reprisekit::glass::Settings {
            mode,
            ..Default::default()
        };
        let t = interfaces::theme_with(Interface::Glass, Default::default(), false, &g);
        (format!("Glass {mode:?}"), t)
    })
    .collect()
}

pub fn render(s: i32) -> Canvas {
    let looks = looks();
    let states = states();
    let col_w = LABEL_W + FIELD_W + 20;
    let (w, h) = (looks.len() as i32 * col_w, 20 + states.len() as i32 * ROW_H);
    let mut c = super::blurred_stripes(w * s, h * s);
    let head = TextRenderer::load(&FontConfig::default());
    let white = Color::rgb(0xFF, 0xFF, 0xFF);
    for (i, (name, base)) in looks.iter().enumerate() {
        let x0 = i as i32 * col_w;
        let r = Rect::new(x0 + 4, 2, col_w - 8, 16).scaled(s);
        head.draw(&mut c, r, name, true, white, Align::Left, 11 * s as u32);
        let one = TextRenderer::load(&base.fonts);
        let theme = base.scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        let cm = base.controls();
        let field_h = breadcrumb::height(base.text);
        for (j, (state, b, width)) in states.iter().enumerate() {
            let y = 20 + j as i32 * ROW_H;
            let label = Rect::new(x0 + 4, y, LABEL_W - 8, field_h).scaled(s);
            head.draw(
                &mut c,
                label,
                state,
                false,
                white,
                Align::Left,
                11 * s as u32,
            );
            let band = Rect::new(x0 + LABEL_W, y - 3, FIELD_W + 8, ROW_H);
            look::draw_client_glass(&mut c, &theme, band.scaled(s), band.height * s, false);
            let field = Rect::new(x0 + LABEL_W + 4, y, *width, field_h);
            let l = breadcrumb::layout(b, field, &one, base.fonts.size_px, &cm);
            let caret = Caret::new(b.editing(), true);
            let size = theme.fonts.size_px;
            breadcrumb::draw(&mut c, &text, &p, b, &l, b.editing(), caret, size);
        }
    }
    c
}
