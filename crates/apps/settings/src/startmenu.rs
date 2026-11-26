//! The Start menu as the Start Menu page edits it: a tree of entries read out of the file and
//! written back as `[[startmenu_changes]]`, what sets it apart from the default menu. A file
//! listing the whole menu as `[[startmenu]]` keeps it whole: its rows name no default row, so
//! they cannot be told apart from an older default's.
//!
//! The default menu is `repriseconf`'s, which the compositor reads too; a test holds it to
//! `config.example.toml`'s block.
//!
//! The changes for a folder are the default rows it leaves out, then its rows in order. The
//! longest run of default rows still in their default order stays put; every other default row
//! and each row of the user's own goes after the staying row above it (`after = ""` for the
//! folder's top). A change the menu does not show (one that does not read or that Start cannot
//! build, or one naming a row this build lacks or put inside a folder the written changes hide)
//! is written back as the file had it, after the rest.
//!
//! Each table written goes over the file's table for the same row, so the comments on its lines
//! stay.

use repriseconf::config::file::FileMenuEntry;
use repriseconf::{Key, key as k, remove};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, TableLike};

pub const STARTMENU: Key = k("", "startmenu");
pub const CHANGES: Key = k("", "startmenu_changes");

/// One row of the Start menu or of a folder in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A line; `id` as an item's.
    Separator { id: Option<String> },
    Item {
        /// The default row the entry stands for; None for a row of the user's own.
        id: Option<String>,
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
            id: None,
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
            Entry::Separator { .. } => None,
        }
    }

    pub fn icon(&self) -> Option<&str> {
        match self {
            Entry::Item { icon, .. } => icon.as_deref(),
            Entry::Separator { .. } => None,
        }
    }

    fn id(&self) -> Option<&str> {
        match self {
            Entry::Item { id, .. } | Entry::Separator { id } => id.as_deref(),
        }
    }

    /// The entry as a `[[startmenu]]` file lists it, with no default row's id.
    fn plain(&self) -> Entry {
        match self {
            Entry::Separator { .. } => Entry::Separator { id: None },
            Entry::Item {
                label,
                icon,
                tip,
                target,
                ..
            } => Entry::Item {
                id: None,
                label: label.clone(),
                icon: icon.clone(),
                tip: tip.clone(),
                target: match target {
                    Target::Folder(v) => Target::Folder(v.iter().map(Entry::plain).collect()),
                    t => t.clone(),
                },
            },
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
    (repriseconf::config::default_startmenu_entries().iter())
        .map(from_file)
        .collect()
}

/// `f` as [`read_entry`] reads its table.
fn from_file(f: &FileMenuEntry) -> Entry {
    if f.separator == Some(true) {
        return Entry::Separator { id: f.id.clone() };
    }
    let target = match (&f.action, &f.command, &f.submenu, &f.items) {
        (Some(a), c, _, _) => Target::Action(a.clone(), c.clone()),
        (None, Some(c), _, _) => Target::Command(c.clone()),
        (None, None, Some(s), _) => Target::Special(s.clone()),
        (None, None, None, items) => {
            Target::Folder(items.iter().flatten().map(from_file).collect())
        }
    };
    Entry::Item {
        id: f.id.clone(),
        label: f.label.clone().unwrap_or_default(),
        icon: f.icon.clone(),
        tip: f.tip.clone(),
        target,
    }
}

/// The file's `[[startmenu]]` (or an inline `startmenu = [{...}]`), else the default menu with
/// its `[[startmenu_changes]]` made; None when it has neither.
pub fn read(doc: &DocumentMut) -> Option<Vec<Entry>> {
    if let Some(item) = repriseconf::get(doc, &STARTMENU) {
        return read_list(item);
    }
    repriseconf::get(doc, &CHANGES)?;
    let changes: Vec<_> = (repriseconf::config::startmenu_changes_in(&doc.to_string())?)
        .into_iter()
        .flatten()
        .collect();
    Some(
        (repriseconf::config::startmenu_with(&changes).iter())
            .map(from_file)
            .collect(),
    )
}

pub(crate) fn tables(item: &Item) -> Option<Vec<&dyn TableLike>> {
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
        return Some(Entry::Separator { id: None });
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
        id: None,
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
        ..
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

/// Write `want` into `doc` when the sheet moved it: not when it equals `base` (what the sheet
/// loaded) or `have` (what the file says now). A whole `[[startmenu]]` list stays whole.
pub fn write(doc: &mut DocumentMut, want: &[Entry], base: &[Entry], have: &[Entry]) {
    if want == base || want == have {
        return;
    }
    let whole = repriseconf::get(doc, &STARTMENU).is_some();
    if whole && plain(want) != plain(&default_menu()) {
        let old = array(doc, &STARTMENU);
        let mut tables = rewritten(&old, want);
        let root = doc.as_table_mut();
        let head = match old.first() {
            Some(t) => prefix(t.decor()),
            None => (root.key(STARTMENU.name)).map_or_else(String::new, |k| prefix(k.leaf_decor())),
        };
        if tables.is_empty() {
            // Without the key Start shows the default menu, so an emptied list stays as `[]`.
            root.insert(STARTMENU.name, toml_edit::value(toml_edit::Array::new()));
            if let Some(mut key) = root.key_mut(STARTMENU.name) {
                key.leaf_decor_mut().set_prefix(head);
            }
            return;
        }
        if old.is_empty() && !head.is_empty() {
            tables[0].decor_mut().set_prefix(head);
            // A header prints its key's decor, so the comment would head every table.
            if let Some(mut key) = doc.as_table_mut().key_mut(STARTMENU.name) {
                key.leaf_decor_mut().clear();
            }
        }
        set_tables(doc, &STARTMENU, tables);
        return;
    }
    if !in_blocks(doc, &CHANGES) {
        log::warn!("startmenu_changes lists something no table; the Start menu is not saved");
        return;
    }
    let old = repriseconf::get(doc, &CHANGES);
    let listed = old.is_none_or(Item::is_array_of_tables);
    // A lone `[startmenu_changes]` table goes first, as the one entry it was likely meant to be;
    // a value that is no table or list stays as the file has it, and so does the menu.
    let lone = match old.filter(|_| !listed).map(lone_table) {
        Some(None) => {
            log::warn!("startmenu_changes is no list of tables; the Start menu is not saved");
            return;
        }
        lone => lone.flatten(),
    };
    // Back to the default menu, a whole list takes the changes it kept from applying with it.
    let made = if whole { Vec::new() } else { changes(want) };
    let hidden: Vec<String> = (made.iter())
        .filter(|t| t.get("hidden").and_then(Item::as_bool) == Some(true))
        .filter_map(|t| t.get("default")?.as_str().map(str::to_string))
        .collect();
    let old = array(doc, &CHANGES);
    let idle = if listed {
        idle_tables(doc, &hidden)
    } else {
        Vec::new()
    };
    let free: Vec<Option<&Table>> = (old.iter().enumerate())
        .map(|(i, t)| (!idle.iter().any(|(j, _)| *j == i)).then_some(t))
        .collect();
    let made_from = paired(&free, &made, 3, |pass, o, n| {
        o.is_some_and(|o| same_change(pass, o, n))
    });
    let from: Vec<Option<usize>> = (lone.iter().map(|_| None))
        .chain(made_from.iter().copied())
        .chain(idle.iter().map(|(i, _)| Some(*i)))
        .collect();
    let mut tables: Vec<Table> = (lone.into_iter())
        .chain(made.into_iter().zip(&made_from).map(|(n, f)| match f {
            Some(j) => merged(&old[*j], n),
            None => n,
        }))
        .chain(idle.into_iter().map(|(_, t)| t))
        .collect();
    keep_head(&old, &from, &mut tables);
    remove(doc, &STARTMENU);
    set_tables(doc, &CHANGES, tables);
}

/// The array of tables `key`, empty when the file has none.
pub(crate) fn array(doc: &DocumentMut, key: &Key) -> Vec<Table> {
    (repriseconf::get(doc, key))
        .and_then(Item::as_array_of_tables)
        .map_or_else(Vec::new, |a| a.iter().cloned().collect())
}

fn prefix(d: &toml_edit::Decor) -> String {
    let p = d.prefix().and_then(|p| p.as_str());
    p.unwrap_or_default().to_string()
}

/// For each of `want`, the index of the item in `old` it takes the place of: the first one not
/// taken that `same` matches in the lowest of `passes` passes.
pub(crate) fn paired<O, W>(
    old: &[O],
    want: &[W],
    passes: usize,
    same: impl Fn(usize, &O, &W) -> bool,
) -> Vec<Option<usize>> {
    let mut from: Vec<Option<usize>> = vec![None; want.len()];
    let mut used = vec![false; old.len()];
    for pass in 0..passes {
        for (w, f) in want.iter().zip(&mut from).filter(|(_, f)| f.is_none()) {
            *f = (0..old.len()).find(|&j| !used[j] && same(pass, &old[j], w));
            if let Some(j) = *f {
                used[j] = true;
            }
        }
    }
    from
}

/// Whether `r` is `w`'s row in `pass`: 0 reads as it, 1 has its label, 2 its command, action or
/// submenu.
fn alike(pass: usize, r: &Entry, w: &Entry) -> bool {
    let target = |e: &Entry| match e {
        Entry::Item { target, .. } if !matches!(target, Target::Folder(_)) => Some(target.clone()),
        _ => None,
    };
    match pass {
        0 => r == w,
        1 => r.label().is_some() && r.label() == w.label(),
        _ => target(r).is_some() && target(r) == target(w),
    }
}

/// Whether the `[[startmenu_changes]]` table `old` is the one `new` rewrites in `pass`: a change to
/// the same default row, or a row of the user's own in the same folder that is [`alike`].
fn same_change(pass: usize, old: &Table, new: &Table) -> bool {
    let text = |t: &Table, k: &str| t.get(k).and_then(Item::as_str).map(str::to_string);
    match (text(old, "default"), text(new, "default")) {
        (Some(a), Some(b)) => a == b,
        (None, None) => {
            text(old, "folder") == text(new, "folder")
                && (read_entry(old).zip(read_entry(new))).is_some_and(|(r, w)| alike(pass, &r, &w))
        }
        _ => false,
    }
}

/// `want` as `[[startmenu]]` tables, each written over the file's table for its row in `old` so
/// the comments on its lines stay. A row's table is the one that reads as the row, else one of
/// its label, else one of its command, action or submenu. The comment above the first table heads
/// the block and stays on top.
fn rewritten(old: &[Table], want: &[Entry]) -> Vec<Table> {
    let read: Vec<Option<Entry>> = old.iter().map(|t| read_entry(t)).collect();
    let from = paired(&read, want, 3, |pass, r, w| {
        r.as_ref().is_some_and(|r| alike(pass, r, w))
    });
    let mut out: Vec<Table> = (want.iter().zip(&from))
        .map(|(w, f)| match f {
            Some(j) => merged(&old[*j], table(w)),
            None => table(w),
        })
        .collect();
    keep_head(old, &from, &mut out);
    out
}

/// Keeps the comment above `old`'s first table on top of `out`, the tables written over `old`
/// (`from` names each one's table in `old`). The old first table, written lower down, takes the
/// spacing the new first one had, and keeps the part of that comment after its last blank line
/// when a comment comes before it too.
pub(crate) fn keep_head(old: &[Table], from: &[Option<usize>], out: &mut [Table]) {
    let first = from.iter().position(|&f| f == Some(0));
    let Some(o) = old.first().filter(|_| !out.is_empty() && first != Some(0)) else {
        return;
    };
    let own = out[0].decor().prefix().map(|_| prefix(out[0].decor()));
    let mut head = prefix(o.decor());
    if let Some(k) = first {
        let lines: Vec<&str> = head.split_inclusive('\n').collect();
        let blank = lines.iter().rposition(|l| l.trim().is_empty());
        let (above, mine) = lines.split_at(blank.map_or(0, |b| b + 1));
        let comment = |ls: &[&str]| ls.iter().any(|l| l.trim_start().starts_with('#'));
        let mine = match comment(above) && comment(mine) {
            true => mine.concat(),
            false => String::new(),
        };
        head.truncate(head.len() - mine.len());
        let d = out[k].decor_mut();
        match (&own, mine.is_empty()) {
            (Some(own), _) => d.set_prefix(
                (own.split_inclusive('\n'))
                    .filter(|l| !l.trim_start().starts_with('#'))
                    .collect::<String>()
                    + &mine,
            ),
            (None, false) => d.set_prefix("\n".to_string() + &mine),
            (None, true) => {
                let suffix = d.suffix().cloned();
                d.clear();
                if let Some(s) = suffix {
                    d.set_suffix(s);
                }
            }
        }
    }
    let own = own.unwrap_or_default();
    if own.contains('#') {
        head += &own;
    }
    out[0].decor_mut().set_prefix(head);
}

/// `new` written over `old`, the table it replaces: keys `new` lacks go, a value `new` changes
/// keeps the comment beside it, keys `new` adds follow the rest, and a folder's items are
/// [`rewritten`] over the old ones.
fn merged(old: &Table, new: Table) -> Table {
    let mut t = old.clone();
    t.set_position(None);
    t.retain(|k, _| new.contains_key(k));
    for (k, v) in new {
        match (t.get_mut(&k), v) {
            (Some(Item::Value(o)), Item::Value(mut n)) => {
                let bare = |v: &toml_edit::Value| {
                    let mut v = v.clone();
                    v.decor_mut().clear();
                    v.to_string()
                };
                if bare(o) != bare(&n) {
                    *n.decor_mut() = o.decor().clone();
                    *o = n;
                }
            }
            (Some(Item::ArrayOfTables(o)), Item::ArrayOfTables(n)) => {
                let kids: Vec<Table> = o.iter().cloned().collect();
                let items = read_list(&Item::ArrayOfTables(n)).unwrap_or_default();
                *o = rewritten(&kids, &items).into_iter().collect();
            }
            (Some(slot), v) => *slot = v,
            (None, v) => {
                t.insert(&k, v);
            }
        }
    }
    t
}

/// Rewrites an inline `key = [{...}]` as `[[key]]` tables, the comment above the key heading the
/// first; false, the file left as it is, when it lists something else.
pub(crate) fn in_blocks(doc: &mut DocumentMut, key: &Key) -> bool {
    let Some(Item::Value(toml_edit::Value::Array(a))) = doc.get(key.name) else {
        return true;
    };
    if !a.iter().all(toml_edit::Value::is_inline_table) {
        return false;
    }
    let root = doc.as_table_mut();
    let head = (root.key(key.name)).map_or_else(String::new, |k| prefix(k.leaf_decor()));
    let Some(Item::Value(toml_edit::Value::Array(a))) = root.remove(key.name) else {
        return true;
    };
    let mut tables: ArrayOfTables = (a.into_iter())
        .filter_map(|v| lone_table(&Item::Value(v)))
        .collect();
    if let Some(t) = tables.get_mut(0) {
        t.decor_mut().set_prefix(head);
    }
    root.insert(key.name, Item::ArrayOfTables(tables));
    true
}

/// `item`, a `[startmenu_changes]` table, as an entry of `[[startmenu_changes]]`.
fn lone_table(item: &Item) -> Option<Table> {
    let mut t = match item {
        Item::Table(t) => t.clone(),
        Item::Value(toml_edit::Value::InlineTable(t)) => t.clone().into_table(),
        _ => return None,
    };
    t.set_implicit(false);
    t.set_dotted(false);
    t.set_position(None);
    Some(t)
}

/// The file's `[[startmenu_changes]]` the menu does not show next to changes hiding the default
/// rows `hidden`, as the file has them, with their places in its list.
fn idle_tables(doc: &DocumentMut, hidden: &[String]) -> Vec<(usize, Table)> {
    let Some(item) = repriseconf::get(doc, &CHANGES) else {
        return Vec::new();
    };
    let read = repriseconf::config::startmenu_changes_in(&doc.to_string()).unwrap_or_default();
    let good: Vec<_> = read.iter().flatten().cloned().collect();
    let mut idle = repriseconf::config::idle_changes(&good, hidden).into_iter();
    let keep: Vec<bool> = (read.iter())
        .map(|c| c.is_none() || idle.next() == Some(true))
        .collect();
    let raw = item.as_array_of_tables().into_iter().flatten();
    (raw.zip(keep).enumerate())
        .filter(|(_, (_, keep))| *keep)
        .map(|(i, (t, _))| (i, t.clone()))
        .collect()
}

/// `tables` as the array `key`, taken out when empty.
pub(crate) fn set_tables(doc: &mut DocumentMut, key: &Key, tables: Vec<Table>) {
    if tables.is_empty() {
        remove(doc, key);
        return;
    }
    let mut a = ArrayOfTables::new();
    for t in tables {
        a.push(t);
    }
    doc[key.name] = Item::ArrayOfTables(a);
}

fn plain(entries: &[Entry]) -> Vec<Entry> {
    entries.iter().map(Entry::plain).collect()
}

/// `want` as `[[startmenu_changes]]` tables against the default menu.
fn changes(want: &[Entry]) -> Vec<Table> {
    let mut out = Vec::new();
    diff("", &default_menu(), want, &mut out);
    out
}

/// The changes turning the default folder `folder` holding `defaults` into `want`, as the module
/// header says.
fn diff(folder: &str, defaults: &[Entry], want: &[Entry], out: &mut Vec<Table>) {
    let mut seen = Vec::new();
    let found: Vec<Option<usize>> = (want.iter())
        .map(|w| {
            let i = (defaults.iter()).position(|d| d.id().is_some() && d.id() == w.id())?;
            let fits = match (&defaults[i], w) {
                (Entry::Item { target: d, .. }, Entry::Item { target: t, .. }) => {
                    matches!((d, t), (Target::Folder(_), Target::Folder(_))) || d == t
                }
                (d, t) => d.label().is_none() && t.label().is_none(),
            };
            (fits && !seen.contains(&i)).then(|| {
                seen.push(i);
                i
            })
        })
        .collect();
    let staying = increasing(&found);
    for (i, d) in defaults.iter().enumerate() {
        if let Some(id) = d.id().filter(|_| !seen.contains(&i)) {
            let mut t = Table::new();
            t["default"] = toml_edit::value(id);
            t["hidden"] = toml_edit::value(true);
            out.push(t);
        }
    }
    let mut anchor = "";
    for ((w, at), stays) in want.iter().zip(&found).zip(staying) {
        let Some(d) = at.map(|i| &defaults[i]) else {
            let mut t = Table::new();
            if !folder.is_empty() {
                t["folder"] = toml_edit::value(folder);
            }
            t["after"] = toml_edit::value(anchor);
            for (k, v) in table(w).iter() {
                t.insert(k, v.clone());
            }
            out.push(t);
            continue;
        };
        let id = d.id().unwrap_or_default();
        let mut t = Table::new();
        t["default"] = toml_edit::value(id);
        if let (
            Entry::Item {
                label, icon, tip, ..
            },
            Entry::Item {
                label: l,
                icon: i,
                tip: p,
                ..
            },
        ) = (d, w)
        {
            if l != label {
                t["label"] = toml_edit::value(l);
            }
            for (key, mine, theirs) in [("icon", i, icon), ("tip", p, tip)] {
                if let Some(v) = mine.as_ref().filter(|v| Some(*v) != theirs.as_ref()) {
                    t[key] = toml_edit::value(v);
                }
            }
        }
        if stays {
            anchor = id;
        } else {
            t["after"] = toml_edit::value(anchor);
        }
        if t.len() > 1 {
            out.push(t);
        }
        if let (Some(dk), Some(wk)) = (d.items(), w.items()) {
            diff(id, dk, wk, out);
        }
    }
}

/// Which of `found`'s default rows stay put: the longest run of them in increasing order.
fn increasing(found: &[Option<usize>]) -> Vec<bool> {
    let n = found.len();
    let mut len = vec![0usize; n];
    let mut prev = vec![None; n];
    for j in 0..n {
        let Some(v) = found[j] else { continue };
        len[j] = 1;
        for i in 0..j {
            if found[i].is_some_and(|u| u < v) && len[i] + 1 > len[j] {
                len[j] = len[i] + 1;
                prev[j] = Some(i);
            }
        }
    }
    let mut out = vec![false; n];
    let mut at = (0..n).filter(|&j| len[j] > 0).max_by_key(|&j| len[j]);
    while let Some(j) = at {
        out[j] = true;
        at = prev[j];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(text: &str) -> Vec<Entry> {
        read(&text.parse().unwrap()).unwrap_or_else(default_menu)
    }

    /// `menu` written into a file that lists the whole menu.
    fn whole(menu: &[Entry]) -> String {
        let mut doc: DocumentMut = "[[startmenu]]\nlabel = \"x\"\n".parse().unwrap();
        write(&mut doc, menu, &[], &[]);
        doc.to_string()
    }

    fn saved(text: &str, want: &[Entry]) -> String {
        let mut doc: DocumentMut = text.parse().unwrap();
        let have = load(text);
        write(&mut doc, want, &have, &have);
        doc.to_string()
    }

    #[test]
    fn network_connections_opens_reprise_network() {
        let m = default_menu();
        assert!(matches!(
            get(&m, &[2, 2]),
            Some(Entry::Item { label, target: Target::Command(c), .. })
                if label == "&Network Connections" && c == "reprise-network"
        ));
    }

    fn ids(entries: &[Entry], out: &mut Vec<String>) {
        for e in entries {
            out.extend(e.id().map(str::to_string));
            ids(e.items().map_or(&[][..], Vec::as_slice), out);
        }
    }

    #[test]
    fn the_default_menu_is_the_example_files() {
        let example = include_str!("../../../../config.example.toml");
        let block: String = (example.lines())
            .skip_while(|l| !l.starts_with("#[[startmenu]]"))
            .take_while(|l| l.starts_with('#'))
            .map(|l| format!("{}\n", &l[1..]))
            .collect();
        let doc: DocumentMut = block.parse().unwrap();
        assert_eq!(read(&doc), Some(plain(&default_menu())));
        let named: Vec<&str> = (block.lines())
            .filter(|l| l.starts_with("[["))
            .filter_map(|l| l.split("# ").nth(1))
            .collect();
        let mut want = Vec::new();
        ids(&default_menu(), &mut want);
        assert_eq!(named, want, "each header names its row's id");
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
                Entry::Separator { id: None },
                Entry::item(
                    "Box",
                    "folder",
                    Target::Folder(vec![
                        Entry::Item {
                            id: None,
                            label: "Edit".into(),
                            icon: None,
                            tip: Some("Edits text".into()),
                            target: Target::Command("gedit".into()),
                        },
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
        assert_eq!(whole(&menu), text);
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
        let text = whole(&menu);
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
command = \"a\"   # runs a

[fonts]
size_px = 12
"
        );
    }

    #[test]
    fn a_rewritten_list_keeps_each_rows_comments_and_its_spacing() {
        let text = "\
# Mine
[[startmenu]]
label = \"A\" # first
command = \"a\"

# The games
[[startmenu]]
label = \"Games\"
[[startmenu.items]]
label = \"Mines\"
command = \"mines\" # the classic
[[startmenu.items]]
separator = true # a line
[[startmenu.items]]
label = \"Cards\"
command = \"cards\"
";
        let mut menu = load(text);
        shift(&mut menu, &[1], -1).unwrap();
        rename(&mut menu, &[0, 2], "Solitaire");
        shift(&mut menu, &[0, 2], -2).unwrap();
        rename(&mut menu, &[1], "B");
        assert_eq!(
            saved(text, &menu),
            "\
# Mine

# The games
[[startmenu]]
label = \"Games\"
[[startmenu.items]]
label = \"Solitaire\"
command = \"cards\"
[[startmenu.items]]
label = \"Mines\"
command = \"mines\" # the classic
[[startmenu.items]]
separator = true # a line

[[startmenu]]
label = \"B\" # first
command = \"a\"
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
        assert_eq!(
            get(&m, &[2, 0]).and_then(Entry::id),
            Some("settings-separator")
        );
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
            put(&mut m, &[1, 0], Entry::Separator { id: None }),
            None,
            "Programs holds no rows"
        );
    }

    #[test]
    fn an_edit_saves_only_its_changes_and_reads_back() {
        let mut m = default_menu();
        take(&mut m, &[4]).unwrap();
        shift(&mut m, &[4], -4).unwrap();
        assert!(rename(&mut m, &[4, 0], "&Files..."));
        put(
            &mut m,
            &[3, 4],
            Entry::program("Scan", "scanner", "simple-scan".into()),
        )
        .unwrap();
        let end = m.len();
        put(
            &mut m,
            &[end],
            Entry::item("New Folder", "", Target::Folder(Vec::new())),
        )
        .unwrap();
        let text = saved("", &m);
        assert_eq!(
            text,
            r#"[[startmenu_changes]]
default = "help"
hidden = true

[[startmenu_changes]]
default = "run"
after = ""

[[startmenu_changes]]
folder = "settings"
after = "printers"
label = "Scan"
icon = "scanner"
command = "simple-scan"

[[startmenu_changes]]
default = "search-files"
label = "&Files..."

[[startmenu_changes]]
after = "turnoff"
label = "New Folder"
items = []
"#
        );
        assert_eq!(load(&text), m);
        assert_eq!(
            saved(&text, &m),
            text,
            "an unchanged menu leaves the file alone"
        );
        assert_eq!(
            saved(&text, &default_menu()),
            "",
            "the default menu takes the key out"
        );
    }

    /// The paths of every folder of entries in `entries`, default or the user's own.
    fn folders(entries: &[Entry], at: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        for (i, e) in entries.iter().enumerate() {
            if let Some(items) = e.items() {
                at.push(i);
                out.push(at.clone());
                folders(items, at, out);
                at.pop();
            }
        }
    }

    fn default_label(id: Option<&str>) -> Option<String> {
        let mut found = None;
        fn find(entries: &[Entry], id: &str, found: &mut Option<String>) {
            for e in entries {
                if e.id() == Some(id) {
                    *found = e.label().map(str::to_string);
                }
                find(e.items().map_or(&[][..], Vec::as_slice), id, found);
            }
        }
        find(&default_menu(), id?, &mut found);
        found
    }

    fn edit(m: &mut Vec<Entry>, roll: &mut impl FnMut(usize) -> usize) {
        let mut all = vec![vec![]];
        folders(m, &mut Vec::new(), &mut all);
        let folder = all[roll(all.len())].clone();
        let Some(len) = level(m, &folder).map(|v| v.len()) else {
            return;
        };
        let mut at = folder.clone();
        at.push(roll(len + 1));
        match roll(8) {
            0 => drop(take(m, &at)),
            1 => drop(shift(m, &at, if roll(2) == 0 { -1 } else { 2 })),
            2 => {
                let back = get(m, &at).and_then(|e| default_label(e.id()));
                let to = back.filter(|_| roll(2) == 0);
                rename(m, &at, to.as_deref().unwrap_or("Re&named"));
            }
            3 | 4 => drop(put(m, &at, Entry::program("Mine", "", "mine".into()))),
            5 => drop(put(
                m,
                &at,
                Entry::item("New Folder", "", Target::Folder(Vec::new())),
            )),
            6 if roll(4) == 0 => level(m, &folder).unwrap().clear(),
            _ => drop(put(m, &at, Entry::program("Other", "x", "other".into()))),
        }
    }

    #[test]
    fn any_run_of_edits_reads_back_as_it_was_saved() {
        let mut seed = 7u32;
        let mut roll = |n: usize| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 16) as usize % n.max(1)
        };
        for run in 0..300 {
            let mut m = default_menu();
            if run % 10 == 0 {
                shift(&mut m, &[4], 1).unwrap();
                shift(&mut m, &[5], -1).unwrap();
                rename(&mut m, &[2, 0], "&Control Panel");
                assert_eq!(changes(&m).len(), 0, "a run undoing itself writes nothing");
            }
            for _ in 0..12 {
                edit(&mut m, &mut roll);
            }
            let text = saved("", &m);
            assert_eq!(load(&text), m, "{text}");
            let mut again = load(&text);
            for _ in 0..6 {
                edit(&mut again, &mut roll);
            }
            let text2 = saved(&text, &again);
            assert_eq!(load(&text2), again, "saved twice:\n{text}\nthen\n{text2}");
            assert_eq!(saved(&text2, &default_menu()), "", "{text2}");
        }
    }

    /// `text` with a numbered comment beside every value and above every table that has none.
    fn commented(text: &str, n: &mut usize) -> String {
        let mut out = String::new();
        for line in text.lines() {
            *n += 1;
            if line.starts_with("[[") {
                out += &format!("# h{n}\n{line}\n");
            } else if line.contains(" = ") && !line.contains('#') {
                out += &format!("{line} # c{n}\n");
            } else {
                out += &format!("{line}\n");
            }
        }
        out
    }

    fn each_comment_once(text: &str) {
        let mut seen = Vec::new();
        for c in text
            .lines()
            .filter_map(|l| l.split_once('#'))
            .map(|(_, c)| c)
        {
            assert!(!seen.contains(&c), "#{c} twice in\n{text}");
            seen.push(c);
        }
    }

    #[test]
    fn any_run_of_edits_on_a_whole_list_reads_back_as_it_was_saved() {
        let mut seed = 11u32;
        let mut roll = |n: usize| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 16) as usize % n.max(1)
        };
        let mut n = 0;
        for _ in 0..100 {
            let mut text = commented(&whole(&plain(&default_menu())[1..]), &mut n);
            for _ in 0..4 {
                let mut again = load(&text);
                for _ in 0..4 {
                    edit(&mut again, &mut roll);
                }
                if roll(6) == 0 {
                    again.clear();
                }
                if plain(&again) == plain(&default_menu()) {
                    break;
                }
                let next = saved(&text, &again);
                assert_eq!(load(&next), again, "saved over:\n{text}\nas\n{next}");
                each_comment_once(&next);
                text = commented(&next, &mut n);
            }
        }
    }

    #[test]
    fn any_run_of_edits_on_changes_keeps_their_comments_and_reads_back() {
        let mut seed = 5u32;
        let mut roll = |n: usize| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 16) as usize % n.max(1)
        };
        let mut n = 0;
        for _ in 0..100 {
            let mut m = default_menu();
            for _ in 0..8 {
                edit(&mut m, &mut roll);
            }
            let mut text = commented(&saved("", &m), &mut n);
            for _ in 0..4 {
                let mut again = load(&text);
                for _ in 0..4 {
                    edit(&mut again, &mut roll);
                }
                let next = saved(&text, &again);
                assert_eq!(load(&next), again, "saved over:\n{text}\nas\n{next}");
                each_comment_once(&next);
                for line in text.lines().filter(|l| l.starts_with("default = ")) {
                    let (id, c) = line.split_once(" #").unwrap();
                    if next.contains(&format!("{id} ")) || next.contains(&format!("{id}\n")) {
                        let kept = next.contains(&format!("#{c}\n"));
                        assert!(kept, "{line} lost its comment:\n{text}\nas\n{next}");
                    }
                }
                text = commented(&next, &mut n);
            }
        }
    }

    #[test]
    fn an_emptied_whole_list_stays_an_empty_menu() {
        let text = "\
terminal = \"foot\"

# My menu
[[startmenu]]
label = \"A\"
command = \"a\"

[fonts]
size_px = 12
";
        let out = saved(text, &[]);
        assert_eq!(
            out,
            "\
terminal = \"foot\"

# My menu
startmenu = []

[fonts]
size_px = 12
"
        );
        assert_eq!(load(&out), Vec::new());
        let back = load(text);
        let again = saved(&out, &back);
        assert!(again.contains("# My menu\n[[startmenu]]\n"), "{again}");
        assert_eq!(load(&again), back);
    }

    #[test]
    fn a_saved_changes_rows_comments_stay_with_it() {
        let mut m = default_menu();
        take(&mut m, &[4]).unwrap();
        rename(&mut m, &[3, 0], "&Files...");
        let end = m.len();
        put(&mut m, &[end], Entry::program("Mine", "", "mine".into())).unwrap();
        let text = saved("", &m)
            .replacen("[[", "# My changes\n[[", 1)
            .replace("\"help\"", "\"help\" # never read")
            .replace("\"&Files...\"", "\"&Files...\" # shorter")
            .replace("\"mine\"", "\"mine\" # my tool")
            .replace(
                "[[startmenu_changes]]\nafter = \"turnoff\"",
                "# mine\n[[startmenu_changes]]\nafter = \"turnoff\"",
            );
        let mut m = load(&text);
        take(&mut m, &[1]).unwrap();
        rename(&mut m, &[2, 0], "&Find...");
        rename(&mut m, &[7], "Mine 2");
        let out = saved(&text, &m);
        assert_eq!(
            out,
            r#"# My changes
[[startmenu_changes]]
default = "documents"
hidden = true

[[startmenu_changes]]
default = "help" # never read
hidden = true

[[startmenu_changes]]
default = "search-files"
label = "&Find..." # shorter

# mine
[[startmenu_changes]]
after = "turnoff"
label = "Mine 2"
command = "mine" # my tool
"#
        );
        assert_eq!(load(&out), m);
    }

    #[test]
    fn a_whole_list_back_at_the_default_menu_takes_the_changes_it_held_back_too() {
        let mut doc = DocumentMut::new();
        let mut a = ArrayOfTables::new();
        for e in default_menu() {
            a.push(table(&e));
        }
        doc["startmenu"] = Item::ArrayOfTables(a);
        let list = format!("{doc}\n[[startmenu]]\nlabel = \"Mine\"\ncommand = \"mine\"\n");
        let bad = "[[startmenu_changes]]\ndefault = \"run\"\ncommand = \"x\"\n";
        let text = format!(
            "[[startmenu_changes]]\ndefault = \"help\"\nhidden = true\n\n\
             [[startmenu_changes]]\ndefault = \"search\"\nhidden = true\n\n\
             [[startmenu_changes]]\ndefault = \"search-files\"\nlabel = \"Files\"\n\n{bad}\n{list}"
        );
        let mut m = load(&text);
        m.pop();
        assert_eq!(
            saved(&text, &m),
            bad,
            "the hidden Help and Search, and the label inside Search, would make Start differ from \
             the page; the change that does not read stays"
        );
    }

    fn without(menu: &[Entry], id: &str) -> Vec<Entry> {
        let mut m = menu.to_vec();
        m.retain(|e| e.id() != Some(id));
        m
    }

    fn count(text: &str, line: &str) -> usize {
        text.lines().filter(|l| *l == line).count()
    }

    #[test]
    fn a_change_inside_a_folder_the_page_hides_survives_an_outside_edit() {
        let base = without(&default_menu(), "search");
        let want = without(&base, "help");
        let text = "[[startmenu_changes]]\ndefault = \"search-files\"\nlabel = \"Files\"\n";
        let mut doc: DocumentMut = text.parse().unwrap();
        write(&mut doc, &want, &base, &load(text));
        let out = doc.to_string();
        assert_eq!(count(&out, "default = \"search\""), 1, "{out}");
        assert_eq!(count(&out, "label = \"Files\""), 1, "{out}");
        assert_eq!(load(&out), want, "{out}");
    }

    #[test]
    fn a_change_the_page_shows_after_an_outside_edit_is_written_once() {
        let text = "[[startmenu_changes]]\ndefault = \"search\"\nhidden = true\n\n\
                    [[startmenu_changes]]\ndefault = \"search-files\"\nlabel = \"Files\"\n";
        let mut want = without(&default_menu(), "help");
        rename(&mut want, &[3, 0], "Files");
        let mut doc: DocumentMut = text.parse().unwrap();
        write(&mut doc, &want, &default_menu(), &load(text));
        let out = doc.to_string();
        assert_eq!(count(&out, "default = \"search-files\""), 1, "{out}");
        assert_eq!(count(&out, "default = \"search\""), 0, "{out}");
        assert_eq!(load(&out), want, "{out}");
    }

    #[test]
    fn a_row_start_cannot_build_is_kept_as_the_file_has_it() {
        let refused = [
            "[[startmenu_changes]]\nlabel = \"P\"\nsubmenu = \"programs\"\n\n[[startmenu_changes.items]]\nlabel = \"A\"\ncommand = \"a\"\n",
            "[[startmenu_changes]]\nlabel = \"C\"\ncommand = \"c\"\n\n[[startmenu_changes.items]]\nlabel = \"A\"\ncommand = \"a\"\n",
            "[[startmenu_changes]]\ncommand = \"c\"\n",
            "[[startmenu_changes]]\nlabel = \"B\"\naction = \"bogus\"\n",
        ];
        for bad in refused {
            assert_eq!(load(bad), default_menu(), "{bad}");
            let out = saved(bad, &without(&default_menu(), "help"));
            assert_eq!(
                out,
                format!("[[startmenu_changes]]\ndefault = \"help\"\nhidden = true\n\n{bad}"),
            );
        }
    }

    #[test]
    fn a_lone_changes_table_becomes_the_first_entry() {
        let text = "# mine\n[startmenu_changes]\ndefault = \"run\"\nlabel = \"&Go...\" # typed\n";
        let want = without(&default_menu(), "help");
        let out = saved(text, &want);
        assert_eq!(
            out,
            "# mine\n[[startmenu_changes]]\ndefault = \"run\"\nlabel = \"&Go...\" # typed\n\n\
             [[startmenu_changes]]\ndefault = \"help\"\nhidden = true\n"
        );
        let scalar = "startmenu_changes = 3\n";
        assert_eq!(
            saved(scalar, &want),
            scalar,
            "a value no table stays, and so does the menu"
        );
    }

    #[test]
    fn an_inline_changes_list_is_written_in_block_form_under_its_comment() {
        let text = "terminal = \"foot\"\n\n# mine\nstartmenu_changes = [\
                    { default = \"run\", label = \"&Go...\" }, \
                    { default = \"newer\", hidden = true }]\n";
        let want = without(&load(text), "help");
        let out = saved(text, &want);
        assert_eq!(
            out,
            "terminal = \"foot\"\n\n# mine\n\
             [[startmenu_changes]]\ndefault = \"help\"\nhidden = true\n\n\
             [[startmenu_changes]]\ndefault = \"run\"\nlabel = \"&Go...\"\n\n\
             [[startmenu_changes]]\ndefault = \"newer\"\nhidden = true\n"
        );
        assert_eq!(load(&out), want);
    }

    #[test]
    fn a_first_changes_comment_below_a_blank_line_stays_with_its_table() {
        let text = "terminal = \"foot\"\n\n# Start menu\n\n# from my laptop\n\
                    [[startmenu_changes]]\ndefault = \"newer\"\nhidden = true\n";
        let want = without(&load(text), "help");
        assert_eq!(
            saved(text, &want),
            "terminal = \"foot\"\n\n# Start menu\n\n\
             [[startmenu_changes]]\ndefault = \"help\"\nhidden = true\n\n\
             # from my laptop\n[[startmenu_changes]]\ndefault = \"newer\"\nhidden = true\n"
        );
        let one = "# Start menu\n[[startmenu_changes]]\ndefault = \"newer\"\nhidden = true\n";
        assert_eq!(
            saved(one, &want),
            "# Start menu\n[[startmenu_changes]]\ndefault = \"help\"\nhidden = true\n\n\
             [[startmenu_changes]]\ndefault = \"newer\"\nhidden = true\n",
            "a comment with no blank line under it heads the block"
        );
    }

    #[test]
    fn a_changes_list_holding_no_table_is_not_saved_over() {
        let text = "startmenu_changes = [\"help\", { default = \"run\", label = \"&Go...\" }]\n";
        assert_eq!(saved(text, &without(&load(text), "help")), text);
    }

    #[test]
    fn a_whole_list_from_an_older_file_stays_whole() {
        let text = "[[startmenu]]\nlabel = \"A\"\ncommand = \"a\"\n";
        let mut m = load(text);
        m.push(Entry::program("B", "", "b".into()));
        assert_eq!(
            saved(text, &m),
            "[[startmenu]]\nlabel = \"A\"\ncommand = \"a\"\n\n[[startmenu]]\nlabel = \"B\"\ncommand = \"b\"\n"
        );
    }
}
