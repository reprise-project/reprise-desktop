//! `[[outputs]]` entries of `config.toml` as Reprise Settings writes them; `toml_edit` leaves the rest of the file as it was. The keys are the ones `FileOutput` in `src/config/file.rs` reads. This crate cannot depend on the compositor, so they change together.

use std::path::Path;

use repriseapp::heads::Transform;
use repriseconf::hz_string;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, TableLike, Value, value};

/// What to record for one output; a None leaves that key as it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputKeys {
    /// Width, height and refresh in mHz; a refresh of 0 is written without its `@`.
    pub mode: Option<(i32, i32, i32)>,
    pub transform: Option<Transform>,
    /// 1 removes the key, since 1 is what a missing one means; a whole number is written as an integer.
    pub scale: Option<f64>,
    pub adaptive_sync: Option<bool>,
    pub position: Option<(i32, i32)>,
}

/// How far from 0 the compositor takes a place (`MAX_POSITION` in `src/outputmgmt.rs`).
const MAX_POSITION: i32 = 1 << 24;

fn mode_text((w, h, mhz): (i32, i32, i32)) -> String {
    if mhz > 0 {
        format!("{w}x{h}@{}", hz_string(mhz))
    } else {
        format!("{w}x{h}")
    }
}

/// Err when `outputs` is not absent, an array of tables or an inline array of tables. Reads only, so callers check before they mutate.
fn check_outputs(doc: &DocumentMut) -> Result<(), String> {
    match doc.get("outputs") {
        None | Some(Item::ArrayOfTables(_)) => Ok(()),
        Some(Item::Value(Value::Array(a))) => {
            if a.iter().all(Value::is_inline_table) {
                Ok(())
            } else {
                Err("`outputs` holds something that is not a table".into())
            }
        }
        Some(_) => Err("`outputs` is not an array of tables".into()),
    }
}

/// The `[[outputs]]` array, an inline `outputs = [{ ... }]` rewritten in block form first; made empty when the file has none. `check_outputs` must have passed.
fn outputs_mut(doc: &mut DocumentMut) -> &mut ArrayOfTables {
    if matches!(doc.get("outputs"), Some(Item::Value(Value::Array(_)))) {
        let Some(Item::Value(Value::Array(inline))) = doc.remove("outputs") else {
            unreachable!("checked above")
        };
        let blocks = inline
            .into_iter()
            .filter_map(|v| match v {
                Value::InlineTable(t) => Some(t.into_table()),
                _ => None,
            })
            .collect();
        doc.insert("outputs", Item::ArrayOfTables(blocks));
    }
    doc.entry("outputs")
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .expect("an array of tables after check_outputs")
}

/// Err for a value the compositor's parser would refuse, which would make it reject the whole file on reload.
fn check_keys(keys: &OutputKeys) -> Result<(), String> {
    if let Some((w, h, mhz)) = keys.mode
        && (w <= 0 || h <= 0 || !(0..=1_000_000).contains(&mhz))
    {
        return Err(format!("mode {w}x{h}@{mhz}mHz is out of range"));
    }
    if let Some(s) = keys.scale
        && !(1.0..=4.0).contains(&s)
    {
        return Err(format!("scale must be between 1 and 4, got {s}"));
    }
    if let Some((x, y)) = keys.position
        && [x, y].iter().any(|v| v.abs() > MAX_POSITION)
    {
        return Err(format!("position [{x}, {y}] is out of range"));
    }
    Ok(())
}

fn entries(doc: &DocumentMut) -> Vec<&dyn TableLike> {
    match doc.get("outputs") {
        Some(Item::ArrayOfTables(a)) => a.iter().map(|t| t as &dyn TableLike).collect(),
        Some(Item::Value(Value::Array(a))) => a
            .iter()
            .filter_map(Value::as_inline_table)
            .map(|t| t as &dyn TableLike)
            .collect(),
        _ => Vec::new(),
    }
}

/// `doc` with the entry called `name` updated (or added at the end) to carry `keys`. Err when `outputs` is not a list of tables or `keys` holds a value the compositor would refuse; `doc` is unchanged on Err.
pub fn set_output_in(doc: &mut DocumentMut, name: &str, keys: &OutputKeys) -> Result<(), String> {
    check_keys(keys)?;
    check_outputs(doc)?;
    // Trailing comments belong to the document and would print after a new entry; they move in front of it instead.
    let listed = entries(doc)
        .iter()
        .any(|t| t.get("name").and_then(Item::as_str) == Some(name));
    let mut tail = String::new();
    if !listed {
        tail = doc.trailing().as_str().unwrap_or_default().to_string();
        doc.set_trailing("");
    }
    let outputs = outputs_mut(doc);
    let found = outputs
        .iter()
        .position(|t| t.get("name").and_then(Item::as_str) == Some(name));
    let index = found.unwrap_or_else(|| {
        let mut t = Table::new();
        t["name"] = value(name);
        if !tail.trim().is_empty() {
            t.decor_mut().set_prefix(format!("{tail}\n"));
        }
        outputs.push(t);
        outputs.len() - 1
    });
    let table = outputs
        .get_mut(index)
        .expect("a found entry or the one just pushed");
    if let Some(m) = keys.mode {
        table["mode"] = value(mode_text(m));
    }
    if let Some(t) = keys.transform {
        table["transform"] = value(t.name());
    }
    if let Some(s) = keys.scale {
        if s == 1.0 {
            table.remove("scale");
        } else if s.fract() == 0.0 {
            table["scale"] = value(s as i64);
        } else {
            table["scale"] = value(s);
        }
    }
    if let Some(v) = keys.adaptive_sync {
        table["adaptive_sync"] = value(v);
    }
    if let Some((x, y)) = keys.position {
        let mut at = Array::new();
        at.push(i64::from(x));
        at.push(i64::from(y));
        table["position"] = value(at);
    }
    Ok(())
}

/// `doc` with `primary = true` on the entry called `name` (added when missing) and on no other. Err when `outputs` is not a list of tables; `doc` is unchanged on Err.
pub fn set_primary_in(doc: &mut DocumentMut, name: &str) -> Result<(), String> {
    set_output_in(doc, name, &OutputKeys::default())?;
    for t in outputs_mut(doc).iter_mut() {
        if t.get("name").and_then(Item::as_str) == Some(name) {
            t["primary"] = value(true);
        } else {
            t.remove("primary");
        }
    }
    Ok(())
}

/// The output the file marks `primary`, the first when several are; None for a file that does not parse or marks none.
pub fn read_primary(text: &str) -> Option<String> {
    let doc: DocumentMut = text.parse().ok()?;
    entries(&doc)
        .into_iter()
        .find(|t| t.get("primary").and_then(Item::as_bool) == Some(true))
        .and_then(|t| t.get("name")?.as_str().map(str::to_string))
}

/// [`set_primary_in`] on the file at `path`, under `repriseconf::edit`'s lock. Err is the error box's text.
pub fn save_primary(path: &Path, name: &str) -> Result<(), &'static str> {
    let edited = repriseconf::edit(path, |text| {
        let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
        set_primary_in(&mut doc, name)?;
        Ok::<_, String>(doc.to_string())
    });
    match edited {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => {
            log::warn!(
                "{} does not parse, refusing to rewrite it: {e}",
                path.display()
            );
            Err("config.toml does not parse; the display setting was not saved.")
        }
        Err(e) => {
            log::warn!("write {}: {e}", path.display());
            Err("config.toml could not be written.")
        }
    }
}

/// [`set_output_in`] on `text`. Err when `text` is not TOML: a file that does not parse is not one to overwrite.
pub fn set_output(text: &str, name: &str, keys: &OutputKeys) -> Result<String, String> {
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    set_output_in(&mut doc, name, keys)?;
    Ok(doc.to_string())
}

/// [`set_output`] on the file at `path`, under `repriseconf::edit`'s lock. Err is the error box's text.
pub fn save(path: &Path, name: &str, keys: &OutputKeys) -> Result<(), &'static str> {
    match repriseconf::edit(path, |text| set_output(text, name, keys)) {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => {
            log::warn!(
                "{} does not parse, refusing to rewrite it: {e}",
                path.display()
            );
            Err("config.toml does not parse; the display setting was not saved.")
        }
        Err(e) => {
            log::warn!("write {}: {e}", path.display());
            Err("config.toml could not be written.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> OutputKeys {
        OutputKeys::default()
    }

    #[test]
    fn a_mode_without_a_refresh_is_written_without_hz() {
        let out = set_output(
            "",
            "Virtual-1",
            &OutputKeys {
                mode: Some((1280, 800, 0)),
                ..keys()
            },
        )
        .unwrap();
        assert!(out.contains("mode = \"1280x800\"\n"), "{out}");
        let out = set_output(
            "",
            "Virtual-1",
            &OutputKeys {
                mode: Some((1920, 1080, 59_940)),
                ..keys()
            },
        )
        .unwrap();
        assert!(out.contains("mode = \"1920x1080@59.94\"\n"), "{out}");
    }

    #[test]
    fn a_new_entry_goes_at_the_end_and_keeps_the_file() {
        let text = "terminal = \"foot\"\n\n[idle]\nlock_after = 0\n";
        let out = set_output(
            text,
            "HDMI-A-1",
            &OutputKeys {
                mode: Some((1920, 1080, 60_000)),
                transform: Some(Transform::Rotate90),
                ..keys()
            },
        )
        .unwrap();
        assert_eq!(
            out,
            format!(
                "{text}\n[[outputs]]\nname = \"HDMI-A-1\"\nmode = \"1920x1080@60\"\ntransform = \"90\"\n"
            )
        );
    }

    #[test]
    fn an_inline_outputs_array_is_edited_not_refused() {
        let text = "outputs = [{ name = \"DP-1\", scale = 2 }, { name = \"DP-2\" }]\n";
        let out = set_output(
            text,
            "DP-1",
            &OutputKeys {
                mode: Some((1920, 1080, 60_000)),
                ..keys()
            },
        )
        .unwrap();
        let doc: DocumentMut = out.parse().unwrap();
        let outputs = doc["outputs"].as_array_of_tables().expect("block form now");
        assert_eq!(outputs.len(), 2);
        let dp1 = outputs.get(0).unwrap();
        assert_eq!(dp1["scale"].as_integer(), Some(2));
        assert_eq!(dp1["mode"].as_str(), Some("1920x1080@60"));
        assert_eq!(
            set_output("outputs = [1]\n", "DP-1", &keys()).unwrap_err(),
            "`outputs` holds something that is not a table"
        );
    }

    #[test]
    fn an_existing_entry_keeps_its_other_keys_and_its_comments() {
        let text = "[[outputs]]\n# the laptop panel\nname = \"eDP-1\"\nscale = 1.5\nmode = \"1280x800\"\n\n[[outputs]]\nname = \"DP-1\"\ntransform = \"90\"\n";
        let out = set_output(
            text,
            "DP-1",
            &OutputKeys {
                transform: Some(Transform::Normal),
                adaptive_sync: Some(true),
                ..keys()
            },
        )
        .unwrap();
        assert_eq!(
            out,
            "[[outputs]]\n# the laptop panel\nname = \"eDP-1\"\nscale = 1.5\nmode = \"1280x800\"\n\n[[outputs]]\nname = \"DP-1\"\ntransform = \"normal\"\nadaptive_sync = true\n"
        );
    }

    #[test]
    fn a_scale_is_written_into_the_entry_and_removed_at_one() {
        let text = "[[outputs]]\nname = \"Virtual-1\"\nmode = \"1920x1080@60\"\n";
        let at = |s: f64| OutputKeys {
            scale: Some(s),
            ..keys()
        };
        let out = set_output(text, "Virtual-1", &at(1.5)).unwrap();
        assert_eq!(
            out,
            "[[outputs]]\nname = \"Virtual-1\"\nmode = \"1920x1080@60\"\nscale = 1.5\n"
        );
        let out = set_output(&out, "Virtual-1", &at(2.0)).unwrap();
        assert!(out.contains("scale = 2\n"), "{out}");
        let out = set_output(&out, "Virtual-1", &at(1.0)).unwrap();
        assert_eq!(out, text, "scale 1 drops the key and keeps the rest");
    }

    #[test]
    fn values_the_compositor_refuses_are_not_written() {
        let text = "[[outputs]]\nname = \"DP-1\"\n";
        let bad = |k: OutputKeys| set_output(text, "DP-1", &k).unwrap_err();
        bad(OutputKeys {
            scale: Some(5.0),
            ..keys()
        });
        bad(OutputKeys {
            scale: Some(f64::NAN),
            ..keys()
        });
        bad(OutputKeys {
            mode: Some((0, 800, 60_000)),
            ..keys()
        });
        bad(OutputKeys {
            mode: Some((1920, 1080, 1_000_001)),
            ..keys()
        });
    }

    #[test]
    fn an_error_leaves_the_doc_as_it_was() {
        let text = "# top\noutputs = [1]\n# end\n";
        let mut doc: DocumentMut = text.parse().unwrap();
        assert!(set_output_in(&mut doc, "DP-1", &keys()).is_err());
        assert_eq!(doc.to_string(), text);
        let text = "[idle]\nlock_after = 0\n# end\n";
        let mut doc: DocumentMut = text.parse().unwrap();
        let bad = OutputKeys {
            scale: Some(9.0),
            ..keys()
        };
        assert!(set_output_in(&mut doc, "DP-1", &bad).is_err());
        assert_eq!(doc.to_string(), text);
    }

    #[test]
    fn an_end_of_file_comment_moves_above_the_new_entry() {
        let out = set_output(
            "[idle]\nlock_after = 0\n# note\n",
            "DP-1",
            &OutputKeys {
                adaptive_sync: Some(true),
                ..keys()
            },
        )
        .unwrap();
        assert_eq!(
            out,
            "[idle]\nlock_after = 0\n# note\n\n[[outputs]]\nname = \"DP-1\"\nadaptive_sync = true\n"
        );
    }

    #[test]
    fn saving_an_output_edits_the_file_in_place() {
        let dir = std::env::temp_dir().join(format!("reprise-outputwrite-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "# mine\n").unwrap();
        let vrr = OutputKeys {
            adaptive_sync: Some(true),
            ..keys()
        };
        assert_eq!(save(&path, "DP-1", &vrr), Ok(()));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# mine\n\n[[outputs]]\nname = \"DP-1\"\nadaptive_sync = true\n"
        );
        std::fs::write(&path, "[[[").unwrap();
        assert!(
            save(&path, "DP-1", &vrr).is_err(),
            "a file that does not parse is left alone"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[[[");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_place_is_written_as_two_numbers() {
        let at = OutputKeys {
            position: Some((-1280, 56)),
            ..OutputKeys::default()
        };
        assert_eq!(
            set_output("", "DP-1", &at).unwrap(),
            "[[outputs]]\nname = \"DP-1\"\nposition = [-1280, 56]\n"
        );
        let far = OutputKeys {
            position: Some((1 << 25, 0)),
            ..OutputKeys::default()
        };
        assert!(set_output("", "DP-1", &far).is_err());
    }

    #[test]
    fn the_primary_mark_sits_on_one_entry() {
        let text =
            "[[outputs]]\nname = \"DP-1\"\nprimary = true\n\n[[outputs]]\nname = \"HDMI-A-1\"\n";
        assert_eq!(read_primary(text).as_deref(), Some("DP-1"));
        let mut doc: DocumentMut = text.parse().unwrap();
        set_primary_in(&mut doc, "HDMI-A-1").unwrap();
        let out = doc.to_string();
        assert_eq!(
            out,
            "[[outputs]]\nname = \"DP-1\"\n\n[[outputs]]\nname = \"HDMI-A-1\"\nprimary = true\n"
        );
        assert_eq!(read_primary(&out).as_deref(), Some("HDMI-A-1"));
        let mut doc: DocumentMut = out.parse().unwrap();
        set_primary_in(&mut doc, "DP-2").unwrap();
        assert_eq!(
            read_primary(&doc.to_string()).as_deref(),
            Some("DP-2"),
            "added"
        );
        assert_eq!(read_primary("[[["), None);
        assert_eq!(read_primary(""), None);
    }
}
