use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use reprisefs::apps::{self, AppEntry};
use reprisekit::controls::openwith::PROGRAM_ICON;
use reprisekit::icontheme::{IconCache, IconTheme};
use reprisekit::raster::Canvas;

use crate::pipewire::App;

/// A program installed a minute ago should still find its icon.
const RESCAN_AFTER: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default)]
pub struct AppIcon {
    pub small: Option<Canvas>,
    pub large: Option<Canvas>,
}

/// Finds each playing program's icon. The tray runs for the whole session, so a program without
/// one is looked up again against a fresh scan of the desktop entries and the icon theme.
pub struct Resolver {
    /// Icon names found, by `App::key`. A program without one has no entry here.
    names: HashMap<String, String>,
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
        Resolver::with_dirs(apps::default_dirs(), IconTheme::default_roots())
    }
}

impl Resolver {
    fn with_dirs(apps_dirs: Vec<PathBuf>, icon_roots: Vec<PathBuf>) -> Resolver {
        Resolver {
            names: HashMap::new(),
            scan: None,
            apps_dirs,
            icon_roots,
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
        }
        let unknown = list.iter().any(|a| !self.names.contains_key(&a.key));
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
        let Some(scan) = self.scan.as_mut() else {
            return BTreeMap::new();
        };
        let mut out = BTreeMap::new();
        for app in list {
            let name = match self.names.get(&app.key) {
                Some(n) => Some(n.clone()),
                None => icon_name(app, &scan.entries, |n| scan.icons.has(n)),
            };
            let small = name.as_deref().and_then(|n| scan.icons.get(n, 16));
            let large = name.as_deref().and_then(|n| scan.icons.get(n, 32));
            // An entry may name an icon whose file is not in the theme yet: that is no find.
            match name {
                Some(n) if small.is_some() || large.is_some() => {
                    self.names.insert(app.key.clone(), n);
                }
                _ => {
                    self.names.remove(&app.key);
                }
            }
            let icon = AppIcon {
                small: small.or_else(|| scan.icons.get(PROGRAM_ICON, 16)),
                large: large.or_else(|| scan.icons.get(PROGRAM_ICON, 32)),
            };
            out.insert(app.key.clone(), icon);
        }
        out
    }
}

/// The icon `app`'s hints, executable or name lead to, first match wins: each hint as a desktop
/// entry id or `StartupWMClass`, then as a theme icon; then the entry whose `Name` is the
/// program's; then the entry whose `Exec` runs the executable.
pub fn icon_name(app: &App, entries: &[AppEntry], has: impl Fn(&str) -> bool) -> Option<String> {
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
    by_name(entries, &app.label)
        .or_else(|| app.binary.as_deref().and_then(|b| by_exec(entries, b)))
        .map(str::to_string)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn app(hints: &[&str], binary: Option<&str>) -> App {
        App {
            key: "k".into(),
            label: hints.last().copied().unwrap_or("K").into(),
            icon_hints: hints.iter().map(|h| h.to_string()).collect(),
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

    #[test]
    fn a_program_installed_after_the_first_scan_finds_its_icon() {
        let d = dirs("late");
        let mut r = Resolver::with_dirs(vec![d.0.join("applications")], vec![d.0.join("icons")]);
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
