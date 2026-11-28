//! The details pane's facts for what the view shows: the folder with nothing selected, one
//! item's own, or a count and size for several. Everything comes from the listing and the reads
//! already landed (a drive's space, an image's size); nothing here touches the file system.

use crate::Explorer;
use crate::place::Place;
use reprisefs::listing::Item;
use reprisefs::props::short_size;
use reprisekit::controls::detailspane::{self, Facts};
use reprisekit::icontheme::IconCache;
use reprisekit::raster::Canvas;
use reprisekit::tr;

impl Explorer {
    /// The details pane shows the selection's facts, under the shell layout.
    pub(crate) fn sync_details(&mut self, ctx: &mut repriseapp::Ctx) {
        if self.shell_on {
            let facts = self.facts_for(&mut ctx.icons);
            self.details.set(facts);
        }
    }

    /// The details pane's facts for the selection, or the folder with none.
    pub fn facts_for(&self, icons: &mut IconCache) -> Facts {
        let chosen: Vec<usize> = (self.sv.selection.set.iter().copied())
            .filter(|&i| i < self.items.len())
            .collect();
        let is_12h = reprisefs::locale::is_12h();
        match chosen[..] {
            [] => self.folder_facts(icons),
            [i] => Facts {
                icon: self.entry_icon(i),
                ..self.item_facts(&self.items[i], is_12h)
            },
            ref many => {
                let bytes: u64 = (many.iter().map(|&i| &self.items[i]))
                    .filter(|it| !it.is_folder())
                    .map(|it| it.size)
                    .sum();
                Facts {
                    icon: self.entry_icon(many[0]),
                    title: tr!("details-pane-selected", count = many.len()),
                    subtitle: String::new(),
                    rows: vec![(tr!("details-pane-size"), short_size(bytes))],
                }
            }
        }
    }

    /// Nothing selected: the folder's icon and name and how many items it lists, and its free
    /// space once read.
    fn folder_facts(&self, icons: &mut IconCache) -> Facts {
        let px = detailspane::ICON;
        let icon = icons.get_unread(self.place.icon_name(), px).flatten();
        let mut rows = Vec::new();
        let free = (self.place.path()).and_then(|dir| self.folder_free.of(&dir));
        if let Some(free) = free.filter(|_| self.place != Place::RecycleBin) {
            rows.push((tr!("details-pane-space-free"), short_size(free)));
        }
        Facts {
            icon: icon.map(|ic| (*ic).clone()),
            title: self.title_of(&self.place),
            subtitle: tr!("details-pane-items", count = self.items.len()),
            rows,
        }
    }

    /// One item's facts: its name and type, then what the listing knows of it.
    fn item_facts(&self, it: &Item, is_12h: bool) -> Facts {
        let mut rows = Vec::new();
        let when = |secs| reprisefs::props::format_pane_time(secs, is_12h);
        if let Some(t) = &it.trashed {
            if let Some(dir) = t.original.parent() {
                rows.push((
                    tr!("details-pane-original-location"),
                    dir.display().to_string(),
                ));
            }
            if let Some(secs) = t.deleted {
                rows.push((tr!("details-pane-date-deleted"), when(secs)));
            }
        }
        match self
            .spaces
            .read
            .get(&it.path)
            .filter(|_| it.drive.is_some())
        {
            Some(&(total, free)) => {
                rows.push((tr!("details-pane-space-free"), short_size(free)));
                rows.push((tr!("details-pane-total-size"), short_size(total)));
            }
            None => {
                if let Some(secs) = it.modified.filter(|_| it.drive.is_none()) {
                    rows.push((tr!("details-pane-date-modified"), when(secs)));
                }
                if !it.is_folder() && it.drive.is_none() {
                    rows.push((tr!("details-pane-size"), short_size(it.size)));
                }
            }
        }
        if let Some(&(w, h)) = self.image_dims.get(&it.path) {
            let size = tr!("details-pane-dimensions-value", w = w, h = h);
            rows.push((tr!("details-pane-dimensions"), size));
        }
        Facts {
            icon: None,
            title: it.label().to_string(),
            subtitle: it.kind.clone(),
            rows,
        }
    }

    /// The listed entry's large icon, or its smaller ones until that is read.
    fn entry_icon(&self, i: usize) -> Option<Canvas> {
        let e = self.sv.entries.get(i)?;
        let icon = (e.icon48.as_ref())
            .or(e.icon32.as_ref())
            .or(e.icon.as_ref())?;
        Some((**icon).clone())
    }
}

#[cfg(test)]
mod tests {
    use crate::place::Place;
    use crate::tests::{test_ctx, test_explorer};
    use reprisefs::listing::Item;
    use reprisefs::places::DriveKind;
    use reprisekit::controls::treeview::Style;
    use reprisekit::interface::Interface;
    use reprisekit::interfaces::theme_for;
    use std::path::PathBuf;

    fn item(name: &str, is_dir: bool, size: u64, modified: Option<i64>) -> Item {
        Item {
            name: name.to_string(),
            path: PathBuf::from("/tmp").join(name),
            is_dir,
            is_link: false,
            exec: false,
            program: None,
            hidden: false,
            size,
            modified,
            mime: None,
            kind: if is_dir {
                "File folder"
            } else {
                "Text Document"
            }
            .into(),
            drive: None,
            mount_root: false,
            trashed: None,
            shortcut: None,
            takes_writes: false,
        }
    }

    fn glass_ctx() -> repriseapp::Ctx {
        let mut ctx = test_ctx();
        ctx.theme = theme_for(Interface::Glass);
        ctx
    }

    #[test]
    fn the_details_pane_shows_item_count_with_nothing_selected() {
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder("/tmp/reports".into()), &mut ctx);
        ex.items = vec![item("a.txt", false, 10, None), item("b", true, 0, None)];
        ex.sv.selection.set.clear();
        let f = ex.facts_for(&mut ctx.icons);
        assert_eq!(f.title, "reports");
        assert_eq!(f.subtitle, "2 items");
        assert!(f.rows.is_empty(), "no free space read yet: {:?}", f.rows);
    }

    #[test]
    fn the_details_pane_shows_size_and_dates_for_one_file() {
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder("/tmp".into()), &mut ctx);
        ex.items = vec![item("notes.txt", false, 4300, Some(1_790_000_000))];
        ex.sv.selection.set_only(Some(0));
        let f = ex.facts_for(&mut ctx.icons);
        assert_eq!(
            (f.title.as_str(), f.subtitle.as_str()),
            ("notes.txt", "Text Document")
        );
        let names: Vec<&str> = f.rows.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Date modified:", "Size:"]);
        assert_eq!(f.rows[1].1, reprisefs::props::short_size(4300));
        let when = reprisefs::props::format_pane_time(1_790_000_000, reprisefs::locale::is_12h());
        assert_eq!(f.rows[0].1, when);
    }

    #[test]
    fn a_folder_shows_its_date_and_no_size() {
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder("/tmp".into()), &mut ctx);
        ex.items = vec![item("work", true, 4096, Some(1_790_000_000))];
        ex.sv.selection.set_only(Some(0));
        let f = ex.facts_for(&mut ctx.icons);
        assert_eq!(f.subtitle, "File folder");
        let names: Vec<&str> = f.rows.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Date modified:"]);
    }

    #[test]
    fn a_drive_shows_free_space_in_the_details_pane() {
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Computer, &mut ctx);
        let mut disk = item("Data", true, 0, None);
        disk.drive = Some(DriveKind::Local);
        disk.kind = "Local Disk".into();
        ex.spaces
            .read
            .insert(disk.path.clone(), (500 << 30, 120 << 30));
        ex.items = vec![disk];
        ex.sv.selection.set_only(Some(0));
        let f = ex.facts_for(&mut ctx.icons);
        let short = reprisefs::props::short_size;
        assert_eq!(
            f.rows,
            [
                ("Space free:".to_string(), short(120 << 30)),
                ("Total size:".to_string(), short(500 << 30)),
            ]
        );
    }

    #[test]
    fn several_selected_count_and_add_up_their_files() {
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder("/tmp".into()), &mut ctx);
        ex.items = vec![
            item("a.txt", false, 1000, None),
            item("b.txt", false, 2000, None),
            item("c", true, 4096, None),
        ];
        ex.sv.selection.set = [0, 1, 2].into();
        let f = ex.facts_for(&mut ctx.icons);
        assert_eq!(f.title, "3 items selected");
        assert_eq!(
            f.rows,
            [("Size:".to_string(), reprisefs::props::short_size(3000))]
        );
    }

    #[test]
    fn the_nav_pane_lists_favorites_computer_network_and_recycle_bin() {
        let mut ctx = glass_ctx();
        let mut ex = test_explorer(Place::Computer, &mut ctx);
        ex.build_tree(&mut ctx);
        assert_eq!(ex.tree.style, Style::Nav);
        let roots: Vec<String> = ex.tree.roots.iter().map(|n| n.label.clone()).collect();
        let named = |p: Place| ex.title_of(&p);
        assert_eq!(
            roots,
            [
                "Favorites".to_string(),
                named(Place::Home),
                named(Place::Computer),
                named(Place::network()),
                named(Place::RecycleBin),
            ]
        );
        let fav = &ex.tree.roots[0];
        assert!(fav.expanded && fav.children.iter().all(|n| !n.has_children));
        assert_eq!(fav.children[0].label, named(Place::Desktop));
        assert!(ex.tree.roots[2].has_children, "Computer opens");
        assert!(!ex.tree.roots[4].has_children, "the Recycle Bin does not");
        assert_eq!(ex.place_of(&[0]), None, "the heading goes nowhere");
    }

    #[test]
    fn the_current_place_is_selected_in_the_nav_pane() {
        let mut ctx = glass_ctx();
        let mut ex = test_explorer(Place::Computer, &mut ctx);
        ex.build_tree(&mut ctx);
        for (place, path) in [
            (Place::Computer, vec![2]),
            (Place::RecycleBin, vec![4]),
            (Place::Desktop, vec![0, 0]),
        ] {
            ex.tree_follow(&place, &mut ctx);
            assert_eq!(ex.tree.selected, Some(path), "{place:?}");
        }
    }

    #[test]
    fn the_expanders_fade_out_once_the_view_takes_the_keyboard() {
        let mut ctx = glass_ctx();
        let mut ex = test_explorer(Place::Computer, &mut ctx);
        ex.build_tree(&mut ctx);
        ex.focus = crate::Focus::Tree;
        assert_eq!(ex.nav_fade(), (false, true), "fading in");
        ex.focus = crate::Focus::View;
        assert!(ex.nav_fade().1, "fading out, the pointer elsewhere");
        ctx.theme = theme_for(Interface::Blue);
        ex.follow_scale(1, &mut ctx);
        assert_eq!(ex.nav_fade(), (false, false), "the Folders tree has none");
    }

    #[test]
    fn a_switch_away_from_glass_brings_the_folders_tree_back() {
        let mut ctx = glass_ctx();
        let mut ex = test_explorer(Place::Computer, &mut ctx);
        ex.build_tree(&mut ctx);
        assert_eq!(ex.tree.style, Style::Nav);
        ctx.theme = theme_for(Interface::Blue);
        ex.follow_scale(1, &mut ctx);
        assert_eq!(ex.tree.style, Style::Explorer);
        assert_eq!(ex.tree.roots.len(), 1, "the Desktop");
        ctx.theme = theme_for(Interface::Glass);
        ex.follow_scale(1, &mut ctx);
        assert_eq!(ex.tree.style, Style::Nav);
        assert_eq!(ex.tree.selected, Some(vec![2]), "Computer picked again");
    }
}
