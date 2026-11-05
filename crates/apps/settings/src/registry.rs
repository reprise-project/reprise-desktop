//! How each page of the Reprise Settings sheet is built. Which pages exist
//! and their headings live in [`repriseconf::pages`], shared with the
//! compositor's search; a page's index in `PropSheet::pages` is its place in [`Page::all`].

use reprisefs::xkb::Layout;
use reprisekit::controls::form::Form;
use reprisekit::controls::propsheet::PropSheet;

use crate::model::Settings;
use crate::pages::{self, Area};

pub use repriseconf::pages::{Entry, GROUPS, Page};

/// What the pages offer besides the settings: installed themes (sound themes included), Qt tools,
/// keyboard layouts and the compositor's outputs.
pub struct Sources<'a> {
    pub icon_themes: &'a [String],
    pub gtk_themes: &'a [String],
    pub qt_tools: bool,
    pub cursor_themes: &'a [String],
    pub sound_themes: &'a [String],
    pub layouts: &'a [Layout],
    pub heads: &'a [repriseapp::heads::Head],
}

/// Page `p` for area `a` before any value is pushed in. The Appearance,
/// Display and Tablet pages depend on what they show; the host rebuilds
/// those ([`crate::appearancepage::follow`], [`crate::displaypage::refresh`],
/// `tabletpage::page`). Remote Desktop starts off and `remotehost` fills it in.
pub fn build(p: Page, src: &Sources, a: Area) -> Form {
    match p {
        Page::Appearance => crate::appearancepage::page(&Settings::default(), a),
        Page::Background => crate::backgroundpage::page(a),
        Page::ScreenSaverLock => crate::saverpage::page(a),
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
        Page::Search => pages::search(a),
        Page::Switcher => pages::switcher(a),
        Page::VirtualDesktops => pages::virtual_desktops(a),
        Page::WindowBehavior => pages::window_behavior(a),
        Page::Keyboard => pages::keyboard(a),
        Page::MouseTouchpad => pages::mouse_touchpad(a),
        Page::Tablet => crate::tabletpage::page(a, &[], 0, &[]),
        Page::Cursor => pages::cursor(a, src.cursor_themes),
        Page::TextKeys => pages::text_and_keys(a),
        Page::MagnifierFilters => crate::magnifierpage::page(a),
        Page::Sound => pages::sound(a, src.sound_themes),
        Page::Power => pages::power(a),
        Page::Startup => pages::startup(a),
        Page::DefaultPrograms => crate::defaultspage::page(a),
        Page::RemoteDesktop => pages::remote_desktop(a, None, false),
        Page::Advanced => pages::advanced(a),
        Page::Backup => pages::backup(a),
    }
}

/// Each heading of [`GROUPS`] with its pages' labels, in list order. Program entries
/// get no row.
fn page_rows() -> Vec<(&'static str, Vec<&'static str>)> {
    GROUPS
        .iter()
        .map(|(heading, entries)| {
            let labels = entries
                .iter()
                .filter_map(|e| match e {
                    Entry::Page(p) => Some(p.label()),
                    Entry::Program(_) => None,
                })
                .collect();
            (*heading, labels)
        })
        .collect()
}

/// The whole sheet at [`Area::MIN`] with `s` pushed in ([`pages::write`]).
pub fn sheet(s: &Settings, src: &Sources) -> PropSheet {
    let rows = page_rows();
    let groups: Vec<(&str, &[&str])> = rows.iter().map(|(h, l)| (*h, l.as_slice())).collect();
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
                sound_themes: &[],
                layouts: &[],
                heads: &[],
            },
        )
    }

    #[test]
    fn seven_headings_over_the_pages() {
        let labels: Vec<&str> = Page::all().map(Page::label).collect();
        assert_eq!(
            labels,
            [
                "Appearance",
                "Background",
                "Screen Saver & Lock",
                "Fonts & Icons",
                "Effects",
                "Taskbar",
                "Start Menu",
                "Search",
                "Window Switcher",
                "Virtual Desktops",
                "Window Behavior",
                "Display",
                "Night Light",
                "Sound",
                "Keyboard",
                "Mouse & Touchpad",
                "Cursor",
                "Tablet",
                "Text & Keys",
                "Magnifier & Filters",
                "Remote Desktop",
                "Power",
                "Startup",
                "Default Programs",
                "Advanced",
                "Backup",
            ]
        );
        let headings: Vec<&str> = GROUPS.iter().map(|g| g.0).collect();
        assert_eq!(
            headings,
            [
                "Personalize",
                "Taskbar & Windows",
                "Display & Sound",
                "Devices",
                "Accessibility",
                "Network",
                "System"
            ]
        );
        for (i, p) in Page::all().enumerate() {
            assert_eq!((p.index(), Page::at(i)), (i, Some(p)));
        }
        assert_eq!(Page::at(Page::all().count()), None);
    }

    #[test]
    fn a_page_is_named_by_its_slug_or_its_old_name() {
        use repriseconf::pages::Entry;
        assert_eq!(Page::ScreenSaverLock.slug(), "screen-saver-and-lock");
        assert_eq!(Page::MouseTouchpad.slug(), "mouse-and-touchpad");
        assert_eq!(Page::TextKeys.slug(), "text-and-keys");
        assert_eq!(Page::MagnifierFilters.slug(), "magnifier-and-filters");
        for p in Page::all() {
            assert_eq!(Page::named(&p.slug()), Some(p));
        }
        for (old, now) in [
            ("screen-saver", Page::ScreenSaverLock),
            ("launcher", Page::Search),
            ("desktops", Page::VirtualDesktops),
            ("windows", Page::WindowBehavior),
            ("mouse", Page::MouseTouchpad),
            ("pointers", Page::Cursor),
            ("accessibility", Page::TextKeys),
            ("vision", Page::MagnifierFilters),
            ("sounds", Page::Sound),
        ] {
            assert_eq!(Page::named(old), Some(now), "{old}");
            assert_eq!(
                repriseconf::pages::named(old),
                Some(Entry::Page(now)),
                "{old}"
            );
        }
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
        assert_eq!(open(&s), Page::Appearance, "the first page opens");
    }

    /// The list's inside is 480 px at the smallest sheet: 33 rows at 585 px, so it scrolls
    /// until the sheet is 105 px taller. On a sheet 56 px shorter, a page opened from its end
    /// (keys or `--page`) comes into view.
    #[test]
    fn the_list_scrolls_to_the_open_page() {
        use reprisekit::controls::form::Key;
        use reprisekit::controls::propsheet::Hit;
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut sheet = built();
        let list = sheet.layout(&text, px).list.expect("a list sheet");
        assert_eq!(list.rows.len(), 33, "seven headings over 26 pages");
        assert_eq!((list.total, list.inner.height), (585, 480));
        assert!(list.scrollbar.is_some(), "the rows overflow by 105 px");
        sheet.size.1 += 105;
        let list = sheet.layout(&text, px).list.unwrap();
        assert!(list.scrollbar.is_none(), "every row shows");

        sheet.size.1 -= 161;
        let list = sheet.layout(&text, px).list.unwrap();
        assert_eq!((list.total, list.inner.height), (585, 424));
        let bar = list.scrollbar.expect("the rows overflow");
        assert_eq!(bar.right(), list.inner.right() + bar.width);
        let last = *list.rows.last().unwrap();
        assert!(last.bottom() > list.inner.bottom(), "Backup is out of view");

        let row = |s: &PropSheet| {
            let l = s.layout(&text, px).list.unwrap();
            let i = l.rows.len() - 1;
            (l.rows[i], l.inner)
        };
        sheet.tabs.open = Page::Backup.index();
        assert!(sheet.reveal_open());
        let (r, inner) = row(&sheet);
        assert_eq!(r.bottom(), inner.bottom(), "Backup at the bottom");
        assert_eq!(
            sheet.hit(r.x + 20, r.y + 8, &text, px),
            Some(Hit::Tab(Page::Backup.index()))
        );

        sheet.click(Some(Hit::Tab(Page::Backup.index())));
        assert!(sheet.tabs.focused);
        sheet.key(Key::Home, None);
        let l = sheet.layout(&text, px).list.unwrap();
        assert_eq!(l.top, 0, "Home scrolls back to the Personalize heading");
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
            sound_themes: &themes,
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
            buttons: 13,
            rings: 1,
            strips: 1,
            dials: 1,
            keys: crate::tabletpage::PAD_KEYS.map(|k| k.0.to_string()).into(),
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

    /// From the smallest to a large size, content holders grow (group boxes that
    /// end at the column's right edge, fields, lists, the tree, previews); combos and trackbars grow no more than
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
                    Control::Group(_) if r.right() == small.right() => {
                        assert_eq!(n.right(), large.right(), "{at}")
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
            (Page::Cursor, pages::CURSOR_PREVIEW),
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

    /// Each control `f`'s access tree names, with the label it is named by
    /// (the rule `Form::access` follows), access keys dropped.
    fn control_labels(f: &Form) -> Vec<(&'static str, String)> {
        use reprisekit::controls::form::Control as C;
        let shown = |s: &str| reprisekit::text::split_mnemonic(s).0;
        let mut label = None;
        let mut out = Vec::new();
        for it in &f.items {
            let named = match &it.control {
                C::Label { text, .. } | C::GroupTitle(text) => {
                    label = Some(shown(text));
                    continue;
                }
                C::Lines(lines) | C::GreyedLines(lines) => {
                    label = Some(shown(&lines.join(" ")));
                    continue;
                }
                C::Group(_) => {
                    label = None;
                    continue;
                }
                C::Separator
                | C::Rule
                | C::Blank
                | C::Picture { .. }
                | C::Monitor(_)
                | C::Spectrum
                | C::Sample { .. }
                | C::Clock(_) => continue,
                C::Check { label: l, .. }
                | C::Radio { label: l, .. }
                | C::Button { label: l, .. } => Some(shown(l)),
                C::Edit { .. }
                | C::Combo(_)
                | C::ListBox { .. }
                | C::Tree(_)
                | C::UpDown(_)
                | C::TrackBar(_)
                | C::Color(_)
                | C::Calendar(_)
                | C::Time(_)
                | C::HotKey(_)
                | C::Memo(_)
                | C::IconList(_)
                | C::Arrange(_) => it.name.map(str::to_string).or(label),
            };
            label = None;
            out.extend(named.map(|l| (it.id, l)));
        }
        out
    }

    #[test]
    fn track_bars_are_named_after_their_setting() {
        let pages = every_page(Area::MIN);
        for (p, id, name) in [
            (Page::Display, crate::displaypage::MODE, "Screen resolution"),
            (Page::NightLight, "nl_temperature", "Color temperature"),
            (Page::MouseTouchpad, "mouse_speed", "Pointer speed"),
            (Page::MouseTouchpad, "touchpad_speed", "Touchpad speed"),
            (Page::MouseTouchpad, "double_click_ms", "Double-click speed"),
            (Page::TextKeys, "slow_keys_delay", "SlowKeys delay"),
            (Page::TextKeys, "mouse_keys_speed", "MouseKeys speed"),
            (
                Page::TextKeys,
                "mouse_keys_acceleration",
                "MouseKeys acceleration",
            ),
            (Page::RemoteDesktop, "quality", "Picture quality"),
            (Page::Appearance, "glass_dark_to", "Dark until"),
        ] {
            let (_, f) = &pages[p.index()];
            let named = control_labels(f).into_iter().find(|(i, _)| *i == id);
            assert_eq!(named, Some((id, name.to_string())), "{}", p.label());
        }
    }

    /// Labels made from run-time data, compared by id only.
    const RUNTIME_LABELS: &[(Page, &str)] = &[(Page::Background, "home")]; // the user's name

    /// Whether a control's label is left out of search: a tablet row's id is its position on
    /// the pad, so its label changes with the pad.
    fn unlisted(p: Page, id: &str) -> bool {
        p == Page::Tablet && crate::tabletpage::ROW_IDS.contains(&id)
    }

    #[test]
    fn every_control_label_is_in_its_pages_search_terms() {
        let appearance = ["classic", "blue"].map(|interface| {
            let s = Settings {
                interface: interface.into(),
                ..Settings::default()
            };
            (Page::Appearance, crate::appearancepage::page(&s, Area::MIN))
        });
        let mut built: Vec<(Page, Vec<(&str, String)>)> =
            Page::all().map(|p| (p, Vec::new())).collect();
        for (p, f) in every_page(Area::MIN).into_iter().chain(appearance) {
            let (_, terms) = built.iter_mut().find(|(q, _)| *q == p).unwrap();
            for t in control_labels(&f) {
                if !unlisted(p, t.0) && !terms.contains(&t) {
                    terms.push(t);
                }
            }
        }
        let same = |p: Page, a: &(&str, String), b: &(&str, String)| {
            a.0 == b.0 && (a.1 == b.1 || RUNTIME_LABELS.contains(&(p, a.0)))
        };
        let lines = |ts: Vec<&(&str, String)>| -> String {
            ts.iter()
                .map(|(id, l)| format!("        ({id:?}, {l:?}),\n"))
                .collect()
        };
        let mut report = String::new();
        for (p, built) in &built {
            let listed: Vec<(&str, String)> = p
                .terms()
                .iter()
                .map(|(id, l)| (*id, l.to_string()))
                .collect();
            let missing: Vec<_> = built
                .iter()
                .filter(|t| !listed.iter().any(|l| same(*p, t, l)))
                .collect();
            let stale: Vec<_> = listed
                .iter()
                .filter(|l| !built.iter().any(|t| same(*p, t, l)))
                .collect();
            let twice: Vec<_> = listed
                .iter()
                .enumerate()
                .filter(|(i, l)| listed[..*i].iter().any(|k| k.0 == l.0))
                .map(|(_, l)| l)
                .collect();
            if !twice.is_empty() {
                report += &format!("Page::{p:?} lists an id twice:\n{}", lines(twice));
            }
            if !missing.is_empty() {
                report += &format!("Page::{p:?} is missing:\n{}", lines(missing));
            }
            if !stale.is_empty() {
                report += &format!("Page::{p:?} lists, with no such control:\n{}", lines(stale));
            }
        }
        assert!(
            report.is_empty(),
            "Page::terms (crates/repriseconf/src/pages/terms.rs) is out of step:\n{report}"
        );
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
