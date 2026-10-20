//! Dragging files out of the file manager and dropping files and links into it,
//! all through the seat's drag and drop: a drag inside one window,
//! between two, and to or from any other program take the same path.

use crate::commands::Command;
use crate::place::Place;
use crate::window::Popup;
use crate::{BTN_LEFT, DRAG_SLOP, Explorer};
use repriseapp::listjob::Job;
use repriseapp::{Accept, Actions, Ctx, DragAction, DragOut, DropKind, Dropped, Offered, Uri};
use repriseapp::{Reply, fielddrop};
use reprisefs::deadline::Gate;
use reprisefs::ops::DropOp;
use reprisekit::controls::addressbar;
use reprisekit::controls::iconview::{self, Ghost};
use reprisekit::controls::shellview::{self, Mode};
use reprisekit::controls::treeview;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::selection::Mods;
use std::path::{Path, PathBuf};

pub const THRESHOLD: i32 = DRAG_SLOP;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    ViewEntry(usize),
    TreeRow(usize),
    Background,
}

pub struct Drag {
    pub button: u32,
    pub origin: (i32, i32),
    pub started: bool,
    pub items: Vec<usize>,
    pub paths: Vec<PathBuf>,
    pub pending_click: Option<(usize, Mods)>,
}

pub struct Incoming {
    pub offered: Offered,
    pub own: bool,
    pub paths: Option<Vec<PathBuf>>,
    pub at: (i32, i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingDrop {
    pub paths: Vec<PathBuf>,
    pub links: Vec<WebLink>,
    pub dir: PathBuf,
    pub source: Actions,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebLink {
    pub url: String,
    pub title: Option<String>,
}

pub fn drop_kind(offered: Offered) -> Option<DropKind> {
    (offered.files || offered.link).then_some(DropKind::Uris)
}

/// `on_device`: every file sits on the target folder's file system ([`Facts`]).
pub fn accept_for(offered: Offered, paths: Option<&[PathBuf]>, on_device: bool) -> Accept {
    let copy = Actions {
        copy: true,
        move_: false,
        ask: false,
    };
    let actions = match paths {
        _ if offered.link => copy,
        Some([]) => copy,
        Some(_) => Actions::ALL,
        None => Actions { ask: true, ..copy },
    };
    let moves = actions.move_ && on_device;
    Accept {
        kind: DropKind::Uris,
        actions,
        preferred: Some(if moves {
            DragAction::Move
        } else {
            DragAction::Copy
        }),
    }
}

pub fn all_on_device(paths: &[PathBuf], dir: &Path) -> bool {
    !paths.is_empty() && paths.iter().all(|p| reprisefs::ops::same_device(p, dir))
}

pub fn lands_in_place(paths: &[PathBuf], dir: &Path) -> bool {
    !paths.is_empty() && paths.iter().all(|p| reprisefs::ops::same_folder(p, dir))
}

/// [`lands_in_place`] by the names alone, for a drop whose [`Facts`] have not answered: the
/// listing's paths and the folder's share their spelling.
pub fn named_in_place(paths: &[PathBuf], dir: &Path) -> bool {
    !paths.is_empty() && paths.iter().all(|p| p.parent() == Some(dir))
}

/// What a drag's files are to the folder under the pointer. Both stat the files and the folder, so
/// they are read on a listing thread; unknown (a mount not answering) is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub on_device: bool,
    pub in_place: bool,
}

/// The most [`Facts`] reads a drag keeps out at once: a drag over dead mounts holds this many
/// threads, and a later folder answers once one of them lands.
const FACTS_OUT: usize = 2;

/// [`Facts`] for one folder a drag asked about, or the read still out.
pub struct FactsRead {
    paths: Vec<PathBuf>,
    dir: PathBuf,
    read: Result<Facts, Job<Facts>>,
}

pub fn ask_default(shortcut: bool, on_device: bool) -> DropOp {
    if shortcut {
        DropOp::Shortcut
    } else if on_device {
        DropOp::Move
    } else {
        DropOp::Copy
    }
}

pub fn answer_for(op: DropOp, source: Actions) -> Option<DragAction> {
    let want = match op {
        DropOp::Move => DragAction::Move,
        DropOp::Copy | DropOp::Shortcut => DragAction::Copy,
    };
    let other = match want {
        DragAction::Copy => DragAction::Move,
        _ => DragAction::Copy,
    };
    if source.has(want) {
        Some(want)
    } else if op == DropOp::Shortcut && source.has(other) {
        Some(other)
    } else {
        None
    }
}

pub fn split_drop(data: Dropped) -> (Vec<PathBuf>, Vec<WebLink>) {
    match data {
        Dropped::Files(p) => (p, Vec::new()),
        Dropped::Uris(items) => {
            let mut paths = Vec::new();
            let mut links = Vec::new();
            for it in items {
                match it {
                    Uri::File(p) => paths.push(p),
                    Uri::Url { url, title } => links.push(WebLink { url, title }),
                }
            }
            (paths, links)
        }
        Dropped::Text(_) => (Vec::new(), Vec::new()),
    }
}

pub fn write_links(links: &[WebLink], dir: &Path) -> Result<(), String> {
    use std::io::Write;
    let mut failed = None;
    for link in links {
        let label = reprisefs::dropdata::link_name(&link.url, link.title.as_deref());
        let name = format!("{label}.desktop");
        let Some(text) = reprisefs::apps::link_text(&label, &link.url) else {
            continue;
        };
        let written = reprisefs::names::numbered(name.as_ref(), reprisefs::names::paren)
            .take(1000)
            .map(|n| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(dir.join(&n))
                    .and_then(|mut f| f.write_all(text.as_bytes()))
            })
            .find(|r| !matches!(r, Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists));
        match written {
            Some(Ok(())) => {}
            Some(Err(e)) => {
                failed.get_or_insert(format!("Cannot create {name}: {e}"));
            }
            None => {
                failed.get_or_insert(format!("Cannot create {name}: every name is taken"));
            }
        }
    }
    failed.map_or(Ok(()), Err)
}

impl Explorer {
    /// The selection change is held back when the press lands on an
    /// entry that is already selected: the press arms a drag of the
    /// selection as it stands, so dragging one of several carries all
    /// of them, and only a release that never became a drag applies the
    /// click: collapsing a multi-selection to the one pressed, or
    /// toggling it out under Ctrl. A press outside the selection
    /// selects (or Ctrl-adds) at once and drags that.
    pub(crate) fn press_entry(
        &mut self,
        i: usize,
        at: (i32, i32),
        time: u32,
        mods: repriseapp::Mods,
        cell: Option<Rect>,
        ctx: &mut Ctx,
    ) -> Reply {
        let (x, y) = at;
        let double = self.clicks.press(i, ctx.theme.metrics.click(x, y, time));
        if double {
            self.sv.selection.set_only(Some(i));
            return self.open_selected(ctx);
        }
        let click = Mods {
            ctrl: mods.ctrl,
            shift: mods.shift,
        };
        let held_back = self.sv.selection.set.contains(&i);
        if !held_back {
            self.sv.selection.click(i, click);
        }
        self.sv.focus_ring = true;
        match cell {
            Some(_) => {
                self.arm_drag(i, BTN_LEFT, x, y);
                if let Some(d) = self.drag.as_mut() {
                    d.pending_click = held_back.then_some((i, click));
                }
            }
            None if held_back => self.sv.selection.click(i, click),
            None => {}
        }
        self.refresh_chrome(ctx);
        Reply::redraw()
    }

    /// A read-only place arms nothing: the drives under Computer and the
    /// bin's contents are not files to move, and every operation behind
    /// a drop ([`Explorer::put_all`]) refuses them anyway.
    pub(crate) fn arm_drag(&mut self, i: usize, button: u32, x: i32, y: i32) {
        if self.place.read_only() {
            return;
        }
        let items: Vec<usize> = self.sv.selection.set.iter().copied().collect();
        let items = if items.contains(&i) { items } else { vec![i] };
        let paths: Vec<PathBuf> = items
            .iter()
            .filter_map(|&i| self.items.get(i).map(|it| it.path.clone()))
            .collect();
        if paths.is_empty() {
            return;
        }
        self.drag = Some(Drag {
            button,
            origin: (x, y),
            started: false,
            items,
            paths,
            pending_click: None,
        });
    }

    pub(crate) fn drag_motion(&mut self, x: i32, y: i32, ctx: &mut Ctx) -> Reply {
        let Some(d) = self.drag.as_ref().filter(|d| !d.started) else {
            return Reply::none();
        };
        let (ox, oy) = d.origin;
        if (x - ox).abs() < THRESHOLD && (y - oy).abs() < THRESHOLD {
            return Reply::none();
        }
        let (image, grab) = self
            .drag_image(ctx, self.drawn_scale)
            .unwrap_or_else(|| (Canvas::new(0, 0), (0, 0)));
        let d = self.drag.as_mut().expect("still armed");
        d.started = true;
        Reply::start_drag(DragOut {
            paths: d.paths.clone(),
            image,
            grab,
        })
    }

    /// The dragged entries' image drawn at `scale`, and the point of it the pointer holds. An
    /// icon file not yet decoded at `scale` is the listing's resampled until its decode lands
    /// (`wake` asks for the image again).
    pub(crate) fn drag_image(&mut self, ctx: &mut Ctx, scale: i32) -> Option<(Canvas, (i32, i32))> {
        let d = self.drag.as_ref()?;
        let s = scale.max(1);
        let lay = self.layout(ctx);
        let sl = self.view_layout(lay.view, ctx);
        let flat = matches!(self.sv.mode, Mode::List | Mode::Details);
        let grown_rows = flat && sl.controls.text.len(shellview::ICON_PX) >= 32;
        let (px, icon_of): (i32, fn(&shellview::Entry) -> _) = match self.sv.mode {
            Mode::Tiles => (48, |e| e.icon48.clone()),
            Mode::Thumbnails | Mode::Icons => (32, |e| e.icon32.clone()),
            Mode::List | Mode::Details if grown_rows => (32, |e| e.icon32.clone()),
            Mode::List | Mode::Details => (16, |e| e.icon.clone()),
        };
        // A List or Details image holds only the rows on screen, so a large selection fetches no icons it won't draw.
        let shown: Vec<(usize, (i32, i32))> = d
            .items
            .iter()
            .filter_map(|&i| sl.cells.get(i).map(|c| (i, *c)))
            .filter(|(_, c)| !flat || !c.intersect(&sl.body).is_empty())
            .map(|(i, c)| (i, (c.x, c.y)))
            .collect();
        let drawn = ctx.icons.scale();
        ctx.icons.set_scale(s);
        let icons: Vec<Option<Canvas>> = shown
            .iter()
            .map(|&(i, _)| {
                let e = self.sv.entries.get(i)?;
                let thumb = e
                    .thumb
                    .as_ref()
                    .filter(|_| self.sv.mode == Mode::Thumbnails);
                let mut icon = match thumb {
                    Some(t) => device_px(t.clone(), s),
                    None => {
                        let worn = icon_of(e)?;
                        let icon = match worn.scale() == s {
                            true => None,
                            false => self.list_icons.fetch(&worn, px, &mut ctx.icons),
                        };
                        device_px(icon.unwrap_or_else(|| (*worn).clone()), s)
                    }
                };
                if e.cut {
                    icon.fade(1, 2);
                }
                Some(icon)
            })
            .collect();
        ctx.icons.set_scale(drawn);
        self.list_icons.decode_misses(s, &ctx.wake);
        let mut theme = ctx.theme.scaled(s);
        if self.sv.mode != Mode::Icons {
            let rows: Vec<shellview::DragRow> = shown
                .iter()
                .zip(&icons)
                .map(|(&(index, _), icon)| shellview::DragRow {
                    index,
                    icon: icon.as_ref(),
                })
                .collect();
            let (image, grab) =
                shellview::drag_image(&theme, &ctx.text, &self.sv, &sl, &rows, d.origin)?;
            return Some((image.at_scale(s), grab));
        }
        let ghosts: Vec<Ghost> = shown
            .iter()
            .zip(&icons)
            .filter_map(|(&(i, at), icon)| {
                let e = self.sv.entries.get(i)?;
                Some(Ghost {
                    at,
                    name: &e.name,
                    icon: icon.as_ref(),
                    shortcut: e.link,
                })
            })
            .collect();
        // `draw_cell`'s unselected label is drawn for the desktop, where
        // the text is white; in the view it sits on white, so the colour
        // is swapped the way `shellgrid` swaps it for the same call.
        theme.colors.desktop_text = theme.colors.window_text;
        let cell = iconview::Cell::grown(ctx.theme.text);
        let (image, grab) = iconview::drag_image(&theme, &ctx.text, &ghosts, cell, d.origin)?;
        Some((image.at_scale(s), grab))
    }

    pub(crate) fn drag_release(&mut self, x: i32, y: i32, ctx: &mut Ctx) -> Reply {
        let Some(d) = self.drag.take() else {
            return Reply::none();
        };
        // A started drag is the compositor's, so this release is one it
        // refused to start; nothing is left to do.
        if d.started {
            return Reply::none();
        }
        if d.button != BTN_LEFT {
            return self.item_menu_at(x, y, ctx);
        }
        if let Some((i, click)) = d.pending_click {
            self.sv.selection.click(i, click);
            self.sv.focus_ring = true;
            self.refresh_chrome(ctx);
        }
        Reply::redraw()
    }

    pub(crate) fn drag_ended(&mut self) -> Reply {
        self.drag = None;
        Reply::none()
    }

    pub(crate) fn item_menu_at(&mut self, x: i32, y: i32, ctx: &mut Ctx) -> Reply {
        let lay = self.layout(ctx);
        let sl = self.view_layout(lay.view, ctx);
        match shellview::hit(&self.sv, &sl, x, y) {
            shellview::Hit::Entry(i) => self.entry_menu_at(i, (x, y), ctx, false),
            _ => {
                let menu = crate::menus::background_menu(&self.menu_state());
                self.popup = Some(Popup::new((x, y), menu, lay.client, ctx, false));
                Reply::redraw()
            }
        }
    }

    pub(crate) fn entry_menu_at(
        &mut self,
        i: usize,
        at: (i32, i32),
        ctx: &mut Ctx,
        by_keyboard: bool,
    ) -> Reply {
        let lay = self.layout(ctx);
        if !self.sv.selection.set.contains(&i) {
            self.sv.selection.set_only(Some(i));
        }
        use crate::fileops::PasteTo;
        use crate::menus::ItemKind;
        let item = self.items.get(i);
        self.menu_paste = item.and_then(|it| match it.shortcut.as_deref() {
            _ if it.is_dir => Some(PasteTo::Folder(it.path.clone())),
            Some(reprisefs::apps::Shortcut::App(app)) => Some(PasteTo::App(app.clone())),
            _ => None,
        });
        let kind = match self.menu_paste {
            Some(PasteTo::Folder(_)) => ItemKind::Folder,
            Some(PasteTo::App(_)) => ItemKind::Launcher,
            _ => ItemKind::File,
        };
        let open_with = if kind == ItemKind::File {
            self.build_open_with(i, ctx)
        } else {
            Vec::new()
        };
        let send_to = self.build_send_to(ctx);
        let eject = self.ejectable(i);
        let menu = crate::menus::item_menu(&self.menu_state(), kind, eject, &open_with, &send_to);
        self.popup = Some(Popup::new(at, menu, lay.client, ctx, by_keyboard));
        Reply::redraw()
    }

    pub(crate) fn cancel_item_drag(&mut self) -> bool {
        if self.drag.as_ref().is_some_and(|d| !d.started) {
            self.drag = None;
            return true;
        }
        false
    }

    pub(crate) fn drag_enter(
        &mut self,
        x: i32,
        y: i32,
        offered: Offered,
        own: bool,
        ctx: &Ctx,
    ) -> Reply {
        let paths = self
            .drag
            .as_ref()
            .filter(|d| own && d.started)
            .map(|d| d.paths.clone());
        self.drop_facts.clear();
        self.incoming = Some(Incoming {
            offered,
            own,
            paths,
            at: (x, y),
        });
        self.drag_answer(x, y, ctx)
    }

    pub(crate) fn drag_files(&mut self, paths: Vec<PathBuf>, ctx: &Ctx) -> Reply {
        let Some(inc) = self.incoming.as_mut() else {
            return Reply::none();
        };
        if inc.paths.is_none() {
            inc.paths = Some(paths);
        }
        let (x, y) = inc.at;
        self.drag_answer(x, y, ctx)
    }

    pub(crate) fn drag_over(&mut self, x: i32, y: i32, ctx: &Ctx) -> Reply {
        let Some(inc) = self.incoming.as_mut() else {
            return Reply::none();
        };
        inc.at = (x, y);
        self.drag_answer(x, y, ctx)
    }

    pub(crate) fn drag_leave(&mut self) -> Reply {
        self.incoming = None;
        self.address.combo.drag_leave();
        self.sv.drop_target = None;
        self.tree.drop_hilight = None;
        Reply::redraw()
    }

    fn drag_answer(&mut self, x: i32, y: i32, ctx: &Ctx) -> Reply {
        let Some(offered) = self.incoming.as_ref().map(|i| i.offered) else {
            return Reply::none();
        };
        if let Some(kind) = fielddrop::kind_for(offered)
            && self.address_drag(x, y, ctx)
        {
            self.sv.drop_target = None;
            self.tree.drop_hilight = None;
            return Reply {
                redraw: true,
                tick: self.address_scroll(ctx).is_some(),
                ..Reply::drag_accept(Some(fielddrop::accept_as(kind, Actions::default())))
            };
        }
        let paths = self.incoming.as_ref().and_then(|i| i.paths.clone());
        let over = drop_kind(offered)
            .and_then(|_| self.drag_target(x, y, paths.as_deref().unwrap_or(&[]), ctx));
        let dir = over.and_then(|t| self.target_dir(&t));
        let accept = dir.as_deref().map(|dir| {
            let files = paths.as_deref().unwrap_or(&[]);
            accept_for(
                offered,
                paths.as_deref(),
                self.facts(files, dir, ctx).unwrap_or_default().on_device,
            )
        });
        let (entry, row) = match over.filter(|_| accept.is_some()) {
            Some(Target::ViewEntry(i)) => (Some(i), None),
            Some(Target::TreeRow(i)) => (None, Some(i)),
            _ => (None, None),
        };
        let redraw = self.sv.drop_target != entry || self.tree.drop_hilight != row;
        self.sv.drop_target = entry;
        self.tree.drop_hilight = row;
        Reply {
            redraw,
            ..Reply::drag_accept(accept)
        }
    }

    pub(crate) fn drag_drop(
        &mut self,
        (x, y): (i32, i32),
        data: Dropped,
        action: DragAction,
        source: Actions,
        ctx: &mut Ctx,
    ) -> Reply {
        let (link, own) = self
            .incoming
            .as_ref()
            .map_or((false, false), |i| (i.offered.link, i.own));
        let (action, source) = match link {
            true => (
                match action {
                    DragAction::Move => DragAction::Copy,
                    a => a,
                },
                Actions {
                    move_: false,
                    ..source
                },
            ),
            false => (action, source),
        };
        if self.address.combo.drop_x.is_some() {
            return self.address_drop(&data, action, ctx);
        }
        self.drag_leave();
        let (paths, links) = split_drop(data);
        let dir = self
            .drag_target(x, y, &paths, ctx)
            .and_then(|t| self.target_dir(&t));
        let asks = action == DragAction::Ask;
        let Some(dir) = dir else {
            return nothing(asks);
        };
        let links = if source.copy { links } else { Vec::new() };
        if paths.is_empty() {
            if links.is_empty() {
                return nothing(asks);
            }
            let reply = self.drop_links(links, &dir, ctx);
            return match asks {
                true => Reply {
                    answer_drop: Some(Some(DragAction::Copy)),
                    ..reply
                },
                false => reply,
            };
        }
        let op = match action {
            DragAction::Move => DropOp::Move,
            DragAction::Copy => DropOp::Copy,
            DragAction::Ask => {
                // Our own drag asks with the left button only for a
                // shortcut; the compositor asks for the right one too.
                let shortcut = own && self.drag.as_ref().is_some_and(|d| d.button == BTN_LEFT);
                let on_device = self.facts(&paths, &dir, ctx).unwrap_or_default().on_device;
                let default = ask_default(shortcut, source.move_ && on_device);
                let menu = crate::menus::drop_menu(default, source);
                let client = self.layout(ctx).client;
                self.pending_drop = Some(PendingDrop {
                    paths,
                    links,
                    dir,
                    source,
                });
                self.popup = Some(Popup::new((x, y), menu, client, ctx, false));
                return Reply::redraw();
            }
        };
        self.land(paths, links, &dir, op, ctx)
    }

    fn land(
        &mut self,
        paths: Vec<PathBuf>,
        links: Vec<WebLink>,
        dir: &Path,
        op: DropOp,
        ctx: &mut Ctx,
    ) -> Reply {
        let mut reply = Reply::redraw();
        if !links.is_empty() {
            reply = self.drop_links(links, dir, ctx);
        }
        if op == DropOp::Move && self.in_place(&paths, dir, ctx) {
            return reply;
        }
        let put = self.drop_into(&paths, dir, op, ctx);
        Reply {
            redraw: reply.redraw || put.redraw,
            ..put
        }
    }

    pub(crate) fn drop_pending(&mut self, op: DropOp, ctx: &mut Ctx) -> Reply {
        let Some(p) = self.pending_drop.take() else {
            return Reply::none();
        };
        let in_place = op == DropOp::Move && self.in_place(&p.paths, &p.dir, ctx);
        let answer = if in_place {
            answer_for(DropOp::Copy, p.source)
        } else {
            answer_for(op, p.source)
        };
        let Some(answer) = answer else {
            return Reply::answer_drop(None);
        };
        Reply {
            answer_drop: Some(Some(answer)),
            ..self.land(p.paths, p.links, &p.dir, op, ctx)
        }
    }

    pub(crate) fn cancel_pending_drop(&mut self) -> Reply {
        match self.pending_drop.take() {
            Some(_) => Reply::answer_drop(None),
            None => Reply::none(),
        }
    }

    pub(crate) fn drop_cancelled(&mut self) -> Reply {
        self.pending_drop = None;
        if self.drop_menu_up() {
            self.popup = None;
        }
        Reply::redraw()
    }

    /// The pointer left the window or the keyboard went elsewhere: a drop
    /// menu up closes, and [`Explorer::settle_drop`] cancels its drop.
    /// After a drop from another program the keyboard stays with that
    /// program, so Escape cannot reach the menu.
    pub(crate) fn close_drop_menu(&mut self) -> bool {
        if self.drop_menu_up() {
            self.popup = None;
            return true;
        }
        false
    }

    fn drop_menu_up(&self) -> bool {
        self.popup
            .as_ref()
            .is_some_and(|p| matches!(p.command(&[0]), Command::Drop(_)))
    }

    pub(crate) fn settle_drop(&mut self, reply: Reply) -> Reply {
        if self.pending_drop.is_none() || self.drop_menu_up() {
            return reply;
        }
        self.pending_drop = None;
        Reply {
            answer_drop: Some(None),
            ..reply
        }
    }

    pub(crate) fn drop_into(
        &mut self,
        paths: &[PathBuf],
        dir: &Path,
        op: DropOp,
        ctx: &mut Ctx,
    ) -> Reply {
        let select = self.place.path().as_deref() == Some(dir);
        self.put_all(paths, dir, &|_| Some(op), select, ctx)
    }

    fn drop_links(&mut self, links: Vec<WebLink>, dir: &Path, ctx: &mut Ctx) -> Reply {
        let dir = dir.to_path_buf();
        let at = dir.clone();
        let work = move |gate: &Gate| match gate.change(&at) {
            true => write_links(&links, &at),
            false => Ok(()),
        };
        let title = DropOp::Shortcut.failure_title();
        let acted = self.act(
            ctx,
            crate::acting::Action::changes(title),
            work,
            move |ex, result, ctx| {
                let reply = if ex.place.path().as_deref() == Some(&*dir) {
                    ex.refresh(ctx)
                } else {
                    Reply::redraw()
                };
                match result {
                    Ok(()) => reply,
                    Err(text) => ex.message_box(title, text, ctx),
                }
            },
        );
        Reply::redraw().and(acted)
    }

    fn address_layout(&self, ctx: &Ctx) -> Option<addressbar::Layout> {
        let r = self.layout(ctx).address?;
        let px = ctx.theme.fonts.size_px;
        let cm = ctx.theme.controls();
        Some(addressbar::layout(&self.address, &ctx.text, px, r, &cm))
    }

    /// A drag at (`x`, `y`) over the address field shows its drop caret there: `true` when the
    /// field takes it.
    fn address_drag(&mut self, x: i32, y: i32, ctx: &Ctx) -> bool {
        match self.address_layout(ctx) {
            Some(al) => self.address.combo.drag_over(&al.combo, x, y),
            None => {
                self.address.combo.drag_leave();
                false
            }
        }
    }

    fn address_scroll(&self, ctx: &Ctx) -> Option<usize> {
        let al = self.address_layout(ctx)?;
        let px = ctx.theme.fonts.size_px;
        self.address.combo.drag_scroll(&al.combo, &ctx.text, px)
    }

    /// The tick while a drag rests on the address field: a step at its edge.
    pub(crate) fn address_tick(&mut self, ctx: &Ctx) -> Reply {
        let (Some(_), Some(al)) = (self.address.combo.drop_x, self.address_layout(ctx)) else {
            return Reply::none();
        };
        let px = ctx.theme.fonts.size_px;
        if !self.address.combo.drag_tick(&al.combo, &ctx.text, px) {
            return Reply::none();
        }
        Reply {
            tick: self.address_scroll(ctx).is_some(),
            ..Reply::redraw()
        }
    }

    /// A drop on the address field: its text, or the files' paths, go in at the drop caret and
    /// the field takes the keyboard.
    fn address_drop(&mut self, data: &Dropped, action: DragAction, ctx: &Ctx) -> Reply {
        let took = self.address_layout(ctx).is_some_and(|al| {
            let px = ctx.theme.fonts.size_px;
            let line = fielddrop::line(data);
            self.address
                .combo
                .drop_text(&al.combo, &line, &ctx.text, px)
        });
        self.drag_leave();
        if took {
            self.focus = crate::Focus::Address;
        }
        Reply {
            answer_drop: (action == DragAction::Ask).then_some(took.then_some(DragAction::Copy)),
            ..Reply::redraw()
        }
    }

    /// Whether a move of `paths` into `dir` leaves them where they are; by their names while the
    /// [`Facts`] are unknown.
    fn in_place(&mut self, paths: &[PathBuf], dir: &Path, ctx: &Ctx) -> bool {
        self.facts(paths, dir, ctx)
            .map_or_else(|| named_in_place(paths, dir), |f| f.in_place)
    }

    /// [`Facts`] of `paths` against `dir`, read once per folder a drag rests on: the first ask waits
    /// the listing wait, later ones take a late answer once it has come. At most [`FACTS_OUT`]
    /// reads are out at once; past that a new folder answers `None`.
    fn facts(&mut self, paths: &[PathBuf], dir: &Path, ctx: &Ctx) -> Option<Facts> {
        if paths.is_empty() {
            return Some(Facts::default());
        }
        for f in &mut self.drop_facts {
            if let Err(job) = &f.read
                && let Some(read) = job.take()
            {
                f.read = Ok(read);
            }
        }
        if let Some(f) = self
            .drop_facts
            .iter()
            .find(|f| f.dir == dir && f.paths == paths)
        {
            return f.read.as_ref().ok().copied();
        }
        if self.drop_facts.iter().filter(|f| f.read.is_err()).count() >= FACTS_OUT {
            return None;
        }
        let (p, d) = (paths.to_vec(), dir.to_path_buf());
        let read = self.read_job(ctx, move || Facts {
            on_device: all_on_device(&p, &d),
            in_place: lands_in_place(&p, &d),
        });
        let facts = read.as_ref().ok().copied();
        self.drop_facts.push(FactsRead {
            paths: paths.to_vec(),
            dir: dir.to_path_buf(),
            read,
        });
        facts
    }

    pub(crate) fn drag_target(
        &self,
        x: i32,
        y: i32,
        paths: &[PathBuf],
        ctx: &Ctx,
    ) -> Option<Target> {
        let lay = self.layout(ctx);
        let px = ctx.theme.fonts.size_px;
        if lay.view.contains(x, y) {
            // A read-only place's entries take no drop: a drive under
            // Computer is a directory with a mount path, and a drop there
            // would write into the mount. The tree branch checks the same
            // through `writable_dir`.
            let here = writable_dir(&self.place)?;
            let sl = self.view_layout(lay.view, ctx);
            let shellview::Hit::Entry(i) = shellview::hit(&self.sv, &sl, x, y) else {
                return (!covers(paths, &here)).then_some(Target::Background);
            };
            let it = self.items.get(i)?;
            let ok = it.is_dir && !it.is_link && !covers(paths, &it.path);
            return ok.then_some(Target::ViewEntry(i));
        }
        let (Some(bl), Some(frame)) = (&lay.bar, lay.tree) else {
            return None;
        };
        if !bl.body.contains(x, y) {
            return None;
        }
        let tl = treeview::layout(&self.tree, &ctx.text, px, frame, &ctx.theme.controls());
        let treeview::Hit::Row(i) = self.tree.hit(&tl, x, y) else {
            return None;
        };
        let place = self
            .tree
            .visible()
            .get(i)
            .and_then(|r| self.place_of(&r.path))?;
        let dir = writable_dir(&place)?;
        // A folder cannot be dropped into itself or into its own tree.
        (!covers(paths, &dir)).then_some(Target::TreeRow(i))
    }

    fn target_dir(&self, t: &Target) -> Option<PathBuf> {
        match *t {
            Target::ViewEntry(i) => self.items.get(i).map(|it| it.path.clone()),
            Target::TreeRow(i) => self
                .tree
                .visible()
                .get(i)
                .and_then(|r| self.place_of(&r.path))
                .as_ref()
                .and_then(writable_dir),
            Target::Background => writable_dir(&self.place),
        }
    }
}

fn nothing(asks: bool) -> Reply {
    if asks {
        Reply::answer_drop(None)
    } else {
        Reply::redraw()
    }
}

fn covers(paths: &[PathBuf], path: &Path) -> bool {
    paths.iter().any(|p| path.starts_with(p))
}

fn writable_dir(place: &Place) -> Option<PathBuf> {
    (!place.read_only()).then(|| place.path()).flatten()
}

#[cfg(test)]
#[path = "drag_tests.rs"]
mod tests;

/// `icon` as `s` device px per logical px, one canvas px each: what a theme scaled to `s` draws.
fn device_px(icon: Canvas, s: i32) -> Canvas {
    let k = icon.scale().max(1);
    let (dw, dh) = icon.device_size();
    let flat = icon.at_scale(1);
    if k == s {
        return flat;
    }
    flat.resample_box(dw * s as u32 / k as u32, dh * s as u32 / k as u32)
}
