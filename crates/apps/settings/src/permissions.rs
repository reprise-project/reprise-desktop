//! The Privacy page's App Permissions list: the answers programs were given, and its Forget.

use crate::registry::Page;
use crate::{SettingsApp, pages, registry};
use repriseapp::Reply;

/// A permission the list shows, kept by its portal in the permission store table of
/// [`table`](Self::table), under an id of the same name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Location,
    Background,
    Screenshot,
    Wallpaper,
    InputCapture,
}

impl Kind {
    pub(crate) const ALL: [Kind; 5] = [
        Kind::Location,
        Kind::Background,
        Kind::Screenshot,
        Kind::Wallpaper,
        Kind::InputCapture,
    ];

    pub(crate) fn table(self) -> &'static str {
        match self {
            Kind::Location => "location",
            Kind::Background => "background",
            Kind::Screenshot => "screenshot",
            Kind::Wallpaper => "wallpaper",
            Kind::InputCapture => "input-capture",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Kind::Location => "Location",
            Kind::Background => "Run in background",
            Kind::Screenshot => "Screenshots",
            Kind::Wallpaper => "Change desktop background",
            Kind::InputCapture => "Capture keyboard and mouse",
        }
    }

    /// Whether an app's stored values allow it; `None` for an answer the portal still asks.
    /// A location answer is `[accuracy, last use]` and any accuracy but `NONE` allows; the
    /// others start with `yes`, `no` or `ask`.
    fn allows(self, values: &[String]) -> Option<bool> {
        let first = values.first().map(String::as_str);
        match (self, first) {
            (Kind::Location, a) => Some(a.is_some_and(|a| a != "NONE")),
            (_, Some("yes")) => Some(true),
            (_, Some("no")) => Some(false),
            _ => None,
        }
    }
}

/// A table's stored values by app id, as the permission store holds them.
type Stored = Vec<(String, Vec<String>)>;

/// A remembered answer: for location, from Reprise's own prompt (`location.toml`), or for a
/// Flatpak program from its portal's permission store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PermissionAnswer {
    pub(crate) app: String,
    pub(crate) kind: Kind,
    pub(crate) allow: bool,
    pub(crate) portal: bool,
}

impl PermissionAnswer {
    /// The permission store entry Forget deletes, as table, id and app; `None` for an answer
    /// Reprise's own prompt keeps.
    pub(crate) fn store_entry(&self) -> Option<(&'static str, &'static str, &str)> {
        let t = self.kind.table();
        self.portal.then_some((t, t, self.app.as_str()))
    }
}

/// The list's answers: the location prompt's, then each table's from `portal`, the ones still
/// to ask left out.
pub(crate) fn permission_answers(
    agent: std::collections::BTreeMap<String, bool>,
    portal: Vec<(Kind, Stored)>,
) -> Vec<PermissionAnswer> {
    let agent = agent.into_iter().map(|(app, allow)| PermissionAnswer {
        app,
        kind: Kind::Location,
        allow,
        portal: false,
    });
    let portal = portal.into_iter().flat_map(|(kind, apps)| {
        apps.into_iter().filter_map(move |(app, values)| {
            Some(PermissionAnswer {
                allow: kind.allows(&values)?,
                app,
                kind,
                portal: true,
            })
        })
    });
    agent.chain(portal).collect()
}

/// Each table's stored values; the tables read before the store stopped answering, none when it
/// does not answer at all. Tests leave the session bus alone.
fn portal_answers() -> Vec<(Kind, Stored)> {
    if cfg!(test) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for kind in Kind::ALL {
        match reprisebus::permissions::answers(kind.table(), kind.table()) {
            Ok(apps) => out.push((kind, apps)),
            Err(e) => {
                log::warn!("no app permissions from the permission store: {e}");
                break;
            }
        }
    }
    out
}

/// Delete `entry` (table, id, app) from the permission store. Tests record it in [`FORGOTTEN`]
/// instead.
fn delete_from_store((table, id, app): (&str, &str, &str)) -> Result<(), String> {
    #[cfg(test)]
    {
        FORGOTTEN.with_borrow_mut(|f| f.push([table, id, app].map(String::from)));
        Ok(())
    }
    #[cfg(not(test))]
    reprisebus::permissions::forget(table, id, app).map_err(|e| e.to_string())
}

#[cfg(test)]
thread_local! {
    pub(crate) static FORGOTTEN: std::cell::RefCell<Vec<[String; 3]>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The permission list as read off the loop: its answers, each row's text and icon name.
#[derive(Debug, Default)]
pub(crate) struct PermissionList {
    pub(crate) answers: Vec<PermissionAnswer>,
    pub(crate) rows: Vec<(String, &'static str, bool)>,
    pub(crate) icons: Vec<Option<String>>,
}

/// A row's program: its desktop entry's name, else its app id. The portals file the programs
/// they could not name under the empty id.
pub(crate) fn program_name(app: &str, entry: Option<&str>) -> String {
    match entry {
        Some(name) => name.into(),
        None if app.is_empty() => "Unidentified programs".into(),
        None => app.into(),
    }
}

/// Reprise's own location answers from `file`, the portals', and the program each names, sorted
/// by the program's name.
fn read_permissions(file: &std::path::Path) -> PermissionList {
    let answers = permission_answers(repriseconf::location::load_from(file), portal_answers());
    let entries = if answers.is_empty() {
        Vec::new()
    } else {
        reprisefs::apps::scan(&reprisefs::apps::default_dirs())
    };
    let mut named: Vec<_> = answers
        .into_iter()
        .map(|a| {
            let e = (!a.app.is_empty())
                .then(|| reprisefs::apps::entry_for_app_id(&entries, &a.app, |_| true))
                .flatten();
            let name = program_name(&a.app, e.map(|e| e.name.as_str()));
            let icon = e.and_then(|e| e.icon.clone());
            (name, icon, a)
        })
        .collect();
    named.sort_by_cached_key(|(name, ..)| name.to_lowercase());
    let mut list = PermissionList::default();
    for (name, icon, a) in named {
        list.rows.push((name, a.kind.label(), a.allow));
        list.icons.push(icon);
        list.answers.push(a);
    }
    list
}

impl SettingsApp {
    /// Read the permission list off the loop the first time the Privacy page shows: the
    /// permission store can take seconds to answer. The list says it is loading until then.
    pub(crate) fn ensure_permissions(&mut self) {
        if !self.permissions_due
            || self.reading_permissions.is_some()
            || registry::open(&self.sheet) != Page::Privacy
        {
            return;
        }
        self.permissions_due = false;
        let file = self.location_file.clone();
        self.reading_permissions = Some(self.spawn(move || read_permissions(&file)));
        pages::show_permissions_loading(&mut self.permissions);
        self.sync_forget();
    }

    pub(crate) fn show_permissions(&mut self, found: PermissionList) {
        self.permission_icons = found.icons;
        self.permission_icons_due = true;
        self.permission_answers = found.answers;
        self.permissions.selection.set_only(None);
        pages::fill_permissions(&mut self.permissions, &found.rows);
        self.sync_forget();
    }

    pub(crate) fn sync_forget(&mut self) {
        let on = self.reading_permissions.is_none() && self.permissions.selection.focus.is_some();
        self.sheet.pages[Page::Privacy.index()].set_enabled(pages::FORGET, on);
    }

    /// Forget drops the picked answer at once, from the state file or the portal's permission
    /// store, so the program is asked again; it waits for no Apply.
    pub(crate) fn forget_permission(&mut self) -> Reply {
        let Some((i, a)) = (self.permissions.selection.focus)
            .filter(|_| self.reading_permissions.is_none())
            .and_then(|i| Some((i, self.permission_answers.get(i)?)))
        else {
            return Reply::none();
        };
        let forgot = match a.store_entry() {
            Some(entry) => delete_from_store(entry),
            None => repriseconf::location::forget_in(&self.location_file, &a.app)
                .map_err(|e| e.to_string()),
        };
        match forgot {
            Ok(()) => {
                self.permission_answers.remove(i);
                self.permission_icons.remove(i);
                self.permissions.rows.remove(i);
                self.permissions.selection.set_only(None);
                self.sync_forget();
            }
            Err(e) => {
                log::warn!("forget the {} answer for {}: {e}", a.kind.table(), a.app);
                self.fail("The answer could not be forgotten.");
                self.permissions_due = true;
                self.ensure_permissions();
            }
        }
        self.sync_apply();
        Reply::redraw()
    }
}
