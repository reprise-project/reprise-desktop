//! Off-loop arrivals (thumbnail worker answers, folder watch flag) and the
//! timer-driven infotip.
//!
//! `Event::Wake` drains the arrivals ([`Explorer::wake`]); `Event::Tick`,
//! requested while a hover waits, shows the tip ([`Explorer::tip_tick`]).

use crate::place::Place;
use crate::window;
use crate::{Explorer, HoverOn, Tip};
use repriseapp::{Ctx, KeyInput, PointerInput, Reply};
use reprisefs::listing::Item;
use reprisekit::controls::shellview::{self, Mode};
use reprisekit::controls::taskpane;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

pub const HOVER: Duration = Duration::from_millis(900);

/// What [`Explorer::follow_gone`] asks of the file system: the nearest folder at or above the
/// gone one, and the folders above the old place that still exist, nearest first.
pub(crate) struct Gone {
    nearest: Option<PathBuf>,
    above: Vec<PathBuf>,
}

/// The thread reading the hovered folder's tip ([`window::folder_tip_lines`]).
/// Dropping it sets the stop flag; the window does not wait for it on close.
pub struct TipWalk {
    pub(crate) path: PathBuf,
    pub(crate) stop: Arc<AtomicBool>,
    rx: Option<Receiver<Vec<String>>>,
}

impl Drop for TipWalk {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Explorer {
    /// Entry point for keys: drops the infotip before acting on the key.
    pub fn key(&mut self, k: KeyInput, ctx: &mut Ctx) -> Reply {
        let tipped = self.hide_tip();
        let mut reply = self.key_inner(k, ctx);
        reply.redraw |= tipped;
        reply
    }

    pub fn pointer(&mut self, p: PointerInput, ctx: &mut Ctx) -> Reply {
        self.status.grip = !ctx.maximized;
        let tipped = !matches!(p, PointerInput::Motion { .. }) && self.hide_tip();
        let mut reply = self.pointer_inner(p, ctx);
        reply.redraw |= tipped;
        reply.tick |= self.tip_pending();
        reply
    }

    /// Queues decodes for images without a thumbnail at the window's `scale` (Thumbnails and
    /// Tiles only). One made at another scale shows until its replacement arrives.
    pub fn request_thumbs(&mut self, scale: i32) {
        if !matches!(self.sv.mode, Mode::Thumbnails | Mode::Tiles) {
            return;
        }
        let r#gen = self.generation;
        let wanted: Vec<PathBuf> = self
            .items
            .iter()
            .zip(&self.sv.entries)
            .filter(|(it, e)| it.is_image() && e.thumb.as_ref().is_none_or(|t| t.scale() != scale))
            .map(|(it, _)| it.path.clone())
            .collect();
        for path in wanted {
            self.thumbs.request(path, r#gen, scale);
        }
    }

    /// The main window draws at `scale`, `ctx.icons`' scale: the chrome and the entries wear
    /// their icons at that scale, icon files once a thread decodes them, and thumbnails made at
    /// another one are asked for again.
    pub fn follow_scale(&mut self, scale: i32, ctx: &mut Ctx) {
        if std::mem::replace(&mut self.drawn_scale, scale) != scale {
            self.rescale_chrome(ctx);
            self.list_icons
                .follow(scale, &mut ctx.icons, &mut self.sv.entries, &ctx.wake);
            self.request_thumbs(scale);
        }
    }

    /// `Event::Wake`: a put, job, tip walk, late listing, drive scan, clash check or Folders node read finished, thumbnails arrived, or
    /// the watch or UDisks2 saw a change (which re-lists), or the folder left (which follows it). A closing window quits here
    /// once its work is done.
    pub fn wake(&mut self, ctx: &mut Ctx) -> Reply {
        let put = self.poll_put(ctx);
        let removed = self.poll_removal(ctx);
        self.show_progress_dialogs();
        let job = self.poll_jobs(ctx);
        let tipped = self.poll_tip_walk(ctx);
        if self.closing && !self.busy() {
            return Reply {
                quit: true,
                ..Default::default()
            };
        }
        let browsed = self.take_browse(ctx);
        let mut dirty = tipped
            | self.take_fills(ctx)
            | self.list_icons.take(&mut self.sv.entries, &mut ctx.icons);
        let mut arrived: Vec<PathBuf> = Vec::new();
        for done in self.thumbs.drain() {
            let Some(&i) = self.item_index.get(&done.path) else {
                continue;
            };
            self.image_dims.insert(done.path.clone(), done.dims);
            let kind = self.items[i].kind.clone();
            if let Some(e) = self.sv.entries.get_mut(i) {
                e.thumb = Some(shellview::frame_thumb(done.thumb, done.dims, done.scale));
                e.detail = shellview::image_detail(done.dims, &kind);
            }
            arrived.push(done.path);
            dirty = true;
        }
        // A finished put already re-listed; its reply may carry an error box.
        let put = job
            .or(removed)
            .or(put)
            .or(self.take_read(ctx))
            .or(self.take_checked(ctx))
            .or(self.take_opened(ctx))
            .or(self.take_gone(ctx))
            .or(self.take_drives(ctx))
            .or(self.take_clashes(ctx))
            .filter(|r| *r != Reply::none());
        if let Some(from) = self.watch.as_mut().and_then(|w| w.take_gone()) {
            let r = self.follow_gone(&from, ctx);
            return put.unwrap_or(r);
        }
        if self.watch.as_mut().is_some_and(|w| w.take_changed()) {
            let r = self.refresh(ctx);
            return put.unwrap_or(r);
        }
        if let Some(r) = self.poll_volumes(ctx) {
            return put.unwrap_or(r);
        }
        if let Some(r) = put {
            return r;
        }
        // Rewrite a tip written before the dimensions were known.
        if let Some(crate::Hover {
            on: HoverOn::Entry(i),
            ..
        }) = self.hover
        {
            let hovered = self.items.get(i).map(|it| it.path.clone());
            if self.tip.is_some() && hovered.is_some_and(|p| arrived.contains(&p)) {
                self.show_tip(ctx);
            }
        }
        Reply {
            redraw: dirty || browsed,
            redraw_all: browsed,
            ..Default::default()
        }
    }

    /// The folder shown, or one above it, was deleted or moved away: show the nearest folder that exists at or above
    /// `from` (its new path after a rename in place, else its old one) in its place in the history. Which folders
    /// still exist is asked on a listing thread; a later answer (a dead mount) lands on a wake
    /// ([`Explorer::take_gone`]), dropped if the window has moved on by then.
    pub(crate) fn follow_gone(&mut self, from: &Path, ctx: &mut Ctx) -> Reply {
        let from = from.to_path_buf();
        let old = self.place.path();
        let read = move || Gone {
            nearest: from.ancestors().find(|p| p.is_dir()).map(Path::to_path_buf),
            above: old
                .iter()
                .flat_map(|o| o.ancestors())
                .filter(|p| p.is_dir())
                .map(Path::to_path_buf)
                .collect(),
        };
        match self.read_job(ctx, read) {
            Ok(gone) => self.follow_gone_to(gone, ctx),
            Err(job) => {
                self.going = Some((self.place.clone(), job));
                Reply::none()
            }
        }
    }

    /// Follows a gone folder whose check ([`Explorer::follow_gone`]) answered late, unless the
    /// window has moved on since.
    pub(crate) fn take_gone(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let gone = self.going.as_ref()?.1.take()?;
        let (from, _) = self.going.take()?;
        (from == self.place).then(|| self.follow_gone_to(gone, ctx))
    }

    fn follow_gone_to(&mut self, gone: Gone, ctx: &mut Ctx) -> Reply {
        let Some(dir) = gone.nearest else {
            return self.refresh(ctx);
        };
        let place = Place::from_path(&dir);
        if place == self.place {
            return self.refresh(ctx);
        }
        // The Folders pane still holds the old names. What changed is a folder above the old path;
        // refilling top down keeps what is open below.
        for p in gone.above.iter().rev() {
            self.refill_tree_node(&Place::from_path(p), ctx);
        }
        self.history.replace(place.clone());
        self.show(place, ctx)
    }

    /// Restores the scroll position after a re-listing, clamped to the new content.
    pub fn restore_scroll(&mut self, top: usize, left: i32, ctx: &Ctx) {
        self.sv.top = top;
        self.sv.left = left;
        let frame = self.layout(ctx).view;
        let sl = self.view_layout(frame, ctx);
        // A zero-step scroll clamps `top` in the mode's unit (px or rows).
        shellview::scroll(&mut self.sv, 0, &sl);
        let max_left = (sl.content.0 - sl.body.width).max(0);
        self.sv.left = self.sv.left.min(max_left);
    }

    /// A hover is armed and its tip not up yet, so the window asks for ticks.
    pub fn tip_pending(&self) -> bool {
        self.hover.is_some() && self.tip.is_none()
    }

    /// `Event::Tick` at `now`: shows the tip once the pointer has rested
    /// [`HOVER`], else asks for another tick. A folder's tip starts its walk
    /// then; whichever of tick and walk finishes last shows it.
    pub fn tip_tick(&mut self, now: Instant, ctx: &Ctx) -> Reply {
        let Some(h) = self.hover else {
            return Reply::none();
        };
        if self.tip.is_some() {
            return Reply::none();
        }
        if now.duration_since(h.since) < HOVER {
            return Reply {
                tick: true,
                ..Reply::none()
            };
        }
        if let HoverOn::Entry(i) = h.on {
            self.walk_folder_tip(i, ctx);
        }
        self.show_tip(ctx);
        Reply {
            redraw: self.tip.is_some(),
            ..Reply::none()
        }
    }

    /// Writes and places the tip for what the pointer rests on: an entry's
    /// infotip or a pane row's plain tip. A folder whose lines have not
    /// arrived gets none yet.
    fn show_tip(&mut self, ctx: &Ctx) {
        let Some(h) = self.hover else {
            return;
        };
        let (lines, plain) = match h.on {
            HoverOn::Entry(i) => {
                let Some(it) = self.items.get(i) else {
                    return;
                };
                let lines = if tip_reads_folder(it) {
                    match &self.folder_tip {
                        Some((path, lines)) if *path == it.path => lines.clone(),
                        _ => return,
                    }
                } else {
                    window::infotip_lines(
                        it,
                        self.image_dims.get(&it.path).copied(),
                        reprisefs::locale::is_12h(),
                    )
                };
                (lines, false)
            }
            HoverOn::Pane(i, j) => match self.pane_tip(i, j) {
                Some(tip) => (vec![tip.to_string()], true),
                None => return,
            },
        };
        let size = if plain {
            reprisekit::controls::tooltip::tip_size(&ctx.text, &ctx.theme, &lines.join("\n"))
        } else {
            reprisekit::controls::infotip::size(&ctx.text, &ctx.theme, &lines)
        };
        let at = reprisekit::controls::infotip::place(h.at, size, self.layout(ctx).client);
        self.tip = Some(Tip { at, lines, plain });
    }

    fn pane_tip(&self, i: usize, j: usize) -> Option<&'static str> {
        self.pane_tips.get(i)?.get(j).copied().flatten()
    }

    /// Starts reading entry `i`'s folder tip on a thread unless a walk for
    /// it is already out or done. The lines land in [`Explorer::poll_tip_walk`].
    fn walk_folder_tip(&mut self, i: usize, ctx: &Ctx) {
        let Some(it) = self.items.get(i).filter(|it| tip_reads_folder(it)) else {
            return;
        };
        if self.tip_walk.as_ref().is_some_and(|w| w.path == it.path) {
            return;
        }
        let path = it.path.clone();
        let show_hidden = self.settings.show_hidden;
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = ctx.wake.clone();
        let (dir, flag) = (path.clone(), stop.clone());
        let spawned = std::thread::Builder::new()
            .name("reprise-explorer-tip".into())
            .spawn(move || {
                if let Some(lines) = window::folder_tip_lines(&dir, show_hidden, &flag)
                    && tx.send(lines).is_ok()
                {
                    wake.wake();
                }
            });
        if let Err(e) = spawned {
            log::warn!("explorer: cannot spawn a tip thread ({e})");
            return;
        }
        self.tip_walk = Some(TipWalk {
            path,
            stop,
            rx: Some(rx),
        });
    }

    /// Takes the walk's lines if it has answered, showing them if the hover
    /// has already waited out [`HOVER`]. `true` when a tip went up.
    pub(crate) fn poll_tip_walk(&mut self, ctx: &Ctx) -> bool {
        let Some(w) = self.tip_walk.as_mut() else {
            return false;
        };
        let Some(rx) = w.rx.as_ref() else {
            return false;
        };
        let lines = match rx.try_recv() {
            Ok(lines) => lines,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                w.rx = None;
                return false;
            }
        };
        w.rx = None;
        self.folder_tip = Some((w.path.clone(), lines));
        match self.hover {
            Some(h) if self.tip.is_none() && h.since.elapsed() >= HOVER => {
                self.show_tip(ctx);
                self.tip.is_some()
            }
            _ => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn tip_walk_out(&self) -> bool {
        self.tip_walk.as_ref().is_some_and(|w| w.rx.is_some())
    }

    /// Reads an image's size from its header on a job, for views that decode
    /// no thumbnail. Each image is asked for once per listing; the answer
    /// rewrites a tip already up and the pane.
    pub(crate) fn read_dims(&mut self, i: usize, ctx: &Ctx) {
        let Some(it) = self.items.get(i).filter(|it| it.is_image()) else {
            return;
        };
        if self.image_dims.contains_key(&it.path) || !self.dims_asked.insert(it.path.clone()) {
            return;
        }
        let path = it.path.clone();
        let r#gen = self.generation;
        self.spawn_job(ctx, move || {
            let dims = read_head(&path).and_then(|head| reprisefs::imagesize::dimensions(&head));
            Box::new(move |ex: &mut Explorer, ctx: &mut Ctx| {
                let Some(dims) = dims.filter(|_| ex.generation == r#gen) else {
                    return Reply::none();
                };
                ex.image_dims.insert(path.clone(), dims);
                let hovered = match ex.hover {
                    Some(crate::Hover {
                        on: HoverOn::Entry(i),
                        ..
                    }) => ex.items.get(i).is_some_and(|it| it.path == path),
                    _ => false,
                };
                if hovered && ex.tip.is_some() {
                    ex.show_tip(ctx);
                }
                ex.build_taskpane(ctx);
                Reply::redraw()
            })
        });
    }

    /// What a tip hangs from at `(x, y)`: a view entry ([`shellview::name_hit`])
    /// or a task pane row with a tip.
    fn tip_target(&self, x: i32, y: i32, lay: &window::Layout, ctx: &Ctx) -> Option<HoverOn> {
        let px = ctx.theme.fonts.size_px;
        if lay.view.contains(x, y) {
            if self.sv.edit.is_some() {
                return None;
            }
            let sl = self.view_layout(lay.view, ctx);
            return shellview::name_hit(&self.sv, &sl, x, y, &ctx.text, px).map(HoverOn::Entry);
        }
        let r = lay.tasks.filter(|r| r.contains(x, y))?;
        let controls = ctx.theme.controls();
        let tl = taskpane::layout(&self.taskpane, &ctx.text, px, r, &controls);
        match taskpane::hit(&self.taskpane, &tl, x, y) {
            taskpane::Hit::Row(i, j) if self.pane_tip(i, j).is_some() => Some(HoverOn::Pane(i, j)),
            _ => None,
        }
    }

    /// Arms, re-arms or drops the tip for the pointer at `(x, y)`
    /// ([`Explorer::tip_target`]). `true` when the window needs repainting.
    pub fn hover_view(&mut self, x: i32, y: i32, lay: &window::Layout, ctx: &Ctx) -> bool {
        match self.tip_target(x, y, lay, ctx) {
            Some(on) if self.hover.is_some_and(|h| h.on == on) && self.tip.is_some() => false,
            Some(on) => {
                let had = self.tip.take().is_some();
                if self.hover.is_none_or(|h| h.on != on) {
                    self.tip_walk = None;
                    self.folder_tip = None;
                    if let HoverOn::Entry(i) = on {
                        self.read_dims(i, ctx);
                    }
                }
                self.hover = Some(crate::Hover {
                    on,
                    at: (x, y),
                    since: Instant::now(),
                });
                had
            }
            None => self.hide_tip(),
        }
    }

    /// Drops the infotip and its arming; `true` when a tip was on screen.
    pub fn hide_tip(&mut self) -> bool {
        self.hover = None;
        self.tip_walk = None;
        self.folder_tip = None;
        self.tip.take().is_some()
    }
}

/// A folder whose tip lists its contents: not a drive, not a symlink to a folder.
fn tip_reads_folder(it: &Item) -> bool {
    it.is_folder() && it.drive.is_none()
}

/// The first [`reprisefs::imagesize::HEAD_BYTES`] of `path`, where an image states its size.
fn read_head(path: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(reprisefs::imagesize::HEAD_BYTES as u64)
        .read_to_end(&mut head)
        .ok()?;
    Some(head)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::place::Place;
    use crate::tests::test_ctx;
    use repriseapp::KeyInput;
    use repriseapp::thumbs::Done;
    use reprisekit::raster::Canvas;

    /// One image (by name; nothing decodes it), one text file and one subfolder, in Thumbnails.
    fn explorer(ctx: &mut Ctx) -> (tempfile::TempDir, Explorer) {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(dir.path().join("a.png"), "not really a png").expect("write");
        std::fs::write(dir.path().join("b.txt"), "text").expect("write");
        std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
        let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), ctx);
        ex.sv.mode = Mode::Thumbnails;
        ex.request_thumbs(1);
        (dir, ex)
    }

    fn index_of(ex: &Explorer, name: &str) -> usize {
        ex.items
            .iter()
            .position(|it| it.name == name)
            .unwrap_or_else(|| panic!("{name} is not in the listing"))
    }

    fn centre(ex: &Explorer, ctx: &Ctx, i: usize) -> (i32, i32) {
        let lay = ex.layout(ctx);
        let sl = shellview::layout(&ex.sv, lay.view, &ctx.theme.controls());
        let cell = sl.cells[i];
        (cell.x + cell.width / 2, cell.y + cell.height / 2)
    }

    fn hover(ex: &mut Explorer, ctx: &Ctx, i: usize) -> bool {
        let (x, y) = centre(ex, ctx, i);
        let lay = ex.layout(ctx);
        ex.hover_view(x, y, &lay, ctx)
    }

    #[test]
    fn only_a_file_with_an_image_mime_is_worth_decoding() {
        let mut ctx = test_ctx();
        let (_dir, ex) = explorer(&mut ctx);
        let by = |name: &str| &ex.items[index_of(&ex, name)];
        assert!(by("a.png").is_image());
        assert!(!by("b.txt").is_image());
        assert!(!by("sub").is_image());
    }

    /// A tick after `HOVER` on the hovered entry.
    fn settle(ex: &mut Explorer, ctx: &Ctx) -> Reply {
        let since = ex.hover.expect("a hover is armed").since;
        ex.tip_tick(since + HOVER, ctx)
    }

    #[test]
    fn a_hover_shows_the_tip_after_900_ms() {
        const EXPECTED_HOVER_MS: u64 = 900;
        let mut ctx = test_ctx();
        let (_dir, mut ex) = explorer(&mut ctx);
        let i = index_of(&ex, "b.txt");

        hover(&mut ex, &ctx, i);
        let since = ex.hover.expect("the motion arms a hover").since;
        assert!(ex.tip.is_none(), "motion alone never shows a tip");
        assert!(ex.tip_pending(), "the window asks for ticks");

        let early = ex.tip_tick(since + Duration::from_millis(EXPECTED_HOVER_MS - 1), &ctx);
        assert!(
            early.tick && !early.redraw,
            "too soon: wait for the next tick"
        );
        assert!(ex.tip.is_none());

        let on_time = ex.tip_tick(since + Duration::from_millis(EXPECTED_HOVER_MS), &ctx);
        assert!(on_time.redraw && !on_time.tick);
        let lines = ex.tip.as_ref().expect("the tick shows it").lines.clone();
        assert!(lines[0].starts_with("Type: "), "{lines:?}");

        let later = settle(&mut ex, &ctx);
        assert!(!later.redraw && !later.tick);
        assert_eq!(ex.tip.as_ref().expect("still up").lines, lines);
    }

    #[test]
    fn moving_off_the_entry_or_pressing_a_key_drops_the_tip() {
        let mut ctx = test_ctx();
        let (_dir, mut ex) = explorer(&mut ctx);
        let i = index_of(&ex, "b.txt");

        hover(&mut ex, &ctx, i);
        settle(&mut ex, &ctx);
        assert!(ex.tip.is_some());
        let lay = ex.layout(&ctx);
        let dirty = ex.hover_view(
            lay.view.x + lay.view.width / 2,
            lay.view.bottom() - 4,
            &lay,
            &ctx,
        );
        assert!(dirty, "the tip that went has to be painted out");
        assert!(ex.tip.is_none() && ex.hover.is_none());

        hover(&mut ex, &ctx, i);
        settle(&mut ex, &ctx);
        assert!(ex.tip.is_some());
        let reply = ex.key(
            KeyInput {
                sym: reprisekit::textedit::keysym::TAB,
                utf8: None,
                mods: repriseapp::Mods::default(),
                edit: None,
            },
            &mut ctx,
        );
        assert!(reply.redraw);
        assert!(ex.tip.is_none() && ex.hover.is_none());
    }

    #[test]
    fn a_folder_tip_lists_the_folder_once_its_job_answers() {
        let mut ctx = test_ctx();
        let (dir, mut ex) = explorer(&mut ctx);
        std::fs::write(dir.path().join("sub").join("inner.txt"), "1234").expect("write");
        let i = index_of(&ex, "sub");

        hover(&mut ex, &ctx, i);
        let since = ex.hover.expect("armed").since;
        let reply = ex.tip_tick(since + HOVER, &ctx);
        assert!(!reply.redraw && ex.tip.is_none());

        let waited = Instant::now()
            .checked_sub(HOVER)
            .expect("a clock past HOVER");
        ex.hover.as_mut().expect("armed").since = waited;
        ex.finish_puts(&mut ctx);
        let lines = &ex.tip.as_ref().expect("the answer shows it").lines;
        assert_eq!(lines, &["Size: 4 bytes", "Files: inner.txt"]);
    }

    /// Past the name in Details (the Size column) arms nothing.
    #[test]
    fn the_tip_shows_in_icons_list_and_details() {
        let mut ctx = test_ctx();
        let (_dir, mut ex) = explorer(&mut ctx);
        let i = index_of(&ex, "b.txt");
        for mode in [Mode::Icons, Mode::List, Mode::Details] {
            ex.sv.set_mode(mode);
            ex.hide_tip();
            let lay = ex.layout(&ctx);
            let sl = shellview::layout(&ex.sv, lay.view, &ctx.theme.controls());
            let px = ctx.theme.fonts.size_px;
            let name = shellview::label_rect(&ex.sv, &sl, i, &ctx.text, px);
            let (x, y) = (name.x + 2, name.y + name.height / 2);
            ex.hover_view(x, y, &lay, &ctx);
            settle(&mut ex, &ctx);
            let lines = &ex.tip.as_ref().unwrap_or_else(|| panic!("{mode:?}")).lines;
            assert!(lines[0].starts_with("Type: "), "{mode:?}: {lines:?}");
            if mode == Mode::Details {
                ex.hover_view(name.right() + 80, y, &lay, &ctx);
                assert!(ex.tip.is_none() && ex.hover.is_none(), "the Size column");
            }
        }
    }

    #[test]
    fn a_place_link_in_the_pane_shows_its_tip() {
        let mut ctx = test_ctx();
        let (_dir, mut ex) = explorer(&mut ctx);
        ex.settings.folders = false;
        ex.build_taskpane(&mut ctx);
        let lay = ex.layout(&ctx);
        let r = lay.tasks.expect("the task pane");
        let px = ctx.theme.fonts.size_px;
        let controls = ctx.theme.controls();
        let tl = taskpane::layout(&ex.taskpane, &ctx.text, px, r, &controls);
        let goto = crate::commands::Command::GoTo(Place::Computer);
        let (i, j) = (0..ex.pane_cmds.len())
            .find_map(|i| Some((i, ex.pane_cmds[i].iter().position(|c| *c == goto)?)))
            .expect("a Computer link");
        let row = tl.rows[i][j];
        ex.hover_view(row.x + 4, row.y + row.height / 2, &lay, &ctx);
        settle(&mut ex, &ctx);
        let tip = ex.tip.as_ref().expect("the link's tip");
        assert!(tip.plain, "a plain tooltip, not an infotip");
        assert_eq!(tip.lines, [reprisefs::places::ShellPlace::Computer.tip()]);

        let task = tl.rows[0][0];
        ex.hover_view(task.x + 4, task.y + task.height / 2, &lay, &ctx);
        assert!(ex.hover.is_none() && ex.tip.is_none(), "a task has no tip");
    }

    /// One walk at a time, only after the rest; a walk still out never holds the window open on close.
    #[test]
    fn a_folder_tip_walk_waits_for_the_rest_and_stops_when_left() {
        let mut ctx = test_ctx();
        let (_dir, mut ex) = explorer(&mut ctx);
        let (sub, txt) = (index_of(&ex, "sub"), index_of(&ex, "b.txt"));

        hover(&mut ex, &ctx, sub);
        assert!(!ex.busy(), "motion alone walks nothing");
        let since = ex.hover.expect("armed").since;
        ex.tip_tick(since + HOVER - Duration::from_millis(1), &ctx);
        assert!(ex.tip_walk.is_none(), "too soon to walk");
        settle(&mut ex, &ctx);
        let walk = ex.tip_walk.as_ref().expect("the rest starts the walk");
        let stop = walk.stop.clone();
        assert!(!ex.busy(), "a tip walk is no work to wait for");

        hover(&mut ex, &ctx, txt);
        assert!(
            stop.load(Ordering::Relaxed),
            "leaving the folder stops its walk"
        );
        assert!(ex.tip_walk.is_none());

        hover(&mut ex, &ctx, sub);
        settle(&mut ex, &ctx);
        let stop = ex.tip_walk.as_ref().expect("walking again").stop.clone();
        ex.hide_tip();
        assert!(stop.load(Ordering::Relaxed), "a dropped tip stops its walk");

        hover(&mut ex, &ctx, sub);
        settle(&mut ex, &ctx);
        assert!(ex.tip_walk.is_some());
        let reply = ex.run(crate::commands::Command::Close, &mut ctx);
        assert!(reply.quit, "close quits with a tip walk out");
    }

    #[test]
    fn a_folder_tip_for_an_entry_left_behind_is_dropped() {
        let mut ctx = test_ctx();
        let (_dir, mut ex) = explorer(&mut ctx);
        let (sub, txt) = (index_of(&ex, "sub"), index_of(&ex, "b.txt"));
        hover(&mut ex, &ctx, sub);
        hover(&mut ex, &ctx, txt);
        ex.finish_puts(&mut ctx);
        assert!(ex.folder_tip.is_none());
        assert!(ex.tip.is_none(), "b.txt's own tip waits for its tick");
    }

    #[test]
    fn another_entry_re_arms_the_tip() {
        let mut ctx = test_ctx();
        let (_dir, mut ex) = explorer(&mut ctx);
        let (a, b) = (index_of(&ex, "a.png"), index_of(&ex, "b.txt"));

        hover(&mut ex, &ctx, a);
        settle(&mut ex, &ctx);
        assert!(ex.tip.is_some());

        assert!(hover(&mut ex, &ctx, b), "the old tip has to be rubbed out");
        assert!(ex.tip.is_none());
        assert!(ex.hover.is_some_and(|h| h.on == HoverOn::Entry(b)) && ex.tip_pending());
    }

    #[test]
    fn a_thumbnail_arriving_rewrites_the_tip_under_the_pointer() {
        let mut ctx = test_ctx();
        let (_dir, mut ex) = explorer(&mut ctx);
        let i = index_of(&ex, "a.png");
        let path = ex.items[i].path.clone();

        hover(&mut ex, &ctx, i);
        settle(&mut ex, &ctx);
        let before = ex.tip.as_ref().expect("a tip").lines.clone();
        assert!(before[0].starts_with("Type: "), "{before:?}");

        ex.thumbs.deliver(Done {
            path,
            thumb: Canvas::new(4, 3),
            dims: (800, 600),
            generation: ex.generation,
            scale: 1,
        });
        let reply = ex.wake(&mut ctx);

        assert!(reply.redraw);
        let after = ex.tip.as_ref().expect("still a tip").lines.clone();
        assert_eq!(after[0], "Dimensions: 800 x 600");
        assert_eq!(ex.sv.entries[i].detail[0], "800 x 600");
        assert!(ex.sv.entries[i].thumb.is_some());
    }

    /// Outside Thumbnails and Tiles the size comes from the file header.
    #[test]
    fn list_view_reads_an_images_size_from_its_header() {
        let mut ctx = test_ctx();
        let (dir, mut ex) = explorer(&mut ctx);
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend(800u32.to_be_bytes());
        png.extend(600u32.to_be_bytes());
        std::fs::write(dir.path().join("a.png"), png).expect("write");
        ex.sv.set_mode(Mode::List);
        let i = index_of(&ex, "a.png");

        let lay = ex.layout(&ctx);
        let sl = shellview::layout(&ex.sv, lay.view, &ctx.theme.controls());
        let px = ctx.theme.fonts.size_px;
        let name = shellview::label_rect(&ex.sv, &sl, i, &ctx.text, px);
        ex.hover_view(name.x + 2, name.y + name.height / 2, &lay, &ctx);
        ex.finish_puts(&mut ctx);
        settle(&mut ex, &ctx);
        let lines = &ex.tip.as_ref().expect("a tip").lines;
        assert_eq!(lines[0], "Dimensions: 800 x 600");
        assert!(ex.sv.entries[i].thumb.is_none(), "List decodes nothing");

        ex.hide_tip();
        ex.image_dims.clear();
        ex.dims_asked.clear();
        ex.sv.selection.clear();
        ex.sv.selection.set.insert(i);
        ex.build_taskpane(&mut ctx);
        ex.finish_puts(&mut ctx);
        let details = match &ex.taskpane.sections.last().expect("Details").body {
            taskpane::Body::Lines(lines) => lines.clone(),
            taskpane::Body::Links(_) => panic!("Details holds lines"),
        };
        assert!(
            details.contains(&("Dimensions: 800 x 600".to_string(), false)),
            "{details:?}"
        );
    }

    #[test]
    fn a_thumbnail_arriving_after_a_sort_lands_on_its_entry() {
        let mut ctx = test_ctx();
        let (dir, mut ex) = explorer(&mut ctx);
        let before = index_of(&ex, "a.png");
        // By Size: b.txt (4 bytes) moves ahead of a.png (16).
        ex.sv.sort = Some((1, true));
        ex.resort();
        let a = index_of(&ex, "a.png");
        assert_ne!(a, before, "the sort moved a.png");

        ex.thumbs.deliver(Done {
            path: dir.path().join("a.png"),
            thumb: Canvas::new(4, 3),
            dims: (800, 600),
            generation: ex.generation,
            scale: 1,
        });
        ex.wake(&mut ctx);

        assert!(ex.sv.entries[a].thumb.is_some(), "a.png has its thumbnail");
        assert_eq!(ex.sv.entries[a].name, "a.png");
        let others = (0..ex.sv.entries.len()).filter(|&i| i != a);
        assert!(others.into_iter().all(|i| ex.sv.entries[i].thumb.is_none()));
    }

    #[test]
    fn a_new_draw_scale_asks_again_and_keeps_the_old_thumbnail_until_then() {
        let mut ctx = test_ctx();
        let (dir, mut ex) = explorer(&mut ctx);
        let path = dir.path().join("a.png");
        let deliver = |ex: &mut Explorer, ctx: &mut Ctx, scale: i32| {
            ex.thumbs.deliver(Done {
                path: path.clone(),
                thumb: Canvas::new(400, 300),
                dims: (800, 600),
                generation: ex.generation,
                scale,
            });
            ex.wake(ctx);
        };
        deliver(&mut ex, &mut ctx, 1);
        ex.follow_scale(1, &mut ctx);
        assert!(!ex.thumbs.asked(&path, 2));
        let a = index_of(&ex, "a.png");

        ctx.icons.set_scale(2);
        ex.follow_scale(2, &mut ctx);
        assert!(ex.thumbs.asked(&path, 2), "asked for at scale 2");
        let kept = ex.sv.entries[a].thumb.as_ref().expect("the old one shows");
        assert_eq!((kept.scale(), kept.device_size()), (1, (94, 71)));

        deliver(&mut ex, &mut ctx, 2);
        let t = ex.sv.entries[a].thumb.as_ref().expect("the new one");
        assert_eq!((t.scale(), t.device_size()), (2, (188, 142)));
        assert!(!ex.thumbs.asked(&path, 2));
    }

    /// A theme carrying every name the window's chrome asks for, each a flat square at 16, 24,
    /// 32 and 48 px.
    fn chrome_theme(root: &Path) -> reprisekit::icontheme::IconCache {
        const NAMES: &str = "go-previous go-next go-up go-jump folder folder-open view-list \
             user-desktop folder-documents user-home computer user-trash user-trash-full \
             folder-new folder-remote folder-copy edit-copy edit-cut edit-delete edit-rename \
             edit-undo mail-send preferences-system system-software-install web-browser \
             drive-harddisk drive-removable-media media-optical document-properties";
        let theme = root.join("flat");
        let dirs = ["16x16/apps", "24x24/apps", "32x32/apps", "48x48/apps"];
        let mut index = format!("[Icon Theme]\nName=Flat\nDirectories={}\n", dirs.join(","));
        for (d, px) in dirs.iter().zip([16u32, 24, 32, 48]) {
            index += &format!("\n[{d}]\nSize={px}\nType=Fixed\n");
            std::fs::create_dir_all(theme.join(d)).expect("mkdir");
            let mut c = Canvas::new(px, px);
            c.pixels.fill(0xff20_8040);
            let png = reprisekit::raster::to_png(&c);
            for name in NAMES.split_whitespace() {
                std::fs::write(theme.join(d).join(format!("{name}.png")), &png).expect("write");
            }
        }
        std::fs::write(theme.join("index.theme"), index).expect("write");
        reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "flat",
            &[root.to_path_buf()],
        ))
    }

    /// The scales of every icon the toolbar, address bar, Folders tree, status bar and task pane
    /// wear.
    fn chrome_scales(ex: &Explorer) -> Vec<i32> {
        fn tree(nodes: &[reprisekit::controls::treeview::Node], out: &mut Vec<i32>) {
            for n in nodes {
                out.extend(
                    [&n.icon, &n.open_icon]
                        .into_iter()
                        .flatten()
                        .map(Canvas::scale),
                );
                tree(&n.children, out);
            }
        }
        let mut out = Vec::new();
        for item in &ex.toolbar.items {
            if let reprisekit::controls::toolbar::Item::Button { icon, .. }
            | reprisekit::controls::toolbar::Item::DropButton { icon, .. } = item
            {
                out.extend(icon.as_ref().map(Canvas::scale));
            }
        }
        let a = &ex.address;
        out.extend(
            a.combo
                .items
                .iter()
                .filter_map(|i| i.icon.as_ref())
                .map(Canvas::scale),
        );
        out.extend(
            [&a.combo.icon, &a.go_icon]
                .into_iter()
                .flatten()
                .map(Canvas::scale),
        );
        tree(&ex.tree.roots, &mut out);
        out.extend(
            ex.status
                .panels
                .iter()
                .filter_map(|p| p.icon.as_ref())
                .map(Canvas::scale),
        );
        for s in &ex.taskpane.sections {
            if let taskpane::Body::Links(rows) = &s.body {
                out.extend(
                    rows.iter()
                        .filter_map(|r| r.icon.as_ref())
                        .map(Canvas::scale),
                );
            }
        }
        out
    }

    #[test]
    fn a_new_draw_scale_fetches_the_chrome_icons_again() {
        let art = tempfile::tempdir().expect("a temp dir");
        let mut ctx = test_ctx();
        ctx.icons = chrome_theme(art.path());
        let (_dir, mut ex) = explorer(&mut ctx);
        ex.follow_scale(1, &mut ctx);
        let at_1 = chrome_scales(&ex);
        assert!(at_1.len() > 15, "the chrome wears {} icons", at_1.len());
        assert!(at_1.iter().all(|&s| s == 1));

        ctx.icons.set_scale(2);
        ex.follow_scale(2, &mut ctx);
        let at_2 = chrome_scales(&ex);
        assert_eq!(at_2.len(), at_1.len());
        let left = at_2.iter().filter(|&&s| s != 2).count();
        assert_eq!(left, 0, "of {} icons, these stay at scale 1", at_2.len());

        ctx.icons.set_scale(1);
        ex.follow_scale(1, &mut ctx);
        assert_eq!(chrome_scales(&ex), at_1);
    }

    /// A place still being read leaves the window on the listing before, refreshes included,
    /// and shows once the read answers.
    #[test]
    fn a_late_listing_lands_and_the_one_before_stays_until_then() {
        let mut ctx = test_ctx();
        let (dir, mut ex) = explorer(&mut ctx);
        let sub = dir.path().join("sub");
        std::fs::write(sub.join("inner.txt"), "x").expect("write");
        let place = Place::Folder(sub);
        let read = ex.reader(place.clone(), &ctx);
        let (answer, gate) = std::sync::mpsc::channel::<()>();
        ex.reading = Some(crate::Reading {
            key: (place.clone(), ex.settings.show_hidden, ctx.icons.scale()),
            job: repriseapp::listjob::Job::spawn(None, move || {
                gate.recv().ok();
                read()
            }),
            again: false,
        });
        let before = ex.place.clone();

        assert_eq!(ex.refresh(&mut ctx), Reply::none(), "the read is newer");
        assert_eq!(ex.place, before);
        index_of(&ex, "a.png");

        answer.send(()).expect("send");
        for _ in 0..500 {
            if ex.place == place {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            ex.wake(&mut ctx);
        }
        assert_eq!(ex.place, place);
        index_of(&ex, "inner.txt");
        assert!(ex.reading.is_none());
    }

    #[test]
    fn a_visit_back_to_a_folder_left_mid_read_joins_that_read() {
        let mut ctx = test_ctx();
        let (dir, mut ex) = explorer(&mut ctx);
        let home = ex.place.clone();
        let place = Place::Folder(dir.path().join("sub"));
        let before = ex.reader(place.clone(), &ctx)();
        let (answer, gate) = std::sync::mpsc::channel::<()>();
        ex.reading = Some(crate::Reading {
            key: (place.clone(), ex.settings.show_hidden, ctx.icons.scale()),
            job: repriseapp::listjob::Job::spawn(None, move || {
                gate.recv().ok();
                before
            }),
            again: false,
        });

        ex.go(home.clone(), &mut ctx);
        assert!(ex.reading.is_none());
        assert_eq!(ex.left.len(), 1, "the read left behind is kept");

        std::fs::write(dir.path().join("sub/late.txt"), "x").expect("write");
        answer.send(()).expect("send");
        ex.go(place.clone(), &mut ctx);
        assert_eq!(ex.place, place);
        assert!(ex.left.is_empty());
        assert!(
            ex.items.iter().all(|it| it.name != "late.txt"),
            "the visit is answered by the read it joined"
        );
        for _ in 0..500 {
            if ex.reading.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            ex.wake(&mut ctx);
        }
        index_of(&ex, "late.txt");
    }

    #[test]
    fn a_refresh_of_a_folder_being_read_waits_on_that_read_then_reads_it_once_more() {
        let mut ctx = test_ctx();
        let (dir, mut ex) = explorer(&mut ctx);
        let place = ex.place.clone();
        let before = ex.reader(place.clone(), &ctx)();
        ex.reading = Some(crate::Reading {
            key: (place, ex.settings.show_hidden, ctx.icons.scale()),
            job: repriseapp::listjob::Job::spawn(None, move || before),
            again: false,
        });
        std::fs::write(dir.path().join("late.txt"), "x").expect("write");

        ex.refresh(&mut ctx);
        assert!(
            ex.items.iter().all(|it| it.name != "late.txt"),
            "the read in flight answers the refresh"
        );
        assert!(ex.reading.is_some(), "a read started after the refresh");
        for _ in 0..500 {
            if ex.reading.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            ex.wake(&mut ctx);
        }
        index_of(&ex, "late.txt");
        assert!(ex.reading.is_none(), "the second read leaves none behind");
    }

    #[test]
    fn a_new_draw_scale_swaps_the_list_icons_and_keeps_the_selection() {
        const OWN: u32 = 0xff80_20a0;
        let mut ctx = test_ctx();
        let (dir, mut ex) = explorer(&mut ctx);
        let art = tempfile::tempdir().expect("a temp dir");
        let own = art.path().join("own.png");
        let mut c = Canvas::new(96, 96);
        c.pixels.fill(OWN);
        std::fs::write(&own, reprisekit::raster::to_png(&c)).expect("write");
        let directory = format!("[Desktop Entry]\nIcon={}\n", own.display());
        std::fs::write(dir.path().join("sub/.directory"), directory).expect("write");
        ex.refresh(&mut ctx);
        let sub = dir.path().join("sub");
        ex.select_paths(std::slice::from_ref(&sub));
        ex.follow_scale(1, &mut ctx);
        let drawn = |ex: &Explorer| {
            let e = &ex.sv.entries[index_of(ex, "sub")];
            [&e.icon, &e.icon32, &e.icon48].map(|c| {
                c.as_ref().map(|c| {
                    let mid = c.get(c.width as i32 / 2, c.height as i32 / 2);
                    (c.scale(), c.device_size().0, mid)
                })
            })
        };
        assert_eq!(drawn(&ex)[2], Some((1, 48, OWN)));

        ctx.icons.set_scale(2);
        ex.follow_scale(2, &mut ctx);
        for _ in 0..500 {
            if drawn(&ex)[0].is_some_and(|(scale, ..)| scale == 2) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            ex.wake(&mut ctx);
        }
        assert_eq!(
            drawn(&ex),
            [Some((2, 32, OWN)), Some((2, 64, OWN)), Some((2, 96, OWN))]
        );
        assert_eq!(ex.selected_paths(), [sub], "the folder is not listed again");
    }

    #[test]
    fn a_refresh_keeps_the_thumbnails_of_unchanged_images() {
        let mut ctx = test_ctx();
        let (dir, mut ex) = explorer(&mut ctx);
        std::fs::write(dir.path().join("c.png"), "not a png either").expect("write");
        ex.refresh(&mut ctx);
        for name in ["a.png", "c.png"] {
            ex.thumbs.deliver(Done {
                path: dir.path().join(name),
                thumb: Canvas::new(4, 3),
                dims: (800, 600),
                generation: ex.generation,
                scale: 1,
            });
        }
        ex.wake(&mut ctx);

        std::fs::write(dir.path().join("c.png"), "a longer body than before").expect("write");
        std::fs::write(dir.path().join("d.txt"), "new").expect("write");
        ex.refresh(&mut ctx);

        let a = index_of(&ex, "a.png");
        assert!(
            ex.sv.entries[a].thumb.is_some(),
            "a.png keeps its thumbnail"
        );
        assert_eq!(ex.sv.entries[a].detail[0], "800 x 600");
        assert!(ex.image_dims.contains_key(&dir.path().join("a.png")));
        let c = index_of(&ex, "c.png");
        assert!(
            ex.sv.entries[c].thumb.is_none(),
            "c.png changed, so it has no thumbnail"
        );
        assert!(!ex.image_dims.contains_key(&dir.path().join("c.png")));
    }
}
