use super::*;
use crate::interface::Interface;
use crate::interfaces::theme_for;

const STAT: &str = "cpu  100 20 50 800 30 0 0 0 0 0\ncpu0 50 10 25 400 15 0 0 0 0 0\n";
const STAT_LATER: &str = "cpu  160 20 80 880 40 0 0 0 0 0\n";
const MEMINFO: &str =
    "MemTotal:       16000000 kB\nMemFree:         2000000 kB\nMemAvailable:    12000000 kB\n";

#[test]
fn the_clock_ticks_on_the_next_second_or_minute() {
    let at = Duration::from_millis(1_000_000_250);
    assert_eq!(next_tick(true, at), Duration::from_millis(750 + 5));
    // 1_000_000_250 ms is 40.25 s into its minute.
    assert_eq!(next_tick(false, at), Duration::from_millis(19_750 + 5));
    assert_eq!(
        next_tick(false, Duration::from_secs(60)),
        Duration::from_millis(60_005),
        "a minute's start waits the whole next minute"
    );
}

#[test]
fn a_clock_keeps_only_what_its_face_draws() {
    let a = Clock::new(Face::Analog, false, (9, 30, 15), "9:30", "Monday");
    assert_eq!(
        a,
        Clock::new(Face::Analog, false, (9, 30, 59), "9:31", "Tuesday")
    );
    assert_eq!(a.time, (9, 30, 0));
    let s = Clock::new(Face::Analog, true, (9, 30, 15), "", "");
    assert_ne!(s, Clock::new(Face::Analog, true, (9, 30, 16), "", ""));
    let d = Clock::new(Face::Digital, false, (9, 30, 15), "9:30", "Monday");
    assert_eq!(
        d,
        Clock::new(Face::Digital, false, (1, 2, 3), "9:30", "Monday")
    );
    assert_ne!(
        d,
        Clock::new(Face::Digital, false, (9, 30, 15), "9:31", "Monday")
    );
}

#[test]
fn cpu_share_is_the_busy_part_of_the_time_between_two_readings() {
    let before = CpuTimes::parse(STAT).unwrap();
    assert_eq!(
        before,
        CpuTimes {
            busy: 170,
            total: 1000
        }
    );
    let after = CpuTimes::parse(STAT_LATER).unwrap();
    // 90 more busy ticks and 90 more idle ones.
    assert_eq!(cpu_share(before, after), Some(0.5));
    assert_eq!(cpu_share(after, after), None, "no time passed");
    assert_eq!(cpu_share(after, before), None, "the counters went back");
    assert_eq!(CpuTimes::parse("intr 1 2 3\n"), None);
    assert_eq!(CpuTimes::parse("cpu  1 x 3 4\n"), None);
}

#[test]
fn memory_share_is_what_available_leaves_of_the_total() {
    assert_eq!(memory_share(MEMINFO), Some(0.25));
    assert_eq!(memory_share("MemTotal: 100 kB\n"), None);
}

#[test]
fn the_meter_keeps_a_minute_of_whole_percents_and_repeats_a_missing_sample() {
    let mut m = Meter::new(MeterStyle::Graph);
    m.push(None, Some(0.5));
    assert_eq!((m.cpu_now(), m.memory_now()), (None, Some(50)));
    m.push(Some(0.123), None);
    assert_eq!((m.cpu.clone(), m.memory.clone()), (vec![12], vec![50, 50]));
    for i in 0..100 {
        m.push(Some(f64::from(i) / 100.0), Some(1.5));
    }
    assert_eq!(m.cpu.len(), HISTORY);
    assert_eq!(m.memory.len(), HISTORY);
    assert_eq!((m.cpu_now(), m.memory_now()), (Some(99), Some(100)));
    assert_eq!(m.cpu[0], 40, "the oldest kept is a minute back");
}

#[test]
fn a_widget_spans_whole_cells() {
    let grid = Cell::square(75);
    assert_eq!(span(grid, (129, 129)), (2, 2));
    assert_eq!(span(grid, (150, 75)), (2, 1));
    assert_eq!(span(grid, (151, 10)), (3, 1));
    let spot = Spot {
        cell: (2, 1),
        span: (2, 2),
    };
    let area = Rect::new(0, 30, 800, 600);
    assert_eq!(spot.rect(grid, area), Rect::new(150, 105, 150, 150));
    assert_eq!(
        spot.panel(grid, area, (130, 140)),
        Rect::new(160, 110, 130, 140)
    );
    let cells: Vec<_> = spot.cells().collect();
    assert_eq!(cells, vec![(2, 1), (2, 2), (3, 1), (3, 2)]);
}

#[test]
fn placing_keeps_off_other_widgets_and_off_icons_where_it_can() {
    let dims = (6, 4);
    let none = HashSet::new();
    let clock = Spot {
        cell: (4, 0),
        span: (2, 2),
    };
    assert_eq!(place(dims, (2, 2), (4, 0), &[], &none), (4, 0));
    assert_eq!(
        place(dims, (3, 1), (4, 0), &[], &none),
        (3, 0),
        "pulled inside the grid's right edge"
    );
    assert_eq!(
        place(dims, (3, 1), (3, 1), &[clock], &none),
        (3, 2),
        "the nearest spot below the clock"
    );
    let icons = HashSet::from([(3, 2), (4, 2)]);
    assert_eq!(
        place(dims, (2, 1), (3, 2), &[clock], &icons),
        (3, 3),
        "the nearest spot with no icon under it"
    );
    let everywhere: HashSet<_> = (0..6).flat_map(|c| (0..4).map(move |r| (c, r))).collect();
    assert_eq!(
        place(dims, (2, 1), (3, 2), &[clock], &everywhere),
        (3, 2),
        "icons everywhere: they make way"
    );
    let full = [Spot {
        cell: (0, 0),
        span: (6, 4),
    }];
    assert_eq!(
        place(dims, (2, 2), (9, 9), &full, &none),
        (4, 2),
        "no room: kept in the grid"
    );
}

#[test]
fn a_press_on_the_panel_hits_and_one_beside_it_does_not() {
    assert!(hit((100, 50), 0, 0));
    assert!(hit((100, 50), 99, 49));
    assert!(!hit((100, 50), 100, 10));
    assert!(!hit((100, 50), -1, 10));
}

fn sample(kind: Kind, alt: bool) -> Content {
    match kind {
        Kind::Clock if alt => Content::Clock(Clock::new(
            Face::Digital,
            true,
            (15, 39, 7),
            "15:39:07",
            "Monday, November 2",
        )),
        Kind::Clock => Content::Clock(Clock::new(Face::Analog, true, (10, 9, 30), "", "")),
        Kind::Calendar => {
            let mut cal = Calendar::new(2026, 11, None, 1);
            cal.today = Some((2026, 11, 2));
            Content::Calendar(cal)
        }
        Kind::Meter => {
            let mut m = Meter::new(if alt {
                MeterStyle::Graph
            } else {
                MeterStyle::Bars
            });
            for i in 0..HISTORY {
                let wave = (i % 20) as f64 / 20.0;
                m.push(Some(wave), Some(0.42));
            }
            Content::Meter(m)
        }
    }
}

fn samples() -> Vec<(String, Content)> {
    Kind::ALL
        .into_iter()
        .flat_map(|k| [false, true].map(|alt| (k, alt)))
        .filter(|&(k, alt)| k != Kind::Calendar || !alt)
        .map(|(k, alt)| {
            (
                format!("{}{}", k.name(), if alt { " alt" } else { "" }),
                sample(k, alt),
            )
        })
        .collect()
}

fn drawn(i: Interface, scale: i32, content: &Content) -> (Canvas, Theme) {
    let theme = theme_for(i).scaled(scale);
    let text = TextRenderer::load(&theme.fonts);
    (draw(&theme, &text, content), theme)
}

#[test]
fn classic_and_blue_draw_a_raised_face_the_size_of_the_panel() {
    for i in [Interface::Classic, Interface::Blue] {
        for scale in [1, 2] {
            for (name, content) in samples() {
                let (c, theme) = drawn(i, scale, &content);
                let cs = &theme.colors;
                let (w, h) = size(&content, &theme.controls());
                let (w, h) = (w * scale, h * scale);
                assert_eq!(
                    (c.width as i32, c.height as i32),
                    (w, h),
                    "{i:?}@{scale} {name}"
                );
                assert_eq!(
                    c.get(w - 1, h - 1),
                    cs.button_dk_shadow.argb(),
                    "{i:?}@{scale} {name}"
                );
                assert_eq!(
                    c.get(w / 2, h - 3 * scale),
                    cs.button_face.argb(),
                    "{i:?}@{scale} {name}"
                );
            }
        }
    }
}

#[test]
fn under_glass_a_widget_is_translucent_glass_inside_its_shadow() {
    for scale in [1, 2] {
        for (name, content) in samples() {
            let (c, theme) = drawn(Interface::Glass, scale, &content);
            let m = panel::margin(&theme);
            let (w, h) = size(&content, &theme.controls());
            assert_eq!(
                (c.width as i32, c.height as i32),
                (
                    (w + m.left + m.right) * scale,
                    (h + m.top + m.bottom) * scale
                ),
                "@{scale} {name}"
            );
            assert_eq!(
                c.get(0, 0) >> 24,
                0,
                "@{scale} {name}: clear past the shadow"
            );
            let a = c.get((m.left + w / 2) * scale, (m.top + h - 3) * scale) >> 24;
            assert!(
                a > 0 && a < 0xFF,
                "@{scale} {name}: translucent glass, alpha {a:x}"
            );
        }
    }
}

#[test]
fn the_digital_face_writes_its_time_in_white_under_glass() {
    let content = sample(Kind::Clock, true);
    let (c, theme) = drawn(Interface::Glass, 1, &content);
    let m = panel::margin(&theme);
    let row = (m.top + PAD..m.top + PAD + TIME_H)
        .any(|y| (m.left..m.left + DIGITAL_W).any(|x| c.get(x, y) == 0xFFFF_FFFF));
    assert!(row, "white time");
}

#[test]
fn the_graph_draws_its_line_at_the_level_of_its_samples() {
    let theme = theme_for(Interface::Classic);
    let p = theme.paint();
    let cs = &theme.colors;
    let r = Rect::new(0, 0, 61, 22);
    let graph = |values: &[u8]| {
        let mut c = Canvas::new(61, 22);
        draw_graph(&mut c, &p, r, values, false);
        c
    };
    let ink = cs.hilight.argb();
    assert_eq!(
        graph(&[100; HISTORY]).get(30, 2),
        ink,
        "full along the well's top"
    );
    assert_eq!(graph(&[0; HISTORY]).get(30, 19), ink, "idle along its foot");
    assert_eq!(
        graph(&[]).get(30, 10),
        cs.window.argb(),
        "no samples: a blank well"
    );
}

/// FNV-1a over the pixels, for pins.
fn hash(c: &Canvas) -> u64 {
    c.pixels
        .iter()
        .flat_map(|p| p.to_le_bytes())
        .chain(c.width.to_le_bytes())
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        })
}

#[test]
fn each_widget_pins_in_each_look_at_scale_1_and_2() {
    crate::pins::fonts();
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for scale in [1, 2] {
            for (name, content) in samples() {
                got.push((
                    format!("{i:?} {scale} {name}"),
                    hash(&drawn(i, scale, &content).0),
                ));
            }
        }
    }
    crate::pins::check("deskwidget", &got);
}

#[test]
fn the_meter_reads_out_two_ranges_and_the_clock_its_time() {
    let theme = theme_for(Interface::Classic);
    let text = TextRenderer::load(&theme.fonts);
    let cm = theme.controls();
    let mut tree = Tree::new(access::node(Role::Window, Rect::new(0, 0, 500, 500)));
    let meter = sample(Kind::Meter, false);
    let g = access(
        &meter,
        Rect::new(10, 10, 210, 54),
        &cm,
        &text,
        11,
        &mut tree,
        access::ROOT,
        "m",
    );
    assert_eq!(tree.get(g).and_then(|n| n.label()), Some("System Meter"));
    let cpu = tree
        .find(Role::ProgressIndicator, "CPU")
        .and_then(|id| tree.get(id));
    assert_eq!(cpu.and_then(|n| n.numeric_value()), Some(95.0));
    let clock = Content::Clock(Clock::new(Face::Analog, false, (9, 5, 0), "", ""));
    access(
        &clock,
        Rect::new(0, 0, 129, 129),
        &cm,
        &text,
        11,
        &mut tree,
        access::ROOT,
        "c",
    );
    let said = tree.nodes().find(|(_, n)| n.value() == Some("9:05"));
    assert!(said.is_some(), "the time read out");
}
