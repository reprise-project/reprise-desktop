//! A raster editor's tool box and colour box in Classic, Blue and Glass, written to
//! `target/reprisekit-gallery-paint@<scale>.png`.

use reprisekit::controls::palette::{self, Palette};
use reprisekit::controls::toolbox::{self, Choice, Tool, ToolBox};
use reprisekit::interface::Interface;
use reprisekit::interfaces::theme_for;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::shape::{self, ellipse, polygon, rounded, stroke};
use reprisekit::theme::{Color, Theme};

const GAP: i32 = 12;
const STRIP_W: i32 = 260;

/// A 16x16 white mask painted at `d` from `cover`, which takes a device pixel and `d`.
fn mask(d: i32, cover: impl Fn(f64, f64, f64) -> f64) -> Canvas {
    let white = Color::rgb(0xFF, 0xFF, 0xFF);
    shape::draw(16, 16, d, |x, y| {
        let k = cover(x, y, f64::from(d)).clamp(0.0, 1.0);
        shape::premul(white, (k * 255.0).round() as u8)
    })
}

fn tools(d: i32) -> Vec<Tool> {
    let tool = |name: &str, glyph: Canvas| Tool {
        name: name.into(),
        glyph: Some(glyph),
    };
    vec![
        tool(
            "Pencil",
            mask(d, |x, y, k| {
                stroke(x, y, &[(3.0 * k, 13.0 * k), (13.0 * k, 3.0 * k)], 2.0 * k)
            }),
        ),
        tool(
            "Brush",
            mask(d, |x, y, k| {
                ellipse(x, y, 5.0 * k, 11.0 * k, 3.0 * k, 3.0 * k).max(stroke(
                    x,
                    y,
                    &[(7.0 * k, 9.0 * k), (13.0 * k, 3.0 * k)],
                    1.0 * k,
                ))
            }),
        ),
        tool(
            "Fill",
            mask(d, |x, y, k| {
                polygon(
                    x,
                    y,
                    &[
                        (8.0 * k, 2.0 * k),
                        (14.0 * k, 8.0 * k),
                        (8.0 * k, 14.0 * k),
                        (2.0 * k, 8.0 * k),
                    ],
                )
            }),
        ),
        tool(
            "Line",
            mask(d, |x, y, k| {
                stroke(x, y, &[(2.0 * k, 14.0 * k), (14.0 * k, 2.0 * k)], 1.0 * k)
            }),
        ),
        tool(
            "Rectangle",
            mask(d, |x, y, k| {
                let outer = rounded(x - 2.0 * k, y - 3.0 * k, 12.0 * k, 10.0 * k, 0.0);
                let inner = rounded(x - 3.0 * k, y - 4.0 * k, 10.0 * k, 8.0 * k, 0.0);
                outer - inner
            }),
        ),
        tool(
            "Ellipse",
            mask(d, |x, y, k| {
                ellipse(x, y, 8.0 * k, 8.0 * k, 6.0 * k, 5.0 * k)
                    - ellipse(x, y, 8.0 * k, 8.0 * k, 5.0 * k, 4.0 * k)
            }),
        ),
    ]
}

fn sizes(d: i32) -> Vec<Choice> {
    (1..=3)
        .map(|w| Choice {
            name: format!("{w} px"),
            glyph: Some(mask(d, move |x, y, k| {
                stroke(
                    x,
                    y,
                    &[(2.0 * k, 8.0 * k), (14.0 * k, 8.0 * k)],
                    f64::from(w) * k,
                )
            })),
        })
        .collect()
}

fn colours() -> Palette {
    let swatches = [
        (0x00, 0x00, 0x00, "Black"),
        (0x80, 0x80, 0x80, "Gray"),
        (0x80, 0x00, 0x00, "Maroon"),
        (0x80, 0x80, 0x00, "Olive"),
        (0x00, 0x80, 0x00, "Green"),
        (0x00, 0x80, 0x80, "Teal"),
        (0x00, 0x00, 0x80, "Navy"),
        (0x80, 0x00, 0x80, "Purple"),
        (0xFF, 0xFF, 0xFF, "White"),
        (0xC0, 0xC0, 0xC0, "Silver"),
        (0xFF, 0x00, 0x00, "Red"),
        (0xFF, 0xFF, 0x00, "Yellow"),
        (0x00, 0xFF, 0x00, "Lime"),
        (0x00, 0xFF, 0xFF, "Aqua"),
        (0x00, 0x00, 0xFF, "Blue"),
        (0xFF, 0x00, 0xFF, "Fuchsia"),
    ];
    Palette {
        swatches: swatches
            .into_iter()
            .map(|(r, g, b, n)| (Color::rgb(r, g, b), n.to_string()))
            .collect(),
        rows: 2,
        primary: Color::rgb(0, 0, 0x80),
        secondary: Color::rgb(0xFF, 0xFF, 0),
    }
}

/// Per look: the tool box with Brush current and Line hot, and the colour box beside it.
pub fn render(scale: i32) -> Canvas {
    let looks: [Theme; 3] = [
        Theme::default(),
        theme_for(Interface::Blue),
        theme_for(Interface::Glass),
    ];
    let tb = ToolBox {
        tools: tools(scale),
        current: 1,
        hot: Some(3),
        pressed: None,
        options: sizes(scale),
        option_cols: 1,
        option: Some(1),
    };
    let pal = colours();
    let box_h = toolbox::height(&tb);
    let cell_w = toolbox::WIDTH + GAP + STRIP_W;
    let mut c = Canvas::new(
        ((GAP + looks.len() as i32 * (cell_w + GAP)) * scale) as u32,
        ((box_h + 2 * GAP) * scale) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(0x60, 0x60, 0x60));
    for (i, theme) in looks.iter().enumerate() {
        let theme = theme.scaled(scale);
        let p = theme.paint();
        let x = GAP + i as i32 * (cell_w + GAP);
        let lay = toolbox::layout(&tb, Rect::new(x, GAP, toolbox::WIDTH, box_h));
        toolbox::draw(&mut c, &p, &tb, &lay);
        let strip = Rect::new(x + toolbox::WIDTH + GAP, GAP, STRIP_W, palette::HEIGHT);
        let lay = palette::layout(&pal, strip);
        palette::draw(&mut c, &p, &pal, &lay);
    }
    c
}
