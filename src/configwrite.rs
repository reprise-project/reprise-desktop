//! What the compositor writes back into `config.toml`: switches the desktop
//! flips (StickyKeys), the system icons after one is removed, and the
//! taskbar's edge and width after a drag. The rest of the file stays
//! byte-identical (`toml_edit` via `repriseconf`). The config watch reloads
//! the write; for a setting already live it finds nothing to apply.

use std::path::Path;
use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};
use toml_edit::{Array, DocumentMut, Value, value};

/// `text` with `[table] key = v`, the rest untouched. Err when `text` is not TOML or `table` is not a table.
pub fn set_value(text: &str, key: repriseconf::Key, v: Value) -> Result<String, String> {
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    repriseconf::set(&mut doc, &key, v).map_err(|e| e.to_string())?;
    Ok(doc.to_string())
}

#[cfg(test)]
pub fn set_flag(text: &str, key: repriseconf::Key, on: bool) -> Result<String, String> {
    set_value(text, key, Value::from(on))
}

type Save = Box<dyn FnOnce() + Send>;

/// The one thread that writes saves, in queue order. None when it could not be spawned.
static WRITER: OnceLock<Option<Sender<Save>>> = OnceLock::new();

/// Queue `set_value` on `path` for the writer thread, under `repriseconf::edit`'s lock
/// (an absent file is an empty one). The lock and the fsync block, so not on the event loop.
/// Failures are logged; the live config already has the setting.
pub fn save_value(path: &Path, key: repriseconf::Key, v: Value) {
    let path = path.to_path_buf();
    queue(Box::new(move || {
        let shown = v.to_string();
        let saved = repriseconf::edit(&path, |text| set_value(text, key, v))
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        let name = format!("{}.{}", key.table, key.name);
        match saved {
            Ok(_) => log::info!("config {}: saved {name} = {shown}", path.display()),
            Err(e) => log::error!("config {}: {e}; {name} not saved", path.display()),
        }
    }));
}

pub fn save_flag(path: &Path, key: repriseconf::Key, on: bool) {
    save_value(path, key, Value::from(on));
}

/// `text` with the top-level `system_icons` set to `icons`, placed before the first table. Err when `text` is not TOML.
pub fn set_system_icons(text: &str, icons: &[String]) -> Result<String, String> {
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let mut new = value(icons.iter().map(String::as_str).collect::<Array>());
    // Keep the old value's spacing and trailing comment.
    if let (Some(old), Some(v)) = (
        doc.get("system_icons").and_then(|i| i.as_value()),
        new.as_value_mut(),
    ) {
        *v.decor_mut() = old.decor().clone();
    }
    doc["system_icons"] = new;
    Ok(doc.to_string())
}

pub fn save_system_icons(path: &Path, icons: Vec<String>) {
    let path = path.to_path_buf();
    queue(Box::new(move || {
        let saved = repriseconf::edit(&path, |text| set_system_icons(text, &icons))
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        match saved {
            Ok(_) => log::info!("config {}: saved system_icons", path.display()),
            Err(e) => log::error!("config {}: {e}; system_icons not saved", path.display()),
        }
    }));
}

/// `text` with `[recycle_bin]` set to `policy`; with `replace` off, an existing table is left as is.
pub fn set_recycle_bin(
    text: &str,
    policy: &reprisefs::bincap::Policy,
    replace: bool,
) -> Result<String, String> {
    let has = text
        .parse::<DocumentMut>()
        .is_ok_and(|d| d.contains_key("recycle_bin"));
    match has && !replace {
        true => Ok(text.to_string()),
        false => policy.write(text),
    }
}

/// Queue `set_recycle_bin` on `path`: Recycle Bin Properties' OK and Apply (`replace`),
/// and moving an older session's setting out of the state file (not `replace`).
pub fn save_recycle_bin(path: &Path, policy: reprisefs::bincap::Policy, replace: bool) {
    let path = path.to_path_buf();
    queue(Box::new(move || {
        let saved = repriseconf::edit(&path, |text| set_recycle_bin(text, &policy, replace))
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        match saved {
            Ok(_) => log::info!("config {}: saved recycle_bin", path.display()),
            Err(e) => log::error!("config {}: {e}; recycle_bin not saved", path.display()),
        }
    }));
}

fn queue(save: Save) {
    let writer = WRITER.get_or_init(|| {
        let (tx, rx) = channel::<Save>();
        let spawned = std::thread::Builder::new()
            .name("configwrite".into())
            .spawn(move || {
                for save in rx {
                    save();
                }
            });
        match spawned {
            Ok(_) => Some(tx),
            Err(e) => {
                log::warn!("configwrite: cannot spawn a thread ({e}); saves run on the loop");
                None
            }
        }
    });
    match writer {
        // The send fails only once the writer has died; then the loop writes.
        Some(tx) => {
            if let Err(e) = tx.send(save) {
                (e.0)();
            }
        }
        None => save(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn a_drag_writes_position_and_side_width_as_the_live_config_has_them() {
        let text = "# mine\n[taskbar]\nbuttons = \"never\"\n";
        let position = repriseconf::key("taskbar", "position");
        let side_width = repriseconf::key("taskbar", "side_width");
        let out = set_value(text, position, Value::from("left")).unwrap();
        let out = set_value(&out, side_width, Value::from(150i64)).unwrap();
        assert!(out.starts_with(text), "{out}");
        assert_eq!(out.matches("position").count(), 1, "{out}");
        let mut live = Config::parse(text).unwrap();
        live.taskbar.position = reprisekit::taskbar::Edge::Left;
        live.taskbar.side_width = 150;
        // The reload the write brings finds what the drag applied.
        let parsed = Config::parse(&out).unwrap();
        assert_eq!(parsed, live);
        assert_eq!(
            crate::configwatch::diff(&live, &parsed),
            crate::configwatch::Changes::default()
        );
        let again = set_value(&out, position, Value::from("top")).unwrap();
        assert_eq!(again.matches("position").count(), 1, "{again}");
        assert_eq!(
            Config::parse(&again).unwrap().taskbar.position,
            reprisekit::taskbar::Edge::Top
        );
    }

    /// The sheet's settings land in `[recycle_bin]`; an older session's setting moves in only while the table is absent.
    #[test]
    fn the_recycle_bin_table_is_written_and_parses() {
        use reprisefs::bincap::Policy;
        let text = "# mine\nterminal = \"foot\"\n";
        let mut p = Policy::default();
        p.global.max_percent = 1;
        let out = set_recycle_bin(text, &p, false).unwrap();
        assert!(out.starts_with(text), "{out}");
        assert_eq!(Policy::parse(&out), p);
        Config::parse(&out).unwrap();
        let other = Policy {
            confirm: false,
            ..Policy::default()
        };
        assert_eq!(set_recycle_bin(&out, &other, false).unwrap(), out);
        let replaced = set_recycle_bin(&out, &other, true).unwrap();
        assert_eq!(Policy::parse(&replaced), other);
    }

    #[test]
    fn a_flag_is_set_in_its_table_and_the_rest_is_kept() {
        let key = repriseconf::key("accessibility", "sticky_keys");
        let text = "# mine\nterminal = \"foot\"\n";
        let out = set_flag(text, key, true).unwrap();
        assert!(out.starts_with(text), "{out}");
        let c = Config::parse(&out).unwrap();
        assert!(c.accessibility.sticky_keys);
        let out = set_flag(&out, key, false).unwrap();
        assert!(!Config::parse(&out).unwrap().accessibility.sticky_keys);
        assert_eq!(out.matches("sticky_keys").count(), 1, "{out}");
        assert!(set_flag("accessibility = 1\n", key, true).is_err());
    }

    #[test]
    fn saves_return_while_another_editor_holds_the_lock_and_land_in_order() {
        use std::os::fd::AsRawFd;
        use std::time::{Duration, Instant};
        let dir = std::env::temp_dir().join(format!("reprise-configwrite-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[idle]\nlock_after = 0\n").unwrap();
        let lock = std::fs::File::create(dir.join(".config.toml.lock")).unwrap();
        // SAFETY: a valid fd from the open file above.
        assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);

        let key = repriseconf::key("accessibility", "sticky_keys");
        let (tx, rx) = std::sync::mpsc::channel();
        let p = path.clone();
        std::thread::spawn(move || {
            for on in [true, false] {
                save_flag(&p, key, on);
            }
            let _ = tx.send(());
        });
        assert!(
            rx.recv_timeout(Duration::from_secs(2)).is_ok(),
            "save_flag returns within 2 s while the lock is held"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[idle]\nlock_after = 0\n"
        );

        drop(lock);
        let on = set_flag("[idle]\nlock_after = 0\n", key, true).unwrap();
        let want = set_flag(&on, key, false).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while std::fs::read_to_string(&path).unwrap() != want && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), want);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(dir.join(".config.toml.lock"));
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn system_icons_are_written_top_level() {
        let out = set_system_icons(
            "terminal = \"foot\"\n\n[idle]\nlock_after = 60\n",
            &["recycle-bin".into()],
        )
        .unwrap();
        let c = Config::parse(&out).unwrap();
        assert_eq!(c.system_icons, vec!["recycle-bin".to_string()]);
        assert_eq!(c.idle.lock_after, 60);
    }

    #[test]
    fn a_trailing_comment_on_system_icons_stays() {
        let out = set_system_icons(
            "system_icons = [\"recycle-bin\", \"computer\"] # desktop\n",
            &["recycle-bin".into()],
        )
        .unwrap();
        assert_eq!(out, "system_icons = [\"recycle-bin\"] # desktop\n");
    }
}
