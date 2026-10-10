//! The Start Menu page: the menu as a tree with Add, Rename, Remove and Move buttons. Buttons edit
//! the pending menu and the tree in step; Apply writes the menu whole.

use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::treeview::{self, Node, Style, TreeView};
use reprisekit::raster::{Canvas, Rect};

use crate::pages::{Area, GX, group};
use crate::startmenu::{self, Entry, Target};

pub const TREE: &str = "sm_tree";
pub const ADD: &str = "sm_add";
pub const FOLDER: &str = "sm_folder";
pub const RENAME: &str = "sm_rename";
pub const REMOVE: &str = "sm_remove";
pub const UP: &str = "sm_up";
pub const DOWN: &str = "sm_down";

pub fn tree_rect(a: Area) -> Rect {
    Rect::new(GX, 64, a.gw() - BUTTON_W - 8, 164 + a.dh)
}
const BUTTON_STEP: i32 = BUTTON_H + 4;

pub const NEW_FOLDER: &str = "New Folder";
const SEPARATOR: &str = "(Separator)";
const ROOT: &str = "Start Menu";

pub fn page(a: Area) -> Form {
    let tree_rect = tree_rect(a);
    let mut items = vec![
        group(a, "sm_group", 44, 196 + a.dh, "Start menu"),
        Item::new(TREE, tree_rect, Control::Tree(tree(&[]))),
    ];
    let buttons = [
        (ADD, "&Add..."),
        (FOLDER, "New &Folder"),
        (RENAME, "Re&name..."),
        (REMOVE, "&Remove"),
        (UP, "Move &Up"),
        (DOWN, "Move Do&wn"),
    ];
    for (i, (id, text)) in buttons.into_iter().enumerate() {
        let r = Rect::new(
            a.gright() - BUTTON_W,
            tree_rect.y + i as i32 * BUTTON_STEP,
            BUTTON_W,
            BUTTON_H,
        );
        items.push(Item::button(id, r, text, true, false));
    }
    items.extend(crate::quickpage::items(a));
    Form::new(items)
}

pub fn shown(e: &Entry) -> String {
    match e.label() {
        Some(l) => reprisekit::text::split_mnemonic(l).0,
        None => SEPARATOR.into(),
    }
}

fn nodes(entries: &[Entry]) -> Vec<Node> {
    entries.iter().map(node).collect()
}

fn node(e: &Entry) -> Node {
    let kids = e.items().map(|v| nodes(v)).unwrap_or_default();
    Node {
        has_children: !kids.is_empty(),
        children: kids,
        ..Node::new(&shown(e), false)
    }
}

fn tree(entries: &[Entry]) -> TreeView {
    let root = Node {
        has_children: true,
        expanded: true,
        children: nodes(entries),
        ..Node::new(ROOT, true)
    };
    TreeView::new(vec![root], Style::Classic)
}

fn tv(form: &Form) -> Option<&TreeView> {
    match form.item(TREE).map(|i| &i.control) {
        Some(Control::Tree(tv)) => Some(tv),
        _ => None,
    }
}

fn tv_mut(form: &mut Form) -> Option<&mut TreeView> {
    match form.item_mut(TREE).map(|i| &mut i.control) {
        Some(Control::Tree(tv)) => Some(tv),
        _ => None,
    }
}

pub fn fill(form: &mut Form, entries: &[Entry]) {
    if let Some(t) = tv_mut(form) {
        *t = tree(entries);
    }
    sync(form, entries);
}

pub fn selected(form: &Form) -> Option<Vec<usize>> {
    let sel = tv(form)?.selected.as_ref()?;
    Some(sel.get(1..)?.to_vec())
}

pub fn sync(form: &mut Form, entries: &[Entry]) {
    let sel = selected(form);
    let row = sel.as_deref().filter(|p| !p.is_empty());
    let e = row.and_then(|p| startmenu::get(entries, p));
    let (i, n) = match row.and_then(|p| p.split_last()) {
        Some((&i, parent)) => {
            let n = match parent {
                [] => entries.len(),
                p => startmenu::get(entries, p)
                    .and_then(Entry::items)
                    .map_or(0, Vec::len),
            };
            (i, n)
        }
        None => (0, 0),
    };
    form.set_enabled(RENAME, e.is_some_and(|e| e.label().is_some()));
    form.set_enabled(REMOVE, e.is_some());
    form.set_enabled(UP, e.is_some() && i > 0);
    form.set_enabled(DOWN, e.is_some() && i + 1 < n);
}

fn at_root(p: &[usize]) -> Vec<usize> {
    let mut out = vec![0];
    out.extend_from_slice(p);
    out
}

fn pick(form: &mut Form, p: &[usize], cm: &ControlMetrics) {
    let rect = form.item(TREE).map_or(tree_rect(Area::MIN), |i| i.rect);

    if let Some(t) = tv_mut(form) {
        t.selected = Some(at_root(p));
        let lay = treeview::layout_without_text(t, rect, cm);
        t.follow(&lay);
    }
}

fn refresh(form: &mut Form, entries: &[Entry], p: &[usize]) {
    let Some(e) = startmenu::get(entries, p) else {
        return;
    };
    let label = shown(e);
    let n = e.items().map_or(0, Vec::len);
    if let Some(node) = tv_mut(form).and_then(|t| t.node_mut(&at_root(p))) {
        node.label = label;
        node.has_children = n > 0;
        if n == 0 {
            node.expanded = false;
        }
    }
}

pub fn shift(form: &mut Form, entries: &mut Vec<Entry>, by: isize, cm: &ControlMetrics) -> bool {
    let Some(p) = selected(form).filter(|p| !p.is_empty()) else {
        return false;
    };
    let Some(to) = startmenu::shift(entries, &p, by) else {
        return false;
    };
    if let Some(t) = tv_mut(form) {
        startmenu::shift(&mut t.roots[0].children, &p, by);
    }
    pick(form, &to, cm);
    sync(form, entries);
    true
}

pub fn remove(form: &mut Form, entries: &mut Vec<Entry>, cm: &ControlMetrics) -> bool {
    let Some(p) = selected(form).filter(|p| !p.is_empty()) else {
        return false;
    };
    if startmenu::take(entries, &p).is_none() {
        return false;
    }
    if let Some(t) = tv_mut(form) {
        startmenu::take(&mut t.roots[0].children, &p);
    }
    let (&i, parent) = p.split_last().unwrap_or((&0, &[]));
    let left = match parent {
        [] => entries.len(),
        pp => startmenu::get(entries, pp)
            .and_then(Entry::items)
            .map_or(0, Vec::len),
    };
    if !parent.is_empty() {
        refresh(form, entries, parent);
    }
    let mut next = parent.to_vec();
    if left > 0 {
        next.push(i.min(left - 1));
    }
    pick(form, &next, cm);
    sync(form, entries);
    true
}

pub fn insert(
    form: &mut Form,
    entries: &mut Vec<Entry>,
    e: Entry,
    cm: &ControlMetrics,
) -> Option<Vec<usize>> {
    let p = startmenu::insertion(entries, selected(form).as_deref());
    let n = node(&e);
    startmenu::put(entries, &p, e)?;
    if let Some(t) = tv_mut(form) {
        startmenu::put(&mut t.roots[0].children, &p, n);
    }
    let parent = &p[..p.len() - 1];
    if !parent.is_empty() {
        refresh(form, entries, parent);
        if let Some(node) = tv_mut(form).and_then(|t| t.node_mut(&at_root(parent))) {
            node.expanded = true;
        }
    }
    pick(form, &p, cm);
    sync(form, entries);
    Some(p)
}

pub fn new_folder(form: &mut Form, entries: &mut Vec<Entry>, cm: &ControlMetrics) -> bool {
    let e = Entry::item(NEW_FOLDER, "", Target::Folder(Vec::new()));
    insert(form, entries, e, cm).is_some()
}

pub fn rename(form: &mut Form, entries: &mut Vec<Entry>, to: &str) -> bool {
    let Some(p) = selected(form).filter(|p| !p.is_empty()) else {
        return false;
    };
    if !startmenu::rename(entries, &p, to) {
        return false;
    }
    refresh(form, entries, &p);
    true
}

pub fn selected_label(form: &Form, entries: &[Entry]) -> Option<String> {
    let p = selected(form)?;
    startmenu::get(entries, &p)?.label().map(str::to_string)
}

fn icon_name(e: &Entry) -> Option<String> {
    let fallback = match e {
        Entry::Separator => return None,
        e if e.is_folder() => "folder",
        _ => "application-x-executable",
    };
    Some(e.icon().unwrap_or(fallback).to_string())
}

pub fn set_icons(form: &mut Form, entries: &[Entry], get: &mut dyn FnMut(&str) -> Option<Canvas>) {
    fn walk(nodes: &mut [Node], entries: &[Entry], get: &mut dyn FnMut(&str) -> Option<Canvas>) {
        for (n, e) in nodes.iter_mut().zip(entries) {
            n.icon = icon_name(e).and_then(|name| get(&name));
            if let Some(kids) = e.items() {
                walk(&mut n.children, kids, get);
            }
        }
    }
    if let Some(t) = tv_mut(form) {
        let root = &mut t.roots[0];
        root.icon = get("folder");
        root.open_icon = get("folder-open").or_else(|| root.icon.clone());
        walk(&mut root.children, entries, get);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::startmenu::default_menu;

    const CM: ControlMetrics = ControlMetrics::CLASSIC;

    fn labels(form: &Form, parent: &[usize]) -> Vec<String> {
        let t = tv(form).unwrap();
        let n = t.node(&at_root(parent)).unwrap();
        n.children.iter().map(|c| c.label.clone()).collect()
    }

    fn open(menu: &[Entry]) -> Form {
        let mut f = page(Area::MIN);
        fill(&mut f, menu);
        f
    }

    fn select(f: &mut Form, p: &[usize]) {
        pick(f, p, &CM);
        let m = default_menu();
        sync(f, &m);
    }

    #[test]
    fn the_tree_shows_the_menu_under_its_root_without_access_marks() {
        let f = open(&default_menu());
        assert_eq!(
            labels(&f, &[]),
            [
                "Programs",
                "Documents",
                "Settings",
                "Search",
                "Help and Support",
                "Run...",
                "(Separator)",
                "Log Off {user}...",
                "Turn Off Computer...",
            ]
        );
        let t = tv(&f).unwrap();
        assert_eq!(t.roots[0].label, "Start Menu");
        assert!(t.roots[0].expanded);
        assert!(t.node(&[0, 2]).unwrap().has_children, "Settings opens");
        assert!(
            !t.node(&[0, 0]).unwrap().has_children,
            "Programs is filled at open"
        );
    }

    #[test]
    fn the_buttons_follow_the_picked_row() {
        let mut f = open(&default_menu());
        let on = |f: &Form, id| match f.item(id).map(|i| &i.control) {
            Some(Control::Button { enabled, .. }) => *enabled,
            _ => panic!("{id} is a button"),
        };
        sync(&mut f, &default_menu());
        for id in [RENAME, REMOVE, UP, DOWN] {
            assert!(!on(&f, id), "{id} greys with nothing picked");
        }
        assert!(on(&f, ADD) && on(&f, FOLDER));
        select(&mut f, &[]);
        assert!(!on(&f, REMOVE), "the root stays");
        select(&mut f, &[0]);
        assert!(on(&f, RENAME) && on(&f, REMOVE) && on(&f, DOWN));
        assert!(!on(&f, UP), "the first row cannot go higher");
        select(&mut f, &[6]);
        assert!(!on(&f, RENAME), "a separator has no name");
        select(&mut f, &[3, 1]);
        assert!(on(&f, UP) && !on(&f, DOWN), "the last row of Search");
    }

    #[test]
    fn a_move_carries_an_open_folder_with_it() {
        let mut m = default_menu();
        let mut f = open(&m);
        tv_mut(&mut f).unwrap().node_mut(&[0, 2]).unwrap().expanded = true;
        pick(&mut f, &[2], &CM);
        assert!(shift(&mut f, &mut m, 1, &CM));
        assert_eq!(selected(&f), Some(vec![3]));
        assert_eq!(labels(&f, &[])[3], "Settings");
        assert!(tv(&f).unwrap().node(&[0, 3]).unwrap().expanded);
        assert_eq!(
            startmenu::get(&m, &[3]).and_then(Entry::label),
            Some("&Settings")
        );
        assert_eq!(
            labels(&f, &[3]),
            [
                "Control Panel",
                "(Separator)",
                "Network Connections",
                "Printers",
                "Taskbar and Start Menu"
            ]
        );
    }

    #[test]
    fn remove_picks_the_next_row_and_an_emptied_folder_closes() {
        let mut m = default_menu();
        let mut f = open(&m);
        pick(&mut f, &[3, 1], &CM);
        assert!(remove(&mut f, &mut m, &CM));
        assert_eq!(selected(&f), Some(vec![3, 0]), "the row before, being last");
        assert!(remove(&mut f, &mut m, &CM));
        assert_eq!(selected(&f), Some(vec![3]), "the emptied folder");
        assert!(!tv(&f).unwrap().node(&[0, 3]).unwrap().has_children);
        assert_eq!(
            startmenu::get(&m, &[3]).and_then(Entry::items),
            Some(&Vec::new())
        );
    }

    #[test]
    fn a_new_folder_opens_its_parent_and_takes_rows_and_a_rename() {
        let mut m = default_menu();
        let mut f = open(&m);
        pick(&mut f, &[4], &CM);
        assert!(new_folder(&mut f, &mut m, &CM));
        assert_eq!(selected(&f), Some(vec![5]));
        let e = Entry::item("&Editor", "", Target::Command("gedit".into()));
        assert_eq!(insert(&mut f, &mut m, e, &CM), Some(vec![5, 0]));
        let t = tv(&f).unwrap();
        assert!(t.node(&[0, 5]).unwrap().expanded && t.node(&[0, 5]).unwrap().has_children);
        assert_eq!(labels(&f, &[5]), ["Editor"]);
        pick(&mut f, &[5], &CM);
        assert_eq!(selected_label(&f, &m).as_deref(), Some(NEW_FOLDER));
        assert!(rename(&mut f, &mut m, "&Tools"));
        assert_eq!(labels(&f, &[])[5], "Tools");
        assert_eq!(
            startmenu::get(&m, &[5]).and_then(Entry::label),
            Some("&Tools")
        );
    }

    #[test]
    fn with_nothing_picked_a_row_goes_to_the_menus_end() {
        let mut m = default_menu();
        let mut f = open(&m);
        let e = Entry::item("Last", "", Target::Command("x".into()));
        assert_eq!(insert(&mut f, &mut m, e, &CM), Some(vec![9]));
        assert_eq!(labels(&f, &[]).last().map(String::as_str), Some("Last"));
    }

    #[test]
    fn the_tree_fills_the_group_beside_a_column_of_six_buttons() {
        let f = page(Area::MIN);
        assert_eq!(tree_rect(Area::MIN), Rect::new(32, 64, 251, 164));
        let tops: Vec<(i32, i32)> = [ADD, FOLDER, RENAME, REMOVE, UP, DOWN]
            .iter()
            .map(|id| {
                let r = f.item(id).unwrap().rect;
                (r.x, r.y)
            })
            .collect();
        assert_eq!(
            tops,
            [
                (291, 64),
                (291, 91),
                (291, 118),
                (291, 145),
                (291, 172),
                (291, 199)
            ]
        );
        let group = f.item("sm_group").unwrap().rect;
        assert!(
            group.bottom() >= tree_rect(Area::MIN).bottom() + 8
                && group.bottom() >= 199 + BUTTON_H + 8
        );
    }

    #[test]
    fn the_picked_row_is_highlighted_at_its_depth() {
        use reprisekit::controls::form;
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::{FontConfig, Theme};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut f = open(&default_menu());
        select(&mut f, &[2]);
        let mut c = Canvas::new(400, 260);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        form::draw(
            &mut c,
            &text,
            &theme.paint(),
            &f,
            false,
            theme.fonts.size_px,
        );
        let cs = &theme.colors;
        let body = tree_rect(Area::MIN).inset(2);
        assert_eq!(
            c.get(tree_rect(Area::MIN).x, tree_rect(Area::MIN).y),
            cs.button_shadow.argb(),
            "the sunken edge"
        );
        assert_eq!(c.get(body.x + 1, body.bottom() - 2), cs.window.argb());
        let lit = |row: i32| {
            let y = body.y + row * treeview::ROW_H + treeview::ROW_H / 2;
            (body.x..body.right())
                .filter(|&x| c.get(x, y) == cs.hilight.argb())
                .min()
        };
        assert_eq!(lit(0), None, "the root");
        assert_eq!(lit(2), None, "Documents");
        assert_eq!(
            lit(3),
            Some(body.x + 2 * treeview::INDENT + treeview::ICON_PX)
        );
        assert_eq!(lit(4), None, "Search");
    }
}
