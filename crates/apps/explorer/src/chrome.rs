//! The window's chrome: the toolbar and the address drop as built once, and the panels, tasks, latches
//! and greyed menu rows that every navigation and selection change move.

use crate::commands::{Command, Menu};
use crate::foldertree;
use crate::icons::chrome_icon_unread;
use crate::place::{Place, drive_mounts};
use crate::window::{self, Popup};
use crate::{Explorer, FOLDERS_ITEM};
use repriseapp::iconjobs::IconWaits;
use repriseapp::listjob::Job;
use repriseapp::{Ctx, Reply};
use reprisefs::listing::Item;
use reprisefs::netshare::Share;
use reprisefs::places::{Drive, DriveKind, Gadget};
use reprisekit::art::viewsicon;
use reprisekit::controls::addressbar::{self, AddressBar};
use reprisekit::controls::breadcrumb::Crumb;
use reprisekit::controls::combo::{self, ListItem};
use reprisekit::controls::commandbar::{self, Commandbar, Drop, RightItem};
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::statusbar;
use reprisekit::controls::toolbar;
use reprisekit::icontheme::IconCache;
use reprisekit::raster::{Canvas, Rect};
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
        let scale = icons.scale();
        let views = (waits.first(icons, &["view-list"], px))
            .or_else(|| Some(viewsicon::draw24_grown(px, scale)));
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
        self.special_icons = (self.specials.iter())
            .map(|&(_, name)| {
                (
                    name,
                    self.chrome_waits
                        .first(&mut ctx.icons, &[name, "folder"], px),
                )
            })
            .collect();
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
        (self.nav.can_back, self.nav.can_forward) = (back, fwd);
        if self.shell_on {
            let facts = self.facts_for(&mut ctx.icons);
            self.details.set(facts);
        }
        self.toolbar.latched = self.settings.folders.then_some(FOLDERS_ITEM);
        self.build_taskpane(ctx);
        let (items, acts) = commands_for(&self.bar_facts()).into_iter().unzip();
        self.cmdbar.set_left(items);
        self.cmdbar_acts = acts;
    }

    /// What the command bar offers commands for: the place and the selection.
    pub(crate) fn bar_facts(&self) -> BarFacts {
        let place = match &self.place {
            Place::Computer => BarPlace::Computer,
            Place::RecycleBin => BarPlace::RecycleBin(self.items.len()),
            Place::Network(_) => BarPlace::Network,
            Place::Archive(..) => BarPlace::Archive,
            _ => BarPlace::Folder,
        };
        let chosen: Vec<usize> = (self.sv.selection.set.iter())
            .copied()
            .filter(|&i| i < self.items.len())
            .collect();
        let picked = match chosen[..] {
            [] => Picked::Nothing,
            [i] if self.place == Place::Computer => Picked::Drive {
                eject: (self.ejectable(i) || self.share_selected(i))
                    .then(|| self.share_selected(i)),
            },
            [i] if self.items[i].is_folder() => Picked::Folder,
            [_] => Picked::File {
                image: self.selected_disk_image().is_some(),
                archive: self.extract_label().is_some(),
            },
            _ => Picked::Several {
                archives: self.extract_label().is_some(),
            },
        };
        BarFacts {
            place,
            picked,
            can_connect: self.shares.gio,
        }
    }

    /// A press on the command bar's `part`, or Enter or Down on it (`by_keyboard`), the bar lying
    /// over `bar`: a command runs, a drop or Organize opens its menu under it, » opens the folded
    /// commands, Views cycles or lists the modes. The help mark does nothing.
    pub fn command_bar_press(
        &mut self,
        part: commandbar::Hit,
        bar: Rect,
        by_keyboard: bool,
        ctx: &mut Ctx,
    ) -> Reply {
        use commandbar::Hit;
        let l = self.cmdbar_layout(bar, ctx);
        let (menu, waits) = match part {
            Hit::Item(i) => match self.cmdbar_acts.get(i).cloned() {
                Some(BarAct { menu: Some(m), .. })
                    if self
                        .cmdbar
                        .left
                        .get(i)
                        .is_some_and(|it| it.drop == Drop::Menu) =>
                {
                    self.bar_menu(m, ctx)
                }
                Some(act) => return self.run(act.run, ctx),
                None => return Reply::none(),
            },
            Hit::ItemDrop(i) => match self.cmdbar_acts.get(i).and_then(|a| a.menu) {
                Some(m) => self.bar_menu(m, ctx),
                None => return Reply::none(),
            },
            Hit::More => {
                let mut m = Menu::default();
                for i in self.cmdbar.folded(&l) {
                    let (Some(item), Some(act)) =
                        (self.cmdbar.left.get(i), self.cmdbar_acts.get(i))
                    else {
                        continue;
                    };
                    let row = reprisekit::controls::menu::MenuItem::entry(item.label.as_str())
                        .enabled(item.enabled);
                    match (item.drop, act.menu) {
                        (Drop::Menu, Some(sub)) => m.sub(row, self.bar_menu(sub, ctx).0),
                        _ => m.row(row, act.run.clone()),
                    }
                }
                (m, IconWaits::default())
            }
            Hit::ViewsCycle => {
                return self.run(Command::Mode(crate::commands::next_mode(self.sv.mode)), ctx);
            }
            Hit::ViewsDrop => (crate::menus::views_menu(self.sv.mode), IconWaits::default()),
            Hit::Help => return Reply::none(),
        };
        if menu.items().is_empty() {
            return Reply::none();
        }
        let Some(under) = part_rect(&l, part) else {
            return Reply::none();
        };
        let at = (under.x, under.bottom());
        let popup = Popup::new(at, menu, ctx, by_keyboard).waiting(waits);
        self.popup = Some(popup.for_bar(part));
        Reply::redraw()
    }

    /// `which` menu, and the row icons it waits on.
    fn bar_menu(&mut self, which: BarMenu, ctx: &mut Ctx) -> (Menu, IconWaits) {
        let mut waits = IconWaits::default();
        let menu = match which {
            BarMenu::Organize => crate::menus::organize_menu(&self.menu_state()),
            BarMenu::New => crate::menus::new_items_menu(&self.menu_state()),
            BarMenu::OpenWith => match self.sv.selection.single() {
                Some(i) => crate::menus::open_with_items(&self.build_open_with(i, &mut waits, ctx)),
                None => Menu::default(),
            },
        };
        (menu, waits)
    }

    /// The command bar laid out over `rect`, as it is drawn.
    pub fn cmdbar_layout(&self, rect: Rect, ctx: &Ctx) -> commandbar::Layout {
        let t = &ctx.theme;
        commandbar::layout(
            &self.cmdbar,
            rect,
            &ctx.text,
            t.fonts.size_px,
            &t.controls(),
        )
    }

    /// The bar as drawn: the part whose menu is open held.
    pub fn cmdbar_shown(&self) -> Commandbar {
        Commandbar {
            pressed: (self.popup.as_ref())
                .and_then(|p| p.from_bar)
                .or(self.cmdbar.pressed),
            ..self.cmdbar.clone()
        }
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

/// The places a breadcrumb walks down to `place`, given the mount points Computer lists, from the
/// nearest named root: Home for a folder under it, the Desktop or Documents folder for one under
/// them, else Computer and then the drive or share. A place with no path is its one shell place,
/// a server under Network. Nothing is read from disk.
pub(crate) fn crumb_places(place: &Place, mounts: &[PathBuf]) -> Vec<Place> {
    let mut chain = place.chain_in(mounts);
    if chain.len() > 1 && chain[0] == Place::Desktop {
        chain.remove(0);
    }
    chain
}

/// [`crumb_places`] as the breadcrumb's segments: a drive by its name, a share by its own, any
/// other place by its title; each one's target the text [`Place::read_address`] reads back as it.
pub(crate) fn crumbs_for(place: &Place, drives: &[Drive], shares: &[Share]) -> Vec<Crumb> {
    let mounts = drive_mounts(drives, shares);
    (crumb_places(place, &mounts).iter())
        .map(|p| Crumb {
            label: crumb_label(p, drives, shares),
            target: crumb_target(p, shares),
        })
        .collect()
}

fn crumb_label(p: &Place, drives: &[Drive], shares: &[Share]) -> String {
    match p {
        Place::Folder(at) => {
            (drives.iter().find(|d| d.mount == *at)).map_or_else(|| p.title_in(shares), Drive::name)
        }
        _ => p.title_in(shares),
    }
}

fn crumb_target(p: &Place, shares: &[Share]) -> String {
    match p {
        Place::Computer => "computer:///".to_string(),
        Place::RecycleBin => "trash:///".to_string(),
        _ if *p == Place::network() => "network:///".to_string(),
        Place::Home | Place::Desktop | Place::Documents => {
            (p.path()).map_or_else(|| p.address_in(shares), |d| d.display().to_string())
        }
        _ => p.address_in(shares),
    }
}

/// A breadcrumb chevron's menu whose subfolder read is still out ([`Explorer::open_crumb_menu`]).
pub(crate) struct CrumbMenu {
    /// The menu it opened with, which the answer replaces while it is still the one open.
    shown: Menu,
    at: (i32, i32),
    /// The next crumb's place, its row bold.
    current: Option<Place>,
    job: Job<Vec<(String, Place)>>,
}

/// The chevron menu: a row per subfolder, `current` bold; one greyed row when there are none.
fn crumb_menu(rows: &[(String, Place)], current: Option<&Place>) -> Menu {
    let mut m = Menu::default();
    for (name, place) in rows {
        let item = MenuItem::entry(name.clone()).bold(current == Some(place));
        m.row(item, Command::GoTo(place.clone()));
    }
    if rows.is_empty() {
        let empty = MenuItem::entry(tr!("chrome-crumb-menu-empty")).enabled(false);
        m.row(empty, Command::Nothing);
    }
    m
}

impl Explorer {
    /// Opens the menu of crumb `i`'s subfolders at `at`. The listing thread reads them, as it
    /// reads the view; the menu opens with them when the read answers within [`crate::LIST_WAIT`],
    /// else empty, taking them when it answers ([`Explorer::land_crumb_menu`]).
    pub(crate) fn open_crumb_menu(&mut self, i: usize, at: (i32, i32), ctx: &mut Ctx) -> Reply {
        let places = crumb_places(&self.place, &self.drive_mounts);
        let Some(place) = places.get(i).cloned() else {
            return Reply::none();
        };
        let current = places.get(i + 1).cloned();
        let world = self.tree_world();
        let read = move || foldertree::subfolders(&place, &world.inputs());
        self.crumb_menu = None;
        let menu = match self.read_job(ctx, read) {
            Ok(rows) => crumb_menu(&rows, current.as_ref()),
            Err(job) => {
                let shown = crumb_menu(&[], None);
                self.crumb_menu = Some(CrumbMenu {
                    shown: shown.clone(),
                    at,
                    current,
                    job,
                });
                shown
            }
        };
        self.popup = Some(window::Popup::new(at, menu, ctx, false));
        Reply::redraw()
    }

    /// A chevron menu's late read answered: the menu takes its rows if it is still the one open.
    /// True when it did.
    pub(crate) fn land_crumb_menu(&mut self, ctx: &Ctx) -> bool {
        let Some(rows) = self.crumb_menu.as_ref().and_then(|m| m.job.take()) else {
            return false;
        };
        let Some(m) = self.crumb_menu.take() else {
            return false;
        };
        if !self.popup.as_ref().is_some_and(|p| *p.menu() == m.shown) {
            return false;
        }
        let menu = crumb_menu(&rows, m.current.as_ref());
        self.popup = Some(window::Popup::new(m.at, menu, ctx, false));
        true
    }
}

/// Where `part` lies in `l`.
fn part_rect(l: &commandbar::Layout, part: commandbar::Hit) -> Option<Rect> {
    use commandbar::Hit;
    match part {
        Hit::Item(i) => l.items.get(i).map(|s| s.main),
        Hit::ItemDrop(i) => l.items.get(i).and_then(|s| s.drop),
        Hit::More => l.more,
        Hit::ViewsCycle => l.views.map(|v| v.0),
        Hit::ViewsDrop => l.views.map(|v| v.1),
        Hit::Help => l.help,
    }
}

/// The command bar with no commands yet: Views and the help mark at its right end.
pub(crate) fn command_bar() -> Commandbar {
    Commandbar {
        right: vec![RightItem::Views, RightItem::Help],
        ..Commandbar::default()
    }
}

/// Where the window is, as the command bar sees it; the Recycle Bin with how many items it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BarPlace {
    Folder,
    Computer,
    RecycleBin(usize),
    Network,
    Archive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Picked {
    Nothing,
    Folder,
    File {
        image: bool,
        archive: bool,
    },
    Several {
        archives: bool,
    },
    /// One of Computer's rows; `eject` when it can be ejected, true for a network share.
    Drive {
        eject: Option<bool>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BarFacts {
    pub place: BarPlace,
    pub picked: Picked,
    pub can_connect: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BarMenu {
    Organize,
    New,
    OpenWith,
}

/// A command bar command: what its label runs, and the menu its drop (or a [`Drop::Menu`] command
/// itself) opens.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BarAct {
    pub run: Command,
    pub menu: Option<BarMenu>,
}

type Row = (String, Command, Drop, Option<BarMenu>);

fn plain(label: String, run: Command) -> Row {
    (label, run, Drop::None, None)
}

/// The command bar's commands for `f`, Organize first, each with what it does. A writable folder
/// ends on New folder, whose drop holds the File menu's New rows.
pub(crate) fn commands_for(f: &BarFacts) -> Vec<(commandbar::Item, BarAct)> {
    let mut rows: Vec<Row> = vec![(
        tr!("commandbar-organize"),
        Command::Nothing,
        Drop::Menu,
        Some(BarMenu::Organize),
    )];
    let connect = || {
        plain(
            tr!("commandbar-connect-to-server"),
            Command::ConnectToServer,
        )
    };
    let extract = || plain(tr!("commandbar-extract-all"), Command::ExtractAll);
    match (f.place, f.picked) {
        (BarPlace::RecycleBin(0), Picked::Nothing) => {}
        (BarPlace::RecycleBin(_), Picked::Nothing) => rows.push(plain(
            tr!("commandbar-empty-recycle-bin"),
            Command::EmptyRecycleBin,
        )),
        (BarPlace::RecycleBin(_), _) => {
            rows.push(plain(tr!("commandbar-restore"), Command::Restore));
        }
        (BarPlace::Archive, _) => rows.push(extract()),
        (BarPlace::Computer, picked) => {
            if let Picked::Drive { eject } = picked {
                rows.push(plain(tr!("commandbar-properties"), Command::Properties));
                match eject {
                    Some(true) => rows.push(plain(tr!("commandbar-disconnect"), Command::Eject)),
                    Some(false) => rows.push(plain(tr!("commandbar-eject"), Command::Eject)),
                    None => {}
                }
            }
            rows.push(plain(
                tr!("commandbar-system-properties"),
                Command::SystemInformation,
            ));
            if f.can_connect {
                rows.push(connect());
            }
            rows.push(plain(tr!("commandbar-open-settings"), Command::Settings));
        }
        (BarPlace::Network, _) => {
            if f.can_connect {
                rows.push(connect());
            }
        }
        (BarPlace::Folder, picked) => {
            match picked {
                Picked::File { image, archive } => {
                    rows.push((
                        tr!("commandbar-open"),
                        Command::Open,
                        Drop::Split,
                        Some(BarMenu::OpenWith),
                    ));
                    if image {
                        rows.push(plain(tr!("commandbar-mount"), Command::MountImage));
                    }
                    if archive {
                        rows.push(extract());
                    }
                }
                Picked::Folder => rows.push(plain(tr!("commandbar-open"), Command::Open)),
                Picked::Several { archives: true } => rows.push(extract()),
                Picked::Several { .. } | Picked::Nothing | Picked::Drive { .. } => {}
            }
            rows.push((
                tr!("commandbar-new-folder"),
                Command::NewFolder,
                Drop::Split,
                Some(BarMenu::New),
            ));
        }
    }
    (rows.into_iter().enumerate())
        .map(|(i, (label, run, drop, menu))| {
            (
                commandbar::Item::new(label, i as u32).drop(drop),
                BarAct { run, menu },
            )
        })
        .collect()
}
