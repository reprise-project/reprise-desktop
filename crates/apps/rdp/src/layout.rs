//! Where each shared screen lands on the client's desktop: whole, paired with a client
//! monitor, or all scaled into the client's monitors together.

use ironrdp_pdu::gcc::{MONITOR_COUNT_MAX, Monitor, MonitorFlags};

/// A shared screen: its place in the session's logical layout and its stream size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenRect {
    pub position: (i32, i32),
    pub logical: (u32, u32),
    pub pixels: (u32, u32),
}

/// A rectangle on the client desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect16 {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

/// Screen `screen` (an index into the screens given to [`Layout::new`]) drawn at `target`,
/// whose sides are even for H.264 4:2:0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub screen: usize,
    pub target: Rect16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Whole,
    Paired,
    Scaled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub kind: Kind,
    pub desktop: (u16, u16),
    /// The monitors ResetGraphics lists.
    pub monitors: Vec<Monitor>,
    /// In screen order: by logical position, top then left.
    pub placed: Vec<Placed>,
    /// Each screen's stream size, by screen index.
    pixels: Vec<(u32, u32)>,
}

impl Layout {
    /// Whole without client monitors or with one, Paired when every screen gets a monitor,
    /// else Scaled. Client monitors that cannot be used give Whole.
    pub fn new(screens: &[ScreenRect], client: &[Monitor]) -> Layout {
        let mut order: Vec<usize> = (0..screens.len()).collect();
        order.sort_by_key(|&i| (screens[i].position.1, screens[i].position.0));
        let pixels = screens.iter().map(|s| s.pixels).collect();
        let whole = whole(screens, &order);
        let client = of_client(client)
            .inspect_err(|why| log::warn!("client monitors ignored: {why}"))
            .ok()
            .flatten();
        let Some((desktop, monitors)) = client else {
            return Layout {
                kind: Kind::Whole,
                desktop: clamp16(whole.size),
                monitors: vec![primary(clamp16(whole.size))],
                placed: whole.placed(),
                pixels,
            };
        };
        if monitors.len() >= screens.len() {
            let mut by_place: Vec<&Monitor> = monitors.iter().collect();
            by_place.sort_by_key(|m| (!m.flags.contains(MonitorFlags::PRIMARY), m.top, m.left));
            let placed = order
                .iter()
                .zip(by_place)
                .map(|(&i, m)| Placed {
                    screen: i,
                    target: fit(screens[i].pixels, m),
                })
                .collect();
            Layout {
                kind: Kind::Paired,
                desktop,
                monitors,
                placed,
                pixels,
            }
        } else {
            Layout {
                kind: Kind::Scaled,
                desktop,
                monitors,
                placed: whole.scaled_into(desktop),
                pixels,
            }
        }
    }

    /// The screen under client point `p` and the point in its stream pixels; `None` outside
    /// every target.
    pub fn to_screen(&self, p: (u16, u16)) -> Option<(usize, (u16, u16))> {
        let placed = self.placed.iter().find(|placed| {
            let t = placed.target;
            let inside = |p: u16, at: u16, len: u16| {
                (u32::from(at)..u32::from(at) + u32::from(len)).contains(&u32::from(p))
            };
            inside(p.0, t.x, t.w) && inside(p.1, t.y, t.h)
        })?;
        let t = placed.target;
        let pixels = self.pixels[placed.screen];
        let axis = |p: u16, origin: u16, target: u16, pixels: u32| {
            u16::try_from(muldiv(u32::from(p - origin), pixels, u32::from(target))).ok()
        };
        Some((
            placed.screen,
            (
                axis(p.0, t.x, t.w, pixels.0)?,
                axis(p.1, t.y, t.h, pixels.1)?,
            ),
        ))
    }

    /// The client point showing stream pixel `p` of `screen`.
    pub fn to_client(&self, screen: usize, p: (u32, u32)) -> Option<(u16, u16)> {
        let t = self
            .placed
            .iter()
            .find(|placed| placed.screen == screen)?
            .target;
        let pixels = self.pixels[screen];
        if p.0 >= pixels.0 || p.1 >= pixels.1 {
            return None;
        }
        let axis = |p: u32, origin: u16, target: u16, pixels: u32| {
            u16::try_from(u32::from(origin) + muldiv(p, u32::from(target), pixels)).ok()
        };
        Some((
            axis(p.0, t.x, t.w, pixels.0)?,
            axis(p.1, t.y, t.h, pixels.1)?,
        ))
    }
}

/// The screens at their offsets in stream pixels, unscaled.
struct Whole {
    size: (u32, u32),
    /// In screen order.
    screens: Vec<Offset>,
}

/// Screen `screen` at offset `at` in the Whole box, `size` stream pixels.
struct Offset {
    screen: usize,
    at: (u32, u32),
    size: (u32, u32),
}

impl Whole {
    fn placed(&self) -> Vec<Placed> {
        self.screens
            .iter()
            .map(|&Offset { screen, at, size }| Placed {
                screen,
                target: rect(at, (even(size.0), even(size.1))),
            })
            .collect()
    }

    /// Every screen scaled by the one factor that fits the box into `desktop`, centred.
    fn scaled_into(&self, desktop: (u16, u16)) -> Vec<Placed> {
        let desktop = (u32::from(desktop.0), u32::from(desktop.1));
        let wide = u64::from(desktop.0) * u64::from(self.size.1)
            <= u64::from(desktop.1) * u64::from(self.size.0);
        let (num, den) = if wide {
            (desktop.0, self.size.0)
        } else {
            (desktop.1, self.size.1)
        };
        let scale = |v: u32| muldiv(v, num, den);
        let margin = (
            (desktop.0 - scale(self.size.0)) / 2,
            (desktop.1 - scale(self.size.1)) / 2,
        );
        self.screens
            .iter()
            .map(|&Offset { screen, at, size }| Placed {
                screen,
                target: rect(
                    (margin.0 + scale(at.0), margin.1 + scale(at.1)),
                    (even(scale(size.0)), even(scale(size.1))),
                ),
            })
            .collect()
    }
}

/// Logical offsets become stream pixels at the largest scale among the screens on each axis,
/// so screens at a lower scale leave a gap and none overlap.
fn whole(screens: &[ScreenRect], order: &[usize]) -> Whole {
    let largest = |axis: fn(&ScreenRect) -> (u32, u32)| {
        screens
            .iter()
            .map(|s| {
                let (pixels, logical) = axis(s);
                (pixels, logical.max(1))
            })
            .max_by(|a, b| {
                (u64::from(a.0) * u64::from(b.1)).cmp(&(u64::from(b.0) * u64::from(a.1)))
            })
            .unwrap_or((1, 1))
    };
    let fx = largest(|s| (s.pixels.0, s.logical.0));
    let fy = largest(|s| (s.pixels.1, s.logical.1));
    let left = screens.iter().map(|s| s.position.0).min().unwrap_or(0);
    let top = screens.iter().map(|s| s.position.1).min().unwrap_or(0);
    let offset = |v: i32, min: i32, (num, den): (u32, u32)| {
        muldiv(
            u32::try_from(i64::from(v) - i64::from(min)).unwrap_or(u32::MAX),
            num,
            den,
        )
    };
    let screens: Vec<_> = order
        .iter()
        .map(|&i| {
            let s = &screens[i];
            let at = (
                offset(s.position.0, left, fx),
                offset(s.position.1, top, fy),
            );
            Offset {
                screen: i,
                at,
                size: s.pixels,
            }
        })
        .collect();
    let size = screens.iter().fold((0, 0), |(w, h), o| {
        (
            w.max(o.at.0.saturating_add(o.size.0)),
            h.max(o.at.1.saturating_add(o.size.1)),
        )
    });
    Whole { size, screens }
}

/// The desktop `client`'s monitors span when they give Paired or Scaled, whatever the screens.
pub fn client_desktop(client: &[Monitor]) -> Option<(u16, u16)> {
    of_client(client).ok().flatten().map(|(desktop, _)| desktop)
}

/// A client's desktop size and its monitors moved so their box starts at (0, 0).
type ClientDesktop = ((u16, u16), Vec<Monitor>);

/// [`usable`] for two or more client monitors; fewer give Whole.
fn of_client(client: &[Monitor]) -> Result<Option<ClientDesktop>, &'static str> {
    match client.len() {
        0 | 1 => Ok(None),
        _ => usable(client).map(Some),
    }
}

fn usable(client: &[Monitor]) -> Result<ClientDesktop, &'static str> {
    if client.len() > MONITOR_COUNT_MAX {
        return Err("more than 16 monitors");
    }
    if client.iter().any(|m| m.right < m.left || m.bottom < m.top) {
        return Err("a monitor has no size");
    }
    // The acceptor echoes the client's monitors only under the same rules: exactly one
    // primary, at (0, 0) (MS-RDPBCGR 2.2.1.3.6.1).
    let mut primaries = client
        .iter()
        .filter(|m| m.flags.contains(MonitorFlags::PRIMARY));
    let (Some(primary), None) = (primaries.next(), primaries.next()) else {
        return Err("not exactly one primary monitor");
    };
    if (primary.left, primary.top) != (0, 0) {
        return Err("the primary monitor is not at 0,0");
    }
    let left = client.iter().map(|m| m.left).min().unwrap_or(0);
    let top = client.iter().map(|m| m.top).min().unwrap_or(0);
    let right = client.iter().map(|m| m.right).max().unwrap_or(0);
    let bottom = client.iter().map(|m| m.bottom).max().unwrap_or(0);
    let side = |min: i32, max: i32| u16::try_from(i64::from(max) - i64::from(min) + 1).ok();
    let (Some(w), Some(h)) = (side(left, right), side(top, bottom)) else {
        return Err("the monitors span more than 65535 pixels");
    };
    let moved = client
        .iter()
        .map(|m| Monitor {
            left: m.left - left,
            top: m.top - top,
            right: m.right - left,
            bottom: m.bottom - top,
            flags: m.flags,
        })
        .collect();
    Ok(((w, h), moved))
}

/// A `pixels`-sized picture fitted into `m` with its aspect kept, centred. `m` lies inside a
/// u16 desktop starting at (0, 0).
fn fit(pixels: (u32, u32), m: &Monitor) -> Rect16 {
    let side = |min: i32, max: i32| u32::try_from(max - min + 1).unwrap_or(0);
    let (mw, mh) = (side(m.left, m.right), side(m.top, m.bottom));
    let (sw, sh) = (pixels.0.max(1), pixels.1.max(1));
    let (w, h) = if u64::from(mw) * u64::from(sh) <= u64::from(mh) * u64::from(sw) {
        (mw, muldiv(sh, mw, sw))
    } else {
        (muldiv(sw, mh, sh), mh)
    };
    let (w, h) = (even(w), even(h));
    let origin =
        |min: i32, room: u32, used: u32| u32::try_from(min).unwrap_or(0) + (room - used) / 2;
    rect((origin(m.left, mw, w), origin(m.top, mh, h)), (w, h))
}

/// The one primary monitor ResetGraphics lists for a Whole desktop.
fn primary((w, h): (u16, u16)) -> Monitor {
    Monitor {
        left: 0,
        top: 0,
        right: i32::from(w) - 1,
        bottom: i32::from(h) - 1,
        flags: MonitorFlags::PRIMARY,
    }
}

fn rect(at: (u32, u32), size: (u32, u32)) -> Rect16 {
    let (x, y) = clamp16(at);
    let (w, h) = clamp16(size);
    Rect16 { x, y, w, h }
}

fn clamp16((a, b): (u32, u32)) -> (u16, u16) {
    let c = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
    (c(a), c(b))
}

fn even(v: u32) -> u32 {
    v & !1
}

/// `v * num / den`, rounded down; 0 when `den` is 0.
fn muldiv(v: u32, num: u32, den: u32) -> u32 {
    if den == 0 {
        return 0;
    }
    u32::try_from(u64::from(v) * u64::from(num) / u64::from(den)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(x: i32, y: i32, w: u32, h: u32) -> ScreenRect {
        ScreenRect {
            position: (x, y),
            logical: (w, h),
            pixels: (w, h),
        }
    }

    fn monitor(left: i32, top: i32, right: i32, bottom: i32, primary: bool) -> Monitor {
        Monitor {
            left,
            top,
            right,
            bottom,
            flags: if primary {
                MonitorFlags::PRIMARY
            } else {
                MonitorFlags::empty()
            },
        }
    }

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect16 {
        Rect16 { x, y, w, h }
    }

    fn targets(layout: &Layout) -> Vec<(usize, Rect16)> {
        layout.placed.iter().map(|p| (p.screen, p.target)).collect()
    }

    #[test]
    fn one_screen_and_no_client_monitors_is_today() {
        let layout = Layout::new(&[screen(0, 0, 1920, 1200)], &[]);
        assert_eq!(layout.kind, Kind::Whole);
        assert_eq!(layout.desktop, (1920, 1200));
        assert_eq!(targets(&layout), vec![(0, rect(0, 0, 1920, 1200))]);
        assert_eq!(layout.monitors, vec![monitor(0, 0, 1919, 1199, true)]);
    }

    #[test]
    fn two_screens_whole_without_client_monitors() {
        let layout = Layout::new(
            &[screen(0, 0, 1920, 1080), screen(1920, 0, 1280, 1024)],
            &[],
        );
        assert_eq!(layout.kind, Kind::Whole);
        assert_eq!(layout.desktop, (3200, 1080));
        assert_eq!(
            targets(&layout),
            vec![(0, rect(0, 0, 1920, 1080)), (1, rect(1920, 0, 1280, 1024))]
        );
    }

    #[test]
    fn one_client_monitor_is_whole() {
        let layout = Layout::new(
            &[screen(0, 0, 1920, 1080), screen(1920, 0, 1280, 1024)],
            &[monitor(0, 0, 2559, 1439, true)],
        );
        assert_eq!(layout.kind, Kind::Whole);
        assert_eq!(layout.desktop, (3200, 1080));
    }

    #[test]
    fn whole_screens_at_mixed_scales_take_the_largest_scale_per_axis() {
        let hidpi = ScreenRect {
            position: (1920, 0),
            logical: (1280, 720),
            pixels: (2560, 1440),
        };
        let layout = Layout::new(&[screen(0, 0, 1920, 1080), hidpi], &[]);
        assert_eq!(layout.desktop, (6400, 1440));
        assert_eq!(
            targets(&layout),
            vec![(0, rect(0, 0, 1920, 1080)), (1, rect(3840, 0, 2560, 1440))]
        );
    }

    #[test]
    fn screens_are_placed_in_position_order() {
        let layout = Layout::new(
            &[screen(1920, 0, 1280, 1024), screen(0, 0, 1920, 1080)],
            &[],
        );
        assert_eq!(
            targets(&layout),
            vec![(1, rect(0, 0, 1920, 1080)), (0, rect(1920, 0, 1280, 1024))]
        );
    }

    #[test]
    fn two_monitors_pair_with_two_screens() {
        let layout = Layout::new(
            &[screen(0, 0, 1920, 1200), screen(1920, 0, 1920, 1080)],
            &[
                monitor(0, 0, 2559, 1439, true),
                monitor(2560, 0, 4479, 1079, false),
            ],
        );
        assert_eq!(layout.kind, Kind::Paired);
        assert_eq!(layout.desktop, (4480, 1440));
        assert_eq!(
            targets(&layout),
            vec![
                (0, rect(128, 0, 2304, 1440)),
                (1, rect(2560, 0, 1920, 1080))
            ]
        );
    }

    #[test]
    fn a_secondary_left_of_the_primary_moves_the_box_to_zero() {
        let layout = Layout::new(
            &[screen(0, 0, 1920, 1080)],
            &[
                monitor(-1920, 0, -1, 1079, false),
                monitor(0, 0, 1919, 1079, true),
            ],
        );
        assert_eq!(layout.kind, Kind::Paired);
        assert_eq!(layout.desktop, (3840, 1080));
        assert_eq!(
            layout.monitors,
            vec![
                monitor(0, 0, 1919, 1079, false),
                monitor(1920, 0, 3839, 1079, true),
            ]
        );
        assert_eq!(targets(&layout), vec![(0, rect(1920, 0, 1920, 1080))]);
    }

    #[test]
    fn screens_after_the_first_pair_with_monitors_top_then_left() {
        let layout = Layout::new(
            &[
                screen(0, 0, 1920, 1080),
                screen(1920, 0, 1920, 1080),
                screen(0, 1080, 1920, 1080),
            ],
            &[
                monitor(0, 1080, 1919, 2159, false),
                monitor(0, 0, 1919, 1079, true),
                monitor(-1920, 1080, -1, 2159, false),
                monitor(-1920, 0, -1, 1079, false),
            ],
        );
        assert_eq!(
            targets(&layout),
            vec![
                (0, rect(1920, 0, 1920, 1080)),
                (1, rect(0, 0, 1920, 1080)),
                (2, rect(0, 1080, 1920, 1080)),
            ]
        );
    }

    #[test]
    fn a_wider_monitor_letterboxes_and_its_bars_take_no_input() {
        let layout = Layout::new(
            &[screen(0, 0, 1920, 1200)],
            &[
                monitor(0, 0, 2559, 1079, true),
                monitor(2560, 0, 4479, 1079, false),
            ],
        );
        assert_eq!(targets(&layout), vec![(0, rect(416, 0, 1728, 1080))]);
        assert_eq!(layout.to_screen((10, 10)), None);
        assert_eq!(layout.to_screen((3000, 540)), None, "an unpaired monitor");
        assert_eq!(layout.to_screen((416 + 864, 540)), Some((0, (960, 600))));
    }

    #[test]
    fn three_screens_on_two_monitors_scale_together() {
        let layout = Layout::new(
            &[
                screen(0, 0, 1920, 1080),
                screen(1920, 0, 1920, 1080),
                screen(3840, 0, 1920, 1080),
            ],
            &[
                monitor(0, 0, 1919, 1079, true),
                monitor(1920, 0, 3839, 1079, false),
            ],
        );
        assert_eq!(layout.kind, Kind::Scaled);
        assert_eq!(layout.desktop, (3840, 1080));
        assert_eq!(
            layout.monitors,
            vec![
                monitor(0, 0, 1919, 1079, true),
                monitor(1920, 0, 3839, 1079, false),
            ]
        );
        assert_eq!(
            targets(&layout),
            vec![
                (0, rect(0, 180, 1280, 720)),
                (1, rect(1280, 180, 1280, 720)),
                (2, rect(2560, 180, 1280, 720)),
            ]
        );
    }

    #[test]
    fn bad_client_monitors_fall_back_to_whole() {
        let screens = [screen(0, 0, 1920, 1080), screen(1920, 0, 1280, 1024)];
        let zero_size = [
            monitor(0, 0, 1919, 1079, true),
            monitor(1920, 0, 1919, 1079, false),
        ];
        let inverted = [
            monitor(0, 0, 1919, 1079, true),
            monitor(1920, 1079, 3839, 0, false),
        ];
        let seventeen: Vec<Monitor> = (0..17)
            .map(|i| monitor(i * 100, 0, i * 100 + 99, 99, i == 0))
            .collect();
        let no_primary = [
            monitor(0, 0, 1919, 1079, false),
            monitor(1920, 0, 3839, 1079, false),
        ];
        let two_primaries = [
            monitor(0, 0, 1919, 1079, true),
            monitor(1920, 0, 3839, 1079, true),
        ];
        let primary_off_origin = [
            monitor(-1920, 0, -1, 1079, false),
            monitor(1, 0, 1920, 1079, true),
        ];
        for client in [
            &zero_size[..],
            &inverted,
            &seventeen,
            &no_primary,
            &two_primaries,
            &primary_off_origin,
        ] {
            let layout = Layout::new(&screens, client);
            assert_eq!(layout.kind, Kind::Whole, "{client:?} is ignored");
            assert_eq!(layout.desktop, (3200, 1080));
        }
    }

    #[test]
    fn odd_targets_round_down_to_even() {
        let whole = Layout::new(&[screen(0, 0, 1365, 767)], &[]);
        assert_eq!(whole.desktop, (1365, 767));
        assert_eq!(targets(&whole), vec![(0, rect(0, 0, 1364, 766))]);

        let paired = Layout::new(
            &[screen(0, 0, 1366, 768)],
            &[
                monitor(0, 0, 1919, 1079, true),
                monitor(1920, 0, 3839, 1079, false),
            ],
        );
        assert_eq!(targets(&paired), vec![(0, rect(0, 1, 1920, 1078))]);
    }

    #[test]
    fn to_client_inverts_to_screen() {
        let paired = Layout::new(
            &[screen(0, 0, 1920, 1200), screen(1920, 0, 1920, 1080)],
            &[
                monitor(0, 0, 2559, 1439, true),
                monitor(2560, 0, 4479, 1079, false),
            ],
        );
        let scaled = Layout::new(
            &[
                screen(0, 0, 1920, 1080),
                screen(1920, 0, 1920, 1080),
                screen(3840, 0, 1920, 1080),
            ],
            &[
                monitor(0, 0, 1919, 1079, true),
                monitor(1920, 0, 3839, 1079, false),
            ],
        );
        for layout in [&paired, &scaled] {
            for placed in &layout.placed {
                let t = placed.target;
                for (dx, dy) in [
                    (0, 0),
                    (1, 1),
                    (7, 13),
                    (t.w / 2, t.h / 3),
                    (t.w - 1, t.h - 1),
                ] {
                    let p = (t.x + dx, t.y + dy);
                    let (screen, s) = layout.to_screen(p).expect("inside a target");
                    assert_eq!(screen, placed.screen);
                    let back = layout
                        .to_client(screen, (u32::from(s.0), u32::from(s.1)))
                        .expect("inside the screen");
                    assert!(
                        back.0.abs_diff(p.0) <= 1 && back.1.abs_diff(p.1) <= 1,
                        "{p:?} comes back within 1 px, got {back:?}"
                    );
                }
            }
        }
        assert_eq!(paired.to_client(0, (1920, 0)), None, "past the stream");
        assert_eq!(paired.to_client(2, (0, 0)), None, "no such screen");
    }
}
