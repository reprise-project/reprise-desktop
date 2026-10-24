//! Tooltip glue: the show delay, hide timer and scene node for taskbar, tray, Welcome, Start menu
//! and desktop tips. A tip sits in `layers.overlay` with no owner tag, so the pointer passes through.

use std::time::Duration;

use reprisekit::classic::frame::{CaptionButton, FrameHit};
use reprisekit::controls::tooltip::{
    INFO_BELOW_POINTER, INFO_BELOW_ROW, SHOW_MS, balloon_size, draw_balloon_tip, draw_info_tip,
    draw_tip, info_size, place, place_balloon, place_below, place_beside, task_tip, tip_size,
};
use reprisekit::startpanel::Spot;
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
    Taskbar { bar: TreeId, hit: TaskbarHit },
    Dialog(usize, usize),
    Menu { depth: usize, row: usize },
    Desktop(usize),
    StartPanel(Spot),
    Caption(WindowId, CaptionButton),
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
        Some(Tip::Icon(vec![place.tip().to_string()]))
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

/// The `snap-layouts` bind's first chord, as an accelerator reads.
fn snap_layouts_keys(binds: &[crate::config::Keybind]) -> Option<String> {
    (binds.iter())
        .find(|b| b.action == crate::config::Action::SnapLayouts)
        .map(|b| crate::winmenu::keybind_label(b.mods, b.keysym))
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
        state.scene.destroy(tree);
        state.redraw_all();
    }
    arm(state, 0);
}

fn fired(state: &mut State) {
    if state.tooltip.shown.is_some() {
        take_down(state);
        return;
    }
    let Some((target, tip)) = state.tooltip.pending.take() else {
        return;
    };
    if !state.menus.is_empty() && !matches!(target, Target::Menu { .. } | Target::StartPanel(_)) {
        return;
    }
    let cursor = state.pointer_location();
    let output = match target {
        Target::Taskbar { bar, .. } => state.bar(bar).map(|b| b.output.clone()),
        Target::Dialog(d, _) => state.dialogs.views.get(d).map(|v| v.output.clone()),
        Target::Menu { depth, .. } => state.menus.get(depth).map(|m| m.output.clone()),
        Target::Desktop(_) => state.output_at(cursor.x, cursor.y),
        Target::StartPanel(_) => state.start_panel_output(),
        Target::Caption(..) => state.output_at(cursor.x, cursor.y),
    };
    let Some(output) = output else {
        return;
    };
    show_tip(
        state,
        target,
        &tip,
        &output,
        (cursor.x as i32, cursor.y as i32),
    );
    arm(state, HIDE_MS);
}

fn show_tip(state: &mut State, target: Target, tip: &Tip, output: &Output, cursor: (i32, i32)) {
    let scale = state.draw_scale_at(Some(output)).max(1);
    let base = &state.config.theme;
    let theme = base.scaled(scale);
    let text = &state.text;
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
    let (canvas, (x, y)) = match tip {
        Tip::Label(label) => {
            let size = tip_size(text, base, label);
            let at = if let Some(button) = caption {
                place_caption(cursor, size, screen, button, state.snap_picker.is_shown())
            } else if beside {
                let right = p.edge == reprisekit::taskbar::Edge::Left;
                place_beside(cursor, p.screen_side(), right, size, screen)
            } else {
                place(cursor, size, screen, below)
            };
            (draw_tip(&theme, text.for_output(theme.scale), label), at)
        }
        Tip::Balloon { title, body } => (
            draw_balloon_tip(&theme, text.for_output(theme.scale), title, body),
            place_balloon(cursor, balloon_size(text, base, title, body), screen),
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
                draw_info_tip(&theme, text.for_output(theme.scale), body),
                place_below(anchor, info_size(text, base, body), screen),
            )
        }
        Tip::Icon(lines) => {
            use reprisekit::controls::infotip;
            (
                infotip::draw(&theme, text.for_output(theme.scale), lines, scale),
                infotip::place(cursor, infotip::size(text, base, lines), screen),
            )
        }
    };
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.overlay);
    crate::glass::leave_out(scene, tree);
    let buffer = scene.buffer_create(tree);
    scene.set_input(buffer, Input::Passthrough);
    scene.set_position(tree, x, y);
    upload_scaled(scene, buffer, &canvas, scale);
    state.tooltip.shown = Some((target, tree));
    state.redraw_all();
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
        Some(keys) if button == CaptionButton::Maximize => format!("{name}\nSnap layouts: {keys}"),
        _ => name.to_string(),
    }
}

fn chevron_tip(arrow: bool, open: bool) -> &'static str {
    if arrow && open {
        "Hide"
    } else {
        "Show hidden icons"
    }
}

pub(crate) fn label_for(state: &State, bar: TreeId, hit: TaskbarHit) -> Option<String> {
    match hit {
        TaskbarHit::Start => Some("Click here to begin".to_string()),
        TaskbarHit::Quick(i) => state.quicklaunch.get(i).map(|q| q.entry.label.clone()),
        TaskbarHit::Clock => Some(crate::taskbar::date_text()),
        TaskbarHit::Pager(i) => Some(format!("Desktop {}", i + 1)),
        TaskbarHit::ShowDesktop => Some("Show desktop".to_string()),
        TaskbarHit::Chevron => {
            let open = state.bar(bar).is_some_and(|b| b.tray_open);
            Some(chevron_tip(crate::trayreveal::arrow_look(state), open).to_string())
        }
        TaskbarHit::More => Some("Show more windows".to_string()),
        TaskbarHit::QuickMore => Some(reprisekit::taskbar::QUICK_MORE_NAME.to_string()),
        TaskbarHit::Tray(i) => match crate::tray::cell_at(state, bar, i) {
            crate::tray::Cell::Hidden(_) => {
                let revealed = state.bar(bar).map_or(0, |b| b.layout.tray_hidden);
                (state.tray.as_ref())
                    .and_then(|t| t.item_at(revealed, 0, i))
                    .map(|it| it.tooltip.clone())
                    .filter(|s| !s.is_empty())
            }
            crate::tray::Cell::Item(i) => state
                .tray
                .as_ref()
                .and_then(|t| t.items.get(i))
                .map(|it| it.tooltip.clone())
                .filter(|s| !s.is_empty()),
            crate::tray::Cell::Builtin(b) => crate::tray::builtin::shown(state)
                .get(b)
                .and_then(|&b| crate::tray::builtin::label(state, b)),
        },
        TaskbarHit::Task(i) => {
            let b = state.bar(bar)?;
            // The preview panel (`taskpreview`) names the windows instead;
            // a pinned app with none has no panel, so it keeps its tip.
            if state.config.previews.taskbar && b.buttons.get(i).is_some_and(|w| !w.is_empty()) {
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
    fn the_flyouts_tip_stays_put() {
        assert_eq!(chevron_tip(false, false), "Show hidden icons");
        assert_eq!(chevron_tip(false, true), "Show hidden icons");
    }
}
