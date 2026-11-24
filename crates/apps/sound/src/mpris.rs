//! The media player the volume popup's strip shows, from the players `reprisebus::mpris`
//! follows: the newest Playing player, else the one that played last and is Paused.

use reprisebus::mpris::{Player, Players};
use reprisekit::controls::volumepopup::{NowPlaying, Transport};

/// Chars kept of a title, an artist list and a player's name.
const TEXT_CHARS: usize = 256;

fn cut(s: &str) -> String {
    reprisefs::names::shown_line(s, TEXT_CHARS)
}

fn file_name(url: &str) -> String {
    let last = url.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let bytes = last.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The strip's line for `p`: its title, else the file it plays, else its name.
fn now_playing(p: &Player) -> NowPlaying {
    let title = [cut(&p.title), cut(&file_name(&p.url)), cut(&p.identity)]
        .into_iter()
        .find(|s| !s.trim().is_empty())
        .unwrap_or_default();
    NowPlaying {
        title,
        artist: cut(&p.artist),
        playing: p.status == reprisebus::mpris::Status::Playing,
        can_previous: p.can_previous(),
        can_next: p.can_next(),
        can_play: p.can_play(),
        can_pause: p.can_pause(),
    }
}

pub fn method(t: Transport) -> reprisebus::mpris::Method {
    match t {
        Transport::Previous => "Previous",
        Transport::PlayPause => "PlayPause",
        Transport::Next => "Next",
    }
}

pub type Pick = Option<(String, NowPlaying)>;

/// The player the strip shows and its bus name.
pub fn pick(players: &Players) -> Pick {
    let (name, p) = players.newest()?;
    Some((name.to_string(), now_playing(p)))
}

pub fn watch(waker: repriseapp::Waker) -> reprisebus::mpris::Watch {
    reprisebus::mpris::watch("reprise-sound-mpris", move || waker.wake())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisebus::bounded::{self, Dict};
    use reprisebus::mpris::About;
    use std::collections::HashMap;
    use zbus::{Message, Variant};

    fn props(entries: Vec<(&str, Variant<'static>)>) -> Dict {
        let map: HashMap<&str, Variant> = entries.into_iter().collect();
        let msg = Message::signal(
            "/org/mpris/MediaPlayer2",
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .build(&map)
        .unwrap();
        bounded::decode(&msg).unwrap()
    }

    fn named(identity: &str) -> About {
        About {
            identity: identity.into(),
            ..About::default()
        }
    }

    #[test]
    fn metadata_and_capabilities_reach_the_strip() {
        let mut p = Players::default();
        let meta: HashMap<String, Variant<'static>> = HashMap::from([
            ("xesam:title".to_string(), Variant::from("Song")),
            (
                "xesam:artist".to_string(),
                Variant::from(vec!["One".to_string(), "Two".to_string()]),
            ),
        ]);
        p.add(
            "org.mpris.MediaPlayer2.a",
            &props(vec![
                ("PlaybackStatus", Variant::from("Playing")),
                ("Metadata", Variant::from(meta)),
                ("CanGoNext", Variant::from(true)),
                ("CanGoPrevious", Variant::from(false)),
                ("CanPlay", Variant::from(true)),
                ("CanPause", Variant::from(true)),
            ]),
            named("A"),
        );
        let (_, np) = pick(&p).unwrap();
        assert_eq!(
            np,
            NowPlaying {
                title: "Song".into(),
                artist: "One, Two".into(),
                playing: true,
                can_previous: false,
                can_next: true,
                can_play: true,
                can_pause: true,
            }
        );
        p.changed(
            "org.mpris.MediaPlayer2.a",
            &props(vec![("CanControl", Variant::from(false))]),
        );
        let (_, np) = pick(&p).unwrap();
        assert!(!np.can_next && !np.can_pause, "uncontrollable: all greyed");
    }

    #[test]
    fn titles_artists_and_player_names_are_cut() {
        let mut p = Players::default();
        let long = "x".repeat(10_000);
        let meta: HashMap<String, Variant<'static>> = HashMap::from([
            ("xesam:title".to_string(), Variant::from(long.clone())),
            ("xesam:artist".to_string(), Variant::from(long.clone())),
        ]);
        p.add(
            "org.mpris.MediaPlayer2.a",
            &props(vec![
                ("PlaybackStatus", Variant::from("Playing")),
                ("Metadata", Variant::from(meta)),
            ]),
            named(&long),
        );
        let (_, np) = pick(&p).unwrap();
        assert_eq!(np.title.chars().count(), TEXT_CHARS);
        assert_eq!(np.artist.chars().count(), TEXT_CHARS);
        p.changed(
            "org.mpris.MediaPlayer2.a",
            &props(vec![(
                "Metadata",
                Variant::from(HashMap::<String, Variant<'static>>::new()),
            )]),
        );
        assert_eq!(pick(&p).unwrap().1.title.chars().count(), TEXT_CHARS);
    }

    #[test]
    fn a_track_with_no_title_shows_its_file_then_the_player() {
        let mut p = Players::default();
        let meta: HashMap<String, Variant<'static>> = HashMap::from([(
            "xesam:url".to_string(),
            Variant::from("file:///home/u/Music/My%20Song.ogg"),
        )]);
        p.add(
            "org.mpris.MediaPlayer2.a",
            &props(vec![
                ("PlaybackStatus", Variant::from("Paused")),
                ("Metadata", Variant::from(meta)),
            ]),
            named("mpv"),
        );
        assert_eq!(pick(&p).unwrap().1.title, "My Song.ogg");
        p.changed(
            "org.mpris.MediaPlayer2.a",
            &props(vec![(
                "Metadata",
                Variant::from(HashMap::<String, Variant<'static>>::new()),
            )]),
        );
        assert_eq!(pick(&p).unwrap().1.title, "mpv");
    }
}
