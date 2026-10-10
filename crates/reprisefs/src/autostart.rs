//! XDG Autostart entry parsing and the text edits reprise-settings needs to hide or override one.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::apps::{desktop_entry_kv, escape_value, exec_argv, split_list, strip_field_codes};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub exec: String,
    /// The `Exec=` value with escapes decoded and field codes kept, written
    /// back by `override_text` (re-serialising `exec` would turn `%%` into a
    /// field code and lose spacing inside quoted arguments).
    pub exec_line: String,
    pub hidden: bool,
    pub only_show_in: Vec<String>,
    pub not_show_in: Vec<String>,
    pub try_exec: Option<String>,
    pub path: Option<PathBuf>,
    pub terminal: bool,
    pub user: bool,
}

pub fn desktop_names(xdg_current_desktop: Option<&str>) -> Vec<String> {
    let names: Vec<String> = xdg_current_desktop
        .unwrap_or("")
        .split(':')
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        vec!["reprise".to_string()]
    } else {
        names
    }
}

impl Entry {
    /// Whether this entry would start on a desktop announcing `desktops`:
    /// never when hidden, else `OnlyShowIn` / `NotShowIn` against any name.
    pub fn runs_on(&self, desktops: &[String]) -> bool {
        if self.hidden {
            return false;
        }
        let named = |list: &[String]| list.iter().any(|d| desktops.iter().any(|x| x == d));
        if !self.only_show_in.is_empty() {
            return named(&self.only_show_in);
        }
        if !self.not_show_in.is_empty() {
            return !named(&self.not_show_in);
        }
        true
    }

    pub fn runs_in(&self, desktop: &str) -> bool {
        self.runs_on(std::slice::from_ref(&desktop.to_string()))
    }

    /// The argv that starts this entry, with no shell: `exec_line` split by
    /// [`exec_argv`], under `terminal -e` when the entry asks for one.
    pub fn argv(&self, terminal: &str) -> Vec<OsString> {
        let argv = exec_argv(&self.exec_line)
            .into_iter()
            .map(OsString::from)
            .collect();
        if self.terminal {
            crate::apps::in_terminal(terminal, argv)
        } else {
            argv
        }
    }
}

/// `Hidden` and the `ShowIn` lists are kept so a settings page can toggle a
/// hidden entry. A `Hidden=true` file with no `Exec` is kept with an empty
/// command: it only deletes the system entry of its id (`merge`).
pub fn parse(id: &str, text: &str, user: bool) -> Option<Entry> {
    let kv = desktop_entry_kv(text);
    let get = |k: &str| kv.get(k).map(|v| v.as_ref());
    if let Some(t) = get("Type")
        && t != "Application"
    {
        return None;
    }
    let hidden = get("Hidden") == Some("true");
    let exec_line = match get("Exec") {
        Some(e) => e.to_string(),
        None if hidden => String::new(),
        None => return None,
    };
    let exec = strip_field_codes(&exec_line);
    if exec.is_empty() && !hidden {
        return None;
    }
    Some(Entry {
        id: id.to_string(),
        name: get("Name").unwrap_or(id).to_string(),
        exec,
        exec_line,
        hidden,
        only_show_in: split_list(get("OnlyShowIn")),
        not_show_in: split_list(get("NotShowIn")),
        try_exec: get("TryExec").filter(|s| !s.is_empty()).map(str::to_string),
        path: get("Path").filter(|s| !s.is_empty()).map(PathBuf::from),
        terminal: get("Terminal") == Some("true"),
        user,
    })
}

pub fn system_dirs() -> Vec<PathBuf> {
    std::env::var("XDG_CONFIG_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/etc/xdg".into())
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn read_dir(dir: &Path, user: bool) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    paths.sort();
    paths
        .iter()
        .filter_map(|p| {
            let id = p.file_name()?.to_str()?.strip_suffix(".desktop")?;
            let text = crate::apps::read_desktop_file(p).ok()?;
            parse(id, &text, user)
        })
        .collect()
}

/// Every entry visible from `config_dirs` and `config_home`, each with
/// `autostart` appended, merged by id and sorted by name. Among the system
/// directories the first with an id wins; a user file shadows the system
/// file of the same id.
pub fn scan(config_dirs: &[PathBuf], config_home: &Path) -> Vec<Entry> {
    let mut system: Vec<Entry> = Vec::new();
    for dir in config_dirs {
        for e in read_dir(&dir.join("autostart"), false) {
            if !system.iter().any(|s| s.id == e.id) {
                system.push(e);
            }
        }
    }
    merge(system, read_dir(&config_home.join("autostart"), true))
}

/// Merge a system and a user entry list by id, the user copy winning,
/// sorted by name case-insensitively. A user file that only says
/// `Hidden=true` (no command) hides the system entry it shadows; with none
/// behind it, it lists nothing.
pub fn merge(system: Vec<Entry>, user: Vec<Entry>) -> Vec<Entry> {
    let mut by_id: HashMap<String, Entry> = HashMap::new();
    for e in system {
        by_id.insert(e.id.clone(), e);
    }
    for e in user {
        if e.exec.is_empty() {
            if let Some(sys) = by_id.get_mut(&e.id) {
                sys.hidden = true;
                sys.user = true;
            }
            continue;
        }
        by_id.insert(e.id.clone(), e);
    }
    let mut entries: Vec<Entry> = by_id.into_values().collect();
    // Entries sharing a name break on their id: HashMap order changes
    // from run to run.
    entries.sort_by_cached_key(|e| (e.name.to_lowercase(), e.id.clone()));
    entries
}

pub fn override_text(e: &Entry) -> String {
    let mut out = format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={}\nHidden={}\n",
        escape_value(&e.name),
        escape_value(&e.exec_line),
        e.hidden
    );
    // The shadow replaces the system entry outright, so the keys that gated
    // it come along.
    if !e.only_show_in.is_empty() {
        out.push_str(&format!("OnlyShowIn={};\n", e.only_show_in.join(";")));
    }
    if !e.not_show_in.is_empty() {
        out.push_str(&format!("NotShowIn={};\n", e.not_show_in.join(";")));
    }
    if let Some(t) = &e.try_exec {
        out.push_str(&format!("TryExec={}\n", escape_value(t)));
    }
    if let Some(p) = &e.path {
        out.push_str(&format!("Path={}\n", escape_value(&p.to_string_lossy())));
    }
    if e.terminal {
        out.push_str("Terminal=true\n");
    }
    out.push_str("X-Reprise-Override=true\n");
    out
}

pub fn is_override(text: &str) -> bool {
    desktop_entry_kv(text)
        .get("X-Reprise-Override")
        .is_some_and(|v| v == "true")
}

/// Rewrite `text`'s `[Desktop Entry]` `Hidden` line to `hidden`, or
/// insert one right after the header when absent. Idempotent, and
/// leaves every other line (other groups, comments, key order) untouched.
pub fn set_hidden(text: &str, hidden: bool) -> String {
    let value = if hidden { "true" } else { "false" };
    let had_trailing_newline = text.ends_with('\n');
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

    let Some(header) = lines.iter().position(|l| l.trim() == "[Desktop Entry]") else {
        return text.to_string();
    };
    let group_end = lines[header + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map(|i| header + 1 + i)
        .unwrap_or(lines.len());

    let hidden_line = (header + 1..group_end).find(|&i| {
        lines[i]
            .split_once('=')
            .map(|(k, _)| k.trim() == "Hidden")
            .unwrap_or(false)
    });

    match hidden_line {
        Some(i) => lines[i] = format!("Hidden={value}"),
        None => lines.insert(header + 1, format!("Hidden={value}")),
    }

    let mut out = lines.join("\n");
    if had_trailing_newline {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), text).unwrap();
    }

    fn tree(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("reprise-autostart-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn scan_reads_system_and_user_directories() {
        let root = tree("read");
        write(
            &root.join("etc/xdg/autostart"),
            "applet.desktop",
            "[Desktop Entry]\nName=Applet\nExec=applet\n",
        );
        write(
            &root.join("home/autostart"),
            "note.desktop",
            "[Desktop Entry]\nName=Note\nExec=note\n",
        );
        let found = scan(&[root.join("etc/xdg")], &root.join("home"));
        let ids: Vec<(&str, bool)> = found.iter().map(|e| (e.id.as_str(), e.user)).collect();
        assert_eq!(ids, vec![("applet", false), ("note", true)]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_lets_a_user_file_replace_a_system_one() {
        let root = tree("override");
        write(
            &root.join("etc/xdg/autostart"),
            "applet.desktop",
            "[Desktop Entry]\nName=Applet\nExec=/system/applet\n",
        );
        write(
            &root.join("home/autostart"),
            "applet.desktop",
            "[Desktop Entry]\nName=Applet\nExec=/user/applet\nHidden=true\n",
        );
        let found = scan(&[root.join("etc/xdg")], &root.join("home"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].exec, "/user/applet");
        assert!(found[0].hidden);
        assert!(found[0].user);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_user_hidden_file_without_exec_hides_the_system_entry() {
        let root = tree("hide-only");
        write(
            &root.join("etc/xdg/autostart"),
            "applet.desktop",
            "[Desktop Entry]\nName=Applet\nExec=/system/applet\n",
        );
        write(
            &root.join("etc/xdg/autostart"),
            "other.desktop",
            "[Desktop Entry]\nName=Other\nExec=/system/other\n",
        );
        write(
            &root.join("home/autostart"),
            "applet.desktop",
            "[Desktop Entry]\nHidden=true\n",
        );
        write(
            &root.join("home/autostart"),
            "gone.desktop",
            "[Desktop Entry]\nHidden=true\n",
        );
        let found = scan(&[root.join("etc/xdg")], &root.join("home"));
        let ids: Vec<(&str, &str, bool, bool)> = found
            .iter()
            .map(|e| (e.id.as_str(), e.exec.as_str(), e.hidden, e.user))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("applet", "/system/applet", true, true),
                ("other", "/system/other", false, false),
            ],
            "the system applet is hidden and keeps its command; a hide-only file with nothing behind it lists nothing"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_takes_the_first_system_directory_that_has_an_id() {
        let root = tree("order");
        write(
            &root.join("first/autostart"),
            "applet.desktop",
            "[Desktop Entry]\nName=Applet\nExec=/first/applet\n",
        );
        write(
            &root.join("second/autostart"),
            "applet.desktop",
            "[Desktop Entry]\nName=Applet\nExec=/second/applet\n",
        );
        let found = scan(
            &[root.join("first"), root.join("second")],
            &root.join("home"),
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].exec, "/first/applet");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_ignores_files_that_are_not_desktop_scan() {
        let root = tree("skip");
        write(
            &root.join("etc/xdg/autostart"),
            "applet.desktop",
            "[Desktop Entry]\nName=Applet\nExec=applet\n",
        );
        write(&root.join("etc/xdg/autostart"), "notes.txt", "Exec=nope\n");
        write(
            &root.join("etc/xdg/autostart"),
            "link.desktop",
            "[Desktop Entry]\nType=Link\nExec=nope\n",
        );
        let found = scan(&[root.join("etc/xdg")], &root.join("home"));
        let ids: Vec<&str> = found.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["applet"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_tolerates_a_missing_directory() {
        let root = tree("missing");
        assert!(scan(&[root.join("nowhere")], &root.join("neither")).is_empty());
    }

    #[test]
    fn parse_reads_the_entry_group() {
        let text = "[Desktop Entry]\nType=Application\nName=Applet\nExec=/usr/bin/applet\nHidden=true\nOnlyShowIn=GNOME;KDE\nNotShowIn=XFCE\nTryExec=applet\n";
        let e = parse("applet", text, false).unwrap();
        assert_eq!(e.id, "applet");
        assert_eq!(e.name, "Applet");
        assert_eq!(e.exec, "/usr/bin/applet");
        assert!(e.hidden);
        assert_eq!(e.only_show_in, vec!["GNOME", "KDE"]);
        assert_eq!(e.not_show_in, vec!["XFCE"]);
        assert_eq!(e.try_exec.as_deref(), Some("applet"));
        assert!(!e.user);
    }

    #[test]
    fn parse_name_falls_back_to_id() {
        let text = "[Desktop Entry]\nExec=/usr/bin/applet\n";
        let e = parse("applet", text, true).unwrap();
        assert_eq!(e.name, "applet");
        assert!(e.user);
    }

    #[test]
    fn parse_strips_field_codes() {
        let text = "[Desktop Entry]\nExec=applet %f --flag %U %%\n";
        let e = parse("applet", text, false).unwrap();
        assert_eq!(e.exec, "applet --flag %");
    }

    #[test]
    fn parse_rejects_non_application_type() {
        let text = "[Desktop Entry]\nType=Link\nExec=applet\n";
        assert!(parse("applet", text, false).is_none());
    }

    #[test]
    fn parse_tolerates_missing_type() {
        let text = "[Desktop Entry]\nExec=applet\n";
        assert!(parse("applet", text, false).is_some());
    }

    #[test]
    fn parse_requires_exec() {
        let text = "[Desktop Entry]\nName=Applet\n";
        assert!(parse("applet", text, false).is_none());
    }

    #[test]
    fn parse_ignores_keys_outside_the_entry_group() {
        let text = "[Desktop Entry]\nExec=applet\n\n[X-Other Group]\nExec=other\nHidden=true\n";
        let e = parse("applet", text, false).unwrap();
        assert_eq!(e.exec, "applet");
        assert!(!e.hidden);
    }

    #[test]
    fn runs_in_only_show_in() {
        let mut e = parse("a", "[Desktop Entry]\nExec=a\nOnlyShowIn=GNOME\n", false).unwrap();
        assert!(e.runs_in("GNOME"));
        assert!(!e.runs_in("KDE"));
        e.only_show_in.clear();
        assert!(e.runs_in("KDE"));
    }

    #[test]
    fn desktop_names_take_every_entry_or_reprise() {
        assert_eq!(
            desktop_names(Some("reprise:wlroots")),
            vec!["reprise", "wlroots"]
        );
        assert_eq!(desktop_names(Some("GNOME")), vec!["GNOME"]);
        assert_eq!(desktop_names(Some(":")), vec!["reprise"]);
        assert_eq!(desktop_names(None), vec!["reprise"]);
        let e = parse("a", "[Desktop Entry]\nExec=a\nOnlyShowIn=wlroots\n", false).unwrap();
        assert!(e.runs_on(&desktop_names(Some("reprise:wlroots"))));
        assert!(!e.runs_in("reprise"));
    }

    #[test]
    fn runs_in_not_show_in() {
        let e = parse("a", "[Desktop Entry]\nExec=a\nNotShowIn=reprise\n", false).unwrap();
        assert!(!e.runs_in("reprise"));
        assert!(e.runs_in("gnome"));
    }

    #[test]
    fn runs_in_never_true_when_hidden() {
        let e = parse("a", "[Desktop Entry]\nExec=a\nHidden=true\n", false).unwrap();
        assert!(!e.runs_in("reprise"));
    }

    #[test]
    fn merge_user_entry_replaces_system_entry_by_id() {
        let system = vec![parse("a", "[Desktop Entry]\nExec=/system/a\n", false).unwrap()];
        let user = vec![parse("a", "[Desktop Entry]\nExec=/user/a\nHidden=true\n", true).unwrap()];
        let merged = merge(system, user);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].exec, "/user/a");
        assert!(merged[0].hidden);
        assert!(merged[0].user);
    }

    #[test]
    fn merge_sorts_by_name_case_insensitively() {
        let system = vec![
            parse("b", "[Desktop Entry]\nName=banana\nExec=b\n", false).unwrap(),
            parse("c", "[Desktop Entry]\nName=cherry\nExec=c\n", false).unwrap(),
        ];
        let user = vec![parse("a", "[Desktop Entry]\nName=Apple\nExec=a\n", true).unwrap()];
        let merged = merge(system, user);
        let names: Vec<&str> = merged.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["Apple", "banana", "cherry"]);
    }

    #[test]
    fn override_text_matches_the_spec_format() {
        let e = parse(
            "a",
            "[Desktop Entry]\nName=Applet\nExec=/usr/bin/applet\nHidden=true\n",
            true,
        )
        .unwrap();
        assert_eq!(
            override_text(&e),
            "[Desktop Entry]\nType=Application\nName=Applet\nExec=/usr/bin/applet\nHidden=true\nX-Reprise-Override=true\n"
        );
    }

    #[test]
    fn override_text_keeps_the_show_in_and_try_exec_keys() {
        let text = "[Desktop Entry]\nName=A\nExec=app\nOnlyShowIn=reprise;GNOME;\n\
                    NotShowIn=KDE;\nTryExec=/usr/bin/app\n";
        let e = parse("a", text, false).unwrap();
        let again = parse("a", &override_text(&e), true).unwrap();
        assert_eq!(again.only_show_in, ["reprise", "GNOME"]);
        assert_eq!(again.not_show_in, ["KDE"]);
        assert_eq!(again.try_exec.as_deref(), Some("/usr/bin/app"));
        assert!(is_override(&override_text(&e)));
    }

    #[test]
    fn override_text_keeps_the_path_and_terminal_keys() {
        let text = "[Desktop Entry]\nName=A\nExec=app\nPath=/srv/my work\nTerminal=true\n";
        let e = parse("a", text, false).unwrap();
        let again = parse("a", &override_text(&e), true).unwrap();
        assert_eq!(again.path.as_deref(), Some(Path::new("/srv/my work")));
        assert!(again.terminal);
    }

    #[test]
    fn argv_wraps_a_terminal_entry_in_the_terminal() {
        let text = "[Desktop Entry]\nExec=top -d \"1 2\" %U 100%%\n";
        let plain = parse("a", text, false).unwrap();
        assert_eq!(plain.argv("foot"), ["top", "-d", "1 2", "100%"]);
        assert_eq!(plain.path, None);
        let term = parse(
            "a",
            "[Desktop Entry]\nExec=top -d 1\nTerminal=true\n",
            false,
        )
        .unwrap();
        assert_eq!(
            term.argv("foot --app-id x"),
            ["foot", "--app-id", "x", "-e", "top", "-d", "1"]
        );
    }

    #[test]
    fn override_text_keeps_the_exec_line_verbatim() {
        let text = "[Desktop Entry]\nName=A\nExec=app \"a  b\" %U 100%%\n";
        let e = parse("a", text, true).unwrap();
        assert_eq!(e.exec, "app \"a  b\" 100%");
        assert!(override_text(&e).contains("\nExec=app \"a  b\" %U 100%%\n"));
        let again = parse("a", &override_text(&e), true).unwrap();
        assert_eq!(again.exec, e.exec);
        assert_eq!(again.exec_line, e.exec_line);
    }

    #[test]
    fn is_override_reads_the_marker() {
        assert!(is_override(
            "[Desktop Entry]\nExec=a\nX-Reprise-Override=true\n"
        ));
        assert!(!is_override("[Desktop Entry]\nExec=a\n"));
    }

    #[test]
    fn set_hidden_inserts_when_absent() {
        let text = "[Desktop Entry]\nType=Application\nExec=a\n";
        let out = set_hidden(text, true);
        assert_eq!(
            out,
            "[Desktop Entry]\nHidden=true\nType=Application\nExec=a\n"
        );
    }

    #[test]
    fn set_hidden_rewrites_when_present() {
        let text = "[Desktop Entry]\nExec=a\nHidden=false\nName=A\n";
        let out = set_hidden(text, true);
        assert_eq!(out, "[Desktop Entry]\nExec=a\nHidden=true\nName=A\n");
    }

    #[test]
    fn set_hidden_is_idempotent() {
        let text = "[Desktop Entry]\nExec=a\n";
        let once = set_hidden(text, true);
        let twice = set_hidden(&once, true);
        assert_eq!(once, twice);
    }

    #[test]
    fn set_hidden_leaves_other_groups_and_comments_untouched() {
        let text = "# a comment\n[Desktop Entry]\nExec=a\n\n[X-Other]\nHidden=false\nFoo=bar\n";
        let out = set_hidden(text, true);
        assert_eq!(
            out,
            "# a comment\n[Desktop Entry]\nHidden=true\nExec=a\n\n[X-Other]\nHidden=false\nFoo=bar\n"
        );
    }
}
