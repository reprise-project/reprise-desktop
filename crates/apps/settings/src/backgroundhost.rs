use crate::backgroundpage::{OwnKey, Target};
use crate::registry::Page;
use crate::{SettingsApp, appearancepage, backgroundpage, pages, wallpapers, winstate};
use repriseapp::Ctx;
use repriseapp::decode::{Decoded, Decoders};
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

impl SettingsApp {
    /// The monitors the Use for list offers: every connected one, labelled as the Display page
    /// labels it, while two or more are connected.
    pub(crate) fn bg_monitors(&self) -> Vec<backgroundpage::Monitor> {
        let heads = &self.display.connected;
        if heads.len() < 2 {
            return Vec::new();
        }
        (heads.iter().enumerate())
            .map(|(i, h)| backgroundpage::Monitor {
                name: h.name.clone(),
                label: crate::displaypage::head_label(i + 1, h),
            })
            .collect()
    }

    /// What the Background page's Use for picks.
    fn bg_target(&self) -> backgroundpage::Target {
        let page = &self.sheet.pages[Page::Background.index()];
        backgroundpage::target(page, &self.want, &self.bg_monitors())
    }

    /// The pending settings as the Background page shows them, for the desktop or monitor Use for
    /// picks.
    pub(crate) fn bg(&self) -> Cow<'_, crate::model::Settings> {
        backgroundpage::view(&self.want, &self.bg_target())
    }

    /// The pending settings as the Background page's monitor shows them.
    fn bg_preview(&self) -> Cow<'_, crate::model::Settings> {
        backgroundpage::preview(&self.want, &self.bg_target())
    }

    /// Refill the Use for list and follow what it picks.
    pub(crate) fn follow_use_for(&mut self) {
        let monitors = self.bg_monitors();
        let page = &mut self.sheet.pages[Page::Background.index()];
        backgroundpage::follow_use_for(page, &self.want, &monitors);
    }

    pub(crate) fn connected_names(&self) -> Vec<String> {
        self.display
            .connected
            .iter()
            .map(|h| h.name.clone())
            .collect()
    }

    /// Make `picked` the pending wallpaper of the desktop or monitor Use for picks, or of all of
    /// them: every connected monitor then shows it, giving up a picture of its own.
    fn set_wallpaper(&mut self, picked: String) {
        match self.bg_target() {
            Target::Desktop(n) => backgroundpage::set_for_desktop(&mut self.want, n, picked),
            Target::Monitor(name) => {
                backgroundpage::set_for_monitor(&mut self.want, &name, OwnKey::Wallpaper, picked)
            }
            Target::All => {
                self.want.wallpaper = picked;
                self.for_all_monitors(OwnKey::Wallpaper);
            }
        }
    }

    /// A pick for all monitors: every connected one takes it, giving up its own `key`.
    fn for_all_monitors(&mut self, key: OwnKey) {
        let names = self.connected_names();
        backgroundpage::clear_for_monitors(&mut self.want, &names, key);
    }

    /// The Position combo moved: the monitor Use for picks takes it as its own, else it is the
    /// position for all, which every connected monitor then takes.
    pub(crate) fn pick_position(&mut self) {
        let page = &self.sheet.pages[Page::Background.index()];
        let Some(value) = (page.combo_selected(backgroundpage::POSITION))
            .and_then(|i| crate::model::POSITIONS.get(i))
            .map(|(v, _)| v.to_string())
        else {
            return;
        };
        match self.bg_target() {
            Target::Monitor(name) => {
                backgroundpage::set_for_monitor(&mut self.want, &name, OwnKey::Position, value)
            }
            _ => {
                self.want.wallpaper_position = value;
                self.for_all_monitors(OwnKey::Position);
            }
        }
    }

    /// Refill the Background list from `wallpapers` with the pending one picked and scrolled into view (once, when the scan lands).
    /// The wallpaper list's icons and the built-ins' thumbnails at the list's icon size, then the
    /// list refilled with them.
    pub(crate) fn load_wallpaper_icons(&mut self, ctx: &mut Ctx) {
        let px = crate::menuhost::list_icon_px(ctx);
        let none =
            (ctx.icons.get("process-stop", px)).or_else(|| ctx.icons.get("dialog-cancel", px));
        self.wallpaper_icons = Some((none, ctx.icons.get("image-x-generic", px)));
        self.folder_icon = ctx.icons.get("folder", px);
        self.fill_wallpapers();
        self.draw_thumbs(ctx);
    }

    /// The built-ins' thumbnails at the list's icon size, in the glass colour and mode `want`
    /// draws in now; [`Self::thumbs_follow_mode`] draws them again when that mode turns.
    pub(crate) fn draw_thumbs(&mut self, ctx: &Ctx) {
        let (px, scale) = (crate::menuhost::list_icon_px(ctx), ctx.icons.scale());
        let glass = appearancepage::glass(&self.want);
        self.thumbs_mode = Some(glass.mode);
        self.thumbing = Some(self.spawn(move || {
            backgroundpage::builtins()
                .into_iter()
                .map(|b| Some(backgroundpage::thumb(b, &glass, px, scale)))
                .collect()
        }));
    }

    /// Draw the thumbnails again once `want`'s mode draws otherwise than they were drawn, as a
    /// schedule does with no event; runs before each draw.
    pub(crate) fn thumbs_follow_mode(&mut self, ctx: &Ctx) {
        if self
            .thumbs_mode
            .is_some_and(|m| m != appearancepage::glass(&self.want).mode)
        {
            self.draw_thumbs(ctx);
        }
    }

    pub(crate) fn fill_wallpapers(&mut self) {
        self.follow_use_for();
        let folders = self.slide_folders();
        let (slides, slideshow, wallpaper) = {
            let v = self.bg();
            (
                backgroundpage::slideshow(&v),
                v.slideshow.clone(),
                v.wallpaper.clone(),
            )
        };
        let page = &mut self.sheet.pages[Page::Background.index()];
        if slides {
            backgroundpage::fill_folders(page, &folders, &slideshow, self.folder_icon.as_ref());
        } else {
            backgroundpage::fill(
                page,
                &self.wallpapers,
                &wallpaper,
                self.wallpaper_icons.as_ref(),
                &self.thumbs,
            );
        }
    }

    pub(crate) fn slide_folders(&self) -> Vec<PathBuf> {
        backgroundpage::folders(&self.wallpapers, wallpapers::expand(&self.want.slideshow))
    }

    pub(crate) fn show_monitor(&mut self) {
        self.follow_use_for();
        let bg = self.bg_preview().into_owned();
        let file = backgroundpage::monitor_file(&bg, self.listed(), self.live_slide());
        backgroundpage::show(
            &mut self.sheet.pages[Page::Background.index()],
            &bg,
            &file,
            self.picture.as_ref(),
            self.listing.is_some() || self.decoding.is_some(),
        );
    }

    fn listed(&self) -> &[PathBuf] {
        self.listed.as_ref().map_or(&[], |(_, p)| p.as_slice())
    }

    fn live_slide(&self) -> Option<&Path> {
        backgroundpage::live_slide(&self.loaded, &self.want, self.slide_now.as_deref())
    }

    /// The compositor's state file changed: reread the picture its slideshow shows and, when it
    /// moved, put it on the monitor.
    pub(crate) fn follow_slide(&mut self) -> bool {
        let now = (self.compositor_state.as_deref()).and_then(repriseconf::slideshow_picture);
        if now == self.slide_now {
            return false;
        }
        self.slide_now = now;
        self.want_preview();
        self.show_monitor();
        true
    }

    /// List the pending slideshow folder on a thread unless it is listed, unchanged since, or being listed already.
    fn list_slide_folder(&mut self) {
        let Some(folder) = wallpapers::expand(&self.want.slideshow) else {
            self.slides_touched.store(false, Ordering::Release);
            return;
        };
        let is = |f: &PathBuf| *f == folder;
        // A change during a listing is kept for when that listing lands.
        if self.listing.as_ref().is_some_and(|(f, _)| is(f)) {
            return;
        }
        let touched = self.slides_touched.swap(false, Ordering::AcqRel);
        if !touched && self.listed.as_ref().is_some_and(|(f, _)| is(f)) {
            return;
        }
        if let Some(w) = &mut self.folder_watch {
            w.follow(&folder);
        }
        let dir = folder.clone();
        self.listing = Some((
            folder,
            self.spawn(move || reprisefs::pictures::in_folder(&dir)),
        ));
    }

    /// The Picture / Slideshow combo moved: a slideshow starts on [`backgroundpage::slideshow_start`] (else Pictures), and Picture keeps the folder it leaves for the next switch.
    pub(crate) fn pick_mode(&mut self) {
        let page = &self.sheet.pages[Page::Background.index()];
        let slides = page.combo_selected(backgroundpage::MODE) == Some(1);
        if slides == backgroundpage::slideshow(&self.want) {
            return;
        }
        if slides {
            self.for_all_monitors(OwnKey::Wallpaper);
            self.want.slideshow =
                backgroundpage::slideshow_start(&self.last_slideshow, &self.want.wallpaper)
                    .unwrap_or_else(|| wallpapers::start_dir().to_string_lossy().into_owned());
        } else {
            self.last_slideshow = std::mem::take(&mut self.want.slideshow);
            if let Some(path) = &self.state_path {
                winstate::save_slideshow(path, &self.last_slideshow);
            }
        }
        self.fill_wallpapers();
    }

    /// A Background row was picked; the file the config already names keeps its spelling (`~/...`).
    pub(crate) fn pick_wallpaper(&mut self) {
        let row = backgroundpage::selected(&self.sheet.pages[Page::Background.index()]);
        if backgroundpage::slideshow(&self.bg()) {
            let folders = self.slide_folders();
            if let Some(f) = row.and_then(|i| folders.get(i)) {
                if wallpapers::expand(&self.want.slideshow).as_ref() != Some(f) {
                    self.want.slideshow = f.to_string_lossy().into_owned();
                }
                self.for_all_monitors(OwnKey::Wallpaper);
            }
            return;
        }
        let picked = match row.map(backgroundpage::row) {
            Some(backgroundpage::Row::Builtin(b)) => {
                self.set_wallpaper(b.uri());
                return;
            }
            Some(backgroundpage::Row::Picture(i)) => self.wallpapers.get(i).cloned(),
            _ => None,
        };
        if picked.is_some() && picked == wallpapers::expand(&self.bg().wallpaper) {
            if self.bg_target() == Target::All {
                self.for_all_monitors(OwnKey::Wallpaper);
            }
            return;
        }
        self.set_wallpaper(
            picked
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
    }

    /// Browse... picked `path`: it joins the list if absent and becomes pending, keeping the config's spelling.
    pub(crate) fn browsed(&mut self, path: PathBuf) {
        // The dialog has just listed `path`, and the rows it is held to
        // are local folders the scan read, so the disk answers at once.
        wallpapers::place(&mut self.wallpapers, &path);
        if wallpapers::expand(&self.bg().wallpaper).as_ref() != Some(&path) {
            self.set_wallpaper(path.to_string_lossy().into_owned());
        } else if self.bg_target() == Target::All {
            self.for_all_monitors(OwnKey::Wallpaper);
        }
        self.fill_wallpapers();
        self.want_preview();
        pages::follow(&mut self.sheet, &self.want);
        self.show_monitor();
    }

    /// Browse... picked the slideshow folder `path`.
    pub(crate) fn slide_folder(&mut self, path: PathBuf) {
        if wallpapers::expand(&self.want.slideshow).as_ref() != Some(&path) {
            self.want.slideshow = path.to_string_lossy().into_owned();
        }
        self.for_all_monitors(OwnKey::Wallpaper);
        self.fill_wallpapers();
        self.want_preview();
        pages::follow(&mut self.sheet, &self.want);
        self.show_monitor();
    }

    /// Decode the pending wallpaper for the monitor unless shown or a decode is under way; that one's end calls this again. A slideshow folder is listed first.
    pub(crate) fn want_preview(&mut self) {
        self.list_slide_folder();
        let want =
            backgroundpage::monitor_file(&self.bg_preview(), self.listed(), self.live_slide());
        let Some(file) = backgroundpage::next_decode(
            &want,
            self.picture.as_ref().map(|p| p.path.as_str()),
            self.decoding.is_some(),
        ) else {
            return;
        };
        // A built-in is drawn at the screen's shape, as large as a picture is decoded, so fine lines shrink smoothly.
        let rx = match backgroundpage::builtin(&want) {
            Some(b) => self.spawn(move || {
                let (w, h) = backgroundpage::DECODE_MAX;
                let canvas = backgroundpage::draw_builtin(b, &Default::default(), w, h);
                Some(Decoded {
                    canvas,
                    width: w,
                    height: h,
                })
            }),
            None => self.spawn(move || {
                Decoders::default_set().decode_file(&file, backgroundpage::DECODE_MAX)
            }),
        };
        self.decoding = Some((want, rx));
    }
}
