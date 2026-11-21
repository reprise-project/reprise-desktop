//! Background page: a monitor showing the pending wallpaper over the desktop
//! colour, one picture or a slideshow, their position, the colour, the
//! slideshow interval, the desktop icons and widgets, and desktop water. The
//! host fills the list (scan and thumbnails run on threads) and hands in the
//! decoded or drawn picture; this module lays the page out and composes the
//! monitor's screen.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use reprisekit::classic::assets::MONITOR_SCREEN;
use reprisekit::classic::widgets::BUTTON_W;
use reprisekit::controls::colorbutton::ColorButton;
use reprisekit::controls::combo::ListItem;
use reprisekit::controls::form::{Control, Form, Item, Screen};
use reprisekit::controls::listbox::ListBox;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::schemes;
use reprisekit::theme::Color;
use reprisekit::wallpaper::{Position, compose};
use reprisekit::wallpapers::{self, Builtin, Design, Variant};

use crate::model::{
    DESKTOP_EXTENSIONS, POSITIONS, SLIDE_INTERVALS, Settings, WALLPAPER_MODES, WATER_STYLES,
};
use crate::pages::{Area, GX, WATER, WATER_STYLE, X, button, choice};
use crate::wallpapers::expand;

pub const MONITOR: &str = "monitor";
pub const MODE: &str = "wallpaper_mode";
pub const WALLPAPER: &str = "wallpaper";
pub const BROWSE: &str = "browse";
/// What a pick is for: every desktop and monitor, or one desktop or monitor with a picture of
/// its own.
pub const USE_FOR: &str = "wallpaper_for";
const USE_FOR_LABEL: &str = "use_for_label";
pub const ALL_DESKTOPS: &str = "All desktops";
pub const ALL_MONITORS: &str = "All monitors";
/// The first Use for row with both desktops and monitors listed.
pub const EVERYWHERE: &str = "Everywhere";
pub const POSITION: &str = "wallpaper_position";
/// The file name extensions combo and its label, greyed while the desktop shows no files.
pub const EXTENSIONS: &str = "desktop_extensions";
const EXTENSIONS_LABEL: &str = "extensions_label";
/// The slideshow row's top at [`Area::MIN`], 10 px under the Desktop icons group.
const SLIDE_Y: i32 = 392;
/// The slideshow's interval combo and Shuffle check, greyed for one picture.
pub const INTERVAL: &str = "slideshow_interval";
const SLIDE_CONTROLS: [&str; 3] = ["interval_label", INTERVAL, "slideshow_shuffle"];
const WATER_NOTE: &str = "water_note";
pub const NONE_ROW: &str = "(None)";
pub const MONITOR_RECT: Rect = Rect::new(X, 44, 191, 168);
/// The right column, beside the monitor; 159 px wide at [`Area::MIN`].
const RX: i32 = 219;
fn rw(a: Area) -> i32 {
    a.right() - RX
}
/// The list, which takes the right column's width and the page's extra height.
pub fn list_rect(a: Area) -> Rect {
    Rect::new(RX, 66, rw(a), 123 + a.dh)
}
/// Rows the list shows at [`Area::MIN`]: 7 of 17 px inside its 2 px frame.
/// A picked row is scrolled to within these.
const LIST_ROWS: usize = 7;
/// Width of the screen the monitor stands for; sets how small a centred or
/// tiled picture shows on it.
const SCREEN_W: f32 = 1920.0;
/// Largest decode for the monitor: four times its screen, so a centred or
/// tiled picture up to 7680 px wide keeps every pixel shown.
pub const DECODE_MAX: (u32, u32) = (608, 448);

/// A decoded wallpaper for the monitor. `canvas` is None when the file
/// did not decode; `source_w` is the file's own width, which `canvas`
/// may have been shrunk from.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub path: String,
    pub canvas: Option<Canvas>,
    pub source_w: u32,
}

/// The built-in wallpapers the list offers after "(None)": every design
/// in each named variant, and the hue-driven ones also in the glass
/// colour.
pub fn builtins() -> Vec<Builtin> {
    Design::ALL
        .iter()
        .flat_map(|&d| {
            let named = d
                .variants()
                .iter()
                .filter_map(move |v| Builtin::named(d, v));
            let glass = d.hue_driven().then_some(Builtin {
                design: d,
                variant: Variant::Glass,
            });
            named.chain(glass)
        })
        .collect()
}

pub fn builtin_label(b: Builtin) -> String {
    let name = b.design.name();
    let cap = name[..1].to_uppercase() + &name[1..];
    match b.variant {
        Variant::Named(v) => format!("{cap} ({v})"),
        Variant::Glass => format!("{cap} (glass colour)"),
        Variant::Hue { hue, dark } => {
            format!("{cap} (hue {hue}{})", if dark { ", dark" } else { "" })
        }
    }
}

pub const BUILTIN_DETAIL: &str = "Reprise";

/// The built-in `[wallpaper] image = s` names, if it names one.
pub fn builtin(s: &str) -> Option<Builtin> {
    Builtin::from_uri(s)?.ok()
}

/// `b` in `glass`'s colour drawn at `w` x `h` through the drawn-picture
/// cache: a list thumbnail or the monitor's screen.
pub fn draw_builtin(b: Builtin, glass: &reprisekit::glass::Settings, w: u32, h: u32) -> Canvas {
    let b = reprisekit::interfaces::resolve_wallpaper(b, glass);
    let name = wallpapers::cache_name(b, w, h, wallpapers::SEED);
    let dir = reprisefs::drawncache::default_dir();
    let px = reprisefs::drawncache::get_or_make(dir.as_deref(), &name, w, h, || {
        wallpapers::render(b, w, h, wallpapers::SEED).pixels
    });
    Canvas::from_pixels(w, h, px)
}

/// `b`'s row icon at `scale`: the picture drawn into the list's `px` square.
pub fn thumb(b: Builtin, glass: &reprisekit::glass::Settings, px: i32, scale: i32) -> Canvas {
    let n = (px * scale.max(1)) as u32;
    let mut t = Canvas::with_scale(px as u32, px as u32, scale);
    t.pixels = draw_builtin(b, glass, n, n).pixels;
    t
}

pub fn label(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The page with an empty list and a black monitor; the host fills the
/// list ([`fill`] or [`fill_folders`]) and the screen ([`show`]). The
/// list takes the page's extra height; the controls under it in the
/// right column, the slideshow row and the water row move down, and the
/// Desktop icons group stays under the monitor.
pub fn page(a: Area) -> Form {
    let right = a.right() - BUTTON_W;
    let dh = a.dh;
    let intervals: Vec<&str> = SLIDE_INTERVALS.iter().map(|c| c.1).collect();
    Form::new(vec![
        Item::new(MONITOR, MONITOR_RECT, Control::Monitor(Screen::Black)),
        Item::label_at("wallpaper_label", RX, 44, 66, "&Background:"),
        choice(MODE, RX + 70, 40, rw(Area::MIN) - 70, WALLPAPER_MODES),
        Item::new(
            WALLPAPER,
            list_rect(a),
            Control::ListBox {
                list: ListBox::new(vec![NONE_ROW.into()], Some(0)),
                icons: vec![None],
            },
        ),
        button(BROWSE, right, 196 + dh, "B&rowse..."),
        Item::label_at("position_label", RX, 230 + dh, BUTTON_W, "&Position:"),
        choice(POSITION, RX, 246 + dh, BUTTON_W, POSITIONS),
        Item::label_at("color_label", right, 230 + dh, BUTTON_W, "&Color:"),
        Item::new(
            "background",
            Rect::new(right, 246 + dh, BUTTON_W, 21),
            Control::Color(ColorButton::new(
                Color::rgb(0, 0, 0),
                schemes::PALETTE.to_vec(),
            )),
        ),
        Item::label_at(USE_FOR_LABEL, RX, 276 + dh, 44, "Use &for:"),
        use_for(dh),
        Item::group(
            "widgets_group",
            Rect::new(RX, 300 + dh, rw(a), 82),
            "Desktop Widgets",
        ),
        Item::check("widget_clock", RX + 12, 322 + dh, "Cl&ock"),
        Item::check("widget_calendar", RX + 12, 344 + dh, "Ca&lendar"),
        Item::check("widget_meter", RX + 12, 366 + dh, "System me&ter"),
        Item::group("icons_group", Rect::new(X, 222, 191, 160), "Desktop icons"),
        Item::check("documents", GX, 244, "&Documents"),
        Item::check("computer", GX, 266, "Co&mputer"),
        Item::check("home", GX, 288, &home_check()),
        Item::check("desktop_icons", GX, 310, "&Show the files in ~/Desktop"),
        Item::label_at(EXTENSIONS_LABEL, GX, 334, 167, "File name e&xtensions:"),
        choice(EXTENSIONS, GX, 350, 167, DESKTOP_EXTENSIONS),
        Item::label_at(
            "interval_label",
            X,
            SLIDE_Y + 4 + dh,
            130,
            "Change picture &every:",
        ),
        Item::combo(
            INTERVAL,
            X + 134,
            SLIDE_Y + dh,
            120,
            &intervals,
            intervals.len(),
        ),
        Item::check("slideshow_shuffle", X + 264, SLIDE_Y + 4 + dh, "Sh&uffle"),
        Item::check(WATER, X, SLIDE_Y + 32 + dh, "Desktop &water:"),
        choice(WATER_STYLE, X + 112, SLIDE_Y + 28 + dh, 80, WATER_STYLES).named("Water style"),
        Item::label_at(
            WATER_NOTE,
            X + 198,
            SLIDE_Y + 32 + dh,
            a.w - 198,
            "Drag on the desktop to stir.",
        ),
    ])
}

/// The Use for combo, whose monitor rows are named by their make and model, any length.
fn use_for(dh: i32) -> Item {
    let mut item = Item::combo(
        USE_FOR,
        RX + 48,
        272 + dh,
        rw(Area::MIN) - 48,
        &[ALL_DESKTOPS],
        1,
    );
    if let Control::Combo(c) = &mut item.control {
        c.elide = true;
    }
    item
}

pub fn slideshow(s: &Settings) -> bool {
    !s.slideshow.is_empty()
}

/// A connected monitor the Use for list offers: its connector name and its Display page label.
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub label: String,
}

impl Monitor {
    /// The end of `label` a row cut to an ellipsis keeps: " on " and the connector, or the
    /// connector alone.
    fn keep(&self) -> usize {
        let Some(head) = self.label.strip_suffix(&self.name) else {
            return 0;
        };
        self.name.len() + if head.ends_with(" on ") { 4 } else { 0 }
    }
}

/// What a pick on the page is for.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    All,
    Desktop(usize),
    /// A monitor by connector name.
    Monitor(String),
}

fn desktop_rows(s: &Settings) -> usize {
    if s.desktops > 1 {
        s.desktops as usize
    } else {
        0
    }
}

/// What the Use for combo picks: its rows are [`use_for_rows`] of `s` and `monitors`.
pub fn target(form: &Form, s: &Settings, monitors: &[Monitor]) -> Target {
    let Some(i) = form.combo_selected(USE_FOR).and_then(|i| i.checked_sub(1)) else {
        return Target::All;
    };
    let n = desktop_rows(s);
    if i < n {
        return Target::Desktop(i);
    }
    (monitors.get(i - n)).map_or(Target::All, |m| Target::Monitor(m.name.clone()))
}

/// The Use for combo's rows: everywhere, then each of the `s.desktops` by name, then each of
/// `monitors` (the host lists them while two or more are connected).
pub fn use_for_rows(s: &Settings, monitors: &[Monitor]) -> Vec<String> {
    let n = desktop_rows(s);
    let first = match (n > 0, monitors.is_empty()) {
        (true, false) => EVERYWHERE,
        (false, false) => ALL_MONITORS,
        _ => ALL_DESKTOPS,
    };
    std::iter::once(first.to_string())
        .chain((0..n).map(|i| repriseconf::config::desktop_name(&s.desktop_names, i)))
        .chain(monitors.iter().map(|m| m.label.clone()))
        .collect()
}

/// `s` as the page shows it for `target`: that desktop's picture (its own, else the one for all),
/// or that monitor's picture and position (its own, else the ones for all); never a slideshow.
pub fn view<'a>(s: &'a Settings, target: &Target) -> Cow<'a, Settings> {
    match target {
        Target::All => Cow::Borrowed(s),
        Target::Desktop(n) => {
            let own = s.wallpaper_desktops.get(*n).filter(|p| !p.is_empty());
            Cow::Owned(Settings {
                wallpaper: own.unwrap_or(&s.wallpaper).clone(),
                slideshow: String::new(),
                ..s.clone()
            })
        }
        Target::Monitor(name) => {
            let own = s.output_wallpapers.iter().find(|o| o.name == *name);
            let or = |v: Option<&String>, all: &String| {
                v.filter(|v| !v.is_empty()).unwrap_or(all).clone()
            };
            Cow::Owned(Settings {
                wallpaper: or(own.map(|o| &o.wallpaper), &s.wallpaper),
                wallpaper_position: or(own.map(|o| &o.position), &s.wallpaper_position),
                slideshow: String::new(),
                ..s.clone()
            })
        }
    }
}

/// Whether `picked` is the picture for all: none, or `s`'s own picture while no slideshow runs.
fn is_shared(s: &Settings, picked: &str) -> bool {
    picked.is_empty()
        || (!slideshow(s)
            && (picked == s.wallpaper
                || (builtin(picked).is_none() && expand(picked) == expand(&s.wallpaper))))
}

/// Give desktop `n` the picture `picked`; the picture for all desktops (or none) leaves it
/// without one of its own. Trailing empty entries are dropped.
pub fn set_for_desktop(s: &mut Settings, n: usize, picked: String) {
    let shared = is_shared(s, &picked);
    let list = &mut s.wallpaper_desktops;
    if list.len() <= n {
        list.resize(n + 1, String::new());
    }
    list[n] = if shared { String::new() } else { picked };
    while list.last().is_some_and(String::is_empty) {
        list.pop();
    }
}

/// Which of a monitor's own keys a pick sets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OwnKey {
    Wallpaper,
    Position,
}

impl OwnKey {
    /// The `[[outputs]]` key it stands for.
    pub fn key(self) -> &'static str {
        match self {
            OwnKey::Wallpaper => "wallpaper",
            OwnKey::Position => "wallpaper_position",
        }
    }

    /// `o`'s value for it; "" for none.
    pub fn of(self, o: &crate::model::OutputWallpaper) -> &str {
        match self {
            OwnKey::Wallpaper => &o.wallpaper,
            OwnKey::Position => &o.position,
        }
    }
}

/// Give monitor `name` its own `value` for `key`; the value for all monitors (or none) leaves it
/// without one. An entry left with neither is dropped.
pub fn set_for_monitor(s: &mut Settings, name: &str, key: OwnKey, value: String) {
    let shared = match key {
        OwnKey::Wallpaper => is_shared(s, &value),
        OwnKey::Position => value.is_empty() || value == s.wallpaper_position,
    };
    let list = &mut s.output_wallpapers;
    let i = (list.iter().position(|o| o.name == name)).unwrap_or_else(|| {
        list.push(crate::model::OutputWallpaper {
            name: name.to_string(),
            ..Default::default()
        });
        list.len() - 1
    });
    let field = match key {
        OwnKey::Wallpaper => &mut list[i].wallpaper,
        OwnKey::Position => &mut list[i].position,
    };
    *field = if shared { String::new() } else { value };
    list.retain(|o| !o.wallpaper.is_empty() || !o.position.is_empty());
}

/// A pick for all monitors: each of `connected` (by connector name) shows it, so their own
/// `key` goes. A monitor not connected keeps its own.
pub fn clear_for_monitors(s: &mut Settings, connected: &[String], key: OwnKey) {
    for name in connected {
        set_for_monitor(s, name, key, String::new());
    }
}

/// The folders the slideshow list offers: each folder holding one of
/// `pictures`, and `extra` (the pending folder, which may hold none the
/// scan found), sorted by row name.
pub fn folders(pictures: &[PathBuf], extra: Option<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = pictures
        .iter()
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .chain(extra)
        .collect();
    out.sort_by_key(|p| (label_folder(p).to_lowercase(), p.clone()));
    out.dedup();
    out
}

pub fn label_folder(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Fill the list with `folders` for a slideshow, `picked` (the config's
/// spelling) selected and scrolled into view. `icon` is a folder's.
pub fn fill_folders(form: &mut Form, folders: &[PathBuf], picked: &str, icon: Option<&Canvas>) {
    let at = expand(picked).and_then(|p| folders.iter().position(|q| *q == p));
    if let Some(Control::ListBox { list, icons }) = form.item_mut(WALLPAPER).map(|i| &mut i.control)
    {
        *list = ListBox::new(folders.iter().map(|p| label_folder(p)).collect(), None);
        if let Some(at) = at {
            list.select(at, LIST_ROWS);
        }
        *icons = vec![icon.cloned(); folders.len()];
    }
}

/// The file the monitor shows for `s`: the pending wallpaper, or for a
/// slideshow the first picture in `listed` (its folder's
/// `reprisefs::pictures::in_folder`, the desktop's first slide); empty
/// for none. A built-in is its `reprise:` spelling in `s`'s glass colour,
/// so a new glass colour is a new picture.
pub fn monitor_file(s: &Settings, listed: &[PathBuf]) -> String {
    if !slideshow(s) {
        let glass = crate::appearancepage::glass(s);
        return match builtin(&s.wallpaper) {
            Some(b) => reprisekit::interfaces::resolve_wallpaper(b, &glass).uri(),
            None => s.wallpaper.clone(),
        };
    }
    let Some(folder) = expand(&s.slideshow) else {
        return String::new();
    };
    listed
        .iter()
        .find(|p| p.parent() == Some(folder.as_path()))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The folder a slideshow starts on, as the config spells it: `last`,
/// the one picked before, else the pending `wallpaper`'s folder; `None`
/// when neither is a folder on disk.
pub fn slideshow_start(last: &str, wallpaper: &str) -> Option<String> {
    if expand(last).is_some_and(|p| p.is_dir()) {
        return Some(last.to_string());
    }
    if builtin(wallpaper).is_some() {
        return None;
    }
    let parent = expand(wallpaper)?.parent()?.to_path_buf();
    parent
        .is_dir()
        .then(|| parent.to_string_lossy().into_owned())
}

/// The Home icon's check: its desktop label ("<user>'s Home"), H its
/// access key.
fn home_check() -> String {
    let label = reprisekit::text::escape_mnemonic(&reprisefs::places::home_label());
    match label.strip_suffix("Home") {
        Some(owner) => format!("{owner}&Home"),
        None => label,
    }
}

/// Fill the list: "(None)", the [`builtins`] by [`builtin_label`], then
/// each of `paths` by its [`label`], with `picked` (the config's value: a
/// path, `~/` opening at home, a `reprise:` built-in, or empty for none)
/// selected and scrolled into view. `icons` are the "(None)" row's and a
/// picture row's; `thumbs` the built-ins', parallel to [`builtins`] (none
/// while they are drawn).
pub fn fill(
    form: &mut Form,
    paths: &[PathBuf],
    picked: &str,
    icons: Option<&(Option<Canvas>, Option<Canvas>)>,
    thumbs: &[Option<Canvas>],
) {
    let built = builtins();
    let rows: Vec<String> = std::iter::once(NONE_ROW.to_string())
        .chain(built.iter().map(|b| builtin_label(*b)))
        .chain(paths.iter().map(|p| label(p)))
        .collect();
    let at = match builtin(picked) {
        Some(b) => built.iter().position(|x| *x == b).map_or(0, |i| i + 1),
        None => expand(picked)
            .and_then(|p| paths.iter().position(|q| *q == p))
            .map_or(0, |i| i + 1 + built.len()),
    };
    let (none, picture) = icons.cloned().unwrap_or_default();
    if let Some(Control::ListBox { list, icons }) = form.item_mut(WALLPAPER).map(|i| &mut i.control)
    {
        *list = ListBox::new(rows, None);
        list.detail = std::iter::once(String::new())
            .chain(std::iter::repeat_n(BUILTIN_DETAIL.to_string(), built.len()))
            .collect();
        list.select(at, LIST_ROWS);
        *icons = std::iter::once(none)
            .chain((0..built.len()).map(|i| thumbs.get(i).cloned().flatten()))
            .chain(std::iter::repeat_n(picture, paths.len()))
            .collect();
    }
}

/// Put `thumbs` on the built-ins' rows of a list [`fill`] filled,
/// leaving its selection and scroll as they are.
pub fn set_thumbs(form: &mut Form, thumbs: &[Option<Canvas>]) {
    if let Some(Control::ListBox { icons, .. }) = form.item_mut(WALLPAPER).map(|i| &mut i.control) {
        for (i, t) in thumbs.iter().enumerate().take(builtins().len()) {
            if let Some(slot) = icons.get_mut(i + 1) {
                *slot = t.clone();
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Row {
    None,
    Builtin(Builtin),
    Picture(usize),
}

/// What list row `i` stands for: "(None)", the [`builtins`], then the
/// host's pictures.
pub fn row(i: usize) -> Row {
    let built = builtins();
    match i.checked_sub(1) {
        None => Row::None,
        Some(k) if k < built.len() => Row::Builtin(built[k]),
        Some(k) => Row::Picture(k - built.len()),
    }
}

pub fn selected(form: &Form) -> Option<usize> {
    match form.item(WALLPAPER).map(|i| &i.control) {
        Some(Control::ListBox { list, .. }) => list.selected,
        _ => None,
    }
}

/// What the monitor shows for `s` with `want` the file it shows
/// ([`monitor_file`]). With none, or while its decode is still running,
/// or when it did not decode, that is the desktop colour alone (but see
/// [`show`]). Once
/// `picture` is `want`'s decode (the same file, however the config
/// spells it), the picture is composed over the colour the way the
/// desktop places it.
pub fn screen(s: &Settings, want: &str, picture: Option<&Picture>) -> Screen {
    let bg = crate::appearancepage::theme(s).colors.background;
    let (_, _, w, h) = MONITOR_SCREEN;
    match picture {
        Some(Picture {
            path,
            canvas: Some(c),
            source_w,
        }) if expand(want).is_some_and(|want| expand(path) == Some(want)) => {
            let position = match builtin(want) {
                Some(_) => Position::Stretch,
                None => Position::parse(&s.wallpaper_position).unwrap_or_default(),
            };
            let scale = w as f32 / SCREEN_W * *source_w as f32 / c.width.max(1) as f32;
            Screen::Picture(compose(c, w as u32, h as u32, position, bg, scale))
        }
        _ => Screen::Color(bg),
    }
}

/// The file to decode for the monitor next: the pending wallpaper `want`
/// (as the config spells it), unless it is none, a decode is `busy`, or
/// the picture last decoded, `shown` (spelled as it was then), is that
/// file already.
pub fn next_decode(want: &str, shown: Option<&str>, busy: bool) -> Option<PathBuf> {
    let file = expand(want)?;
    let on = shown.and_then(expand).is_some_and(|p| p == file);
    (!on && !busy).then_some(file)
}

/// Put [`screen`] on the monitor, except that while a slideshow folder is
/// listed or a picture decoded (`pending`) a picture already shown stays,
/// so a pick goes from one picture straight to the next.
pub fn show(form: &mut Form, s: &Settings, want: &str, picture: Option<&Picture>, pending: bool) {
    let want = screen(s, want, picture);
    if let Some(Control::Monitor(on)) = form.item_mut(MONITOR).map(|i| &mut i.control)
        && *on != want
        && !(pending && matches!(on, Screen::Picture(_)))
    {
        *on = want;
    }
}

/// Grey the file name extensions while the desktop shows no files, the water style while water is
/// off and the stirring note unless the style is a fluid, and show on the Color button the desktop
/// colour `s` gives while `[colors] background` is unset. The rows that follow what Use for picks
/// follow too while it picks all or a desktop; [`follow_use_for`] fills it and follows a monitor.
pub fn follow(form: &mut Form, s: &Settings) {
    let picked = form.combo_selected(USE_FOR).unwrap_or(0);
    match picked.checked_sub(1) {
        None => follow_target(form, s, &Target::All),
        Some(n) if n < desktop_rows(s) => follow_target(form, s, &Target::Desktop(n)),
        Some(_) => {}
    }
    form.set_enabled(EXTENSIONS_LABEL, s.desktop_icons);
    form.set_enabled(EXTENSIONS, s.desktop_icons);
    form.set_enabled(WATER_STYLE, s.wallpaper_water != "off");
    let fluid = reprisekit::water::Water::parse(&s.wallpaper_water).is_some_and(|w| w.is_fluid());
    form.set_enabled(WATER_NOTE, fluid);
    if s.background.is_empty()
        && let Some(Control::Color(b)) = form.item_mut("background").map(|i| &mut i.control)
    {
        b.color = crate::appearancepage::theme(s).colors.background;
    }
}

/// Fill the Use for combo from `s` and `monitors` (the row picked stays picked while listed), then
/// follow what it picks.
pub fn follow_use_for(form: &mut Form, s: &Settings, monitors: &[Monitor]) {
    let rows = use_for_rows(s, monitors);
    if let Some(Control::Combo(c)) = form.item_mut(USE_FOR).map(|i| &mut i.control)
        && !c.items.iter().map(|i| &i.text).eq(rows.iter())
    {
        let was = c.items.get(c.selected).map(|i| i.text.clone());
        let at = was.and_then(|w| rows.iter().position(|r| *r == w));
        let firsts = rows.len() - monitors.len();
        c.items = (rows.iter().enumerate())
            .map(|(i, r)| ListItem {
                keep: i.checked_sub(firsts).map_or(0, |m| monitors[m].keep()),
                ..ListItem::new(r.as_str())
            })
            .collect();
        c.rows = rows.len();
        c.selected = at.unwrap_or(0);
    }
    form.set_enabled(USE_FOR_LABEL, rows.len() > 1);
    form.set_enabled(USE_FOR, rows.len() > 1);
    let t = target(form, s, monitors);
    follow_target(form, s, &t);
}

/// Show for `t` Picture or Slideshow (only all of them takes a slideshow), the position, greyed
/// while there is no picture file, and the slideshow's controls, greyed without one.
fn follow_target(form: &mut Form, s: &Settings, t: &Target) {
    form.set_enabled(MODE, *t == Target::All);
    let s = &*view(s, t);
    let show = slideshow(s);
    form.set_combo(MODE, show as usize);
    let has = (!s.wallpaper.is_empty() && builtin(&s.wallpaper).is_none()) || show;
    form.set_enabled("position_label", has);
    form.set_enabled(POSITION, has);
    if let Some(i) = POSITIONS
        .iter()
        .position(|(v, _)| *v == s.wallpaper_position)
    {
        form.set_combo(POSITION, i);
    }
    for id in SLIDE_CONTROLS {
        form.set_enabled(id, show);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Desktop icons group's extensions row on Show, and greyed with the files off. The rows
    /// above are left out: the Home check box reads the user's name.
    #[test]
    fn the_desktop_icons_group_pins_its_pixels_in_every_look() {
        reprisekit::pins::fonts();
        use reprisekit::controls::form;
        use reprisekit::interface::Interface;
        fn fnv(c: &Canvas) -> u64 {
            c.pixels.iter().fold(0xcbf29ce484222325, |h, p| {
                (h ^ *p as u64).wrapping_mul(0x100000001b3)
            })
        }
        let row = Rect::new(X, 330, 191, 52);
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = reprisekit::theme::Theme {
                access_keys: true,
                ..reprisekit::interfaces::theme_for(i)
            };
            let text = reprisekit::text::TextRenderer::load(&theme.fonts);
            let mut hashes = [0; 2];
            for (k, files) in [true, false].into_iter().enumerate() {
                let mut f = page(Area::MIN);
                let s = Settings {
                    desktop_icons: files,
                    desktop_extensions: "show".into(),
                    ..Settings::default()
                };
                follow(&mut f, &s);
                follow_use_for(&mut f, &s, &[]);
                f.set_combo(EXTENSIONS, 2);
                let mut c = Canvas::new(row.right() as u32, row.bottom() as u32);
                c.fill_rect(c.bounds(), theme.colors.button_face);
                form::draw(
                    &mut c,
                    &text,
                    &theme.paint(),
                    &f,
                    false,
                    theme.fonts.size_px,
                );
                hashes[k] = fnv(&c.copy_rect(row));
            }
            assert_ne!(
                hashes[0], hashes[1],
                "{i:?}: the combo greys with the files off"
            );
            let states = ["shown", "greyed"];
            got.extend(
                states
                    .into_iter()
                    .zip(hashes)
                    .map(|(state, h)| (format!("{i:?} {state}"), h)),
            );
        }
        let file = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/pins/desktop_icons_group.txt"
        );
        reprisekit::pins::check_file(std::path::Path::new(file), &got);
    }

    #[test]
    fn a_desktop_shows_its_own_picture_else_the_one_for_all() {
        let mut s = Settings {
            desktops: 3,
            wallpaper: "~/w/a.jpg".into(),
            slideshow: "~/Slides".into(),
            wallpaper_desktops: vec!["".into(), "reprise:tarn/dusk".into()],
            desktop_names: vec!["Work".into()],
            ..Settings::default()
        };
        assert_eq!(view(&s, &Target::All).slideshow, "~/Slides");
        assert_eq!(view(&s, &Target::Desktop(0)).wallpaper, "~/w/a.jpg");
        assert_eq!(
            view(&s, &Target::Desktop(0)).slideshow,
            "",
            "a desktop shows one picture"
        );
        assert_eq!(view(&s, &Target::Desktop(1)).wallpaper, "reprise:tarn/dusk");
        assert_eq!(view(&s, &Target::Desktop(2)).wallpaper, "~/w/a.jpg");
        assert_eq!(
            use_for_rows(&s, &[]),
            [ALL_DESKTOPS, "Work", "Desktop 2", "Desktop 3"]
        );
        set_for_desktop(&mut s, 2, "/p/c.png".into());
        assert_eq!(s.wallpaper_desktops, ["", "reprise:tarn/dusk", "/p/c.png"]);
        set_for_desktop(&mut s, 2, String::new());
        assert_eq!(
            s.wallpaper_desktops,
            ["", "reprise:tarn/dusk"],
            "(None) drops it"
        );
        s.slideshow.clear();
        set_for_desktop(&mut s, 1, "~/w/a.jpg".into());
        assert!(
            s.wallpaper_desktops.is_empty(),
            "the picture for all drops it"
        );
        s.desktops = 1;
        assert_eq!(use_for_rows(&s, &[]), [ALL_DESKTOPS]);
    }

    #[test]
    fn a_monitor_shows_its_own_picture_and_position_else_the_ones_for_all() {
        let monitors = vec![
            Monitor {
                name: "DP-1".into(),
                label: "1. DP-1".into(),
            },
            Monitor {
                name: "HDMI-A-1".into(),
                label: "2. Dell U2412M on HDMI-A-1".into(),
            },
        ];
        let mut s = Settings {
            desktops: 2,
            wallpaper: "~/w/a.jpg".into(),
            wallpaper_position: "fill".into(),
            slideshow: "~/Slides".into(),
            ..Settings::default()
        };
        assert_eq!(
            use_for_rows(&s, &monitors),
            [
                EVERYWHERE,
                "Desktop 1",
                "Desktop 2",
                "1. DP-1",
                "2. Dell U2412M on HDMI-A-1"
            ]
        );
        let mut f = page(Area::MIN);
        follow_use_for(&mut f, &s, &monitors);
        f.set_combo(USE_FOR, 4);
        let hdmi = Target::Monitor("HDMI-A-1".into());
        assert_eq!(target(&f, &s, &monitors), hdmi);
        f.set_combo(USE_FOR, 2);
        assert_eq!(target(&f, &s, &monitors), Target::Desktop(1));
        assert_eq!(view(&s, &hdmi).wallpaper, "~/w/a.jpg");
        assert_eq!(view(&s, &hdmi).slideshow, "", "a monitor shows one picture");
        set_for_monitor(
            &mut s,
            "HDMI-A-1",
            OwnKey::Wallpaper,
            "reprise:tarn/dusk".into(),
        );
        set_for_monitor(&mut s, "HDMI-A-1", OwnKey::Position, "center".into());
        set_for_monitor(&mut s, "DP-1", OwnKey::Position, "fill".into());
        assert_eq!(
            s.output_wallpapers,
            [crate::model::OutputWallpaper {
                name: "HDMI-A-1".into(),
                wallpaper: "reprise:tarn/dusk".into(),
                position: "center".into(),
            }],
            "the position for all is no position of its own"
        );
        assert_eq!(view(&s, &hdmi).wallpaper, "reprise:tarn/dusk");
        assert_eq!(view(&s, &hdmi).wallpaper_position, "center");
        assert_eq!(
            view(&s, &Target::Monitor("DP-1".into())).wallpaper_position,
            "fill"
        );
        set_for_monitor(&mut s, "HDMI-A-1", OwnKey::Wallpaper, String::new());
        assert_eq!(s.output_wallpapers[0].wallpaper, "", "(None) drops it");
        clear_for_monitors(&mut s, &["HDMI-A-1".into()], OwnKey::Position);
        assert!(
            s.output_wallpapers.is_empty(),
            "an entry left with neither goes"
        );
        let one = Settings {
            desktops: 1,
            ..Settings::default()
        };
        assert_eq!(use_for_rows(&one, &monitors[..0]), [ALL_DESKTOPS]);
        assert_eq!(
            use_for_rows(&one, &monitors),
            [ALL_MONITORS, "1. DP-1", "2. Dell U2412M on HDMI-A-1"]
        );
    }

    #[test]
    fn all_monitors_keeps_a_disconnected_monitors_own_picture() {
        let mut s = Settings {
            output_wallpapers: ["DP-1", "DP-2"]
                .map(|n| crate::model::OutputWallpaper {
                    name: n.into(),
                    wallpaper: "/w/own.png".into(),
                    position: "tile".into(),
                })
                .to_vec(),
            ..Settings::default()
        };
        clear_for_monitors(&mut s, &["DP-1".into()], OwnKey::Wallpaper);
        let names: Vec<(&str, &str, &str)> = (s.output_wallpapers.iter())
            .map(|o| (o.name.as_str(), o.wallpaper.as_str(), o.position.as_str()))
            .collect();
        assert_eq!(
            names,
            [("DP-1", "", "tile"), ("DP-2", "/w/own.png", "tile")]
        );
    }

    #[test]
    fn a_monitor_picked_greys_the_mode_and_shows_its_position() {
        let monitors = vec![
            Monitor {
                name: "DP-1".into(),
                label: "1. DP-1".into(),
            },
            Monitor {
                name: "DP-2".into(),
                label: "2. DP-2".into(),
            },
        ];
        let s = Settings {
            wallpaper: "/w/a.jpg".into(),
            wallpaper_position: "fill".into(),
            output_wallpapers: vec![crate::model::OutputWallpaper {
                name: "DP-2".into(),
                wallpaper: String::new(),
                position: "tile".into(),
            }],
            desktops: 1,
            ..Settings::default()
        };
        let mut f = page(Area::MIN);
        follow_use_for(&mut f, &s, &monitors);
        f.set_combo(USE_FOR, 2);
        follow_use_for(&mut f, &s, &monitors);
        let combo = |f: &Form, id: &str| match &f.item(id).unwrap().control {
            Control::Combo(c) => (c.enabled, c.selected),
            _ => panic!("{id} is a combo"),
        };
        assert!(!combo(&f, MODE).0, "a monitor shows one picture");
        assert_eq!(combo(&f, POSITION), (true, 3), "DP-2 tiles");
        follow_use_for(&mut f, &s, &monitors[..1]);
        assert_eq!(
            f.combo_selected(USE_FOR),
            Some(0),
            "a monitor unplugged is no longer picked"
        );
        assert_eq!(combo(&f, POSITION), (true, 0));
    }

    /// The Use for row with all desktops picked, with a desktop picked, and greyed with one desktop.
    #[test]
    fn the_use_for_row_pins_its_pixels_in_every_look() {
        reprisekit::pins::fonts();
        use reprisekit::controls::form;
        use reprisekit::interface::Interface;
        fn fnv(c: &Canvas) -> u64 {
            c.pixels.iter().fold(0xcbf29ce484222325, |h, p| {
                (h ^ *p as u64).wrapping_mul(0x100000001b3)
            })
        }
        let row = Rect::new(RX, 270, rw(Area::MIN), 25);
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = reprisekit::theme::Theme {
                access_keys: true,
                ..reprisekit::interfaces::theme_for(i)
            };
            let text = reprisekit::text::TextRenderer::load(&theme.fonts);
            let mut hashes = [0; 3];
            for (k, (desktops, pick)) in [(4, 0), (4, 2), (1, 0)].into_iter().enumerate() {
                let mut f = page(Area::MIN);
                let s = Settings {
                    desktops,
                    ..Settings::default()
                };
                follow(&mut f, &s);
                follow_use_for(&mut f, &s, &[]);
                f.set_combo(USE_FOR, pick);
                let mut c = Canvas::new(row.right() as u32, row.bottom() as u32);
                c.fill_rect(c.bounds(), theme.colors.button_face);
                form::draw(
                    &mut c,
                    &text,
                    &theme.paint(),
                    &f,
                    false,
                    theme.fonts.size_px,
                );
                hashes[k] = fnv(&c.copy_rect(row));
            }
            assert!(
                hashes[0] != hashes[1] && hashes[0] != hashes[2],
                "{i:?}: a desktop's name and the greyed row differ from all desktops"
            );
            let states = ["all", "Desktop 2", "greyed"];
            got.extend(
                states
                    .into_iter()
                    .zip(hashes)
                    .map(|(state, h)| (format!("{i:?} {state}"), h)),
            );
        }
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/pins/use_for_row.txt");
        reprisekit::pins::check_file(std::path::Path::new(file), &got);
    }

    #[test]
    fn the_use_for_row_sits_between_the_colour_and_the_widgets() {
        let f = page(Area::MIN);
        let combo = f.item(USE_FOR).unwrap().rect;
        let colour = f.item("background").unwrap().rect;
        let widgets = f.item("widgets_group").unwrap().rect;
        assert_eq!(combo, Rect::new(RX + 48, 272, rw(Area::MIN) - 48, 21));
        assert!(combo.y > colour.bottom() && combo.bottom() < widgets.y);
        assert!(combo.right() <= Area::MIN.right());
    }

    #[test]
    fn the_list_names_each_picture_and_picks_the_pending_one() {
        let mut f = page(Area::MIN);
        let paths = [
            PathBuf::from("/w/Blue Hills.jpg"),
            PathBuf::from("/p/tile.PNG"),
        ];
        fill(&mut f, &paths, "/p/tile.PNG", None, &[]);
        let n = builtins().len();
        match &f.item(WALLPAPER).unwrap().control {
            Control::ListBox { list, icons } => {
                assert_eq!(list.items[0], "(None)");
                assert_eq!(list.items[n + 1..], ["Blue Hills", "tile"]);
                assert_eq!(list.selected, Some(n + 2));
                assert_eq!(icons.len(), n + 3);
            }
            _ => panic!("the wallpaper list is a list box"),
        }
        assert_eq!(selected(&f).map(row), Some(Row::Picture(1)));
        fill(&mut f, &paths, "", None, &[]);
        assert_eq!(selected(&f), Some(0), "no wallpaper is (None)");
    }

    #[test]
    fn the_list_opens_with_the_builtins_and_picks_the_pending_one() {
        let mut f = page(Area::MIN);
        let thumbs: Vec<Option<Canvas>> = builtins()
            .iter()
            .map(|_| Some(Canvas::new(16, 16)))
            .collect();
        fill(
            &mut f,
            &[PathBuf::from("/w/a.jpg")],
            "reprise:tarn/dusk",
            None,
            &thumbs,
        );
        let Control::ListBox { list, icons } = &f.item(WALLPAPER).unwrap().control else {
            panic!("the wallpaper list is a list box");
        };
        let at = list.selected.unwrap();
        assert_eq!(list.items[at], "Tarn (dusk)");
        assert_eq!(list.detail[at], "Reprise", "a built-in says whose it is");
        assert!(icons[at].is_some(), "a built-in row shows its thumbnail");
        assert_eq!(row(at), Row::Builtin(builtin("reprise:tarn/dusk").unwrap()));
        assert!(list.items.contains(&"Flow (glass colour)".to_string()));
        assert!(list.items.contains(&"Terraces (frost)".to_string()));
        assert_eq!(builtins().len(), 42 + 4);
        // The glass-coloured row is picked for its own spelling.
        fill(&mut f, &[], "reprise:caustic/glass", None, &[]);
        assert_eq!(
            selected(&f).map(row),
            Some(Row::Builtin(builtin("reprise:caustic/glass").unwrap()))
        );
    }

    #[test]
    fn a_builtin_on_the_monitor_is_its_glass_coloured_spelling_and_fills_it() {
        let mut s = Settings {
            wallpaper: "reprise:flow/glass".into(),
            glass_mode: "dark".into(),
            ..Settings::default()
        };
        assert_eq!(monitor_file(&s, &[]), "reprise:flow/night");
        s.wallpaper = "reprise:ridges/dusk".into();
        assert_eq!(monitor_file(&s, &[]), "reprise:ridges/dusk");
        let mut f = page(Area::MIN);
        follow(&mut f, &s);
        follow_use_for(&mut f, &s, &[]);
        match &f.item(POSITION).unwrap().control {
            Control::Combo(c) => assert!(!c.enabled),
            _ => panic!("the position is a combo"),
        }
    }

    #[test]
    fn a_slideshow_lists_the_folders_and_shows_the_first_picture_of_its_own() {
        let pics = [
            PathBuf::from("/w/b/dunes.jpg"),
            PathBuf::from("/w/a/hills.jpg"),
            PathBuf::from("/w/a/lake.jpg"),
        ];
        let found = folders(&pics, Some(PathBuf::from("/x/Mine")));
        assert_eq!(
            found,
            ["/w/a", "/w/b", "/x/Mine"].map(PathBuf::from).to_vec(),
            "each folder once, by name"
        );
        let mut f = page(Area::MIN);
        fill_folders(&mut f, &found, "/w/b", None);
        match &f.item(WALLPAPER).unwrap().control {
            Control::ListBox { list, .. } => {
                assert_eq!(list.items, ["a", "b", "Mine"]);
                assert_eq!(list.selected, Some(1));
            }
            _ => panic!("the wallpaper list is a list box"),
        }
        let s = Settings {
            wallpaper: "/w/b/dunes.jpg".into(),
            slideshow: "/w/a".into(),
            ..Settings::default()
        };
        let listed = [
            PathBuf::from("/w/a/hills.jpg"),
            PathBuf::from("/w/a/lake.jpg"),
        ];
        assert_eq!(monitor_file(&s, &listed), "/w/a/hills.jpg");
        let one = Settings {
            slideshow: String::new(),
            ..s
        };
        assert_eq!(monitor_file(&one, &listed), "/w/b/dunes.jpg");
    }

    #[test]
    fn a_slideshow_shows_the_first_picture_of_its_own_folder_listing() {
        let s = Settings {
            slideshow: "/far/Trips".into(),
            ..Settings::default()
        };
        assert_eq!(monitor_file(&s, &[]), "", "the desktop colour until listed");
        let other = [PathBuf::from("/w/a/hills.jpg")];
        assert_eq!(monitor_file(&s, &other), "", "another folder's listing");
        let listed = [
            PathBuf::from("/far/Trips/coast.jpg"),
            PathBuf::from("/far/Trips/hut.png"),
        ];
        assert_eq!(monitor_file(&s, &listed), "/far/Trips/coast.jpg");
    }

    #[test]
    fn a_slideshow_starts_on_the_folder_picked_before_else_the_wallpapers() {
        let dir = std::env::temp_dir().join(format!("reprise-slide-start-{}", std::process::id()));
        let (trips, walls) = (dir.join("Trips"), dir.join("Walls"));
        std::fs::create_dir_all(&trips).unwrap();
        std::fs::create_dir_all(&walls).unwrap();
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let wall = s(&walls.join("dunes.jpg"));
        assert_eq!(slideshow_start(&s(&trips), &wall), Some(s(&trips)));
        assert_eq!(slideshow_start("", &wall), Some(s(&walls)));
        assert_eq!(
            slideshow_start(&s(&dir.join("gone")), &wall),
            Some(s(&walls)),
            "a folder since removed is skipped"
        );
        assert_eq!(slideshow_start("", "reprise:flow/night"), None);
        assert_eq!(slideshow_start("", ""), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_slideshow_controls_follow_the_mode() {
        let mut f = page(Area::MIN);
        let mut s = Settings::default();
        follow(&mut f, &s);
        follow_use_for(&mut f, &s, &[]);
        assert_eq!(f.combo_selected(MODE), Some(0));
        let on = |f: &Form, id: &str| match &f.item(id).unwrap().control {
            Control::Check { enabled, .. } => *enabled,
            Control::Combo(c) => c.enabled,
            _ => true,
        };
        assert!(!on(&f, INTERVAL) && !on(&f, "slideshow_shuffle"));
        s.slideshow = "~/Pictures".into();
        follow(&mut f, &s);
        follow_use_for(&mut f, &s, &[]);
        assert_eq!(f.combo_selected(MODE), Some(1));
        assert!(on(&f, INTERVAL) && on(&f, "slideshow_shuffle"));
    }

    #[test]
    fn only_a_pending_wallpaper_not_on_the_monitor_is_decoded() {
        assert_eq!(
            next_decode("/w/b.png", Some("/w/a.png"), true),
            None,
            "one decode at a time"
        );
        assert_eq!(
            next_decode("/w/b.png", Some("/w/a.png"), false),
            Some(PathBuf::from("/w/b.png"))
        );
        assert_eq!(
            next_decode("/w/b.png", None, false),
            Some(PathBuf::from("/w/b.png"))
        );
        assert_eq!(
            next_decode("", None, false),
            None,
            "no wallpaper, nothing to decode"
        );
        if let Some(home) = std::env::var_os("HOME") {
            let want = Path::new(&home).join("w/a.png");
            assert_eq!(
                next_decode(&want.to_string_lossy(), Some("~/w/a.png"), false),
                None,
                "the monitor shows the file however the config spells it"
            );
        }
    }
}
