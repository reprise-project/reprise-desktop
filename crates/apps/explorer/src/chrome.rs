//! The window's chrome: the toolbar and the address drop as built once, and the panels, tasks, latches
//! and greyed menu rows that every navigation and selection change move.

use crate::icons::chrome_icon_unread;
use crate::place::Place;
use crate::window;
use crate::{Explorer, FOLDERS_ITEM};
use repriseapp::Ctx;
use repriseapp::iconjobs::IconWaits;
use repriseapp::listjob::Job;
use reprisefs::listing::Item;
use reprisefs::places::{DriveKind, Gadget};
use reprisekit::art::viewsicon;
use reprisekit::controls::addressbar::{self, AddressBar};
use reprisekit::controls::combo::{self, ListItem};
use reprisekit::controls::statusbar;
use reprisekit::controls::toolbar;
use reprisekit::icontheme::IconCache;
use reprisekit::raster::Canvas;
use reprisekit::theme::TextUnits;
use reprisekit::tr;
use std::path::PathBuf;

/// A selected shortcut's path and its status text, or the read of it still out.
pub(crate) type LinkStatus = (PathBuf, Result<Option<String>, Job<Option<String>>>);

/// `place`'s address bar icon at text `u`, a blank stand-in noted in `waits` while it decodes.
pub(crate) fn address_icon(
    waits: &mut IconWaits,
    icons: &mut IconCache,
    place: &Place,
    u: TextUnits,
) -> Option<Canvas> {
    waits.first(icons, &[place.icon_name()], addressbar::field_icon_px(u))
}

impl Explorer {
    pub(crate) fn build_toolbar(&mut self, ctx: &mut Ctx) {
        let px = toolbar::big_icon(ctx.theme.text);
        let (waits, icons) = (&mut self.chrome_waits, &mut ctx.icons);
        let mut icon = |names: &[&str]| chrome_icon_unread(waits, icons, names, px);
        let (back, forward, up, folders) = (
            icon(&["go-previous"]),
            icon(&["go-next"]),
            icon(&["go-up"]),
            icon(&["folder"]),
        );
        // Few themes carry a view icon, so the drawn one stands in rather than the placeholder.
        let views = (waits.first(icons, &["view-list"], px)).or_else(|| Some(viewsicon::draw24()));
        self.toolbar.items = vec![
            toolbar::Item::DropButton {
                id: toolbar::BACK,
                icon: back,
                label: Some(tr!("chrome-build-toolbar-back")),
                enabled: false,
            },
            toolbar::Item::DropButton {
                id: toolbar::FORWARD,
                icon: forward,
                label: Some(String::new()),
                enabled: false,
            },
            toolbar::Item::Button {
                id: toolbar::UP,
                icon: up,
                label: None,
                enabled: true,
            },
            toolbar::Item::Separator,
            toolbar::Item::Button {
                id: toolbar::FOLDERS,
                icon: folders,
                label: Some(tr!("chrome-build-toolbar-folders")),
                enabled: true,
            },
            toolbar::Item::Separator,
            toolbar::Item::DropButton {
                id: toolbar::VIEWS,
                icon: views,
                label: None,
                enabled: true,
            },
        ];
    }

    fn go_icon(&mut self, ctx: &mut Ctx) -> Option<Canvas> {
        let names = &["go-jump", "go-next"];
        let px = addressbar::go_icon_px(ctx.theme.text);
        chrome_icon_unread(&mut self.chrome_waits, &mut ctx.icons, names, px)
    }

    /// The address drop's places: the shell tree, one row a place, the Desktop's places in the Folders tree's order with the drives under Computer.
    /// The field names `self.place`, which a window still reading its first folder already holds.
    pub(crate) fn build_address(&mut self, ctx: &mut Ctx) {
        let (rows, places) = self.address_rows(ctx);
        let go = self.go_icon(ctx);
        self.address = AddressBar::new(rows, 0, go);
        self.address_places = places;
        let icon = address_icon(
            &mut self.chrome_waits,
            &mut ctx.icons,
            &self.place,
            ctx.theme.text,
        );
        let text = self.place.address_in(&self.shares.shares);
        self.reset_address(&text, icon);
    }

    fn address_rows(&mut self, ctx: &mut Ctx) -> (Vec<ListItem>, Vec<Place>) {
        let mut shell: Vec<(String, Place, i32, &'static str)> = vec![(
            tr!("chrome-address-rows-desktop"),
            Place::Desktop,
            0,
            Place::Desktop.icon_name(),
        )];
        for p in crate::place::desktop_places() {
            let computer = p == Place::Computer;
            let icon = p.icon_name();
            shell.push((p.title(), p, 1, icon));
            if computer {
                for d in &self.drives {
                    shell.push((d.name(), Place::Folder(d.mount.clone()), 2, d.kind().icon()));
                }
                for sh in &self.shares.shares {
                    let kind = match sh.is_portable() {
                        true => DriveKind::Portable(Gadget::from_icon(&sh.icon)),
                        false => DriveKind::Network,
                    };
                    let place = Place::Folder(sh.path.clone());
                    shell.push((sh.name.clone(), place, 2, kind.icon()));
                }
            }
        }
        let mut rows = Vec::new();
        let mut places = Vec::new();
        let px = combo::row_icon_px(ctx.theme.text);
        for (text, place, indent, icon) in shell {
            let icon = self.chrome_waits.first(&mut ctx.icons, &[icon], px);
            rows.push(ListItem::shell(text, icon, indent));
            places.push(place);
        }
        (rows, places)
    }

    /// The toolbar, address bar, Folders tree, status bar and task pane fetch their icons again
    /// from `ctx.icons`, now at another scale; what they show and their states stay.
    pub(crate) fn rescale_chrome(&mut self, ctx: &mut Ctx) {
        self.rewear_chrome(ctx);
        crate::foldertree::refetch_icons(&mut self.tree, &mut self.tree_places, &mut ctx.icons);
        self.refill_tree_art(ctx);
    }

    /// The toolbar, address bar, status bar and task pane fetch their icons again: the names
    /// their stand-ins waited on landed.
    pub(crate) fn rewear_chrome(&mut self, ctx: &mut Ctx) {
        self.chrome_waits = IconWaits::default();
        self.build_toolbar(ctx);
        let (rows, _) = self.address_rows(ctx);
        for (item, row) in self.address.combo.items.iter_mut().zip(rows) {
            item.icon = row.icon;
        }
        self.address.combo.icon = address_icon(
            &mut self.chrome_waits,
            &mut ctx.icons,
            &self.place,
            ctx.theme.text,
        );
        self.address.go_icon = self.go_icon(ctx);
        let px = crate::fileops::menu_icon_px(&ctx.theme);
        self.favorite_icons = [&["folder"][..], &["folder-remote", "network-server"]]
            .map(|names| self.chrome_waits.first(&mut ctx.icons, names, px));
        self.pane_key = None;
        self.refresh_chrome(ctx);
    }

    /// The status panels, task pane and toolbar's enabled states, which every navigation and selection
    /// change moves. Menus are not rebuilt here: every key and press syncs them before the bar can open.
    pub(crate) fn refresh_chrome(&mut self, ctx: &mut Ctx) {
        let chosen: Vec<usize> = self
            .sv
            .selection
            .set
            .iter()
            .copied()
            .filter(|&i| i < self.items.len())
            .collect();
        // With nothing selected the panels count the whole folder; with a selection they count it instead.
        let sized: Box<dyn Iterator<Item = &Item>> = if chosen.is_empty() {
            Box::new(self.items.iter())
        } else {
            Box::new(chosen.iter().filter_map(|&i| self.items.get(i)))
        };
        // Computer leaves the size panel blank; a folder prints its total, "0 bytes" included.
        let bytes: Option<u64> = self
            .place
            .path()
            .map(|_| sized.filter(|i| !i.is_folder()).map(|i| i.size).sum());
        let px = statusbar::icon_px(ctx.theme.text);
        let zone = self.chrome_waits.first(&mut ctx.icons, &["computer"], px);
        self.status.panels = window::status_panels(
            self.items.len(),
            chosen.len(),
            bytes,
            &tr!("chrome-refresh-chrome-computer"),
            zone,
        );
        if chosen.is_empty()
            && self.search.shown().is_none()
            && let Some(free) = (self.place.path()).and_then(|d| self.folder_free.of(&d))
        {
            self.status.panels[0].text = tr!(
                "status-objects-free",
                objects = self.items.len(),
                free = window::short_size(free)
            );
        }
        if self.search.no_tool() {
            self.status.panels[0].text = reprisefs::search::no_tool();
        }
        // One item picked in the Recycle Bin says where it came from, a search hit the folder it is
        // in, one shortcut elsewhere where it points.
        if let [i] = chosen[..]
            && let Some(it) = self.items.get(i)
        {
            if self.place == Place::RecycleBin {
                self.status.panels[0].text =
                    window::bin_status_text(it, reprisefs::locale::is_12h());
            } else if let Some(text) = self.hit_folder(it) {
                self.status.panels[0].text = text;
            } else if (it.is_link || it.shortcut.is_some())
                && let Some(text) = self.link_status(it.path.clone(), ctx)
            {
                self.status.panels[0].text = text;
            }
        }

        let (back, fwd, up) = (
            self.can_back(),
            self.history.can_forward(),
            self.place.parent_in(&self.drive_mounts).is_some(),
        );
        for (i, on) in [(0, back), (1, fwd), (2, up)] {
            match &mut self.toolbar.items[i] {
                toolbar::Item::Button { enabled, .. }
                | toolbar::Item::DropButton { enabled, .. } => *enabled = on,
                toolbar::Item::Separator => {}
            }
        }
        self.toolbar.latched = self.settings.folders.then_some(FOLDERS_ITEM);
        self.build_taskpane(ctx);
    }

    /// "In Folder: <dir>" for a search hit, which the status bar and infotip lead with: results
    /// from several folders can share a name.
    pub(crate) fn hit_folder(&self, it: &Item) -> Option<String> {
        self.search.shown()?;
        let dir = it.path.parent()?;
        Some(tr!("status-in-folder", dir = dir.display().to_string()))
    }

    /// Where the shortcut at `path` points, for the status bar, read on a listing thread. While a
    /// read is out, for any shortcut in the listing, no other starts: a folder of shortcuts on a
    /// dead mount holds one thread. [`Explorer::take_link_status`] takes the answer.
    fn link_status(&mut self, path: PathBuf, ctx: &Ctx) -> Option<String> {
        match &self.link_status {
            Some((at, Ok(text))) if *at == path => return text.clone(),
            Some((_, Err(_))) => return None,
            _ => {}
        }
        let (at, globs) = (path.clone(), self.globs.clone());
        let read = self.read_job(ctx, move || {
            let l = reprisefs::launcher::Launcher::at(&at, &globs)?;
            window::shortcut_status_text(&l, &at)
        });
        let text = read.as_ref().ok().cloned().flatten();
        self.link_status = Some((path, read));
        text
    }

    /// A shortcut's status read that answered late puts its text up; true when it did.
    pub(crate) fn take_link_status(&mut self, ctx: &mut Ctx) -> bool {
        let Some((at, Err(job))) = &self.link_status else {
            return false;
        };
        let Some(text) = job.take() else {
            return false;
        };
        self.link_status = Some((at.clone(), Ok(text)));
        self.refresh_chrome(ctx);
        true
    }
}
