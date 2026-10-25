//! What the mixer shows for a snapshot: the Playback or Recording lines, which ones the Properties dialog hid, and the device the status bar names. Nothing here draws or writes.

use std::collections::BTreeSet;

use repriseaudio::{Channels, Device, Snapshot};
use reprisekit::controls::mixer::{self, Check, Line};

pub const MASTER: &str = "master";
pub const MASTER_TITLE: &str = "Volume Control";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Playback,
    Recording,
}

impl View {
    pub fn title(self) -> &'static str {
        match self {
            View::Playback => "Volume Control",
            View::Recording => "Recording Control",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub view: View,
    /// The output device Playback's first line sets, by `node.name`; `None` follows the default.
    pub device: Option<String>,
    pub hidden: BTreeSet<String>,
    pub advanced: bool,
}

impl Options {
    pub fn new(view: View) -> Options {
        Options {
            view,
            device: None,
            hidden: BTreeSet::new(),
            advanced: false,
        }
    }
}

pub fn app_key(key: &str) -> String {
    format!("app:{key}")
}

pub fn source_key(name: &str) -> String {
    format!("source:{name}")
}

/// The output device Playback sets: `device`, else the default, else the first.
pub fn sink<'a>(snap: &'a Snapshot, device: Option<&str>) -> Option<&'a Device> {
    let named = |n: Option<&str>| n.and_then(|n| snap.sinks.iter().find(|d| d.name == n));
    named(device)
        .or_else(|| named(snap.default_sink.as_deref()))
        .or_else(|| snap.sinks.first())
}

pub fn source(snap: &Snapshot) -> Option<&Device> {
    let default = snap.default_source.as_deref();
    snap.sources
        .iter()
        .find(|d| Some(d.name.as_str()) == default)
        .or_else(|| snap.sources.first())
}

pub fn volume_pos(v: f64) -> usize {
    let last = (mixer::STEPS - 1) as f64;
    (v * last).round().clamp(0.0, last) as usize
}

pub fn volume_of(pos: usize) -> f64 {
    pos as f64 / (mixer::STEPS - 1) as f64
}

fn balance(ch: &Channels) -> Option<usize> {
    ch.stereo().then(|| mixer::balance_pos(ch.balance()))
}

pub fn all_lines(snap: &Snapshot, o: &Options) -> Vec<Line> {
    match o.view {
        View::Playback => {
            let out = sink(snap, o.device.as_deref());
            let mut v = vec![Line {
                key: MASTER.into(),
                title: MASTER_TITLE.into(),
                volume: out.map_or(0, |d| volume_pos(d.volume)),
                balance: out.and_then(|d| balance(&d.channels)),
                check: Check::MuteAll,
                checked: out.is_some_and(|d| d.mute),
                live: out.is_some(),
            }];
            v.extend(snap.apps.iter().map(|a| {
                // The column stands for the program's loudest stream.
                let loudest = a
                    .streams
                    .iter()
                    .max_by(|x, y| x.channels.volume().total_cmp(&y.channels.volume()));
                Line {
                    key: app_key(&a.key),
                    title: a.label.clone(),
                    volume: volume_pos(a.volume),
                    balance: loudest.and_then(|s| balance(&s.channels)),
                    check: Check::Mute,
                    checked: a.mute,
                    live: true,
                }
            }));
            v
        }
        View::Recording => {
            let picked = source(snap).map(|d| d.name.as_str());
            snap.sources
                .iter()
                .map(|d| Line {
                    key: source_key(&d.name),
                    title: d.label.clone(),
                    volume: volume_pos(d.volume),
                    balance: balance(&d.channels),
                    check: Check::Select,
                    checked: Some(d.name.as_str()) == picked,
                    live: true,
                })
                .collect()
        }
    }
}

/// The lines the mixer shows: [`all_lines`] less the hidden ones, with each recent write standing in for the snapshot until it settles.
pub fn lines(snap: &Snapshot, o: &Options, recent: &Recent) -> Vec<Line> {
    let mut v: Vec<Line> = all_lines(snap, o)
        .into_iter()
        .filter(|l| !o.hidden.contains(&l.key))
        .collect();
    for l in &mut v {
        if let Some((volume, balance)) = recent.get(&l.key) {
            l.volume = volume;
            if l.balance.is_some() {
                l.balance = Some(balance);
            }
        }
    }
    v
}

pub fn status(snap: &Snapshot, o: &Options) -> String {
    let d = match o.view {
        View::Playback => sink(snap, o.device.as_deref()),
        View::Recording => source(snap),
    };
    d.map(|d| d.label.clone()).unwrap_or_default()
}

pub type Recent = repriseaudio::Recent<String, (usize, usize)>;

pub fn nodes<'a>(snap: &'a Snapshot, o: &Options, key: &str) -> Vec<(u32, &'a Channels)> {
    if key == MASTER {
        return sink(snap, o.device.as_deref())
            .map(|d| vec![(d.id, &d.channels)])
            .unwrap_or_default();
    }
    if let Some(k) = key.strip_prefix("app:") {
        return snap
            .apps
            .iter()
            .find(|a| a.key == k)
            .map(|a| a.streams.iter().map(|s| (s.id, &s.channels)).collect())
            .unwrap_or_default();
    }
    if let Some(n) = key.strip_prefix("source:") {
        return snap
            .sources
            .iter()
            .filter(|d| d.name == n)
            .map(|d| (d.id, &d.channels))
            .collect();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriseaudio::{App, Stream};
    use std::time::Instant;

    fn dev(id: u32, name: &str, levels: &[f64]) -> Device {
        let channels = Channels {
            map: match levels.len() {
                2 => vec!["FL".into(), "FR".into()],
                _ => vec!["MONO".into(); levels.len()],
            },
            levels: levels.to_vec(),
            shares: Vec::new(),
        };
        Device {
            id,
            name: name.into(),
            label: format!("{name} label"),
            volume: channels.volume(),
            mute: false,
            card: None,
            channels,
            route: None,
        }
    }

    fn snap() -> Snapshot {
        let stream = |id, levels: &[f64]| Stream {
            id,
            channels: Channels {
                map: vec!["FL".into(), "FR".into()],
                levels: levels.to_vec(),
                shares: Vec::new(),
            },
        };
        Snapshot {
            sinks: vec![
                dev(40, "hdmi", &[0.5, 0.5]),
                dev(41, "speakers", &[0.8, 0.4]),
            ],
            sources: vec![dev(50, "mic", &[0.6]), dev(51, "line", &[0.3, 0.3])],
            default_sink: Some("speakers".into()),
            default_source: Some("line".into()),
            cards: Vec::new(),
            apps: vec![App {
                key: "firefox".into(),
                label: "Firefox".into(),
                icon_hints: Vec::new(),
                pids: Vec::new(),
                binary: None,
                streams: vec![stream(90, &[0.2, 0.2]), stream(91, &[0.6, 0.3])],
                volume: 0.6,
                mute: true,
            }],
        }
    }

    #[test]
    fn playback_is_the_default_output_then_each_program() {
        let o = Options::new(View::Playback);
        let ls = lines(&snap(), &o, &Recent::default());
        let keys: Vec<&str> = ls.iter().map(|l| l.key.as_str()).collect();
        assert_eq!(keys, ["master", "app:firefox"]);
        assert_eq!(ls[0].title, "Volume Control");
        assert_eq!(ls[0].volume, 48, "the speakers at 80 %");
        assert_eq!(ls[0].balance, Some(5), "the right at half the left");
        assert_eq!(ls[0].check, Check::MuteAll);
        assert_eq!(ls[1].title, "Firefox");
        assert_eq!(ls[1].balance, Some(5), "the loudest stream's balance");
        assert!(ls[1].checked, "muted");
        assert_eq!(status(&snap(), &o), "speakers label");
    }

    #[test]
    fn a_picked_output_device_takes_the_first_line() {
        let mut o = Options::new(View::Playback);
        o.device = Some("hdmi".into());
        let ls = lines(&snap(), &o, &Recent::default());
        assert_eq!((ls[0].volume, ls[0].balance), (30, Some(10)));
        assert_eq!(nodes(&snap(), &o, MASTER)[0].0, 40);
        assert_eq!(status(&snap(), &o), "hdmi label");
    }

    #[test]
    fn recording_lists_each_input_and_selects_the_default() {
        let o = Options::new(View::Recording);
        let ls = lines(&snap(), &o, &Recent::default());
        let keys: Vec<&str> = ls.iter().map(|l| l.key.as_str()).collect();
        assert_eq!(keys, ["source:mic", "source:line"]);
        assert!(ls.iter().all(|l| l.check == Check::Select));
        assert!(!ls[0].checked && ls[1].checked);
        assert_eq!(ls[0].balance, None, "a mono input has no balance");
        assert_eq!(status(&snap(), &o), "line label");
    }

    #[test]
    fn hidden_lines_are_left_out() {
        let mut o = Options::new(View::Playback);
        o.hidden.insert(app_key("firefox"));
        assert_eq!(lines(&snap(), &o, &Recent::default()).len(), 1);
        assert_eq!(all_lines(&snap(), &o).len(), 2, "the list still has it");
    }

    #[test]
    fn with_no_output_the_first_line_greys() {
        let mut s = snap();
        s.sinks.clear();
        let ls = lines(&s, &Options::new(View::Playback), &Recent::default());
        assert!(!ls[0].live);
        assert_eq!(status(&s, &Options::new(View::Playback)), "");
    }

    #[test]
    fn a_recent_write_stands_in_until_it_settles() {
        let mut r = Recent::default();
        r.note(MASTER.into(), (3, 14), Instant::now());
        let ls = lines(&snap(), &Options::new(View::Playback), &r);
        assert_eq!((ls[0].volume, ls[0].balance), (3, Some(14)));
        let mut old = Recent::default();
        old.note(
            MASTER.into(),
            (3, 14),
            Instant::now() - repriseaudio::SETTLE,
        );
        let ls = lines(&snap(), &Options::new(View::Playback), &old);
        assert_eq!(ls[0].volume, 48);
    }

    #[test]
    fn a_program_line_writes_to_each_stream() {
        let o = Options::new(View::Playback);
        let ids: Vec<u32> = nodes(&snap(), &o, &app_key("firefox"))
            .iter()
            .map(|n| n.0)
            .collect();
        assert_eq!(ids, [90, 91]);
        assert_eq!(nodes(&snap(), &o, &source_key("mic"))[0].0, 50);
        assert!(nodes(&snap(), &o, "app:gone").is_empty());
    }
}
