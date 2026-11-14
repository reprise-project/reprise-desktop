//! How each page of the Reprise Settings sheet is built. Which pages exist
//! and their headings live in [`repriseconf::pages`], shared with the
//! compositor's search; a page's index in `PropSheet::pages` is its place in [`Page::all`].

use reprisefs::xkb::Layout;
use reprisekit::controls::form::Form;
use reprisekit::controls::pagelist;
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
    /// The layout an empty `[keyboard] layout` gives ([`crate::pages::system_layout`]).
    pub system_layout: &'a str,
    pub heads: &'a [repriseapp::heads::Head],
}

/// Page `p` for area `a` before any value is pushed in. The Appearance,
/// Display and Tablet pages depend on what they show; the host rebuilds
/// those ([`crate::appearancepage::follow`], [`crate::displaypage::refresh`],
/// `tabletpage::page`). Remote Desktop starts off and `remotehost` fills it in, as `loginhost`
/// does Login Manager, `airplanehost` Network and `thunderbolthost` Thunderbolt.
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
        Page::Mouse => pages::mouse(a),
        Page::Touchpad => pages::touchpad(a),
        Page::Tablet => crate::tabletpage::page(a, &[], 0, &[]),
        Page::Cursor => pages::cursor(a, src.cursor_themes),
        Page::TextKeys => pages::text_and_keys(a),
        Page::PointerAlerts => pages::pointer_and_alerts(a),
        Page::MagnifierFilters => crate::magnifierpage::page(a),
        Page::Sound => pages::sound(a, src.sound_themes),
        Page::ScreenCapture => crate::capturepage::page(&Settings::default(), a),
        Page::Notifications => pages::notifications(a),
        Page::Startup => pages::startup(a),
        Page::DefaultPrograms => crate::defaultspage::page(a),
        Page::Privacy => pages::privacy(a),
        Page::RegionLanguage => crate::regionpage::page(a),
        Page::Network => pages::network(a),
        Page::Thunderbolt => pages::thunderbolt(a),
        Page::RemoteDesktop => pages::remote_desktop(a, None, false, None),
        Page::LoginManager => pages::login_manager(a),
        Page::Advanced => pages::advanced(a),
        Page::Backup => pages::backup(a),
    }
}

/// Each heading of [`GROUPS`] with its rows, in list order.
fn page_rows() -> Vec<(&'static str, Vec<pagelist::Item<'static>>)> {
    GROUPS
        .iter()
        .map(|(heading, entries)| {
            let labels = entries
                .iter()
                .map(|e| match e {
                    Entry::Page(p) => pagelist::Item::Page(p.label()),
                    Entry::Program(id) => {
                        let p = repriseconf::pages::program(id).expect("every link is in PROGRAMS");
                        pagelist::Item::Link {
                            label: p.row_label(),
                            name: p.label,
                        }
                    }
                })
                .collect();
            (*heading, labels)
        })
        .collect()
}

/// The whole sheet at [`Area::MIN`] with `s` pushed in ([`pages::write`]).
pub fn sheet(s: &Settings, src: &Sources) -> PropSheet {
    let rows = page_rows();
    let groups: Vec<(&str, &[pagelist::Item])> =
        rows.iter().map(|(h, l)| (*h, l.as_slice())).collect();
    let forms = Page::all().map(|p| build(p, src, Area::MIN)).collect();
    let mut sheet = PropSheet::with_list(&groups, forms, pages::CLIENT);
    pages::write(&mut sheet, s, src.layouts, src.system_layout);
    sheet
}

pub fn open(sheet: &PropSheet) -> Page {
    Page::at(sheet.tabs.open).expect("one page per row of the list")
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriseconf::search::all_words;
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
                system_layout: "us",
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
                "Screen Capture",
                "Notifications",
                "Keyboard",
                "Mouse",
                "Touchpad",
                "Cursor",
                "Tablet",
                "Thunderbolt",
                "Text & Keys",
                "Pointer & Alerts",
                "Magnifier & Filters",
                "Network",
                "Remote Desktop",
                "Startup",
                "Default Programs",
                "Privacy",
                "Region & Language",
                "Login Manager",
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
        assert_eq!(Page::Touchpad.slug(), "touchpad");
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
            ("mouse-and-touchpad", Page::Mouse),
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

    /// The list's inside is 456 px at the smallest sheet: 48 rows at 840 px, so it scrolls
    /// until the sheet is 384 px taller. On a sheet 56 px shorter, a page opened from its end
    /// (keys or `--page`) comes into view.
    #[test]
    fn the_list_scrolls_to_the_open_page() {
        use reprisekit::controls::form::Key;
        use reprisekit::controls::propsheet::Hit;
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut sheet = built();
        let list = sheet.layout(&text, px).list.expect("a list sheet");
        assert_eq!(list.rows.len(), 48, "seven headings, 34 pages, seven links");
        assert_eq!((list.total, list.inner.height), (840, 456));
        assert!(list.scrollbar.is_some(), "the rows overflow by 384 px");
        sheet.size.1 += 384;
        let list = sheet.layout(&text, px).list.unwrap();
        assert!(list.scrollbar.is_none(), "every row shows");

        sheet.size.1 -= 440;
        let list = sheet.layout(&text, px).list.unwrap();
        assert_eq!((list.total, list.inner.height), (840, 400));
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

    /// Every row's text, a link's with its dots, fits beside the scrollbar in each look at
    /// scale 1 and 2, where the list is at its narrowest.
    #[test]
    fn every_list_row_fits_beside_the_scrollbar_in_every_look() {
        use reprisekit::interface::Interface;
        use reprisekit::interfaces::theme_for;
        let mut sheet = built();
        let font = sheet.font;
        let mut labels = sheet.tabs.labels.clone();
        let list = sheet.list_mut().unwrap();
        labels.extend(list.links.iter().map(|l| format!("{}...", l.label)));
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = theme_for(i);
            let p = theme.paint();
            let cm = p.look.controls().grown(p.text);
            let lay = pagelist::layout(list, Rect::new(0, 0, pagelist::WIDTH, 456), &cm);
            assert!(lay.scrollbar.is_some());
            for s in [1, 2] {
                let theme = theme_for(i).scaled(s);
                let text = TextRenderer::load(&theme.fonts);
                let room = s * (lay.rows[1].width - pagelist::PAGE_X);
                for l in &labels {
                    let w = text.dialog(font).measure(l, false, theme.fonts.size_px);
                    assert!(
                        w <= room,
                        "{i:?} at {s}x: {l} is {w} px, the row holds {room}"
                    );
                }
            }
        }
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
            system_layout: "us",
            heads: &[],
        };
        let glass = Settings {
            interface: "glass".into(),
            ..Settings::default()
        };
        let mut display = crate::displaypage::Display::with_heads(vec![
            crate::displaypage::test_head("eDP-1"),
            crate::displaypage::test_head("HDMI-A-1"),
        ]);
        display.sensor = Some(false);
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
                    Page::Privacy => Some(pages::permissions(a)),
                    Page::DefaultPrograms => Some(crate::defaultspage::list(a)),
                    _ => None,
                };
                f.items.extend(list.map(side));
                (p, f)
            })
            .chain(capture_variants(a))
            .collect()
    }

    /// The Screen Capture page with each row the format, rate mode and audio codec swap in.
    fn capture_variants(a: Area) -> impl Iterator<Item = (Page, Form)> {
        let jpg = Settings {
            screenshot_format: "jpg".into(),
            recording_rate: "average".into(),
            recording_audio_codec: "flac".into(),
            ..Settings::default()
        };
        let jxl = Settings {
            screenshot_format: "jxl".into(),
            ..Settings::default()
        };
        [jpg, jxl]
            .into_iter()
            .map(move |s| (Page::ScreenCapture, crate::capturepage::page(&s, a)))
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
            (Page::DefaultPrograms, SIDE_LIST),
            (Page::Search, crate::providerpage::LIST),
        ] {
            let (was, now) = grower(p, id);
            assert_eq!(now, was + large.dh, "{}: {id}", p.label());
        }
        let (loc_was, loc_now) = grower(Page::Privacy, SIDE_LIST);
        let (clip_was, clip_now) = grower(Page::Privacy, crate::clipaccess::LIST);
        assert_eq!(
            (loc_now - loc_was) + (clip_now - clip_was),
            large.dh,
            "Privacy's two lists share the extra height"
        );
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
                | C::Arrange(_)
                | C::Progress { .. } => it.name.map(str::to_string).or(label),
            };
            label = None;
            out.extend(named.map(|l| (it.id, l)));
        }
        out
    }

    #[test]
    fn unlabelled_controls_are_named_after_their_setting() {
        let pages = every_page(Area::MIN);
        for (p, id, name) in [
            (Page::Display, crate::displaypage::MODE, "Screen resolution"),
            (Page::NightLight, "nl_temperature", "Color temperature"),
            (Page::Mouse, "mouse_speed", "Pointer speed"),
            (Page::Touchpad, "touchpad_speed", "Touchpad speed"),
            (Page::Mouse, "double_click_ms", "Double-click speed"),
            (Page::Mouse, "mouse_scroll_speed", "Scroll speed"),
            (Page::Touchpad, "touchpad_scroll_speed", "Scroll speed"),
            (Page::TextKeys, "slow_keys_delay", "SlowKeys delay"),
            (Page::TextKeys, "bounce_keys_delay", "BounceKeys delay"),
            (Page::PointerAlerts, "mouse_keys_speed", "MouseKeys speed"),
            (
                Page::PointerAlerts,
                "mouse_keys_acceleration",
                "MouseKeys acceleration",
            ),
            (Page::RemoteDesktop, "quality", "Picture quality"),
            (Page::Appearance, "glass_dark_to", "Dark until"),
            (Page::StartMenu, crate::menupage::TREE, "Start menu"),
            (
                Page::ScreenCapture,
                crate::capturepage::QUALITY,
                "Recording quality",
            ),
        ] {
            let (_, f) = &pages[p.index()];
            let named = control_labels(f).into_iter().find(|(i, _)| *i == id);
            assert_eq!(named, Some((id, name.to_string())), "{}", p.label());
        }
        let variants: Vec<(Page, Form)> = capture_variants(Area::MIN).collect();
        for (f, id, name) in [
            (
                &variants[0].1,
                crate::capturepage::JPG_QUALITY,
                "JPG quality",
            ),
            (
                &variants[1].1,
                crate::capturepage::JXL_QUALITY,
                "JPEG XL quality",
            ),
        ] {
            let named = control_labels(f).into_iter().find(|(i, _)| *i == id);
            assert_eq!(named, Some((id, name.to_string())));
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
        use reprisekit::controls::form::Control as C;
        let mut titles: Vec<(Page, String)> = Vec::new();
        for (p, f) in every_page(Area::MIN).into_iter().chain(appearance) {
            let (_, terms) = built.iter_mut().find(|(q, _)| *q == p).unwrap();
            for t in control_labels(&f) {
                if !unlisted(p, t.0) && !terms.contains(&t) {
                    terms.push(t);
                }
            }
            titles.extend(f.items.iter().filter_map(|it| match &it.control {
                C::Group(t) | C::GroupTitle(t) => Some((p, reprisekit::text::split_mnemonic(t).0)),
                _ => None,
            }));
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
        let unfound: Vec<&(Page, String)> = titles
            .iter()
            .filter(|(p, t)| {
                let hay: Vec<&str> = [p.label()]
                    .into_iter()
                    .chain(p.keywords().iter().copied())
                    .chain(p.terms().iter().map(|&(_, l)| l))
                    .collect();
                !UNSEARCHED_TITLES.contains(&t.as_str()) && !all_words(t, &hay)
            })
            .collect();
        assert!(
            unfound.is_empty(),
            "group titles a search misses, to add as page keywords: {unfound:?}"
        );
    }

    /// Group titles that name nothing a user searches for.
    const UNSEARCHED_TITLES: &[&str] = &["Preview", "Connected Computers", "Sample"];

    #[test]
    fn the_area_follows_the_sheet_size() {
        assert_eq!(Area::MIN.sheet_size(), built().size);
        assert_eq!(Area::of_size((100, 100)), Area::MIN, "never smaller");
        let a = Area::of_size(LARGE);
        assert_eq!(a.sheet_size(), LARGE);
        assert_eq!((a.w, a.dh), (LARGE.0 - 186, LARGE.1 - 528));
    }
}
