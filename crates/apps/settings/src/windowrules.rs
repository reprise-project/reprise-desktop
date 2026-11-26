//! `[[window_rules]]` as the Window Rules page edits them. Apply writes each rule over the table it
//! came from, so the comments on its lines and keys the page does not know stay with it, and the
//! comment above the first rule stays on top of the list.

use crate::startmenu::{array, in_blocks, keep_head, paired, set_tables, tables};
use repriseconf::{Key, key as k};
use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value, value};

pub const KEY: Key = k("", "window_rules");

/// The keys a rule's table holds that the page edits.
const KEYS: [&str; 11] = [
    "app_id",
    "title",
    "size",
    "position",
    "output",
    "state",
    "desktop",
    "always_on_top",
    "sticky",
    "tab_group",
    "from_menu",
];

/// One `[[window_rules]]` table; `None` is a key the table leaves out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rule {
    pub app_id: String,
    pub title: Option<String>,
    pub size: Option<[i64; 2]>,
    pub position: Option<[i64; 2]>,
    pub output: Option<String>,
    /// One of the compositor's states: normal, maximized, minimized or fullscreen.
    pub state: Option<String>,
    /// 1-based, as the file counts.
    pub desktop: Option<i64>,
    pub always_on_top: Option<bool>,
    pub sticky: Option<bool>,
    pub tab_group: Option<String>,
    /// Always Open Here's own rule, which the window menu replaces and removes.
    pub from_menu: bool,
}

/// The file's rules in file order, a table without an `app_id` as one with an empty one.
pub fn read(doc: &DocumentMut) -> Vec<Rule> {
    (repriseconf::get(doc, &KEY).and_then(tables))
        .map(|ts| ts.into_iter().map(read_rule).collect())
        .unwrap_or_default()
}

fn read_rule(t: &dyn TableLike) -> Rule {
    let text = |name: &str| t.get(name).and_then(Item::as_str).map(str::to_string);
    let flag = |name: &str| t.get(name).and_then(Item::as_bool);
    let pair = |name: &str| {
        let a = t.get(name)?.as_array()?;
        match (a.len(), a.get(0)?.as_integer(), a.get(1)?.as_integer()) {
            (2, Some(x), Some(y)) => Some([x, y]),
            _ => None,
        }
    };
    Rule {
        app_id: text("app_id").unwrap_or_default(),
        title: text("title"),
        size: pair("size"),
        position: pair("position"),
        output: text("output"),
        state: text("state"),
        desktop: t.get("desktop").and_then(Item::as_integer),
        always_on_top: flag("always_on_top"),
        sticky: flag("sticky"),
        tab_group: text("tab_group"),
        from_menu: flag("from_menu") == Some(true),
    }
}

fn table(r: &Rule) -> Table {
    let mut t = Table::new();
    let pair = |[a, b]: [i64; 2]| value(Array::from_iter([a, b]));
    t["app_id"] = value(&r.app_id);
    if let Some(v) = &r.title {
        t["title"] = value(v);
    }
    if let Some(v) = r.size {
        t["size"] = pair(v);
    }
    if let Some(v) = r.position {
        t["position"] = pair(v);
    }
    if let Some(v) = &r.output {
        t["output"] = value(v);
    }
    if let Some(v) = &r.state {
        t["state"] = value(v);
    }
    if let Some(v) = r.desktop {
        t["desktop"] = value(v);
    }
    for (name, v) in [("always_on_top", r.always_on_top), ("sticky", r.sticky)] {
        if let Some(v) = v {
            t[name] = value(v);
        }
    }
    if let Some(v) = &r.tab_group {
        t["tab_group"] = value(v);
    }
    if r.from_menu {
        t["from_menu"] = value(true);
    }
    t
}

/// `want`, edited from `base`, once the list reads `have`: a rule added or removed outside the
/// sheet since `base` (Always Open Here, a hand edit) is added or removed, and the edits stay.
pub fn merge(want: &[Rule], base: &[Rule], have: &[Rule]) -> Vec<Rule> {
    if want == base {
        return have.to_vec();
    }
    let mut rules: Vec<Rule> = (want.iter())
        .filter(|r| !base.contains(r) || have.contains(r))
        .cloned()
        .collect();
    let added = have
        .iter()
        .filter(|r| !base.contains(r) && !want.contains(r));
    rules.extend(added.cloned());
    rules
}

/// Write `want` over `base` (as loaded or last saved) into `doc`, whose rules read as `have`, as
/// [`merge`] says; none left drops the key. An inline list goes into `[[window_rules]]` blocks.
pub fn write(doc: &mut DocumentMut, want: &[Rule], base: &[Rule], have: &[Rule]) {
    if want == base || want == have {
        return;
    }
    if !in_blocks(doc, &KEY) {
        log::warn!("window_rules lists something no table; the window rules are not saved");
        return;
    }
    let rules = merge(want, base, have);
    let old = array(doc, &KEY);
    let read: Vec<Rule> = old.iter().map(|t| read_rule(t)).collect();
    let from = paired(&read, &rules, 3, |pass, r, w| match pass {
        0 => r == w,
        1 => r.app_id == w.app_id && r.title == w.title,
        _ => r.app_id == w.app_id,
    });
    let mut out: Vec<Table> = (rules.iter().zip(&from))
        .map(|(w, f)| match f {
            Some(j) if read[*j] == *w => {
                let mut t = old[*j].clone();
                t.set_position(None);
                t
            }
            Some(j) => merged(&old[*j], table(w)),
            None => table(w),
        })
        .collect();
    keep_head(&old, &from, &mut out);
    set_tables(doc, &KEY, out);
}

/// `new` written over `old`: a key the page edits and `new` lacks goes, any other key stays, and a
/// changed value keeps the comment beside it.
fn merged(old: &Table, new: Table) -> Table {
    let mut t = old.clone();
    t.set_position(None);
    t.retain(|k, _| new.contains_key(k) || !KEYS.contains(&k));
    for (k, v) in new {
        match (t.get_mut(&k), v) {
            (Some(Item::Value(o)), Item::Value(mut n)) => {
                if bare(o) != bare(&n) {
                    *n.decor_mut() = o.decor().clone();
                    *o = n;
                }
            }
            (Some(slot), v) => *slot = v,
            (None, v) => {
                t.insert(&k, v);
            }
        }
    }
    t
}

fn bare(v: &Value) -> String {
    let mut v = v.clone();
    v.decor_mut().clear();
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(text: &str, want: &[Rule]) -> String {
        let mut doc: DocumentMut = text.parse().unwrap();
        let have = read(&doc);
        write(&mut doc, want, &have, &have);
        doc.to_string()
    }

    fn rule(app_id: &str) -> Rule {
        Rule {
            app_id: app_id.into(),
            ..Rule::default()
        }
    }

    const FILE: &str = "\
terminal = \"foot\"  # mine

# Rules I keep
[[window_rules]]
app_id = \"foot\"   # the shell
size = [800, 600]
note = \"mine\"

# the editor
[[window_rules]]
app_id = \"reprise.notepad\"
state = \"maximized\"

[session]
restore = false   # keep
";

    #[test]
    fn every_key_reads_and_writes_back() {
        let full = Rule {
            app_id: "foot".into(),
            title: Some("*build*".into()),
            size: Some([800, 600]),
            position: Some([-40, 40]),
            output: Some("DP-1".into()),
            state: Some("maximized".into()),
            desktop: Some(2),
            always_on_top: Some(true),
            sticky: Some(false),
            tab_group: Some("shells".into()),
            from_menu: true,
        };
        let text = saved("", std::slice::from_ref(&full));
        assert_eq!(
            text,
            "[[window_rules]]\napp_id = \"foot\"\ntitle = \"*build*\"\nsize = [800, 600]\n\
             position = [-40, 40]\noutput = \"DP-1\"\nstate = \"maximized\"\ndesktop = 2\n\
             always_on_top = true\nsticky = false\ntab_group = \"shells\"\nfrom_menu = true\n"
        );
        assert_eq!(read(&text.parse().unwrap()), [full]);
        let inline: DocumentMut = "window_rules = [{ app_id = \"foot\", desktop = 3 }]\n"
            .parse()
            .unwrap();
        assert_eq!(
            read(&inline),
            [Rule {
                desktop: Some(3),
                ..rule("foot")
            }]
        );
    }

    #[test]
    fn an_unchanged_list_leaves_the_file_as_it_is() {
        let doc: DocumentMut = FILE.parse().unwrap();
        assert_eq!(saved(FILE, &read(&doc)), FILE);
    }

    #[test]
    fn an_edit_keeps_the_comments_the_unknown_keys_and_the_keys_around_the_list() {
        let mut rules = read(&FILE.parse().unwrap());
        rules[0].size = Some([1024, 768]);
        rules[1].state = None;
        rules[1].sticky = Some(true);
        assert_eq!(
            saved(FILE, &rules),
            "\
terminal = \"foot\"  # mine

# Rules I keep
[[window_rules]]
app_id = \"foot\"   # the shell
size = [1024, 768]
note = \"mine\"

# the editor
[[window_rules]]
app_id = \"reprise.notepad\"
sticky = true

[session]
restore = false   # keep
"
        );
    }

    #[test]
    fn moved_rules_take_their_comments_and_the_head_stays_on_top() {
        let mut rules = read(&FILE.parse().unwrap());
        rules.swap(0, 1);
        let text = saved(FILE, &rules);
        assert_eq!(
            text,
            "\
terminal = \"foot\"  # mine

# Rules I keep

# the editor
[[window_rules]]
app_id = \"reprise.notepad\"
state = \"maximized\"

[[window_rules]]
app_id = \"foot\"   # the shell
size = [800, 600]
note = \"mine\"

[session]
restore = false   # keep
"
        );
        assert_eq!(read(&text.parse().unwrap()), rules);
    }

    #[test]
    fn removing_every_rule_takes_the_key_out_and_leaves_valid_toml() {
        let text = saved(FILE, &[]);
        let doc: DocumentMut = text.parse().expect("still TOML");
        assert!(read(&doc).is_empty());
        assert!(!text.contains("window_rules"), "{text}");
        assert!(text.contains("terminal = \"foot\"  # mine"), "{text}");
        assert!(text.contains("restore = false   # keep"), "{text}");
        assert!(
            text.contains("# Rules I keep"),
            "the comment above the list stays: {text}"
        );
        assert!(
            !text.contains("# the editor"),
            "a rule's own comment goes with it: {text}"
        );
        repriseconf::config::Config::parse(&text).expect("the compositor reads it");
    }

    #[test]
    fn a_rule_written_elsewhere_meanwhile_survives_an_apply() {
        let base = read(&FILE.parse().unwrap());
        let mut want = base.clone();
        want.push(rule("kitty"));
        let menu = Rule {
            from_menu: true,
            ..rule("firefox")
        };
        let mut doc: DocumentMut = FILE.parse().unwrap();
        doc["window_rules"]
            .as_array_of_tables_mut()
            .unwrap()
            .push(table(&menu));
        let have = read(&doc);
        write(&mut doc, &want, &base, &have);
        let ids: Vec<String> = read(&doc).into_iter().map(|r| r.app_id).collect();
        assert_eq!(ids, ["foot", "reprise.notepad", "kitty", "firefox"]);

        let mut doc: DocumentMut = FILE.parse().unwrap();
        doc["window_rules"]
            .as_array_of_tables_mut()
            .unwrap()
            .remove(1);
        let have = read(&doc);
        write(&mut doc, &want, &base, &have);
        let ids: Vec<String> = read(&doc).into_iter().map(|r| r.app_id).collect();
        assert_eq!(
            ids,
            ["foot", "kitty"],
            "a rule removed meanwhile stays removed"
        );
    }

    #[test]
    fn an_inline_list_is_written_in_block_form_under_its_comment() {
        let text = "\
# my rules
window_rules = [{ app_id = \"foot\", desktop = 3 }, { app_id = \"kitty\" }]
";
        let mut rules = read(&text.parse().unwrap());
        rules[1].sticky = Some(true);
        let out = saved(text, &rules);
        assert_eq!(
            out,
            "\
# my rules
[[window_rules]]
app_id = \"foot\"
desktop = 3

[[window_rules]]
app_id = \"kitty\"
sticky = true
"
        );
        repriseconf::config::Config::parse(&out).expect("the compositor reads it");
    }

    #[test]
    fn a_change_to_one_rule_leaves_the_others_as_the_file_has_them() {
        let text = "\
[[window_rules]]
app_id = \"foot\"
from_menu = false
size = [800, 600.0]

[[window_rules]]
app_id = \"kitty\"
from_menu = false
";
        let mut rules = read(&text.parse().unwrap());
        rules[1].desktop = Some(2);
        assert_eq!(
            saved(text, &rules),
            "\
[[window_rules]]
app_id = \"foot\"
from_menu = false
size = [800, 600.0]

[[window_rules]]
app_id = \"kitty\"
desktop = 2
"
        );
    }

    #[test]
    fn a_merge_takes_the_rules_added_and_removed_since_the_base_into_the_edits() {
        let [a, b, c, menu] = ["a", "b", "c", "firefox"].map(rule);
        let edited = Rule {
            desktop: Some(2),
            ..b.clone()
        };
        let base = [a.clone(), b.clone(), c.clone()];
        let want = [a.clone(), edited.clone(), c.clone()];
        let have = [b.clone(), c.clone(), menu.clone()];
        assert_eq!(merge(&want, &base, &have), [edited, c, menu]);
        assert_eq!(
            merge(&base, &base, &have),
            have,
            "nothing pending: the file's"
        );
    }
}
