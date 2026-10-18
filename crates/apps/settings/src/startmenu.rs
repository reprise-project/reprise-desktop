//! `[[startmenu]]` as the Start Menu page edits it: a tree of entries
//! read out of the file and written back whole, as `[[keybinds]]` is.
//!
//! **The default menu here mirrors `default_startmenu` in
//! `src/config.rs`**, the menu a file without `[[startmenu]]` gets; a test
//! holds both to `config.example.toml`'s block.

use repriseconf::{Key, key as k, remove};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, TableLike};

pub const STARTMENU: Key = k("", "startmenu");

/// One row of the Start menu or of a folder in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Separator,
    Item {
        /// The label as the file has it: `&` marks the access key and
        /// `{user}` stands for the login name.
        label: String,
        /// An icon theme name or an absolute path.
        icon: Option<String>,
        /// The infotip, carried through untouched.
        tip: Option<String>,
        target: Target,
    },
}

/// What picking a row does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// `command = "..."`, run by the shell.
    Command(String),
    /// `action = "..."`, and the `command` a `spawn` action carries.
    Action(String, Option<String>),
    /// `submenu = "programs"`, `"settings"` or `"documents"`: a folder the
    /// compositor fills when it opens.
    Special(String),
    /// `items = [...]`: a folder of these entries.
    Folder(Vec<Entry>),
}

impl Entry {
    pub fn item(label: &str, icon: &str, target: Target) -> Entry {
        Entry::Item {
            label: label.into(),
            icon: (!icon.is_empty()).then(|| icon.into()),
            tip: None,
            target,
        }
    }

    /// A row running `command`, labelled `name` as Add Item's Name field
    /// holds it: plain text, so an `&` in it (a program's "Software &
    /// Updates") is doubled rather than read as the access key.
    pub fn program(name: &str, icon: &str, command: String) -> Entry {
        Entry::item(
            &reprisekit::text::escape_mnemonic(name),
            icon,
            Target::Command(command),
        )
    }

    fn tipped(self, text: &str) -> Entry {
        match self {
            Entry::Item {
                label,
                icon,
                target,
                ..
            } => Entry::Item {
                label,
                icon,
                tip: Some(text.into()),
                target,
            },
            sep => sep,
        }
    }

    /// The folder's entries, for a folder the page can open and fill.
    pub fn items(&self) -> Option<&Vec<Entry>> {
        match self {
            Entry::Item {
                target: Target::Folder(v),
                ..
            } => Some(v),
            _ => None,
        }
    }

    fn items_mut(&mut self) -> Option<&mut Vec<Entry>> {
        match self {
            Entry::Item {
                target: Target::Folder(v),
                ..
            } => Some(v),
            _ => None,
        }
    }

    /// Whether the row opens a submenu: a folder of entries or one the
    /// compositor fills.
    pub fn is_folder(&self) -> bool {
        matches!(
            self,
            Entry::Item {
                target: Target::Folder(_) | Target::Special(_),
                ..
            }
        )
    }

    pub fn label(&self) -> Option<&str> {
        match self {
            Entry::Item { label, .. } => Some(label),
            Entry::Separator => None,
        }
    }

    pub fn icon(&self) -> Option<&str> {
        match self {
            Entry::Item { icon, .. } => icon.as_deref(),
            Entry::Separator => None,
        }
    }
}

/// A tree the page edits in step with the menu: the entries themselves,
/// and the tree view's nodes standing for them.
pub trait Branch: Sized {
    fn kids(&mut self) -> Option<&mut Vec<Self>>;
}

impl Branch for Entry {
    fn kids(&mut self) -> Option<&mut Vec<Entry>> {
        self.items_mut()
    }
}

impl Branch for reprisekit::controls::treeview::Node {
    fn kids(&mut self) -> Option<&mut Vec<Self>> {
        Some(&mut self.children)
    }
}

/// The sibling list the node at `parent` holds; `roots` for `[]`.
pub fn level<'a, T: Branch>(roots: &'a mut Vec<T>, parent: &[usize]) -> Option<&'a mut Vec<T>> {
    let mut at = roots;
    for &i in parent {
        at = at.get_mut(i)?.kids()?;
    }
    Some(at)
}

/// Move the entry at `path` `by` places among its siblings. Returns its
/// new path; None at either end.
pub fn shift<T: Branch>(roots: &mut Vec<T>, path: &[usize], by: isize) -> Option<Vec<usize>> {
    let (&i, parent) = path.split_last()?;
    let v = level(roots, parent)?;
    let to = i.checked_add_signed(by).filter(|&to| to < v.len())?;
    if i >= v.len() {
        return None;
    }
    let e = v.remove(i);
    v.insert(to, e);
    let mut out = parent.to_vec();
    out.push(to);
    Some(out)
}

pub fn take<T: Branch>(roots: &mut Vec<T>, path: &[usize]) -> Option<T> {
    let (&i, parent) = path.split_last()?;
    let v = level(roots, parent)?;
    (i < v.len()).then(|| v.remove(i))
}

/// Put `e` at `path`, which names its place among its new siblings.
pub fn put<T: Branch>(roots: &mut Vec<T>, path: &[usize], e: T) -> Option<()> {
    let (&i, parent) = path.split_last()?;
    let v = level(roots, parent)?;
    (i <= v.len()).then(|| v.insert(i, e))
}

/// Where a new entry goes with `selected` picked (None for the menu
/// itself): at the end of a picked folder of entries, else right after
/// the picked row, else at the end of the menu.
pub fn insertion(entries: &[Entry], selected: Option<&[usize]>) -> Vec<usize> {
    let Some(sel) = selected.filter(|s| !s.is_empty()) else {
        return vec![entries.len()];
    };
    if let Some(items) = get(entries, sel).and_then(Entry::items) {
        let mut p = sel.to_vec();
        p.push(items.len());
        return p;
    }
    let mut p = sel.to_vec();
    if let Some(last) = p.last_mut() {
        *last += 1;
    }
    p
}

pub fn get<'a>(entries: &'a [Entry], path: &[usize]) -> Option<&'a Entry> {
    let (first, rest) = path.split_first()?;
    let mut e = entries.get(*first)?;
    for &i in rest {
        e = e.items()?.get(i)?;
    }
    Some(e)
}

/// Give the entry at `path` a new label; a separator has none.
pub fn rename(entries: &mut Vec<Entry>, path: &[usize], to: &str) -> bool {
    let Some((&i, parent)) = path.split_last() else {
        return false;
    };
    match level(entries, parent).and_then(|v| v.get_mut(i)) {
        Some(Entry::Item { label, .. }) => {
            *label = to.into();
            true
        }
        _ => false,
    }
}

/// The menu a file without `[[startmenu]]` gets.
pub fn default_menu() -> Vec<Entry> {
    use Target as T;
    let cmd = |s: &str| T::Command(s.into());
    let act = |s: &str| T::Action(s.into(), None);
    vec![
        Entry::item(
            "&Programs",
            "applications-other",
            T::Special("programs".into()),
        ),
        Entry::item(
            "&Documents",
            "folder-documents",
            T::Special("documents".into()),
        ),
        Entry::item(
            "&Settings",
            "preferences-system",
            T::Folder(vec![
                Entry::item(
                    "&Control Panel",
                    "preferences-other",
                    T::Special("settings".into()),
                )
                .tipped(
                    "Provides options for you to customize the appearance and functionality \
                     of your computer, add or remove programs, and set up network connections \
                     and user accounts.",
                ),
                Entry::Separator,
                Entry::item(
                    "&Network Connections",
                    "network-workgroup",
                    cmd("nm-connection-editor"),
                )
                .tipped(
                    "Displays existing network connections on this computer and helps you \
                     create new ones",
                ),
                Entry::item("&Printers", "printer", cmd("reprise-printers")).tipped(
                    "Add, remove, and configure local and network printers, and see what is \
                     printing.",
                ),
                Entry::item(
                    "&Taskbar and Start Menu",
                    "preferences-desktop",
                    cmd("reprise-settings --page taskbar"),
                )
                .tipped(
                    "Customize the Start Menu and the taskbar, such as the types of items to \
                     be displayed and how they should appear.",
                ),
            ]),
        ),
        Entry::item(
            "Sear&ch",
            "system-search",
            T::Folder(vec![
                Entry::item(
                    "&For Files or Folders...",
                    "system-search",
                    cmd("reprise-explorer \"$HOME\""),
                ),
                Entry::item(
                    "On the &Internet...",
                    "web-browser",
                    cmd("xdg-open https://duckduckgo.com/"),
                ),
            ]),
        ),
        Entry::item(
            "&Help and Support",
            "help-browser",
            cmd("xdg-open https://github.com/reprise-project/reprise-desktop"),
        ),
        Entry::item("&Run...", "system-run", act("run")),
        Entry::Separator,
        Entry::item("&Log Off {user}...", "gnome-session-logout", act("logoff")),
        Entry::item("T&urn Off Computer...", "system-shutdown", act("turnoff")),
    ]
}

/// The file's `[[startmenu]]` (or an inline `startmenu = [{...}]`); None
/// when it has none. A row the compositor would refuse (no label, or
/// nothing to do) is left out.
pub fn read(doc: &DocumentMut) -> Option<Vec<Entry>> {
    repriseconf::get(doc, &STARTMENU).and_then(read_list)
}

fn tables(item: &Item) -> Option<Vec<&dyn TableLike>> {
    if let Some(a) = item.as_array_of_tables() {
        return Some(a.iter().map(|t| t as &dyn TableLike).collect());
    }
    Some(
        item.as_array()?
            .iter()
            .filter_map(|v| v.as_inline_table().map(|t| t as &dyn TableLike))
            .collect(),
    )
}

fn read_list(item: &Item) -> Option<Vec<Entry>> {
    Some(tables(item)?.into_iter().filter_map(read_entry).collect())
}

fn read_entry(t: &dyn TableLike) -> Option<Entry> {
    let text = |name: &str| {
        t.get(name)
            .and_then(Item::as_str)
            .map(str::to_string)
            .filter(|s| !s.is_empty())
    };
    if t.get("separator").and_then(Item::as_bool) == Some(true) {
        return Some(Entry::Separator);
    }
    let target = if let Some(a) = text("action") {
        Target::Action(a, text("command"))
    } else if let Some(c) = text("command") {
        Target::Command(c)
    } else if let Some(s) = text("submenu") {
        Target::Special(s)
    } else {
        // A folder emptied by hand can lose its `items` key; it is still
        // a folder, as the compositor reads it.
        Target::Folder(t.get("items").map_or(Some(Vec::new()), read_list)?)
    };
    Some(Entry::Item {
        label: t.get("label").and_then(Item::as_str)?.to_string(),
        icon: text("icon"),
        tip: text("tip"),
        target,
    })
}

fn table(e: &Entry) -> Table {
    let mut t = Table::new();
    let Entry::Item {
        label,
        icon,
        tip,
        target,
    } = e
    else {
        t["separator"] = toml_edit::value(true);
        return t;
    };
    t["label"] = toml_edit::value(label);
    if let Some(icon) = icon {
        t["icon"] = toml_edit::value(icon);
    }
    match target {
        Target::Command(c) => t["command"] = toml_edit::value(c),
        Target::Action(a, c) => {
            t["action"] = toml_edit::value(a);
            if let Some(c) = c {
                t["command"] = toml_edit::value(c);
            }
        }
        Target::Special(s) => t["submenu"] = toml_edit::value(s),
        // An empty array of tables prints as nothing, so an empty folder
        // is written `items = []`.
        Target::Folder(items) if items.is_empty() => {
            t["items"] = toml_edit::value(toml_edit::Array::new());
        }
        Target::Folder(items) => {
            let mut a = ArrayOfTables::new();
            for e in items {
                a.push(table(e));
            }
            t["items"] = Item::ArrayOfTables(a);
        }
    }
    if let Some(tip) = tip {
        t["tip"] = toml_edit::value(tip);
    }
    t
}

/// Write `want` into `doc` as `[[startmenu]]` when the sheet moved it:
/// not when it equals `base` (what the sheet loaded) or `have` (what the
/// file says now), and as no key at all when it is the default menu. The
/// comment above the block's first table stays where it was.
pub fn write(doc: &mut DocumentMut, want: &[Entry], base: &[Entry], have: &[Entry]) {
    if want == base || want == have {
        return;
    }
    let old = repriseconf::get(doc, &STARTMENU)
        .and_then(Item::as_array_of_tables)
        .and_then(|a| a.get(0))
        .map(|t| t.decor().clone());
    if want == default_menu() {
        remove(doc, &STARTMENU);
        return;
    }
    let mut a = ArrayOfTables::new();
    for e in want {
        a.push(table(e));
    }
    if let (Some(decor), Some(first)) = (old, a.get_mut(0)) {
        *first.decor_mut() = decor;
    }
    doc[STARTMENU.name] = Item::ArrayOfTables(a);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(text: &str) -> Vec<Entry> {
        read(&text.parse().unwrap()).unwrap_or_else(default_menu)
    }

    fn saved(text: &str, want: &[Entry]) -> String {
        let mut doc: DocumentMut = text.parse().unwrap();
        let have = load(text);
        write(&mut doc, want, &have, &[]);
        doc.to_string()
    }

    #[test]
    fn the_default_menu_is_the_example_files() {
        let example = include_str!("../../../../config.example.toml");
        let doc: DocumentMut = example.parse().unwrap();
        assert_eq!(read(&doc), Some(default_menu()));
    }

    #[test]
    fn every_kind_of_row_reads_and_writes_back() {
        let text = "\
[[startmenu]]
label = \"&Go\"
action = \"run\"

[[startmenu]]
separator = true

[[startmenu]]
label = \"Box\"
icon = \"folder\"

[[startmenu.items]]
label = \"Edit\"
command = \"gedit\"
tip = \"Edits text\"

[[startmenu.items]]
label = \"Apps\"
submenu = \"programs\"

[[startmenu]]
label = \"Spawned\"
action = \"spawn\"
command = \"foot\"
";
        let menu = load(text);
        assert_eq!(
            menu,
            vec![
                Entry::item("&Go", "", Target::Action("run".into(), None)),
                Entry::Separator,
                Entry::item(
                    "Box",
                    "folder",
                    Target::Folder(vec![
                        Entry::item("Edit", "", Target::Command("gedit".into()))
                            .tipped("Edits text"),
                        Entry::item("Apps", "", Target::Special("programs".into())),
                    ])
                ),
                Entry::item(
                    "Spawned",
                    "",
                    Target::Action("spawn".into(), Some("foot".into()))
                ),
            ]
        );
        let mut doc = DocumentMut::new();
        write(&mut doc, &menu, &[], &[]);
        assert_eq!(doc.to_string(), text);
    }

    #[test]
    fn a_programs_ampersand_is_kept_as_text() {
        let e = Entry::program("Software & Updates", "", "x".into());
        assert_eq!(e.label(), Some("Software && Updates"));
        assert_eq!(crate::menupage::shown(&e), "Software & Updates");
    }

    #[test]
    fn an_empty_folder_is_written_with_an_empty_items_list() {
        let menu = vec![
            Entry::item("New Folder", "", Target::Folder(Vec::new())),
            Entry::item(
                "Box",
                "",
                Target::Folder(vec![Entry::item("Inner", "", Target::Folder(Vec::new()))]),
            ),
        ];
        let mut doc = DocumentMut::new();
        write(&mut doc, &menu, &[], &[]);
        let text = doc.to_string();
        assert_eq!(
            text,
            "\
[[startmenu]]
label = \"New Folder\"
items = []

[[startmenu]]
label = \"Box\"

[[startmenu.items]]
label = \"Inner\"
items = []
"
        );
        assert_eq!(load(&text), menu);
    }

    #[test]
    fn a_folder_without_an_items_key_reads_as_empty() {
        assert_eq!(
            load("[[startmenu]]\nlabel = \"Old\"\n"),
            vec![Entry::item("Old", "", Target::Folder(Vec::new()))]
        );
    }

    #[test]
    fn a_rewrite_keeps_the_comment_above_the_block_and_the_keys_around_it() {
        let text = "\
terminal = \"foot\"

# The Start menu, top to bottom.
[[startmenu]]
label = \"A\"
command = \"a\"   # runs a

[[startmenu]]
label = \"B\"
command = \"b\"

[fonts]
size_px = 12
";
        let mut menu = load(text);
        shift(&mut menu, &[1], -1).unwrap();
        assert_eq!(
            saved(text, &menu),
            "\
terminal = \"foot\"

# The Start menu, top to bottom.
[[startmenu]]
label = \"B\"
command = \"b\"

[[startmenu]]
label = \"A\"
command = \"a\"

[fonts]
size_px = 12
"
        );
    }

    #[test]
    fn the_default_menu_is_written_as_no_block_at_all() {
        let text = "[[startmenu]]\nlabel = \"A\"\ncommand = \"a\"\n";
        assert_eq!(saved(text, &default_menu()), "");
        assert_eq!(
            saved(text, &load(text)),
            text,
            "an unchanged menu leaves the file alone"
        );
    }

    #[test]
    fn rows_move_within_their_folder_and_stop_at_its_ends() {
        let mut m = default_menu();
        assert_eq!(shift(&mut m, &[0], -1), None);
        assert_eq!(shift(&mut m, &[8], 1), None);
        assert_eq!(shift(&mut m, &[2, 0], 1), Some(vec![2, 1]));
        assert_eq!(get(&m, &[2, 0]), Some(&Entry::Separator));
        assert_eq!(
            get(&m, &[2, 1]).and_then(Entry::label),
            Some("&Control Panel")
        );
        assert_eq!(shift(&mut m, &[0], 1), Some(vec![1]));
        assert_eq!(get(&m, &[0]).and_then(Entry::label), Some("&Documents"));
    }

    #[test]
    fn a_new_row_goes_into_a_picked_folder_or_after_the_picked_row() {
        let m = default_menu();
        assert_eq!(insertion(&m, None), vec![9], "the menu's end");
        assert_eq!(insertion(&m, Some(&[])), vec![9]);
        assert_eq!(insertion(&m, Some(&[2])), vec![2, 5], "into Settings");
        assert_eq!(
            insertion(&m, Some(&[2, 1])),
            vec![2, 2],
            "after the separator"
        );
        assert_eq!(
            insertion(&m, Some(&[0])),
            vec![1],
            "Programs is filled by the compositor, so after it"
        );
    }

    #[test]
    fn take_put_and_rename_work_at_any_depth() {
        let mut m = default_menu();
        let e = take(&mut m, &[3, 1]).unwrap();
        assert_eq!(e.label(), Some("On the &Internet..."));
        put(&mut m, &[0], e).unwrap();
        assert_eq!(m.len(), 10);
        assert_eq!(get(&m, &[4]).and_then(Entry::items).map(Vec::len), Some(1));
        assert!(rename(&mut m, &[4, 0], "&Files"));
        assert_eq!(get(&m, &[4, 0]).and_then(Entry::label), Some("&Files"));
        assert!(!rename(&mut m, &[7], "x"), "a separator has no label");
        assert_eq!(
            put(&mut m, &[1, 0], Entry::Separator),
            None,
            "Programs holds no rows"
        );
    }
}
