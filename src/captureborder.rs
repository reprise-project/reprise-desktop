//! The red border around each output, window and area being captured (`[privacy]
//! capture_border`). Only the backend's screen frames add it, so no capture shows it.

use std::time::{Duration, Instant};

use smithay::{
    backend::renderer::{
        element::{Id, Kind, solid::SolidColorRenderElement},
        utils::CommitCounter,
    },
    output::Output,
    reexports::calloop::{
        RegistrationToken,
        timer::{TimeoutAction, Timer},
    },
    utils::{Logical, Physical, Rectangle},
};

use crate::compositor::State;

const COLOUR: [u8; 3] = [0xFF, 0x20, 0x20];
/// Design pixels; whole physical pixels at the output's scale.
const WIDTH: i32 = 3;

/// Two wlr-screencopy frames of one area this close together make it a
/// recording rather than a screenshot.
const SUSTAIN: Duration = Duration::from_secs(1);
/// How long a recording's border outlives its last frame.
const HOLD: Duration = Duration::from_secs(1);
const TICK: Duration = Duration::from_millis(250);
/// An ext session gets a border once open this long, so a one-frame
/// screenshot never flashes one.
const GRACE: Duration = Duration::from_millis(300);

/// A wlr-screencopy capture: the output and, for an area, its
/// output-local logical rect.
pub type WlrKey = (Output, Option<Rectangle<i32, Logical>>);

#[derive(Default)]
pub struct CaptureBorder {
    tracks: Vec<Track<WlrKey>>,
    tick: Option<RegistrationToken>,
    ids: Vec<Id>,
    shown: Vec<Edge>,
}

struct Track<K> {
    key: K,
    last: Instant,
    sustained: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Captured<O, W> {
    Output(O),
    Window(W),
    /// An area of an output, output-local logical pixels.
    Region(O, Rectangle<i32, Logical>),
}

pub struct WindowView {
    pub frame: Rectangle<i32, Logical>,
    pub mapped: bool,
    pub minimized: bool,
    pub on_current_desktop: bool,
}

/// A rect the border goes around, global logical pixels: inside its
/// edge, or just outside it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Edge {
    rect: Rectangle<i32, Logical>,
    outside: bool,
}

/// A frame of `key` arrived at `now`: a second one within `SUSTAIN`
/// marks a recording.
fn arrive<K: PartialEq>(tracks: &mut Vec<Track<K>>, key: K, now: Instant) {
    match tracks.iter_mut().find(|t| t.key == key) {
        Some(t) => {
            t.sustained = now.duration_since(t.last) <= SUSTAIN;
            t.last = now;
        }
        None => tracks.push(Track {
            key,
            last: now,
            sustained: false,
        }),
    }
}

/// Drop the tracks that are neither waiting nor fresh.
fn prune<K: PartialEq>(tracks: &mut Vec<Track<K>>, waiting: &[K], now: Instant) {
    tracks.retain(|t| waiting.contains(&t.key) || now.duration_since(t.last) <= SUSTAIN.max(HOLD));
}

/// The wlr captures that get a border: sustained ones until `HOLD`
/// after their last frame, and any with a `copy_with_damage` frame
/// waiting, which only a recorder asks for.
fn live_wlr<'a, K: PartialEq>(
    tracks: &'a [Track<K>],
    waiting: &[K],
    now: Instant,
) -> impl Iterator<Item = &'a K> {
    tracks
        .iter()
        .filter(move |t| {
            waiting.contains(&t.key) || (t.sustained && now.duration_since(t.last) <= HOLD)
        })
        .map(|t| &t.key)
}

/// What `targets` puts a border around, deduplicated: a whole output
/// inside its edge, a shown window and an area just outside theirs.
fn edges<O, W: Copy>(
    targets: &[Captured<O, W>],
    output_rect: impl Fn(&O) -> Option<Rectangle<i32, Logical>>,
    window: impl Fn(W) -> Option<WindowView>,
) -> Vec<Edge> {
    let mut out: Vec<Edge> = Vec::new();
    for t in targets {
        let edge = match t {
            Captured::Output(o) => output_rect(o).map(|rect| Edge {
                rect,
                outside: false,
            }),
            Captured::Window(w) => window(*w)
                .filter(|v| v.mapped && !v.minimized && v.on_current_desktop)
                .map(|v| Edge {
                    rect: v.frame,
                    outside: true,
                }),
            Captured::Region(o, r) => output_rect(o).and_then(|geo| {
                let rect = Rectangle::new(geo.loc + r.loc, r.size).intersection(geo)?;
                Some(Edge {
                    rect,
                    outside: true,
                })
            }),
        };
        if let Some(e) = edge
            && !out.contains(&e)
        {
            out.push(e);
        }
    }
    out
}

/// `edge`'s four strips on the output at `output` (logical) and `scale`,
/// output-local physical pixels: top, bottom, left, right. A border
/// outside its rect stays inside `bounds`, so a maximized window's shows.
fn strips(
    edge: Edge,
    bounds: Rectangle<i32, Logical>,
    output: Rectangle<i32, Logical>,
    scale: f64,
) -> Option<[Rectangle<i32, Physical>; 4]> {
    let outer = if edge.outside {
        let mut r = edge.rect;
        r.loc -= (WIDTH, WIDTH).into();
        r.size += (2 * WIDTH, 2 * WIDTH).into();
        r.intersection(bounds)?
    } else {
        edge.rect
    };
    if !outer.overlaps(output) {
        return None;
    }
    let px = |v: i32, origin: i32| (f64::from(v - origin) * scale).round() as i32;
    let x0 = px(outer.loc.x, output.loc.x);
    let y0 = px(outer.loc.y, output.loc.y);
    let x1 = px(outer.loc.x + outer.size.w, output.loc.x);
    let y1 = px(outer.loc.y + outer.size.h, output.loc.y);
    let w = ((f64::from(WIDTH) * scale).round() as i32).max(1);
    let rect = |x, y, w: i32, h: i32| Rectangle::new((x, y).into(), (w.max(0), h.max(0)).into());
    Some([
        rect(x0, y0, x1 - x0, w),
        rect(x0, y1 - w, x1 - x0, w),
        rect(x0, y0 + w, w, y1 - y0 - 2 * w),
        rect(x1 - w, y0 + w, w, y1 - y0 - 2 * w),
    ])
}

fn current_edges(state: &State) -> Vec<Edge> {
    if !state.config.capture_border {
        return Vec::new();
    }
    let mut targets = state.captures.targets(GRACE);
    let waiting = state.captures.wlr_waiting();
    let now = Instant::now();
    targets.extend(
        live_wlr(&state.capture_border.tracks, &waiting, now).map(|(o, r)| match r {
            Some(r) => Captured::Region(o.clone(), *r),
            None => Captured::Output(o.clone()),
        }),
    );
    if targets.is_empty() {
        return Vec::new();
    }
    let locked = state.is_locked();
    edges(&targets, reprisescene::output_rect, |w| {
        let win = state.win(w)?;
        let f = win.frame_rect();
        Some(WindowView {
            frame: Rectangle::new((f.x, f.y).into(), (f.width, f.height).into()),
            mapped: win.mapped && !locked,
            minimized: state.minimized_along_chain(w),
            on_current_desktop: state.on_current_desktop(w),
        })
    })
}

/// The border's elements on `output`, for the screen only.
pub fn elements(state: &mut State, output: &Output) -> Vec<SolidColorRenderElement> {
    let edges = current_edges(state);
    if edges.is_empty() {
        return Vec::new();
    }
    let Some(geo) = reprisescene::output_rect(output) else {
        return Vec::new();
    };
    let bounds = state
        .scene
        .outputs()
        .iter()
        .filter_map(reprisescene::output_rect)
        .fold(geo, |a, b| a.merge(b));
    let scale = output.current_scale().fractional_scale();
    let rects: Vec<Rectangle<i32, Physical>> = edges
        .iter()
        .filter_map(|e| strips(*e, bounds, geo, scale))
        .flatten()
        .filter(|r| !r.is_empty())
        .collect();
    let ids = &mut state.capture_border.ids;
    if ids.len() < rects.len() {
        ids.resize_with(rects.len(), Id::new);
    }
    let [r, g, b] = COLOUR.map(|c| f32::from(c) / 255.0);
    rects
        .into_iter()
        .zip(ids.iter())
        .map(|(rect, id)| {
            SolidColorRenderElement::new(
                id.clone(),
                rect,
                CommitCounter::default(),
                [r, g, b, 1.0],
                Kind::Unspecified,
            )
        })
        .collect()
}

/// Captures or the setting changed: redraw every output if the border did.
pub fn refresh(state: &mut State) {
    let now = current_edges(state);
    if now != state.capture_border.shown {
        log::info!("capture border around {} rects", now.len());
        state.capture_border.shown = now;
        crate::backend::schedule_all(state, crate::backend::Ask::Scene);
    }
}

/// An ext session opened: look again once it is `GRACE` old.
pub fn session_opened(state: &mut State) {
    let armed = state
        .handle
        .insert_source(Timer::from_duration(GRACE), |_, _, state| {
            refresh(state);
            TimeoutAction::Drop
        });
    if let Err(e) = armed {
        log::warn!("capture border: timer: {e}");
    }
}

/// A wlr-screencopy frame of `key` arrived (and is filled or waiting).
pub fn wlr_frame(state: &mut State, key: WlrKey) {
    arrive(&mut state.capture_border.tracks, key, Instant::now());
    if state.capture_border.tick.is_none() {
        match state
            .handle
            .insert_source(Timer::from_duration(TICK), |_, _, state| tick(state))
        {
            Ok(t) => state.capture_border.tick = Some(t),
            Err(e) => log::warn!("capture border: timer: {e}"),
        }
    }
    refresh(state);
}

/// While wlr captures are tracked: forget the stale ones and drop a
/// border whose hold ran out or whose recorder went.
fn tick(state: &mut State) -> TimeoutAction {
    let waiting = state.captures.wlr_waiting();
    prune(&mut state.capture_border.tracks, &waiting, Instant::now());
    refresh(state);
    if state.capture_border.tracks.is_empty() {
        state.capture_border.tick = None;
        return TimeoutAction::Drop;
    }
    TimeoutAction::ToDuration(TICK)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn output(o: &u8) -> Option<Rectangle<i32, Logical>> {
        match o {
            0 => Some(rect(0, 0, 1920, 1080)),
            1 => Some(rect(1920, 0, 1280, 1024)),
            _ => None,
        }
    }

    fn view(frame: Rectangle<i32, Logical>) -> WindowView {
        WindowView {
            frame,
            mapped: true,
            minimized: false,
            on_current_desktop: true,
        }
    }

    #[test]
    fn an_output_is_bordered_inside_its_edge() {
        let e = edges(&[Captured::<u8, u8>::Output(1)], output, |_| None);
        assert_eq!(
            e,
            [Edge {
                rect: rect(1920, 0, 1280, 1024),
                outside: false
            }]
        );
    }

    #[test]
    fn a_window_is_bordered_outside_its_frame() {
        let e = edges(&[Captured::<u8, u8>::Window(7)], output, |_| {
            Some(view(rect(100, 80, 640, 480)))
        });
        assert_eq!(
            e,
            [Edge {
                rect: rect(100, 80, 640, 480),
                outside: true
            }]
        );
    }

    #[test]
    fn a_region_is_bordered_outside_it_in_global_pixels() {
        let e = edges(
            &[Captured::<u8, u8>::Region(1, rect(10, 20, 300, 200))],
            output,
            |_| None,
        );
        assert_eq!(
            e,
            [Edge {
                rect: rect(1930, 20, 300, 200),
                outside: true
            }]
        );
    }

    #[test]
    fn a_minimized_or_other_desktop_or_unmapped_window_has_no_border() {
        let hidden = [
            WindowView {
                minimized: true,
                ..view(rect(0, 0, 10, 10))
            },
            WindowView {
                on_current_desktop: false,
                ..view(rect(0, 0, 10, 10))
            },
            WindowView {
                mapped: false,
                ..view(rect(0, 0, 10, 10))
            },
        ];
        for v in hidden {
            let one = std::cell::Cell::new(Some(v));
            assert!(edges(&[Captured::<u8, u8>::Window(1)], output, |_| one.take()).is_empty());
        }
    }

    #[test]
    fn the_same_target_twice_is_bordered_once() {
        let e = edges(
            &[Captured::<u8, u8>::Output(0), Captured::Output(0)],
            output,
            |_| None,
        );
        assert_eq!(e.len(), 1);
    }

    #[test]
    fn a_single_wlr_frame_gets_no_border_and_sustained_frames_do() {
        let t0 = Instant::now();
        let mut tracks = Vec::new();
        arrive(&mut tracks, 'a', t0);
        assert_eq!(live_wlr(&tracks, &[], t0).count(), 0, "a screenshot");
        arrive(&mut tracks, 'a', t0 + Duration::from_millis(40));
        let at = |ms| t0 + Duration::from_millis(ms);
        assert_eq!(live_wlr(&tracks, &[], at(40)).collect::<Vec<_>>(), [&'a']);
        assert_eq!(live_wlr(&tracks, &[], at(1040)).count(), 1, "held");
        assert_eq!(live_wlr(&tracks, &[], at(1100)).count(), 0, "hold over");
    }

    #[test]
    fn two_frames_far_apart_are_two_screenshots() {
        let t0 = Instant::now();
        let mut tracks = Vec::new();
        arrive(&mut tracks, 'a', t0);
        arrive(&mut tracks, 'a', t0 + Duration::from_secs(3));
        assert_eq!(
            live_wlr(&tracks, &[], t0 + Duration::from_secs(3)).count(),
            0
        );
    }

    #[test]
    fn a_waiting_damage_frame_keeps_the_border_until_it_goes() {
        let t0 = Instant::now();
        let mut tracks = Vec::new();
        arrive(&mut tracks, 'a', t0);
        let late = t0 + Duration::from_secs(60);
        assert_eq!(live_wlr(&tracks, &['a'], late).count(), 1);
        prune(&mut tracks, &['a'], late);
        assert_eq!(tracks.len(), 1);
        prune(&mut tracks, &[], late);
        assert!(tracks.is_empty());
    }

    #[test]
    fn strips_at_scale_1_inside_an_output() {
        let geo = rect(1920, 0, 1280, 1024);
        let edge = Edge {
            rect: geo,
            outside: false,
        };
        let s = strips(edge, rect(0, 0, 3200, 1080), geo, 1.0).unwrap();
        let p = |x, y, w, h| Rectangle::<i32, Physical>::new((x, y).into(), (w, h).into());
        assert_eq!(
            s,
            [
                p(0, 0, 1280, 3),
                p(0, 1021, 1280, 3),
                p(0, 3, 3, 1018),
                p(1277, 3, 3, 1018),
            ]
        );
    }

    #[test]
    fn strips_at_scale_2_are_six_pixels_outside_the_window() {
        let geo = rect(0, 0, 960, 540);
        let edge = Edge {
            rect: rect(100, 50, 200, 100),
            outside: true,
        };
        let s = strips(edge, geo, geo, 2.0).unwrap();
        let p = |x, y, w, h| Rectangle::<i32, Physical>::new((x, y).into(), (w, h).into());
        assert_eq!(
            s,
            [
                p(194, 94, 412, 6),
                p(194, 300, 412, 6),
                p(194, 100, 6, 200),
                p(600, 100, 6, 200),
            ]
        );
    }

    #[test]
    fn a_maximized_window_keeps_its_border_on_screen() {
        let geo = rect(0, 0, 1920, 1080);
        let edge = Edge {
            rect: rect(0, 0, 1920, 1050),
            outside: true,
        };
        let s = strips(edge, geo, geo, 1.0).unwrap();
        assert_eq!(s[0].loc, (0, 0).into());
        assert_eq!(s[1].loc.y, 1050, "below the frame, above the taskbar");
        assert_eq!(s[3].loc.x, 1917);
    }

    #[test]
    fn a_border_off_this_output_draws_nothing_here() {
        let edge = Edge {
            rect: rect(2000, 10, 100, 100),
            outside: true,
        };
        let here = rect(0, 0, 1920, 1080);
        assert!(strips(edge, rect(0, 0, 3200, 1080), here, 1.0).is_none());
    }
}
