//! The Privacy page's App Permissions list: the answers programs were given, and its Forget.

use crate::registry::Page;
use crate::{SettingsApp, pages, registry, take};
use repriseapp::Reply;
use std::sync::mpsc::Receiver;

/// A permission the list shows, kept by its portal in the permission store table of
/// [`table`](Self::table): under an id of the same name, or one id per restore token for the
/// screen sharing and remote control grants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Location,
    Background,
    Screenshot,
    Wallpaper,
    InputCapture,
    ScreenCast,
    RemoteDesktop,
}

impl Kind {
    pub(crate) const ALL: [Kind; 7] = [
        Kind::Location,
        Kind::Background,
        Kind::Screenshot,
        Kind::Wallpaper,
        Kind::InputCapture,
        Kind::ScreenCast,
        Kind::RemoteDesktop,
    ];

    pub(crate) fn table(self) -> &'static str {
        match self {
            Kind::Location => "location",
            Kind::Background => "background",
            Kind::Screenshot => "screenshot",
            Kind::Wallpaper => "wallpaper",
            Kind::InputCapture => "input-capture",
            Kind::ScreenCast => "screencast",
            Kind::RemoteDesktop => "remote-desktop",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Kind::Location => "Location",
            Kind::Background => "Run in background",
            Kind::Screenshot => "Screenshots",
            Kind::Wallpaper => "Change desktop background",
            Kind::InputCapture => "Capture keyboard and mouse",
            Kind::ScreenCast => "Share the screen",
            Kind::RemoteDesktop => "Remote control",
        }
    }

    fn by_restore_token(self) -> bool {
        matches!(self, Kind::ScreenCast | Kind::RemoteDesktop)
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

/// What a table holds for one program: its stored values and the ids they are under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Held {
    pub(crate) app: String,
    pub(crate) values: Vec<String>,
    pub(crate) ids: Vec<String>,
}

/// The values of a table kept under an id named after it, by app id.
pub(crate) fn by_id(kind: Kind, apps: Vec<(String, Vec<String>)>) -> Vec<Held> {
    let ids = vec![kind.table().to_string()];
    (apps.into_iter())
        .map(|(app, values)| Held {
            app,
            values,
            ids: ids.clone(),
        })
        .collect()
}

/// A table of restore tokens, given as each token with its values by app id, folded into one
/// program each holding all of its tokens. Sorted by app id, each program's tokens by id.
pub(crate) fn by_token(
    tokens: impl IntoIterator<Item = (String, impl IntoIterator<Item = (String, Vec<String>)>)>,
) -> Vec<Held> {
    let mut apps = std::collections::BTreeMap::<String, Held>::new();
    for (id, held) in tokens {
        for (app, values) in held {
            let h = apps.entry(app.clone()).or_insert(Held {
                app,
                values,
                ids: Vec::new(),
            });
            h.ids.push(id.clone());
        }
    }
    let mut out: Vec<Held> = apps.into_values().collect();
    out.iter_mut().for_each(|h| h.ids.sort());
    out
}

/// A remembered answer: for location, from Reprise's own prompt (`location.toml`), or for a
/// Flatpak program from its portal's permission store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PermissionAnswer {
    pub(crate) app: String,
    pub(crate) kind: Kind,
    pub(crate) allow: bool,
    /// The permission store ids holding it in its kind's table; none for an answer Reprise's
    /// own prompt keeps.
    pub(crate) ids: Vec<String>,
}

/// The list's answers: the location prompt's, then each table's from `portal`, the ones still
/// to ask left out.
pub(crate) fn permission_answers(
    agent: std::collections::BTreeMap<String, bool>,
    portal: Vec<(Kind, Vec<Held>)>,
) -> Vec<PermissionAnswer> {
    let agent = agent.into_iter().map(|(app, allow)| PermissionAnswer {
        app,
        kind: Kind::Location,
        allow,
        ids: Vec::new(),
    });
    let portal = portal.into_iter().flat_map(|(kind, apps)| {
        apps.into_iter().filter_map(move |h| {
            Some(PermissionAnswer {
                allow: kind.allows(&h.values)?,
                app: h.app,
                kind,
                ids: h.ids,
            })
        })
    });
    agent.chain(portal).collect()
}

/// Each table's stored values; the tables read before the store stopped answering, none when it
/// does not answer at all. Tests leave the session bus alone.
fn portal_answers() -> Vec<(Kind, Vec<Held>)> {
    if cfg!(test) {
        return Vec::new();
    }
    use reprisebus::permissions::{answers, entries};
    let mut out = Vec::new();
    for kind in Kind::ALL {
        let t = kind.table();
        let held = match kind.by_restore_token() {
            true => reprisebus::session()
                .and_then(|conn| reprisebus::wait(entries(&conn, t)))
                .map(|e| by_token(e.into_iter().map(|e| (e.id, e.apps)))),
            false => answers(t, t).map(|apps| by_id(kind, apps)),
        };
        match held {
            Ok(held) => out.push((kind, held)),
            Err(e) => {
                log::warn!("no app permissions from the permission store: {e}");
                break;
            }
        }
    }
    out
}

/// Drop `a` from the state file, or from every permission store entry holding it. Blocking.
fn forget(a: &PermissionAnswer, location_file: &std::path::Path) -> Result<(), String> {
    if a.ids.is_empty() {
        return repriseconf::location::forget_in(location_file, &a.app).map_err(|e| e.to_string());
    }
    let token = a.kind.by_restore_token();
    let ids = match token {
        true => tokens_holding(a)?,
        false => a.ids.clone(),
    };
    let table = a.kind.table();
    (ids.iter()).try_for_each(|id| delete_from_store((table, id, &a.app), token))
}

/// The restore tokens of `a`'s table holding its program now, as one stored after the list was
/// read would otherwise be kept. Tests take the listed ones.
fn tokens_holding(a: &PermissionAnswer) -> Result<Vec<String>, String> {
    if cfg!(test) {
        return Ok(a.ids.clone());
    }
    let table = a.kind.table();
    let entries = reprisebus::session()
        .and_then(|conn| reprisebus::wait(reprisebus::permissions::entries(&conn, table)))
        .map_err(|e| e.to_string())?;
    let held = by_token(entries.into_iter().map(|e| (e.id, e.apps)));
    Ok((held.into_iter())
        .find(|h| h.app == a.app)
        .map_or_else(Vec::new, |h| h.ids))
}

/// Delete `entry` (table, id, app) from the permission store, for a restore token `token` with
/// the token once no other program holds it. Tests record it in [`FORGOTTEN`] instead.
fn delete_from_store((table, id, app): (&str, &str, &str), token: bool) -> Result<(), String> {
    #[cfg(test)]
    {
        let _ = token;
        FORGOTTEN.with_borrow_mut(|f| f.push([table, id, app].map(String::from)));
        Ok(())
    }
    #[cfg(not(test))]
    match token {
        true => reprisebus::permissions::forget_token(table, id, app),
        false => reprisebus::permissions::forget(table, id, app),
    }
    .map_err(|e| e.to_string())
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
        let on = self.reading_permissions.is_none()
            && self.forgetting.is_none()
            && self.permissions.selection.focus.is_some();
        self.sheet.pages[Page::Privacy.index()].set_enabled(pages::FORGET, on);
    }

    /// Forget drops the picked answer from the state file or every entry of the portal's
    /// permission store holding it, so the program is asked again; it waits for no Apply. The
    /// store can take seconds, so the job runs off the loop and the row goes when it is done.
    pub(crate) fn forget_permission(&mut self) -> Reply {
        let Some(a) = (self.permissions.selection.focus)
            .filter(|_| self.reading_permissions.is_none() && self.forgetting.is_none())
            .and_then(|i| self.permission_answers.get(i))
            .cloned()
        else {
            return Reply::none();
        };
        let (job, file) = (a.clone(), self.location_file.clone());
        let rx = self.start_forget(move || forget(&job, &file));
        self.forgetting = Some((a, rx));
        self.sync_apply();
        Reply::redraw()
    }

    #[cfg(not(test))]
    fn start_forget(
        &self,
        job: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) -> Receiver<Result<(), String>> {
        self.spawn(job)
    }

    /// Tests run the job on their own thread, where [`FORGOTTEN`] records it, and take the reply
    /// with [`SettingsApp::forget_done`].
    #[cfg(test)]
    fn start_forget(
        &self,
        job: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) -> Receiver<Result<(), String>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(job());
        rx
    }

    /// The Forget under way finished: drop its row, or say it failed and read the list again.
    /// True when the page changed.
    pub(crate) fn forget_done(&mut self) -> bool {
        let Some((_, rx)) = &self.forgetting else {
            return false;
        };
        let forgot = match take(rx) {
            Ok(None) => return false,
            Ok(Some(r)) => r,
            Err(()) => Err("the job ended without a result".into()),
        };
        let Some((a, _)) = self.forgetting.take() else {
            return false;
        };
        match forgot {
            Ok(()) => {
                if let Some(i) = self.permission_answers.iter().position(|x| *x == a) {
                    self.permission_answers.remove(i);
                    self.permission_icons.remove(i);
                    self.permissions.rows.remove(i);
                    self.permissions.selection.set_only(None);
                }
            }
            Err(e) => {
                log::warn!("forget the {} answer for {}: {e}", a.kind.table(), a.app);
                self.fail("The answer could not be forgotten.");
                self.permissions_due = true;
                self.ensure_permissions();
            }
        }
        self.sync_apply();
        true
    }
}
