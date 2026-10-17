//! Move and Copy through a folder picker (`reprisekit::controls::browsefolder`)
//! over a folder tree of its own; Accept runs the same move or copy a paste does.

use crate::dialogs::{Dialog, dialog_key};
use crate::foldertree::{self, Inputs, Kids};
use crate::place::Place;
use crate::{BTN_LEFT, Explorer};
use repriseapp::listjob::Job;
use repriseapp::{Ctx, Event, PointerInput, Reply};
use reprisefs::names::Verdict;
use reprisefs::ops::DropOp;
use reprisekit::controls::browsefolder::{self, BrowseFolder};
use reprisekit::controls::form::{self, Form};
use reprisekit::controls::msgbox;
use reprisekit::controls::treeview::{Style, TreeView};
use reprisekit::look::Paint;
use reprisekit::pointer::DoubleClick;
use reprisekit::raster::Canvas;
use reprisekit::textedit::keysym;
use std::path::{Path, PathBuf};

pub struct Browse {
    pub bf: BrowseFolder,
    pub places: Vec<Place>,
    pub items: Vec<PathBuf>,
    pub mv: bool,
    /// The pick the buttons were last judged for; judging stats the folder, and
    /// every pointer motion ends in [`Explorer::sync_browse_buttons`].
    pub judged: Option<Option<Place>>,
    /// The judging of the pick still being read (a dead mount); both buttons grey until it lands.
    pub judging: Option<Job<(bool, bool)>>,
    /// The walk to the items' folder still being read: the picker shows the Desktop until it
    /// lands, and a row picked by then keeps the pick.
    pub walking: Option<(Place, Job<foldertree::Chain>)>,
    /// Opened nodes whose children are still being read.
    pub filling: Vec<(Place, Job<Kids>)>,
    pub clicks: DoubleClick<Option<browsefolder::Hit>>,
    pub refused: Option<Refused>,
}

pub const ERROR: repriseapp::WinId = "browse error";
pub const ERROR_TITLE: &str = "Error Renaming File or Folder";

pub struct Refused {
    pub path: reprisekit::controls::treeview::Path,
    pub name: String,
    pub text: String,
}

impl Browse {
    fn picked(&self) -> Option<Place> {
        let path = self.bf.tree.selected.as_deref()?;
        foldertree::place_of(&self.bf.tree, &self.places, path)
    }

    /// The judging of a new pick, Accept's and Make New Folder's, for a listing thread; `None`
    /// when the pick was judged already.
    pub fn judge(&mut self) -> Option<impl FnOnce() -> (bool, bool) + Send + 'static> {
        let pick = self.picked();
        if self.judged.as_ref() == Some(&pick) {
            return None;
        }
        self.judged = Some(pick.clone());
        self.judging = None;
        let (mv, items) = (self.mv, self.items.clone());
        Some(move || match &pick {
            Some(p) => (accept_ok(mv, &items, p), new_folder_ok(p)),
            None => (false, false),
        })
    }
}

pub fn prompt(mv: bool, items: &[PathBuf]) -> String {
    let (verb, button) = if mv {
        ("move", "Move")
    } else {
        ("copy", "Copy")
    };
    let what = match items {
        [one] => format!(
            "'{}'",
            one.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ),
        many => format!("the {} selected items", many.len()),
    };
    format!("Pick the folder to {verb} {what} to, then click {button}.")
}

/// A folder the picker can put things in: a place with a directory this process
/// may create entries in ([`reprisefs::ops::can_write`]).
fn writable_dir(place: &Place) -> Option<PathBuf> {
    let dir = (!place.read_only()).then(|| place.path()).flatten()?;
    (dir.is_dir() && reprisefs::ops::can_write(&dir)).then_some(dir)
}

pub fn new_folder_ok(place: &Place) -> bool {
    writable_dir(place).is_some()
}

/// Whether the items can go into `place`: writable, not one of them or below
/// one, and for a move not their own folder.
pub fn accept_ok(mv: bool, items: &[PathBuf], place: &Place) -> bool {
    let Some(dir) = writable_dir(place) else {
        return false;
    };
    items
        .iter()
        .all(|it| !dir.starts_with(it) && !(mv && it.parent() == Some(dir.as_path())))
}

/// Where the rename box's `typed` name takes the folder at `from`: `None` when
/// unchanged, `Err` with the reason when refused (`reprisefs::names::verdict`).
pub fn rename_target(from: &Path, typed: &str) -> Option<Result<PathBuf, String>> {
    let dir = from.parent()?;
    let old = from.file_name()?.to_string_lossy().into_owned();
    let existing: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    match reprisefs::names::verdict(&old, typed, &existing) {
        Verdict::Cancel => None,
        Verdict::Refuse(why) => Some(Err(why)),
        Verdict::Rename(name) => Some(Ok(dir.join(name))),
    }
}

pub fn make_new_folder(dir: &Path) -> std::io::Result<PathBuf> {
    let path = reprisefs::ops::unique_path(dir, "New Folder");
    std::fs::create_dir(&path)?;
    Ok(path)
}

impl Explorer {
    pub fn open_browse(&mut self, mv: bool, ctx: &mut Ctx) -> Reply {
        let items = self.selected_paths();
        self.browse_items(mv, items, ctx)
    }

    pub fn browse_items(&mut self, mv: bool, items: Vec<PathBuf>, ctx: &mut Ctx) -> Reply {
        if items.is_empty() {
            return Reply::none();
        }
        let inp = Inputs {
            show_hidden: self.settings.show_hidden,
            drives: &self.drives,
            volumes: &self.volumes,
            drive_mounts: &self.drive_mounts,
        };
        let mut tree = TreeView::new(Vec::new(), Style::Picker);
        let mut places = Vec::new();
        foldertree::build(&mut tree, &mut places, &inp, &mut ctx.icons);
        let here = items[0].parent().map(Place::from_path);
        let mut walking = None;
        if let Some(here) = &here {
            let chain = foldertree::to_fill(&tree, &places, here, inp.drive_mounts);
            match self.read_job(ctx, self.chain_reader(chain)) {
                Ok(read) => foldertree::follow_with(
                    &mut tree,
                    &mut places,
                    here,
                    &inp,
                    &mut ctx.icons,
                    read,
                ),
                Err(job) => walking = Some((here.clone(), job)),
            }
        }
        let accept = if mv { "Move" } else { "Copy" };
        let mut b = Browse {
            bf: BrowseFolder::new(prompt(mv, &items), accept.into(), tree),
            places,
            items,
            mv,
            judged: None,
            judging: None,
            walking,
            filling: Vec::new(),
            clicks: DoubleClick::default(),
            refused: None,
        };
        if self.keyed {
            b.bf.cue(true);
        }
        self.sync_browse_buttons(&mut b, ctx);
        // The walk stops above a folder the tree does not show (a hidden one); Enter must not send the items there unasked.
        if here.is_none() || b.picked() != here {
            b.bf.set_focus(browsefolder::Focus::Tree);
        }
        let lay = b.bf.layout(
            &self.dialog_text,
            self.forms.fonts.size_px,
            &self.forms.controls(),
        );
        b.bf.tree.follow(&lay.tree);
        self.dialog = Some(Dialog::Browse(b));
        Reply::redraw_all()
    }

    pub fn browse_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let mut b = match self.dialog.take() {
            Some(Dialog::Browse(b)) => b,
            other => {
                self.dialog = other;
                return Reply::none();
            }
        };
        let lay = b.bf.layout(
            &self.dialog_text,
            self.forms.fonts.size_px,
            &self.forms.controls(),
        );
        let press = match ev {
            Event::Pointer(
                p @ PointerInput::Press {
                    button: BTN_LEFT, ..
                },
            ) if b.bf.tree.edit.is_some() => Some(p),
            _ => None,
        };
        let cued = match &ev {
            Event::Key(k) => {
                b.bf.cue(k.mods.alt || matches!(k.sym, keysym::ALT_L | keysym::ALT_R))
            }
            _ => false,
        };
        let out = match ev {
            Event::Key(k) => match dialog_key(&k) {
                Some(key) => b.bf.key(&lay, key),
                None => browsefolder::Event::None,
            },
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) if b.bf.tree.edit.is_some() => {
                let hit = b.bf.hit(&lay, x, y);
                b.bf.click(&lay, hit)
            }
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                time,
                ..
            }) => {
                let hit = b.bf.hit(&lay, x, y);
                let double = b.clicks.press(hit, ctx.theme.metrics.click(x, y, time));
                if double {
                    b.bf.double_click(&lay, hit)
                } else {
                    b.bf.click(&lay, hit)
                }
            }
            Event::Pointer(PointerInput::Motion { x, y }) => {
                b.bf.motion(&lay, &self.dialog_text, self.forms.fonts.size_px, x, y)
            }
            Event::Pointer(PointerInput::Release { .. }) => b.bf.release(),
            Event::Pointer(PointerInput::Leave) => b.bf.leave(),
            Event::Pointer(PointerInput::Scroll { x, y, rows }) => b.bf.wheel(&lay, x, y, rows),
            Event::Resized => {
                b.bf.set_size(
                    ctx.size,
                    &self.dialog_text,
                    self.forms.fonts.size_px,
                    &self.forms.controls(),
                );
                browsefolder::Event::Redraw
            }
            _ => browsefolder::Event::None,
        };
        let inp = Inputs {
            show_hidden: self.settings.show_hidden,
            drives: &self.drives,
            volumes: &self.volumes,
            drive_mounts: &self.drive_mounts,
        };
        let reply = match out {
            browsefolder::Event::None if cued => Reply::redraw(),
            browsefolder::Event::None => Reply::none(),
            browsefolder::Event::Redraw | browsefolder::Event::Selected(_) => Reply::redraw(),
            browsefolder::Event::Expand(p) => {
                if let Some(place) = foldertree::place_of(&b.bf.tree, &b.places, &p)
                    && !b.filling.iter().any(|(q, _)| *q == place)
                {
                    let read = self.kids_reader_in(&b.bf.tree, &b.places, &p, place.clone());
                    match self.read_job(ctx, read) {
                        Ok(kids) => foldertree::fill_with(
                            &mut b.bf.tree,
                            &mut b.places,
                            p,
                            kids,
                            &inp,
                            &mut ctx.icons,
                        ),
                        Err(job) => b.filling.push((place, job)),
                    }
                }
                Reply::redraw()
            }
            browsefolder::Event::NewFolder(p) => {
                let Some(place) = foldertree::place_of(&b.bf.tree, &b.places, &p) else {
                    return self.put_back(b, Reply::none());
                };
                let made = match writable_dir(&place) {
                    Some(dir) => make_new_folder(&dir),
                    None => return self.put_back(b, Reply::none()),
                };
                let made = match made {
                    Ok(path) => path,
                    Err(e) => {
                        return self.message_box(
                            "Error Creating Folder",
                            format!("Cannot create New Folder: {e}"),
                            ctx,
                        );
                    }
                };
                foldertree::fill(
                    &mut b.bf.tree,
                    &mut b.places,
                    p.clone(),
                    &inp,
                    &mut ctx.icons,
                );
                if let Some(n) = b.bf.tree.node_mut(&p) {
                    n.expanded = n.has_children;
                }
                let name = made.file_name().map(|n| n.to_string_lossy().into_owned());
                let row = b.bf.tree.node(&p).and_then(|n| {
                    n.children
                        .iter()
                        .position(|c| Some(&c.label) == name.as_ref())
                });
                if let Some(i) = row {
                    let lay = b.bf.layout(
                        &self.dialog_text,
                        self.forms.fonts.size_px,
                        &self.forms.controls(),
                    );
                    b.bf.new_folder_made([p.as_slice(), &[i]].concat(), &lay);
                }
                self.refill_tree_node(&place, ctx);
                self.undos.push(reprisefs::undo::Undoable::New(made));
                self.sync_undo();
                Reply::redraw_all()
            }
            browsefolder::Event::Rename(p, typed) => {
                let Some(from) = foldertree::place_of(&b.bf.tree, &b.places, &p)
                    .and_then(|pl| pl.path())
                    .filter(|_| p.len() > 1)
                else {
                    return self.put_back_then(b, Reply::redraw(), press, ctx);
                };
                let Some(to) = rename_target(&from, &typed) else {
                    return self.put_back_then(b, Reply::redraw(), press, ctx);
                };
                let old = from
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let done = match &to {
                    Ok(to) => reprisefs::hidden::rename(&from, to)
                        .map_err(|e| reprisefs::names::failed_text(&old, &e)),
                    Err(why) => Err(why.clone()),
                };
                if let Err(text) = done {
                    log::warn!("explorer: naming {} failed: {text}", from.display());
                    if self.error_icon.is_none() {
                        self.error_icon =
                            crate::chrome_icon(ctx, &["dialog-error", "dialog-warning"], 32);
                    }
                    b.refused = Some(Refused {
                        path: p,
                        name: old,
                        text,
                    });
                    return self.put_back(b, Reply::redraw_all());
                }
                let to = to.expect("renamed above");
                let parent = p[..p.len() - 1].to_vec();
                foldertree::fill(
                    &mut b.bf.tree,
                    &mut b.places,
                    parent.clone(),
                    &inp,
                    &mut ctx.icons,
                );
                let name = to.file_name().map(|n| n.to_string_lossy().into_owned());
                let row = b.bf.tree.node(&parent).and_then(|n| {
                    n.children
                        .iter()
                        .position(|c| Some(&c.label) == name.as_ref())
                });
                if let Some(i) = row {
                    b.bf.tree.selected = Some([parent.as_slice(), &[i]].concat());
                    let lay = b.bf.layout(
                        &self.dialog_text,
                        self.forms.fonts.size_px,
                        &self.forms.controls(),
                    );
                    b.bf.tree.follow(&lay.tree);
                }
                self.after_rename(&from, &to, true, ctx);
                self.sync_browse_buttons(&mut b, ctx);
                return self.put_back_then(b, Reply::redraw_all(), press, ctx);
            }
            browsefolder::Event::Accept(p) => {
                let dir = foldertree::place_of(&b.bf.tree, &b.places, &p).and_then(|pl| pl.path());
                let Some(dir) = dir else {
                    return self.put_back(b, Reply::none());
                };
                let op = if b.mv { DropOp::Move } else { DropOp::Copy };
                self.put_all(&b.items, &dir, &|_| Some(op), true, ctx);
                return Reply::redraw_all();
            }
            browsefolder::Event::Cancel => return Reply::redraw_all(),
            browsefolder::Event::Resize => Reply::resize(),
        };
        // A click opens a row as it picks it without a `Selected`, so every event re-checks the pick.
        self.sync_browse_buttons(&mut b, ctx);
        self.put_back(b, reply)
    }

    /// Judges the picker's buttons for a new pick on a listing thread ([`Browse::judge`]).
    pub(crate) fn sync_browse_buttons(&self, b: &mut Browse, ctx: &Ctx) {
        let Some(read) = b.judge() else {
            return;
        };
        match self.read_job(ctx, read) {
            Ok((accept, new_folder)) => b.bf.set_enabled(accept, new_folder),
            Err(job) => {
                b.bf.set_enabled(false, false);
                b.judging = Some(job);
            }
        }
    }

    /// Takes the picker's reads that landed after their wait: the walk to the items' folder,
    /// opened nodes and the pick's judging. True when one did.
    pub(crate) fn take_browse(&mut self, ctx: &mut Ctx) -> bool {
        let mut b = match self.dialog.take() {
            Some(Dialog::Browse(b)) => b,
            other => {
                self.dialog = other;
                return false;
            }
        };
        let inp = Inputs {
            show_hidden: self.settings.show_hidden,
            drives: &self.drives,
            volumes: &self.volumes,
            drive_mounts: &self.drive_mounts,
        };
        let mut landed = false;
        if let Some(read) = b.walking.as_ref().and_then(|(_, job)| job.take())
            && let Some((here, _)) = b.walking.take()
        {
            if b.bf.tree.selected.as_deref() == Some(&[0]) {
                foldertree::follow_with(
                    &mut b.bf.tree,
                    &mut b.places,
                    &here,
                    &inp,
                    &mut ctx.icons,
                    read,
                );
                let lay = b.bf.layout(
                    &self.dialog_text,
                    self.forms.fonts.size_px,
                    &self.forms.controls(),
                );
                b.bf.tree.follow(&lay.tree);
            }
            landed = true;
        }
        let mut done = Vec::new();
        b.filling.retain(|(place, job)| match job.take() {
            Some(kids) => {
                done.push((place.clone(), kids));
                false
            }
            None => true,
        });
        for (place, kids) in done {
            if let Some(path) = foldertree::path_of(&b.bf.tree, &b.places, &place, inp.drive_mounts)
            {
                foldertree::fill_with(
                    &mut b.bf.tree,
                    &mut b.places,
                    path,
                    kids,
                    &inp,
                    &mut ctx.icons,
                );
            }
            landed = true;
        }
        self.sync_browse_buttons(&mut b, ctx);
        if let Some((accept, new_folder)) = b.judging.as_ref().and_then(Job::take) {
            b.judging = None;
            b.bf.set_enabled(accept, new_folder);
            landed = true;
        }
        self.dialog = Some(Dialog::Browse(b));
        landed
    }

    pub fn browse_error_fields(&self) -> Option<msgbox::Fields> {
        let Some(Dialog::Browse(b)) = &self.dialog else {
            return None;
        };
        let r = b.refused.as_ref()?;
        Some(msgbox::Fields {
            text: r.text.clone(),
            icon: self.error_icon.clone(),
            buttons: vec!["OK".to_string()],
            default: 0,
            min_text_h: None,
            min_w: repriseapp::caption_fit(ERROR_TITLE),
        })
    }

    fn browse_error_form(&self) -> Option<Form> {
        let f = self.browse_error_fields()?;
        let (px, u) = (self.forms.fonts.size_px, self.forms.text);
        Some(self.built(msgbox::form(&f, &self.dialog_text, px, u)))
    }

    pub fn browse_error_event(&mut self, ev: Event) -> Reply {
        let out = match ev {
            Event::CloseRequest => form::Event::Cancel,
            Event::Key(k) => match (dialog_key(&k), self.browse_error_form()) {
                (Some(key), Some(mut f)) => f.key(key),
                _ => return Reply::none(),
            },
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => {
                let Some(mut f) = self.browse_error_form() else {
                    return Reply::none();
                };
                let hit = f.hit(x, y, &self.dialog_text, self.forms.fonts.size_px);
                f.click(hit);
                f.release()
            }
            _ => return Reply::none(),
        };
        let closed = match out {
            form::Event::Cancel | form::Event::Default => true,
            form::Event::Pressed(id) => id == msgbox::OK,
            _ => false,
        };
        if !closed {
            return Reply::redraw();
        }
        if let Some(Dialog::Browse(b)) = self.dialog.as_mut()
            && let Some(r) = b.refused.take()
        {
            b.bf.reopen_edit(r.path, &r.name);
        }
        Reply::redraw_all()
    }

    pub fn draw_browse_error(&self, c: &mut Canvas, ctx: &Ctx) {
        c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
        if let Some(f) = self.browse_error_form() {
            let p = Paint {
                s: 1,
                ..self.forms.paint()
            };
            form::draw(c, &self.dialog_text, &p, &f, true, self.forms.fonts.size_px);
        }
    }

    pub fn access_browse_error(&self, tree: &mut reprisekit::access::Tree) {
        if let Some(f) = self.browse_error_form() {
            let px = self.forms.fonts.size_px;
            repriseapp::oneshot::access_form(&f, tree, &self.dialog_text, px);
        }
    }

    fn put_back(&mut self, b: Browse, reply: Reply) -> Reply {
        self.dialog = Some(Dialog::Browse(b));
        reply
    }

    fn put_back_then(
        &mut self,
        b: Browse,
        reply: Reply,
        press: Option<PointerInput>,
        ctx: &mut Ctx,
    ) -> Reply {
        let reply = self.put_back(b, reply);
        let Some(p) = press else {
            return reply;
        };
        let again = self.browse_event(Event::Pointer(p), ctx);
        Reply {
            redraw: reply.redraw || again.redraw,
            redraw_all: reply.redraw_all || again.redraw_all,
            ..again
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let t = std::env::temp_dir().join(format!("reprise-browse-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(t.join("sub/deeper")).unwrap();
        std::fs::write(t.join("a.txt"), "").unwrap();
        t
    }

    #[test]
    fn prompts_name_one_item_or_count_several() {
        assert_eq!(
            prompt(true, &[PathBuf::from("/x/a.txt")]),
            "Pick the folder to move 'a.txt' to, then click Move."
        );
        assert_eq!(
            prompt(false, &[PathBuf::from("/x/a"), PathBuf::from("/x/b")]),
            "Pick the folder to copy the 2 selected items to, then click Copy."
        );
    }

    #[test]
    fn accept_rules_moving_home_or_inside_itself_is_greyed() {
        let t = dir("rules");
        let file = [t.join("a.txt")];
        let folder = [t.join("sub")];
        assert!(
            !accept_ok(true, &file, &Place::Folder(t.clone())),
            "a move into its own folder does nothing"
        );
        assert!(
            accept_ok(false, &file, &Place::Folder(t.clone())),
            "a copy there makes Copy of"
        );
        assert!(accept_ok(true, &file, &Place::Folder(t.join("sub"))));
        assert!(
            !accept_ok(true, &folder, &Place::Folder(t.join("sub"))),
            "into itself"
        );
        assert!(
            !accept_ok(false, &folder, &Place::Folder(t.join("sub/deeper"))),
            "below itself"
        );
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn accept_rules_grey_places_and_read_only() {
        let file = [PathBuf::from("/tmp/a.txt")];
        for p in [Place::Computer, Place::RecycleBin] {
            assert!(!accept_ok(true, &file, &p), "{p:?}");
            assert!(!new_folder_ok(&p), "{p:?}");
        }
        assert!(
            !accept_ok(false, &file, &Place::Folder(PathBuf::from("/proc"))),
            "/proc has no write bit"
        );
        assert!(!new_folder_ok(&Place::Folder(PathBuf::from("/proc"))));
    }

    #[test]
    fn folders_the_process_cannot_write_grey_both_buttons() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let shut = t.path().join("shut");
        std::fs::create_dir(&shut).unwrap();
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o555)).unwrap();
        let file = [t.path().join("a.txt")];
        let judged: Vec<_> = [shut.clone(), PathBuf::from("/usr")]
            .into_iter()
            .map(|d| {
                let place = Place::Folder(d.clone());
                (d, accept_ok(false, &file, &place), new_folder_ok(&place))
            })
            .collect();
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o755)).unwrap();
        if std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(t.path()).unwrap()) == 0 {
            return;
        }
        for (d, accept, new_folder) in judged {
            assert!(!accept, "Copy is greyed on {}", d.display());
            assert!(!new_folder, "Make New Folder is greyed on {}", d.display());
        }
    }

    #[test]
    fn the_buttons_are_judged_once_per_pick() {
        use reprisekit::controls::treeview::Node;
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let (one, two) = (t.path().join("one"), t.path().join("two"));
        for d in [&one, &two] {
            std::fs::create_dir(d).unwrap();
        }
        let mut tree = TreeView::new(
            vec![Node::new("one", false), Node::new("two", false)],
            Style::Picker,
        );
        tree.roots[1].id = 1;
        tree.selected = Some(vec![0]);
        let mut b = Browse {
            bf: BrowseFolder::new(String::new(), "Copy".into(), tree),
            places: vec![Place::Folder(one.clone()), Place::Folder(two.clone())],
            items: vec![t.path().join("a.txt")],
            mv: false,
            judged: None,
            judging: None,
            walking: None,
            filling: Vec::new(),
            clicks: DoubleClick::default(),
            refused: None,
        };
        let judge = |b: &mut Browse| {
            if let Some(read) = b.judge() {
                let (accept, new_folder) = read();
                b.bf.set_enabled(accept, new_folder);
            }
        };
        judge(&mut b);
        assert!(b.bf.accept_enabled, "one is writable");
        for d in [&one, &two] {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o555)).unwrap();
        }
        judge(&mut b);
        let kept = b.bf.accept_enabled;
        b.bf.tree.selected = Some(vec![1]);
        judge(&mut b);
        let two_judged = b.bf.accept_enabled;
        for d in [&one, &two] {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        if std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(t.path()).unwrap()) == 0 {
            return;
        }
        assert!(kept, "the same pick is not judged again");
        assert!(!two_judged, "a new pick is judged");
    }

    #[test]
    fn make_new_folder_numbers_past_taken_names() {
        let t = dir("newfolder");
        let a = make_new_folder(&t).unwrap();
        let b = make_new_folder(&t).unwrap();
        assert_eq!(a, t.join("New Folder"));
        assert_eq!(b, t.join("New Folder (2)"));
        assert!(b.is_dir());
        std::fs::remove_dir_all(&t).ok();
    }
}
