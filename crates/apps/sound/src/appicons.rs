use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use reprisefs::apps::{self, AppEntry};
use reprisekit::controls::openwith::PROGRAM_ICON;
use reprisekit::icontheme::IconCache;
use reprisekit::raster::Canvas;

use crate::pipewire::App;

/// A program installed a minute ago should still find its icon.
const RESCAN_AFTER: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default)]
pub struct AppIcon {
    pub small: Option<Canvas>,
    pub large: Option<Canvas>,
}

#[derive(Default)]
pub struct Resolver {
    names: HashMap<String, String>,
    entries: Option<(Vec<AppEntry>, Instant)>,
}

impl Resolver {
    pub fn icons(&mut self, list: &[App], icons: &mut IconCache) -> BTreeMap<String, AppIcon> {
        let mut out = BTreeMap::new();
        for app in list {
            let name = match self.names.get(&app.key) {
                Some(n) => n.clone(),
                None => {
                    let entries = self.entries();
                    let n = icon_name(app, entries, |n| icons.has(n));
                    self.names.insert(app.key.clone(), n.clone());
                    n
                }
            };
            let icon = AppIcon {
                small: icons.get(&name, 16).or_else(|| icons.get(PROGRAM_ICON, 16)),
                large: icons.get(&name, 32).or_else(|| icons.get(PROGRAM_ICON, 32)),
            };
            out.insert(app.key.clone(), icon);
        }
        out
    }

    fn entries(&mut self) -> &[AppEntry] {
        let fresh = self
            .entries
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() < RESCAN_AFTER);
        if !fresh {
            self.entries = Some((apps::scan(&apps::default_dirs()), Instant::now()));
        }
        self.entries.as_ref().map_or(&[], |(e, _)| e.as_slice())
    }
}

pub fn icon_name(app: &App, entries: &[AppEntry], has: impl Fn(&str) -> bool) -> String {
    for hint in &app.icon_hints {
        if let Some(icon) = apps::icon_for_app_id(entries, hint) {
            return icon.to_string();
        }
        let lower = hint.to_lowercase();
        for name in [hint.as_str(), lower.as_str()] {
            if has(name) {
                return name.to_string();
            }
        }
    }
    app.binary
        .as_deref()
        .and_then(|b| by_exec(entries, b))
        .unwrap_or(PROGRAM_ICON)
        .to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn app(hints: &[&str], binary: Option<&str>) -> App {
        App {
            key: "k".into(),
            label: "K".into(),
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

    #[test]
    fn a_desktop_entry_names_the_icon() {
        let none = |_: &str| false;
        assert_eq!(
            icon_name(&app(&["firefox"], None), &entries(), none),
            "firefox",
            "by StartupWMClass"
        );
        assert_eq!(
            icon_name(
                &app(&["Chrome"], Some("google-chrome-stable")),
                &entries(),
                none
            ),
            "google-chrome",
            "by the executable Exec runs"
        );
    }

    #[test]
    fn a_theme_name_serves_when_no_entry_matches() {
        let has = |n: &str| n == "vlc";
        assert_eq!(icon_name(&app(&["VLC"], Some("vlc")), &[], has), "vlc");
    }

    #[test]
    fn an_earlier_hint_wins() {
        let has = |n: &str| n == "spotify-client" || n == "spotify";
        assert_eq!(
            icon_name(&app(&["spotify-client", "spotify"], None), &[], has),
            "spotify-client"
        );
    }

    #[test]
    fn nothing_matching_is_a_plain_program() {
        assert_eq!(
            icon_name(&app(&["mystery"], Some("mystery")), &entries(), |_| false),
            PROGRAM_ICON
        );
    }
}
