//! Folder Options: a 360x376 sheet with General, View and File Types tabs over [`Settings`],
//! which the host reads from and writes to `config.toml`.
//!
//! General picks where a folder opens and whether a click or a double click opens an item; View
//! holds hidden items, known types' extensions, item descriptions and the thumbnail cache. Each
//! of the two has Restore Defaults, which puts back its own page's boxes. File Types points at
//! Default Programs, which the host opens.

use crate::controls::form::{Event as FormEvent, Form, Item};
use crate::controls::propsheet::{self, PropSheet};
use crate::raster::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub new_window: bool,
    pub single_click: bool,
    pub show_hidden: bool,
    pub hide_extensions: bool,
    pub tips: bool,
    pub cache_thumbnails: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            new_window: false,
            single_click: false,
            show_hidden: false,
            hide_extensions: true,
            tips: true,
            cache_thumbnails: true,
        }
    }
}

/// What a sheet event asks of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// File Types' button: open Default Programs.
    DefaultPrograms,
}

pub const SIZE: (i32, i32) = (360, 376);
pub const TABS: [&str; 3] = ["General", "View", "File Types"];
pub const DEFAULTS: &str = "defaults";
pub const PROGRAMS: &str = "programs";

/// The sheet and what its boxes say against what was last saved, so Apply greys when they agree.
#[derive(Clone, Debug, PartialEq)]
pub struct FolderSheet {
    pub pending: Settings,
    pub applied: Settings,
    pub sheet: PropSheet,
}

/// Restore Defaults, at the page's bottom right.
fn restore_defaults() -> Item {
    Item::button_at(DEFAULTS, 250, 306, 95, "Restore &Defaults", true)
}

impl FolderSheet {
    /// Opened on General.
    pub fn new(settings: Settings) -> FolderSheet {
        let mut s = FolderSheet {
            pending: settings,
            applied: settings,
            sheet: PropSheet::new(&[], Vec::new(), SIZE),
        };
        s.sheet = PropSheet::new(&TABS, s.pages(), SIZE);
        s.sheet.set_apply_enabled(false);
        s
    }

    pub(crate) fn page_general(&self) -> Form {
        let p = &self.pending;
        Form::new(vec![
            Item::group("browse", Rect::new(15, 40, 330, 70), "Browse folders"),
            Item::radio(
                "same",
                27,
                60,
                "Open each folder in the s&ame window",
                "browse",
            )
            .checked(!p.new_window),
            Item::radio(
                "own",
                27,
                82,
                "Open each folder in its o&wn window",
                "browse",
            )
            .checked(p.new_window),
            Item::group(
                "click",
                Rect::new(15, 120, 330, 70),
                "Click items as follows",
            ),
            Item::radio(
                "single",
                27,
                140,
                "&Single-click to open an item (point to select)",
                "click",
            )
            .checked(p.single_click),
            Item::radio(
                "double",
                27,
                162,
                "D&ouble-click to open an item (single-click to select)",
                "click",
            )
            .checked(!p.single_click),
            restore_defaults(),
        ])
    }

    pub(crate) fn page_view(&self) -> Form {
        let p = &self.pending;
        Form::new(vec![
            Item::group(
                "hidden",
                Rect::new(15, 40, 330, 70),
                "Hidden files and folders",
            ),
            Item::radio(
                "hide",
                27,
                60,
                "Do &not show hidden files and folders",
                "hidden",
            )
            .checked(!p.show_hidden),
            Item::radio("show", 27, 82, "&Show hidden files and folders", "hidden")
                .checked(p.show_hidden),
            Item::check(
                "extensions",
                21,
                124,
                "&Hide extensions for known file types",
            )
            .checked(p.hide_extensions),
            Item::check("tips", 21, 146, "Show &pop-up description for folder items")
                .checked(p.tips),
            Item::check("thumbs", 21, 168, "Do not cache &thumbnails").checked(!p.cache_thumbnails),
            restore_defaults(),
        ])
    }

    pub(crate) fn page_types(&self) -> Form {
        Form::new(vec![
            Item::lines_at(
                "types",
                15,
                42,
                330,
                &[
                    "The program that opens each type of file is picked",
                    "in Default Programs.",
                ],
            ),
            Item::button_at(PROGRAMS, 15, 78, 120, "Default &Programs...", true),
        ])
    }

    fn pages(&self) -> Vec<Form> {
        vec![self.page_general(), self.page_view(), self.page_types()]
    }

    fn dirty(&self) -> bool {
        self.pending != self.applied
    }

    /// The pages built again from `pending`, keeping the focus and the pointer.
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

    /// The boxes show `settings`, saved elsewhere, where the user has not changed them here.
    pub fn saved_elsewhere(&mut self, settings: Settings) {
        if self.pending == self.applied {
            self.pending = settings;
        }
        self.applied = settings;
        self.rebuild();
    }

    /// The sheet's event: OK and Apply hand the settings over, Apply keeping the sheet open.
    pub fn event(&mut self, ev: propsheet::Event) -> Outcome {
        match ev {
            propsheet::Event::Redraw => Outcome::Redraw,
            propsheet::Event::Cancel => Outcome::Close,
            propsheet::Event::Ok => Outcome::Save {
                settings: self.pending,
                close: true,
            },
            propsheet::Event::Apply => {
                self.applied = self.pending;
                self.rebuild();
                Outcome::Save {
                    settings: self.pending,
                    close: false,
                }
            }
            propsheet::Event::Page(_, FormEvent::Pressed(PROGRAMS)) => Outcome::DefaultPrograms,
            propsheet::Event::Page(page, FormEvent::Pressed(DEFAULTS)) => {
                let d = Settings::default();
                let p = &mut self.pending;
                match page {
                    0 => (p.new_window, p.single_click) = (d.new_window, d.single_click),
                    _ => {
                        let kept = (p.new_window, p.single_click);
                        *p = d;
                        (p.new_window, p.single_click) = kept;
                    }
                }
                self.rebuild();
                Outcome::Redraw
            }
            propsheet::Event::Page(page, FormEvent::Changed(_)) => self.changed(page),
            propsheet::Event::Page(..) => Outcome::Redraw,
            propsheet::Event::None | propsheet::Event::Link(_) | propsheet::Event::Search => {
                Outcome::None
            }
        }
    }

    /// Reads the open page's boxes into `pending`.
    fn changed(&mut self, page: usize) -> Outcome {
        let Some(form) = self.sheet.pages.get(page) else {
            return Outcome::None;
        };
        let on = |id: &str| form.checked(id) == Some(true);
        let p = &mut self.pending;
        match page {
            0 => {
                p.new_window = on("own");
                p.single_click = on("single");
            }
            1 => {
                p.show_hidden = on("show");
                p.hide_extensions = on("extensions");
                p.tips = on("tips");
                p.cache_thumbnails = !on("thumbs");
            }
            _ => return Outcome::Redraw,
        }
        let dirty = self.dirty();
        self.sheet.set_apply_enabled(dirty);
        Outcome::Redraw
    }
}

#[cfg(test)]
mod tests;
