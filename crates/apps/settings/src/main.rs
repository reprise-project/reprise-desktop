//! Reprise Settings: a property sheet over `config.toml` and the XDG autostart directories.
//!
//! Pages are listed in `registry::GROUPS`; `--page night-light` opens on one, and a second start hands its page to the running sheet (`reprisebus::instance`). OK and Apply rewrite `config.toml` with `toml_edit` and the autostart files. The Startup list's Add... and Remove, Quick Launch changes and the Advanced page's Forget act at once. The Display page goes through wlr-output-management (`repriseapp::heads`); only a Yes on the Monitor Settings question writes `[[outputs]]`.

mod additem;
mod appearancepage;
mod autostart;
mod backgroundpage;
mod cursors;
mod defaultspage;
mod displayflow;
mod displayhost;
mod displaypage;
mod layoutdlg;
mod menupage;
mod model;
mod monitordlg;
mod outputwrite;
mod pages;
mod quickpage;
mod registry;
mod renamedlg;
mod saverdlg;
mod saverpage;
mod shortcutdlg;
mod startmenu;
mod tabletpage;
mod visionpage;
mod wallpapers;
mod winstate;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use repriseapp::decode::{Decoded, Decoders};
use repriseapp::filedlg::{Host, Resolved, Step, type_keep};
use repriseapp::keys::{dialog_key, form_input};
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::openwith::ChooserInput;
use repriseapp::{
    App, BTN_LEFT, Child, Ctx, DropForm, Event, FileDrop, KeyInput, PointerInput, Reply, Waker,
    WinId,
};
use reprisebus::instance::{self, Claim};
use reprisefs::autostart::Entry;
use reprisekit::access::{ROOT, Tree};
use reprisekit::classic::widgets::BarHit;
use reprisekit::controls::filedlg::{self, DlgMode, Event as FileEvent, Fields, FileDlg};
use reprisekit::controls::form;
use reprisekit::controls::listview::{self, ListView};
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::msgbox;
use reprisekit::controls::openwith::{self, OpenWith, Program};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::icontheme::IconTheme;
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::{FontConfig, Theme};

use crate::pages::Area;
use crate::registry::Page;

/// The file dialog: Startup Add... and Background Browse....
const ADD_WIN: WinId = "add";
const SHORTCUT_WIN: WinId = "shortcut";
const LAYOUT_WIN: WinId = "layout";
const ADD_ITEM_WIN: WinId = "add_item";
const RENAME_WIN: WinId = "rename";
const SAVER_WIN: WinId = "saver";

const CHOOSER_WIN: WinId = "openwith";

const ERROR_WIN: WinId = "error";

struct Chooser {
    mime: String,
    ow: OpenWith,
    input: ChooserInput,
    icon: Option<Canvas>,
    row_icons: Vec<Option<Canvas>>,
    icons_due: bool,
}

/// The icon themes the combo offers: every directory with an `index.theme` under the icon roots.
fn icon_themes() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for root in IconTheme::default_roots() {
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        for e in rd.flatten() {
            if !e.path().join("index.theme").is_file() {
                continue;
            }
            let Some(name) = e.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out.sort();
    out
}

/// Theme directories the compositor writes the generated GTK theme into (`GTK_SLOTS` in `src/toolkits.rs`).
const GENERATED_GTK_THEMES: [&str; 2] = ["Reprise", "Reprise-alt"];

fn qt_tools() -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|d| d.join("qt6ct").is_file() || d.join("qt5ct").is_file())
}

/// The GTK themes the Programs combo offers; the generated ones stay out, the Reprise row stands for them.
fn gtk_themes() -> Vec<String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.clone().map(|h| h.join(".local/share")));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let roots = data_home
        .map(|d| d.join("themes"))
        .into_iter()
        .chain(home.map(|h| h.join(".themes")))
        .chain(
            data_dirs
                .split(':')
                .filter(|s| !s.is_empty())
                .map(|d| PathBuf::from(d).join("themes")),
        );
    let mut out = vec!["Adwaita".to_string()];
    for root in roots {
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if !p.join("gtk-3.0").is_dir() && !p.join("gtk-4.0").is_dir() {
                continue;
            }
            if let Some(name) = e.file_name().to_str()
                && !GENERATED_GTK_THEMES.contains(&name)
                && !out.iter().any(|o| o == name)
            {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out
}

/// Names of `s`'s programs by app id, from their `.desktop` files; a program without one shows as its app id.
fn app_names(s: &model::Settings) -> HashMap<String, String> {
    let ids: Vec<&String> = (s.programs.iter().map(|p| &p.app))
        .chain(&s.denied)
        .collect();
    if ids.is_empty() {
        return HashMap::new();
    }
    let entries = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
    ids.into_iter()
        .filter_map(|id| {
            let e = reprisefs::apps::entry_for_app_id(&entries, id, |_| true)?;
            Some((id.clone(), e.name.clone()))
        })
        .collect()
}

/// `config.toml`'s text, and the error message when it is unreadable or does not parse; a missing file is the defaults.
fn read_config(path: &Path) -> (String, Option<String>) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            log::warn!("read {}: {e}", path.display());
            return (
                String::new(),
                Some(format!(
                    "config.toml could not be read: {e}.\nThe sheet shows the defaults."
                )),
            );
        }
    };
    let err = text.parse::<toml_edit::DocumentMut>().err().map(|e| {
        let line = e
            .span()
            .and_then(|r| text.get(..r.start))
            .map_or(1, |before| before.matches('\n').count() + 1);
        let what = e
            .message()
            .lines()
            .next()
            .unwrap_or("")
            .trim_end_matches('.');
        format!(
            "config.toml does not parse at line {line}: {what}.\n\
             The sheet shows the defaults. Apply leaves the file alone until it parses."
        )
    });
    (text, err)
}

fn page_arg(mut args: impl Iterator<Item = String>) -> Option<Page> {
    while let Some(a) = args.next() {
        let name = match a.strip_prefix("--page=") {
            Some(n) => n.to_string(),
            None if a == "--page" => args.next()?,
            None => continue,
        };
        let page = Page::named(&name);
        if page.is_none() {
            log::warn!("--page {name:?} names no page");
        }
        return page;
    }
    None
}

/// The bus name the running sheet owns; a later start sends it a `page` action.
const BUS_NAME: &str = "org.reprise.Settings";
const PAGE_ACTION: &str = "page";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    Autostart,
    Wallpaper,
    SlideFolder,
}

fn take<T>(rx: &Receiver<T>) -> Result<Option<T>, ()> {
    match rx.try_recv() {
        Ok(v) => Ok(Some(v)),
        Err(TryRecvError::Empty) => Ok(None),
        Err(TryRecvError::Disconnected) => Err(()),
    }
}

struct SettingsApp {
    path: PathBuf,
    loaded: model::Settings,
    want: model::Settings,
    sheet: PropSheet,

    entries: Vec<Entry>,
    applied_checks: Vec<bool>,
    /// What the compositor filters `OnlyShowIn` / `NotShowIn` against.
    desktops: Vec<String>,
    /// The list's own `checks` are the pending state.
    list: ListView,
    list_focus: bool,

    shortcuts: ListView,
    shortcuts_focus: bool,
    shortcut: Option<shortcutdlg::ShortcutDialog>,
    shortcut_target: Option<model::Row>,
    app_names: HashMap<String, String>,

    defaults: Option<defaultspage::Defaults>,
    types: ListView,
    types_focus: bool,
    chooser: Option<Chooser>,

    locations: ListView,
    locations_focus: bool,
    location_ids: Vec<String>,
    location_file: PathBuf,

    pads: Vec<reprisebus::tray::Pad>,
    pad_picked: usize,
    pads_fresh: bool,

    xkb: Vec<reprisefs::xkb::Layout>,
    add_layout: Option<layoutdlg::LayoutDialog>,

    add_item: Option<additem::AddItemDialog>,
    rename: Option<renamedlg::RenameDialog>,
    menu_icons_due: bool,
    saver_dlg: Option<saverdlg::SaverDialog>,
    saver_preview: Option<(saverpage::Preview, std::time::Instant)>,
    saver_ticking: bool,
    saver_photos: Option<(String, Receiver<Vec<Canvas>>)>,
    add_to_quick: bool,

    quick_dir: Option<PathBuf>,
    quick: Vec<quickpage::Launcher>,
    quick_icons_due: bool,
    controls: ControlMetrics,

    add: Option<FileDlg>,
    add_host: Host,
    add_for: Pick,

    error: Option<MsgBox>,
    dialog_text: TextRenderer,
    dialog_px: u32,
    err_icon: Option<Canvas>,
    open_error: Option<String>,
    preview_for: Option<(String, i64, reprisekit::theme::Color)>,
    display: displayhost::DisplayHost,
    wallpapers: Vec<PathBuf>,
    picture: Option<backgroundpage::Picture>,
    wallpaper_icons: Option<(Option<Canvas>, Option<Canvas>)>,
    folder_icon: Option<Canvas>,
    thumbs: Vec<Option<Canvas>>,
    thumbing: Option<Receiver<Vec<Option<Canvas>>>>,
    requests: Option<instance::Requests>,
    waker: Option<Waker>,
    scan: Option<Receiver<Vec<PathBuf>>>,
    decoding: Option<(String, Receiver<Option<Decoded>>)>,
    state_path: Option<PathBuf>,
    window: (i32, i32),
    window_dirty: bool,
}

impl SettingsApp {
    fn new(page: Option<Page>, requests: Option<instance::Requests>) -> SettingsApp {
        let mut app = SettingsApp::open(repriseconf::config_path());
        if let Some(p) = page {
            app.sheet.tabs.open = p.index();
            app.sheet.reveal_open();
        }
        app.requests = requests;
        let state = winstate::path();
        app.window = winstate::load(&state, Area::MIN.sheet_size());
        app.state_path = Some(state);
        app
    }

    /// The area the pages fill at the sheet's size now.
    fn area(&self) -> Area {
        Area::of(&self.sheet)
    }

    /// Lay the sheet out for a main window of client `size`: the sheet
    /// takes it (no smaller than its smallest), every page takes the
    /// layout its builder gives it there, and the Pointers preview is
    /// drawn again at its new size. What the controls hold stays. True
    /// when anything moved.
    fn fit_window(&mut self, size: (i32, i32)) -> bool {
        let min = Area::MIN.sheet_size();
        let size = (size.0.max(min.0), size.1.max(min.1));
        if size == self.sheet.size {
            return false;
        }
        self.sheet.resize(size);
        let a = self.area();
        let offset = self.sheet.page_offset();
        let src = registry::Sources {
            icon_themes: &[],
            gtk_themes: &[],
            qt_tools: false,
            cursor_themes: &[],
            layouts: &[],
            heads: &[],
        };
        for p in Page::all() {
            // The pages built from what they show are built from it here
            // too, so the template has the controls the page has.
            let template = match p {
                Page::Appearance => appearancepage::page(&self.want, a),
                Page::Display => displaypage::page(&self.display.model, a),
                Page::Tablet => {
                    tabletpage::page(a, &self.pads, self.pad_picked, &self.want.tablet_pads)
                }
                _ => registry::build(p, &src, a),
            };
            self.sheet.pages[p.index()].take_layout(&template, offset);
        }
        // Command takes the Startup list's new width; a column the user
        // dragged keeps its width until the window resizes.
        pages::fit_list_columns(&mut self.list, pages::list(a));
        self.preview_for = None;
        self.sheet.reveal_open();
        true
    }

    fn save_window(&mut self) {
        if !self.window_dirty {
            return;
        }
        self.window_dirty = false;
        if let Some(path) = &self.state_path {
            winstate::save(path, self.window);
        }
    }

    fn open(path: PathBuf) -> SettingsApp {
        let (text, open_error) = read_config(&path);
        let loaded = model::load(&text);
        let xkb = reprisefs::xkb::load("");
        let mut sheet = registry::sheet(
            &loaded,
            &registry::Sources {
                icon_themes: &icon_themes(),
                gtk_themes: &gtk_themes(),
                qt_tools: qt_tools(),
                cursor_themes: &cursors::themes(),
                layouts: &xkb,
                heads: &[],
            },
        );
        let wallpapers: Vec<PathBuf> = wallpapers::expand(&loaded.wallpaper).into_iter().collect();
        backgroundpage::fill(
            &mut sheet.pages[Page::Background.index()],
            &wallpapers,
            &loaded.wallpaper,
            None,
            &[],
        );
        let dialog_text = TextRenderer::load(&FontConfig::default());
        let dialog_px = Theme::default().fonts.size_px;
        let mut app = SettingsApp {
            path,
            want: loaded.clone(),
            loaded,
            sheet,
            entries: Vec::new(),
            applied_checks: Vec::new(),
            desktops: reprisefs::autostart::desktop_names(
                std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
            ),
            list: pages::new_list(),
            list_focus: false,
            shortcuts: pages::new_shortcut_list(),
            shortcuts_focus: false,
            shortcut: None,
            shortcut_target: None,
            app_names: HashMap::new(),
            defaults: None,
            pads: Vec::new(),
            pad_picked: 0,
            pads_fresh: false,
            types: defaultspage::new_list(),
            types_focus: false,
            chooser: None,
            locations: pages::new_location_list(),
            locations_focus: false,
            location_ids: Vec::new(),
            location_file: repriseconf::location::path(),
            xkb,
            add_layout: None,
            add_item: None,
            rename: None,
            saver_dlg: None,
            saver_preview: None,
            saver_ticking: false,
            saver_photos: None,
            menu_icons_due: true,
            add_to_quick: false,
            quick_dir: reprisefs::quicklaunch::dir(),
            quick: Vec::new(),
            quick_icons_due: true,
            controls: ControlMetrics::CLASSIC,
            add: None,
            add_host: Host::default(),
            add_for: Pick::Autostart,
            error: None,
            dialog_text,
            dialog_px,
            err_icon: None,
            open_error,
            preview_for: None,
            display: displayhost::DisplayHost::default(),
            wallpapers,
            picture: None,
            wallpaper_icons: None,
            folder_icon: None,
            thumbs: Vec::new(),
            thumbing: None,
            requests: None,
            waker: None,
            scan: None,
            decoding: None,
            state_path: None,
            window: Area::MIN.sheet_size(),
            window_dirty: false,
        };
        app.display
            .model
            .set_primary_saved(outputwrite::read_primary(&text));
        app.rescan();
        app.app_names = app_names(&app.want);
        app.fill_shortcuts();
        app.reload_locations();
        app.quick_reload(None);
        app.show_monitor();
        app
    }

    fn refresh_preview(&mut self, window: reprisekit::theme::Color) {
        let key = (
            self.want.cursor_theme.clone(),
            self.want.cursor_size,
            window,
        );
        if self.preview_for.as_ref() == Some(&key) {
            return;
        }
        let pic = cursors::preview(&key.0, key.1, pages::preview_size(self.area()), window);
        self.sheet.pages[Page::Pointers.index()].set_picture(pages::CURSOR_PREVIEW, pic);
        self.preview_for = Some(key);
    }

    fn user_autostart(&self) -> PathBuf {
        autostart::user_dir(&reprisefs::autostart::config_home())
    }

    /// Re-read the autostart directories; unapplied checks are dropped, the files changed under them.
    fn rescan(&mut self) {
        self.entries = reprisefs::autostart::scan(
            &reprisefs::autostart::system_dirs(),
            &reprisefs::autostart::config_home(),
        );
        // Checked means it starts here: a system entry fenced off by
        // `OnlyShowIn` is listed unchecked, the way the compositor skips it.
        self.applied_checks = self
            .entries
            .iter()
            .map(|e| e.runs_on(&self.desktops))
            .collect();
        let checks = self.applied_checks.clone();
        pages::fill_list(&mut self.list, &self.entries, &checks);
        self.sync_remove();
    }

    fn checks(&self) -> Vec<bool> {
        self.list.checks.clone().unwrap_or_default()
    }

    fn dirty(&self) -> bool {
        self.want != self.loaded
            || self.checks() != self.applied_checks
            || self.display.model.dirty()
            || self.defaults.as_ref().is_some_and(|d| d.dirty())
    }

    /// Remove works on user entries only; a system entry has no file of the user's.
    fn sync_remove(&mut self) {
        let on = self
            .list
            .selection
            .focus
            .and_then(|i| self.entries.get(i))
            .is_some_and(|e| e.user);
        self.sheet.pages[Page::Startup.index()].set_enabled(pages::REMOVE, on);
    }

    fn sync_apply(&mut self) {
        self.sync_remove();
        self.sync_forget();
        menupage::sync(
            &mut self.sheet.pages[Page::StartMenu.index()],
            &self.want.startmenu,
        );
        quickpage::sync(
            &mut self.sheet.pages[Page::StartMenu.index()],
            self.quick.len(),
        );
        if let Some(d) = &self.defaults {
            defaultspage::sync_buttons(
                &mut self.sheet.pages[Page::DefaultPrograms.index()],
                d,
                self.types.selection.focus,
            );
        }
        let dirty = self.dirty();
        self.sheet.set_apply_enabled(dirty);
    }

    /// Write `config.toml` and the autostart files whose check moved. False when a write failed; the sheet stays dirty.
    fn apply(&mut self) -> bool {
        if let Err(reason) = model::validate(&self.want, &self.loaded) {
            self.fail(&reason);
            return false;
        }
        let mut ok = true;
        if self.want != self.loaded {
            // Re-read under the lock: the user's editor or reprise-power may
            // have written since the sheet opened.
            let saved = repriseconf::edit(&self.path, |text| {
                model::save(text, &self.want, &self.loaded)
            });
            match saved {
                // A file that stopped parsing is not ours to rewrite: it holds keys the sheet does not know.
                Ok(Err(e)) => {
                    log::warn!(
                        "{} does not parse, refusing to rewrite it: {e}",
                        self.path.display()
                    );
                    self.fail("config.toml does not parse; nothing was written.");
                    ok = false;
                }
                Ok(Ok(_)) => self.loaded = self.want.clone(),
                Err(e) => {
                    log::warn!("write {}: {e}", self.path.display());
                    self.fail("config.toml could not be written.");
                    ok = false;
                }
            }
        }
        let dir = self.user_autostart();
        let checks = self.checks();
        let mut changed = false;
        let mut failed_autostart = false;
        for (i, e) in self.entries.iter().enumerate() {
            let (Some(&want), Some(&have)) = (checks.get(i), self.applied_checks.get(i)) else {
                continue;
            };
            if want == have {
                continue;
            }
            match autostart::set_enabled(&dir, e, want) {
                Ok(()) => changed = true,
                Err(err) => {
                    log::warn!("autostart {}: {err}", e.id);
                    failed_autostart = true;
                    ok = false;
                }
            }
        }
        if failed_autostart {
            self.fail("The autostart entry could not be written.");
        }
        if changed {
            self.rescan();
        }
        if !self.apply_defaults() {
            ok = false;
        }
        self.sync_apply();
        ok
    }

    fn apply_defaults(&mut self) -> bool {
        let Some(d) = self.defaults.as_mut().filter(|d| d.dirty()) else {
            return true;
        };
        let written = reprisefs::props::edit_defaults(&d.edits).and_then(|()| {
            let path = reprisefs::props::user_mimeapps()?;
            std::fs::read_to_string(path)
        });
        match written {
            Ok(now) => {
                d.applied(now);
                self.refill_defaults();
                true
            }
            Err(e) => {
                log::warn!("mimeapps.list: {e}");
                self.fail("The default programs could not be written to mimeapps.list.");
                false
            }
        }
    }

    /// Show `reason` in the error box; the first failure wins.
    fn fail(&mut self, reason: &str) {
        if self.error.is_none() {
            // `dialog-error` is the theme's, so it is `None` until the
            // first event primed it and the box draws without a column.
            self.error = Some(MsgBox::new(
                "Reprise Settings",
                msgbox::Fields {
                    text: reason.to_string(),
                    icon: self.err_icon.clone(),
                    buttons: vec!["OK".into()],
                    default: 0,
                    min_text_h: None,
                    min_w: 0,
                },
            ));
        }
    }

    fn error_event(&mut self, ev: Event) -> Reply {
        let Some(b) = &mut self.error else {
            return Reply::none();
        };
        match b.event(&ev, &self.dialog_text, self.dialog_px) {
            oneshot::Outcome::Closed(_) => {
                self.error = None;
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    fn revert(&mut self) {
        self.want = self.loaded.clone();
        pages::write(&mut self.sheet, &self.want, &self.xkb);
        self.menu_icons_due = true;
        self.fill_shortcuts();
        let checks = self.applied_checks.clone();
        pages::fill_list(&mut self.list, &self.entries, &checks);
        self.display.model.reset();
        displaypage::refresh(&mut self.sheet, &self.display.model);
        if let Some(d) = &mut self.defaults {
            d.revert();
        }
        self.refill_defaults();
        self.fill_wallpapers();
        self.want_preview();
        self.show_monitor();
        self.sync_apply();
    }

    /// The Interface combo moved off `from`: the scheme and the unset defaults follow the new interface ([`model::switch_interface`]); picking back the loaded one restores the loaded scheme and glass colour.
    fn interface_moved(&mut self, from: &str) {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        model::switch_interface(&mut self.want, from, &text, &self.loaded);
        pages::write_forms(&mut self.sheet, &self.want);
    }

    /// Refill the Background list from `wallpapers` with the pending one picked and scrolled into view (once, when the scan lands).
    fn fill_wallpapers(&mut self) {
        let folders = self.slide_folders();
        let page = &mut self.sheet.pages[Page::Background.index()];
        if backgroundpage::slideshow(&self.want) {
            backgroundpage::fill_folders(
                page,
                &folders,
                &self.want.slideshow,
                self.folder_icon.as_ref(),
            );
        } else {
            backgroundpage::fill(
                page,
                &self.wallpapers,
                &self.want.wallpaper,
                self.wallpaper_icons.as_ref(),
                &self.thumbs,
            );
        }
    }

    fn slide_folders(&self) -> Vec<PathBuf> {
        backgroundpage::folders(&self.wallpapers, wallpapers::expand(&self.want.slideshow))
    }

    fn show_monitor(&mut self) {
        let file = backgroundpage::monitor_file(&self.want, &self.wallpapers);
        backgroundpage::show(
            &mut self.sheet.pages[Page::Background.index()],
            &self.want,
            &file,
            self.picture.as_ref(),
        );
    }

    /// The Picture / Slideshow combo moved: a slideshow starts on the pending wallpaper's folder.
    fn pick_mode(&mut self) {
        let page = &self.sheet.pages[Page::Background.index()];
        let slides = page.combo_selected(backgroundpage::MODE) == Some(1);
        if slides == backgroundpage::slideshow(&self.want) {
            return;
        }
        self.want.slideshow = if slides {
            wallpapers::expand(&self.want.wallpaper)
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .unwrap_or_else(wallpapers::start_dir)
                .to_string_lossy()
                .into_owned()
        } else {
            String::new()
        };
        self.fill_wallpapers();
    }

    /// A Background row was picked; the file the config already names keeps its spelling (`~/...`).
    fn pick_wallpaper(&mut self) {
        let row = backgroundpage::selected(&self.sheet.pages[Page::Background.index()]);
        if backgroundpage::slideshow(&self.want) {
            let folders = self.slide_folders();
            if let Some(f) = row.and_then(|i| folders.get(i))
                && wallpapers::expand(&self.want.slideshow).as_ref() != Some(f)
            {
                self.want.slideshow = f.to_string_lossy().into_owned();
            }
            return;
        }
        let picked = match row.map(backgroundpage::row) {
            Some(backgroundpage::Row::Builtin(b)) => {
                self.want.wallpaper = b.uri();
                return;
            }
            Some(backgroundpage::Row::Picture(i)) => self.wallpapers.get(i),
            _ => None,
        };
        if picked.is_some() && picked.cloned() == wallpapers::expand(&self.want.wallpaper) {
            return;
        }
        self.want.wallpaper = picked
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
    }

    /// Browse... picked `path`: it joins the list if absent and becomes pending, keeping the config's spelling.
    fn browsed(&mut self, path: PathBuf) {
        // The dialog has just listed `path`, and the rows it is held to
        // are local folders the scan read, so the disk answers at once.
        wallpapers::place(&mut self.wallpapers, &path);
        if wallpapers::expand(&self.want.wallpaper).as_ref() != Some(&path) {
            self.want.wallpaper = path.to_string_lossy().into_owned();
        }
        self.fill_wallpapers();
        self.want_preview();
        pages::follow(&mut self.sheet, &self.want);
        self.show_monitor();
    }

    /// Browse... picked the slideshow folder `path`.
    fn slide_folder(&mut self, path: PathBuf) {
        if wallpapers::expand(&self.want.slideshow).as_ref() != Some(&path) {
            self.want.slideshow = path.to_string_lossy().into_owned();
        }
        self.fill_wallpapers();
        self.want_preview();
        pages::follow(&mut self.sheet, &self.want);
        self.show_monitor();
    }

    /// Run `job` on a thread; `Event::Wake` fires when its result is in or the thread ended without one.
    fn spawn<T: Send + 'static>(&self, job: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
        /// Wakes the loop on drop, so a panicking job wakes it too.
        struct WakeOnDrop(Option<Waker>);
        impl Drop for WakeOnDrop {
            fn drop(&mut self) {
                if let Some(w) = &self.0 {
                    w.wake();
                }
            }
        }
        let (tx, rx) = channel();
        let wake = WakeOnDrop(self.waker.clone());
        std::thread::spawn(move || {
            // Declared first, dropped last: the result is sent, or the
            // sender gone, before the loop looks.
            let _wake = wake;
            let tx = tx;
            let _ = tx.send(job());
        });
        rx
    }

    /// Decode the pending wallpaper for the monitor unless shown or a decode is under way; that one's end calls this again.
    fn want_preview(&mut self) {
        let want = backgroundpage::monitor_file(&self.want, &self.wallpapers);
        let Some(file) = backgroundpage::next_decode(
            &want,
            self.picture.as_ref().map(|p| p.path.as_str()),
            self.decoding.is_some(),
        ) else {
            return;
        };
        // A built-in is drawn at the screen's shape, as large as a picture is decoded, so fine lines shrink smoothly.
        let rx = match backgroundpage::builtin(&want) {
            Some(b) => self.spawn(move || {
                let (w, h) = backgroundpage::DECODE_MAX;
                let canvas = backgroundpage::draw_builtin(b, &Default::default(), w, h);
                Some(Decoded {
                    canvas,
                    width: w,
                    height: h,
                })
            }),
            None => self.spawn(move || {
                Decoders::default_set().decode_file(&file, backgroundpage::DECODE_MAX)
            }),
        };
        self.decoding = Some((want, rx));
    }

    /// A thread is done: fill the list from the scan or put the decode on the monitor; a later start opens its page unless a dialog is up.
    fn wake(&mut self) -> Reply {
        let mut changed = false;
        let mut activate = None;
        while let Some(req) = self.requests.as_ref().and_then(instance::Requests::next) {
            changed |= self.request(&req);
            activate = req.token.or(activate);
        }
        if let Some(rx) = &self.scan {
            let found = match take(rx) {
                Ok(None) => None,
                Ok(Some(found)) => Some(found),
                Err(()) => {
                    log::warn!("the picture scan ended without a result");
                    Some(Vec::new())
                }
            };
            if let Some(found) = found {
                self.scan = None;
                self.wallpapers = wallpapers::add_missing(found, &self.wallpapers);
                self.fill_wallpapers();
                changed = true;
            }
        }
        if let Some(rx) = &self.thumbing {
            let drawn = match take(rx) {
                Ok(None) => None,
                Ok(Some(t)) => Some(t),
                Err(()) => Some(Vec::new()),
            };
            if let Some(t) = drawn {
                self.thumbing = None;
                self.thumbs = t;
                if !backgroundpage::slideshow(&self.want) {
                    backgroundpage::set_thumbs(
                        &mut self.sheet.pages[Page::Background.index()],
                        &self.thumbs,
                    );
                }
                changed = true;
            }
        }
        let done = match &self.decoding {
            Some((path, rx)) => match take(rx) {
                Ok(None) => None,
                Ok(Some(d)) => Some((path.clone(), d)),
                Err(()) => Some((path.clone(), None)),
            },
            None => None,
        };
        if let Some((path, d)) = done {
            self.decoding = None;
            self.picture = Some(match d {
                Some(d) => backgroundpage::Picture {
                    path,
                    canvas: Some(d.canvas),
                    source_w: d.width,
                },
                None => {
                    log::warn!("{path}: not a picture this can decode");
                    backgroundpage::Picture {
                        path,
                        canvas: None,
                        source_w: 0,
                    }
                }
            });
            self.want_preview();
            self.show_monitor();
            changed = true;
        }
        let mut reply = if changed {
            Reply::redraw()
        } else {
            Reply::none()
        };
        reply.activate = activate;
        reply
    }

    fn request(&mut self, req: &instance::Request) -> bool {
        match &req.action {
            Some((action, params)) if action == PAGE_ACTION && self.children().is_empty() => {
                match params.as_slice() {
                    [name] => Page::named(name).is_some_and(|p| self.open_page(p)),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn open_page(&mut self, page: Page) -> bool {
        if self.sheet.tabs.open == page.index() {
            return false;
        }
        self.blur_side_lists();
        self.sheet.tabs.open = page.index();
        self.sheet.reveal_open();
        self.tab_opened();
        true
    }

    fn edit_file(&self) -> Reply {
        let apps = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
        let globs = reprisefs::apps::mime_globs();
        Reply {
            redraw: true,
            spawn: vec![reprisefs::props::open_argv_in(
                &self.path,
                &globs,
                &apps,
                &self.want.terminal,
            )],
            ..Default::default()
        }
    }

    /// Remove drops a user entry's file at once; a system entry only unchecks.
    fn remove_selected(&mut self) -> Reply {
        let Some(e) = self
            .list
            .selection
            .focus
            .and_then(|i| self.entries.get(i))
            .cloned()
        else {
            return Reply::none();
        };
        match autostart::remove(&self.user_autostart(), &e) {
            Ok(()) => self.rescan(),
            Err(err) => log::warn!("remove {}: {err}", e.id),
        }
        self.sync_apply();
        Reply::redraw()
    }

    fn reload_locations(&mut self) {
        let answers = repriseconf::location::load_from(&self.location_file);
        let entries = if answers.is_empty() {
            Vec::new()
        } else {
            reprisefs::apps::scan(&reprisefs::apps::default_dirs())
        };
        let rows: Vec<(String, bool)> = answers
            .iter()
            .map(|(id, &allow)| {
                let name = reprisefs::apps::entry_for_app_id(&entries, id, |_| true)
                    .map_or_else(|| id.clone(), |e| e.name.clone());
                (name, allow)
            })
            .collect();
        self.location_ids = answers.into_keys().collect();
        pages::fill_locations(&mut self.locations, &rows);
        self.sync_forget();
    }

    fn sync_forget(&mut self) {
        let on = self.locations.selection.focus.is_some();
        self.sheet.pages[Page::Advanced.index()].set_enabled(pages::FORGET, on);
    }

    /// Forget drops the picked answer from the state file at once; it waits for no Apply.
    fn forget_location(&mut self) -> Reply {
        let Some(id) = (self.locations.selection.focus).and_then(|i| self.location_ids.get(i))
        else {
            return Reply::none();
        };
        if let Err(e) = repriseconf::location::forget_in(&self.location_file, id) {
            log::warn!("forget the location answer for {id}: {e}");
            self.fail("The location answer could not be forgotten.");
        }
        self.reload_locations();
        self.sync_apply();
        Reply::redraw()
    }

    /// Give the Startup list the keyboard or take it away; the dotted rectangle goes with it.
    fn focus_list(&mut self, on: bool) {
        self.list_focus = on;
        self.list.focus_ring = on;
        if on {
            self.sheet.open_mut().set_focus(None);
            self.sheet.tabs.focused = false;
        }
    }

    fn focus_shortcuts(&mut self, on: bool) {
        self.shortcuts_focus = on;
        self.shortcuts.focus_ring = on;
        if on {
            self.sheet.open_mut().set_focus(None);
            self.sheet.tabs.focused = false;
        }
    }

    fn focus_locations(&mut self, on: bool) {
        self.locations_focus = on;
        self.locations.focus_ring = on;
        if on {
            self.sheet.open_mut().set_focus(None);
            self.sheet.tabs.focused = false;
        }
    }

    fn on_page(&self, r: Rect) -> Rect {
        pages::on_sheet(&self.sheet, r)
    }

    fn side_list(&mut self) -> Option<(&mut ListView, Rect, &mut bool)> {
        let a = self.area();
        let (list, shortcuts) = (
            self.on_page(pages::list(a)),
            self.on_page(pages::shortcuts(a)),
        );
        let types = self.on_page(defaultspage::list(a));
        let locations = self.on_page(pages::locations(a));
        match registry::open(&self.sheet) {
            Page::Startup => Some((&mut self.list, list, &mut self.list_focus)),
            Page::Keyboard => Some((&mut self.shortcuts, shortcuts, &mut self.shortcuts_focus)),
            Page::DefaultPrograms => Some((&mut self.types, types, &mut self.types_focus)),
            Page::Advanced => Some((&mut self.locations, locations, &mut self.locations_focus)),
            _ => None,
        }
    }

    fn focus_side_list(&mut self, on: bool) {
        match registry::open(&self.sheet) {
            Page::Startup => self.focus_list(on),
            Page::Keyboard => self.focus_shortcuts(on),
            Page::DefaultPrograms => self.focus_types(on),
            Page::Advanced => self.focus_locations(on),
            _ => {}
        }
    }

    fn blur_side_lists(&mut self) {
        self.focus_list(false);
        self.focus_shortcuts(false);
        self.focus_types(false);
        self.focus_locations(false);
    }

    fn fill_shortcuts(&mut self) {
        let names = &self.app_names;
        pages::fill_shortcuts(&mut self.shortcuts, &self.want, &|app| {
            names.get(app).cloned().unwrap_or_else(|| app.to_string())
        });
        let vision = &mut self.sheet.pages[Page::Vision.index()];
        crate::visionpage::show_zoom_keys(vision, &self.want.shortcuts);
    }

    /// Every key taken but `row`'s; a program's is under its list text so the conflict note names it.
    fn taken_keys(&self, row: Option<model::Row>) -> Vec<model::Shortcut> {
        let binds = (self.want.shortcuts.iter().enumerate())
            .filter(|(i, _)| row != Some(model::Row::Bind(*i)))
            .map(|(_, o)| o.clone());
        let name = |app: &str| {
            self.app_names
                .get(app)
                .cloned()
                .unwrap_or_else(|| app.to_string())
        };
        let programs = (self.want.programs.iter().enumerate())
            .filter(|(i, p)| row != Some(model::Row::Program(*i)) && !p.keys.is_empty())
            .map(|(_, p)| {
                model::Shortcut::new(&p.keys, &model::program_text(p, &name(&p.app)), "")
            });
        binds.chain(programs).collect()
    }

    fn open_shortcut(&mut self, row: Option<usize>) {
        let target = row.map(model::Row::Bind);
        let sc = row.and_then(|i| self.want.shortcuts.get(i));
        let others = self.taken_keys(target);
        self.shortcut = Some(shortcutdlg::ShortcutDialog::new(sc, others));
        self.shortcut_target = target;
    }

    fn edit_shortcut(&mut self) -> Reply {
        let Some(i) = self.shortcuts.selection.focus else {
            return Reply::none();
        };
        match model::row(&self.want, i) {
            Some(model::Row::Bind(j)) => self.open_shortcut(Some(j)),
            Some(r @ model::Row::Program(j)) => {
                let others = self.taken_keys(Some(r));
                let keys = &self.want.programs[j].keys;
                self.shortcut = Some(shortcutdlg::ShortcutDialog::for_program(keys, others));
                self.shortcut_target = Some(r);
            }
            Some(model::Row::Denied(_)) | None => return Reply::none(),
        }
        Reply::redraw_all()
    }

    /// OK in the Shortcut dialog: `sc` replaces the row it was opened on or is added after the binds. Returns its list row.
    fn accept_shortcut(&mut self, sc: model::Shortcut) -> usize {
        let row = match self.shortcut_target.take() {
            Some(model::Row::Program(j)) if j < self.want.programs.len() => {
                self.want.programs[j].keys = sc.keys;
                self.want.shortcuts.len() + j
            }
            Some(model::Row::Bind(i)) if i < self.want.shortcuts.len() => {
                self.want.shortcuts[i] = sc;
                i
            }
            _ => {
                self.want.shortcuts.push(sc);
                self.want.shortcuts.len() - 1
            }
        };
        self.shortcut = None;
        self.fill_shortcuts();
        row
    }

    fn remove_shortcut(&mut self) -> Reply {
        let Some(r) = (self.shortcuts.selection.focus).and_then(|i| model::row(&self.want, i))
        else {
            return Reply::none();
        };
        model::forget(&mut self.want, r);
        self.fill_shortcuts();
        self.sync_apply();
        Reply::redraw()
    }

    fn shortcut_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.shortcut.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => shortcutdlg::Outcome::Cancel,
                Event::Blink => {
                    d.form.caret = ctx.caret_on;
                    return Reply::redraw();
                }
                ev => match form_input(&mut d.form, &ev, ctx) {
                    Some(ev) => d.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        match out {
            shortcutdlg::Outcome::None => Reply::none(),
            shortcutdlg::Outcome::Redraw => Reply::redraw(),
            shortcutdlg::Outcome::Cancel => {
                self.shortcut = None;
                self.shortcut_target = None;
                Reply::redraw_all()
            }
            shortcutdlg::Outcome::Accept(sc) => {
                let row = self.accept_shortcut(sc);
                let lay = listview::layout(
                    &self.shortcuts,
                    self.on_page(pages::shortcuts(self.area())),
                    &ctx.theme.look.controls(),
                );
                self.shortcuts.select(Some(row), &lay);
                self.sync_apply();
                Reply::redraw_all()
            }
        }
    }

    fn ensure_pads(&mut self) -> bool {
        if registry::open(&self.sheet) != Page::Tablet {
            self.pads_fresh = false;
            return false;
        }
        if self.pads_fresh {
            return false;
        }
        self.pads_fresh = true;
        self.pads = reprisebus::tray::tablet_pads().unwrap_or_else(|e| {
            log::warn!("tablet pads: {e}");
            Vec::new()
        });
        if self.pad_picked >= self.pads.len() {
            self.pad_picked = 0;
        }
        self.refill_tablet();
        true
    }

    fn refill_tablet(&mut self) {
        let page = tabletpage::page(
            self.area(),
            &self.pads,
            self.pad_picked,
            &self.want.tablet_pads,
        );
        self.sheet.set_page(Page::Tablet.index(), page);
    }

    fn tablet_changed(&mut self, id: &str) -> Reply {
        let page = &self.sheet.pages[Page::Tablet.index()];
        let row = page.combo_selected(id).unwrap_or(0);
        if id == tabletpage::PAD {
            self.pad_picked = row.min(self.pads.len().saturating_sub(1));
            self.refill_tablet();
            self.sheet.pages[Page::Tablet.index()].focus_id(tabletpage::PAD);
            return Reply::redraw();
        }
        let Some(pad) = self.pads.get(self.pad_picked) else {
            return Reply::none();
        };
        let Some(i) = tabletpage::ROW_IDS.iter().position(|r| *r == id) else {
            return Reply::none();
        };
        let Some(&slot) = tabletpage::slots(pad).get(i) else {
            return Reply::none();
        };
        let list = &mut self.want.tablet_pads;
        let had = tabletpage::entry_for(list, &pad.name).map_or("", |e| list[e].get(slot));
        let value = tabletpage::value_at(row, had);
        tabletpage::bind(list, &pad.name, slot, &value);
        self.sync_apply();
        Reply::redraw()
    }

    fn ensure_defaults(&mut self) {
        if self.defaults.is_some() || registry::open(&self.sheet) != Page::DefaultPrograms {
            return;
        }
        self.defaults = Some(defaultspage::Defaults::load());
        self.refill_defaults();
    }

    fn refill_defaults(&mut self) {
        let Some(d) = &self.defaults else {
            return;
        };
        let keep = defaultspage::picked(&self.types, d).map(|t| t.mime.clone());
        defaultspage::fill_list(&mut self.types, d, keep.as_deref());
        defaultspage::fill(
            &mut self.sheet.pages[Page::DefaultPrograms.index()],
            d,
            &self.want.terminal,
            self.types.selection.focus,
        );
    }

    fn focus_types(&mut self, on: bool) {
        self.types_focus = on;
        self.types.focus_ring = on;
        if on {
            self.sheet.open_mut().set_focus(None);
            self.sheet.tabs.focused = false;
        }
    }

    fn defaults_changed(&mut self, id: &str) -> Reply {
        if id == defaultspage::DRIVE_INSERT {
            pages::read_one(&self.sheet, id, &mut self.want);
            self.sync_apply();
            return Reply::redraw();
        }
        let page = &self.sheet.pages[Page::DefaultPrograms.index()];
        let Some(d) = self.defaults.as_mut() else {
            return Reply::redraw();
        };
        if id == defaultspage::FILTER {
            if let Some(form::Control::Edit { edit, .. }) = page.item(id).map(|i| &i.control) {
                d.set_filter(&edit.text);
            }
            let keep = defaultspage::picked(&self.types, d).map(|t| t.mime.clone());
            defaultspage::fill_list(&mut self.types, d, keep.as_deref());
            self.types.top = 0;
            self.sync_apply();
            return Reply::redraw();
        }
        let row = page.combo_selected(id).unwrap_or(0);
        if id == defaultspage::TERMINAL {
            if let Some(t) = d.terminal_pick(row) {
                self.want.terminal = t.to_string();
            }
        } else if let Some(k) = defaultspage::KINDS.iter().find(|k| k.id == id) {
            d.pick(k, row);
        }
        self.refill_defaults();
        self.sync_apply();
        Reply::redraw()
    }

    fn restore_default(&mut self) -> Reply {
        let Some(d) = self.defaults.as_mut() else {
            return Reply::none();
        };
        let Some(mime) = defaultspage::picked(&self.types, d).map(|t| t.mime.clone()) else {
            return Reply::none();
        };
        d.set(&mime, None);
        self.refill_defaults();
        self.sync_apply();
        Reply::redraw()
    }

    /// Change...: the Open With chooser over the picked type, "Always use" ticked and fixed, Browse... greyed.
    fn open_chooser(&mut self) -> Reply {
        let Some(d) = &self.defaults else {
            return Reply::none();
        };
        let Some(t) = defaultspage::picked(&self.types, d) else {
            return Reply::none();
        };
        let apps = &d.apps;
        let default = d.current(&t.mime);
        let (recommended, others) = reprisefs::apps::chooser_groups(apps, Some(&t.mime), default);
        let program = |i: usize| Program {
            id: apps[i].id.clone(),
            name: apps[i].name.clone(),
            icon: apps[i].icon.clone(),
        };
        let name = match &t.ext {
            Some(e) => format!("{} ({e})", t.description),
            None => t.description.clone(),
        };
        let mut ow = OpenWith::new(
            &name,
            recommended.into_iter().map(program).collect(),
            others.into_iter().map(program).collect(),
        );
        if let Some(i) = default {
            ow.select_id(&apps[i].id);
        }
        ow.always = openwith::Check {
            on: true,
            enabled: false,
        };
        ow.browse_enabled = false;
        self.chooser = Some(Chooser {
            mime: t.mime.clone(),
            ow,
            input: ChooserInput::opened(false),
            icon: None,
            row_icons: Vec::new(),
            icons_due: true,
        });
        Reply::redraw_all()
    }

    fn chooser_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let Some(c) = self.chooser.as_mut() else {
            return Reply::none();
        };
        let out = match ev {
            Event::CloseRequest => openwith::Event::Cancel,
            ev => {
                let cm = ctx.theme.look.controls();
                let lay = c.ow.layout(&self.dialog_text, self.dialog_px, (0, 0), &cm);
                c.input.event(&mut c.ow, &lay, &ev)
            }
        };
        match out {
            openwith::Event::None | openwith::Event::Browse => Reply::none(),
            openwith::Event::Redraw => Reply::redraw(),
            openwith::Event::Cancel => {
                self.chooser = None;
                Reply::redraw_all()
            }
            openwith::Event::Choose { id, .. } => {
                let mime = c.mime.clone();
                self.chooser = None;
                if let Some(d) = self.defaults.as_mut() {
                    let now = d.current(&mime).map(|i| d.apps[i].id.as_str());
                    if now != Some(id.as_str()) {
                        d.set(&mime, Some(&id));
                    }
                }
                self.refill_defaults();
                self.sync_apply();
                Reply::redraw_all()
            }
        }
    }

    fn menu_button(&mut self, id: &str) -> Reply {
        let (page, menu, cm) = (
            &mut self.sheet.pages[Page::StartMenu.index()],
            &mut self.want.startmenu,
            &self.controls,
        );
        let done = match id {
            menupage::FOLDER => menupage::new_folder(page, menu, cm),
            menupage::REMOVE => menupage::remove(page, menu, cm),
            menupage::UP => menupage::shift(page, menu, -1, cm),
            menupage::DOWN => menupage::shift(page, menu, 1, cm),
            _ => false,
        };
        self.menu_changed(done)
    }

    fn menu_changed(&mut self, done: bool) -> Reply {
        if !done {
            return Reply::redraw();
        }
        self.menu_icons_due = true;
        self.sync_apply();
        Reply::redraw_all()
    }

    fn add_item_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.add_item.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => additem::Outcome::Cancel,
                Event::Blink => {
                    d.form.caret = ctx.caret_on;
                    return Reply::redraw();
                }
                ev => match form_input(&mut d.form, &ev, ctx) {
                    Some(ev) => d.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        match out {
            additem::Outcome::None => Reply::none(),
            additem::Outcome::Redraw => Reply::redraw(),
            additem::Outcome::Cancel => {
                self.add_item = None;
                Reply::redraw_all()
            }
            additem::Outcome::Accept(v) if self.add_to_quick => {
                self.add_item = None;
                self.quick_add(&v)
            }
            additem::Outcome::Accept(v) => {
                self.add_item = None;
                let icon = v.icon.as_deref().unwrap_or("");
                let e = startmenu::Entry::program(&v.name, icon, v.command);
                let done = menupage::insert(
                    &mut self.sheet.pages[Page::StartMenu.index()],
                    &mut self.want.startmenu,
                    e,
                    &self.controls,
                )
                .is_some();
                self.menu_changed(done)
            }
        }
    }

    fn saver_dlg_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.saver_dlg.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => saverdlg::Outcome::Cancel,
                Event::Blink => {
                    d.form.caret = ctx.caret_on;
                    return Reply::redraw();
                }
                ev => match form_input(&mut d.form, &ev, ctx) {
                    Some(ev) => d.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        match out {
            saverdlg::Outcome::None => Reply::none(),
            saverdlg::Outcome::Redraw => Reply::redraw(),
            saverdlg::Outcome::Cancel => {
                self.saver_dlg = None;
                Reply::redraw_all()
            }
            saverdlg::Outcome::Accept => {
                if let Some(d) = self.saver_dlg.take() {
                    d.apply(&mut self.want);
                }
                self.sync_apply();
                Reply::redraw_all()
            }
        }
    }

    /// Step the Screen Saver page's monitor saver and put the frame up. Whether it wants the next tick.
    fn saver_tick(&mut self, ctx: &Ctx) -> bool {
        if self.sheet.tabs.open != Page::ScreenSaver.index() {
            self.saver_preview = None;
            return false;
        }
        let kind = match saverpage::row(&self.want) {
            0 => reprisekit::saver::Kind::Blank,
            _ => saverpage::kind(&self.want),
        };
        let options = saverpage::options(&self.want);
        let stale = self
            .saver_preview
            .as_ref()
            .is_none_or(|(p, _)| p.kind != kind || p.options != options);
        if stale {
            let p = saverpage::Preview::new(kind, options, &ctx.text);
            self.saver_preview = Some((p, std::time::Instant::now()));
        }
        self.want_saver_photos();
        let Some((p, last)) = self.saver_preview.as_mut() else {
            return false;
        };
        if let Some((folder, rx)) = &self.saver_photos
            && folder == &self.want.saver_folder
            && let Ok(Some(pics)) = take(rx)
        {
            p.photos = pics;
            self.saver_photos = None;
        }
        let ms = last.elapsed().as_millis() as u64;
        *last = std::time::Instant::now();
        let frame = p.frame(ms);
        saverpage::show(&mut self.sheet.pages[Page::ScreenSaver.index()], frame);
        true
    }

    fn want_saver_photos(&mut self) {
        let Some((p, _)) = &self.saver_preview else {
            return;
        };
        let folder = self.want.saver_folder.clone();
        let have = self.saver_photos.as_ref().map(|(f, _)| f == &folder);
        if p.kind != reprisekit::saver::Kind::Photos || !p.photos.is_empty() || have == Some(true) {
            return;
        }
        let pictures = self.wallpapers.clone();
        let dir = wallpapers::expand(&folder)
            .or_else(|| reprisefs::places::user_dir("PICTURES", "Pictures"));
        let rx = self.spawn(move || {
            let Some(dir) = dir else {
                return Vec::new();
            };
            let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
                .map(|rd| rd.flatten().map(|e| e.path()).collect())
                .unwrap_or_default();
            files.retain(|p| wallpapers::is_picture(p));
            if files.is_empty() {
                files = pictures
                    .into_iter()
                    .filter(|p| p.starts_with(&dir))
                    .collect();
            }
            files.sort();
            files
                .iter()
                .take(3)
                .filter_map(|f| Decoders::default_set().decode_file(f, (304, 224)))
                .map(|d| d.canvas)
                .collect()
        });
        self.saver_photos = Some((folder, rx));
    }

    fn rename_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.rename.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => renamedlg::Outcome::Cancel,
                Event::Blink => {
                    d.form.caret = ctx.caret_on;
                    return Reply::redraw();
                }
                ev => match form_input(&mut d.form, &ev, ctx) {
                    Some(ev) => d.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        match out {
            renamedlg::Outcome::None => Reply::none(),
            renamedlg::Outcome::Redraw => Reply::redraw(),
            renamedlg::Outcome::Cancel => {
                self.rename = None;
                Reply::redraw_all()
            }
            renamedlg::Outcome::Accept(label) => {
                self.rename = None;
                let done = menupage::rename(
                    &mut self.sheet.pages[Page::StartMenu.index()],
                    &mut self.want.startmenu,
                    &label,
                );
                self.menu_changed(done)
            }
        }
    }

    /// Look up icons a change left missing; runs after every event.
    fn prime_icons(&mut self, ctx: &mut Ctx) {
        let mut get = |name: &str| ctx.icons.get(name, 16);
        if self.menu_icons_due {
            menupage::set_icons(
                &mut self.sheet.pages[Page::StartMenu.index()],
                &self.want.startmenu,
                &mut get,
            );
            self.menu_icons_due = false;
        }
        if self.quick_icons_due {
            quickpage::set_icons(
                &mut self.sheet.pages[Page::StartMenu.index()],
                &self.quick,
                &mut get,
            );
            self.quick_icons_due = false;
        }
        if let Some(d) = self.add_item.as_mut().filter(|d| d.icons_due) {
            d.set_icons(&mut get);
        }
        if let Some(c) = self.chooser.as_mut().filter(|c| c.icons_due) {
            c.row_icons =
                c.ow.list
                    .rows
                    .iter()
                    .map(|r| openwith::icon_name(r).and_then(&mut get))
                    .collect();
            c.icon = ctx.icons.get(openwith::ICON, 32);
            c.icons_due = false;
        }
    }

    fn quick_reload(&mut self, file: Option<&str>) {
        self.quick = self
            .quick_dir
            .as_deref()
            .map(quickpage::load)
            .unwrap_or_default();
        let pick = file.and_then(|f| self.quick.iter().position(|l| l.file == f));
        quickpage::fill(
            &mut self.sheet.pages[Page::StartMenu.index()],
            &self.quick,
            pick,
        );
        self.quick_icons_due = true;
    }

    fn quick_add(&mut self, v: &additem::NewItem) -> Reply {
        let Some(dir) = self.quick_dir.clone() else {
            return Reply::redraw_all();
        };
        let written = quickpage::add(&dir, v).and_then(|file| {
            self.quick.push(quickpage::Launcher {
                file: file.clone(),
                name: v.name.clone(),
                icon: None,
            });
            quickpage::store(&dir, &self.quick).map(|()| file)
        });
        let file = match written {
            Ok(f) => Some(f),
            Err(e) => {
                log::warn!("quick launch add {:?}: {e}", v.name);
                self.fail("The Quick Launch button could not be written.");
                None
            }
        };
        self.quick_reload(file.as_deref());
        self.sync_apply();
        Reply::redraw_all()
    }

    fn quick_button(&mut self, id: &str) -> Reply {
        let page = &self.sheet.pages[Page::StartMenu.index()];
        let (Some(dir), Some(i)) = (self.quick_dir.clone(), quickpage::selected(page)) else {
            return Reply::none();
        };
        let Some(file) = self.quick.get(i).map(|l| l.file.clone()) else {
            return Reply::none();
        };
        let mut pick = Some(file.clone());
        let done = match id {
            quickpage::REMOVE => {
                self.quick.remove(i);
                pick = self
                    .quick
                    .get(i)
                    .or(self.quick.last())
                    .map(|l| l.file.clone());
                quickpage::remove(&dir, &file).and_then(|()| quickpage::store(&dir, &self.quick))
            }
            quickpage::UP if i > 0 => {
                self.quick.swap(i, i - 1);
                quickpage::store(&dir, &self.quick)
            }
            quickpage::DOWN if i + 1 < self.quick.len() => {
                self.quick.swap(i, i + 1);
                quickpage::store(&dir, &self.quick)
            }
            _ => return Reply::none(),
        };
        if let Err(e) = done {
            log::warn!("quick launch {id} {file}: {e}");
            self.fail("The Quick Launch folder could not be written.");
        }
        self.quick_reload(pick.as_deref());
        self.sync_apply();
        Reply::redraw()
    }

    fn layouts_changed(&mut self, select: Option<usize>) -> Reply {
        pages::fill_layouts(&mut self.sheet, &self.want, &self.xkb, select);
        self.sync_apply();
        Reply::redraw()
    }

    fn remove_layout(&mut self) -> Reply {
        let Some(i) = pages::layout_selection(&self.sheet) else {
            return Reply::none();
        };
        self.want.remove_layout(i);
        let left = self.want.layouts().len();
        self.layouts_changed((left > 0).then(|| i.min(left - 1)))
    }

    fn move_layout_up(&mut self) -> Reply {
        match pages::layout_selection(&self.sheet) {
            Some(i) if self.want.move_layout_up(i) => self.layouts_changed(Some(i - 1)),
            _ => Reply::none(),
        }
    }

    fn add_layout_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.add_layout.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => layoutdlg::Outcome::Cancel,
                Event::Blink => return Reply::none(),
                ev => match form_input(&mut d.form, &ev, ctx) {
                    Some(ev) => d.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        match out {
            layoutdlg::Outcome::None => Reply::none(),
            layoutdlg::Outcome::Redraw => Reply::redraw(),
            layoutdlg::Outcome::Cancel => {
                self.add_layout = None;
                Reply::redraw_all()
            }
            layoutdlg::Outcome::Accept(layout, variant) => {
                self.add_layout = None;
                if self.want.add_layout(&layout, &variant) {
                    let last = self.want.layouts().len() - 1;
                    self.layouts_changed(Some(last));
                }
                Reply::redraw_all()
            }
        }
    }

    fn sheet_event(&mut self, ev: propsheet::Event) -> Reply {
        match ev {
            propsheet::Event::None => Reply::none(),
            propsheet::Event::Redraw => Reply::redraw(),
            propsheet::Event::Page(p, form::Event::Changed(id)) => {
                if p == Page::Display.index()
                    && displaypage::read(&mut self.display.model, &self.sheet.pages[p], id)
                {
                    return self.display_changed();
                }
                if p == Page::DefaultPrograms.index() {
                    return self.defaults_changed(id);
                }
                if p == Page::Tablet.index() {
                    return self.tablet_changed(id);
                }
                // Only the control just worked is read back: reading all would write clamped values for keys the control cannot show.
                let from = self.want.interface.clone();
                pages::read_one(&self.sheet, id, &mut self.want);
                if self.want.interface != from {
                    self.interface_moved(&from);
                }
                if id == saverpage::SAVER {
                    let row = self.sheet.pages[Page::ScreenSaver.index()].combo_selected(id);
                    saverpage::pick(&mut self.want, row.unwrap_or(0));
                }
                if id == saverpage::WAIT
                    && let Some(form::Control::UpDown(u)) = self.sheet.pages
                        [Page::ScreenSaver.index()]
                    .item(id)
                    .map(|i| &i.control)
                {
                    saverpage::set_wait(&mut self.want, u.value() as i64);
                }
                if id == backgroundpage::MODE {
                    self.pick_mode();
                    self.want_preview();
                }
                if id == backgroundpage::WALLPAPER {
                    self.pick_wallpaper();
                    self.want_preview();
                }
                pages::follow(&mut self.sheet, &self.want);
                self.show_monitor();
                self.sync_apply();
                Reply::redraw()
            }
            propsheet::Event::Page(_, form::Event::Pressed(id)) => match id {
                pages::EDIT_FILE => self.edit_file(),
                displaypage::IDENTIFY => self.identify(),
                pages::TRAY_CUSTOMIZE => {
                    // Off the loop: the call waits on the compositor.
                    std::thread::spawn(|| {
                        if let Err(e) = reprisebus::tray::customize() {
                            log::warn!("tray Customize: {e}");
                        }
                    });
                    Reply::none()
                }
                pages::ADD => {
                    self.open_add(Pick::Autostart);
                    Reply::redraw_all()
                }
                saverpage::SETTINGS => {
                    let kind = saverpage::kind(&self.want);
                    self.saver_dlg = Some(saverdlg::SaverDialog::new(kind, &self.want));
                    Reply::redraw_all()
                }
                saverpage::PREVIEW => {
                    let name = saverpage::kind(&self.want).name().to_string();
                    // Off the loop: the call waits on the compositor.
                    std::thread::spawn(move || {
                        if let Err(e) = reprisebus::tray::preview_screen_saver(&name) {
                            log::warn!("screen saver Preview: {e}");
                        }
                    });
                    Reply::none()
                }
                backgroundpage::BROWSE => {
                    self.open_add(if backgroundpage::slideshow(&self.want) {
                        Pick::SlideFolder
                    } else {
                        Pick::Wallpaper
                    });
                    Reply::redraw_all()
                }
                pages::REMOVE => self.remove_selected(),
                defaultspage::CHANGE => self.open_chooser(),
                defaultspage::RESTORE => self.restore_default(),
                pages::FORGET => self.forget_location(),
                pages::SC_ADD => {
                    self.open_shortcut(None);
                    Reply::redraw_all()
                }
                pages::SC_EDIT => self.edit_shortcut(),
                pages::SC_REMOVE => self.remove_shortcut(),
                pages::KB_ADD => {
                    self.add_layout = Some(layoutdlg::LayoutDialog::new(&self.xkb));
                    Reply::redraw_all()
                }
                pages::KB_REMOVE => self.remove_layout(),
                pages::KB_UP => self.move_layout_up(),
                menupage::ADD | quickpage::ADD => {
                    let apps = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
                    self.add_item = Some(additem::AddItemDialog::new(apps, &self.want.terminal));
                    self.add_to_quick = id == quickpage::ADD;
                    Reply::redraw_all()
                }
                quickpage::REMOVE | quickpage::UP | quickpage::DOWN => self.quick_button(id),
                menupage::RENAME => {
                    let page = &self.sheet.pages[Page::StartMenu.index()];
                    match menupage::selected_label(page, &self.want.startmenu) {
                        Some(l) => {
                            self.rename = Some(renamedlg::RenameDialog::new(&l));
                            Reply::redraw_all()
                        }
                        None => Reply::none(),
                    }
                }
                menupage::FOLDER | menupage::REMOVE | menupage::UP | menupage::DOWN => {
                    self.menu_button(id)
                }
                _ => Reply::redraw(),
            },
            propsheet::Event::Page(..) => Reply::redraw(),
            propsheet::Event::Apply => {
                if self.apply() {
                    self.display.pending = Some(displayhost::Press::Apply);
                }
                Reply::redraw()
            }
            propsheet::Event::Ok => {
                // A failed write leaves the sheet open and dirty; otherwise the outputs' answer decides when it closes (`displayflow`).
                if self.apply() {
                    self.display.pending = Some(displayhost::Press::Ok);
                }
                Reply::redraw()
            }
            propsheet::Event::Cancel => {
                // Quits at once unless a request to the outputs is out,
                // whose answer is put back first.
                self.revert();
                self.display.pending = Some(displayhost::Press::Cancel);
                Reply::redraw()
            }
        }
    }

    /// The side list's keys while focused. Space toggles a Startup check.
    fn list_key(&mut self, key: &form::Key, cm: &ControlMetrics) -> Reply {
        let Some((list, rect, _)) = self.side_list() else {
            return Reply::none();
        };
        let lay = listview::layout(list, rect, cm);
        // Both lists are single-select: no Ctrl or Shift reaches them.
        let mods = reprisekit::selection::Mods::default();
        let moved = match key {
            form::Key::Up => list.step(-1, &lay, mods),
            form::Key::Down => list.step(1, &lay, mods),
            form::Key::Home => list.step(-(list.rows.len() as i64), &lay, mods),
            form::Key::End => list.step(list.rows.len() as i64, &lay, mods),
            form::Key::PageUp => list.step(-(lay.rows as i64), &lay, mods),
            form::Key::PageDown => list.step(lay.rows as i64, &lay, mods),
            form::Key::Space => {
                let Some(i) = list.selection.focus else {
                    return Reply::none();
                };
                list.toggle(i)
            }
            _ => false,
        };
        if !moved {
            return Reply::none();
        }
        self.sync_apply();
        Reply::redraw()
    }

    /// Tab on a page with a side list: focus runs page list → side list → the form → OK → Cancel → Apply → page list. Only one dotted rectangle is ever on the sheet.
    fn side_tab(&mut self, back: bool) -> Reply {
        let held =
            self.list_focus || self.shortcuts_focus || self.types_focus || self.locations_focus;
        if held {
            self.focus_side_list(false);
            let page = self.sheet.open_mut();
            if back {
                page.set_focus(None);
                self.sheet.tabs.focused = true;
            } else {
                page.focus_edge(false);
            }
            return Reply::redraw();
        }
        if self.sheet.tabs.focused && !back {
            self.focus_side_list(true);
            return Reply::redraw();
        }
        if !self.sheet.tabs.focused && back {
            if !self.sheet.open_mut().tab_no_wrap(true) {
                self.focus_side_list(true);
            }
            return Reply::redraw();
        }
        let ev = self.sheet.key(form::Key::Tab { back }, None);
        self.sheet_event(ev)
    }

    /// A page with a side list starts on that list (its first tab stop) unless the page list kept the focus.
    fn tab_opened(&mut self) {
        if !self.sheet.tabs.focused {
            self.focus_side_list(true);
        }
    }

    fn key(&mut self, k: KeyInput, cm: &ControlMetrics) -> Reply {
        let ctrl_tab = propsheet::page_step(k.sym, k.mods.ctrl, k.mods.shift);
        if let Some(dir) = form::shove_dir(k.sym, k.mods.ctrl, k.mods.shift, k.mods.alt)
            && let Some(ev) = self.sheet.shove(dir)
        {
            return self.sheet_event(ev);
        }
        let Some(key) = dialog_key(&k) else {
            return Reply::none();
        };
        if ctrl_tab.is_some() {
            self.blur_side_lists();
            let ev = self.sheet.key(key, ctrl_tab);
            self.tab_opened();
            return self.sheet_event(ev);
        }
        if let Some((_, _, focus)) = self.side_list() {
            let held = *focus;
            if let form::Key::Tab { back } = key {
                return self.side_tab(back);
            }
            if held {
                // Enter on a shortcut row edits it, on a file type
                // changes its program; on the Startup list it stays the
                // sheet's OK.
                if key == form::Key::Enter && self.sheet.tabs.open == Page::Keyboard.index() {
                    return self.edit_shortcut();
                }
                if key == form::Key::Enter && self.sheet.tabs.open == Page::DefaultPrograms.index()
                {
                    return self.open_chooser();
                }
                if let Some(key) = pages::list_key(&key) {
                    return self.list_key(&key, cm);
                }
            }
        }
        let ev = self.sheet.key(key, None);
        self.sheet_event(ev)
    }

    fn list_press(&mut self, x: i32, y: i32, cm: &ControlMetrics) -> Reply {
        // A click gives the list the keyboard and the ring; `select`
        // alone does not turn the ring on.
        self.focus_side_list(true);
        let Some((list, rect, _)) = self.side_list() else {
            return Reply::none();
        };
        let lay = listview::layout(list, rect, cm);
        let hit = list.hit(&lay, x, y);
        // A box pressed is held; it toggles on the release over it
        // (`list_release`). A header column held draws pressed.
        list.press_check(hit);
        list.header.press(hit.column(), false);
        match hit {
            listview::Hit::Check(i) => list.select(Some(i), &lay),
            listview::Hit::Row(i) => list.select(Some(i), &lay),
            listview::Hit::VBar(BarHit::Back) => {
                list.scroll(-1, &lay);
            }
            listview::Hit::VBar(BarHit::Forward) => {
                list.scroll(1, &lay);
            }
            _ => return Reply::redraw(),
        }
        self.sync_apply();
        Reply::redraw()
    }

    fn held_check_hit(
        &mut self,
        x: i32,
        y: i32,
        cm: &ControlMetrics,
    ) -> Option<(&mut ListView, listview::Hit)> {
        let (list, rect, _) = self.side_list()?;
        if !list.held_check.holding() {
            return None;
        }
        let lay = listview::layout(list, rect, cm);
        let hit = list.hit(&lay, x, y);
        Some((list, hit))
    }

    /// Light the side list's header column under the pointer; true when that changed what draws.
    fn list_hover(&mut self, x: i32, y: i32, ctx: &Ctx) -> bool {
        let cm = ctx.theme.look.controls();
        let styled = ctx.styled();
        let Some((list, rect, _)) = self.side_list() else {
            return false;
        };
        let column = if styled && rect.contains(x, y) {
            let lay = listview::layout(list, rect, &cm);
            list.hit(&lay, x, y).column()
        } else {
            None
        };
        list.header.motion(column) && styled
    }

    /// The pointer left or let go: the header column goes back to rest; true when that changed what draws.
    fn list_unhover(&mut self, release: bool, ctx: &Ctx) -> bool {
        let styled = ctx.styled();
        let Some((list, _, _)) = self.side_list() else {
            return false;
        };
        let changed = if release {
            let held = list.header.holding();
            list.header.release();
            held
        } else {
            list.header.leave()
        };
        changed && styled
    }

    fn list_motion(&mut self, x: i32, y: i32, cm: &ControlMetrics) -> Option<Reply> {
        let (list, hit) = self.held_check_hit(x, y, cm)?;
        Some(if list.motion_check(hit) {
            Reply::redraw()
        } else {
            Reply::none()
        })
    }

    fn list_release(&mut self, x: i32, y: i32, cm: &ControlMetrics) -> Option<Reply> {
        let (list, hit) = self.held_check_hit(x, y, cm)?;
        if list.release_check(hit).is_some() {
            self.sync_apply();
        }
        Some(Reply::redraw())
    }

    /// Open the file dialog for `pick`: Add... at home, Browse... at Pictures.
    fn open_add(&mut self, pick: Pick) {
        self.add_for = pick;
        let (dir, fields) = match pick {
            Pick::Autostart => (
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/")),
                Fields {
                    title: "Add".into(),
                    accept_label: "Open".into(),
                    mode: DlgMode::Open,
                    multiple: false,
                    filters: vec![
                        "Desktop Entries (*.desktop)".into(),
                        "All Files (*.*)".into(),
                    ],
                },
            ),
            Pick::Wallpaper => (
                wallpapers::start_dir(),
                Fields {
                    title: "Browse".into(),
                    accept_label: "Open".into(),
                    mode: DlgMode::Open,
                    multiple: false,
                    filters: vec![wallpapers::FILTER.into(), "All Files (*.*)".into()],
                },
            ),
            Pick::SlideFolder => (
                wallpapers::expand(&self.want.slideshow)
                    .filter(|p| p.is_dir())
                    .unwrap_or_else(wallpapers::start_dir),
                Fields {
                    title: "Browse for Folder".into(),
                    accept_label: "OK".into(),
                    mode: DlgMode::Folder,
                    multiple: false,
                    filters: Vec::new(),
                },
            ),
        };
        self.add = Some(self.add_host.open(&dir, fields));
    }

    fn close_add(&mut self) -> Reply {
        self.add = None;
        Reply::redraw_all()
    }

    fn add_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let pick = self.add_for;
        let Some(d) = &mut self.add else {
            return Reply::none();
        };
        let host = &mut self.add_host;
        let keep = type_keep(d);
        host.ensure_listed(d, &mut ctx.icons, &keep);
        match host.event(d, &ev, &ctx.text, ctx.theme.fonts.size_px, ctx.size) {
            Step::Closed | Step::Out(FileEvent::Cancel) => self.close_add(),
            Step::None => Reply::none(),
            Step::Redraw => Reply::redraw(),
            Step::Resize => Reply::resize(),
            Step::Out(FileEvent::NeedListing(id)) => {
                host.list(d, id, &mut ctx.icons, &keep);
                Reply::redraw()
            }
            Step::Out(FileEvent::FilterChanged(_)) => {
                let keep = type_keep(d);
                if let Some(id) = host.folder {
                    host.list(d, id, &mut ctx.icons, &keep);
                }
                Reply::redraw()
            }
            Step::Out(FileEvent::Accept(names)) => {
                let Some(name) = names.into_iter().next() else {
                    return Reply::none();
                };
                match host.resolve(&name) {
                    Ok(Resolved::Folder(_, path)) if pick == Pick::SlideFolder => {
                        self.slide_folder(path);
                        self.sync_apply();
                        self.close_add()
                    }
                    Ok(Resolved::Folder(id, _)) => {
                        host.list(d, id, &mut ctx.icons, &keep);
                        d.clear_file_name();
                        Reply::redraw()
                    }
                    Ok(Resolved::File(path)) => {
                        match pick {
                            Pick::Autostart => {
                                if let Err(e) = autostart::add(&self.user_autostart(), &path) {
                                    log::warn!("add {}: {e}", path.display());
                                    self.fail(&format!("Cannot add '{}': {e}.", path.display()));
                                }
                                self.rescan();
                            }
                            Pick::Wallpaper => match wallpapers::refusal(&path) {
                                Some(why) => self.fail(&why),
                                None => self.browsed(path),
                            },
                            Pick::SlideFolder => {}
                        }
                        self.sync_apply();
                        self.close_add()
                    }
                    Err(msg) => {
                        log::warn!("add {name}: {msg}");
                        self.fail(&msg);
                        self.close_add()
                    }
                }
            }
            Step::Out(_) => Reply::redraw(),
        }
    }
}

impl App for SettingsApp {
    fn title(&self) -> String {
        "Reprise Settings".into()
    }

    fn app_id(&self) -> String {
        "reprise.settings".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.window
    }

    /// Smallest sizes: the sheet's pages at their smallest, the file dialog's opening size.
    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        match &self.add {
            _ if win == repriseapp::MAIN => Some(Area::MIN.sheet_size()),
            Some(d) if win == ADD_WIN => Some(d.min_client()),
            _ => None,
        }
    }

    fn shortcuts_inhibited(&self) -> Option<WinId> {
        self.shortcut
            .as_ref()
            .filter(|d| d.capturing())
            .map(|_| SHORTCUT_WIN)
    }

    fn output_management(&self) -> bool {
        true
    }

    fn children(&self) -> Vec<Child> {
        let mut kids = Vec::new();
        if self.add.is_some() {
            kids.push(Child {
                id: ADD_WIN,
                title: match self.add_for {
                    Pick::Autostart => "Add",
                    Pick::Wallpaper => "Browse",
                    Pick::SlideFolder => "Browse for Folder",
                }
                .into(),
                size: filedlg::CLIENT,
                fixed: false,
                modal: true,
            });
        }
        if let Some(d) = &self.shortcut {
            kids.push(Child {
                id: SHORTCUT_WIN,
                title: d.title.into(),
                size: shortcutdlg::CLIENT,
                fixed: true,
                modal: true,
            });
        }
        if self.add_layout.is_some() {
            kids.push(Child {
                id: LAYOUT_WIN,
                title: "Add Layout".into(),
                size: layoutdlg::CLIENT,
                fixed: true,
                modal: true,
            });
        }
        if self.add_item.is_some() {
            kids.push(Child {
                id: ADD_ITEM_WIN,
                title: "Add Item".into(),
                size: additem::CLIENT,
                fixed: true,
                modal: true,
            });
        }
        if self.rename.is_some() {
            kids.push(Child {
                id: RENAME_WIN,
                title: "Rename".into(),
                size: renamedlg::CLIENT,
                fixed: true,
                modal: true,
            });
        }
        if let Some(d) = &self.saver_dlg {
            kids.push(Child {
                id: SAVER_WIN,
                title: d.title(),
                size: d.client,
                fixed: true,
                modal: true,
            });
        }
        if self.display.flow.question.is_some() {
            kids.push(Child {
                id: displayhost::MONITOR_WIN,
                title: "Monitor Settings".into(),
                size: monitordlg::CLIENT,
                fixed: true,
                modal: true,
            });
        }
        if self.chooser.is_some() {
            kids.push(Child {
                id: CHOOSER_WIN,
                title: openwith::TITLE.into(),
                size: openwith::CLIENT,
                fixed: true,
                modal: true,
            });
        }
        if let Some(b) = &self.error {
            kids.push(b.child(ERROR_WIN, &self.dialog_text, self.dialog_px));
        }
        kids
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.controls = ctx.theme.look.controls();
        let main = win == repriseapp::MAIN;
        let moved = main && self.fit_window(ctx.size);
        match ev {
            // A maximized window's size is the screen's, not one to open
            // at next time.
            Event::Resized if main && !ctx.maximized && ctx.size != self.window => {
                self.window = ctx.size;
                self.window_dirty = true;
            }
            Event::Focus(false) if main => self.save_window(),
            _ => {}
        }
        let mut reply = self.route(win, ev, ctx);
        reply.redraw |= moved;
        if let Some(press) = self.display.pending.take() {
            let r = self.start_display(press, &mut ctx.heads);
            reply.quit |= r.quit;
            reply.redraw_all |= r.redraw_all;
        }
        self.ensure_defaults();
        if win == repriseapp::MAIN
            && !self.saver_ticking
            && self.sheet.tabs.open == Page::ScreenSaver.index()
        {
            self.saver_ticking = true;
            reply.tick = true;
        }
        reply.redraw |= self.ensure_pads();
        self.prime_icons(ctx);
        if reply.quit {
            self.save_window();
        }
        reply
    }

    fn drop_form(&mut self, win: WinId) -> Option<DropForm<'_>> {
        let form = match win {
            repriseapp::MAIN => self.sheet.open_mut(),
            SHORTCUT_WIN => &mut self.shortcut.as_mut()?.form,
            ADD_ITEM_WIN => &mut self.add_item.as_mut()?.form,
            RENAME_WIN => &mut self.rename.as_mut()?.form,
            SAVER_WIN => &mut self.saver_dlg.as_mut()?.form,
            _ => return None,
        };
        Some(DropForm { form, px: None })
    }

    fn drop_file_dialog(&mut self, win: WinId) -> Option<FileDrop<'_>> {
        (win == ADD_WIN).then_some(FileDrop {
            dlg: self.add.as_mut()?,
            host: &self.add_host,
        })
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        if win == repriseapp::MAIN {
            self.fit_window(ctx.size);
        }
        self.paint(win, c, ctx);
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        self.describe(win, tree, ctx);
    }

    fn start(&mut self, waker: Waker) {
        if let Some(r) = &self.requests {
            let w = waker.clone();
            r.wake_with(move || w.wake());
            // For a request that came before the loop ran.
            waker.wake();
        }
        self.waker = Some(waker);
        let had = self.wallpapers.clone();
        self.scan = Some(
            self.spawn(move || wallpapers::merge(wallpapers::scan(&wallpapers::dirs()), &had)),
        );
        self.want_preview();
    }
}

impl SettingsApp {
    fn route(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        if matches!(ev, Event::Wake) {
            return self.wake();
        }
        self.fit_columns(&ctx.theme.look.controls());
        if self.err_icon.is_none() {
            self.err_icon = ctx.icons.get("dialog-error", 32);
        }
        if self.display.icon.is_none() {
            self.display.icon = ctx.icons.get("dialog-question", 32);
        }
        if self.wallpaper_icons.is_none() {
            let none = ctx
                .icons
                .get("process-stop", 16)
                .or_else(|| ctx.icons.get("dialog-cancel", 16));
            self.wallpaper_icons = Some((none, ctx.icons.get("image-x-generic", 16)));
            self.folder_icon = ctx.icons.get("folder", 16);
            self.fill_wallpapers();
            let (scale, glass) = (ctx.icons.scale(), appearancepage::glass(&self.want));
            self.thumbing = Some(self.spawn(move || {
                backgroundpage::builtins()
                    .into_iter()
                    .map(|b| Some(backgroundpage::thumb(b, &glass, scale)))
                    .collect()
            }));
        }
        self.add_host.icons.prime(&mut ctx.icons);
        if let Some(reason) = self.open_error.take() {
            self.fail(&reason);
        }
        if win == ERROR_WIN {
            return self.error_event(ev);
        }
        if win == ADD_WIN {
            return self.add_event(ev, ctx);
        }
        if win == SHORTCUT_WIN {
            return self.shortcut_event(ev, ctx);
        }
        if win == LAYOUT_WIN {
            return self.add_layout_event(ev, ctx);
        }
        if win == ADD_ITEM_WIN {
            return self.add_item_event(ev, ctx);
        }
        if win == RENAME_WIN {
            return self.rename_event(ev, ctx);
        }
        if win == SAVER_WIN {
            return self.saver_dlg_event(ev, ctx);
        }
        if win == displayhost::MONITOR_WIN {
            return self.monitor_event(ev, ctx);
        }
        if win == CHOOSER_WIN {
            return self.chooser_event(ev, ctx);
        }
        let px = ctx.theme.fonts.size_px;
        if self.sheet.tabs.open != Page::Startup.index() {
            self.focus_list(false);
        }
        if self.sheet.tabs.open != Page::Keyboard.index() {
            self.focus_shortcuts(false);
        }
        if self.sheet.tabs.open != Page::DefaultPrograms.index() {
            self.focus_types(false);
        }
        if self.sheet.tabs.open != Page::Advanced.index() {
            self.focus_locations(false);
        }
        match ev {
            Event::Heads(heads) => self.heads_changed(heads),
            Event::HeadsApplied { serial, ok } => self.heads_applied(serial, ok, &mut ctx.heads),
            Event::Tick => {
                let mut r = self.display_tick(&mut ctx.heads);
                self.saver_ticking = false;
                if self.saver_tick(ctx) {
                    r.tick = true;
                    r.redraw = true;
                }
                if self.sheet.ticking() {
                    r.redraw |=
                        self.sheet.tick(std::time::Instant::now()) == propsheet::Event::Redraw;
                    r.tick = true;
                }
                r
            }
            Event::CloseRequest => self.display_close(&mut ctx.heads),
            Event::Blink => {
                self.sheet.open_mut().caret = ctx.caret_on;
                Reply::redraw()
            }
            Event::Key(k) => self.key(k, &ctx.theme.look.controls()),
            Event::FieldDropped(e) => {
                let ev = self.sheet.dropped(e);
                self.sheet_event(ev)
            }
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => {
                // A combo's list dropped over the side list covers it.
                let dropped = self.sheet.open().popup_open();
                if !dropped
                    && self
                        .side_list()
                        .is_some_and(|(_, rect, _)| rect.contains(x, y))
                {
                    return self.list_press(x, y, &ctx.theme.look.controls());
                }
                let hit = self.sheet.hit(x, y, &ctx.text, px);
                let was = self.sheet.tabs.open;
                let ev = self.sheet.click(hit);
                if self.sheet.tabs.focused {
                    self.blur_side_lists();
                } else if self.sheet.tabs.open != was {
                    self.blur_side_lists();
                    self.tab_opened();
                }
                let r = self.sheet_event(ev);
                Reply {
                    tick: r.tick || self.sheet.ticking(),
                    ..r
                }
            }
            Event::Pointer(PointerInput::Motion { x, y }) => {
                let lit = self.list_hover(x, y, ctx);
                if let Some(r) = self.list_motion(x, y, &ctx.theme.look.controls()) {
                    return r;
                }
                let hit = self.sheet.hit(x, y, &ctx.text, px);
                let ev = self.sheet.motion(x, y, hit);
                let r = self.sheet_event(ev);
                Reply {
                    redraw: r.redraw || lit,
                    ..r
                }
            }
            Event::Pointer(PointerInput::Leave) => {
                let lit = self.list_unhover(false, ctx);
                let ev = self.sheet.leave();
                let r = self.sheet_event(ev);
                Reply {
                    redraw: r.redraw || lit,
                    ..r
                }
            }
            Event::Pointer(PointerInput::Scroll { x, y, rows }) => {
                let hit = self.sheet.hit(x, y, &ctx.text, px);
                let ev = self.sheet.wheel(hit, rows);
                if ev != propsheet::Event::None {
                    return self.sheet_event(ev);
                }
                let Some((list, rect, _)) = self.side_list() else {
                    return Reply::none();
                };
                if !rect.contains(x, y) {
                    return Reply::none();
                }
                let lay = listview::layout(list, rect, &ctx.theme.look.controls());
                if list.scroll(rows as i64, &lay) {
                    Reply::redraw()
                } else {
                    Reply::none()
                }
            }
            Event::Pointer(PointerInput::Release {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => {
                let lit = self.list_unhover(true, ctx);
                if let Some(r) = self.list_release(x, y, &ctx.theme.look.controls()) {
                    return r;
                }
                let ev = self.sheet.release();
                let r = self.sheet_event(ev);
                Reply {
                    redraw: r.redraw || lit,
                    ..r
                }
            }
            _ => Reply::none(),
        }
    }

    fn fit_columns(&mut self, cm: &ControlMetrics) {
        let a = self.area();
        pages::fit_shortcut_columns(&mut self.shortcuts, cm, pages::shortcuts(a));
        defaultspage::fit_columns(&mut self.types, cm, defaultspage::list(a));
        pages::fit_location_columns(&mut self.locations, cm, pages::locations(a));
    }

    fn paint(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.fit_columns(&ctx.theme.look.controls());
        let cs = &ctx.theme.colors;
        let px = ctx.theme.fonts.size_px;
        if win == ERROR_WIN {
            if let Some(b) = &self.error {
                b.draw(c, &ctx.theme, &self.dialog_text, self.dialog_px);
            }
            return;
        }
        if win == SHORTCUT_WIN {
            c.fill_rect(c.bounds(), cs.button_face);
            if let Some(d) = &self.shortcut {
                form::draw(
                    c,
                    &ctx.text,
                    &Paint {
                        s: 1,
                        ..ctx.theme.paint()
                    },
                    &d.form,
                    true,
                    px,
                );
            }
            return;
        }
        if matches!(win, LAYOUT_WIN | ADD_ITEM_WIN | RENAME_WIN | SAVER_WIN) {
            c.fill_rect(c.bounds(), cs.button_face);
            let dialog = match win {
                LAYOUT_WIN => self.add_layout.as_ref().map(|d| &d.form),
                ADD_ITEM_WIN => self.add_item.as_ref().map(|d| &d.form),
                SAVER_WIN => self.saver_dlg.as_ref().map(|d| &d.form),
                _ => self.rename.as_ref().map(|d| &d.form),
            };
            if let Some(f) = dialog {
                let p = Paint {
                    s: 1,
                    ..ctx.theme.paint()
                };
                form::draw(c, &ctx.text, &p, f, true, px);
            }
            return;
        }
        if win == displayhost::MONITOR_WIN {
            c.fill_rect(c.bounds(), cs.button_face);
            if let Some(q) = &self.display.flow.question {
                let p = Paint {
                    s: 1,
                    ..ctx.theme.paint()
                };
                form::draw(c, &ctx.text, &p, &q.dialog.form, true, px);
            }
            return;
        }
        if win == CHOOSER_WIN {
            if let Some(ch) = &self.chooser {
                let lay = ch.ow.layout(
                    &self.dialog_text,
                    self.dialog_px,
                    (0, 0),
                    &ctx.theme.look.controls(),
                );
                let rows: Vec<Option<&Canvas>> = ch.row_icons.iter().map(Option::as_ref).collect();
                ch.ow.draw(
                    c,
                    &self.dialog_text,
                    &Paint {
                        s: 1,
                        ..ctx.theme.paint()
                    },
                    &lay,
                    ch.icon.as_ref(),
                    &rows,
                    ch.input.cues(),
                    self.dialog_px,
                );
            }
            return;
        }
        if win == ADD_WIN {
            if let Some(d) = &self.add {
                d.draw(
                    c,
                    &ctx.text,
                    &Paint {
                        s: 1,
                        ..ctx.theme.paint()
                    },
                    Rect::new(0, 0, d.client().0, d.client().1),
                    px,
                );
            }
            return;
        }
        if self.sheet.tabs.open == Page::Pointers.index() {
            self.refresh_preview(cs.window);
        }
        c.fill_rect(c.bounds(), cs.button_face);
        propsheet::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &self.sheet,
            true,
            px,
        );
        if self.sheet.tabs.open == Page::Startup.index() {
            let r = self.on_page(pages::list(self.area()));
            pages::draw_list(c, &ctx.text, &ctx.theme, &self.list, r, px, 1);
        }
        if self.sheet.tabs.open == Page::Keyboard.index() {
            let r = self.on_page(pages::shortcuts(self.area()));
            pages::draw_shortcuts(c, &ctx.text, &ctx.theme, &self.shortcuts, r, px, 1);
        }
        if self.sheet.tabs.open == Page::DefaultPrograms.index() {
            let r = self.on_page(defaultspage::list(self.area()));
            pages::draw_list(c, &ctx.text, &ctx.theme, &self.types, r, px, 1);
            // A combo dropped over the list covers it.
            let p = Paint {
                s: 1,
                ..ctx.theme.paint()
            };
            propsheet::draw_popups(c, &ctx.text, &p, &self.sheet, px);
        }
        if self.sheet.tabs.open == Page::Advanced.index() {
            let r = self.on_page(pages::locations(self.area()));
            pages::draw_list(c, &ctx.text, &ctx.theme, &self.locations, r, px, 1);
        }
    }

    fn describe(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        self.fit_columns(&ctx.theme.look.controls());

        let px = ctx.theme.fonts.size_px;
        match win {
            ERROR_WIN => {
                if let Some(b) = &self.error {
                    b.access(tree, &self.dialog_text, self.dialog_px);
                }
            }
            SHORTCUT_WIN => {
                if let Some(d) = &self.shortcut {
                    oneshot::access_form(&d.form, tree, &ctx.text, px);
                }
            }
            LAYOUT_WIN => {
                if let Some(d) = &self.add_layout {
                    oneshot::access_form(&d.form, tree, &ctx.text, px);
                }
            }
            ADD_ITEM_WIN => {
                if let Some(d) = &self.add_item {
                    oneshot::access_form(&d.form, tree, &ctx.text, px);
                }
            }
            RENAME_WIN => {
                if let Some(d) = &self.rename {
                    oneshot::access_form(&d.form, tree, &ctx.text, px);
                }
            }
            SAVER_WIN => {
                if let Some(d) = &self.saver_dlg {
                    oneshot::access_form(&d.form, tree, &ctx.text, px);
                }
            }
            displayhost::MONITOR_WIN => {
                if let Some(q) = &self.display.flow.question {
                    oneshot::access_form(&q.dialog.form, tree, &ctx.text, px);
                }
            }
            CHOOSER_WIN => {
                if let Some(ch) = &self.chooser {
                    let cm = ctx.theme.look.controls();
                    let lay = ch.ow.layout(&self.dialog_text, self.dialog_px, (0, 0), &cm);
                    ch.ow.access(tree, &lay);
                }
            }
            ADD_WIN => {
                if let Some(d) = &self.add {
                    let r = Rect::new(0, 0, d.client().0, d.client().1);
                    d.access(tree, r, &ctx.text, px);
                }
            }
            _ => {
                let (panel, mut focus) = self.sheet.access(tree, ROOT, &ctx.text, px);
                if let Some((list, rect, focused)) = self.side_list() {
                    let lay = listview::layout(list, rect, &ctx.theme.look.controls());
                    let row = list.access(&lay, tree, panel, "side list");
                    if *focused {
                        focus = Some(row);
                    }
                }
                if let Some(f) = focus {
                    tree.set_focus(f);
                }
            }
        }
    }
}

fn main() {
    env_logger::init();
    let page = page_arg(std::env::args().skip(1));
    let action = page.map(|p| (PAGE_ACTION.to_string(), vec![p.slug()]));
    let requests = match instance::claim(BUS_NAME, action) {
        Claim::First(r) => Some(r),
        Claim::Handed => return,
        Claim::Alone => None,
    };
    repriseapp::run(Box::new(SettingsApp::new(page, requests)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "reprise-settings-main-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("temp dir");
        p
    }

    fn app(tag: &str) -> SettingsApp {
        SettingsApp::open(tmp(tag).join("config.toml"))
    }

    fn test_ctx() -> Ctx {
        Ctx {
            theme: Theme::default(),
            text: TextRenderer::load(&FontConfig::default()),
            icons: reprisekit::icontheme::IconCache::new(IconTheme::load("test", &[])),
            caret_on: true,
            size: (10, 10),
            maximized: false,
            origin: (0, 0),
            wake: repriseapp::Waker::noop(),
            heads: repriseapp::heads::HeadRequests::default(),
            text_size: 100,
        }
    }

    const MAIN: WinId = repriseapp::MAIN;

    fn page_request(name: &str) -> instance::Request {
        instance::Request {
            action: Some((PAGE_ACTION.into(), vec![name.into()])),
            token: None,
        }
    }

    fn pick_row(a: &mut SettingsApp, i: usize) {
        let page = &mut a.sheet.pages[Page::Background.index()];
        if let Some(form::Control::ListBox { list, .. }) = page
            .item_mut(backgroundpage::WALLPAPER)
            .map(|i| &mut i.control)
        {
            list.select(i, 11);
        }
        a.pick_wallpaper();
    }

    /// Under Blue the list starts on Blue's own built-in; a built-in
    /// picked is written as `reprise:design/variant`, and "(None)" as an
    /// empty image, which stands for none rather than Blue's default.
    #[test]
    fn the_background_list_picks_and_writes_a_builtin() {
        let dir = tmp("builtin");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[appearance]\ninterface = \"blue\"\n").unwrap();
        let mut a = SettingsApp::open(path.clone());
        let at = backgroundpage::selected(&a.sheet.pages[Page::Background.index()]);
        assert_eq!(
            at.map(backgroundpage::row),
            Some(backgroundpage::Row::Builtin(
                backgroundpage::builtin("reprise:tarn/blue").unwrap()
            ))
        );
        let dusk = backgroundpage::builtins()
            .iter()
            .position(|b| b.to_string() == "ridges/dusk")
            .unwrap();
        pick_row(&mut a, dusk + 1);
        assert_eq!(a.want.wallpaper, "reprise:ridges/dusk");
        assert!(a.apply());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("image = \"reprise:ridges/dusk\""), "{text}");
        pick_row(&mut a, 0);
        assert!(a.apply());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("image = \"\""), "{text}");
    }

    #[test]
    fn the_drive_combo_shows_and_writes_on_insert() {
        let dir = tmp("drive-insert");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[drives]\non_insert = \"mount\"\n").unwrap();
        let mut a = SettingsApp::open(path.clone());
        let dp = Page::DefaultPrograms.index();
        assert_eq!(
            a.sheet.pages[dp].combo_selected(defaultspage::DRIVE_INSERT),
            Some(1),
            "Mount it"
        );
        a.sheet.pages[dp].set_combo(defaultspage::DRIVE_INSERT, 2);
        a.sheet_event(propsheet::Event::Page(
            dp,
            form::Event::Changed(defaultspage::DRIVE_INSERT),
        ));
        assert!(a.apply());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "[drives]\non_insert = \"open\"\n");
        a.sheet.pages[dp].set_combo(defaultspage::DRIVE_INSERT, 0);
        a.sheet_event(propsheet::Event::Page(
            dp,
            form::Event::Changed(defaultspage::DRIVE_INSERT),
        ));
        assert!(a.apply());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text, "",
            "Ask what to do is the default and removes the key"
        );
    }

    #[test]
    fn a_drop_on_a_page_field_is_read_into_the_settings() {
        let mut a = app("drop");
        let mut ctx = test_ctx();
        let px = ctx.theme.fonts.size_px;
        a.sheet.tabs.open = Page::Advanced.index();
        let form = a.drop_form(MAIN).expect("the open page").form;
        let r = form
            .item("task_manager")
            .unwrap()
            .placed(&reprisekit::controls::metrics::ControlMetrics::CLASSIC);
        assert!(form.drag_over(r.x + 4, r.y + r.height / 2, &ctx.text, px));
        let changed = form.drop_text("htop", &ctx.text, px);
        assert_eq!(changed, form::Event::Changed("task_manager"));
        a.event(MAIN, Event::FieldDropped(changed), &mut ctx);
        assert!(
            a.want.task_manager.contains("htop"),
            "{}",
            a.want.task_manager
        );
    }

    #[test]
    fn a_later_start_opens_its_page_unless_a_dialog_is_up() {
        let mut a = app("request");
        assert!(a.request(&page_request("fonts-and-icons")));
        assert_eq!(a.sheet.tabs.open, Page::FontsAndIcons.index());
        assert!(!a.request(&page_request("nowhere")));
        assert!(!a.request(&instance::Request::default()));
        a.fail("broken");
        assert!(!a.request(&page_request("startup")));
        assert_eq!(a.sheet.tabs.open, Page::FontsAndIcons.index());
    }

    #[test]
    fn a_program_row_edits_its_keys_and_delete_forgets_it() {
        let dir = tmp("programs");
        std::fs::write(
            dir.join("config.toml"),
            "[[global_shortcuts]]\napp = \"org.example.NoSuchApp\"\nid = \"talk\"\n\
             description = \"Hold to talk\"\nkeys = \"Pause\"\n\
             [portal]\nshortcuts_denied = [\"org.example.NoSuchNotes\"]\n",
        )
        .unwrap();
        let mut app = SettingsApp::open(dir.join("config.toml"));
        let n = app.want.shortcuts.len();
        assert_eq!(
            app.shortcuts.rows[n].cells,
            ["Pause", "org.example.NoSuchApp: Hold to talk"]
        );
        app.shortcuts.selection.set_only(Some(n + 1));
        app.edit_shortcut();
        assert!(app.shortcut.is_none(), "a refusal has nothing to edit");

        app.shortcuts.selection.set_only(Some(n));
        app.edit_shortcut();
        let d = app.shortcut.as_mut().expect("the dialog is up");
        assert_eq!(d.title, "Change Shortcut");
        d.event(form::Event::Pressed(shortcutdlg::CLEAR));
        let shortcutdlg::Outcome::Accept(sc) = d.event(form::Event::Pressed(shortcutdlg::OK))
        else {
            panic!("no keys is a valid answer");
        };
        assert_eq!(app.accept_shortcut(sc), n);
        assert_eq!(app.want.programs[0].keys, "");
        assert_eq!(app.want.shortcuts.len(), n, "no bind was added");
        assert_eq!(app.shortcuts.rows[n].cells[0], "None");

        app.shortcuts.selection.set_only(Some(n + 1));
        app.remove_shortcut();
        assert!(app.want.denied.is_empty());
        app.shortcuts.selection.set_only(Some(n));
        app.remove_shortcut();
        assert!(app.want.programs.is_empty());
        assert_eq!(app.shortcuts.rows.len(), n);
    }

    #[test]
    fn a_rebound_zoom_key_rewrites_the_vision_note() {
        let dir = tmp("zoom-note");
        std::fs::write(dir.join("config.toml"), "").unwrap();
        let mut app = SettingsApp::open(dir.join("config.toml"));
        let note = |app: &SettingsApp| match &app.sheet.pages[Page::Vision.index()]
            .item("mag_note")
            .unwrap()
            .control
        {
            form::Control::Lines(lines) => lines.join(" "),
            _ => panic!("the note is lines"),
        };
        assert!(note(&app).starts_with("Super+= zooms in"), "{}", note(&app));
        let i = (app.want.shortcuts.iter())
            .position(|sc| sc.action == "zoom-in")
            .expect("zoom-in is bound by default");
        app.shortcut_target = Some(model::Row::Bind(i));
        app.accept_shortcut(model::Shortcut::new("Ctrl+Alt+plus", "zoom-in", ""));
        assert!(
            note(&app).starts_with("Ctrl+Alt++ zooms in"),
            "{}",
            note(&app)
        );
    }

    #[test]
    fn a_programs_taken_keys_name_the_program() {
        let dir = tmp("program-taken");
        std::fs::write(
            dir.join("config.toml"),
            "[[global_shortcuts]]\napp = \"org.example.NoSuchApp\"\nid = \"talk\"\n\
             description = \"Hold to talk\"\nkeys = \"Pause\"\n\
             [[global_shortcuts]]\napp = \"org.example.NoSuchApp\"\nid = \"mute\"\nkeys = \"F9\"\n",
        )
        .unwrap();
        let app = SettingsApp::open(dir.join("config.toml"));
        let taken = app.taken_keys(None);
        let n = app.want.shortcuts.len();
        assert_eq!(taken[n].action, "org.example.NoSuchApp: Hold to talk");
        assert_eq!(
            taken[n + 1].action,
            "org.example.NoSuchApp: mute",
            "no description: the id"
        );
    }

    /// DP-1 listed with a smaller resolution picked, Apply or OK pressed
    /// and taken, so the question is up.
    fn question_up(tag: &str, press: displayhost::Press) -> (SettingsApp, Ctx) {
        let mut app = app(tag);
        let mut ctx = test_ctx();
        app.event(
            MAIN,
            Event::Heads(vec![displaypage::test_head("DP-1")]),
            &mut ctx,
        );
        app.display.model.pick_resolution(1);
        app.start_display(press, &mut ctx.heads);
        let asked = ctx.heads.take();
        assert_eq!(asked.len(), 1, "the mode is asked for");
        let r = app.event(
            MAIN,
            Event::HeadsApplied {
                serial: asked[0].0,
                ok: true,
            },
            &mut ctx,
        );
        assert!(r.tick, "the countdown runs on the main window's ticks");
        assert!(
            app.children()
                .iter()
                .any(|c| c.id == displayhost::MONITOR_WIN),
            "Monitor Settings is up"
        );
        (app, ctx)
    }

    #[test]
    fn the_questions_close_box_puts_the_old_mode_back_and_keeps_the_sheet() {
        let (mut app, mut ctx) = question_up("question-close", displayhost::Press::Apply);
        let r = app.event(displayhost::MONITOR_WIN, Event::CloseRequest, &mut ctx);
        assert!(!r.quit);
        assert!(app.display.flow.question.is_none());
        let back = ctx.heads.take();
        assert_eq!(back.len(), 1);
        assert_eq!(
            back[0].1,
            vec![("DP-1".to_string(), displaypage::test_head("DP-1").state())]
        );
        let r = app.event(
            MAIN,
            Event::HeadsApplied {
                serial: back[0].0,
                ok: true,
            },
            &mut ctx,
        );
        assert!(!r.quit, "the sheet stays open after the put-back");
        assert!(!app.path.exists(), "nothing written");
    }

    #[test]
    fn yes_after_ok_writes_the_output_and_quits() {
        let (mut app, mut ctx) = question_up("question-yes", displayhost::Press::Ok);
        let q = app.display.flow.question.as_ref().expect("the question");
        let yes = q.dialog.form.item(monitordlg::YES).expect("a Yes button");
        let r = yes.placed(&ctx.theme.look.controls());
        let (x, y) = (r.x + r.width / 2, r.y + r.height / 2);
        let mods = repriseapp::Mods::default();
        let press = PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time: 0,
            mods,
        };
        let release = PointerInput::Release {
            x,
            y,
            button: BTN_LEFT,
            mods,
        };
        let win = displayhost::MONITOR_WIN;
        assert!(!app.event(win, Event::Pointer(press), &mut ctx).quit);
        assert!(app.event(win, Event::Pointer(release), &mut ctx).quit);
        let text = std::fs::read_to_string(&app.path).expect("config written");
        assert!(
            text.contains("name = \"DP-1\"") && text.contains("mode = \"1280x720@60\""),
            "DP-1's [[outputs]] entry has the new mode: {text}"
        );
    }

    /// Two outputs side by side; the second dragged under the first and
    /// made the primary one. Apply writes the primary at once and asks
    /// for both places in one request; Yes writes both.
    #[test]
    fn a_kept_arrangement_writes_every_place_and_the_primary() {
        let mut app = app("arrangement");
        let mut ctx = test_ctx();
        let mut b = displaypage::test_head("HDMI-A-1");
        b.position = (1920, 0);
        app.event(
            MAIN,
            Event::Heads(vec![displaypage::test_head("DP-1"), b]),
            &mut ctx,
        );
        let mut well = reprisekit::controls::arrange::Arrange::new(app.display.model.tiles(), 1);
        well.tiles[1].pos = (0, 1080);
        app.display.model.set_arrangement(&well);
        app.display.model.make_primary();
        app.start_display(displayhost::Press::Apply, &mut ctx.heads);
        let text = std::fs::read_to_string(&app.path).expect("the primary is written at once");
        assert!(text.contains("primary = true"), "{text}");
        let asked = ctx.heads.take();
        assert_eq!(asked.len(), 1, "one request");
        let places: Vec<_> = asked[0]
            .1
            .iter()
            .map(|(n, c)| (n.as_str(), c.position))
            .collect();
        assert_eq!(
            places,
            vec![("DP-1", Some((0, 0))), ("HDMI-A-1", Some((0, 1080)))]
        );
        app.event(
            MAIN,
            Event::HeadsApplied {
                serial: asked[0].0,
                ok: true,
            },
            &mut ctx,
        );
        let effects = app.display.flow.answer(true, &mut ctx.heads);
        app.display_effects(effects);
        let text = std::fs::read_to_string(&app.path).unwrap();
        assert_eq!(
            outputwrite::read_primary(&text).as_deref(),
            Some("HDMI-A-1")
        );
        assert!(
            text.contains("position = [0, 0]") && text.contains("position = [0, 1080]"),
            "{text}"
        );
    }

    #[test]
    fn a_refused_vrr_on_ok_says_so_and_keeps_the_sheet() {
        let mut app = app("vrr-refused");
        let mut ctx = test_ctx();
        // The test output supports VRR and runs without it.
        app.event(
            MAIN,
            Event::Heads(vec![displaypage::test_head("DP-1")]),
            &mut ctx,
        );
        app.display.model.set_vrr(true);
        let r = app.start_display(displayhost::Press::Ok, &mut ctx.heads);
        assert!(!r.quit, "waits for the answer");
        let asked = ctx.heads.take();
        assert_eq!(asked.len(), 1, "only VRR is asked for");
        let r = app.event(
            MAIN,
            Event::HeadsApplied {
                serial: asked[0].0,
                ok: false,
            },
            &mut ctx,
        );
        assert!(!r.quit, "the sheet stays open");
        let text = app.error.as_ref().map(|b| b.fields.text.clone());
        assert_eq!(text.as_deref(), Some(displayflow::VRR_REFUSED));
        assert!(
            app.children().iter().any(|c| c.id == ERROR_WIN),
            "the error box is up"
        );
        assert!(!app.display.model.pick().unwrap().vrr, "the box unticks");
    }

    #[test]
    fn a_save_that_fails_shows_why_and_does_not_quit() {
        let mut app = app("save-fails");
        std::fs::write(&app.path, "[general\n").expect("write");
        let keys = outputwrite::OutputKeys {
            adaptive_sync: Some(true),
            ..outputwrite::OutputKeys::default()
        };
        let r = app.display_effects(vec![
            displayflow::Effect::Save("DP-1".into(), keys),
            displayflow::Effect::Quit,
        ]);
        assert!(!r.quit, "the error stays on screen");
        let text = app.error.as_ref().map(|b| b.fields.text.clone());
        assert_eq!(
            text.as_deref(),
            Some("config.toml does not parse; the display setting was not saved.")
        );
    }

    /// A resize lays every page out afresh and keeps what the controls hold; the size is kept for the next run unless maximized.
    #[test]
    fn the_window_resizes_and_its_size_is_kept() {
        let mut app = app("resize");
        assert!(!app.fixed());
        let min = app.min_size(MAIN).unwrap();
        assert_eq!(min, (544, 528));
        assert_eq!(app.sheet.size, min, "built at its smallest");
        let (w, h) = winstate::DEFAULT;
        assert!(
            w * 10 >= min.0 * 13 && h * 10 >= min.1 * 12,
            "opens noticeably larger"
        );
        let state = tmp("resize-state").join("settings.toml");
        app.state_path = Some(state.clone());
        let startup = Page::Startup.index();
        let add = |app: &SettingsApp| app.sheet.pages[startup].item(pages::ADD).unwrap().rect;
        let before = add(&app);
        let windows = Page::Windows.index();
        let tabs = !app.sheet.pages[windows].checked("window_tabs").unwrap();
        app.sheet.pages[windows].set_check("window_tabs", tabs);

        let mut ctx = test_ctx();
        ctx.size = (900, 700);
        app.event(MAIN, Event::Resized, &mut ctx);
        assert_eq!(app.sheet.size, (900, 700));
        let after = add(&app);
        assert_eq!(
            (after.x, after.y),
            (before.x + 356, before.y + 172),
            "Add... keeps to the longer, wider list's corner"
        );
        assert_eq!(app.sheet.pages[windows].checked("window_tabs"), Some(tabs));
        app.event(MAIN, Event::Focus(false), &mut ctx);
        assert_eq!(winstate::load(&state, min), (900, 700), "kept");

        ctx.size = (1900, 1000);
        ctx.maximized = true;
        app.event(MAIN, Event::Resized, &mut ctx);
        assert_eq!(app.sheet.size, (1900, 1000));
        app.event(MAIN, Event::Focus(false), &mut ctx);
        assert_eq!(winstate::load(&state, min), (900, 700), "not the screen's");

        ctx.size = (300, 200);
        ctx.maximized = false;
        app.event(MAIN, Event::Resized, &mut ctx);
        assert_eq!(app.sheet.size, min, "no smaller than its smallest");
        assert_eq!(add(&app), before);
    }

    #[test]
    fn the_page_argument_names_a_page() {
        let args = |v: &[&str]| page_arg(v.iter().map(|s| s.to_string()));
        assert_eq!(args(&["--page", "night-light"]), Some(Page::NightLight));
        assert_eq!(args(&["--page=fonts-and-icons"]), Some(Page::FontsAndIcons));
        assert_eq!(args(&["--page", "startup"]), Some(Page::Startup));
        assert_eq!(args(&["--page", "appearance"]), Some(Page::Appearance));
        assert_eq!(args(&["--page", "background"]), Some(Page::Background));
        assert_eq!(args(&["--page", "nowhere"]), None);
        assert_eq!(args(&[]), None);
    }

    #[test]
    fn a_missing_config_opens_on_the_defaults_without_an_error() {
        let dir = tmp("missing");
        assert_eq!(read_config(&dir.join("config.toml")), (String::new(), None));
    }

    #[test]
    fn a_config_that_does_not_parse_names_its_line() {
        let path = tmp("broken").join("config.toml");
        std::fs::write(&path, "[general]\nx = \n").expect("write");
        let (text, err) = read_config(&path);
        assert_eq!(text, "[general]\nx = \n");
        let err = err.expect("a parse error for the box");
        assert!(
            err.starts_with("config.toml does not parse at line 2: "),
            "the box names line 2, got {err:?}"
        );
    }

    #[test]
    fn an_unreadable_config_is_reported() {
        // A directory where the file should be fails the read with EISDIR.
        let path = tmp("unreadable").join("config.toml");
        std::fs::create_dir_all(&path).expect("mkdir");
        let (text, err) = read_config(&path);
        assert_eq!(text, "");
        let err = err.expect("a read error for the box");
        assert!(
            err.starts_with("config.toml could not be read: "),
            "the box says the read failed, got {err:?}"
        );
    }

    fn forget_enabled(app: &SettingsApp) -> bool {
        match &app.sheet.pages[Page::Advanced.index()]
            .item(pages::FORGET)
            .unwrap()
            .control
        {
            form::Control::Button { enabled, .. } => *enabled,
            _ => panic!("Forget is a button"),
        }
    }

    #[test]
    fn forget_drops_a_location_answer_from_the_file() {
        let dir = tmp("location");
        let file = dir.join("location.toml");
        std::fs::write(
            &file,
            "[apps]\n\"org.example.NoSuchMaps\" = true\n\"org.example.NoSuchWeather\" = false\n",
        )
        .unwrap();
        let mut app = SettingsApp::open(dir.join("config.toml"));
        app.location_file = file.clone();
        app.reload_locations();
        let cells: Vec<Vec<String>> = app.locations.rows.iter().map(|r| r.cells.clone()).collect();
        assert_eq!(
            cells,
            [
                ["org.example.NoSuchMaps", "Allowed"],
                ["org.example.NoSuchWeather", "Denied"],
            ]
            .map(|r| r.map(String::from).to_vec())
        );
        assert!(!forget_enabled(&app), "Forget greys with no row picked");

        app.locations.selection.set_only(Some(0));
        app.sync_apply();
        assert!(forget_enabled(&app));
        app.forget_location();
        assert_eq!(
            repriseconf::location::load_from(&file),
            std::collections::BTreeMap::from([("org.example.NoSuchWeather".to_string(), false)])
        );
        assert_eq!(app.locations.rows.len(), 1);
        assert_eq!(app.locations.rows[0].cells[0], "org.example.NoSuchWeather");
        assert!(!app.dirty(), "Forget leaves nothing for Apply");
    }
}
