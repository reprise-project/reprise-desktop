//! `org.freedesktop.impl.portal.Background`: which apps run with and
//! without a window, the "running in the background" prompt, and the
//! autostart entries of `RequestBackground`.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use zbus::zvariant::{ObjectPath, OwnedValue};

const STATE_BACKGROUND: u32 = 0;
const STATE_RUNNING: u32 = 1;

const RESULT_FORBID: u32 = 0;
const RESULT_ALLOW: u32 = 1;
const RESULT_ALLOW_ONCE: u32 = 2;

const AUTOSTART_DBUS: u32 = 1;

pub struct Background {
    pub frontend: crate::Frontend,
    pub link: Option<crate::inputlink::Link>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Background")]
impl Background {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    async fn get_app_state(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        self.frontend.check(&hdr).await?;
        // With either list missing, every app with a window would read as in the background, and
        // xdg-desktop-portal kills a background app the user forbade; an error skips the round.
        let sandboxed = (self.link.as_ref())
            .and_then(crate::inputlink::Link::sandboxed)
            .ok_or_else(|| zbus::fdo::Error::Failed("no list of sandboxed windows".into()))?;
        let states = blocking::unblock(move || {
            let wl = crate::Wayland::connect().map_err(|e| {
                log::warn!("GetAppState: {e}");
                zbus::fdo::Error::Failed(e)
            })?;
            let windowed: Vec<String> = wl.toplevels().into_iter().map(|t| t.app_id).collect();
            Ok::<_, zbus::fdo::Error>(app_states(&running_flatpaks(), &windowed, &sandboxed))
        })
        .await?;
        Ok(states
            .into_iter()
            .map(|(id, s)| (id, OwnedValue::from(s)))
            .collect())
    }

    async fn notify_background(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        name: String,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        self.frontend.check(&hdr).await?;
        let cancel = Arc::new(AtomicBool::new(false));
        crate::export_request(server, &handle, cancel.clone(), &self.frontend).await?;
        let id = app_id.clone();
        let answer = blocking::unblock(move || {
            let app = if name.trim().is_empty() {
                crate::remotedesktop::display_name(&id)
            } else {
                name
            };
            let input = crate::permit::Input {
                parent: None,
                app,
                offer_remember: true,
                items: Vec::new(),
                preview: None,
            };
            crate::permit::ask_with(crate::permit::Kind::Background, input, &cancel)
        })
        .await;
        let _ = server.remove::<crate::Request, _>(handle.clone()).await;
        let result = background_result(answer);
        log::info!("NotifyBackground for {app_id:?}: {result}");
        let mut r = HashMap::new();
        r.insert("result".to_string(), OwnedValue::from(result));
        Ok((crate::RESPONSE_OK, r))
    }

    async fn enable_autostart(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        app_id: String,
        enable: bool,
        commandline: Vec<String>,
        flags: u32,
    ) -> zbus::fdo::Result<bool> {
        self.frontend.check(&hdr).await?;
        let dir = repriseconf::xdg_home("XDG_CONFIG_HOME", ".config").join("autostart");
        let id = app_id.clone();
        let done = blocking::unblock(move || {
            set_autostart(&dir, &id, enable, &commandline, flags & AUTOSTART_DBUS != 0)
        })
        .await;
        match done {
            Ok(on) => Ok(on),
            Err(e) => {
                log::warn!("EnableAutostart for {app_id:?}: {e}");
                Ok(false)
            }
        }
    }
}

/// The frontend's answer for the prompt: Allow with Remember allows for
/// good, Deny forbids, and Allow alone or no answer allows this run.
fn background_result(answer: crate::permit::Answer) -> u32 {
    use crate::permit::Answer;
    match answer {
        Answer::Allow { remember: true } => RESULT_ALLOW,
        Answer::Deny => RESULT_FORBID,
        Answer::Allow { remember: false } | Answer::Dismissed => RESULT_ALLOW_ONCE,
    }
}

/// Every app id with a window as running, and every other app in
/// `running` as in the background. `windowed` holds the windows' own app
/// ids, `sandboxed` the sandbox app ids of the clients with a window.
fn app_states(
    running: &[String],
    windowed: &[String],
    sandboxed: &BTreeSet<String>,
) -> HashMap<String, u32> {
    let mut out: HashMap<String, u32> = running
        .iter()
        .filter(|id| !id.is_empty())
        .map(|id| (id.clone(), STATE_BACKGROUND))
        .collect();
    for id in windowed.iter().chain(sandboxed) {
        if !id.is_empty() && !crate::HIDDEN_APPS.contains(&id.as_str()) {
            out.insert(id.clone(), STATE_RUNNING);
        }
    }
    out
}

/// The app ids of the live Flatpak instances under
/// `$XDG_RUNTIME_DIR/.flatpak`.
fn running_flatpaks() -> Vec<String> {
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return Vec::new();
    };
    let Ok(dirs) = std::fs::read_dir(Path::new(&runtime).join(".flatpak")) else {
        return Vec::new();
    };
    let ids: BTreeSet<String> = dirs
        .flatten()
        .filter_map(|d| instance_app(&d.path()))
        .collect();
    ids.into_iter().collect()
}

/// The app id of the instance in `dir`, if its process still runs.
fn instance_app(dir: &Path) -> Option<String> {
    let pid = std::fs::read_to_string(dir.join("pid")).ok()?;
    let pid: u32 = pid.trim().parse().ok()?;
    if !Path::new(&format!("/proc/{pid}")).exists() {
        return None;
    }
    reprisefs::flatpak::app_of_info(&std::fs::read_to_string(dir.join("info")).ok()?)
}

/// Write or remove `<dir>/<app_id>.desktop`; Ok(true) when an entry is
/// now in place.
fn set_autostart(
    dir: &Path,
    app_id: &str,
    enable: bool,
    commandline: &[String],
    dbus: bool,
) -> Result<bool, String> {
    if !is_app_id(app_id) {
        return Err("not a Flatpak app id".into());
    }
    let path: PathBuf = dir.join(format!("{app_id}.desktop"));
    if !enable {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.to_string()),
        };
    }
    let text = autostart_entry(app_id, commandline, dbus)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    reprisefs::ops::write_atomic(&path, text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(true)
}

/// A Flatpak app id: dot-separated names of letters, digits, `_` and `-`,
/// so it is also a safe file name.
fn is_app_id(id: &str) -> bool {
    id.len() <= 255
        && id.contains('.')
        && id.split('.').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

/// The autostart entry that runs `commandline` inside `app_id`'s sandbox.
/// Err when an argument holds a control character, which no Exec line
/// carries.
fn autostart_entry(app_id: &str, commandline: &[String], dbus: bool) -> Result<String, String> {
    if commandline.iter().any(|a| a.chars().any(char::is_control)) {
        return Err("a control character in the command line".into());
    }
    let mut exec = vec!["flatpak".to_string(), "run".to_string()];
    if let Some((command, args)) = commandline.split_first() {
        exec.push(format!("--command={command}"));
        exec.push(app_id.to_string());
        exec.extend(args.iter().cloned());
    } else {
        exec.push(app_id.to_string());
    }
    let exec: Vec<String> = exec.iter().map(|a| exec_arg(a)).collect();
    let mut text = format!(
        "[Desktop Entry]\nType=Application\nName={app_id}\nExec={}\nX-Flatpak={app_id}\n",
        exec.join(" ")
    );
    if dbus {
        text.push_str("DBusActivatable=true\n");
    }
    Ok(text)
}

/// One Exec argument as the desktop entry spec quotes it, then escaped
/// for a string value: `%` doubled (field codes) and `\` doubled.
fn exec_arg(arg: &str) -> String {
    const RESERVED: &str = " \t\"'\\><~|&;$*?#()`";
    let quoted = if arg.is_empty() || arg.chars().any(|c| RESERVED.contains(c)) {
        let mut q = String::from("\"");
        for c in arg.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                q.push('\\');
            }
            q.push(c);
        }
        q.push('"');
        q
    } else {
        arg.to_string()
    };
    quoted.replace('\\', "\\\\").replace('%', "%%")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn an_app_with_a_window_runs_and_one_without_is_in_the_background() {
        let states = app_states(
            &strings(&["org.example.Chat", "org.example.Sync", ""]),
            &strings(&["org.example.Chat", "foot", "reprise.portal", ""]),
            &BTreeSet::new(),
        );
        assert_eq!(states.len(), 3, "{states:?}");
        assert_eq!(states["org.example.Chat"], STATE_RUNNING);
        assert_eq!(states["foot"], STATE_RUNNING);
        assert_eq!(states["org.example.Sync"], STATE_BACKGROUND);
    }

    #[test]
    fn an_app_whose_sandbox_has_a_window_runs_whatever_the_window_calls_itself() {
        let states = app_states(
            &strings(&["com.valvesoftware.Steam", "org.example.Sync"]),
            &strings(&["steam"]),
            &BTreeSet::from(["com.valvesoftware.Steam".to_string()]),
        );
        assert_eq!(states["com.valvesoftware.Steam"], STATE_RUNNING);
        assert_eq!(states["org.example.Sync"], STATE_BACKGROUND);
    }

    #[test]
    fn the_autostart_entry_runs_the_command_in_the_apps_sandbox() {
        let text = autostart_entry(
            "org.example.Sync",
            &strings(&["sync-app", "--background", "a b", "50%"]),
            false,
        )
        .unwrap();
        assert_eq!(
            text,
            "[Desktop Entry]\nType=Application\nName=org.example.Sync\n\
             Exec=flatpak run --command=sync-app org.example.Sync --background \"a b\" 50%%\n\
             X-Flatpak=org.example.Sync\n"
        );
        let bare = autostart_entry("org.example.Sync", &[], true).unwrap();
        assert!(
            bare.contains("\nExec=flatpak run org.example.Sync\n"),
            "{bare}"
        );
        assert!(bare.ends_with("DBusActivatable=true\n"), "{bare}");
    }

    #[test]
    fn exec_arguments_are_quoted_and_escaped_for_the_entry() {
        assert_eq!(exec_arg("plain"), "plain");
        assert_eq!(exec_arg(""), "\"\"");
        assert_eq!(exec_arg("$HOME"), "\"\\\\$HOME\"");
        assert_eq!(exec_arg("a\\b"), "\"a\\\\\\\\b\"");
        assert_eq!(exec_arg("say \"hi\""), "\"say \\\\\"hi\\\\\"\"");
        assert!(autostart_entry("org.example.A", &strings(&["a\nb"]), false).is_err());
    }

    #[test]
    fn autostart_writes_and_removes_only_an_app_ids_entry() {
        let dir = tempfile::tempdir().unwrap();
        let auto = dir.path().join("autostart");
        let cmd = strings(&["app"]);
        assert_eq!(
            set_autostart(&auto, "org.example.A", true, &cmd, false),
            Ok(true)
        );
        let file = auto.join("org.example.A.desktop");
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .contains("X-Flatpak=org.example.A")
        );
        assert_eq!(
            set_autostart(&auto, "org.example.A", false, &cmd, false),
            Ok(false)
        );
        assert!(!file.exists());
        assert_eq!(
            set_autostart(&auto, "org.example.A", false, &cmd, false),
            Ok(false)
        );
        for bad in ["../evil", "noperiod", "org..x", "org.x/y", ""] {
            assert!(
                set_autostart(&auto, bad, true, &cmd, false).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn allow_with_remember_allows_for_good_and_no_answer_allows_once() {
        use crate::permit::Answer;
        assert_eq!(
            background_result(Answer::Allow { remember: true }),
            RESULT_ALLOW
        );
        assert_eq!(
            background_result(Answer::Allow { remember: false }),
            RESULT_ALLOW_ONCE
        );
        assert_eq!(background_result(Answer::Dismissed), RESULT_ALLOW_ONCE);
        assert_eq!(background_result(Answer::Deny), RESULT_FORBID);
    }
}
