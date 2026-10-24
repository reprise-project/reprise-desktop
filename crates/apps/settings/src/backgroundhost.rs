use crate::registry::Page;
use crate::{SettingsApp, backgroundpage, pages, wallpapers, winstate};
use repriseapp::decode::{Decoded, Decoders};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

impl SettingsApp {
    /// Refill the Background list from `wallpapers` with the pending one picked and scrolled into view (once, when the scan lands).
    pub(crate) fn fill_wallpapers(&mut self) {
        let folders = self.slide_folders();
        let page = &mut self.sheet.pages[Page::Background.index()];
        if backgroundpage::slideshow(&self.want) {
            backgroundpage::fill_folders(
                page,
                &folders,
                &self.want.slideshow,
                self.folder_icon.as_ref(),
            );
        } else {
            backgroundpage::fill(
                page,
                &self.wallpapers,
                &self.want.wallpaper,
                self.wallpaper_icons.as_ref(),
                &self.thumbs,
            );
        }
    }

    pub(crate) fn slide_folders(&self) -> Vec<PathBuf> {
        backgroundpage::folders(&self.wallpapers, wallpapers::expand(&self.want.slideshow))
    }

    pub(crate) fn show_monitor(&mut self) {
        let file = backgroundpage::monitor_file(&self.want, self.listed());
        backgroundpage::show(
            &mut self.sheet.pages[Page::Background.index()],
            &self.want,
            &file,
            self.picture.as_ref(),
            self.listing.is_some() || self.decoding.is_some(),
        );
    }

    fn listed(&self) -> &[PathBuf] {
        self.listed.as_ref().map_or(&[], |(_, p)| p.as_slice())
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
        if backgroundpage::slideshow(&self.want) {
            let folders = self.slide_folders();
            if let Some(f) = row.and_then(|i| folders.get(i))
                && wallpapers::expand(&self.want.slideshow).as_ref() != Some(f)
            {
                self.want.slideshow = f.to_string_lossy().into_owned();
            }
            return;
        }
        let picked = match row.map(backgroundpage::row) {
            Some(backgroundpage::Row::Builtin(b)) => {
                self.want.wallpaper = b.uri();
                return;
            }
            Some(backgroundpage::Row::Picture(i)) => self.wallpapers.get(i),
            _ => None,
        };
        if picked.is_some() && picked.cloned() == wallpapers::expand(&self.want.wallpaper) {
            return;
        }
        self.want.wallpaper = picked
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
    }

    /// Browse... picked `path`: it joins the list if absent and becomes pending, keeping the config's spelling.
    pub(crate) fn browsed(&mut self, path: PathBuf) {
        // The dialog has just listed `path`, and the rows it is held to
        // are local folders the scan read, so the disk answers at once.
        wallpapers::place(&mut self.wallpapers, &path);
        if wallpapers::expand(&self.want.wallpaper).as_ref() != Some(&path) {
            self.want.wallpaper = path.to_string_lossy().into_owned();
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
        self.fill_wallpapers();
        self.want_preview();
        pages::follow(&mut self.sheet, &self.want);
        self.show_monitor();
    }

    /// Decode the pending wallpaper for the monitor unless shown or a decode is under way; that one's end calls this again. A slideshow folder is listed first.
    pub(crate) fn want_preview(&mut self) {
        self.list_slide_folder();
        let want = backgroundpage::monitor_file(&self.want, self.listed());
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
