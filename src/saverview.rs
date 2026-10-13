//! The screen saver on screen: `[screensaver] name` once the idle timer reaches `[idle] blank_after`
//! (`idle::blank`), or any saver for Reprise Settings' Preview button until the next input. Each output
//! gets a tree in `layers.saver`, above the lock screen: a black rect and what the saver draws over it,
//! stepped by one timer at `saver::STEP_MS` (30 a second) while something moves.
//!
//! Per-step cost: Starfield and Mystify hand their marks to a stroke node the GPU draws at the
//! output's resolution; the logo moves one buffer and uploads again only when its colour changes; Photos uploads a picture
//! once and then only moves the new one's opacity during the fade; Blank runs no timer.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use reprisefx::{Stroke, Strokes};
use reprisekit::saver::{Kind, Mark, STEP_MS, Saver};
use reprisescene::{BufferId, CustomElement, CustomNode, Input, RectId, TreeId};
use smithay::{
    backend::renderer::gles::GlesRenderer,
    input::pointer::CursorImageStatus,
    output::Output,
    reexports::calloop::{
        RegistrationToken, channel,
        timer::{TimeoutAction, Timer},
    },
    utils::{Logical, Physical, Point, Rectangle, Scale, Size},
};

use crate::compositor::State;
use crate::glass::GlassCtx;
use crate::raster::Canvas;

/// Input this soon after Preview starts a saver is the click that asked for it, not a wish to stop.
const PREVIEW_GRACE: Duration = Duration::from_millis(800);

#[derive(Default)]
pub struct SaverState {
    run: Option<Run>,
    generation: u64,
}

struct Run {
    kind: Kind,
    grace_until: Option<Instant>,
    outs: Vec<Out>,
    timer: Option<RegistrationToken>,
    photos: Option<RegistrationToken>,
    photo_tx: Option<channel::Sender<Decoded>>,
    last: Instant,
    generation: u64,
}

struct Decoded {
    generation: u64,
    out: usize,
    path: Option<PathBuf>,
    picture: Option<Canvas>,
}

struct Out {
    tree: TreeId,
    _black: RectId,
    saver: Saver,
    /// Starfield and Mystify: the marks the stroke node draws, and its commit.
    marks: Option<(SharedMarks, Rc<Strokes>)>,
    logo: Option<(BufferId, Option<usize>)>,
    shown: Option<BufferId>,
    incoming: Option<BufferId>,
    current: Option<PathBuf>,
}

/// Start `kind` on every output, replacing any saver running. `preview` is Preview's run, which the next input after a moment stops.
pub fn start(state: &mut State, kind: Kind, preview: bool) {
    stop(state);
    state.saver.generation += 1;
    let generation = state.saver.generation;
    log::info!(
        "screen saver: {}{}",
        kind.name(),
        if preview { " (preview)" } else { "" }
    );
    let opts = state.config.screensaver.options.clone();
    let outputs: Vec<Output> = state.scene.outputs().to_vec();
    let mut outs = Vec::new();
    for (i, o) in outputs.into_iter().enumerate() {
        let b = state.layout_box(Some(&o));
        if b.width <= 0 {
            continue;
        }
        let tree = state.scene.tree_create(state.layers.saver);
        state.scene.set_position(tree, b.x, b.y);
        let black = state
            .scene
            .rect_create(tree, (b.width, b.height), [0.0, 0.0, 0.0, 1.0]);
        let seed = generation as u32 ^ (i as u32 + 1).wrapping_mul(0x9E37_79B9);
        let saver = Saver::new(
            kind,
            &opts,
            b.width as u32,
            b.height as u32,
            seed,
            &state.text,
        );
        let marks = matches!(kind, Kind::Starfield | Kind::Mystify).then(|| {
            let marks = Rc::new(RefCell::new(Vec::new()));
            let strokes = Rc::new(Strokes::default());
            let node = state.scene.custom_create(
                tree,
                Box::new(MarksNode {
                    ctx: state.glass.clone(),
                    marks: marks.clone(),
                    strokes: strokes.clone(),
                    extent: Size::from((b.width, b.height)),
                }),
            );
            state.scene.set_input(node, Input::Passthrough);
            (marks, strokes)
        });
        let logo = (kind == Kind::Logo).then(|| (state.scene.buffer_create(tree), None));
        outs.push(Out {
            tree,
            _black: black,
            saver,
            marks,
            logo,
            shown: None,
            incoming: None,
            current: None,
        });
    }
    state.saver.run = Some(Run {
        kind,
        grace_until: preview.then(|| Instant::now() + PREVIEW_GRACE),
        outs,
        timer: None,
        photos: None,
        photo_tx: None,
        last: Instant::now(),
        generation,
    });
    if kind == Kind::Photos {
        listen_for_photos(state);
    }
    tick(state);
}

pub fn running(state: &State) -> bool {
    state.saver.run.is_some()
}

static HIDDEN: CursorImageStatus = CursorImageStatus::Hidden;

/// The pointer an output draws: none while a saver covers the screen; the first input that stops it brings it back.
pub fn cursor<'a>(saver: &SaverState, cursor: &'a CursorImageStatus) -> &'a CursorImageStatus {
    if saver.run.is_some() { &HIDDEN } else { cursor }
}

pub fn stop(state: &mut State) {
    let Some(run) = state.saver.run.take() else {
        return;
    };
    if let Some(t) = run.timer {
        state.handle.remove(t);
    }
    if let Some(t) = run.photos {
        state.handle.remove(t);
    }
    for o in run.outs {
        state.scene.destroy(o.tree);
    }
    state.redraw_all();
}

pub fn activity(state: &mut State) {
    let Some(run) = &state.saver.run else {
        return;
    };
    if run.grace_until.is_some_and(|t| Instant::now() < t) {
        return;
    }
    stop(state);
}

pub fn outputs_changed(state: &mut State) {
    let Some(run) = &state.saver.run else {
        return;
    };
    let (kind, preview) = (run.kind, run.grace_until.is_some());
    start(state, kind, preview);
}

/// One step: move every output's saver on by the time since the last, show it, and arm the timer for the next change.
fn tick(state: &mut State) {
    let Some(run) = state.saver.run.as_mut() else {
        return;
    };
    run.timer = None;
    if state.idle.steps.screen_off {
        return;
    }
    let ms = run.last.elapsed().as_millis() as u64;
    run.last = Instant::now();
    for o in &mut run.outs {
        o.saver.advance(ms);
    }
    show(state);
    request_photos(state);
    let Some(run) = state.saver.run.as_mut() else {
        return;
    };
    let wait = match run.kind {
        Kind::Blank => None,
        Kind::Photos => run
            .outs
            .iter()
            .filter_map(|o| o.saver.photo_wait_ms())
            .min(),
        _ => Some(STEP_MS),
    };
    let Some(wait) = wait else {
        return;
    };
    let timer = Timer::from_duration(Duration::from_millis(wait));
    match state.handle.insert_source(timer, |_, _, state| {
        tick(state);
        TimeoutAction::Drop
    }) {
        Ok(t) => {
            if let Some(run) = state.saver.run.as_mut() {
                run.timer = Some(t);
            }
        }
        Err(e) => log::warn!("screen saver timer: {}", e.error),
    }
}

fn show(state: &mut State) {
    let Some(mut run) = state.saver.run.take() else {
        return;
    };
    let scene = &mut state.scene;
    for o in &mut run.outs {
        if let Some((marks, strokes)) = &o.marks {
            *marks.borrow_mut() = o.saver.marks();
            strokes.changed();
        }
        if let Some((buf, uploaded)) = &mut o.logo {
            let color = o.saver.logo_color();
            if *uploaded != color
                && let Some(word) = o.saver.logo()
            {
                crate::output::upload_scaled(scene, *buf, &word, 1);
                *uploaded = color;
            }
            if let Some((x, y)) = o.saver.logo_at() {
                scene.set_position(*buf, x, y);
            }
        }
        if o.shown.is_some() || o.incoming.is_some() {
            match (o.saver.photo_fade(), o.incoming) {
                (Some(level), Some(buf)) => scene.set_opacity(buf, level),
                (None, Some(buf)) => {
                    scene.set_opacity(buf, 1.0);
                    if let Some(old) = o.shown.replace(buf) {
                        scene.destroy(old);
                    }
                    o.incoming = None;
                }
                _ => {}
            }
        }
    }
    state.saver.run = Some(run);
    state.redraw_all();
}

type SharedMarks = Rc<RefCell<Vec<Mark>>>;

/// Starfield's or Mystify's marks over one output, in logical units from the tree's corner.
struct MarksNode {
    ctx: Rc<GlassCtx>,
    marks: SharedMarks,
    strokes: Rc<Strokes>,
    extent: Size<i32, Logical>,
}

impl CustomNode<GlesRenderer> for MarksNode {
    fn size(&self) -> Size<i32, Logical> {
        self.extent
    }

    fn elements(
        &self,
        _renderer: &mut GlesRenderer,
        location: Point<f64, Physical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> Vec<CustomElement<GlesRenderer>> {
        let Some(fx) = self.ctx.stroke_fx() else {
            return Vec::new();
        };
        let geometry = Rectangle::new(
            location.to_i32_round(),
            self.extent.to_f64().to_physical(scale).to_i32_round(),
        );
        let strokes = to_physical(
            &self.marks.borrow(),
            scale,
            location - geometry.loc.to_f64(),
        );
        vec![CustomElement::new(
            self.strokes.element(&fx, geometry, &strokes, alpha),
        )]
    }
}

/// `marks` (logical) as strokes in physical pixels from the element's corner, which lies `shift`
/// before the node's own.
fn to_physical(marks: &[Mark], scale: Scale<f64>, shift: Point<f64, Physical>) -> Vec<Stroke> {
    let (sx, sy) = (scale.x as f32, scale.y as f32);
    let (dx, dy) = (shift.x as f32, shift.y as f32);
    let at = |p: (f32, f32)| [p.0 * sx + dx, p.1 * sy + dy];
    marks
        .iter()
        .map(|m| {
            let c = |v: u8| f32::from(v) / 255.0;
            let color = [c(m.color.r), c(m.color.g), c(m.color.b), 1.0];
            if m.from == m.to {
                return Stroke::dot(at(m.from), m.width * sx, color);
            }
            Stroke {
                from: at(m.from),
                to: at(m.to),
                width: m.width * sx,
                color,
            }
        })
        .collect()
}

fn listen_for_photos(state: &mut State) {
    let (tx, rx) = channel::channel::<Decoded>();
    let inserted = state.handle.insert_source(rx, |event, _, state| {
        let channel::Event::Msg(d) = event else {
            return;
        };
        let Some(run) = state.saver.run.as_mut() else {
            return;
        };
        if run.generation != d.generation {
            return;
        }
        let Some(o) = run.outs.get_mut(d.out) else {
            return;
        };
        let Some(picture) = d.picture else {
            log::info!("screen saver: no pictures to show");
            return;
        };
        o.current = d.path;
        o.saver.push_photo(picture.clone());
        let buf = state.scene.buffer_create(o.tree);
        crate::output::upload_scaled(&mut state.scene, buf, &picture, 1);
        state.scene.set_opacity(buf, 0.0);
        if let Some(old) = o.incoming.replace(buf) {
            state.scene.destroy(old);
        }
        if let Some(t) = run.timer.take() {
            state.handle.remove(t);
        }
        run.last = Instant::now();
        tick(state);
    });
    match inserted {
        Ok(t) => {
            if let Some(run) = state.saver.run.as_mut() {
                run.photos = Some(t);
                run.photo_tx = Some(tx);
            }
        }
        Err(e) => log::warn!("screen saver photos: {}", e.error),
    }
}

fn request_photos(state: &mut State) {
    let folder = state.config.screensaver.folder.clone();
    let Some(run) = state.saver.run.as_mut() else {
        return;
    };
    let Some(tx) = run.photo_tx.clone() else {
        return;
    };
    for (i, o) in run.outs.iter_mut().enumerate() {
        if !o.saver.take_photo_request() {
            continue;
        }
        let (w, h) = o.saver.size();
        let current = o.current.clone();
        let (tx, folder, generation) = (tx.clone(), folder.clone(), run.generation);
        let spawned = std::thread::Builder::new()
            .name("saver-photo".into())
            .spawn(move || {
                let (path, picture) = decode_next(folder.as_deref(), current.as_deref(), w, h);
                let _ = tx.send(Decoded {
                    generation,
                    out: i,
                    path,
                    picture,
                });
            });
        if let Err(e) = spawned {
            log::warn!("screen saver photos: no decode thread: {e}");
        }
    }
}

/// On a thread: pick at random the picture after `current` in `folder` (Pictures when None) and decode
/// it fitted to `w` x `h`. A file that does not decode is skipped.
fn decode_next(
    folder: Option<&Path>,
    current: Option<&Path>,
    w: u32,
    h: u32,
) -> (Option<PathBuf>, Option<Canvas>) {
    let folder = folder
        .map(Path::to_path_buf)
        .or_else(|| reprisefs::places::user_dir("PICTURES", "Pictures"));
    let list = folder
        .as_deref()
        .map(reprisefs::pictures::in_folder)
        .unwrap_or_default();
    let mut current = current.map(Path::to_path_buf);
    for _ in 0..list.len().min(8) {
        let Some(path) =
            crate::slideshow::next(&list, current.as_deref(), true, crate::slideshow::roll())
        else {
            break;
        };
        if let Some(picture) = crate::wallpaper::load_fit(&path, w, h) {
            return (Some(path), Some(picture));
        }
        current = Some(path);
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::input::pointer::CursorIcon;

    fn running(kind: Kind) -> SaverState {
        SaverState {
            run: Some(Run {
                kind,
                grace_until: None,
                outs: Vec::new(),
                timer: None,
                photos: None,
                photo_tx: None,
                last: Instant::now(),
                generation: 1,
            }),
            generation: 1,
        }
    }

    #[test]
    fn marks_reach_physical_pixels_at_the_output_scale_and_stars_snap() {
        use reprisekit::theme::Color;
        let m = Mark {
            from: (10.0, 20.0),
            to: (30.0, 20.0),
            width: 2.0,
            color: Color::rgb(255, 0, 51),
        };
        let s = to_physical(&[m], Scale::from(2.0), Point::from((0.5, -0.25)));
        assert_eq!(
            s,
            vec![Stroke {
                from: [20.5, 39.75],
                to: [60.5, 39.75],
                width: 4.0,
                color: [1.0, 0.0, 0.2, 1.0],
            }]
        );
        let star = Mark { to: m.from, ..m };
        assert_eq!(
            to_physical(&[star], Scale::from(1.5), Point::from((0.0, 0.0)))[0],
            Stroke::dot([15.0, 30.0], 3.0, [1.0, 0.0, 0.2, 1.0]),
            "a star is a dot on whole pixels"
        );
    }

    #[test]
    fn a_running_saver_hides_the_pointer_and_its_end_shows_it() {
        let arrow = CursorImageStatus::Named(CursorIcon::Default);
        assert!(matches!(
            cursor(&running(Kind::Starfield), &arrow),
            CursorImageStatus::Hidden
        ));
        assert!(matches!(
            cursor(&SaverState::default(), &arrow),
            CursorImageStatus::Named(CursorIcon::Default)
        ));
    }
}
