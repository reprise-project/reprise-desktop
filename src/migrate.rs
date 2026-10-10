//! One-time startup move of `[display]` from the state file into `config.toml`. A stored value
//! replaces the config's own.

use std::path::Path;

use toml_edit::{DocumentMut, Item, Table, TableLike};

pub fn display_to_config(config: &str, state: &str) -> Result<Option<(String, String)>, String> {
    let mut st: DocumentMut = state.parse().map_err(|e| format!("state: {e}"))?;
    let Some(Item::Table(display)) = st.remove("display") else {
        return Ok(None);
    };
    let mut cfg: DocumentMut = config.parse().map_err(|e| format!("config: {e}"))?;
    let moves: [(&str, Option<&str>, &str); 5] = [
        ("wallpaper", Some("wallpaper"), "image"),
        ("wallpaper_position", Some("wallpaper"), "position"),
        ("background", Some("colors"), "background"),
        ("color_scheme", Some("appearance"), "color_scheme"),
        ("system_icons", None, "system_icons"),
    ];
    for (from, table, key) in moves {
        let Some(v) = display.get(from).and_then(Item::as_value) else {
            continue;
        };
        if from == "color_scheme" && v.as_str() == Some("Glass") {
            continue;
        }
        let at: &mut dyn TableLike = match table {
            None => cfg.as_table_mut(),
            Some(name) => {
                let t = cfg.entry(name).or_insert_with(|| Item::Table(Table::new()));
                t.as_table_like_mut()
                    .ok_or_else(|| format!("config: {name} is not a table"))?
            }
        };
        let mut v = v.clone();
        v.decor_mut().clear();
        at.insert(key, Item::Value(v));
    }
    Ok(Some((cfg.to_string(), st.to_string())))
}

pub fn run(config_path: &Path, state_path: &Path) {
    let Ok(state) = std::fs::read_to_string(state_path) else {
        return;
    };
    let config = std::fs::read_to_string(config_path).unwrap_or_default();
    let rest = match display_to_config(&config, &state) {
        Ok(None) => return,
        Ok(Some((_, rest))) => rest,
        Err(e) => {
            log::error!("migrating [display] to {}: {e}", config_path.display());
            return;
        }
    };
    // Config first: a crash between the two writes leaves [display] in the
    // state. The next start copies the same values over the same keys,
    // which changes nothing, and then drops the table.
    let moved = repriseconf::edit(config_path, |text| {
        Ok::<_, String>(match display_to_config(text, &state)? {
            Some((config, _)) => config,
            None => text.to_string(),
        })
    });
    match moved {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return log::error!("migrating [display] to {}: {e}", config_path.display()),
        Err(e) => return log::error!("migrating [display] to {}: {e}", config_path.display()),
    }
    if let Err(e) = repriseconf::write_atomic(state_path, &rest) {
        log::error!("writing {}: {e}", state_path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: &str = "run_history = [\"foot\"]\n\n[display]\nwallpaper = \"/w/a.jpg\"\nwallpaper_position = \"tile\"\nbackground = \"#102030\"\ncolor_scheme = \"Brick\"\nfont_size = \"large\"\nsystem_icons = [\"recycle-bin\"]\n";

    #[test]
    fn display_keys_move_into_the_config() {
        let (config, state) = display_to_config("terminal = \"foot\"\n", STATE)
            .unwrap()
            .unwrap();
        let c = crate::config::Config::parse(&config).unwrap();
        assert_eq!(
            c.wallpaper.image.as_deref(),
            Some(std::path::Path::new("/w/a.jpg"))
        );
        assert_eq!(c.wallpaper.position, crate::wallpaper::Position::Tile);
        assert_eq!(c.color_scheme.as_deref(), Some("Brick"));
        assert_eq!(c.system_icons, vec!["recycle-bin".to_string()]);
        assert!(config.contains("background = \"#102030\""));
        assert!(
            config.starts_with("terminal = \"foot\"\n"),
            "the file's own lines stay first"
        );
        assert!(!state.contains("[display]"));
        assert!(state.contains("run_history"), "the rest of the state stays");
    }

    #[test]
    fn the_stored_value_replaces_the_configs() {
        let (config, _) = display_to_config("[wallpaper]\nimage = \"/mine.png\"\n", STATE)
            .unwrap()
            .unwrap();
        assert!(config.contains("/w/a.jpg"));
        assert!(!config.contains("/mine.png"));
    }

    #[test]
    fn inline_tables_in_the_config_take_the_keys() {
        let cfg =
            "colors = { background = \"#000000\" }\nappearance = { interface = \"classic\" }\n";
        let (config, _) = display_to_config(cfg, STATE).unwrap().unwrap();
        let c = crate::config::Config::parse(&config).unwrap();
        assert_eq!(c.color_scheme.as_deref(), Some("Brick"));
        assert!(config.contains("#102030"));
        assert!(!config.contains("#000000"));
        assert!(config.contains("interface = \"classic\""));
    }

    #[test]
    fn glass_is_not_a_scheme_to_write() {
        let state = "[display]\ncolor_scheme = \"Glass\"\n";
        let (config, _) = display_to_config("", state).unwrap().unwrap();
        assert!(!config.contains("color_scheme"));
    }

    #[test]
    fn no_display_table_is_nothing_to_do() {
        assert_eq!(
            display_to_config("terminal = \"foot\"\n", "run_history = []\n"),
            Ok(None)
        );
    }

    #[test]
    fn migration_is_idempotent() {
        let (config, state) = display_to_config("", STATE).unwrap().unwrap();
        assert_eq!(display_to_config(&config, &state), Ok(None));
        let (again, _) = display_to_config(&config, STATE).unwrap().unwrap();
        assert_eq!(again, config);
    }

    #[test]
    fn text_that_is_not_toml_is_an_error() {
        assert!(display_to_config("= nope", STATE).is_err());
        assert!(display_to_config("", "[display\n").is_err());
    }

    #[test]
    fn system_icons_land_at_the_top_level_of_a_config_ending_in_a_table() {
        let (config, _) = display_to_config("[idle]\nlock_after = 60\n", STATE)
            .unwrap()
            .unwrap();
        let c = crate::config::Config::parse(&config).unwrap();
        assert_eq!(c.system_icons, vec!["recycle-bin".to_string()]);
        assert_eq!(c.idle.lock_after, 60);
    }
}
