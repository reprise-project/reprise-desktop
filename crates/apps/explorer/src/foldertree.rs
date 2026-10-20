//! The folder tree shared by every `TreeView` over a `Place` chain (the Folders pane and the Move/Copy
//! picker): building it, filling a node lazily, and walking it down to the place it should show. The
//! caller hands over its own `TreeView` and place table plus an [`Inputs`] borrow; `window::draw` hands
//! the Explorer's tree to `reprisekit::controls::treeview`, and `place` decides what a place is.

use crate::place::{self, Place, list};
use reprisekit::controls::treeview::{Node, TreeView};
use reprisekit::icontheme::IconCache;
use reprisekit::raster::Canvas;
use std::path::PathBuf;

/// What building or filling a tree needs to know about the world, borrowed from whichever host owns
/// the places (they differ per host only in tests; Explorer and the folder picker share one scan).
pub struct Inputs<'a> {
    pub show_hidden: bool,
    pub drives: &'a [reprisefs::places::Drive],
    pub volumes: &'a reprisefs::udisks::Snapshot,
    pub drive_mounts: &'a [PathBuf],
}

/// [`Inputs`] owned, for a read on a listing thread.
pub struct World {
    /// The scale icon files are decoded at.
    pub scale: i32,
    pub show_hidden: bool,
    pub drives: Vec<reprisefs::places::Drive>,
    pub volumes: reprisefs::udisks::Snapshot,
    pub drive_mounts: Vec<PathBuf>,
    /// The folders the list gives their own icons ([`reprisefs::places::special_folders`]).
    pub specials: Vec<(PathBuf, &'static str)>,
}

impl World {
    pub fn inputs(&self) -> Inputs<'_> {
        Inputs {
            show_hidden: self.show_hidden,
            drives: &self.drives,
            volumes: &self.volumes,
            drive_mounts: &self.drive_mounts,
        }
    }
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
    own: &Own,
    icons: &mut IconCache,
) -> Node {
    let id = places.len() as u64;
    let (icon, open_icon) = worn_icons(&place, own, icons);
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

/// A node's closed and open icons at `icons`' scale.
fn node_icons(place: &Place, icons: &mut IconCache) -> (Option<Canvas>, Option<Canvas>) {
    let name = place.icon_name();
    let icon = icons.get(name, 16);
    let open_icon = if name == "folder" {
        icons.get("folder-open", 16).or_else(|| icon.clone())
    } else {
        icon.clone()
    };
    (icon, open_icon)
}

/// A folder node's own icon (`meta::folder_icon`) or special-folder icon, read with its parent's
/// children.
#[derive(Clone, Default)]
pub enum Own {
    #[default]
    None,
    /// Theme names, best first: the folder's own, then its special-folder icon.
    Named(Vec<String>),
    /// An icon file, decoded at 16 px and the read's scale.
    File(Option<Canvas>),
}

impl Own {
    /// `place`'s own icon, an icon file decoded at `scale`, else its icon among `specials`. A
    /// drive's or other mount point's is not read: the mount may never answer.
    fn read(place: &Place, inp: &Inputs, scale: i32, specials: &[(PathBuf, &str)]) -> Own {
        let Place::Folder(dir) = place else {
            return Own::None;
        };
        if inp.drive_mounts.contains(dir) || reprisefs::listing::is_mount_root(dir) {
            return Own::None;
        }
        let special = specials
            .iter()
            .find(|(d, _)| d == dir)
            .map(|(_, n)| n.to_string());
        match reprisefs::meta::folder_icon(dir) {
            Some(f) if f.starts_with('/') => Own::File(reprisekit::icontheme::decode_file(
                std::path::Path::new(&f),
                16,
                scale,
            )),
            own => match own.into_iter().chain(special).collect::<Vec<_>>() {
                names if names.is_empty() => Own::None,
                names => Own::Named(names),
            },
        }
    }
}

/// [`node_icons`] with the folder's own or special-folder icon worn instead, the first the theme
/// carries or its file decoded; open or closed alike.
fn worn_icons(place: &Place, own: &Own, icons: &mut IconCache) -> (Option<Canvas>, Option<Canvas>) {
    let icon = match own {
        Own::None => None,
        Own::Named(names) => names.iter().find_map(|n| icons.get(n, 16)),
        Own::File(art) => art.clone(),
    };
    match icon {
        Some(icon) => (Some(icon.clone()), Some(icon)),
        None => node_icons(place, icons),
    }
}

/// Every node fetches its icons again from `icons`, now at another scale. A folder's own or
/// special-folder icon gives way to the plain folder until its parent is filled again.
pub fn refetch_icons(tree: &mut TreeView, places: &[Place], icons: &mut IconCache) {
    fn walk(nodes: &mut [Node], places: &[Place], icons: &mut IconCache) {
        for n in nodes {
            if let Some(place) = places.get(n.id as usize) {
                (n.icon, n.open_icon) = node_icons(place, icons);
            }
            walk(&mut n.children, places, icons);
        }
    }
    walk(&mut tree.roots, places, icons);
}

/// Build a tree rooted at the Desktop, its own node expanded and the root selected. Its children
/// are read off the host's loop: a symlink among them may point into a mount that never answers.
pub fn build(tree: &mut TreeView, places: &mut Vec<Place>, icons: &mut IconCache) {
    places.clear();
    let mut root = tree_node(places, "Desktop", Place::Desktop, true, &Own::None, icons);
    root.expanded = true;
    tree.roots = vec![root];
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
) -> bool {
    let Some(node) = tree.node(&path) else {
        return false;
    };
    let known: Vec<&Place> = node
        .children
        .iter()
        .map(|n| &places[n.id as usize])
        .collect();
    let specials = reprisefs::places::special_folders();
    let kids = read_kids(
        &places[node.id as usize],
        inp,
        &known,
        icons.scale(),
        &specials,
    );
    fill_with(tree, places, path, kids, inp, icons)
}

/// The children of the places a walk fills ([`to_fill`]), read ahead for [`follow_with`].
pub type Chain = Vec<(Place, Kids)>;

/// A node's children as [`read_kids`] read them: label, place, whether the child opens (`None`
/// for one the node already held, which keeps its own) and its own icon.
pub type Kids = Vec<(String, Place, Option<bool>, Own)>;

/// The file system half of [`fill`]: `place`'s children with their own icons (icon files decoded
/// at `scale`) or their icons among `specials`, and for those not in `known` whether they hold a
/// folder. A host reads them off its loop for [`follow_with`].
pub fn read_kids(
    place: &Place,
    inp: &Inputs,
    known: &[&Place],
    scale: i32,
    specials: &[(PathBuf, &str)],
) -> Kids {
    let mut kids: Vec<(String, Place)> = Vec::new();
    match place {
        Place::Desktop => {
            for p in DESKTOP_PLACES {
                kids.push((p.title(), p));
            }
            kids.extend(subfolders(place, inp));
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
        _ => kids.extend(subfolders(place, inp)),
    }
    kids.into_iter()
        .map(|(label, p)| {
            let opens = (!known.contains(&&p)).then(|| opens(&p, inp));
            let own = Own::read(&p, inp, scale, specials);
            (label, p, opens, own)
        })
        .collect()
}

/// Whether `p`'s node shows a +. A drive's or other mount point's is shown unread: one that never
/// answers would hold up its siblings, and opening an empty one drops the +.
fn opens(p: &Place, inp: &Inputs) -> bool {
    *p != Place::RecycleBin
        && p.path().is_none_or(|dir| {
            inp.drive_mounts.contains(&dir)
                || reprisefs::listing::is_mount_root(&dir)
                || has_subfolder(&dir, inp)
        })
}

/// [`fill`] with the node's children already read: true when one wears its own or a
/// special-folder icon, which a new scale reads again.
pub fn fill_with(
    tree: &mut TreeView,
    places: &mut Vec<Place>,
    path: Vec<usize>,
    kids: Kids,
    inp: &Inputs,
    icons: &mut IconCache,
) -> bool {
    let art = (kids.iter()).any(|(.., own)| matches!(own, Own::File(Some(_)) | Own::Named(_)));
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
        .map(|(_, p, ..)| old.iter().position(|n| place_at(n) == Some(p)))
        .collect();
    let nodes: Vec<Node> = kids
        .into_iter()
        .zip(&was)
        .map(
            |((label, p, opened, own), was)| match was.and_then(|i| old[i].take()) {
                Some(n) => {
                    let (icon, open_icon) = worn_icons(&p, &own, icons);
                    Node {
                        label,
                        icon,
                        open_icon,
                        ..n
                    }
                }
                None => {
                    let expandable = opened.unwrap_or_else(|| opens(&p, inp));
                    tree_node(places, &label, p, expandable, &own, icons)
                }
            },
        )
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
    art
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

/// The path of the node standing for `place` when its children are worth reading again: a folder
/// renamed, made or deleted in the view changes them. Only an opened node (with children, or read
/// empty) qualifies; an unopened one fills correctly when opened.
pub fn refill_path(
    tree: &TreeView,
    places: &[Place],
    place: &Place,
    mounts: &[PathBuf],
) -> Option<Vec<usize>> {
    let path = path_of(tree, places, place, mounts)?;
    let n = tree.node(&path)?;
    (!(n.children.is_empty() && n.has_children)).then_some(path)
}

/// The places whose nodes [`follow_with`] fills on its way to `place`: from the first one not yet
/// read, every place left on the chain.
pub fn to_fill(tree: &TreeView, places: &[Place], place: &Place, mounts: &[PathBuf]) -> Vec<Place> {
    let chain = place.chain_in(mounts);
    let mut path: Vec<usize> = Vec::new();
    for (k, want) in chain.iter().enumerate() {
        let Some(i) = child_index(tree, places, &path, want) else {
            break;
        };
        path.push(i);
        if tree
            .node(&path)
            .is_some_and(|n| n.has_children && n.children.is_empty())
        {
            return chain[k..].to_vec();
        }
    }
    Vec::new()
}

/// Walk the tree down to `place`, expanding and filling what it takes, and select it. The chain is
/// [`Place::chain_in`] over the cached mounts, so a folder is reached through Computer and the drive
/// whose mount owns it. The caller scrolls the selection into view.
/// A node whose place `read` holds ([`read_kids`] of it) fills from there; any other is read here.
/// Gives the places filled, each with [`fill_with`]'s answer.
pub fn follow_with(
    tree: &mut TreeView,
    places: &mut Vec<Place>,
    place: &Place,
    inp: &Inputs,
    icons: &mut IconCache,
    mut read: Chain,
) -> Vec<(Place, bool)> {
    let mut filled = Vec::new();
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
            match read.iter().position(|(p, _)| p == want) {
                Some(k) => {
                    let kids = read.swap_remove(k).1;
                    let art = fill_with(tree, places, path.clone(), kids, inp, icons);
                    filled.push((want.clone(), art));
                }
                None => {
                    let art = fill(tree, places, path.clone(), inp, icons);
                    filled.push((want.clone(), art));
                }
            }
        }
    }
    if !path.is_empty() {
        tree.selected = Some(path);
    }
    filled
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisefs::places::Drive;
    use reprisefs::udisks::Snapshot;

    #[test]
    fn a_drive_shows_a_plus_unread_and_a_folder_only_with_a_subfolder() {
        let t = tempfile::tempdir().expect("a temp dir");
        let (drive, folder) = (t.path().join("gone/DEAD"), t.path().join("empty"));
        std::fs::create_dir(&folder).expect("mkdir empty");
        let volumes = Snapshot::default();
        let mounts = [drive.clone()];
        let inp = Inputs {
            show_hidden: false,
            drives: &[],
            volumes: &volumes,
            drive_mounts: &mounts,
        };
        assert!(opens(&Place::Folder(drive), &inp), "a drive is not read");
        assert!(!opens(&Place::Folder(folder.clone()), &inp));
        std::fs::create_dir(folder.join("sub")).expect("mkdir sub");
        assert!(opens(&Place::Folder(folder), &inp));
    }

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
        build(&mut tree, &mut places, &mut icons);
        let c = Place::Folder(home.path().join("b/c"));
        follow_with(&mut tree, &mut places, &c, &inp, &mut icons, Vec::new());

        std::fs::create_dir(home.path().join("a")).expect("mkdir a");
        let here = Place::Folder(home.path().to_path_buf());
        let path = refill_path(&tree, &places, &here, &mounts).expect("an opened node");
        fill(&mut tree, &mut places, path, &inp, &mut icons);

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

    #[test]
    fn a_folder_wears_its_own_icon_and_a_refill_swaps_it() {
        let home = tempfile::tempdir().expect("a temp home");
        let art = tempfile::tempdir().expect("a temp dir");
        std::fs::create_dir(home.path().join("own")).expect("mkdir own");
        let wear = |argb: u32| {
            let file = art.path().join(format!("{argb:x}.png"));
            let mut c = Canvas::new(16, 16);
            c.pixels.fill(argb);
            std::fs::write(&file, reprisekit::raster::to_png(&c)).expect("write png");
            let entry = format!("[Desktop Entry]\nIcon={}\n", file.display());
            std::fs::write(home.path().join("own/.directory"), entry).expect("write");
        };
        let volumes = Snapshot::default();
        let inp = Inputs {
            show_hidden: true,
            drives: &[],
            volumes: &volumes,
            drive_mounts: &[],
        };
        let mut icons = IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]));
        let mut places = vec![Place::Folder(home.path().to_path_buf())];
        let mut tree = TreeView::new(vec![Node::new("home", true)], Default::default());
        let worn = |tree: &TreeView| {
            let n = &tree.roots[0].children[0];
            [&n.icon, &n.open_icon].map(|c| c.as_ref().map(|c| c.get(8, 8)))
        };
        wear(0xff10_2030);
        fill(&mut tree, &mut places, vec![0], &inp, &mut icons);
        assert_eq!(worn(&tree), [Some(0xff10_2030); 2], "closed and open");
        wear(0xff40_5060);
        fill(&mut tree, &mut places, vec![0], &inp, &mut icons);
        assert_eq!(
            worn(&tree),
            [Some(0xff40_5060); 2],
            "the kept node wears the new icon"
        );
        std::fs::remove_file(home.path().join("own/.directory")).expect("rm");
        fill(&mut tree, &mut places, vec![0], &inp, &mut icons);
        assert_eq!(
            worn(&tree)[0],
            icons.get("folder", 16).map(|c| c.get(8, 8)),
            "and the plain folder once it has none"
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

    /// The walk fills its nodes from children read beforehand (on a listing's thread) without
    /// reading the folders again: it reaches the folder though the folders are gone by then.
    #[test]
    fn follow_with_fills_from_children_read_beforehand() {
        let home = tempfile::tempdir().expect("a temp home");
        std::fs::create_dir_all(home.path().join("a/b/c")).expect("mkdir a/b/c");
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
        build(&mut tree, &mut places, &mut icons);
        let target = Place::Folder(home.path().join("a/b"));
        let read: Vec<(Place, Kids)> = to_fill(&tree, &places, &target, &mounts)
            .into_iter()
            .map(|p| {
                let kids = read_kids(&p, &inp, &[], 1, &[]);
                (p, kids)
            })
            .collect();
        assert!(read.iter().any(|(p, _)| *p == target), "the target is read");
        std::fs::remove_dir_all(home.path().join("a")).expect("rm a");

        follow_with(&mut tree, &mut places, &target, &inp, &mut icons, read);
        let selected = tree.selected.as_deref().expect("a selection");
        assert_eq!(place_of(&tree, &places, selected), Some(target));
        assert!(
            tree.node(selected).is_some_and(|n| n.children.len() == 1),
            "b holds c as read"
        );
    }

    /// `follow_with` walks Desktop, Computer and the "/" drive down to a deep folder, expanding and
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
        build(&mut tree, &mut places, &mut icons);

        let target = Place::Folder(home.path().join("a/b"));
        follow_with(
            &mut tree,
            &mut places,
            &target,
            &inp,
            &mut icons,
            Vec::new(),
        );

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
