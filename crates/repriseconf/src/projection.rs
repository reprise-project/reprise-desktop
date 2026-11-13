//! What each display runs for a `reprisekit::projection::Projection`: the plan the compositor's
//! display switcher and the Display page's "Multiple displays" combo both apply.

pub use reprisekit::projection::Projection;

/// One mode a display lists: width and height in pixels, refresh in mHz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    pub refresh_mhz: i32,
    pub preferred: bool,
}

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

/// The size every display lists, the largest by area; for each display, its mode at that size:
/// the preferred one when it is that size, else the fastest.
pub fn common_mode(screens: &[Screen]) -> Option<Vec<usize>> {
    let first = screens.first()?;
    let size = (first.modes.iter())
        .map(|m| (m.width, m.height))
        .filter(|&(w, h)| {
            screens
                .iter()
                .all(|s| s.modes.iter().any(|m| (m.width, m.height) == (w, h)))
        })
        .max_by_key(|&(w, h)| (i64::from(w) * i64::from(h), w))?;
    let pick = |s: &Screen| {
        let at = |m: &Mode| (m.width, m.height) == size;
        (s.modes.iter().position(|m| at(m) && m.preferred)).or_else(|| {
            (s.modes.iter().enumerate())
                .filter(|(_, m)| at(m))
                .max_by_key(|(_, m)| m.refresh_mhz)
                .map(|(i, _)| i)
        })
    };
    screens.iter().map(pick).collect()
}

/// What each of `screens` runs for `p`, in their order; None when `p` is not available for them
/// (`Projection::available`). Picking what they show already keeps every display as it is.
///
/// - Extend: the primary display at its place, the others to its right in order, each in the
///   mode it runs (its usual one when off or copying).
/// - Duplicate: every display at the primary's place and scale, in the largest common mode; with
///   none in common each keeps its usual mode and the compositor fits the primary into the others.
/// - Only: that display in its usual mode, the others off.
pub fn plan(screens: &[Screen], primary: Option<&str>, p: Projection) -> Option<Vec<Want>> {
    if !p.available(screens.len()) {
        return None;
    }
    if current(screens) == p {
        return Some(screens.iter().map(Screen::keep).collect());
    }
    let src = source(screens, primary);
    match p {
        Projection::Only(k) => Some(
            (screens.iter().enumerate())
                .map(|(i, s)| match i == k {
                    true => Want {
                        enabled: true,
                        mode: s.usual_mode(),
                        position: if s.enabled { s.position } else { (0, 0) },
                        scale: s.scale,
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
            let copying = current(screens) == Projection::Duplicate;
            let mode = |o: &Screen| match o.enabled && !copying {
                true => o.current.or_else(|| o.usual_mode()),
                false => (o.modes.iter().position(|m| m.preferred)).or_else(|| o.usual_mode()),
            };
            let s = &screens[src];
            let (mut x, y) = if s.enabled { s.position } else { (0, 0) };
            let mut order: Vec<usize> = vec![src];
            order.extend((0..screens.len()).filter(|&i| i != src));
            let mut out = vec![None; screens.len()];
            for i in order {
                let o = &screens[i];
                let m = mode(o);
                out[i] = Some(Want {
                    enabled: true,
                    mode: m,
                    position: (x, y),
                    scale: o.scale,
                });
                x += m.map_or(0, |m| o.logical(m, o.scale).0);
            }
            out.into_iter().collect()
        }
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
            },
        ]
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
