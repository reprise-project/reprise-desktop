//! `[toolkits] gtk4_colors = true`: writes the stylesheet
//! `reprisekit::toolkit::gtk4` generates to `$XDG_CONFIG_HOME/gtk-4.0/gtk.css`.
//! GTK4 reads it once at program start and doesn't watch it, so running
//! programs keep their colours.
//!
//! The file is the user's: Reprise writes it only when it is absent or starts
//! with `toolkit::gtk4::MARKER`, and removes it only when it carries the marker.

use crate::config::Config;
use reprisekit::toolkit::gtk4::{MARKER, css};
use std::io::ErrorKind;
use std::path::Path;

#[derive(Debug, Default)]
pub struct Gtk4 {
    refused: bool,
}

/// Write, rewrite or remove `config_home/gtk-4.0/gtk.css` per `cfg`; writes only on change.
pub fn update(state: &mut Gtk4, cfg: &Config, config_home: &Path) {
    let path = config_home.join("gtk-4.0/gtk.css");
    let current = std::fs::read_to_string(&path);
    let ours = matches!(&current, Ok(t) if t.starts_with(MARKER));
    if !cfg.toolkits.gtk4_colors {
        state.refused = false;
        if ours && let Err(e) = std::fs::remove_file(&path) {
            log::warn!(
                "{}: {e}; GTK4 programs keep the look's colours",
                path.display()
            );
        }
        return;
    }
    let text = css(&cfg.theme.colors, cfg.interface);
    match current {
        Ok(t) if t == text => {}
        Err(e) if e.kind() == ErrorKind::NotFound => write(state, &path, &text),
        _ if ours => write(state, &path, &text),
        _ => {
            if !state.refused {
                log::warn!(
                    "[toolkits] gtk4_colors: {} is not Reprise's; left as it is",
                    path.display()
                );
                state.refused = true;
            }
        }
    }
}

fn write(state: &mut Gtk4, path: &Path, text: &str) {
    match repriseconf::write_atomic(path, text) {
        Ok(()) => state.refused = false,
        Err(e) => log::warn!("{}: {e}; GTK4 programs keep their colours", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let p =
                std::env::temp_dir().join(format!("reprise-gtk4-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            Scratch(p)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn on() -> Config {
        Config::parse("[toolkits]\ngtk4_colors = true\n").unwrap()
    }

    #[test]
    fn writes_follows_the_look_and_goes_when_off() {
        let tmp = Scratch::new("cycle");
        let file = tmp.0.join("gtk-4.0/gtk.css");
        let mut s = Gtk4::default();
        let mut cfg = on();
        update(&mut s, &cfg, &tmp.0);
        let first = std::fs::read_to_string(&file).unwrap();
        assert_eq!(first, css(&cfg.theme.colors, cfg.interface));
        assert!(first.starts_with(MARKER));
        cfg.theme.colors = reprisekit::schemes::scheme("Tokyo Night").unwrap();
        update(&mut s, &cfg, &tmp.0);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            css(&cfg.theme.colors, cfg.interface)
        );
        update(&mut s, &Config::default(), &tmp.0);
        assert!(!file.exists());
    }

    #[test]
    fn the_same_look_writes_nothing() {
        let tmp = Scratch::new("same");
        let file = tmp.0.join("gtk-4.0/gtk.css");
        let mut s = Gtk4::default();
        update(&mut s, &on(), &tmp.0);
        let old = std::time::SystemTime::UNIX_EPOCH;
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(old)
            .unwrap();
        update(&mut s, &on(), &tmp.0);
        assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), old);
    }

    #[test]
    fn a_users_own_file_stays() {
        let tmp = Scratch::new("own");
        let file = tmp.0.join("gtk-4.0/gtk.css");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let own = "window { background: pink; }\n";
        std::fs::write(&file, own).unwrap();
        let mut s = Gtk4::default();
        update(&mut s, &on(), &tmp.0);
        assert!(s.refused);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), own);
        update(&mut s, &Config::default(), &tmp.0);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), own);
    }

    #[test]
    fn off_writes_nothing() {
        let tmp = Scratch::new("off");
        update(&mut Gtk4::default(), &Config::default(), &tmp.0);
        assert!(!tmp.0.exists());
    }
}
