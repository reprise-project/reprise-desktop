//! App ids: the name a program goes by, apart from the desktop file it may have
//! ([`DesktopId`]). Telegram goes by "org.telegram.desktop"; its file is
//! "org.telegram.desktop.desktop".

use super::{AppEntry, DesktopId};
use std::borrow::Cow;

/// NAME_MAX: an app id names a file and a scope unit, neither longer.
const MAX_LEN: usize = 255;

/// Not empty, at most [`MAX_LEN`] bytes of UTF-8, no C0, DEL or C1 control character.
const fn valid(id: &[u8]) -> bool {
    if id.is_empty() || id.len() > MAX_LEN {
        return false;
    }
    let mut i = 0;
    while i < id.len() {
        let c1 = id[i] == 0xc2 && i + 1 < id.len() && id[i + 1] >= 0x80 && id[i + 1] <= 0x9f;
        if id[i] < 0x20 || id[i] == 0x7f || c1 {
            return false;
        }
        i += 1;
    }
    true
}

/// The name a program goes by: a Wayland app id, an X11 class, a Flatpak id, the `<id>` of an
/// `app-reprise-<id>-*.scope`, a notification's `desktop-entry` hint. Kept as stated, with no
/// `.desktop` taken off. Equality is exact; [`AppId::is_like`] and [`entry_for_app_id`] match a
/// desktop file case-insensitively.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AppId(Cow<'static, str>);

impl AppId {
    /// `id` when it can name a program: not empty, at most 255 bytes, no control character.
    pub fn new(id: &str) -> Option<AppId> {
        valid(id.as_bytes()).then(|| AppId(Cow::Owned(id.to_string())))
    }

    /// [`AppId::new`] for a literal; an invalid one fails to compile in a `const`.
    pub const fn from_static(id: &'static str) -> AppId {
        assert!(valid(id.as_bytes()), "not an app id");
        AppId(Cow::Borrowed(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The files this program may have, best first: `<id>.desktop`, then the id itself when it
    /// ends in `.desktop` (a program naming its file).
    pub fn desktop_ids(&self) -> Vec<DesktopId> {
        (DesktopId::from_file_name(&format!("{}.desktop", self.0)).into_iter())
            .chain(DesktopId::from_file_name(&self.0))
            .collect()
    }

    /// Whether `other` names the same program, ignoring ASCII case: a window's app id or X11
    /// class often differs in case from its file's name.
    pub fn is_like(&self, other: &str) -> bool {
        self.0.eq_ignore_ascii_case(other)
    }
}

impl std::fmt::Display for AppId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<str> for AppId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for AppId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// The entry of the program going by `app_id`: a file [`AppId::desktop_ids`] names exactly,
/// else one whose [`DesktopId::app_id`] or `StartupWMClass` is [`AppId::is_like`] it. Only
/// entries `want` takes count.
pub fn entry_for_app_id<'a>(
    entries: &'a [AppEntry],
    app_id: &AppId,
    want: impl Fn(&AppEntry) -> bool,
) -> Option<&'a AppEntry> {
    let take = |f: &dyn Fn(&AppEntry) -> bool| entries.iter().find(|e| want(e) && f(e));
    (app_id.desktop_ids().iter())
        .find_map(|id| take(&|e| e.id == *id))
        .or_else(|| take(&|e| app_id.is_like(e.id.stem())))
        .or_else(|| {
            take(&|e| {
                e.startup_wm_class
                    .as_deref()
                    .is_some_and(|c| app_id.is_like(c))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> AppId {
        AppId::new(s).unwrap()
    }

    fn entry(file: &str, class: Option<&str>) -> AppEntry {
        AppEntry {
            plain_name: None,
            name: file.into(),
            exec_line: "x".into(),
            terminal: false,
            id: file.parse().unwrap(),
            icon: None,
            startup_wm_class: class.map(Into::into),
            categories: Vec::new(),
            mime_types: Vec::new(),
            shown: true,
            path: None,
            actions: Vec::new(),
        }
    }

    #[test]
    fn an_app_id_is_kept_as_stated() {
        assert_eq!(id("org.telegram.desktop").as_str(), "org.telegram.desktop");
        assert_eq!(id("Firefox").as_str(), "Firefox");
        for bad in ["", "a\nb", "\u{7f}", "a\u{85}"] {
            assert_eq!(AppId::new(bad), None, "{bad:?}");
        }
        assert!(AppId::new(&"a".repeat(255)).is_some());
        assert_eq!(AppId::new(&"a".repeat(256)), None, "256 bytes");
        assert_eq!(
            AppId::from_static("org.kde.kdeconnect.app"),
            id("org.kde.kdeconnect.app")
        );
        assert!(id("Firefox").is_like("firefox"));
        assert_ne!(id("Firefox"), id("firefox"), "equality is exact");
    }

    #[test]
    fn an_app_id_ending_in_desktop_names_its_own_file_second() {
        let ids = |s: &str| -> Vec<String> {
            id(s)
                .desktop_ids()
                .iter()
                .map(ToString::to_string)
                .collect()
        };
        assert_eq!(
            ids("org.telegram.desktop"),
            ["org.telegram.desktop.desktop", "org.telegram.desktop"]
        );
        assert_eq!(ids("firefox"), ["firefox.desktop"]);
        for none in ["../etc/passwd", ".desktop", ".x"] {
            assert!(ids(none).is_empty(), "{none:?}");
        }
    }

    #[test]
    fn an_exact_file_wins_over_case_and_class_matches() {
        let entries = [
            entry("Calc.desktop", None),
            entry("calc.desktop", None),
            entry("x.desktop", None),
            entry("code.desktop", Some("Code")),
        ];
        let found = |s: &str| entry_for_app_id(&entries, &id(s), |_| true).map(|e| e.id.as_str());
        assert_eq!(found("calc"), Some("calc.desktop"));
        assert_eq!(
            found("CALC"),
            Some("Calc.desktop"),
            "first case-insensitive stem"
        );
        assert_eq!(
            found("x.desktop"),
            Some("x.desktop"),
            "a program naming its file"
        );
        assert_eq!(found("CODE"), Some("code.desktop"));
        assert_eq!(found("nothing"), None);
        let none = entry_for_app_id(&entries, &id("calc"), |e| e.id != "calc.desktop");
        assert_eq!(
            none.map(|e| e.id.as_str()),
            Some("Calc.desktop"),
            "want filters first"
        );
    }
}
