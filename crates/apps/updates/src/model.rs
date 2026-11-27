//! What reprise-updates knows and says, with no bus: the updates a check
//! found (packages, Flatpak apps and runtimes, firmware), the app's phase,
//! the lines the window and tray show, and when a balloon is worth sending.

use std::collections::BTreeSet;

use reprisekit::tr;

use crate::restart::Due;

/// How much an update matters, as PackageKit rates it (firmware maps its
/// urgency onto it, Flatpak is `Normal`). Most to least urgent, which is the
/// order inside a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Security,
    Important,
    Bugfix,
    Normal,
    Enhancement,
    Low,
}

impl Kind {
    pub fn label(self) -> String {
        match self {
            Kind::Security => tr!("update-kind-security"),
            Kind::Important => tr!("update-kind-important"),
            Kind::Bugfix => tr!("update-kind-bugfix"),
            Kind::Normal => tr!("update-kind-normal"),
            Kind::Enhancement => tr!("update-kind-enhancement"),
            Kind::Low => tr!("update-kind-low"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Install {
    System,
    User,
}

impl Install {
    pub fn label(self) -> String {
        match self {
            Install::System => tr!("update-install-system"),
            Install::User => tr!("update-install-user"),
        }
    }
}

/// Where an update comes from, in list order: packages, Flatpak apps, runtimes, firmware.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    Package,
    App(Install),
    Runtime(Install),
    Firmware,
}

impl Origin {
    pub fn group(self) -> Group {
        match self {
            Origin::Package => Group::Packages,
            Origin::App(_) | Origin::Runtime(_) => Group::Flatpak,
            Origin::Firmware => Group::Firmware,
        }
    }
}

/// A source of updates: a list heading. A source that is not installed sends nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Packages,
    Flatpak,
    Firmware,
}

impl Group {
    pub fn title(self) -> String {
        match self {
            Group::Packages => tr!("update-group-packages"),
            Group::Flatpak => tr!("update-group-flatpak"),
            Group::Firmware => tr!("update-group-firmware"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    /// The PackageKit package id (`name;version;arch;data`), the Flatpak ref
    /// (`app/org.example.App/x86_64/stable`), or the fwupd device id.
    pub id: String,
    pub name: String,
    pub current: String,
    pub version: String,
    /// The line under the list: package summary, Flatpak origin, or firmware notes.
    pub summary: String,
    pub kind: Kind,
    pub origin: Origin,
}

impl Update {
    /// Unique across sources: a runtime can be in both Flatpak installations under one ref.
    pub fn key(&self) -> String {
        format!("{:?}:{}", self.origin, self.id)
    }

    /// "1.2.2 → 1.2.4" when the installed version is known and differs, else the new one.
    pub fn version_cell(&self) -> String {
        if self.current.is_empty() || self.current == self.version {
            self.version.clone()
        } else {
            format!("{} → {}", self.current, self.version)
        }
    }

    /// Urgency for a package or firmware; app or runtime and installation for a Flatpak ref.
    pub fn type_cell(&self) -> String {
        match self.origin {
            Origin::Package | Origin::Firmware => self.kind.label(),
            Origin::App(i) => tr!("update-origin-app", install = i.label()),
            Origin::Runtime(i) => tr!("update-origin-runtime", install = i.label()),
        }
    }

    pub fn about(&self) -> String {
        let what = match self.origin {
            Origin::Package => self.kind.label(),
            _ => self.name.clone(),
        };
        if self.summary.is_empty() {
            what
        } else {
            format!("{what}: {}", self.summary)
        }
    }
}

pub fn sort(updates: &mut [Update]) {
    updates.sort_by_cached_key(|u| (u.origin.group(), u.kind, u.origin, u.name.to_lowercase()));
}

pub type Found = Vec<(Group, Result<Vec<Update>, String>)>;

/// A source that answered replaces its updates, one that failed keeps them,
/// and one that sent nothing (no longer installed) loses them. Returns the first failure too.
pub fn merge(old: &[Update], found: Found) -> (Vec<Update>, Option<String>) {
    let mut out = Vec::new();
    let mut failed = None;
    for (group, result) in found {
        match result {
            Ok(u) => out.extend(u),
            Err(why) => {
                out.extend(old.iter().filter(|u| u.origin.group() == group).cloned());
                failed.get_or_insert(why);
            }
        }
    }
    sort(&mut out);
    (out, failed)
}

/// A malformed id is its own name with no version.
pub fn split_package_id(id: &str) -> (&str, &str) {
    let mut parts = id.split(';');
    let name = parts.next().unwrap_or(id);
    let version = parts.next().unwrap_or("");
    (name, version)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Restart {
    #[default]
    None,
    Session,
    System,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Unchecked,
    Checking,
    Ready,
    /// An install is running: progress 0 to 1 (None while unknown), the current step, and what a
    /// device being updated asks the user to do before it carries on.
    Installing {
        fraction: Option<f64>,
        doing: String,
        request: Option<String>,
    },
    Installed(Restart),
    /// The system was updated outside an install here and needs a restart (see `restart`).
    RestartDue(crate::restart::Due),
    Failed(String),
}

impl Phase {
    pub fn busy(&self) -> bool {
        matches!(self, Phase::Checking | Phase::Installing { .. })
    }
}

pub fn count(n: usize) -> String {
    tr!("update-count", count = n)
}

/// `picked` is how many of `updates` are checked for install.
pub fn headline(phase: &Phase, updates: &[Update], picked: usize) -> String {
    match phase {
        Phase::Unchecked => tr!("headline-unchecked"),
        Phase::Checking => tr!("headline-checking"),
        Phase::Ready if updates.is_empty() => tr!("headline-up-to-date"),
        Phase::Ready if picked == 0 => {
            tr!("headline-none-selected", updates = count(updates.len()))
        }
        Phase::Ready if picked < updates.len() => tr!(
            "headline-some-selected",
            picked = picked,
            updates = count(updates.len())
        ),
        Phase::Ready => tr!(
            "headline-available",
            updates = count(updates.len()),
            count = updates.len()
        ),
        Phase::Installing {
            request: Some(r), ..
        } => r.clone(),
        Phase::Installing { .. } => tr!("headline-installing"),
        Phase::Installed(Restart::System) | Phase::RestartDue(_) if picked > 0 => {
            tr!(
                "headline-restart-picked",
                updates = count(picked),
                picked = picked
            )
        }
        Phase::Installed(Restart::System) => tr!("headline-installed-restart"),
        Phase::Installed(Restart::Session) => tr!("headline-installed-logoff"),
        Phase::Installed(Restart::None) => tr!("headline-installed"),
        Phase::RestartDue(Due::System) => tr!("headline-due-system"),
        Phase::RestartDue(Due::Reprise) => tr!("headline-due-reprise"),
        Phase::Failed(message) => message.clone(),
    }
}

pub fn progress_line(phase: &Phase) -> String {
    match phase {
        Phase::Installing {
            fraction, doing, ..
        } => match fraction {
            Some(f) => tr!(
                "progress-line",
                doing = doing.as_str(),
                percent = ((f * 100.0).round() as i32).to_string()
            ),
            None => doing.clone(),
        },
        _ => String::new(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayLook {
    pub icon: &'static str,
    pub tip: String,
    pub active: bool,
}

pub fn tray_look(phase: &Phase, updates: &[Update]) -> TrayLook {
    let urgent = updates.iter().any(|u| u.kind == Kind::Security);
    let available = if urgent {
        "software-update-urgent"
    } else {
        "software-update-available"
    };
    match phase {
        Phase::Installing { fraction, .. } => TrayLook {
            icon: "software-update-available",
            tip: match fraction {
                Some(f) => tr!(
                    "tip-installing-percent",
                    percent = ((f * 100.0).round() as i32).to_string()
                ),
                None => tr!("tip-installing"),
            },
            active: true,
        },
        Phase::Installed(Restart::System) => TrayLook {
            icon: "system-reboot",
            tip: tr!("tip-restart"),
            active: true,
        },
        Phase::RestartDue(Due::System) => TrayLook {
            icon: "system-reboot",
            tip: tr!("tip-due-system"),
            active: true,
        },
        Phase::RestartDue(Due::Reprise) => TrayLook {
            icon: "system-reboot",
            tip: tr!("tip-due-reprise"),
            active: true,
        },
        Phase::Installed(Restart::Session) => TrayLook {
            icon: "system-log-out",
            tip: tr!("tip-logoff"),
            active: true,
        },
        _ if updates.is_empty() => TrayLook {
            icon: "software-update-available",
            tip: tr!("tip-up-to-date"),
            active: false,
        },
        _ => TrayLook {
            icon: available,
            tip: tr!(
                "tip-available",
                updates = count(updates.len()),
                count = updates.len()
            ),
            active: true,
        },
    }
}

/// The ids a balloon already announced, so a repeat check stays quiet.
#[derive(Debug, Default)]
pub struct Announced(BTreeSet<String>);

impl Announced {
    /// Whether `updates` holds one not announced yet; all are announced afterwards.
    /// An update that went away is forgotten, so it is news again if it returns.
    pub fn news(&mut self, updates: &[Update]) -> bool {
        let now: BTreeSet<String> = updates.iter().map(Update::key).collect();
        let news = now.difference(&self.0).next().is_some();
        self.0 = now;
        news
    }
}

pub fn news_balloon(updates: &[Update]) -> (String, String) {
    let summary = if updates.iter().any(|u| u.kind == Kind::Security) {
        tr!("balloon-news-security")
    } else {
        tr!("balloon-news")
    };
    (
        summary,
        tr!(
            "balloon-news-body",
            updates = count(updates.len()),
            count = updates.len()
        ),
    )
}

pub fn check_failed(why: &str) -> String {
    tr!("check-failed", why = why)
}

pub fn install_failed(why: &str) -> String {
    tr!("install-failed", why = why)
}

pub fn installed_balloon(restart: Restart) -> (String, String) {
    let body = match restart {
        Restart::System => tr!("balloon-installed-restart"),
        Restart::Session => tr!("balloon-installed-logoff"),
        Restart::None => tr!("balloon-installed-none"),
    };
    (tr!("balloon-installed-title"), body)
}

pub fn restart_balloon(due: Due) -> (String, String) {
    match due {
        Due::System => (tr!("balloon-due-system-title"), tr!("balloon-due-system")),
        Due::Reprise => (tr!("balloon-due-reprise-title"), tr!("balloon-due-reprise")),
    }
}

/// Something a device asks for while its firmware installs: `now` holds the install until it is
/// done, otherwise it finishes the update afterwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub message: String,
    pub now: bool,
}

pub fn request_balloon(r: &Request) -> (String, String) {
    let summary = if r.now {
        tr!("balloon-request-now")
    } else {
        tr!("balloon-request-later")
    };
    (summary, r.message.clone())
}

/// The balloon closing the window sends: a request to do now, which only the headline showed.
pub fn closed_balloon(phase: &Phase) -> Option<(String, String)> {
    match phase {
        Phase::Installing {
            request: Some(message),
            ..
        } => Some(request_balloon(&Request {
            message: message.clone(),
            now: true,
        })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_of_a_thousand_or_more_have_no_group_separator() {
        assert_eq!(count(1234), "1234 updates");
        assert_eq!(
            tray_look(&Phase::Ready, &vec![pkg("a", Kind::Normal); 1234]).tip,
            "1234 updates are available"
        );
    }

    #[test]
    fn a_restart_only_reprise_asked_for_says_logging_off_is_enough() {
        let line = headline(&Phase::RestartDue(Due::Reprise), &[], 0);
        assert!(line.starts_with("Reprise was updated. Log off"), "{line}");
        let (summary, body) = restart_balloon(Due::Reprise);
        assert_eq!(summary, "Reprise was updated");
        assert!(body.starts_with("Log off and back on"), "{body}");
        let system = headline(&Phase::RestartDue(Due::System), &[], 0);
        assert!(system.contains("Some devices may not work"), "{system}");
    }

    #[test]
    fn closing_on_a_request_to_do_now_balloons_it_and_closing_otherwise_does_not() {
        let installing = |request: Option<&str>| Phase::Installing {
            fraction: Some(0.5),
            doing: "Waiting for you".into(),
            request: request.map(String::from),
        };
        let replug = "Unplug Webcam and plug it back in to continue the update.";
        assert_eq!(
            closed_balloon(&installing(Some(replug))),
            Some(("Firmware update needs you".into(), replug.into()))
        );
        assert_eq!(closed_balloon(&installing(None)), None);
        assert_eq!(closed_balloon(&Phase::Ready), None);
    }

    fn pkg(name: &str, kind: Kind) -> Update {
        Update {
            id: format!("{name};1.0;x86_64;repo"),
            name: name.into(),
            current: String::new(),
            version: "1.0".into(),
            summary: String::new(),
            kind,
            origin: Origin::Package,
        }
    }

    fn from(name: &str, origin: Origin) -> Update {
        Update {
            origin,
            ..pkg(name, Kind::Normal)
        }
    }

    #[test]
    fn groups_come_in_order_and_apps_before_runtimes() {
        let mut u = vec![
            from("webcam", Origin::Firmware),
            from("Platform", Origin::Runtime(Install::User)),
            from("zlib", Origin::Package),
            from("Editor", Origin::App(Install::System)),
            pkg("openssl", Kind::Security),
        ];
        sort(&mut u);
        let names: Vec<&str> = u.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["openssl", "zlib", "Editor", "Platform", "webcam"]);
    }

    #[test]
    fn a_failed_source_keeps_its_updates_and_a_gone_one_drops_them() {
        let old = [
            from("zlib", Origin::Package),
            from("Editor", Origin::App(Install::User)),
            from("webcam", Origin::Firmware),
        ];
        let found: Found = vec![
            (Group::Packages, Ok(vec![from("bash", Origin::Package)])),
            (Group::Flatpak, Err("No network.".into())),
        ];
        let (u, failed) = merge(&old, found);
        let names: Vec<&str> = u.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["bash", "Editor"]);
        assert_eq!(failed.as_deref(), Some("No network."));
    }

    #[test]
    fn the_cells_show_the_version_step_and_the_flatpak_kind() {
        let mut fw = from("webcam", Origin::Firmware);
        fw.current = "1.2.2".into();
        fw.version = "1.2.4".into();
        assert_eq!(fw.version_cell(), "1.2.2 → 1.2.4");
        assert_eq!(fw.type_cell(), "Update");
        assert_eq!(pkg("bash", Kind::Normal).version_cell(), "1.0");
        let rt = from("Platform", Origin::Runtime(Install::System));
        assert_eq!(rt.type_cell(), "Runtime (system)");
        assert_eq!(
            from("Editor", Origin::App(Install::User)).type_cell(),
            "Application (user)"
        );
        // One ref in both installations is two updates.
        let user = from("Platform", Origin::Runtime(Install::User));
        assert_ne!(rt.key(), user.key());
    }

    #[test]
    fn a_package_id_splits_into_name_and_version() {
        assert_eq!(
            split_package_id("firefox;131.0-1;x86_64;extra"),
            ("firefox", "131.0-1")
        );
        assert_eq!(split_package_id("odd"), ("odd", ""));
    }

    #[test]
    fn security_updates_sort_first_then_by_name() {
        let mut u = vec![
            pkg("zlib", Kind::Normal),
            pkg("Bash", Kind::Normal),
            pkg("openssl", Kind::Security),
        ];
        sort(&mut u);
        let names: Vec<&str> = u.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["openssl", "Bash", "zlib"]);
    }

    #[test]
    fn the_headline_counts_and_agrees_in_number() {
        let one = [pkg("a", Kind::Normal)];
        let two = [pkg("a", Kind::Normal), pkg("b", Kind::Normal)];
        assert_eq!(
            headline(&Phase::Ready, &one, 1),
            "1 update is available for your computer. Click Install to download and install it."
        );
        assert!(headline(&Phase::Ready, &two, 2).starts_with("2 updates are available"));
        assert_eq!(
            headline(&Phase::Ready, &[], 0),
            "Your computer is up to date."
        );
    }

    #[test]
    fn the_headline_counts_the_checked_updates_when_some_are_unchecked() {
        let one = [pkg("a", Kind::Normal)];
        let eight: Vec<Update> = (0..8)
            .map(|i| pkg(&format!("p{i}"), Kind::Normal))
            .collect();
        assert_eq!(
            headline(&Phase::Ready, &eight, 5),
            "5 of 8 updates selected. Click Install to download and install them."
        );
        assert_eq!(
            headline(&Phase::Ready, &eight, 1),
            "1 of 8 updates selected. Click Install to download and install it."
        );
        assert_eq!(
            headline(&Phase::Ready, &eight, 0),
            "0 of 8 updates selected. Check the updates you want to install."
        );
        assert_eq!(
            headline(&Phase::Ready, &one, 0),
            "0 of 1 update selected. Check the updates you want to install."
        );
        assert!(headline(&Phase::Ready, &eight, 8).starts_with("8 updates are available"));
    }

    #[test]
    fn the_icon_hides_with_nothing_to_do() {
        assert!(!tray_look(&Phase::Ready, &[]).active);
        assert!(!tray_look(&Phase::Unchecked, &[]).active);
        let look = tray_look(&Phase::Ready, &[pkg("a", Kind::Normal)]);
        assert!(look.active);
        assert_eq!(look.tip, "1 update is available");
        assert_eq!(look.icon, "software-update-available");
        let urgent = tray_look(&Phase::Ready, &[pkg("openssl", Kind::Security)]);
        assert_eq!(urgent.icon, "software-update-urgent");
        assert!(tray_look(&Phase::Installed(Restart::System), &[]).active);
    }

    #[test]
    fn an_install_shows_its_progress() {
        let p = Phase::Installing {
            fraction: Some(0.426),
            doing: "Downloading firefox".into(),
            request: None,
        };
        assert_eq!(progress_line(&p), "Downloading firefox (43%)");
        assert_eq!(tray_look(&p, &[]).tip, "Installing updates (43%)");
        assert!(p.busy());
        assert!(!Phase::Ready.busy());
    }

    #[test]
    fn only_a_new_update_is_news() {
        let mut a = Announced::default();
        assert!(!a.news(&[]), "nothing is no news");
        let first = [pkg("a", Kind::Normal)];
        assert!(a.news(&first));
        assert!(!a.news(&first), "the same update again");
        let both = [pkg("a", Kind::Normal), pkg("b", Kind::Normal)];
        assert!(a.news(&both), "b is new");
        assert!(!a.news(&first), "one went away");
        assert!(a.news(&both), "and came back");
    }
}
