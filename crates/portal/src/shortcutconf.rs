//! `[[global_shortcuts]]` and `[portal] shortcuts_denied` in config.toml, as the
//! GlobalShortcuts portal reads and writes them. An app with entries is allowed; a denied
//! one is never asked again until taken off the list. Keys are keybind text
//! ("Ctrl+Alt+space"), "" when unassigned. The compositor takes the same entries' chords
//! (`globalshortcuts`).

use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value, value};

const LIST: &str = "global_shortcuts";
const DENIED: repriseconf::Key = repriseconf::key("portal", "shortcuts_denied");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub description: String,
    pub keys: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    pub id: String,
    pub description: String,
    pub trigger: String,
}

fn str_of<'a>(t: &'a Table, k: &str) -> &'a str {
    t.get(k).and_then(Item::as_str).unwrap_or("")
}

fn tables(doc: &DocumentMut) -> impl Iterator<Item = &Table> {
    doc.get(LIST)
        .and_then(Item::as_array_of_tables)
        .into_iter()
        .flat_map(ArrayOfTables::iter)
}

pub fn entries(doc: &DocumentMut, app: &str) -> Vec<Entry> {
    tables(doc)
        .filter(|t| str_of(t, "app") == app)
        .map(|t| Entry {
            id: str_of(t, "id").into(),
            description: str_of(t, "description").into(),
            keys: str_of(t, "keys").into(),
        })
        .collect()
}

fn denied_list(doc: &DocumentMut) -> Vec<String> {
    repriseconf::get(doc, &DENIED)
        .and_then(Item::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

pub fn is_denied(doc: &DocumentMut, app: &str) -> bool {
    denied_list(doc).iter().any(|a| a == app)
}

fn set_denied(doc: &mut DocumentMut, list: &[String]) {
    if list.is_empty() {
        repriseconf::remove(doc, &DENIED);
    } else {
        let arr: Array = list.iter().map(String::as_str).collect();
        // A file holding only comments keeps them as trailing text, which
        // a new table would be written above; carry them along as its prefix.
        let lead = doc
            .as_table()
            .is_empty()
            .then(|| doc.trailing().as_str().unwrap_or("").to_string());
        // [portal] is a plain table this module owns; set() only fails on a non-table.
        let _ = repriseconf::set(doc, &DENIED, Value::Array(arr));
        if let Some(lead) = lead.filter(|l| !l.is_empty())
            && let Some(t) = doc.get_mut("portal").and_then(Item::as_table_mut)
        {
            t.decor_mut().set_prefix(lead);
            doc.set_trailing("");
        }
    }
}

pub fn deny(doc: &mut DocumentMut, app: &str) {
    let mut list = denied_list(doc);
    if !list.iter().any(|a| a == app) {
        list.push(app.into());
        set_denied(doc, &list);
    }
}

/// `trigger` (the shortcuts spec's `CTRL+ALT+space`) as keybind text,
/// or None when it names Num Lock, an unknown key or only modifiers.
pub fn trigger_keys(trigger: &str) -> Option<String> {
    let parts: Vec<&str> = trigger.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last()?;
    let mut out = Vec::new();
    for m in mods {
        out.push(match m.to_ascii_uppercase().as_str() {
            "CTRL" => "Ctrl",
            "ALT" => "Alt",
            "SHIFT" => "Shift",
            "LOGO" => "Super",
            _ => return None,
        });
    }
    out.push(key);
    let keys = out.join("+");
    (repriseconf::keybind::check(&keys).is_ok() && !repriseconf::keybind::modifier_only(&keys))
        .then_some(keys)
}

/// Whether `keys` is free for a program: not the desktop's (reserved, a
/// file bind or, without `[[keybinds]]`, a default one) and not another
/// entry's. Chords compare by keysym, as the compositor takes them.
fn free(doc: &DocumentMut, keys: &str) -> bool {
    if repriseconf::keybind::reserved(keys) {
        return false;
    }
    let file_binds: Option<Vec<String>> = doc
        .get("keybinds")
        .and_then(Item::as_array_of_tables)
        .map(|a| a.iter().map(|t| str_of(t, "keys").to_string()).collect());
    let binds = file_binds.unwrap_or_else(|| {
        repriseconf::keybind::DEFAULTS
            .iter()
            .map(|(k, _)| k.to_string())
            .collect()
    });
    let same = |b: &str| repriseconf::keybind::same(b, keys);
    !binds.iter().any(|b| same(b)) && !tables(doc).any(|t| same(str_of(t, "keys")))
}

fn assign(doc: &DocumentMut, trigger: &str) -> String {
    trigger_keys(trigger)
        .filter(|k| free(doc, k))
        .unwrap_or_default()
}

/// "description: keys" for each wanted shortcut, as a first bind would
/// assign them; the permit prompt lists these.
pub fn preview(doc: &DocumentMut, wanted: &[Wanted]) -> Vec<String> {
    let mut d = doc.clone();
    bind(&mut d, "\u{0}preview", wanted)
        .into_iter()
        .map(|e| {
            let keys = if e.keys.is_empty() {
                "no key".to_string()
            } else {
                e.keys
            };
            let what = if e.description.is_empty() {
                e.id
            } else {
                e.description
            };
            format!("{what}: {keys}")
        })
        .collect()
}

/// Allow `app` and give it `wanted`: a new id gets its trigger when free,
/// else no key; an existing one keeps its keys and takes a new
/// description. Ids not in `wanted` stay. Returns `wanted`'s entries in
/// its order.
pub fn bind(doc: &mut DocumentMut, app: &str, wanted: &[Wanted]) -> Vec<Entry> {
    if is_denied(doc, app) {
        let list: Vec<String> = denied_list(doc).into_iter().filter(|a| a != app).collect();
        set_denied(doc, &list);
    }
    for w in wanted {
        let found = doc
            .get_mut(LIST)
            .and_then(Item::as_array_of_tables_mut)
            .and_then(|a| {
                a.iter_mut()
                    .find(|t| str_of(t, "app") == app && str_of(t, "id") == w.id)
            });
        match found {
            Some(t) => {
                if !w.description.is_empty() && str_of(t, "description") != w.description {
                    t["description"] = value(&w.description);
                }
            }
            None => {
                let keys = assign(doc, &w.trigger);
                let mut t = Table::new();
                t["app"] = value(app);
                t["id"] = value(&w.id);
                t["description"] = value(&w.description);
                t["keys"] = value(keys);
                match doc.get_mut(LIST).and_then(Item::as_array_of_tables_mut) {
                    Some(a) => a.push(t),
                    None => {
                        let mut a = ArrayOfTables::new();
                        a.push(t);
                        doc[LIST] = Item::ArrayOfTables(a);
                    }
                }
            }
        }
    }
    let all = entries(doc, app);
    wanted
        .iter()
        .filter_map(|w| all.iter().find(|e| e.id == w.id).cloned())
        .collect()
}

pub fn read() -> DocumentMut {
    std::fs::read_to_string(repriseconf::config_path())
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or_default()
}

/// Apply `f` to the config under the shared edit lock and write it back
/// if it changed (`repriseconf::edit` skips an unchanged write). Err when
/// the file cannot be read, parsed or written; the file is then left as
/// it was.
pub fn update<T>(f: impl FnOnce(&mut DocumentMut) -> T) -> Result<T, String> {
    let mut out = None;
    repriseconf::edit(&repriseconf::config_path(), |text| {
        let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
        out = Some(f(&mut doc));
        Ok::<_, String>(doc.to_string())
    })
    .map_err(|e| e.to_string())??;
    out.ok_or_else(|| "config edit did not run".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(s: &str) -> DocumentMut {
        s.parse().unwrap()
    }
    fn w(id: &str, trigger: &str) -> Wanted {
        Wanted {
            id: id.into(),
            description: format!("{id} desc"),
            trigger: trigger.into(),
        }
    }

    #[test]
    fn triggers_become_keybind_text() {
        assert_eq!(
            trigger_keys("CTRL+ALT+space").as_deref(),
            Some("Ctrl+Alt+space")
        );
        assert_eq!(trigger_keys("Pause").as_deref(), Some("Pause"));
        assert_eq!(
            trigger_keys("LOGO+SHIFT+k").as_deref(),
            Some("Super+Shift+k")
        );
        assert_eq!(
            trigger_keys("NUM+k"),
            None,
            "Num Lock is not a modifier binds use"
        );
        assert_eq!(trigger_keys("CTRL+Nope"), None);
        assert_eq!(trigger_keys("CTRL+Shift_L"), None, "modifier-only");
        assert_eq!(trigger_keys(""), None);
    }

    #[test]
    fn a_first_bind_assigns_free_triggers_and_lifts_a_denial() {
        let mut d = doc("[portal]\nshortcuts_denied = [\"t\", \"o\"]\n");
        let got = bind(&mut d, "t", &[w("talk", "Pause"), w("mute", "")]);
        assert_eq!(
            got,
            [
                Entry {
                    id: "talk".into(),
                    description: "talk desc".into(),
                    keys: "Pause".into()
                },
                Entry {
                    id: "mute".into(),
                    description: "mute desc".into(),
                    keys: "".into()
                },
            ]
        );
        assert!(!is_denied(&d, "t"));
        assert!(is_denied(&d, "o"));
        assert_eq!(entries(&d, "t"), got);
    }

    #[test]
    fn a_taken_trigger_is_left_unassigned() {
        let mut d = doc("");
        bind(&mut d, "a", &[w("x", "F9")]);
        let got = bind(&mut d, "b", &[w("y", "F9"), w("z", "ALT+Tab")]);
        assert_eq!(got[0].keys, "", "F9 is a's");
        assert_eq!(got[1].keys, "", "Alt+Tab is a default bind");
        let mut d = doc("[[keybinds]]\nkeys = \"Super+t\"\naction = \"terminal\"\n");
        assert_eq!(
            bind(&mut d, "c", &[w("t", "ALT+Tab")])[0].keys,
            "Alt+Tab",
            "the file's binds replace the defaults"
        );
        assert_eq!(bind(&mut d, "c", &[w("u", "LOGO+t")])[0].keys, "");
    }

    #[test]
    fn a_trigger_naming_a_taken_keysym_another_way_is_left_unassigned() {
        let mut d = doc("[[keybinds]]\nkeys = \"Super+Prior\"\naction = \"terminal\"\n");
        assert_eq!(
            bind(&mut d, "a", &[w("x", "LOGO+Page_Up")])[0].keys,
            "",
            "Page_Up is Prior"
        );
        bind(&mut d, "a", &[w("y", "CTRL+Next")]);
        assert_eq!(bind(&mut d, "b", &[w("z", "CTRL+Page_Down")])[0].keys, "");
        assert_eq!(
            bind(&mut d, "b", &[w("v", "CTRL+ALT+F2")])[0].keys,
            "",
            "Ctrl+Alt+F2 switches the VT"
        );
    }

    #[test]
    fn a_reserved_trigger_is_left_unassigned() {
        let mut d = doc("");
        let got = bind(
            &mut d,
            "a",
            &[w("x", "LOGO+SHIFT+Escape"), w("y", "XF86AudioPlay")],
        );
        assert!(got.iter().all(|e| e.keys.is_empty()), "{got:?}");
    }

    #[test]
    fn a_later_bind_keeps_keys_and_takes_new_descriptions() {
        let mut d = doc(
            "[[global_shortcuts]]\napp = \"t\"\nid = \"talk\"\ndescription = \"old\"\nkeys = \"F8\"\n",
        );
        let got = bind(&mut d, "t", &[w("talk", "Pause")]);
        assert_eq!(
            got,
            [Entry {
                id: "talk".into(),
                description: "talk desc".into(),
                keys: "F8".into()
            }]
        );
    }

    #[test]
    fn ids_a_session_did_not_send_stay_in_the_file() {
        let mut d = doc("");
        bind(&mut d, "t", &[w("a", "F5"), w("b", "F6")]);
        assert_eq!(bind(&mut d, "t", &[w("a", "F5")]).len(), 1);
        assert_eq!(entries(&d, "t").len(), 2);
    }

    #[test]
    fn a_bind_leaves_a_denied_list_without_the_app_untouched() {
        let denied = "[portal]\nshortcuts_denied = [\n  \"a\", # spammy\n  \"b\",\n]\n";
        let mut d = doc(denied);
        bind(&mut d, "t", &[w("x", "F5")]);
        assert!(d.to_string().starts_with(denied), "{d}");
    }

    /// A bind that asks for what the app already has leaves the text as
    /// it was, so `update` writes nothing.
    #[test]
    fn a_repeated_bind_leaves_the_text_unchanged() {
        let text = "# mine\n[[global_shortcuts]] # talk\napp = \"t\"\nid = \"talk\"\ndescription = \"talk desc\"\nkeys = \"F8\"\n";
        let mut d = doc(text);
        bind(&mut d, "t", &[w("talk", "Pause")]);
        assert_eq!(d.to_string(), text);
    }

    #[test]
    fn deny_records_the_app_once() {
        let mut d = doc("# mine\n");
        deny(&mut d, "n");
        deny(&mut d, "n");
        assert!(is_denied(&d, "n"));
        assert!(d.to_string().starts_with("# mine\n"), "{d}");
        assert_eq!(d.to_string().matches("\"n\"").count(), 1);
    }

    #[test]
    fn the_prompt_lists_what_bind_would_give() {
        let d = doc("");
        assert_eq!(
            preview(&d, &[w("talk", "Pause"), w("x", "ALT+Tab")]),
            ["talk desc: Pause", "x desc: no key"]
        );
    }
}
