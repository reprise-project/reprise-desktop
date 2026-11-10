//! The login screen's states and locks in each interface at 800x600, some over a picture,
//! written to `target/reprisekit-gallery-logon@<scale>.png`.

mod glass;

use reprisekit::art::userpic;
use reprisekit::blue::Variant;
use reprisekit::interface::Interface;
use reprisekit::interfaces::logon::{draw_logon, logon_layout};
use reprisekit::interfaces::{default_wallpaper, theme_with};
use reprisekit::logon::{Logon, LogonKey, LogonUser, Part, PowerAction, UserNote, text};
use reprisekit::pointer::Interaction;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Color;
use reprisekit::wallpapers;

const OUTPUT: Rect = Rect::new(0, 0, 800, 600);
const GAP: i32 = 10;
const COLS: i32 = 3;

fn users() -> Vec<LogonUser> {
    [
        ("alice", "Alice Liddell"),
        ("bob", ""),
        ("carol", "Carol Shaw"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (name, real))| LogonUser {
        name: name.into(),
        real_name: real.into(),
        picture: Some(userpic::draw(userpic::ALL[i], userpic::SAVED_PX)),
        note: [Some(UserNote::Remote), Some(UserNote::LoggedOn), None][i],
    })
    .collect()
}

/// Each screen, its Blue scheme and the built-in wallpaper standing in for a picture beneath.
fn screens() -> Vec<(Logon, Variant, Option<&'static str>)> {
    let empty = Logon::new(Interface::Classic, Vec::new(), false, None);
    let mut filled = Logon::new(Interface::Classic, Vec::new(), false, Some("alice"));
    filled.key(LogonKey::Text("hunter2"));
    let mut message = filled.clone();
    message.fail(text::incorrect());
    let mut busy = filled.clone();
    busy.set_busy(true);
    let mut caps = filled.clone();
    caps.caps_lock = true;
    let mut badge = filled.clone();
    badge.layout = Some("EN".into());
    badge.message = Some(text::power_refused());
    let starts = [Interface::Classic, Interface::Blue, Interface::Glass]
        .map(|i| Logon::new(i, users(), true, Some("carol")));
    let admins = starts.clone().map(|mut m| {
        m.admin(PowerAction::PowerOff);
        m
    });
    let pictured = [
        (filled.clone(), "tarn/dusk"),
        (starts[1].clone(), "tarn/dusk"),
        (starts[1].clone(), "tarn/day"),
    ]
    .map(|(mut m, picture)| {
        m.over_picture = true;
        (m, Variant::Blue, Some(picture))
    });
    let locks = [Interface::Classic, Interface::Blue, Interface::Glass]
        .map(|i| Logon::locked(i, users().remove(0), true, true));
    [empty, filled, message, busy, caps, badge]
        .into_iter()
        .chain(starts)
        .chain(admins)
        .chain(locks)
        .map(|m| (m, Variant::Blue))
        .chain(blue_screens())
        .chain(glass::screens().into_iter().map(|m| (m, Variant::Blue)))
        .map(|(m, v)| (m, v, None))
        .chain(pictured)
        .collect()
}

/// Blue's tiles with one hot, a tile open empty, with a message, Caps Lock and the badge, and
/// busy, the typed screen, then the open tile with its message in Olive Green and Silver.
fn blue_screens() -> Vec<(Logon, Variant)> {
    let rest = Logon::new(Interface::Blue, users(), true, Some("carol"));
    let mut hot = rest.clone();
    hot.lit = Some((Part::Tile(0), Interaction::Hot));
    let mut open = rest.clone();
    open.click(Part::Tile(1));
    let mut message = open.clone();
    message.key(LogonKey::Text("x"));
    message.fail(text::incorrect());
    message.caps_lock = true;
    message.layout = Some("EN".into());
    let mut busy = open.clone();
    busy.key(LogonKey::Text("hunter2"));
    busy.set_busy(true);
    let typed = Logon::new(Interface::Blue, Vec::new(), false, None);
    [hot, open, message.clone(), busy, typed]
        .map(|m| (m, Variant::Blue))
        .into_iter()
        .chain([Variant::Olive, Variant::Silver].map(|v| (message.clone(), v)))
        .collect()
}

pub fn render(scale: i32) -> Canvas {
    let screens = screens();
    let rows = (screens.len() as i32 + COLS - 1) / COLS;
    let at = |i: i32, n: i32| (GAP + i * (n + GAP)) * scale;
    let mut c = Canvas::new(
        at(COLS, OUTPUT.width) as u32,
        at(rows, OUTPUT.height) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(0x60, 0x60, 0x60));
    for (i, (m, v, picture)) in screens.iter().enumerate() {
        let theme =
            theme_with(m.interface, *v, &reprisekit::glass::Settings::default()).scaled(scale);
        let text = TextRenderer::load(&theme.fonts);
        let l = logon_layout(m, &theme, &text, OUTPUT);
        let (w, h) = (
            (OUTPUT.width * scale) as u32,
            (OUTPUT.height * scale) as u32,
        );
        let mut screen = match default_wallpaper(m.interface) {
            Some(b) if m.interface == Interface::Glass => wallpapers::render(b, w, h, 0),
            _ => Canvas::new(w, h),
        };
        if let Some(picture) = picture {
            let picture = wallpapers::Builtin::parse(picture).expect("a built-in");
            let mut over = Canvas::new(w, h);
            draw_logon(&mut over, m, &l, &theme, text.for_output(scale), scale);
            screen = wallpapers::render(picture, w, h, 0);
            screen.blit_over(0, 0, &over);
        } else {
            draw_logon(&mut screen, m, &l, &theme, text.for_output(scale), scale);
        }
        let i = i as i32;
        c.blit_canvas(
            at(i % COLS, OUTPUT.width),
            at(i / COLS, OUTPUT.height),
            &screen,
        );
    }
    c
}
