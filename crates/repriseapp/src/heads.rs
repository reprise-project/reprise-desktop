//! Output heads as wlr-output-management describes them, and the pure
//! state machine behind the protocol: its events come in as [`In`], and
//! [`Model`] answers with [`Out`]: the heads to announce, a configuration
//! to send, the answer to a program's request. `outputmgmt.rs` puts it on
//! the wire; nothing here holds a Wayland object.
//!
//! The protocol says whether a head's adaptive sync is on, not whether it
//! can be. The model asks per enabled head with a test-only configuration
//! turning it on, which a head without it fails, and announces the heads
//! after the answers. A test cancelled [`TRIES`] times leaves the answer
//! unknown; the next `done` asks that head again.
//!
//! One configuration is in flight at a time, built against the last
//! `done` serial and naming every head: a disabled one stays disabled, an
//! enabled one keeps what it runs except what the request sets. A request
//! may set several heads at once (an arrangement moves them together). One
//! cancelled because the outputs changed under it goes again with the new
//! serial, [`TRIES`] sends at most.

use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeadMode {
    pub width: i32,
    pub height: i32,
    /// In mHz; 0 when the compositor did not say.
    pub refresh_mhz: i32,
    pub preferred: bool,
}

impl HeadMode {
    /// Whether `other` is the same size and refresh, preferred or not.
    pub fn same(&self, other: &HeadMode) -> bool {
        (self.width, self.height, self.refresh_mhz)
            == (other.width, other.height, other.refresh_mhz)
    }
}

/// A head's rotation or flip: `wl_output.transform`, named the way
/// `[[outputs]] transform` spells it. Rotations are counter-clockwise, as
/// the protocol's are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum Transform {
    #[default]
    Normal = 0,
    Rotate90 = 1,
    Rotate180 = 2,
    Rotate270 = 3,
    Flipped = 4,
    Flipped90 = 5,
    Flipped180 = 6,
    Flipped270 = 7,
}

impl Transform {
    pub const ALL: [Transform; 8] = [
        Transform::Normal,
        Transform::Rotate90,
        Transform::Rotate180,
        Transform::Rotate270,
        Transform::Flipped,
        Transform::Flipped90,
        Transform::Flipped180,
        Transform::Flipped270,
    ];

    pub fn wl(self) -> u32 {
        self as u32
    }

    /// The transform for a `wl_output.transform` value; `Normal` past
    /// the eight.
    pub fn from_wl(v: u32) -> Transform {
        Transform::ALL.get(v as usize).copied().unwrap_or_default()
    }

    pub fn name(self) -> &'static str {
        match self {
            Transform::Normal => "normal",
            Transform::Rotate90 => "90",
            Transform::Rotate180 => "180",
            Transform::Rotate270 => "270",
            Transform::Flipped => "flipped",
            Transform::Flipped90 => "flipped-90",
            Transform::Flipped180 => "flipped-180",
            Transform::Flipped270 => "flipped-270",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AdaptiveSync {
    #[default]
    Unsupported,
    Off,
    On,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Head {
    /// The connector, "DP-1", which `[[outputs]] name` matches.
    pub name: String,
    pub description: String,
    /// Empty when the compositor did not say.
    pub make: String,
    pub model: String,
    /// In the compositor's order.
    pub modes: Vec<HeadMode>,
    /// Which of `modes` runs; None on a disabled head or a custom mode.
    pub current: Option<usize>,
    pub enabled: bool,
    pub position: (i32, i32),
    pub transform: Transform,
    pub scale: f64,
    pub adaptive_sync: AdaptiveSync,
}

impl Head {
    /// The mode it runs; None when `current` is unset or not in `modes`.
    pub fn current_mode(&self) -> Option<HeadMode> {
        self.current.and_then(|i| self.modes.get(i)).copied()
    }

    /// What it runs as a configuration that puts it back: its mode,
    /// transform, scale and place.
    pub fn state(&self) -> HeadConfig {
        HeadConfig {
            mode: self.current_mode(),
            transform: Some(self.transform),
            scale: Some(self.scale),
            adaptive_sync: None,
            position: Some(self.position),
        }
    }
}

/// What to set on a head; a None keeps what it runs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeadConfig {
    pub mode: Option<HeadMode>,
    pub transform: Option<Transform>,
    pub scale: Option<f64>,
    pub adaptive_sync: Option<bool>,
    /// The head's top-left in the layout.
    pub position: Option<(i32, i32)>,
}

pub type HeadSet = Vec<(String, HeadConfig)>;

/// The configurations a program asked for during one event
/// (`Ctx::configure_head`), each with the number its answer carries.
#[derive(Debug, Default)]
pub struct HeadRequests {
    next: u32,
    asked: Vec<(u32, HeadSet)>,
}

impl HeadRequests {
    pub fn configure(&mut self, name: &str, cfg: HeadConfig) -> u32 {
        self.configure_all(vec![(name.to_string(), cfg)])
    }

    /// Queue one configuration setting every head in `heads`; returns
    /// its number.
    pub fn configure_all(&mut self, heads: HeadSet) -> u32 {
        self.next = self.next.wrapping_add(1);
        self.asked.push((self.next, heads));
        self.next
    }

    pub fn take(&mut self) -> Vec<(u32, HeadSet)> {
        std::mem::take(&mut self.asked)
    }
}

pub type ObjId = u32;

/// A wlr-output-management event, its objects by id.
#[derive(Clone, Debug, PartialEq)]
pub enum In {
    Head(ObjId),
    Name(ObjId, String),
    Description(ObjId, String),
    Make(ObjId, String),
    Model(ObjId, String),
    /// A head, then its new mode.
    Mode(ObjId, ObjId),
    ModeSize(ObjId, i32, i32),
    ModeRefresh(ObjId, i32),
    ModePreferred(ObjId),
    ModeFinished(ObjId),
    Enabled(ObjId, bool),
    /// A head, then the mode it runs.
    CurrentMode(ObjId, ObjId),
    Position(ObjId, i32, i32),
    Transform(ObjId, u32),
    Scale(ObjId, f64),
    AdaptiveSync(ObjId, bool),
    HeadFinished(ObjId),
    Done(u32),
    /// A configuration's answer, by its [`Configure::id`].
    Succeeded(u32),
    Failed(u32),
    Cancelled(u32),
}

/// One head's part of a configuration: `enable_head`, with these set.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConfigHead {
    pub mode: Option<ObjId>,
    pub transform: Option<Transform>,
    pub scale: Option<f64>,
    pub adaptive_sync: Option<bool>,
    pub position: Option<(i32, i32)>,
}

/// A configuration to create, fill and send.
#[derive(Clone, Debug, PartialEq)]
pub struct Configure {
    /// Its answer comes back as `In::Succeeded(id)` and the like.
    pub id: u32,
    /// The `done` serial it is built against.
    pub serial: u32,
    /// `test` rather than `apply`.
    pub test: bool,
    /// Every head, `None` to disable it.
    pub heads: Vec<(ObjId, Option<ConfigHead>)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Heads(Vec<Head>),
    Applied { serial: u32, ok: bool },
    Configure(Configure),
}

/// How often a cancelled configuration is sent before it counts as failed.
const TRIES: u8 = 3;

#[derive(Clone, Debug, PartialEq)]
enum Job {
    Apply {
        serial: u32,
        heads: HeadSet,
        tries: u8,
    },
    Probe {
        head: ObjId,
        tries: u8,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Default)]
struct Draft {
    /// Everything but `modes`, `current` and `adaptive_sync`, which
    /// [`Model::snapshot`] fills in.
    head: Head,
    modes: Vec<ObjId>,
    current: Option<ObjId>,
    vrr_on: bool,
    /// None until answered.
    vrr_supported: Option<bool>,
}

#[derive(Debug)]
pub struct Model {
    version: u32,
    heads: Vec<(ObjId, Draft)>,
    modes: Vec<(ObjId, HeadMode)>,
    serial: Option<u32>,
    queue: VecDeque<Job>,
    flight: Option<(u32, Job)>,
    next_id: u32,
}

impl Model {
    /// A model for a manager bound at `version`; adaptive sync needs 4.
    pub fn new(version: u32) -> Model {
        Model {
            version,
            heads: Vec::new(),
            modes: Vec::new(),
            serial: None,
            queue: VecDeque::new(),
            flight: None,
            next_id: 0,
        }
    }

    fn draft(&mut self, h: ObjId) -> Option<&mut Draft> {
        self.heads
            .iter_mut()
            .find(|(id, _)| *id == h)
            .map(|(_, d)| d)
    }

    fn mode_mut(&mut self, m: ObjId) -> Option<&mut HeadMode> {
        self.modes
            .iter_mut()
            .find(|(id, _)| *id == m)
            .map(|(_, md)| md)
    }

    fn mode_of(&self, m: ObjId) -> Option<HeadMode> {
        self.modes
            .iter()
            .find(|(id, _)| *id == m)
            .map(|(_, md)| *md)
    }

    pub fn event(&mut self, ev: In) -> Vec<Out> {
        let mut out = Vec::new();
        match ev {
            In::Head(h) => self.heads.push((h, Draft::default())),
            In::Name(h, s) => {
                if let Some(d) = self.draft(h) {
                    d.head.name = s;
                }
            }
            In::Description(h, s) => {
                if let Some(d) = self.draft(h) {
                    d.head.description = s;
                }
            }
            In::Make(h, s) => {
                if let Some(d) = self.draft(h) {
                    d.head.make = s;
                }
            }
            In::Model(h, s) => {
                if let Some(d) = self.draft(h) {
                    d.head.model = s;
                }
            }
            In::Mode(h, m) => {
                self.modes.push((m, HeadMode::default()));
                if let Some(d) = self.draft(h) {
                    d.modes.push(m);
                }
            }
            In::ModeSize(m, w, h) => {
                if let Some(md) = self.mode_mut(m) {
                    (md.width, md.height) = (w, h);
                }
            }
            In::ModeRefresh(m, r) => {
                if let Some(md) = self.mode_mut(m) {
                    md.refresh_mhz = r;
                }
            }
            In::ModePreferred(m) => {
                if let Some(md) = self.mode_mut(m) {
                    md.preferred = true;
                }
            }
            In::ModeFinished(m) => {
                self.modes.retain(|(id, _)| *id != m);
                for (_, d) in &mut self.heads {
                    d.modes.retain(|id| *id != m);
                    if d.current == Some(m) {
                        d.current = None;
                    }
                }
            }
            In::Enabled(h, on) => {
                if let Some(d) = self.draft(h) {
                    d.head.enabled = on;
                }
            }
            In::CurrentMode(h, m) => {
                if let Some(d) = self.draft(h) {
                    d.current = Some(m);
                }
            }
            In::Position(h, x, y) => {
                if let Some(d) = self.draft(h) {
                    d.head.position = (x, y);
                }
            }
            In::Transform(h, t) => {
                if let Some(d) = self.draft(h) {
                    d.head.transform = Transform::from_wl(t);
                }
            }
            In::Scale(h, s) => {
                if let Some(d) = self.draft(h) {
                    d.head.scale = s;
                }
            }
            In::AdaptiveSync(h, on) => {
                if let Some(d) = self.draft(h) {
                    d.vrr_on = on;
                    if on {
                        d.vrr_supported = Some(true);
                    }
                }
            }
            In::HeadFinished(h) => {
                self.heads.retain(|(id, _)| *id != h);
                self.queue
                    .retain(|j| !matches!(j, Job::Probe { head, .. } if *head == h));
            }
            In::Done(serial) => {
                self.serial = Some(serial);
                self.queue_probes();
                self.pump(&mut out);
                if !self.probing() {
                    out.push(Out::Heads(self.snapshot()));
                }
            }
            In::Succeeded(id) => self.answered(id, Answer::Succeeded, &mut out),
            In::Failed(id) => self.answered(id, Answer::Failed, &mut out),
            In::Cancelled(id) => self.answered(id, Answer::Cancelled, &mut out),
        }
        out
    }

    /// Queue `cfg` for the head called `name`, answered as `serial`.
    pub fn configure(&mut self, serial: u32, name: &str, cfg: HeadConfig) -> Vec<Out> {
        self.configure_all(serial, vec![(name.to_string(), cfg)])
    }

    /// Queue one configuration setting every head in `heads`, answered
    /// as `serial`.
    pub fn configure_all(&mut self, serial: u32, heads: HeadSet) -> Vec<Out> {
        self.queue.push_back(Job::Apply {
            serial,
            heads,
            tries: 0,
        });
        let mut out = Vec::new();
        self.pump(&mut out);
        out
    }

    /// The manager is gone: fail the request in flight and every queued
    /// one, oldest first, and announce no heads. Leaves the model empty.
    pub fn finish(&mut self) -> Vec<Out> {
        let jobs = self.flight.take().map(|(_, j)| j).into_iter();
        let mut out: Vec<Out> = jobs
            .chain(self.queue.drain(..))
            .filter_map(|j| match j {
                Job::Apply { serial, .. } => Some(Out::Applied { serial, ok: false }),
                Job::Probe { .. } => None,
            })
            .collect();
        self.heads.clear();
        self.modes.clear();
        self.serial = None;
        out.push(Out::Heads(Vec::new()));
        out
    }

    fn queue_probes(&mut self) {
        let can = self.version >= 4;
        let mut ask = Vec::new();
        for (id, d) in &mut self.heads {
            if !can {
                d.vrr_supported = Some(false);
            } else if d.vrr_supported.is_none() && d.head.enabled {
                ask.push(*id);
            }
        }
        for head in ask {
            let asked = self
                .queue
                .iter()
                .chain(self.flight.as_ref().map(|(_, j)| j))
                .any(|j| matches!(j, Job::Probe { head: h, .. } if *h == head));
            if !asked {
                self.queue.push_back(Job::Probe { head, tries: 0 });
            }
        }
    }

    fn probing(&self) -> bool {
        self.queue
            .iter()
            .chain(self.flight.as_ref().map(|(_, j)| j))
            .any(|j| matches!(j, Job::Probe { .. }))
    }

    fn pump(&mut self, out: &mut Vec<Out>) {
        while self.flight.is_none() {
            let Some(serial) = self.serial else {
                return;
            };
            let Some(job) = self.queue.pop_front() else {
                return;
            };
            match self.build(&job, serial) {
                Some(c) => {
                    self.flight = Some((c.id, job));
                    out.push(Out::Configure(c));
                }
                None => match job {
                    Job::Apply { serial, .. } => out.push(Out::Applied { serial, ok: false }),
                    Job::Probe { head, .. } => {
                        if let Some(d) = self.draft(head) {
                            d.vrr_supported = Some(false);
                        }
                    }
                },
            }
        }
    }

    /// The configuration `job` needs; None when one of its heads or modes
    /// is gone.
    fn build(&mut self, job: &Job, serial: u32) -> Option<Configure> {
        let sets: Vec<(ObjId, ConfigHead)> = match job {
            Job::Apply { heads, .. } => heads
                .iter()
                .map(|(name, cfg)| {
                    let (id, d) = self.heads.iter().find(|(_, d)| d.head.name == *name)?;
                    let mode = match cfg.mode {
                        None => None,
                        Some(want) => Some(
                            *d.modes
                                .iter()
                                .find(|m| self.mode_of(**m).is_some_and(|x| x.same(&want)))?,
                        ),
                    };
                    let set = ConfigHead {
                        mode,
                        transform: cfg.transform,
                        scale: cfg.scale,
                        adaptive_sync: cfg.adaptive_sync,
                        position: cfg.position,
                    };
                    Some((*id, set))
                })
                .collect::<Option<_>>()?,
            Job::Probe { head, .. } => {
                self.heads
                    .iter()
                    .find(|(id, d)| id == head && d.head.enabled)?;
                let set = ConfigHead {
                    adaptive_sync: Some(true),
                    ..ConfigHead::default()
                };
                vec![(*head, set)]
            }
        };
        self.next_id = self.next_id.wrapping_add(1);
        let heads = self
            .heads
            .iter()
            .map(|(id, d)| {
                let part = if let Some((_, set)) = sets.iter().find(|(t, _)| t == id) {
                    Some(*set)
                } else if d.head.enabled {
                    Some(ConfigHead::default())
                } else {
                    None
                };
                (*id, part)
            })
            .collect();
        Some(Configure {
            id: self.next_id,
            serial,
            test: matches!(job, Job::Probe { .. }),
            heads,
        })
    }

    fn answered(&mut self, id: u32, answer: Answer, out: &mut Vec<Out>) {
        let Some((_, job)) = self.flight.take_if(|(f, _)| *f == id) else {
            return;
        };
        let probe = matches!(job, Job::Probe { .. });
        match (job, answer) {
            (
                Job::Apply {
                    serial,
                    heads,
                    tries,
                },
                Answer::Cancelled,
            ) if tries + 1 < TRIES => self.queue.push_front(Job::Apply {
                serial,
                heads,
                tries: tries + 1,
            }),
            (Job::Apply { serial, .. }, a) => out.push(Out::Applied {
                serial,
                ok: a == Answer::Succeeded,
            }),
            (Job::Probe { head, tries }, Answer::Cancelled) if tries + 1 < TRIES => {
                self.queue.push_front(Job::Probe {
                    head,
                    tries: tries + 1,
                })
            }
            // Cancelled every time: still unknown, asked again after the
            // next done.
            (Job::Probe { .. }, Answer::Cancelled) => {}
            (Job::Probe { head, .. }, a) => {
                if let Some(d) = self.draft(head) {
                    d.vrr_supported = Some(a == Answer::Succeeded);
                }
            }
        }
        self.pump(out);
        if probe && !self.probing() {
            out.push(Out::Heads(self.snapshot()));
        }
    }

    fn snapshot(&self) -> Vec<Head> {
        self.heads
            .iter()
            .map(|(_, d)| {
                let ids: Vec<ObjId> = d
                    .modes
                    .iter()
                    .copied()
                    .filter(|m| self.mode_of(*m).is_some())
                    .collect();
                let mut h = d.head.clone();
                h.modes = ids.iter().filter_map(|m| self.mode_of(*m)).collect();
                h.current = d
                    .current
                    .filter(|_| d.head.enabled)
                    .and_then(|c| ids.iter().position(|m| *m == c));
                h.adaptive_sync = match (d.vrr_on, d.vrr_supported) {
                    (true, _) => AdaptiveSync::On,
                    (false, Some(true)) => AdaptiveSync::Off,
                    _ => AdaptiveSync::Unsupported,
                };
                h
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(width: i32, height: i32, refresh_mhz: i32) -> HeadMode {
        HeadMode {
            width,
            height,
            refresh_mhz,
            preferred: false,
        }
    }

    fn feed(m: &mut Model, evs: Vec<In>) -> Vec<Out> {
        evs.into_iter().flat_map(|e| m.event(e)).collect()
    }

    /// Head 10, "DP-1", enabled on mode 11 (1920x1080@60, preferred) of
    /// 11 and 12 (1280x720@60), adaptive sync off.
    fn dp1() -> Vec<In> {
        vec![
            In::Head(10),
            In::Name(10, "DP-1".into()),
            In::Make(10, "Dell Inc.".into()),
            In::Model(10, "DELL U2412M".into()),
            In::Mode(10, 11),
            In::ModeSize(11, 1920, 1080),
            In::ModeRefresh(11, 60_000),
            In::ModePreferred(11),
            In::Mode(10, 12),
            In::ModeSize(12, 1280, 720),
            In::ModeRefresh(12, 60_000),
            In::Enabled(10, true),
            In::CurrentMode(10, 11),
            In::Position(10, 0, 0),
            In::Transform(10, 0),
            In::Scale(10, 1.0),
            In::AdaptiveSync(10, false),
        ]
    }

    fn two_more() -> Vec<In> {
        vec![
            In::Head(20),
            In::Name(20, "HDMI-A-1".into()),
            In::Enabled(20, true),
            In::Head(30),
            In::Name(30, "DP-2".into()),
            In::Enabled(30, false),
        ]
    }

    #[test]
    fn a_done_announces_the_heads_with_their_modes() {
        let mut m = Model::new(3);
        assert_eq!(feed(&mut m, dp1()), vec![], "nothing before done");
        let out = m.event(In::Done(5));
        assert_eq!(
            out,
            vec![Out::Heads(vec![Head {
                name: "DP-1".into(),
                description: String::new(),
                make: "Dell Inc.".into(),
                model: "DELL U2412M".into(),
                modes: vec![
                    HeadMode {
                        preferred: true,
                        ..mode(1920, 1080, 60_000)
                    },
                    mode(1280, 720, 60_000),
                ],
                current: Some(0),
                enabled: true,
                position: (0, 0),
                transform: Transform::Normal,
                scale: 1.0,
                adaptive_sync: AdaptiveSync::Unsupported,
            }])],
            "below version 4 there is no adaptive sync"
        );
    }

    /// The first done asks each enabled head once, with a test turning
    /// adaptive sync on; the heads go out when the answers are in.
    #[test]
    fn adaptive_sync_support_is_asked_once_with_a_test() {
        let mut m = Model::new(4);
        feed(&mut m, dp1());
        let probe = Out::Configure(Configure {
            id: 1,
            serial: 5,
            test: true,
            heads: vec![(
                10,
                Some(ConfigHead {
                    adaptive_sync: Some(true),
                    ..ConfigHead::default()
                }),
            )],
        });
        assert_eq!(
            m.event(In::Done(5)),
            vec![probe],
            "no heads before the answer"
        );
        let out = m.event(In::Succeeded(1));
        let [Out::Heads(h)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!(h[0].adaptive_sync, AdaptiveSync::Off);
        assert!(
            matches!(m.event(In::Done(6)).as_slice(), [Out::Heads(_)]),
            "asked once: the next done only announces"
        );
        let mut m = Model::new(4);
        feed(&mut m, dp1());
        m.event(In::Done(5));
        let out = m.event(In::Failed(1));
        let [Out::Heads(h)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!(h[0].adaptive_sync, AdaptiveSync::Unsupported);
        let mut m = Model::new(4);
        feed(&mut m, dp1());
        m.event(In::AdaptiveSync(10, true));
        let out = m.event(In::Done(5));
        let [Out::Heads(h)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!(h[0].adaptive_sync, AdaptiveSync::On, "on is proof enough");
    }

    /// Three cancels leave the answer unknown: the head shows no adaptive
    /// sync, and the next done asks again.
    #[test]
    fn adaptive_sync_cancelled_three_times_is_asked_again() {
        let mut m = Model::new(4);
        feed(&mut m, dp1());
        m.event(In::Done(5));
        m.event(In::Cancelled(1));
        m.event(In::Cancelled(2));
        let out = m.event(In::Cancelled(3));
        let [Out::Heads(h)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!(h[0].adaptive_sync, AdaptiveSync::Unsupported);
        let out = m.event(In::Done(6));
        assert!(
            matches!(out.as_slice(), [Out::Configure(c)] if c.test && c.id == 4 && c.serial == 6),
            "{out:?}"
        );
    }

    #[test]
    fn a_configuration_names_every_head_and_sets_only_the_target() {
        let mut m = Model::new(3);
        feed(&mut m, dp1());
        feed(&mut m, two_more());
        m.event(In::Done(7));
        let out = m.configure(
            1,
            "DP-1",
            HeadConfig {
                mode: Some(mode(1280, 720, 60_000)),
                transform: Some(Transform::Rotate90),
                ..HeadConfig::default()
            },
        );
        assert_eq!(
            out,
            vec![Out::Configure(Configure {
                id: 1,
                serial: 7,
                test: false,
                heads: vec![
                    (
                        10,
                        Some(ConfigHead {
                            mode: Some(12),
                            transform: Some(Transform::Rotate90),
                            ..ConfigHead::default()
                        })
                    ),
                    (20, Some(ConfigHead::default())),
                    (30, None),
                ],
            })]
        );
    }

    /// An arrangement places two heads in one configuration; a head it
    /// names that is gone fails the whole request.
    #[test]
    fn one_configuration_places_several_heads() {
        let mut m = Model::new(3);
        feed(&mut m, dp1());
        feed(&mut m, two_more());
        m.event(In::Done(7));
        let at = |x, y| HeadConfig {
            position: Some((x, y)),
            ..HeadConfig::default()
        };
        let out = m.configure_all(
            1,
            vec![("DP-1".into(), at(0, 0)), ("HDMI-A-1".into(), at(0, 1080))],
        );
        let [Out::Configure(c)] = out.as_slice() else {
            panic!("{out:?}")
        };
        let placed = |p| {
            Some(ConfigHead {
                position: Some(p),
                ..ConfigHead::default()
            })
        };
        assert_eq!(
            c.heads,
            vec![(10, placed((0, 0))), (20, placed((0, 1080))), (30, None)]
        );
        m.event(In::Succeeded(c.id));
        let out = m.configure_all(
            2,
            vec![("DP-1".into(), at(0, 0)), ("DP-9".into(), at(1, 1))],
        );
        assert_eq!(
            out,
            vec![Out::Applied {
                serial: 2,
                ok: false
            }]
        );
    }

    #[test]
    fn one_configuration_is_in_flight_at_a_time() {
        let mut m = Model::new(3);
        feed(&mut m, dp1());
        let scale = |s| HeadConfig {
            scale: Some(s),
            ..HeadConfig::default()
        };
        assert_eq!(
            m.configure(1, "DP-1", scale(2.0)),
            vec![],
            "waits for the first done"
        );
        let out = m.event(In::Done(7));
        assert!(
            matches!(out.as_slice(), [Out::Configure(c), Out::Heads(_)] if c.serial == 7),
            "{out:?}"
        );
        assert_eq!(
            m.configure(2, "DP-1", scale(1.0)),
            vec![],
            "waits for the first answer"
        );
        let out = m.event(In::Succeeded(1));
        assert_eq!(
            out[0],
            Out::Applied {
                serial: 1,
                ok: true
            }
        );
        assert!(matches!(&out[1], Out::Configure(c) if c.id == 2), "{out:?}");
    }

    #[test]
    fn a_finished_manager_fails_what_was_asked_and_empties_the_heads() {
        let mut m = Model::new(3);
        feed(&mut m, dp1());
        m.event(In::Done(7));
        m.configure(1, "DP-1", HeadConfig::default());
        m.configure(2, "DP-1", HeadConfig::default());
        assert_eq!(
            m.finish(),
            vec![
                Out::Applied {
                    serial: 1,
                    ok: false
                },
                Out::Applied {
                    serial: 2,
                    ok: false
                },
                Out::Heads(vec![]),
            ],
            "the one in flight, then the queued one, then no heads"
        );
    }

    #[test]
    fn a_cancelled_configuration_goes_again_with_the_new_serial() {
        let mut m = Model::new(3);
        feed(&mut m, dp1());
        m.event(In::Done(7));
        let scale = HeadConfig {
            scale: Some(2.0),
            ..HeadConfig::default()
        };
        m.configure(1, "DP-1", scale);
        m.event(In::Done(8));
        let out = m.event(In::Cancelled(1));
        assert!(
            matches!(out.as_slice(), [Out::Configure(c)] if c.id == 2 && c.serial == 8),
            "{out:?}"
        );
        assert!(matches!(m.event(In::Cancelled(2)).as_slice(), [Out::Configure(c)] if c.id == 3));
        assert_eq!(
            m.event(In::Cancelled(3)),
            vec![Out::Applied {
                serial: 1,
                ok: false
            }],
            "three sends at most"
        );
    }

    #[test]
    fn a_mode_or_head_that_is_not_there_fails_without_a_request() {
        let mut m = Model::new(3);
        feed(&mut m, dp1());
        m.event(In::Done(7));
        let odd = HeadConfig {
            mode: Some(mode(800, 600, 60_000)),
            ..HeadConfig::default()
        };
        assert_eq!(
            m.configure(1, "DP-1", odd),
            vec![Out::Applied {
                serial: 1,
                ok: false
            }]
        );
        assert_eq!(
            m.configure(2, "HDMI-A-9", HeadConfig::default()),
            vec![Out::Applied {
                serial: 2,
                ok: false
            }]
        );
    }

    #[test]
    fn a_finished_head_or_mode_leaves_the_list() {
        let mut m = Model::new(3);
        feed(&mut m, dp1());
        m.event(In::Done(5));
        m.event(In::ModeFinished(12));
        let out = m.event(In::Done(6));
        let [Out::Heads(h)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!(h[0].modes.len(), 1);
        m.event(In::HeadFinished(10));
        assert_eq!(m.event(In::Done(7)), vec![Out::Heads(vec![])]);
    }

    #[test]
    fn requests_are_numbered_and_taken_once() {
        let mut r = HeadRequests::default();
        let a = r.configure("DP-1", HeadConfig::default());
        let b = r.configure("DP-1", HeadConfig::default());
        assert_ne!(a, b);
        assert_eq!(r.take().iter().map(|q| q.0).collect::<Vec<_>>(), vec![a, b]);
        assert!(r.take().is_empty());
    }

    #[test]
    fn a_transform_is_spelled_as_the_config_spells_it() {
        assert_eq!(Transform::from_wl(5), Transform::Flipped90);
        assert_eq!(Transform::Flipped90.name(), "flipped-90");
        assert_eq!(Transform::Rotate270.wl(), 3);
        assert_eq!(Transform::from_wl(99), Transform::Normal);
    }

    #[test]
    fn ctx_queues_what_the_program_asks() {
        let mut ctx = crate::Ctx {
            theme: reprisekit::theme::Theme::default(),
            text: reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default()),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: (10, 10),
            maximized: false,
            origin: (0, 0),
            wake: crate::Waker::noop(),
            heads: HeadRequests::default(),
            text_size: 100,
        };
        let n = ctx.configure_head("DP-1", HeadConfig::default());
        assert_eq!(
            ctx.heads.take(),
            vec![(n, vec![("DP-1".to_string(), HeadConfig::default())])]
        );
    }
}
