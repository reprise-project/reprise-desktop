//! The Screen Saver & Lock page: the saver and its wait (`[idle] blank_after`),
//! then the Lock group, the paired-device lock (`[lock]`) included.

use std::path::{Path, PathBuf};

use reprisebus::bluez::Device;
use reprisekit::classic::assets::MONITOR_SCREEN;
use reprisekit::classic::widgets::BUTTON_W;
use reprisekit::controls::combo::{Combo, ListItem};
use reprisekit::controls::form::{Control, Form, Item, Screen};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::saver::{self, Kind, Options, Saver};
use reprisekit::text::TextRenderer;

use crate::model::Settings;
use crate::pages::{Area, GX, X, button, spin};

pub const MONITOR: &str = "saver_monitor";
pub const SAVER: &str = "saver";
pub const SETTINGS: &str = "saver_settings";
pub const PREVIEW: &str = "saver_preview";
pub const WAIT: &str = "saver_wait";
pub const AWAY: &str = "away_device";
pub const AWAY_AFTER: &str = "away_after_s";
const AWAY_ROWS: usize = 8;
pub fn monitor_rect(a: Area) -> Rect {
    Rect::new(X + (a.w - 191) / 2, 44, 191, 168)
}
pub const NONE_ROW: &str = "(None)";
/// Seconds of wait a saver picked over "(None)" starts with.
pub const WAIT_DEFAULT: i64 = 600;

pub fn page(a: Area) -> Form {
    let rows: Vec<&str> = std::iter::once(NONE_ROW)
        .chain(Kind::ALL.iter().map(|k| k.label()))
        .collect();
    let bx = GX + 156;
    let mut items = vec![
        Item::new(MONITOR, monitor_rect(a), Control::Monitor(Screen::Black)),
        Item::group("saver_group", Rect::new(X, 222, a.w, 84), "Screen saver"),
        Item::combo(SAVER, GX, 244, 150, &rows, rows.len()),
        button(SETTINGS, bx, 243, "Se&ttings..."),
        button(PREVIEW, bx + BUTTON_W + 6, 243, "Pre&view"),
    ];
    items.extend(spin("wait_label", WAIT, GX, 282, 40, "&Wait:", (1, 1440)));
    items.push(Item::label_at("wait_units", GX + 100, 282, 60, "minutes"));

    items.push(Item::group(
        "lock_group",
        Rect::new(X, 314, a.w, 160),
        "Lock",
    ));
    let gw = a.gw();
    let seconds = |id, y| Item::label_at(id, GX + 174, y, gw - 174, "seconds (0 is off)");
    items.extend(spin(
        "lock_label",
        "lock_after",
        GX,
        336,
        114,
        "&Lock after:",
        (0, 99999),
    ));
    items.push(seconds("lock_units", 336));
    items.extend(spin(
        "fade_label",
        "lock_fade",
        GX,
        360,
        114,
        "&Fade before lock:",
        (0, 99999),
    ));
    items.push(seconds("fade_units", 360));
    items.push(Item::check(
        "lock_on_sleep",
        GX,
        382,
        "Lock the computer when it goes to &sleep",
    ));
    items.push(Item::check(
        "lock_wallpaper",
        GX,
        404,
        "Show the wallpaper on the lock scree&n",
    ));
    items.push(Item::label_at(
        "away_label",
        GX,
        426,
        gw,
        "Lock when this device &disconnects:",
    ));
    items.push(Item::combo(AWAY, GX, 442, 150, &[NONE_ROW], AWAY_ROWS));
    items.extend(spin(
        "away_after_label",
        AWAY_AFTER,
        GX + 160,
        446,
        40,
        "Aft&er:",
        (0, 3600),
    ));
    items.push(Item::label_at("away_units", GX + 260, 446, 60, "seconds"));
    Form::new(items)
}

pub fn kind(s: &Settings) -> Kind {
    Kind::parse(&s.saver).unwrap_or_default()
}

pub fn row(s: &Settings) -> usize {
    if s.blank_after <= 0 {
        return 0;
    }
    Kind::ALL.iter().position(|&k| k == kind(s)).unwrap_or(0) + 1
}

pub fn pick(s: &mut Settings, row: usize) {
    match row.checked_sub(1).and_then(|i| Kind::ALL.get(i)) {
        None => s.blank_after = 0,
        Some(k) => {
            s.saver = k.name().to_string();
            if s.blank_after <= 0 {
                s.blank_after = WAIT_DEFAULT;
            }
        }
    }
}

pub fn set_wait(s: &mut Settings, minutes: i64) {
    s.blank_after = minutes.max(1) * 60;
}

pub fn options(s: &Settings) -> Options {
    let n = |v: i64| v.clamp(0, u32::MAX as i64) as u32;
    Options {
        speed: n(s.saver_speed),
        stars: n(s.saver_stars),
        shapes: n(s.saver_shapes),
        cycle_colors: s.saver_cycle,
        photo_seconds: n(s.saver_photo_seconds),
    }
    .clamped()
}

/// Put `s` on the combo and wait, greying what means nothing: the buttons and
/// wait under "(None)", Settings... for Blank.
pub fn follow(form: &mut Form, s: &Settings) {
    let at = row(s);
    form.set_combo(SAVER, at);
    let on = at > 0;
    form.set_enabled(SETTINGS, on && kind(s) != Kind::Blank);
    form.set_enabled(PREVIEW, on);
    for id in ["wait_label", WAIT, "wait_units"] {
        form.set_enabled(id, on);
    }
    if let Some(Control::UpDown(u)) = form.item_mut(WAIT).map(|i| &mut i.control) {
        u.set(((s.blank_after.max(60)) / 60).min(i32::MAX as i64) as i32);
    }
}

/// The device combo's rows: "(None)", the paired `devices`, then the configured device when
/// BlueZ does not list it, by its address or path.
pub fn away_rows(devices: &[Device], s: &Settings) -> Vec<String> {
    let mut rows = vec![NONE_ROW.to_string()];
    rows.extend(devices.iter().map(|d| match d.name.as_str() {
        "" => d.address.clone(),
        n => n.to_string(),
    }));
    if !s.away_device.is_empty() && !devices.iter().any(|d| d.is(&s.away_device)) {
        rows.push(s.away_device.clone());
    }
    rows
}

pub fn away_row(devices: &[Device], s: &Settings) -> usize {
    if s.away_device.is_empty() {
        return 0;
    }
    devices
        .iter()
        .position(|d| d.is(&s.away_device))
        .unwrap_or(devices.len())
        + 1
}

/// Row `row` of [`away_rows`] picked: a device stores its address, the unlisted row keeps the value.
pub fn pick_away(devices: &[Device], s: &mut Settings, row: usize) {
    match row.checked_sub(1) {
        None => s.away_device.clear(),
        Some(i) => {
            if let Some(d) = devices.get(i) {
                s.away_device = d.address.clone();
            }
        }
    }
}

/// Fill the device combo from `devices` and put `s` on it; the grace greys under "(None)".
pub fn follow_away(form: &mut Form, devices: &[Device], s: &Settings) {
    let rows = away_rows(devices, s);
    let at = away_row(devices, s);
    if let Some(Control::Combo(c)) = form.item_mut(AWAY).map(|i| &mut i.control) {
        let same = c.items.iter().map(|i| &i.text).eq(rows.iter());
        if !same {
            let enabled = c.enabled;
            *c = Combo::with_items(rows.into_iter().map(ListItem::new).collect(), at, AWAY_ROWS);
            c.enabled = enabled;
        }
    }
    form.set_combo(AWAY, at);
    for id in ["away_after_label", AWAY_AFTER, "away_units"] {
        form.set_enabled(id, at > 0);
    }
}

pub struct Preview {
    pub saver: Saver,
    pub kind: Kind,
    pub options: Options,
    /// `[screensaver] folder` as the sheet holds it, empty for Pictures.
    pub folder: String,
    /// The Photos folder's pictures, as the full-screen saver lists them.
    pub files: Vec<PathBuf>,
    /// The picture shown last, which the next pick skips.
    pub current: Option<PathBuf>,
}

pub fn screen_size() -> (u32, u32) {
    let (_, _, w, h) = MONITOR_SCREEN;
    (w as u32, h as u32)
}

impl Preview {
    /// Seeded so the page shows the same each time it opens.
    pub fn new(kind: Kind, options: Options, folder: &str, text: &TextRenderer) -> Preview {
        let (w, h) = screen_size();
        Preview {
            saver: Saver::new(kind, &options, w, h, 7, text),
            kind,
            options,
            folder: folder.to_string(),
            files: Vec::new(),
            current: None,
        }
    }

    /// Whether this runs the saver `s` picks; a Photos folder edit makes it start over.
    pub fn runs(&self, kind: Kind, s: &Settings) -> bool {
        self.kind == kind
            && self.options == options(s)
            && (kind != Kind::Photos || self.folder == s.saver_folder)
    }

    pub fn frame(&mut self, ms: u64) -> Canvas {
        self.saver.advance(ms);
        let (w, h) = screen_size();
        let mut c = Canvas::new(w, h);
        self.saver.draw(&mut c);
        c
    }
}

impl Preview {
    /// Whether Photos wants its next picture; once true, it waits for [`Preview::show_photo`].
    pub fn wants_photo(&mut self) -> bool {
        !self.files.is_empty() && self.saver.take_photo_request()
    }

    pub fn show_photo(&mut self, path: PathBuf, picture: &Canvas) {
        let (w, h) = screen_size();
        self.current = Some(path);
        self.saver.push_photo(saver::fit(picture, w, h));
    }
}

/// The next picture for Photos: one of `files` at random but `current`, as full screen picks,
/// with `decode` skipping a file that does not decode (eight tries at most). `roll` gives each try
/// its dice.
pub fn pick_photo(
    files: &[PathBuf],
    current: Option<&Path>,
    mut roll: impl FnMut() -> u32,
    mut decode: impl FnMut(&Path) -> Option<Canvas>,
) -> Option<(PathBuf, Canvas)> {
    let mut current = current.map(Path::to_path_buf);
    for _ in 0..files.len().min(8) {
        let path = reprisefs::pictures::next(files, current.as_deref(), true, roll())?;
        if let Some(picture) = decode(&path) {
            return Some((path, picture));
        }
        current = Some(path);
    }
    None
}

pub fn show(form: &mut Form, frame: Canvas) {
    if let Some(Control::Monitor(on)) = form.item_mut(MONITOR).map(|i| &mut i.control) {
        *on = Screen::Picture(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_is_a_wait_of_zero_and_a_saver_turns_it_on() {
        let mut s = Settings::default();
        assert_eq!(row(&s), 0, "blank_after 0 shows (None)");
        pick(&mut s, 2);
        assert_eq!(
            (s.saver.as_str(), s.blank_after),
            ("starfield", WAIT_DEFAULT)
        );
        assert_eq!(row(&s), 2);
        s.blank_after = 120;
        pick(&mut s, 3);
        assert_eq!(
            (s.saver.as_str(), s.blank_after),
            ("mystify", 120),
            "a wait set stays"
        );
        pick(&mut s, 0);
        assert_eq!((s.saver.as_str(), s.blank_after), ("mystify", 0));
        set_wait(&mut s, 15);
        assert_eq!(s.blank_after, 900);
    }

    #[test]
    fn the_buttons_follow_the_saver() {
        let mut f = page(Area::MIN);
        let mut s = Settings::default();
        let on = |f: &Form, id| match &f.item(id).unwrap().control {
            Control::Button { enabled, .. } => *enabled,
            _ => unreachable!(),
        };
        follow(&mut f, &s);
        assert!(!on(&f, SETTINGS) && !on(&f, PREVIEW));
        s.blank_after = 300;
        follow(&mut f, &s);
        assert_eq!(f.combo_selected(SAVER), Some(1), "Blank");
        assert!(
            !on(&f, SETTINGS) && on(&f, PREVIEW),
            "Blank has no settings"
        );
        s.saver = "logo".into();
        follow(&mut f, &s);
        assert!(on(&f, SETTINGS) && on(&f, PREVIEW));
        match &f.item(WAIT).unwrap().control {
            Control::UpDown(u) => assert_eq!(u.value(), 5, "300 s is 5 minutes"),
            _ => unreachable!(),
        }
    }

    fn paired() -> Vec<Device> {
        let device = |n: u8, name: &str| Device {
            path: format!("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_0{n}"),
            address: format!("AA:BB:CC:DD:EE:0{n}"),
            name: name.into(),
            paired: true,
            connected: false,
        };
        vec![device(1, "Phone"), device(2, "")]
    }

    #[test]
    fn the_away_list_is_none_then_the_paired_devices() {
        let devices = paired();
        let mut s = Settings::default();
        assert_eq!(
            away_rows(&devices, &s),
            ["(None)", "Phone", "AA:BB:CC:DD:EE:02"]
        );
        assert_eq!(away_row(&devices, &s), 0);
        pick_away(&devices, &mut s, 1);
        assert_eq!(s.away_device, "AA:BB:CC:DD:EE:01");
        assert_eq!(away_row(&devices, &s), 1);
        s.away_device = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_02".into();
        assert_eq!(away_row(&devices, &s), 2, "a path names its device");
        pick_away(&devices, &mut s, 0);
        assert_eq!(s.away_device, "");
    }

    #[test]
    fn a_device_bluez_does_not_list_keeps_its_row() {
        let s = Settings {
            away_device: "11:22:33:44:55:66".into(),
            ..Settings::default()
        };
        assert_eq!(away_rows(&[], &s), ["(None)", "11:22:33:44:55:66"]);
        assert_eq!(away_row(&[], &s), 1);
        let mut kept = s.clone();
        pick_away(&[], &mut kept, 1);
        assert_eq!(kept.away_device, s.away_device, "picking it again keeps it");
    }

    #[test]
    fn the_grace_greys_without_a_device() {
        let mut f = page(Area::MIN);
        let devices = paired();
        let mut s = Settings::default();
        let on = |f: &Form, id| match &f.item(id).unwrap().control {
            Control::UpDown(u) => u.enabled,
            Control::Label { enabled, .. } => *enabled,
            _ => unreachable!(),
        };
        follow_away(&mut f, &devices, &s);
        assert!(!on(&f, AWAY_AFTER) && !on(&f, "away_units"));
        s.away_device = "aa:bb:cc:dd:ee:01".into();
        follow_away(&mut f, &devices, &s);
        assert_eq!(f.combo_selected(AWAY), Some(1));
        assert!(on(&f, AWAY_AFTER) && on(&f, "away_units"));
    }

    #[test]
    fn a_photo_is_any_picture_in_the_folder_but_the_one_shown() {
        let files: Vec<PathBuf> = ["a.jpg", "b.jpg", "c.jpg", "d.jpg", "e.jpg"]
            .map(PathBuf::from)
            .to_vec();
        let decode = |_: &Path| Some(Canvas::new(1, 1));
        let picks: Vec<PathBuf> = (0..5)
            .filter_map(|r| pick_photo(&files, None, || r, decode).map(|(p, _)| p))
            .collect();
        assert_eq!(
            picks, files,
            "every picture can come up, past the first three"
        );
        for r in 0..8 {
            let (p, _) = pick_photo(&files, Some(Path::new("e.jpg")), || r, decode).unwrap();
            assert_ne!(p, PathBuf::from("e.jpg"), "roll {r}");
        }
    }

    #[test]
    fn a_picture_that_does_not_decode_is_skipped() {
        let files: Vec<PathBuf> = ["bad.jpg", "good.jpg"].map(PathBuf::from).to_vec();
        let mut rolls = [0, 0].into_iter();
        let got = pick_photo(
            &files,
            None,
            || rolls.next().unwrap_or(0),
            |p| (p == Path::new("good.jpg")).then(|| Canvas::new(1, 1)),
        );
        assert_eq!(got.map(|(p, _)| p), Some(PathBuf::from("good.jpg")));
        assert!(pick_photo(&files, None, || 0, |_| None).is_none());
    }

    #[test]
    fn the_monitor_runs_the_saver_from_a_fixed_seed() {
        let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let opts = Options::default();
        let mut p = Preview::new(Kind::Starfield, opts.clone(), "", &text);
        let frame = p.frame(saver::STEP_MS * 20);
        let mut want = Saver::new(Kind::Starfield, &opts, 152, 112, 7, &text);
        for _ in 0..20 {
            want.step();
        }
        let mut c = Canvas::new(152, 112);
        want.draw(&mut c);
        assert_eq!(
            frame.pixels, c.pixels,
            "the page's frame is the kit's at seed 7"
        );
    }
    #[test]
    fn a_photos_folder_edit_starts_the_monitor_over() {
        let text = TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let mut s = Settings {
            saver: "photos".into(),
            saver_folder: "/srv/a".into(),
            ..Settings::default()
        };
        let p = Preview::new(Kind::Photos, options(&s), &s.saver_folder, &text);
        assert!(p.runs(Kind::Photos, &s));
        s.saver_folder = "/srv/b".into();
        assert!(!p.runs(Kind::Photos, &s), "another folder lists again");
        let p = Preview::new(Kind::Logo, options(&s), "/srv/a", &text);
        assert!(p.runs(Kind::Logo, &s), "the folder means nothing to Logo");
    }
}
