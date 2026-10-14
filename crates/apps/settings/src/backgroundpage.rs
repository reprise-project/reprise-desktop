//! Background page: a monitor showing the pending wallpaper over the desktop
//! colour, one picture or a slideshow, their position, the colour, the
//! slideshow interval and the desktop icons. The host fills the list (scan
//! and thumbnails run on threads) and hands in the decoded or drawn picture;
//! this module lays the page out and composes the monitor's screen.

use std::path::{Path, PathBuf};

use reprisekit::classic::assets::MONITOR_SCREEN;
use reprisekit::classic::widgets::BUTTON_W;
use reprisekit::controls::colorbutton::ColorButton;
use reprisekit::controls::form::{Control, Form, Item, Screen};
use reprisekit::controls::listbox::ListBox;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::schemes;
use reprisekit::theme::Color;
use reprisekit::wallpaper::{Position, compose};
use reprisekit::wallpapers::{self, Builtin, Design, Variant};

use crate::model::{DESKTOP_EXTENSIONS, POSITIONS, SLIDE_INTERVALS, Settings, WALLPAPER_MODES};
use crate::pages::{Area, GX, X, button, choice};
use crate::wallpapers::expand;

pub const MONITOR: &str = "monitor";
pub const MODE: &str = "wallpaper_mode";
pub const WALLPAPER: &str = "wallpaper";
pub const BROWSE: &str = "browse";
/// The file name extensions combo and its label, greyed while the desktop shows no files.
pub const EXTENSIONS: &str = "desktop_extensions";
const EXTENSIONS_LABEL: &str = "extensions_label";
/// The Slideshow group's top at [`Area::MIN`], 11 px under the Desktop icons group.
const SLIDE_Y: i32 = 393;
/// The slideshow's interval combo and Shuffle check, and the group round
/// them, greyed for one picture.
pub const INTERVAL: &str = "slideshow_interval";
const SLIDE_CONTROLS: [&str; 4] = [
    "slideshow_group",
    "interval_label",
    INTERVAL,
    "slideshow_shuffle",
];
pub const NONE_ROW: &str = "(None)";
pub const MONITOR_RECT: Rect = Rect::new(X, 44, 191, 168);
/// The right column, beside the monitor; 159 px wide at [`Area::MIN`].
const RX: i32 = 219;
fn rw(a: Area) -> i32 {
    a.right() - RX
}
/// The list, which takes the right column's width and the page's extra height.
pub fn list_rect(a: Area) -> Rect {
    Rect::new(RX, 66, rw(a), 191 + a.dh)
}
/// Rows the list shows at [`Area::MIN`]: 11 of 17 px inside its 2 px frame.
/// A picked row is scrolled to within these.
const LIST_ROWS: usize = 11;
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

pub fn thumb_size(scale: i32) -> u32 {
    16 * scale.max(1) as u32
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

/// `b`'s row icon at `scale`: the picture drawn into the 16 px square.
pub fn thumb(b: Builtin, glass: &reprisekit::glass::Settings, scale: i32) -> Canvas {
    let n = thumb_size(scale);
    let mut t = Canvas::with_scale(16, 16, scale);
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
/// right column and the Slideshow group move down, and the Desktop icons
/// group stays under the monitor.
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
        button(BROWSE, right, 264 + dh, "B&rowse..."),
        Item::label_at("position_label", RX, 310 + dh, BUTTON_W, "&Position:"),
        choice("wallpaper_position", RX, 326 + dh, BUTTON_W, POSITIONS),
        Item::label_at("color_label", right, 310 + dh, BUTTON_W, "&Color:"),
        Item::new(
            "background",
            Rect::new(right, 326 + dh, BUTTON_W, 21),
            Control::Color(ColorButton::new(
                Color::rgb(0, 0, 0),
                schemes::PALETTE.to_vec(),
            )),
        ),
        Item::group("icons_group", Rect::new(X, 222, 191, 160), "Desktop icons"),
        Item::check("documents", GX, 244, "&Documents"),
        Item::check("computer", GX, 266, "Co&mputer"),
        Item::check("home", GX, 288, &home_check()),
        Item::check("desktop_icons", GX, 310, "&Show the files in ~/Desktop"),
        Item::label_at(EXTENSIONS_LABEL, GX, 334, 167, "File name e&xtensions:"),
        choice(EXTENSIONS, GX, 350, 167, DESKTOP_EXTENSIONS),
        Item::group(
            "slideshow_group",
            Rect::new(X, SLIDE_Y + dh, a.w, 70),
            "Slideshow",
        ),
        Item::label_at(
            "interval_label",
            GX,
            SLIDE_Y + 24 + dh,
            130,
            "Change picture &every:",
        ),
        Item::combo(
            INTERVAL,
            GX + 134,
            SLIDE_Y + 20 + dh,
            120,
            &intervals,
            intervals.len(),
        ),
        Item::check("slideshow_shuffle", GX, SLIDE_Y + 46 + dh, "Sh&uffle"),
    ])
}

pub fn slideshow(s: &Settings) -> bool {
    !s.slideshow.is_empty()
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
    let label = reprisefs::places::home_label().replace('&', "&&");
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
/// or when it did not decode, that is the desktop colour alone. Once
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

pub fn show(form: &mut Form, s: &Settings, want: &str, picture: Option<&Picture>) {
    let want = screen(s, want, picture);
    if let Some(Control::Monitor(on)) = form.item_mut(MONITOR).map(|i| &mut i.control)
        && *on != want
    {
        *on = want;
    }
}

/// Show Picture or Slideshow, grey the position while there is no
/// wallpaper and the slideshow's controls without a slideshow, and show
/// on the Color button the desktop colour `s` gives while `[colors]
/// background` is unset.
pub fn follow(form: &mut Form, s: &Settings) {
    let show = slideshow(s);
    form.set_combo(MODE, show as usize);
    let has = (!s.wallpaper.is_empty() && builtin(&s.wallpaper).is_none()) || show;
    form.set_enabled("position_label", has);
    form.set_enabled("wallpaper_position", has);
    for id in SLIDE_CONTROLS {
        form.set_enabled(id, show);
    }
    form.set_enabled(EXTENSIONS_LABEL, s.desktop_icons);
    form.set_enabled(EXTENSIONS, s.desktop_icons);
    if s.background.is_empty()
        && let Some(Control::Color(b)) = form.item_mut("background").map(|i| &mut i.control)
    {
        b.color = crate::appearancepage::theme(s).colors.background;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Desktop icons group's extensions row on Show, and greyed with the files off. The rows
    /// above are left out: the Home check box reads the user's name.
    #[test]
    fn the_desktop_icons_group_pins_its_pixels_in_every_look() {
        if !reprisekit::text::test_tahoma() {
            return;
        }
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
            let theme = reprisekit::interfaces::theme_for(i);
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
            got.push(hashes);
        }
        assert_eq!(
            got,
            [
                [0xa9cc8b84ed350ffe, 0x66f605a354ffe528],
                [0x058be053ad8eaf77, 0x9aebc2626f26bc70],
                [0xf241b02b1fdb815c, 0x110665e0b085858e],
            ],
            "Classic, Blue, Glass: shown, greyed"
        );
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
        match &f.item("wallpaper_position").unwrap().control {
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
        assert_eq!(f.combo_selected(MODE), Some(0));
        let on = |f: &Form, id: &str| match &f.item(id).unwrap().control {
            Control::Check { enabled, .. } => *enabled,
            Control::Combo(c) => c.enabled,
            _ => true,
        };
        assert!(!on(&f, INTERVAL) && !on(&f, "slideshow_shuffle"));
        s.slideshow = "~/Pictures".into();
        follow(&mut f, &s);
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
