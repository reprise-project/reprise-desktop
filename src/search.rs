//! What the launcher and the Glass Start menu's search box share: the calculator
//! row, fuzzy app ranking, settings rows and the debounced file search. Every
//! text change cancels the search in flight and takes a new generation from one
//! process-wide counter, so a late answer for older text, or for a closed box, is dropped.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use repriseconf::pages::{PROGRAMS, Page};
use reprisefs::apps::AppEntry;
use reprisekit::controls::launcher::FileRow;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{LoopHandle, RegistrationToken};

use crate::compositor::State;

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

/// Whether a word of `name` starts with `query`, the rest of `query` running on
/// from there ("sticky no" fits "Sticky Notes"). The Start menu lists only such names.
pub fn word_starts(query: &str, name: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return false;
    }
    let name = name.to_lowercase();
    let split = |c: char| matches!(c, ' ' | '-' | '_' | '.');
    let mut word_start = true;
    for (i, c) in name.char_indices() {
        if word_start && !split(c) && name[i..].starts_with(&query) {
            return true;
        }
        word_start = split(c);
    }
    false
}

const SETTINGS_MIN_CHARS: usize = 2;
const SETTINGS_PROGRAM: &str = "reprise-settings";
pub const CALC_ICON: &str = "accessories-calculator";
pub const PAGE_ICON: &str = "preferences-system";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Page(Page),
    App(usize),
}

impl Setting {
    pub fn name(self, apps: &[AppEntry]) -> String {
        match self {
            Setting::Page(p) => p.label().to_string(),
            Setting::App(i) => apps[i].name.clone(),
        }
    }

    pub fn icon(self, apps: &[AppEntry]) -> Option<String> {
        match self {
            Setting::Page(_) => Some(PAGE_ICON.to_string()),
            Setting::App(i) => apps[i].icon.clone(),
        }
    }
}

/// The settings pages and programs `query` finds, at most `max`: name-word
/// matches, then keyword matches. Programs not installed or already in `programs` are left out.
pub fn rank_settings(
    query: &str,
    apps: &[AppEntry],
    programs: &[usize],
    max: usize,
) -> Vec<Setting> {
    if query.trim().chars().count() < SETTINGS_MIN_CHARS {
        return Vec::new();
    }
    let pages = Page::all().map(|p| (Setting::Page(p), p.label().to_string(), p.keywords()));
    let installed = PROGRAMS.iter().filter_map(|p| {
        let i = apps.iter().position(|a| a.id == p.id)?;
        (!programs.contains(&i)).then(|| (Setting::App(i), apps[i].name.clone(), p.keywords))
    });
    let all: Vec<_> = pages.chain(installed).collect();
    let by_name = all.iter().filter(|(_, name, _)| word_starts(query, name));
    let by_keyword = all.iter().filter(|(_, name, keywords)| {
        !word_starts(query, name) && keywords.iter().any(|k| word_starts(query, k))
    });
    by_name
        .chain(by_keyword)
        .map(|(s, ..)| *s)
        .take(max)
        .collect()
}

pub fn calc_row(query: &str) -> Option<(String, String)> {
    let answer = crate::calc::answer(query)?;
    Some((format!("= {answer}"), answer))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchOwner {
    Launcher,
    StartPanel,
}

#[derive(Default)]
pub struct FileSearch {
    pub files: Vec<PathBuf>,
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
    timer: Option<RegistrationToken>,
}

impl FileSearch {
    pub fn new() -> Self {
        Self {
            generation: next_generation(),
            ..Self::default()
        }
    }

    fn bump(&mut self) -> u64 {
        self.generation = next_generation();
        if let Some(c) = self.cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.generation
    }

    fn accepts(&self, generation: u64) -> bool {
        generation == self.generation
    }

    pub fn stop(&mut self, handle: &LoopHandle<State>) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::Relaxed);
        }
        if let Some(t) = self.timer.take() {
            handle.remove(t);
        }
    }
}

fn searches_files(query: &str, enabled: bool) -> bool {
    enabled && query.chars().count() >= MIN_CHARS
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
    mime: &reprisefs::apps::MimeDb,
    icons: &crate::icontheme::IconCache,
) -> String {
    if p.is_dir() {
        return "folder".to_string();
    }
    let m = p
        .file_name()
        .and_then(|n| mime.mime_of_name(&n.to_string_lossy()));
    reprisefs::icons::pick_icon(m.as_deref(), p, &|n| icons.has(n))
}

pub(crate) fn file_row(
    p: &Path,
    home: Option<&Path>,
    mime: &reprisefs::apps::MimeDb,
    icons: &mut crate::icontheme::IconCache,
    px: i32,
) -> FileRow {
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let icon = file_icon_name(p, mime, icons);
    FileRow {
        name,
        folder: p.parent().map(|d| tilde(d, home)).unwrap_or_default(),
        icon: icons.get(&icon, px),
    }
}

impl State {
    pub fn open_setting(&mut self, s: Setting) {
        match s {
            Setting::Page(p) => {
                self.spawn_argv_activated(&[SETTINGS_PROGRAM, "--page", &p.slug()]);
            }
            Setting::App(i) => {
                if let Some(entry) = self.app_index.get(i).cloned() {
                    self.launch_app(&entry);
                }
            }
        }
    }

    pub fn file_search_mut(&mut self, o: SearchOwner) -> Option<&mut FileSearch> {
        match o {
            SearchOwner::Launcher => self.launcher.as_mut().map(|v| &mut v.search),
            SearchOwner::StartPanel => self.start_panel.as_mut().map(|v| &mut v.search),
        }
    }

    pub fn file_query_changed(&mut self, o: SearchOwner, query: &str, max: usize) -> Vec<PathBuf> {
        let searching = searches_files(query, self.config.launcher_files);
        let Some(f) = self.file_search_mut(o) else {
            return Vec::new();
        };
        f.bump();
        let kept: Vec<PathBuf> = if searching {
            f.files
                .iter()
                .filter(|p| {
                    p.file_name().is_some_and(|n| {
                        reprisekit::fuzzy::score(query, &n.to_string_lossy()).is_some()
                    })
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let old = f.timer.take();
        if let Some(t) = old {
            self.handle.remove(t);
        }
        if searching {
            let query = query.to_string();
            let token =
                self.handle
                    .insert_source(Timer::from_duration(DEBOUNCE), move |_, _, state| {
                        if let Some(f) = state.file_search_mut(o) {
                            f.timer = None;
                        }
                        state.file_search_fire(o, query.clone(), max);
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
        kept
    }

    fn file_search_fire(&mut self, o: SearchOwner, query: String, max: usize) {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        if !searches_files(&query, self.config.launcher_files) {
            return;
        }
        let Some(f) = self.file_search_mut(o) else {
            return;
        };
        if let Some(c) = f.cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        f.cancel = Some(cancel.clone());
        let generation = f.generation;
        self.spawn_fs_job(move || {
            let q = reprisefs::search::Query::launcher(&query, &home);
            let paths = reprisefs::search::run(&q, &cancel);
            let best: Vec<PathBuf> = reprisekit::fuzzy::rank_paths(&query, &paths)
                .into_iter()
                .take(max)
                .map(|i| paths[i].clone())
                .collect();
            Box::new(move |state: &mut State| state.files_found(o, generation, best))
        });
    }

    fn files_found(&mut self, o: SearchOwner, generation: u64, paths: Vec<PathBuf>) {
        let Some(f) = self.file_search_mut(o) else {
            return;
        };
        if !f.accepts(generation) {
            return;
        }
        f.cancel = None;
        match o {
            SearchOwner::Launcher => {
                self.show_launcher_files(paths);
                self.redraw_launcher();
            }
            SearchOwner::StartPanel => self.start_panel_files(paths),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_word_of_the_name_starts_with_the_text() {
        for name in [
            "Notepad",
            "Sticky Notes",
            "GNOME no-op",
            "file_notes",
            "a.Note",
        ] {
            assert!(word_starts("no", name), "{name}");
        }
        for name in ["Network Connections", "Print Editor", "Piano"] {
            assert!(!word_starts("no", name), "{name}");
        }
        assert!(word_starts("NO", "Notepad"), "case-insensitive");
        assert!(word_starts("sticky no", "Sticky Notes"));
        assert!(!word_starts("", "Notepad"));
    }

    fn installed(id: &str, name: &str) -> AppEntry {
        AppEntry {
            id: id.into(),
            ..app(name, true)
        }
    }

    #[test]
    fn settings_are_found_by_name_then_by_keyword() {
        let apps = [
            installed("reprise-network.desktop", "Network Connections"),
            installed("reprise-power.desktop", "Power Options"),
        ];
        let rank = |q: &str| rank_settings(q, &apps, &[], 8);
        assert_eq!(rank("wifi"), [Setting::App(0)]);
        assert_eq!(rank("Wi-Fi"), [Setting::App(0)]);
        assert_eq!(
            rank("mouse"),
            [
                Setting::Page(Page::Mouse),
                Setting::Page(Page::Accessibility)
            ],
            "the Mouse page by name, then Accessibility by MouseKeys"
        );
        assert_eq!(rank("touchpad"), [Setting::Page(Page::Mouse)]);
        assert_eq!(rank("cursor"), [Setting::Page(Page::Pointers)]);
        assert_eq!(
            rank("power"),
            [Setting::Page(Page::Power), Setting::App(1)],
            "names in list order, pages first"
        );
        assert_eq!(
            rank("dis"),
            [Setting::Page(Page::Display)],
            "a word's start, not its middle"
        );
        assert_eq!(
            rank("sleep"),
            [Setting::Page(Page::Power)],
            "a keyword after no name"
        );
        assert_eq!(rank("grayscale"), [Setting::Page(Page::Vision)]);
        assert_eq!(rank("colour blind"), [Setting::Page(Page::Vision)]);
        assert_eq!(rank("magnifier"), [Setting::Page(Page::Vision)]);
        assert!(rank("w").is_empty(), "one character finds nothing");
        assert!(rank("notepad").is_empty());
    }

    #[test]
    fn a_settings_program_listed_or_missing_is_left_out() {
        let apps = [installed("reprise-network.desktop", "Network Connections")];
        assert!(rank_settings("network", &apps, &[0], 8).is_empty());
        assert!(rank_settings("bluetooth", &apps, &[], 8).is_empty());
        assert_eq!(rank_settings("sc", &apps, &[], 1).len(), 1, "capped");
    }

    #[test]
    fn a_sum_gets_an_answer_row_and_a_plain_number_none() {
        assert_eq!(calc_row("2+3*4"), Some(("= 14".into(), "14".into())));
        assert_eq!(calc_row("42"), None);
        assert_eq!(calc_row("notes"), None);
    }

    #[test]
    fn a_stale_answer_is_dropped() {
        let mut f = FileSearch::new();
        let g = f.bump();
        let now = f.bump();
        assert!(!f.accepts(g));
        assert!(f.accepts(now));
    }

    #[test]
    fn an_answer_for_a_closed_search_box_fits_no_later_one() {
        let mut first = FileSearch::new();
        let stale = first.bump();
        let mut second = FileSearch::new();
        assert!(!second.accepts(stale));
        second.bump();
        assert!(!second.accepts(stale), "after the second's own bump too");
    }

    #[test]
    fn files_are_searched_from_two_characters_unless_turned_off() {
        assert!(!searches_files("a", true));
        assert!(searches_files("ab", true));
        assert!(!searches_files("ab", false));
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
