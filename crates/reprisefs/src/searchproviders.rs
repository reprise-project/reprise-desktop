//! The programs that answer the launcher's search over D-Bus: each one's
//! `gnome-shell/search-providers/*.ini` under the XDG data directories, and the order and on/off
//! state `[launcher] providers` and `providers_off` give them.

use std::path::{Path, PathBuf};

/// The only `org.gnome.Shell.SearchProvider2` file version read.
const VERSION: &str = "2";
const GROUP: &str = "[Shell Search Provider]";

/// The desktop ids of programs whose provider searches through localsearch and so starts the
/// file indexer when asked: none is asked while `[launcher] contents` is off.
pub const STARTS_INDEXER: [&str; 1] = ["org.gnome.Nautilus.desktop"];

/// Whether asking `desktop_id`'s provider starts the file indexer ([`STARTS_INDEXER`]).
pub fn starts_indexer(desktop_id: &str) -> bool {
    STARTS_INDEXER.contains(&desktop_id)
}

/// One provider file: whose program it is and where on the session bus it answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provider {
    /// The program's desktop entry, `org.gnome.Calculator.desktop`; the provider's key in config.
    pub desktop_id: String,
    pub bus_name: String,
    pub object_path: String,
    /// The file asks to stay off until turned on.
    pub default_disabled: bool,
}

/// The provider `text` describes; None for another version or a file missing a key.
pub fn parse(text: &str) -> Option<Provider> {
    let kv = crate::apps::group_kv(text, GROUP);
    let get = |k: &str| kv.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
    if get("Version")? != VERSION {
        return None;
    }
    Some(Provider {
        desktop_id: get("DesktopId")?.to_string(),
        bus_name: get("BusName")?.to_string(),
        object_path: get("ObjectPath")?.to_string(),
        default_disabled: get("DefaultDisabled") == Some("true"),
    })
}

/// The provider directories, most important first: the user's data directory, then
/// `XDG_DATA_DIRS`.
pub fn default_dirs() -> Vec<PathBuf> {
    (crate::apps::default_dirs().iter())
        .filter_map(|apps| apps.parent())
        .map(|share| share.join("gnome-shell/search-providers"))
        .collect()
}

/// Every provider in `dirs`, one per program: a program's file in an earlier directory hides one
/// in a later. Within a directory files are read in name order. Blocks on the file system.
pub fn scan(dirs: &[PathBuf]) -> Vec<Provider> {
    let mut out: Vec<Provider> = Vec::new();
    for dir in dirs {
        for path in ini_files(dir) {
            let Some(p) = std::fs::read_to_string(&path).ok().and_then(|t| parse(&t)) else {
                continue;
            };
            if !out.iter().any(|o| o.desktop_id == p.desktop_id) {
                out.push(p);
            }
        }
    }
    out
}

fn ini_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = (rd.flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ini"))
        .collect();
    files.sort();
    files
}

/// Whether `p` answers: turned off by `off`, else on unless its file says otherwise and `order`
/// does not name it.
pub fn is_on(p: &Provider, order: &[String], off: &[String]) -> bool {
    let id = &p.desktop_id;
    !off.contains(id) && (!p.default_disabled || order.contains(id))
}

/// `providers` in the order `order` names them, then the rest by `name` (case-folded), each with
/// whether it is on ([`is_on`]).
pub fn arrange<'a>(
    providers: &'a [Provider],
    order: &[String],
    off: &[String],
    name: impl Fn(&Provider) -> String,
) -> Vec<(&'a Provider, bool)> {
    let listed = (order.iter()).filter_map(|id| providers.iter().find(|p| p.desktop_id == *id));
    let mut rest: Vec<&Provider> = (providers.iter())
        .filter(|p| !order.contains(&p.desktop_id))
        .collect();
    rest.sort_by_cached_key(|p| name(p).to_lowercase());
    let mut out: Vec<&Provider> = Vec::new();
    for p in listed.chain(rest) {
        if !out.iter().any(|o| o.desktop_id == p.desktop_id) {
            out.push(p);
        }
    }
    out.into_iter().map(|p| (p, is_on(p, order, off))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALC: &str = "[Shell Search Provider]\n\
        DesktopId=org.gnome.Calculator.desktop\n\
        BusName=org.gnome.Calculator.SearchProvider\n\
        ObjectPath=/org/gnome/Calculator/SearchProvider\n\
        Version=2\n";

    fn provider(id: &str, default_disabled: bool) -> Provider {
        Provider {
            desktop_id: id.into(),
            bus_name: format!("org.example.{id}"),
            object_path: "/".into(),
            default_disabled,
        }
    }

    #[test]
    fn a_version_2_file_names_its_program_and_bus_address() {
        assert_eq!(
            parse(CALC),
            Some(Provider {
                desktop_id: "org.gnome.Calculator.desktop".into(),
                bus_name: "org.gnome.Calculator.SearchProvider".into(),
                object_path: "/org/gnome/Calculator/SearchProvider".into(),
                default_disabled: false,
            })
        );
        let off = format!("{CALC}DefaultDisabled=true\n");
        assert!(parse(&off).unwrap().default_disabled);
        assert_eq!(parse(&CALC.replace("Version=2", "Version=1")), None);
        assert_eq!(parse(&CALC.replace("BusName", "Bus")), None);
        assert_eq!(parse(&CALC.replace("[Shell", "[Other")), None);
    }

    #[test]
    fn an_earlier_directory_hides_a_later_ones_file_for_the_same_program() {
        let root = std::env::temp_dir().join(format!("reprise-providers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (user, system) = (root.join("user"), root.join("system"));
        for d in [&user, &system] {
            std::fs::create_dir_all(d).unwrap();
        }
        let mine = CALC.replace("/org/gnome/Calculator/SearchProvider", "/mine");
        std::fs::write(user.join("calc.ini"), mine).unwrap();
        std::fs::write(system.join("calc.ini"), CALC).unwrap();
        let other = CALC.replace("Calculator.desktop", "Characters.desktop");
        std::fs::write(system.join("chars.ini"), other).unwrap();
        std::fs::write(system.join("notes.txt"), CALC).unwrap();
        let found = scan(&[user, system, root.join("missing")]);
        let got: Vec<(&str, &str)> = (found.iter())
            .map(|p| (p.desktop_id.as_str(), p.object_path.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("org.gnome.Calculator.desktop", "/mine"),
                (
                    "org.gnome.Characters.desktop",
                    "/org/gnome/Calculator/SearchProvider"
                )
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn listed_providers_come_first_in_their_order_then_the_rest_by_name() {
        let all = [
            provider("c.desktop", false),
            provider("a.desktop", false),
            provider("b.desktop", false),
            provider("d.desktop", false),
        ];
        let order = ["d.desktop", "gone.desktop", "c.desktop"].map(String::from);
        let got: Vec<&str> = arrange(&all, &order, &[], |p| p.desktop_id.to_uppercase())
            .into_iter()
            .map(|(p, _)| p.desktop_id.as_str())
            .collect();
        assert_eq!(got, ["d.desktop", "c.desktop", "a.desktop", "b.desktop"]);
    }

    #[test]
    fn a_provider_is_on_unless_turned_off_or_off_by_default_and_unlisted() {
        let plain = provider("a.desktop", false);
        let shy = provider("b.desktop", true);
        let none: [String; 0] = [];
        let a = ["a.desktop".to_string()];
        let b = ["b.desktop".to_string()];
        assert!(is_on(&plain, &none, &none));
        assert!(!is_on(&plain, &none, &a), "turned off");
        assert!(!is_on(&shy, &none, &none), "off until listed");
        assert!(is_on(&shy, &b, &none), "listed: on");
        assert!(!is_on(&shy, &b, &b), "listed and turned off");
    }
}
