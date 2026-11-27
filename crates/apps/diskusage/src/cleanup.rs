//! The Clean Up box: [`reprisefs::cleanup`]'s parts, each with its size and a check box, the
//! confirm box naming what goes, and the removal on a thread.

use reprisefs::cleanup::{Found, Part, Places};
use reprisefs::props::short_size;
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{Form, Item};
use reprisekit::raster::Rect;
use reprisekit::theme::TextUnits;
use reprisekit::tr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const CLIENT_W: i32 = 380;
const MARGIN: i32 = 10;
const INTRO_H: i32 = 26;
const GROUP_Y: i32 = 42;
const ROW_Y: i32 = GROUP_Y + 20;
const ROW_H: i32 = 20;
const SIZE_X: i32 = 250;
const ABOUT_H: i32 = 70;

pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";
const TOTAL: &str = "total";
const ABOUT: &str = "about";

/// What the Clean Up thread posts: a part's count, tagged with the box that asked for it, or what
/// a removal left.
pub enum Msg {
    Found(u64, Part, Found),
    Cleaned(Vec<Cleaned>),
}

/// One part's removal.
pub struct Cleaned {
    pub part: Part,
    /// Bytes gone: the confirmed size, or for a part that failed what a count afterwards misses;
    /// crash dumps and Flatpak runtimes are not counted again, so their failure frees 0.
    pub freed: u64,
    pub result: Result<(), String>,
}

fn check_id(p: Part) -> &'static str {
    match p {
        Part::RecycleBin => "recycle-bin",
        Part::Thumbnails => "thumbnails",
        Part::Caches => "caches",
        Part::Coredumps => "coredumps",
        Part::Flatpak => "flatpak",
    }
}

fn size_id(p: Part) -> &'static str {
    match p {
        Part::RecycleBin => "recycle-bin-size",
        Part::Thumbnails => "thumbnails-size",
        Part::Caches => "caches-size",
        Part::Coredumps => "coredumps-size",
        Part::Flatpak => "flatpak-size",
    }
}

/// The part's name, with an `&` before its access key unless `plain`.
pub fn name(p: Part, plain: bool) -> String {
    let s = match p {
        Part::RecycleBin => tr!("cleanup-recycle-bin"),
        Part::Thumbnails => tr!("cleanup-thumbnails"),
        Part::Caches => tr!("cleanup-caches"),
        Part::Coredumps => tr!("cleanup-coredumps"),
        Part::Flatpak => tr!("cleanup-flatpak"),
    };
    match plain {
        true => reprisekit::text::split_mnemonic(&s).0,
        false => s,
    }
}

fn about(p: Part) -> String {
    match p {
        Part::RecycleBin => tr!("cleanup-about-recycle-bin"),
        Part::Thumbnails => tr!("cleanup-about-thumbnails"),
        Part::Caches => tr!("cleanup-about-caches"),
        Part::Coredumps => tr!("cleanup-about-coredumps"),
        Part::Flatpak => tr!("cleanup-about-flatpak"),
    }
}

/// The box's client size for `parts` rows, at 100% text.
pub fn client(parts: usize) -> (i32, i32) {
    let group_h = 24 + ROW_H * parts as i32;
    let about_y = GROUP_Y + group_h + 30;
    (CLIENT_W, about_y + ABOUT_H + 8 + BUTTON_H + 8)
}

pub struct Cleanup {
    pub form: Form,
    pub places: Places,
    /// The parts offered, each with its count once it is in.
    pub parts: Vec<(Part, Option<Found>)>,
    /// Tags this box's counts, so one posted for an earlier box is dropped.
    pub generation: u64,
    /// The part the description is of: the row last focused or pointed at.
    described: Part,
    /// The focus [`Cleanup::sync`] last saw, so a focus move takes the description.
    seen_focus: Option<usize>,
    /// A key or press reached the box: the focus is the user's from then on.
    pub touched: bool,
    /// Set when the box goes, so its count stops.
    pub stop: Arc<AtomicBool>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Cleanup {
    pub fn new(places: Places, generation: u64) -> Cleanup {
        let parts: Vec<(Part, Option<Found>)> =
            places.parts().into_iter().map(|p| (p, None)).collect();
        let (w, h) = client(parts.len());
        let inner = CLIENT_W - 2 * MARGIN;
        let group_h = 24 + ROW_H * parts.len() as i32;
        let mut items = vec![
            Item::lines(
                "intro",
                Rect::new(MARGIN, MARGIN, inner, INTRO_H),
                &[&tr!("cleanup-intro")],
            ),
            Item::group(
                "files",
                Rect::new(MARGIN, GROUP_Y, inner, group_h),
                &tr!("cleanup-files"),
            ),
        ];
        for (i, (p, _)) in parts.iter().enumerate() {
            let y = ROW_Y + ROW_H * i as i32;
            items.push(Item::check(check_id(*p), MARGIN + 10, y, &name(*p, false)).enabled(false));
            items.push(Item::value_at(
                size_id(*p),
                SIZE_X,
                y,
                CLIENT_W - SIZE_X - 2 * MARGIN,
                &tr!("cleanup-counting"),
            ));
        }
        let total_y = GROUP_Y + group_h + 8;
        items.push(Item::label_at(TOTAL, MARGIN, total_y, inner, ""));
        let about_y = total_y + 22;
        items.push(Item::group(
            "about-group",
            Rect::new(MARGIN, about_y, inner, ABOUT_H),
            &tr!("cleanup-description"),
        ));
        items.push(Item::lines(
            ABOUT,
            Rect::new(MARGIN + 10, about_y + 18, inner - 20, ABOUT_H - 26),
            &[],
        ));
        let by = h - BUTTON_H - 8;
        items.push(Item::button(
            OK,
            Rect::new(w - 2 * BUTTON_W - 14, by, BUTTON_W, BUTTON_H),
            &reprisekit::kit_tr!("ok"),
            false,
            true,
        ));
        items.push(Item::button(
            CANCEL,
            Rect::new(w - BUTTON_W - 8, by, BUTTON_W, BUTTON_H),
            &reprisekit::kit_tr!("cancel"),
            true,
            false,
        ));
        let described = parts.first().map_or(Part::RecycleBin, |(p, _)| *p);
        let mut c = Cleanup {
            form: Form::new(items),
            places,
            parts,
            generation,
            described,
            seen_focus: None,
            touched: false,
            stop: Arc::new(AtomicBool::new(false)),
        };
        for (p, _) in c.parts.clone() {
            c.form.set_check(check_id(p), p.checked_first());
        }
        c.sync();
        c
    }

    /// Takes `part`'s count: its box takes input, or unchecks and stays grey when there is
    /// nothing it can remove. The first box to take input takes the focus from Cancel.
    pub fn found(&mut self, part: Part, found: Found) {
        let some = found.removable && (found.size > 0 || !found.items.is_empty());
        self.form.set_text(size_id(part), &short_size(found.size));
        self.form.set_enabled(check_id(part), some);
        if !some {
            self.form.set_check(check_id(part), false);
        }
        let on_cancel = self.form.focus() == self.form.index_of(CANCEL);
        if some && !self.touched && on_cancel {
            self.form.focus_id(check_id(part));
        }
        if let Some((_, f)) = self.parts.iter_mut().find(|(p, _)| *p == part) {
            *f = Some(found);
        }
        self.sync();
    }

    /// The parts checked, each with its count; empty while a checked part is still counting.
    pub fn picked(&self) -> Vec<(Part, &Found)> {
        let checked = |p: Part| self.form.checked(check_id(p)) == Some(true);
        if self.parts.iter().any(|(p, f)| f.is_none() && checked(*p)) {
            return Vec::new();
        }
        (self.parts.iter())
            .filter_map(|(p, f)| Some((*p, f.as_ref()?)))
            .filter(|(p, _)| self.form.checked(check_id(*p)) == Some(true))
            .collect()
    }

    /// The total, the OK button and the description, after any change.
    pub fn sync(&mut self) {
        let focus = self.form.focus();
        let focused = (focus.filter(|_| focus != self.seen_focus))
            .and_then(|i| self.form.items.get(i))
            .and_then(|it| self.parts.iter().find(|(p, _)| check_id(*p) == it.id));
        if let Some((p, _)) = focused {
            self.described = *p;
        }
        self.seen_focus = focus;
        let picked = self.picked();
        let total: u64 = picked.iter().map(|(_, f)| f.size).sum();
        let ok = !picked.is_empty();
        let total = match ok {
            true => tr!("cleanup-total", size = short_size(total)),
            false if self.parts.iter().any(|(_, f)| f.is_none()) => tr!("cleanup-counting"),
            false => tr!("cleanup-total", size = short_size(0)),
        };
        self.form.set_text(TOTAL, &total);
        self.form.set_enabled(OK, ok);
        self.form.set_lines(ABOUT, &[about(self.described)]);
    }

    /// The description follows the pointer onto a row, a greyed one too; true when it changed.
    pub fn point(&mut self, x: i32, y: i32, u: TextUnits) -> bool {
        let row = (0..self.parts.len()).find(|&i| {
            let top = ROW_Y - 3 + ROW_H * i as i32;
            (u.len(top)..u.len(top + ROW_H)).contains(&y)
                && (u.len(MARGIN)..u.len(CLIENT_W - MARGIN)).contains(&x)
        });
        match row.map(|i| self.parts[i].0) {
            Some(p) if p != self.described => {
                self.described = p;
                self.form.set_lines(ABOUT, &[about(p)]);
                true
            }
            _ => false,
        }
    }

    /// The confirm box's text: what goes, part by part, a folder part with its folder and
    /// Flatpak's runtimes by ref.
    pub fn confirm_text(&self) -> String {
        let mut lines = vec![tr!("cleanup-confirm"), String::new()];
        for (p, f) in self.picked() {
            let size = short_size(f.size);
            lines.push(match &f.folder {
                Some(folder) => tr!(
                    "cleanup-confirm-folder-row",
                    part = name(p, true),
                    folder = folder.display().to_string(),
                    size = size
                ),
                None => tr!("cleanup-confirm-row", part = name(p, true), size = size),
            });
            lines.extend(f.items.iter().map(|r| {
                let installation = if r.system { "system" } else { "user" };
                tr!(
                    "cleanup-confirm-runtime",
                    runtime = format!("{}/{}/{}", r.id, r.arch, r.branch),
                    installation = installation
                )
            }));
        }
        lines.join("\n")
    }
}

/// Counts each of `places`' parts in turn, posting each count as it is in, until `stop` is set.
pub fn count(
    places: Places,
    generation: u64,
    stop: Arc<AtomicBool>,
    post: impl Fn(Msg) + Send + 'static,
) {
    std::thread::spawn(move || {
        for p in places.parts() {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            post(Msg::Found(generation, p, places.measure(p, &stop)));
        }
    });
}

/// Removes `parts` in turn, each as far as the count the confirm box named, posting what each
/// freed and left.
pub fn clean(places: Places, parts: Vec<(Part, Found)>, post: impl FnOnce(Msg) + Send + 'static) {
    std::thread::spawn(move || {
        let done = (parts.into_iter())
            .map(|(part, f)| {
                let result = places.clean(part, &f).map_err(|e| e.to_string());
                let freed = match (&result, part) {
                    (Ok(()), _) => f.size,
                    (Err(_), Part::Coredumps | Part::Flatpak) => 0,
                    (Err(_), _) => {
                        let left = places.measure(part, &AtomicBool::new(false)).size;
                        f.size.saturating_sub(left)
                    }
                };
                Cleaned {
                    part,
                    freed,
                    result,
                }
            })
            .collect();
        post(Msg::Cleaned(done));
    });
}
