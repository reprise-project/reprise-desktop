//! Drag-and-drop of desktop icons and Start menu programs: a see-through copy of the dragged icons rides
//! under the pointer and the release drops them on a folder icon, a desktop cell, the Recycle Bin or Quick
//! Launch. The left button does the natural move, copy or shortcut (or what the held modifiers pick); the
//! right button asks with Copy Here / Move Here / Create Shortcuts Here.
//!
//! Past the drag threshold the drag becomes the seat's (`dndsource`) so it can drop into another program;
//! drops on the desktop's own targets still end in [`State::perform_all`] or the drop menu. A system icon's
//! drag stays on the desktop and ends in [`State::drag_finish`].
//!
//! Files and folders go to Explorer (`filejobs`), which asks about a taken name, shows the progress and says
//! why a drop failed. The launchers a drop makes of what is no file (a Start menu program's copy, a shortcut to
//! a system icon) are written here.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use reprisefs::apps::AppEntry;
use reprisefs::undo::SharedUndo;
use reprisekit::taskbar::{TaskbarHit, TaskbarLayout};
use reprisescene::{BufferId, Input, TreeId};
use smithay::output::Output;
use smithay::utils::{Logical, Point};

use crate::{
    compositor::State,
    desktop::{self, Target},
    dnddrop::Answer,
    fsjobs::Action,
    input::{BTN_LEFT, BTN_RIGHT, Hit, Interaction},
    menuview::MenuKind,
    output::upload_scaled,
    raster::Rect,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DragSource {
    /// A desktop icon: which one, its file, and the shortcut base name (`DesktopEntry::shortcut_base`).
    Icon {
        id: desktop::IconId,
        path: PathBuf,
        name: String,
    },
    App {
        entry: AppEntry,
        file: PathBuf,
    },
    File {
        path: PathBuf,
        name: String,
    },
}

impl DragSource {
    pub fn file(path: PathBuf) -> DragSource {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        DragSource::File { path, name }
    }

    pub fn path(&self) -> &Path {
        match self {
            DragSource::Icon { path, .. } | DragSource::File { path, .. } => path,
            DragSource::App { file, .. } => file,
        }
    }

    pub(crate) fn name(&self) -> &str {
        match self {
            DragSource::Icon { name, .. } | DragSource::File { name, .. } => name,
            DragSource::App { entry, .. } => &entry.name,
        }
    }

    /// A system icon (the Recycle Bin, a place): moves about the desktop or becomes a shortcut, never copied.
    pub fn is_system(&self) -> bool {
        matches!(
            self,
            DragSource::Icon {
                id: desktop::IconId::System(_),
                ..
            }
        )
    }
}

/// What a drop on the Recycle Bin deletes: a dragged icon its selection, as Delete would; another program's files their paths.
#[derive(Clone, Debug, PartialEq, Eq)]
enum BinDrop {
    Icons(desktop::IconId),
    Files(Vec<PathBuf>),
}

/// The Recycle Bin's part of a drop of `sources`, the grabbed one first. A Start menu program is never deleted.
fn bin_drop(sources: &[DragSource]) -> Option<BinDrop> {
    match sources.first()? {
        DragSource::Icon { id, .. } => Some(BinDrop::Icons(id.clone())),
        DragSource::File { .. } => Some(BinDrop::Files(
            sources
                .iter()
                .filter(|s| matches!(s, DragSource::File { .. }))
                .map(|s| s.path().to_path_buf())
                .collect(),
        )),
        DragSource::App { .. } => None,
    }
}

/// A shortcut to a system icon: its icon and the command opening its place (`desktop::place_command`; None when not UTF-8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlaceLink {
    icon: String,
    command: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropTarget {
    /// Quick Launch gap `slot` (see `TaskbarLayout::drop_slot`).
    Quick(usize),
    /// Gap `gap` on bar `bar`'s task row (`taskbar::pin_drop_gap`), where `[taskbar] pinned` puts the
    /// Quick Launch programs.
    Pin {
        bar: TreeId,
        gap: usize,
    },
    Folder(PathBuf),
    /// A desktop cell, as a 100% cell (`desktop::to_saved`): a drop's file work answers later,
    /// maybe after the text size has grown the grid (`State::desktop_drop_at`).
    Desktop((i32, i32)),
    Trash,
}

pub use reprisefs::ops::DropOp;

/// Which drop menu rows can be picked: another program's drag may allow only a copy or only a move. A
/// shortcut leaves the file in place, so it goes with the copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allowed {
    pub copy: bool,
    pub moves: bool,
}

impl Allowed {
    pub const ALL: Allowed = Allowed {
        copy: true,
        moves: true,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramPress {
    Menu { depth: usize, row: usize },
    StartPanel(reprisekit::startpanel::Spot),
}

impl ProgramPress {
    /// Whether the row is still on screen with `menus` menus open and the
    /// Glass Start menu open or not (`start_panel`).
    fn shown(self, menus: usize, start_panel: bool) -> bool {
        match self {
            ProgramPress::Menu { depth, .. } => menus > depth,
            ProgramPress::StartPanel(_) => start_panel,
        }
    }
}

pub struct DragState {
    /// What was grabbed first, then the rest of the desktop selection (see `State::drag_sources`).
    pub sources: Vec<DragSource>,
    pub button: u32,
    image: Option<BufferId>,
    offset: (i32, i32),
    /// Where the image was grabbed and the scale it is drawn at: crossing to an output of another
    /// scale draws it again from the same grab.
    grab: Point<f64, Logical>,
    image_scale: i32,
    /// Armed on a program row: the row to activate if the button comes up without a drag.
    row: Option<ProgramPress>,
    pub seat: bool,
    /// Where a desktop icon drag last put its image while water is on: its wake runs from here, the drop splashes here.
    pub water_last: Option<Point<f64, Logical>>,
}

#[derive(Default)]
pub struct Drags {
    pub armed: Option<DragState>,
    /// Reprise's own target under the seat's drag of the desktop's icons, and its answer (`dndsource`).
    pub over: Option<(DropTarget, Answer)>,
    pub pending_drop: Option<(Vec<DragSource>, DropTarget)>,
}

/// What a left drop of `grabbed` on `target` does with no modifier: move an icon about the desktop or
/// into a folder; a menu program is a copy of its .desktop file; anything else is a shortcut.
pub fn left_drop_op(grabbed: &DragSource, target: &DropTarget) -> DropOp {
    match (grabbed, target) {
        (
            DragSource::Icon { .. },
            DropTarget::Desktop(_) | DropTarget::Folder(_) | DropTarget::Trash,
        ) => DropOp::Move,
        (DragSource::App { .. }, _) => DropOp::Copy,
        _ => DropOp::Shortcut,
    }
}

/// The scale to draw a drag image at again, drawn at `drawn` and now over an output drawn at
/// `here`; None keeps it (same scale, or off every output).
fn redraw_scale(drawn: i32, here: Option<i32>) -> Option<i32> {
    here.filter(|&s| s != drawn)
}

/// Whether a drop of `sources` may land on a desktop cell. With `show_files` false only system icons
/// move about it (a file would go into ~/Desktop and vanish); another program's drag passes no sources, so it is refused.
fn takes_cell(show_files: bool, sources: &[DragSource]) -> bool {
    show_files || (!sources.is_empty() && sources.iter().all(DragSource::is_system))
}

/// The `files` that sit directly in `dir`, through links alike. Resolves each file's folder, so a
/// host runs it off its event loop.
pub(crate) fn in_folder(files: Vec<PathBuf>, dir: &Path) -> HashSet<PathBuf> {
    files
        .into_iter()
        .filter(|p| reprisefs::ops::same_folder(p, dir))
        .collect()
}

fn desktop_entry_named(entries: &[desktop::DesktopEntry], name: &str) -> Option<usize> {
    entries
        .iter()
        .position(|e| e.kind == desktop::Kind::File && e.name == name)
}

/// One source of a batch: its desktop cell as a 100% cell (`desktop::to_saved`; None in a folder
/// or with auto-arrange) and the place a shortcut to a system icon opens.
pub(crate) type Put = (DragSource, Option<(i32, i32)>, Option<PlaceLink>);

pub(crate) type Failure = (String, String);

/// The launcher text of a shortcut named `name` to `source`; a shortcut to a system icon opens
/// `link`'s place.
pub(crate) fn shortcut_text(
    source: &DragSource,
    link: Option<&PlaceLink>,
    name: &str,
) -> Option<String> {
    {
        let (icon, command) = match (source, link) {
            (_, Some(link)) => (Some(link.icon.clone()), link.command.clone()),
            (DragSource::App { entry, .. }, None) => {
                let argv = reprisefs::apps::exec_argv(&entry.exec_line);
                (
                    entry.icon.clone(),
                    Some(reprisefs::apps::command_line(&argv)),
                )
            }
            (DragSource::Icon { path, .. } | DragSource::File { path, .. }, None) => (
                Some(if path.is_dir() {
                    "folder".to_string()
                } else {
                    reprisefs::icons::icon_for_file(path).to_string()
                }),
                reprisefs::apps::open_command_for(path),
            ),
        };
        reprisefs::apps::launcher_text(
            name,
            icon.as_deref(),
            reprisefs::apps::Launch::Exec(command.as_deref()?),
        )
    }
}

/// Copy a Start menu program's file into `dir`, or make a shortcut to `source` there, named past
/// `existing` and numbered past a name `dir` holds. A program's file is copied whatever `op` says,
/// since it belongs to the system.
fn put_in(
    source: &DragSource,
    dir: &Path,
    op: DropOp,
    existing: &[String],
    link: Option<&PlaceLink>,
) -> Result<String, reprisefs::ops::PutError> {
    let is_app = matches!(source, DragSource::App { .. });
    let shortcut = |name: &str| shortcut_text(source, link, name);
    reprisefs::ops::put_in(
        source.path(),
        source.name(),
        is_app,
        dir,
        op,
        existing,
        &shortcut,
    )
    .map(|landed| landed.to_string_lossy().into_owned())
}

/// Write link file `text` as `name.desktop` in `dir`, numbered past a taken name, and mark it
/// executable and trusted. Where that cannot be set (no `user.` xattrs) the file stays as a plain
/// `.desktop` file, which runs nothing. Returns its path.
fn write_link(dir: &Path, name: &str, text: &str) -> std::io::Result<PathBuf> {
    use std::io::Write;
    let (mut file, dest) = reprisefs::ops::create_new_numbered(dir, &format!("{name}.desktop"))?;
    if let Err(e) = file.write_all(text.as_bytes()) {
        let _ = std::fs::remove_file(&dest);
        return Err(e);
    }
    drop(file);
    if let Err(e) = reprisefs::apps::allow_launching(&dest) {
        log::info!("{}: kept as a plain file: {e}", dest.display());
    }
    Ok(dest)
}

impl State {
    /// Arm a drag of `sources` (the grabbed one first) with `button` held; nothing shows until the pointer travels `desktop::DRAG_PX`.
    pub fn drag_arm(&mut self, sources: Vec<DragSource>, button: u32) {
        self.arm_drag(sources, button, None);
    }

    /// A press on row `row` of the menu at `depth` with `button`: a program row arms a drag and is
    /// picked on release if the pointer stays put; any other row acts now, left button only.
    pub fn menu_row_press(&mut self, depth: usize, row: Option<usize>, button: u32) {
        if button == BTN_RIGHT
            && let Some(row) = row
            && let Some(MenuKind::QuickOverflow { files, .. }) =
                self.menus.get(depth).map(|m| &m.kind)
            && let Some(index) = files
                .get(row)
                .and_then(|f| self.quicklaunch.iter().position(|q| &q.file == f))
        {
            let p = self.pointer_location();
            self.open_quick_menu(index, (p.x as i32, p.y as i32));
            return;
        }
        if button == BTN_RIGHT
            && let Some(row) = row
            && let Some(MenuKind::JumpList { rows, target, .. }) =
                self.menus.get(depth).map(|m| &m.kind)
            && let Some(Some(crate::taskbar::JumpRow::Recent(path))) = rows.get(row)
            && let Some(e) = &target.entry
        {
            let (app, path) = (e.id.as_str().to_string(), path.clone());
            self.open_jump_file_menu(depth, &app, &path);
            return;
        }
        if button == BTN_RIGHT
            && let Some(row) = row
            && let Some(MenuKind::Programs(apps)) = self.menus.get(depth).map(|m| &m.kind)
            && let Some(app) = apps
                .get(row)
                .and_then(|a| self.app_index.iter().position(|e| e.id == a.id))
        {
            self.open_list_program_menu(app);
            return;
        }
        let Some(row) = row.filter(|_| button == BTN_LEFT) else {
            return;
        };
        let app = match self.menus.get(depth).map(|m| &m.kind) {
            Some(MenuKind::Programs(apps)) => apps.get(row).cloned(),
            _ => None,
        };
        let file = app.as_ref().and_then(|a| reprisefs::apps::find_file(&a.id));
        match (app, file) {
            (Some(entry), Some(file)) => self.arm_drag(
                vec![DragSource::App { entry, file }],
                BTN_LEFT,
                Some(ProgramPress::Menu { depth, row }),
            ),
            _ => self.menu_activate(depth, row),
        }
    }

    /// A left press on a Glass Start menu program row (`spot`): arm a drag of `entry` (file `file`); release runs it if the pointer stayed put.
    pub fn start_panel_row_press(
        &mut self,
        spot: reprisekit::startpanel::Spot,
        entry: AppEntry,
        file: PathBuf,
    ) {
        self.arm_drag(
            vec![DragSource::App { entry, file }],
            BTN_LEFT,
            Some(ProgramPress::StartPanel(spot)),
        );
    }

    fn arm_drag(&mut self, sources: Vec<DragSource>, button: u32, row: Option<ProgramPress>) {
        if sources.is_empty() {
            return;
        }
        let pos = self.pointer_location();
        self.drags.armed = Some(DragState {
            sources,
            button,
            image: None,
            offset: (0, 0),
            grab: pos,
            image_scale: 1,
            row,
            seat: false,
            water_last: None,
        });
        self.interaction = Interaction::Drag {
            x0: pos.x,
            y0: pos.y,
            started: false,
        };
    }

    /// The pointer moved to `pos` with a drag armed: start past `desktop::DRAG_PX`, then keep the image under it.
    pub fn drag_view_motion(&mut self, pos: Point<f64, Logical>) {
        let Interaction::Drag { x0, y0, started } = self.interaction else {
            return;
        };
        if !started {
            let slop = f64::from(desktop::DRAG_PX);
            if (pos.x - x0).abs() < slop && (pos.y - y0).abs() < slop {
                return;
            }
            // A row whose menu shut under the held button (a key, another
            // press) has nothing left to drag.
            let row = self.drags.armed.as_ref().and_then(|d| d.row);
            if row.is_some_and(|r| !r.shown(self.menus.len(), self.start_panel.is_some())) {
                self.cancel_interaction();
                return;
            }
            self.interaction = Interaction::Drag {
                x0,
                y0,
                started: true,
            };
            // A program leaves its menu behind: the menu closes and only the drag image follows.
            match self.drags.armed.as_ref().and_then(|d| d.row) {
                Some(ProgramPress::Menu { .. }) => self.close_menu(),
                Some(ProgramPress::StartPanel(_)) => self.close_start_panel(),
                None => {}
            }
            self.start_drag_image(pos);
            // Past the threshold the drag becomes the seat's, so it can leave the desktop.
            if self.hand_over() {
                return;
            }
        }
        self.place_drag_image(pos);
        let sources = self.drags.armed.as_ref().map(|d| d.sources.clone());
        if let Some(sources) = sources {
            let target = self.drop_target_at(&sources, pos);
            self.drop_hilite(target.as_ref());
        }
    }

    pub fn place_drag_image(&mut self, pos: Point<f64, Logical>) {
        let here = self
            .output_at(pos.x, pos.y)
            .map(|o| self.draw_scale_at(Some(&o)));
        let redraw = self.drags.armed.as_ref().and_then(|d| {
            d.image?;
            redraw_scale(d.image_scale, here).map(|scale| (d.grab, scale))
        });
        let mut was = Vec::new();
        if let Some((grab, scale)) = redraw {
            if let Some(image) = self.drags.armed.as_mut().and_then(|d| d.image.take()) {
                was = self.scene.outputs_of(image);
                self.scene.destroy(image);
            }
            self.draw_drag_image(grab, scale);
        }
        let Some((image, (ox, oy))) = self
            .drags
            .armed
            .as_ref()
            .and_then(|d| Some((d.image?, d.offset)))
        else {
            crate::backend::schedule_on(self, &was, crate::backend::Ask::Content);
            return;
        };
        let at = (pos.x as i32 - ox, pos.y as i32 - oy);
        self.show_drag_image(image, at, was);
        if !self.ripples_on() {
            return;
        }
        let last = self
            .drags
            .armed
            .as_mut()
            .filter(|d| {
                d.sources
                    .iter()
                    .any(|s| matches!(s, DragSource::Icon { .. }))
            })
            .map(|d| d.water_last.replace(pos));
        if let Some(last) = last {
            self.water_icon_drag(last, pos);
        }
    }

    /// Take drag `d`'s image off the screen. A placed icon drag splashes where its image last lay, if on the desktop, and returns the water its icons.
    pub fn clear_drag_image(&mut self, d: &mut DragState) {
        if let Some(image) = d.image.take() {
            self.destroy_redrawn(image);
        }
        if let Some(at) = d.water_last.take() {
            self.water_icon_drop(at);
            self.water_obstacles();
        }
    }

    /// Draw the drag image at half strength, grabbed where the pointer sat (`pos`): the selected desktop
    /// cells as they lie, or a menu program's cell by its middle. A set bigger than
    /// `desktop_drag_window` is cut to it and fades toward its edges.
    fn start_drag_image(&mut self, pos: Point<f64, Logical>) {
        let scale = self
            .draw_scale_at(self.output_at(pos.x, pos.y).as_ref())
            .max(1);
        if let Some(d) = self.drags.armed.as_mut() {
            d.grab = pos;
        }
        self.draw_drag_image(pos, scale);
        self.place_drag_image(pos);
    }

    /// Icons the drag image showed a stand-in for landed: it draws again where it lies.
    pub(crate) fn drag_icons_landed(&mut self) {
        let Some(d) = self.drags.armed.as_mut() else {
            return;
        };
        let (Some(image), grab, scale) = (d.image.take(), d.grab, d.image_scale) else {
            return;
        };
        let at = self.scene.position(image);
        let was = self.scene.outputs_of(image);
        self.scene.destroy(image);
        self.draw_drag_image(grab, scale);
        match self.drags.armed.as_ref().and_then(|d| d.image).zip(at) {
            Some((image, at)) => self.show_drag_image(image, (at.x, at.y), was),
            None => crate::backend::schedule_on(self, &was, crate::backend::Ask::Content),
        }
    }

    /// Show drag image `image` at `at`, asking for frames on `was` (where an image it replaces
    /// showed), the outputs it leaves and those it reaches. A picture over glass changes nothing
    /// the glass blurs.
    fn show_drag_image(&mut self, image: BufferId, at: (i32, i32), mut was: Vec<Output>) {
        if self.scene.enabled(image) && self.scene.position(image) == Some(at.into()) {
            crate::backend::schedule_on(self, &was, crate::backend::Ask::Content);
            return;
        }
        let before = self.scene.outputs_of(image);
        self.scene.set_position(image, at.0, at.1);
        self.scene.set_enabled(image, true);
        for o in before.into_iter().chain(self.scene.outputs_of(image)) {
            if !was.contains(&o) {
                was.push(o);
            }
        }
        crate::backend::schedule_on(self, &was, crate::backend::Ask::Content);
    }

    /// Draw the image grabbed at `pos` at `scale` into a fresh buffer.
    fn draw_drag_image(&mut self, pos: Point<f64, Logical>, scale: i32) {
        let waits = self.icons.waits();
        self.draw_drag_image_at(pos, scale);
        self.note_icon_waits(waits, crate::icon::IconWaiter::Drag);
    }

    fn draw_drag_image_at(&mut self, pos: Point<f64, Logical>, scale: i32) {
        let theme = self.config.theme.scaled(scale);
        let (x, y) = (pos.x as i32, pos.y as i32);
        let Some(d) = self.drags.armed.as_ref() else {
            return;
        };
        let view = self.desktop.as_ref();
        // Every cell to draw: name, icon, shortcut arrow and offset from the image's top-left (layout px).
        let mut cells: Vec<(String, String, bool, Rect)> = Vec::new();
        let grid = self.desktop_grid();
        let mut offset = (grid.w / 2, grid.h / 2);
        let around = Rect::new(x - grid.w / 2, y - grid.h / 2, grid.w, grid.h);
        for source in &d.sources {
            match source {
                DragSource::Icon { id, name, .. } => {
                    let index = view.and_then(|v| desktop::find(&v.entries, id));
                    let entry = index.and_then(|i| view?.entries.get(i));
                    let icon = entry
                        .map(|e| e.icon.clone())
                        .unwrap_or_else(|| "text-x-generic".into());
                    let shortcut = entry.is_some_and(|e| e.shortcut);
                    let cell = view
                        .zip(index)
                        .and_then(|(v, i)| v.cells.get(i).copied())
                        .unwrap_or(around);
                    let label = entry
                        .map(|e| e.label.clone())
                        .unwrap_or_else(|| name.clone());
                    cells.push((label, icon, shortcut, cell));
                }
                DragSource::App { entry, .. } => cells.push((
                    entry.name.clone(),
                    entry
                        .icon
                        .clone()
                        .unwrap_or_else(|| "application-x-executable".into()),
                    true,
                    around,
                )),
                DragSource::File { .. } => {}
            }
        }
        let Some(first) = cells.first().map(|c| c.3) else {
            return;
        };
        let bounds = cells.iter().skip(1).fold(first, |b, c| b.union(&c.3));
        // Desktop icons show only round the pointer: a big set is cut to a square there.
        let icons = matches!(d.sources.first(), Some(DragSource::Icon { .. }));
        let shown = if icons {
            desktop::desktop_drag_window(bounds, (x, y))
        } else {
            bounds
        };
        if icons {
            offset = (x - shown.x, y - shown.y);
        }
        let mut canvas = reprisekit::raster::Canvas::new(
            (shown.width * scale) as u32,
            (shown.height * scale) as u32,
        );
        let in_view = cells.iter().filter(|c| !c.3.intersect(&shown).is_empty());
        for (name, icon_name, shortcut, cell) in in_view {
            let px = desktop::ICON_PX * scale;
            // A desktop icon's icon file is the desktop read's to decode; the drag shows what it
            // decoded.
            let icon = if icons && icon_name.starts_with('/') {
                self.icons.file_put(icon_name, px).flatten()
            } else {
                crate::icon::unread(&mut self.icons, icon_name, px)
            };
            // A desktop icon drags as it shows, selected; a menu program has no selection.
            let selected = icons.then_some(desktop::Selection {
                ring: false,
                origin: (cell.x, cell.y),
            });
            let one = desktop::draw_cell(
                &theme,
                self.text.for_output(scale),
                name,
                icon.as_deref(),
                *shortcut,
                selected,
                None,
                None,
            );
            canvas.blit_canvas((cell.x - shown.x) * scale, (cell.y - shown.y) * scale, &one);
        }
        let clipped = shown != bounds;
        let size = (shown.width, shown.height);
        canvas.fade_by(|px, py| {
            desktop::desktop_drag_strength((px / scale, py / scale), offset, size, clipped)
        });
        let image = self.scene.buffer_create(self.layers.overlay);
        // Off until `show_drag_image` places it, so its first ask is only where it lands.
        self.scene.set_enabled(image, false);
        self.scene.set_input(image, Input::Passthrough);
        upload_scaled(&mut self.scene, image, &canvas, scale);
        if let Some(d) = self.drags.armed.as_mut() {
            d.image = Some(image);
            d.offset = offset;
            d.image_scale = scale;
        }
    }

    /// End the desktop's own drag and take its state, the image gone: whether it had started, and what it carried. None when none was armed.
    fn take_drag(&mut self) -> Option<(bool, DragState)> {
        let started = matches!(self.interaction, Interaction::Drag { started: true, .. });
        self.interaction = Interaction::Idle;
        let mut d = self.drags.armed.take()?;
        self.drop_hilite(None);
        self.clear_drag_image(&mut d);
        Some((started, d))
    }

    /// Something took the pointer away from the desktop's drag (`cancel_interaction`, the lock): it ends, dropping nothing.
    pub fn drag_view_cancel(&mut self) {
        if self.take_drag().is_some() {
            self.settle_click(true);
        }
    }

    /// The button came up on the desktop's drag: drop, or if it never started, do what the press alone
    /// means (a right press on an icon opens its menu, a program row picks).
    pub fn drag_finish(&mut self) {
        let Some((started, d)) = self.take_drag() else {
            return;
        };
        let pos = self.pointer_location();
        let at = (pos.x as i32, pos.y as i32);
        let Some(grabbed) = d.sources.first().cloned() else {
            return;
        };
        self.settle_click(started);
        if !started {
            if let (BTN_RIGHT, DragSource::Icon { id, .. }) = (d.button, &grabbed) {
                let index = self
                    .desktop
                    .as_ref()
                    .and_then(|v| desktop::find(&v.entries, id));
                if let Some(index) = index {
                    self.open_icon_menu(index, at);
                }
            } else {
                match d.row {
                    Some(ProgramPress::Menu { depth, row }) if self.menus.len() > depth => {
                        self.menu_activate(depth, row);
                    }
                    Some(ProgramPress::StartPanel(spot)) => self.start_panel_click(spot),
                    _ => {}
                }
            }
            return;
        }
        let Some(target) = self.drop_target_at(&d.sources, pos) else {
            return;
        };
        if d.button == BTN_RIGHT {
            self.cancel_waiting_drop();
            self.drags.pending_drop = Some((d.sources, target));
            self.open_drop_menu(at, DropOp::Move, Allowed::ALL);
            return;
        }
        let op = left_drop_op(&grabbed, &target);
        self.perform_all(&d.sources, &target, op);
    }

    /// Draw the folder icon or Recycle Bin a drag is over as selected; None or another target clears it.
    pub fn drop_hilite(&mut self, target: Option<&DropTarget>) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let index = match target {
            Some(DropTarget::Folder(dir)) => view
                .entries
                .iter()
                .position(|e| matches!(&e.target, Target::Open(p) if p == dir)),
            Some(DropTarget::Trash) => view
                .entries
                .iter()
                .position(|e| e.kind == desktop::Kind::RecycleBin),
            _ => None,
        };
        if view.drop_hilite != index {
            view.drop_hilite = index;
            self.relayout_desktop();
        }
    }

    /// What sits under layout point `pos` for a drop of `sources`: the Quick Launch band, the task row
    /// for launchers and programs while it holds the pins (see [`pins_on_row`]), a folder icon (not a
    /// dragged one), or a desktop cell (see [`takes_cell`]); None otherwise. Another program's drag
    /// passes no sources.
    pub fn drop_target_at(
        &self,
        sources: &[DragSource],
        pos: Point<f64, Logical>,
    ) -> Option<DropTarget> {
        let dragged = |e: &desktop::DesktopEntry| {
            let id = e.id();
            sources
                .iter()
                .any(|s| matches!(s, DragSource::Icon { id: dragged, .. } if *dragged == id))
        };
        match self.scene_hit(pos) {
            Hit::Taskbar { bar, hit } => {
                let b = self.bar(bar)?;
                let (lx, ly) = self.bar_local(b, pos.x, pos.y);
                if let Some(slot) = quick_drop(&b.layout, hit, lx, ly) {
                    return Some(DropTarget::Quick(slot));
                }
                let entries = self.desktop.as_ref().map_or(&[][..], |v| &v.entries[..]);
                if !self.config.taskbar.pinned
                    || sources.is_empty()
                    || !sources.iter().all(|s| pins_on_row(s, entries))
                {
                    return None;
                }
                let at = row_drop(&b.layout, hit, lx, ly)?;
                let tasks = b.layout.along_rects(&b.layout.tasks);
                let gap = crate::taskbar::pin_drop_gap(&tasks, &b.slots, at);
                Some(DropTarget::Pin { bar, gap })
            }
            Hit::Desktop { index } => {
                let view = self.desktop.as_ref()?;
                // A system icon only moves about the desktop, and takes whatever is dragged with it.
                let system = sources.iter().any(DragSource::is_system);
                if let Some(entry) = index.and_then(|i| view.entries.get(i))
                    && !dragged(entry)
                    && !system
                {
                    // A Start menu program is never trashed; files are.
                    if entry.kind == desktop::Kind::RecycleBin {
                        return (!matches!(sources.first(), Some(DragSource::App { .. })))
                            .then_some(DropTarget::Trash);
                    }
                    if let Target::Open(p) = &entry.target
                        && entry.is_dir
                    {
                        return Some(DropTarget::Folder(p.clone()));
                    }
                }
                if !takes_cell(self.config.desktop_icons, sources) {
                    return None;
                }
                Some(self.desktop_drop_at(pos.x as i32, pos.y as i32))
            }
            _ => None,
        }
    }

    /// Do the drop of `sources` (the grabbed one first): a drag's, or a Paste's, Send To's or Create
    /// Shortcut's, its file work in Explorer ([`Self::put_into`]). On a desktop cell the grabbed icon
    /// lands there and the others keep their grid offsets, falling to the next free cell if taken.
    pub fn perform_all(&mut self, sources: &[DragSource], target: &DropTarget, op: DropOp) {
        if self.desktop_locked() {
            log::info!("desktop: a drop under the lock is refused");
            return;
        }
        let files: Vec<PathBuf> = sources
            .iter()
            .filter_map(|s| match s {
                DragSource::File { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        match (target, self.desktop.as_ref()) {
            (DropTarget::Desktop(_), Some(view)) if !files.is_empty() => {
                // Whether a file sits in the desktop folder resolves links, which a file on a
                // mount that never answers holds.
                let dir = view.dir.clone();
                let (sources, target) = (sources.to_vec(), target.clone());
                self.spawn_fs_action(Action::reads(op.failure_title()), move |_| {
                    let here = in_folder(files, &dir);
                    Box::new(move |state: &mut State| state.put_all(&sources, &target, op, &here))
                });
            }
            _ => self.put_all(sources, target, op, &HashSet::new()),
        }
    }

    /// [`Self::perform_all`] once `here` holds the files of `sources` that sit in the desktop
    /// folder.
    pub(crate) fn put_all(
        &mut self,
        sources: &[DragSource],
        target: &DropTarget,
        op: DropOp,
        here: &HashSet<PathBuf>,
    ) {
        if self.desktop_locked() {
            log::info!("desktop: a drop under the lock is refused");
            return;
        }
        match target {
            DropTarget::Quick(slot) => {
                let srcs = sources.iter().map(|s| s.path().to_path_buf()).collect();
                self.quick_add(srcs, Some(*slot), Vec::new());
            }
            DropTarget::Pin { bar, gap } => {
                let srcs: Vec<PathBuf> = sources.iter().map(|s| s.path().to_path_buf()).collect();
                let (slot, anchored) = self.row_insert(*bar, *gap, srcs.len());
                self.quick_add(srcs, Some(slot), anchored);
            }
            // The same questions as Delete: Confirm File Delete, and whether to take a place off the desktop.
            DropTarget::Trash => match bin_drop(sources) {
                Some(BinDrop::Icons(id)) => {
                    let index = self
                        .desktop
                        .as_ref()
                        .and_then(|v| desktop::find(&v.entries, &id));
                    if let Some(index) = index {
                        self.trash_selection(index);
                    }
                }
                Some(BinDrop::Files(paths)) => self.trash_files(paths),
                None => {}
            },
            DropTarget::Folder(dir) => {
                let puts = sources.iter().map(|s| (s.clone(), None, None)).collect();
                self.put_into(dir.clone(), op, puts, Vec::new(), here);
            }
            DropTarget::Desktop(saved) => {
                let area = self.desktop_area();
                let Some(view) = self.desktop.as_ref() else {
                    return;
                };
                let dir = view.dir.clone();
                let existing: Vec<String> = view.entries.iter().map(|e| e.name.clone()).collect();
                let auto = view.auto_arrange;
                let grid = self.desktop_grid();
                let cell = desktop::from_saved(grid, *saved);
                let grid_of = |r: &Rect| desktop::cell_at(grid, area, r.x, r.y);
                let index_of = |s: &DragSource| match s {
                    DragSource::Icon { id, .. } => desktop::find(&view.entries, id),
                    // Another program's file already on the desktop is one of its icons, found by name.
                    DragSource::File { path, name } if here.contains(path) => {
                        desktop_entry_named(&view.entries, name)
                    }
                    DragSource::App { .. } | DragSource::File { .. } => None,
                };
                // A moved icon frees its cell; a copy or shortcut keeps the original.
                let moving = matches!(op, DropOp::Move);
                let moved: std::collections::HashSet<usize> = if moving {
                    sources.iter().filter_map(index_of).collect()
                } else {
                    Default::default()
                };
                let mut taken: std::collections::HashSet<(i32, i32)> = view
                    .cells
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !moved.contains(i))
                    .map(|(_, r)| grid_of(r))
                    .collect();
                taken.extend(self.icon_blocked());
                let grabbed_grid = sources
                    .first()
                    .and_then(index_of)
                    .and_then(|i| view.cells.get(i))
                    .map(grid_of);
                // Cells are settled here on the loop; the file work runs on a thread and the names come back.
                let mut puts: Vec<Put> = Vec::new();
                let mut places: Vec<(String, (i32, i32))> = Vec::new();
                for source in sources {
                    if matches!(source, DragSource::Icon { .. }) && index_of(source).is_none() {
                        continue;
                    }
                    // Where this one would sit keeping its offset from the grabbed icon; the grabbed one (and a menu program) takes the drop cell.
                    let wanted = match (
                        grabbed_grid,
                        index_of(source).and_then(|i| view.cells.get(i)),
                    ) {
                        (Some(g), Some(r)) => {
                            let own = grid_of(r);
                            (cell.0 + own.0 - g.0, cell.1 + own.1 - g.1)
                        }
                        _ => cell,
                    };
                    let cell = desktop::free_cell_near(grid, area, &taken, wanted);
                    taken.insert(cell);
                    match (source, op) {
                        (DragSource::Icon { .. }, DropOp::Move) => {
                            let key = index_of(source)
                                .and_then(|i| view.entries.get(i))
                                .map(|e| e.name.clone());
                            if let Some(key) = key.filter(|_| !auto) {
                                places.push((key, cell));
                            }
                        }
                        // Another program's file already on the desktop moves to the drop cell as its icon would.
                        (DragSource::File { path, name }, DropOp::Move) if here.contains(path) => {
                            if !auto {
                                places.push((name.clone(), cell));
                            }
                        }
                        _ => {
                            let link = index_of(source)
                                .and_then(|i| view.entries.get(i))
                                .filter(|e| e.kind != desktop::Kind::File)
                                .map(|e| PlaceLink {
                                    icon: e.icon.clone(),
                                    command: desktop::place_command(e),
                                });
                            let saved = desktop::to_saved(grid, cell);
                            puts.push((source.clone(), (!auto).then_some(saved), link));
                        }
                    }
                }
                for (key, cell) in places {
                    self.place_icon(&key, cell);
                }
                if puts.is_empty() {
                    return;
                }
                self.put_into(dir, op, puts, existing, here);
            }
        }
    }

    /// Carry out `puts` into `dir`: files and folders go to Explorer, and launchers of what is no
    /// file ([`Self::write_launchers`]) are written here, all named past `existing`, the desktop's
    /// names. A put onto the desktop selects its icons once the desktop shows them, and one with a
    /// cell lands there; `here` holds the sources that sit in the desktop folder.
    fn put_into(
        &mut self,
        dir: PathBuf,
        op: DropOp,
        puts: Vec<Put>,
        existing: Vec<String>,
        here: &HashSet<PathBuf>,
    ) {
        let (launchers, files): (Vec<Put>, Vec<Put>) = puts
            .into_iter()
            .partition(|(s, _, link)| link.is_some() || matches!(s, DragSource::App { .. }));
        if !files.is_empty() {
            let same_dir = |p: &Path| here.contains(p) || p.parent() == Some(dir.as_path());
            let on_desktop = self.desktop.as_ref().is_some_and(|v| v.dir == dir);
            let mut taken = existing.clone();
            let landings = (files.iter())
                .filter(|_| on_desktop)
                .filter_map(|(s, cell, _)| {
                    let name =
                        crate::filejobs::landing_name(s.path(), op, same_dir(s.path()), &taken)?;
                    taken.push(name.clone());
                    Some((name, *cell))
                })
                .collect();
            let sources: Vec<PathBuf> = files
                .into_iter()
                .map(|(s, ..)| s.path().to_path_buf())
                .collect();
            self.put_files(&dir, op, sources, landings);
        }
        if !launchers.is_empty() {
            self.write_launchers(dir, op, launchers, existing);
        }
    }

    /// Write the launchers of `puts` into `dir` off the loop: a Start menu program's file copied
    /// and allowed to launch, a shortcut to one, or a shortcut to a system icon, which is never
    /// copied. One with a cell lands there; the first failure stops the rest and says why.
    pub(crate) fn write_launchers(
        &mut self,
        dir: PathBuf,
        op: DropOp,
        puts: Vec<Put>,
        existing: Vec<String>,
    ) {
        let history = self.desktop_history();
        let action = Action::changes(op.failure_title(), State::rescan_desktop);
        self.spawn_fs_action(action, move |gate| {
            if !gate.change(&dir) {
                return Box::new(|_| {});
            }
            history.iter().for_each(SharedUndo::clear);
            let mut taken = existing;
            let mut landed = Vec::new();
            let mut failed = None;
            for (source, cell, link) in &puts {
                let op = if link.is_some() { DropOp::Shortcut } else { op };
                match put_in(source, &dir, op, &taken, link.as_ref()) {
                    Ok(name) => {
                        taken.push(name.clone());
                        landed.push((name, *cell));
                    }
                    Err(e) => {
                        log::warn!("drop into {}: {e}", dir.display());
                        failed = Some(op.failure(source.name(), &e));
                        break;
                    }
                }
            }
            Box::new(move |state: &mut State| {
                for (name, cell) in landed {
                    if let Some(cell) = cell {
                        state.land_icon(&name, cell);
                    }
                }
                state.show_drop_failure(failed);
            })
        });
    }

    /// Make a web link file for each of `links` (URL, page title) another program dropped on `target`:
    /// on a desktop cell the first lands there and the rest in the free cells after it; on a folder icon
    /// they go by name. Other targets take none. Each is named after its page
    /// ([`reprisefs::dropdata::link_name`]), numbered past a taken name.
    pub fn put_links(&mut self, links: Vec<(String, Option<String>)>, target: &DropTarget) {
        if links.is_empty() {
            return;
        }
        let (dir, cells) = match target {
            DropTarget::Folder(dir) => (dir.clone(), vec![None; links.len()]),
            DropTarget::Desktop(saved) => {
                let (area, grid) = (self.desktop_area(), self.desktop_grid());
                let cell = desktop::from_saved(grid, *saved);
                let Some(view) = self.desktop.as_ref() else {
                    return;
                };
                let mut taken: std::collections::HashSet<(i32, i32)> = view
                    .cells
                    .iter()
                    .map(|r| desktop::cell_at(grid, area, r.x, r.y))
                    .collect();
                taken.extend(self.icon_blocked());
                let cells = links
                    .iter()
                    .map(|_| {
                        let c = desktop::free_cell_near(grid, area, &taken, cell);
                        taken.insert(c);
                        (!view.auto_arrange).then_some(desktop::to_saved(grid, c))
                    })
                    .collect();
                (view.dir.clone(), cells)
            }
            DropTarget::Quick(_) | DropTarget::Pin { .. } | DropTarget::Trash => return,
        };
        let history = self.desktop_history();
        let action = Action::changes(DropOp::Shortcut.failure_title(), State::rescan_desktop);
        self.spawn_fs_action(action, move |gate| {
            if !gate.change(&dir) {
                return Box::new(|_| {});
            }
            history.iter().for_each(SharedUndo::clear);
            let mut landed = Vec::new();
            let mut failed = None;
            for ((url, title), cell) in links.iter().zip(cells) {
                let name = reprisefs::dropdata::link_name(url, title.as_deref());
                let Some(text) = reprisefs::apps::link_text(&name, url) else {
                    continue;
                };
                match write_link(&dir, &name, &text) {
                    Ok(dest) => {
                        let file = dest.file_name().unwrap_or_default();
                        landed.push((file.to_string_lossy().into_owned(), cell));
                    }
                    Err(e) => {
                        log::warn!("drop into {}: {e}", dir.display());
                        failed = Some(DropOp::Shortcut.failure(&name, &e));
                        break;
                    }
                }
            }
            Box::new(move |state| {
                for (name, cell) in landed {
                    if let Some(cell) = cell {
                        state.land_icon(&name, cell);
                    }
                }
                state.show_drop_failure(failed);
            })
        });
    }

    pub(crate) fn show_drop_failure(&mut self, failed: Option<Failure>) {
        if let Some((title, text)) = failed {
            log::warn!("{title}: {text}");
            self.open_dialog(crate::dialog::Dialog::error(&title, text));
        }
    }
}

/// How far along the task row a drop at bar point (`x`, `y`) over `hit` lands, if on the row.
fn row_drop(layout: &TaskbarLayout, hit: TaskbarHit, x: i32, y: i32) -> Option<i32> {
    let at = layout.along(x, y);
    let on_row = matches!(hit, TaskbarHit::Task(_) | TaskbarHit::More)
        || (hit == TaskbarHit::Nothing && at >= layout.tasks_x() && at < layout.tasks_limit());
    on_row.then_some(at)
}

/// Whether `source` becomes a pin on the task row: a Start menu program, a launcher or a program
/// file. A desktop icon goes by its entry in `entries` as the scan read it; a launcher whose
/// program is untrusted counts, and gets Allow Launching's question (`quicklaunch::add`).
fn pins_on_row(source: &DragSource, entries: &[desktop::DesktopEntry]) -> bool {
    let launcher = |p: &Path| p.extension().is_some_and(|e| e == "desktop");
    match source {
        DragSource::App { .. } => true,
        DragSource::Icon { .. } if source.is_system() => false,
        DragSource::Icon { id, path, .. } => {
            match desktop::find(entries, id).map(|i| &entries[i]) {
                Some(e) if e.is_dir => false,
                Some(e) => match &e.target {
                    Target::Launch(_) | Target::Run(..) => true,
                    Target::Uri(_) => false,
                    Target::Open(_) => e.allow_launching,
                },
                None => launcher(path),
            }
        }
        DragSource::File { path, .. } => launcher(path),
    }
}

/// The Quick Launch slot a drop at bar point (`x`, `y`) over `hit` adds a button at, if on the band.
fn quick_drop(layout: &TaskbarLayout, hit: TaskbarHit, x: i32, y: i32) -> Option<usize> {
    let on_band = matches!(hit, TaskbarHit::Quick(_) | TaskbarHit::QuickMore)
        || (matches!(hit, TaskbarHit::Nothing | TaskbarHit::Grip(_)) && layout.on_quick_band(x, y));
    on_band.then(|| layout.drop_slot(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_on_the_quick_launch_more_cell_adds_past_the_shown_buttons() {
        use reprisekit::taskbar::{Bands, BarSpan, ClockText, Edge, TaskbarCounts, TaskbarShape};
        let m = crate::theme::Metrics::default();
        let span = BarSpan {
            edge: Edge::Bottom,
            length: 800,
            thickness: m.taskbar_height,
        };
        let counts = TaskbarCounts {
            quick: 5,
            tasks: 1,
            ..TaskbarCounts::default()
        };
        let bands = Bands {
            tasks_first: false,
            first_width: Some(0),
        };
        let clock = ClockText { time: 30, date: 30 };
        let layout = TaskbarLayout::compute_on(
            &m,
            &TaskbarShape::CLASSIC,
            span,
            false,
            counts,
            clock,
            bands,
        );
        let more = layout.quick_more.expect("the band ends in a » cell");
        let (x, y) = (more.x + more.width / 2, more.y + more.height / 2);
        let hit = layout.hit(x, y);
        assert_eq!(hit, TaskbarHit::QuickMore);
        assert_eq!(quick_drop(&layout, hit, x, y), Some(layout.quick.len()));
    }

    /// A program dragged from the Start menu before its icon decodes shows a stand-in, and the
    /// image draws again with the icon once it lands.
    #[test]
    fn a_drag_image_draws_again_when_its_cold_icon_lands() {
        let root = crate::icon::svg_theme(&["test-app"], &[]);
        let mut h = crate::icon::two_outputs("classic", &root);
        let entry = AppEntry {
            plain_name: None,
            name: "Test".into(),
            exec_line: "true".into(),
            terminal: false,
            id: "test.desktop".parse().unwrap(),
            icon: Some("test-app".into()),
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            shown: true,
            path: None,
            actions: Vec::new(),
        };
        let spot = reprisekit::startpanel::Spot::Program(0);
        h.state
            .start_panel_row_press(spot, entry, PathBuf::from("/srv/test.desktop"));
        let pos = h.state.pointer_location();
        let waits = h.state.icons.waits();
        h.state.start_drag_image(pos);
        assert!(h.state.icons.waits() > waits, "the image shows a stand-in");
        let image =
            |h: &crate::harness::Harness| h.state.drags.armed.as_ref().and_then(|d| d.image);
        let cold = image(&h).expect("the drag image");
        crate::icon::land(&mut h);
        let landed = image(&h).expect("the drag image");
        assert_ne!(landed, cold, "drawn again on landing");
        let icon = h
            .state
            .icons
            .get_unread("test-app", desktop::ICON_PX)
            .flatten();
        assert!(
            icon.is_some_and(|c| crate::icon::red(&c)),
            "the icon landed"
        );
        let waits = h.state.icons.waits();
        h.state.drag_icons_landed();
        assert_eq!(
            h.state.icons.waits(),
            waits,
            "the icon drew without waiting"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_drag_image_asks_for_frames_only_on_the_outputs_it_crosses() {
        use crate::backend::take_asked_outputs;
        let root = crate::icon::svg_theme(&["test-app"], &[]);
        let mut h = crate::icon::two_outputs("classic", &root);
        let entry = AppEntry {
            plain_name: None,
            name: "Test".into(),
            exec_line: "true".into(),
            terminal: false,
            id: "test.desktop".parse().unwrap(),
            icon: Some("test-app".into()),
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            shown: true,
            path: None,
            actions: Vec::new(),
        };
        let spot = reprisekit::startpanel::Spot::Program(0);
        h.state
            .start_panel_row_press(spot, entry, PathBuf::from("/srv/test.desktop"));
        take_asked_outputs();
        h.state.start_drag_image((300.0, 300.0).into());
        assert_eq!(take_asked_outputs(), ["TEST-1"], "it shows on TEST-1");
        h.state.place_drag_image((400.0, 320.0).into());
        assert_eq!(take_asked_outputs(), ["TEST-1"], "a move on TEST-1");
        h.state.place_drag_image((1020.0, 320.0).into());
        assert_eq!(
            take_asked_outputs(),
            ["TEST-1", "TEST-2"],
            "the image across the edge shows on both"
        );
        h.state.place_drag_image((400.0, 320.0).into());
        take_asked_outputs();
        let mut d = h.state.drags.armed.take().expect("the drag");
        h.state.clear_drag_image(&mut d);
        assert_eq!(take_asked_outputs(), ["TEST-1"], "it goes from TEST-1");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_program_row_drags_only_while_its_menu_is_open() {
        let row = ProgramPress::Menu { depth: 1, row: 3 };
        assert!(row.shown(2, false));
        assert!(!row.shown(1, true), "its submenu shut");
        assert!(!row.shown(0, false), "Start shut");
        let spot = reprisekit::startpanel::Spot::Program(0);
        let panel = ProgramPress::StartPanel(spot);
        assert!(panel.shown(0, true));
        assert!(!panel.shown(1, false), "the Glass Start menu shut");
    }

    #[test]
    fn a_drag_image_is_drawn_again_over_an_output_of_another_scale() {
        assert_eq!(
            redraw_scale(1, Some(2)),
            Some(2),
            "scale 1 to a scale-2 output"
        );
        assert_eq!(redraw_scale(2, Some(1)), Some(1), "and back");
        assert_eq!(redraw_scale(2, Some(2)), None, "same scale");
        assert_eq!(
            redraw_scale(2, None),
            None,
            "off every output it keeps its drawing"
        );
    }

    fn fixture(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("reprise-drag-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn with_the_files_off_only_system_icons_land_on_a_desktop_cell() {
        let icon = |id| DragSource::Icon {
            id,
            path: PathBuf::from("/home/u/Desktop"),
            name: "x".into(),
        };
        let bin = icon(desktop::IconId::System(desktop::Kind::RecycleBin));
        let file = DragSource::file(PathBuf::from("/home/u/a.txt"));
        let desk = icon(desktop::IconId::File(PathBuf::from(
            "/home/u/Desktop/b.txt",
        )));
        let (bin, file, desk) = (&[bin][..], &[file][..], &[desk][..]);
        let mixed = [bin, file].concat();
        for sources in [&[][..], file, desk, bin, &mixed] {
            assert!(takes_cell(true, sources));
        }
        assert!(takes_cell(false, bin));
        assert!(!takes_cell(false, &[]), "another program's drag is refused");
        assert!(!takes_cell(false, file));
        assert!(!takes_cell(false, desk));
        assert!(!takes_cell(false, &mixed));
    }

    #[test]
    fn a_left_drop_moves_icons_copies_programs_and_links_the_rest() {
        let icon = DragSource::Icon {
            id: desktop::IconId::File(PathBuf::from("/d/a.txt")),
            path: PathBuf::from("/d/a.txt"),
            name: "a.txt".into(),
        };
        let app = DragSource::App {
            entry: reprisefs::apps::parse_desktop(
                "[Desktop Entry]\nType=Application\nName=Vim\nExec=vim\n",
                &"vim.desktop".parse().unwrap(),
            )
            .unwrap(),
            file: PathBuf::from("/usr/share/applications/vim.desktop"),
        };
        let folder = DropTarget::Folder(PathBuf::from("/d/Reports"));
        for target in [
            DropTarget::Desktop((1, 2)),
            folder.clone(),
            DropTarget::Trash,
        ] {
            assert_eq!(left_drop_op(&icon, &target), DropOp::Move);
        }
        assert_eq!(left_drop_op(&icon, &DropTarget::Quick(0)), DropOp::Shortcut);
        assert_eq!(left_drop_op(&app, &folder), DropOp::Copy);
        let file = DragSource::file(PathBuf::from("/x/b.txt"));
        assert_eq!(
            left_drop_op(&file, &DropTarget::Desktop((0, 0))),
            DropOp::Shortcut
        );
    }

    #[test]
    fn a_drop_on_the_bin_deletes_an_icons_selection_or_the_files() {
        let id = desktop::IconId::File("/d/a.txt".into());
        let icon = DragSource::Icon {
            id: id.clone(),
            path: PathBuf::from("/d/a.txt"),
            name: "a.txt".into(),
        };
        assert_eq!(bin_drop(&[icon]), Some(BinDrop::Icons(id)));
        let files = [
            DragSource::file(PathBuf::from("/x/b.txt")),
            DragSource::file(PathBuf::from("/x/c.txt")),
        ];
        assert_eq!(
            bin_drop(&files),
            Some(BinDrop::Files(vec![
                PathBuf::from("/x/b.txt"),
                PathBuf::from("/x/c.txt")
            ]))
        );
        assert_eq!(bin_drop(&[]), None);
    }

    #[test]
    fn a_dropped_file_on_the_desktop_is_found_by_its_file_name() {
        let entry = |name: &str, kind| desktop::DesktopEntry {
            name: name.into(),
            label: name.trim_end_matches(".txt").into(),
            icon: "text-x-generic".into(),
            target: desktop::Target::Open(PathBuf::from(name)),
            path: PathBuf::from(name),
            shortcut: false,
            kind,
            is_dir: false,
            allow_launching: false,
            picture: None,
        };
        let entries = [
            entry("notes.txt", desktop::Kind::RecycleBin),
            entry("a.txt", desktop::Kind::File),
            entry("notes.txt", desktop::Kind::File),
        ];
        assert_eq!(desktop_entry_named(&entries, "notes.txt"), Some(2));
        assert_eq!(desktop_entry_named(&entries, "notes"), None);
    }

    #[test]
    fn a_program_is_copied_and_allowed_to_launch_whatever_the_op() {
        let base = fixture("putin");
        let _c = Cleanup(base.clone());
        let desk = base.join("Desktop");
        std::fs::create_dir_all(&desk).unwrap();
        let app_file = base.join("firefox.desktop");
        std::fs::write(
            &app_file,
            "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox\n",
        )
        .unwrap();
        let app = DragSource::App {
            entry: reprisefs::apps::parse_desktop(
                &std::fs::read_to_string(&app_file).unwrap(),
                &"firefox.desktop".parse().unwrap(),
            )
            .unwrap(),
            file: app_file.clone(),
        };
        assert_eq!(
            put_in(&app, &desk, DropOp::Move, &[], None).unwrap(),
            "firefox.desktop"
        );
        assert!(app_file.exists() && desk.join("firefox.desktop").exists());
        assert_eq!(
            put_in(&app, &desk, DropOp::Copy, &[], None).unwrap(),
            "firefox (2).desktop"
        );
        assert!(desk.join("firefox (2).desktop").exists());
        let name = put_in(&app, &desk, DropOp::Shortcut, &[], None).unwrap();
        assert_eq!(name, "Shortcut to Firefox.desktop");
        let text = std::fs::read_to_string(desk.join(&name)).unwrap();
        assert!(text.contains("Exec=firefox"), "{text}");
    }

    #[test]
    fn a_system_icon_shortcut_opens_its_place_and_copies_nothing() {
        let base = fixture("place");
        let _c = Cleanup(base.clone());
        let desk = base.join("Desktop");
        let bin = base.join("Trash").join("files");
        std::fs::create_dir_all(&desk).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("old.txt"), "x").unwrap();
        let source = DragSource::Icon {
            id: desktop::IconId::System(desktop::Kind::RecycleBin),
            path: bin.clone(),
            name: "Recycle Bin".into(),
        };
        let link = PlaceLink {
            icon: "user-trash-full".into(),
            command: desktop::place_command(&desktop::recycle_bin(bin.clone(), false)),
        };
        let name = put_in(&source, &desk, DropOp::Shortcut, &[], Some(&link)).unwrap();
        assert_eq!(name, "Shortcut to Recycle Bin.desktop");
        let text = std::fs::read_to_string(desk.join("Shortcut to Recycle Bin.desktop")).unwrap();
        assert!(
            text.contains("Exec=reprise-explorer \"Recycle Bin\"\n"),
            "{text}"
        );
        assert!(text.contains("Icon=user-trash-full"));
        let landed: Vec<_> = std::fs::read_dir(&desk)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(landed, ["Shortcut to Recycle Bin.desktop"]);
    }

    #[test]
    fn shortcuts_and_copied_programs_scan_as_launchers() {
        use std::os::unix::fs::PermissionsExt;
        let base = fixture("launchers");
        let _c = Cleanup(base.clone());
        let desk = base.join("Desktop");
        std::fs::create_dir_all(&desk).unwrap();
        let app_file = base.join("firefox.desktop");
        std::fs::write(
            &app_file,
            "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox\n",
        )
        .unwrap();
        std::fs::set_permissions(&app_file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let app = DragSource::App {
            entry: reprisefs::apps::parse_desktop(
                &std::fs::read_to_string(&app_file).unwrap(),
                &"firefox.desktop".parse().unwrap(),
            )
            .unwrap(),
            file: app_file.clone(),
        };
        put_in(&app, &desk, DropOp::Copy, &[], None).unwrap();
        put_in(&app, &desk, DropOp::Shortcut, &[], None).unwrap();
        let entries = desktop::scan(
            &desk,
            &reprisefs::apps::MimeDb::empty(),
            reprisefs::names::Extensions::Hide,
            false,
        );
        for name in ["Shortcut to Firefox.desktop", "firefox.desktop"] {
            let e = entries.iter().find(|e| e.name == name);
            assert!(
                e.is_some_and(|e| matches!(e.target, desktop::Target::Launch(_))),
                "{name} scans as a launcher"
            );
        }
        let mode = std::fs::metadata(&app_file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644, "the program's own file stays 0644");
    }

    fn desktop_entry(
        path: &str,
        target: Target,
        is_dir: bool,
        allow: bool,
    ) -> desktop::DesktopEntry {
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        desktop::DesktopEntry {
            label: name.clone(),
            name,
            icon: String::new(),
            target,
            path: PathBuf::from(path),
            shortcut: false,
            kind: desktop::Kind::File,
            is_dir,
            allow_launching: allow,
            picture: None,
        }
    }

    #[test]
    fn only_launchers_and_programs_pin_on_the_task_row() {
        use reprisefs::program::Program;
        let icon = |path: &str| DragSource::Icon {
            id: desktop::IconId::File(PathBuf::from(path)),
            path: PathBuf::from(path),
            name: path.into(),
        };
        let open = |p: &str| Target::Open(PathBuf::from(p));
        let entries = [
            desktop_entry(
                "/d/tool",
                Target::Run("/d/tool".into(), Program::Binary),
                false,
                false,
            ),
            desktop_entry("/d/new.desktop", open("/d/new.desktop"), false, true),
            desktop_entry("/d/notes.txt", open("/d/notes.txt"), false, false),
            desktop_entry(
                "/d/web.desktop",
                Target::Uri("https://example.org".into()),
                false,
                false,
            ),
            desktop_entry("/d/apps.desktop", open("/d/apps.desktop"), true, false),
        ];
        assert!(pins_on_row(&icon("/d/tool"), &entries), "a program file");
        assert!(
            pins_on_row(&icon("/d/new.desktop"), &entries),
            "an untrusted launcher, asked about when it runs"
        );
        assert!(
            !pins_on_row(&icon("/d/notes.txt"), &entries),
            "a plain file"
        );
        assert!(
            !pins_on_row(&icon("/d/web.desktop"), &entries),
            "a web link"
        );
        assert!(!pins_on_row(&icon("/d/apps.desktop"), &entries), "a folder");
        let system = DragSource::Icon {
            id: desktop::IconId::System(desktop::Kind::RecycleBin),
            path: PathBuf::new(),
            name: "Recycle Bin".into(),
        };
        assert!(!pins_on_row(&system, &entries));
        assert!(pins_on_row(&DragSource::file("/x/app.desktop".into()), &[]));
        assert!(!pins_on_row(&DragSource::file("/x/a.txt".into()), &[]));
    }

    #[test]
    fn a_launcher_dropped_on_the_pinned_row_pins_at_that_slot() {
        use crate::config::{Action, QuickLaunchEntry};
        let mut config = crate::config::Config::default();
        config.taskbar.pinned = true;
        let mut h = crate::harness::Harness::with_config(config);
        h.output("TEST-1", 1024, 768);
        let pin = |file: &str| crate::quicklaunch::QuickItem {
            file: file.into(),
            entry: QuickLaunchEntry {
                label: file.into(),
                icon: None,
                action: Action::Spawn("true".into()),
            },
            app: None,
        };
        h.state.quicklaunch = vec![pin("a.desktop"), pin("b.desktop")];
        let mut c = h.connect();
        let w = h.map_toplevel(&mut c, "test.other", 300, 200);
        h.dispatch();
        let bar = &h.state.bars[0];
        let tree = bar.tree;
        let pins = |h: &crate::harness::Harness| -> Vec<_> {
            h.state.bars[0].slots.iter().map(|s| s.pin).collect()
        };
        assert_eq!(pins(&h), [Some(0), Some(1), None]);
        let origin = h.state.bar_drawn(&bar.output).shown;
        let at = |h: &crate::harness::Harness, i: usize, right: bool| {
            let r = h.state.bars[0].layout.tasks[i];
            let x = if right { r.right() - 2 } else { r.x + 2 };
            Point::from((
                f64::from(origin.x + x),
                f64::from(origin.y + r.y + r.height / 2),
            ))
        };
        let launcher = [DragSource::file("/x/app.desktop".into())];
        let target = |h: &crate::harness::Harness, i, right| {
            h.state.drop_target_at(&launcher, at(h, i, right))
        };
        assert_eq!(
            target(&h, 0, false),
            Some(DropTarget::Pin { bar: tree, gap: 0 })
        );
        assert_eq!(
            target(&h, 1, true),
            Some(DropTarget::Pin { bar: tree, gap: 2 }),
            "past the second pin's centre"
        );
        assert_eq!(
            target(&h, 2, false),
            Some(DropTarget::Pin { bar: tree, gap: 2 }),
            "on the running button, short of its centre"
        );
        assert_eq!(
            h.state.row_insert(tree, 2, 1),
            (2, vec![(vec![w.id], None, usize::MAX)]),
            "the window stays past the new pin"
        );
        assert_eq!(
            target(&h, 2, true),
            Some(DropTarget::Pin { bar: tree, gap: 3 })
        );
        let (slot, anchored) = h.state.row_insert(tree, 3, 1);
        assert_eq!(slot, 2);
        h.state.anchor_units(anchored);
        h.state.quicklaunch.insert(slot, pin("app.desktop"));
        h.state.redraw_taskbars();
        h.dispatch();
        assert_eq!(
            pins(&h),
            [Some(0), Some(1), None, Some(2)],
            "the new pin lands past the window it was dropped beyond"
        );

        let text = [DragSource::file("/x/notes.txt".into())];
        assert_eq!(h.state.drop_target_at(&text, at(&h, 0, false)), None);
        assert_eq!(
            h.state.drop_target_at(&[], at(&h, 0, false)),
            None,
            "another program's drag"
        );
        h.state.config.taskbar.pinned = false;
        h.state.redraw_taskbars();
        h.dispatch();
        assert_eq!(
            h.state.drop_target_at(&launcher, at(&h, 0, false)),
            None,
            "a task button takes no launcher while Quick Launch has its own band"
        );
    }
}
