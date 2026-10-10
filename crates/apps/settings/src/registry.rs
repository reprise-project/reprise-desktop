//! How each page of the Reprise Settings sheet is built. Which pages exist
//! and their headings live in [`repriseconf::pages`], shared with the
//! compositor's search; a page's index in `PropSheet::pages` is its place in [`GROUPS`].

use reprisefs::xkb::Layout;
use reprisekit::controls::form::Form;
use reprisekit::controls::propsheet::PropSheet;

use crate::model::Settings;
use crate::pages::{self, Area};

pub use repriseconf::pages::{GROUPS, Page};

/// What the pages offer besides the settings: installed themes, Qt tools,
/// keyboard layouts and the compositor's outputs.
pub struct Sources<'a> {
    pub icon_themes: &'a [String],
    pub gtk_themes: &'a [String],
    pub qt_tools: bool,
    pub cursor_themes: &'a [String],
    pub layouts: &'a [Layout],
    pub heads: &'a [repriseapp::heads::Head],
}

/// Page `p` for area `a` before any value is pushed in. The Appearance,
/// Display and Tablet pages depend on what they show; the host rebuilds
/// those ([`crate::appearancepage::follow`], [`crate::displaypage::refresh`],
/// `tabletpage::page`).
pub fn build(p: Page, src: &Sources, a: Area) -> Form {
    match p {
        Page::Appearance => crate::appearancepage::page(&Settings::default(), a),
        Page::Background => crate::backgroundpage::page(a),
        Page::ScreenSaver => crate::saverpage::page(a),
        Page::Display => crate::displaypage::page(
            &crate::displaypage::Display::with_heads(src.heads.to_vec()),
            a,
        ),
        Page::NightLight => pages::night_light(a),
        Page::FontsAndIcons => {
            pages::fonts_and_icons(a, src.icon_themes, src.gtk_themes, src.qt_tools)
        }
        Page::Effects => pages::effects(a),
        Page::Taskbar => pages::taskbar(a),
        Page::StartMenu => crate::menupage::page(a),
        Page::Launcher => pages::launcher(a),
        Page::Switcher => pages::switcher(a),
        Page::Desktops => pages::desktops(a),
        Page::Windows => pages::windows(a),
        Page::Keyboard => pages::keyboard(a),
        Page::Mouse => pages::mouse(a),
        Page::Tablet => crate::tabletpage::page(a, &[], 0, &[]),
        Page::Pointers => pages::pointers(a, src.cursor_themes),
        Page::Accessibility => pages::accessibility(a),
        Page::Vision => crate::visionpage::page(a),
        Page::Power => pages::power(a),
        Page::Startup => pages::startup(a),
        Page::DefaultPrograms => crate::defaultspage::page(a),
        Page::Advanced => pages::advanced(a),
    }
}

/// The whole sheet at [`Area::MIN`] with `s` pushed in ([`pages::write`]).
pub fn sheet(s: &Settings, src: &Sources) -> PropSheet {
    let labels: Vec<Vec<&str>> = GROUPS
        .iter()
        .map(|(_, ps)| ps.iter().map(|p| p.label()).collect())
        .collect();
    let groups: Vec<(&str, &[&str])> = GROUPS
        .iter()
        .zip(&labels)
        .map(|((heading, _), l)| (*heading, l.as_slice()))
        .collect();
    let forms = Page::all().map(|p| build(p, src, Area::MIN)).collect();
    let mut sheet = PropSheet::with_list(&groups, forms, pages::CLIENT);
    pages::write(&mut sheet, s, src.layouts);
    sheet
}

pub fn open(sheet: &PropSheet) -> Page {
    Page::at(sheet.tabs.open).expect("one page per row of the list")
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::controls::pagelist;
    use reprisekit::raster::Rect;
    use reprisekit::text::TextRenderer;
    use reprisekit::theme::{FontConfig, Theme};

    pub fn built() -> PropSheet {
        let themes = vec!["hicolor".to_string()];
        sheet(
            &Settings::default(),
            &Sources {
                icon_themes: &themes,
                gtk_themes: &[],
                qt_tools: false,
                cursor_themes: &[],
                layouts: &[],
                heads: &[],
            },
        )
    }

    #[test]
    fn four_headings_over_the_pages() {
        let labels: Vec<&str> = Page::all().map(Page::label).collect();
        assert_eq!(
            labels,
            [
                "Appearance",
                "Background",
                "Screen Saver",
                "Display",
                "Night Light",
                "Fonts & Icons",
                "Effects",
                "Taskbar",
                "Start Menu",
                "Launcher",
                "Window Switcher",
                "Desktops",
                "Windows",
                "Keyboard",
                "Mouse",
                "Tablet",
                "Pointers",
                "Accessibility",
                "Vision",
                "Power",
                "Startup",
                "Default Programs",
                "Advanced",
            ]
        );
        let headings: Vec<&str> = GROUPS.iter().map(|g| g.0).collect();
        assert_eq!(headings, ["Look", "Desktop", "Input", "System"]);
        for (i, p) in Page::all().enumerate() {
            assert_eq!((p.index(), Page::at(i)), (i, Some(p)));
        }
        assert_eq!(Page::at(Page::all().count()), None);
    }

    #[test]
    fn a_page_is_named_by_its_label_in_lower_case() {
        assert_eq!(Page::FontsAndIcons.slug(), "fonts-and-icons");
        assert_eq!(Page::named("fonts-and-icons"), Some(Page::FontsAndIcons));
        assert_eq!(Page::named("display"), Some(Page::Display));
        assert_eq!(Page::named("night-light"), Some(Page::NightLight));
        assert_eq!(Page::named("accessibility"), Some(Page::Accessibility));
        assert_eq!(Page::named("vision"), Some(Page::Vision));
        assert_eq!(Page::named("taskbar"), Some(Page::Taskbar));
        assert_eq!(Page::named("start-menu"), Some(Page::StartMenu));
        assert_eq!(Page::named("startup"), Some(Page::Startup));
        assert_eq!(Page::named("window-switcher"), Some(Page::Switcher));
        assert_eq!(Page::named("appearance"), Some(Page::Appearance));
        assert_eq!(Page::named("windows"), Some(Page::Windows));
        assert_eq!(Page::named("background"), Some(Page::Background));
        assert_eq!(Page::named("autostart"), None, "the page is Startup");
        assert_eq!(Page::named("Night Light"), None);
    }

    #[test]
    fn the_sheet_is_the_pages_client_plus_the_list() {
        let s = built();
        assert_eq!(
            s.size,
            (pages::CLIENT.0 + pagelist::ADVANCE, pages::CLIENT.1)
        );
        assert_eq!(s.size, (544, 528));
        assert_eq!(s.pages.len(), Page::all().count());
        assert_eq!(
            s.tabs.labels,
            Page::all().map(Page::label).collect::<Vec<_>>()
        );
        assert_eq!(open(&s), GROUPS[0].1[0], "the first page opens");
    }

    /// The list's inside is 480 px: 27 rows at 471 px, so every row shows. On a
    /// sheet 56 px shorter the list scrolls, and a page opened from its end (keys
    /// or `--page`) comes into view.
    #[test]
    fn the_list_scrolls_to_the_open_page() {
        use reprisekit::controls::form::Key;
        use reprisekit::controls::propsheet::Hit;
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut sheet = built();
        let list = sheet.layout(&text, px).list.expect("a list sheet");
        assert_eq!(list.rows.len(), 27, "four headings over 23 pages");
        assert_eq!((list.total, list.inner.height), (471, 480));
        assert!(list.scrollbar.is_none(), "every row shows");

        sheet.size.1 -= 56;
        let list = sheet.layout(&text, px).list.unwrap();
        assert_eq!((list.total, list.inner.height), (471, 424));
        let bar = list.scrollbar.expect("the rows overflow");
        assert_eq!(bar.right(), list.inner.right() + bar.width);
        let last = *list.rows.last().unwrap();
        assert!(
            last.bottom() > list.inner.bottom(),
            "Advanced is out of view"
        );

        let row = |s: &PropSheet| {
            let l = s.layout(&text, px).list.unwrap();
            let i = l.rows.len() - 1;
            (l.rows[i], l.inner)
        };
        sheet.tabs.open = Page::Advanced.index();
        assert!(sheet.reveal_open());
        let (r, inner) = row(&sheet);
        assert_eq!(r.bottom(), inner.bottom(), "Advanced at the bottom");
        assert_eq!(
            sheet.hit(r.x + 20, r.y + 8, &text, px),
            Some(Hit::Tab(Page::Advanced.index()))
        );

        sheet.click(Some(Hit::Tab(Page::Advanced.index())));
        assert!(sheet.tabs.focused);
        sheet.key(Key::Home, None);
        let l = sheet.layout(&text, px).list.unwrap();
        assert_eq!(l.top, 0, "Home scrolls back to the Look heading");
        sheet.key(Key::End, None);
        assert_eq!(row(&sheet).0.bottom(), row(&sheet).1.bottom());
    }

    const LARGE: (i32, i32) = (1400, 900);

    const SIDE_LIST: &str = "side_list";

    /// Every page in `a` at its fullest: the Glass Appearance page, two outputs,
    /// a pad with two columns of rows. A page with a host-drawn list carries its
    /// rect as an item, so nothing may cover it.
    fn every_page(a: Area) -> Vec<(Page, Form)> {
        let themes = vec!["hicolor".to_string()];
        let src = Sources {
            icon_themes: &themes,
            gtk_themes: &themes,
            qt_tools: true,
            cursor_themes: &themes,
            layouts: &[],
            heads: &[],
        };
        let glass = Settings {
            interface: "glass".into(),
            ..Settings::default()
        };
        let display = crate::displaypage::Display::with_heads(vec![
            crate::displaypage::test_head("DP-1"),
            crate::displaypage::test_head("HDMI-A-1"),
        ]);
        let pad = reprisebus::tray::Pad {
            name: "Pad".into(),
            buttons: 18,
            rings: 1,
            strips: 1,
        };
        let side = |r| {
            reprisekit::controls::form::Item::new(
                SIDE_LIST,
                r,
                reprisekit::controls::form::Control::Picture {
                    canvas: None,
                    frame: 2,
                },
            )
        };
        Page::all()
            .map(|p| {
                let mut f = match p {
                    Page::Appearance => crate::appearancepage::page(&glass, a),
                    Page::Display => crate::displaypage::page(&display, a),
                    Page::Tablet => crate::tabletpage::page(a, std::slice::from_ref(&pad), 0, &[]),
                    _ => build(p, &src, a),
                };
                let list = match p {
                    Page::Startup => Some(pages::list(a)),
                    Page::Keyboard => Some(pages::shortcuts(a)),
                    Page::Advanced => Some(pages::locations(a)),
                    Page::DefaultPrograms => Some(crate::defaultspage::list(a)),
                    _ => None,
                };
                f.items.extend(list.map(side));
                (p, f)
            })
            .collect()
    }

    /// Item `it`'s placed rect, widened for a check box's or radio's label.
    fn extent(
        it: &reprisekit::controls::form::Item,
        cm: &reprisekit::controls::metrics::ControlMetrics,
        text: &TextRenderer,
        px: u32,
    ) -> Rect {
        use reprisekit::controls::form::Control;
        let r = it.placed(cm);
        match &it.control {
            Control::Check { label, .. } | Control::Radio { label, .. } => {
                let (shown, _) = reprisekit::text::split_mnemonic(label);
                let w = cm.check_label + text.measure(&shown, false, px);
                Rect::new(r.x, r.y, w, r.height)
            }
            _ => r,
        }
    }

    fn overlap(a: Rect, b: Rect) -> bool {
        a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
    }

    fn inside(inner: Rect, outer: Rect) -> bool {
        inner.x >= outer.x
            && inner.y >= outer.y
            && inner.right() <= outer.right()
            && inner.bottom() <= outer.bottom()
    }

    /// At the smallest, default and a large size, in every look, each page keeps
    /// to its column: no control covers another (check box labels included),
    /// each sits wholly inside or outside each group box, and group boxes don't overlap.
    #[test]
    fn no_control_covers_another_or_leaves_the_page_at_any_size() {
        use reprisekit::controls::form::Control;
        use reprisekit::interface::Interface;
        let sizes = [Area::MIN.sheet_size(), crate::winstate::DEFAULT, LARGE];
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = reprisekit::interfaces::theme_for(i);
            let text = TextRenderer::load(&theme.fonts);
            let (cm, px) = (theme.look.controls(), theme.fonts.size_px);
            for size in sizes {
                let a = Area::of_size(size);
                let page = Rect::new(6, 28, a.w + 28, 464 + a.dh);
                for (p, f) in every_page(a) {
                    let rects: Vec<(&str, bool, Rect)> = f
                        .items
                        .iter()
                        .map(|it| {
                            let group = matches!(it.control, Control::Group(_));
                            (it.id, group, extent(it, &cm, &text, px))
                        })
                        .collect();
                    for (k, &(id, group, r)) in rects.iter().enumerate() {
                        let at = format!("{i:?} {size:?} {}: {id} at {r:?}", p.label());
                        assert!(inside(r, page), "{at} leaves the page {page:?}");
                        if !group {
                            assert!(r.right() <= a.right(), "{at} passes the column");
                        }
                        for &(other, other_group, o) in &rects[k + 1..] {
                            let clash = match (group, other_group) {
                                (false, false) | (true, true) => overlap(r, o),
                                (true, false) => overlap(r, o) && !inside(o, r),
                                (false, true) => overlap(r, o) && !inside(r, o),
                            };
                            assert!(!clash, "{at} overlaps {other} at {o:?}");
                        }
                    }
                }
            }
        }
    }

    /// From the smallest to a large size, content holders grow (group boxes,
    /// fields, lists, the tree, previews); combos and trackbars grow no more than
    /// their most; check boxes, radios, buttons, up-downs, colour buttons and time
    /// fields keep their size. The page's list or preview takes the extra height.
    #[test]
    fn content_grows_with_the_window_and_fixed_controls_keep_their_size() {
        use reprisekit::controls::form::Control;
        let (small, large) = (Area::MIN, Area::of_size(LARGE));
        assert!(large.dw() > 0 && large.dh > 0);
        let before = every_page(small);
        for ((p, f), (_, g)) in before.iter().zip(every_page(large)) {
            assert_eq!(f.items.len(), g.items.len(), "{}", p.label());
            for (it, now) in f.items.iter().zip(&g.items) {
                let (r, n) = (it.rect, now.rect);
                let at = format!("{}: {} from {r:?} to {n:?}", p.label(), it.id);
                assert_eq!(it.id, now.id);
                match &it.control {
                    Control::Group(_) if r.width == small.w => {
                        assert_eq!(n.width, large.w, "{at}")
                    }
                    Control::Group(_) => assert_eq!(n.width, r.width, "{at}"),
                    Control::Edit { .. }
                    | Control::ListBox { .. }
                    | Control::Tree(_)
                    | Control::Sample { .. }
                    | Control::Arrange(_)
                    | Control::Lines(_) => assert!(n.width > r.width, "{at}"),
                    Control::Combo(_) => assert!(
                        n.width >= r.width && n.width <= pages::COMBO_MAX.max(r.width),
                        "{at}"
                    ),
                    Control::TrackBar(_) => assert!(
                        n.width >= r.width && n.width <= pages::TRACK_MAX.max(r.width),
                        "{at}"
                    ),
                    Control::Check { .. }
                    | Control::Radio { .. }
                    | Control::Button { .. }
                    | Control::UpDown(_)
                    | Control::Color(_)
                    | Control::Time(_)
                    | Control::Monitor(_) => {
                        assert_eq!((n.width, n.height), (r.width, r.height), "{at}")
                    }
                    _ => {}
                }
            }
        }
        let grower = |p: Page, id: &str| {
            let h = |pages: &[(Page, Form)]| {
                let (_, f) = &pages[p.index()];
                f.item(id).unwrap().rect.height
            };
            (h(&before), h(&every_page(large)))
        };
        for (p, id) in [
            (Page::Appearance, crate::appearancepage::SAMPLE),
            (Page::Background, crate::backgroundpage::WALLPAPER),
            (Page::Display, crate::displaypage::ARRANGE),
            (Page::StartMenu, crate::menupage::TREE),
            (Page::Pointers, pages::CURSOR_PREVIEW),
            (Page::Startup, SIDE_LIST),
            (Page::Keyboard, SIDE_LIST),
            (Page::Advanced, SIDE_LIST),
            (Page::DefaultPrograms, SIDE_LIST),
        ] {
            let (was, now) = grower(p, id);
            assert_eq!(now, was + large.dh, "{}: {id}", p.label());
        }
    }

    #[test]
    fn each_page_names_each_control_once() {
        for (p, f) in every_page(Area::MIN) {
            let mut ids: Vec<&str> = f.items.iter().map(|it| it.id).collect();
            ids.sort_unstable();
            let n = ids.len();
            ids.dedup();
            assert_eq!(ids.len(), n, "{}", p.label());
        }
    }

    #[test]
    fn the_area_follows_the_sheet_size() {
        assert_eq!(Area::MIN.sheet_size(), built().size);
        assert_eq!(Area::of_size((100, 100)), Area::MIN, "never smaller");
        let a = Area::of_size(LARGE);
        assert_eq!(a.sheet_size(), LARGE);
        assert_eq!((a.w, a.dh), (LARGE.0 - 186, LARGE.1 - 528));
    }
}
