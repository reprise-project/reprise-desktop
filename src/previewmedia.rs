//! The MPRIS players `reprisebus::mpris` follows, which window each plays for, and the calls the
//! task preview's media buttons and the lock's strip (`lockmedia`) make. A window's player is one its own process owns;
//! else, of the players whose process shows no window, one whose process runs as the window's app
//! (its Flatpak or snap id, or the Reprise app scope it was started in), else one whose
//! `DesktopEntry` names that app. Of several, Playing beats Paused beats Stopped, then the one
//! that played last.

use std::cmp::Reverse;

use reprisebus::mpris::{Player, Players, Status, Watch};
use reprisekit::controls::volumepopup::{NowPlaying, Transport};
use smithay::reexports::calloop::{LoopHandle, ping::make_ping};

use crate::compositor::State;
use crate::window::WindowId;

#[derive(Default)]
pub struct PreviewPlayers {
    pub players: Players,
    watch: Option<Watch>,
    /// The presses `send` made, by bus name and method.
    #[cfg(test)]
    pub sent: Vec<(String, &'static str)>,
}

impl PreviewPlayers {
    pub fn start(handle: &LoopHandle<'static, State>) -> PreviewPlayers {
        let (ping, source) = match make_ping() {
            Ok(p) => p,
            Err(e) => {
                log::warn!("preview players: no ping: {e}");
                return PreviewPlayers::default();
            }
        };
        if let Err(e) = handle.insert_source(source, |_, _, state| changed(state)) {
            log::warn!("preview players: no source: {}", e.error);
            return PreviewPlayers::default();
        }
        PreviewPlayers {
            watch: Some(reprisebus::mpris::watch("preview-mpris", move || {
                ping.ping()
            })),
            ..PreviewPlayers::default()
        }
    }
}

fn changed(state: &mut State) {
    let Some(w) = &state.preview_players.watch else {
        return;
    };
    let Some(players) = w.players.try_iter().last() else {
        return;
    };
    state.preview_players.players = players;
    crate::taskpreview::players_changed(state);
    if crate::lockmedia::moved(state) {
        crate::lockview::update(state);
    }
    if state.switcher.panel.is_some() {
        state.redraw_switcher();
    }
}

/// The player for a window whose client is process `pid` and whose app goes by `keys`
/// (`launcherentry::app_keys`), by the module's rule; `owns_window` tells a process showing a
/// window.
pub fn pick<'a>(
    players: &'a Players,
    pid: Option<u32>,
    keys: &[String],
    owns_window: impl Fn(u32) -> bool,
) -> Option<(&'a str, &'a Player)> {
    let names = |id: &str| {
        let files = reprisefs::apps::AppId::new(id).map(|a| a.desktop_ids());
        (files.iter().flatten()).any(|d| keys.contains(&crate::launcherentry::key(&d.app_id())))
    };
    let rank = |p: &Player| {
        if pid.is_some() && p.pid == pid {
            return Some(0);
        }
        if p.pid.is_some_and(&owns_window) {
            return None;
        }
        if p.app.as_deref().is_some_and(names) {
            return Some(1);
        }
        names(&p.desktop_entry).then_some(2)
    };
    let status = |p: &Player| match p.status {
        Status::Playing => 0,
        Status::Paused => 1,
        Status::Stopped => 2,
    };
    (players.iter())
        .filter_map(|(n, p)| Some((rank(p)?, n, p)))
        .min_by_key(|&(r, _, p)| (r, status(p), Reverse(p.active)))
        .map(|(_, n, p)| (n, p))
}

/// The bus name of each of `windows`' players and what its buttons show.
pub fn players_of(state: &State, windows: &[WindowId]) -> Vec<Option<(String, NowPlaying)>> {
    (picked(state, windows).into_iter())
        .map(|p| p.map(|(name, p)| (name.to_string(), now_playing(p))))
        .collect()
}

/// Window `w`'s player's state, if it has a player.
pub fn status_of(state: &State, w: WindowId) -> Option<Status> {
    picked(state, &[w])[0].map(|(_, p)| p.status)
}

/// Each of `windows`' players by the module's rule.
fn picked<'a>(state: &'a State, windows: &[WindowId]) -> Vec<Option<(&'a str, &'a Player)>> {
    let players = &state.preview_players.players;
    if players.iter().next().is_none() {
        return vec![None; windows.len()];
    }
    let pid = |w| state.client_pid(w).and_then(|p| u32::try_from(p).ok());
    let shown: Vec<u32> = state.windows.iter().filter_map(|w| pid(w.id)).collect();
    let player = |w: WindowId| {
        let app = state.win(w)?.app_id();
        let keys = crate::launcherentry::app_keys(&app, None, &state.app_index);
        pick(players, pid(w), &keys, |p| shown.contains(&p))
    };
    windows.iter().map(|&w| player(w)).collect()
}

/// What `p`'s buttons and strip show: its title and artist as it sent them, its state and what
/// it can do.
pub fn now_playing(p: &Player) -> NowPlaying {
    NowPlaying {
        title: p.title.clone(),
        artist: p.artist.clone(),
        playing: p.status == Status::Playing,
        can_previous: p.can_previous(),
        can_next: p.can_next(),
        can_play: p.can_play(),
        can_pause: p.can_pause(),
    }
}

/// Press button `t` of player `bus`, off the event loop.
pub fn send(state: &mut State, bus: &str, t: Transport) {
    let method = match t {
        Transport::Previous => "Previous",
        Transport::PlayPause => "PlayPause",
        Transport::Next => "Next",
    };
    #[cfg(test)]
    state.preview_players.sent.push((bus.to_string(), method));
    if let Some(w) = &state.preview_players.watch {
        w.send(bus, method);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use reprisebus::mpris::About;

    /// A player's properties: `status`, every transport button live but Previous unless `previous`.
    pub(crate) fn status(status: &str, previous: bool) -> reprisebus::bounded::Dict {
        let map = std::collections::HashMap::from([
            ("PlaybackStatus", zbus::Variant::from(status)),
            ("CanGoPrevious", zbus::Variant::from(previous)),
            ("CanGoNext", zbus::Variant::from(true)),
            ("CanPlay", zbus::Variant::from(true)),
            ("CanPause", zbus::Variant::from(true)),
        ]);
        let msg = zbus::Message::signal("/", "org.freedesktop.DBus.Properties", "Changed")
            .build(&map)
            .unwrap();
        reprisebus::bounded::decode(&msg).unwrap()
    }

    fn about(pid: u32, app: Option<&str>, entry: &str) -> About {
        About {
            pid: Some(pid),
            app: app.map(str::to_string),
            desktop_entry: entry.to_string(),
            ..About::default()
        }
    }

    fn players(list: &[(&str, About)]) -> Players {
        let mut p = Players::default();
        for (name, a) in list {
            p.add(name, &Default::default(), a.clone());
        }
        p
    }

    fn picked(p: &Players, pid: u32, keys: &[&str], windowed: &[u32]) -> Option<String> {
        let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        pick(p, Some(pid), &keys, |q| windowed.contains(&q)).map(|(n, _)| n.to_string())
    }

    #[test]
    fn a_windows_own_process_owns_its_player() {
        let p = players(&[
            (
                "org.mpris.MediaPlayer2.mpv.instance10",
                about(10, None, "mpv"),
            ),
            (
                "org.mpris.MediaPlayer2.mpv.instance20",
                about(20, None, "mpv"),
            ),
        ]);
        assert_eq!(
            picked(&p, 20, &["mpv"], &[10, 20]).as_deref(),
            Some("org.mpris.MediaPlayer2.mpv.instance20"),
            "the other mpv's player has a window of its own"
        );
        assert_eq!(picked(&p, 30, &["mpv"], &[10, 20, 30]), None);
    }

    #[test]
    fn a_windowless_player_goes_to_its_apps_windows_by_sandbox_then_desktop_entry() {
        let p = players(&[
            (
                "org.mpris.MediaPlayer2.spotify",
                about(50, Some("com.spotify.Client"), "spotify"),
            ),
            (
                "org.mpris.MediaPlayer2.music",
                about(60, None, "org.example.Music"),
            ),
        ]);
        assert_eq!(
            picked(&p, 5, &["com.spotify.client"], &[5]).as_deref(),
            Some("org.mpris.MediaPlayer2.spotify"),
            "the Flatpak's proxy plays for the app's window"
        );
        assert_eq!(
            picked(&p, 6, &["org.example.music"], &[6]).as_deref(),
            Some("org.mpris.MediaPlayer2.music"),
            "by the DesktopEntry it names"
        );
        assert_eq!(
            picked(&p, 6, &["org.example.music"], &[6, 60]),
            None,
            "a player whose process shows a window plays for that window alone"
        );
    }

    #[test]
    fn of_several_players_the_playing_one_wins() {
        let mut p = players(&[
            ("org.mpris.MediaPlayer2.a", about(7, None, "")),
            ("org.mpris.MediaPlayer2.b", about(7, None, "")),
        ]);
        p.changed("org.mpris.MediaPlayer2.a", &status("Paused", true));
        assert_eq!(
            picked(&p, 7, &[], &[7]).as_deref(),
            Some("org.mpris.MediaPlayer2.a"),
            "paused beats stopped"
        );
        p.changed("org.mpris.MediaPlayer2.b", &status("Playing", true));
        assert_eq!(
            picked(&p, 7, &[], &[7]).as_deref(),
            Some("org.mpris.MediaPlayer2.b")
        );
    }
}
