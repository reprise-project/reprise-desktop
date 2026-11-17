//! Recycle Bin Properties' host half: its window, the drives' tabs read off the loop, and OK and
//! Apply writing `[recycle_bin]` in `config.toml`, which every host's Delete reads.

use crate::Explorer;
use crate::acting::Action;
use crate::dialogs::SheetId;
use repriseapp::listjob::Job;
use repriseapp::{Child, Ctx, Event, Reply, WinId};
use reprisefs::bincap::{self, Policy};
use reprisefs::deadline::Gate;
use reprisefs::trash::Bins;
use reprisekit::controls::binsheet::{self, BinSheet, Drive, Outcome, Rule, Settings};
use reprisekit::controls::propsheet;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const BIN_PROPS: WinId = WinId::named("bin-props");
const TITLE: &str = "Recycle Bin Properties";

const _: () = assert!(binsheet::MAX_DAYS == bincap::MAX_DAYS);

pub(crate) struct BinProps {
    pub(crate) sheet: BinSheet,
    /// The `config.toml` OK and Apply write.
    pub(crate) config: PathBuf,
    bins: Bins,
    /// The home trash's drive, the first tab after Global.
    home: Option<Drive>,
    /// Each drive's tab as its read found it, None for no bin.
    tabs: HashMap<PathBuf, Option<Drive>>,
    /// The tab reads still out. A drive that never answers keeps its read, and no second one
    /// starts.
    reads: Vec<(PathBuf, Job<Option<Drive>>)>,
    /// Some bin held an item as the sheet opened, so its window wears the full bin.
    full: bool,
    /// Which opening of the sheet window this is ([`crate::dialogs::SheetId`]).
    pub(crate) opened: u64,
}

impl BinProps {
    pub(crate) fn id(&self) -> SheetId {
        SheetId {
            win: BIN_PROPS,
            opened: self.opened,
        }
    }

    pub(crate) fn icon(&self) -> &'static str {
        if self.full {
            "user-trash-full"
        } else {
            "user-trash"
        }
    }
}

impl Explorer {
    /// Opens on Global and the home drive's tab; the other drives' tabs fill in as the drive scan
    /// and each drive's read answer, since a drive can be a mount that never answers.
    pub(crate) fn open_bin_props(&mut self, ctx: &mut Ctx) -> Reply {
        if self.bin_props.is_some() {
            return Reply::none();
        }
        let work = |_: &Gate| {
            let bins = Bins::system()?;
            let home = home_drive(&bins);
            let full = (bins.files_dirs().iter()).any(|f| !reprisefs::places::trash_empty(f));
            Some((bins, home, full))
        };
        self.act(ctx, Action::reads(TITLE), work, |ex, read, ctx| {
            let Some((bins, home, full)) = read else {
                return Reply::none();
            };
            let mut sheet =
                BinSheet::new(settings_of(&bins.policy), home.iter().cloned().collect());
            sheet.sheet.fit_tabs(&ctx.text, ctx.forms().fonts.size_px);
            let opened = ex.next_sheet();
            ex.bin_props = Some(BinProps {
                sheet,
                config: repriseconf::config_path(),
                bins,
                home,
                tabs: HashMap::new(),
                reads: Vec::new(),
                full,
                opened,
            });
            ex.rescan_send_drives(ctx);
            ex.read_bin_tabs(ctx);
            Reply::redraw_all()
        })
    }

    /// Starts a tab read for each drive the scan found that has neither a tab nor a read out.
    fn read_bin_tabs(&mut self, ctx: &Ctx) {
        let Some(b) = self.bin_props.as_ref() else {
            return;
        };
        let unread: Vec<PathBuf> = (self.send.drives.found().iter())
            .filter(|t| !b.tabs.contains_key(*t) && !b.reads.iter().any(|(r, _)| r == *t))
            .cloned()
            .collect();
        let bins = b.bins.clone();
        let jobs: Vec<_> = (unread.into_iter())
            .map(|top| {
                let (bins, at) = (bins.clone(), top.clone());
                (top, self.spawn_read(ctx, move || drive_tab(&bins, &at)))
            })
            .collect();
        if let Some(b) = self.bin_props.as_mut() {
            b.reads.extend(jobs);
        }
    }

    /// The drives' scan and tab reads that answered since the last wake, shown as tabs. A drive no
    /// longer found drops its tab and its read, so one mounted there later is read afresh.
    pub(crate) fn take_bin_tabs(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let b = self.bin_props.as_mut()?;
        let found = self.send.drives.found();
        let (reads, tabs) = (&mut b.reads, &mut b.tabs);
        tabs.retain(|t, _| found.contains(t));
        reads.retain(|(top, job)| {
            if !found.contains(top) {
                return false;
            }
            match job.take() {
                Some(tab) => {
                    tabs.insert(top.clone(), tab);
                    false
                }
                None => true,
            }
        });
        let drives = (b.home.iter().cloned())
            .chain(
                found
                    .iter()
                    .filter_map(|t| b.tabs.get(t).cloned().flatten()),
            )
            .collect();
        let shown = b.sheet.show_drives(drives);
        if shown {
            b.sheet.sheet.fit_tabs(&ctx.text, ctx.forms().fonts.size_px);
        }
        self.read_bin_tabs(ctx);
        shown.then(Reply::redraw_all)
    }

    /// The mounted drives changed: an open sheet scans them again.
    pub(crate) fn bin_drives_changed(&mut self, ctx: &mut Ctx) {
        if self.bin_props.is_some() {
            self.rescan_send_drives(ctx);
        }
    }

    pub fn bin_props_child(&self) -> Option<Child> {
        let b = self.bin_props.as_ref()?;
        Some(Child {
            id: BIN_PROPS,
            title: TITLE.into(),
            size: self.forms.text.size(b.sheet.sheet.size),
            fixed: true,
            modal: false,
        })
    }

    pub fn draw_bin_props(&self, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        c.fill_rect(c.bounds(), forms.colors.button_face);
        if let Some(b) = &self.bin_props {
            let p = Paint {
                s: 1,
                ..forms.paint()
            };
            propsheet::draw(c, &ctx.text, &p, &b.sheet.sheet, true, forms.fonts.size_px);
        }
    }

    pub fn bin_props_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if matches!(ev, Event::CloseRequest) {
            return self.close_bin_props(ctx);
        }
        let Some(b) = self.bin_props.as_mut() else {
            return Reply::none();
        };
        let sheet = &mut b.sheet.sheet;
        let clip = repriseapp::keys::form_clipboard(sheet.open().focused_edit(), &ev);
        let out = match ev {
            Event::Blink => {
                let on = !sheet.open().caret;
                sheet.open_mut().blink(on);
                return Reply::redraw();
            }
            Event::Pasted(text) => sheet.paste(text),
            ev => repriseapp::keys::sheet_input(sheet, &ev, ctx).unwrap_or(propsheet::Event::None),
        };
        let reply = match b.sheet.event(out) {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Close => self.close_bin_props(ctx),
            Outcome::Save { settings, close } => self.save_bin_settings(&settings, close, ctx),
        };
        reply.with_clipboard(clip)
    }

    /// Closes the sheet, the one way it goes, with the boxes it raised ([`Explorer::dismiss`]).
    fn close_bin_props(&mut self, ctx: &mut Ctx) -> Reply {
        let mut reply = Reply::redraw_all();
        for d in (self.dialogs).withdraw(|d| d.sheet() == Some(BIN_PROPS)) {
            reply = reply.and(self.dismiss(d, ctx));
        }
        self.bin_props = None;
        reply
    }

    /// Writes `settings` into `[recycle_bin]`; OK closes the sheet first. A write that fails says
    /// so in a box.
    fn save_bin_settings(&mut self, settings: &Settings, close: bool, ctx: &mut Ctx) -> Reply {
        let Some((path, sheet)) = self.bin_props.as_ref().map(|b| (b.config.clone(), b.id()))
        else {
            return Reply::none();
        };
        let closed = match close {
            true => self.close_bin_props(ctx),
            false => Reply::none(),
        };
        let policy = policy_of(settings);
        let work = move |gate: &Gate| {
            if !gate.change(&path) {
                return Ok(());
            }
            let saved = repriseconf::edit(&path, |text| policy.write(text));
            saved.map_err(|e| e.to_string()).and_then(|r| r.map(drop))
        };
        let reply = self.act(
            ctx,
            Action::changes(TITLE).of(sheet),
            work,
            move |ex, saved, ctx| match saved {
                Ok(()) => Reply::none(),
                Err(e) => {
                    log::error!("recycle bin: settings not saved: {e}");
                    let text = format!("Cannot save the Recycle Bin settings in config.toml: {e}");
                    ex.message_over(Some(sheet), TITLE, text, ctx)
                }
            },
        );
        closed.and(reply).and(Reply::redraw_all())
    }
}

fn rule_of(r: bincap::Rule) -> Rule {
    Rule {
        remove_immediately: r.remove_immediately,
        max_percent: r.max_percent,
    }
}

fn bincap_rule(r: Rule) -> bincap::Rule {
    bincap::Rule {
        remove_immediately: r.remove_immediately,
        max_percent: r.max_percent,
    }
}

pub(crate) fn settings_of(p: &Policy) -> Settings {
    Settings {
        confirm: p.confirm,
        independent: p.independent,
        global: rule_of(p.global),
        drives: (p.drives.iter())
            .map(|(k, r)| (k.clone(), rule_of(*r)))
            .collect(),
        older_than_days: p.older_than_days,
    }
}

pub(crate) fn policy_of(s: &Settings) -> Policy {
    Policy {
        confirm: s.confirm,
        independent: s.independent,
        global: bincap_rule(s.global),
        drives: (s.drives.iter())
            .map(|(k, r)| (k.clone(), bincap_rule(*r)))
            .collect(),
        older_than_days: s.older_than_days,
    }
}

/// The home trash's drive, as "Local Disk (/)" by its mount, sized by `statvfs`.
fn home_drive(bins: &Bins) -> Option<Drive> {
    let top = bins.home_top()?;
    Some(Drive {
        key: bincap::HOME.into(),
        label: format!("Local Disk ({})", top.display()),
        size: fs_size(top)?,
    })
}

/// The tab of `top`, a drive the scan found, by folder name: None for the home trash's drive and a
/// drive with no Recycle Bin that cannot make one. Sized by `statvfs`, keyed as
/// `[recycle_bin.drives]`. Looks inside `top`, so it runs off the loop.
pub(crate) fn drive_tab(bins: &Bins, top: &Path) -> Option<Drive> {
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

fn fs_size(path: &Path) -> Option<u64> {
    reprisefs::places::statvfs_totals(path).map(|(total, _)| total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::place::Place;
    use crate::tests::{test_ctx, test_explorer, wake_until};
    use reprisekit::textedit::keysym;

    fn key(sym: u32) -> Event {
        let text = (sym == keysym::SPACE).then(|| " ".to_string());
        Event::Key(repriseapp::KeyInput {
            sym,
            edit: text.clone().map(reprisekit::textedit::EditKey::Insert),
            utf8: text,
            mods: repriseapp::Mods::default(),
        })
    }

    /// An Explorer in the Recycle Bin with its Properties up, writing to `config`.
    fn sheet_up(config: &Path, ctx: &mut Ctx) -> Explorer {
        let mut ex = test_explorer(Place::RecycleBin, ctx);
        ex.properties(ctx);
        wake_until(&mut ex, ctx, |ex| ex.bin_props.is_some());
        ex.bin_props.as_mut().expect("the sheet").config = config.to_path_buf();
        ex
    }

    #[test]
    fn the_recycle_bins_properties_with_nothing_selected_open_recycle_bin_properties() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut ctx = test_ctx();
        let ex = sheet_up(&dir.path().join("config.toml"), &mut ctx);
        assert!(ex.sv.selection.single().is_none());
        assert!(ex.props.is_none(), "no file's sheet");
        let child = ex.children().into_iter().find(|c| c.id == BIN_PROPS);
        assert_eq!(child.map(|c| c.title), Some(TITLE.to_string()));
        let tabs = &ex.bin_props.as_ref().unwrap().sheet.sheet.tabs.labels;
        assert_eq!(tabs.first().map(String::as_str), Some("Global"));
    }

    #[test]
    fn ok_writes_the_boxes_into_the_recycle_bin_table_and_closes() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let config = dir.path().join("config.toml");
        let mine = "# mine\nterminal = \"foot\"\n";
        std::fs::write(&config, mine).expect("write");
        let mut ctx = test_ctx();
        let mut ex = sheet_up(&config, &mut ctx);
        let b = ex.bin_props.as_mut().unwrap();
        b.sheet.sheet.open_mut().focus_id("no_bin");
        ex.bin_props_event(key(keysym::SPACE), &mut ctx);
        ex.bin_props_event(key(keysym::RETURN), &mut ctx);
        assert!(ex.bin_props.is_none(), "OK closes the sheet");
        wake_until(&mut ex, &mut ctx, |ex| ex.acting.is_empty());
        let text = std::fs::read_to_string(&config).expect("read");
        assert!(text.starts_with(mine), "{text}");
        assert!(Policy::parse(&text).global.remove_immediately, "{text}");
        assert!(ex.dialog().is_none());
    }

    #[test]
    fn settings_that_cannot_be_written_say_so_in_a_box() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let config = dir.path().join("config.toml");
        std::fs::write(&config, "[recycle_bin\n").expect("write");
        let mut ctx = test_ctx();
        let mut ex = sheet_up(&config, &mut ctx);
        ex.bin_props_event(key(keysym::RETURN), &mut ctx);
        wake_until(&mut ex, &mut ctx, |ex| ex.dialog().is_some());
        match ex.dialog() {
            Some(crate::dialogs::Dialog::Message { title, text, .. }) => {
                assert_eq!(title, TITLE);
                assert!(
                    text.starts_with("Cannot save the Recycle Bin settings in config.toml: "),
                    "{text}"
                );
            }
            _ => panic!("an error box is up"),
        }
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "[recycle_bin\n");
    }

    /// Apply's failed write says so over the sheet, which takes the box when it closes; OK's
    /// says so over the folder window, since OK closed the sheet first.
    #[test]
    fn an_apply_that_cannot_be_written_says_so_over_the_sheet_and_leaves_with_it() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let config = dir.path().join("config.toml");
        std::fs::write(&config, "[recycle_bin\n").expect("write");
        let mut ctx = test_ctx();
        let mut ex = sheet_up(&config, &mut ctx);
        ex.save_bin_settings(&settings_of(&Policy::default()), false, &mut ctx);
        wake_until(&mut ex, &mut ctx, |ex| ex.dialog().is_some());
        assert_eq!(ex.parent_of(ex.dialog_win()), BIN_PROPS);
        ex.message_box("Explorer", "about the folder".into(), &mut ctx);

        ex.bin_props_event(Event::CloseRequest, &mut ctx);
        assert!(ex.bin_props.is_none(), "the sheet closes");
        assert_eq!(ex.dialogs.len(), 1, "the sheet's box left with it");
        assert!(matches!(
            ex.dialog(),
            Some(crate::dialogs::Dialog::Message { text, .. }) if text == "about the folder"
        ));

        let mut ex = sheet_up(&config, &mut ctx);
        ex.save_bin_settings(&settings_of(&Policy::default()), true, &mut ctx);
        wake_until(&mut ex, &mut ctx, |ex| ex.dialog().is_some());
        assert_eq!(ex.parent_of(ex.dialog_win()), repriseapp::MAIN);
    }

    /// Apply's failed write landing after the sheet closed and opened again says so over the
    /// folder window, and stays up when the new sheet closes.
    #[test]
    fn an_apply_failing_after_its_sheet_was_opened_again_covers_the_folder_window() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let config = dir.path().join("config.toml");
        std::fs::write(&config, "[recycle_bin\n").expect("write");
        let mut ctx = test_ctx();
        let mut ex = sheet_up(&config, &mut ctx);
        let gate = std::sync::Arc::new(std::sync::RwLock::new(()));
        let writing = gate.write().expect("lock");
        ex.held = Some(gate.clone());
        ex.save_bin_settings(&settings_of(&Policy::default()), false, &mut ctx);
        ex.held = None;
        ex.bin_props_event(Event::CloseRequest, &mut ctx);
        ex.properties(&mut ctx);
        wake_until(&mut ex, &mut ctx, |ex| ex.bin_props.is_some());
        drop(writing);
        wake_until(&mut ex, &mut ctx, |ex| ex.dialog().is_some());
        assert_eq!(ex.parent_of(ex.dialog_win()), repriseapp::MAIN);
        ex.bin_props_event(Event::CloseRequest, &mut ctx);
        assert!(ex.dialog().is_some(), "the failure stays up");
    }

    #[test]
    fn a_drive_gets_a_tab_when_it_has_a_bin_or_can_make_one() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().expect("a temp dir");
        let (stick, shut) = (t.path().join("STICK"), t.path().join("SHUT"));
        std::fs::create_dir_all(&stick).unwrap();
        std::fs::create_dir_all(&shut).unwrap();
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o555)).unwrap();
        let bins = Bins::new(t.path().join("home"), vec![stick.clone(), shut.clone()]);
        let tab = drive_tab(&bins, &stick).expect("STICK can make a bin");
        assert_eq!(tab.label, "STICK");
        assert!(tab.size > 0);
        assert_eq!(drive_tab(&bins, &shut), None, "SHUT cannot");
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn the_settings_carry_every_field_of_the_policy() {
        let mut p = Policy {
            confirm: false,
            independent: true,
            older_than_days: 12,
            ..Policy::default()
        };
        p.global.max_percent = 25;
        p.drives.insert(
            "5A1F".into(),
            bincap::Rule {
                remove_immediately: true,
                max_percent: 3,
            },
        );
        assert_eq!(policy_of(&settings_of(&p)), p);
    }
}
