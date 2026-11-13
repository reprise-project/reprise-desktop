use super::{blurred_stripes, cued};
use reprisekit::classic::frame::{Chrome, FrameGeometry, FrameState, Glow};
use reprisekit::controls::taskpreview;
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::look;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::{Color, FontConfig, Theme};

/// The snap layouts panel in Classic, Blue and Glass over a 1920x1050 work area, the display switcher (Extend lit, Only 1 hot, Only 2 greyed), then snap assist in the right half of a half-screen desktop. Glass's panels lie on mid-grey so the translucent sheet shows.
pub fn render_snap_layouts() -> Canvas {
    use reprisekit::controls::projector::{Projector, ProjectorLayout};
    use reprisekit::overview::{self, OverviewLayout};
    use reprisekit::projection::Projection;
    use reprisekit::snaplayouts::{CellSpec, PickerLayout, Shape};
    let (w, h) = (920, 696);
    let mut c = Canvas::new(3 * w as u32, h as u32);
    let text = TextRenderer::load(&FontConfig::default());
    let cols = |cuts: &[i32]| -> Vec<Rect> {
        cuts.windows(2)
            .map(|p| Rect::new(p[0], 0, p[1] - p[0], 1050))
            .collect()
    };
    let spec = |zones| CellSpec {
        zones,
        ..CellSpec::default()
    };
    let fixed = vec![
        spec(cols(&[0, 960, 1920])),
        spec(cols(&[0, 1280, 1920])),
        spec(cols(&[0, 640, 1280, 1920])),
        spec(vec![
            Rect::new(0, 0, 960, 1050),
            Rect::new(960, 0, 960, 525),
            Rect::new(960, 525, 960, 525),
        ]),
        spec(vec![
            Rect::new(0, 0, 960, 525),
            Rect::new(0, 525, 960, 525),
            Rect::new(960, 0, 960, 525),
            Rect::new(960, 525, 960, 525),
        ]),
        spec(cols(&[0, 480, 1440, 1920])),
    ];
    let suggestion = CellSpec {
        zones: cols(&[0, 960, 1920]),
        suggestion: true,
        filled: vec![(0, "Untitled - Notepad".into())],
    };
    let bar_order = [0, 1, 3, 4, 2, 5].map(|i| fixed[i].clone());
    let cells: Vec<CellSpec> = [vec![suggestion.clone()], fixed].concat();
    let bar_cells: Vec<CellSpec> = [vec![suggestion], bar_order.to_vec()].concat();
    let mut app = Canvas::new(16, 16);
    app.fill_rect(app.bounds(), Color::rgb(0x31, 0x6A, 0xC5));
    for (k, i) in [Interface::Classic, Interface::Blue, Interface::Glass]
        .into_iter()
        .enumerate()
    {
        let theme = cued(interfaces::theme_for(i));
        let x0 = k as i32 * w;
        c.fill_rect(Rect::new(x0, 0, w, h), theme.colors.background);
        let grid = PickerLayout::compute(Shape::Grid, (1920, 1050), &cells);
        let row = PickerLayout::compute(Shape::Row, (1920, 1050), &bar_cells);
        let row_top = 12 + grid.height + 12;
        let switcher = ProjectorLayout::compute();
        let switcher_top = row_top + row.height + 12;
        if i == Interface::Glass {
            let band = Rect::new(x0, 0, w, switcher_top + switcher.height + 12);
            c.fill_rect(band, Color::rgb(0x80, 0x80, 0x80));
        }
        let panel = interfaces::draw_snap_picker(&theme, &grid, &[((0, 0), &app)], Some((0, 1)));
        c.blit_over(x0 + (w - grid.width) / 2, 12, &panel);
        let bar = interfaces::draw_snap_picker(&theme, &row, &[((0, 0), &app)], Some((3, 1)));
        c.blit_over(x0 + (w - row.width) / 2, row_top, &bar);
        let p = Projector {
            hot: Some(2),
            enabled: [true, true, true, false],
            ..Projector::new(Projection::Extend, 2)
        };
        let sw = interfaces::draw_projector(&theme, &switcher, &text, &p);
        c.blit_over(x0 + (w - switcher.width) / 2, switcher_top, &sw);
        let top = switcher_top + switcher.height + 16;
        let desk = Rect::new(x0, top, w, h - top);
        let left = Rect::new(desk.x, desk.y, w / 2, desk.height);
        c.fill_rect(left, theme.colors.window);
        c.fill_rect(
            Rect::new(left.x, left.y, left.width, 18),
            theme.colors.active_title,
        );
        let zone = Rect::new(desk.x + w / 2, desk.y, w / 2, desk.height);
        let ol =
            OverviewLayout::compute_assist(2, Rect::new(0, 0, zone.width, zone.height), (w, h));
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(icon.bounds(), Color::rgb(0x31, 0x6A, 0xC5));
        let titles = ["Untitled - Notepad", "Terminal"];
        let pictures: Vec<Option<Rect>> = (0..2)
            .map(|n| Some(taskpreview::fit(800, 600, ol.well(n).unwrap(), 1)))
            .collect();
        let cells: Vec<overview::Cell> = (0..2)
            .map(|n| overview::Cell {
                title: titles[n],
                icon: Some(&icon),
                picture: pictures[n],
                stand_in: None,
            })
            .collect();
        let mut layer = Canvas::new(zone.width as u32, zone.height as u32);
        interfaces::draw_snap_assist(&mut layer, &theme, &text, &ol, &cells, None, Some(1));
        for r in pictures.iter().flatten() {
            layer.fill_rect(*r, Color::rgb(0xFF, 0xFF, 0xFF));
        }
        c.blit_over(zone.x, zone.y, &layer);
    }
    c
}

/// The window overview over a dimmed desktop in each look: five windows, the second selected, the fourth hot, the third without a picture.
pub fn render_overview() -> Canvas {
    use reprisekit::overview::{self, OverviewLayout};
    let (w, h) = (960, 540);
    let mut c = Canvas::new(3 * w as u32, h as u32);
    let text = TextRenderer::load(&FontConfig::default());
    for (k, i) in [Interface::Classic, Interface::Blue, Interface::Glass]
        .into_iter()
        .enumerate()
    {
        let theme = cued(interfaces::theme_for(i));
        let x0 = k as i32 * w;
        c.fill_rect(Rect::new(x0, 0, w, h), theme.colors.background);
        let mut dim = Canvas::new(w as u32, h as u32);
        dim.fill_rect(dim.bounds(), Color::rgb(0, 0, 0));
        c.blit_over_faded(x0, 0, &dim, 153, 255);
        let l = OverviewLayout::compute(5, Rect::new(0, 0, w, h));
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(icon.bounds(), Color::rgb(0x31, 0x6A, 0xC5));
        let titles = [
            "Terminal",
            "Untitled - Notepad",
            "My Documents",
            "Calculator",
            "Run",
        ];
        let pictures: Vec<Option<Rect>> = (0..5)
            .map(|n| (n != 2).then(|| taskpreview::fit(800, 600, l.well(n).unwrap(), 1)))
            .collect();
        let cells: Vec<overview::Cell> = (0..5)
            .map(|n| overview::Cell {
                title: titles[n],
                icon: Some(&icon),
                picture: pictures[n],
                stand_in: None,
            })
            .collect();
        let mut layer = Canvas::new(w as u32, h as u32);
        interfaces::draw_overview(&mut layer, &theme, &text, &l, &cells, 1, Some(3));
        for r in pictures.iter().flatten() {
            layer.fill_rect(*r, Color::rgb(0xFF, 0xFF, 0xFF));
            layer.fill_rect(
                Rect::new(r.x, r.y, r.width, 8),
                Color::rgb(0xDD, 0xE6, 0xF2),
            );
        }
        c.blit_over(x0, 0, &layer);
    }
    c
}

/// Tab groups' captions in Classic, navy and silver Blue, and light and dark Glass over the stripes; the last row is Classic at scale 2.
pub fn render_caption_tabs(pct: u32) -> Canvas {
    use reprisekit::controls::captiontabs::{self, CaptionTabs, TabEntry, TabHit};
    let dark = reprisekit::glass::Settings {
        mode: reprisekit::glass::Mode::Dark,
        ..Default::default()
    };
    let themes = [
        (cued(Theme::default()), 1),
        (cued(interfaces::theme_for(Interface::Blue)), 1),
        (
            cued(interfaces::blue_theme(reprisekit::blue::Variant::Silver)),
            1,
        ),
        (cued(interfaces::theme_for(Interface::Glass)), 1),
        (
            cued(interfaces::theme_with(
                Interface::Glass,
                reprisekit::blue::Variant::Blue,
                &dark,
            )),
            1,
        ),
        (cued(Theme::default()), 2),
    ];
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = blurred_stripes(950, 72 * themes.len() as i32 + 40);
    let mut y = 10;
    for (base, s) in &themes {
        let base = &base.with_text_size(pct);
        let theme = base.scaled(*s);
        let caption_text = match theme.look.style() {
            Some(st) => TextRenderer::load(&st.caption_text.font()),
            None => TextRenderer::load(&FontConfig::default()),
        };
        let icon = reprisekit::icon::fallback(&theme, theme.icon_size());
        let entry = |title| TabEntry {
            title,
            icon: Some(&icon),
        };
        let four = CaptionTabs {
            tabs: vec![
                entry("notes.txt - Notepad"),
                entry("user@host: ~"),
                entry("Downloads"),
                entry("report.odt"),
            ],
            selected: 1,
            hot: Some(TabHit {
                tab: 2,
                close: true,
            }),
            held: None,
            ghost: None,
        };
        let ghost = CaptionTabs {
            tabs: vec![
                entry("Calculator"),
                entry("Untitled - Paint"),
                entry("Notes"),
            ],
            selected: 0,
            hot: None,
            held: None,
            ghost: Some(2),
        };
        let narrow = CaptionTabs {
            selected: 0,
            hot: None,
            ..four.clone()
        };
        let rows: Vec<(i32, i32, bool, &CaptionTabs)> = if *s == 1 {
            vec![
                (10, 520, true, &four),
                (560, 250, false, &ghost),
                (840, 90, true, &narrow),
            ]
        } else {
            vec![(10, 450, true, &four)]
        };
        let mut h = 0;
        for (x, cw, active, tabs) in rows {
            let g = FrameGeometry::layout(&base.metrics, cw, 20, false, Chrome::WINDOW).scaled(*s);
            let st = FrameState {
                title: "",
                active,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: true,
            };
            let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            look::draw_frame(&mut f, &theme, &text, &caption_text, &g, &st);
            captiontabs::draw(&mut f, &theme, &text, &caption_text, &g, tabs, active);
            f.fill_rect(g.client, theme.colors.button_face);
            c.blit_over(x, y, &f);
            h = h.max(g.total.1);
        }
        y += h + 10;
    }
    c
}
