//! The window's chrome: the toolbar and the address drop as built once, and the panels, tasks, latches
//! and greyed menu rows that every navigation and selection change move.

use crate::icons::chrome_icon;
use crate::place::Place;
use crate::window;
use crate::{Explorer, FOLDERS_ITEM};
use repriseapp::Ctx;
use reprisefs::listing::Item;
use reprisekit::art::viewsicon;
use reprisekit::controls::addressbar::AddressBar;
use reprisekit::controls::combo::ListItem;
use reprisekit::controls::toolbar;

impl Explorer {
    pub(crate) fn build_toolbar(&mut self, ctx: &mut Ctx) {
        let icon = |ctx: &mut Ctx, names: &[&str]| chrome_icon(ctx, names, 24);
        self.toolbar.items = vec![
            toolbar::Item::DropButton {
                id: toolbar::BACK,
                icon: icon(ctx, &["go-previous"]),
                label: Some("Back".to_string()),
                enabled: false,
            },
            toolbar::Item::DropButton {
                id: toolbar::FORWARD,
                icon: icon(ctx, &["go-next"]),
                label: Some(String::new()),
                enabled: false,
            },
            toolbar::Item::Button {
                id: toolbar::UP,
                icon: icon(ctx, &["go-up"]),
                label: None,
                enabled: true,
            },
            toolbar::Item::Separator,
            toolbar::Item::Button {
                id: toolbar::FOLDERS,
                icon: icon(ctx, &["folder"]),
                label: Some("Folders".to_string()),
                enabled: true,
            },
            toolbar::Item::Separator,
            toolbar::Item::DropButton {
                id: toolbar::VIEWS,
                // Few themes carry a view icon, so the drawn one stands in rather than the placeholder.
                icon: ctx
                    .icons
                    .get("view-list", 24)
                    .or_else(|| Some(viewsicon::draw24())),
                label: None,
                enabled: true,
            },
        ];
    }

    /// The address drop's places: the shell tree, one row a place, the Desktop's places in the Folders tree's order with the drives under Computer.
    /// The field names `self.place`, which a window still reading its first folder already holds.
    pub(crate) fn build_address(&mut self, ctx: &mut Ctx) {
        let (rows, places) = self.address_rows(ctx);
        let go = chrome_icon(ctx, &["go-jump", "go-next"], 20);
        self.address = AddressBar::new(rows, 0, go);
        self.address_places = places;
        let icon = ctx.icons.get(self.place.icon_name(), 16);
        self.address.set_place(&self.place.address(), icon);
    }

    fn address_rows(&self, ctx: &mut Ctx) -> (Vec<ListItem>, Vec<Place>) {
        let mut shell: Vec<(String, Place, i32, &'static str)> = vec![(
            "Desktop".into(),
            Place::Desktop,
            0,
            Place::Desktop.icon_name(),
        )];
        for p in crate::foldertree::DESKTOP_PLACES {
            let computer = p == Place::Computer;
            let icon = p.icon_name();
            shell.push((p.title(), p, 1, icon));
            if computer {
                for d in &self.drives {
                    shell.push((d.name(), Place::Folder(d.mount.clone()), 2, d.kind().icon()));
                }
            }
        }
        let mut rows = Vec::new();
        let mut places = Vec::new();
        for (text, place, indent, icon) in shell {
            rows.push(ListItem::shell(text, ctx.icons.get(icon, 16), indent));
            places.push(place);
        }
        (rows, places)
    }

    /// The toolbar, address bar, Folders tree, status bar and task pane fetch their icons again
    /// from `ctx.icons`, now at another scale; what they show and their states stay.
    pub(crate) fn rescale_chrome(&mut self, ctx: &mut Ctx) {
        self.build_toolbar(ctx);
        let (rows, _) = self.address_rows(ctx);
        for (item, row) in self.address.combo.items.iter_mut().zip(rows) {
            item.icon = row.icon;
        }
        self.address.combo.icon = ctx.icons.get(self.place.icon_name(), 16);
        self.address.go_icon = chrome_icon(ctx, &["go-jump", "go-next"], 20);
        crate::foldertree::refetch_icons(&mut self.tree, &self.tree_places, &mut ctx.icons);
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
        let zone = ctx.icons.get("computer", 16);
        self.status.panels =
            window::status_panels(self.items.len(), chosen.len(), bytes, "Computer", zone);
        // One item picked in the Recycle Bin says where it came from; one shortcut elsewhere, where it points.
        if let [i] = chosen[..]
            && let Some(it) = self.items.get(i)
        {
            if self.place == Place::RecycleBin {
                self.status.panels[0].text =
                    window::bin_status_text(it, reprisefs::locale::is_12h());
            } else if (it.is_link || it.shortcut.is_some())
                && let Some(text) = reprisefs::launcher::Launcher::at(&it.path, &self.globs)
                    .and_then(|l| window::shortcut_status_text(&l, &it.path))
            {
                self.status.panels[0].text = text;
            }
        }

        let (back, fwd, up) = (
            self.history.can_back(),
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
}
