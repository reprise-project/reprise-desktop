//! The dialogs on the scene (`State::dialogs`, a stack; the top one has the keyboard), placed on the
//! output under the pointer, with key and pointer routing. Model, layout and drawing live in `dialog.rs`.
//!
//! Each dialog is one tree in `layers.menu` (`layers.windows` for a windowed one): `content` (the
//! client area, the only part the glass backdrop sees), the frame's `Backdrop`, then the buffer. A
//! Welcome panel stands over a still of its output (`output_still`) that fades to grey.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use reprisekit::controls::filedlg;
use reprisekit::controls::form::Key;
use reprisekit::controls::grouplist;
use reprisekit::controls::openwith;
use reprisekit::controls::shellview;
use reprisekit::cues::Cues;
use reprisekit::pointer::DoubleClick;
use reprisescene::{BufferId, TreeId};
use smithay::input::keyboard::{KeysymHandle, ModifiersState};
use smithay::output::Output;
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::{SceneOwner, State};
use crate::dialog::{self, Dialog, DialogHit, DialogLayout, Outcome, Placement, Style};
use crate::fsjobs::Action;
use crate::glass::{Backdrop, frame_pane};
use crate::input::Interaction;
use crate::output::upload_scaled;
use crate::raster::{Canvas, Rect};
use crate::textedit::EditKey;
use crate::window::keep_inside;
use reprisekit::frame::{CaptionButton, Edges};

#[derive(Default)]
pub struct Dialogs {
    pub views: Vec<DialogView>,
    /// The top dialog has the keyboard until a window takes the focus.
    pub active: bool,
    pub run_history: Vec<String>,
}

pub struct DialogView {
    pub dialog: Dialog,
    /// Unique for the compositor's run, so deferred work finds its dialog or learns it is gone.
    pub id: u64,
    /// Creation order among task buttons with the same anchor (`taskbar::next_seq`).
    pub seq: u64,
    /// Pinned slots before its task button (`taskbar::Place`); `usize::MAX` until a drag moves it.
    pub anchor: usize,
    /// The dialog this one was opened over, which takes no input meanwhile.
    pub parent: Option<u64>,
    pub layout: DialogLayout,
    tree: TreeId,
    /// The client area alone: what the glass backdrop sees. Empty when no glass is translucent.
    content: BufferId,
    /// What `content` shows, so an unchanged redraw uploads nothing.
    content_shown: Option<(Canvas, Rect, i32)>,
    glass_tree: TreeId,
    backdrop: Backdrop,
    buffer: BufferId,
    pub x: i32,
    pub y: i32,
    pub output: Output,
    /// Last press in the field or a form's edit field, for double-click word select.
    field_press: DoubleClick<usize>,
    /// A list row's last press, for double-click as OK.
    row_press: DoubleClick<usize>,
    /// The wheel's sub-notch remainder, in 120ths of a notch.
    wheel: i32,
    /// Icons the last draw used, so a caret blink skips the lookups.
    icons: Option<(IconKey, DialogIcons)>,
    /// Jobs decoding icon files the icons wait for; they are looked up again when one lands.
    decodes: crate::fsjobs::Reads<IconFiles>,
    /// Last full raster without its caret, and its scale: a blink repaints the field over a clone.
    still_canvas: Option<(Canvas, i32)>,
    /// Keyboard cues: a pointer-opened dialog starts without them ([`Cues::key`]), a key-opened one has them.
    pub cues: Cues,
    /// A Welcome panel's fade-in timer, while it fades.
    fade: Option<RegistrationToken>,
    /// A windowed dialog minimised from its task button: its tree is off and it sits at the
    /// bottom of the stack.
    pub minimized: bool,
}

impl DialogView {
    pub fn tree(&self) -> TreeId {
        self.tree
    }

    pub fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.layout.size.0, self.layout.size.1)
    }
}

const FADE_STEPS: i32 = 25;
const FADE_TICK: std::time::Duration = std::time::Duration::from_millis(40);

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// A key press the top dialog takes: keysyms, modifier mask (`keys::mask`), and its editing key if any.
#[derive(Clone, Debug, PartialEq)]
pub struct DialogKey {
    pub syms: Vec<u32>,
    pub mods: u32,
    pub edit: Option<EditKey>,
}

impl DialogKey {
    /// The press `handle` stands for under `mods`; the key route sets the editing key from xkb state.
    pub fn of(mods: &ModifiersState, handle: &KeysymHandle<'_>) -> DialogKey {
        DialogKey {
            syms: handle.modified_syms().iter().map(|s| s.raw()).collect(),
            mods: crate::keys::mask(mods),
            edit: None,
        }
    }
}

/// Load what the dialogs keep from the state file; runs once after `State::new`. Recycle Bin settings
/// an older state file held move into `config.toml`'s `[recycle_bin]` unless already set there.
pub fn setup(state: &mut State) {
    let persisted = crate::state::load();
    state.dialogs.run_history = persisted.run_history;
    if let Some(old) = persisted.bin {
        crate::configwrite::save_recycle_bin(&state.config_path, old.policy(), false);
        crate::state::drop_key("bin");
    }
    crate::fsjobs::setup(state);
}

/// The stack order after raising dialog `index`: its family (parent chain up to a root dialog, and
/// everything opened over any of them) moves to the top in current order. `links` is `(id, parent)`, bottom first.
/// Hide the underlines of every dialog but the one at stack index `keyboard`, which has the keyboard.
/// Whether any were out.
fn blur_cues_without_keyboard<'a>(
    cues: impl Iterator<Item = &'a mut Cues>,
    keyboard: Option<usize>,
) -> bool {
    let mut blurred = false;
    for (i, c) in cues.enumerate() {
        if Some(i) != keyboard {
            blurred |= c.blur();
        }
    }
    blurred
}

fn raise_order(links: &[(u64, Option<u64>)], index: usize) -> Vec<usize> {
    let parent = |i: usize| {
        links[i]
            .1
            .and_then(|p| links.iter().position(|&(id, _)| id == p))
    };
    let root = |mut i: usize| {
        while let Some(p) = parent(i) {
            i = p;
        }
        i
    };
    let top = root(index);
    let (family, rest): (Vec<usize>, Vec<usize>) = (0..links.len()).partition(|&i| root(i) == top);
    rest.into_iter().chain(family).collect()
}

/// The layer a dialog's tree stands in: among the windows when [`Dialog::windowed`], else over them.
fn dialog_layer(d: &Dialog, layers: &crate::compositor::Layers) -> TreeId {
    if d.windowed() {
        layers.windows
    } else {
        layers.menu
    }
}

/// Where a dialog of `size` at `origin` goes when its output's usable area becomes `area`: moved as
/// little as keeps it inside; `None` for no area or a Welcome panel.
fn refit_origin(
    d: &Dialog,
    origin: (i32, i32),
    size: (i32, i32),
    area: Rect,
) -> Option<(i32, i32)> {
    if area.width <= 0 || matches!(d.style, Style::Welcome) {
        return None;
    }
    Some(keep_inside(origin, size, area))
}

/// Layout point in dialog pixels. Flooring keeps the pixel under the pointer left of or above the layout origin.
fn to_local(lx: f64, ly: f64, origin: (i32, i32)) -> (i32, i32) {
    (lx.floor() as i32 - origin.0, ly.floor() as i32 - origin.1)
}

/// What the dialog has at a layout point: a sizable frame's border under `theme`'s metrics, else
/// `DialogLayout::hit`. The renderer is needed to measure text for a byte index in an edit field.
pub fn hit(
    v: &DialogView,
    lx: f64,
    ly: f64,
    text: &reprisekit::text::TextRenderer,
    theme: &crate::theme::Theme,
) -> Option<DialogHit> {
    let (x, y) = to_local(lx, ly, (v.x, v.y));
    if let Some(e) = v.layout.sizing_edges(x, y, &theme.metrics) {
        return Some(DialogHit::Resize(e));
    }
    v.layout.hit(&v.dialog, x, y, text, theme.fonts.size_px)
}

/// The cursor over `hit`: size arrows on a sizable frame's border and Browse's grip.
pub fn cursor_over(hit: Option<DialogHit>) -> &'static str {
    match hit {
        Some(DialogHit::Resize(e)) => e.cursor_name(),
        Some(DialogHit::Browse(filedlg::Hit::Grip)) => (Edges::BOTTOM | Edges::RIGHT).cursor_name(),
        _ => "default",
    }
}

/// The frame a sizable dialog takes after the pointer moved (`dx`, `dy`) from a press sizing `edges` of
/// `start`: client no smaller than `min_client`, undragged edges fixed. `pad` is the frame around the client.
fn sized_frame(
    start: Rect,
    edges: Edges,
    (dx, dy): (i32, i32),
    min_client: (i32, i32),
    pad: (i32, i32),
) -> Rect {
    let limits = crate::window::SizeLimits {
        min: min_client,
        max: (None, None),
    };
    crate::input::resize_rect(start, edges, dx, dy, limits, pad)
}

/// The glass a dialog's backdrop covers, local to the dialog: the whole frame with look-rounded outer
/// corners, client clear; nothing without glass or `wanted` (`glass::wanted`), or for a Welcome panel.
fn glass_pane(
    theme: &crate::theme::Theme,
    wanted: bool,
    layout: &DialogLayout,
) -> Option<crate::glass::Pane> {
    match (theme.glass(), &layout.frame) {
        (Some(g), Some(f)) if wanted => Some(frame_pane(f, g.corner, false)),
        _ => None,
    }
}

fn pressed_button(hit: Option<DialogHit>) -> Option<usize> {
    match hit {
        Some(DialogHit::Button(i)) => Some(i),
        _ => None,
    }
}

#[derive(Clone, PartialEq, Eq)]
struct IconKey {
    scale: i32,
    icon: Option<String>,
    buttons: Vec<Option<String>>,
    logo: bool,
}

struct DialogIcons {
    icon: Option<Canvas>,
    caption: Option<Canvas>,
    buttons: Vec<Option<Canvas>>,
    logo: Option<Canvas>,
    /// Row icons by name and logical size at the key's scale; `None` for a name the theme lacks.
    rows: HashMap<(String, i32), Option<Canvas>>,
}

/// Icon `name` at `px` from `cache`. An icon file is never read here: one not decoded at `px` yet
/// goes on `undecoded` for a job and shows nothing meanwhile.
fn cached(
    cache: &mut reprisekit::icontheme::IconCache,
    name: &str,
    px: i32,
    undecoded: &mut Vec<(String, i32)>,
) -> Option<Canvas> {
    if !name.starts_with('/') {
        return cache.get(name, px);
    }
    let put = cache.file_put(name, px);
    if put.is_none() {
        undecoded.push((name.to_string(), px));
    }
    put.flatten().map(std::sync::Arc::unwrap_or_clone)
}

type IconFiles = Vec<(String, i32)>;

#[derive(Debug, PartialEq)]
enum Decode {
    /// Decode job `read` for these files.
    Start(u64, IconFiles),
    /// Look the icons up again then: the running job will have turned stuck.
    Retry(std::time::Duration),
    /// Nothing to start; a job landing looks the icons up again.
    Wait,
}

/// What a draw does with the icon files it found `undecoded`. A job hands each file to the loop
/// as it decodes it: a file a job has yet to reach is left to it, and of a stuck job only the
/// file it is blocked on, so a file on a dead mount holds one job, not one per draw.
fn next_decode(
    decodes: &mut crate::fsjobs::Reads<IconFiles>,
    mut undecoded: IconFiles,
    now: std::time::Instant,
) -> Decode {
    undecoded.sort_unstable();
    undecoded.dedup();
    let busy: Vec<&(String, i32)> = (decodes.in_flight(now))
        .flat_map(|(stuck, rest)| &rest[..rest.len().min(if stuck { 1 } else { usize::MAX })])
        .collect();
    undecoded.retain(|f| !busy.contains(&f));
    if undecoded.is_empty() {
        return Decode::Wait;
    }
    if let Some(read) = decodes.start(now, undecoded.clone()) {
        return Decode::Start(read, undecoded);
    }
    decodes.retry_in(now).map_or(Decode::Wait, Decode::Retry)
}

/// The names `redraw_dialog` resolves for `d`.
fn icon_key(d: &Dialog, scale: i32) -> IconKey {
    IconKey {
        scale,
        icon: d.icon.clone(),
        buttons: d.buttons.iter().map(|b| b.icon.clone()).collect(),
        logo: matches!(d.style, Style::Welcome),
    }
}

/// The rows of `d` that draw an icon, ascending, with name and logical size: chooser rows at 16 px, or a
/// Properties sheet's Opens with row 0 at 16 px and its page pictures (`dialog::page_pictures`) at 32 px.
fn row_icon_names(d: &Dialog) -> Vec<(usize, String, i32)> {
    match d.chooser() {
        Some(c) => {
            let rows = &c.ow.list.rows;
            let top = c.ow.list.top;
            (top..rows.len().min(top + openwith::ROWS))
                .filter_map(|i| openwith::icon_name(&rows[i]).map(|n| (i, n.to_string(), 16)))
                .collect()
        }
        None => {
            let opens = match d.form().map(|f| &f.kind) {
                Some(crate::dialog::Kind::Properties(b)) => b.opens_icon.as_deref(),
                _ => None,
            };
            let pics = dialog::page_pictures(d)
                .into_iter()
                .enumerate()
                .map(|(k, (_, n, px))| (k + 1, n, px));
            opens
                .map(|n| (0, n.to_string(), 16))
                .into_iter()
                .chain(pics)
                .collect()
        }
    }
}

impl State {
    /// Show `d`, placed per `d.placement` on the output under the pointer. A dialog with the same
    /// `identity` is raised instead of reopened, its cues out when a key asked ([`Cues::reopen`]).
    /// A Welcome panel takes the screen alone and closes the switchers; any other dialog opened
    /// under the 3D switcher joins its stack (`flip_join`) and stays hidden until its cards land
    /// (`flip_hide_dialog`).
    pub fn open_dialog(&mut self, d: Dialog) {
        if matches!(d.style, Style::Welcome) {
            self.switcher_cancel();
            self.close_all_dialogs();
            self.open_view(d, None);
            return;
        }
        let same = self
            .dialogs
            .views
            .iter()
            .position(|v| v.dialog.identity() == d.identity());
        match same {
            Some(i) => {
                let v = &mut self.dialogs.views[i];
                let id = v.id;
                let shown = v.cues.reopen(self.key_input.last_key_input);
                self.raise_dialog(i);
                if shown && let Some(j) = self.dialogs.views.iter().position(|v| v.id == id) {
                    self.redraw_dialog(j);
                }
            }
            None => self.open_view(d, None),
        }
    }

    /// Show `d` over the top dialog as its child, which takes the keyboard and the parent's pointer input.
    fn open_dialog_over(&mut self, d: Dialog) {
        let parent = self.dialogs.views.last().map(|v| v.id);
        self.open_view(d, parent);
    }

    /// Show `d` as the child of `parent`, raised first; on its own when `parent` is closed or None.
    fn open_dialog_over_id(&mut self, d: Dialog, parent: Option<u64>) {
        match self.dialogs.views.iter().position(|v| Some(v.id) == parent) {
            Some(i) => {
                self.raise_dialog(i);
                self.open_view(d, parent);
            }
            None => self.open_dialog(d),
        }
    }

    fn open_view(&mut self, mut d: Dialog, parent: Option<u64>) {
        let pos = self.pointer_location();
        let Some(out) = self
            .output_at(pos.x, pos.y)
            .or_else(|| self.scene.outputs().first().cloned())
        else {
            return;
        };
        let area = self.usable_area(Some(&out));
        if area.width <= 0 {
            return;
        }
        let theme = &self.config.theme;
        d.fit_in(theme, &self.text, (area.width, area.height));
        let layout = DialogLayout::compute(theme, &self.text, &d);
        let (x, y) = match d.placement {
            Placement::ByStart => {
                dialog::by_start(area, self.bar_placement(&out).edge, layout.size)
            }
            Placement::Center => (
                area.x + (area.width - layout.size.0) / 2,
                area.y + (area.height - layout.size.1) / 2,
            ),
            Placement::CenterAs(h) => (
                area.x + (area.width - layout.size.0) / 2,
                area.y + (area.height - h) / 2,
            ),
            Placement::UpperThird => {
                let b = self.layout_box(Some(&out));
                (
                    b.x + (b.width - layout.size.0) / 2,
                    b.y + (b.height - layout.size.1) / 3,
                )
            }
            Placement::At(x, y) => keep_inside((x, y), layout.size, area),
        };
        let welcome = matches!(d.style, Style::Welcome);
        let tree = self.scene.tree_create(dialog_layer(&d, &self.layers));
        self.scene
            .set_data(tree, Some(SceneOwner::Dialog(self.dialogs.views.len())));
        // Only `content` shows in the backdrop.
        let content = self.scene.buffer_create(tree);
        let glass_tree = self.scene.tree_create(tree);
        let backdrop = Backdrop::new(&self.glass);
        let buffer = self.scene.buffer_create(tree);
        crate::glass::leave_out(&mut self.scene, buffer);
        self.scene.set_position(tree, x, y);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let view = DialogView {
            dialog: d,
            id,
            seq: crate::taskbar::next_seq(),
            anchor: usize::MAX,
            parent,
            layout,
            tree,
            content,
            content_shown: None,
            glass_tree,
            backdrop,
            buffer,
            x,
            y,
            output: out,
            field_press: DoubleClick::default(),
            row_press: DoubleClick::default(),
            wheel: 0,
            icons: None,
            decodes: Default::default(),
            still_canvas: None,
            cues: Cues::opened(self.key_input.last_key_input),
            fade: None,
            minimized: false,
        };
        self.dialogs.views.push(view);
        self.activate_dialogs();
        self.blur_dialogs_without_keyboard();
        self.restart_caret();
        self.redraw_dialogs();
        self.sync_dialog_tasks(false);
        if welcome {
            self.grey_the_screen_under(id);
        }
        self.flip_hide_dialog(tree);
        self.flip_join();
        self.run_picker_requests();
    }

    /// Freeze the screen behind Welcome panel `id` and fade it to grey. Taken once the panel has the
    /// keyboard, so windows and task buttons show inactive, with the panel's tree off.
    fn grey_the_screen_under(&mut self, id: u64) {
        let Some(v) = self.dialogs.views.iter().find(|v| v.id == id) else {
            return;
        };
        let (tree, out, x, y) = (v.tree, v.output.clone(), v.x, v.y);
        self.scene.set_enabled(tree, false);
        let still = self.without_debug_panels(false, |s| s.output_still(&out));
        self.scene.set_enabled(tree, true);
        let Some(mut c) = still else {
            return;
        };
        crate::snapshot::greyscale(&mut c);
        let b = self.layout_box(Some(&out));
        let node = self.scene.buffer_create(tree);
        self.scene.lower_to_bottom(node);
        crate::glass::leave_out(&mut self.scene, node);
        self.scene.set_position(node, b.x - x, b.y - y);
        self.scene
            .buffer_set_canvas(node, &c.pixels, c.width as i32, c.height as i32);
        self.scene
            .buffer_set_dest_size(node, Some((b.width, b.height).into()));
        self.scene.set_opacity(node, 0.0);
        let fade = self.fade_still(id, node);
        if let Some(v) = self.dialogs.views.iter_mut().find(|v| v.id == id) {
            v.fade = fade;
        }
    }

    /// Raise dialog `index` with its family (`raise_order`) and give the keyboard to the top one; a
    /// minimised one is restored first.
    fn raise_dialog(&mut self, index: usize) {
        let Some(v) = self.dialogs.views.get_mut(index) else {
            return;
        };
        let restored = std::mem::take(&mut v.minimized);
        let (tree, windowed) = (v.tree, v.dialog.windowed());
        if restored {
            self.scene.set_enabled(tree, true);
        }
        // Over the windows even when it already tops the dialogs' stack.
        if windowed {
            self.scene.raise_to_top(tree);
        }
        let links: Vec<(u64, Option<u64>)> = self
            .dialogs
            .views
            .iter()
            .map(|v| (v.id, v.parent))
            .collect();
        let order = raise_order(&links, index);
        let first_moved = order.iter().enumerate().position(|(k, &i)| k != i);
        if let Some(first) = first_moved {
            let mut old: Vec<Option<DialogView>> = std::mem::take(&mut self.dialogs.views)
                .into_iter()
                .map(Some)
                .collect();
            self.dialogs.views = order.iter().filter_map(|&i| old[i].take()).collect();
            self.renumber_dialogs();
            // A windowed dialog that moved down keeps its place among the windows.
            for i in first..self.dialogs.views.len() {
                let v = &self.dialogs.views[i];
                if !v.dialog.windowed() {
                    self.scene.raise_to_top(v.tree);
                }
            }
        }
        let activated = self.activate_dialogs();
        let blurred = self.blur_dialogs_without_keyboard();
        if activated || blurred || first_moved.is_some() || restored {
            self.restart_caret();
            self.redraw_dialogs();
            self.sync_dialog_tasks(false);
        }
        if restored {
            self.redraw_all();
        }
    }

    /// Minimise windowed dialog `id`: its tree goes off and it drops to the bottom of the stack,
    /// so the keyboard passes to the dialog now on top or, with none shown, to the top window.
    pub fn minimize_dialog(&mut self, id: u64) {
        let Some(i) = self.dialogs.views.iter().position(|v| v.id == id) else {
            return;
        };
        if self.dialogs.views[i].minimized || !self.dialogs.views[i].dialog.windowed() {
            return;
        }
        let mut v = self.dialogs.views.remove(i);
        v.minimized = true;
        self.scene.set_enabled(v.tree, false);
        self.dialogs.views.insert(0, v);
        self.settle_dialogs();
        self.blur_dialogs_without_keyboard();
        self.redraw_dialogs();
        self.sync_dialog_tasks(false);
        self.redraw_all();
    }

    /// A left click on dialog `id`'s task button: an active windowed dialog minimises, any other
    /// comes up with the keyboard.
    pub fn task_dialog_click(&mut self, id: u64) {
        let windowed = (self.dialogs.views.iter()).any(|v| v.id == id && v.dialog.windowed());
        if windowed && self.active_dialog_id() == Some(id) {
            self.minimize_dialog(id);
        } else {
            self.focus_dialog(id);
        }
    }

    /// Whether the top dialog has the keyboard; a Welcome panel always does, a minimised one never.
    pub fn dialog_has_keyboard(&self) -> bool {
        self.dialogs.views.last().is_some_and(|v| {
            !v.minimized && (self.dialogs.active || matches!(v.dialog.style, Style::Welcome))
        })
    }

    /// The active dialog: the top one while it has the keyboard. Its task button draws pushed in.
    pub fn active_dialog_id(&self) -> Option<u64> {
        self.dialogs
            .views
            .last()
            .filter(|_| self.dialog_has_keyboard())
            .map(|v| v.id)
    }

    /// Whether the top dialog is a Welcome panel, which keeps the keyboard even from Alt+Tab.
    pub fn dialog_is_modal(&self) -> bool {
        self.dialogs
            .views
            .last()
            .is_some_and(|v| matches!(v.dialog.style, Style::Welcome))
    }

    /// Redraw the taskbars after the stack or keyboard changed, when some dialog has a button or
    /// `closed_one` had one.
    fn sync_dialog_tasks(&mut self, closed_one: bool) {
        if closed_one || self.dialogs.views.iter().any(|v| v.dialog.on_taskbar()) {
            self.redraw_taskbars();
        }
    }

    /// Raise dialog `id` and give it the keyboard, as its task button click does; no-op when closed.
    pub fn focus_dialog(&mut self, id: u64) {
        if let Some(i) = self.dialogs.views.iter().position(|v| v.id == id) {
            self.raise_dialog(i);
        }
    }

    /// Give the keyboard to the top dialog; the focused window goes inactive (`focus_top` returns it
    /// when the last dialog closes). Returns whether the dialogs lacked it before.
    fn activate_dialogs(&mut self) -> bool {
        if self.dialogs.active {
            return false;
        }
        self.dialogs.active = true;
        self.end_bar_keys();
        self.drop_window_focus();
        true
    }

    /// A window took the focus: dialogs go inactive. No-op when they lacked the keyboard.
    pub fn deactivate_dialogs(&mut self) {
        if !self.dialogs.active {
            return;
        }
        self.dialogs.active = false;
        self.blur_dialogs_without_keyboard();
        self.redraw_dialogs();
        self.sync_dialog_tasks(false);
    }

    /// Keep the underlines on the dialog with the keyboard only; whether any others hid.
    fn blur_dialogs_without_keyboard(&mut self) -> bool {
        let keyboard = self
            .dialog_has_keyboard()
            .then(|| self.dialogs.views.len() - 1);
        blur_cues_without_keyboard(self.dialogs.views.iter_mut().map(|v| &mut v.cues), keyboard)
    }

    fn close_dialog(&mut self) {
        if let Some(i) = self.dialogs.views.len().checked_sub(1) {
            self.close_dialog_at(i);
        }
    }

    /// Close the top dialog and raise the one it was opened over; returns whether that one is still open.
    fn close_to_parent(&mut self) -> bool {
        let parent = self.dialogs.views.last().and_then(|v| v.parent);
        self.close_dialog();
        match parent.and_then(|p| self.dialogs.views.iter().position(|v| v.id == p)) {
            Some(i) => {
                self.raise_dialog(i);
                true
            }
            None => false,
        }
    }

    pub fn close_dialog_at(&mut self, index: usize) {
        let v = self.dialogs.views.remove(index);
        let on_taskbar = v.dialog.on_taskbar();
        self.destroy_dialog(v);
        self.settle_dialogs();
        self.sync_dialog_tasks(on_taskbar);
    }

    /// After dialogs left the stack or went minimised: redraw the new top, or return the keyboard
    /// to the windows when no dialog is shown.
    fn settle_dialogs(&mut self) {
        self.renumber_dialogs();
        if self.dialogs.views.last().is_none_or(|v| v.minimized) {
            self.stop_caret();
            if self.dialogs.active {
                self.dialogs.active = false;
                self.focus_top();
            }
        } else {
            self.restart_caret();
            self.redraw_top_dialog();
        }
    }

    pub fn close_all_dialogs(&mut self) {
        while !self.dialogs.views.is_empty() {
            self.close_dialog();
        }
    }

    fn fade_still(&mut self, id: u64, node: BufferId) -> Option<RegistrationToken> {
        let mut step = 0;
        let added =
            self.handle
                .insert_source(Timer::from_duration(FADE_TICK), move |_, _, state| {
                    step += 1;
                    state
                        .scene
                        .set_opacity(node, (step as f32 / FADE_STEPS as f32).min(1.0));
                    state.redraw_all();
                    if step < FADE_STEPS {
                        return TimeoutAction::ToDuration(FADE_TICK);
                    }
                    if let Some(v) = state.dialogs.views.iter_mut().find(|v| v.id == id) {
                        v.fade = None;
                    }
                    TimeoutAction::Drop
                });
        match added {
            Ok(t) => Some(t),
            Err(e) => {
                log::warn!("dialog: no fade timer: {}", e.error);
                self.scene.set_opacity(node, 1.0);
                None
            }
        }
    }

    fn destroy_dialog(&mut self, mut v: DialogView) {
        if v.dialog.picker().is_some() {
            self.pickthumbs.forget(v.id);
        }
        self.switcher_forget(crate::switcherview::Entry::Dialog(v.id));
        if let Some(t) = v.fade.take() {
            self.handle.remove(t);
        }
        if matches!(
            self.interaction,
            Interaction::DialogMove { .. }
                | Interaction::DialogResize { .. }
                | Interaction::DialogPress
                | Interaction::FieldDrag { .. }
        ) {
            self.interaction = Interaction::Idle;
        }
        v.backdrop
            .set(&mut self.scene, &self.glass, v.glass_tree, None);
        self.scene.destroy(v.tree);
        self.redraw_all();
        crate::access::touch(self);
    }

    /// Pull every dialog back inside its output's usable area after a layout change
    /// (`update_layout`); the tray's Customize page lays itself out in the new area first.
    pub fn keep_dialogs_inside(&mut self) {
        for i in 0..self.dialogs.views.len() {
            if self.dialogs.views[i].dialog.tray().is_some() {
                self.refit_dialog(i);
                self.redraw_dialog(i);
            }
            let v = &self.dialogs.views[i];
            let area = self.usable_area(Some(&v.output));
            if let Some((x, y)) = refit_origin(&v.dialog, (v.x, v.y), v.layout.size, area) {
                self.move_dialog(i, x, y);
            }
        }
    }

    pub fn close_dialogs_off_scene(&mut self) {
        let (mut i, mut removed, mut on_taskbar) = (0, false, false);
        while i < self.dialogs.views.len() {
            if !self.scene.outputs().contains(&self.dialogs.views[i].output) {
                let v = self.dialogs.views.remove(i);
                on_taskbar |= v.dialog.on_taskbar();
                removed = true;
                self.destroy_dialog(v);
            } else {
                i += 1;
            }
        }
        if removed {
            self.settle_dialogs();
            self.sync_dialog_tasks(on_taskbar);
        }
    }

    fn renumber_dialogs(&mut self) {
        for (i, v) in self.dialogs.views.iter().enumerate() {
            self.scene.set_data(v.tree, Some(SceneOwner::Dialog(i)));
        }
    }

    pub(crate) fn redraw_top_dialog(&mut self) {
        if let Some(i) = self.dialogs.views.len().checked_sub(1) {
            self.redraw_dialog(i);
        }
    }

    /// Redraw every dialog, for a keyboard or theme change.
    pub fn redraw_dialogs(&mut self) {
        for i in 0..self.dialogs.views.len() {
            self.redraw_dialog(i);
        }
    }

    /// The caret timer fired: repaint only the field over the kept raster; full redraw if none or the scale moved.
    pub fn blink_dialog(&mut self) {
        let Some(i) = self.dialogs.views.len().checked_sub(1) else {
            return;
        };
        let v = &self.dialogs.views[i];
        let scale = self.draw_scale_at(Some(&v.output)).max(1);
        let Some((still, at)) = v.still_canvas.as_ref().filter(|(_, at)| *at == scale) else {
            self.redraw_dialog(i);
            return;
        };
        let mut canvas = still.clone();
        if self.caret.on {
            let theme = self.config.theme.scaled(*at);
            dialog::draw_field(&mut canvas, &theme, &self.text, &v.layout, &v.dialog, true);
        }
        let buffer = v.buffer;
        upload_scaled(&mut self.scene, buffer, &canvas, scale);
        self.redraw_all();
    }

    /// Recompute the top dialog's layout and redraw it; the open tab grows, so rects go stale on tab change.
    fn relayout_top_dialog(&mut self) {
        let Some(i) = self.dialogs.views.len().checked_sub(1) else {
            return;
        };
        self.refit_dialog(i);
        self.redraw_dialog(i);
    }

    /// Fit dialog `index` to the theme and its output's usable area and lay it out again, after
    /// a tab or theme change; one whose new size runs past the area's edge moves back inside.
    pub fn refit_dialog(&mut self, index: usize) {
        let Some(output) = self.dialogs.views.get(index).map(|v| v.output.clone()) else {
            return;
        };
        let area = self.usable_area(Some(&output));
        let State {
            config,
            text,
            dialogs,
            ..
        } = self;
        let v = &mut dialogs.views[index];
        let theme = &config.theme;
        let before = v.layout.size;
        v.dialog.fit_in(theme, text, (area.width, area.height));
        v.layout = DialogLayout::compute(theme, text, &v.dialog);
        if v.layout.size != before
            && let Some((x, y)) = refit_origin(&v.dialog, (v.x, v.y), v.layout.size, area)
        {
            self.move_dialog(index, x, y);
        }
    }

    pub fn invalidate_dialogs(&mut self) {
        for v in &mut self.dialogs.views {
            v.icons = None;
        }
    }

    /// Draw dialog `index` again: raster, caret on the blink's shown half while it has the keyboard, glass.
    pub fn redraw_dialog(&mut self, index: usize) {
        let active = index + 1 == self.dialogs.views.len() && self.dialog_has_keyboard();
        let caret_on = self.caret.on && active;
        let Some(v) = self.dialogs.views.get(index) else {
            return;
        };
        let output = v.output.clone();
        let base = self.config.theme.clone();
        let scale = self.draw_scale_at(Some(&output)).max(1);
        let rescaled = self.dialogs.views[index]
            .dialog
            .picker_mut()
            .is_some_and(|p| p.follow_scale(scale, &mut self.icons));
        if rescaled {
            self.run_picker_requests();
        }
        let theme = base.scaled(scale);
        let wanted = crate::glass::wanted(&self.config);
        let preedit = active
            .then(|| self.ime_field_preedit(crate::caret::Editor::Dialog))
            .flatten();
        let State {
            dialogs,
            icons: cache,
            text,
            scene,
            glass,
            ..
        } = self;
        let v = &mut dialogs.views[index];
        let px = dialog::ICON_PX * scale;
        let key = icon_key(&v.dialog, scale);
        let mut undecoded = Vec::new();
        if v.icons.as_ref().map(|(k, _)| k) != Some(&key) {
            let mut get = |n: &Option<String>, px: i32| {
                n.as_deref()
                    .and_then(|n| cached(cache, n, px, &mut undecoded))
            };
            let set = key.icon.as_deref() == Some(reprisekit::controls::filesheet::SET_ICON);
            let icons = DialogIcons {
                icon: if set {
                    Some(reprisekit::controls::filesheet::set_picture(scale))
                } else {
                    get(&key.icon, px)
                },
                caption: get(&key.icon, theme.icon_size()),
                buttons: key.buttons.iter().map(|n| get(n, px)).collect(),
                logo: if key.logo {
                    cache.get("start-here", px)
                } else {
                    None
                },
                rows: HashMap::new(),
            };
            v.icons = Some((key, icons));
        }
        let names = row_icon_names(&v.dialog);
        let Some((_, icons)) = v.icons.as_mut() else {
            return;
        };
        for (_, n, size) in &names {
            icons
                .rows
                .entry((n.clone(), *size))
                .or_insert_with(|| cached(cache, n, size * scale, &mut undecoded));
        }
        let mut row_icons: Vec<Option<&Canvas>> =
            vec![None; names.last().map_or(0, |(i, _, _)| i + 1)];
        for (i, n, size) in &names {
            row_icons[*i] = icons.rows[&(n.clone(), *size)].as_ref();
        }
        // A form's focused edit field draws its caret on the blink's shown half.
        if let Some(page) = v.dialog.form_mut().and_then(|f| f.open_page_mut()) {
            page.caret = caret_on;
        }
        let mut canvas = dialog::draw(
            &theme,
            text,
            &v.layout,
            icons.icon.as_ref(),
            icons.caption.as_ref(),
            &icons.buttons,
            &row_icons,
            icons.logo.as_ref(),
            &v.dialog,
            false,
            v.dialog.lit(),
            v.cues,
            active,
            preedit.as_ref(),
        );
        if let (Some(c), Some(client)) = (v.dialog.copy(), v.layout.page) {
            dialog::draw_copy(
                &mut canvas,
                &base,
                text,
                c,
                client,
                scale,
                active,
                v.cues.keys,
            );
        }
        // What the backdrop sees: the client area.
        let part = wanted
            .then(|| client_part(&canvas, &v.layout, scale))
            .flatten();
        match part {
            Some((c, at)) => {
                let shown = Some((c, at, scale));
                if v.content_shown != shown {
                    if let Some((c, at, _)) = &shown {
                        upload_scaled(scene, v.content, c, scale);
                        scene.set_position(v.content, at.x, at.y);
                    }
                    v.content_shown = shown;
                }
            }
            None => {
                if v.content_shown.take().is_some() {
                    scene.buffer_clear(v.content);
                }
            }
        }
        if v.dialog.field.is_some() {
            v.still_canvas = Some((canvas.clone(), scale));
            if caret_on {
                dialog::draw_field(&mut canvas, &theme, text, &v.layout, &v.dialog, true);
            }
        }
        upload_scaled(scene, v.buffer, &canvas, scale);
        let pane = glass_pane(&base, wanted, &v.layout);
        v.backdrop.set(scene, glass, v.glass_tree, pane);
        let origin = (v.x, v.y);
        let id = v.id;
        let decode = next_decode(&mut v.decodes, undecoded, std::time::Instant::now());
        if active {
            let spot = canvas.take_spot();
            self.ime_field_drawn(crate::caret::Editor::Dialog, spot, origin, scale);
        }
        match decode {
            Decode::Start(read, files) => {
                self.decode_dialog_icons(id, read, files);
                self.relook_dialog_icons(id, crate::fsjobs::STUCK);
            }
            Decode::Retry(wait) => self.relook_dialog_icons(id, wait),
            Decode::Wait => {}
        }
        self.redraw_all();
        crate::access::touch(self);
    }

    /// Decodes dialog `id`'s icon files on a file job, since one can sit on a mount that never
    /// answers, handing each to the loop as it lands so one dead file holds back no other.
    fn decode_dialog_icons(&mut self, id: u64, read: u64, files: IconFiles) {
        let tx = self.fsjobs.sender();
        self.spawn_fs_job(move || {
            let mut rest = Vec::new();
            for (f, px) in files {
                let c = reprisekit::icontheme::decode_file(std::path::Path::new(&f), px, 1);
                let art = vec![(f, px, c)];
                match &tx {
                    Some(tx) => {
                        let _ = tx.send(Box::new(move |state: &mut State| {
                            state.dialog_icons_decoded(id, read, art, false)
                        }));
                    }
                    None => rest.extend(art),
                }
            }
            Box::new(move |state: &mut State| state.dialog_icons_decoded(id, read, rest, true))
        });
    }

    /// Takes icon files job `read` decoded for dialog `id`, and draws the dialog with them.
    fn dialog_icons_decoded(
        &mut self,
        id: u64,
        read: u64,
        art: Vec<(String, i32, Option<Canvas>)>,
        done: bool,
    ) {
        let files: IconFiles = art.iter().map(|(f, px, _)| (f.clone(), *px)).collect();
        self.icons.put_files(art);
        let Some(i) = self.dialogs.views.iter().position(|v| v.id == id) else {
            return;
        };
        let v = &mut self.dialogs.views[i];
        if done {
            v.decodes.land(read);
        } else if let Some(rest) = v.decodes.reading_mut(read) {
            rest.retain(|f| !files.contains(f));
        }
        v.icons = None;
        self.redraw_dialog(i);
    }

    /// Looks dialog `id`'s icons up again in `wait`, when a decode job still running will have
    /// turned stuck and a newer one may take the files it holds back.
    fn relook_dialog_icons(&mut self, id: u64, wait: std::time::Duration) {
        let added = self
            .handle
            .insert_source(Timer::from_duration(wait), move |_, _, state| {
                if let Some(i) = state.dialogs.views.iter().position(|v| v.id == id) {
                    state.dialogs.views[i].icons = None;
                    state.redraw_dialog(i);
                }
                TimeoutAction::Drop
            });
        if let Err(e) = added {
            log::warn!("dialog icon retry timer: {e}");
        }
    }

    /// Dialog `id`'s tree, placed where it stands.
    pub fn dialog_access(&self, id: u64) -> Option<reprisekit::access::Tree> {
        use reprisekit::access::{ROOT, accesskit::Role};
        let v = self.dialogs.views.iter().find(|v| v.id == id)?;
        let (w, h) = v.layout.size;
        let rect = Rect::new(v.x, v.y, w, h);
        let mut tree = crate::access::root(Role::Dialog, &v.dialog.title, rect);
        if matches!(v.dialog.style, Style::Welcome)
            && let Some(n) = tree.get_mut(ROOT)
        {
            n.set_modal();
        }
        let px = self.config.theme.fonts.size_px;
        if let (Some(c), Some(client)) = (v.dialog.copy(), v.layout.page) {
            dialog::access_copy(c, client, &self.config.theme, &self.text, &mut tree);
            return Some(tree);
        }
        let focus = dialog::access(&v.dialog, &v.layout, &self.text, px, &mut tree);
        tree.set_focus(focus);
        Some(tree)
    }

    /// The pointer moved onto dialog `index`'s caption button `b` when `over` is
    /// `Some((index, Some(b)))`; a styled look redraws on hot changes, Classic does not.
    pub fn dialog_hover_caption(&mut self, over: Option<(usize, Option<CaptionButton>)>) {
        let styled = self.config.theme.look.style().is_some();
        for i in 0..self.dialogs.views.len() {
            let hot = (over.filter(|o| styled && o.0 == i)).and_then(|o| o.1);
            let v = &mut self.dialogs.views[i];
            if v.dialog.caption_hot != hot {
                v.dialog.caption_hot = hot;
                if styled {
                    self.redraw_dialog(i);
                }
            }
        }
    }

    /// The pointer moved with nothing held: `at` is `Some((index, over))` or off every dialog. A styled
    /// look lights the control under it; Classic redraws nothing.
    pub fn dialog_hover(&mut self, at: Option<(usize, Option<DialogHit>)>) {
        let styled = self.config.theme.look.lights_pointer();
        let pos = self.pointer_location();
        for i in 0..self.dialogs.views.len() {
            let v = &mut self.dialogs.views[i];
            let redraw = match at {
                Some((j, over)) if j == i => {
                    let p = to_local(pos.x, pos.y, (v.x, v.y));
                    v.dialog.hover(over, p, styled)
                }
                _ => v.dialog.leave(styled),
            };
            if redraw {
                self.redraw_dialog(i);
            }
        }
    }

    /// A wheel frame over `over` on dialog `index` (`discrete` in 120ths of a notch, `continuous` its sign
    /// only). The tray list scrolls a row a notch and ignores it while a combo is open; Browse goes to
    /// `browse_wheel`; others take `WHEEL_ROWS` a notch. True when a dialog is there.
    pub fn dialog_wheel(
        &mut self,
        index: usize,
        over: Option<DialogHit>,
        discrete: i32,
        continuous: f64,
    ) -> bool {
        let Some(id) = self.dialogs.views.get(index).map(|v| v.id) else {
            return false;
        };
        if self.dialogs.views.iter().any(|v| v.parent == Some(id)) {
            return true;
        }
        if let Some(handled) = self.browse_wheel(index, discrete, continuous) {
            return handled;
        }
        let v = &mut self.dialogs.views[index];
        if let Some(t) = v.dialog.tray_mut() {
            let (steps, _) = crate::tray::wheel_steps(discrete, continuous, 0);
            if t.wheel(dialog::tray_part(over), -steps) {
                self.redraw_dialog(index);
            }
            return true;
        }
        let (steps, rest) = crate::pointers::wheel_steps(discrete, continuous, v.wheel);
        v.wheel = rest;
        if steps == 0 {
            return true;
        }
        let rows = -steps * reprisekit::pointer::WHEEL_ROWS;
        match v.dialog.wheel(over, rows) {
            Outcome::Nothing => {}
            Outcome::Redraw => self.redraw_dialog(index),
            out if index + 1 == self.dialogs.views.len() => self.apply_dialog(out),
            _ => {}
        }
        true
    }

    /// The wheel over Browse dialog `index`: scrolls the part under the pointer, `WHEEL_ROWS` a notch.
    fn browse_wheel(&mut self, index: usize, discrete: i32, continuous: f64) -> Option<bool> {
        self.dialogs.views.get(index)?.dialog.picker()?;
        if self.dialogs.views.len() - 1 != index {
            return Some(true);
        }
        let (steps, rest) = crate::pointers::wheel_steps(discrete, continuous, self.panels.wheel);
        self.panels.wheel = rest;
        if steps == 0 {
            return Some(true);
        }
        let pos = self.pointer_location();
        let hit = match self.dialog_hit(index, pos.x, pos.y) {
            Some(DialogHit::Browse(h)) => Some(h),
            _ => None,
        };
        let v = &mut self.dialogs.views[index];
        let (Some(p), Some(client)) = (v.dialog.picker_mut(), v.layout.page) else {
            return Some(true);
        };
        let outcome = p.wheel(client, hit, -steps * reprisekit::pointer::WHEEL_ROWS);
        self.apply_dialog(outcome);
        Some(true)
    }

    pub fn dialog_hit(&self, index: usize, lx: f64, ly: f64) -> Option<DialogHit> {
        let v = self.dialogs.views.get(index)?;
        hit(v, lx, ly, &self.text, &self.config.theme)
    }

    /// Start sizing the top dialog from `edges`, via its border or Browse's grip.
    fn begin_dialog_size(&mut self, edges: Edges) {
        let Some(index) = self.dialogs.views.len().checked_sub(1) else {
            return;
        };
        let pos = self.pointer_location();
        let v = &self.dialogs.views[index];
        let (w, h) = v.layout.size;
        self.interaction = Interaction::DialogResize {
            index,
            edges,
            start: Rect::new(v.x, v.y, w, h),
            cx: pos.x,
            cy: pos.y,
        };
        self.set_cursor(edges.cursor_name());
    }

    /// The sizing pointer moved (`dx`, `dy`) since the press: the dialog takes the new client size and
    /// is laid out again; an unchanged size draws nothing.
    pub fn size_dialog(&mut self, index: usize, edges: Edges, start: Rect, dx: i32, dy: i32) {
        let State {
            config,
            text,
            dialogs,
            ..
        } = self;
        let Some(v) = dialogs.views.get_mut(index) else {
            return;
        };
        let Some(page) = v.layout.page else {
            return;
        };
        let pad = (v.layout.size.0 - page.width, v.layout.size.1 - page.height);
        let Some(p) = v.dialog.picker_mut() else {
            return;
        };
        let r = sized_frame(start, edges, (dx, dy), p.dlg.min_client(), pad);
        let client = (r.width - pad.0, r.height - pad.1);
        if client == p.dlg.client() {
            return;
        }
        p.dlg.set_size(client);
        v.layout = DialogLayout::compute(&config.theme, text, &v.dialog);
        self.move_dialog(index, r.x, r.y);
        self.redraw_dialog(index);
    }

    /// Put dialog `index`'s top-left at (`x`, `y`) in layout coordinates.
    pub fn move_dialog(&mut self, index: usize, x: i32, y: i32) {
        let Some(v) = self.dialogs.views.get_mut(index) else {
            return;
        };
        if (v.x, v.y) == (x, y) {
            return;
        }
        v.x = x;
        v.y = y;
        let tree = v.tree;
        self.scene.set_position(tree, x, y);
        if index + 1 == self.dialogs.views.len() {
            self.ime_field_moved(crate::caret::Editor::Dialog, (x, y));
        }
        self.redraw_all();
    }

    /// A thumb button pressed on dialog `index`: Back or Forward on a top Browse dialog, else ignored.
    pub fn dialog_nav(&mut self, index: usize, nav: reprisekit::pointer::Nav) {
        if self.dialogs.views.len().checked_sub(1) != Some(index) {
            return;
        }
        let outcome = match self
            .dialogs
            .views
            .last_mut()
            .and_then(|v| v.dialog.picker_mut())
        {
            Some(p) => p.nav(nav),
            None => return,
        };
        self.apply_dialog(outcome);
    }

    /// A left press on dialog `index` at `hit` (`None` for the bare body). A dialog under the top one is
    /// raised first; one with another opened over it raises that one instead.
    pub fn dialog_press(&mut self, index: usize, hit: Option<DialogHit>, time_msec: u32) {
        let Some(id) = self.dialogs.views.get(index).map(|v| v.id) else {
            return;
        };
        self.raise_dialog(index);
        if self.dialogs.views.last().is_some_and(|v| v.id != id) {
            return;
        }
        let top = self.dialogs.views.len() - 1;
        if matches!(hit, Some(DialogHit::Caption)) {
            // The raise renumbered the stack; the pressed dialog is the top one now.
            let pos = self.pointer_location();
            let v = &self.dialogs.views[top];
            self.interaction = Interaction::DialogMove {
                index: top,
                dx: pos.x - f64::from(v.x),
                dy: pos.y - f64::from(v.y),
            };
            return;
        }
        if let Some(DialogHit::Resize(e)) = hit {
            self.begin_dialog_size(e);
            return;
        }
        let lights = self.config.theme.look.lights_pointer();
        let v = &mut self.dialogs.views[top];
        let outcome = match hit {
            Some(h @ (DialogHit::Close | DialogHit::Minimize)) => {
                if let Some(b) = dialog::caption_button(Some(h)) {
                    v.dialog.press_caption(b);
                }
                self.interaction = Interaction::DialogPress;
                Outcome::Redraw
            }
            // The ? box does nothing but shows the press.
            Some(DialogHit::Help) => {
                v.dialog.press_help();
                self.interaction = Interaction::DialogPress;
                if lights {
                    Outcome::Redraw
                } else {
                    Outcome::Nothing
                }
            }
            Some(DialogHit::Button(i)) => {
                if v.dialog.press_button(i) {
                    self.interaction = Interaction::DialogPress;
                    Outcome::Redraw
                } else {
                    Outcome::Nothing
                }
            }
            Some(DialogHit::Field(x))
                if v.dialog
                    .field
                    .as_ref()
                    .is_some_and(|f| f.combo.open.is_none()) =>
            {
                self.press_dialog_field(x, time_msec)
            }
            other => self.press_dialog_body(other, time_msec),
        };
        self.apply_dialog(outcome);
    }

    /// A left press at the pointer for double-click pairing, in the theme's double-click time.
    fn dialog_click(&self, time_msec: u32) -> reprisekit::pointer::Click {
        let p = self.pointer_location();
        let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
        self.config.theme.metrics.click(x, y, time_msec)
    }

    /// The font size dialogs are set in.
    fn top_dialog_px(&self) -> u32 {
        self.config.theme.fonts.size_px
    }

    /// A left press in the top dialog's field at text offset `x`: caret under the pointer (Shift
    /// extends), maybe a drag-select; a second press in the double-click time selects the word.
    fn press_dialog_field(&mut self, x: i32, time_msec: u32) -> Outcome {
        let at = self.dialog_click(time_msec);
        let shift = self
            .seat
            .get_keyboard()
            .is_some_and(|k| k.modifier_state().shift);
        let px = self.top_dialog_px();
        self.restart_caret();
        let State { dialogs, text, .. } = self;
        let Some(v) = dialogs.views.last_mut() else {
            return Outcome::Nothing;
        };
        if v.field_press.press(0, at) {
            v.dialog.double_click_field(text, px, x)
        } else {
            let out = v.dialog.press_field(text, px, x, shift);
            self.interaction = Interaction::FieldDrag {
                editor: crate::caret::Editor::Dialog,
            };
            out
        }
    }

    /// A left press elsewhere on the top dialog: a Browse picker or the dialog clicks `hit`, double-click
    /// on a repeat press on the same row. A held thumb or form button keeps the pointer until release.
    fn press_dialog_body(&mut self, hit: Option<DialogHit>, time_msec: u32) -> Outcome {
        let at = self.dialog_click(time_msec);
        let Some(v) = self.dialogs.views.last_mut() else {
            return Outcome::Nothing;
        };
        let double = match hit {
            Some(DialogHit::Chooser(openwith::Hit::List(grouplist::Hit::Row(i)))) => {
                v.row_press.press(i, at)
            }
            Some(DialogHit::Browse(filedlg::Hit::Shell(shellview::Hit::Entry(i)))) => {
                v.row_press.press(i, at)
            }
            Some(DialogHit::Control(h))
                if matches!(h.part, reprisekit::controls::form::Part::Edit(_)) =>
            {
                v.field_press.press(h.item, at)
            }
            _ => {
                v.row_press.reset();
                v.field_press.reset();
                false
            }
        };
        let out = match (v.dialog.picker_mut(), v.layout.page) {
            (Some(p), Some(client)) => {
                let hit = match hit {
                    Some(DialogHit::Browse(h)) => Some(h),
                    _ => None,
                };
                if double {
                    p.double_click(client, hit)
                } else {
                    p.click(client, hit)
                }
            }
            _ if double => v.dialog.double_click(hit),
            _ => v.dialog.click(hit),
        };
        if v.dialog.tray().is_some_and(|t| t.ticking()) {
            let id = v.id;
            self.tick_tray(id);
        }
        let Some(v) = self.dialogs.views.last_mut() else {
            return out;
        };
        let page = v.dialog.form().and_then(|f| f.open_page());
        if page.is_some_and(|p| p.drag.is_some()) || v.dialog.holding_thumb() {
            self.interaction = Interaction::FieldDrag {
                editor: crate::caret::Editor::Dialog,
            };
        } else if page.is_some_and(|p| p.holding())
            || v.dialog.chooser().is_some_and(|c| c.ow.holding())
            || v.dialog.tray().is_some_and(|t| t.holding())
            || v.dialog.copy().is_some_and(|c| c.cp.form.holding())
            || v.dialog.field_lit.holding()
        {
            // A held push button acts on release.
            self.interaction = Interaction::DialogPress;
        }
        out
    }

    /// Tick the tray's Customize page `id` while its page arrow is held: the page steps as the
    /// arrow repeats.
    fn tick_tray(&mut self, id: u64) {
        let every = reprisekit::controls::scrollbar::REPEAT_EVERY;
        let added = self
            .handle
            .insert_source(Timer::from_duration(every), move |now, _, state| {
                let Some(i) = state.dialogs.views.iter().position(|v| v.id == id) else {
                    return TimeoutAction::Drop;
                };
                let Some(t) = state.dialogs.views[i].dialog.tray_mut() else {
                    return TimeoutAction::Drop;
                };
                let moved = t.tick(now) != reprisekit::controls::traycustomize::Event::None;
                let ticking = t.ticking();
                if moved {
                    state.redraw_dialog(i);
                }
                if ticking {
                    TimeoutAction::ToDuration(every)
                } else {
                    TimeoutAction::Drop
                }
            });
        if let Err(e) = added {
            log::warn!("dialog: no scroll timer: {}", e.error);
        }
    }

    /// The pointer, held since a field or thumb press, is at (`lx`, `ly`): extend the selection or drive the thumb.
    pub fn dialog_drag_field(&mut self, lx: f64, ly: f64) {
        let px = self.top_dialog_px();
        let State { dialogs, text, .. } = self;
        let Some(v) = dialogs.views.last_mut() else {
            return;
        };
        let outcome = if let Some(fd) = v.dialog.form_mut() {
            let (x, y) = to_local(lx, ly, (v.x, v.y));
            let over = fd.open_page().and_then(|p| p.hit(x, y, text, px));
            let ev = fd
                .open_page_mut()
                .map_or(reprisekit::controls::form::Event::None, |p| {
                    p.motion(x, y, over)
                });
            fd.event(ev)
        } else if v.dialog.holding_thumb() {
            let y = to_local(lx, ly, (v.x, v.y)).1;
            v.dialog.drag_list(&v.layout, y)
        } else {
            let Some(field) = v.layout.field else {
                return;
            };
            let x =
                to_local(lx, ly, (v.x, v.y)).0 - field.x - reprisekit::classic::widgets::FIELD_PAD;
            v.dialog.drag_field(text, px, x)
        };
        self.apply_dialog(outcome);
    }

    /// The pointer moved with a push button held: it draws pushed only while over it.
    pub fn dialog_drag_button(&mut self) {
        let pos = self.pointer_location();
        let Some(i) = self.dialogs.views.len().checked_sub(1) else {
            return;
        };
        let over = self.dialog_hit(i, pos.x, pos.y);
        let styled = self.config.theme.look.lights_pointer();
        let px = self.top_dialog_px();
        let State { dialogs, text, .. } = self;
        let v = &mut dialogs.views[i];
        if v.dialog.holding_button() {
            if v.dialog.drag_button(pressed_button(over)) {
                self.redraw_dialog(i);
            }
            return;
        }
        if v.dialog.caption_box.holding() {
            if v.dialog.drag_caption(over) {
                self.redraw_dialog(i);
            }
            return;
        }
        if v.dialog.help_box.holding() {
            if v.dialog.drag_help(matches!(over, Some(DialogHit::Help))) && styled {
                self.redraw_dialog(i);
            }
            return;
        }
        if v.dialog.field_lit.holding() {
            if v.dialog.drag_drop(over) && styled {
                self.redraw_dialog(i);
            }
            return;
        }
        if let (Some(c), Some(client)) = (v.dialog.chooser_mut(), v.layout.page) {
            let h = match over {
                Some(DialogHit::Chooser(h)) => Some(h),
                _ => None,
            };
            let lay =
                c.ow.layout(text, px, (client.x, client.y), &v.layout.controls);
            let y = to_local(pos.x, pos.y, (v.x, v.y)).1;
            let ev = match c.ow.drag(&lay, y) {
                Some(ev) => ev,
                None => c.ow.motion(h),
            };
            if ev != openwith::Event::None {
                self.redraw_dialog(i);
            }
            return;
        }
        if let (Some(t), Some(client)) = (v.dialog.tray_mut(), v.layout.page) {
            let lay = t.layout(text, px, (client.x, client.y));
            let y = to_local(pos.x, pos.y, (v.x, v.y)).1;
            let ev = match t.drag(&lay, y) {
                Some(ev) => ev,
                None => t.motion(dialog::tray_part(over)),
            };
            if ev != reprisekit::controls::traycustomize::Event::None {
                self.redraw_dialog(i);
            }
            return;
        }
        if v.dialog.picker().is_some() {
            if v.dialog.drag_picker(over) {
                self.redraw_dialog(i);
            }
            return;
        }
        let (x, y) = to_local(pos.x, pos.y, (v.x, v.y));
        let Some(fd) = v.dialog.form_mut() else {
            return;
        };
        let on = fd.open_page().and_then(|p| p.hit(x, y, text, px));
        let ev = fd
            .open_page_mut()
            .map_or(reprisekit::controls::form::Event::None, |p| {
                p.motion(x, y, on)
            });
        let outcome = fd.event(ev);
        self.apply_dialog(outcome);
    }

    /// The pointer let go after a push button press: acts only on the pressed button.
    pub fn dialog_release_button(&mut self) {
        let pos = self.pointer_location();
        let Some(i) = self.dialogs.views.len().checked_sub(1) else {
            return;
        };
        let over = self.dialog_hit(i, pos.x, pos.y);
        let styled = self.config.theme.look.lights_pointer();
        let v = &mut self.dialogs.views[i];
        if v.dialog.caption_box.holding() {
            v.dialog.drag_caption(over);
            let outcome = match v.dialog.release_caption() {
                Some(CaptionButton::Close) => Outcome::Close,
                Some(CaptionButton::Minimize) => {
                    let id = v.id;
                    self.minimize_dialog(id);
                    return;
                }
                _ => Outcome::Redraw,
            };
            self.apply_dialog(outcome);
            return;
        }
        if v.dialog.help_box.holding() {
            v.dialog.drag_help(matches!(over, Some(DialogHit::Help)));
            if v.dialog.release_help() && styled {
                self.redraw_dialog(i);
            }
            return;
        }
        let outcome = match v.dialog.release_button(pressed_button(over)) {
            Some(out) => out,
            None => v.dialog.release(over),
        };
        self.apply_dialog(outcome);
    }

    /// The pointer was released: a held thumb lets go.
    pub fn dialog_release(&mut self) {
        let outcome = match self.dialogs.views.last_mut() {
            Some(v) => v.dialog.release(None),
            None => return,
        };
        self.apply_dialog(outcome);
    }

    /// The pointer is taken away mid-press: the top dialog lets go without acting.
    pub fn dialog_cancel_press(&mut self) {
        let outcome = match self.dialogs.views.last_mut() {
            Some(v) => v.dialog.cancel_press(),
            None => return,
        };
        self.apply_dialog(outcome);
    }

    pub fn dialog_paste(&mut self, text: &str) {
        let outcome = match self.dialogs.views.last_mut() {
            Some(v) => v.dialog.paste(text),
            None => return,
        };
        self.apply_dialog(outcome);
    }

    /// A key press for the top dialog; always consumed.
    pub fn dialog_key(&mut self, k: &DialogKey) {
        use crate::keys::{KEY_Alt_L, KEY_Alt_R, MOD_ALT, MOD_CTRL, MOD_SHIFT};
        let alt = k.mods & MOD_ALT != 0;
        // Any key brings out focus rectangles, Alt the underlines ([`Cues::key`]); bare Alt redraws here.
        let alt_key = k.syms.iter().any(|&s| s == KEY_Alt_L || s == KEY_Alt_R);
        if let Some(v) = self.dialogs.views.last_mut()
            && v.cues.key(alt || alt_key)
        {
            self.redraw_top_dialog();
        }
        let ctrl = k.mods & MOD_CTRL != 0;
        let shift = k.mods & MOD_SHIFT != 0;
        // The table every repriseapp program reads; Ctrl+Tab cycles pages.
        let special = k
            .syms
            .iter()
            .find_map(|&sym| reprisekit::controls::form::special_key(sym, shift, alt));
        if let (Some(Key::Tab { back }), true) = (&special, ctrl) {
            let back = *back;
            let outcome = match self.dialogs.views.last_mut() {
                Some(v) => v.dialog.cycle_page(back),
                None => return,
            };
            self.apply_dialog(outcome);
            return;
        }
        // Everything else is an editing key. Alt+letter is an access key even in a field; bare Backspace
        // stays its own key so a form can route it to the focused item.
        let key = match special.or_else(|| {
            k.edit.clone().and_then(|e| match e {
                EditKey::Insert(s) if alt => {
                    let mut chars = s.chars();
                    match (chars.next(), chars.next()) {
                        (Some(c), None) if !c.is_whitespace() => Some(Key::Access(c)),
                        _ => None,
                    }
                }
                EditKey::Backspace { word: false } => Some(Key::Backspace),
                e => Some(Key::Edit(e)),
            })
        }) {
            Some(k) => k,
            None => return,
        };
        if let Key::Edit(e) = &key {
            self.clip_edit_key(e);
        }
        let outcome = match self.dialogs.views.last_mut() {
            Some(v) => match (v.dialog.picker_mut(), v.layout.page) {
                (Some(p), Some(client)) => p.key(client, key),
                // Ctrl+Enter breaks a memo line; Ctrl+Home/End move its caret to the ends.
                _ if ctrl => v.dialog.key(reprisekit::controls::form::ctrl_key(key)),
                _ => v.dialog.key(key),
            },
            None => return,
        };
        self.restart_caret();
        self.apply_dialog(outcome);
    }

    /// Run's Browse...: on the home folder, in the corner by the Start button, 3 px in from the screen edge.
    fn browse_run(&mut self) {
        let start = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| std::path::PathBuf::from("/"));
        let pos = self.pointer_location();
        let out = self.output_at(pos.x, pos.y);
        let scale = self.draw_scale_at(out.as_ref());
        let icons = crate::filepick::icons(&mut self.icons, scale);
        let mut picker =
            crate::filepick::FilePicker::run_command(&start, icons, self.config.recent_files);
        picker.remember_in(repriseconf::filedialog::path());
        let mut d = Dialog::browse(picker);
        let kept = self.restore_browse_in(&mut d);
        let area = self.usable_area(out.as_ref());
        let edge = out
            .as_ref()
            .map_or(self.config.taskbar.position, |o| self.bar_placement(o).edge);
        let size = DialogLayout::compute(&self.config.theme, &self.text, &d).size;
        let (x, y) = dialog::browse_by_start(area, edge, size);
        d.placement = Placement::At(x, y);
        self.open_browse(d, kept);
    }

    /// The Open With chooser's Browse...: "Open With..." on `/usr/bin`, cascaded 3 px right and 22 px below the chooser.
    fn browse_program(&mut self) {
        let scale = self
            .dialogs
            .views
            .last()
            .map_or(1, |v| self.draw_scale_at(Some(&v.output)));
        let icons = crate::filepick::icons(&mut self.icons, scale);
        let mut picker = crate::filepick::FilePicker::program(
            std::path::Path::new("/usr/bin"),
            icons,
            self.config.recent_files,
        );
        picker.remember_in(repriseconf::filedialog::path());
        let mut d = Dialog::browse(picker);
        d.title = "Open With...".into();
        let kept = self.restore_browse_in(&mut d);
        if let Some(chooser) = self.dialogs.views.last() {
            d.placement = Placement::At(chooser.x + 3, chooser.y + 22);
        }
        self.open_browse(d, kept);
    }

    /// Sizes Browse dialog `d` from the kept file dialog geometry; the place it goes, if any.
    fn restore_browse_in(&self, d: &mut Dialog) -> Option<(i32, i32)> {
        let layout = DialogLayout::compute(&self.config.theme, &self.text, d);
        let page = layout.page?;
        let pad = (layout.size.0 - page.width, layout.size.1 - page.height);
        self.restore_browse(d.picker_mut()?, pad)
    }

    /// Opens Browse dialog `d` over the top dialog, moved to `kept` once it shows.
    fn open_browse(&mut self, d: Dialog, kept: Option<(i32, i32)>) {
        let before = self.dialogs.views.len();
        self.open_dialog_over(d);
        let top = self.dialogs.views.len();
        if let Some((x, y)) = kept
            && top > before
        {
            self.move_dialog(top - 1, x, y);
        }
    }

    /// Open the Properties sheet for `info`, or raise the one open on its file. An unwalked folder reads
    /// "Calculating..." until a walk off the loop returns totals.
    pub fn open_properties(
        &mut self,
        info: &reprisefs::props::FileInfo,
        icon: &str,
        at: (i32, i32),
    ) {
        let open_already = self.sheet_on(&info.path).is_some();
        self.open_dialog(Dialog::properties(info, icon, at));
        if open_already || !info.is_dir || info.contains.is_some() {
            return;
        }
        let Some(id) = self.sheet_on(&info.path) else {
            return;
        };
        let path = info.path.clone();
        self.spawn_fs_job(move || {
            let totals = reprisefs::props::folder_totals(&path);
            Box::new(move |state| state.folder_totals_in(id, totals))
        });
    }

    /// A folder walk for sheet `id` is done; a sheet closed meanwhile drops the totals.
    fn folder_totals_in(&mut self, id: u64, totals: reprisefs::props::FolderTotals) {
        let Some(i) = self.dialogs.views.iter().position(|v| v.id == id) else {
            return;
        };
        if self.dialogs.views[i].dialog.set_folder_totals(totals) {
            self.redraw_dialog(i);
        }
    }

    /// The id of the open Properties sheet on `path`; `open_dialog` keeps it unique.
    fn sheet_on(&self, path: &std::path::Path) -> Option<u64> {
        self.dialogs
            .views
            .iter()
            .find(|v| v.dialog.sheet_path() == Some(path))
            .map(|v| v.id)
    }

    /// The rest of a Properties OK or Apply after `write_properties`: the desktop follows a rename to
    /// `renamed`, then sheet `sheet` closes or stays open on `written`; a closed sheet is left alone.
    fn finish_properties(
        &mut self,
        sheet: Option<u64>,
        path: std::path::PathBuf,
        renamed: Option<std::path::PathBuf>,
        written: &dialog::Written,
        header: Option<crate::desktop::DesktopEntry>,
        close: bool,
    ) {
        if let Some(to) = &renamed {
            self.desktop_renamed(&path, to);
        }
        let Some(i) = self.dialogs.views.iter().position(|v| Some(v.id) == sheet) else {
            return;
        };
        if close {
            self.close_dialog_at(i);
            return;
        }
        match &written.reread {
            // A rewritten shortcut's sheet is rebuilt, its header wearing the icon the desktop gives it now.
            Some(info) => {
                // The desktop may still list the file under its name before a rename.
                let on_desktop = self.desktop.as_ref().filter(|v| {
                    v.entries
                        .iter()
                        .any(|e| e.path == written.path || e.path == path)
                });
                let worn = on_desktop.zip(header).map(|(v, mut e)| {
                    let has = |n: &str| self.icons.has(n);
                    crate::desktop::wear_file_icons(std::slice::from_mut(&mut e), &v.mime, &has);
                    e.icon
                });
                let d = &mut self.dialogs.views[i].dialog;
                let icon = worn.or_else(|| d.icon.clone()).unwrap_or_default();
                d.sheet_reread(info, &icon);
                self.refit_dialog(i);
            }
            None => self.dialogs.views[i].dialog.sheet_applied(written),
        }
        self.redraw_dialog(i);
    }

    /// Runs every Browse dialog's queued folder reads off the loop, and fetches the thumbnails
    /// of the pictures it lists in Thumbnails or Tiles; replies find dialogs by id.
    fn run_picker_requests(&mut self) {
        let jobs: Vec<(u64, crate::filepick::Request)> = self
            .dialogs
            .views
            .iter_mut()
            .filter_map(|v| Some((v.id, v.dialog.picker_mut()?.take_requests())))
            .flat_map(|(id, rs)| rs.into_iter().map(move |r| (id, r)))
            .collect();
        for (id, req) in jobs {
            self.spawn_fs_job(move || {
                let reply = req.run();
                Box::new(move |state| state.picker_reply(id, reply))
            });
        }
        let pictures: Vec<_> = self
            .dialogs
            .views
            .iter_mut()
            .filter_map(|v| Some((v.id, v.dialog.picker_mut()?.take_pictures()?)))
            .collect();
        for (id, (generation, px, paths)) in pictures {
            let to_loop = self.fsjobs.sender();
            self.pickthumbs.want(to_loop, id, generation, px, paths);
        }
    }

    /// Thumbnails fetched for Browse dialog `id`'s listing `generation`; a dialog closed or
    /// relisted meanwhile drops them.
    pub(crate) fn picker_thumbs(
        &mut self,
        id: u64,
        generation: u64,
        done: Vec<crate::pickthumbs::Done>,
    ) {
        let Some(i) = self.dialogs.views.iter().position(|v| v.id == id) else {
            return;
        };
        let Some(p) = self.dialogs.views[i].dialog.picker_mut() else {
            return;
        };
        if p.thumbs(generation, done) {
            self.redraw_dialog(i);
        }
    }

    /// A folder read for Browse dialog `id` is done; a pick acts as if the dialog were on top.
    fn picker_reply(&mut self, id: u64, reply: crate::filepick::Reply) {
        let Some(i) = self.dialogs.views.iter().position(|v| v.id == id) else {
            return;
        };
        let scale = self.draw_scale_at(Some(&self.dialogs.views[i].output));
        let Some(p) = self.dialogs.views[i].dialog.picker_mut() else {
            return;
        };
        match p.reply(reply, &mut self.icons, scale) {
            Outcome::Nothing => {}
            Outcome::Redraw => {
                self.run_picker_requests();
                self.redraw_dialog(i);
            }
            pick => {
                self.raise_dialog(i);
                self.apply_dialog(pick);
            }
        }
    }

    fn spawn_from_dialog(&mut self, cmd: &str) {
        self.spawn_activated(cmd);
    }

    fn spawn_argv_from_dialog<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(&mut self, argv: &[S]) {
        self.spawn_argv_activated(argv);
    }

    pub(crate) fn apply_dialog(&mut self, outcome: Outcome) {
        self.run_picker_requests();
        match outcome {
            Outcome::Nothing => {}
            Outcome::Redraw => self.redraw_top_dialog(),
            Outcome::Relayout => self.relayout_top_dialog(),
            Outcome::Size(edges) => self.begin_dialog_size(edges),
            Outcome::Close | Outcome::CopyCancel
                if self
                    .dialogs
                    .views
                    .last()
                    .is_some_and(|v| v.dialog.copy().is_some()) =>
            {
                let id = self.dialogs.views[self.dialogs.views.len() - 1].id;
                self.cancel_put(id);
            }
            Outcome::CopyCancel => {}
            Outcome::CopyPause(on) => {
                if let Some(id) = self.dialogs.views.last().map(|v| v.id) {
                    self.pause_put(id, on);
                }
            }
            Outcome::Close => self.close_dialog(),
            Outcome::Spawn(cmd) => {
                self.close_dialog();
                self.spawn_from_dialog(&cmd);
            }
            Outcome::EndSession(goal, cmd) => {
                self.close_dialog();
                crate::sessionend::begin(self, goal, Some(cmd));
            }
            Outcome::SessionEnd(choice) => crate::sessionend::answer(self, choice),
            Outcome::Location { desktop_id, answer } => {
                crate::location::answer(self, &desktop_id, answer)
            }
            Outcome::Delete { paths, erase } => {
                self.close_dialog();
                self.delete_paths(paths, erase);
            }
            Outcome::EmptyBin => {
                self.close_dialog();
                self.empty_bin();
            }
            Outcome::Notice(d) => self.open_dialog_over(*d),
            Outcome::Run(cmd) => {
                self.close_dialog();
                dialog::remember(&mut self.dialogs.run_history, &cmd);
                crate::state::save_dialogs(&self.dialogs.run_history);
                self.run_line_activated(&cmd);
            }
            Outcome::Act(a) => {
                self.close_dialog();
                self.run_action(&a, 0);
            }
            Outcome::OpenWith {
                path,
                app,
                always,
                mime,
            } => self.apply_open_with(&path, &app, always, mime.as_deref()),
            Outcome::ChangeDefault(path) => {
                // A symlink's Change... asks for the type of its target, resolved in a file job.
                self.spawn_fs_job(move || {
                    let real = std::fs::canonicalize(&path).unwrap_or(path);
                    Box::new(move |state: &mut State| {
                        let d = state.choose_program(&real, true);
                        state.open_dialog_over(d);
                    })
                });
            }
            Outcome::SetDefault { path, app, mime } => {
                self.apply_set_default(&path, &app, mime.as_deref())
            }
            Outcome::AdvancedAttributes { subject, robots } => {
                let t = &self.config.theme;
                let px = t.fonts.size_px;
                let d = Dialog::advanced_attributes(subject, robots, &self.text, px, t.text);
                self.open_dialog_over(d)
            }
            Outcome::AdvancedChosen(chosen) => {
                if !self.close_to_parent() {
                    return;
                }
                if let Some(i) = self.dialogs.views.len().checked_sub(1)
                    && self.dialogs.views[i].dialog.advanced_chosen(chosen)
                {
                    self.redraw_dialog(i);
                }
            }
            Outcome::BrowseRun => self.browse_run(),
            Outcome::BrowseProgram => self.browse_program(),
            Outcome::ProgramPick(path) => {
                if !self.close_to_parent() {
                    return;
                }
                let outcome = match self.dialogs.views.last_mut() {
                    Some(v) => v.dialog.add_program(&path),
                    None => Outcome::Nothing,
                };
                self.apply_dialog(outcome);
            }
            Outcome::RunPick(path) => {
                if !self.close_to_parent() {
                    return;
                }
                // The field holds text, so a non-UTF-8 name cannot go into it.
                let Some(picked) = path.to_str() else {
                    self.open_dialog_over(Dialog::not_utf8_program("Run", &path));
                    return;
                };
                let quoted = reprisefs::ops::shell_quote(picked);
                let outcome = match self.dialogs.views.last_mut() {
                    Some(v) => v.dialog.set_field(&quoted),
                    None => Outcome::Nothing,
                };
                self.restart_caret();
                self.apply_dialog(outcome);
            }
            Outcome::Properties {
                path,
                name,
                attrs,
                hidden,
                summary,
                folder_icon,
                launcher,
                close,
                recurse,
            } => self.apply_properties(
                Props {
                    path,
                    name,
                    attrs,
                    hidden,
                    summary,
                    folder_icon,
                    launcher,
                },
                close,
                recurse,
            ),
            Outcome::SetProperties {
                paths,
                attrs,
                hidden,
                folder_icon,
                close,
                recurse,
            } => self.apply_set_properties(
                SetWrite {
                    paths,
                    attrs,
                    hidden,
                    folder_icon,
                },
                close,
                recurse,
            ),
            Outcome::ChangeIcon {
                name,
                current,
                launcher,
            } => self.open_change_icon(&name, current, launcher),
            Outcome::FindTarget(program) => self.spawn_argv_from_dialog(&[
                std::ffi::OsStr::new("reprise-explorer"),
                std::ffi::OsStr::new("--select"),
                program.as_os_str(),
            ]),
            Outcome::IconChosen(icon) => {
                if !self.close_to_parent() {
                    return;
                }
                if let Some(i) = self.dialogs.views.len().checked_sub(1)
                    && self.dialogs.views[i].dialog.icon_chosen(icon)
                {
                    self.redraw_dialog(i);
                }
            }
            Outcome::HideSystemIcon(name) => {
                self.close_dialog();
                self.hide_system_icons(&[name]);
            }
            Outcome::DeleteIcons { hide, files } => {
                self.close_dialog();
                self.hide_system_icons(&hide);
                self.trash_files(files);
            }
            Outcome::EndProgram(pid) => {
                self.close_dialog();
                self.end_program(pid);
            }
            Outcome::TurnOn(aid) => {
                self.close_dialog();
                self.set_aid(aid, true);
                self.aid_switched(aid, true);
            }
            Outcome::TrayChoices(c) => {
                self.close_dialog();
                crate::tray::apply_choices(self, c);
            }
            Outcome::Replace { batch, answer } => {
                self.close_dialog();
                self.replace_answered(*batch, answer);
            }
            Outcome::TooBig { mut asker, answer } => {
                self.close_dialog();
                asker.answer(answer);
                self.too_big_step(*asker);
            }
            Outcome::BinSettings { settings, close } => {
                crate::configwrite::save_recycle_bin(&self.config_path, settings, true);
                if close {
                    self.close_dialog();
                } else {
                    self.redraw_top_dialog();
                }
            }
        }
    }

    /// OK on the Open With chooser: run `path` with `app` (a desktop-file id, or a program path from
    /// Browse...) and with `always` make it the default for `mime`; a browsed program gets a desktop
    /// entry first ([`reprisefs::apps::program_entry`]).
    fn apply_open_with(
        &mut self,
        path: &std::path::Path,
        app: &str,
        always: bool,
        mime: Option<&str>,
    ) {
        self.close_dialog();
        let apps: &[reprisefs::apps::AppEntry] = &self.app_index;
        let written = match (always, mime) {
            (true, Some(mime)) => browsed_entry(apps, app, mime),
            _ => None,
        };
        match apps.iter().find(|a| a.id == app).or(written.as_ref()) {
            Some(entry) => {
                if let (true, Some(mime)) = (always, mime)
                    && let Err(e) = reprisefs::props::set_default_app(mime, &entry.id)
                {
                    log::warn!("open with: could not set the default for {mime}: {e}");
                }
                let argv = reprisefs::apps::open_argv(entry, &self.config.terminal, path);
                let app = entry.id.clone();
                self.spawn_app_activated(&argv, &app);
            }
            // A browsed program not made default runs by path with the file as its one argument.
            None if app.starts_with('/') => {
                let argv = [std::ffi::OsStr::new(app), path.as_os_str()];
                self.spawn_argv_from_dialog(&argv);
            }
            None => {}
        }
    }

    /// OK on the chooser a sheet's Change... opened: make `app` the default for `mime` and show it on
    /// the sheet for `path`; a browsed program gets a desktop entry first.
    fn apply_set_default(&mut self, path: &std::path::Path, app: &str, mime: Option<&str>) {
        let sheet_open = self.close_to_parent();
        let apps: &[reprisefs::apps::AppEntry] = &self.app_index;
        let written = mime.and_then(|m| browsed_entry(apps, app, m));
        let Some(entry) = apps.iter().find(|a| a.id == app).or(written.as_ref()) else {
            return;
        };
        let (name, icon) = (entry.name.clone(), entry.icon.clone());
        if let Some(mime) = mime
            && let Err(e) = reprisefs::props::set_default_app(mime, &entry.id)
        {
            log::warn!("change: could not set the default for {mime}: {e}");
            return;
        }
        // The sheet is back on top; its Opens with row shows the new program.
        let Some(i) = self.dialogs.views.len().checked_sub(1) else {
            return;
        };
        if !sheet_open {
            return;
        }
        let sheet = &mut self.dialogs.views[i];
        if sheet.dialog.sheet_path() == Some(path) && sheet.dialog.set_opens_with(name, icon) {
            self.redraw_dialog(i);
        }
    }

    /// A Properties OK or Apply: Read-only and Hidden (asking first for a non-empty folder), then the rename.
    fn apply_properties(&mut self, write: Props, close: bool, recurse: Option<bool>) {
        let sheet = self.sheet_on(&write.path);
        match (!write.attrs.is_empty() || write.hidden.is_some(), recurse) {
            // A non-empty folder gets Confirm Attribute Changes first, since the change can reach the
            // whole tree; the look is a folder read, so it runs off the loop.
            (true, None) => {
                self.spawn_fs_action(Action::reads("Properties"), move |_| {
                    let ask = has_entries(&write.path);
                    Box::new(move |state| {
                        if ask {
                            let c = dialog::AttrConfirm {
                                write: dialog::AttrWrite::One {
                                    path: write.path,
                                    name: write.name,
                                    hidden: write.hidden,
                                    folder_icon: write.folder_icon,
                                },
                                attrs: write.attrs,
                                close,
                            };
                            state.confirm_attributes_over(sheet, c);
                        } else {
                            state.write_properties(sheet, write, false, close);
                        }
                    })
                });
            }
            _ => {
                if recurse.is_some() {
                    self.close_dialog();
                }
                self.write_properties(sheet, write, recurse == Some(true), close);
            }
        }
    }

    /// A set's Properties OK or Apply: Confirm Attribute Changes first when Read-only or Hidden moved
    /// and a non-empty folder is among the paths; then `write_set_properties`.
    fn apply_set_properties(&mut self, write: SetWrite, close: bool, recurse: Option<bool>) {
        let sheet = self
            .dialogs
            .views
            .iter()
            .find(|v| v.dialog.set_paths() == Some(write.paths.as_slice()))
            .map(|v| v.id);
        match (!write.attrs.is_empty() || write.hidden.is_some(), recurse) {
            (true, None) => {
                self.spawn_fs_action(Action::reads("Properties"), move |_| {
                    let ask = write.paths.iter().any(|p| has_entries(p));
                    Box::new(move |state| {
                        if ask {
                            let c = dialog::AttrConfirm {
                                write: dialog::AttrWrite::Set {
                                    paths: write.paths,
                                    hidden: write.hidden,
                                    folder_icon: write.folder_icon,
                                },
                                attrs: write.attrs,
                                close,
                            };
                            state.confirm_attributes_over(sheet, c);
                        } else {
                            state.write_set_properties(sheet, write, false, close);
                        }
                    })
                });
            }
            _ => {
                if recurse.is_some() {
                    self.close_dialog();
                }
                let tree = recurse == Some(true);
                self.write_set_properties(sheet, write, tree, close);
            }
        }
    }

    /// Write a set's Properties off the loop (`reprisefs::props::write_set`); then the desktop follows
    /// renames and sheet `sheet` closes or takes the new paths.
    fn write_set_properties(
        &mut self,
        sheet: Option<u64>,
        write: SetWrite,
        tree: bool,
        close: bool,
    ) {
        let action = Action::changes("Properties", State::rescan_desktop);
        self.spawn_fs_action(action, move |gate| {
            let SetWrite {
                paths,
                attrs,
                hidden,
                folder_icon,
            } = write;
            if !paths.first().is_some_and(|p| gate.change(p)) {
                return Box::new(|_| {});
            }
            let mut failed = Vec::new();
            if let Some(icon) = &folder_icon {
                for dir in paths.iter().filter(|p| p.is_dir()) {
                    if let Err(e) = reprisefs::meta::set_folder_icon(dir, icon.as_deref()) {
                        log::warn!(
                            "properties: could not set the icon of {}: {e}",
                            dir.display()
                        );
                        failed.push((dir.clone(), e.to_string()));
                    }
                }
            }
            let w = reprisefs::props::write_set(paths, attrs, hidden, tree);
            let failed = failed.into_iter().chain(w.failed).next().map(|(p, e)| {
                let name = p.file_name().unwrap_or(p.as_os_str());
                format!("Cannot change {}: {e}", name.to_string_lossy())
            });
            let icons = folder_icon.is_some();
            Box::new(move |state| {
                if icons {
                    state.rescan_desktop();
                }
                state.finish_set_properties(sheet, w.now, &w.renames, close);
                if let Some(text) = failed {
                    state.open_dialog_over_id(Dialog::error("Properties", text), sheet);
                }
            })
        });
    }

    /// The Change Icon box for the folder `name`, over the top sheet: icons from
    /// `filesheet::FOLDER_ICON_NAMES`, a folder's own icon first, `current` picked (standard folder for
    /// None). A launcher (`launcher`) lists installed programs' icons (`reprisefs::launcher::icon_names`).
    fn open_change_icon(&mut self, name: &str, current: Option<String>, launcher: bool) {
        use reprisekit::controls::filesheet::{FOLDER_ICON_NAMES, STANDARD_FOLDER_ICON};
        let scale = self
            .dialogs
            .views
            .last()
            .map_or(1, |v| self.draw_scale_at(Some(&v.output)).max(1));
        let apps: &[reprisefs::apps::AppEntry] = &self.app_index;
        let icons = &mut self.icons;
        let mut names: Vec<String> = if launcher {
            reprisefs::launcher::icon_names(apps, |n| icons.has(n))
        } else {
            FOLDER_ICON_NAMES
                .iter()
                .filter(|n| icons.has(n))
                .map(|n| n.to_string())
                .collect()
        };
        // A folder always offers the standard folder; either offers its own icon from outside the list first.
        let standard = (!launcher).then(|| STANDARD_FOLDER_ICON.to_string());
        for must in [standard, current.clone()].into_iter().flatten() {
            if !names.contains(&must) {
                names.insert(0, must);
            }
        }
        let pictures = names
            .iter()
            .map(|n| icons.get(n, dialog::ICON_PX * scale))
            .collect();
        let mut list = reprisekit::controls::iconlist::IconList::new(names, pictures, None);
        match current.as_deref() {
            Some(c) => {
                list.pick(c);
            }
            None if !launcher => {
                list.pick(STANDARD_FOLDER_ICON);
            }
            None => {}
        }
        let source = self.config.icons.theme.clone();
        self.open_dialog_over(Dialog::change_icon(name, &source, list, launcher));
    }

    /// The rest of a set's OK or Apply after `write_set_properties`: the desktop follows renames, then
    /// sheet `sheet` closes or takes the items' paths `now`; a closed sheet is left alone.
    fn finish_set_properties(
        &mut self,
        sheet: Option<u64>,
        now: Vec<std::path::PathBuf>,
        renames: &[(std::path::PathBuf, std::path::PathBuf)],
        close: bool,
    ) {
        for (from, to) in renames {
            self.desktop_renamed(from, to);
        }
        let Some(i) = self.dialogs.views.iter().position(|v| Some(v.id) == sheet) else {
            return;
        };
        if close {
            self.close_dialog_at(i);
        } else {
            self.dialogs.views[i].dialog.set_applied(now);
            self.redraw_dialog(i);
        }
    }

    /// Open the combined Properties sheet for `set` titled `name`, or raise the one open on it. Unwalked
    /// folders are walked off the loop and their totals join the sheet's.
    pub fn open_set_properties(
        &mut self,
        set: &reprisefs::props::SetInfo,
        name: &str,
        at: (i32, i32),
    ) {
        let find = |state: &State| {
            state
                .dialogs
                .views
                .iter()
                .find(|v| v.dialog.set_paths() == Some(set.paths.as_slice()))
                .map(|v| v.id)
        };
        let open_already = find(self).is_some();
        self.open_dialog(Dialog::set_properties(set, name, at));
        if open_already || !set.walking {
            return;
        }
        let Some(id) = find(self) else {
            return;
        };
        let paths = set.paths.clone();
        self.spawn_fs_job(move || {
            let mut sum = reprisefs::props::FolderTotals::default();
            for d in paths.iter().filter(|p| p.is_dir()) {
                let t = reprisefs::props::folder_totals(d);
                sum.files += t.files;
                sum.folders += t.folders;
                sum.size += t.size;
                sum.on_disk += t.on_disk;
            }
            Box::new(move |state| state.folder_totals_in(id, sum))
        });
    }

    /// Ask Confirm Attribute Changes over sheet `sheet`, raised first; a closed sheet drops the change.
    fn confirm_attributes_over(&mut self, sheet: Option<u64>, c: dialog::AttrConfirm) {
        let Some(i) = self.dialogs.views.iter().position(|v| Some(v.id) == sheet) else {
            return;
        };
        self.raise_dialog(i);
        let t = &self.config.theme;
        let d = Dialog::confirm_attributes(c, &self.text, t.fonts.size_px, t.text);
        self.open_view(d, sheet);
    }

    /// Write a Properties OK or Apply off the loop, so a hung network mount cannot stall the desktop:
    /// Summary fields, launcher keys, folder icon, attributes and Hidden lists (`reprisefs::props::write_item`,
    /// whole tree with `tree`), then the rename to `name`. `finish_properties` follows on the loop.
    fn write_properties(&mut self, sheet: Option<u64>, w: Props, tree: bool, close: bool) {
        let reread = w
            .rereads()
            .then(|| self.dialogs.views.iter().find(|v| Some(v.id) == sheet))
            .flatten()
            .and_then(|v| v.dialog.sheet_name())
            .map(|name| (name.to_string(), self.app_index.clone()));
        let mime = self.desktop.as_ref().map(|v| v.mime.clone());
        let shown = self.config.desktop_extensions;
        let action = Action::changes("Properties", State::rescan_desktop);
        self.spawn_fs_action(action, move |gate| {
            let Props {
                path,
                name,
                attrs,
                hidden,
                summary,
                folder_icon,
                launcher,
            } = w;
            if !gate.change(&path) {
                return Box::new(|_| {});
            }
            // Every write is tried; the first failure is shown and each landed field goes back as written.
            let mut failed: Option<std::io::Error> = None;
            let mut note = |r: std::io::Result<()>| match r {
                Ok(()) => true,
                Err(e) => {
                    failed.get_or_insert(e);
                    false
                }
            };
            let summary_ok = note(reprisefs::meta::write_summary(&path, &summary));
            let launcher_ok = launcher.is_empty()
                || note(
                    reprisefs::launcher::write_keys(&path, &launcher).inspect_err(|e| {
                        log::warn!("properties: could not write {}: {e}", path.display())
                    }),
                );
            let icon_ok = match &folder_icon {
                Some(icon) => note(
                    reprisefs::meta::set_folder_icon(&path, icon.as_deref()).inspect_err(|e| {
                        log::warn!(
                            "properties: could not set the icon of {}: {e}",
                            path.display()
                        )
                    }),
                ),
                None => true,
            };
            let attrs_ok = note(reprisefs::props::write_item(&path, &attrs, hidden, tree));
            let mut rename_ok = true;
            let renamed = name.map(|n| path.with_file_name(n)).filter(|to| {
                let r = reprisefs::hidden::rename(&path, to).inspect_err(|e| {
                    log::warn!("properties: could not rename {}: {e}", path.display())
                });
                rename_ok = note(r);
                rename_ok
            });
            let failed = failed.map(|e| {
                let name = path.file_name().unwrap_or(path.as_os_str());
                format!("Cannot change {}: {e}", name.to_string_lossy())
            });
            let icon = folder_icon.is_some() || !launcher.is_empty();
            let now = renamed.clone().unwrap_or_else(|| path.clone());
            // A new target or execute bit can change the header icon, Opens with, Description,
            // Target rows and tabs.
            let reread = reread
                .filter(|_| launcher_ok && attrs_ok)
                .and_then(|(name, apps)| reprisefs::props::glance(&now, &name, &apps).ok());
            // The header wears the icon the desktop gives the rewritten file, read here rather
            // than waiting on the desktop's rescan.
            let header = reread
                .as_ref()
                .zip(mime)
                .map(|(_, mime)| crate::desktop::entry_at(&now, &mime, shown));
            let written = dialog::Written {
                path: now,
                attrs: if attrs_ok { attrs } else { Default::default() },
                hidden: hidden.filter(|_| attrs_ok && rename_ok),
                summary: if summary_ok { summary } else { Vec::new() },
                folder_icon: folder_icon.filter(|_| icon_ok),
                launcher: if launcher_ok { launcher } else { Vec::new() },
                reread,
            };
            Box::new(move |state| {
                // The desktop draws the new icon.
                if icon {
                    state.rescan_desktop();
                }
                let ok = failed.is_none();
                state.finish_properties(sheet, path, renamed, &written, header, close && ok);
                if let Some(text) = failed {
                    state.open_dialog_over_id(Dialog::error("Properties", text), sheet);
                }
            })
        });
    }
}

/// The desktop entry written for `app` when it is a program path picked through Browse..., so it can
/// be `mime`'s default; None for an id or when the entry cannot be written.
fn browsed_entry(
    apps: &[reprisefs::apps::AppEntry],
    app: &str,
    mime: &str,
) -> Option<reprisefs::apps::AppEntry> {
    if !app.starts_with('/') || apps.iter().any(|a| a.id == app) {
        return None;
    }
    reprisefs::apps::program_entry(std::path::Path::new(app), Some(mime))
        .map_err(|e| log::warn!("open with: no desktop entry for {app}: {e}"))
        .ok()
}

fn has_entries(path: &std::path::Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut d| d.next().is_some())
}

/// What a set's Properties OK or Apply writes: attributes and Hidden per item, and the icon Change Icon gave its folders.
struct SetWrite {
    paths: Vec<std::path::PathBuf>,
    attrs: reprisefs::props::AttrMoves,
    hidden: Option<bool>,
    folder_icon: Option<Option<String>>,
}

/// What one item's Properties OK or Apply writes: the rename to `name`, and Read-only and Hidden when moved.
struct Props {
    path: std::path::PathBuf,
    name: Option<String>,
    attrs: reprisefs::props::AttrMoves,
    hidden: Option<bool>,
    summary: Vec<(usize, String)>,
    folder_icon: Option<Option<String>>,
    launcher: Vec<(reprisefs::launcher::Key, String)>,
}

impl Props {
    /// Whether the sheet is read again once this lands: launcher keys and the execute bit
    /// decide its rows.
    fn rereads(&self) -> bool {
        !self.launcher.is_empty() || self.attrs.executable.is_some()
    }
}

/// The client area out of the full raster `canvas` (drawn at `scale`) and its logical rect; None without a frame.
fn client_part(canvas: &Canvas, layout: &DialogLayout, scale: i32) -> Option<(Canvas, Rect)> {
    let client = layout.frame.as_ref()?.client;
    let r = client.scaled(scale);
    let mut out = Canvas::new(r.width.max(1) as u32, r.height.max(1) as u32);
    out.blit_canvas(-r.x, -r.y, canvas);
    Some((out, client))
}

#[cfg(test)]
mod tests {
    use super::{
        blur_cues_without_keyboard, client_part, glass_pane, raise_order, refit_origin,
        row_icon_names, to_local,
    };
    use crate::dialog::Dialog;
    use crate::raster::Rect;

    #[test]
    fn an_icon_file_not_decoded_yet_is_left_for_a_job_and_shows_nothing() {
        use super::cached;
        use crate::raster::Canvas;
        use reprisekit::icontheme::{IconCache, IconTheme};
        let mut cache = IconCache::new(IconTheme::load("test", &[]));
        let mut undecoded = Vec::new();
        let file = "/srv/icons/app.png";
        assert_eq!(cached(&mut cache, file, 32, &mut undecoded), None);
        assert_eq!(undecoded, [(file.to_string(), 32)]);
        cache.put_file(file, 32, Some(Canvas::new(32, 32)));
        undecoded.clear();
        let shown = cached(&mut cache, file, 32, &mut undecoded);
        assert_eq!(shown.map(|c| c.width), Some(32));
        assert_eq!(cached(&mut cache, file, 16, &mut undecoded), None);
        assert_eq!(
            undecoded,
            [(file.to_string(), 16)],
            "another size is its own decode"
        );
        cache.put_file(file, 16, None);
        undecoded.clear();
        assert_eq!(cached(&mut cache, file, 16, &mut undecoded), None);
        assert!(
            undecoded.is_empty(),
            "a file that failed to decode is not tried again"
        );
    }

    #[test]
    fn an_icon_file_stuck_decoding_holds_one_job_and_the_others_decode_once_it_is_stuck() {
        use super::{Decode, next_decode};
        use crate::fsjobs::{Reads, STUCK};
        let t0 = std::time::Instant::now();
        let dead = ("/media/stick/app.png".to_string(), 32);
        let queued = ("/srv/icons/queued.png".to_string(), 32);
        let good = ("/srv/icons/new.png".to_string(), 32);
        let mut decodes = Reads::default();
        let both = vec![queued.clone(), dead.clone()];
        let Decode::Start(held, files) = next_decode(&mut decodes, both.clone(), t0) else {
            panic!("the first decode starts");
        };
        assert_eq!(files, [dead.clone(), queued.clone()]);
        assert_eq!(
            next_decode(&mut decodes, both.clone(), t0 + STUCK / 4),
            Decode::Wait,
            "files a young job has yet to decode are left to it"
        );
        assert_eq!(
            next_decode(&mut decodes, vec![good.clone()], t0 + STUCK / 4),
            Decode::Retry(STUCK * 3 / 4),
            "a new file waits while the held decode is young"
        );
        assert_eq!(
            next_decode(&mut decodes, vec![dead.clone()], t0 + STUCK * 5),
            Decode::Wait,
            "the file a stuck job is blocked on is not decoded again"
        );
        let all = vec![dead.clone(), queued.clone(), good.clone()];
        let Decode::Start(read, files) = next_decode(&mut decodes, all, t0 + STUCK) else {
            panic!("the other files decode once the held job is stuck");
        };
        assert_eq!(
            files,
            [good, queued],
            "the dead file stays with the held job"
        );
        assert!(decodes.land(read));
        assert!(!decodes.land(held));
    }

    use reprisekit::controls::form::Key;
    use reprisekit::controls::openwith::{Program, ROWS};

    #[test]
    fn only_the_dialog_with_the_keyboard_keeps_its_underlines() {
        use reprisekit::cues::Cues;
        let mut stack = [Cues::ALL; 3];
        assert!(blur_cues_without_keyboard(stack.iter_mut(), Some(2)));
        assert_eq!(stack.map(|c| c.keys), [false, false, true]);
        assert!(stack[0].focus, "the rectangles stay");
        assert!(
            !blur_cues_without_keyboard(stack.iter_mut(), Some(2)),
            "nothing left to hide"
        );
        assert!(blur_cues_without_keyboard(stack.iter_mut(), None));
        assert_eq!(stack.map(|c| c.keys), [false; 3]);
    }

    #[test]
    fn a_windowed_dialog_stacks_with_the_windows_and_others_stand_over_them() {
        use crate::compositor::{DeskScene, Layers};
        use reprisekit::controls::copyprogress::{CopyProgress, Kind};
        use reprisescene::TreeId;
        let mut scene = DeskScene::new();
        let layers = Layers::new(&mut scene);
        let cp = CopyProgress::new(
            Kind::Copy,
            "Downloads",
            "KINGSTON",
            crate::theme::Color::rgb(0, 0, 0),
        );
        let copy = Dialog::copy_progress(cp, "/media/KINGSTON".into());
        let run = Dialog::run(&[]);
        let put = |scene: &mut DeskScene, layer: TreeId| {
            let tree = scene.tree_create(layer);
            let b = scene.buffer_create(tree);
            scene.buffer_set_canvas(b, &[0xff00_0000; 16 * 16], 16, 16);
            tree
        };
        let dialog = put(&mut scene, super::dialog_layer(&copy, &layers));
        let window = put(&mut scene, layers.windows);
        let top = |scene: &DeskScene| {
            let hit = scene
                .node_at(scene.root(), (8.0, 8.0).into())
                .expect("a hit");
            scene.parent(hit.node).expect("a tree")
        };
        assert_eq!(
            top(&scene),
            window,
            "the window raised last covers the dialog"
        );
        scene.raise_to_top(dialog);
        assert_eq!(top(&scene), dialog, "the dialog comes up over the window");
        scene.raise_to_top(window);
        assert_eq!(top(&scene), window, "a clicked window goes over it again");
        scene.raise_to_top(dialog);
        scene.set_enabled(dialog, false);
        assert_eq!(top(&scene), window, "minimised, it lets the window show");

        let menu = super::dialog_layer(&run, &layers);
        assert_eq!(menu, layers.menu);
        let run_tree = put(&mut scene, menu);
        scene.raise_to_top(window);
        assert_eq!(
            top(&scene),
            run_tree,
            "a dialog that is not windowed stays over the windows"
        );
    }

    #[test]
    fn a_glass_dialogs_backdrop_covers_its_frame_only() {
        use crate::dialog::DialogLayout;
        use reprisekit::interface::Interface;
        use reprisekit::interfaces::theme_for;
        let glass = theme_for(Interface::Glass);
        let text = reprisekit::text::TextRenderer::load(&glass.fonts);
        let run = DialogLayout::compute(&glass, &text, &Dialog::run(&[]));
        let f = run.frame.unwrap();
        let pane = glass_pane(&glass, true, &run).expect("glass under the frame");
        assert_eq!(pane.shapes.len(), 1);
        assert_eq!(pane.shapes[0].rect, Rect::new(0, 0, f.total.0, f.total.1));
        assert_eq!(
            pane.shapes[0].corner,
            reprisekit::glass::CORNER,
            "the rounded corners"
        );
        assert_eq!(pane.holes, vec![f.client], "the client stays clear");
        assert_eq!(f.total, run.size, "the frame is the whole dialog");
        assert!(glass_pane(&glass, false, &run).is_none(), "opaque glass");
        let classic = crate::theme::Theme::default();
        let cl = DialogLayout::compute(&classic, &text, &Dialog::run(&[]));
        assert!(glass_pane(&classic, true, &cl).is_none(), "Classic");
        let welcome = DialogLayout::compute(&classic, &text, &Dialog::log_off());
        assert!(glass_pane(&classic, true, &welcome).is_none(), "Welcome");
        let welcome = DialogLayout::compute(&glass, &text, &Dialog::log_off());
        assert!(
            glass_pane(&glass, true, &welcome).is_some(),
            "a Welcome panel under glass is framed"
        );
    }

    #[test]
    fn a_dialogs_backdrop_content_is_its_client_area() {
        use crate::dialog::DialogLayout;
        use crate::raster::Canvas;
        use reprisekit::interface::Interface;
        use reprisekit::interfaces::theme_for;
        let glass = theme_for(Interface::Glass);
        let text = reprisekit::text::TextRenderer::load(&glass.fonts);
        let run = DialogLayout::compute(&glass, &text, &Dialog::run(&[]));
        let client = run.frame.as_ref().unwrap().client;
        let (w, h) = (run.size.0 as u32 * 2, run.size.1 as u32 * 2);
        let mut canvas = Canvas::new(w, h);
        for (i, p) in canvas.pixels.iter_mut().enumerate() {
            *p = 0xff00_0000 | i as u32;
        }
        let (part, at) = client_part(&canvas, &run, 2).expect("a framed dialog");
        assert_eq!(at, client);
        assert_eq!(
            (part.width, part.height),
            (client.width as u32 * 2, client.height as u32 * 2)
        );
        let (x, y) = (client.x as u32 * 2, client.y as u32 * 2);
        assert_eq!(
            part.pixels[0],
            0xff00_0000 | (y * w + x),
            "the client's top-left"
        );
        let classic = crate::theme::Theme::default();
        let welcome = DialogLayout::compute(&classic, &text, &Dialog::log_off());
        assert!(client_part(&canvas, &welcome, 2).is_none(), "Welcome");
    }

    /// A 404x611 sheet at (900, 469) on 1920x1080 whose usable area shrinks to 1280x692 (150% scale, 28 px
    /// taskbar) moves up and left just enough to fit; a fitting one stays, a Welcome panel is left alone.
    #[test]
    fn a_dialog_left_outside_a_shrunken_output_moves_back_inside() {
        let d = Dialog::run(&[]);
        let area = Rect::new(0, 0, 1280, 692);
        assert_eq!(
            refit_origin(&d, (900, 469), (404, 611), area),
            Some((876, 81))
        );
        assert_eq!(
            refit_origin(&d, (200, 10), (404, 611), area),
            Some((200, 10))
        );
        assert_eq!(
            refit_origin(&d, (900, 469), (404, 611), Rect::default()),
            None
        );
        let welcome = Dialog::log_off();
        assert_eq!(refit_origin(&welcome, (900, 469), (404, 611), area), None);
    }

    #[test]
    fn a_point_left_of_the_layout_origin_maps_to_the_pixel_under_it() {
        // A dialog at x -300 on an output left of the primary: layout -299.5 is inside its first column, -300.5 outside.
        assert_eq!(to_local(-299.5, 10.5, (-300, 10)), (0, 0));
        assert_eq!(to_local(-300.5, 9.5, (-300, 10)), (-1, -1));
        assert_eq!(to_local(12.7, 3.2, (10, 0)), (2, 3));
    }

    #[test]
    fn the_chooser_resolves_icons_for_its_visible_rows_only() {
        let others = (0..40)
            .map(|i| Program {
                id: format!("app{i}.desktop"),
                name: format!("Program {i:02}"),
                icon: Some(format!("icon{i}")),
            })
            .collect();
        let mut d = Dialog::open_with(
            std::path::Path::new("/tmp/a.txt"),
            None,
            vec![],
            others,
            None,
            false,
        );
        let names = row_icon_names(&d);
        assert_eq!(names.len(), ROWS, "one name per visible row");
        assert_eq!(
            names[0],
            (0, "folder".to_string(), 16),
            "the group header draws a folder"
        );
        assert_eq!(names[1], (1, "icon0".to_string(), 16));
        for _ in 0..40 {
            d.key(Key::Down);
        }
        let names = row_icon_names(&d);
        assert_eq!(names.len(), ROWS, "one name per visible row");
        assert_eq!(
            names.first(),
            Some(&(30, "icon29".to_string(), 16)),
            "the list scrolled to its last page"
        );
        assert_eq!(names.last(), Some(&(40, "icon39".to_string(), 16)));
    }

    #[test]
    fn raising_a_child_brings_its_parent_up_under_it() {
        // Run (0), its Browse (1), then a sheet opened on its own (2).
        let links = [(0, None), (1, Some(0)), (2, None)];
        assert_eq!(raise_order(&links, 1), vec![2, 0, 1]);
    }

    #[test]
    fn raising_a_parent_keeps_its_child_on_top() {
        let links = [(0, None), (1, Some(0)), (2, None)];
        assert_eq!(raise_order(&links, 0), vec![2, 0, 1]);
    }

    #[test]
    fn raising_the_top_family_keeps_the_order() {
        let links = [(0, None), (1, None), (2, Some(1)), (3, Some(2))];
        assert_eq!(raise_order(&links, 1), vec![0, 1, 2, 3]);
        assert_eq!(raise_order(&links, 3), vec![0, 1, 2, 3]);
    }

    #[test]
    fn a_child_whose_parent_closed_raises_alone() {
        let links = [(0, None), (5, Some(9)), (2, None)];
        assert_eq!(raise_order(&links, 1), vec![0, 2, 1]);
    }

    #[test]
    fn a_write_of_the_execute_bit_or_launcher_keys_reads_the_sheet_again() {
        let props = |attrs: reprisefs::props::AttrMoves, launcher| super::Props {
            path: "/home/u/Desktop/run.sh".into(),
            name: None,
            attrs,
            hidden: None,
            summary: Vec::new(),
            folder_icon: None,
            launcher,
        };
        let attrs = |read_only, executable| reprisefs::props::AttrMoves {
            read_only,
            executable,
            ..Default::default()
        };
        assert!(props(attrs(None, Some(false)), Vec::new()).rereads());
        assert!(props(attrs(None, Some(true)), Vec::new()).rereads());
        let link = vec![(reprisefs::launcher::Key::Link, "a".to_string())];
        assert!(props(attrs(None, None), link).rereads());
        assert!(
            !props(attrs(Some(true), None), Vec::new()).rereads(),
            "Read-only changes no row"
        );
    }
}
