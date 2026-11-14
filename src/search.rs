//! What the launcher and the Glass Start menu's search box share: the calculator
//! row, fuzzy app ranking, settings rows and the debounced search for files and programs'
//! results, one section per [`Source`]. Every text change cancels the search in flight and
//! takes a new generation from one process-wide counter, so a late answer for older text, or
//! for a closed box, is dropped.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use reprisebus::searchprovider::Meta;
use repriseconf::pages::{Hit, PROGRAMS, Page};
use reprisefs::apps::AppEntry;
use reprisekit::controls::launcher::FoundRow;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{LoopHandle, RegistrationToken};

use crate::compositor::State;

pub mod providers;
pub use providers::Provider;

const DEBOUNCE: Duration = Duration::from_millis(150);
const MIN_CHARS: usize = 2;

static GENERATIONS: AtomicU64 = AtomicU64::new(0);

fn next_generation() -> u64 {
    GENERATIONS.fetch_add(1, Ordering::Relaxed) + 1
}

pub fn rank_apps(query: &str, apps: &[AppEntry], max: usize) -> Vec<usize> {
    reprisekit::fuzzy::rank(query, apps.iter().map(|a| a.name.as_str()))
        .into_iter()
        // A NoDisplay entry opens files but is not offered to launch.
        .filter(|&i| apps[i].shown)
        .take(max)
        .collect()
}

/// An open window as the launcher finds it: its title and its program's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowName {
    pub title: String,
    pub app: String,
}

/// The windows `query` finds, best first, at most `max`, each with its tier
/// ([`reprisekit::fuzzy::tier`]): the better of its title's and its program's.
/// Ties keep `wins`' order.
pub fn rank_windows(query: &str, wins: &[WindowName], max: usize) -> Vec<(usize, u32)> {
    let tier = |n: &str| reprisekit::fuzzy::tier(query, n);
    let mut hits: Vec<(usize, u32)> = (wins.iter().enumerate())
        .filter_map(|(i, w)| Some((i, tier(&w.title).max(tier(&w.app))?)))
        .collect();
    hits.sort_by_key(|&(_, t)| std::cmp::Reverse(t));
    hits.truncate(max);
    hits
}

/// Whether the window rows go above the program rows: the best window matches at
/// least as well as the best program, so Enter goes back to a program already open.
pub fn windows_first(best_window: Option<u32>, best_app: Option<u32>) -> bool {
    match (best_window, best_app) {
        (Some(w), Some(a)) => w >= a,
        (w, _) => w.is_some(),
    }
}

const SETTINGS_MIN_CHARS: usize = 2;
const SETTINGS_PROGRAM: &str = "reprise-settings";
pub const CALC_ICONS: [&str; 2] = ["accessories-calculator", "applications-accessories"];
pub const PAGE_ICON: &str = "preferences-system";

/// The calculator row's icon at `px`: the first of [`CALC_ICONS`] that draws
/// ([`crate::icon::unread_first`]).
pub fn calc_icon(
    icons: &mut crate::icontheme::IconCache,
    px: i32,
) -> Option<crate::raster::Canvas> {
    crate::icon::unread_first(icons, CALC_ICONS, px).map(Arc::unwrap_or_clone)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Page(Page),
    /// A control on the page, by the id `--focus` takes.
    Control(Page, &'static str),
    App(usize),
}

impl Setting {
    pub fn name(self, apps: &[AppEntry]) -> String {
        match self {
            Setting::Page(p) | Setting::Control(p, _) => p.label().to_string(),
            Setting::App(i) => apps[i].name.clone(),
        }
    }

    pub fn icon(self, apps: &[AppEntry]) -> Option<String> {
        match self {
            Setting::Page(_) | Setting::Control(..) => Some(PAGE_ICON.to_string()),
            Setting::App(i) => apps[i].icon.clone(),
        }
    }
}

/// The settings pages and programs `query` finds, at most `max`: label, then keyword, then
/// control-label matches. What is not installed, and programs already in `programs`, is left out.
pub fn rank_settings(
    query: &str,
    apps: &[AppEntry],
    programs: &[usize],
    max: usize,
) -> Vec<Setting> {
    if query.trim().chars().count() < SETTINGS_MIN_CHARS {
        return Vec::new();
    }
    let pages = Page::all().filter(|&p| page_installed(p)).filter_map(|p| {
        let hit = p.find(query)?;
        Some((
            hit,
            hit.focus()
                .map_or(Setting::Page(p), |id| Setting::Control(p, id)),
        ))
    });
    let installed = PROGRAMS.iter().filter_map(|p| {
        let i = apps.iter().position(|a| a.id == p.id)?;
        if programs.contains(&i) {
            return None;
        }
        Some((p.find(query, &apps[i].name)?, Setting::App(i)))
    });
    let mut found: Vec<(Hit, Setting)> = pages.chain(installed).collect();
    found.sort_by_key(|&(hit, _)| match hit {
        Hit::Label => 0,
        Hit::Keyword(_) => 1,
        Hit::Control(_) => 2,
    });
    found.into_iter().map(|(_, s)| s).take(max).collect()
}

/// [`Page::installed`]; under test the Login Manager is never installed, whatever the host has.
fn page_installed(p: Page) -> bool {
    match cfg!(test) {
        true => p != Page::LoginManager,
        false => p.installed(),
    }
}

/// The command that opens Settings on `page`, with `focus` (a control id) focused.
fn page_argv(page: Page, focus: Option<&str>) -> Vec<String> {
    let slug = page.slug();
    let mut argv = vec![SETTINGS_PROGRAM, "--page", &slug];
    argv.extend(focus.iter().flat_map(|&id| ["--focus", id]));
    argv.into_iter().map(String::from).collect()
}

/// The answer row's text and what Enter copies: a sum (`= 14`, `14`) or a unit conversion
/// (`5 km = 3.10686 mi`, `3.10686`).
pub fn calc_row(query: &str) -> Option<(String, String)> {
    match reprisekit::calc::answer(query) {
        Some(answer) => Some((format!("= {answer}"), answer)),
        None => reprisekit::units::answer(query),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchOwner {
    Launcher,
    StartPanel,
}

/// Where a section of found rows comes from, in the order the sections show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Names under the home folder ([`reprisefs::search`]).
    Names,
    /// Text and metadata in localsearch's index ([`reprisebus::localsearch`]), when installed and
    /// `[launcher] contents` is on.
    Contents,
    /// A program's search provider ([`reprisebus::searchprovider`]).
    Provider(Arc<Provider>),
}

impl Source {
    pub fn heading(&self) -> &str {
        match self {
            Source::Names => "Files",
            Source::Contents => "Contents",
            Source::Provider(p) => &p.name,
        }
    }

    fn rows(&self, max: usize) -> usize {
        match self {
            Source::Names => max,
            Source::Contents => max.min(CONTENTS_ROWS),
            Source::Provider(_) => max.min(PROVIDER_ROWS),
        }
    }

    /// Whether it searches for `query`: from [`MIN_CHARS`], files only while `files` is on.
    fn searches(&self, query: &str, files: bool) -> bool {
        query.chars().count() >= MIN_CHARS && (files || matches!(self, Source::Provider(_)))
    }

    /// The paths it finds for `query` under `home`, best first, at most `max`. Blocks: runs on a
    /// file job.
    fn find(&self, query: &str, home: &Path, max: usize, cancel: &AtomicBool) -> Vec<PathBuf> {
        match self {
            Source::Names => {
                let q = reprisefs::search::Query::launcher(query, home);
                let paths = reprisefs::search::run(&q, cancel);
                (reprisekit::fuzzy::rank_paths(query, &paths).into_iter())
                    .take(max)
                    .map(|i| paths[i].clone())
                    .collect()
            }
            Source::Contents => contents(query, max, cancel),
            Source::Provider(_) => Vec::new(),
        }
    }

    /// The rows `found` for older text that stay up for `query` until its own answer lands: names
    /// that still match it, every content row and program result.
    fn kept(&self, query: &str, found: &[Found]) -> Vec<Found> {
        match self {
            Source::Names => (found.iter())
                .filter(|f| {
                    f.path().and_then(Path::file_name).is_some_and(|n| {
                        reprisekit::fuzzy::score(query, &n.to_string_lossy()).is_some()
                    })
                })
                .cloned()
                .collect(),
            Source::Contents | Source::Provider(_) => found.to_vec(),
        }
    }
}

const CONTENTS_ROWS: usize = 5;
const PROVIDER_ROWS: usize = 3;
/// How long a content search waits for the indexer's whole answer.
const CONTENTS_TIMEOUT: Duration = Duration::from_secs(3);

/// The local files localsearch finds holding `query`'s words; none when it is not installed.
fn contents(query: &str, max: usize, cancel: &AtomicBool) -> Vec<PathBuf> {
    let found = reprisebus::session()
        .map_err(|e| e.to_string())
        .and_then(|conn| {
            reprisebus::localsearch::search(&conn, query, max, CONTENTS_TIMEOUT, cancel)
        });
    match found {
        Ok(urls) => urls.iter().filter_map(|u| file_path(u)).collect(),
        Err(e) => {
            log::debug!("content search: {e}");
            Vec::new()
        }
    }
}

/// The local path a `file://` URL names.
fn file_path(url: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let rest = url.strip_prefix("file://")?;
    let path = std::ffi::OsString::from_vec(reprisefs::trash::percent_decode(rest));
    let path = PathBuf::from(path);
    path.is_absolute().then_some(path)
}

/// A found row: a file, or a result a program's search provider found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    Path(PathBuf),
    Result(Meta),
}

impl Found {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Found::Path(p) => Some(p),
            Found::Result(_) => None,
        }
    }

    /// What its row is called: the file's name, the result's.
    pub fn name(&self) -> String {
        match self {
            Found::Path(p) => (p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Found::Result(m) => m.name.clone(),
        }
    }
}

fn paths(found: Vec<PathBuf>) -> Vec<Found> {
    found.into_iter().map(Found::Path).collect()
}

/// A section: its source, the source's last answer, and what the section shows of it: the paths
/// an earlier section shows left out.
#[derive(Debug)]
struct Section {
    source: Source,
    found: Vec<Found>,
    shown: Vec<Found>,
}

/// One search box's found rows: each [`Source`]'s section, and what each found path is.
#[derive(Default)]
pub struct FileSearch {
    sections: Vec<Section>,
    /// What each found path is, as the search's job read it: rows, menus and opening ask this
    /// rather than the file system, which may not answer for a found file.
    facts: std::collections::HashMap<PathBuf, crate::desktop::FileFacts>,
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
    /// Dropped to stop the providers' search in flight ([`providers::ask`]).
    asking: Option<async_channel::Sender<()>>,
    timer: Option<RegistrationToken>,
}

impl FileSearch {
    /// Files by name, by contents when `contents` is on, then each of `providers`.
    pub fn new(contents: bool, providers: Vec<Arc<Provider>>) -> Self {
        let sources = std::iter::once(Source::Names)
            .chain(contents.then_some(Source::Contents))
            .chain(providers.into_iter().map(Source::Provider));
        Self {
            sections: sources
                .map(|source| Section {
                    source,
                    found: Vec::new(),
                    shown: Vec::new(),
                })
                .collect(),
            generation: next_generation(),
            ..Self::default()
        }
    }

    /// Each section's source and the rows it shows, in order; a section may be empty.
    pub fn sections(&self) -> impl Iterator<Item = (&Source, &[Found])> {
        (self.sections.iter()).map(|s| (&s.source, s.shown.as_slice()))
    }

    /// Row `row` of section `section`.
    pub fn found(&self, section: usize, row: usize) -> Option<&Found> {
        self.sections.get(section)?.shown.get(row)
    }

    /// Row `row` of section `section`, when it is a file.
    pub fn path(&self, section: usize, row: usize) -> Option<&Path> {
        self.found(section, row)?.path()
    }

    /// Row `row` of section `section`, when it is a program's result, with its provider.
    pub fn result(&self, section: usize, row: usize) -> Option<(&Arc<Provider>, &Meta)> {
        let s = self.sections.get(section)?;
        match (&s.source, s.shown.get(row)?) {
            (Source::Provider(p), Found::Result(m)) => Some((p, m)),
            _ => None,
        }
    }

    pub fn is_dir(&self, p: &Path) -> bool {
        self.facts.get(p).is_some_and(|f| f.is_dir)
    }

    /// What found path `p` is; a plain file when the search did not read it.
    pub fn facts_of(&self, p: &Path) -> crate::desktop::FileFacts {
        (self.facts.get(p).cloned()).unwrap_or_else(|| crate::desktop::FileFacts::plain(p))
    }

    /// Section `section`'s answer: `found`, best first.
    pub(crate) fn set(&mut self, section: usize, found: Vec<Found>) {
        let Some(s) = self.sections.get_mut(section) else {
            return;
        };
        s.found = found;
        let mut seen = std::collections::HashSet::new();
        for s in &mut self.sections {
            s.shown = (s.found.iter())
                .filter(|f| f.path().is_none_or(|p| seen.insert(p.to_path_buf())))
                .cloned()
                .collect();
        }
        let sections = &self.sections;
        self.facts.retain(|p, _| {
            (sections.iter()).any(|s| s.found.iter().any(|f| f.path() == Some(p.as_path())))
        });
    }

    fn bump(&mut self) -> u64 {
        self.generation = next_generation();
        if let Some(c) = self.cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.asking = None;
        self.generation
    }

    /// The sections a search for `query` asks, with their sources.
    fn asked(&self, query: &str, files: bool) -> Vec<(usize, Source)> {
        (self.sections.iter().enumerate())
            .filter(|(_, s)| s.source.searches(query, files))
            .map(|(i, s)| (i, s.source.clone()))
            .collect()
    }

    fn accepts(&self, generation: u64) -> bool {
        generation == self.generation
    }

    pub fn stop(&mut self, handle: &LoopHandle<State>) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::Relaxed);
        }
        self.asking = None;
        if let Some(t) = self.timer.take() {
            handle.remove(t);
        }
    }
}

/// The folder file searches look in: `$HOME`; in tests the test process's own, so no test
/// finds the user's files.
fn search_root() -> Option<PathBuf> {
    #[cfg(test)]
    return Some(crate::harness::scratch());
    #[cfg(not(test))]
    std::env::var_os("HOME").map(PathBuf::from)
}

pub(crate) fn tilde(dir: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| dir.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => dir.display().to_string(),
    }
}

pub(crate) fn file_icon_name(
    p: &Path,
    is_dir: bool,
    mime: &reprisefs::apps::MimeDb,
    icons: &crate::icontheme::IconCache,
) -> String {
    if is_dir {
        return "folder".to_string();
    }
    let m = p
        .file_name()
        .and_then(|n| mime.mime_of_name(&n.to_string_lossy()));
    reprisefs::icons::pick_icon(m.as_deref(), p, &|n| icons.has_unread(n))
}

/// The type icon a file row shows for `p` at `px` ([`crate::icon::unread`]).
pub(crate) fn file_row_icon(
    p: &Path,
    is_dir: bool,
    mime: &reprisefs::apps::MimeDb,
    icons: &mut crate::icontheme::IconCache,
    px: i32,
) -> Option<crate::raster::Canvas> {
    let icon = file_icon_name(p, is_dir, mime, icons);
    crate::icon::unread(icons, &icon, px).map(std::sync::Arc::unwrap_or_clone)
}

impl FileSearch {
    /// Row `f` of `source`'s section as the launcher shows it, its icon at `px`.
    pub(crate) fn row(
        &self,
        source: &Source,
        f: &Found,
        home: Option<&Path>,
        mime: &reprisefs::apps::MimeDb,
        icons: &mut crate::icontheme::IconCache,
        px: i32,
    ) -> FoundRow {
        let detail = match f {
            Found::Path(p) => p.parent().map(|d| tilde(d, home)).unwrap_or_default(),
            Found::Result(m) => m.description.clone(),
        };
        FoundRow {
            name: f.name(),
            detail,
            icon: self.icon(source, f, mime, icons, px),
        }
    }

    /// Row `f`'s icon at `px`: a file's type, a result's own or its program's.
    pub(crate) fn icon(
        &self,
        source: &Source,
        f: &Found,
        mime: &reprisefs::apps::MimeDb,
        icons: &mut crate::icontheme::IconCache,
        px: i32,
    ) -> Option<crate::raster::Canvas> {
        match (f, source) {
            (Found::Path(p), _) => file_row_icon(p, self.is_dir(p), mime, icons, px),
            (Found::Result(m), Source::Provider(p)) => p.result_icon(m, icons, px),
            (Found::Result(_), _) => None,
        }
    }
}

impl State {
    pub fn open_setting(&mut self, s: Setting) {
        match s {
            Setting::Page(p) => self.spawn_argv_activated(&page_argv(p, None)),
            Setting::Control(p, id) => self.spawn_argv_activated(&page_argv(p, Some(id))),
            Setting::App(i) => {
                if let Some(entry) = self.app_index.get(i).cloned() {
                    self.launch_app(&entry);
                }
            }
        }
    }

    /// A new search box's sections ([`FileSearch::new`]) with the sources turned on.
    pub fn new_search(&self) -> FileSearch {
        let c = &self.config;
        FileSearch::new(
            c.launcher_contents,
            providers::enabled(
                &self.search_providers,
                &self.app_index,
                &c.launcher_providers,
                &c.launcher_providers_off,
                c.launcher_contents,
            ),
        )
    }

    pub fn file_search_mut(&mut self, o: SearchOwner) -> Option<&mut FileSearch> {
        match o {
            SearchOwner::Launcher => self.launcher.as_mut().map(|v| &mut v.search),
            SearchOwner::StartPanel => self.start_panel.as_mut().map(|v| &mut v.search),
        }
    }

    /// The text changed: stop the search in flight, keep each section's rows that still fit the
    /// new text ([`Source::kept`]) and arm the timer for the next search. A section that does not
    /// search for the text ([`Source::searches`]) empties.
    pub fn file_query_changed(&mut self, o: SearchOwner, query: &str, max: usize) {
        let files = self.config.launcher_files;
        let Some(f) = self.file_search_mut(o) else {
            return;
        };
        f.bump();
        let mut searching = false;
        for i in 0..f.sections.len() {
            let s = &f.sections[i];
            let kept = match s.source.searches(query, files) {
                true => s.source.kept(query, &s.found),
                false => Vec::new(),
            };
            searching |= s.source.searches(query, files);
            f.set(i, kept);
        }
        if let Some(t) = f.timer.take() {
            self.handle.remove(t);
        }
        if !searching {
            return;
        }
        let query = query.to_string();
        let token =
            self.handle
                .insert_source(Timer::from_duration(DEBOUNCE), move |_, _, state| {
                    if let Some(f) = state.file_search_mut(o) {
                        f.timer = None;
                    }
                    state.file_search_fire(o, &query, max);
                    TimeoutAction::Drop
                });
        match token {
            Ok(t) => {
                if let Some(f) = self.file_search_mut(o) {
                    f.timer = Some(t);
                }
            }
            Err(e) => log::warn!("file search: no timer: {}", e.error),
        }
    }

    /// Starts every section's search for `query`: files on file jobs, the providers together on a
    /// thread of their own ([`providers::ask`]); each section lands on its own.
    fn file_search_fire(&mut self, o: SearchOwner, query: &str, max: usize) {
        let Some(home) = search_root() else {
            return;
        };
        let files = self.config.launcher_files;
        let sender = self.fsjobs.sender();
        let Some(f) = self.file_search_mut(o) else {
            return;
        };
        if let Some(c) = f.cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        f.cancel = Some(cancel.clone());
        let generation = f.generation;
        let sources = f.asked(query, files);
        let mut asked = Vec::new();
        for (section, source) in sources {
            let max = source.rows(max);
            if let Source::Provider(p) = source {
                asked.push((section, p, max));
                continue;
            }
            let (query, home, cancel) = (query.to_string(), home.clone(), cancel.clone());
            self.spawn_fs_job(move || {
                if cancel.load(Ordering::Relaxed) {
                    return Box::new(|_| {});
                }
                let best = source.find(&query, &home, max, &cancel);
                let facts = (best.iter())
                    .map(|p| (p.clone(), crate::desktop::file_facts(p)))
                    .collect();
                Box::new(move |state: &mut State| {
                    state.files_found(o, generation, section, paths(best), facts)
                })
            });
        }
        if let (false, Some(sender)) = (asked.is_empty(), sender) {
            let asking = providers::ask(asked, query, sender, move |section, found| {
                Box::new(move |state: &mut State| {
                    state.files_found(o, generation, section, found, Default::default())
                })
            });
            if let Some(f) = self.file_search_mut(o) {
                f.asking = asking;
            }
        }
    }

    fn files_found(
        &mut self,
        o: SearchOwner,
        generation: u64,
        section: usize,
        found: Vec<Found>,
        facts: std::collections::HashMap<PathBuf, crate::desktop::FileFacts>,
    ) {
        let Some(f) = self.file_search_mut(o) else {
            return;
        };
        if !f.accepts(generation) {
            return;
        }
        f.facts.extend(facts);
        f.set(section, found);
        match o {
            SearchOwner::Launcher => {
                self.show_launcher_found();
                self.redraw_launcher();
            }
            SearchOwner::StartPanel => self.start_panel_found(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A found row's icon comes from what the search's job found, with no stat: a path that does
    /// not exist wears the folder icon when it was found as a folder.
    fn win(title: &str, app: &str) -> WindowName {
        WindowName {
            title: title.into(),
            app: app.into(),
        }
    }

    #[test]
    fn a_window_matches_by_title_or_program_name_best_first() {
        let wins = [
            win("notes.txt - Text Editor", "Text Editor"),
            win("Reprise - Mozilla Firefox", "Firefox"),
            win("Terminal", "Terminal"),
        ];
        let found: Vec<usize> = rank_windows("fire", &wins, 5).iter().map(|h| h.0).collect();
        assert_eq!(found, [1], "the program's name finds Firefox");
        let found: Vec<usize> = rank_windows("notes", &wins, 5)
            .iter()
            .map(|h| h.0)
            .collect();
        assert_eq!(found, [0], "the title finds the editor");
        let found: Vec<usize> = rank_windows("te", &wins, 5).iter().map(|h| h.0).collect();
        assert_eq!(found, [0, 2], "equal matches in the given order");
        assert_eq!(rank_windows("te", &wins, 1).len(), 1);
        assert!(rank_windows("", &wins, 5).is_empty());
        assert!(rank_windows("zzz", &wins, 5).is_empty());
    }

    #[test]
    fn windows_go_first_when_they_match_at_least_as_well_as_the_best_program() {
        let wins = [win("Reprise - Mozilla Firefox", "Firefox")];
        let w = rank_windows("fire", &wins, 5).first().map(|h| h.1);
        let app = reprisekit::fuzzy::tier("fire", "Firefox");
        assert!(windows_first(w, app), "an open Firefox beats starting one");
        let w = rank_windows("ff", &wins, 5).first().map(|h| h.1);
        let app = reprisekit::fuzzy::tier("ff", "FF Tool");
        assert!(!windows_first(w, app), "a better program match leads");
        assert!(windows_first(Some(1), None));
        assert!(!windows_first(None, Some(1)));
        assert!(!windows_first(None, None));
    }

    #[test]
    fn a_found_rows_icon_asks_no_stat() {
        let theme = reprisekit::icontheme::IconTheme::load("test", &[]);
        let icons = reprisekit::icontheme::IconCache::new(theme);
        let mime = reprisefs::apps::MimeDb::empty();
        let gone = Path::new("/nonexistent/reprise-found/Reports");
        assert_eq!(file_icon_name(gone, true, &mime, &icons), "folder");
        assert_eq!(file_icon_name(gone, false, &mime, &icons), "text-x-generic");
    }

    fn app(name: &str, shown: bool) -> AppEntry {
        AppEntry {
            name: name.into(),
            exec_line: name.to_lowercase(),
            terminal: false,
            id: format!("{name}.desktop"),
            icon: None,
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            shown,
        }
    }

    #[test]
    fn rank_apps_drops_hidden_entries_and_caps() {
        let apps = [
            app("Notepad", true),
            app("Notes hidden", false),
            app("Nautilus", true),
        ];
        assert_eq!(rank_apps("n", &apps, 1).len(), 1);
        assert!(!rank_apps("notes", &apps, 8).contains(&1));
    }

    fn installed(id: &str, name: &str) -> AppEntry {
        AppEntry {
            id: id.into(),
            ..app(name, true)
        }
    }

    fn as_pages(found: Vec<Setting>) -> Vec<Setting> {
        found
            .into_iter()
            .map(|s| match s {
                Setting::Control(p, _) => Setting::Page(p),
                s => s,
            })
            .collect()
    }

    #[test]
    fn settings_are_found_by_name_then_by_keyword() {
        let apps = [
            installed("reprise-network.desktop", "Network Connections"),
            installed("reprise-power.desktop", "Power Options"),
            installed("reprise-printers.desktop", "Printers"),
            installed("reprise-updates.desktop", "Software Updates"),
        ];
        let rank = |q: &str| as_pages(rank_settings(q, &apps, &[], 8));
        assert_eq!(rank("wifi"), [Setting::App(0)]);
        assert_eq!(rank("Wi-Fi"), [Setting::App(0)]);
        assert_eq!(
            rank("mouse"),
            [
                Setting::Page(Page::Mouse),
                Setting::Page(Page::Touchpad),
                Setting::Page(Page::PointerAlerts)
            ],
            "the Mouse page by name, then Touchpad by its external mouse and Pointer & Alerts by MouseKeys"
        );
        assert_eq!(rank("touchpad"), [Setting::Page(Page::Touchpad)]);
        assert_eq!(
            rank("cursor"),
            [
                Setting::Page(Page::Cursor),
                Setting::Page(Page::MagnifierFilters)
            ],
            "the Cursor page by name, then Magnifier & Filters by its text cursor check"
        );
        assert_eq!(rank("power"), [Setting::App(1)], "Power Options by name");
        assert_eq!(
            rank("dis")[..2],
            [
                Setting::Page(Page::Display),
                Setting::Page(Page::Notifications)
            ],
            "a word's start, not its middle: Display, then \"do not disturb\""
        );
        assert_eq!(
            rank("sleep"),
            [Setting::App(1), Setting::Page(Page::ScreenSaverLock)],
            "a Power Options word, then the lock-on-sleep check"
        );
        assert_eq!(rank("lid"), [Setting::App(1)]);
        assert_eq!(rank("transparency"), [Setting::Page(Page::Appearance)]);
        assert_eq!(rank("dpi"), [Setting::Page(Page::Display)]);
        assert_eq!(
            rank("lock screen"),
            [
                Setting::Page(Page::ScreenSaverLock),
                Setting::Page(Page::Effects),
                Setting::Page(Page::Keyboard),
                Setting::Page(Page::RemoteDesktop)
            ],
            "Effects by its Animate the lock screen box, Remote Desktop by its wait before locking \
             on a screen change"
        );
        assert_eq!(rank("hot corner"), [Setting::Page(Page::Switcher)]);
        assert_eq!(rank("notifications"), [Setting::Page(Page::Notifications)]);
        assert_eq!(rank("quiet hours"), [Setting::Page(Page::Notifications)]);
        assert_eq!(rank("clipboard"), [Setting::Page(Page::Privacy)]);
        assert_eq!(rank("print queue"), [Setting::App(2)]);
        assert_eq!(rank("firmware"), [Setting::App(3)]);
        assert_eq!(rank("grayscale"), [Setting::Page(Page::MagnifierFilters)]);
        assert_eq!(
            rank("colour blind"),
            [Setting::Page(Page::MagnifierFilters)]
        );
        assert_eq!(rank("magnifier"), [Setting::Page(Page::MagnifierFilters)]);
        assert!(rank("w").is_empty(), "one character finds nothing");
        assert!(rank("notepad").is_empty());
    }

    #[test]
    fn the_page_is_found_by_rdp_and_remmina() {
        let rank = |q: &str| as_pages(rank_settings(q, &[], &[], 8));
        for q in ["rdp", "remmina", "desktop sharing"] {
            assert_eq!(rank(q), [Setting::Page(Page::RemoteDesktop)], "{q}");
        }
        assert_eq!(
            rank("remote"),
            [
                Setting::Page(Page::RemoteDesktop),
                Setting::Page(Page::Privacy)
            ],
            "remote: the Remote Desktop page, then Privacy's remote control grants"
        );
    }

    #[test]
    fn a_control_label_finds_its_page_after_names_and_keywords() {
        let rank = |q: &str| rank_settings(q, &[], &[], 8);
        // The tier a page's row ranks in, read back from the page's own hit.
        let tier = |q: &str, s: &Setting| match s {
            Setting::Page(p) | Setting::Control(p, _) => match p.find(q) {
                Some(Hit::Label) => 0,
                Some(Hit::Keyword(_)) => 1,
                Some(Hit::Control(_)) => 2,
                None => panic!("{p:?} is listed for {q:?}"),
            },
            Setting::App(_) => panic!("no programs installed"),
        };
        assert!(rank("repeat delay").contains(&Setting::Control(Page::Keyboard, "repeat_delay")));
        for q in ["keyboard", "hours", "dis"] {
            let tiers: Vec<u8> = rank(q).iter().map(|s| tier(q, s)).collect();
            assert!(tiers.is_sorted(), "{q}: {tiers:?}");
        }
        assert_eq!(
            rank("keyboard").first(),
            Some(&Setting::Page(Page::Keyboard))
        );
        assert_eq!(
            rank("hours")[..2],
            [
                Setting::Control(Page::Notifications, "quiet_hours"),
                Setting::Control(Page::Appearance, "glass_dark_from")
            ],
            "the keyword's control before an earlier page's control label"
        );
    }

    #[test]
    fn a_keyword_naming_one_control_opens_it_focused_in_the_keyword_tier() {
        let first = |q: &str| rank_settings(q, &[], &[], 8).first().copied();
        for (q, page, id) in [
            ("history", Page::Privacy, "clipboard_history"),
            ("quiet hours", Page::Notifications, "quiet_hours"),
            ("3d", Page::Switcher, "switcher_flip"),
            ("default browser", Page::DefaultPrograms, "dp_browser"),
        ] {
            assert_eq!(first(q), Some(Setting::Control(page, id)), "{q}");
        }
        assert_eq!(
            first("mouse speed"),
            Some(Setting::Page(Page::Mouse)),
            "three speeds: the page opens unfocused"
        );
    }

    #[test]
    fn a_control_row_opens_its_page_focused() {
        assert_eq!(
            page_argv(Page::Keyboard, Some("repeat_delay")),
            [
                "reprise-settings",
                "--page",
                "keyboard",
                "--focus",
                "repeat_delay"
            ]
        );
        assert_eq!(
            page_argv(Page::NightLight, None),
            ["reprise-settings", "--page", "night-light"]
        );
    }

    #[test]
    fn a_settings_program_listed_or_missing_is_left_out() {
        let apps = [installed("reprise-network.desktop", "Network Connections")];
        let no_app = |found: Vec<Setting>| !found.iter().any(|s| matches!(s, Setting::App(_)));
        assert!(no_app(rank_settings("network", &apps, &[0], 8)));
        assert!(no_app(rank_settings("bluetooth", &apps, &[], 8)));
        assert_eq!(rank_settings("sc", &apps, &[], 1).len(), 1, "capped");
    }

    #[test]
    fn a_sum_gets_an_answer_row_and_a_plain_number_none() {
        assert_eq!(calc_row("2+3*4"), Some(("= 14".into(), "14".into())));
        assert_eq!(calc_row("42"), None);
        assert_eq!(calc_row("notes"), None);
        assert_eq!(
            calc_row("5 km in mi"),
            Some(("5 km = 3.10686 mi".into(), "3.10686".into()))
        );
    }

    #[test]
    fn a_stale_answer_is_dropped() {
        let mut f = FileSearch::new(true, Vec::new());
        let g = f.bump();
        let now = f.bump();
        assert!(!f.accepts(g));
        assert!(f.accepts(now));
    }

    #[test]
    fn an_answer_for_a_closed_search_box_fits_no_later_one() {
        let mut first = FileSearch::new(true, Vec::new());
        let stale = first.bump();
        let mut second = FileSearch::new(true, Vec::new());
        assert!(!second.accepts(stale));
        second.bump();
        assert!(!second.accepts(stale), "after the second's own bump too");
    }

    fn found(ps: &[&str]) -> Vec<Found> {
        ps.iter().map(|p| Found::Path(p.into())).collect()
    }

    fn calc() -> Arc<Provider> {
        Arc::new(Provider {
            addr: reprisebus::searchprovider::Address {
                bus_name: "org.example.Calc".into(),
                path: "/".into(),
            },
            name: "Calculator".into(),
            icon: None,
        })
    }

    fn result(id: &str) -> Found {
        Found::Result(Meta {
            id: id.into(),
            name: format!("{id} = 5"),
            description: String::new(),
            gicon: None,
        })
    }

    #[test]
    fn a_file_found_by_name_is_left_out_of_the_contents_whichever_lands_first() {
        let mut f = FileSearch::new(true, Vec::new());
        f.set(1, found(&["/h/a.txt", "/h/b.pdf"]));
        let shown = |f: &FileSearch| -> Vec<(Source, Vec<Found>)> {
            f.sections().map(|(s, p)| (s.clone(), p.to_vec())).collect()
        };
        assert_eq!(
            shown(&f),
            [
                (Source::Names, vec![]),
                (Source::Contents, found(&["/h/a.txt", "/h/b.pdf"]))
            ]
        );
        f.set(0, found(&["/h/b.pdf", "/h/c.md"]));
        assert_eq!(
            shown(&f),
            [
                (Source::Names, found(&["/h/b.pdf", "/h/c.md"])),
                (Source::Contents, found(&["/h/a.txt"]))
            ]
        );
        assert_eq!(f.path(1, 0), Some(Path::new("/h/a.txt")));
        assert_eq!(f.path(1, 1), None);
        f.set(0, Vec::new());
        assert_eq!(
            f.path(1, 1),
            Some(Path::new("/h/b.pdf")),
            "back once names drop it"
        );
    }

    #[test]
    fn each_provider_gets_a_section_after_the_files_with_its_results() {
        let p = calc();
        let mut f = FileSearch::new(true, vec![p.clone()]);
        let heads: Vec<&str> = f.sections().map(|(s, _)| s.heading()).collect();
        assert_eq!(heads, ["Files", "Contents", "Calculator"]);
        f.set(2, vec![result("a"), result("b")]);
        let (provider, meta) = f.result(2, 1).expect("a program's result");
        assert_eq!((provider, meta.id.as_str()), (&p, "b"));
        assert_eq!(f.path(2, 0), None, "a result is no file");
        assert_eq!(f.result(0, 0), None);
    }

    #[test]
    fn content_search_off_has_no_contents_section_and_asks_only_names_and_programs() {
        let off = FileSearch::new(false, vec![calc()]);
        let heads: Vec<&str> = off.sections().map(|(s, _)| s.heading()).collect();
        assert_eq!(heads, ["Files", "Calculator"]);
        let asked: Vec<Source> = (off.asked("quarterly", true).into_iter())
            .map(|(_, s)| s)
            .collect();
        assert_eq!(asked, [Source::Names, Source::Provider(calc())]);
        let on = FileSearch::new(true, vec![calc()]);
        let asked: Vec<Source> = (on.asked("quarterly", true).into_iter())
            .map(|(_, s)| s)
            .collect();
        assert_eq!(
            asked,
            [Source::Names, Source::Contents, Source::Provider(calc())]
        );
    }

    #[test]
    fn new_text_keeps_the_names_that_still_match_and_every_content_and_program_row() {
        let rows = found(&["/h/notes.txt", "/h/budget.ods"]);
        assert_eq!(Source::Names.kept("note", &rows), found(&["/h/notes.txt"]));
        assert_eq!(Source::Contents.kept("note", &rows), rows);
        let results = vec![result("a")];
        assert_eq!(Source::Provider(calc()).kept("x", &results), results);
    }

    #[test]
    fn contents_show_at_most_five_rows_and_a_program_three() {
        assert_eq!(Source::Names.rows(8), 8);
        assert_eq!(Source::Contents.rows(8), 5);
        assert_eq!(Source::Contents.rows(3), 3);
        assert_eq!(Source::Provider(calc()).rows(8), 3);
    }

    #[test]
    fn programs_are_asked_from_two_characters_even_with_file_search_off() {
        let p = Source::Provider(calc());
        assert!(!p.searches("a", true));
        assert!(p.searches("ab", false));
        assert!(!Source::Names.searches("a", true));
        assert!(Source::Names.searches("ab", true));
        assert!(!Source::Contents.searches("ab", false));
    }

    #[test]
    fn a_found_url_names_a_local_path() {
        assert_eq!(
            file_path("file:///home/u/Q3%20report.pdf"),
            Some(PathBuf::from("/home/u/Q3 report.pdf"))
        );
        assert_eq!(file_path("https://example.com/a"), None);
        assert_eq!(file_path("file://host/a"), None);
    }

    #[test]
    fn a_file_row_writes_its_home_folder_as_a_tilde() {
        let home = Path::new("/home/u");
        assert_eq!(tilde(Path::new("/home/u"), Some(home)), "~");
        assert_eq!(tilde(Path::new("/home/u/Docs/a"), Some(home)), "~/Docs/a");
        assert_eq!(tilde(Path::new("/home/user2"), Some(home)), "/home/user2");
        assert_eq!(tilde(Path::new("/srv"), None), "/srv");
    }
}
