//! Desktop file ids, kept apart from the app ids programs state: Telegram's app id is
//! "org.telegram.desktop" and its desktop id "org.telegram.desktop.desktop".

use super::AppId;

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

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The id without its `.desktop`, which a program with this file goes by.
    pub fn stem(&self) -> &str {
        &self.0[..self.0.len() - SUFFIX.len()]
    }

    /// The [`stem`](Self::stem) as the app id it is.
    pub fn app_id(&self) -> AppId {
        AppId::new(self.stem()).expect("a stem is a non-empty file name")
    }

    /// Whether this is the file of the program going by `app_id`: `app_id` is exactly the stem.
    pub fn matches_app_id(&self, app_id: &AppId) -> bool {
        self.stem() == app_id.as_str()
    }
}

impl std::fmt::Display for DesktopId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A name [`DesktopId::from_file_name`] refuses.
#[derive(Debug, PartialEq, Eq)]
pub struct NotDesktopId;

impl std::fmt::Display for NotDesktopId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not a desktop file name")
    }
}

impl std::error::Error for NotDesktopId {}

impl std::str::FromStr for DesktopId {
    type Err = NotDesktopId;

    fn from_str(name: &str) -> Result<DesktopId, NotDesktopId> {
        DesktopId::from_file_name(name).ok_or(NotDesktopId)
    }
}

impl PartialEq<str> for DesktopId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<String> for DesktopId {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}

impl PartialEq<&str> for DesktopId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

#[cfg(test)]
mod tests {
    use super::{AppId, DesktopId};

    #[test]
    fn a_file_name_is_a_desktop_id_only_with_a_stem_and_the_suffix() {
        assert_eq!(
            DesktopId::from_file_name("firefox.desktop").map(|d| d.stem().to_string()),
            Some("firefox".into())
        );
        for bad in ["firefox", ".desktop", ".hidden.desktop", "a/b.desktop", ""] {
            assert_eq!(DesktopId::from_file_name(bad), None, "{bad:?}");
            assert!(bad.parse::<DesktopId>().is_err(), "{bad:?}");
        }
        let long = format!("{}.desktop", "a".repeat(248));
        assert_eq!(DesktopId::from_file_name(&long), None, "256 bytes");
    }

    #[test]
    fn the_stem_drops_one_suffix_only() {
        let id = DesktopId::from_file_name("x.desktop.desktop").unwrap();
        assert_eq!(id.stem(), "x.desktop");
        assert_eq!(id.app_id(), AppId::new("x.desktop").unwrap());
        assert!(id.matches_app_id(&AppId::new("x.desktop").unwrap()));
        assert!(!id.matches_app_id(&AppId::new("x").unwrap()));
        let other_case = AppId::new("X.desktop").unwrap();
        assert!(!id.matches_app_id(&other_case), "case-sensitive");
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
