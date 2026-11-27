//! `[[outputs]]` entries of `config.toml` as Reprise Settings writes them; `toml_edit` leaves the rest of the file as it was. The keys are the ones `repriseconf::config::file::FileOutput` reads.

use std::path::{Path, PathBuf};

use repriseapp::heads::{Head, Transform};
use repriseconf::config::VrrPolicy;
use repriseconf::displays::{Layouts, Monitor, monitor_ids, outputs_fingerprint};
use repriseconf::hz_string;
use repriseconf::projection::Home;

use crate::model::OutputWallpaper;
use reprisekit::tr;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, TableLike, Value, value};

/// What to record for one output; a None leaves that key as it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputKeys {
    /// Width, height and refresh in mHz; a refresh of 0 is written without its `@`.
    pub mode: Option<(i32, i32, i32)>,
    pub transform: Option<Transform>,
    /// 1 removes the key, since 1 is what a missing one means; a whole number is written as an integer.
    pub scale: Option<f64>,
    pub adaptive_sync: Option<VrrPolicy>,
    pub position: Option<(i32, i32)>,
    /// `Some(None)` removes the key.
    pub icc_profile: Option<Option<PathBuf>>,
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
                Err(tr!("outputwrite-outputs-holds-something-that-is-not"))
            }
        }
        Some(_) => Err(tr!("outputwrite-outputs-is-not-an-array-of")),
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
        return Err(tr!(
            "outputwrite-mode-out-of-range",
            w = w.to_string(),
            h = h.to_string(),
            mhz = mhz.to_string()
        ));
    }
    if let Some(s) = keys.scale
        && !(1.0..=4.0).contains(&s)
    {
        return Err(tr!("outputwrite-scale-out-of-range", s = s.to_string()));
    }
    if let Some((x, y)) = keys.position
        && [x, y].iter().any(|v| v.abs() > MAX_POSITION)
    {
        return Err(tr!(
            "outputwrite-position-out-of-range",
            x = x.to_string(),
            y = y.to_string()
        ));
    }
    if let Some(Some(p)) = &keys.icc_profile
        && (!p.is_absolute() || p.to_str().is_none())
    {
        return Err(tr!(
            "outputwrite-not-absolute-path",
            path = p.display().to_string()
        ));
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

/// The `[[outputs]]` entry called `name`, added at the end when there is none. `check_outputs`
/// must have passed.
fn entry_mut<'a>(doc: &'a mut DocumentMut, name: &str) -> &'a mut Table {
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
    outputs
        .get_mut(index)
        .expect("a found entry or the one just pushed")
}

/// Each `[[outputs]]` entry's own `wallpaper` and `wallpaper_position`, for the entries that set
/// either.
pub fn read_wallpapers(doc: &DocumentMut) -> Vec<OutputWallpaper> {
    let text = |t: &dyn TableLike, k: &str| {
        (t.get(k).and_then(Item::as_str))
            .unwrap_or_default()
            .to_string()
    };
    entries(doc)
        .into_iter()
        .filter_map(|t| {
            let o = OutputWallpaper {
                name: t.get("name")?.as_str()?.to_string(),
                wallpaper: text(t, "wallpaper"),
                position: text(t, "wallpaper_position"),
            };
            (!o.wallpaper.is_empty() || !o.position.is_empty()).then_some(o)
        })
        .collect()
}

/// Write the outputs' own wallpaper keys that moved from `base` to `want`, entry by entry: an
/// emptied key is removed, an entry left holding only its name goes, and an output without an
/// entry gets one only for a key it sets. Err (and `doc` unchanged) when `outputs` is not a list
/// of tables.
pub fn write_wallpapers(
    doc: &mut DocumentMut,
    want: &[OutputWallpaper],
    base: &[OutputWallpaper],
) -> Result<(), String> {
    let of = |list: &[OutputWallpaper], name: &str| {
        (list.iter().find(|o| o.name == name))
            .cloned()
            .unwrap_or_else(|| OutputWallpaper {
                name: name.to_string(),
                ..Default::default()
            })
    };
    let mut names: Vec<&str> = want.iter().map(|o| o.name.as_str()).collect();
    names.extend(base.iter().map(|o| o.name.as_str()));
    names.dedup();
    let moved: Vec<(OutputWallpaper, OutputWallpaper)> = (names.iter())
        .map(|n| (of(want, n), of(base, n)))
        .filter(|(w, b)| w != b)
        .collect();
    if moved.is_empty() {
        return Ok(());
    }
    check_outputs(doc)?;
    for (w, b) in moved {
        let keys = [
            ("wallpaper", &w.wallpaper, &b.wallpaper),
            ("wallpaper_position", &w.position, &b.position),
        ];
        let listed =
            (entries(doc).iter()).any(|t| t.get("name").and_then(Item::as_str) == Some(&w.name));
        if !listed && keys.iter().all(|(_, v, _)| v.is_empty()) {
            continue;
        }
        let table = entry_mut(doc, &w.name);
        for (k, v, was) in keys {
            if v == was {
                continue;
            }
            if v.is_empty() {
                table.remove(k);
            } else {
                table[k] = value(v.as_str());
            }
        }
        if table.iter().all(|(k, _)| k == "name") {
            let outputs = outputs_mut(doc);
            outputs.retain(|t| t.get("name").and_then(Item::as_str) != Some(&w.name));
        }
    }
    Ok(())
}

/// `doc` with the entry called `name` updated (or added at the end) to carry `keys`. Err when `outputs` is not a list of tables or `keys` holds a value the compositor would refuse; `doc` is unchanged on Err.
pub fn set_output_in(doc: &mut DocumentMut, name: &str, keys: &OutputKeys) -> Result<(), String> {
    check_keys(keys)?;
    check_outputs(doc)?;
    let table = entry_mut(doc, name);
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
    match keys.adaptive_sync {
        Some(VrrPolicy::Off) => table["adaptive_sync"] = value(false),
        Some(VrrPolicy::On) => table["adaptive_sync"] = value(true),
        Some(VrrPolicy::Fullscreen) => table["adaptive_sync"] = value("fullscreen"),
        None => {}
    }
    match &keys.icc_profile {
        Some(Some(p)) => table["icc_profile"] = value(p.to_str().expect("checked above")),
        Some(None) => {
            table.remove("icc_profile");
        }
        None => {}
    }
    if let Some((x, y)) = keys.position {
        let mut at = Array::new();
        at.push(i64::from(x));
        at.push(i64::from(y));
        table["position"] = value(at);
    }
    Ok(())
}

/// Each entry's `icc_profile`, a `~/` one under `home`; empty for a file that does not parse.
pub fn read_profiles(text: &str, home: Option<&Path>) -> Vec<(String, PathBuf)> {
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return Vec::new();
    };
    entries(&doc)
        .into_iter()
        .filter_map(|t| {
            let name = t.get("name")?.as_str()?;
            let p = t.get("icc_profile")?.as_str().filter(|p| !p.is_empty())?;
            let path = match (p.strip_prefix("~/"), home) {
                (Some(rest), Some(home)) => home.join(rest),
                _ => PathBuf::from(p),
            };
            Some((name.to_string(), path))
        })
        .collect()
}

/// `adaptive_sync` as the file has it: the top-level key, and each `[[outputs]]` entry's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedVrr {
    pub top: VrrPolicy,
    pub outputs: Vec<(String, VrrPolicy)>,
}

impl SavedVrr {
    /// What output `name` runs under: its entry's key, else the top-level one.
    pub fn of(&self, name: &str) -> VrrPolicy {
        (self.outputs.iter())
            .find(|(n, _)| n == name)
            .map_or(self.top, |(_, p)| *p)
    }

    /// Output `name`'s key as just written.
    pub fn set(&mut self, name: &str, p: VrrPolicy) {
        self.outputs.retain(|(n, _)| n != name);
        self.outputs.push((name.to_string(), p));
    }
}

fn vrr_policy(item: Option<&Item>) -> Option<VrrPolicy> {
    let v = item?.as_value()?;
    v.as_bool()
        .map(VrrPolicy::of)
        .or_else(|| VrrPolicy::word(v.as_str()?))
}

/// The `adaptive_sync` keys of `text`; Off for a file that does not parse.
pub fn read_vrr(text: &str) -> SavedVrr {
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return SavedVrr::default();
    };
    let outputs = (entries(&doc).into_iter())
        .filter_map(|t| {
            let name = t.get("name")?.as_str()?;
            Some((name.to_string(), vrr_policy(t.get("adaptive_sync"))?))
        })
        .collect();
    SavedVrr {
        top: vrr_policy(doc.get("adaptive_sync")).unwrap_or_default(),
        outputs,
    }
}

/// The output the file marks `primary`, the first when several are; None for a file that does not parse or marks none.
pub fn read_primary(text: &str) -> Option<String> {
    let doc: DocumentMut = text.parse().ok()?;
    entries(&doc)
        .into_iter()
        .find(|t| t.get("primary").and_then(Item::as_bool) == Some(true))
        .and_then(|t| t.get("name")?.as_str().map(str::to_string))
}

/// Each of `connected`'s home by name (`projection::homes`): its place in the saved side-by-side
/// arrangement in `layouts` under `config`'s `[[outputs]]`, else its `[[outputs]]` entry.
pub fn homes(connected: &[Head], layouts: &str, config: &str) -> Vec<(String, Home)> {
    let monitors: Vec<Monitor<'_>> = (connected.iter())
        .map(|h| Monitor {
            connector: &h.name,
            make: &h.make,
            model: &h.model,
            serial: &h.serial,
        })
        .collect();
    let ids = monitor_ids(&monitors);
    let outputs = (repriseconf::config::Config::parse_lenient(config))
        .map(|(c, _)| c.outputs)
        .unwrap_or_default();
    let saved = Layouts::parse(layouts);
    let extended = (saved.find(&ids, &outputs_fingerprint(&outputs)))
        .and_then(|l| l.extended())
        .unwrap_or_default();
    let names: Vec<String> = connected.iter().map(|h| h.name.clone()).collect();
    let homes = repriseconf::projection::homes(extended, &ids, &names, &outputs);
    names.into_iter().zip(homes).collect()
}

/// The output the session takes as primary: the one the saved layout in `layouts` marks for the
/// `connected` monitors under `config`'s `[[outputs]]`, else the one `config` marks.
pub fn session_primary(connected: &[Head], layouts: &str, config: &str) -> Option<String> {
    let monitors: Vec<Monitor<'_>> = (connected.iter())
        .map(|h| Monitor {
            connector: &h.name,
            make: &h.make,
            model: &h.model,
            serial: &h.serial,
        })
        .collect();
    let ids = monitor_ids(&monitors);
    let outputs = (repriseconf::config::Config::parse_lenient(config))
        .map(|(c, _)| c.outputs)
        .unwrap_or_default();
    let saved = Layouts::parse(layouts);
    let marked = (saved.find(&ids, &outputs_fingerprint(&outputs)))
        .and_then(|l| l.monitors.iter().find(|(_, m)| m.primary))
        .and_then(|(id, _)| ids.iter().position(|i| i == id));
    match marked {
        Some(i) => Some(connected[i].name.clone()),
        None => read_primary(config),
    }
}

/// Why [`set_output`] left the text as it was.
#[derive(Debug, PartialEq)]
pub enum Unsaved {
    /// The text is not TOML: a file that does not parse is not one to overwrite.
    Parse(String),
    /// [`set_output_in`]'s refusal of a value or of the file's `outputs`.
    Refused(String),
}

/// [`set_output_in`] on `text`.
pub fn set_output(text: &str, name: &str, keys: &OutputKeys) -> Result<String, Unsaved> {
    let mut doc: DocumentMut = text.parse().map_err(|e| Unsaved::Parse(format!("{e}")))?;
    set_output_in(&mut doc, name, keys).map_err(Unsaved::Refused)?;
    Ok(doc.to_string())
}

/// [`set_output`] on the file at `path`, under `repriseconf::edit`'s lock. Err is the error box's text.
pub fn save(path: &Path, name: &str, keys: &OutputKeys) -> Result<(), String> {
    match repriseconf::edit(path, |text| set_output(text, name, keys)) {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(Unsaved::Parse(e))) => {
            log::warn!(
                "{} does not parse, refusing to rewrite it: {e}",
                path.display()
            );
            Err(tr!("outputwrite-config-toml-does-not-parse-the"))
        }
        Ok(Err(Unsaved::Refused(why))) => {
            log::warn!("{name} in {}: {why}", path.display());
            Err(tr!("outputwrite-not-saved-why", why = why.to_string()))
        }
        Err(e) => {
            log::warn!("write {}: {e}", path.display());
            Err(tr!("outputwrite-config-toml-could-not-be-written"))
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
            Unsaved::Refused("`outputs` holds something that is not a table".into())
        );
    }

    #[test]
    fn adaptive_sync_is_written_as_a_bool_or_fullscreen_and_read_back() {
        let mut text = "adaptive_sync = true\n".to_string();
        for (name, p) in [("DP-1", VrrPolicy::Fullscreen), ("DP-2", VrrPolicy::Off)] {
            let vrr = OutputKeys {
                adaptive_sync: Some(p),
                ..OutputKeys::default()
            };
            text = set_output(&text, name, &vrr).unwrap();
        }
        assert!(
            text.contains("name = \"DP-1\"\nadaptive_sync = \"fullscreen\"\n"),
            "{text}"
        );
        assert!(
            text.contains("name = \"DP-2\"\nadaptive_sync = false\n"),
            "{text}"
        );
        let saved = read_vrr(&text);
        assert_eq!(saved.of("DP-1"), VrrPolicy::Fullscreen);
        assert_eq!(saved.of("DP-2"), VrrPolicy::Off);
        assert_eq!(saved.of("HDMI-A-1"), VrrPolicy::On, "the top-level key");
        let c = repriseconf::config::Config::parse(&text).unwrap();
        assert_eq!(
            c.adaptive_sync_for_output_name("DP-1"),
            VrrPolicy::Fullscreen
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
                adaptive_sync: Some(VrrPolicy::On),
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
                adaptive_sync: Some(VrrPolicy::On),
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
            adaptive_sync: Some(VrrPolicy::On),
            ..keys()
        };
        assert_eq!(save(&path, "DP-1", &vrr), Ok(()));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# mine\n\n[[outputs]]\nname = \"DP-1\"\nadaptive_sync = true\n"
        );
        std::fs::write(&path, "[[[").unwrap();
        assert_eq!(
            save(&path, "DP-1", &vrr),
            Err("config.toml does not parse; the display setting was not saved.".into()),
            "a file that does not parse is left alone"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[[[");
        std::fs::write(&path, "outputs = 1\n").unwrap();
        assert_eq!(
            save(&path, "DP-1", &vrr),
            Err("The display setting was not saved: `outputs` is not an array of tables.".into()),
            "a refused value says why"
        );
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
    fn the_first_entry_marked_primary_is_read() {
        let text = "[[outputs]]\nname = \"DP-1\"\n\n[[outputs]]\nname = \"HDMI-A-1\"\nprimary = true\n\n[[outputs]]\nname = \"DP-2\"\nprimary = true\n";
        assert_eq!(read_primary(text).as_deref(), Some("HDMI-A-1"));
        assert_eq!(read_primary("[[["), None);
        assert_eq!(read_primary(""), None);
    }

    #[test]
    fn a_profile_is_written_read_back_and_removed() {
        let set = |text: &str, p: Option<&str>| {
            let keys = OutputKeys {
                icc_profile: Some(p.map(PathBuf::from)),
                ..keys()
            };
            set_output(text, "DP-1", &keys).unwrap()
        };
        let text = set(
            "[[outputs]]\nname = \"DP-1\"\nscale = 2\n",
            Some("/a b/x.icc"),
        );
        assert!(text.contains("icc_profile = \"/a b/x.icc\"\n"), "{text}");
        assert!(text.contains("scale = 2\n"), "the rest stays: {text}");
        assert_eq!(
            read_profiles(&text, None),
            [("DP-1".to_string(), PathBuf::from("/a b/x.icc"))]
        );
        let gone = set(&text, None);
        assert!(!gone.contains("icc_profile"), "{gone}");
        assert!(read_profiles(&gone, None).is_empty());
        let keys = OutputKeys {
            icc_profile: Some(Some(PathBuf::from("x.icc"))),
            ..keys()
        };
        assert!(set_output("", "DP-1", &keys).is_err(), "a relative path");
        let home = "[[outputs]]\nname = \"DP-1\"\nicc_profile = \"~/icc/x.icc\"\n";
        assert_eq!(
            read_profiles(home, Some(Path::new("/home/u"))),
            [("DP-1".to_string(), PathBuf::from("/home/u/icc/x.icc"))]
        );
    }
}
