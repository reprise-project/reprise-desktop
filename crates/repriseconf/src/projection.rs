//! What each display runs for a `reprisekit::projection::Projection`: the plan the compositor's
//! display switcher and the Display page's "Multiple displays" combo both apply.

pub use reprisekit::projection::Projection;

use crate::config::{OfferedMode, OutputConfig, OutputMode};
use crate::displays::MonitorLayout;

/// One mode a display lists: width and height in pixels, refresh in mHz.
pub type Mode = OfferedMode;

/// One connected display as the plan sees it, in the Display page's order.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub name: String,
    pub modes: Vec<Mode>,
    /// The index in `modes` it runs; None while off.
    pub current: Option<usize>,
    pub enabled: bool,
    pub position: (i32, i32),
    pub scale: f64,
    /// Turned a quarter, so its layout box is taller than wide for a landscape mode.
    pub turned: bool,
    pub home: Home,
}

/// What a display goes back to once it stops copying or comes back on (`homes`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Home {
    pub mode: Option<OutputMode>,
    pub position: Option<(i32, i32)>,
    pub scale: Option<f64>,
    /// It was the primary display when `position` was saved.
    pub primary: bool,
}

/// Each connected display's home, in `connectors`' order (`ids` their monitor ids alike): its
/// entry in `extended`, the set's saved side-by-side arrangement
/// (`displays::Layout::extended`), else its `[[outputs]]` entry in `outputs`.
pub fn homes(
    extended: &[(String, MonitorLayout)],
    ids: &[String],
    connectors: &[String],
    outputs: &[OutputConfig],
) -> Vec<Home> {
    (connectors.iter().zip(ids))
        .map(|(c, id)| {
            let saved = (extended.iter()).find(|(i, m)| i == id && m.enabled);
            if let Some((_, m)) = saved {
                return Home {
                    mode: m.mode,
                    position: Some(m.position),
                    scale: Some(f64::from(m.scale)),
                    primary: m.primary,
                };
            }
            let pin = outputs.iter().find(|o| o.name == *c);
            Home {
                mode: pin.and_then(|o| o.mode),
                position: pin.and_then(|o| o.position),
                scale: pin.map(|o| f64::from(o.scale)),
                primary: false,
            }
        })
        .collect()
}

/// What a plan asks of one display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Want {
    pub enabled: bool,
    /// The index in its `modes`; None when it goes off.
    pub mode: Option<usize>,
    pub position: (i32, i32),
    pub scale: f64,
}

impl Screen {
    /// The mode it shows when turned on: the one it runs, else its preferred one, else its first.
    fn usual_mode(&self) -> Option<usize> {
        self.current
            .filter(|_| self.enabled)
            .or_else(|| self.modes.iter().position(|m| m.preferred))
            .or_else(|| (!self.modes.is_empty()).then_some(0))
    }

    /// Its layout size at mode `i` and `scale`.
    fn logical(&self, i: usize, scale: f64) -> (i32, i32) {
        let Some(m) = self.modes.get(i) else {
            return (0, 0);
        };
        let (w, h) = if self.turned {
            (m.height, m.width)
        } else {
            (m.width, m.height)
        };
        let s = if scale > 0.0 { scale } else { 1.0 };
        (
            (f64::from(w) / s).round() as i32,
            (f64::from(h) / s).round() as i32,
        )
    }

    /// The mode it goes back to: its home's when it lists it, else its usual one.
    fn home_mode(&self) -> Option<usize> {
        (self.home.mode.and_then(|m| m.pick(&self.modes)))
            .or_else(|| self.modes.iter().position(|m| m.preferred))
            .or_else(|| self.usual_mode())
    }

    /// The mode it runs on its own: the one it runs while on, else its home one.
    fn own_mode(&self) -> Option<&Mode> {
        let i = match self.enabled {
            true => self.current.or_else(|| self.home_mode()),
            false => self.home_mode(),
        };
        self.modes.get(i?)
    }

    fn keep(&self) -> Want {
        Want {
            enabled: self.enabled,
            mode: self.current.filter(|_| self.enabled),
            position: self.position,
            scale: self.scale,
        }
    }

    fn off(&self) -> Want {
        Want {
            enabled: false,
            mode: None,
            ..self.keep()
        }
    }
}

/// The display a duplicate copies: the primary one when connected, else the first.
fn source(screens: &[Screen], primary: Option<&str>) -> usize {
    primary
        .and_then(|p| screens.iter().position(|s| s.name == p))
        .unwrap_or(0)
}

/// What `screens` show now: one on is `Only` it, two or more on at one top-left `Duplicate`,
/// anything else `Extend`.
pub fn current(screens: &[Screen]) -> Projection {
    let on: Vec<usize> = (0..screens.len()).filter(|&i| screens[i].enabled).collect();
    match on.as_slice() {
        [one] => Projection::Only(*one),
        [first, rest @ ..]
            if rest
                .iter()
                .all(|&i| screens[i].position == screens[*first].position) =>
        {
            Projection::Duplicate
        }
        _ => Projection::Extend,
    }
}

/// The mode each display runs while copying, by index in its `modes`; None when they list no size
/// in common. The size is the largest they all list that fits inside the mode each runs on its
/// own, else the smallest they all list: copying never enlarges a display past its own mode while
/// a size within it exists. At that size each takes a refresh they all list (to the hertz), the
/// one most of their preferred modes run, then the highest; else its preferred one, else its
/// fastest.
pub fn common_mode(screens: &[Screen]) -> Option<Vec<usize>> {
    let first = screens.first()?;
    let size_of = |m: &Mode| (m.width, m.height);
    let sizes: Vec<(i32, i32)> = (first.modes.iter().map(size_of))
        .filter(|&z| {
            screens
                .iter()
                .all(|s| s.modes.iter().any(|m| size_of(m) == z))
        })
        .collect();
    let area = |&(w, h): &(i32, i32)| (i64::from(w) * i64::from(h), w);
    let fits = |&(w, h): &(i32, i32)| {
        (screens.iter()).all(|s| s.own_mode().is_none_or(|m| w <= m.width && h <= m.height))
    };
    let size = (sizes.iter().filter(|z| fits(z)).max_by_key(|z| area(z)))
        .or_else(|| sizes.iter().min_by_key(|z| area(z)))
        .copied()?;
    let at = |m: &Mode| size_of(m) == size;
    let hz = |m: &Mode| (m.refresh_mhz + 500) / 1000;
    let preferring = |z: i32| {
        (screens.iter())
            .filter(|s| s.modes.iter().any(|m| at(m) && m.preferred && hz(m) == z))
            .count()
    };
    let shared_hz = (first.modes.iter().filter(|m| at(m)).map(hz))
        .filter(|&z| {
            screens
                .iter()
                .all(|s| s.modes.iter().any(|m| at(m) && hz(m) == z))
        })
        .max_by_key(|&z| (preferring(z), z));
    let pick = |s: &Screen| {
        (s.modes.iter().enumerate())
            .filter(|(_, m)| at(m) && shared_hz.is_none_or(|z| hz(m) == z))
            .max_by_key(|(_, m)| (m.preferred, m.refresh_mhz))
            .map(|(i, _)| i)
    };
    screens.iter().map(pick).collect()
}

/// Whether any two of `wants`' boxes overlap, `screens` alike.
fn overlap(screens: &[Screen], wants: &[Want]) -> bool {
    let rect = |i: usize| {
        let w = &wants[i];
        let (sw, sh) = w.mode.map_or((0, 0), |m| screens[i].logical(m, w.scale));
        (w.position.0, w.position.1, sw, sh)
    };
    (0..wants.len()).any(|i| {
        let (ax, ay, aw, ah) = rect(i);
        (i + 1..wants.len()).any(|j| {
            let (bx, by, bw, bh) = rect(j);
            ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah
        })
    })
}

/// What each of `screens` runs for `p`, in their order; None when `p` is not available for them
/// (`Projection::available`). Picking what they show already keeps every display as it is.
///
/// A display that is off or copying goes back to its `home`: its mode and scale (its preferred
/// mode without one).
///
/// - Extend: a display on its own keeps what it runs. Each display goes at its home place when
///   every display going home has one, the primary is still the one they were saved with, and no
///   two overlap; else the primary display at its place and the others to its right in order.
/// - Duplicate: every display at the primary's place and scale, in `common_mode`; with none in
///   common each keeps its usual mode and the compositor fits the primary into the others.
/// - Only: that display, the others off.
pub fn plan(screens: &[Screen], primary: Option<&str>, p: Projection) -> Option<Vec<Want>> {
    if !p.available(screens.len()) {
        return None;
    }
    if current(screens) == p {
        return Some(screens.iter().map(Screen::keep).collect());
    }
    let src = source(screens, primary);
    let copying = current(screens) == Projection::Duplicate;
    match p {
        Projection::Only(k) => Some(
            (screens.iter().enumerate())
                .map(|(i, s)| match i == k {
                    true => Want {
                        position: if s.enabled { s.position } else { (0, 0) },
                        ..own(s, copying)
                    },
                    false => s.off(),
                })
                .collect(),
        ),
        Projection::Duplicate => {
            let s = &screens[src];
            let at = if s.enabled { s.position } else { (0, 0) };
            let common = common_mode(screens);
            Some(
                (screens.iter().enumerate())
                    .map(|(i, o)| Want {
                        enabled: true,
                        mode: common
                            .as_ref()
                            .map_or_else(|| o.usual_mode(), |c| Some(c[i])),
                        position: at,
                        scale: s.scale,
                    })
                    .collect(),
            )
        }
        Projection::Extend => {
            let mut wants: Vec<Want> = screens.iter().map(|o| own(o, copying)).collect();
            let going_home = |o: &&Screen| !o.enabled || copying;
            let at_home = (screens.iter().filter(going_home)).all(|o| o.home.position.is_some())
                && (screens.iter().position(|o| o.home.primary)).is_none_or(|i| i == src)
                && !overlap(screens, &wants);
            if at_home {
                return Some(wants);
            }
            let s = &screens[src];
            let (mut x, y) = if s.enabled { s.position } else { (0, 0) };
            let mut order: Vec<usize> = vec![src];
            order.extend((0..screens.len()).filter(|&i| i != src));
            for i in order {
                let w = &mut wants[i];
                w.position = (x, y);
                x += w.mode.map_or(0, |m| screens[i].logical(m, w.scale).0);
            }
            Some(wants)
        }
    }
}

/// `o` turned on as it runs on its own: what it runs, or its home when it is off or `copying`.
fn own(o: &Screen, copying: bool) -> Want {
    match o.enabled && !copying {
        true => Want {
            enabled: true,
            mode: o.current.or_else(|| o.usual_mode()),
            position: o.position,
            scale: o.scale,
        },
        false => Want {
            enabled: true,
            mode: o.home_mode(),
            position: o.home.position.unwrap_or(o.position),
            scale: o.home.scale.unwrap_or(o.scale),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(width: i32, height: i32, refresh_mhz: i32, preferred: bool) -> Mode {
        Mode {
            width,
            height,
            refresh_mhz,
            preferred,
        }
    }

    /// A laptop panel at 2560x1600 (scale 1.25) and a projector that tops out at 1920x1080,
    /// extended side by side.
    fn pair() -> Vec<Screen> {
        vec![
            Screen {
                name: "eDP-1".into(),
                modes: vec![
                    mode(2560, 1600, 60_000, true),
                    mode(1920, 1080, 60_000, false),
                    mode(1280, 720, 60_000, false),
                ],
                current: Some(0),
                enabled: true,
                position: (0, 0),
                scale: 1.25,
                turned: false,
                home: Home::default(),
            },
            Screen {
                name: "HDMI-A-1".into(),
                modes: vec![
                    mode(1920, 1080, 60_000, true),
                    mode(1920, 1080, 50_000, false),
                    mode(1280, 720, 60_000, false),
                ],
                current: Some(0),
                enabled: true,
                position: (2048, 0),
                scale: 1.0,
                turned: false,
                home: Home::default(),
            },
        ]
    }

    /// `screens` once they run `plan`.
    fn run(screens: &mut [Screen], plan: &[Want]) {
        for (s, w) in screens.iter_mut().zip(plan) {
            s.enabled = w.enabled;
            s.current = w.mode;
            s.position = w.position;
            s.scale = w.scale;
        }
    }

    /// A display's size, place and scale.
    type Run = (i32, i32, (i32, i32), f64);

    fn picked(screens: &[Screen]) -> Vec<Option<Run>> {
        (screens.iter())
            .map(|s| {
                let m = s.modes[s.current?];
                Some((m.width, m.height, s.position, s.scale))
            })
            .collect()
    }

    /// Two monitors that list many sizes past their own, the first preferring a small one.
    fn wide_lists() -> Vec<Screen> {
        let list = |own: (i32, i32)| {
            [
                (5120, 2160, 50_000),
                (1920, 1080, 60_000),
                (1440, 900, 59_901),
            ]
            .into_iter()
            .chain([(1280, 800, 59_810)])
            .map(|(w, h, r)| mode(w, h, r, (w, h) == own))
            .collect()
        };
        let screen = |name: &str, own: (i32, i32), current, x| Screen {
            name: name.into(),
            modes: list(own),
            current: Some(current),
            enabled: true,
            position: (x, 0),
            scale: 1.0,
            turned: false,
            home: Home::default(),
        };
        vec![
            screen("Virtual-1", (1280, 800), 1, 0),
            screen("Virtual-2", (0, 0), 2, 1920),
        ]
    }

    fn pins() -> Vec<OutputConfig> {
        let pin = |name: &str, mode: &str| OutputConfig {
            name: name.into(),
            scale: 1.0,
            mode: Some(OutputMode::parse(mode).unwrap()),
            adaptive_sync: None,
            transform: Default::default(),
            position: None,
            primary: false,
            icc_profile: None,
        };
        vec![pin("Virtual-1", "1920x1080"), pin("Virtual-2", "1440x900")]
    }

    fn names(s: &[Screen]) -> Vec<String> {
        s.iter().map(|s| s.name.clone()).collect()
    }

    /// `s` as the compositor saves it, by monitor id = connector.
    fn saved(s: &[Screen], primary: usize) -> Vec<(String, MonitorLayout)> {
        (s.iter().enumerate())
            .map(|(i, s)| {
                let m = s.modes[s.current.unwrap()];
                let layout = MonitorLayout {
                    enabled: s.enabled,
                    mode: Some(OutputMode {
                        width: m.width,
                        height: m.height,
                        refresh_mhz: Some(m.refresh_mhz),
                    }),
                    position: s.position,
                    scale: s.scale as f32,
                    transform: Default::default(),
                    primary: i == primary,
                };
                (s.name.clone(), layout)
            })
            .collect()
    }

    fn set_homes(s: &mut [Screen], extended: &[(String, MonitorLayout)], outputs: &[OutputConfig]) {
        let n = names(s);
        for (s, h) in s.iter_mut().zip(homes(extended, &n, &n, outputs)) {
            s.home = h;
        }
    }

    #[test]
    fn what_shows_now_reads_off_the_places_and_who_is_on() {
        let mut s = pair();
        assert_eq!(current(&s), Projection::Extend);
        s[1].position = (0, 0);
        assert_eq!(current(&s), Projection::Duplicate);
        s[0].enabled = false;
        assert_eq!(current(&s), Projection::Only(1));
    }

    #[test]
    fn duplicate_puts_every_display_at_the_primary_in_the_largest_common_mode() {
        let plan = plan(&pair(), Some("HDMI-A-1"), Projection::Duplicate).unwrap();
        assert_eq!(
            plan,
            vec![
                Want {
                    enabled: true,
                    mode: Some(1),
                    position: (2048, 0),
                    scale: 1.0,
                },
                Want {
                    enabled: true,
                    mode: Some(0),
                    position: (2048, 0),
                    scale: 1.0,
                },
            ]
        );
    }

    #[test]
    fn with_no_common_mode_each_display_keeps_its_own() {
        let mut s = pair();
        s[1].modes = vec![mode(1024, 768, 60_000, true)];
        assert_eq!(common_mode(&s), None);
        let plan = plan(&s, None, Projection::Duplicate).unwrap();
        assert_eq!((plan[0].mode, plan[1].mode), (Some(0), Some(0)));
        assert_eq!((plan[0].position, plan[1].position), ((0, 0), (0, 0)));
        assert_eq!(plan[1].scale, 1.25, "the primary's scale");
    }

    #[test]
    fn extend_after_duplicate_lays_the_displays_out_from_the_primary_in_their_own_modes() {
        let mut s = pair();
        s[0].current = Some(1);
        s[0].scale = 1.0;
        s[1].position = (0, 0);
        let plan = plan(&s, Some("eDP-1"), Projection::Extend).unwrap();
        assert_eq!(
            plan.iter()
                .map(|w| (w.mode, w.position))
                .collect::<Vec<_>>(),
            vec![(Some(0), (0, 0)), (Some(0), (2560, 0))]
        );
    }

    #[test]
    fn duplicate_takes_the_largest_common_size_inside_each_displays_own_mode() {
        let s = wide_lists();
        let common = common_mode(&s).unwrap();
        let sizes: Vec<_> = (common.iter().zip(&s))
            .map(|(&i, s)| (s.modes[i].width, s.modes[i].height))
            .collect();
        assert_eq!(sizes, [(1440, 900), (1440, 900)], "not 5120x2160");
    }

    #[test]
    fn with_no_common_size_inside_both_duplicate_takes_the_smallest_and_a_shared_refresh() {
        let mut s = pair();
        s[0].modes = vec![
            mode(1280, 800, 60_000, true),
            mode(1920, 1080, 59_940, false),
        ];
        s[0].current = Some(0);
        s[1].modes = vec![
            mode(1920, 1080, 75_000, true),
            mode(1920, 1080, 60_000, false),
            mode(2560, 1440, 60_000, false),
        ];
        assert_eq!(
            common_mode(&s),
            Some(vec![1, 1]),
            "1920x1080 at 60 Hz on both"
        );
    }

    #[test]
    fn extend_after_duplicate_puts_every_display_back_in_its_saved_mode_place_and_scale() {
        let mut s = pair();
        let before = picked(&s);
        set_homes(&mut s, &saved(&pair(), 0), &[]);
        let dup = plan(&s, Some("eDP-1"), Projection::Duplicate).unwrap();
        run(&mut s, &dup);
        assert_eq!(current(&s), Projection::Duplicate);
        let back = plan(&s, Some("eDP-1"), Projection::Extend).unwrap();
        run(&mut s, &back);
        assert_eq!(picked(&s), before);
    }

    #[test]
    fn extend_after_duplicate_with_only_outputs_pins_runs_the_pinned_modes_side_by_side() {
        let mut s = wide_lists();
        set_homes(&mut s, &[], &pins());
        let dup = plan(&s, None, Projection::Duplicate).unwrap();
        run(&mut s, &dup);
        let back = plan(&s, None, Projection::Extend).unwrap();
        run(&mut s, &back);
        assert_eq!(
            picked(&s),
            [
                Some((1920, 1080, (0, 0), 1.0)),
                Some((1440, 900, (1920, 0), 1.0))
            ]
        );
    }

    #[test]
    fn a_saved_arrangement_outranks_the_outputs_pins() {
        let mut s = wide_lists();
        s[1].position = (0, 800);
        s[0].current = Some(3);
        let ours = saved(&s, 0);
        set_homes(&mut s, &ours, &pins());
        let dup = plan(&s, None, Projection::Duplicate).unwrap();
        run(&mut s, &dup);
        let back = plan(&s, None, Projection::Extend).unwrap();
        run(&mut s, &back);
        assert_eq!(
            picked(&s),
            [
                Some((1280, 800, (0, 0), 1.0)),
                Some((1440, 900, (0, 800), 1.0))
            ]
        );
    }

    #[test]
    fn a_primary_picked_while_copying_is_where_extend_starts() {
        let mut s = pair();
        set_homes(&mut s, &saved(&pair(), 0), &[]);
        let dup = plan(&s, Some("eDP-1"), Projection::Duplicate).unwrap();
        run(&mut s, &dup);
        let back = plan(&s, Some("HDMI-A-1"), Projection::Extend).unwrap();
        assert_eq!((back[0].position, back[1].position), ((1920, 0), (0, 0)));
        assert_eq!(
            (back[0].mode, back[0].scale),
            (Some(0), 1.25),
            "its own mode"
        );
    }

    #[test]
    fn extend_after_only_brings_the_off_display_back_at_its_saved_place() {
        let mut s = pair();
        s[1].position = (0, 1280);
        let before = picked(&s);
        let ours = saved(&s, 0);
        set_homes(&mut s, &ours, &[]);
        let only = plan(&s, None, Projection::Only(0)).unwrap();
        run(&mut s, &only);
        let back = plan(&s, None, Projection::Extend).unwrap();
        run(&mut s, &back);
        assert_eq!(picked(&s), before);
    }

    #[test]
    fn only_turns_the_others_off_and_brings_an_off_display_back_in_its_preferred_mode() {
        let mut s = pair();
        let only = plan(&s, None, Projection::Only(1)).unwrap();
        assert!(only[1].enabled && !only[0].enabled);
        assert_eq!(only[0].mode, None);
        s[0].enabled = false;
        s[0].current = None;
        let back = plan(&s, None, Projection::Only(0)).unwrap();
        assert_eq!((back[0].enabled, back[0].mode), (true, Some(0)));
        assert!(!back[1].enabled);
    }

    #[test]
    fn picking_what_shows_changes_nothing_and_one_display_offers_only_itself() {
        let s = pair();
        let same = plan(&s, None, Projection::Extend).unwrap();
        assert_eq!(same[1].position, (2048, 0));
        assert_eq!(plan(&s[..1], None, Projection::Duplicate), None);
        assert!(plan(&s[..1], None, Projection::Only(0)).is_some());
    }

    #[test]
    fn a_turned_display_extends_by_its_height() {
        let mut s = pair();
        s[0].turned = true;
        s[0].scale = 1.0;
        s[0].enabled = false;
        let plan = plan(&s, Some("eDP-1"), Projection::Extend).unwrap();
        assert_eq!(plan[1].position, (1600, 0));
    }
}
