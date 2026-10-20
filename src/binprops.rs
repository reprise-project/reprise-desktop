//! Recycle Bin Properties: a 344x400 sheet with a Global tab and one tab per
//! drive with a Recycle Bin. A pure `reprisekit::controls::propsheet` model
//! over a [`reprisefs::bincap::Policy`]; `dialogview.rs` opens it and writes
//! the result into `[recycle_bin]` in `config.toml`, which every host's
//! Delete reads (`reprisefs::trash::Bins`).
//!
//! Global holds two radios: "Use one setting for all drives" makes its group
//! live and greys every drive's tab (which shows the global setting);
//! "Configure drives independently" greys the group and makes each drive's
//! tab live. A setting is "Do not move files to the Recycle Bin" (Delete
//! erases on that drive) and the trackbar, the trash's cap as 0 to 100% of
//! the drive, greyed while files are not moved. "Display delete confirmation
//! dialog" is global.

use crate::dialog::{Dialog, FormDialog, Kind, Outcome, Placement, Style};
use crate::raster::Rect;
use crate::text::Align;
use reprisefs::bincap::{Policy, Rule};
use reprisekit::controls::form::{Control, Event, Form, Item};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::trackbar::TrackBar;
use serde::Deserialize;
use std::path::Path;

/// Settings from the state file, before `[recycle_bin]`: read once and moved into `config.toml` (`dialogview::setup`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct OldBinSettings {
    pub no_bin: bool,
    pub no_confirm: bool,
}

impl OldBinSettings {
    pub fn policy(self) -> Policy {
        let mut p = Policy {
            confirm: !self.no_confirm,
            ..Policy::default()
        };
        p.global.remove_immediately = self.no_bin;
        p
    }
}

/// A drive's tab: its `[recycle_bin.drives]` key, label and size in bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drive {
    pub key: String,
    pub label: String,
    pub size: u64,
}

/// What the boxes say now against what was last written, so Apply greys when they agree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinSheet {
    pub pending: Policy,
    pub applied: Policy,
    pub drives: Vec<Drive>,
}

const SIZE: (i32, i32) = (338, 376);
const STOPS: usize = 101;
const TICK_EVERY: usize = 10;

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
            "Do not move files to the &Recycle Bin.",
        )
        .checked(rule.remove_immediately)
        .enabled(live)
        .wraps(),
        Item::label(
            "no_bin_2",
            Rect::new(check_x, 106, 250, 13),
            "Remove files immediately when deleted",
        )
        .check_wrap()
        .enabled(live),
        Item::new(
            "size",
            Rect::new(bar_x, 143, 211, reprisekit::trackbar::HEIGHT),
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
    pub fn new(settings: Policy, drives: Vec<Drive>) -> BinSheet {
        BinSheet {
            pending: settings.clone(),
            applied: settings,
            drives,
        }
    }

    fn page_global(&self) -> Form {
        let p = &self.pending;
        let mut items = vec![
            Item::radio(
                "independent",
                31,
                42,
                "&Configure drives independently",
                "drives",
            )
            .checked(p.independent),
            Item::group("group", Rect::new(21, 70, 296, 161), ""),
            // The second radio and its label sit on the group's top line, which breaks
            // for 156 px from x 30 in every look.
            Item::new("gap", Rect::new(30, 70, 156, 2), Control::Blank),
            Item::radio("all", 31, 64, "&Use one setting for all drives", "drives")
                .checked(!p.independent),
        ];
        items.extend(size_controls(
            30,
            54,
            139,
            46,
            &["&Maximum size of Recycle Bin", "(percent of each drive)"],
            p.global,
            !p.independent,
        ));
        items.push(
            Item::check("confirm", 30, 242, "&Display delete confirmation dialog")
                .checked(p.confirm),
        );
        Form::new(items)
    }

    fn page_drive(&self, d: &Drive) -> Form {
        let rule = self.pending.rule(&d.key);
        let reserved = match rule.remove_immediately {
            true => 0,
            false => rule.cap(d.size),
        };
        let mut items = vec![
            Item::label("size_of_l", Rect::new(21, 41, 120, 13), "Size of drive:"),
            Item::label("size_of", Rect::new(153, 41, 160, 13), &short_size(d.size)),
            Item::label("reserved_l", Rect::new(21, 63, 120, 13), "Space reserved:"),
            Item::label(
                "reserved",
                Rect::new(153, 63, 160, 13),
                &short_size(reserved),
            ),
        ];
        items.extend(size_controls(
            21,
            45,
            130,
            37,
            &["&Maximum size of Recycle Bin (percent of drive)"],
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

    pub fn sheet(&self) -> PropSheet {
        let labels: Vec<&str> = std::iter::once("Global")
            .chain(self.drives.iter().map(|d| d.label.as_str()))
            .collect();
        let mut sheet = PropSheet::new(&labels, self.pages(), SIZE);
        sheet.set_apply_enabled(self.dirty());
        sheet
    }
}

pub use reprisefs::props::short_size;

/// The home trash's drive, as "Local Disk (/)" by its mount, sized by `statvfs`.
pub fn home_drive(bins: &reprisefs::trash::Bins) -> Option<Drive> {
    let top = bins.home_top()?;
    Some(Drive {
        key: reprisefs::bincap::HOME.into(),
        label: format!("Local Disk ({})", top.display()),
        size: fs_size(top)?,
    })
}

/// The tab of `top`, a drive of `reprisefs::places::mounted_drives`, by folder name: None for
/// the home trash's drive and a drive with no Recycle Bin that cannot make one. Sized by
/// `statvfs`, keyed as `[recycle_bin.drives]`. Looks inside `top`, so it runs on a file job.
pub fn drive_tab(bins: &reprisefs::trash::Bins, top: &Path) -> Option<Drive> {
    if bins.home_top() == Some(top) {
        return None;
    }
    bins.bin_for(&top.join("x"), false)?;
    let size = fs_size(top)?;
    let label = top
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| top.display().to_string());
    Some(Drive {
        key: bins.drive_key(top),
        label,
        size,
    })
}

/// Shows `drives` as the mounted drives' tabs after the home drive's, keeping the open tab
/// when its drive stays and what the boxes say. False when the tabs were already these.
pub fn show_drives(fd: &mut FormDialog, drives: Vec<Drive>) -> bool {
    let open = fd.open_index();
    let Kind::RecycleBin(sheet) = &mut fd.kind else {
        return false;
    };
    let home = (sheet.drives.iter()).filter(|d| d.key == reprisefs::bincap::HOME);
    let all: Vec<Drive> = home.cloned().chain(drives).collect();
    if all == sheet.drives {
        return false;
    }
    let open_key = (open.checked_sub(1))
        .and_then(|i| sheet.drives.get(i))
        .map(|d| d.key.clone());
    let reopen = (open_key.and_then(|k| all.iter().position(|d| d.key == k))).map_or(0, |i| i + 1);
    sheet.drives = all;
    let sheet = sheet.clone();
    *fd = FormDialog::from_sheet(sheet.sheet(), Kind::RecycleBin(sheet));
    if let Some(s) = fd.sheet_mut() {
        s.open_tab(reopen);
    }
    true
}

fn fs_size(path: &std::path::Path) -> Option<u64> {
    reprisefs::places::statvfs_totals(path).map(|(total, _)| total)
}

/// Opened on Global at `at` (the bin's cell). `icon` is the bin's desktop icon name, shown by a styled caption and Alt+Tab.
pub fn dialog(sheet: BinSheet, at: (i32, i32), icon: &str) -> Dialog {
    let form = FormDialog::from_sheet(sheet.sheet(), Kind::RecycleBin(sheet));
    Dialog {
        help: true,
        icon: Some(icon.into()),
        ..Dialog::new(
            "Recycle Bin Properties",
            Style::Form(Box::new(form)),
            Placement::At(at.0, at.1),
        )
    }
}

fn rebuild(fd: &mut FormDialog) {
    let Kind::RecycleBin(sheet) = &fd.kind else {
        return;
    };
    let focus_id = fd
        .open_page()
        .and_then(|f| f.focus().map(|i| f.items[i].id));
    // The new pages start with a bare pointer: carry the old open page's pointer
    // over, or a just-released box draws Normal until the next motion and a dragged thumb lets go.
    let old_form = fd.open_page().cloned();
    let (pages, dirty) = (sheet.pages(), sheet.dirty());
    if let Some(s) = fd.sheet_mut() {
        s.set_pages(pages);
        s.set_apply_enabled(dirty);
    }
    if let (Some(id), Some(f)) = (focus_id, fd.open_page_mut()) {
        f.focus_id(id);
    }
    if let (Some(old), Some(f)) = (&old_form, fd.open_page_mut()) {
        f.carry_pointer(old);
    }
}

/// OK and Apply hand the settings to the host; Apply keeps the sheet open.
pub fn event(fd: &mut FormDialog, page: usize, ev: Event) -> Outcome {
    let form = fd.body.pages().get(page).cloned();
    let Kind::RecycleBin(sheet) = &mut fd.kind else {
        return Outcome::Nothing;
    };
    match ev {
        Event::None => Outcome::Nothing,
        Event::Redraw => Outcome::Redraw,
        Event::Cancel | Event::Pressed(propsheet::CANCEL) => Outcome::Close,
        Event::Default | Event::Pressed(propsheet::OK) => Outcome::BinSettings {
            settings: sheet.pending.clone(),
            close: true,
        },
        Event::Pressed(propsheet::APPLY) => {
            sheet.applied = sheet.pending.clone();
            let out = Outcome::BinSettings {
                settings: sheet.pending.clone(),
                close: false,
            };
            rebuild(fd);
            out
        }
        Event::Pressed(_) => Outcome::Nothing,
        Event::Changed(id) => {
            let Some(form) = form else {
                return Outcome::Nothing;
            };
            let checked = |id: &str| form.checked(id) == Some(true);
            match id {
                "independent" | "all" => sheet.pending.independent = checked("independent"),
                "confirm" => sheet.pending.confirm = checked("confirm"),
                "no_bin" => {
                    if let Some(r) = sheet.rule_mut(page) {
                        r.remove_immediately = checked("no_bin");
                    }
                }
                "size" => {
                    let pos = form.track_pos("size").unwrap_or(0);
                    if let Some(r) = sheet.rule_mut(page) {
                        r.max_percent = pos.min(100) as u8;
                    }
                }
                _ => return Outcome::Redraw,
            }
            rebuild(fd);
            Outcome::Redraw
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialog::DialogLayout;
    use crate::text::TextRenderer;
    use crate::theme::{FontConfig, Theme};
    use reprisekit::controls::form::Key;

    #[test]
    fn the_old_state_file_settings_become_a_policy_for_every_drive() {
        let p = OldBinSettings {
            no_bin: true,
            no_confirm: true,
        }
        .policy();
        assert!(!p.confirm && !p.independent);
        assert!(p.global.remove_immediately);
        assert_eq!(p.global.max_percent, 10);
        assert_eq!(OldBinSettings::default().policy(), Policy::default());
    }

    fn drive(key: &str, label: &str) -> Drive {
        Drive {
            key: key.into(),
            label: label.into(),
            size: 21_365_735_424,
        }
    }

    fn sheet() -> BinSheet {
        BinSheet::new(Policy::default(), vec![drive("home", "Local Disk (/home)")])
    }

    #[test]
    fn a_landing_drive_tab_keeps_the_home_tab_the_open_tab_and_the_boxes() {
        let mut d = dialog(sheet(), (0, 0), "user-trash");
        let Style::Form(fd) = &mut d.style else {
            panic!()
        };
        assert!(show_drives(fd, vec![drive("b", "B")]));
        fd.sheet_mut().unwrap().open_tab(2);
        let Kind::RecycleBin(s) = &mut fd.kind else {
            panic!()
        };
        s.pending.confirm = false;
        assert!(show_drives(fd, vec![drive("a", "A"), drive("b", "B")]));
        assert!(!show_drives(fd, vec![drive("a", "A"), drive("b", "B")]));
        let labels = &fd.sheet().unwrap().tabs.labels;
        assert_eq!(labels, &["Global", "Local Disk (/home)", "A", "B"]);
        assert_eq!(fd.open_index(), 3, "B's tab stays open");
        let Kind::RecycleBin(s) = &fd.kind else {
            panic!()
        };
        assert!(!s.pending.confirm, "what the boxes said");
        fd.sheet_mut().unwrap().open_tab(2);
        assert!(show_drives(fd, vec![drive("b", "B")]));
        assert_eq!(fd.open_index(), 0, "A's tab went: Global opens");
    }

    #[test]
    fn a_drive_gets_a_tab_when_it_has_a_bin_or_can_make_one() {
        use std::os::unix::fs::PermissionsExt;
        let t = std::env::temp_dir().join(format!("reprise-bin-tabs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let (stick, shut) = (t.join("STICK"), t.join("SHUT"));
        std::fs::create_dir_all(&stick).unwrap();
        std::fs::create_dir_all(&shut).unwrap();
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o555)).unwrap();
        let bins = reprisefs::trash::Bins::new(t.join("home"), vec![stick.clone(), shut.clone()]);
        let tab = drive_tab(&bins, &stick).expect("STICK can make a bin");
        assert_eq!(tab.label, "STICK");
        assert!(tab.size > 0);
        assert_eq!(drive_tab(&bins, &shut), None, "SHUT cannot");
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&t);
    }

    /// The sheet carries the bin's icon for a styled caption and Alt+Tab; its Classic body has no place for it.
    #[test]
    fn the_sheet_carries_the_bins_icon() {
        let d = dialog(sheet(), (0, 0), "user-trash");
        assert_eq!(d.icon.as_deref(), Some("user-trash"));
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        assert_eq!(DialogLayout::compute(&theme, &text, &d).icon, None);
    }

    #[test]
    fn every_page_fits_its_text_in_every_look_and_size() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            let s = sheet();
            let mut pages = vec![("global".to_string(), s.page_global())];
            pages.extend(s.drives.iter().map(|d| (d.key.clone(), s.page_drive(d))));
            pages
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    /// "Do not move files"'s second line starts a pixel right of the first in
    /// Classic and under it in Blue and Glass, 16 px past the box. Its "D" tops
    /// out 3 px below the box's top in Classic and 2 px in Blue and Glass, and
    /// the second line's "R" 13 px below the "D". The group's top line runs to
    /// x 29 and on from x 186 in every look.
    #[test]
    fn the_do_not_move_second_line_follows_the_label_in_every_look() {
        if !reprisekit::text::test_tahoma() {
            return;
        }
        use reprisekit::interface::Interface;
        let text = TextRenderer::load(&FontConfig::default());
        let page = sheet().page_global();
        let ink = |c: &reprisekit::raster::Canvas, r: Rect| {
            (r.x..r.right()).find(|&x| (r.y..r.bottom()).any(|y| c.get(x, y) & 0xFF < 0xA0))
        };
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = reprisekit::interfaces::theme_for(i);
            let mut c = reprisekit::raster::Canvas::new(340, 260);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            reprisekit::controls::form::draw(&mut c, &text, &theme.paint(), &page, false, 11);
            let first = ink(&c, Rect::new(44, 93, 30, 13)).expect("the first line");
            let second = ink(&c, Rect::new(44, 106, 30, 13)).expect("the second line");
            let step = match i {
                Interface::Classic => 1,
                _ => 0,
            };
            assert_eq!(second, first + step, "{i:?}: the second line {step} px in");
            if i != Interface::Classic {
                assert_eq!(second, 30 + 16, "{i:?}: the second line 16 px past the box");
            }
            let top = |x: i32, y: std::ops::Range<i32>| {
                y.into_iter()
                    .find(|&y| (x..x + 6).any(|x| c.get(x, y) & 0xFF < 0xA0))
            };
            let (d_top, r_top) = match i {
                Interface::Classic => (96, 109),
                _ => (95, 108),
            };
            assert_eq!(top(first, 88..104), Some(d_top), "{i:?}: the D's top row");
            assert_eq!(top(second, 104..120), Some(r_top), "{i:?}: the R's top row");
            let line = c.get(250, 70);
            assert_eq!(c.get(29, 70), line, "{i:?}: the line up to x 29");
            assert_eq!(c.get(186, 70), line, "{i:?}: the line on from x 186");
            assert_ne!(c.get(185, 70), line, "{i:?}: the break up to x 185");
        }
    }

    /// The live trackbar's thumb sits at the policy's percentage (10% on the
    /// second tick, 50% on the sixth) with the percentage in the text colour;
    /// greyed with its group, in the shadow colour.
    #[test]
    fn the_trackbar_and_its_percentage_follow_the_policy() {
        if !reprisekit::text::test_tahoma() {
            return;
        }
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let draw = |s: &BinSheet| {
            let mut c = reprisekit::raster::Canvas::new(340, 260);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            reprisekit::controls::form::draw(
                &mut c,
                &text,
                &theme.paint(),
                &s.page_global(),
                false,
                11,
            );
            c
        };
        let page = sheet().page_global();
        let bar = page.item("size").unwrap();
        let lay = match &bar.control {
            Control::TrackBar(t) => {
                reprisekit::trackbar::layout(t, bar.rect.x, bar.rect.y, bar.rect.width)
            }
            _ => panic!(),
        };
        assert_eq!(
            lay.thumb.x + lay.thumb.width / 2,
            lay.ticks[10],
            "10% on the second tick"
        );
        // The thumb covers the channel where it sits: 10% and 50% differ on both ticks' columns.
        let mut s = sheet();
        let ten = draw(&s);
        s.pending.global.max_percent = 50;
        let c = draw(&s);
        let row = lay.channel.y + 1;
        for x in [lay.ticks[10], lay.ticks[50]] {
            assert_ne!(ten.get(x, row), c.get(x, row), "x {x}");
        }
        assert_eq!(ten.get(lay.ticks[90], row), c.get(lay.ticks[90], row));
        let ink = |c: &reprisekit::raster::Canvas, colour: u32| {
            (176..189).any(|y| (139..180).any(|x| c.get(x, y) == colour))
        };
        assert!(
            ink(&c, theme.colors.button_text.argb()),
            "the live percentage"
        );
        s.pending.independent = true;
        let c = draw(&s);
        assert!(
            !ink(&c, theme.colors.button_text.argb()),
            "greyed with its group"
        );
        assert!(ink(&c, theme.colors.button_shadow.argb()));
    }

    /// The areas are 1024x768 and 800x600 outputs over a 40 px taskbar.
    #[test]
    fn a_sheet_taller_than_its_output_keeps_to_it_and_scrolls_its_page() {
        use crate::dialog::DialogHit;
        use reprisekit::interface::Interface;
        const AREA: (i32, i32) = (800, 560);
        let text = TextRenderer::load(&FontConfig::default());
        for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let one = reprisekit::interfaces::theme_for(look);
            let big = one.with_text_size(200);
            let px = big.fonts.size_px;
            for area in [(1024, 728), AREA] {
                let mut d = dialog(sheet(), (0, 0), "user-trash-full");
                let grown = DialogLayout::compute(&big, &text, &{
                    let mut g = d.clone();
                    g.fit(&big, &text);
                    g
                });
                assert!(grown.size.1 > area.1, "{look:?}: taller than {area:?}");
                d.fit_in(&big, &text, area);
                let l = DialogLayout::compute(&big, &text, &d);
                assert!(
                    l.size.0 <= area.0 && l.size.1 <= area.1,
                    "{look:?}: {:?} inside {area:?}",
                    l.size
                );
            }
            let mut d = dialog(sheet(), (0, 0), "user-trash-full");
            d.fit_in(&big, &text, AREA);
            let l = DialogLayout::compute(&big, &text, &d);
            let sl = d.form().unwrap().sheet().unwrap().layout(&text, px);
            let bar = sl.bar.expect("the page scrolls");
            for b in [sl.ok, sl.cancel, sl.apply] {
                assert!(
                    b.y >= bar.bottom() && b.bottom() < l.size.1,
                    "{look:?}: {b:?}"
                );
            }
            let confirm = |d: &Dialog| d.form().unwrap().pages()[0].rect("confirm").unwrap();
            let before = confirm(&d);
            let (x, y) = (bar.x + bar.width / 2, bar.bottom() - 2);
            let hit = l.hit(&d, x, y, &text, px);
            assert!(
                matches!(hit, Some(DialogHit::PageBar(_))),
                "{look:?}: {hit:?}"
            );
            assert_eq!(d.click(hit), Outcome::Redraw);
            assert!(confirm(&d).y < before.y, "{look:?}: the page steps down");
            assert!(d.ticking() && d.holding_page_bar(), "{look:?}");
            assert_eq!(d.release(hit), Outcome::Redraw);
            assert!(!d.ticking() && !d.holding_page_bar(), "{look:?}");
            let stepped = confirm(&d);
            assert_eq!(
                d.wheel(None, 1),
                Outcome::Redraw,
                "{look:?}: the wheel scrolls"
            );
            assert!(confirm(&d).y < stepped.y, "{look:?}");

            d.fit_in(&one, &text, AREA);
            let mut whole = dialog(sheet(), (0, 0), "user-trash-full");
            whole.fit_in(&one, &text, AREA);
            assert_eq!(
                DialogLayout::compute(&one, &text, &d).size,
                DialogLayout::compute(&one, &text, &whole).size,
                "{look:?}: whole at 100%"
            );
            let sl = d
                .form()
                .unwrap()
                .sheet()
                .unwrap()
                .layout(&text, one.fonts.size_px);
            assert_eq!(sl.bar, None, "{look:?}");
        }
    }

    #[test]
    fn the_wheel_over_a_pages_horizontal_bar_scrolls_it_sideways() {
        use reprisekit::interface::Interface;
        let text = TextRenderer::load(&FontConfig::default());
        for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let big = reprisekit::interfaces::theme_for(look).with_text_size(200);
            let px = big.fonts.size_px;
            let mut d = dialog(sheet(), (0, 0), "user-trash-full");
            d.fit_in(&big, &text, (400, 560));
            let l = DialogLayout::compute(&big, &text, &d);
            let sl = d.form().unwrap().sheet().unwrap().layout(&text, px);
            let hbar = sl.hbar.expect("the page scrolls sideways");
            let hit = l.hit(
                &d,
                hbar.x + hbar.width / 2,
                hbar.y + hbar.height / 2,
                &text,
                px,
            );
            let confirm = |d: &Dialog| d.form().unwrap().pages()[0].rect("confirm").unwrap();
            let before = confirm(&d);
            assert_eq!(d.wheel(hit, 1), Outcome::Redraw, "{look:?}");
            let after = confirm(&d);
            assert!(after.x < before.x, "{look:?}: the page moves left");
            assert_eq!(after.y, before.y, "{look:?}: the page keeps its height");
        }
    }

    /// Four drives' tabs wrap onto a second row: the sheet grows 18 px, the page
    /// and every control sit 18 px lower, and a rebuild keeps them there.
    #[test]
    fn a_second_row_of_tabs_grows_the_sheet_and_survives_a_rebuild() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let px = theme.fonts.size_px;
        let drives = ["/", "/boot", "/home", "/var"]
            .iter()
            .map(|m| drive(m, &format!("Local Disk ({m})")))
            .collect();
        let mut d = dialog(
            BinSheet::new(Policy::default(), drives),
            (0, 0),
            "user-trash-full",
        );
        let Style::Form(fd) = &mut d.style else {
            panic!()
        };
        fd.fit_tabs(&text, px);
        fd.fit_tabs(&text, px);
        assert_eq!(fd.size, (344, 418));
        assert_eq!(
            fd.sheet().unwrap().layout(&text, px).page,
            Rect::new(9, 67, 326, 312)
        );
        let ok = |fd: &FormDialog| fd.pages()[0].rect("ok").unwrap();
        assert_eq!(ok(fd), Rect::new(98, 385, 75, 23));
        rebuild(fd);
        assert_eq!(ok(fd), Rect::new(98, 385, 75, 23));
        assert_eq!(
            fd.pages()[0].rect("confirm").unwrap(),
            Rect::new(33, 281, 13, 13)
        );
        let l = DialogLayout::compute(&theme, &text, &d);
        let page = Rect::new(9, 67, 326, 312);
        assert!(
            l.tabs
                .iter()
                .all(|r| r.x >= page.x && r.right() <= page.right())
        );
        assert_eq!(l.tabs[0].bottom(), page.y, "the open tab meets the page");
    }

    /// The controls in the window's coordinates, frame and caption included.
    #[test]
    fn pages_sit_on_their_pixels() {
        let d = dialog(sheet(), (0, 0), "user-trash-full");
        let pages = d.form().unwrap().pages();
        assert_eq!(pages.len(), 2);
        let g = &pages[0];
        let rect = |id: &str| g.rect(id).unwrap();
        assert_eq!(rect("independent"), Rect::new(34, 63, 12, 12));
        assert_eq!(rect("all"), Rect::new(34, 85, 12, 12));
        assert_eq!(rect("group"), Rect::new(24, 91, 296, 161));
        assert_eq!(rect("no_bin"), Rect::new(33, 114, 13, 13));
        assert_eq!(rect("confirm"), Rect::new(33, 263, 13, 13));
        // The trackbar's channel runs x 57 to 267 at y 171, 7 rows under the control's top, ticks 62 to 262.
        let bar = g.item("size").unwrap();
        let r = g.rect("size").unwrap();
        let lay = match &bar.control {
            Control::TrackBar(t) => reprisekit::trackbar::layout(t, r.x, r.y, r.width),
            _ => panic!(),
        };
        assert_eq!(lay.channel, Rect::new(57, 171, 211, 4));
        assert_eq!(lay.ticks.first(), Some(&62));
        assert_eq!(lay.ticks.last(), Some(&262));
        assert!(matches!(
            bar.control,
            Control::TrackBar(TrackBar {
                pos: 10,
                count: 101,
                enabled: true,
                ..
            })
        ));
        assert!(matches!(
            g.item("all").unwrap().control,
            Control::Radio {
                checked: true,
                enabled: true,
                ..
            }
        ));
        assert!(matches!(
            g.item("confirm").unwrap().control,
            Control::Check {
                checked: true,
                enabled: true,
                ..
            }
        ));
        assert!(matches!(
            g.item("apply").unwrap().control,
            Control::Button { enabled: false, .. }
        ));
        assert_eq!(rect("ok"), Rect::new(98, 367, 75, 23));
        assert_eq!(rect("cancel"), Rect::new(179, 367, 75, 23));
        assert_eq!(rect("apply"), Rect::new(260, 367, 75, 23));
        // The drive's page: sizes at x 156, every control greyed while one setting serves every drive.
        let d = &pages[1];
        let label = |id: &str| match &d.item(id).unwrap().control {
            Control::Label { text, .. } => text.clone(),
            _ => panic!(),
        };
        assert_eq!(label("size_of"), "19.8 GB");
        assert_eq!(label("reserved"), "1.98 GB");
        assert!(matches!(
            d.item("no_bin").unwrap().control,
            Control::Check { enabled: false, .. }
        ));
        assert_eq!(d.rect("no_bin").unwrap().x, 24);
        assert_eq!(d.rect("size_of").unwrap(), Rect::new(156, 62, 160, 13));
        assert!(d.item("confirm").is_none());
    }

    fn sheet_of(d: &mut Dialog) -> &BinSheet {
        let Kind::RecycleBin(s) = &d.form_mut().unwrap().kind else {
            panic!()
        };
        s
    }

    fn press(d: &mut Dialog, page: usize, id: &'static str, key: Key) -> Outcome {
        let fd = d.form_mut().unwrap();
        let f = fd.open_page_mut().unwrap();
        f.focus_id(id);
        let ev = f.key(key);
        event(fd, page, ev)
    }

    #[test]
    fn boxes_feed_the_settings_and_ok_applies_them() {
        let mut d = dialog(sheet(), (36, 261), "user-trash-full");
        assert_eq!(press(&mut d, 0, "no_bin", Key::Space), Outcome::Redraw);
        assert_eq!(press(&mut d, 0, "confirm", Key::Space), Outcome::Redraw);
        let mut want = Policy {
            confirm: false,
            ..Policy::default()
        };
        want.global.remove_immediately = true;
        assert_eq!(sheet_of(&mut d).pending, want);
        let fd = d.form_mut().unwrap();
        let page = fd.open_page().unwrap();
        assert!(matches!(
            page.item("apply").unwrap().control,
            Control::Button { enabled: true, .. }
        ));
        assert!(
            matches!(
                page.item("size").unwrap().control,
                Control::TrackBar(TrackBar { enabled: false, .. })
            ),
            "no size while files are not moved"
        );
        assert_eq!(
            event(fd, 0, Event::Pressed("apply")),
            Outcome::BinSettings {
                settings: want.clone(),
                close: false
            }
        );
        assert!(matches!(
            fd.open_page().unwrap().item("apply").unwrap().control,
            Control::Button { enabled: false, .. }
        ));
        assert_eq!(
            event(fd, 0, Event::Default),
            Outcome::BinSettings {
                settings: want,
                close: true
            }
        );
        assert_eq!(event(fd, 0, Event::Cancel), Outcome::Close);
    }

    /// The trackbar's keys move the cap a percent at a time, ten with Page Down; the label follows.
    #[test]
    fn the_trackbar_sets_the_percentage() {
        let mut d = dialog(sheet(), (0, 0), "user-trash-full");
        press(&mut d, 0, "size", Key::Left);
        assert_eq!(sheet_of(&mut d).pending.global.max_percent, 9);
        press(&mut d, 0, "size", Key::PageDown);
        assert_eq!(sheet_of(&mut d).pending.global.max_percent, 19);
        let page = &d.form().unwrap().pages()[0];
        assert!(matches!(
            &page.item("pct").unwrap().control,
            Control::Label { text, .. } if text == "19%"
        ));
        let drive = &d.form().unwrap().pages()[1];
        assert!(matches!(
            &drive.item("reserved").unwrap().control,
            Control::Label { text, .. } if text == "3.78 GB"
        ));
    }

    /// "Configure drives independently" greys Global's group and makes each
    /// drive's tab live; a drive's setting starts from the global one.
    #[test]
    fn independent_drives_take_their_own_settings() {
        let drives = vec![drive("home", "Local Disk (/)"), drive("5A1F", "STICK")];
        let mut d = dialog(
            BinSheet::new(Policy::default(), drives),
            (0, 0),
            "user-trash",
        );
        let ev = {
            let fd = d.form_mut().unwrap();
            let f = fd.open_page_mut().unwrap();
            f.focus_id("all");
            f.key(Key::Up)
        };
        assert_eq!(ev, Event::Changed("independent"));
        event(d.form_mut().unwrap(), 0, ev);
        assert!(sheet_of(&mut d).pending.independent);
        let pages = d.form().unwrap().pages();
        assert!(matches!(
            pages[0].item("no_bin").unwrap().control,
            Control::Check { enabled: false, .. }
        ));
        assert!(matches!(
            pages[2].item("no_bin").unwrap().control,
            Control::Check { enabled: true, .. }
        ));

        if let Some(s) = d.form_mut().unwrap().sheet_mut() {
            s.tabs.open = 2;
        }
        press(&mut d, 2, "no_bin", Key::Space);
        let p = &sheet_of(&mut d).pending;
        assert!(p.rule("5A1F").remove_immediately);
        assert!(!p.rule("home").remove_immediately);
        assert_eq!(p.drives.len(), 1, "only the drive that changed");
        let drive = &d.form().unwrap().pages()[2];
        assert!(matches!(
            &drive.item("reserved").unwrap().control,
            Control::Label { text, .. } if text == "0 bytes"
        ));
    }

    #[test]
    fn sheet_is_344x400_with_a_tab_per_drive() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let d = dialog(sheet(), (36, 261), "user-trash-full");
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (344, 400));
        let strip = d.form().unwrap().sheet().unwrap().frame().0;
        assert_eq!(strip, (9, 29));
        assert_eq!(l.page, Some(Rect::new(9, 49, 326, 312)));
        assert_eq!(l.tabs.len(), 2);
        assert!(l.help.is_some());
    }

    /// Writes `target/binprops-{global,drive}.png` for eyeballing; run with `--ignored`.
    #[test]
    #[ignore]
    fn render_binprops_preview() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        for (name, tab) in [("global", 0), ("drive", 1)] {
            let mut d = dialog(sheet(), (36, 261), "user-trash-full");
            if let Some(t) = d
                .form_mut()
                .and_then(|f| f.sheet_mut().map(|s| &mut s.tabs))
            {
                t.open = tab;
            }
            let l = DialogLayout::compute(&theme, &text, &d);
            let c = crate::dialog::draw(
                &theme,
                &text,
                &l,
                None,
                None,
                &[],
                &[],
                None,
                &d,
                false,
                None,
                reprisekit::cues::Cues::NONE,
                true,
                None,
            );
            reprisekit::raster::write_preview(&c, &format!("binprops-{name}.png"));
        }
    }
}
