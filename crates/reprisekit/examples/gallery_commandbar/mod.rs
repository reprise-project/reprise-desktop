//! The command bar in Glass light and dark, at rest, with a command hot, held and its menu open,
//! a split command's drop hot, Views hot, the keyboard on a command, and narrow enough that
//! commands fold into », written to `target/reprisekit-gallery-command-bar@<scale>.png`; and its
//! menus' rows with their glyphs, to `target/reprisekit-gallery-command-menus@<scale>.png`.

use crate::interfaces;
use reprisekit::art::cmdglyph::{self, CmdGlyph};
use reprisekit::controls::commandbar::{self, Commandbar, Drop, Hit, Item, RightItem};
use reprisekit::controls::menu::{ContextBitmaps, MenuItem, MenuLayout, MenuStyle, draw_menu};
use reprisekit::interface::Interface;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::theme::{Color, FontConfig, Theme};

const BAR_W: i32 = 560;
const NARROW_W: i32 = 260;
const LABEL_W: i32 = 110;

fn sample() -> Commandbar {
    Commandbar {
        left: vec![
            Item::new("Organize", 0).drop(Drop::Menu),
            Item::new("Open", 1).drop(Drop::Split),
            Item::new("Extract all files", 2),
            Item::new("New folder", 3).drop(Drop::Split),
            Item::new("Eject", 4).enabled(false),
        ],
        right: vec![RightItem::Views, RightItem::Help],
        ..Commandbar::default()
    }
}

fn states() -> Vec<(&'static str, Commandbar, i32)> {
    let with = |f: &dyn Fn(&mut Commandbar)| {
        let mut b = sample();
        f(&mut b);
        b
    };
    vec![
        ("rest", sample(), BAR_W),
        ("Organize hot", with(&|b| b.hot = Some(Hit::Item(0))), BAR_W),
        (
            "Organize open",
            with(&|b| b.pressed = Some(Hit::Item(0))),
            BAR_W,
        ),
        (
            "Open drop hot",
            with(&|b| b.hot = Some(Hit::ItemDrop(1))),
            BAR_W,
        ),
        (
            "Open held",
            with(&|b| b.pressed = Some(Hit::Item(1))),
            BAR_W,
        ),
        ("Views hot", with(&|b| b.hot = Some(Hit::ViewsCycle)), BAR_W),
        (
            "Views list",
            with(&|b| b.pressed = Some(Hit::ViewsDrop)),
            BAR_W,
        ),
        ("keyboard", with(&|b| b.focus = Some(Hit::Item(2))), BAR_W),
        ("folded", sample(), NARROW_W),
        ("» hot", with(&|b| b.hot = Some(Hit::More)), NARROW_W),
    ]
}

fn looks() -> Vec<(String, Theme)> {
    reprisekit::glass::Mode::ALL
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
    let row_h = looks
        .iter()
        .map(|l| commandbar::height(&l.1.controls()))
        .max()
        .unwrap_or(commandbar::HEIGHT)
        + 8;
    let col_w = LABEL_W + BAR_W + 16;
    let (w, h) = (looks.len() as i32 * col_w, 20 + states.len() as i32 * row_h);
    let mut c = super::blurred_stripes(w * s, h * s);
    let head = TextRenderer::load(&FontConfig::default());
    let white = Color::rgb(0xFF, 0xFF, 0xFF);
    for (j, (name, base)) in looks.iter().enumerate() {
        let x0 = j as i32 * col_w;
        let r = Rect::new(x0 + LABEL_W, 2, BAR_W, 16).scaled(s);
        head.draw(&mut c, r, name, true, white, Align::Left, 11 * s as u32);
        let one = TextRenderer::load(&base.fonts);
        let theme = base.scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        for (i, (label, bar, bar_w)) in states.iter().enumerate() {
            let y = 20 + i as i32 * row_h;
            let r = Rect::new(x0 + 4, y + 6, LABEL_W - 8, 16).scaled(s);
            head.draw(&mut c, r, label, true, white, Align::Left, 11 * s as u32);
            let cm = base.controls();
            let area = Rect::new(x0 + LABEL_W, y, *bar_w, commandbar::height(&cm));
            let l = commandbar::layout(bar, area, &one, base.fonts.size_px, &cm);
            commandbar::draw(&mut c, &text, &p, bar, &l, theme.fonts.size_px);
        }
    }
    c
}

/// Organize's rows and the Views drop as the command bar opens them, the first row lit, in each
/// look side by side.
pub fn render_menus(s: i32) -> Canvas {
    let glyph = |g| Some(cmdglyph::canvas(g, 16, s).at_scale(1));
    let organize = vec![
        MenuItem::entry("Cut").icon(glyph(CmdGlyph::Cut)),
        MenuItem::entry("Copy").icon(glyph(CmdGlyph::Copy)),
        MenuItem::entry("Paste")
            .icon(glyph(CmdGlyph::Paste))
            .enabled(false),
        MenuItem::Separator,
        MenuItem::entry("Layout")
            .icon(glyph(CmdGlyph::Layout))
            .submenu(true),
        MenuItem::entry("Details pane").checked(true),
        MenuItem::entry("Preview pane").icon(glyph(CmdGlyph::PreviewPane)),
        MenuItem::entry("Navigation pane").icon(glyph(CmdGlyph::NavPane)),
        MenuItem::Separator,
        MenuItem::entry("Delete").icon(glyph(CmdGlyph::Delete)),
    ];
    let views = [
        ("Thumbnails", CmdGlyph::Thumbnails),
        ("Tiles", CmdGlyph::Tiles),
        ("Icons", CmdGlyph::Icons),
        ("List", CmdGlyph::List),
        ("Details", CmdGlyph::Details),
    ]
    .map(|(label, g)| {
        MenuItem::entry(label)
            .icon(glyph(g))
            .radio(true)
            .checked(g == CmdGlyph::Details)
    })
    .to_vec();
    let looks = looks();
    let col_w = 420;
    let (w, h) = (looks.len() as i32 * col_w, 300);
    let mut c = super::blurred_stripes(w * s, h * s);
    let head = TextRenderer::load(&FontConfig::default());
    let white = Color::rgb(0xFF, 0xFF, 0xFF);
    for (j, (name, base)) in looks.iter().enumerate() {
        let x0 = j as i32 * col_w + 8;
        let r = Rect::new(x0, 2, col_w, 16).scaled(s);
        head.draw(&mut c, r, name, true, white, Align::Left, 11 * s as u32);
        let theme = base.scaled(s);
        let text = TextRenderer::load(&base.fonts);
        let style = MenuStyle {
            width: 180,
            ..MenuStyle::context(base, ContextBitmaps::Icons)
        };
        let mut x = x0;
        for items in [&organize, &views] {
            let l = MenuLayout::compute_styled(&style, items, usize::MAX).scaled(s);
            let mut m = Canvas::new(l.width as u32, l.height as u32);
            draw_menu(&mut m, &theme, &text, &l, items, Some(0), "", false);
            c.blit_over(x * s, 24 * s, &m);
            x += l.width / s + 16;
        }
    }
    c
}
