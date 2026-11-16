//! Desktop file ids, kept apart from the app ids programs state: Telegram's app id is
//! "org.telegram.desktop" and its desktop id "org.telegram.desktop.desktop".

const SUFFIX: &str = ".desktop";
/// NAME_MAX: no file name, so no desktop id, is longer.
const MAX_LEN: usize = 255;

/// A file name ending in `.desktop`, as [`super::AppEntry::id`] holds it; its program states the
/// [`stem`](Self::stem) as app id. Compared case-sensitively, like file names.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DesktopId(String);

impl DesktopId {
    /// `name` when a desktop file can have it: a stem before `.desktop`, at most 255 bytes, no
    /// `/` and no leading `.`.
    pub fn from_file_name(name: &str) -> Option<DesktopId> {
        let ok = name.len() <= MAX_LEN
            && !name.contains('/')
            && !name.starts_with('.')
            && name.strip_suffix(SUFFIX).is_some_and(|s| !s.is_empty());
        ok.then(|| DesktopId(name.to_string()))
    }

    /// The file `name` names the way `gtk-launch` and the portals read it: `name` itself when it
    /// ends in `.desktop`, else `name` with `.desktop` added.
    pub fn named(name: &str) -> Option<DesktopId> {
        Self::from_file_name(name).or_else(|| Self::from_file_name(&format!("{name}{SUFFIX}")))
    }

    /// The files a program stating `app_id` (or sending it as `desktop-entry`) may have, best
    /// first: `<app_id>.desktop`, then `app_id` itself when it ends in `.desktop`.
    pub fn for_app_id(app_id: &str) -> Vec<DesktopId> {
        if app_id.starts_with('.') || app_id.contains('/') {
            return Vec::new();
        }
        (Self::from_file_name(&format!("{app_id}{SUFFIX}")).into_iter())
            .chain(Self::from_file_name(app_id))
            .collect()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The app id this file's program states: the id without its `.desktop`.
    pub fn stem(&self) -> &str {
        &self.0[..self.0.len() - SUFFIX.len()]
    }

    /// `name` without one `.desktop` when it is a desktop id, else `name` as it is: for a string
    /// that holds either a desktop id or a program's own name.
    pub fn stem_of(name: &str) -> &str {
        match Self::from_file_name(name) {
            Some(_) => &name[..name.len() - SUFFIX.len()],
            None => name,
        }
    }

    /// Whether a program stating `app_id` is this file's: `app_id` is exactly the stem.
    pub fn matches_app_id(&self, app_id: &str) -> bool {
        self.stem() == app_id
    }
}

impl std::fmt::Display for DesktopId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::DesktopId;

    fn ids(v: Vec<DesktopId>) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_file_name_is_a_desktop_id_only_with_a_stem_and_the_suffix() {
        assert_eq!(
            DesktopId::from_file_name("firefox.desktop").map(|d| d.stem().to_string()),
            Some("firefox".into())
        );
        for bad in ["firefox", ".desktop", ".hidden.desktop", "a/b.desktop", ""] {
            assert_eq!(DesktopId::from_file_name(bad), None, "{bad:?}");
        }
        let long = format!("{}.desktop", "a".repeat(248));
        assert_eq!(DesktopId::from_file_name(&long), None, "256 bytes");
    }

    #[test]
    fn an_app_id_ending_in_desktop_names_its_own_file_first() {
        assert_eq!(
            ids(DesktopId::for_app_id("org.telegram.desktop")),
            ["org.telegram.desktop.desktop", "org.telegram.desktop"]
        );
        assert_eq!(ids(DesktopId::for_app_id("firefox")), ["firefox.desktop"]);
        for bad in ["", "../etc/passwd", ".desktop", ".x"] {
            assert!(DesktopId::for_app_id(bad).is_empty(), "{bad:?}");
        }
    }

    #[test]
    fn the_stem_drops_one_suffix_only() {
        let id = DesktopId::from_file_name("x.desktop.desktop").unwrap();
        assert_eq!(id.stem(), "x.desktop");
        assert!(id.matches_app_id("x.desktop"));
        assert!(!id.matches_app_id("x"));
        assert!(!id.matches_app_id("X.desktop"), "case-sensitive");
        assert_eq!(DesktopId::stem_of("x.desktop.desktop"), "x.desktop");
        assert_eq!(DesktopId::stem_of("foot"), "foot");
        assert_eq!(DesktopId::stem_of(".desktop"), ".desktop");
    }

    #[test]
    fn a_launch_name_gets_the_suffix_once() {
        let named = |n| DesktopId::named(n).map(|d| d.to_string());
        assert_eq!(named("vim").as_deref(), Some("vim.desktop"));
        assert_eq!(named("vim.desktop").as_deref(), Some("vim.desktop"));
        assert_eq!(
            named("org.telegram.desktop").as_deref(),
            Some("org.telegram.desktop")
        );
        assert_eq!(named("a/b"), None);
    }
}
