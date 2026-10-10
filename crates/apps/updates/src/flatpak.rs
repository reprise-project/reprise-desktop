//! The Flatpak side: application and runtime updates in the system and user installation, through the `flatpak`
//! command (it does the pull, signature checks and polkit prompt itself). Output is read in the C locale.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use crate::model::{Install, Kind, Origin, Update};

const FLATPAK: &str = "flatpak";

pub fn present() -> bool {
    Command::new(FLATPAK)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn flag(i: Install) -> &'static str {
    match i {
        Install::System => "--system",
        Install::User => "--user",
    }
}

/// Run `flatpak ARGS` to its end: its output, or the last thing it said
/// on stderr.
fn run(args: &[&str]) -> Result<String, String> {
    let out = Command::new(FLATPAK)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Flatpak could not be started: {e}."))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(complaint(&String::from_utf8_lossy(&out.stderr)))
    }
}

fn complaint(stderr: &str) -> String {
    let line = stderr
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("Flatpak failed.");
    let line = line.strip_prefix("error:").map_or(line, str::trim_start);
    let mut s = line.to_string();
    if let Some(first) = s.get(..1) {
        s.replace_range(..1, &first.to_uppercase());
    }
    if !s.ends_with('.') {
        s.push('.');
    }
    s
}

/// What is installed, from `flatpak list --columns=ref,name,version`:
/// a ref without its `app/` or `runtime/` to its name and version.
fn parse_list(out: &str) -> HashMap<String, (String, String)> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let r = f.next()?.trim();
            let name = f.next().unwrap_or("").trim();
            let version = f.next().unwrap_or("").trim();
            (!r.is_empty()).then(|| (r.to_string(), (name.to_string(), version.to_string())))
        })
        .collect()
}

/// The updates in `flatpak remote-ls --updates
/// --columns=ref,name,version,branch,origin,download-size`, named and
/// versioned as `installed` has them where it does.
fn parse_updates(
    out: &str,
    install: Install,
    installed: &HashMap<String, (String, String)>,
) -> Vec<Update> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').map(str::trim).collect();
            let r = *f.first()?;
            let (kind, bare) = r.split_once('/')?;
            let origin = match kind {
                "app" => Origin::App(install),
                "runtime" => Origin::Runtime(install),
                _ => return None,
            };
            let field = |i: usize| f.get(i).copied().unwrap_or("");
            let (now_name, now_version) = installed
                .get(bare)
                .map(|(n, v)| (n.as_str(), v.as_str()))
                .unwrap_or(("", ""));
            let id = bare.split('/').next().unwrap_or(bare);
            let name = [now_name, field(1), id]
                .into_iter()
                .find(|n| !n.is_empty())
                .unwrap_or(id);
            let version = match field(2) {
                "" => field(3),
                v => v,
            };
            let summary = match (field(4), field(5)) {
                ("", _) => String::new(),
                (o, "") => format!("from {o}"),
                (o, size) => format!("from {o}, {size} to download"),
            };
            Some(Update {
                id: r.to_string(),
                name: name.to_string(),
                current: if field(2).is_empty() {
                    String::new()
                } else {
                    now_version.to_string()
                },
                version: version.to_string(),
                summary,
                kind: Kind::Normal,
                origin,
            })
        })
        .collect()
}

fn updates_in(install: Install) -> Result<Vec<Update>, String> {
    let installed = parse_list(&run(&[
        "list",
        flag(install),
        "--columns=ref,name,version",
    ])?);
    if installed.is_empty() {
        return Ok(Vec::new());
    }
    let out = run(&[
        "remote-ls",
        "--updates",
        flag(install),
        "--columns=ref,name,version,branch,origin,download-size",
    ])?;
    Ok(parse_updates(&out, install, &installed))
}

pub fn updates() -> Result<Vec<Update>, String> {
    let mut out = updates_in(Install::System)?;
    out.extend(updates_in(Install::User)?);
    Ok(out)
}

/// The ref a `flatpak update` output line says it is starting on, as
/// `Updating app/org.example.App/x86_64/stable` (or `Installing` for a
/// ref the update pulls in).
fn started(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix("Updating ")
        .or_else(|| line.strip_prefix("Installing "))?;
    let r = rest.split_whitespace().next()?;
    r.contains('/').then_some(r)
}

/// Update `refs` in `install`, calling `step` with each ref the command
/// starts on. Any related ref they need (a newer runtime, an extension)
/// comes along.
pub fn install(
    install: Install,
    refs: &[String],
    step: &mut dyn FnMut(&str),
) -> Result<(), String> {
    if refs.is_empty() {
        return Ok(());
    }
    let mut child = Command::new(FLATPAK)
        .args(["update", flag(install), "--noninteractive", "-y"])
        .args(refs)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Flatpak could not be started: {e}."))?;
    // stderr is read on a thread of its own, so a chatty one never
    // fills its pipe while stdout is followed here.
    let err = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut e, &mut s);
            s
        })
    });
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(r) = started(line.trim()) {
                step(r);
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let stderr = err.and_then(|t| t.join().ok()).unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(complaint(&stderr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "org.example.Editor/x86_64/stable\tEditor\t2.1.0\n\
        org.freedesktop.Platform.GL.default/x86_64/26.08-extra\tMesa (Extra)\t26.1.8\n\
        org.freedesktop.Platform/x86_64/26.08\tFreedesktop Platform\tfreedesktop-sdk-26.08.0\n";

    const UPDATES: &str = "app/org.example.Editor/x86_64/stable\teditor\t2.2.0\tstable\tflathub\t1.2 MB\n\
        runtime/org.freedesktop.Platform.GL.default/x86_64/26.08-extra\tdefault\t26.1.9\t26.08-extra\tflathub\t150.5 MB\n\
        runtime/org.freedesktop.Platform/x86_64/26.08\tfreedesktop platform\t\t26.08\tflathub\t281.7 MB\n";

    #[test]
    fn remote_updates_take_their_names_and_versions_from_what_is_installed() {
        let u = parse_updates(UPDATES, Install::System, &parse_list(LIST));
        assert_eq!(u.len(), 3);
        assert_eq!(u[0].origin, Origin::App(Install::System));
        assert_eq!(u[0].id, "app/org.example.Editor/x86_64/stable");
        assert_eq!(u[0].name, "Editor");
        assert_eq!(u[0].version_cell(), "2.1.0 → 2.2.0");
        assert_eq!(u[0].summary, "from flathub, 1.2 MB to download");
        assert_eq!(u[1].origin, Origin::Runtime(Install::System));
        assert_eq!(u[1].name, "Mesa (Extra)");
        assert_eq!(u[2].version_cell(), "26.08");
        assert_eq!(u[2].name, "Freedesktop Platform");
    }

    #[test]
    fn a_ref_not_listed_as_installed_goes_by_its_id() {
        let u = parse_updates(
            "app/org.example.Tool/x86_64/master\t\t\tmaster\ttest\t\n",
            Install::User,
            &HashMap::new(),
        );
        assert_eq!(u[0].name, "org.example.Tool");
        assert_eq!(u[0].version, "master");
        assert_eq!(u[0].summary, "from test");
    }

    #[test]
    fn update_lines_name_the_ref_they_start() {
        assert_eq!(
            started("Updating app/org.example.Editor/x86_64/stable"),
            Some("app/org.example.Editor/x86_64/stable")
        );
        assert_eq!(
            started("Installing runtime/org.example.Ext/x86_64/1"),
            Some("runtime/org.example.Ext/x86_64/1")
        );
        assert_eq!(started("Updating appstream data"), None);
        assert_eq!(started("Nothing to do."), None);
    }

    #[test]
    fn a_failure_says_its_last_line() {
        assert_eq!(
            complaint("Looking for updates…\nerror: Unable to load summary from remote flathub\n"),
            "Unable to load summary from remote flathub."
        );
        assert_eq!(complaint(""), "Flatpak failed.");
    }
}
