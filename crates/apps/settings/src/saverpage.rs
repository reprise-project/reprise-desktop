//! The Screen Saver page: a monitor running the pending saver, the saver
//! combo, Settings... and Preview, and the minutes of no input that start it
//! (`[idle] blank_after`, shown in seconds on the Power page).

use std::path::{Path, PathBuf};

use reprisekit::classic::assets::MONITOR_SCREEN;
use reprisekit::classic::widgets::BUTTON_W;
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
        Item::group("saver_group", Rect::new(X, 222, a.w, 106), "Screen saver"),
        Item::combo(SAVER, GX, 244, 150, &rows, rows.len()),
        button(SETTINGS, bx, 243, "Se&ttings..."),
        button(PREVIEW, bx + BUTTON_W + 6, 243, "Pre&view"),
    ];
    items.extend(spin("wait_label", WAIT, GX, 282, 40, "&Wait:", (1, 1440)));
    items.push(Item::label_at("wait_units", GX + 100, 282, 60, "minutes"));
    items.push(Item::label_at(
        "saver_note",
        GX,
        306,
        a.gw(),
        "Lock after on the Power page locks the screen under it.",
    ));
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

pub struct Preview {
    pub saver: Saver,
    pub kind: Kind,
    pub options: Options,
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
    pub fn new(kind: Kind, options: Options, text: &TextRenderer) -> Preview {
        let (w, h) = screen_size();
        Preview {
            saver: Saver::new(kind, &options, w, h, 7, text),
            kind,
            options,
            files: Vec::new(),
            current: None,
        }
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
        let mut p = Preview::new(Kind::Starfield, opts.clone(), &text);
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
}
