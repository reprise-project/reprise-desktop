//! The dialogs on the scene (`State::dialogs`, a stack; the top one has the keyboard), placed on the
//! output under the pointer, with key and pointer routing. Model, layout and drawing live in `dialog.rs`.
//!
//! Each dialog is one tree in `layers.menu` (`layers.windows` for a windowed one): `content` (the
//! client area, the only part the glass backdrop sees), the frame's `Backdrop`, then the buffer. A
//! Welcome panel stands over a still of its output (`output_still`) that fades to grey.

mod browse;
mod draw;
mod input;
mod outcome;
mod properties;
mod stack;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;

use reprisekit::controls::filedlg;
use reprisekit::controls::openwith;
use reprisekit::cues::Cues;
use reprisekit::pointer::DoubleClick;
use reprisescene::{BufferId, TreeId};
use smithay::input::keyboard::{KeysymHandle, ModifiersState};
use smithay::output::Output;
use smithay::reexports::calloop::RegistrationToken;

use crate::compositor::State;
use crate::dialog::{self, Dialog, DialogHit, DialogLayout, Outcome, Style};
use crate::glass::{Backdrop, frame_pane};
use crate::raster::{Canvas, Rect};
use crate::textedit::EditKey;
use crate::window::keep_inside;
use reprisekit::frame::Edges;

#[derive(Default)]
pub struct Dialogs {
    pub views: Vec<DialogView>,
    /// Counts blinks for the top dialog's label tip while the pointer rests on a label.
    tip_timer: Option<RegistrationToken>,
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

/// A reader's Focus at dialog point `at`: the control there takes the keyboard and the focus
/// rectangles come out, as a key brings them. Whether the dialog needs a redraw.
pub(crate) fn reader_focus(
    dialog: &mut Dialog,
    cues: &mut Cues,
    layout: &DialogLayout,
    at: (i32, i32),
    text: &reprisekit::text::TextRenderer,
    px: u32,
) -> bool {
    let cued = cues.key(false);
    dialog.focus_at(layout, at.0, at.1, text, px) == Outcome::Redraw || cued
}

/// The stack order after raising dialog `index`: its family (parent chain up to a root dialog, and
/// everything opened over any of them) moves to the top in current order. `links` is `(id, parent)`, bottom first.
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
