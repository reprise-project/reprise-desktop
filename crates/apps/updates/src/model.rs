//! What reprise-updates knows and says, with no bus: the updates a check
//! found (packages, Flatpak apps and runtimes, firmware), the app's phase,
//! the lines the window and tray show, and when a balloon is worth sending.

use std::collections::BTreeSet;

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
    pub fn label(self) -> &'static str {
        match self {
            Kind::Security => "Security update",
            Kind::Important => "Important update",
            Kind::Bugfix => "Bug fix",
            Kind::Normal => "Update",
            Kind::Enhancement => "Enhancement",
            Kind::Low => "Optional update",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Install {
    System,
    User,
}

impl Install {
    pub fn label(self) -> &'static str {
        match self {
            Install::System => "system",
            Install::User => "user",
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
    pub fn title(self) -> &'static str {
        match self {
            Group::Packages => "System packages",
            Group::Flatpak => "Flatpak",
            Group::Firmware => "Firmware",
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
            Origin::Package | Origin::Firmware => self.kind.label().into(),
            Origin::App(i) => format!("Application ({})", i.label()),
            Origin::Runtime(i) => format!("Runtime ({})", i.label()),
        }
    }

    pub fn about(&self) -> String {
        let what = match self.origin {
            Origin::Package => self.kind.label(),
            _ => self.name.as_str(),
        };
        if self.summary.is_empty() {
            what.to_string()
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
    /// An install is running: progress 0 to 1 (None while unknown) and the current step.
    Installing {
        fraction: Option<f64>,
        doing: String,
    },
    Installed(Restart),
    Failed(String),
}

impl Phase {
    pub fn busy(&self) -> bool {
        matches!(self, Phase::Checking | Phase::Installing { .. })
    }
}

pub fn count(n: usize) -> String {
    if n == 1 {
        "1 update".into()
    } else {
        format!("{n} updates")
    }
}

pub fn headline(phase: &Phase, updates: &[Update]) -> String {
    match phase {
        Phase::Unchecked => "Reprise has not checked for updates yet.".into(),
        Phase::Checking => "Checking for updates...".into(),
        Phase::Ready if updates.is_empty() => "Your computer is up to date.".into(),
        Phase::Ready => format!(
            "{} {} available for your computer. Click Install to download and install {}.",
            count(updates.len()),
            if updates.len() == 1 { "is" } else { "are" },
            if updates.len() == 1 { "it" } else { "them" },
        ),
        Phase::Installing { .. } => {
            "Installing updates. You can close this window; the updates keep installing.".into()
        }
        Phase::Installed(Restart::System) => {
            "The updates were installed. Restart your computer to finish installing them.".into()
        }
        Phase::Installed(Restart::Session) => {
            "The updates were installed. Log off and back on to finish installing them.".into()
        }
        Phase::Installed(Restart::None) => "The updates were installed.".into(),
        Phase::Failed(message) => message.clone(),
    }
}

pub fn progress_line(phase: &Phase) -> String {
    match phase {
        Phase::Installing { fraction, doing } => match fraction {
            Some(f) => format!("{doing} ({}%)", (f * 100.0).round() as i32),
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
                Some(f) => format!("Installing updates ({}%)", (f * 100.0).round() as i32),
                None => "Installing updates".into(),
            },
            active: true,
        },
        Phase::Installed(Restart::System) => TrayLook {
            icon: "system-reboot",
            tip: "Restart your computer to finish installing updates".into(),
            active: true,
        },
        Phase::Installed(Restart::Session) => TrayLook {
            icon: "system-log-out",
            tip: "Log off to finish installing updates".into(),
            active: true,
        },
        _ if updates.is_empty() => TrayLook {
            icon: "software-update-available",
            tip: "Your computer is up to date".into(),
            active: false,
        },
        _ => TrayLook {
            icon: available,
            tip: format!(
                "{} {} available",
                count(updates.len()),
                if updates.len() == 1 { "is" } else { "are" }
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
        "Security updates are ready for your computer"
    } else {
        "Updates are ready for your computer"
    };
    (
        summary.into(),
        format!(
            "{} {} available. Click here to review and install {}.",
            count(updates.len()),
            if updates.len() == 1 { "is" } else { "are" },
            if updates.len() == 1 { "it" } else { "them" },
        ),
    )
}

pub fn check_failed(why: &str) -> String {
    format!("Reprise could not check for updates. {why}")
}

pub fn install_failed(why: &str) -> String {
    format!("The updates could not be installed. {why}")
}

pub fn installed_balloon(restart: Restart) -> (String, String) {
    let body = match restart {
        Restart::System => "Restart your computer to finish installing them.",
        Restart::Session => "Log off and back on to finish installing them.",
        Restart::None => "Your computer is up to date.",
    };
    ("Updates installed".into(), body.into())
}

/// `[power] reboot` from the config, the command the Turn Off dialog's Restart runs too.
pub fn reboot_command(config: &str) -> String {
    config
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("power")?.get("reboot")?.as_str().map(str::to_string))
        .unwrap_or_else(|| "systemctl reboot".into())
}

#[cfg(test)]
mod tests {
    use super::*;

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
            headline(&Phase::Ready, &one),
            "1 update is available for your computer. Click Install to download and install it."
        );
        assert!(headline(&Phase::Ready, &two).starts_with("2 updates are available"));
        assert_eq!(headline(&Phase::Ready, &[]), "Your computer is up to date.");
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

    #[test]
    fn restart_now_runs_the_configured_reboot() {
        assert_eq!(reboot_command(""), "systemctl reboot");
        assert_eq!(
            reboot_command("[power]\nreboot = \"loginctl reboot\"\n"),
            "loginctl reboot"
        );
        assert_eq!(reboot_command("not toml ["), "systemctl reboot");
    }
}
