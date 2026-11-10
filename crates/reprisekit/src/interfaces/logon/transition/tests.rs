use super::*;
use crate::art::userpic;
use crate::blue::Variant;
use crate::interfaces::logon::draw_logon;
use crate::interfaces::{blue_theme, theme_for};
use crate::logon::{LogonKey, LogonUser, Moves, PowerAction, Screen, UserNote, text as strings};

const OUTPUT: Rect = Rect::new(0, 0, 1024, 768);

/// A change's name, the screen before it, and what changes it.
type Case = (&'static str, Logon, fn(&mut Logon));

fn fnv(c: &Canvas) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in &c.pixels {
        for b in p.to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

fn users(n: usize) -> Vec<LogonUser> {
    (0..n)
        .map(|i| LogonUser {
            name: format!("user{i}"),
            real_name: if i == 0 {
                "Alice Liddell".into()
            } else {
                String::new()
            },
            picture: (i == 0).then(|| userpic::draw(userpic::ALL[3], 128)),
            note: (i == 1).then_some(UserNote::LoggedOn),
            stock: i,
        })
        .collect()
}

/// `interface`'s login screen on `screen`, moving, nothing noted yet.
fn screen(interface: Interface, screen: Screen) -> Logon {
    let mut m = Logon::new(interface, users(3), true, Some("user0"));
    m.moves = Moves::Full;
    match screen {
        Screen::Tiles if interface == Interface::Glass => {
            m.click(Part::SwitchUser);
        }
        Screen::Tile(i) if interface == Interface::Blue => {
            m.click(Part::Tile(i));
        }
        _ => {}
    }
    m.take_motion();
    m
}

/// One change played: the screen after, its theme and text, the change and its transition.
struct Played {
    m: Logon,
    theme: Theme,
    text: TextRenderer,
    s: i32,
    motion: Motion,
    t: Transition,
}

fn theme(m: &Logon, v: Variant) -> Theme {
    match m.interface {
        Interface::Blue => blue_theme(v),
        i => theme_for(i),
    }
}

fn play(mut m: Logon, v: Variant, s: i32, act: impl FnOnce(&mut Logon)) -> Played {
    act(&mut m);
    let motion = m.take_motion().expect("the change is noted");
    let theme = theme(&m, v).scaled(s);
    let text = TextRenderer::load(&theme.fonts);
    let l = logon_layout(&m, &theme, &text, OUTPUT);
    let d = Drawn {
        m: &m,
        l: &l,
        theme: &theme,
        text: &text,
        s,
    };
    let t = Transition::begin(&motion, &d).expect("the change plays");
    Played {
        m,
        theme,
        text,
        s,
        motion,
        t,
    }
}

impl Played {
    fn layout(&self) -> LogonLayout {
        logon_layout(&self.m, &self.theme, &self.text, OUTPUT)
    }

    fn still(&self, m: &Logon) -> Canvas {
        let l = logon_layout(m, &self.theme, &self.text, OUTPUT);
        let s = self.s;
        let mut c = Canvas::new((OUTPUT.width * s) as u32, (OUTPUT.height * s) as u32);
        draw_logon(&mut c, m, &l, &self.theme, self.text.for_output(s), s);
        c
    }

    /// The frame `at` ms in, over the static screen after as the host's canvas holds it.
    fn at(&self, at: u32) -> Canvas {
        let mut c = self.still(&self.m);
        self.t.frame(&mut c, &self.m, at);
        c
    }

    fn before(&self) -> Canvas {
        let mut was = self.motion.was.model().clone();
        was.spin = self.m.spin;
        self.still(&was)
    }

    fn chrome(&self) -> Canvas {
        let d = Drawn {
            m: &self.m,
            l: &self.layout(),
            theme: &self.theme,
            text: &self.text,
            s: self.s,
        };
        d.canvas(&self.m, d.l, Show::Chrome)
    }
}

fn glass_changes() -> Vec<Case> {
    let tile = || screen(Interface::Glass, Screen::Tile(0));
    let tiles = || screen(Interface::Glass, Screen::Tiles);
    let typed = |m: &mut Logon| {
        m.key(LogonKey::Text("abc"));
    };
    let mut busy = tile();
    typed(&mut busy);
    busy.sent();
    busy.take_motion();
    vec![
        ("pick", tiles(), |m| {
            m.click(Part::Tile(0));
        }),
        ("pick other", tiles(), |m| {
            m.click(Part::OtherUser);
        }),
        ("back", tile(), |m| {
            m.key(LogonKey::Escape);
        }),
        ("enter tiles", tiles(), Logon::enter),
        ("enter tile", tile(), Logon::enter),
        (
            "busy",
            {
                let mut m = tile();
                typed(&mut m);
                m
            },
            Logon::sent,
        ),
        ("status", busy.clone(), Logon::starting),
        ("fail", busy.clone(), |m| m.fail(strings::incorrect())),
        ("admin", tile(), |m| m.admin(PowerAction::PowerOff)),
    ]
}

fn blue_changes() -> Vec<Case> {
    let tiles = || screen(Interface::Blue, Screen::Tiles);
    let mut busy = screen(Interface::Blue, Screen::Tile(1));
    busy.key(LogonKey::Text("abc"));
    busy.sent();
    busy.take_motion();
    vec![
        ("pick", tiles(), |m| {
            m.click(Part::Tile(1));
        }),
        ("back", screen(Interface::Blue, Screen::Tile(1)), |m| {
            m.key(LogonKey::Escape);
        }),
        ("enter", tiles(), Logon::enter),
        ("fail", busy, |m| m.fail(strings::incorrect())),
    ]
}

/// Pins at the start, the middle and the end of every change at scale 1 and 2.
fn pins(changes: Vec<Case>, v: Variant, got: &mut Vec<(String, u64)>) {
    for (name, m, act) in changes {
        for s in [1, 2] {
            let p = play(m.clone(), v, s, act);
            let ms = p.t.ms();
            for at in [0, ms / 2, ms] {
                got.push((format!("{name} {v:?} {at}ms @{s}"), fnv(&p.at(at))));
            }
        }
    }
}

#[test]
fn glass_motion_pins_at_scale_1_and_2() {
    crate::pins::fonts();
    let mut got = Vec::new();
    pins(glass_changes(), Variant::Blue, &mut got);
    crate::pins::check("glass/logon-motion", &got);
}

#[test]
fn blue_motion_pins_per_scheme_at_scale_1_and_2() {
    crate::pins::fonts();
    let mut got = Vec::new();
    for v in Variant::ALL {
        pins(blue_changes(), v, &mut got);
    }
    crate::pins::check("blue/logon-motion", &got);
}

/// The meaning of every change: it starts on the screen before and ends on the screen after,
/// pixel for pixel, differs from both midway, and moves nothing outside its rect.
fn check(name: &str, p: &Played) {
    let (ms, s) = (p.t.ms(), p.s);
    let after = p.still(&p.m);
    let start = match p.t.change {
        Change::Enter => p.chrome(),
        _ => p.before(),
    };
    assert!(
        p.at(0).pixels == start.pixels,
        "{name} @{s}: starts on the screen before"
    );
    assert!(
        p.at(ms).pixels == after.pixels,
        "{name} @{s}: ends on the static screen"
    );
    let mut mid = p.at(ms / 2);
    assert!(
        mid.pixels != start.pixels && mid.pixels != after.pixels,
        "{name} @{s}: moves"
    );
    p.t.settle(&mut mid, &p.m);
    assert!(
        mid.pixels == after.pixels,
        "{name} @{s}: settles from midway on the static screen"
    );
    assert!(
        p.t.rest(&p.m).pixels == after.pixels,
        "{name} @{s}: rests on the static screen"
    );
    let l = p.layout();
    let d = Drawn {
        m: &p.m,
        l: &l,
        theme: &p.theme,
        text: &p.text,
        s,
    };
    let over = Transition::begin_over(&p.motion, &d, Some(p.before())).expect("it plays");
    let mut c = after.clone();
    over.frame(&mut c, &p.m, ms / 2);
    assert!(
        c.pixels == p.at(ms / 2).pixels,
        "{name} @{s}: over the canvas shown before, the same frames"
    );
    let r = p.t.rect();
    let w = after.width as i32;
    for (i, (a, b)) in start.pixels.iter().zip(&after.pixels).enumerate() {
        let (x, y) = (i as i32 % w, i as i32 / w);
        if !r.contains(x, y) {
            assert_eq!(a, b, "{name} @{s}: ({x}, {y}) outside the rect changes");
        }
    }
}

#[test]
fn every_glass_change_runs_from_the_screen_before_to_the_screen_after() {
    for (name, m, act) in glass_changes() {
        for s in [1, 2] {
            check(name, &play(m.clone(), Variant::Blue, s, act));
        }
    }
}

#[test]
fn every_blue_change_runs_from_the_screen_before_to_the_screen_after() {
    for (name, m, act) in blue_changes() {
        for v in [Variant::Blue, Variant::Silver] {
            check(name, &play(m.clone(), v, 1, act));
        }
    }
    let p = play(blue_changes().remove(0).1, Variant::Blue, 1, |m| {
        m.click(Part::Tile(1));
    });
    assert_eq!(p.t.carried_at(0), None, "Blue flies no frame");
}

#[test]
fn blue_busy_and_status_show_at_once() {
    let mut m = screen(Interface::Blue, Screen::Tile(1));
    m.key(LogonKey::Text("abc"));
    m.sent();
    let motion = m.take_motion().unwrap();
    let theme = blue_theme(Variant::Blue);
    let text = TextRenderer::load(&theme.fonts);
    let l = logon_layout(&m, &theme, &text, OUTPUT);
    let d = Drawn {
        m: &m,
        l: &l,
        theme: &theme,
        text: &text,
        s: 1,
    };
    assert!(Transition::begin(&motion, &d).is_none());
}

#[test]
fn a_picked_frame_flies_between_its_slot_and_its_place() {
    for s in [1, 2] {
        let p = play(glass_changes().remove(0).1, Variant::Blue, s, |m| {
            m.click(Part::Tile(0));
        });
        let (a, mid, b) = (
            p.t.carried_at(0).unwrap(),
            p.t.carried_at(p.t.ms() / 2).unwrap(),
            p.t.carried_at(p.t.ms()).unwrap(),
        );
        assert!(a.width < mid.width && mid.width < b.width, "@{s}: it grows");
        let between = |v: i32, x: i32, y: i32| (x.min(y)..=x.max(y)).contains(&v);
        assert!(
            between(mid.x, a.x, b.x) && between(mid.y, a.y, b.y),
            "@{s}: {mid:?} between"
        );
    }
}

#[test]
fn entering_tiles_rise_one_after_another() {
    let p = play(
        screen(Interface::Glass, Screen::Tiles),
        Variant::Blue,
        1,
        Logon::enter,
    );
    let l = p.layout();
    let early = p.at(p.t.plan.into.last().unwrap().window.from);
    let first = l.rect(Part::Tile(0)).unwrap();
    let last = l.rect(Part::OtherUser).unwrap();
    let ink = |c: &Canvas, r: Rect| {
        (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) >> 24 > 0)
            .count()
    };
    assert!(ink(&early, first) > 0, "the first tile has begun");
    assert_eq!(ink(&early, last), 0, "the last tile waits its turn");
}

#[test]
fn escape_mid_pick_flies_back_from_where_the_frame_is() {
    let p = play(
        screen(Interface::Glass, Screen::Tiles),
        Variant::Blue,
        1,
        |m| {
            m.click(Part::Tile(0));
        },
    );
    let mid = p.t.ms() / 2;
    let shown = p.at(mid);
    let mut m = p.m.clone();
    m.key(LogonKey::Escape);
    let motion = m.take_motion().unwrap();
    assert_eq!(motion.change, Change::Back(Part::Tile(0)));
    let l = logon_layout(&m, &p.theme, &p.text, OUTPUT);
    let d = Drawn {
        m: &m,
        l: &l,
        theme: &p.theme,
        text: &p.text,
        s: 1,
    };
    let back = Transition::then(&p.t, mid, &motion, &d).expect("the way back plays");
    assert_eq!(back.carried_at(0), p.t.carried_at(mid), "from where it is");
    let mut c = p.still(&m);
    back.frame(&mut c, &m, 0);
    assert!(
        c.pixels == shown.pixels,
        "the first frame back is the frame it left"
    );
    let mut end = p.still(&m);
    back.frame(&mut end, &m, back.ms());
    assert!(end.pixels == p.still(&m).pixels, "it ends on the tiles");
}

#[test]
fn keys_typed_while_the_block_fades_in_show_at_the_end() {
    let mut p = play(
        screen(Interface::Glass, Screen::Tiles),
        Variant::Blue,
        1,
        |m| {
            m.click(Part::Tile(0));
        },
    );
    p.m.key(LogonKey::Text("abc"));
    let l = p.layout();
    let d = Drawn {
        m: &p.m,
        l: &l,
        theme: &p.theme,
        text: &p.text,
        s: 1,
    };
    p.t.refresh_now(&d);
    assert_eq!(p.m.password.text, "abc");
    assert!(
        p.at(p.t.ms()).pixels == p.still(&p.m).pixels,
        "the end shows three bullets"
    );
    let mut mid = p.at(p.t.ms() / 2);
    p.t.settle(&mut mid, &p.m);
    assert!(
        mid.pixels == p.still(&p.m).pixels,
        "settled, the box shows three bullets"
    );
}

#[test]
fn a_line_that_comes_up_under_an_entering_block_ends_in_place() {
    for s in [1, 2] {
        let user = users(1).remove(0);
        let mut lock = Logon::locked(Interface::Glass, user, false, false);
        lock.moves = Moves::Full;
        let mut p = play(lock, Variant::Blue, s, Logon::enter);
        p.m.caps_lock = true;
        let l = p.layout();
        let d = Drawn {
            m: &p.m,
            l: &l,
            theme: &p.theme,
            text: &p.text,
            s,
        };
        p.t.refresh_now(&d);
        assert!(
            p.at(p.t.ms()).pixels == p.still(&p.m).pixels,
            "@{s}: the Caps Lock line ends under the box"
        );
    }
}

/// Prints the median and worst cost of one frame (compose plus a copy of its rect, as the host's upload
/// does) of each Glass and Blue change on a 1920x1080 output at scales 1 and 2. Run with
/// `--ignored --nocapture`; the budget is 8 ms.
#[test]
#[ignore]
fn transition_frame_cost() {
    const FULL_HD: Rect = Rect::new(0, 0, 1920, 1080);
    const FRAMES: u32 = 60;
    for (changes, v) in [
        (glass_changes(), Variant::Blue),
        (blue_changes(), Variant::Blue),
    ] {
        for (name, m0, act) in changes {
            for s in [1, 2] {
                let mut m = m0.clone();
                act(&mut m);
                let Some(motion) = m.take_motion() else {
                    continue;
                };
                let theme = theme(&m, v).scaled(s);
                let text = TextRenderer::load(&theme.fonts);
                let l = logon_layout(&m, &theme, &text, FULL_HD);
                let d = Drawn {
                    m: &m,
                    l: &l,
                    theme: &theme,
                    text: &text,
                    s,
                };
                let Some(t) = Transition::begin(&motion, &d) else {
                    continue;
                };
                let (w, h) = ((FULL_HD.width * s) as u32, (FULL_HD.height * s) as u32);
                let mut c = Canvas::new(w, h);
                draw_logon(&mut c, &m, &l, &theme, text.for_output(s), s);
                let mut shown = c.clone();
                let mut ms: Vec<f64> = (0..FRAMES)
                    .map(|i| {
                        let start = std::time::Instant::now();
                        let r = t.frame(&mut c, &m, t.ms() * i / FRAMES);
                        copy_in(&mut shown, &c, r);
                        start.elapsed().as_secs_f64() * 1000.0
                    })
                    .collect();
                ms.sort_by(f64::total_cmp);
                let r = t.rect();
                println!(
                    "{:?} {name} @{s}: median {:.2} ms, worst {:.2} ms, rect {}x{}",
                    m.interface,
                    ms[ms.len() / 2],
                    ms[ms.len() - 1],
                    r.width,
                    r.height
                );
            }
        }
    }
}

/// Prints the median cost of a click's draw and of the settle frame against painting the screen
/// whole, per change, 1920x1080 at scales 1 and 2. Run with `--ignored --nocapture`.
#[test]
#[ignore]
fn click_and_settle_cost() {
    const FULL_HD: Rect = Rect::new(0, 0, 1920, 1080);
    const RUNS: usize = 11;
    let median = |mut ms: Vec<f64>| {
        ms.sort_by(f64::total_cmp);
        ms[ms.len() / 2]
    };
    let time = |f: &mut dyn FnMut()| {
        median(
            (0..RUNS)
                .map(|_| {
                    let start = std::time::Instant::now();
                    f();
                    start.elapsed().as_secs_f64() * 1000.0
                })
                .collect(),
        )
    };
    for (changes, v) in [
        (glass_changes(), Variant::Blue),
        (blue_changes(), Variant::Blue),
    ] {
        for (name, m0, act) in changes {
            for s in [1, 2] {
                let mut m = m0.clone();
                act(&mut m);
                let Some(motion) = m.take_motion() else {
                    continue;
                };
                let theme = theme(&m, v).scaled(s);
                let text = TextRenderer::load(&theme.fonts);
                let l = logon_layout(&m, &theme, &text, FULL_HD);
                let d = Drawn {
                    m: &m,
                    l: &l,
                    theme: &theme,
                    text: &text,
                    s,
                };
                let (w, h) = ((FULL_HD.width * s) as u32, (FULL_HD.height * s) as u32);
                let mut shown = Canvas::new(w, h);
                let whole = |shown: &mut Canvas| {
                    let mut c = Canvas::new(w, h);
                    draw_logon(&mut c, &m, &l, &theme, text.for_output(s), s);
                    shown.pixels.copy_from_slice(&c.pixels);
                    c
                };
                let painted_click = time(&mut || {
                    let mut c = whole(&mut shown);
                    if let Some(t) = Transition::begin(&motion, &d) {
                        t.frame(&mut c, &m, 0);
                    }
                });
                let mut befores: Vec<Canvas> = (0..RUNS).map(|_| shown.clone()).collect();
                let click = time(&mut || {
                    let before = befores.pop();
                    if let Some(t) = Transition::begin_over(&motion, &d, before) {
                        let mut c = t.rest(&m);
                        t.frame(&mut c, &m, 0);
                        shown.pixels.copy_from_slice(&c.pixels);
                    }
                });
                let Some(t) = Transition::begin(&motion, &d) else {
                    continue;
                };
                let settle_whole = time(&mut || {
                    whole(&mut shown);
                });
                let mut c = whole(&mut shown);
                t.frame(&mut c, &m, t.ms() / 2);
                let settle_rect = time(&mut || {
                    let r = t.settle(&mut c, &m);
                    copy_in(&mut shown, &c, r);
                });
                println!(
                    "{:?} {name} @{s}: click {click:.2} ms (painted whole {painted_click:.2} \
                     ms), settle {settle_rect:.2} ms (painted whole {settle_whole:.2} ms)",
                    m.interface,
                );
            }
        }
    }
}
