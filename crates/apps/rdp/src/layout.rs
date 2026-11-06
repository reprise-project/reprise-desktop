//! Where each shared screen lands on the client's desktop: whole, paired with a client
//! monitor, or all scaled into the client's monitors together.

use ironrdp_pdu::gcc::{MONITOR_COUNT_MAX, Monitor, MonitorFlags};
use std::sync::{Arc, Mutex};

/// The smallest and largest desktop side a client is offered: the acceptor's own bounds, inside
/// ResetGraphics' 32766.
const DESKTOP_SIDES: std::ops::RangeInclusive<u16> = 200..=8192;

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
    /// The monitors ResetGraphics lists, relative to the primary's top left as the client sent
    /// them (MS-RDPBCGR 2.2.1.3.6.1); `placed` targets are relative to the desktop's.
    pub monitors: Vec<Monitor>,
    /// In screen order: by logical position, top then left.
    pub placed: Vec<Placed>,
    /// Each screen's stream size, by screen index.
    pixels: Vec<(u32, u32)>,
}

impl Layout {
    /// Whole without client monitors or with one, Paired when every screen gets a monitor,
    /// else Scaled. Client monitors that cannot be used give Whole, and a Whole box past
    /// [`DESKTOP_SIDES`] is Scaled into the largest desktop of its shape.
    pub fn new(screens: &[ScreenRect], client: &[Monitor]) -> Layout {
        let order = order(screens);
        let pixels = screens.iter().map(|s| s.pixels).collect();
        let whole = whole(screens, &order);
        let Some((desktop, moved)) = of_client(client).ok().flatten() else {
            if whole.size.0.max(whole.size.1) <= u32::from(*DESKTOP_SIDES.end()) {
                return Layout::bitmaps(screens);
            }
            let desktop = fitted(whole.size);
            return Layout {
                kind: Kind::Scaled,
                desktop,
                monitors: vec![primary(desktop)],
                placed: whole.scaled_into(desktop),
                pixels,
            };
        };
        let monitors = client.to_vec();
        if monitors.len() >= screens.len() {
            let mut by_place: Vec<&Monitor> = moved.iter().collect();
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

    /// Whole, every screen at its own pixels as bitmaps are drawn; the desktop stops at
    /// [`DESKTOP_SIDES`], and what lies past it is not shown.
    pub fn bitmaps(screens: &[ScreenRect]) -> Layout {
        let whole = whole(screens, &order(screens));
        let max = *DESKTOP_SIDES.end();
        let (w, h) = clamp16(whole.size);
        let desktop = (w.min(max), h.min(max));
        Layout {
            kind: Kind::Whole,
            desktop,
            monitors: vec![primary(desktop)],
            placed: whole.placed(),
            pixels: screens.iter().map(|s| s.pixels).collect(),
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

    /// The stream size of `screen`.
    pub fn pixels(&self, screen: usize) -> Option<(u32, u32)> {
        self.pixels.get(screen).copied()
    }

    /// Where `screen` is drawn on the client desktop.
    pub fn target(&self, screen: usize) -> Option<Rect16> {
        let placed = self.placed.iter().find(|p| p.screen == screen)?;
        Some(placed.target)
    }

    /// The client point showing stream pixel `p` of `screen`; `None` past the desktop.
    pub fn to_client(&self, screen: usize, p: (u32, u32)) -> Option<(u16, u16)> {
        let t = self.target(screen)?;
        let pixels = self.pixels[screen];
        if p.0 >= pixels.0 || p.1 >= pixels.1 {
            return None;
        }
        let axis = |p: u32, origin: u16, target: u16, pixels: u32, desktop: u16| {
            let at = u32::from(origin) + muldiv(p, u32::from(target), pixels);
            u16::try_from(at).ok().filter(|&at| at < desktop)
        };
        Some((
            axis(p.0, t.x, t.w, pixels.0, self.desktop.0)?,
            axis(p.1, t.y, t.h, pixels.1, self.desktop.1)?,
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

/// Screen indices by logical position, top then left.
fn order(screens: &[ScreenRect]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..screens.len()).collect();
    order.sort_by_key(|&i| (screens[i].position.1, screens[i].position.0));
    order
}

/// The largest desktop within [`DESKTOP_SIDES`] of `size`'s shape, its sides even.
fn fitted((w, h): (u32, u32)) -> (u16, u16) {
    let (min, max) = (
        u32::from(*DESKTOP_SIDES.start()),
        u32::from(*DESKTOP_SIDES.end()),
    );
    let (w, h) = match w >= h {
        true => (max, muldiv(h, max, w)),
        false => (muldiv(w, max, h), max),
    };
    clamp16((even(w).max(min), even(h).max(min)))
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

/// A connection's shared screens and its client's monitors, and the layout they give now.
#[derive(Debug)]
pub struct Desk {
    screens: Vec<ScreenRect>,
    /// Empty when they cannot be used.
    client: Vec<Monitor>,
    /// The client is sent bitmaps: [`Layout::bitmaps`] from then on.
    bitmaps: bool,
    layout: Layout,
}

/// The [`Desk`] a connection's screens and its input share.
pub type SharedDesk = Arc<Mutex<Desk>>;

impl Desk {
    /// Logs why when `client`'s monitors cannot be used.
    pub fn new(screens: Vec<ScreenRect>, client: &[Monitor]) -> Desk {
        let client = match of_client(client) {
            Ok(Some(_)) => client.to_vec(),
            Ok(None) => Vec::new(),
            Err(why) => {
                log::warn!("client monitors ignored: {why}");
                Vec::new()
            }
        };
        let layout = Layout::new(&screens, &client);
        Desk {
            screens,
            client,
            bitmaps: false,
            layout,
        }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// `screen` streams `pixels` now; true if the layout changed.
    pub fn resized(&mut self, screen: usize, pixels: (u32, u32)) -> bool {
        match self.screens.get_mut(screen) {
            Some(s) if s.pixels != pixels => s.pixels = pixels,
            _ => return false,
        }
        self.relayout()
    }

    /// Every screen at its offset, unscaled, for bitmaps from now on; true if the layout
    /// changed.
    pub fn whole(&mut self) -> bool {
        self.bitmaps = true;
        self.relayout()
    }

    fn relayout(&mut self) -> bool {
        let layout = match self.bitmaps {
            true => Layout::bitmaps(&self.screens),
            false => Layout::new(&self.screens, &self.client),
        };
        let changed = layout != self.layout;
        self.layout = layout;
        changed
    }
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
    let overlap = |a: &Monitor, b: &Monitor| {
        a.left <= b.right && b.left <= a.right && a.top <= b.bottom && b.top <= a.bottom
    };
    let mut pairs = (0..client.len()).flat_map(|i| (i + 1..client.len()).map(move |j| (i, j)));
    if pairs.any(|(i, j)| overlap(&client[i], &client[j])) {
        return Err("two monitors overlap");
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
    if !DESKTOP_SIDES.contains(&w) || !DESKTOP_SIDES.contains(&h) {
        return Err("the monitors span less than 200 or more than 8192 pixels");
    }
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
                monitor(-1920, 0, -1, 1079, false),
                monitor(0, 0, 1919, 1079, true),
            ],
            "the reset lists the monitors relative to the primary"
        );
        assert_eq!(targets(&layout), vec![(0, rect(1920, 0, 1920, 1080))]);
    }

    #[test]
    fn a_client_box_the_acceptor_would_refuse_is_whole() {
        let screens = [screen(0, 0, 1920, 1080), screen(1920, 0, 1280, 1024)];
        let five_wide: Vec<Monitor> = (0..5)
            .map(|i| monitor(i * 1920, 0, i * 1920 + 1919, 1079, i == 0))
            .collect();
        let short = [monitor(0, 0, 99, 99, true), monitor(100, 0, 199, 99, false)];
        let widest: Vec<Monitor> = (0..4)
            .map(|i| monitor(i * 2048, 0, i * 2048 + 2047, 1151, i == 0))
            .collect();
        for client in [&five_wide[..], &short] {
            let layout = Layout::new(&screens, client);
            assert_eq!(layout.kind, Kind::Whole, "{client:?} span past 200..=8192");
            assert_eq!(client_desktop(client), None);
        }
        assert_eq!(client_desktop(&widest), Some((8192, 1152)));
    }

    #[test]
    fn unusable_client_monitors_are_logged_once_per_desk() {
        crate::listen::tests::capture_log();
        let two_primaries = [
            monitor(0, 0, 1919, 1079, true),
            monitor(1920, 0, 3839, 1079, true),
        ];
        let mut desk = Desk::new(vec![screen(0, 0, 1920, 1080)], &two_primaries);
        assert!(desk.resized(0, (3840, 2160)));
        assert!(
            !desk.resized(0, (3840, 2160)),
            "the same size changes nothing"
        );
        assert!(desk.resized(0, (1920, 1080)));
        assert_eq!(desk.layout().kind, Kind::Whole);
        assert_eq!(
            crate::listen::tests::times_logged(
                "client monitors ignored: not exactly one primary monitor"
            ),
            1
        );
    }

    #[test]
    fn a_desk_follows_its_screens_and_goes_whole() {
        let mut desk = Desk::new(
            vec![screen(0, 0, 1920, 1200), screen(1920, 0, 1920, 1080)],
            &[
                monitor(0, 0, 2559, 1439, true),
                monitor(2560, 0, 4479, 1079, false),
            ],
        );
        assert_eq!(desk.layout().kind, Kind::Paired);
        assert!(desk.resized(1, (3840, 2160)));
        assert_eq!(desk.layout().target(1), Some(rect(2560, 0, 1920, 1080)));
        assert_eq!(desk.layout().pixels(1), Some((3840, 2160)));
        assert!(desk.whole());
        assert_eq!(desk.layout().kind, Kind::Whole);
        assert_eq!(desk.layout().desktop, (7680, 2160));
        assert!(!desk.whole(), "already whole");
        assert!(desk.resized(1, (1920, 1080)));
        assert_eq!(desk.layout().kind, Kind::Whole, "it stays whole");
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
        let overlapping = [
            monitor(0, 0, 1919, 1079, true),
            monitor(1919, 0, 3838, 1079, false),
        ];
        for client in [
            &zero_size[..],
            &inverted,
            &seventeen,
            &no_primary,
            &two_primaries,
            &primary_off_origin,
            &overlapping,
        ] {
            let layout = Layout::new(&screens, client);
            assert_eq!(layout.kind, Kind::Whole, "{client:?} is ignored");
            assert_eq!(layout.desktop, (3200, 1080));
        }
    }

    #[test]
    fn three_4k_screens_scale_into_8192() {
        let screens = [
            screen(0, 0, 3840, 2160),
            screen(3840, 0, 3840, 2160),
            screen(7680, 0, 3840, 2160),
        ];
        let layout = Layout::new(&screens, &[]);
        assert_eq!(layout.kind, Kind::Scaled);
        assert_eq!(layout.desktop, (8192, 1536));
        assert_eq!(layout.monitors, vec![monitor(0, 0, 8191, 1535, true)]);
        assert_eq!(
            targets(&layout),
            vec![
                (0, rect(0, 0, 2730, 1536)),
                (1, rect(2730, 0, 2730, 1536)),
                (2, rect(5461, 0, 2730, 1536)),
            ]
        );

        let bitmaps = Layout::bitmaps(&screens);
        assert_eq!(bitmaps.kind, Kind::Whole);
        assert_eq!(bitmaps.desktop, (8192, 2160), "bitmaps are cut at 8192");
        assert_eq!(bitmaps.target(2), Some(rect(7680, 0, 3840, 2160)));
        assert_eq!(bitmaps.to_client(2, (511, 0)), Some((8191, 0)));
        assert_eq!(bitmaps.to_client(2, (512, 0)), None, "past the desktop");
    }

    #[test]
    fn a_scale_2_screen_right_of_a_3840_wide_one_scales_into_8192() {
        let laptop = ScreenRect {
            position: (3840, 0),
            logical: (1440, 900),
            pixels: (2880, 1800),
        };
        let layout = Layout::new(&[screen(0, 0, 3840, 2160), laptop], &[]);
        assert_eq!(layout.kind, Kind::Scaled, "the whole box is 10560 wide");
        assert_eq!(layout.desktop, (8192, 1674));
        assert_eq!(
            targets(&layout),
            vec![(0, rect(4, 0, 2976, 1674)), (1, rect(5956, 0, 2232, 1394))]
        );
    }

    #[test]
    fn a_desk_past_8192_for_bitmaps_is_whole_and_cut() {
        let mut desk = Desk::new(
            vec![
                screen(0, 0, 3840, 2160),
                screen(3840, 0, 3840, 2160),
                screen(7680, 0, 3840, 2160),
            ],
            &[],
        );
        assert_eq!(desk.layout().kind, Kind::Scaled);
        assert!(desk.whole());
        assert_eq!(desk.layout().kind, Kind::Whole);
        assert_eq!(desk.layout().desktop, (8192, 2160));
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
