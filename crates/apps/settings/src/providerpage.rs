//! The Search page's Program Results list: the installed programs' search providers in the order
//! their sections show, each ticked while the launcher asks it, with Move Up and Move Down; one
//! that starts the file indexer greys while content search is off. The providers are read off the
//! loop the first time the page opens.

use crate::SettingsApp;
use crate::model::Settings;
use crate::registry::{self, Page};
use repriseapp::Reply;
use reprisefs::searchproviders::{self, Provider};
use reprisekit::controls::form::{Control, Form};
use reprisekit::controls::listbox::ListBox;
use reprisekit::controls::openwith;
use reprisekit::raster::Canvas;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

pub const LIST: &str = "search_providers";
pub const UP: &str = "search_providers_up";
pub const DOWN: &str = "search_providers_down";
/// The list's rows at [`Area::MIN`](crate::pages::Area::MIN).
pub const ROWS: usize = 6;
/// The dimmed text after a row held back by content search being off.
pub const HELD_NOTE: &str = "Needs Find files by their contents";

/// A provider whose program has a shown desktop entry, with that entry's name and icon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub provider: Provider,
    pub name: String,
    pub icon: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub on: bool,
    /// Never asked while content search is off ([`searchproviders::starts_indexer`]).
    pub held: bool,
}

/// The providers in `dirs` whose program has a shown entry in `app_dirs`. Blocks on the file
/// system.
pub fn read_installed(dirs: &[PathBuf], app_dirs: &[PathBuf]) -> Vec<Installed> {
    let providers = searchproviders::scan(dirs);
    if providers.is_empty() {
        return Vec::new();
    }
    let apps = reprisefs::apps::scan(app_dirs);
    (providers.into_iter())
        .filter_map(|provider| {
            let e = (apps.iter()).find(|a| a.shown && a.id == provider.desktop_id)?;
            Some(Installed {
                name: e.name.clone(),
                icon: e.icon.clone(),
                provider,
            })
        })
        .collect()
}

/// The rows as `s` orders and ticks them ([`searchproviders::arrange`]), held while its content
/// search is off.
pub fn rows(installed: &[Installed], s: &Settings) -> Vec<Row> {
    let providers: Vec<Provider> = installed.iter().map(|i| i.provider.clone()).collect();
    let of = |p: &Provider| installed.iter().find(|i| i.provider == *p);
    let name = |p: &Provider| of(p).map(|i| i.name.clone()).unwrap_or_default();
    let (order, off) = (&s.launcher_providers, &s.launcher_providers_off);
    (searchproviders::arrange(&providers, order, off, name).into_iter())
        .filter_map(|(p, on)| {
            let i = of(p)?;
            Some(Row {
                id: p.desktop_id.clone(),
                name: i.name.clone(),
                icon: i.icon.clone(),
                on,
                held: !s.launcher_contents && searchproviders::starts_indexer(&p.desktop_id),
            })
        })
        .collect()
}

/// `rows` into `s`: every row's id in order, the unticked ones turned off. Ids `s` names for
/// programs not installed now keep their place at the end. True when `s` changed.
pub fn write(rows: &[Row], s: &mut Settings) -> bool {
    let shown = |id: &String| rows.iter().any(|r| r.id == *id);
    let order: Vec<String> = (rows.iter().map(|r| r.id.clone()))
        .chain(s.launcher_providers.iter().filter(|id| !shown(id)).cloned())
        .collect();
    let off: Vec<String> = (rows.iter().filter(|r| !r.on).map(|r| r.id.clone()))
        .chain(
            s.launcher_providers_off
                .iter()
                .filter(|id| !shown(id))
                .cloned(),
        )
        .collect();
    let changed = order != s.launcher_providers || off != s.launcher_providers_off;
    s.launcher_providers = order;
    s.launcher_providers_off = off;
    changed
}

fn list(form: &Form) -> Option<&ListBox> {
    match form.item(LIST).map(|i| &i.control) {
        Some(Control::ListBox { list, .. }) => Some(list),
        _ => None,
    }
}

fn list_mut(form: &mut Form) -> Option<(&mut ListBox, &mut Vec<Option<Canvas>>)> {
    match form.item_mut(LIST).map(|i| &mut i.control) {
        Some(Control::ListBox { list, icons }) => Some((list, icons)),
        _ => None,
    }
}

/// Show `rows` with row `pick` picked, a greyed Loading... while they are read (None), or a greyed
/// note when no program offers results; a held row greys with [`HELD_NOTE`] after its name. True
/// when the rows' names changed, which drops their icons. Move Up and Move Down follow the picked
/// row.
pub fn fill(form: &mut Form, rows: Option<&[Row]>, pick: Option<usize>) -> bool {
    let Some((list, icons)) = list_mut(form) else {
        return false;
    };
    let items: Vec<String> = match rows {
        Some([]) => vec!["No installed program offers search results.".into()],
        Some(rows) => rows.iter().map(|r| r.name.clone()).collect(),
        None => vec!["Loading...".into()],
    };
    let rows = rows.unwrap_or_default();
    let renamed = list.items != items;
    let pick = pick.filter(|&i| i < rows.len());
    if renamed {
        *list = ListBox::new(items, pick);
        *icons = Vec::new();
    } else {
        list.selected = pick;
    }
    list.checks = rows.iter().map(|r| r.on).collect();
    list.greyed = match rows.is_empty() {
        true => vec![true],
        false => rows.iter().map(|r| r.held).collect(),
    };
    list.detail = (rows.iter())
        .map(|r| match r.held {
            true => HELD_NOTE.to_string(),
            false => String::new(),
        })
        .collect();
    form.set_enabled(UP, pick.is_some_and(|i| i > 0));
    form.set_enabled(DOWN, pick.is_some_and(|i| i + 1 < rows.len()));
    renamed
}

/// Whether button `id` is lit.
fn lit(form: &Form, id: &str) -> bool {
    matches!(
        form.item(id).map(|i| &i.control),
        Some(Control::Button { enabled: true, .. })
    )
}

/// Each row's tick as the list holds it, and the picked row; true when a tick changed.
fn read(form: &Form, rows: &mut [Row]) -> (Option<usize>, bool) {
    let Some(list) = list(form) else {
        return (None, false);
    };
    let mut ticked = false;
    for (row, &on) in rows.iter_mut().zip(&list.checks) {
        ticked |= row.on != on;
        row.on = on;
    }
    (list.selected.filter(|&i| i < rows.len()), ticked)
}

/// The Program Results list's state.
#[derive(Default)]
pub(crate) struct Providers {
    /// None until read.
    installed: Option<Vec<Installed>>,
    pub(crate) job: Option<Receiver<Vec<Installed>>>,
    rows: Vec<Row>,
    icons_due: bool,
}

impl SettingsApp {
    /// Read the providers off the loop the first time the Search page shows, and keep the list in
    /// step with the pending settings. True when the list changed.
    pub(crate) fn ensure_providers(&mut self) -> bool {
        if registry::open(&self.sheet) != Page::Search {
            return false;
        }
        let p = &mut self.providers;
        if p.installed.is_none() && p.job.is_none() {
            let dirs = searchproviders::default_dirs();
            let app_dirs = self.app_dirs.clone();
            self.providers.job = Some(self.spawn(move || read_installed(&dirs, &app_dirs)));
        }
        self.show_providers(None)
    }

    /// The read finished: list what it found. True when it did.
    pub(crate) fn providers_done(&mut self) -> bool {
        let Some(rx) = &self.providers.job else {
            return false;
        };
        let found = match crate::take(rx) {
            Ok(None) => return false,
            Ok(Some(found)) => found,
            Err(()) => Vec::new(),
        };
        self.providers.job = None;
        self.providers.installed = Some(found);
        self.show_providers(None);
        true
    }

    /// The list from the pending settings, row `pick` (else the picked one) picked. True when it
    /// changed.
    fn show_providers(&mut self, pick: Option<usize>) -> bool {
        let page = &mut self.sheet.pages[Page::Search.index()];
        let p = &mut self.providers;
        let rows = (p.installed.as_deref()).map(|i| rows(i, &self.want));
        let pick = pick.or_else(|| list(page).and_then(|l| l.selected));
        let renamed = fill(page, rows.as_deref(), pick);
        p.icons_due |= renamed;
        let rows = rows.unwrap_or_default();
        let changed = renamed || rows != p.rows;
        p.rows = rows;
        changed
    }

    /// A row was ticked, unticked or picked.
    pub(crate) fn providers_changed(&mut self) -> Reply {
        let page = &self.sheet.pages[Page::Search.index()];
        let (pick, ticked) = read(page, &mut self.providers.rows);
        if ticked && write(&self.providers.rows, &mut self.want) {
            self.sync_apply();
        }
        self.show_providers(pick);
        Reply::redraw()
    }

    /// Move Up (`up`) or Move Down on the picked row.
    pub(crate) fn move_provider(&mut self, up: bool) -> Reply {
        let page = &self.sheet.pages[Page::Search.index()];
        let Some(i) = read(page, &mut self.providers.rows).0 else {
            return Reply::none();
        };
        let to = match up {
            true => i.checked_sub(1),
            false => Some(i + 1).filter(|&j| j < self.providers.rows.len()),
        };
        let Some(to) = to else {
            return Reply::none();
        };
        self.providers.rows.swap(i, to);
        if write(&self.providers.rows, &mut self.want) {
            self.sync_apply();
        }
        self.show_providers(Some(to));
        let page = &mut self.sheet.pages[Page::Search.index()];
        if !lit(page, if up { UP } else { DOWN }) {
            page.focus_id(LIST);
        }
        Reply::redraw()
    }

    /// Each row's icon: its program's, else the generic program icon.
    pub(crate) fn set_provider_icons(&mut self, get: &mut dyn FnMut(&str) -> Option<Canvas>) {
        if !self.providers.icons_due {
            return;
        }
        let loaded = (self.providers.rows.iter())
            .map(|r| {
                (r.icon.as_deref())
                    .and_then(&mut *get)
                    .or_else(|| get(openwith::PROGRAM_ICON))
            })
            .collect();
        if let Some((_, icons)) = list_mut(&mut self.sheet.pages[Page::Search.index()]) {
            *icons = loaded;
        }
        self.providers.icons_due = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::{Area, search};

    fn installed(id: &str, name: &str, default_disabled: bool) -> Installed {
        Installed {
            provider: Provider {
                desktop_id: id.into(),
                bus_name: format!("org.example.{name}"),
                object_path: "/".into(),
                default_disabled,
            },
            name: name.into(),
            icon: Some(format!("{name}-icon")),
        }
    }

    fn all() -> Vec<Installed> {
        vec![
            installed("files.desktop", "Files", false),
            installed("calc.desktop", "Calculator", false),
            installed("web.desktop", "Web", true),
        ]
    }

    fn summary(rows: &[Row]) -> Vec<(&str, bool)> {
        rows.iter().map(|r| (r.name.as_str(), r.on)).collect()
    }

    #[test]
    fn rows_come_by_name_ticked_unless_off_by_default() {
        let rows = rows(&all(), &Settings::default());
        assert_eq!(
            summary(&rows),
            [("Calculator", true), ("Files", true), ("Web", false)]
        );
    }

    #[test]
    fn writing_the_rows_lists_every_id_in_order_and_the_unticked_off() {
        let mut s = Settings {
            launcher_providers: vec!["gone.desktop".into()],
            launcher_providers_off: vec!["gone.desktop".into()],
            ..Settings::default()
        };
        let mut r = rows(&all(), &s);
        r.swap(0, 2);
        r[0].on = true;
        r[1].on = false;
        assert!(write(&r, &mut s));
        assert_eq!(
            s.launcher_providers,
            [
                "web.desktop",
                "files.desktop",
                "calc.desktop",
                "gone.desktop"
            ]
        );
        assert_eq!(s.launcher_providers_off, ["files.desktop", "gone.desktop"]);
        assert_eq!(
            summary(&rows(&all(), &s)),
            [("Web", true), ("Files", false), ("Calculator", true)],
            "read back as written"
        );
        assert!(!write(&r, &mut s), "nothing moved");
    }

    /// The held note fits after the name on the list's row, past its check box and icon.
    #[test]
    fn a_held_row_fits_its_note() {
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::{FontConfig, Theme};
        reprisekit::pins::fonts();
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let f = search(Area::MIN);
        let w = f.item(LIST).unwrap().rect.width;
        let used =
            4 + 42 + text.measure("Files", false, px) + 8 + text.measure(HELD_NOTE, false, px);
        assert!(used <= w - 4, "{used} > {}", w - 4);
    }

    #[test]
    fn a_provider_that_starts_the_indexer_is_held_while_content_search_is_off() {
        let files = installed("org.gnome.Nautilus.desktop", "Files", false);
        let calc = installed("calc.desktop", "Calculator", false);
        let held = |s: &Settings| -> Vec<(String, bool)> {
            (rows(&[files.clone(), calc.clone()], s).into_iter())
                .map(|r| (r.name, r.held))
                .collect()
        };
        let pair = |n: &str, h| (n.to_string(), h);
        let off = Settings::default();
        assert_eq!(held(&off), [pair("Calculator", false), pair("Files", true)]);
        let on = Settings {
            launcher_contents: true,
            ..Settings::default()
        };
        assert_eq!(held(&on), [pair("Calculator", false), pair("Files", false)]);
        let mut f = search(Area::MIN);
        fill(&mut f, Some(&rows(&[files, calc], &off)), None);
        let lb = list(&f).unwrap();
        assert_eq!(lb.greyed, [false, true]);
        assert_eq!(lb.detail, ["", HELD_NOTE]);
        assert_eq!(lb.checks, [true, true], "the tick stays as set");
    }

    #[test]
    fn the_list_says_loading_then_lists_the_rows_with_their_ticks() {
        let mut f = search(Area::MIN);
        fill(&mut f, None, None);
        let lb = list(&f).unwrap();
        assert_eq!(lb.items, ["Loading..."]);
        assert_eq!(lb.greyed, [true]);
        assert!(lb.checks.is_empty());
        assert!(!lit(&f, UP) && !lit(&f, DOWN));

        let r = rows(&all(), &Settings::default());
        assert!(fill(&mut f, Some(&r), Some(1)));
        let lb = list(&f).unwrap();
        assert_eq!(lb.items, ["Calculator", "Files", "Web"]);
        assert_eq!(lb.checks, [true, true, false]);
        assert_eq!(lb.selected, Some(1));
        assert!(lit(&f, UP) && lit(&f, DOWN));
        fill(&mut f, Some(&r), Some(2));
        assert!(!lit(&f, DOWN), "the last row");

        fill(&mut f, Some(&[]), None);
        let lb = list(&f).unwrap();
        assert_eq!(lb.items, ["No installed program offers search results."]);
        assert_eq!(lb.greyed, [true]);
    }
}
