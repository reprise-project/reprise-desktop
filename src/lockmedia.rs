//! The lock screen's now-playing strip. The console's lock shows the player a single media control
//! goes to (`Players::newest`: the newest playing, else the last paused) by its title and artist
//! alone, and its buttons call that player's Previous, PlayPause or Next and nothing else: no
//! window opens or rises, and no art or other property of the player is read. The remote side's
//! lock shows none, as the player plays on this computer.

use reprisebus::mpris::{Players, Status};
use reprisekit::media::{NowPlaying, Transport};

use crate::compositor::State;
use crate::console::Side;

/// The player the lock shows and what its strip shows, by its bus name.
fn shown(players: &Players) -> Option<(&str, NowPlaying)> {
    let (bus, p) = players.newest()?;
    debug_assert!(matches!(p.status, Status::Playing | Status::Paused));
    Some((bus, crate::previewmedia::now_playing(p)))
}

/// What `side`'s lock shows: the console's player, or none.
pub fn media(state: &State, side: Side) -> Option<NowPlaying> {
    match side {
        Side::Console => shown(&state.preview_players.players).map(|(_, np)| np),
        Side::Remote => None,
    }
}

/// A press of `side`'s strip button `t`: the player shown now gets `t`'s call, if it can take it.
pub fn press(state: &mut State, side: Side, t: Transport) {
    if side != Side::Console {
        return;
    }
    let Some((bus, np)) = shown(&state.preview_players.players) else {
        return;
    };
    if np.can(t) {
        let bus = bus.to_string();
        crate::previewmedia::send(state, &bus, t);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::process::Command;

    use reprisebus::mpris::{About, Players};
    use reprisekit::access::accesskit::Role;
    use reprisekit::logon::Part;
    use reprisekit::media::Transport;
    use zbus::Variant;

    use crate::harness::Harness;
    use crate::lockview::{Helpers, Lock};

    const BUS: &str = "org.mpris.MediaPlayer2.mpv.instance7";
    const KEY_TAB: u32 = 15;
    const KEY_ENTER: u32 = 28;
    const KEY_A: u32 = 30;

    /// One player, `status`, with Previous live as `previous` says, playing "Song" by "Band" from
    /// a local file, with art on the network and an identity of its own.
    fn players(status: &str, previous: bool) -> Players {
        let meta: HashMap<String, Variant<'static>> = HashMap::from([
            ("xesam:title".to_string(), Variant::from("Song")),
            (
                "xesam:artist".to_string(),
                Variant::from(vec!["Band".to_string()]),
            ),
            (
                "xesam:url".to_string(),
                Variant::from("file:///home/u/private.mp3"),
            ),
            (
                "mpris:artUrl".to_string(),
                Variant::from("https://example.com/art.png"),
            ),
        ]);
        let props = HashMap::from([
            ("PlaybackStatus", Variant::from(status)),
            ("Metadata", Variant::from(meta)),
            ("CanGoPrevious", Variant::from(previous)),
            ("CanGoNext", Variant::from(true)),
            ("CanPlay", Variant::from(true)),
            ("CanPause", Variant::from(true)),
        ]);
        let msg = zbus::Message::signal("/", "org.freedesktop.DBus.Properties", "Changed")
            .build(&props)
            .unwrap();
        let mut p = Players::default();
        let about = About {
            identity: "Hidden Player".into(),
            desktop_entry: "hidden".into(),
            pid: Some(7),
            app: None,
        };
        p.add(BUS, &reprisebus::bounded::decode(&msg).unwrap(), about);
        p
    }

    /// A harness locked behind its own screen over one window, the password check never passing.
    fn locked(with: Option<Players>) -> Harness {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        h.map_toplevel(&mut c, "behind", 300, 200);
        h.state.lock.helpers = Helpers {
            password: |_, _| {
                let mut c = Command::new("sh");
                c.arg("-c").arg("read p; echo '= 7'");
                c
            },
            finger: |_| None,
            ..Helpers::default()
        };
        if let Some(p) = with {
            h.state.preview_players.players = p;
        }
        crate::lockview::lock(&mut h.state);
        h
    }

    fn lock(h: &Harness) -> &Lock {
        h.state.lock.console.as_ref().expect("locked")
    }

    fn sent(h: &Harness) -> Vec<(String, &'static str)> {
        h.state.preview_players.sent.clone()
    }

    fn tap(h: &mut Harness, code: u32) {
        h.key(code, true);
        h.key(code, false);
        h.dispatch();
    }

    fn click(h: &mut Harness, part: Part) {
        let p = lock(h).part(part).expect("the part shows");
        h.click(p.x, p.y);
        h.dispatch();
    }

    /// The lock's state the strip must leave alone: still locked, the window behind unfocused,
    /// nothing typed.
    fn untouched(h: &Harness) {
        assert!(h.state.console_locked(), "the session stays locked");
        assert_eq!(h.state.focused, None, "no window takes the keyboard");
        assert_eq!(lock(h).input(), "", "nothing reaches the password");
    }

    #[test]
    fn the_strips_buttons_call_only_the_players_previous_play_pause_and_next() {
        let mut h = locked(Some(players("Playing", true)));
        for t in Transport::ALL {
            click(&mut h, Part::Media(t));
        }
        let bus = BUS.to_string();
        assert_eq!(
            sent(&h),
            [
                (bus.clone(), "Previous"),
                (bus.clone(), "PlayPause"),
                (bus, "Next")
            ]
        );
        untouched(&h);
        assert_eq!(
            lock(&h).view.model.focus,
            Part::Password,
            "the password keeps the focus"
        );
    }

    #[test]
    fn a_greyed_button_calls_nothing() {
        let mut h = locked(Some(players("Paused", false)));
        click(&mut h, Part::Media(Transport::Previous));
        assert!(sent(&h).is_empty(), "the player can't go back");
        untouched(&h);
    }

    #[test]
    fn tab_reaches_the_buttons_after_the_screens_own_and_enter_presses_them_and_nothing_else() {
        let mut h = locked(Some(players("Playing", true)));
        let stops = lock(&h).view.model.stops();
        let at = |p| stops.iter().position(|&s| s == p).unwrap();
        assert_eq!(at(Part::Password), 0, "the password comes first");
        assert_eq!(
            stops[stops.len() - 3..],
            Transport::ALL.map(Part::Media),
            "the media buttons come last"
        );
        for _ in 0..at(Part::Media(Transport::PlayPause)) {
            tap(&mut h, KEY_TAB);
        }
        assert_eq!(lock(&h).view.model.focus, Part::Media(Transport::PlayPause));
        tap(&mut h, KEY_A);
        assert!(sent(&h).is_empty(), "a letter presses nothing");
        tap(&mut h, KEY_ENTER);
        assert_eq!(sent(&h), [(BUS.to_string(), "PlayPause")]);
        untouched(&h);
        let tree = crate::lockview::access(&h.state).expect("the lock's tree");
        assert_eq!(tree.find(Role::Button, "Pause"), Some(tree.focus()));
    }

    #[test]
    fn the_strip_shows_the_title_and_artist_and_nothing_else_of_the_player() {
        let h = locked(Some(players("Playing", true)));
        assert_eq!(
            lock(&h)
                .view
                .model
                .media
                .as_ref()
                .map(|np| (np.title.as_str(), np.artist.as_str())),
            Some(("Song", "Band"))
        );
        let tree = crate::lockview::access(&h.state).expect("the lock's tree");
        let said: Vec<String> = (tree.nodes())
            .flat_map(|(_, n)| [n.label(), n.value()])
            .flatten()
            .map(str::to_string)
            .collect();
        assert!(said.iter().any(|s| s == "Song, Band"), "{said:?}");
        for hidden in ["Hidden Player", "hidden", "private.mp3", "example.com"] {
            assert!(
                !said.iter().any(|s| s.contains(hidden)),
                "{hidden} in {said:?}"
            );
        }
    }

    #[test]
    fn a_player_going_while_locked_takes_the_strip_and_its_focus_away() {
        let mut h = locked(Some(players("Playing", true)));
        for _ in 0..lock(&h).view.model.stops().len() - 1 {
            tap(&mut h, KEY_TAB);
        }
        assert_eq!(lock(&h).view.model.focus, Part::Media(Transport::Next));
        h.state.preview_players.players = Players::default();
        crate::lockview::update(&mut h.state);
        let l = lock(&h);
        assert_eq!(l.view.model.media, None);
        assert_eq!(l.view.model.focus, Part::Password, "the focus goes home");
        let layout = l.view.layout.as_ref().expect("laid out");
        assert_eq!(layout.media, None);
        assert!(
            !layout
                .parts
                .iter()
                .any(|(p, _)| matches!(p, Part::Media(_)))
        );
        tap(&mut h, KEY_ENTER);
        assert!(sent(&h).is_empty());
    }

    #[test]
    fn with_no_player_the_lock_has_no_strip_and_the_same_stops_and_parts() {
        let h = locked(None);
        let bare = locked(Some(players("Stopped", true)));
        for h in [&h, &bare] {
            let l = lock(h);
            assert_eq!(l.view.model.media, None, "a stopped player shows nothing");
            assert_eq!(l.view.layout.as_ref().and_then(|l| l.media), None);
        }
        let parts = |h: &Harness| lock(h).view.layout.clone().map(|l| l.parts);
        assert_eq!(parts(&h), parts(&bare));
        assert_eq!(lock(&h).view.model.stops(), lock(&bare).view.model.stops());
    }

    #[test]
    fn the_remote_sides_lock_shows_no_player() {
        let mut h = locked(Some(players("Playing", true)));
        assert!(super::media(&h.state, crate::console::Side::Remote).is_none());
        super::press(
            &mut h.state,
            crate::console::Side::Remote,
            Transport::PlayPause,
        );
        assert!(sent(&h).is_empty());
    }
}
