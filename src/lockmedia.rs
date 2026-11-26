//! The lock screen's now-playing strip. The console's lock shows the player the media keys go to
//! (`Players::target`) while it plays or is paused, by its title and artist alone, and its
//! buttons call that player's Previous, PlayPause or Next and nothing else: no window opens or
//! rises, and no art or other property of the player is read. A press goes to the player the
//! strip showed, by the unique bus name it had then. The remote side's lock shows none, and
//! neither does the console's in lock mode: the session and its player are the remote user's.

use reprisebus::mpris::Status;
use reprisekit::media::{NowPlaying, Transport};

use crate::compositor::State;
use crate::console::Side;

/// The player a lock's strip shows: its unique bus name and what the strip shows of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub owner: String,
    pub np: NowPlaying,
}

/// What `side`'s lock shows now: the console's player outside lock mode, or none.
pub fn shown(state: &State, side: Side) -> Option<Shown> {
    if side != Side::Console || crate::console::remote_side(state) {
        return None;
    }
    let (_, p) = state.preview_players.players.newest()?;
    debug_assert!(matches!(p.status, Status::Playing | Status::Paused));
    Some(Shown {
        owner: p.owner.clone(),
        np: crate::previewmedia::now_playing(p),
    })
}

/// Whether a lock's strip would show another player or other text or state than it does.
pub fn moved(state: &State) -> bool {
    [Side::Console, Side::Remote].into_iter().any(|side| {
        state
            .lock
            .on(side)
            .is_some_and(|l| l.media != shown(state, side))
    })
}

/// A press of `side`'s strip button `t`: the player the strip shows gets `t`'s call, if it can
/// take it.
pub fn press(state: &mut State, side: Side, t: Transport) {
    if side != Side::Console || crate::console::remote_side(state) {
        return;
    }
    let Some(Shown { owner, np }) = state.lock.on(side).and_then(|l| l.media.clone()) else {
        return;
    };
    if np.can(t) {
        crate::previewmedia::send(state, &owner, t);
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

    use crate::console::Side;
    use crate::harness::Harness;
    use crate::lockview::{Helpers, Lock};

    const BUS: &str = "org.mpris.MediaPlayer2.mpv.instance7";
    const OWNER: &str = ":1.7";
    const KEY_TAB: u32 = 15;
    const KEY_ENTER: u32 = 28;
    const KEY_A: u32 = 30;

    /// One player, `status`, with Previous live as `previous` says, playing "Song" by "Band" from
    /// a local file, with art on the network and an identity of its own.
    fn players(status: &str, previous: bool) -> Players {
        let mut p = Players::default();
        add(&mut p, BUS, OWNER, "Song", status, previous);
        p
    }

    /// Player `bus`, owned by `owner`, `status` and playing `title` by "Band"; Previous live as
    /// `previous` says.
    fn add(p: &mut Players, bus: &str, owner: &str, title: &str, status: &str, previous: bool) {
        let meta: HashMap<String, Variant<'static>> = HashMap::from([
            ("xesam:title".to_string(), Variant::from(title.to_string())),
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
        let about = About {
            identity: "Hidden Player".into(),
            desktop_entry: "hidden".into(),
            owner: owner.into(),
            pid: Some(7),
            app: None,
        };
        p.add(bus, &reprisebus::bounded::decode(&msg).unwrap(), about);
    }

    /// Player `bus` turns `status`.
    fn turn(p: &mut Players, bus: &str, status: &str) {
        p.changed(bus, &crate::previewmedia::tests::status(status, true));
    }

    fn strip_title(h: &Harness) -> Option<String> {
        lock(h).view.model.media.as_ref().map(|np| np.title.clone())
    }

    /// The bus a media key's PlayPause goes to.
    fn key_target(h: &Harness) -> Option<String> {
        match h.state.player_job("PlayPause")? {
            crate::media::Job::Player { bus, .. } => Some(bus),
            _ => None,
        }
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
    fn the_strips_buttons_call_only_the_players_previous_play_pause_and_next_by_its_unique_name() {
        let mut h = locked(Some(players("Playing", true)));
        for t in Transport::ALL {
            click(&mut h, Part::Media(t));
        }
        let bus = OWNER.to_string();
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
    fn tab_reaches_the_buttons_after_the_screens_own_and_enter_presses_them() {
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
        let tree = crate::lockview::access(&h.state).expect("the lock's tree");
        assert_eq!(tree.find(Role::Button, "Pause"), Some(tree.focus()));
        tap(&mut h, KEY_ENTER);
        assert_eq!(sent(&h), [(OWNER.to_string(), "PlayPause")]);
        untouched(&h);
    }

    #[test]
    fn typing_on_a_focused_media_button_types_into_the_password() {
        let mut h = locked(Some(players("Playing", true)));
        for _ in 0..lock(&h).view.model.stops().len() - 2 {
            tap(&mut h, KEY_TAB);
        }
        assert_eq!(lock(&h).view.model.focus, Part::Media(Transport::PlayPause));
        tap(&mut h, KEY_A);
        let l = lock(&h);
        assert_eq!((l.view.model.focus, l.input()), (Part::Password, "a"));
        tap(&mut h, KEY_ENTER);
        assert!(sent(&h).is_empty(), "Enter unlocks, it does not play");
        let m = &lock(&h).view.model;
        assert!(
            m.busy || m.message.is_some(),
            "the password went to the check"
        );
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
        assert!(super::shown(&h.state, Side::Remote).is_none());
        super::press(&mut h.state, Side::Remote, Transport::PlayPause);
        assert!(sent(&h).is_empty());
    }

    #[test]
    fn in_lock_mode_the_consoles_lock_shows_no_player_and_presses_nothing() {
        let mut h = locked(Some(players("Playing", true)));
        assert!(strip_title(&h).is_some());
        let mut portal = h.connect_portal();
        let remote = portal.lock_screen();
        remote.set_layout(&[(0, 0, 1280, 720, 100)]);
        h.roundtrip(&mut portal);
        assert!(crate::console::remote_side(&h.state));
        let l = lock(&h);
        assert_eq!((&l.media, &l.view.model.media), (&None, &None));
        assert_eq!(l.view.layout.as_ref().and_then(|l| l.media), None);
        assert!(super::shown(&h.state, Side::Console).is_none());
        super::press(&mut h.state, Side::Console, Transport::PlayPause);
        assert!(sent(&h).is_empty());
    }

    #[test]
    fn the_strip_and_the_media_keys_follow_the_same_player() {
        let mut p = Players::default();
        add(
            &mut p,
            "org.mpris.MediaPlayer2.a",
            ":1.10",
            "A",
            "Playing",
            true,
        );
        add(
            &mut p,
            "org.mpris.MediaPlayer2.b",
            ":1.11",
            "B",
            "Paused",
            true,
        );
        let mut h = locked(Some(p.clone()));
        let agree = |h: &Harness, title: &str, owner: &str| {
            assert_eq!(strip_title(h).as_deref(), Some(title));
            assert_eq!(
                key_target(h).as_deref(),
                Some(owner),
                "the key goes to {title}"
            );
        };
        agree(&h, "A", ":1.10");
        let mut set = |h: &mut Harness, bus: &str, status: &str| {
            turn(&mut p, bus, status);
            h.state.preview_players.players = p.clone();
            crate::lockview::update(&mut h.state);
        };
        set(&mut h, "org.mpris.MediaPlayer2.b", "Playing");
        agree(&h, "B", ":1.11");
        set(&mut h, "org.mpris.MediaPlayer2.b", "Paused");
        agree(&h, "A", ":1.10");
        set(&mut h, "org.mpris.MediaPlayer2.a", "Paused");
        agree(&h, "B", ":1.11");
        set(&mut h, "org.mpris.MediaPlayer2.b", "Stopped");
        set(&mut h, "org.mpris.MediaPlayer2.a", "Stopped");
        assert_eq!(strip_title(&h), None, "a stopped player shows no strip");
        assert!(key_target(&h).is_some(), "but takes the keys");
    }

    #[test]
    fn a_press_goes_to_the_player_the_strip_showed() {
        let mut h = locked(Some(players("Playing", true)));
        let mut p = players("Playing", true);
        add(
            &mut p,
            "org.mpris.MediaPlayer2.other",
            ":1.20",
            "Other",
            "Playing",
            true,
        );
        h.state.preview_players.players = p;
        click(&mut h, Part::Media(Transport::PlayPause));
        assert_eq!(sent(&h), [(OWNER.to_string(), "PlayPause")]);
    }

    #[test]
    fn only_a_change_the_strip_shows_redraws_the_lock() {
        let mut h = locked(Some(players("Playing", true)));
        assert!(!super::moved(&h.state));
        let mut p = players("Playing", true);
        add(
            &mut p,
            "org.mpris.MediaPlayer2.idle",
            ":1.30",
            "Idle",
            "Stopped",
            true,
        );
        h.state.preview_players.players = p.clone();
        assert!(!super::moved(&h.state), "a stopped player is not shown");
        turn(&mut p, BUS, "Paused");
        h.state.preview_players.players = p;
        assert!(super::moved(&h.state), "Pause turns to Play");
    }
}
