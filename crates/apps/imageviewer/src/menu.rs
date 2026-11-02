//! The menu bar's rows and what each does.

use super::*;

fn mi(label: &str, shortcut: Option<&str>) -> MenuItem {
    MenuItem::item(label).shortcut(shortcut.map(str::to_string))
}

pub(crate) mod title {
    pub const FILE: usize = 0;
    pub const EDIT: usize = 1;
    pub const VIEW: usize = 2;
    pub const IMAGE: usize = 3;
}

pub(crate) mod row {
    pub const OPEN: usize = 0;
    pub const BACKGROUND: usize = 2;
    pub const EXIT: usize = 4;
    pub const COPY: usize = 0;
    pub const DELETE: usize = 1;
    pub const PREVIOUS: usize = 0;
    pub const NEXT: usize = 1;
    pub const FIRST: usize = 2;
    pub const LAST: usize = 3;
    pub const ZOOM_IN: usize = 5;
    pub const ZOOM_OUT: usize = 6;
    pub const FIT: usize = 7;
    pub const ACTUAL: usize = 8;
    pub const FULL_SCREEN: usize = 10;
    pub const SLIDESHOW: usize = 11;
    pub const STATUS_BAR: usize = 13;
    pub const ROTATE_CW: usize = 0;
    pub const ROTATE_CCW: usize = 1;
}

pub(crate) fn menus() -> Vec<(&'static str, Vec<MenuItem>)> {
    vec![
        (
            "&File",
            vec![
                mi("&Open...", Some("Ctrl+O")),
                MenuItem::Separator,
                mi("Set as Desktop &Background", None),
                MenuItem::Separator,
                mi("E&xit", None),
            ],
        ),
        (
            "&Edit",
            vec![mi("&Copy", Some("Ctrl+C")), mi("&Delete", Some("Del"))],
        ),
        (
            "&View",
            vec![
                mi("&Previous Picture", Some("Left")),
                mi("&Next Picture", Some("Right")),
                mi("&First Picture", Some("Home")),
                mi("&Last Picture", Some("End")),
                MenuItem::Separator,
                mi("Zoom &In", Some("+")),
                mi("Zoom O&ut", Some("-")),
                mi("Fit to &Window", Some("Ctrl+0")),
                mi("&Actual Size", Some("Ctrl+1")),
                MenuItem::Separator,
                mi("F&ull Screen", Some("F11")),
                mi("&Slide Show", Some("F5")),
                MenuItem::Separator,
                mi("Status &Bar", None).checked(true),
            ],
        ),
        (
            "&Image",
            vec![
                mi("Rotate &Clockwise", Some("Ctrl+R")),
                mi("Rotate C&ounterclockwise", Some("Ctrl+L")),
            ],
        ),
    ]
}

impl Viewer {
    /// An accelerator reaches here without the menu, so a greyed row refuses it too.
    pub(crate) fn command(&mut self, title: usize, row: usize, ctx: &Ctx) -> Reply {
        if !matches!(
            self.bar.menus[title][row],
            MenuItem::Entry { enabled: true, .. }
        ) {
            return Reply::none();
        }
        let at = None;
        match (title, row) {
            (title::FILE, row::OPEN) => self.open_dialog(),
            (title::FILE, row::BACKGROUND) => return self.set_background(),
            (title::FILE, row::EXIT) => {
                return Reply {
                    quit: true,
                    ..Reply::none()
                };
            }
            (title::EDIT, row::COPY) => return self.copy(),
            (title::EDIT, row::DELETE) => return self.ask_delete(false),
            (title::VIEW, row::PREVIOUS) => return self.go(Step::Previous),
            (title::VIEW, row::NEXT) => return self.go(Step::Next),
            (title::VIEW, row::FIRST) => return self.go(Step::First),
            (title::VIEW, row::LAST) => return self.go(Step::Last),
            (title::VIEW, row::ZOOM_IN) => self.zoom_step(true, at, ctx),
            (title::VIEW, row::ZOOM_OUT) => self.zoom_step(false, at, ctx),
            (title::VIEW, row::FIT) => self.view.fit(),
            (title::VIEW, row::ACTUAL) => {
                let lay = self.view_layout(ctx);
                self.view.zoom_to(1.0, at, &lay);
            }
            (title::VIEW, row::FULL_SCREEN) => return self.full_screen(!self.full),
            (title::VIEW, row::SLIDESHOW) => return self.slideshow(true),
            (title::VIEW, row::STATUS_BAR) => {
                self.status_bar = !self.status_bar;
                if let MenuItem::Entry { checked, .. } =
                    &mut self.bar.menus[title::VIEW][row::STATUS_BAR]
                {
                    *checked = self.status_bar;
                }
            }
            (title::IMAGE, row::ROTATE_CW) => self.view.turn(true),
            (title::IMAGE, row::ROTATE_CCW) => self.view.turn(false),
            _ => {}
        }
        Reply::redraw()
    }

    /// Grey the rows that cannot act on what shows.
    pub(crate) fn sync_menus(&mut self) {
        let file = self.folder.current().is_some();
        let shown = self.picture.is_some();
        let many = self.folder.pictures.len() > 1;
        let background = self
            .folder
            .current()
            .is_some_and(reprisefs::pictures::is_picture);
        let live = [
            (title::FILE, row::BACKGROUND, shown && background),
            (title::EDIT, row::COPY, file),
            (title::EDIT, row::DELETE, file && self.delete.is_none()),
            (title::VIEW, row::PREVIOUS, many),
            (title::VIEW, row::NEXT, many),
            (title::VIEW, row::FIRST, many),
            (title::VIEW, row::LAST, many),
            (title::VIEW, row::ZOOM_IN, shown),
            (title::VIEW, row::ZOOM_OUT, shown),
            (title::VIEW, row::FIT, shown),
            (title::VIEW, row::ACTUAL, shown),
            (title::VIEW, row::SLIDESHOW, file),
            (title::IMAGE, row::ROTATE_CW, shown),
            (title::IMAGE, row::ROTATE_CCW, shown),
        ];
        for (t, r, on) in live {
            if let MenuItem::Entry { enabled, .. } = &mut self.bar.menus[t][r] {
                *enabled = on;
            }
        }
    }
}
