//! Which audio devices count as connecting, and the defaults `[sounds] on_connect` gives them.
//!
//! A device is a card (`Audio/Device`). It connects when a card appears that the last snapshot
//! did not have, by `object.serial`, so one that drops and comes back connects again; setting
//! the same default twice changes nothing, so there is no debounce. A card whose profile was
//! `off` connects when it turns on by itself (a monitor plugged into an HDMI port), not when
//! the sheet's Enable turned it on. A profile switch between two live profiles (a headset
//! moving to its headset profile) is not a connect, and nodes without a card (null sinks,
//! filters, loopbacks) never are. During the first [`SETTLING`] of a `pw-dump` run every card
//! counts as already there: the session is starting or PipeWire restarted.
//!
//! A connected card's first sink becomes the default output once it appears, within [`WAIT`];
//! with `PlaybackAndRecording` its first source becomes the default input. With `Playback`, a
//! default input WirePlumber moves onto the card's source by priority is put back, so nothing
//! records from a Bluetooth headset and it stays on its playback profile.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use repriseconf::config::OnConnect;

use crate::pipewire::{Device, Snapshot};

/// How long after a `pw-dump` run starts the cards it reports count as already there.
pub const SETTLING: Duration = Duration::from_secs(5);
/// How long a connected card's nodes have to appear.
pub const WAIT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    serial: Option<u64>,
    off: bool,
}

#[derive(Debug)]
struct Arrival {
    card: u32,
    serial: Option<u64>,
    at: Instant,
    /// The default input before the card came, which `Playback` keeps.
    input: Option<String>,
    output_set: bool,
    input_set: bool,
}

/// A default to set: `id` is the node, `name` its `node.name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetDefault {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Default)]
pub struct Arrivals {
    /// The `pw-dump` run the snapshots come from and when its first one came.
    run: Option<(u32, Instant)>,
    cards: BTreeMap<u32, Seen>,
    input: Option<String>,
    arrivals: Vec<Arrival>,
    /// Cards the sheet is switching on.
    asked: BTreeSet<u32>,
}

impl Arrivals {
    /// Card `id` is being switched on from here, which is no connect.
    pub fn asked(&mut self, id: u32) {
        self.asked.insert(id);
    }

    /// Take the next snapshot of `pw-dump` run `run`: the defaults to set for `choice`.
    pub fn update(
        &mut self,
        snap: &Snapshot,
        run: u32,
        choice: OnConnect,
        now: Instant,
    ) -> Vec<SetDefault> {
        if self.run.is_none_or(|(r, _)| r != run) {
            self.run = Some((run, now));
            self.arrivals.clear();
        }
        let settled = self.run.is_some_and(|(_, at)| now >= at + SETTLING);
        for c in &snap.cards {
            let before = self.cards.get(&c.id).filter(|s| s.serial == c.serial);
            let turned_on = before.is_some_and(|s| s.off && !c.off);
            let asked = turned_on && self.asked.remove(&c.id);
            let waiting = (self.arrivals.iter()).any(|a| a.card == c.id && a.serial == c.serial);
            if settled && (before.is_none() || turned_on && !asked) && !waiting {
                self.arrivals.push(Arrival {
                    card: c.id,
                    serial: c.serial,
                    at: now,
                    input: self.input.clone(),
                    output_set: false,
                    input_set: false,
                });
            }
        }
        self.asked
            .retain(|id| snap.cards.iter().any(|c| c.id == *id && c.off));
        self.arrivals.retain(|a| {
            now < a.at + WAIT && (snap.cards.iter()).any(|c| c.id == a.card && c.serial == a.serial)
        });
        self.cards = (snap.cards.iter())
            .map(|c| {
                let seen = Seen {
                    serial: c.serial,
                    off: c.off,
                };
                (c.id, seen)
            })
            .collect();
        self.input.clone_from(&snap.default_source);

        let mut out = Vec::new();
        if choice == OnConnect::Keep {
            self.arrivals.clear();
            return out;
        }
        for a in &mut self.arrivals {
            if !a.output_set
                && let Some(d) = of_card(&snap.sinks, a.card)
            {
                out.push(set(d));
                a.output_set = true;
            }
            if a.input_set {
                continue;
            }
            let Some(d) = of_card(&snap.sources, a.card) else {
                continue;
            };
            match choice {
                OnConnect::PlaybackAndRecording => {
                    out.push(set(d));
                    a.input_set = true;
                }
                OnConnect::Playback => {
                    let moved_here = snap.default_source.as_deref() == Some(&d.name)
                        && snap.configured_source.as_deref() != Some(&d.name);
                    let back = (a.input.as_deref())
                        .filter(|n| *n != d.name)
                        .and_then(|n| snap.sources.iter().find(|s| s.name == n));
                    if moved_here && let Some(back) = back {
                        out.push(set(back));
                        a.input_set = true;
                    }
                }
                OnConnect::Keep => {}
            }
        }
        out
    }
}

fn of_card(list: &[Device], card: u32) -> Option<&Device> {
    list.iter().find(|d| d.card == Some(card))
}

fn set(d: &Device) -> SetDefault {
    SetDefault {
        id: d.id,
        name: d.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipewire::Card;

    fn dev(id: u32, name: &str, card: Option<u32>) -> Device {
        Device {
            id,
            name: name.into(),
            serial: Some(id.into()),
            label: name.into(),
            volume: 0.5,
            mute: false,
            card,
            channels: Default::default(),
            route: None,
        }
    }

    fn card(id: u32, serial: u64, off: bool) -> Card {
        Card {
            id,
            serial: Some(serial),
            label: format!("card {id}"),
            off,
            sinks: true,
            sources: true,
            off_index: Some(0),
            best: Some(1),
        }
    }

    /// The built-in card 40 with its speaker and mic, both defaults, and a null sink.
    fn start() -> Snapshot {
        Snapshot {
            sinks: vec![dev(50, "null", None), dev(51, "speaker", Some(40))],
            sources: vec![dev(60, "mic", Some(40))],
            default_sink: Some("speaker".into()),
            default_source: Some("mic".into()),
            configured_sink: Some("speaker".into()),
            configured_source: None,
            cards: vec![card(40, 400, false)],
            apps: Vec::new(),
        }
    }

    /// `start` with headset card 41 (serial `serial`): its sink and, with `mic`, its source.
    fn headset(serial: u64, mic: bool) -> Snapshot {
        let mut s = start();
        s.cards.push(card(41, serial, false));
        s.sinks.push(dev(70, "headset_out", Some(41)));
        if mic {
            s.sources.push(dev(71, "headset_in", Some(41)));
        }
        s
    }

    fn names(sets: &[SetDefault]) -> Vec<&str> {
        sets.iter().map(|s| s.name.as_str()).collect()
    }

    /// A tracker that has seen `start` and settled.
    fn settled(t0: Instant) -> Arrivals {
        let mut a = Arrivals::default();
        assert!(a.update(&start(), 1, OnConnect::Playback, t0).is_empty());
        a
    }

    fn later(t0: Instant, secs: u64) -> Instant {
        t0 + SETTLING + Duration::from_secs(secs)
    }

    #[test]
    fn a_new_card_takes_playback_and_leaves_the_input() {
        let t0 = Instant::now();
        let mut a = settled(t0);
        let sets = a.update(&headset(410, true), 1, OnConnect::Playback, later(t0, 0));
        assert_eq!(
            sets,
            [SetDefault {
                id: 70,
                name: "headset_out".into()
            }]
        );
    }

    #[test]
    fn playback_and_recording_also_takes_the_input() {
        let t0 = Instant::now();
        let mut a = settled(t0);
        let choice = OnConnect::PlaybackAndRecording;
        let sets = a.update(&headset(410, true), 1, choice, later(t0, 0));
        assert_eq!(names(&sets), ["headset_out", "headset_in"]);
    }

    #[test]
    fn keep_sets_nothing() {
        let t0 = Instant::now();
        let mut a = settled(t0);
        let sets = a.update(&headset(410, true), 1, OnConnect::Keep, later(t0, 0));
        assert!(sets.is_empty());
    }

    #[test]
    fn the_nodes_may_come_after_the_card() {
        let t0 = Instant::now();
        let mut a = settled(t0);
        let mut bare = start();
        bare.cards.push(card(41, 410, false));
        let choice = OnConnect::PlaybackAndRecording;
        assert!(a.update(&bare, 1, choice, later(t0, 0)).is_empty());
        let out_only = {
            let mut s = bare.clone();
            s.sinks.push(dev(70, "headset_out", Some(41)));
            s
        };
        let sets = a.update(&out_only, 1, choice, later(t0, 1));
        assert_eq!(names(&sets), ["headset_out"]);
        let sets = a.update(&headset(410, true), 1, choice, later(t0, 2));
        assert_eq!(names(&sets), ["headset_in"], "the output is set once");
        assert!(
            a.update(&headset(410, true), 1, choice, later(t0, 3))
                .is_empty()
        );
    }

    #[test]
    fn nodes_that_come_too_late_are_left_alone() {
        let t0 = Instant::now();
        let mut a = settled(t0);
        let mut bare = start();
        bare.cards.push(card(41, 410, false));
        a.update(&bare, 1, OnConnect::Playback, later(t0, 0));
        let late = later(t0, WAIT.as_secs());
        assert!(
            a.update(&headset(410, true), 1, OnConnect::Playback, late)
                .is_empty()
        );
    }

    #[test]
    fn cards_there_while_the_run_settles_are_not_new() {
        let t0 = Instant::now();
        let mut a = Arrivals::default();
        assert!(a.update(&start(), 1, OnConnect::Playback, t0).is_empty());
        let early = t0 + SETTLING - Duration::from_millis(1);
        assert!(
            a.update(&headset(410, true), 1, OnConnect::Playback, early)
                .is_empty()
        );
        assert!(
            a.update(&headset(410, true), 1, OnConnect::Playback, later(t0, 0))
                .is_empty()
        );
    }

    #[test]
    fn a_restarted_pipewire_reports_its_cards_again_without_a_connect() {
        let t0 = Instant::now();
        let mut a = settled(t0);
        a.update(&headset(410, false), 1, OnConnect::Playback, later(t0, 0));
        let mut again = headset(900, false);
        again.cards[0].serial = Some(901);
        let t1 = later(t0, 60);
        assert!(a.update(&again, 2, OnConnect::Playback, t1).is_empty());
        let t2 = t1 + SETTLING;
        assert!(a.update(&again, 2, OnConnect::Playback, t2).is_empty());
    }

    #[test]
    fn a_card_that_drops_and_comes_back_connects_again() {
        let t0 = Instant::now();
        let mut a = settled(t0);
        let choice = OnConnect::Playback;
        a.update(&headset(410, false), 1, choice, later(t0, 0));
        a.update(&start(), 1, choice, later(t0, 1));
        let sets = a.update(&headset(420, false), 1, choice, later(t0, 2));
        assert_eq!(names(&sets), ["headset_out"]);
        let sets = a.update(&headset(430, false), 1, choice, later(t0, 3));
        assert_eq!(
            names(&sets),
            ["headset_out"],
            "a drop shorter than one snapshot still shows as a new serial"
        );
    }

    #[test]
    fn a_profile_switch_is_not_a_connect() {
        let t0 = Instant::now();
        let mut a = settled(t0);
        let mut hfp = start();
        hfp.sinks[1] = dev(52, "speaker_hdmi", Some(40));
        assert!(
            a.update(&hfp, 1, OnConnect::Playback, later(t0, 0))
                .is_empty()
        );
        let mut gap = start();
        gap.sinks.truncate(1);
        gap.sources.clear();
        assert!(
            a.update(&gap, 1, OnConnect::Playback, later(t0, 1))
                .is_empty()
        );
        assert!(
            a.update(&start(), 1, OnConnect::Playback, later(t0, 2))
                .is_empty()
        );
    }

    #[test]
    fn a_card_turning_on_by_itself_connects_but_not_one_the_sheet_enabled() {
        let t0 = Instant::now();
        let mut off = start();
        off.cards.push(card(41, 410, true));
        let mut a = Arrivals::default();
        a.update(&off, 1, OnConnect::Playback, t0);
        let sets = a.update(&headset(410, false), 1, OnConnect::Playback, later(t0, 0));
        assert_eq!(names(&sets), ["headset_out"], "a monitor plugged into HDMI");

        let mut a = Arrivals::default();
        a.update(&off, 1, OnConnect::Playback, t0);
        a.asked(41);
        let sets = a.update(&headset(410, false), 1, OnConnect::Playback, later(t0, 0));
        assert!(sets.is_empty());
    }

    #[test]
    fn a_node_without_a_card_never_connects() {
        let t0 = Instant::now();
        let mut a = settled(t0);
        let mut null = start();
        null.sinks.push(dev(80, "virtual", None));
        null.sources.push(dev(81, "loopback", None));
        let choice = OnConnect::PlaybackAndRecording;
        assert!(a.update(&null, 1, choice, later(t0, 0)).is_empty());
    }

    #[test]
    fn playback_puts_back_an_input_wireplumber_moved_to_the_headset() {
        let t0 = Instant::now();
        let mut a = settled(t0);
        let sets = a.update(&headset(410, true), 1, OnConnect::Playback, later(t0, 0));
        assert_eq!(names(&sets), ["headset_out"]);
        let mut moved = headset(410, true);
        moved.default_source = Some("headset_in".into());
        let sets = a.update(&moved, 1, OnConnect::Playback, later(t0, 1));
        assert_eq!(
            sets,
            [SetDefault {
                id: 60,
                name: "mic".into()
            }]
        );
        assert!(
            a.update(&moved, 1, OnConnect::Playback, later(t0, 2))
                .is_empty()
        );
    }

    #[test]
    fn playback_leaves_an_input_picked_with_set_default() {
        let t0 = Instant::now();
        let mut a = settled(t0);
        a.update(&headset(410, true), 1, OnConnect::Playback, later(t0, 0));
        let mut picked = headset(410, true);
        picked.default_source = Some("headset_in".into());
        picked.configured_source = Some("headset_in".into());
        assert!(
            a.update(&picked, 1, OnConnect::Playback, later(t0, 1))
                .is_empty()
        );
    }
}
