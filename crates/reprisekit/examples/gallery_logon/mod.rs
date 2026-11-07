//! The login screen at 800x600: Classic's dialog states, then each interface's starting screen,
//! written to `target/reprisekit-gallery-logon@<scale>.png`.

use reprisekit::art::userpic;
use reprisekit::interface::Interface;
use reprisekit::interfaces::logon::{draw_logon, logon_layout};
use reprisekit::interfaces::theme_for;
use reprisekit::logon::{Logon, LogonKey, LogonUser, UserNote, text};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Color;

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
        picture: Some(userpic::draw(userpic::ALL[i], 48)),
        note: (i == 1).then_some(UserNote::LoggedOn),
    })
    .collect()
}

fn screens() -> Vec<Logon> {
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
    [empty, filled, message, busy, caps, badge]
        .into_iter()
        .chain(starts)
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
    for (i, m) in screens.iter().enumerate() {
        let theme = theme_for(m.interface).scaled(scale);
        let text = TextRenderer::load(&theme.fonts);
        let l = logon_layout(m, &theme, &text, OUTPUT);
        let mut screen = Canvas::new(
            (OUTPUT.width * scale) as u32,
            (OUTPUT.height * scale) as u32,
        );
        draw_logon(&mut screen, m, &l, &theme, text.for_output(scale), scale);
        let i = i as i32;
        c.blit_canvas(
            at(i % COLS, OUTPUT.width),
            at(i / COLS, OUTPUT.height),
            &screen,
        );
    }
    c
}
