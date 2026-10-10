//! The folder tree shared by every `TreeView` over a `Place` chain (the Folders pane and the Move/Copy
//! picker): building it, filling a node lazily, and walking it down to the place it should show. The
//! caller hands over its own `TreeView` and place table plus an [`Inputs`] borrow; `window::draw` hands
//! the Explorer's tree to `reprisekit::controls::treeview`, and `place` decides what a place is.

use crate::place::{self, Place, list};
use reprisekit::controls::treeview::{Node, TreeView};
use reprisekit::icontheme::IconCache;
use std::path::PathBuf;

/// What building or filling a tree needs to know about the world, borrowed from whichever host owns
/// the places (they differ per host only in tests; Explorer and the folder picker share one scan).
pub struct Inputs<'a> {
    pub show_hidden: bool,
    pub drives: &'a [reprisefs::places::Drive],
    pub volumes: &'a reprisefs::udisks::Snapshot,
    pub drive_mounts: &'a [PathBuf],
}

/// The shell places under the Desktop, in the tree's order (the address drop's too), ahead of the Desktop folder's own subfolders.
pub const DESKTOP_PLACES: [Place; 4] = [
    Place::Documents,
    Place::Home,
    Place::Computer,
    Place::RecycleBin,
];

fn tree_node(
    places: &mut Vec<Place>,
    label: &str,
    place: Place,
    has_children: bool,
    icons: &mut IconCache,
) -> Node {
    let id = places.len() as u64;
    let name = place.icon_name();
    let icon = icons.get(name, 16);
    let open_icon = if name == "folder" {
        icons.get("folder-open", 16).or_else(|| icon.clone())
    } else {
        icon.clone()
    };
    places.push(place);
    Node {
        label: label.to_string(),
        icon,
        open_icon,
        has_children,
        expanded: false,
        children: Vec::new(),
        id,
    }
}

/// Build a tree rooted at the Desktop, its own node expanded and filled, and the root selected.
pub fn build(tree: &mut TreeView, places: &mut Vec<Place>, inp: &Inputs, icons: &mut IconCache) {
    places.clear();
    let mut root = tree_node(places, "Desktop", Place::Desktop, true, icons);
    root.expanded = true;
    tree.roots = vec![root];
    fill(tree, places, vec![0], inp, icons);
    tree.selected = Some(vec![0]);
}

/// Read a tree node's children. The Desktop holds Documents, Home, Computer and the Recycle Bin, then
/// its own subfolders; Computer holds its drives and the user's Home. A child that was there before
/// keeps its node (open or not, with what it holds), and a selection below the node follows its child
/// to the new row, or lands on the node when that child is gone.
pub fn fill(
    tree: &mut TreeView,
    places: &mut Vec<Place>,
    path: Vec<usize>,
    inp: &Inputs,
    icons: &mut IconCache,
) {
    let Some(node) = tree.node(&path) else {
        return;
    };
    let place = places[node.id as usize].clone();
    let mut kids: Vec<(String, Place)> = Vec::new();
    match &place {
        Place::Desktop => {
            for p in DESKTOP_PLACES {
                kids.push((p.title(), p));
            }
            kids.extend(subfolders(&place, inp));
        }
        Place::Computer => {
            kids.extend(place::computer_tree_children(&list(
                &Place::Computer,
                &reprisefs::apps::MimeDb::empty(),
                inp.show_hidden,
                inp.drives,
                inp.volumes,
            )));
        }
        Place::RecycleBin => {}
        _ => kids.extend(subfolders(&place, inp)),
    }
    let mut old: Vec<Option<Node>> = tree
        .node_mut(&path)
        .map(|n| {
            std::mem::take(&mut n.children)
                .into_iter()
                .map(Some)
                .collect()
        })
        .unwrap_or_default();
    let place_at = |n: &Option<Node>| n.as_ref().map(|n| &places[n.id as usize]);
    let was: Vec<Option<usize>> = kids
        .iter()
        .map(|(_, p)| old.iter().position(|n| place_at(n) == Some(p)))
        .collect();
    let nodes: Vec<Node> = kids
        .into_iter()
        .zip(&was)
        .map(|((label, p), was)| match was.and_then(|i| old[i].take()) {
            Some(n) => Node { label, ..n },
            None => {
                let expandable =
                    p != Place::RecycleBin && p.path().is_none_or(|dir| has_subfolder(&dir, inp));
                tree_node(places, &label, p, expandable, icons)
            }
        })
        .collect();
    let depth = path.len();
    if let Some(sel) = tree.selected.as_mut()
        && sel.len() > depth
        && sel.starts_with(&path)
    {
        match was.iter().position(|&w| w == Some(sel[depth])) {
            Some(i) => sel[depth] = i,
            None => sel.truncate(depth),
        }
    }
    if let Some(node) = tree.node_mut(&path) {
        node.has_children = !nodes.is_empty();
        if nodes.is_empty() {
            node.expanded = false;
        }
        node.children = nodes;
    }
    compact(tree, places);
}

/// `places` is indexed by `Node::id` and [`tree_node`] hands out a fresh id per node, so repeated
/// expanding and collapsing would grow it forever. Renumber the live nodes after every fill: the ids
/// are the tree's own and nothing outside it holds one (the selection is a path, not an id).
fn compact(tree: &mut TreeView, places: &mut Vec<Place>) {
    fn walk(nodes: &mut [Node], old: &[Place], out: &mut Vec<Place>) {
        for n in nodes {
            let Some(place) = old.get(n.id as usize) else {
                continue;
            };
            out.push(place.clone());
            n.id = (out.len() - 1) as u64;
            walk(&mut n.children, old, out);
        }
    }
    let old = std::mem::take(places);
    let mut roots = std::mem::take(&mut tree.roots);
    let mut fresh = Vec::with_capacity(old.len());
    walk(&mut roots, &old, &mut fresh);
    tree.roots = roots;
    *places = fresh;
}

pub fn place_of(tree: &TreeView, places: &[Place], path: &[usize]) -> Option<Place> {
    tree.node(path).map(|n| places[n.id as usize].clone())
}

/// A place's subfolders for the tree, through the same [`place::list`] the view reads, so the tree
/// hides what the view hides, minus any child that is another drive's mount point (it belongs under
/// Computer). No MIME globs: nothing here needs a file's kind.
fn subfolders(place: &Place, inp: &Inputs) -> Vec<(String, Place)> {
    let listed = list(
        place,
        &reprisefs::apps::MimeDb::empty(),
        inp.show_hidden,
        inp.drives,
        inp.volumes,
    );
    place::tree_subfolders(&listed, inp.drive_mounts)
}

/// Whether `dir` holds a folder the tree would show under it: one the listing keeps (a hidden folder
/// only with hidden files shown) that is not another drive's mount point. A symlink to a folder is a
/// shortcut, which the tree leaves out. The read stops at the first.
fn has_subfolder(dir: &std::path::Path, inp: &Inputs) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    let listed = if inp.show_hidden {
        reprisefs::hidden::Listed::default()
    } else {
        reprisefs::hidden::Listed::read(dir)
    };
    rd.flatten().any(|e| {
        let shown = inp.show_hidden || !listed.hides(&e.file_name());
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        shown && is_dir && !inp.drive_mounts.contains(&e.path())
    })
}

/// The path of the node standing for `place`, without expanding or filling anything: the
/// [`Place::chain_in`] chain matched against the nodes already held. `None` when the tree has not been walked that far.
pub fn path_of(
    tree: &TreeView,
    places: &[Place],
    place: &Place,
    mounts: &[PathBuf],
) -> Option<Vec<usize>> {
    let chain = place.chain_in(mounts);
    let mut path: Vec<usize> = Vec::new();
    for want in &chain {
        let i = child_index(tree, places, &path, want)?;
        path.push(i);
    }
    (!path.is_empty()).then_some(path)
}

/// The index under `path` (among the roots for an empty one) of the node standing for `want`; `None` if no node does.
fn child_index(tree: &TreeView, places: &[Place], path: &[usize], want: &Place) -> Option<usize> {
    let level = if path.is_empty() {
        &tree.roots
    } else {
        &tree.node(path)?.children
    };
    level.iter().position(|n| &places[n.id as usize] == want)
}

/// Re-read the children of the node standing for `place`, when the tree holds one. A folder renamed,
/// made or deleted in the view changes them, and [`follow`] only fills a node whose children are still empty.
pub fn refill(
    tree: &mut TreeView,
    places: &mut Vec<Place>,
    place: &Place,
    inp: &Inputs,
    icons: &mut IconCache,
) {
    let Some(path) = path_of(tree, places, place, inp.drive_mounts) else {
        return;
    };
    // Only an opened node (with children, or read empty) is worth re-reading; an unopened one fills correctly when opened.
    if tree
        .node(&path)
        .is_none_or(|n| n.children.is_empty() && n.has_children)
    {
        return;
    }
    fill(tree, places, path, inp, icons);
}

/// Walk the tree down to `place`, expanding and filling what it takes, and select it. The chain is
/// [`Place::chain_in`] over the cached mounts, so a folder is reached through Computer and the drive
/// whose mount owns it. The caller scrolls the selection into view.
pub fn follow(
    tree: &mut TreeView,
    places: &mut Vec<Place>,
    place: &Place,
    inp: &Inputs,
    icons: &mut IconCache,
) {
    let chain = place.chain_in(inp.drive_mounts);
    let mut path: Vec<usize> = Vec::new();
    for want in &chain {
        let Some(i) = child_index(tree, places, &path, want) else {
            break;
        };
        path.push(i);
        let fill_it = match tree.node_mut(&path) {
            Some(n) if n.has_children => {
                n.expanded = true;
                n.children.is_empty()
            }
            _ => false,
        };
        if fill_it {
            fill(tree, places, path.clone(), inp, icons);
        }
    }
    if !path.is_empty() {
        tree.selected = Some(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisefs::places::Drive;
    use reprisefs::udisks::Snapshot;

    /// Re-reading an open folder keeps what was open below it and the selection on its node, when a new sibling sorts ahead of both.
    #[test]
    fn a_refill_keeps_open_subfolders_and_the_selection() {
        let home = tempfile::tempdir().expect("a temp home");
        std::fs::create_dir_all(home.path().join("b/c")).expect("mkdir b/c");
        let root_drive = Drive {
            mount: PathBuf::from("/"),
            device: "/dev/sda1".into(),
            fs: "ext4".into(),
            label: None,
            removable: false,
            optical: false,
            total: 1,
            free: 1,
        };
        let volumes = Snapshot::default();
        let mounts = [PathBuf::from("/")];
        let inp = Inputs {
            show_hidden: true,
            drives: std::slice::from_ref(&root_drive),
            volumes: &volumes,
            drive_mounts: &mounts,
        };
        let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
        let mut tree = TreeView::default();
        let mut places: Vec<Place> = Vec::new();
        build(&mut tree, &mut places, &inp, &mut icons);
        let c = Place::Folder(home.path().join("b/c"));
        follow(&mut tree, &mut places, &c, &inp, &mut icons);

        std::fs::create_dir(home.path().join("a")).expect("mkdir a");
        let here = Place::Folder(home.path().to_path_buf());
        refill(&mut tree, &mut places, &here, &inp, &mut icons);

        let b = Place::Folder(home.path().join("b"));
        let b_path = path_of(&tree, &places, &b, &mounts).expect("b's node");
        assert!(
            tree.node(&b_path)
                .is_some_and(|n| n.expanded && n.children.len() == 1),
            "b stays open over c"
        );
        assert_eq!(
            tree.selected
                .as_deref()
                .and_then(|p| place_of(&tree, &places, p)),
            Some(c),
            "the selection follows c to its new row"
        );
        assert!(
            path_of(
                &tree,
                &places,
                &Place::Folder(home.path().join("a")),
                &mounts
            )
            .is_some()
        );
    }

    /// A folder shows a + box only when it holds a subfolder the tree would list: not an empty one, one
    /// holding only files, or (while hidden files are hidden) one whose only subfolder is hidden.
    #[test]
    fn only_a_folder_with_a_subfolder_has_a_plus_box() {
        let home = tempfile::tempdir().expect("a temp home");
        for d in [
            "empty",
            "files",
            "full/inner",
            "dotted/.inner",
            "listed/inner",
        ] {
            std::fs::create_dir_all(home.path().join(d)).expect("mkdir");
        }
        std::fs::write(home.path().join("files/a.txt"), "").expect("a file");
        std::fs::write(home.path().join("listed/.hidden"), "inner\n").expect("a .hidden");
        let volumes = Snapshot::default();
        let boxes = |show_hidden: bool| {
            let inp = Inputs {
                show_hidden,
                drives: &[],
                volumes: &volumes,
                drive_mounts: &[],
            };
            let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
            let mut places = vec![Place::Folder(home.path().to_path_buf())];
            let mut tree = TreeView::new(vec![Node::new("home", true)], Default::default());
            fill(&mut tree, &mut places, vec![0], &inp, &mut icons);
            tree.roots[0]
                .children
                .iter()
                .map(|n| (n.label.clone(), n.has_children))
                .collect::<Vec<_>>()
        };
        let want = |dotted| {
            [
                ("dotted", dotted),
                ("empty", false),
                ("files", false),
                ("full", true),
                ("listed", dotted),
            ]
            .map(|(l, b)| (l.to_string(), b))
            .to_vec()
        };
        assert_eq!(boxes(false), want(false), "hidden files hidden");
        assert_eq!(boxes(true), want(true), "hidden files shown");
    }

    /// `follow` walks Desktop, Computer and the "/" drive down to a deep folder, expanding and
    /// selecting it, so the same walk works for the Move/Copy picker's tree.
    #[test]
    fn follow_opens_and_picks_a_nested_folder() {
        let home = tempfile::tempdir().expect("a temp home");
        std::fs::create_dir_all(home.path().join("a/b")).expect("mkdir a/b");
        let root_drive = Drive {
            mount: PathBuf::from("/"),
            device: "/dev/sda1".into(),
            fs: "ext4".into(),
            label: None,
            removable: false,
            optical: false,
            total: 1,
            free: 1,
        };
        let volumes = Snapshot::default();
        let mounts = [PathBuf::from("/")];
        let inp = Inputs {
            // A temp dir's own name starts with a dot, so every ancestor down to it must show hidden entries.
            show_hidden: true,
            drives: std::slice::from_ref(&root_drive),
            volumes: &volumes,
            drive_mounts: &mounts,
        };
        let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
        let mut tree = TreeView::default();
        let mut places: Vec<Place> = Vec::new();
        build(&mut tree, &mut places, &inp, &mut icons);

        let target = Place::Folder(home.path().join("a/b"));
        follow(&mut tree, &mut places, &target, &inp, &mut icons);

        assert_eq!(
            tree.selected
                .as_deref()
                .and_then(|p| place_of(&tree, &places, p)),
            Some(target)
        );
        let a_path = path_of(
            &tree,
            &places,
            &Place::Folder(home.path().join("a")),
            &mounts,
        )
        .expect("a's own node");
        assert!(
            tree.node(&a_path).is_some_and(|n| n.expanded),
            "the folder's ancestor is expanded"
        );
    }
}
