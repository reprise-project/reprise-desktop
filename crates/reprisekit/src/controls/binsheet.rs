//! Recycle Bin Properties: a 338x376 sheet with a Global tab and one tab per drive with a
//! Recycle Bin, over [`Settings`] the host reads from and writes to `[recycle_bin]`.
//!
//! Global holds two radios: "Use one setting for all drives" makes its group
//! live and greys every drive's tab (which shows the global setting);
//! "Configure drives independently" greys the group and makes each drive's
//! tab live. A setting is "Do not move files to the Recycle Bin" (Delete
//! erases on that drive) and the trackbar, the trash's cap as 0 to 100% of
//! the drive, greyed while files are not moved. "Display delete confirmation
//! dialog" and "Delete items older than N days" are global.

use crate::controls::copyprogress::bytes_text;
use crate::controls::form::{Control, Event as FormEvent, Form, Item};
use crate::controls::propsheet::{self, PropSheet};
use crate::controls::updown::{self, UpDown};
use crate::raster::Rect;
use crate::text::Align;
use crate::trackbar::{self, TrackBar};
use std::collections::BTreeMap;

/// The most "Delete items older than" takes.
pub const MAX_DAYS: u16 = 3650;

/// A drive's setting: erase on Delete, or keep a trash capped at `max_percent` of the drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub remove_immediately: bool,
    pub max_percent: u8,
}

impl Rule {
    /// The cap in bytes on a drive of `size` bytes.
    pub fn cap(self, size: u64) -> u64 {
        (u128::from(size) * u128::from(self.max_percent.min(100)) / 100) as u64
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub confirm: bool,
    /// Each drive follows its entry in `drives` (the global rule where it has none).
    pub independent: bool,
    pub global: Rule,
    /// Each drive's own rule by [`Drive::key`], kept while `independent` is off.
    pub drives: BTreeMap<String, Rule>,
    /// 0 keeps deleted items however old.
    pub older_than_days: u16,
}

impl Settings {
    pub fn rule(&self, key: &str) -> Rule {
        match self.independent {
            true => self.drives.get(key).copied().unwrap_or(self.global),
            false => self.global,
        }
    }
}

/// A drive's tab: its settings key, label and size in bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drive {
    pub key: String,
    pub label: String,
    pub size: u64,
}

/// What a sheet event asks of the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    /// Cancel: close without saving.
    Close,
    /// OK (`close`) or Apply: save `settings`.
    Save {
        settings: Settings,
        close: bool,
    },
}

/// The sheet and what its boxes say against what was last saved, so Apply greys when they agree.
#[derive(Clone, Debug, PartialEq)]
pub struct BinSheet {
    pub pending: Settings,
    pub applied: Settings,
    pub drives: Vec<Drive>,
    /// The days box's number, kept while the box is unticked and `older_than_days` is 0.
    days: u16,
    pub sheet: PropSheet,
}

pub const SIZE: (i32, i32) = (338, 376);
const STOPS: usize = 101;
const TICK_EVERY: usize = 10;
/// What the days box offers before a number was picked.
const DEFAULT_DAYS: u16 = 30;

/// The "Do not move files" pair, trackbar, percentage and caption (one label
/// per line, at most two) both pages share, with the page's own x for each
/// (inside the Global group box; a drive's are 9 px further left). `live` greys the lot when off; the
/// trackbar and labels also grey while files are not moved.
#[allow(clippy::too_many_arguments)]
fn size_controls(
    check_x: i32,
    bar_x: i32,
    pct_x: i32,
    caption_x: i32,
    caption: &[&str],
    rule: Rule,
    live: bool,
) -> Vec<Item> {
    let mut bar = TrackBar::new(usize::from(rule.max_percent), STOPS).paged(TICK_EVERY, TICK_EVERY);
    let sized = live && !rule.remove_immediately;
    bar.enabled = sized;
    let mut items = vec![
        Item::check(
            "no_bin",
            check_x,
            93,
            &tr!("binsheet-do-not-move-files-to-the"),
        )
        .checked(rule.remove_immediately)
        .enabled(live)
        .wraps(),
        Item::label(
            "no_bin_2",
            Rect::new(check_x, 106, 250, 13),
            &tr!("binsheet-remove-files-immediately-when-deleted"),
        )
        .check_wrap()
        .enabled(live),
        Item::new(
            "size",
            Rect::new(bar_x, 143, 211, trackbar::HEIGHT),
            Control::TrackBar(bar),
        ),
        Item::aligned(
            "pct",
            Rect::new(pct_x, 176, 41, 13),
            &format!("{}%", rule.max_percent),
            Align::Center,
        )
        .enabled(sized),
    ];
    for (line, (id, text)) in ["size_l", "size_l2"].into_iter().zip(caption).enumerate() {
        let y = 197 + 13 * line as i32;
        items.push(Item::label(id, Rect::new(caption_x, y, 265, 13), text).enabled(sized));
    }
    items
}

impl BinSheet {
    /// Opened on Global.
    pub fn new(settings: Settings, drives: Vec<Drive>) -> BinSheet {
        let days = match settings.older_than_days {
            0 => DEFAULT_DAYS,
            d => d,
        };
        let mut s = BinSheet {
            pending: settings.clone(),
            applied: settings,
            drives,
            days,
            sheet: PropSheet::new(&[] as &[&str], Vec::new(), SIZE),
        };
        s.sheet = s.build();
        s
    }

    fn page_global(&self) -> Form {
        let p = &self.pending;
        let mut items = vec![
            Item::radio(
                "independent",
                31,
                42,
                &tr!("binsheet-configure-drives-independently"),
                "drives",
            )
            .checked(p.independent),
            Item::group("group", Rect::new(21, 70, 296, 161), ""),
            // The second radio and its label sit on the group's top line, which breaks
            // for 156 px from x 30 in every look.
            Item::new("gap", Rect::new(30, 70, 156, 2), Control::Blank),
            Item::radio(
                "all",
                31,
                64,
                &tr!("binsheet-use-one-setting-for-all-drives"),
                "drives",
            )
            .checked(!p.independent),
        ];
        items.extend(size_controls(
            30,
            54,
            139,
            46,
            &[
                &tr!("binsheet-maximum-size-of-recycle-bin"),
                &tr!("binsheet-percent-of-each-drive"),
            ],
            p.global,
            !p.independent,
        ));
        items.push(
            Item::check(
                "confirm",
                30,
                242,
                &tr!("binsheet-display-delete-confirmation-dialog"),
            )
            .checked(p.confirm),
        );
        let aging = p.older_than_days > 0;
        let mut days = UpDown::new(i32::from(self.days), 1, i32::from(MAX_DAYS));
        days.enabled = aging;
        items.extend([
            Item::check("aging", 30, 266, &tr!("binsheet-delete-items-older-than")).checked(aging),
            Item::new(
                "days",
                Rect::new(190, 262, updown::WIDTH, updown::HEIGHT),
                Control::UpDown(days),
            ),
            Item::label("days_l", Rect::new(246, 266, 40, 13), "days").enabled(aging),
        ]);
        Form::new(items)
    }

    fn page_drive(&self, d: &Drive) -> Form {
        let rule = self.pending.rule(&d.key);
        let reserved = match rule.remove_immediately {
            true => 0,
            false => rule.cap(d.size),
        };
        let mut items = vec![
            Item::label(
                "size_of_l",
                Rect::new(21, 41, 120, 13),
                &tr!("binsheet-size-of-drive"),
            ),
            Item::label("size_of", Rect::new(153, 41, 160, 13), &bytes_text(d.size)),
            Item::label(
                "reserved_l",
                Rect::new(21, 63, 120, 13),
                &tr!("binsheet-space-reserved"),
            ),
            Item::label(
                "reserved",
                Rect::new(153, 63, 160, 13),
                &bytes_text(reserved),
            ),
        ];
        items.extend(size_controls(
            21,
            45,
            130,
            37,
            &[&tr!("binsheet-maximum-size-of-recycle-bin-percent")],
            rule,
            self.pending.independent,
        ));
        Form::new(items)
    }

    fn pages(&self) -> Vec<Form> {
        std::iter::once(self.page_global())
            .chain(self.drives.iter().map(|d| self.page_drive(d)))
            .collect()
    }

    fn dirty(&self) -> bool {
        self.pending != self.applied
    }

    /// Global's rule, or drive `page - 1`'s own, which starts as a copy of the global one.
    fn rule_mut(&mut self, page: usize) -> Option<&mut Rule> {
        if page == 0 {
            return Some(&mut self.pending.global);
        }
        let key = self.drives.get(page - 1)?.key.clone();
        let global = self.pending.global;
        Some(self.pending.drives.entry(key).or_insert(global))
    }

    fn build(&self) -> PropSheet {
        let labels: Vec<String> = std::iter::once(tr!("binsheet-global"))
            .chain(self.drives.iter().map(|d| d.label.clone()))
            .collect();
        let mut sheet = PropSheet::new(&labels, self.pages(), SIZE);
        sheet.set_apply_enabled(self.dirty());
        sheet
    }

    /// The pages built again from the boxes, keeping the focus and the pointer: the new pages
    /// start with a bare pointer, so a just-released box would draw Normal until the next motion
    /// and a dragged thumb would let go.
    fn rebuild(&mut self) {
        let old = self.sheet.open().clone();
        let focus = old.focus().map(|i| old.items[i].id);
        let (pages, dirty) = (self.pages(), self.dirty());
        self.sheet.set_pages(pages);
        self.sheet.set_apply_enabled(dirty);
        let page = self.sheet.open_mut();
        if let Some(id) = focus {
            page.focus_id(id);
        }
        page.carry_pointer(&old);
    }

    /// Shows `drives` as the tabs after Global, keeping the open tab when its drive stays and what
    /// the boxes say. False when the tabs were already these; else the host fits the tabs again
    /// ([`PropSheet::fit_tabs`]).
    pub fn show_drives(&mut self, drives: Vec<Drive>) -> bool {
        if drives == self.drives {
            return false;
        }
        let open = self.sheet.tabs.open;
        let open_key = (open.checked_sub(1))
            .and_then(|i| self.drives.get(i))
            .map(|d| d.key.clone());
        let reopen =
            (open_key.and_then(|k| drives.iter().position(|d| d.key == k))).map_or(0, |i| i + 1);
        self.drives = drives;
        self.sheet = self.build();
        self.sheet.open_tab(reopen);
        true
    }

    /// The sheet's event: OK and Apply hand the settings over, Apply keeping the sheet open.
    pub fn event(&mut self, ev: propsheet::Event) -> Outcome {
        match ev {
            propsheet::Event::Redraw => Outcome::Redraw,
            propsheet::Event::Cancel => Outcome::Close,
            propsheet::Event::Ok => Outcome::Save {
                settings: self.pending.clone(),
                close: true,
            },
            propsheet::Event::Apply => {
                self.applied = self.pending.clone();
                self.rebuild();
                Outcome::Save {
                    settings: self.pending.clone(),
                    close: false,
                }
            }
            propsheet::Event::Page(page, FormEvent::Changed(id)) => self.changed(page, id),
            propsheet::Event::Page(..) => Outcome::Redraw,
            propsheet::Event::None | propsheet::Event::Link(_) | propsheet::Event::Search => {
                Outcome::None
            }
        }
    }

    fn changed(&mut self, page: usize, id: &str) -> Outcome {
        let Some(form) = self.sheet.pages.get(page) else {
            return Outcome::None;
        };
        let checked = |id: &str| form.checked(id) == Some(true);
        match id {
            "independent" | "all" => self.pending.independent = checked("independent"),
            "confirm" => self.pending.confirm = checked("confirm"),
            "aging" => {
                self.pending.older_than_days = match checked("aging") {
                    true => self.days,
                    false => 0,
                }
            }
            "days" => {
                if let Some(Control::UpDown(u)) = form.item("days").map(|i| &i.control) {
                    self.days = u.value().clamp(1, i32::from(MAX_DAYS)) as u16;
                }
                if self.pending.older_than_days > 0 {
                    self.pending.older_than_days = self.days;
                }
                // The pages stay: rebuilding them would drop the digits being typed.
                let dirty = self.dirty();
                self.sheet.set_apply_enabled(dirty);
                return Outcome::Redraw;
            }
            "no_bin" => {
                let on = checked("no_bin");
                if let Some(r) = self.rule_mut(page) {
                    r.remove_immediately = on;
                }
            }
            "size" => {
                let pos = form.track_pos("size").unwrap_or(0);
                if let Some(r) = self.rule_mut(page) {
                    r.max_percent = pos.min(100) as u8;
                }
            }
            _ => return Outcome::Redraw,
        }
        self.rebuild();
        Outcome::Redraw
    }
}

#[cfg(test)]
mod tests;
