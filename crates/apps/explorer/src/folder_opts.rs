//! Folder Options' host half: its window, OK and Apply writing `[folders]` and
//! `[thumbnail_cache] enabled` in `config.toml`, and every window following the file
//! ([`Explorer::take_config`]). View > Hidden Items writes `[folders] show_hidden` the same way.

use crate::Explorer;
use crate::acting::Action;
use crate::dialogs::SheetId;
use crate::settings::Folders;
use repriseapp::{Child, Ctx, Event, Reply, WinId};
use repriseconf::fields::Field;
use repriseconf::{key as k, lens};
use reprisefs::deadline::Gate;
use reprisekit::controls::folderopts::{FolderSheet, Outcome, Settings};
use reprisekit::controls::propsheet;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::tr;
use std::ffi::OsStr;

pub const FOLDER_OPTS: WinId = WinId::named("folder-options");
fn title() -> String {
    tr!("folder-options-title-folder-options")
}

pub(crate) struct FolderOpts {
    pub(crate) sheet: FolderSheet,
    /// Which opening of the sheet window this is ([`crate::dialogs::SheetId`]).
    pub(crate) opened: u64,
}

impl FolderOpts {
    pub(crate) fn id(&self) -> SheetId {
        SheetId {
            win: FOLDER_OPTS,
            opened: self.opened,
        }
    }
}

/// The sheet's boxes for `f`.
pub(crate) fn settings_of(f: &Folders) -> Settings {
    Settings {
        new_window: f.new_window,
        single_click: f.single_click,
        show_hidden: f.show_hidden,
        hide_extensions: f.hide_extensions,
        tips: f.tips,
        cache_thumbnails: f.cache_thumbnails,
    }
}

/// The folder settings as the boxes `s` leave them.
fn folders_of(s: &Settings) -> Folders {
    Folders {
        new_window: s.new_window,
        single_click: s.single_click,
        show_hidden: s.show_hidden,
        hide_extensions: s.hide_extensions,
        tips: s.tips,
        cache_thumbnails: s.cache_thumbnails,
    }
}

/// The keys the sheet writes, as `config.toml` spells them.
#[derive(Clone)]
struct Keys {
    new_window: bool,
    single_click: bool,
    show_hidden: bool,
    hide_extensions: bool,
    tips: bool,
    thumbnails: bool,
}

impl Keys {
    fn of(s: &Settings) -> Keys {
        Keys {
            new_window: s.new_window,
            single_click: s.single_click,
            show_hidden: s.show_hidden,
            hide_extensions: s.hide_extensions,
            tips: s.tips,
            thumbnails: s.cache_thumbnails,
        }
    }
}

fn fields() -> Vec<Field<Keys>> {
    vec![
        Field::Flag(k("folders", "new_window"), lens!(new_window)),
        Field::Flag(k("folders", "single_click"), lens!(single_click)),
        Field::Flag(k("folders", "show_hidden"), lens!(show_hidden)),
        Field::Flag(k("folders", "hide_extensions"), lens!(hide_extensions)),
        Field::Flag(k("folders", "tips"), lens!(tips)),
        Field::Flag(k("thumbnail_cache", "enabled"), lens!(thumbnails)),
    ]
}

/// `text` with the keys `want` moved from `base`; a key back at its default goes.
fn write(text: &str, want: &Settings, base: &Settings) -> Result<String, String> {
    let def = Keys::of(&Settings::default());
    repriseconf::fields::save_text(text, &fields(), &Keys::of(want), &Keys::of(base), &def)
}

impl Explorer {
    pub(crate) fn open_folder_opts(&mut self, ctx: &mut Ctx) -> Reply {
        if self.folder_opts.is_some() {
            return Reply::none();
        }
        let mut sheet = FolderSheet::new(settings_of(&self.folders));
        sheet.sheet.fit_tabs(&ctx.text, ctx.forms().fonts.size_px);
        let opened = self.next_sheet();
        self.cancel_pointer();
        self.folder_opts = Some(FolderOpts { sheet, opened });
        Reply::redraw_all()
    }

    pub fn folder_opts_child(&self) -> Option<Child> {
        let f = self.folder_opts.as_ref()?;
        Some(Child {
            id: FOLDER_OPTS,
            title: title(),
            size: self.forms.text.size(f.sheet.sheet.size),
            fixed: true,
            modal: true,
        })
    }

    pub fn draw_folder_opts(&self, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        c.fill_rect(c.bounds(), forms.colors.button_face);
        if let Some(f) = &self.folder_opts {
            let p = Paint {
                s: 1,
                ..forms.paint()
            };
            propsheet::draw(c, &ctx.text, &p, &f.sheet.sheet, true, forms.fonts.size_px);
        }
    }

    pub fn folder_opts_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if matches!(ev, Event::CloseRequest) {
            return self.close_folder_opts(ctx);
        }
        let Some(f) = self.folder_opts.as_mut() else {
            return Reply::none();
        };
        let sheet = &mut f.sheet.sheet;
        let out = repriseapp::keys::sheet_input(sheet, &ev, ctx).unwrap_or(propsheet::Event::None);
        match f.sheet.event(out) {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Close => self.close_folder_opts(ctx),
            Outcome::Save { settings, close } => {
                let base = settings_of(&self.folders);
                self.save_folders(settings, base, close, ctx)
            }
            Outcome::DefaultPrograms => {
                let argv = ["reprise-settings", "--page", "default-programs"].map(OsStr::new);
                crate::spawn_in(&argv, "reprise-settings")
            }
        }
    }

    /// Closes the sheet, the one way it goes, with the boxes it raised ([`Explorer::dismiss`]).
    fn close_folder_opts(&mut self, ctx: &mut Ctx) -> Reply {
        let mut reply = Reply::redraw_all();
        for d in (self.dialogs).withdraw(|d| d.sheet() == Some(FOLDER_OPTS)) {
            reply = reply.and(self.dismiss(d, ctx));
        }
        self.folder_opts = None;
        reply
    }

    /// View > Hidden Items: every window and the desktop follow `[folders] show_hidden`.
    pub(crate) fn toggle_hidden(&mut self, ctx: &mut Ctx) -> Reply {
        let base = settings_of(&self.folders);
        let want = Settings {
            show_hidden: !base.show_hidden,
            ..base
        };
        self.save_folders(want, base, false, ctx)
    }

    /// Shows `want` in this window at once and writes the keys it moved from `base`; OK closes
    /// the sheet first. A write that fails says so in a box.
    fn save_folders(
        &mut self,
        want: Settings,
        base: Settings,
        close: bool,
        ctx: &mut Ctx,
    ) -> Reply {
        let sheet = self.folder_opts.as_ref().map(FolderOpts::id);
        let mut reply = match close {
            true => self.close_folder_opts(ctx),
            false => Reply::redraw_all(),
        };
        let sheet = sheet.filter(|_| !close);
        reply = reply.and(self.set_folders(folders_of(&want), ctx));
        let Some(path) = self.config_file.clone() else {
            return reply;
        };
        self.folder_saves.asked += 1;
        let (n, written) = (self.folder_saves.asked, self.folder_saves.written.clone());
        let work = move |gate: &Gate| {
            let mut last = reprisebus::mutex::lock(&written);
            if *last > n || !gate.change(&path) {
                return Ok(());
            }
            *last = n;
            let saved = repriseconf::edit(&path, |text| write(text, &want, &base));
            saved.map_err(|e| e.to_string()).and_then(|r| r.map(drop))
        };
        let action = match sheet {
            Some(s) => Action::changes(title()).of(s),
            None => Action::changes(title()),
        };
        let saved = self.act(ctx, action, work, move |ex, saved, ctx| match saved {
            Ok(()) => Reply::none(),
            Err(e) => {
                log::error!("folder options: not saved: {e}");
                let text = tr!(
                    "folder-options-save-folders-cannot-save-the-folder-options-in",
                    e = e.to_string()
                );
                ex.message_over(sheet, &title(), text, ctx)
            }
        });
        reply.and(saved)
    }

    /// The config file changed on disk: this window follows what it says now.
    pub(crate) fn take_config(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        if !self.config_watch.as_mut()?.take_changed() {
            return None;
        }
        Some(self.set_folders(Folders::current(), ctx))
    }

    /// Lists again when what shows changed; an open sheet shows `f` where its boxes are untouched.
    pub(crate) fn set_folders(&mut self, f: Folders, ctx: &mut Ctx) -> Reply {
        if f == self.folders {
            return Reply::none();
        }
        let relist = (f.show_hidden, f.hide_extensions)
            != (self.folders.show_hidden, self.folders.hide_extensions);
        self.folders = f;
        self.click_open = None;
        if let Some(o) = self.folder_opts.as_mut() {
            o.sheet.saved_elsewhere(settings_of(&f));
        }
        self.refresh_chrome(ctx);
        if !relist {
            return Reply::redraw_all();
        }
        self.rescan_tree(ctx);
        self.refresh(ctx).and(Reply::redraw_all())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::place::Place;
    use crate::tests::{test_ctx, test_explorer, wake_until};
    use reprisekit::textedit::keysym;

    fn key(sym: u32) -> Event {
        let text = (sym == keysym::SPACE).then(|| " ".to_string());
        Event::Key(repriseapp::KeyInput {
            sym,
            edit: text.clone().map(reprisekit::textedit::EditKey::Insert),
            utf8: text,
            mods: repriseapp::Mods::default(),
        })
    }

    fn hidden_folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(dir.path().join(".secret"), "x").expect("write");
        std::fs::write(dir.path().join("notes.txt"), "x").expect("write");
        dir
    }

    fn names(ex: &Explorer) -> Vec<&str> {
        ex.sv.entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn tools_folder_options_opens_the_sheet_on_general() {
        let mut ctx = test_ctx();
        let dir = hidden_folder();
        let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.run(crate::commands::Command::FolderOptions, &mut ctx);
        let child = ex.children().into_iter().find(|c| c.id == FOLDER_OPTS);
        let child = child.expect("the sheet's window");
        assert_eq!(child.title, title());
        assert!(child.modal);
        let tabs = &ex.folder_opts.as_ref().unwrap().sheet.sheet.tabs;
        assert_eq!(tabs.labels, ["General", "View", "File Types"]);
        assert_eq!(tabs.open, 0);
    }

    #[test]
    fn ok_writes_the_moved_keys_lists_hidden_items_and_closes() {
        let dir = hidden_folder();
        let config = dir.path().join("config.toml");
        let mine = "# mine\nterminal = \"foot\"\ndesktop_extensions = \"hide_launchers\"\n";
        std::fs::write(&config, mine).expect("write");
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.refresh(&mut ctx);
        assert_eq!(names(&ex), ["config.toml", "notes.txt"]);
        ex.open_folder_opts(&mut ctx);
        ex.config_file = Some(config.clone());
        let f = ex.folder_opts.as_mut().unwrap();
        f.sheet.sheet.open_tab(1);
        f.sheet.sheet.open_mut().focus_id("show");
        ex.folder_opts_event(key(keysym::SPACE), &mut ctx);
        ex.folder_opts_event(key(keysym::RETURN), &mut ctx);
        assert!(ex.folder_opts.is_none(), "OK closes the sheet");
        assert_eq!(names(&ex), [".secret", "config.toml", "notes.txt"]);
        wake_until(&mut ex, &mut ctx, |ex| ex.acting.is_empty());
        let text = std::fs::read_to_string(&config).expect("read");
        assert!(text.starts_with(mine), "{text}");
        assert!(text.ends_with("[folders]\nshow_hidden = true\n"), "{text}");
        let c = repriseconf::config::Config::parse(&text).expect("parses");
        assert!(c.folders.show_hidden);
    }

    #[test]
    fn the_extensions_box_writes_folders_hide_extensions_and_leaves_the_desktops_key() {
        let off = Settings::default();
        let on = Settings {
            hide_extensions: true,
            ..off
        };
        let desktop = "desktop_extensions = \"show\"\n";
        let hidden = write(desktop, &on, &off).unwrap();
        assert!(hidden.starts_with(desktop), "{hidden}");
        assert!(
            hidden.ends_with("[folders]\nhide_extensions = true\n"),
            "{hidden}"
        );
        assert_eq!(write(&hidden, &off, &on).unwrap(), desktop);
        assert!(folders_of(&on).hide_extensions && !folders_of(&off).hide_extensions);
    }

    #[test]
    fn hidden_items_writes_show_hidden_and_a_second_press_takes_it_out() {
        let dir = hidden_folder();
        let config = dir.path().join("config.toml");
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.config_file = Some(config.clone());
        ex.run(crate::commands::Command::ToggleHidden, &mut ctx);
        wake_until(&mut ex, &mut ctx, |ex| ex.acting.is_empty());
        assert!(ex.folders.show_hidden);
        assert!(ex.items.iter().any(|it| it.name == ".secret"));
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "[folders]\nshow_hidden = true\n"
        );
        ex.run(crate::commands::Command::ToggleHidden, &mut ctx);
        wake_until(&mut ex, &mut ctx, |ex| ex.acting.is_empty());
        assert!(!ex.folders.show_hidden);
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "");
    }

    #[test]
    fn known_extensions_hide_and_a_rename_puts_the_hidden_one_back() {
        let dir = hidden_folder();
        std::fs::write(dir.path().join("data.zzz"), "x").expect("write");
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.globs = std::sync::Arc::new(reprisefs::apps::MimeDb::parse(["50:text/plain:*.txt\n"]));
        ex.set_folders(
            Folders {
                hide_extensions: false,
                ..ex.folders
            },
            &mut ctx,
        );
        assert_eq!(names(&ex), ["data.zzz", "notes.txt"]);
        ex.set_folders(
            Folders {
                hide_extensions: true,
                ..ex.folders
            },
            &mut ctx,
        );
        assert_eq!(names(&ex), ["data.zzz", "notes"], "only a known type's");
        let i = ex
            .items
            .iter()
            .position(|it| it.name == "notes.txt")
            .unwrap();
        ex.sv.selection.set_only(Some(i));
        ex.begin_rename(&mut ctx);
        let (_, edit) = ex.sv.edit.as_mut().expect("the rename box");
        assert_eq!(edit.text, "notes");
        *edit = reprisekit::textedit::TextEdit::new("plans", false);
        ex.commit_rename(&mut ctx);
        wake_until(&mut ex, &mut ctx, |ex| ex.acting.is_empty());
        assert!(dir.path().join("plans.txt").exists());
    }

    #[test]
    fn a_change_saved_elsewhere_relists_and_shows_in_the_open_sheet() {
        let dir = hidden_folder();
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.open_folder_opts(&mut ctx);
        let hidden = Folders {
            show_hidden: true,
            ..ex.folders
        };
        ex.set_folders(hidden, &mut ctx);
        assert!(ex.items.iter().any(|it| it.name == ".secret"));
        let sheet = &ex.folder_opts.as_ref().unwrap().sheet;
        assert!(sheet.pending.show_hidden && sheet.applied.show_hidden);
    }
}
