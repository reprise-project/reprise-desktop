//! The Favorites menu: Add to Favorites, Organize Favorites and a row per bookmark of GTK's
//! bookmarks file ([`reprisefs::pins`]), which GTK's and the portal's file dialogs list too.

use std::path::PathBuf;

use repriseapp::{Ctx, Reply};
use reprisefs::netshare::Address;
use reprisefs::pins::{Bookmarks, Favorite, Target};
use reprisekit::controls::favorites::{self as fav, NameBox, Row, State};
use reprisekit::controls::form::{self, Form};

use crate::Explorer;
use crate::commands::Command;
use crate::dialogs::Dialog;
use crate::place::Place;

/// The favorites the menu lists at most; Organize Favorites lists the first
/// [`reprisefs::pins::MAX_FAVORITES`].
pub const MENU_ROWS: usize = 40;

/// Add Favorite, naming the folder `dir`.
pub struct AddFavorite {
    pub name: NameBox,
    dir: PathBuf,
}

/// Organize Favorites over the bookmarks as last read.
pub struct Organize {
    pub form: Form,
    shown: Vec<Favorite>,
}

/// A favorite's row in the Favorites menu: its label and what picking it does, `None` greyed.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuRow {
    pub label: String,
    pub go: Option<Command>,
}

/// What opening `f` does: a folder there goes to it, a server address connects through
/// Connect to Server; a missing folder or another kind of address does nothing.
fn opening(f: &Favorite) -> Option<Command> {
    match &f.target {
        Target::Folder(path) => f.present.then(|| Command::GoTo(Place::from_path(path))),
        Target::Uri(uri) => Address::parse(uri).map(|_| Command::Navigate(uri.clone())),
    }
}

fn state(f: &Favorite) -> State {
    match (&f.target, opening(f)) {
        (_, Some(_)) => State::Opens,
        (Target::Folder(_), None) => State::Missing,
        (Target::Uri(_), None) => State::Unsupported,
    }
}

pub fn menu_rows(favorites: &[Favorite]) -> Vec<MenuRow> {
    (favorites.iter().take(MENU_ROWS))
        .map(|f| MenuRow {
            label: f.label.clone(),
            go: opening(f),
        })
        .collect()
}

fn rows(shown: &[Favorite]) -> Vec<Row> {
    shown
        .iter()
        .map(|f| Row {
            label: f.label.clone(),
            address: match &f.target {
                Target::Folder(path) => path.display().to_string(),
                Target::Uri(_) => f.target.default_name(),
            },
            state: state(f),
        })
        .collect()
}

impl Organize {
    /// Lists `shown`, keeping the pick on the favorite it was on, else at its row.
    fn relist(&mut self, shown: Vec<Favorite>) {
        let at = fav::selected(&self.form);
        let target = at.and_then(|i| self.shown.get(i)).map(|f| f.target.clone());
        let keep = target.and_then(|t| shown.iter().position(|f| f.target == t));
        self.shown = shown;
        fav::set_rows(&mut self.form, &rows(&self.shown), keep.or(at));
    }
}

impl Explorer {
    pub(crate) fn favorites(&self) -> std::sync::Arc<Vec<Favorite>> {
        self.pins
            .as_ref()
            .map(|p| p.favorites())
            .unwrap_or_default()
    }

    /// The folder Add to Favorites adds: the place shown, when it is a folder.
    pub(crate) fn favorite_dir(&self) -> Option<PathBuf> {
        match self.place {
            Place::Computer | Place::Network(_) | Place::RecycleBin => None,
            _ => self.place.path(),
        }
    }

    /// Favorites > Add to Favorites...: Add Favorite with the folder's name, or the name it has
    /// as a favorite already.
    pub(crate) fn open_add_favorite(&mut self) -> Reply {
        let Some(dir) = self.favorite_dir() else {
            return Reply::none();
        };
        let target = Target::Folder(dir.clone());
        let label = (self.favorites().iter())
            .find(|f| f.target == target)
            .map_or_else(|| target.default_name(), |f| f.label.clone());
        let mut name = NameBox::new(&label);
        name.form = self.built(name.form);
        self.raise(Dialog::AddFavorite(Box::new(AddFavorite { name, dir })))
    }

    pub(crate) fn open_organize(&mut self) -> Reply {
        let shown = self.favorites().to_vec();
        let form = self.built(fav::organize(
            &rows(&shown),
            (!shown.is_empty()).then_some(0),
        ));
        self.raise(Dialog::Organize(Box::new(Organize { form, shown })))
    }

    /// The bookmarks file changed: Organize Favorites lists it again.
    pub(crate) fn favorites_changed(&mut self) -> bool {
        let shown = self.favorites().to_vec();
        match self.dialogs.up_mut() {
            Some(Dialog::Organize(o)) if o.shown != shown => {
                o.relist(shown);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn add_favorite_answered(&mut self, out: form::Event, ctx: &mut Ctx) -> Reply {
        use reprisekit::controls::namebox::Outcome;
        let Some(Dialog::AddFavorite(a)) = self.dialogs.up_mut() else {
            return Reply::none();
        };
        match a.name.event(out) {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Cancel => self.dismiss_up(ctx),
            Outcome::Accept(name) => {
                let Some(Dialog::AddFavorite(a)) = self.take_up() else {
                    return Reply::none();
                };
                let dir = a.dir;
                self.edit_favorites(move |b| b.add(&dir, &name));
                Reply::redraw_all()
            }
        }
    }

    /// Hands `edit` to the pins thread, which writes the file off this loop; the menu, the places
    /// and an open Organize box follow once it reads the file back.
    pub(crate) fn edit_favorites(
        &mut self,
        edit: impl FnOnce(&mut Bookmarks) -> bool + Send + 'static,
    ) {
        if let Some(p) = &self.pins {
            p.edit(edit);
        }
    }

    /// An edit the pins thread could not write: Organize Favorites closes and a message says why.
    pub(crate) fn take_favorites_failed(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let e = self.pins.as_mut()?.take_failed()?;
        if matches!(self.dialogs.up_mut(), Some(Dialog::Organize(_))) {
            self.take_up();
        }
        let reply = self.message_box(
            "Favorites",
            format!("Explorer cannot save your Favorites.\n\n{e}"),
            ctx,
        );
        Some(reply.and(Reply::redraw_all()))
    }

    /// Whether a Favorites edit is still to be written.
    pub(crate) fn favorites_busy(&self) -> bool {
        self.pins.as_ref().is_some_and(|p| p.busy())
    }

    /// Organize Favorites' answer: an edit goes to the bookmarks file at once.
    pub(crate) fn organize_answered(&mut self, out: form::Event, ctx: &mut Ctx) -> Reply {
        let Some(Dialog::Organize(o)) = self.dialogs.up_mut() else {
            return Reply::none();
        };
        let picked = fav::selected(&o.form).and_then(|i| o.shown.get(i).cloned());
        let named = o
            .form
            .focus()
            .is_some_and(|i| o.form.items[i].id == fav::NAME);
        let edit: Box<dyn FnOnce(&mut Bookmarks) -> bool + Send> = match (out, picked) {
            (form::Event::Changed(fav::LIST), _) => {
                fav::show_selected(&mut o.form, &rows(&o.shown));
                return Reply::redraw();
            }
            (form::Event::Changed(fav::NAME), _) => {
                fav::follow_name(&mut o.form, &rows(&o.shown));
                return Reply::redraw();
            }
            (form::Event::Cancel | form::Event::Pressed(fav::CLOSE), _) => {
                return self.dismiss_up(ctx);
            }
            (form::Event::Default, _) if !named => return self.dismiss_up(ctx),
            (form::Event::Default | form::Event::Pressed(fav::RENAME), Some(f)) => {
                let name = fav::typed_name(&o.form).to_string();
                if name.is_empty() || name == f.label {
                    return Reply::none();
                }
                Box::new(move |b| b.rename(&f.target, &name))
            }
            (form::Event::Pressed(fav::DELETE), Some(f)) => Box::new(move |b| b.remove(&f.target)),
            (form::Event::Pressed(fav::UP), Some(f)) => {
                Box::new(move |b| b.move_entry(&f.target, true))
            }
            (form::Event::Pressed(fav::DOWN), Some(f)) => {
                Box::new(move |b| b.move_entry(&f.target, false))
            }
            (form::Event::None, _) => return Reply::none(),
            _ => return Reply::redraw(),
        };
        self.edit_favorites(edit);
        Reply::redraw()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{test_ctx, test_explorer};
    use reprisekit::controls::form::Control;
    use std::path::Path;

    fn with_file(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let file = dir.path().join("gtk-3.0/bookmarks");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, text).unwrap();
        (dir, file)
    }

    /// An Explorer reading `file`, once its first read has landed.
    fn reading(file: &Path, ctx: &mut Ctx) -> Explorer {
        let mut ex = test_explorer(Place::Computer, ctx);
        let mut pins = reprisefs::pins::Pins::start_at(file.to_path_buf(), || {});
        pins.wait_first(std::time::Duration::from_secs(10));
        pins.take_changed();
        ex.pins = Some(pins);
        ex
    }

    /// Waits for the pins thread to write the edits sent and read the file back, then relists
    /// Organize as a wake does.
    fn landed(ex: &mut Explorer) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while ex.favorites_busy() || !ex.pins.as_mut().is_some_and(|p| p.take_changed()) {
            assert!(std::time::Instant::now() < end, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        ex.favorites_changed();
    }

    fn wait_for(ex: &mut Explorer, done: impl Fn(&Explorer) -> bool) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(ex) && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(done(ex), "timed out");
    }

    fn organize_form(ex: &mut Explorer) -> &mut Form {
        match ex.dialogs.up_mut() {
            Some(Dialog::Organize(o)) => &mut o.form,
            _ => panic!("Organize Favorites is up"),
        }
    }

    fn listed(ex: &mut Explorer) -> Vec<String> {
        match &organize_form(ex).item(fav::LIST).unwrap().control {
            Control::ListBox { list, .. } => list.items.clone(),
            _ => unreachable!(),
        }
    }

    fn pick(ex: &mut Explorer, row: usize, ctx: &mut Ctx) {
        if let Some(Control::ListBox { list, .. }) = organize_form(ex)
            .item_mut(fav::LIST)
            .map(|i| &mut i.control)
        {
            list.selected = Some(row);
        }
        ex.dialog_outcome(form::Event::Changed(fav::LIST), ctx);
    }

    #[test]
    fn a_server_opens_through_connect_and_an_unknown_scheme_is_greyed() {
        let at = |t: Target, present| Favorite {
            label: "x".into(),
            target: t,
            present,
        };
        let rows = menu_rows(&[
            at(Target::Folder("/srv".into()), true),
            at(Target::Folder("/gone".into()), false),
            at(Target::Uri("sftp://h/srv".into()), true),
            at(Target::Uri("recent:///".into()), true),
        ]);
        let go: Vec<Option<Command>> = rows.into_iter().map(|r| r.go).collect();
        assert_eq!(
            go,
            [
                Some(Command::GoTo(Place::from_path(Path::new("/srv")))),
                None,
                Some(Command::Navigate("sftp://h/srv".into())),
                None,
            ]
        );
    }

    #[test]
    fn add_to_favorites_writes_the_folder_under_the_name_typed() {
        let mut ctx = test_ctx();
        let (_dir, file) = with_file("# mine\nsftp://h/srv Server\n");
        let mut ex = reading(&file, &mut ctx);
        let work = tempfile::tempdir().unwrap();
        ex.go(Place::Folder(work.path().to_path_buf()), &mut ctx);
        ex.run(Command::AddFavorite, &mut ctx);
        let Some(Dialog::AddFavorite(a)) = ex.dialogs.up_mut() else {
            panic!("Add Favorite is up");
        };
        let own = work
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(a.name.value(), Some(own));
        if let Some(Control::Edit { edit, .. }) =
            a.name.form.item_mut("name").map(|i| &mut i.control)
        {
            edit.set_selected("Work & play");
        }
        ex.dialog_outcome(form::Event::Default, &mut ctx);
        assert!(ex.dialog().is_none());
        landed(&mut ex);
        let want = format!(
            "# mine\nsftp://h/srv Server\n{} Work & play\n",
            reprisefs::thumbnails::file_uri(work.path())
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), want);
    }

    #[test]
    fn add_to_favorites_on_a_favorite_offers_its_name() {
        let mut ctx = test_ctx();
        let work = tempfile::tempdir().unwrap();
        let line = format!("{} Job\n", reprisefs::thumbnails::file_uri(work.path()));
        let (_dir, file) = with_file(&line);
        let mut ex = reading(&file, &mut ctx);
        ex.go(Place::Folder(work.path().to_path_buf()), &mut ctx);
        ex.run(Command::AddFavorite, &mut ctx);
        let Some(Dialog::AddFavorite(a)) = ex.dialogs.up_mut() else {
            panic!("Add Favorite is up");
        };
        assert_eq!(a.name.value().as_deref(), Some("Job"));
    }

    #[test]
    fn add_to_favorites_is_greyed_off_a_folder() {
        let mut ctx = test_ctx();
        let ex = test_explorer(Place::Computer, &mut ctx);
        assert_eq!(ex.favorite_dir(), None);
    }

    #[test]
    fn organize_renames_moves_and_deletes_in_the_file() {
        let mut ctx = test_ctx();
        let (_dir, file) =
            with_file("file:///srv/a A\n\n# note\nsmb://nas/share\nrecent:/// Recent\n");
        let mut ex = reading(&file, &mut ctx);
        ex.run(Command::OrganizeFavorites, &mut ctx);
        assert_eq!(listed(&mut ex), ["A", "smb://nas/share", "Recent"]);

        pick(&mut ex, 1, &mut ctx);
        let f = organize_form(&mut ex);
        if let Some(Control::Edit { edit, .. }) = f.item_mut(fav::NAME).map(|i| &mut i.control) {
            edit.set_selected("NAS");
        }
        ex.dialog_outcome(form::Event::Changed(fav::NAME), &mut ctx);
        ex.dialog_outcome(form::Event::Pressed(fav::RENAME), &mut ctx);
        landed(&mut ex);
        ex.dialog_outcome(form::Event::Pressed(fav::UP), &mut ctx);
        landed(&mut ex);
        assert_eq!(listed(&mut ex), ["NAS", "A", "Recent"]);
        assert_eq!(
            fav::selected(organize_form(&mut ex)),
            Some(0),
            "the pick moves along"
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "smb://nas/share NAS\n\n# note\nfile:///srv/a A\nrecent:/// Recent\n"
        );

        pick(&mut ex, 2, &mut ctx);
        ex.dialog_outcome(form::Event::Pressed(fav::DELETE), &mut ctx);
        landed(&mut ex);
        assert_eq!(listed(&mut ex), ["NAS", "A"]);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "smb://nas/share NAS\n\n# note\nfile:///srv/a A\n"
        );
        ex.dialog_outcome(form::Event::Pressed(fav::CLOSE), &mut ctx);
        assert!(ex.dialog().is_none());
    }

    #[test]
    fn organize_follows_a_change_another_program_writes() {
        let mut ctx = test_ctx();
        let (_dir, file) = with_file("file:///srv/a A\n");
        let mut ex = reading(&file, &mut ctx);
        ex.run(Command::OrganizeFavorites, &mut ctx);
        std::fs::write(&file, "file:///srv/b B\nfile:///srv/a A\n").unwrap();
        wait_for(&mut ex, |ex| ex.favorites().len() == 2);
        assert!(ex.favorites_changed());
        assert_eq!(listed(&mut ex), ["B", "A"]);
        assert_eq!(
            fav::selected(organize_form(&mut ex)),
            Some(1),
            "the pick stays on A"
        );
    }

    #[test]
    fn add_to_favorites_on_a_file_it_cannot_read_says_so_and_leaves_it() {
        let mut ctx = test_ctx();
        let big = "file:///srv/a\n".repeat(80_000);
        let (_dir, file) = with_file(&big);
        let mut ex = reading(&file, &mut ctx);
        let work = tempfile::tempdir().unwrap();
        ex.go(Place::Folder(work.path().to_path_buf()), &mut ctx);
        ex.run(Command::AddFavorite, &mut ctx);
        ex.dialog_outcome(form::Event::Default, &mut ctx);
        wait_for(&mut ex, |ex| !ex.favorites_busy());
        assert!(ex.take_favorites_failed(&mut ctx).is_some());
        match ex.dialog() {
            Some(Dialog::Message { title, text, .. }) => {
                assert_eq!(title, "Favorites");
                assert!(
                    text.starts_with("Explorer cannot save your Favorites."),
                    "{text}"
                );
            }
            _ => panic!("a message box is up"),
        }
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            big,
            "left as it was"
        );
    }

    #[test]
    fn a_pin_right_after_an_add_lands_beside_it_and_a_failed_pin_says_so() {
        let mut ctx = test_ctx();
        let (_dir, file) = with_file("");
        let mut ex = reading(&file, &mut ctx);
        let (work, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        ex.go(Place::Folder(work.path().to_path_buf()), &mut ctx);
        ex.run(Command::AddFavorite, &mut ctx);
        ex.dialog_outcome(form::Event::Default, &mut ctx);
        ex.set_pinned(other.path(), true);
        landed(&mut ex);
        let uri = reprisefs::thumbnails::file_uri;
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            format!("{}\n{}\n", uri(work.path()), uri(other.path()))
        );

        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        ex.set_pinned(work.path(), false);
        wait_for(&mut ex, |ex| !ex.favorites_busy());
        assert!(ex.take_favorites_failed(&mut ctx).is_some());
        assert!(matches!(ex.dialog(), Some(Dialog::Message { .. })));
    }

    #[test]
    fn a_failed_edit_keeps_a_closed_window_up_for_its_box() {
        let mut ctx = test_ctx();
        let (_dir, file) = with_file("");
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        let mut ex = reading(&file, &mut ctx);
        let work = tempfile::tempdir().unwrap();
        ex.set_pinned(work.path(), true);
        assert!(!ex.close(&mut ctx).quit, "the edit is still to be written");
        wait_for(&mut ex, |ex| !ex.favorites_busy());
        assert!(!ex.wake(&mut ctx).quit, "the failure is still to be told");
        assert!(matches!(ex.dialog(), Some(Dialog::Message { .. })));
    }

    #[test]
    fn the_favorites_menu_lists_each_bookmark() {
        let mut ctx = test_ctx();
        let (_dir, file) = with_file("file:///nonexistent-fav/Gone\nsftp://h/srv Server\n");
        let ex = reading(&file, &mut ctx);
        let rows = ex.menu_state().favorites;
        let got: Vec<(&str, bool)> = rows
            .iter()
            .map(|r| (r.label.as_str(), r.go.is_some()))
            .collect();
        assert_eq!(got, [("Gone", false), ("Server", true)]);
    }
}
