//! `[toolkits]`: writes the generated GTK3 theme and Qt palette at start and on every retheme.
//!
//! GTK3 reloads its theme only when `gtk-theme` names another one, so a new look goes into the
//! [`GTK_SLOTS`] directory GTK is not reading and the portal then names that one. A Qt tool's conf
//! is written only when absent or carrying `toolkit::QT_CONF_MARKER`; a user's own is left alone.

use crate::config::{Config, TOOLKIT_REPRISE};
use reprisekit::toolkit;
use std::path::{Path, PathBuf};

mod gtk4;

/// The two GTK theme directories the CSS alternates between.
pub const GTK_SLOTS: [&str; 2] = ["Reprise", "Reprise-alt"];

/// In the order `QT_QPA_PLATFORMTHEME` prefers them.
const QT_TOOLS: [&str; 2] = ["qt6ct", "qt5ct"];

/// What has been written this session.
#[derive(Debug, Default)]
pub struct Generated {
    gtk: Option<usize>,
    gtk_css: String,
    qt: Option<&'static str>,
    gtk4: gtk4::Gtk4,
}

impl Generated {
    /// The generated GTK theme's name, once written.
    pub fn gtk_theme(&self) -> Option<&'static str> {
        self.gtk.map(|i| GTK_SLOTS[i])
    }

    /// The Qt platform theme reading the generated palette, if any.
    pub fn qt(&self) -> Option<&'static str> {
        self.qt
    }

    fn gtk_slot_for(&self, css: &str) -> Option<usize> {
        match self.gtk {
            Some(_) if self.gtk_css == css => None,
            Some(i) => Some(1 - i),
            None => Some(0),
        }
    }
}

/// Where the files go: `$XDG_DATA_HOME` and `$XDG_CONFIG_HOME`, and the Qt tools found on `PATH`.
pub struct Dirs {
    pub data_home: PathBuf,
    pub config_home: PathBuf,
    pub qt_tools: Vec<&'static str>,
}

impl Dirs {
    pub fn from_env() -> Dirs {
        Dirs {
            data_home: repriseconf::xdg_home("XDG_DATA_HOME", ".local/share"),
            config_home: repriseconf::xdg_home("XDG_CONFIG_HOME", ".config"),
            qt_tools: qt_tools_installed(),
        }
    }
}

/// The Qt tools with an executable on `PATH`.
pub fn qt_tools_installed() -> Vec<&'static str> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    QT_TOOLS
        .into_iter()
        .filter(|t| std::env::split_paths(&path).any(|d| d.join(t).is_file()))
        .collect()
}

/// Write what `state`'s `[toolkits]` keys ask for, and hand the launcher the Qt tool that took the
/// palette. Runs at start, before the Settings portal first answers, and on every retheme.
pub fn refresh(state: &mut crate::compositor::State) {
    update(&mut state.toolkits, &state.config, &Dirs::from_env());
    state.env.qt_generated = state.toolkits.qt();
}

/// Write the generated theme and palette for `cfg`'s look, as its `[toolkits]` keys ask.
pub fn update(generated: &mut Generated, cfg: &Config, dirs: &Dirs) {
    gtk4::update(&mut generated.gtk4, cfg, &dirs.config_home);
    let colors = &cfg.theme.colors;
    if cfg.toolkits.gtk_theme == TOOLKIT_REPRISE {
        let css = toolkit::gtk3_css(colors, cfg.interface);
        if let Some(slot) = generated.gtk_slot_for(&css) {
            let dir = dirs
                .data_home
                .join("themes")
                .join(GTK_SLOTS[slot])
                .join("gtk-3.0");
            // The pictures first: GTK reads them once the CSS names them.
            let written = toolkit::gtk3_assets(colors, cfg.interface)
                .iter()
                .try_for_each(|(rel, bytes)| write_bytes_if_changed(&dir.join(rel), bytes))
                .and_then(|()| write_if_changed(&dir.join("gtk.css"), &css));
            match written {
                Ok(_) => {
                    generated.gtk = Some(slot);
                    generated.gtk_css = css;
                }
                Err(e) => log::warn!("{}: {e}; GTK keeps its theme", dir.display()),
            }
        }
    }
    generated.qt = None;
    if cfg.toolkits.qt_platform_theme == TOOLKIT_REPRISE {
        if dirs.qt_tools.is_empty() {
            log::warn!(
                "[toolkits] qt_platform_theme = \"reprise\" needs qt6ct or qt5ct; Qt takes \"auto\""
            );
        }
        let palette = toolkit::qt_colors(colors);
        for tool in &dirs.qt_tools {
            match write_qt(
                &dirs.config_home.join(tool),
                tool,
                &palette,
                &cfg.icons.theme,
                (cfg.accessibility.text_size != 100).then_some(&cfg.theme.fonts),
            ) {
                Ok(()) => {
                    generated.qt.get_or_insert(tool);
                }
                Err(e) => log::warn!("{tool}: {e}; Qt programs it would theme take \"auto\""),
            }
        }
    }
}

/// Write `palette` to `dir/colors/Reprise.conf` and `dir/<tool>.conf` pointing at it, unless that
/// conf is the user's own (no marker). The conf is rewritten whenever the palette changed: the
/// tools watch their directory, not `colors/`, so the rename makes a running program reread it.
fn write_qt(
    dir: &Path,
    tool: &str,
    palette: &str,
    icon_theme: &str,
    font: Option<&reprisekit::theme::FontConfig>,
) -> std::io::Result<()> {
    let conf = dir.join(format!("{tool}.conf"));
    if let Ok(text) = std::fs::read_to_string(&conf)
        && !text.starts_with(toolkit::QT_CONF_MARKER)
    {
        return Err(std::io::Error::other(format!(
            "{} is not Reprise's; left as it is",
            conf.display()
        )));
    }
    let colors = dir.join("colors/Reprise.conf");
    let palette_changed = write_if_changed(&colors, palette)?;
    let text = toolkit::qt_conf(&colors.to_string_lossy(), icon_theme, font);
    if palette_changed || std::fs::read_to_string(&conf).ok().as_deref() != Some(text.as_str()) {
        repriseconf::write_atomic(&conf, &text)?;
    }
    Ok(())
}

/// Write `bytes` to `path`, making its directory, unless it holds them already. Not synced or
/// renamed: a torn picture heals on the next retheme, and syncing forty would hold up the loop.
fn write_bytes_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if std::fs::read(path).is_ok_and(|b| b == bytes) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes)
}

/// Write `text` to `path` atomically unless it holds it already.
fn write_if_changed(path: &Path, text: &str) -> std::io::Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|t| t == text) {
        return Ok(false);
    }
    repriseconf::write_atomic(path, text)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let p = std::env::temp_dir()
                .join(format!("reprise-toolkits-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            Scratch(p)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn dirs(root: &Path, qt: &[&'static str]) -> Dirs {
        Dirs {
            data_home: root.join("data"),
            config_home: root.join("config"),
            qt_tools: qt.to_vec(),
        }
    }

    fn reprise() -> Config {
        Config::parse("[toolkits]\ngtk_theme = \"reprise\"\nqt_platform_theme = \"reprise\"\n")
            .unwrap()
    }

    #[test]
    fn the_gtk_theme_alternates_between_its_two_slots() {
        let tmp = Scratch::new("slots");
        let d = dirs(tmp.path(), &[]);
        let mut g = Generated::default();
        let mut cfg = reprise();
        update(&mut g, &cfg, &d);
        assert_eq!(g.gtk_theme(), Some("Reprise"));
        let first = d.data_home.join("themes/Reprise/gtk-3.0/gtk.css");
        assert_eq!(
            std::fs::read_to_string(&first).unwrap(),
            toolkit::gtk3_css(&cfg.theme.colors, cfg.interface)
        );
        for (rel, bytes) in toolkit::gtk3_assets(&cfg.theme.colors, cfg.interface) {
            let file = d.data_home.join("themes/Reprise/gtk-3.0").join(&rel);
            assert_eq!(std::fs::read(&file).ok(), Some(bytes), "{rel}");
        }
        update(&mut g, &cfg, &d);
        assert_eq!(g.gtk_theme(), Some("Reprise"), "the same look stays put");
        cfg.theme.colors = reprisekit::schemes::scheme("Tokyo Night").unwrap();
        update(&mut g, &cfg, &d);
        assert_eq!(g.gtk_theme(), Some("Reprise-alt"));
        assert!(
            std::fs::read_to_string(d.data_home.join("themes/Reprise-alt/gtk-3.0/gtk.css"))
                .unwrap()
                .contains("gtk-contained-dark.css")
        );
        cfg.theme.colors = reprisekit::schemes::scheme("Brick").unwrap();
        update(&mut g, &cfg, &d);
        assert_eq!(g.gtk_theme(), Some("Reprise"), "and back to the first");
    }

    #[test]
    fn other_values_write_nothing() {
        let tmp = Scratch::new("other");
        let d = dirs(tmp.path(), &["qt6ct"]);
        let mut g = Generated::default();
        update(&mut g, &Config::default(), &d);
        assert_eq!((g.gtk_theme(), g.qt()), (None, None));
        assert!(!tmp.path().join("data").exists());
        assert!(!tmp.path().join("config").exists());
    }

    #[test]
    fn qt_writes_under_the_marker_and_leaves_a_users_conf_alone() {
        let tmp = Scratch::new("qt");
        let d = dirs(tmp.path(), &["qt6ct", "qt5ct"]);
        let mut g = Generated::default();
        let own = "[Appearance]\nstyle=kvantum\n";
        let qt5 = d.config_home.join("qt5ct/qt5ct.conf");
        std::fs::create_dir_all(qt5.parent().unwrap()).unwrap();
        std::fs::write(&qt5, own).unwrap();
        update(&mut g, &reprise(), &d);
        assert_eq!(g.qt(), Some("qt6ct"));
        let colors = d.config_home.join("qt6ct/colors/Reprise.conf");
        assert_eq!(
            std::fs::read_to_string(&colors).unwrap(),
            toolkit::qt_colors(&reprise().theme.colors)
        );
        let conf = std::fs::read_to_string(d.config_home.join("qt6ct/qt6ct.conf")).unwrap();
        assert!(conf.starts_with(toolkit::QT_CONF_MARKER));
        assert!(conf.contains(&format!("color_scheme_path={}\n", colors.display())));
        assert_eq!(std::fs::read_to_string(&qt5).unwrap(), own);
        assert!(!d.config_home.join("qt5ct/colors").exists());

        let only5 = dirs(tmp.path(), &["qt5ct"]);
        update(&mut g, &reprise(), &only5);
        assert_eq!(g.qt(), None, "the user's qt5ct.conf stands");
        update(&mut g, &reprise(), &dirs(tmp.path(), &[]));
        assert_eq!(g.qt(), None, "no tool, no generated theme");
    }
}
