//! Glass's login screens: the tile at rest, frame hot, typed into with the arrow hot, Other user,
//! the tiles, the error, after its OK, busy, and power hot with Caps Lock and a layout badge.

use reprisekit::art::userpic;
use reprisekit::interface::Interface;
use reprisekit::logon::{Logon, LogonKey, LogonUser, Part, UserNote, text};
use reprisekit::pointer::Interaction;

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
        picture: (i != 1).then(|| userpic::draw(userpic::ALL[i * 3], 256)),
        note: [Some(UserNote::Remote), Some(UserNote::LoggedOn), None][i],
        stock: i,
    })
    .collect()
}

pub fn screens() -> Vec<Logon> {
    let rest = Logon::new(Interface::Glass, users(), true, Some("carol"));
    let mut frame_hot = rest.clone();
    frame_hot.lit = Some((Part::Tile(2), Interaction::Hot));
    let mut typed = rest.clone();
    typed.key(LogonKey::Text("hunter2"));
    typed.lit = Some((Part::Submit, Interaction::Hot));
    let mut other = rest.clone();
    other.click(Part::SwitchUser);
    other.click(Part::OtherUser);
    other.key(LogonKey::Text("dave"));
    let mut tiles = rest.clone();
    tiles.click(Part::SwitchUser);
    tiles.lit = Some((Part::Tile(0), Interaction::Hot));
    let mut message = typed.clone();
    message.lit = None;
    message.fail(text::incorrect());
    let mut after_ok = message.clone();
    after_ok.click(Part::MessageOk);
    let mut busy = typed.clone();
    busy.lit = None;
    busy.set_busy(true);
    let mut power = rest.clone();
    power.caps_lock = true;
    power.layout = Some("EN".into());
    power.lit = Some((Part::Power, Interaction::Hot));
    vec![
        rest, frame_hot, typed, other, tiles, message, after_ok, busy, power,
    ]
}
