use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use reprisebus::tray::WindowIcon;
use reprisefs::apps::{self, AppEntry};
use reprisekit::controls::openwith::PROGRAM_ICON;
use reprisekit::icon::IconSource;
use reprisekit::icontheme::{IconCache, IconTheme};
use reprisekit::raster::Canvas;

use crate::pipewire::App;

/// A program installed a minute ago should still find its icon.
const RESCAN_AFTER: Duration = Duration::from_secs(30);

/// A program's window may map after its stream starts, so a window that was not found is asked
/// for again after this.
const WINDOW_RETRY: Duration = Duration::from_secs(5);

/// Asks the compositor for the icon of the window playing (pids, title), off the UI thread.
type Ask = Arc<dyn Fn(&[u32], &str) -> Option<WindowIcon> + Send + Sync>;

/// A window lookup's answer for an `App::key`.
type Landed = (String, Option<WindowIcon>);

/// What the compositor said about a program's window.
enum Looked {
    Asking,
    Landed(WindowIcon),
    Found(AppIcon),
    Missed(Instant),
}

#[derive(Clone, Debug, Default)]
pub struct AppIcon {
    pub small: Option<Canvas>,
    pub large: Option<Canvas>,
}

/// Finds each playing program's icon, first match wins: an icon hint that resolves, the icon of
/// the window playing it, the name and executable heuristics of [`icon_name`]. The tray runs for
/// the whole session, so a program without one is looked up again against a fresh scan of the
/// desktop entries and the icon theme.
pub struct Resolver {
    /// Icon names its hints found, by `App::key`. A program without one has no entry here.
    names: HashMap<String, String>,
    windows: HashMap<String, Looked>,
    ask: Ask,
    landed: (Sender<Landed>, Receiver<Landed>),
    waker: Option<repriseapp::Waker>,
    scan: Option<Scan>,
    apps_dirs: Vec<PathBuf>,
    icon_roots: Vec<PathBuf>,
}

/// The desktop entries and the icon theme as they were at `at`: a theme lists each directory
/// once, so only a new one sees icons installed after it was loaded.
struct Scan {
    entries: Vec<AppEntry>,
    icons: IconCache,
    at: Instant,
}

impl Default for Resolver {
    fn default() -> Self {
        let ask: Ask = Arc::new(
            |pids, title| match reprisebus::tray::window_icon(pids, title) {
                Ok(icon) => icon,
                Err(e) => {
                    log::debug!("sound: no window icon for {title:?}: {e}");
                    None
                }
            },
        );
        Resolver::with_dirs(apps::default_dirs(), IconTheme::default_roots(), ask)
    }
}

impl Resolver {
    fn with_dirs(apps_dirs: Vec<PathBuf>, icon_roots: Vec<PathBuf>, ask: Ask) -> Resolver {
        Resolver {
            names: HashMap::new(),
            windows: HashMap::new(),
            ask,
            landed: mpsc::channel(),
            waker: None,
            scan: None,
            apps_dirs,
            icon_roots,
        }
    }

    /// Wakes the program when a window icon lands, for [`Resolver::take_landed`].
    pub fn set_waker(&mut self, waker: repriseapp::Waker) {
        self.waker = Some(waker);
    }

    /// Takes the window icons the compositor answered with; true when one may change an icon.
    pub fn take_landed(&mut self) -> bool {
        let mut changed = false;
        while let Ok((key, icon)) = self.landed.1.try_recv() {
            changed |= self.land(key, icon);
        }
        changed
    }

    fn land(&mut self, key: String, icon: Option<WindowIcon>) -> bool {
        let found = icon.is_some();
        let looked = icon.map_or(Looked::Missed(Instant::now()), Looked::Landed);
        self.windows.insert(key, looked);
        found
    }

    /// Starts asking for `app`'s window unless an answer is pending or a miss is recent.
    fn ask_window(&mut self, app: &App) {
        let due = match self.windows.get(&app.key) {
            None => true,
            Some(Looked::Missed(at)) => at.elapsed() >= WINDOW_RETRY,
            Some(_) => false,
        };
        if !due || (app.pids.is_empty() && app.label.is_empty()) {
            return;
        }
        self.windows.insert(app.key.clone(), Looked::Asking);
        let (ask, tx, waker) = (self.ask.clone(), self.landed.0.clone(), self.waker.clone());
        let (key, pids, title) = (app.key.clone(), app.pids.clone(), app.label.clone());
        std::thread::spawn(move || {
            let icon = ask(&pids, &title);
            if tx.send((key, icon)).is_ok()
                && let Some(w) = waker
            {
                w.wake();
            }
        });
    }

    /// The window icon found for `key`, turning a landed answer into canvases once.
    fn window_icon(&mut self, key: &str, icons: &mut IconCache) -> Option<AppIcon> {
        let looked = self.windows.get_mut(key)?;
        if let Looked::Landed(icon) = looked {
            let found = window_canvases(icon, icons);
            *looked = match found {
                Some(f) => Looked::Found(f),
                None => Looked::Missed(Instant::now()),
            };
        }
        match looked {
            Looked::Found(icon) => Some(icon.clone()),
            _ => None,
        }
    }

    /// Icons for `list` in the theme and at the scale of `like`.
    pub fn icons(&mut self, list: &[App], like: &IconCache) -> BTreeMap<String, AppIcon> {
        let (theme, scale) = (like.theme_name(), like.scale());
        if self
            .scan
            .as_ref()
            .is_some_and(|s| s.icons.theme_name() != theme || s.icons.scale() != scale)
        {
            self.scan = None;
            self.names.clear();
            self.windows
                .retain(|_, l| matches!(l, Looked::Asking | Looked::Missed(_)));
        }
        let unknown = list.iter().any(|a| {
            !self.names.contains_key(&a.key)
                && !matches!(self.windows.get(&a.key), Some(Looked::Found(_)))
        });
        let old = self
            .scan
            .as_ref()
            .is_none_or(|s| s.at.elapsed() >= RESCAN_AFTER);
        if unknown && old {
            let mut icons = IconCache::new(IconTheme::load(&theme, &self.icon_roots));
            icons.set_scale(scale);
            self.scan = Some(Scan {
                entries: apps::scan(&self.apps_dirs),
                icons,
                at: Instant::now(),
            });
        }
        let Some(mut scan) = self.scan.take() else {
            return BTreeMap::new();
        };
        let mut out = BTreeMap::new();
        for app in list {
            let icon = self.icon(app, &mut scan);
            let generic = |s: &mut Scan, px| s.icons.get(PROGRAM_ICON, px);
            let icon = AppIcon {
                small: icon.small.or_else(|| generic(&mut scan, 16)),
                large: icon.large.or_else(|| generic(&mut scan, 32)),
            };
            out.insert(app.key.clone(), icon);
        }
        self.scan = Some(scan);
        out
    }

    fn icon(&mut self, app: &App, scan: &mut Scan) -> AppIcon {
        let named = |s: &mut Scan, n: &str| AppIcon {
            small: s.icons.get(n, 16),
            large: s.icons.get(n, 32),
        };
        let hinted = match self.names.get(&app.key) {
            Some(n) => Some(n.clone()),
            None => hint_icon(app, &scan.entries, |n| scan.icons.has(n)),
        };
        // An entry may name an icon whose file is not in the theme yet: that is no find.
        if let Some(n) = hinted {
            let icon = named(scan, &n);
            if icon.small.is_some() || icon.large.is_some() {
                self.names.insert(app.key.clone(), n);
                return icon;
            }
        }
        self.names.remove(&app.key);
        self.ask_window(app);
        if let Some(icon) = self.window_icon(&app.key, &mut scan.icons) {
            return icon;
        }
        match guessed_icon(app, &scan.entries).map(str::to_string) {
            Some(n) => named(scan, &n),
            None => AppIcon::default(),
        }
    }
}

/// The icon `app`'s hints, executable or name lead to: [`hint_icon`], then [`guessed_icon`].
pub fn icon_name(app: &App, entries: &[AppEntry], has: impl Fn(&str) -> bool) -> Option<String> {
    hint_icon(app, entries, has).or_else(|| guessed_icon(app, entries).map(str::to_string))
}

/// Canvases for a window icon at the cache's scale; None for a name the theme lacks.
fn window_canvases(icon: &WindowIcon, icons: &mut IconCache) -> Option<AppIcon> {
    let found = match icon {
        WindowIcon::Named(n) => AppIcon {
            small: icons.get(n, 16),
            large: icons.get(n, 32),
        },
        WindowIcon::Pixels {
            width,
            height,
            pixels,
        } => {
            let src = IconSource {
                width: *width,
                height: *height,
                pixels: pixels.clone(),
            };
            let scale = icons.scale();
            let at = |px: i32| Some(src.to_canvas(px * scale).at_scale(scale));
            AppIcon {
                small: at(16),
                large: at(32),
            }
        }
    };
    (found.small.is_some() || found.large.is_some()).then_some(found)
}

/// The icon `app`'s hints name, first match wins: each hint as a desktop entry id or
/// `StartupWMClass`, then as a theme icon.
fn hint_icon(app: &App, entries: &[AppEntry], has: impl Fn(&str) -> bool) -> Option<String> {
    for hint in &app.icon_hints {
        if let Some(icon) = apps::icon_for_app_id(entries, hint) {
            return Some(icon.to_string());
        }
        let lower = hint.to_lowercase();
        for name in [hint.as_str(), lower.as_str()] {
            if has(name) {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// The icon `app`'s name or executable guess at, first match wins: the entry whose `Name` is the
/// program's; then the entry whose `Exec` runs the executable; then the one entry whose `Name`
/// starts with the program's name as a word.
fn guessed_icon<'a>(app: &App, entries: &'a [AppEntry]) -> Option<&'a str> {
    by_name(entries, &app.label)
        .or_else(|| app.binary.as_deref().and_then(|b| by_exec(entries, b)))
        .or_else(|| by_name_prefix(entries, &app.label))
}

fn by_exec<'a>(entries: &'a [AppEntry], binary: &str) -> Option<&'a str> {
    let base = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    let want = base(binary);
    entries
        .iter()
        .filter(|e| e.icon.is_some())
        .find(|e| {
            apps::exec_argv(&e.exec_line)
                .first()
                .is_some_and(|p| base(p) == want)
        })
        .and_then(|e| e.icon.as_deref())
}

/// An Electron program reports the shared `electron` executable but its own product name, which
/// its desktop entry usually shows.
fn by_name<'a>(entries: &'a [AppEntry], name: &str) -> Option<&'a str> {
    entries
        .iter()
        .filter(|e| e.icon.is_some())
        .find(|e| e.name.eq_ignore_ascii_case(name.trim()))
        .and_then(|e| e.icon.as_deref())
}

/// A browser edition reports its family's name ("Brave") and executable, while its entry names the
/// edition ("Brave Origin") and runs a wrapper. Two editions installed side by side match neither.
fn by_name_prefix<'a>(entries: &'a [AppEntry], name: &str) -> Option<&'a str> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let mut found = entries.iter().filter(|e| {
        e.icon.is_some()
            && e.name
                .get(..name.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(name))
            && e.name[name.len()..].starts_with(' ')
    });
    match (found.next(), found.next()) {
        (Some(e), None) => e.icon.as_deref(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn app(hints: &[&str], binary: Option<&str>) -> App {
        App {
            key: "k".into(),
            label: hints.last().copied().unwrap_or("K").into(),
            icon_hints: hints.iter().map(|h| h.to_string()).collect(),
            pids: Vec::new(),
            binary: binary.map(str::to_string),
            streams: Vec::new(),
            volume: 1.0,
            mute: false,
        }
    }

    fn entry(id: &str, exec: &str, icon: &str, class: Option<&str>) -> AppEntry {
        AppEntry {
            name: id.into(),
            exec_line: exec.into(),
            terminal: false,
            id: format!("{id}.desktop"),
            icon: Some(icon.into()),
            startup_wm_class: class.map(str::to_string),
            categories: Vec::new(),
            mime_types: Vec::new(),
            shown: true,
        }
    }

    fn entries() -> Vec<AppEntry> {
        vec![
            entry(
                "org.mozilla.firefox",
                "firefox %u",
                "firefox",
                Some("firefox"),
            ),
            entry(
                "google-chrome",
                "/usr/bin/google-chrome-stable %U",
                "google-chrome",
                None,
            ),
        ]
    }

    fn name(app: &App, entries: &[AppEntry], has: impl Fn(&str) -> bool) -> String {
        icon_name(app, entries, has).unwrap_or_else(|| PROGRAM_ICON.into())
    }

    #[test]
    fn a_desktop_entry_names_the_icon() {
        let none = |_: &str| false;
        assert_eq!(
            name(&app(&["firefox"], None), &entries(), none),
            "firefox",
            "by StartupWMClass"
        );
        assert_eq!(
            name(
                &app(&["Chrome"], Some("google-chrome-stable")),
                &entries(),
                none
            ),
            "google-chrome",
            "by the executable Exec runs"
        );
    }

    /// The properties Chromium-based browsers and Firefox give their playback streams: the
    /// icon hint is the desktop entry's id, the executable the one behind the launcher script.
    #[test]
    fn browsers_find_their_desktop_entries() {
        let installed = vec![
            entry(
                "brave-browser",
                "brave %U",
                "brave-desktop",
                Some("brave-browser"),
            ),
            entry(
                "chromium",
                "/usr/bin/chromium %U",
                "chromium",
                Some("chromium"),
            ),
            entry("firefox", "/usr/lib/firefox/firefox %u", "firefox", None),
            entry(
                "com.brave.Browser",
                "/usr/bin/flatpak run --branch=stable --command=brave com.brave.Browser @@u %U @@",
                "com.brave.Browser",
                None,
            ),
        ];
        let none = |_: &str| false;
        for (hints, binary, want) in [
            (
                &["brave-browser", "brave", "Brave"][..],
                "brave",
                "brave-desktop",
            ),
            (
                &["chromium-browser", "chromium", "Chromium"],
                "chromium",
                "chromium",
            ),
            (&["firefox", "Firefox"], "firefox", "firefox"),
            (
                &["com.brave.Browser", "brave", "Brave"],
                "brave",
                "com.brave.Browser",
            ),
        ] {
            assert_eq!(name(&app(hints, Some(binary)), &installed, none), want);
        }
    }

    /// The stream Brave Origin plays: Brave's icon name, executable and name, none of which its
    /// entry carries.
    #[test]
    fn a_browser_edition_is_found_by_its_family_name() {
        let mut origin = entry(
            "brave-origin",
            "brave-origin %U",
            "brave-origin",
            Some("brave-origin"),
        );
        origin.name = "Brave Origin".into();
        let mut installed = vec![origin, entries()[0].clone()];
        let brave = app(&["brave-browser", "brave", "Brave"], Some("brave"));
        assert_eq!(name(&brave, &installed, |_| false), "brave-origin");

        let mut nightly = entry("brave-nightly", "brave-nightly %U", "brave-nightly", None);
        nightly.name = "Brave Nightly".into();
        installed.push(nightly);
        assert_eq!(
            icon_name(&brave, &installed, |_| false),
            None,
            "two editions leave the choice open"
        );
        let mut braver = installed[0].clone();
        braver.name = "Braveheart".into();
        assert_eq!(
            icon_name(&brave, &[braver], |_| false),
            None,
            "the name ends at a word"
        );
    }

    #[test]
    fn an_electron_program_is_found_by_its_product_name() {
        let mut demo = entry("org.example.Demo", "demo %U", "org.example.Demo", None);
        demo.name = "Demo App".into();
        let installed = [
            entry("demo-cli", "demo-cli", "utilities-terminal", None),
            demo,
        ];
        assert_eq!(
            name(
                &app(&["demo-app", "electron", "Demo App"], Some("electron")),
                &installed,
                |_| false
            ),
            "org.example.Demo"
        );
    }

    #[test]
    fn a_theme_name_serves_when_no_entry_matches() {
        let has = |n: &str| n == "vlc";
        assert_eq!(name(&app(&["VLC"], Some("vlc")), &[], has), "vlc");
    }

    #[test]
    fn an_earlier_hint_wins() {
        let has = |n: &str| n == "spotify-client" || n == "spotify";
        assert_eq!(
            name(&app(&["spotify-client", "spotify"], None), &[], has),
            "spotify-client"
        );
    }

    #[test]
    fn nothing_matching_is_a_plain_program() {
        assert_eq!(
            icon_name(&app(&["mystery"], Some("mystery")), &entries(), |_| false),
            None
        );
    }

    struct Dirs(PathBuf);

    impl Drop for Dirs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A scalable "hicolor" theme with no icons yet, and an empty applications directory.
    fn dirs(tag: &str) -> Dirs {
        let base = std::env::temp_dir().join(format!("reprise-sound-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("applications")).unwrap();
        std::fs::create_dir_all(base.join("icons/hicolor")).unwrap();
        std::fs::write(
            base.join("icons/hicolor/index.theme"),
            "[Icon Theme]\nName=Hicolor\nDirectories=scalable/apps\n\n[scalable/apps]\nSize=32\nMinSize=8\nMaxSize=256\nType=Scalable\n",
        )
        .unwrap();
        Dirs(base)
    }

    fn install(d: &Dirs, id: &str) {
        let apps = d.0.join("icons/hicolor/scalable/apps");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(
            apps.join(format!("{id}.svg")),
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="red"/></svg>"#,
        )
        .unwrap();
        std::fs::write(
            d.0.join(format!("applications/{id}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName=Late\nExec={id}\nIcon={id}\n"),
        )
        .unwrap();
    }

    fn resolver(d: &Dirs, ask: Ask) -> Resolver {
        Resolver::with_dirs(vec![d.0.join("applications")], vec![d.0.join("icons")], ask)
    }

    /// Waits for the window lookup a call to `icons` started and takes its answer.
    fn land(r: &mut Resolver) {
        let (key, icon) = r.landed.1.recv_timeout(Duration::from_secs(5)).unwrap();
        r.land(key, icon);
    }

    fn centre(c: &Option<Canvas>) -> Option<u32> {
        let c = c.as_ref()?;
        Some(c.pixels[(c.height / 2 * c.width + c.width / 2) as usize])
    }

    const RED: u32 = 0xffff_0000;
    const BLUE: u32 = 0xff00_00ff;

    fn counting(calls: &Arc<AtomicUsize>, icon: Option<WindowIcon>) -> Ask {
        let calls = calls.clone();
        Arc::new(move |pids, title| {
            assert_eq!((pids, title), (&[4312][..], "Late"));
            calls.fetch_add(1, Ordering::SeqCst);
            icon.clone()
        })
    }

    #[test]
    fn the_window_icon_comes_after_the_hints_and_before_the_guesses() {
        let d = dirs("order");
        install(&d, "late-app");
        let like = IconCache::new(IconTheme::load("hicolor", &[]));
        let calls = Arc::new(AtomicUsize::new(0));
        let blue = WindowIcon::Pixels {
            width: 1,
            height: 1,
            pixels: vec![BLUE],
        };
        let mut r = resolver(&d, counting(&calls, Some(blue)));
        let mut hinted = app(&["late-app"], None);
        hinted.pids = vec![4312];
        let list = [hinted];
        assert_eq!(centre(&r.icons(&list, &like)["k"].small), Some(RED));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "a hint that resolves asks for no window"
        );

        let mut guessed = app(&["Late"], None);
        guessed.key = "g".into();
        guessed.pids = vec![4312];
        let list = [guessed];
        assert_eq!(
            centre(&r.icons(&list, &like)["g"].small),
            Some(RED),
            "the entry named Late stands in while the window is asked for"
        );
        land(&mut r);
        let icon = &r.icons(&list, &like)["g"];
        assert_eq!(centre(&icon.small), Some(BLUE));
        assert_eq!(
            icon.large.as_ref().map(|c| (c.width, c.height)),
            Some((32, 32))
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_window_not_found_is_asked_for_again_after_window_retry() {
        let d = dirs("retry");
        let like = IconCache::new(IconTheme::load("hicolor", &[]));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut r = resolver(&d, counting(&calls, None));
        let mut late = app(&["Late"], None);
        late.pids = vec![4312];
        let list = [late];
        r.icons(&list, &like);
        land(&mut r);
        r.icons(&list, &like);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "a recent miss stands");
        if let Some(Looked::Missed(at)) = r.windows.get_mut("k") {
            *at -= WINDOW_RETRY;
        }
        r.icons(&list, &like);
        land(&mut r);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_program_installed_after_the_first_scan_finds_its_icon() {
        let d = dirs("late");
        let mut r = resolver(&d, Arc::new(|_, _| None));
        let like = IconCache::new(IconTheme::load("hicolor", &[]));
        let mut late = app(&["late-app"], Some("late-app"));
        late.key = "late-app".into();
        let list = [late];
        assert!(r.icons(&list, &like)["late-app"].small.is_none());
        install(&d, "late-app");
        assert!(
            r.icons(&list, &like)["late-app"].small.is_none(),
            "the scan stands for RESCAN_AFTER"
        );
        if let Some(s) = &mut r.scan {
            s.at -= RESCAN_AFTER;
        }
        let icon = &r.icons(&list, &like)["late-app"];
        assert_eq!(
            icon.small.as_ref().map(|c| (c.width, c.height)),
            Some((16, 16))
        );
        assert!(icon.large.is_some());
    }
}
