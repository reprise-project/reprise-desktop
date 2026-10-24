use crate::registry::Page;
use crate::{SettingsApp, saverdlg, saverpage, take, wallpapers};
use repriseapp::decode::Decoders;
use repriseapp::keys::form_input;
use repriseapp::{Ctx, Event, Reply};

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

    /// Step the Screen Saver page's monitor saver and put the frame up. Whether it wants the next tick.
    pub(crate) fn saver_tick(&mut self, ctx: &Ctx) -> bool {
        if self.sheet.tabs.open != Page::ScreenSaver.index() {
            self.saver_preview = None;
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
        if let Some((folder, rx)) = &self.saver_photos
            && folder == &self.want.saver_folder
            && let Ok(Some(pics)) = take(rx)
        {
            p.files = pics;
            self.saver_photos = None;
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
        saverpage::show(&mut self.sheet.pages[Page::ScreenSaver.index()], frame);
        true
    }

    fn want_saver_photos(&mut self) {
        let Some((p, _)) = &self.saver_preview else {
            return;
        };
        let folder = self.want.saver_folder.clone();
        let have = self.saver_photos.as_ref().map(|(f, _)| f == &folder);
        if p.kind != reprisekit::saver::Kind::Photos || !p.files.is_empty() || have == Some(true) {
            return;
        }
        let dir = wallpapers::expand(&folder)
            .or_else(|| reprisefs::places::user_dir("PICTURES", "Pictures"));
        let rx = self.spawn(move || {
            dir.as_deref()
                .map(reprisefs::pictures::in_folder)
                .unwrap_or_default()
        });
        self.saver_photos = Some((folder, rx));
    }
}
