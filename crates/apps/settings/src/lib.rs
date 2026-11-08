//! Reprise Settings: a property sheet over `config.toml` and the XDG autostart directories.
//!
//! Pages are listed in `registry::GROUPS`. `--page night-light` opens on one (`--focus mouse_keys` then focuses a control there), and a second start hands its page to the running sheet (`reprisebus::instance`). OK and Apply rewrite `config.toml` with `toml_edit` and the autostart files, and save the Login Manager page through `pkexec` (`loginhost`); a change written elsewhere while the sheet is open is reread (`configwatch`). The Startup list's Add... and Remove, Quick Launch changes, the Privacy page's Forget, the Advanced page's Clear and the Backup page's Export... and Import... (`backup`, also `--export`/`--import FILE`) act at once. The Display page goes through wlr-output-management (`repriseapp::heads`); only a Yes on the Monitor Settings question writes `[[outputs]]`.

#[cfg(test)]
reprisebus::test_session!();

mod additem;
mod appearancepage;
mod apply;
mod autostart;
mod backgroundhost;
mod backgroundpage;
mod backup;
mod backuphost;
mod colorhost;
mod configwatch;
mod cursors;
mod defaultshost;
mod defaultspage;
mod displayflow;
mod displayhost;
mod displaypage;
mod draw;
mod events;
mod filepick;
mod keyboardhost;
mod layoutdlg;
mod linkhost;
mod locations;
mod loginhost;
mod magnifierpage;
mod menuhost;
mod menupage;
mod model;
mod monitordlg;
mod notifyhost;
mod notifypage;
mod outputwrite;
mod pages;
mod quickpage;
mod registry;
mod remotehost;
mod renamedlg;
mod saverdlg;
mod saverhost;
mod saverpage;
mod searchhost;
mod shortcutdlg;
mod sidelists;
mod startmenu;
mod tablethost;
mod tabletpage;
mod wake;
mod wallpapers;
mod winstate;

use crate::pages::Area;
use crate::registry::Page;
use filepick::Pick;
use locations::{Located, LocationAnswer};
use repriseapp::decode::Decoded;
use repriseapp::filedlg::Host;
use repriseapp::oneshot::MsgBox;
use repriseapp::openwith::ChooserInput;
use reprisebus::instance::{self, Claim};
use repriseconf::pages::Entry;
use reprisekit::access::Tree;
use reprisekit::controls::filedlg::{self, FileDlg};
use reprisekit::controls::hosted::Hosted;
use reprisekit::controls::listview::ListView;
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::openwith::{self, OpenWith};
use reprisekit::controls::propsheet::PropSheet;
use reprisekit::icontheme::IconTheme;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::{TextUnits, Theme};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, TryRecvError};

use repriseapp::{App, Child, Ctx, DropForm, Event, FileDrop, PointerInput, Reply, Waker, WinId};

/// The file dialog: Startup Add... and Background Browse....
const ADD_WIN: WinId = WinId::named("add");
const SHORTCUT_WIN: WinId = WinId::named("shortcut");
const COLOR_WIN: WinId = WinId::named("color");
const LAYOUT_WIN: WinId = WinId::named("layout");
const ADD_ITEM_WIN: WinId = WinId::named("add_item");
const RENAME_WIN: WinId = WinId::named("rename");
const SAVER_WIN: WinId = WinId::named("saver");

const CHOOSER_WIN: WinId = WinId::named("openwith");

const ERROR_WIN: WinId = WinId::named("error");
const ASK_WIN: WinId = WinId::named("ask");

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

/// `--page <name>` and `--focus <control id>`, in either order and with or without `=`. A name
/// a separate program took over gives that program, unless `--focus` names a control a page holds.
fn page_arg(mut args: impl Iterator<Item = String>) -> Option<(Entry, Option<String>)> {
    let (mut page, mut focus) = (None, None);
    while let Some(a) = args.next() {
        let (flag, value) = match a.split_once('=') {
            Some((f, v)) => (f.to_string(), Some(v.to_string())),
            None => (a, None),
        };
        let slot = match flag.as_str() {
            "--page" => &mut page,
            "--focus" => &mut focus,
            _ => continue,
        };
        *slot = value.or_else(|| args.next());
    }
    let name = page?;
    let Some(entry) = repriseconf::pages::named(&name) else {
        log::warn!("--page {name:?} names no page");
        return None;
    };
    let entry = match focus.as_deref() {
        Some(id) => holding(entry, id),
        None => entry,
    };
    Some((entry, focus))
}

/// `entry`, or the page whose search terms hold control `id` when `entry` is a program or a
/// page whose terms don't (a control that moved pages keeps its `--focus` callers).
fn holding(entry: Entry, id: &str) -> Entry {
    let holds = |p: &Page| p.terms().iter().any(|&(i, _)| i == id);
    if let Entry::Page(p) = entry
        && holds(&p)
    {
        return entry;
    }
    Page::all().find(holds).map_or(entry, Entry::Page)
}

#[derive(Debug, PartialEq, Eq)]
enum BackupVerb {
    Export(PathBuf),
    Import(PathBuf),
}

/// `--export FILE` or `--import FILE`, with or without `=`.
fn backup_arg(args: &[String]) -> Option<BackupVerb> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let (flag, value) = match a.split_once('=') {
            Some((f, v)) => (f, Some(v.to_string())),
            None => (a.as_str(), None),
        };
        let make = match flag {
            "--export" => BackupVerb::Export,
            "--import" => BackupVerb::Import,
            _ => continue,
        };
        return Some(make(PathBuf::from(value.or_else(|| it.next().cloned())?)));
    }
    None
}

/// Export or import without a window; the exit status.
fn run_backup(verb: BackupVerb) -> i32 {
    let roots = backup::Roots::current();
    let done = match &verb {
        BackupVerb::Export(file) => backup::export_to(file, &roots)
            .map(|()| format!("Exported the settings to {}.", file.display())),
        BackupVerb::Import(file) => backup::read(file).and_then(|bundle| {
            let kept = backup::import(&bundle, &roots)?;
            let what: Vec<&str> = bundle.replaces().iter().map(|e| e.what).collect();
            Ok(format!(
                "Imported {}: {}.\nThe files it replaced are in {}.",
                file.display(),
                what.join(", "),
                kept.display()
            ))
        }),
    };
    match done {
        Ok(line) => {
            println!("{line}");
            0
        }
        Err(why) => {
            eprintln!("{why}");
            1
        }
    }
}

/// The bus name the running sheet owns; a later start sends it a `page` action.
const BUS_NAME: &str = "org.reprise.Settings";
const PAGE_ACTION: &str = "page";

/// A channel's next value; `Err` once the sender is gone.
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
    /// Each page's hit for the search field's query (`searchhost`); empty while the field is.
    search_hits: Vec<Option<repriseconf::pages::Hit>>,

    entries: Vec<reprisefs::autostart::Entry>,
    applied_checks: Vec<bool>,
    /// What the compositor filters `OnlyShowIn` / `NotShowIn` against.
    desktops: Vec<String>,
    /// The list's own `checks` are the pending state.
    list: ListView,
    /// The list width, bar width and text units the Startup columns were last filled to.
    list_fit: Option<(i32, i32, TextUnits)>,
    list_focus: bool,

    shortcuts: ListView,
    shortcuts_focus: bool,
    shortcut: Option<shortcutdlg::ShortcutDialog>,
    /// The Color dialog an Other... opened, with the page and the colour button it sets.
    color_dlg: Option<(repriseapp::colordlg::ColorDialog, usize, &'static str)>,
    /// The Color dialog's custom colours.
    custom_colors: [reprisekit::theme::Color; reprisekit::controls::colordlg::CUSTOM],
    shortcut_target: Option<model::Row>,
    app_names: HashMap<String, String>,
    /// Where desktop entries are looked up ([`linkhost::dirs`]).
    app_dirs: Vec<PathBuf>,
    /// Each link row's shown entry, in `repriseconf::pages::links()` order.
    links: Vec<Option<reprisefs::apps::AppEntry>>,

    defaults: Option<defaultspage::Defaults>,
    types: ListView,
    types_focus: bool,
    chooser: Option<Chooser>,

    locations: ListView,
    locations_focus: bool,
    location_answers: Vec<LocationAnswer>,
    /// Each location row's icon name, looked up once `location_icons_due`.
    location_icons: Vec<Option<String>>,
    location_icons_due: bool,
    location_file: PathBuf,
    /// The location list is read the first time the Privacy page shows, and again after a
    /// failed Forget; `locating` holds the read under way.
    locations_due: bool,
    locating: Option<Receiver<Located>>,

    pads: Vec<reprisebus::tray::Pad>,
    pad_picked: usize,
    pads_fresh: bool,

    /// The Notifications page's programs: those the compositor saw send notifications (asked each time
    /// the page opens, `notify_fresh`) and those the config lists.
    notify_programs: Vec<notifypage::Program>,
    notify_seen: Vec<(String, String)>,
    /// The installed desktop entries' names of the listed programs not seen this session.
    notify_names: HashMap<String, String>,
    notify_fresh: bool,

    /// The Remote Desktop server as last read; `remote_job` holds the read or act under way,
    /// an act while `remote_acting`.
    remote: remotehost::Remote,
    remote_job: Option<Receiver<remotehost::Reading>>,
    remote_acting: bool,

    /// The Login Manager page's machine settings, read when it first opens with the accounts
    /// `login_source` gives; `login_job` holds a save through `login_run` under way.
    login: Option<loginhost::Login>,
    login_files: loginhost::Files,
    login_source: fn() -> repriselogin::users::Source,
    login_run: fn(&str) -> std::io::Result<std::process::Output>,
    login_users: Option<Receiver<Vec<String>>>,
    login_job: Option<loginhost::Saving>,
    /// The page's next read of the front's fingerprint.
    login_poll: Option<Receiver<Option<String>>>,
    /// The pages this machine lacks the program for, kept out of the list ([`Page::installed`]).
    absent: Vec<Page>,

    xkb: Vec<reprisefs::xkb::Layout>,
    /// The layout an empty `[keyboard] layout` gives, read once at start.
    system_layout: String,
    add_layout: Option<layoutdlg::LayoutDialog>,

    add_item: Option<additem::AddItemDialog>,
    rename: Option<renamedlg::RenameDialog>,
    menu_icons_due: bool,
    saver_dlg: Option<saverdlg::SaverDialog>,
    saver_preview: Option<(saverpage::Preview, std::time::Instant)>,
    saver_ticking: bool,
    /// BlueZ's paired devices, listed each time the Screen Saver & Lock page opens.
    away_devices: Vec<reprisebus::bluez::Device>,
    away_fresh: bool,
    saver_photos: Option<(String, Receiver<Vec<PathBuf>>)>,
    /// The Photos preview's next picture, decoding.
    saver_photo: Option<Receiver<Option<(PathBuf, Canvas)>>>,
    add_to_quick: bool,

    quick_dir: Option<PathBuf>,
    quick: Vec<quickpage::Launcher>,
    quick_icons_due: bool,
    controls: ControlMetrics,

    add: Option<FileDlg>,
    add_host: Host,
    add_for: Pick,

    error: Option<MsgBox>,
    asking: Option<backuphost::Ask>,
    dialog_text: Rc<TextRenderer>,
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
    /// The Advanced page's Clear, under way.
    clearing: Option<Receiver<reprisefs::thumbnails::Cleared>>,
    requests: Option<instance::Requests>,
    waker: Option<Waker>,
    /// Set by the `configwatch` thread when `config.toml` changed on disk.
    config_touched: Arc<AtomicBool>,
    scan: Option<Receiver<Vec<PathBuf>>>,
    /// The slideshow folder and its pictures, listed for the monitor.
    listed: Option<(PathBuf, Vec<PathBuf>)>,
    listing: Option<(PathBuf, Receiver<Vec<PathBuf>>)>,
    /// Follows the folder last listed; sets `slides_touched` when a file in it changes.
    folder_watch: Option<configwatch::FolderWatch>,
    slides_touched: Arc<AtomicBool>,
    /// The slideshow folder picked before Picture was chosen; kept in the state file.
    last_slideshow: String,
    decoding: Option<(String, Receiver<Option<Decoded>>)>,
    state_path: Option<PathBuf>,
    /// The size the state file kept at start, which the window opens at for each text size.
    kept: winstate::Kept,
    window: (i32, i32),
    window_dirty: bool,
    units: TextUnits,
    /// `[accessibility] text_size` now, kept beside the window size.
    text_size: u32,
    /// The control a page request named, and its page, until [`SettingsApp::take_pending_focus`]
    /// gives it the keyboard; a key, a button press or another page drops it.
    pending_focus: Option<(Page, String)>,
}

impl SettingsApp {
    fn new(
        page: Option<(Page, Option<String>)>,
        requests: Option<instance::Requests>,
    ) -> SettingsApp {
        let system = pages::system_layout(std::env::var("XKB_DEFAULT_LAYOUT").ok().as_deref());
        let mut app = SettingsApp::open(repriseconf::config_path(), system);
        if let Some((p, focus)) = page {
            app.open_page(p, focus.as_deref());
        }
        app.requests = requests;
        app.text_size = repriseconf::text_size();
        app.units = Theme::default().with_form_text(app.text_size).text;
        let state = winstate::path();
        app.kept = winstate::kept(&state);
        app.window = app.initial_size();
        app.last_slideshow = winstate::slideshow(&state);
        app.custom_colors = winstate::custom_colors(&state);
        app.state_path = Some(state);
        app
    }

    /// The area the pages fill at the sheet's size now.
    fn area(&self) -> Area {
        Area::of(&self.sheet)
    }

    /// Lay the sheet out for a main window of client `size`: the sheet
    /// takes it (no smaller than its smallest), every page takes the
    /// layout its builder gives it there, and the Cursor preview is
    /// drawn again at its new size. What the controls hold stays. A window
    /// smaller than the smallest sheet shows it scrolled. True when
    /// anything moved.
    fn fit_window(&mut self, size: (i32, i32)) -> bool {
        let (u, min) = (self.units, Area::MIN.sheet_size());
        let shown = (u.within(size.0), u.within(size.1));
        let size = (shown.0.max(min.0), shown.1.max(min.1));
        if size == self.sheet.size {
            if !self.sheet.fit(shown) {
                return false;
            }
            // A shorter window shortens the page list too.
            self.sheet.reveal_open();
            return true;
        }
        self.sheet.resize(size);
        let a = self.area();
        let offset = self.sheet.page_offset();
        let src = registry::Sources {
            icon_themes: &[],
            gtk_themes: &[],
            qt_tools: false,
            cursor_themes: &[],
            sound_themes: &[],
            layouts: &[],
            system_layout: &self.system_layout,
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
                Page::RemoteDesktop => {
                    pages::remote_desktop(a, None, false, self.remote.manager.then_some(""))
                }
                _ => registry::build(p, &src, a),
            };
            self.sheet.pages[p.index()].take_layout(&template, offset);
        }
        self.preview_for = None;
        self.sheet.fit(shown);
        self.sheet.reveal_open();
        true
    }

    /// Take the text size the forms draw at, and lay the sheet out at it before the next draw.
    fn follow_text(&mut self, ctx: &Ctx) {
        let forms = ctx.forms();
        self.units = forms.text;
        self.text_size = ctx.text_size;
        self.sheet.drawn_with(&Paint {
            s: 1,
            ..forms.paint()
        });
    }

    /// The smallest main window: the smallest sheet grown with the text.
    fn min_window(&self) -> (i32, i32) {
        self.units.size(Area::MIN.sheet_size())
    }

    /// A dialog's forms theme on `base`: the dialog font at the text size.
    fn dialog_theme(&self, base: &Theme) -> Theme {
        let mut t = base.clone();
        t.text = self.units;
        t.fonts.size_px = self.units.px(self.dialog_px);
        t
    }

    fn save_window(&mut self) {
        if !self.window_dirty {
            return;
        }
        self.window_dirty = false;
        if let Some(path) = &self.state_path {
            winstate::save(path, self.window, self.text_size);
        }
    }

    /// The app on the config at `path`; `system_layout` is what an empty `[keyboard] layout` gives.
    fn open(path: PathBuf, system_layout: String) -> SettingsApp {
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
                sound_themes: &repriseaudio::sounds::themes(&repriseaudio::sounds::sound_dirs()),
                layouts: &xkb,
                system_layout: &system_layout,
                heads: &[],
            },
        );
        let wallpapers: Vec<PathBuf> = wallpapers::expand(&loaded.wallpaper)
            .filter(|_| backgroundpage::builtin(&loaded.wallpaper).is_none())
            .into_iter()
            .collect();
        backgroundpage::fill(
            &mut sheet.pages[Page::Background.index()],
            &wallpapers,
            &loaded.wallpaper,
            None,
            &[],
        );
        let dialog_text = repriseapp::start_text();
        let dialog_px = Theme::default().fonts.size_px;
        let mut app = SettingsApp {
            path,
            want: loaded.clone(),
            loaded,
            sheet,
            search_hits: Vec::new(),
            entries: Vec::new(),
            applied_checks: Vec::new(),
            desktops: reprisefs::autostart::desktop_names(
                std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
            ),
            list: pages::new_list(),
            list_fit: None,
            list_focus: false,
            shortcuts: pages::new_shortcut_list(),
            shortcuts_focus: false,
            shortcut: None,
            color_dlg: None,
            custom_colors: [reprisekit::theme::Color::rgb(0xFF, 0xFF, 0xFF);
                reprisekit::controls::colordlg::CUSTOM],
            shortcut_target: None,
            app_names: HashMap::new(),
            app_dirs: linkhost::dirs(),
            links: Vec::new(),
            defaults: None,
            pads: Vec::new(),
            pad_picked: 0,
            pads_fresh: false,
            notify_programs: Vec::new(),
            notify_seen: Vec::new(),
            notify_names: HashMap::new(),
            notify_fresh: false,
            remote: remotehost::Remote::default(),
            remote_job: None,
            remote_acting: false,
            login: None,
            login_files: loginhost::Files::default(),
            login_source: loginhost::machine_users,
            login_run: loginhost::pkexec,
            login_users: None,
            login_job: None,
            login_poll: None,
            absent: absent_pages(),
            types: defaultspage::new_list(),
            types_focus: false,
            chooser: None,
            locations: pages::new_location_list(),
            locations_focus: false,
            location_answers: Vec::new(),
            location_icons: Vec::new(),
            location_icons_due: false,
            location_file: repriseconf::location::path(),
            locations_due: true,
            locating: None,
            xkb,
            system_layout,
            add_layout: None,
            add_item: None,
            rename: None,
            saver_dlg: None,
            saver_preview: None,
            saver_ticking: false,
            away_devices: Vec::new(),
            away_fresh: false,
            saver_photos: None,
            saver_photo: None,
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
            asking: None,
            dialog_text,
            dialog_px,
            err_icon: None,
            open_error,
            preview_for: None,
            display: displayhost::DisplayHost {
                layouts: layouts_file(),
                ..Default::default()
            },
            wallpapers,
            picture: None,
            wallpaper_icons: None,
            folder_icon: None,
            thumbs: Vec::new(),
            thumbing: None,
            clearing: None,
            requests: None,
            waker: None,
            config_touched: Arc::default(),
            scan: None,
            listed: None,
            listing: None,
            folder_watch: None,
            slides_touched: Arc::default(),
            last_slideshow: String::new(),
            decoding: None,
            state_path: None,
            kept: winstate::Kept {
                size: Area::MIN.sheet_size(),
                text_size: None,
            },
            window: Area::MIN.sheet_size(),
            window_dirty: false,
            units: TextUnits::ONE,
            text_size: 100,
            pending_focus: None,
        };
        app.display
            .model
            .set_primary_saved(outputwrite::read_primary(&text));
        app.rescan();
        app.app_names = app_names(&app.want);
        app.fill_shortcuts();
        app.sync_restore();
        app.quick_reload(None);
        app.show_monitor();
        app.find_links();
        app.show_links();
        app.search("");
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
        self.sheet.pages[Page::Cursor.index()].set_picture(pages::CURSOR_PREVIEW, pic);
        self.preview_for = Some(key);
    }

    fn edit_file(&self) -> Reply {
        let apps = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
        let globs = reprisefs::apps::mime_globs();
        let open = reprisefs::props::open_in(&self.path, &globs, &apps, &self.want.terminal);
        Reply {
            redraw: true,
            spawn: vec![open.argv],
            spawn_app: open.app,
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
}

impl App for SettingsApp {
    fn title(&self) -> String {
        "Reprise Settings".into()
    }

    fn app_id(&self) -> String {
        "reprise.settings".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.kept.at(self.text_size, self.min_window())
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
                    Pick::Program => "Browse",
                    Pick::Export => "Export",
                    Pick::Import => "Import",
                }
                .into(),
                size: self.add.as_ref().map_or(filedlg::CLIENT, |d| d.client()),
                fixed: false,
                modal: true,
            });
        }
        if let Some((d, ..)) = &self.color_dlg {
            kids.push(d.child(COLOR_WIN, self.units));
        }
        if let Some(d) = &self.shortcut {
            kids.push(Child {
                id: SHORTCUT_WIN,
                title: d.title.into(),
                size: self.units.size(shortcutdlg::CLIENT),
                fixed: true,
                modal: true,
            });
        }
        if self.add_layout.is_some() {
            kids.push(Child {
                id: LAYOUT_WIN,
                title: "Add Layout".into(),
                size: self.units.size(layoutdlg::CLIENT),
                fixed: true,
                modal: true,
            });
        }
        if self.add_item.is_some() {
            kids.push(Child {
                id: ADD_ITEM_WIN,
                title: "Add Item".into(),
                size: self.units.size(additem::CLIENT),
                fixed: true,
                modal: true,
            });
        }
        if self.rename.is_some() {
            kids.push(Child {
                id: RENAME_WIN,
                title: "Rename".into(),
                size: self.units.size(renamedlg::CLIENT),
                fixed: true,
                modal: true,
            });
        }
        if let Some(d) = &self.saver_dlg {
            kids.push(Child {
                id: SAVER_WIN,
                title: d.title(),
                size: self.units.size(d.client),
                fixed: true,
                modal: true,
            });
        }
        if self.display.flow.question.is_some() {
            kids.push(Child {
                id: displayhost::MONITOR_WIN,
                title: "Monitor Settings".into(),
                size: self.units.size(monitordlg::CLIENT),
                fixed: true,
                modal: true,
            });
        }
        if self.chooser.is_some() {
            kids.push(Child {
                id: CHOOSER_WIN,
                title: openwith::TITLE.into(),
                size: self.units.size(openwith::CLIENT),
                fixed: true,
                modal: true,
            });
        }
        if let Some(a) = &self.asking {
            kids.push(a.msg.child(
                ASK_WIN,
                &self.dialog_text,
                &self.dialog_theme(&Theme::default()),
            ));
        }
        if let Some(b) = &self.error {
            kids.push(b.child(
                ERROR_WIN,
                &self.dialog_text,
                &self.dialog_theme(&Theme::default()),
            ));
        }
        kids
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.dialog_text = ctx.text.clone();
        self.controls = ctx.theme.look.controls();
        self.follow_text(ctx);
        let main = win == repriseapp::MAIN;
        let moved = main && self.fit_window(ctx.size);
        match ev {
            // A maximized window's size is the screen's and one at its pick the program's
            // (held inside the work area or not): neither is one to open at next time.
            Event::Resized if main && !ctx.maximized && !ctx.at_pick && ctx.size != self.window => {
                self.window = ctx.size;
                self.window_dirty = true;
            }
            Event::Focus(false) if main => self.save_window(),
            Event::Key(_) | Event::Pointer(PointerInput::Press { .. }) => self.pending_focus = None,
            _ => {}
        }
        let focused = main && self.take_pending_focus();
        let mut reply = self.route(win, ev, ctx);
        reply.redraw |= moved || focused;
        if let Some(press) = self.display.pending.take() {
            let r = self.start_display(press, &mut ctx.heads);
            reply.quit |= r.quit;
            reply.redraw_all |= r.redraw_all;
        }
        self.ensure_defaults();
        self.ensure_locations();
        self.ensure_remote();
        self.ensure_login();
        if win == repriseapp::MAIN
            && !self.saver_ticking
            && self.sheet.tabs.open == Page::ScreenSaverLock.index()
        {
            self.saver_ticking = true;
            reply.tick = true;
        }
        reply.redraw |= self.ensure_pads();
        reply.redraw |= self.ensure_notify_programs();
        reply.redraw |= self.ensure_away_devices();
        self.prime_icons(ctx);
        if reply.quit
            && let Some(s) = &mut self.login_job
        {
            reply.quit = false;
            s.quit = true;
        }
        if reply.quit {
            self.save_window();
        }
        reply
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.dialog_text = ctx.text.clone();
        self.follow_text(ctx);
        if win == repriseapp::MAIN {
            self.fit_window(ctx.size);
            self.take_pending_focus();
        }
        self.paint(win, c, ctx);
    }

    fn start(&mut self, waker: Waker) -> repriseapp::Start {
        if let Some(r) = &self.requests {
            let w = waker.clone();
            r.wake_with(move || w.wake());
            // For a request that came before the loop ran.
            waker.wake();
        }
        let w = waker.clone();
        configwatch::spawn(&self.path, self.config_touched.clone(), move || w.wake());
        let w = waker.clone();
        match configwatch::FolderWatch::spawn(self.slides_touched.clone(), move || w.wake()) {
            Ok(watch) => self.folder_watch = Some(watch),
            Err(e) => log::warn!("the slideshow folder is not watched: {e}"),
        }
        self.waker = Some(waker);
        let had = self.wallpapers.clone();
        self.scan = Some(
            self.spawn(move || wallpapers::merge(wallpapers::scan(&wallpapers::dirs()), &had)),
        );
        self.want_preview();
        repriseapp::Start::Continue
    }
}

impl repriseapp::Extended for SettingsApp {
    /// Smallest sizes: the sheet's pages at their smallest, the file dialog's opening size.
    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        match &self.add {
            _ if win == repriseapp::MAIN => Some(self.min_window()),
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

    fn popup(&mut self, win: WinId, ctx: &Ctx) -> Option<Box<dyn Hosted + '_>> {
        if win == LAYOUT_WIN {
            return Some(Box::new(&mut self.add_layout.as_mut()?.form));
        }
        repriseapp::reader_popup(self, win, ctx)
    }

    fn reader_form(&mut self, win: WinId) -> Option<repriseapp::ReaderForm<'_>> {
        if win == repriseapp::MAIN {
            return Some(repriseapp::ReaderForm::Sheet(&mut self.sheet));
        }
        self.drop_form(win).map(repriseapp::ReaderForm::Form)
    }

    /// A control a reader focuses on the page takes the keyboard from a side list too.
    fn reader(&mut self, win: WinId, act: repriseapp::Reader, ctx: &Ctx) -> bool {
        let changed = repriseapp::reader_forms(self, win, act, ctx);
        if changed && win == repriseapp::MAIN && self.sheet.open().focus().is_some() {
            self.blur_side_lists();
        }
        changed
    }

    fn drop_file_dialog(&mut self, win: WinId) -> Option<FileDrop<'_>> {
        (win == ADD_WIN).then_some(FileDrop {
            dlg: self.add.as_mut()?,
            host: &mut self.add_host,
        })
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        self.describe(win, tree, ctx);
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(args: Vec<std::ffi::OsString>) {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    if let Some(verb) = backup_arg(&args) {
        std::process::exit(run_backup(verb));
    }
    let page = match page_arg(args.into_iter()) {
        Some((Entry::Program(id), _)) => {
            if linkhost::start(id) {
                return;
            }
            let label = repriseconf::pages::program(id).map_or(id, |p| p.label);
            log::warn!("--page: {label} is not installed");
            None
        }
        Some((Entry::Page(p), focus)) => Some((p, focus)),
        None => None,
    };
    let action = page.as_ref().map(|(p, focus)| {
        let params = std::iter::once(p.slug()).chain(focus.clone()).collect();
        (PAGE_ACTION.to_string(), params)
    });
    let requests = match instance::claim(BUS_NAME, action) {
        Claim::First(r) => Some(r),
        Claim::Handed => return,
        Claim::Alone => None,
    };
    repriseapp::run(Box::new(SettingsApp::new(page, requests))).unwrap_or_else(|e| e.exit());
}

/// The pages whose program this machine lacks; none under test.
fn absent_pages() -> Vec<Page> {
    if cfg!(test) {
        return Vec::new();
    }
    Page::all().filter(|p| !p.installed()).collect()
}

/// The compositor's saved layouts; in tests a file of the test process's own.
fn layouts_file() -> std::path::PathBuf {
    #[cfg(test)]
    return std::env::temp_dir().join(format!(
        "reprise-settings-displays-{}.toml",
        std::process::id()
    ));
    #[cfg(not(test))]
    repriseconf::displays::path()
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "login_tests.rs"]
mod login_tests;
