//! Drawing the stack and the chip into the scene.

use reprisekit::controls::notifysheet::{
    SheetIcons, SheetLayout, chip_layout, draw_chip, drip_bead, drip_bead_share, notify_look,
    place_stack, sheet_hit,
};
use reprisekit::controls::tooltip::{draw_notify_balloon, place_notify};
use reprisekit::look::widgets::BUTTON_ICON;
use reprisekit::taskbar::Edge;
use reprisekit::textedit::Caret;
use reprisescene::Input;
use smithay::output::Output;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use std::time::{Duration, Instant};

use crate::caret::Editor;
use crate::compositor::State;
use crate::glass::Backdrop;
use crate::output::upload_scaled;
use crate::raster::{Canvas, Rect};

use super::clock::{hold_clocks, rearm};
use super::layout::{Drawn, Painted, lay_out_open, sheet_glass};
use super::motion::{aim_open, arm_tick, clip_to, follow_slot, pose_balloon};
use super::pointer::{rehold, rehover};
use super::stack::chip_count;
use super::warm::warm_openings;
use super::{AGE_TICK, Balloon, Bead, Chip, NotifyView, cap, overlay_tree, replying};

/// A sheet's app line time: "now", then minutes, then hours.
pub(super) fn relative(d: Duration) -> String {
    match d.as_secs() / 60 {
        0 => "now".to_string(),
        m @ 1..60 => format!("{m} min ago"),
        m => format!("{} h ago", m / 60),
    }
}

/// Redraws the sheets' app lines every `AGE_TICK` while one is drawn.
fn arm_age(state: &mut State) {
    if state.notify.age_timer.is_some() {
        return;
    }
    match state
        .handle
        .insert_source(Timer::from_duration(AGE_TICK), |_, _, state| {
            paint(state);
            TimeoutAction::ToDuration(AGE_TICK)
        }) {
        Ok(t) => state.notify.age_timer = Some(t),
        Err(e) => log::warn!("notification age timer: {}", e.error),
    }
}

fn disarm_age(state: &mut State) {
    if let Some(t) = state.notify.age_timer.take() {
        state.handle.remove(t);
    }
}

/// Lays out, places and draws every balloon and the chip, and arms the clocks and the app line
/// tick for what is drawn; a stack of sheets at rest then re-checks the still pointer.
pub(super) fn paint(state: &mut State) {
    hold_clocks(state);
    rearm(state);
    aim_open(state);
    if state.notify.stack.is_empty() {
        state.notify.pointer_on = false;
        drop_chip(state);
        disarm_age(state);
        return;
    }
    if !lay_stack(state, false) {
        return;
    }
    rehold(state, Instant::now());
    if replying(&state.notify).is_none() {
        state.ime_field_drawn(Editor::NotifyReply, None, (0, 0), 1);
    }
    let v = &state.notify;
    if v.stack
        .iter()
        .any(|b| matches!(b.drawn, Some(Drawn::Sheet(_))))
    {
        arm_age(state);
    } else {
        disarm_age(state);
    }
    redraw(state);
    crate::access::touch(state);
    arm_tick(state);
    warm_openings(state);
    rehover(state);
}

/// Lays out and places every balloon and the chip. All are drawn, or with `opening_only` just
/// the sheets whose opening moved since; false with no output.
pub(super) fn lay_stack(state: &mut State, opening_only: bool) -> bool {
    let Some(output) = state.first_output() else {
        return false;
    };
    if let Some(old) = state.notify.output.replace(output.clone())
        && old != output
    {
        state.notify.left.push(old);
    }
    let p = state.bar_placement(&output);
    let area = crate::barplace::clear_of(state.usable_area(Some(&output)), &p);
    let shape = state.config.glass.notification.shape();
    let now = Instant::now();
    let laid: Vec<(Drawn, Option<[SheetLayout; 2]>, f64)> = state
        .notify
        .stack
        .iter()
        .map(|b| {
            let t = b.motion.open.value(now);
            let (d, ends) = lay_out_open(
                &state.config.theme.scaled(b.scale),
                state.text.for_output(b.scale),
                &b.item,
                b.reply.is_some(),
                p.edge,
                shape,
                &relative(b.shown_at.elapsed()),
                t,
            );
            (d, ends, t)
        })
        .collect();
    let chip_scale = state.draw_scale_at(Some(&output)).max(1);
    let chip = chip_count(state.notify.queue.len(), cap(state)).map(|more| {
        let theme = state.config.theme.scaled(chip_scale);
        let (w, h) = chip_layout(&theme, state.text.for_output(theme.scale), more);
        (more, (w / chip_scale, h / chip_scale))
    });
    let footprints: Vec<(i32, i32)> = laid
        .iter()
        .zip(&state.notify.stack)
        .map(|((d, ..), b)| {
            let (w, h) = d.footprint();
            (w / b.scale, h / b.scale)
        })
        .collect();
    let (spots, chip_at) = place_stack(area, &footprints, chip.map(|c| c.1), p.edge);
    let e = state.config.effects;
    let slide_ms = if e.opens() { e.window_open_ms } else { 0 };
    let same_edge = state.notify.edge.replace(p.edge) == Some(p.edge);
    let opening = state
        .notify
        .stack
        .iter()
        .any(|b| !b.motion.open.settled(now));
    for (i, ((d, ends, t), spot)) in laid.into_iter().zip(spots).enumerate() {
        let b = &mut state.notify.stack[i];
        let at = match &d {
            Drawn::Balloon(lay) => place_notify(area, lay, b.scale, p.edge),
            Drawn::Sheet(_) => spot,
        };
        let sheet = matches!(b.drawn, Some(Drawn::Sheet(_)));
        if !opening
            && same_edge
            && let Some(old @ Drawn::Sheet(_)) = &b.drawn
            && b.at.0 == at.0
        {
            let span = |d: &Drawn| d.footprint().1 / b.scale;
            let (was, now_at) = ((b.at.1, span(old)), (at.1, span(&d)));
            follow_slot(
                &mut b.motion,
                was,
                now_at,
                p.edge == Edge::Top,
                now,
                slide_ms,
            );
        }
        if opening_only && sheet && b.open_drawn == t {
            b.at = at;
            continue;
        }
        paint_one(state, i, d, ends, t, at);
    }
    match chip.zip(chip_at) {
        Some(((more, size), at)) => draw_chip_at(state, more, at, size, chip_scale),
        None => drop_chip(state),
    }
    true
}

/// A sheet whose app icon is missing shows the stock one in its slot, unless its picture fills it.
pub(super) fn slot_needs_icon(lay: &SheetLayout, has_picture: bool) -> bool {
    !has_picture || lay.picture != Some(lay.icon)
}

/// The app icon and picture sheet `b` shows on `lay`, device px.
pub(super) fn sheet_icons(
    icons: &mut crate::icontheme::IconCache,
    b: &Balloon,
    lay: &SheetLayout,
) -> (Option<Canvas>, Option<Canvas>) {
    let slot = lay.icon.width;
    let picture = crate::tray::closest(b.item.image(), slot);
    let app_px = lay.badge.map_or(slot, |r| r.width);
    let app = crate::tray::closest(b.item.app_icon(), app_px).or_else(|| {
        slot_needs_icon(lay, picture.is_some())
            .then(|| icons.get("dialog-information", slot))
            .flatten()
    });
    (app, picture)
}

/// Draws balloon `i` as `drawn` at `at`, logical, a Droplet part way open `t` of the way between
/// its `ends`; one drawn alike before is only placed.
fn paint_one(
    state: &mut State,
    i: usize,
    drawn: Drawn,
    ends: Option<[SheetLayout; 2]>,
    t: f64,
    at: (i32, i32),
) {
    let mut opening = std::mem::take(&mut state.notify.stack[i].opening);
    let mut kept = std::mem::take(&mut state.notify.stack[i].glass);
    let b = &state.notify.stack[i];
    let preedit = b
        .reply
        .as_ref()
        .and_then(|_| state.ime_field_preedit(Editor::NotifyReply));
    let pushed = match b.held {
        Some((part, true)) => Some(part),
        _ => None,
    };
    let looks = Painted {
        drawn: drawn.clone(),
        open: t,
        pushed,
        hot: b.hot,
    };
    let fresh = b.reply.is_some() || b.painted.as_ref() != Some(&looks);
    let scale = b.scale;
    let theme = state.config.theme.scaled(scale);
    let reply = b.reply.as_ref().map(|r| {
        (
            &r.edit,
            Caret::new(r.focused, state.caret.on),
            preedit.as_ref(),
        )
    });
    let (mut canvas, pane) = match &drawn {
        Drawn::Balloon(lay) => {
            let canvas = fresh.then(|| {
                let icon_px = theme.icon_size();
                let icon = crate::tray::closest(b.item.icon(), icon_px)
                    .or_else(|| state.icons.get("dialog-information", icon_px));
                let actions = b.item.button_icon_canvases(BUTTON_ICON * scale);
                draw_notify_balloon(
                    &theme,
                    state.text.for_output(theme.scale),
                    lay,
                    icon.as_ref(),
                    &actions,
                    pushed,
                    b.hot,
                    reply,
                )
            });
            (canvas, None)
        }
        Drawn::Sheet(lay) => {
            let canvas = fresh.then(|| {
                let (app, picture) = sheet_icons(&mut state.icons, b, lay);
                let actions = b.item.button_icon_canvases(BUTTON_ICON * scale);
                let icons = SheetIcons {
                    app: app.as_ref(),
                    picture: picture.as_ref(),
                    actions: &actions,
                };
                let builds = opening.builds();
                let canvas = match &ends {
                    Some([shut, open]) => opening.draw(
                        &mut kept,
                        &theme,
                        state.text.for_output(theme.scale),
                        shut,
                        open,
                        t,
                        icons,
                        b.tint.flatten(),
                        pushed,
                        b.hot,
                        reply,
                    ),
                    None => kept.draw(
                        &theme,
                        state.text.for_output(theme.scale),
                        lay,
                        icons,
                        b.tint.flatten(),
                        pushed,
                        b.hot,
                        reply,
                    ),
                };
                if opening.builds() > builds {
                    log::info!("notify: an opening painted its layers in the frame");
                }
                canvas
            });
            let look = notify_look(&theme);
            let pane = look
                .filter(|_| crate::glass::wanted(&state.config))
                .map(|look| sheet_glass(lay, &look, scale));
            (canvas, pane)
        }
    };
    let replying = b.reply.is_some();
    let (ox, oy) = drawn.origin();
    let (glass_tree, buffer) = (b.glass_tree, b.buffer);
    if let Some(canvas) = &canvas {
        upload_scaled(&mut state.scene, buffer, canvas, scale);
        let input = match &drawn {
            Drawn::Balloon(_) => Input::All,
            Drawn::Sheet(lay) => {
                let lay = lay.clone();
                Input::Test(Box::new(move |p| {
                    let at = |v: f64| (v * f64::from(scale)).floor() as i32;
                    sheet_hit(&lay, at(p.x), at(p.y)).is_some()
                }))
            }
        };
        state.scene.set_input(buffer, input);
    }
    let (edge, scale_from) = (state.notify.edge, state.config.effects.window_scale);
    let State {
        notify,
        scene,
        glass,
        ..
    } = state;
    let bounds = notify.bounds();
    let b = &mut notify.stack[i];
    b.opening = opening;
    b.glass = kept;
    b.painted = Some(looks);
    b.at = at;
    b.drawn = Some(drawn);
    b.open_drawn = t;
    if pane.is_some() {
        b.backdrop.get_or_insert_with(|| Backdrop::new(glass));
    } else if let Some(mut gone) = b.backdrop.take() {
        gone.set(scene, glass, glass_tree, None);
    }
    b.pane = pane;
    if b.motion.drip
        && b.bead.is_none()
        && drip_bead_share(b.motion.k.value(Instant::now())) > 0.0
        && let Some((c, body)) = drip_bead(&theme)
    {
        let buffer = scene.buffer_create(b.tree);
        scene.set_input(buffer, Input::Passthrough);
        upload_scaled(scene, buffer, &c, scale);
        let logical =
            |r: Rect| Rect::new(r.x / scale, r.y / scale, r.width / scale, r.height / scale);
        b.bead = Some(Bead {
            buffer,
            body: logical(body),
            size: (c.width as i32 / scale, c.height as i32 / scale),
        });
    }
    pose_balloon(scene, glass, b, Instant::now(), edge, scale_from, bounds);
    if replying && let Some(canvas) = canvas.as_mut() {
        let origin = (at.0 - ox / scale, at.1 - oy / scale);
        let spot = canvas.take_spot();
        state.ime_field_drawn(Editor::NotifyReply, spot, origin, scale);
    }
}

/// Draws the chip saying `more` wait at `at`, `size` logical.
fn draw_chip_at(state: &mut State, more: usize, at: (i32, i32), size: (i32, i32), scale: i32) {
    let c = match state.notify.chip.take() {
        Some(c) => c,
        None => {
            let tree = overlay_tree(state);
            let buffer = state.scene.buffer_create(tree);
            Chip {
                tree,
                buffer,
                at,
                size,
                more,
                hot: false,
            }
        }
    };
    let canvas = draw_chip(
        &state.config.theme.scaled(scale),
        state.text.for_output(scale),
        more,
        c.hot,
    );
    upload_scaled(&mut state.scene, c.buffer, &canvas, scale);
    state.scene.set_position(c.tree, at.0, at.1);
    let bounds = state.notify.bounds();
    state.scene.set_clip(c.tree, clip_to(bounds, at));
    state.notify.chip = Some(Chip {
        at,
        size,
        more,
        ..c
    });
}

fn drop_chip(state: &mut State) {
    if let Some(c) = state.notify.chip.take() {
        state.scene.destroy(c.tree);
        redraw(state);
    }
}

/// The outputs a change to the stack draws on: the one it is placed on and those it left.
pub(super) fn frame_outputs(v: &NotifyView) -> Option<Vec<Output>> {
    let mut outputs = v.left.clone();
    outputs.extend(v.output.clone());
    (!outputs.is_empty()).then_some(outputs)
}

/// Ask for a frame for a change to the stack on the outputs it draws on; every output before
/// it was placed.
pub(super) fn redraw(state: &mut State) {
    match frame_outputs(&state.notify) {
        Some(outputs) => {
            state.notify.left.clear();
            crate::backend::schedule_on(state, &outputs, crate::backend::Ask::Scene);
        }
        None => state.redraw_all(),
    }
}
