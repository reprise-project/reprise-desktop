//! The dialogs on the scene (`State::dialogs`, a stack; the top one has the keyboard), placed on the
//! output under the pointer, with key and pointer routing. Model, layout and drawing live in `dialog.rs`.
//!
//! Each dialog is one tree in `layers.menu`: `content` (the client area, the only part the glass
//! backdrop sees), the frame's `Backdrop`, then the buffer. A Welcome panel stands over a still of
//! its output (`output_still`) that fades to grey.

mod browse;
mod complete;
mod draw;
mod input;
mod outcome;
mod power;
mod stack;

#[cfg(test)]
mod tests;

use std::sync::atomic::AtomicU64;

use reprisekit::cues::Cues;
use reprisekit::pointer::DoubleClick;
use reprisescene::{BufferId, TreeId};
use smithay::input::keyboard::{KeysymHandle, ModifiersState};
use smithay::output::Output;
use smithay::reexports::calloop::RegistrationToken;

use crate::compositor::State;
use crate::dialog::{Dialog, DialogHit, DialogLayout, Outcome, Style};
use crate::glass::{Backdrop, frame_pane};
use crate::raster::{Canvas, Rect};
use crate::textedit::EditKey;
use crate::window::keep_inside;

#[derive(Default)]
pub struct Dialogs {
    pub views: Vec<DialogView>,
    /// Boxes raised while a Welcome panel was up, oldest first; they open once no panel is.
    pub held: Vec<Dialog>,
    /// Counts blinks for the top dialog's label tip while the pointer rests on a label.
    tip_timer: Option<RegistrationToken>,
    /// The top dialog has the keyboard until a window takes the focus.
    pub active: bool,
    pub run_history: Vec<String>,
    /// The pinned folders, for the desktop's folder menus.
    pub pins: Option<reprisefs::pins::Pins>,
    /// The file dialog children Browse... started, each over its dialog.
    pub browsing: Vec<browse::Browsing>,
    /// Run boxes whose line is being checked; each takes no second OK meanwhile.
    pub checking: Vec<u64>,
    /// The dialog last seen with the keyboard ([`State::follow_consent_keyboard`]).
    pub keyboard_holder: Option<u64>,
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
    wheel: reprisekit::pointer::WheelCarry,
    /// Icons the last draw used, so a caret blink skips the lookups.
    icons: Option<(IconKey, DialogIcons)>,
    /// Jobs decoding icon files the icons wait for; they are looked up again when one lands.
    decodes: crate::fsjobs::Reads<IconFiles>,
    /// Last full raster without its caret, and its scale: a blink repaints the field over a clone.
    still_canvas: Option<(Canvas, i32)>,
    /// Keyboard cues: a pointer-opened dialog starts without them ([`Cues::key`]), a key-opened one has them.
    pub cues: Cues,
    /// A Welcome panel's still fading in, and when the fade began.
    fade: Option<(BufferId, std::time::Instant)>,
}

impl DialogView {
    pub fn tree(&self) -> TreeId {
        self.tree
    }

    pub fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.layout.size.0, self.layout.size.1)
    }
}

const FADE: std::time::Duration = std::time::Duration::from_millis(1000);

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
        crate::configwrite::save_recycle_bin(&state.config_path, old.policy());
        crate::state::drop_key("bin");
    }
    crate::fsjobs::setup(state);
    state.dialogs.pins = Some(reprisefs::pins::Pins::start(|| {}));
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

/// Where a dialog of `size` at `origin` goes when its output's usable area becomes `area`: moved as
/// little as keeps it inside; `None` for no area or a Welcome panel.
fn refit_origin(
    d: &Dialog,
    origin: (i32, i32),
    size: (i32, i32),
    area: Rect,
) -> Option<(i32, i32)> {
    if area.width <= 0 || matches!(d.style, Style::Welcome(_)) {
        return None;
    }
    Some(keep_inside(origin, size, area))
}

/// Layout point in dialog pixels. Flooring keeps the pixel under the pointer left of or above the layout origin.
fn to_local(lx: f64, ly: f64, origin: (i32, i32)) -> (i32, i32) {
    (lx.floor() as i32 - origin.0, ly.floor() as i32 - origin.1)
}

/// What the dialog has at a layout point (`DialogLayout::hit`). The renderer is needed to measure
/// text for a byte index in an edit field.
pub fn hit(
    v: &DialogView,
    lx: f64,
    ly: f64,
    text: &reprisekit::text::TextRenderer,
    theme: &crate::theme::Theme,
) -> Option<DialogHit> {
    let (x, y) = to_local(lx, ly, (v.x, v.y));
    v.layout.hit(&v.dialog, x, y, text, theme.fonts.size_px)
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
}

struct DialogIcons {
    icon: Option<Canvas>,
    caption: Option<Canvas>,
}

/// Icon `name` at `px` from `cache`. An icon file is never read here: one not decoded at `px` yet
/// goes on `undecoded` for a job and shows nothing meanwhile. A theme icon still decoding shows
/// nothing until it lands ([`crate::icon::IconWaiter::Dialog`]).
fn cached(
    cache: &mut reprisekit::icontheme::IconCache,
    name: &str,
    px: i32,
    undecoded: &mut Vec<(String, i32)>,
) -> Option<Canvas> {
    if !name.starts_with('/') {
        return cache
            .get_unread(name, px)
            .flatten()
            .map(std::sync::Arc::unwrap_or_clone);
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
