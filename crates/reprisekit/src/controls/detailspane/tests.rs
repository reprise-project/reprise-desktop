use super::*;
use crate::interface::Interface;
use crate::interfaces::{theme_for, theme_with};
use crate::theme::{Color, FontConfig, Theme};

const PANE: Rect = Rect::new(0, 0, 640, HEIGHT);

fn file() -> Facts {
    Facts {
        icon: None,
        title: "notes.txt".into(),
        subtitle: "Text Document".into(),
        rows: vec![
            ("Date modified:".into(), "11/28/2026 9:41 AM".into()),
            ("Size:".into(), "4.2 KB".into()),
            ("Date created:".into(), "11/02/2026 3:15 PM".into()),
            ("Dimensions:".into(), "640 x 480".into()),
        ],
    }
}

fn pane(facts: Facts) -> DetailsPane {
    DetailsPane { facts }
}

fn text() -> TextRenderer {
    TextRenderer::load(&FontConfig::default())
}

fn glass_cm() -> ControlMetrics {
    theme_for(Interface::Glass).controls()
}

#[test]
fn facts_flow_down_columns_with_names_right_aligned() {
    let lay = layout(&pane(file()), &text(), 12, PANE, &glass_cm());
    assert_eq!((lay.icon.width, lay.icon.height), (ICON, ICON));
    assert_eq!(
        lay.icon.y,
        (HEIGHT - ICON) / 2,
        "the icon centred down the pane"
    );
    assert_eq!(lay.rows.len(), 4);
    let (a, b, d) = (lay.rows[0], lay.rows[1], lay.rows[3]);
    assert_eq!(a.name.right(), b.name.right(), "names end together");
    assert_eq!(a.value.x, b.value.x, "values start together");
    assert_eq!(b.value.y - a.value.y, LINE_H);
    assert!(a.name.x > lay.title.right(), "past the title's column");
    assert_eq!(d.name.y, a.name.y, "the fourth fact opens a second column");
    assert!(d.name.x > a.value.right());
}

#[test]
fn a_column_that_would_pass_the_right_edge_is_left_out() {
    let narrow = Rect::new(0, 0, 330, HEIGHT);
    let lay = layout(&pane(file()), &text(), 12, narrow, &glass_cm());
    assert!(lay.rows.len() < 4, "{} facts", lay.rows.len());
    assert!(lay.rows.iter().all(|r| r.value.right() <= narrow.right()));
}

#[test]
fn nothing_but_a_title_lays_no_subtitle_or_facts() {
    let f = Facts {
        title: "11 items".into(),
        ..Facts::default()
    };
    let lay = layout(&pane(f), &text(), 12, PANE, &glass_cm());
    assert_eq!(lay.subtitle, None);
    assert!(lay.rows.is_empty());
}

#[test]
fn hit_finds_the_icon_the_title_and_each_fact() {
    let lay = layout(&pane(file()), &text(), 12, PANE, &glass_cm());
    let at = |r: Rect| hit(&lay, r.x + 1, r.y + 1);
    assert_eq!(at(lay.icon), Hit::Icon);
    assert_eq!(at(lay.title), Hit::Title);
    assert_eq!(at(lay.rows[1].value), Hit::Row(1));
    assert_eq!(hit(&lay, PANE.right() - 2, PANE.y + 2), Hit::Pane);
    assert_eq!(hit(&lay, PANE.right() + 2, PANE.y + 2), Hit::Nothing);
}

#[test]
fn set_reports_a_change_only_when_the_facts_differ() {
    let mut dp = DetailsPane::default();
    assert!(dp.set(file()));
    assert!(!dp.set(file()));
}

#[test]
fn the_pane_grows_with_the_text() {
    let one = theme_for(Interface::Glass);
    let two = one.with_text_size(200);
    assert_eq!(height(one.text), HEIGHT);
    assert_eq!(height(two.text), 2 * HEIGHT);
}

#[test]
fn access_names_the_pane_and_reads_each_fact() {
    let dp = pane(file());
    let lay = layout(&dp, &text(), 12, PANE, &glass_cm());
    let mut t = Tree::window("Explorer", (700, 100));
    let id = access(&dp, &lay, &mut t, access::ROOT);
    assert_eq!(t.get(id).and_then(|n| n.label()), Some("Details pane"));
    let values: Vec<String> = (t.nodes())
        .filter(|(_, n)| n.role() == Role::Label)
        .filter_map(|(_, n)| n.value().map(str::to_string))
        .collect();
    assert!(values.contains(&"notes.txt".to_string()), "{values:?}");
    assert!(values.contains(&"Size: 4.2 KB".to_string()), "{values:?}");
}

fn looks() -> Vec<(String, Theme)> {
    let mut out = vec![("Classic".to_string(), Theme::default())];
    for mode in crate::glass::Mode::ALL {
        let g = crate::glass::Settings {
            mode,
            ..Default::default()
        };
        let t = theme_with(Interface::Glass, Default::default(), false, &g);
        out.push((format!("Glass {mode:?}"), t));
    }
    out
}

/// A stand-in 48 px icon at scale `s`: a blue square inset 8 px.
fn icon(s: i32) -> Canvas {
    let mut c = Canvas::new((ICON * s) as u32, (ICON * s) as u32);
    c.fill_rect(
        Rect::new(8 * s, 8 * s, 32 * s, 32 * s),
        Color::rgb(0x40, 0x80, 0xC0),
    );
    c
}

/// `facts` laid out at 100% and drawn at `theme`'s scale `s`, its icon drawn for that scale.
fn render(theme: &Theme, facts: Facts, s: i32) -> Canvas {
    let one = TextRenderer::load(&theme.fonts);
    let mut dp = pane(facts);
    let lay = layout(&dp, &one, theme.fonts.size_px, PANE, &theme.controls());
    if dp.facts.icon.is_some() {
        dp.facts.icon = Some(icon(s));
    }
    let theme = theme.scaled(s);
    let text = TextRenderer::load(&theme.fonts);
    let mut c = Canvas::new((PANE.width * s) as u32, (PANE.height * s) as u32);
    draw(
        &mut c,
        &text,
        &theme.paint(),
        &dp,
        &lay,
        theme.fonts.size_px,
    );
    c
}

fn scenes() -> Vec<(&'static str, Facts)> {
    vec![
        (
            "folder",
            Facts {
                icon: Some(icon(1)),
                title: "Documents".into(),
                subtitle: "11 items".into(),
                rows: vec![("Space free:".into(), "120 GB".into())],
            },
        ),
        (
            "file",
            Facts {
                icon: Some(icon(1)),
                ..file()
            },
        ),
        (
            "several",
            Facts {
                icon: Some(icon(1)),
                title: "3 items selected".into(),
                subtitle: String::new(),
                rows: vec![("Size:".into(), "12.6 KB".into())],
            },
        ),
    ]
}

#[test]
fn details_pane_pins_per_state_at_scale_1_and_2() {
    crate::pins::fonts();
    let mut got = Vec::new();
    for (name, theme) in looks() {
        for (state, facts) in scenes() {
            for s in [1, 2] {
                let h = crate::pins::fnv(&render(&theme, facts.clone(), s));
                got.push((format!("{name} {state} {s}"), h));
            }
        }
    }
    crate::pins::check("controls/detailspane", &got);
}

/// Glass's backdrop: a pale blue line a design px tall, shading down into the pale face; dark
/// Glass's darker than the light one's throughout.
#[test]
fn the_glass_backdrop_opens_with_a_line_and_shades_into_its_face() {
    let looks = looks();
    for s in [1, 2] {
        let light = render(&looks[1].1, Facts::default(), s);
        let dark = render(&looks[2].1, Facts::default(), s);
        let x = 600 * s;
        let row = |c: &Canvas, y: i32| c.get(x, y) & 0xFF_FFFF;
        assert_eq!(row(&light, 0), 0xCC_D9EA, "scale {s}: the line");
        assert_eq!(
            row(&light, s - 1),
            0xCC_D9EA,
            "scale {s}: the line is a design px"
        );
        assert_eq!(
            row(&light, (HEIGHT - 1) * s),
            0xF1_F5FB,
            "scale {s}: the face"
        );
        let lum = |v: u32| (v >> 16 & 0xFF) + (v >> 8 & 0xFF) + (v & 0xFF);
        assert!(
            lum(row(&light, s)) < lum(row(&light, 3 * s)),
            "scale {s}: shading lighter"
        );
        for y in [0, 2 * s, (HEIGHT - 1) * s] {
            assert!(
                lum(row(&dark, y)) < lum(row(&light, y)),
                "scale {s}: dark at {y}"
            );
        }
    }
}
