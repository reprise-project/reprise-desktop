//! Tooltip glue: the show delay, hide timer and scene node for taskbar, tray, Welcome, Start menu
//! and desktop tips. A tip sits in `layers.overlay` with no owner tag, so the pointer passes through.

use std::time::Duration;

use reprisekit::classic::frame::{CaptionButton, FrameHit};
use reprisekit::controls::tooltip::{
    INFO_BELOW_POINTER, INFO_BELOW_ROW, SHOW_MS, balloon_size, draw_balloon_tip, draw_info_tip,
    draw_tip, info_size, place, place_balloon, place_below, place_beside, shadow_room, task_tip,
    tip_size,
};
use reprisekit::startpanel::Spot;
use reprisekit::tr;
use reprisescene::{Input, TreeId};
use smithay::output::Output;
use smithay::reexports::calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};

use crate::compositor::State;
use crate::output::upload_scaled;
use crate::raster::Rect;
use crate::taskbar::TaskbarHit;
use crate::window::WindowId;

const HIDE_MS: u64 = 5000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Taskbar {
        bar: TreeId,
        hit: TaskbarHit,
    },
    Dialog(usize, usize),
    Menu {
        depth: usize,
        row: usize,
    },
    Desktop(usize),
    StartPanel(Spot),
    Caption(WindowId, CaptionButton),
    /// A notification's icon button: its id and the button.
    Notify(u32, usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tip {
    Label(String),
    Balloon { title: String, body: String },
    Info(String),
    Icon(Vec<String>),
}

#[derive(Default)]
pub struct TooltipView {
    timer: Option<RegistrationToken>,
    pending: Option<(Target, Tip)>,
    shown: Option<(Target, TreeId)>,
    resting: Option<Target>,
    /// The target a press landed on: it gets no tip again until the
    /// pointer rests on another, so a click whose target moves or changes
    /// its text (the tray arrow) leaves no stale tip behind.
    pressed: Option<Target>,
    /// The pointer where the shown tip opened, so a tip whose text changes redraws in place.
    at: (i32, i32),
}

impl TooltipView {
    fn note_press(&mut self) {
        self.pressed = self.resting;
    }

    fn holds_back(&mut self, target: Target) -> bool {
        if self.pressed == Some(target) {
            return true;
        }
        self.pressed = None;
        false
    }

    fn any(&self, f: impl Fn(&Target) -> bool) -> bool {
        self.shown.as_ref().is_some_and(|(t, _)| f(t))
            || self.pending.as_ref().is_some_and(|(t, _)| f(t))
    }
}

fn arm(state: &mut State, ms: u64) {
    if let Some(t) = state.tooltip.timer.take() {
        state.handle.remove(t);
    }
    if ms == 0 {
        return;
    }
    let timer = Timer::from_duration(Duration::from_millis(ms));
    match state.handle.insert_source(timer, |_, _, state| {
        state.tooltip.timer = None;
        fired(state);
        TimeoutAction::Drop
    }) {
        Ok(t) => state.tooltip.timer = Some(t),
        Err(e) => log::warn!("tooltip timer: {}", e.error),
    }
}

/// The pointer is over `hit` on bar `bar`. A new target with a tip
/// replaces whatever is showing or waiting and starts the show delay; the
/// same target as before changes nothing, so the tip stays up while the
/// pointer wanders within one button.
pub fn hover(state: &mut State, bar: TreeId, hit: TaskbarHit) {
    rest_on(state, Target::Taskbar { bar, hit }, |s| {
        label_for(s, bar, hit).map(Tip::Label)
    });
}

pub fn hover_dialog(state: &mut State, d: usize, i: Option<usize>) {
    let Some(i) = i else {
        hide(state);
        return;
    };
    rest_on(state, Target::Dialog(d, i), |s| {
        s.dialogs
            .views
            .get(d)
            .and_then(|v| v.dialog.tip(i))
            .map(|(title, body)| Tip::Balloon { title, body })
    });
}

pub fn hover_desktop(state: &mut State, index: Option<usize>) {
    let Some(i) = index else {
        if state.tooltip.any(|t| matches!(t, Target::Desktop(_))) {
            hide(state);
        }
        return;
    };
    rest_on(state, Target::Desktop(i), |s| {
        let view = s.desktop.as_ref()?;
        let place = crate::desktop::shell_place(&view.entries.get(i)?.kind)?;
        Some(Tip::Icon(vec![place.tip()]))
    });
}

pub fn hover_caption(state: &mut State, window: WindowId, hit: FrameHit) {
    let FrameHit::Button(button) = hit else {
        if state.tooltip.any(|t| matches!(t, Target::Caption(..))) {
            hide(state);
        }
        return;
    };
    rest_on(state, Target::Caption(window, button), |s| {
        let win = s.win(window)?;
        let snap = (button == CaptionButton::Maximize && win.snappable())
            .then(|| snap_layouts_keys(&s.config.keybinds))
            .flatten();
        Some(Tip::Label(caption_tip(
            button,
            win.maximized,
            snap.as_deref(),
        )))
    });
}

/// The pointer is over a notification's icon button, `(id, button, label)`, or over none.
pub fn hover_notify(state: &mut State, icon: Option<(u32, usize, String)>) {
    let Some((id, button, label)) = icon else {
        if state.tooltip.any(|t| matches!(t, Target::Notify(..))) {
            hide(state);
        }
        return;
    };
    rest_on(state, Target::Notify(id, button), |_| {
        Some(Tip::Label(label))
    });
}

/// The `snap-layouts` bind's first chord, as the key sheet spells it.
fn snap_layouts_keys(binds: &[crate::config::Keybind]) -> Option<String> {
    (binds.iter())
        .find(|b| b.action == crate::config::Action::SnapLayouts)
        .map(|b| crate::keysheetview::chord_text(b.mods, b.keysym))
}

pub fn hover_menu(state: &mut State, depth: usize, row: Option<usize>) {
    let Some(row) = row else {
        if state
            .tooltip
            .any(|t| matches!(t, Target::Menu { depth: d, .. } if *d == depth))
        {
            hide(state);
        }
        return;
    };
    rest_on(state, Target::Menu { depth, row }, |s| {
        s.menus.get(depth)?.kind.tip(row).map(Tip::Info)
    });
}

pub fn hover_start_panel(state: &mut State, spot: Option<Spot>) {
    let Some(spot) = spot else {
        if state.tooltip.any(|t| matches!(t, Target::StartPanel(_))) {
            hide(state);
        }
        return;
    };
    rest_on(state, Target::StartPanel(spot), |s| {
        s.start_panel_tip(spot).map(Tip::Info)
    });
}

pub fn menus_closed_above(state: &mut State, depth: usize) {
    if state
        .tooltip
        .any(|t| matches!(t, Target::Menu { depth: d, .. } if *d > depth))
    {
        hide(state);
    }
}

pub fn menus_closed(state: &mut State) {
    if state.tooltip.any(|t| matches!(t, Target::Menu { .. })) {
        hide(state);
    }
}

pub fn output_gone(state: &mut State) {
    let bars: Vec<TreeId> = state.bars.iter().map(|b| b.tree).collect();
    if state
        .tooltip
        .any(|t| matches!(t, Target::Taskbar { bar, .. } if !bars.contains(bar)))
    {
        hide(state);
    }
}

fn rest_on(state: &mut State, target: Target, tip: impl FnOnce(&State) -> Option<Tip>) {
    if state.tooltip.any(|t| *t == target) {
        return;
    }
    hide(state);
    state.tooltip.resting = Some(target);
    if state.tooltip.holds_back(target) {
        return;
    }
    if let Some(tip) = tip(state) {
        state.tooltip.pending = Some((target, tip));
        arm(state, u64::from(SHOW_MS));
    }
    crate::netspeed::sync(state);
}

pub fn press(state: &mut State) {
    state.tooltip.note_press();
    take_down(state);
}

pub fn hide(state: &mut State) {
    state.tooltip.resting = None;
    take_down(state);
}

fn take_down(state: &mut State) {
    if state.tooltip.pending.is_none() && state.tooltip.shown.is_none() {
        return;
    }
    state.tooltip.pending = None;
    if let Some((_, tree)) = state.tooltip.shown.take() {
        state.destroy_redrawn(tree);
    }
    arm(state, 0);
    crate::netspeed::sync(state);
}

fn fired(state: &mut State) {
    if state.tooltip.shown.is_some() {
        take_down(state);
        return;
    }
    let Some((target, mut tip)) = state.tooltip.pending.take() else {
        return;
    };
    // The speed was first read when the pointer came to rest: read it again for a rate.
    if let Target::Taskbar { bar, hit } = target
        && speed_target(state, target)
    {
        crate::netspeed::sample(state);
        if let Some(label) = label_for(state, bar, hit) {
            tip = Tip::Label(label);
        }
    }
    if !state.menus.is_empty() && !matches!(target, Target::Menu { .. } | Target::StartPanel(_)) {
        crate::netspeed::sync(state);
        return;
    }
    let cursor = state.pointer_location();
    let output = match target {
        Target::Taskbar { bar, .. } => state.bar(bar).map(|b| b.output.clone()),
        Target::Dialog(d, _) => state.dialogs.views.get(d).map(|v| v.output.clone()),
        Target::Menu { depth, .. } => state.menus.get(depth).map(|m| m.output.clone()),
        Target::Desktop(_) => state.output_at(cursor.x, cursor.y),
        Target::StartPanel(_) => state.start_panel_output(),
        Target::Caption(..) | Target::Notify(..) => state.output_at(cursor.x, cursor.y),
    };
    let Some(output) = output else {
        crate::netspeed::sync(state);
        return;
    };
    state.tooltip.at = (cursor.x as i32, cursor.y as i32);
    show_tip(state, target, &tip, &output, state.tooltip.at);
    arm(state, HIDE_MS);
}

/// Whether `target`'s tip carries the network speed: the meter's, and the network icon's.
fn speed_target(state: &State, target: Target) -> bool {
    let Target::Taskbar { bar, hit } = target else {
        return false;
    };
    match hit {
        TaskbarHit::Meter => true,
        TaskbarHit::Tray(i) => {
            tray_item(state, bar, i).is_some_and(|it| it.key == crate::netspeed::NETWORK_ITEM)
        }
        _ => false,
    }
}

/// Whether a tip carrying the network speed is up or waiting to show.
pub fn shows_speed(state: &State) -> bool {
    state.tooltip.any(|&t| speed_target(state, t))
}

/// The speed changed: a tip showing it is drawn again in place, its hide timer left running.
pub fn speed_changed(state: &mut State) {
    let Some((target, tree)) = state.tooltip.shown else {
        return;
    };
    let Target::Taskbar { bar, hit } = target else {
        return;
    };
    if !speed_target(state, target) {
        return;
    }
    let (Some(label), Some(output)) = (
        label_for(state, bar, hit),
        state.bar(bar).map(|b| b.output.clone()),
    ) else {
        return;
    };
    state.destroy_redrawn(tree);
    show_tip(state, target, &Tip::Label(label), &output, state.tooltip.at);
}

fn tray_item(state: &State, bar: TreeId, i: usize) -> Option<&crate::tray::TrayItem> {
    let tray = state.tray.as_ref()?;
    match crate::tray::cell_at(state, bar, i) {
        crate::tray::Cell::Hidden(_) => {
            let revealed = state.bar(bar).map_or(0, |b| b.layout.tray_hidden);
            tray.item_at(revealed, 0, i)
        }
        crate::tray::Cell::Item(i) => tray.items.get(i),
        crate::tray::Cell::Builtin(_) => None,
    }
}

fn show_tip(state: &mut State, target: Target, tip: &Tip, output: &Output, cursor: (i32, i32)) {
    let scale = state.draw_scale_at(Some(output)).max(1);
    let (canvas, (x, y)) = drawn_tip(state, target, tip, output, cursor);
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.overlay);
    crate::glass::leave_out(scene, tree);
    crate::output::keep_on(scene, tree, output);
    let buffer = scene.buffer_create(tree);
    scene.set_input(buffer, Input::Passthrough);
    scene.set_position(tree, x, y);
    upload_scaled(scene, buffer, &canvas, scale);
    state.tooltip.shown = Some((target, tree));
    state.redraw_nodes(Vec::new(), &[tree.into()]);
}

/// `tip` drawn at `output`'s scale and where it goes, in layout px. It is sized as it draws
/// there: hinted text at a larger size can run wider than the scale times its width at scale 1.
fn drawn_tip(
    state: &State,
    target: Target,
    tip: &Tip,
    output: &Output,
    cursor: (i32, i32),
) -> (crate::raster::Canvas, (i32, i32)) {
    let scale = state.draw_scale_at(Some(output)).max(1);
    let base = &state.config.theme;
    let theme = base.scaled(scale);
    let text = state.text.for_output(scale);
    let layout = |(w, h): (i32, i32)| ((w + scale - 1) / scale, (h + scale - 1) / scale);
    let screen = state.layout_box(Some(output));
    // A taskbar tip opens on the screen side of the bar: above the
    // pointer over a bottom bar, under it below a top bar, beside a side
    // bar level with the pointer.
    let p = state.bar_placement(output);
    let below = p.edge == reprisekit::taskbar::Edge::Top;
    let beside = matches!(target, Target::Taskbar { .. }) && p.edge.is_side();
    let caption = match target {
        Target::Caption(w, b) => caption_button_rect(state, w, b),
        _ => None,
    };
    let room = shadow_room(base, screen);
    match tip {
        Tip::Label(label) => {
            let size = layout(tip_size(text, &theme, label));
            let at = if let Some(button) = caption {
                place_caption(cursor, size, room, button, state.snap_picker.is_shown())
            } else if beside {
                let right = p.edge == reprisekit::taskbar::Edge::Left;
                place_beside(cursor, p.screen_side(), right, size, room)
            } else {
                place(cursor, size, room, below)
            };
            (draw_tip(&theme, text, label), at)
        }
        Tip::Balloon { title, body } => (
            draw_balloon_tip(&theme, text, title, body),
            place_balloon(
                cursor,
                layout(balloon_size(text, &theme, title, body)),
                screen,
            ),
        ),
        Tip::Info(body) => {
            let row = match target {
                Target::Menu { depth, row } => menu_row_rect(state, depth, row),
                _ => None,
            };
            // A menu row's tip hangs under the row unless the pointer is
            // on it; every other infotip hangs under the pointer.
            let anchor = match row {
                Some(r) if !r.contains(cursor.0, cursor.1) => {
                    (r.x + INFO_BELOW_ROW.0, r.bottom() + INFO_BELOW_ROW.1)
                }
                _ => (cursor.0, cursor.1 + INFO_BELOW_POINTER),
            };
            (
                draw_info_tip(&theme, text, body),
                place_below(anchor, layout(info_size(text, &theme, body)), room),
            )
        }
        Tip::Icon(lines) => {
            use reprisekit::controls::infotip;
            (
                infotip::draw(&theme, text, lines, scale),
                infotip::place(cursor, layout(infotip::size(text, &theme, lines)), room),
            )
        }
    }
}

fn menu_row_rect(state: &State, depth: usize, row: usize) -> Option<Rect> {
    let m = state.menus.get(depth)?;
    let r = *m.layout.rows.get(row)?;
    Some(Rect::new(m.x + r.x, m.y + r.y, r.width, r.height))
}

/// Caption button `b` of window `w` in layout coordinates.
fn caption_button_rect(state: &State, w: WindowId, b: CaptionButton) -> Option<Rect> {
    let win = state.win(w)?;
    let frame = win.frame_rect();
    let r = win.geom.button_rect(b);
    Some(Rect::new(frame.x + r.x, frame.y + r.y, r.width, r.height))
}

/// Where caption `button`'s tip goes: under the pointer, or above it while the snap layout
/// `picker` hangs under the button; with no room above (a maximized window), left of the button.
fn place_caption(
    cursor: (i32, i32),
    size: (i32, i32),
    screen: Rect,
    button: Rect,
    picker: bool,
) -> (i32, i32) {
    if !picker {
        return place(cursor, size, screen, true);
    }
    let above = place(cursor, size, screen, false);
    if above.1 + size.1 < cursor.1 {
        return above;
    }
    place_beside(cursor, button.x, false, size, screen)
}

/// A caption button's tip; the maximize button's names the `snap-layouts`
/// keys on a second line when they are bound.
fn caption_tip(button: CaptionButton, maximized: bool, snap_keys: Option<&str>) -> String {
    let name = button.name(maximized);
    match snap_keys {
        Some(keys) if button == CaptionButton::Maximize => {
            format!("{name}\n{}", tr!("bar-tip-snap-layouts", keys = keys))
        }
        _ => name.to_string(),
    }
}

fn chevron_tip(arrow: bool, open: bool) -> String {
    if arrow && open {
        tr!("bar-tip-hide")
    } else {
        tr!("bar-tip-show-hidden-icons")
    }
}

pub(crate) fn label_for(state: &State, bar: TreeId, hit: TaskbarHit) -> Option<String> {
    match hit {
        TaskbarHit::Start => Some(tr!("bar-tip-start")),
        TaskbarHit::Quick(i) => state.quicklaunch.get(i).map(|q| q.entry.label.clone()),
        TaskbarHit::Clock => Some(crate::taskbar::date_text()),
        TaskbarHit::Pager(i) => Some(state.desktop_name(i)),
        TaskbarHit::ShowDesktop => Some(tr!("bar-tip-show-desktop")),
        TaskbarHit::Chevron => {
            let open = state.bar(bar).is_some_and(|b| b.tray_open);
            Some(chevron_tip(crate::trayreveal::arrow_look(state), open))
        }
        TaskbarHit::More => Some(tr!("bar-tip-more-windows")),
        TaskbarHit::QuickMore => Some(reprisekit::taskbar::quick_more_name()),
        TaskbarHit::Meter => Some(match crate::netspeed::line(state) {
            Some(line) => format!("{}\n{line}", tr!("bar-tip-network-speed")),
            None => tr!("bar-tip-network-speed"),
        }),
        TaskbarHit::Tray(i) => match crate::tray::cell_at(state, bar, i) {
            crate::tray::Cell::Builtin(b) => crate::tray::builtin::shown(state)
                .get(b)
                .and_then(|&b| crate::tray::builtin::label(state, b)),
            _ => {
                let item = tray_item(state, bar, i).filter(|it| !it.tooltip.is_empty())?;
                let speed = (item.key == crate::netspeed::NETWORK_ITEM)
                    .then(|| crate::netspeed::line(state))
                    .flatten();
                Some(match speed {
                    Some(line) => format!("{}\n{line}", item.tooltip),
                    None => item.tooltip.clone(),
                })
            }
        },
        TaskbarHit::Task(i) => {
            let b = state.bar(bar)?;
            // The preview panel (`taskpreview`) names the windows instead;
            // a pinned app with none has no panel, so it keeps its tip.
            if state.config.previews.taskbar
                && b.slots.get(i).is_some_and(|s| !s.windows.is_empty())
            {
                return None;
            }
            let rect = *b.layout.tasks.get(i)?;
            let (title, bold) = state.task_button_name(b, i)?;
            let size = state.config.theme.fonts.size_px;
            task_tip(&state.text, size, &title, bold, rect.width)
        }
        TaskbarHit::Grip(_) | TaskbarHit::Resize | TaskbarHit::Nothing => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arrows_tip_follows_its_run() {
        assert_eq!(chevron_tip(true, false), "Show hidden icons");
        assert_eq!(chevron_tip(true, true), "Hide");
    }

    #[test]
    fn a_pressed_target_gets_no_tip_until_the_pointer_moves_on() {
        let (arrow, icon) = (Target::Desktop(0), Target::Desktop(1));
        let mut v = TooltipView {
            resting: Some(arrow),
            ..TooltipView::default()
        };
        v.note_press();
        assert!(v.holds_back(arrow), "the pressed target stays quiet");
        assert!(
            v.holds_back(arrow),
            "for as long as the pointer stays on it"
        );
        assert!(!v.holds_back(icon), "another target gets its tip");
        assert!(!v.holds_back(arrow), "and the pressed one again after that");
    }

    #[test]
    fn an_open_menu_keeps_the_desktop_icons_tip_down() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.pointer_to(40.0, 40.0);
        h.state.open_desktop_menu((60, 40));
        assert!(!h.state.menus.is_empty());
        rest_on(&mut h.state, Target::Desktop(0), |_| {
            Some(Tip::Icon(vec!["Recycle Bin".into()]))
        });
        h.run_for(u64::from(SHOW_MS) + 200);
        assert!(
            h.state.tooltip.shown.is_none(),
            "only the open menu's rows tip"
        );
    }

    #[test]
    fn keys_through_the_start_menu_raise_no_infotip_and_the_pointer_does() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.toggle_start_from_keys();
        h.dispatch();
        h.state.menu_key(crate::keys::KEY_s);
        let depth = h.state.menus.len() - 1;
        assert_eq!(depth, 1, "S opens Settings");
        let row = h.state.menus[depth].hovered.expect("its first row is lit");
        assert!(h.state.menus[depth].kind.tip(row).is_some());
        let up = |h: &crate::harness::Harness| {
            h.state.tooltip.shown.is_some() || h.state.tooltip.pending.is_some()
        };
        h.run_for(u64::from(SHOW_MS) + 200);
        assert!(!up(&h), "a row the keys light gets no tip");
        h.state.menu_hover(depth, Some(row));
        h.run_for(u64::from(SHOW_MS) + 200);
        assert!(
            h.state.tooltip.shown.is_some(),
            "the pointer resting on it does"
        );
        assert_eq!(h.state.menus.len(), 3, "Control Panel's own list opened");
        h.state.menu_key(crate::keys::KEY_Left);
        assert!(
            h.state.tooltip.shown.is_some(),
            "closing the list leaves it"
        );
        h.state.menu_key(crate::keys::KEY_Down);
        assert!(!up(&h), "a key moving on takes it down");
    }

    #[test]
    fn the_maximize_tip_names_the_snap_layouts_keys_when_bound() {
        use CaptionButton::{Close, Maximize, Minimize};
        let keys = Some("Super+Z");
        assert_eq!(
            caption_tip(Maximize, false, keys),
            "Maximize\nSnap layouts: Super+Z"
        );
        assert_eq!(
            caption_tip(Maximize, true, Some("Ctrl+Alt+L")),
            "Restore Down\nSnap layouts: Ctrl+Alt+L",
            "a rebound key shows"
        );
        assert_eq!(caption_tip(Maximize, false, None), "Maximize", "unbound");
        assert_eq!(caption_tip(Minimize, false, keys), "Minimize");
        assert_eq!(caption_tip(Close, true, keys), "Close");
    }

    #[test]
    fn a_caption_tip_clears_the_button_under_the_pointer() {
        let screen = Rect::new(0, 0, 1920, 1080);
        let size = (120, 30);
        let max = Rect::new(1860, 4, 21, 21);
        let on = (1870, 14);
        let at = place_caption(on, size, screen, max, true);
        let tip = Rect::new(at.0, at.1, size.0, size.1);
        assert_eq!(
            at,
            (1738, 0),
            "left of the maximize button of a maximized window"
        );
        assert!(!tip.intersects(max), "the button stays clear");
        let low = Rect::new(900, 300, 21, 21);
        assert_eq!(
            place_caption((910, 310), size, screen, low, true),
            (909, 272),
            "above the pointer with room there"
        );
        assert_eq!(
            place_caption(on, size, screen, max, false),
            (1799, 35),
            "under the pointer with no picker"
        );
    }

    #[test]
    fn the_snap_layouts_keys_read_from_the_bind() {
        use crate::config::Config;
        let keys = |toml: &str| snap_layouts_keys(&Config::parse(toml).unwrap().keybinds);
        assert_eq!(keys("").as_deref(), Some("Super+Z"));
        let unbound = "[[keybinds]]\nkeys = \"Super+z\"\naction = \"none\"\n";
        assert_eq!(keys(unbound), None);
        let rebound =
            format!("{unbound}[[keybinds]]\nkeys = \"Ctrl+Alt+l\"\naction = \"snap-layouts\"\n");
        assert_eq!(keys(&rebound).as_deref(), Some("Ctrl+Alt+L"));
    }

    #[test]
    fn a_dialog_opened_under_a_resting_pointer_takes_the_desktop_tip_down() {
        let mut h = crate::harness::Harness::new();
        let out = h.output("TEST-1", 1024, 768);
        let area = h.state.usable_area(Some(&out));
        let (cx, cy) = (area.x + area.width / 2, area.y + area.height / 2);
        h.pointer_to(f64::from(cx), f64::from(cy));
        rest_on(&mut h.state, Target::Desktop(0), |_| {
            Some(Tip::Icon(vec!["Recycle Bin".into()]))
        });
        h.state.open_dialog(crate::dialog::Dialog::error(
            "Error",
            "Under the pointer.".into(),
        ));
        h.run_for(u64::from(SHOW_MS) + 200);
        assert!(
            h.state.tooltip.shown.is_none() && h.state.tooltip.pending.is_none(),
            "the pointer rests on the dialog, so the icon's tip stays down"
        );
    }

    #[test]
    fn a_speed_tip_reads_the_counters_only_while_it_is_pending_or_up() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let bar = h.state.bars[0].tree;
        let meter = Target::Taskbar {
            bar,
            hit: TaskbarHit::Meter,
        };
        rest_on(&mut h.state, meter, |s| {
            label_for(s, bar, TaskbarHit::Meter).map(Tip::Label)
        });
        assert!(crate::netspeed::sampling(&h.state), "pending");
        h.run_for(u64::from(SHOW_MS) + 200);
        assert!(h.state.tooltip.shown.is_some());
        assert!(crate::netspeed::sampling(&h.state), "shown");
        hide(&mut h.state);
        assert!(!crate::netspeed::sampling(&h.state), "hidden");
    }

    #[test]
    fn a_tip_at_the_right_edge_at_scale_2_keeps_its_border_on_the_screen() {
        use smithay::output::Scale;
        for text_size in [100, 200] {
            let toml = format!("[accessibility]\ntext_size = {text_size}\n");
            let config = crate::config::Config::parse(&toml).unwrap();
            let mut h = crate::harness::Harness::with_config(config);
            let out = h.output("TEST-1", 1024, 768);
            out.change_current_state(None, None, Some(Scale::Integer(2)), None);
            h.state.update_layout();
            let screen = h.state.layout_box(Some(&out));
            let tip = Tip::Label("Wednesday, September 30, 2026".into());
            let cursor = (screen.right() - 2, screen.y + screen.height / 2);
            let (canvas, (x, _)) = drawn_tip(&h.state, Target::Desktop(0), &tip, &out, cursor);
            let end = 2 * x + canvas.width as i32;
            assert!(
                end <= 2 * (screen.right() - 1),
                "at {text_size}% the tip ends at device x {end}, at most {}",
                2 * (screen.right() - 1)
            );
        }
    }

    #[test]
    fn the_flyouts_tip_stays_put() {
        assert_eq!(chevron_tip(false, false), "Show hidden icons");
        assert_eq!(chevron_tip(false, true), "Show hidden icons");
    }
}
