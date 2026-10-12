//! Date and Time Properties: the pure model.
//!
//! One `PropSheet` with two tabs. Date & Time holds the month combo, the
//! year spinner, the month grid, the analog clock, the time field and the
//! network time check box; Time Zone holds the zone combo over a panel
//! naming the zone. Nothing here reads the clock or a bus: the host feeds
//! it the time every second and what timedated reports, and carries out
//! the [`Change`] it hands back.
//!
//! The date and the time follow the clock until the user edits either;
//! from then on they hold what was set until an Apply or OK carries it
//! out. While network time is on (checked, applied or not) the date and
//! time controls are greyed, since the time server owns the clock.

use reprisekit::art::clockface::Face;
use reprisekit::controls::calendar::{self, Calendar, MONTHS};
use reprisekit::controls::combo::Combo;
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::controls::timefield::TimeField;
use reprisekit::controls::updown::UpDown;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::Align;
use reprisekit::theme::{Color, TextUnits};

/// The sheet's client size: 404x341 outside, less the frame.
pub const CLIENT: (i32, i32) = (398, 317);

pub const TABS: [&str; 2] = ["Date & Time", "Time Zone"];
pub const TAB_DATE: usize = 0;
pub const TAB_ZONE: usize = 1;

const YEARS: (i32, i32) = (1970, 2099);

/// The zone panel's inside, the size of the picture it holds.
const PANEL: Rect = Rect::new(19, 64, 360, 188);

/// The system clock's settings as timedated reports them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct System {
    /// The zone's name ("Europe/Budapest").
    pub zone: String,
    /// Network time synchronization is on.
    pub ntp: bool,
    /// A time server client is installed, so `ntp` can be turned on.
    pub can_ntp: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    /// 1..=12.
    pub month: u32,
    pub day: u32,
    pub h: u32,
    pub m: u32,
    pub s: u32,
}

/// What an Apply or OK asks the host to carry out; `None` leaves a
/// setting as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub zone: Option<String>,
    pub ntp: Option<bool>,
    /// A date and time in the new zone (or the current one).
    pub time: Option<Local>,
}

/// What the host does after an event.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    None,
    Redraw,
    Apply(Change),
    ApplyAndClose(Change),
    Close,
}

pub struct Sheet {
    pub sheet: PropSheet,
    applied: System,
    /// The zones the combo lists, in its order.
    zones: Vec<String>,
    /// The user changed the date or the time since the last Apply.
    edited: bool,
}

/// "America/Argentina/Buenos_Aires" as the panel shows it: the city
/// ("Buenos Aires") and the region above it ("America, Argentina").
pub fn place(zone: &str) -> (String, String) {
    let (region, city) = zone.rsplit_once('/').unwrap_or(("", zone));
    (
        city.replace('_', " "),
        region.replace('/', ", ").replace('_', " "),
    )
}

impl Sheet {
    /// `zones` is timedated's list; `now` the local time; `first_weekday`
    /// the locale's (0 Sunday); `twelve` whether it writes 12-hour time.
    pub fn new(
        system: &System,
        zones: &[String],
        now: Local,
        first_weekday: u32,
        twelve: bool,
    ) -> Sheet {
        let mut zones = zones.to_vec();
        if !zones.contains(&system.zone) {
            zones.push(system.zone.clone());
            zones.sort();
        }
        let pages = vec![
            date_page(now, first_weekday, twelve, system),
            zone_page(&zones, &system.zone),
        ];
        let mut s = Sheet {
            sheet: PropSheet::new(&TABS, pages, CLIENT),
            applied: system.clone(),
            zones,
            edited: false,
        };
        s.tick(now);
        s.write_zone();
        s.write_ntp();
        s
    }

    /// The clock moved on: the date and time follow it unless the user
    /// edited them; today's outline follows it always.
    pub fn tick(&mut self, now: Local) {
        let edited = self.edited;
        let page = &mut self.sheet.pages[TAB_DATE];
        if let Some(Control::Calendar(cal)) = page.item_mut("cal").map(|i| &mut i.control) {
            cal.today = Some((now.year, now.month, now.day));
            if !edited {
                cal.year = now.year;
                cal.month = now.month;
                cal.day = Some(now.day);
            }
        }
        if edited {
            return;
        }
        page.set_combo("month", now.month as usize - 1);
        if let Some(Control::UpDown(u)) = page.item_mut("year").map(|i| &mut i.control) {
            u.set(now.year);
        }
        if let Some(Control::Time(tf)) = page.item_mut("time").map(|i| &mut i.control) {
            tf.set(now.h, now.m, now.s);
        }
        self.write_clock();
    }

    pub fn shown(&self) -> Local {
        let page = &self.sheet.pages[TAB_DATE];
        let mut t = Local::default();
        if let Some(Control::Calendar(cal)) = page.item("cal").map(|i| &i.control) {
            (t.year, t.month, t.day) = (cal.year, cal.month, cal.day.unwrap_or(1));
        }
        if let Some(Control::Time(tf)) = page.item("time").map(|i| &i.control) {
            (t.h, t.m, t.s) = (tf.h, tf.m, tf.s);
        }
        t
    }

    fn pending_zone(&self) -> String {
        let i = self.sheet.pages[TAB_ZONE]
            .combo_selected("zone")
            .unwrap_or(0);
        self.zones.get(i).cloned().unwrap_or_default()
    }

    fn pending_ntp(&self) -> bool {
        self.sheet.pages[TAB_DATE].checked("ntp").unwrap_or(false)
    }

    /// What Apply would carry out now.
    pub fn change(&self) -> Change {
        let zone = self.pending_zone();
        let ntp = self.pending_ntp();
        Change {
            zone: (zone != self.applied.zone).then_some(zone),
            ntp: (ntp != self.applied.ntp).then_some(ntp),
            time: (self.edited && !ntp).then(|| self.shown()),
        }
    }

    pub fn dirty(&self) -> bool {
        self.change() != Change::default()
    }

    pub fn event(&mut self, ev: propsheet::Event) -> Outcome {
        match ev {
            propsheet::Event::None => Outcome::None,
            propsheet::Event::Redraw => Outcome::Redraw,
            propsheet::Event::Page(_, form::Event::Changed(id)) => {
                self.read(id);
                Outcome::Redraw
            }
            propsheet::Event::Page(..) => Outcome::Redraw,
            propsheet::Event::Apply if self.dirty() => {
                self.sheet.set_apply_enabled(false);
                Outcome::Apply(self.change())
            }
            propsheet::Event::Apply => Outcome::Redraw,
            propsheet::Event::Ok if self.dirty() => Outcome::ApplyAndClose(self.change()),
            propsheet::Event::Ok | propsheet::Event::Cancel => Outcome::Close,
        }
    }

    /// The host carried the change out and read `system` back: that is
    /// the new baseline, and the date and time follow the clock again.
    pub fn applied(&mut self, system: &System, now: Local) {
        self.applied = system.clone();
        self.edited = false;
        self.write_ntp();
        self.tick(now);
        self.sheet.set_apply_enabled(self.dirty());
    }

    /// The change failed: what the host read back is the baseline and
    /// the user's values stay pending with Apply live.
    pub fn unapplied(&mut self, system: &System) {
        self.applied = system.clone();
        self.sheet.set_apply_enabled(self.dirty());
    }

    /// Take control `id`'s new value.
    fn read(&mut self, id: &'static str) {
        let page = &mut self.sheet.pages[TAB_DATE];
        match id {
            "month" => {
                let m = page.combo_selected("month").unwrap_or(0) as u32 + 1;
                if let Some(Control::Calendar(cal)) = page.item_mut("cal").map(|i| &mut i.control) {
                    cal.set_month(m);
                }
                self.edited = true;
            }
            "year" => {
                let y = match page.item("year").map(|i| &i.control) {
                    Some(Control::UpDown(u)) => u.value(),
                    _ => YEARS.0,
                };
                if let Some(Control::Calendar(cal)) = page.item_mut("cal").map(|i| &mut i.control) {
                    cal.set_year(y);
                }
                self.edited = true;
            }
            "cal" | "time" => {
                self.edited = true;
                self.write_clock();
            }
            "ntp" => self.write_ntp(),
            "zone" => self.write_zone(),
            _ => {}
        }
        let dirty = self.dirty();
        self.sheet.set_apply_enabled(dirty);
    }

    /// Size the zone panel for text drawn at `u`: unlike a picture it grows with the page.
    pub fn fit(&mut self, u: TextUnits) {
        let grown = u.rect(PANEL);
        let Some(item) = self.sheet.pages[TAB_ZONE].item_mut("panel") else {
            return;
        };
        if (item.rect.width, item.rect.height) != (grown.width, grown.height) {
            item.rect = Rect {
                x: PANEL.x,
                y: PANEL.y,
                ..grown
            };
            item.control = panel_well(grown);
        }
    }

    fn write_clock(&mut self) {
        let page = &mut self.sheet.pages[TAB_DATE];
        let face = match page.item("time").map(|i| &i.control) {
            Some(Control::Time(tf)) => Face {
                h: tf.h,
                m: tf.m,
                s: tf.s,
            },
            _ => Face::default(),
        };
        if let Some(item) = page.item_mut("clock") {
            item.control = Control::Clock(face);
        }
    }

    fn write_ntp(&mut self) {
        let can = self.applied.can_ntp || self.applied.ntp;
        let page = &mut self.sheet.pages[TAB_DATE];
        page.set_enabled("ntp", can);
        let manual = !page.checked("ntp").unwrap_or(false);
        for id in ["month", "year", "cal", "time"] {
            page.set_enabled(id, manual);
        }
    }

    fn write_zone(&mut self) {
        let (city, region) = place(&self.pending_zone());
        let page = &mut self.sheet.pages[TAB_ZONE];
        page.set_text("city", &city);
        page.set_text("region", &region);
    }
}

fn date_page(now: Local, first_weekday: u32, twelve: bool, system: &System) -> Form {
    let mut items = vec![
        Item::group("date_group", Rect::new(16, 39, 183, 197), "Date"),
        Item::combo("month", 27, 58, 75, &MONTHS, 12),
        Item::new(
            "year",
            Rect::new(112, 58, 79, 21),
            Control::UpDown(UpDown::new(now.year, YEARS.0, YEARS.1).with_buddy()),
        ),
        Item::new(
            "cal",
            Rect::new(27, 90, calendar::W, calendar::H),
            Control::Calendar(Calendar::new(
                now.year,
                now.month,
                Some(now.day),
                first_weekday,
            )),
        ),
        Item::group("time_group", Rect::new(208, 39, 170, 197), "Time"),
        Item::new(
            "clock",
            Rect::new(225, 71, 132, 100),
            Control::Clock(Face::default()),
        ),
        Item::new(
            "time",
            Rect::new(249, 201, 103, 20),
            Control::Time(TimeField::new(now.h, now.m, now.s, twelve)),
        ),
        Item::check(
            "ntp",
            16,
            250,
            "Automatically &synchronize with an Internet time server",
        )
        .checked(system.ntp),
    ];
    if let Some(Item {
        control: Control::Combo(c),
        ..
    }) = items.get_mut(1)
    {
        c.selected = now.month as usize - 1;
    }
    Form::new(items)
}

/// The zone panel: a white well with a 2 px frame filling `r`.
fn panel_well(r: Rect) -> Control {
    let mut white = Canvas::new((r.width - 4) as u32, (r.height - 4) as u32);
    white.fill_rect(white.bounds(), Color::rgb(255, 255, 255));
    Control::Picture {
        canvas: Some(white),
        frame: 2,
    }
}

fn zone_page(zones: &[String], current: &str) -> Form {
    let labels: Vec<String> = zones.iter().map(|z| z.replace('_', " ")).collect();
    let rows: Vec<&str> = labels.iter().map(String::as_str).collect();
    let selected = zones.iter().position(|z| z == current).unwrap_or(0);
    let mid = PANEL.y + PANEL.height / 2;
    Form::new(vec![
        Item::new(
            "zone",
            Rect::new(18, 37, 362, reprisekit::controls::combo::COMBO_H),
            Control::Combo(Combo::new(&rows, selected, 16)),
        ),
        Item::new("panel", PANEL, panel_well(PANEL)),
        Item::aligned(
            "city",
            Rect::new(PANEL.x, mid - 14, PANEL.width, 13),
            "",
            Align::Center,
        ),
        Item::aligned(
            "region",
            Rect::new(PANEL.x, mid + 1, PANEL.width, 13),
            "",
            Align::Center,
        ),
        // Every zone's rules carry their daylight saving changes, and the
        // clock always follows them: there is nothing to turn off.
        Item::check(
            "dst",
            21,
            257,
            "Automatically adjust clock for daylight saving changes",
        )
        .checked(true)
        .enabled(false),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Local {
        Local {
            year: 2026,
            month: 9,
            day: 24,
            h: 23,
            m: 41,
            s: 24,
        }
    }

    fn system() -> System {
        System {
            zone: "Europe/Budapest".into(),
            ntp: false,
            can_ntp: true,
        }
    }

    fn zones() -> Vec<String> {
        ["America/New_York", "Europe/Budapest", "UTC"]
            .map(String::from)
            .to_vec()
    }

    fn sheet() -> Sheet {
        Sheet::new(&system(), &zones(), now(), 0, true)
    }

    /// Writes target/datetime-preview.png (both tabs side by side) for a
    /// visual check.
    #[test]
    #[ignore]
    fn render_sheet() {
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::{FontConfig, Theme};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut s = sheet();
        let (w, h) = CLIENT;
        let mut out = Canvas::new(2 * w as u32, h as u32);
        for tab in [TAB_DATE, TAB_ZONE] {
            s.sheet.tabs.open = tab;
            let mut c = Canvas::new(w as u32, h as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            propsheet::draw(&mut c, &text, &theme.paint(), &s.sheet, false, 11);
            out.blit_canvas(tab as i32 * w, 0, &c);
        }
        reprisekit::raster::write_preview(&out, "datetime-preview.png");
    }

    #[test]
    fn a_zone_names_its_city_and_region() {
        assert_eq!(
            place("America/Argentina/Buenos_Aires"),
            ("Buenos Aires".into(), "America, Argentina".into())
        );
        assert_eq!(place("UTC"), ("UTC".into(), String::new()));
    }

    #[test]
    fn the_date_and_time_follow_the_clock_until_edited() {
        let mut s = sheet();
        let later = Local { s: 25, ..now() };
        s.tick(later);
        assert_eq!(s.shown(), later);
        assert!(!s.dirty());
        // Stepping the time's hour is an edit: the clock stops moving it.
        let page = s.sheet.open_mut();
        page.focus_id("time");
        let ev = page.key(form::Key::Up);
        assert_eq!(ev, form::Event::Changed("time"));
        s.event(propsheet::Event::Page(TAB_DATE, ev));
        assert!(s.dirty());
        s.tick(Local { s: 40, ..now() });
        assert_eq!(s.shown().s, 25, "an edited time holds");
        assert_eq!(
            s.change().time.map(|t| t.h),
            Some(12),
            "11 PM stepped to 12 PM"
        );
        assert_eq!(s.change().zone, None);
    }

    #[test]
    fn the_month_and_year_move_the_grid() {
        let mut s = sheet();
        let page = &mut s.sheet.pages[TAB_DATE];
        page.set_combo("month", 1);
        s.event(propsheet::Event::Page(
            TAB_DATE,
            form::Event::Changed("month"),
        ));
        assert_eq!((s.shown().month, s.shown().day), (2, 24));
        let page = &mut s.sheet.pages[TAB_DATE];
        if let Some(Control::UpDown(u)) = page.item_mut("year").map(|i| &mut i.control) {
            u.set(2028);
        }
        s.event(propsheet::Event::Page(
            TAB_DATE,
            form::Event::Changed("year"),
        ));
        assert_eq!(s.shown().year, 2028);
        assert_eq!(
            s.change().time.map(|t| (t.year, t.month, t.day)),
            Some((2028, 2, 24))
        );
    }

    #[test]
    fn network_time_greys_the_date_and_time() {
        let mut s = sheet();
        s.sheet.pages[TAB_DATE].set_check("ntp", true);
        s.event(propsheet::Event::Page(
            TAB_DATE,
            form::Event::Changed("ntp"),
        ));
        let page = &s.sheet.pages[TAB_DATE];
        assert!(matches!(&page.item("cal").unwrap().control, Control::Calendar(c) if !c.enabled));
        assert!(matches!(&page.item("time").unwrap().control, Control::Time(t) if !t.enabled));
        assert!(matches!(&page.item("year").unwrap().control, Control::UpDown(u) if !u.enabled));
        assert_eq!(
            s.change(),
            Change {
                ntp: Some(true),
                ..Change::default()
            }
        );
        // A machine with no time server greys the check box itself.
        let s = Sheet::new(
            &System {
                can_ntp: false,
                ..system()
            },
            &zones(),
            now(),
            0,
            true,
        );
        assert!(matches!(
            &s.sheet.pages[TAB_DATE].item("ntp").unwrap().control,
            Control::Check { enabled: false, .. }
        ));
    }

    #[test]
    fn a_new_zone_is_applied_and_becomes_the_baseline() {
        let mut s = sheet();
        s.sheet.pages[TAB_ZONE].set_combo("zone", 2);
        s.event(propsheet::Event::Page(
            TAB_ZONE,
            form::Event::Changed("zone"),
        ));
        assert!(matches!(
            &s.sheet.pages[TAB_ZONE].item("city").unwrap().control,
            Control::Label { text, .. } if text == "UTC"
        ));
        let out = s.event(propsheet::Event::Apply);
        assert_eq!(
            out,
            Outcome::Apply(Change {
                zone: Some("UTC".into()),
                ..Change::default()
            })
        );
        s.applied(
            &System {
                zone: "UTC".into(),
                ..system()
            },
            now(),
        );
        assert!(!s.dirty());
        assert_eq!(s.event(propsheet::Event::Ok), Outcome::Close);
    }

    #[test]
    fn the_zone_panel_grows_with_the_text() {
        let mut s = sheet();
        let u = TextUnits::new(17, 11);
        s.fit(u);
        let page = &s.sheet.pages[TAB_ZONE];
        page.drawn_with(&reprisekit::look::Paint {
            text: u,
            ..reprisekit::theme::Theme::default().paint()
        });
        assert_eq!(page.rect("panel"), Some(u.rect(PANEL)));
        assert!(matches!(
            &page.item("panel").unwrap().control,
            Control::Picture { canvas: Some(c), .. } if (c.width, c.height) == (553, 286)
        ));
        s.fit(TextUnits::ONE);
        assert_eq!(s.sheet.pages[TAB_ZONE].item("panel").unwrap().rect, PANEL);
    }

    #[test]
    fn a_zone_timedated_did_not_list_is_still_in_the_combo() {
        let s = Sheet::new(
            &System {
                zone: "Etc/GMT+5".into(),
                ..system()
            },
            &zones(),
            now(),
            0,
            true,
        );
        assert_eq!(s.pending_zone(), "Etc/GMT+5");
        assert!(!s.dirty());
    }
}
