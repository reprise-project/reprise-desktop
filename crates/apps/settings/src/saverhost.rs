use crate::registry::Page;
use crate::{SettingsApp, saverdlg, saverpage, take, wallpapers};
use repriseapp::decode::Decoders;
use repriseapp::keys::form_input;
use repriseapp::{Ctx, Event, Reply};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

/// The Photos folder's pictures: being listed, or as listed.
pub(crate) enum Photos {
    Listing(Receiver<Vec<PathBuf>>),
    Listed(Vec<PathBuf>),
}

impl SettingsApp {
    pub(crate) fn saver_dlg_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some(d) = self.saver_dlg.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => saverdlg::Outcome::Cancel,
                Event::Blink => {
                    d.form.blink(ctx.caret_on);
                    return Reply::redraw();
                }
                ev => match form_input(&mut d.form, &ev, ctx) {
                    Some(ev) => d.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        match out {
            saverdlg::Outcome::None => Reply::none(),
            saverdlg::Outcome::Redraw => Reply::redraw(),
            saverdlg::Outcome::Cancel => {
                self.saver_dlg = None;
                Reply::redraw_all()
            }
            saverdlg::Outcome::Accept => {
                if let Some(d) = self.saver_dlg.take() {
                    d.apply(&mut self.want);
                }
                self.sync_apply();
                Reply::redraw_all()
            }
        }
    }

    /// Step the Screen Saver & Lock page's monitor saver and put the frame up. Whether it
    /// wants the next tick.
    pub(crate) fn saver_tick(&mut self, ctx: &Ctx) -> bool {
        if self.sheet.tabs.open != Page::ScreenSaverLock.index() {
            self.saver_preview = None;
            self.saver_photos = None;
            return false;
        }
        let kind = match saverpage::row(&self.want) {
            0 => reprisekit::saver::Kind::Blank,
            _ => saverpage::kind(&self.want),
        };
        let stale = self
            .saver_preview
            .as_ref()
            .is_none_or(|(p, _)| !p.runs(kind, &self.want));
        if stale {
            let options = saverpage::options(&self.want);
            let p = saverpage::Preview::new(kind, options, &self.want.saver_folder, &ctx.text);
            self.saver_preview = Some((p, std::time::Instant::now()));
            self.saver_photo = None;
        }
        self.want_saver_photos();
        let Some((p, last)) = self.saver_preview.as_mut() else {
            return false;
        };
        if let Some((folder, photos)) = &mut self.saver_photos
            && *folder == self.want.saver_folder
        {
            if let Photos::Listing(rx) = photos {
                match take(rx) {
                    Ok(None) => {}
                    Ok(Some(pics)) => *photos = Photos::Listed(pics),
                    Err(()) => *photos = Photos::Listed(Vec::new()),
                }
            }
            if let Photos::Listed(pics) = photos
                && p.files.is_empty()
            {
                p.files.clone_from(pics);
            }
        }
        if let Some(rx) = &self.saver_photo
            && let Ok(Some(got)) = take(rx)
        {
            if let Some((path, picture)) = got {
                p.show_photo(path, &picture);
            }
            self.saver_photo = None;
        }
        let ms = last.elapsed().as_millis() as u64;
        *last = std::time::Instant::now();
        let frame = p.frame(ms);
        if p.wants_photo() {
            let (files, current) = (p.files.clone(), p.current.clone());
            self.saver_photo = Some(self.spawn(move || {
                saverpage::pick_photo(&files, current.as_deref(), reprisefs::pictures::roll, |f| {
                    Decoders::default_set()
                        .decode_file(f, (304, 224))
                        .map(|d| d.canvas)
                })
            }));
        }
        saverpage::show(&mut self.sheet.pages[Page::ScreenSaverLock.index()], frame);
        true
    }

    /// List the paired devices when the Screen Saver & Lock page comes up. Whether the page
    /// changed.
    pub(crate) fn ensure_away_devices(&mut self) -> bool {
        if crate::registry::open(&self.sheet) != Page::ScreenSaverLock {
            self.away_fresh = false;
            return false;
        }
        if self.away_fresh {
            return false;
        }
        self.away_fresh = true;
        // Tests leave the system bus alone.
        if !cfg!(test) {
            self.away_reading = Some(self.spawn(reprisebus::bluez::paired));
        }
        false
    }

    /// BlueZ listed the paired devices: offer them on the page.
    pub(crate) fn away_done(&mut self) -> bool {
        let Some(rx) = &self.away_reading else {
            return false;
        };
        self.away_devices = match take(rx) {
            Ok(None) => return false,
            Ok(Some(found)) => found,
            Err(()) => Vec::new(),
        };
        self.away_reading = None;
        self.follow_away();
        true
    }

    pub(crate) fn follow_away(&mut self) {
        saverpage::follow_away(
            &mut self.sheet.pages[Page::ScreenSaverLock.index()],
            &self.away_devices,
            &self.want,
        );
    }

    fn want_saver_photos(&mut self) {
        let Some((p, _)) = &self.saver_preview else {
            return;
        };
        let folder = self.want.saver_folder.clone();
        let have = self
            .saver_photos
            .as_ref()
            .is_some_and(|(f, _)| *f == folder);
        if p.kind != reprisekit::saver::Kind::Photos || have {
            return;
        }
        let dir = wallpapers::expand(&folder)
            .or_else(|| reprisefs::places::user_dir("PICTURES", "Pictures"));
        let rx = self.spawn(move || {
            dir.as_deref()
                .map(reprisefs::pictures::in_folder)
                .unwrap_or_default()
        });
        self.saver_photos = Some((folder, Photos::Listing(rx)));
    }
}
